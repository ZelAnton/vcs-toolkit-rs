//! Behavioural tests for the MCP server's concurrency contract.
//!
//! These tests deliberately drive the public tool methods from independent Tokio
//! tasks. The runner records command entry/exit and can park one selected spawn,
//! which lets the suite prove ordering without inferring it from mutex internals.

use super::*;
use processkit::testing::{Reply, ScriptedRunner};
use processkit::{Command, ProcessResult, ProcessRunner};
use rmcp::handler::server::wrapper::Parameters;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use vcs_core::vcs_git::Git;
use vcs_forge::vcs_github::GitHub;

const TEST_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
struct Event {
    id: usize,
    entering: bool,
    command: String,
}

/// A scripted runner that exposes actual command overlap and an optional spawn
/// barrier. The barrier is matched by argv prefix (program included).
struct ConcurrencyRunner {
    inner: ScriptedRunner,
    delay: Duration,
    blocked_prefix: Option<Vec<String>>,
    blocked_entered: Semaphore,
    blocked_release: Semaphore,
    next_id: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    events: Mutex<Vec<Event>>,
}

impl ConcurrencyRunner {
    fn new(inner: ScriptedRunner, delay: Duration) -> Self {
        Self {
            inner,
            delay,
            blocked_prefix: None,
            blocked_entered: Semaphore::new(0),
            blocked_release: Semaphore::new(0),
            next_id: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            events: Mutex::new(Vec::new()),
        }
    }

    fn blocking(mut self, prefix: &[&str]) -> Self {
        self.blocked_prefix = Some(prefix.iter().map(|part| (*part).to_owned()).collect());
        self
    }

    fn argv(command: &Command) -> Vec<String> {
        std::iter::once(command.program())
            .chain(command.arguments().iter().map(|arg| arg.as_os_str()))
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn is_blocked(&self, command: &Command) -> bool {
        self.blocked_prefix
            .as_ref()
            .is_some_and(|prefix| Self::argv(command).starts_with(prefix))
    }

    async fn wait_until_blocked(&self) {
        timeout(TEST_DEADLINE, self.blocked_entered.acquire())
            .await
            .expect("the selected command reached the runner before the deadline")
            .expect("the test semaphore stays open")
            .forget();
    }

    fn release_blocked(&self) {
        self.blocked_release.add_permits(1);
    }

    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }

    fn events(&self) -> Vec<Event> {
        self.events.lock().expect("event lock").clone()
    }

    fn record(&self, event: Event) {
        self.events.lock().expect("event lock").push(event);
    }
}

#[async_trait::async_trait]
impl ProcessRunner for ConcurrencyRunner {
    async fn output_string(&self, command: &Command) -> processkit::Result<ProcessResult<String>> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let command_line = command.command_line();
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        self.record(Event {
            id,
            entering: true,
            command: command_line.clone(),
        });

        if self.is_blocked(command) {
            self.blocked_entered.add_permits(1);
            self.blocked_release
                .acquire()
                .await
                .expect("the test semaphore stays open")
                .forget();
        } else if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }

        let result = self.inner.output_string(command).await;
        self.record(Event {
            id,
            entering: false,
            command: command_line,
        });
        self.active.fetch_sub(1, Ordering::SeqCst);
        result
    }
}

fn scripted() -> ScriptedRunner {
    ScriptedRunner::new()
        .on(["git", "symbolic-ref"], Reply::ok("main\n"))
        .on(["gh", "--version"], Reply::ok("gh version 2.95.0\n"))
        .fallback(Reply::ok(""))
}

fn server(runner: Arc<ConcurrencyRunner>, with_forge: bool) -> VcsMcpServer {
    let repo = Repo::from_git("/repo", "/repo", Git::with_runner(runner.clone()));
    let forge = with_forge.then(|| Forge::from_github("/repo", GitHub::with_runner(runner)));
    VcsMcpServer::new(repo, forge, WriteGate::All)
}

async fn join_all(
    handles: Vec<tokio::task::JoinHandle<Result<rmcp::model::CallToolResult, ErrorData>>>,
) {
    timeout(TEST_DEADLINE, async {
        for handle in handles {
            handle
                .await
                .expect("tool task did not panic")
                .expect("tool call succeeded");
        }
    })
    .await
    .expect("parallel MCP workload did not deadlock");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_repo_mutations_never_overlap_runner_spawns() {
    const MUTATIONS: usize = 12;
    let runner = Arc::new(ConcurrencyRunner::new(
        scripted(),
        Duration::from_millis(10),
    ));
    let server = server(runner.clone(), false);

    let handles = (0..MUTATIONS)
        .map(|n| {
            let server = server.clone();
            tokio::spawn(async move {
                server
                    .repo_create_branch(Parameters(CreateBranchParams {
                        name: format!("parallel-{n}"),
                    }))
                    .await
            })
        })
        .collect();
    join_all(handles).await;

    assert_eq!(
        runner.max_active(),
        1,
        "two repo mutations entered the process runner at the same time"
    );
    let events = runner.events();
    assert_eq!(events.len(), MUTATIONS * 2);
    for pair in events.chunks_exact(2) {
        assert!(pair[0].entering, "first event must enter: {pair:?}");
        assert!(!pair[1].entering, "second event must exit: {pair:?}");
        assert_eq!(pair[0].id, pair[1].id, "spawns interleaved: {pair:?}");
        assert_eq!(pair[0].command, pair[1].command);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reads_and_remote_forge_writes_finish_while_repo_writer_holds_lock() {
    let runner =
        Arc::new(ConcurrencyRunner::new(scripted(), Duration::ZERO).blocking(&["git", "branch"]));
    let server = server(runner.clone(), true);

    let writer = {
        let server = server.clone();
        tokio::spawn(async move {
            server
                .repo_create_branch(Parameters(CreateBranchParams {
                    name: "parked-writer".into(),
                }))
                .await
        })
    };
    runner.wait_until_blocked().await;

    let independent = timeout(TEST_DEADLINE, async {
        tokio::join!(
            server.repo_current_branch(),
            server.forge_pr_comment(Parameters(PrCommentParams {
                number: 7,
                body: "remote-only".into(),
            }))
        )
    })
    .await
    .expect("read and remote forge mutation must not wait for the repo write lock");
    independent.0.expect("read completed");
    independent.1.expect("remote forge mutation completed");
    assert!(
        !writer.is_finished(),
        "the writer must still be parked when independent calls finish"
    );

    runner.release_blocked();
    timeout(TEST_DEADLINE, writer)
        .await
        .expect("writer resumed")
        .expect("writer task did not panic")
        .expect("writer succeeded");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mixed_parallel_tool_load_completes_without_deadlock() {
    let runner = Arc::new(ConcurrencyRunner::new(scripted(), Duration::from_millis(3)));
    let server = server(runner, true);
    let mut handles = Vec::new();

    for n in 0..4 {
        let server = server.clone();
        handles.push(tokio::spawn(async move {
            server
                .repo_create_branch(Parameters(CreateBranchParams {
                    name: format!("mixed-{n}"),
                }))
                .await
        }));
    }
    for _ in 0..4 {
        let server = server.clone();
        handles.push(tokio::spawn(
            async move { server.repo_current_branch().await },
        ));
    }
    {
        let server = server.clone();
        handles.push(tokio::spawn(async move {
            server
                .repo_checkout(Parameters(CheckoutParams {
                    reference: "main".into(),
                }))
                .await
        }));
    }
    {
        let server = server.clone();
        handles.push(tokio::spawn(async move {
            server
                .repo_rebase(Parameters(RebaseParams {
                    onto: "main".into(),
                }))
                .await
        }));
    }
    for n in 1..=4 {
        let server = server.clone();
        handles.push(tokio::spawn(async move {
            server
                .forge_pr_comment(Parameters(PrCommentParams {
                    number: n,
                    body: format!("remote-{n}"),
                }))
                .await
        }));
    }
    for n in 10..=12 {
        let server = server.clone();
        handles.push(tokio::spawn(async move {
            server
                .forge_pr_checkout(Parameters(PrNumberParams { number: n }))
                .await
        }));
    }
    {
        let server = server.clone();
        handles.push(tokio::spawn(async move {
            server
                .forge_pr_merge(Parameters(PrMergeParams {
                    number: 20,
                    strategy: MergeStrategyArg::Merge,
                    auto: false,
                    delete_branch: false,
                }))
                .await
        }));
    }
    {
        let server = server.clone();
        handles.push(tokio::spawn(async move {
            server
                .forge_pr_close(Parameters(PrCloseParams {
                    number: 21,
                    delete_branch: false,
                }))
                .await
        }));
    }

    join_all(handles).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_forge_mutations_share_repo_serialization_and_do_not_interleave() {
    let runner = Arc::new(ConcurrencyRunner::new(
        scripted(),
        Duration::from_millis(10),
    ));
    let server = server(runner.clone(), true);

    let repo = {
        let server = server.clone();
        tokio::spawn(async move {
            server
                .repo_create_branch(Parameters(CreateBranchParams {
                    name: "beside-forge".into(),
                }))
                .await
        })
    };
    let checkout = {
        let server = server.clone();
        tokio::spawn(async move {
            server
                .forge_pr_checkout(Parameters(PrNumberParams { number: 21 }))
                .await
        })
    };
    let merge = {
        let server = server.clone();
        tokio::spawn(async move {
            server
                .forge_pr_merge(Parameters(PrMergeParams {
                    number: 22,
                    strategy: MergeStrategyArg::Squash,
                    auto: false,
                    delete_branch: true,
                }))
                .await
        })
    };
    let close = {
        let server = server.clone();
        tokio::spawn(async move {
            server
                .forge_pr_close(Parameters(PrCloseParams {
                    number: 23,
                    delete_branch: true,
                }))
                .await
        })
    };
    join_all(vec![repo, checkout, merge, close]).await;

    assert_eq!(
        runner.max_active(),
        1,
        "local forge mutations must use the repo write lock"
    );
    let starts: Vec<_> = runner
        .events()
        .into_iter()
        .filter(|event| event.entering)
        .map(|event| event.command)
        .collect();
    for (index, command) in starts.iter().enumerate() {
        if command.contains("gh --version") {
            let next = starts
                .get(index + 1)
                .expect("version probe has an operation");
            assert!(
                next.contains("gh pr checkout")
                    || next.contains("gh pr merge")
                    || next.contains("gh pr close"),
                "another task interleaved between a forge version probe and operation: {starts:?}"
            );
        }
    }
    for operation in ["gh pr checkout", "gh pr merge", "gh pr close"] {
        assert!(
            starts.iter().any(|command| command.contains(operation)),
            "missing {operation} in {starts:?}"
        );
    }
}
