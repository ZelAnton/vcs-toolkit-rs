use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use processkit::{CancellationToken, JobRunner, ProcessRunner};
use serde::Serialize;
use vcs_cli_support::OutputBudget;
use vcs_core::vcs_git::{DiffLine, Git, GitApi, GitPush, RefName, RevSpec};
use vcs_core::vcs_jj::Jj;
use vcs_core::{BackendKind, ChangeKind, FileChange, FileDiff, OperationState, Repo, RepoSnapshot};
use vcs_forge::{
    Forge, ForgeApi, ForgeAuth, ForgeCapabilities, ForgeKind,
    vcs_gitea::Gitea,
    vcs_github::{
        GitHub, GitHubApi, GitHubHost, PrCreate as GitHubPrCreate, PullRequest, WorkflowRun,
    },
    vcs_gitlab::GitLab,
};

use crate::cli::{ChangesMode, Invocation, Operation};
use crate::contract::{
    AgentError, AgentResult, CONTRACT_VERSION, DetailKey, ErrorDescriptor, ErrorKind, ExitBand,
    FailureDomain, MachineEnvelope,
};
use crate::redaction::{RedactionPolicy, redact_text};

const DEFAULT_DEADLINE: Duration = Duration::from_secs(120);

/// Policy carried across every outcome implementation and projected onto every
/// typed client. No outcome owns a second process-launch path.
pub struct ExecutionPolicy {
    pub cancellation: CancellationToken,
    pub deadline: Duration,
    pub content_budget: OutputBudget,
}

impl ExecutionPolicy {
    pub fn new(content_max_bytes: usize) -> Self {
        Self {
            cancellation: CancellationToken::new(),
            deadline: DEFAULT_DEADLINE,
            content_budget: OutputBudget::bytes(content_max_bytes),
        }
    }

    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }
}

#[derive(Serialize)]
struct ProbeData {
    contract: ContractCompatibility,
    commands: CommandCapabilities,
    execution: ExecutionCapabilities,
    limits: Limits,
    exit_bands: Vec<ExitBand>,
    error_kinds: Vec<ErrorDescriptor>,
}

#[derive(Serialize)]
struct ContractCompatibility {
    schema: &'static str,
    minimum_client_contract: &'static str,
    maximum_client_contract: &'static str,
    compatibility: &'static str,
}

#[derive(Serialize)]
struct CommandCapabilities {
    supported: Vec<&'static str>,
    reserved: Vec<&'static str>,
}

#[derive(Serialize)]
struct ExecutionCapabilities {
    vcs_backends: Vec<&'static str>,
    forges: Vec<&'static str>,
    subprocess_route: &'static str,
    process_tree_containment: &'static str,
    cancellation: &'static str,
    deadlines: &'static str,
    deadline_default_seconds: u64,
    processkit_cli_composition: &'static str,
    raw_command_escape_hatch: bool,
}

#[derive(Serialize)]
struct Limits {
    machine_output_default_bytes: usize,
    machine_output_max_bytes: usize,
    content_output_default_bytes: usize,
    content_overflow: &'static str,
    machine_paths_included: bool,
}

#[derive(Serialize)]
struct InspectData {
    repository: RepositoryIdentity,
    working_copy: WorkingCopy,
    remotes: Vec<RemoteData>,
    forge: ForgeData,
    capabilities: RepositoryCapabilities,
    read_semantics: ReadSemantics,
}

#[derive(Serialize)]
struct ChangesData {
    repository: RepositoryIdentity,
    mode: &'static str,
    content_max_bytes: usize,
    counts: ChangeCounts,
    files: Vec<ChangedPath>,
    diff: Option<Vec<StructuredFileDiff>>,
    read_semantics: ReadSemantics,
}

#[derive(Serialize)]
struct CommitData {
    repository: RepositoryIdentity,
    before: CommitIdentity,
    after: CommitIdentity,
    included_paths: Vec<MachinePath>,
    unrelated_changes_preserved: bool,
    semantics: CommitSemantics,
}

#[derive(Clone, Serialize)]
struct CommitIdentity {
    revision: String,
    change_id: Option<String>,
}

#[derive(Clone, Copy, Serialize)]
struct CommitSemantics {
    selection: &'static str,
    backend_selection: &'static str,
    refs_advanced: bool,
    index_may_change_for_selected_paths: bool,
    unrelated_index_preserved: bool,
    repository_hooks_executed: bool,
    working_copy_content_mutated: bool,
    push_performed: bool,
    switch_performed: bool,
    conflict_repair_performed: bool,
}

#[derive(Serialize)]
struct PublishData {
    repository: RepositoryIdentity,
    expected_revision: String,
    remote_revision: String,
    remote: String,
    source: String,
    target: String,
    forge: &'static str,
    account: String,
    push: StepEvidence,
    change_request: ChangeRequestEvidence,
    checkpoint: &'static str,
    exact_revision_verified: bool,
}

#[derive(Serialize)]
struct StepEvidence {
    state: &'static str,
    irreversible: bool,
    verified: bool,
}

#[derive(Serialize)]
struct ChangeRequestEvidence {
    state: &'static str,
    number: u64,
    url: String,
    source: String,
    target: String,
}

#[derive(Serialize)]
struct CiData {
    repository: RepositoryIdentity,
    forge: &'static str,
    source: String,
    expected_revision: String,
    exact_revision_verified: bool,
    terminal: bool,
    successful: bool,
    runs: Vec<CiRunEvidence>,
    wait: Option<CiWaitEvidence>,
}

#[derive(Clone, Serialize)]
struct CiRunEvidence {
    id: u64,
    workflow: String,
    revision: String,
    status: String,
    conclusion: Option<String>,
    url: String,
}

#[derive(Serialize)]
struct CiWaitEvidence {
    total_deadline_seconds: u64,
    poll_seconds: u64,
    inactivity_watchdog: &'static str,
    diagnostic_budget: &'static str,
}

#[derive(Clone, Serialize)]
struct RepositoryIdentity {
    backend: &'static str,
    root: MachinePath,
    cwd: MachinePath,
}

#[derive(Serialize)]
struct WorkingCopy {
    branch_kind: &'static str,
    branch: Option<String>,
    revision: Option<String>,
    change_id: Option<String>,
    dirty: bool,
    tracked_changes: Option<usize>,
    untracked: Option<usize>,
    total_changes: usize,
    conflicted: bool,
    conflict_count: Option<usize>,
    operation: &'static str,
    upstream: Option<UpstreamData>,
}

#[derive(Serialize)]
struct UpstreamData {
    branch: String,
    ahead: Option<usize>,
    behind: Option<usize>,
}

#[derive(Serialize)]
struct RemoteData {
    name: String,
    url: String,
}

#[derive(Serialize)]
struct ForgeData {
    detection: &'static str,
    kind: Option<&'static str>,
    remote: Option<String>,
    capabilities: Fact<ForgeCapabilitiesData>,
    auth: Fact<ForgeAuthData>,
}

#[derive(Serialize)]
struct Fact<T> {
    status: &'static str,
    reason: Option<&'static str>,
    value: Option<T>,
}

impl<T> Fact<T> {
    fn known(value: T) -> Self {
        Self {
            status: "known",
            reason: None,
            value: Some(value),
        }
    }

    fn unavailable(reason: &'static str) -> Self {
        Self {
            status: "unavailable",
            reason: Some(reason),
            value: None,
        }
    }

    fn not_applicable() -> Self {
        Self {
            status: "not_applicable",
            reason: None,
            value: None,
        }
    }
}

#[derive(Serialize)]
struct ForgeCapabilitiesData {
    cli_version: Option<String>,
    cli_supported: bool,
    authenticated: bool,
    pr_create: bool,
    pr_comment: bool,
    pr_edit: bool,
    pr_labels: bool,
    pr_checks: bool,
    pr_merge: bool,
    pr_approve: bool,
    pr_request_changes: bool,
    issue_create: bool,
    issue_close: bool,
    issue_reopen: bool,
    issue_comment: bool,
    issue_labels: bool,
    release_create: bool,
    release_delete: bool,
}

#[derive(Serialize)]
struct ForgeAuthData {
    authenticated: Option<bool>,
    active_account: Option<String>,
    accounts: Vec<ForgeAccountData>,
    repository_visible: Option<bool>,
}

#[derive(Serialize)]
struct ForgeAccountData {
    host: String,
    login: String,
    active: Option<bool>,
}

#[derive(Serialize)]
struct RepositoryCapabilities {
    inspect: bool,
    changes_summary: bool,
    changes_full: bool,
    lossless_status_paths: bool,
    full_diff_non_utf8_paths: &'static str,
    raw_cli_fallback: bool,
}

#[derive(Serialize)]
struct ReadSemantics {
    refs_mutated: bool,
    index_mutated: bool,
    working_copy_content_mutated: bool,
    working_copy_snapshot: &'static str,
    operation_log_may_advance: bool,
}

#[derive(Serialize)]
struct ChangeCounts {
    paths: usize,
    added: usize,
    modified: usize,
    deleted: usize,
    renamed: usize,
    files_with_line_diff: usize,
    insertions: usize,
    deletions: usize,
}

#[derive(Serialize)]
struct ChangedPath {
    path: MachinePath,
    old_path: Option<MachinePath>,
    kind: &'static str,
}

#[derive(Serialize)]
struct StructuredFileDiff {
    path: MachinePath,
    old_path: Option<MachinePath>,
    kind: &'static str,
    hunks: Vec<StructuredHunk>,
}

#[derive(Serialize)]
struct StructuredHunk {
    old_start: usize,
    old_lines: usize,
    new_start: usize,
    new_lines: usize,
    section: String,
    lines: Vec<StructuredLine>,
}

#[derive(Serialize)]
struct StructuredLine {
    kind: &'static str,
    text: String,
}

/// JSON-safe, lossless native-path representation. `value` is a UTF-8 path
/// when possible, otherwise a hex encoding of the OS-native units. `display`
/// is informational and may be lossy; consumers that round-trip use `value`.
#[derive(Clone, Serialize)]
struct MachinePath {
    display: String,
    encoding: &'static str,
    value: Option<String>,
}

impl MachinePath {
    fn from_path(path: &Path, include: bool) -> Self {
        if !include {
            return Self {
                display: "[REDACTED_PATH]".to_owned(),
                encoding: "redacted",
                value: None,
            };
        }
        if let Some(value) = path.to_str() {
            return Self {
                display: value.to_owned(),
                encoding: "utf-8",
                value: Some(value.to_owned()),
            };
        }
        machine_path_non_utf8(path)
    }
}

pub async fn execute(
    invocation: &Invocation,
    policy: &ExecutionPolicy,
) -> AgentResult<MachineEnvelope> {
    with_outcome_deadline(
        invocation.operation,
        invocation.include_machine_paths,
        policy,
        execute_inner(invocation, policy),
    )
    .await
}

async fn with_outcome_deadline<T>(
    operation: Operation,
    include_paths: bool,
    policy: &ExecutionPolicy,
    outcome: impl Future<Output = AgentResult<T>>,
) -> AgentResult<T> {
    tokio::pin!(outcome);
    let deadline = tokio::time::sleep(policy.deadline);
    tokio::pin!(deadline);
    tokio::select! {
        biased;
        _ = &mut deadline => {
            policy.cancellation.cancel();
            Err(Box::new(
                AgentError::new(
                    operation.name(),
                    ErrorKind::Timeout,
                    "outcome_deadline_exceeded",
                    true,
                )
                .include_machine_paths(include_paths),
            ))
        }
        result = &mut outcome => result,
    }
}

async fn execute_inner(
    invocation: &Invocation,
    policy: &ExecutionPolicy,
) -> AgentResult<MachineEnvelope> {
    match invocation.operation {
        Operation::Probe => Ok(probe(invocation)),
        Operation::Inspect
        | Operation::Changes
        | Operation::Commit
        | Operation::Publish
        | Operation::CiStatus
        | Operation::CiWait => {
            let repository = invocation
                .repository
                .as_deref()
                .ok_or_else(|| Box::new(AgentError::internal("parsed_repository_missing")))?;
            let repo = open_repo(
                repository,
                invocation.operation,
                invocation.include_machine_paths,
                policy,
            )?;
            execute_repository(invocation, policy, &repo).await
        }
    }
}

async fn execute_repository<R: ProcessRunner>(
    invocation: &Invocation,
    policy: &ExecutionPolicy,
    repo: &Repo<R>,
) -> AgentResult<MachineEnvelope> {
    match invocation.operation {
        Operation::Inspect => {
            let remotes = repo.remotes().await.map_err(|error| {
                Box::new(map_core_error(
                    Operation::Inspect,
                    invocation.include_machine_paths,
                    error,
                ))
            })?;
            let forge_remote = preferred_forge_remote(&remotes)
                .map(|(remote, _)| redact_metadata(&remote.url, invocation.include_machine_paths));
            let forge = build_forge(&remotes, repo.cwd(), policy);
            let data = inspect_repo(
                repo,
                remotes,
                forge.as_deref(),
                forge_remote,
                invocation.include_machine_paths,
            )
            .await?;
            Ok(MachineEnvelope::success(Operation::Inspect.name(), data))
        }
        Operation::Changes => {
            let data = changes_repo(
                repo,
                invocation.changes_mode,
                invocation.content_max_bytes,
                invocation.include_machine_paths,
            )
            .await?;
            Ok(MachineEnvelope::success(Operation::Changes.name(), data))
        }
        Operation::Commit => {
            let data = commit_repo(repo, invocation).await?;
            Ok(MachineEnvelope::success(Operation::Commit.name(), data))
        }
        Operation::Publish => {
            let data = publish_repo(repo, invocation, policy).await?;
            Ok(MachineEnvelope::success(Operation::Publish.name(), data))
        }
        Operation::CiStatus => {
            let data = ci_status_repo(repo, invocation, policy).await?;
            Ok(MachineEnvelope::success(Operation::CiStatus.name(), data))
        }
        Operation::CiWait => {
            let data = ci_wait_repo(repo, invocation, policy).await?;
            Ok(MachineEnvelope::success(Operation::CiWait.name(), data))
        }
        _ => Err(Box::new(AgentError::internal(
            "repository_operation_mismatch",
        ))),
    }
}

fn probe(invocation: &Invocation) -> MachineEnvelope {
    MachineEnvelope::success(
        Operation::Probe.name(),
        ProbeData {
            contract: ContractCompatibility {
                schema: "https://zelanton.github.io/vcs-toolkit-rs/vcs-agent/v1/envelope.schema.json",
                minimum_client_contract: CONTRACT_VERSION,
                maximum_client_contract: CONTRACT_VERSION,
                compatibility: "same-contract-version",
            },
            commands: CommandCapabilities {
                supported: vec![
                    "probe",
                    "inspect",
                    "changes",
                    "commit",
                    "publish",
                    "ci status",
                    "ci wait",
                ],
                reserved: vec![],
            },
            execution: ExecutionCapabilities {
                vcs_backends: vec!["git", "jujutsu"],
                forges: vec!["github", "gitlab", "gitea"],
                subprocess_route: "vcs-toolkit-typed-clients",
                process_tree_containment: "processkit-rs",
                cancellation: "processkit-cancellation-token",
                deadlines: "per-outcome",
                deadline_default_seconds: DEFAULT_DEADLINE.as_secs(),
                processkit_cli_composition: "external-executable",
                raw_command_escape_hatch: false,
            },
            limits: Limits {
                machine_output_default_bytes: crate::cli::DEFAULT_MAX_OUTPUT_BYTES,
                machine_output_max_bytes: crate::cli::MAX_MAX_OUTPUT_BYTES,
                content_output_default_bytes: crate::cli::DEFAULT_CONTENT_MAX_BYTES,
                content_overflow: "fail-loud-no-truncation",
                machine_paths_included: invocation.include_machine_paths,
            },
            exit_bands: ExitBand::contract(),
            error_kinds: ErrorKind::contract(),
        },
    )
}

async fn publish_repo<R: ProcessRunner>(
    repo: &Repo<R>,
    invocation: &Invocation,
    policy: &ExecutionPolicy,
) -> AgentResult<PublishData> {
    let include_paths = invocation.include_machine_paths;
    let expected = invocation
        .expected_revision
        .as_deref()
        .expect("publish parser requires expected revision");
    let remote = invocation
        .remote
        .as_deref()
        .expect("publish parser requires remote");
    let source = invocation
        .source
        .as_deref()
        .expect("publish parser requires source");
    let target = invocation
        .target
        .as_deref()
        .expect("publish parser requires target");
    let expected_remote = invocation
        .expected_remote_revision
        .as_deref()
        .expect("publish parser requires expected remote revision");
    let expected_forge = invocation
        .forge
        .as_deref()
        .expect("publish parser requires forge");
    let expected_account = invocation
        .expected_account
        .as_deref()
        .expect("publish parser requires account");

    if repo.kind() != BackendKind::Git {
        return Err(Box::new(
            AgentError::new(
                Operation::Publish.name(),
                ErrorKind::Unsupported,
                "jujutsu_exact_push_unsupported",
                false,
            )
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(include_paths),
        ));
    }
    let Some(git) = repo.git() else {
        return Err(Box::new(AgentError::internal("git_client_missing")));
    };
    let source_ref = checked_ref(source, Operation::Publish, include_paths)?;
    let target_ref = checked_ref(target, Operation::Publish, include_paths)?;
    let expected_spec = RevSpec::new(expected.to_owned()).map_err(|_| {
        Box::new(
            AgentError::invalid_input_for(Operation::Publish.name(), "expected_revision_invalid")
                .include_machine_paths(include_paths),
        )
    })?;
    let local_revision = git
        .resolve_commit(repo.cwd(), &expected_spec)
        .await
        .map_err(|error| {
            Box::new(
                AgentError::from_processkit(
                    Operation::Publish.name(),
                    FailureDomain::Backend,
                    &error,
                )
                .include_machine_paths(include_paths),
            )
        })?;
    if local_revision != expected {
        return Err(Box::new(publish_denied(
            "expected_revision_must_be_full_object_id",
            include_paths,
        )));
    }
    let current_branch = git.current_branch(repo.cwd()).await.map_err(|error| {
        Box::new(
            AgentError::from_processkit(Operation::Publish.name(), FailureDomain::Backend, &error)
                .include_machine_paths(include_paths),
        )
    })?;
    if current_branch.as_deref() != Some(source) {
        return Err(Box::new(publish_denied(
            "source_branch_not_current",
            include_paths,
        )));
    }
    let head = git
        .resolve_commit(
            repo.cwd(),
            &RevSpec::new("HEAD").expect("literal HEAD is valid"),
        )
        .await
        .map_err(|error| {
            Box::new(
                AgentError::from_processkit(
                    Operation::Publish.name(),
                    FailureDomain::Backend,
                    &error,
                )
                .include_machine_paths(include_paths),
            )
        })?;
    if head != local_revision {
        return Err(Box::new(publish_denied(
            "expected_revision_not_current_head",
            include_paths,
        )));
    }

    let remotes = repo
        .remotes()
        .await
        .map_err(|error| Box::new(map_core_error(Operation::Publish, include_paths, error)))?;
    let selected = remotes
        .iter()
        .filter(|candidate| candidate.name == remote)
        .collect::<Vec<_>>();
    if selected.len() != 1 {
        return Err(Box::new(publish_denied(
            if selected.is_empty() {
                "selected_remote_missing"
            } else {
                "selected_remote_ambiguous"
            },
            include_paths,
        )));
    }
    if remote != "origin" {
        return Err(Box::new(
            AgentError::new(
                Operation::Publish.name(),
                ErrorKind::Unsupported,
                "forge_repository_binding_for_non_origin_unsupported",
                false,
            )
            .with_detail(DetailKey::Remote, remote)
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(include_paths),
        ));
    }
    let detected_forge = ForgeKind::from_remote_url(&selected[0].url);
    if detected_forge.map(forge_kind) != Some(expected_forge) {
        return Err(Box::new(
            publish_denied("remote_forge_identity_mismatch", include_paths)
                .with_detail(DetailKey::Forge, expected_forge),
        ));
    }
    if detected_forge != Some(ForgeKind::GitHub) {
        return Err(Box::new(
            AgentError::new(
                Operation::Publish.name(),
                ErrorKind::Unsupported,
                "forge_account_or_idempotent_publish_unsupported",
                false,
            )
            .with_detail(DetailKey::Forge, expected_forge)
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(include_paths),
        ));
    }

    let github = verified_github_repository(
        repo.cwd(),
        &selected[0].url,
        policy,
        Operation::Publish,
        include_paths,
    )
    .await?;
    let capabilities = github.capabilities().await.map_err(|error| {
        Box::new(
            AgentError::from_processkit(Operation::Publish.name(), FailureDomain::Forge, &error)
                .with_detail(DetailKey::Checkpoint, "preflight")
                .include_machine_paths(include_paths),
        )
    })?;
    if !capabilities.is_supported() {
        return Err(Box::new(
            AgentError::new(
                Operation::Publish.name(),
                ErrorKind::Unsupported,
                "publish_capability_unavailable",
                false,
            )
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(include_paths),
        ));
    }
    let auth = github.auth_info().await.map_err(|error| {
        Box::new(
            AgentError::from_processkit(
                Operation::Publish.name(),
                FailureDomain::Authentication,
                &error,
            )
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(include_paths),
        )
    })?;
    if !auth.authed || auth.active().map(|account| account.login.as_str()) != Some(expected_account)
    {
        return Err(Box::new(
            AgentError::new(
                Operation::Publish.name(),
                ErrorKind::Authentication,
                "forge_account_identity_mismatch",
                false,
            )
            .with_detail(DetailKey::Account, expected_account)
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(include_paths),
        ));
    }

    let before_pr = find_change_request(
        &github,
        repo.cwd(),
        source,
        target,
        &local_revision,
        "preflight",
        include_paths,
    )
    .await?;
    let remote_before = git
        .remote_branch_revision(repo.cwd(), remote, &source_ref)
        .await
        .map_err(|error| {
            Box::new(
                AgentError::from_processkit(
                    Operation::Publish.name(),
                    FailureDomain::Backend,
                    &error,
                )
                .include_machine_paths(include_paths),
            )
        })?;
    let expected_remote = if expected_remote == "absent" {
        None
    } else {
        Some(expected_remote.to_owned())
    };
    let push_state = if remote_before.as_deref() == Some(local_revision.as_str()) {
        "already_satisfied"
    } else {
        if remote_before != expected_remote {
            return Err(Box::new(
                publish_denied("remote_revision_preflight_mismatch", include_paths)
                    .with_detail(
                        DetailKey::Revision,
                        remote_before.unwrap_or_else(|| "absent".into()),
                    )
                    .with_detail(DetailKey::Checkpoint, "before_push"),
            ));
        }
        let revision_ref = RefName::new(local_revision.clone())
            .map_err(|_| Box::new(AgentError::internal("resolved_git_object_id_invalid")))?;
        let push = git
            .push(
                repo.cwd(),
                GitPush::refspec(&revision_ref, &source_ref).remote(remote),
            )
            .await;
        match push {
            Ok(()) => "performed",
            Err(error) => {
                let observed = git
                    .remote_branch_revision(repo.cwd(), remote, &source_ref)
                    .await
                    .map_err(|_| {
                        Box::new(
                            publish_unknown("push_postflight_unavailable", include_paths)
                                .with_detail(DetailKey::Checkpoint, "after_push"),
                        )
                    })?;
                if observed.as_deref() == Some(local_revision.as_str()) {
                    "recovered_after_error"
                } else if error.is_timeout() || error.is_cancelled() {
                    return Err(Box::new(
                        publish_unknown("push_outcome_unknown", include_paths)
                            .with_detail(DetailKey::Checkpoint, "after_push"),
                    ));
                } else if observed == expected_remote {
                    return Err(Box::new(
                        AgentError::from_processkit(
                            Operation::Publish.name(),
                            FailureDomain::Backend,
                            &error,
                        )
                        .with_detail(DetailKey::Checkpoint, "push_not_applied")
                        .include_machine_paths(include_paths),
                    ));
                } else {
                    return Err(Box::new(
                        publish_unknown("remote_changed_during_push", include_paths)
                            .with_detail(DetailKey::Checkpoint, "after_push"),
                    ));
                }
            }
        }
    };
    let remote_after = git
        .remote_branch_revision(repo.cwd(), remote, &source_ref)
        .await
        .map_err(|_| {
            Box::new(
                publish_unknown("push_postflight_unavailable", include_paths)
                    .with_detail(DetailKey::Checkpoint, "after_push"),
            )
        })?;
    if remote_after.as_deref() != Some(local_revision.as_str()) {
        return Err(Box::new(
            publish_unknown("push_postflight_revision_mismatch", include_paths)
                .with_detail(DetailKey::Checkpoint, "after_push"),
        ));
    }

    let (change_request, pr_state) = if let Some(pr) = before_pr {
        (pr, "already_satisfied")
    } else if let Some(pr) = find_change_request(
        &github,
        repo.cwd(),
        source,
        target,
        &local_revision,
        "after_push",
        include_paths,
    )
    .await?
    {
        (pr, "discovered_after_push")
    } else {
        let create = GitHubPrCreate::new(
            invocation.title.as_deref().expect("publish title"),
            invocation.body.as_deref().expect("publish body"),
        )
        .head(source)
        .base(target);
        match github.pr_create(repo.cwd(), create).await {
            Ok(created_url) => {
                let Some(mut pr) = find_change_request(
                    &github,
                    repo.cwd(),
                    source,
                    target,
                    &local_revision,
                    "after_pr_create",
                    include_paths,
                )
                .await?
                else {
                    return Err(Box::new(
                        publish_unknown("change_request_create_unverified", include_paths)
                            .with_detail(DetailKey::Checkpoint, "after_pr_create")
                            .with_detail(DetailKey::Revision, &local_revision),
                    ));
                };
                if pr.url.is_empty() {
                    pr.url = created_url;
                }
                (pr, "created")
            }
            Err(error) => match find_change_request(
                &github,
                repo.cwd(),
                source,
                target,
                &local_revision,
                "after_pr_create",
                include_paths,
            )
            .await
            {
                Ok(Some(pr)) => (pr, "recovered_after_error"),
                Ok(None) => {
                    let mapped = AgentError::from_processkit(
                        Operation::Publish.name(),
                        FailureDomain::Forge,
                        &error,
                    )
                    .with_detail(DetailKey::Checkpoint, "push_succeeded_pr_failed")
                    .with_detail(DetailKey::Revision, &local_revision)
                    .include_machine_paths(include_paths);
                    return Err(Box::new(mapped));
                }
                Err(_) => {
                    return Err(Box::new(
                        publish_unknown("change_request_outcome_unknown", include_paths)
                            .with_detail(DetailKey::Checkpoint, "after_pr_create")
                            .with_detail(DetailKey::Revision, &local_revision),
                    ));
                }
            },
        }
    };

    Ok(PublishData {
        repository: repository_identity(repo, include_paths),
        expected_revision: local_revision.clone(),
        remote_revision: local_revision,
        remote: redact_metadata(remote, include_paths),
        source: source.to_owned(),
        target: target_ref.as_str().to_owned(),
        forge: "github",
        account: redact_metadata(expected_account, include_paths),
        push: StepEvidence {
            state: push_state,
            irreversible: true,
            verified: true,
        },
        change_request: ChangeRequestEvidence {
            state: pr_state,
            number: change_request.number,
            url: redact_metadata(&change_request.url, include_paths),
            source: change_request.head_ref_name,
            target: change_request.base_ref_name,
        },
        checkpoint: "publish_complete",
        exact_revision_verified: true,
    })
}

async fn ci_status_repo<R: ProcessRunner>(
    repo: &Repo<R>,
    invocation: &Invocation,
    policy: &ExecutionPolicy,
) -> AgentResult<CiData> {
    query_ci(repo, invocation, policy, false).await
}

async fn ci_wait_repo<R: ProcessRunner>(
    repo: &Repo<R>,
    invocation: &Invocation,
    policy: &ExecutionPolicy,
) -> AgentResult<CiData> {
    let source = invocation.source.as_deref().expect("ci source");
    let expected = invocation
        .expected_revision
        .as_deref()
        .expect("ci expected revision");
    let github = checked_github_ci(repo, invocation, policy).await?;
    loop {
        let runs = github
            .run_list(repo.cwd(), 100, Some(source.to_owned()))
            .await
            .map_err(|error| {
                Box::new(
                    AgentError::from_processkit(
                        Operation::CiWait.name(),
                        FailureDomain::Forge,
                        &error,
                    )
                    .include_machine_paths(invocation.include_machine_paths),
                )
            })?;
        let exact = select_exact_ci_runs(
            runs,
            expected,
            Operation::CiWait,
            invocation.include_machine_paths,
        )?;
        if exact.is_empty() {
            tokio::time::sleep(Duration::from_secs(invocation.poll_seconds)).await;
            continue;
        }
        for run in exact.iter().filter(|run| !ci_run_terminal(run)) {
            let completed = github
                .run_watch(repo.cwd(), run.database_id)
                .await
                .map_err(|error| {
                    Box::new(
                        AgentError::from_processkit(
                            Operation::CiWait.name(),
                            FailureDomain::Forge,
                            &error,
                        )
                        .with_detail(DetailKey::Checkpoint, "pr_succeeded_ci_interrupted")
                        .with_detail(DetailKey::Revision, expected)
                        .with_detail(DetailKey::RunId, run.database_id.to_string())
                        .include_machine_paths(invocation.include_machine_paths),
                    )
                })?;
            ensure_ci_run_revision_unchanged(
                &completed,
                expected,
                invocation.include_machine_paths,
            )?;
        }
        let mut data = query_ci(repo, invocation, policy, true).await?;
        data.wait = Some(CiWaitEvidence {
            total_deadline_seconds: invocation.wait_seconds,
            poll_seconds: invocation.poll_seconds,
            inactivity_watchdog: "github-run-watch-300s",
            diagnostic_budget: "processkit-drop-oldest-256KiB-256-lines",
        });
        if data.terminal && data.successful {
            return Ok(data);
        }
        if data.terminal {
            let conclusion = data
                .runs
                .iter()
                .filter_map(|run| run.conclusion.as_deref())
                .collect::<Vec<_>>()
                .join(",");
            return Err(Box::new(
                AgentError::new(
                    Operation::CiWait.name(),
                    ErrorKind::Forge,
                    "ci_terminal_not_successful",
                    false,
                )
                .with_detail(DetailKey::Revision, expected)
                .with_detail(DetailKey::Conclusion, conclusion)
                .include_machine_paths(invocation.include_machine_paths),
            ));
        }
    }
}

async fn query_ci<R: ProcessRunner>(
    repo: &Repo<R>,
    invocation: &Invocation,
    policy: &ExecutionPolicy,
    waiting: bool,
) -> AgentResult<CiData> {
    let source = invocation.source.as_deref().expect("ci source");
    let expected = invocation
        .expected_revision
        .as_deref()
        .expect("ci expected revision");
    let github = checked_github_ci(repo, invocation, policy).await?;
    let runs = github
        .run_list(repo.cwd(), 100, Some(source.to_owned()))
        .await
        .map_err(|error| {
            Box::new(
                AgentError::from_processkit(
                    invocation.operation.name(),
                    FailureDomain::Forge,
                    &error,
                )
                .include_machine_paths(invocation.include_machine_paths),
            )
        })?;
    let exact = select_exact_ci_runs(
        runs,
        expected,
        invocation.operation,
        invocation.include_machine_paths,
    )?;
    if exact.is_empty() {
        return Err(Box::new(
            AgentError::new(
                invocation.operation.name(),
                ErrorKind::Denied,
                "ci_expected_revision_not_found",
                waiting,
            )
            .with_detail(DetailKey::Revision, expected)
            .include_machine_paths(invocation.include_machine_paths),
        ));
    }
    let terminal = exact.iter().all(ci_run_terminal);
    let successful = terminal && exact.iter().all(|run| run.conclusion == "success");
    let runs = exact.into_iter().map(map_ci_run).collect();
    Ok(CiData {
        repository: repository_identity(repo, invocation.include_machine_paths),
        forge: "github",
        source: source.to_owned(),
        expected_revision: expected.to_owned(),
        exact_revision_verified: true,
        terminal,
        successful,
        runs,
        wait: None,
    })
}

async fn checked_github_ci<R: ProcessRunner>(
    repo: &Repo<R>,
    invocation: &Invocation,
    policy: &ExecutionPolicy,
) -> AgentResult<GitHub<JobRunner>> {
    let forge = invocation.forge.as_deref().expect("ci forge");
    if forge != "github" {
        return Err(Box::new(
            AgentError::new(
                invocation.operation.name(),
                ErrorKind::Unsupported,
                "exact_revision_ci_unsupported_for_forge",
                false,
            )
            .with_detail(DetailKey::Forge, forge)
            .include_machine_paths(invocation.include_machine_paths),
        ));
    }
    let remotes = repo.remotes().await.map_err(|error| {
        Box::new(map_core_error(
            invocation.operation,
            invocation.include_machine_paths,
            error,
        ))
    })?;
    let origin = remotes
        .iter()
        .filter(|remote| remote.name == "origin")
        .collect::<Vec<_>>();
    if origin.len() != 1 {
        return Err(Box::new(
            AgentError::new(
                invocation.operation.name(),
                ErrorKind::Denied,
                if origin.is_empty() {
                    "origin_remote_missing"
                } else {
                    "origin_remote_ambiguous"
                },
                false,
            )
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(invocation.include_machine_paths),
        ));
    }
    if ForgeKind::from_remote_url(&origin[0].url) != Some(ForgeKind::GitHub) {
        return Err(Box::new(
            AgentError::new(
                invocation.operation.name(),
                ErrorKind::Denied,
                "origin_forge_identity_mismatch",
                false,
            )
            .with_detail(DetailKey::Forge, forge)
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(invocation.include_machine_paths),
        ));
    }
    verified_github_repository(
        repo.cwd(),
        &origin[0].url,
        policy,
        invocation.operation,
        invocation.include_machine_paths,
    )
    .await
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GitHubRepositoryIdentity {
    owner: String,
    name: String,
}

fn configured_github_client<R: ProcessRunner>(
    client: GitHub<R>,
    remote_url: &str,
    policy: &ExecutionPolicy,
    operation: Operation,
    include_paths: bool,
) -> AgentResult<GitHub<R>> {
    let host = GitHubHost::from_remote_url(remote_url).map_err(|_| {
        Box::new(
            AgentError::new(
                operation.name(),
                ErrorKind::Denied,
                "github_remote_host_unverified",
                false,
            )
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(include_paths),
        )
    })?;
    Ok(client
        .default_timeout(policy.deadline)
        .default_cancel_on(policy.cancellation.clone())
        .default_output_budget(policy.content_budget)
        .default_env_remove("GH_REPO")
        .with_host(host))
}

async fn verified_github_repository(
    cwd: &Path,
    remote_url: &str,
    policy: &ExecutionPolicy,
    operation: Operation,
    include_paths: bool,
) -> AgentResult<GitHub<JobRunner>> {
    let expected = github_repository_identity(remote_url).ok_or_else(|| {
        Box::new(
            AgentError::new(
                operation.name(),
                ErrorKind::Denied,
                "github_remote_repository_identity_unverified",
                false,
            )
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(include_paths),
        )
    })?;
    let github =
        configured_github_client(GitHub::new(), remote_url, policy, operation, include_paths)?;
    verify_github_repository(&github, cwd, &expected, operation, include_paths).await?;
    Ok(github)
}

async fn verify_github_repository<R: ProcessRunner>(
    github: &GitHub<R>,
    cwd: &Path,
    expected: &GitHubRepositoryIdentity,
    operation: Operation,
    include_paths: bool,
) -> AgentResult<()> {
    let actual = github.repo_view(cwd).await.map_err(|error| {
        Box::new(
            AgentError::from_processkit(operation.name(), FailureDomain::Forge, &error)
                .with_detail(DetailKey::Checkpoint, "preflight")
                .include_machine_paths(include_paths),
        )
    })?;
    if !actual.owner.eq_ignore_ascii_case(&expected.owner)
        || !actual.name.eq_ignore_ascii_case(&expected.name)
    {
        return Err(Box::new(
            AgentError::new(
                operation.name(),
                ErrorKind::Denied,
                "forge_repository_identity_mismatch",
                false,
            )
            .with_detail(DetailKey::Checkpoint, "preflight")
            .include_machine_paths(include_paths),
        ));
    }
    Ok(())
}

fn github_repository_identity(remote_url: &str) -> Option<GitHubRepositoryIdentity> {
    let url = remote_url.trim();
    if url.is_empty() || url.contains(['?', '#', '\\', '%']) || url.chars().any(char::is_whitespace)
    {
        return None;
    }

    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/')?.1
    } else if let Some((authority, path)) = url.split_once(':') {
        if authority.contains('/') || !authority.contains('.') {
            return None;
        }
        path
    } else {
        url.split_once('/')?.1
    };
    let path = path.strip_suffix('/').unwrap_or(path);
    let mut components = path.split('/');
    let owner = components.next()?;
    let name_with_suffix = components.next()?;
    let name = name_with_suffix
        .strip_suffix(".git")
        .unwrap_or(name_with_suffix);
    if components.next().is_some()
        || owner.is_empty()
        || name.is_empty()
        || matches!(owner, "." | "..")
        || matches!(name, "." | "..")
    {
        return None;
    }
    Some(GitHubRepositoryIdentity {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

fn select_exact_ci_runs(
    runs: Vec<WorkflowRun>,
    expected: &str,
    operation: Operation,
    include_paths: bool,
) -> AgentResult<Vec<WorkflowRun>> {
    let mut exact = runs
        .into_iter()
        .filter(|run| run.head_sha == expected)
        .collect::<Vec<_>>();
    exact.sort_by(|left, right| {
        ci_workflow_name(left)
            .cmp(ci_workflow_name(right))
            .then(left.database_id.cmp(&right.database_id))
    });
    for pair in exact.windows(2) {
        if ci_workflow_name(&pair[0]) == ci_workflow_name(&pair[1]) {
            return Err(Box::new(
                AgentError::new(
                    operation.name(),
                    ErrorKind::Denied,
                    "ci_revision_match_ambiguous",
                    false,
                )
                .with_detail(DetailKey::Revision, expected)
                .include_machine_paths(include_paths),
            ));
        }
    }
    Ok(exact)
}

fn ci_workflow_name(run: &WorkflowRun) -> &str {
    if run.workflow_name.is_empty() {
        &run.name
    } else {
        &run.workflow_name
    }
}

fn ci_run_terminal(run: &WorkflowRun) -> bool {
    run.status == "completed" && !run.conclusion.is_empty()
}

fn ensure_ci_run_revision_unchanged(
    completed: &WorkflowRun,
    expected_revision: &str,
    include_paths: bool,
) -> AgentResult<()> {
    if completed.head_sha == expected_revision {
        return Ok(());
    }
    Err(Box::new(
        AgentError::new(
            Operation::CiWait.name(),
            ErrorKind::OutcomeUnknown,
            "ci_run_revision_changed",
            true,
        )
        .with_detail(DetailKey::Checkpoint, "ci_postflight")
        .with_detail(DetailKey::Revision, expected_revision)
        .with_detail(DetailKey::RunId, completed.database_id.to_string())
        .include_machine_paths(include_paths),
    ))
}

fn map_ci_run(run: WorkflowRun) -> CiRunEvidence {
    CiRunEvidence {
        id: run.database_id,
        workflow: ci_workflow_name(&run).to_owned(),
        revision: run.head_sha,
        status: run.status,
        conclusion: (!run.conclusion.is_empty()).then_some(run.conclusion),
        url: run.url,
    }
}

async fn find_change_request<R: ProcessRunner>(
    github: &GitHub<R>,
    cwd: &Path,
    source: &str,
    target: &str,
    expected_revision: &str,
    checkpoint: &'static str,
    include_paths: bool,
) -> AgentResult<Option<PullRequest>> {
    let matches = github
        .pr_list_for_branch(cwd, source, target)
        .await
        .map_err(|error| {
            Box::new(
                AgentError::from_processkit(
                    Operation::Publish.name(),
                    FailureDomain::Forge,
                    &error,
                )
                .with_detail(DetailKey::Checkpoint, checkpoint)
                .with_detail(DetailKey::Revision, expected_revision)
                .include_machine_paths(include_paths),
            )
        })?;
    select_verified_change_request(
        matches,
        source,
        target,
        expected_revision,
        checkpoint,
        include_paths,
    )
}

fn select_verified_change_request(
    candidates: Vec<PullRequest>,
    source: &str,
    target: &str,
    expected_revision: &str,
    checkpoint: &'static str,
    include_paths: bool,
) -> AgentResult<Option<PullRequest>> {
    let mut exact = Vec::new();
    for candidate in candidates.into_iter().filter(|candidate| {
        candidate.state == "OPEN"
            && candidate.head_ref_name == source
            && candidate.base_ref_name == target
    }) {
        match candidate.is_cross_repository {
            Some(true) => continue,
            None => {
                return Err(Box::new(
                    AgentError::new(
                        Operation::Publish.name(),
                        ErrorKind::Unsupported,
                        "change_request_repository_identity_unavailable",
                        false,
                    )
                    .with_detail(DetailKey::Checkpoint, checkpoint)
                    .with_detail(DetailKey::Revision, expected_revision)
                    .include_machine_paths(include_paths),
                ));
            }
            Some(false) => {}
        }
        if candidate.head_ref_oid.is_empty() {
            return Err(Box::new(
                AgentError::new(
                    Operation::Publish.name(),
                    ErrorKind::Unsupported,
                    "change_request_revision_identity_unavailable",
                    false,
                )
                .with_detail(DetailKey::Checkpoint, checkpoint)
                .with_detail(DetailKey::Revision, expected_revision)
                .include_machine_paths(include_paths),
            ));
        }
        if candidate.head_ref_oid != expected_revision {
            return Err(Box::new(
                publish_denied("change_request_revision_mismatch", include_paths)
                    .with_detail(DetailKey::Checkpoint, checkpoint)
                    .with_detail(DetailKey::Revision, expected_revision),
            ));
        }
        exact.push(candidate);
    }

    match exact.len() {
        0 => Ok(None),
        1 => Ok(exact.into_iter().next()),
        _ => Err(Box::new(
            publish_denied("change_request_match_ambiguous", include_paths)
                .with_detail(DetailKey::Source, source)
                .with_detail(DetailKey::Target, target)
                .with_detail(DetailKey::Revision, expected_revision)
                .with_detail(DetailKey::Checkpoint, checkpoint),
        )),
    }
}

fn checked_ref(value: &str, operation: Operation, include_paths: bool) -> AgentResult<RefName> {
    RefName::new(value.to_owned()).map_err(|_| {
        Box::new(
            AgentError::invalid_input_for(operation.name(), "branch_name_invalid")
                .include_machine_paths(include_paths),
        )
    })
}

fn publish_denied(code: &'static str, include_paths: bool) -> AgentError {
    AgentError::new(Operation::Publish.name(), ErrorKind::Denied, code, false)
        .include_machine_paths(include_paths)
}

fn publish_unknown(code: &'static str, include_paths: bool) -> AgentError {
    AgentError::new(
        Operation::Publish.name(),
        ErrorKind::OutcomeUnknown,
        code,
        true,
    )
    .include_machine_paths(include_paths)
}

async fn commit_repo<R: ProcessRunner>(
    repo: &Repo<R>,
    invocation: &Invocation,
) -> AgentResult<CommitData> {
    let include_paths = invocation.include_machine_paths;
    if !invocation.write_intent {
        return Err(Box::new(commit_gate_error(
            ErrorKind::Denied,
            "write_intent_required",
            include_paths,
        )));
    }
    let expected = invocation.expected_revision.as_deref().ok_or_else(|| {
        Box::new(commit_gate_error(
            ErrorKind::InvalidInput,
            "expected_revision_required",
            include_paths,
        ))
    })?;
    let message = invocation.message.as_deref().ok_or_else(|| {
        Box::new(commit_gate_error(
            ErrorKind::InvalidInput,
            "message_required",
            include_paths,
        ))
    })?;
    if invocation.commit_paths.is_empty() {
        return Err(Box::new(commit_gate_error(
            ErrorKind::InvalidInput,
            "path_required",
            include_paths,
        )));
    }
    if matches!(repo.kind(), BackendKind::Jj) {
        return Err(Box::new(commit_gate_error(
            ErrorKind::Unsupported,
            "jujutsu_atomic_commit_unsupported",
            include_paths,
        )));
    }
    let before_snapshot = repo
        .snapshot()
        .await
        .map_err(|error| Box::new(map_core_error(Operation::Commit, include_paths, error)))?;
    ensure_clear_preflight(&before_snapshot, include_paths)?;
    let before = commit_identity(repo, before_snapshot, include_paths, false).await?;
    if before.revision != expected {
        return Err(Box::new(commit_gate_error(
            ErrorKind::Denied,
            "stale_expected_revision",
            include_paths,
        )));
    }

    let changed_before = repo
        .changed_files_exact()
        .await
        .map_err(|error| Box::new(map_core_error(Operation::Commit, include_paths, error)))?;
    let unrelated_before =
        partition_preflight_changes(&changed_before, &invocation.commit_paths, include_paths)?;

    let included_paths = invocation
        .commit_paths
        .iter()
        .map(|path| MachinePath::from_path(path, include_paths))
        .collect::<Vec<_>>();
    let semantics = commit_semantics(repo.kind());
    let preview = MachineEnvelope::success(
        Operation::Commit.name(),
        CommitData {
            repository: repository_identity(repo, include_paths),
            before: redact_commit_identity(before.clone(), include_paths),
            after: redact_commit_identity(before.clone(), include_paths),
            included_paths: included_paths.clone(),
            unrelated_changes_preserved: true,
            semantics,
        },
    );
    let preview_bytes = serde_json::to_vec_pretty(&preview)
        .expect("commit preview DTO is serializable")
        .len()
        + 1;
    // Commit/revision IDs have backend-fixed widths in normal operation, but
    // Jujutsu's shortest unique change ID can grow. Reserve space so the final
    // success can never be replaced by an output-limit error after mutation.
    if preview_bytes.saturating_add(256) > invocation.max_output_bytes {
        return Err(Box::new(
            AgentError::output_limit(Operation::Commit.name(), invocation.max_output_bytes)
                .include_machine_paths(include_paths),
        ));
    }

    let evidence = repo
        .commit_paths_checked(&invocation.commit_paths, message, expected)
        .await
        .map_err(|error| Box::new(map_core_error(Operation::Commit, include_paths, error)))?;
    if evidence.before_revision != before.revision
        || evidence.before_change_id.as_ref() != before.change_id.as_ref()
        || !same_path_set(&evidence.included_paths, &invocation.commit_paths)
    {
        return Err(Box::new(commit_gate_error(
            ErrorKind::OutcomeUnknown,
            "commit_observed_paths_or_base_mismatch",
            include_paths,
        )));
    }

    let after_snapshot = repo.snapshot().await.map_err(|_| {
        Box::new(commit_gate_error(
            ErrorKind::OutcomeUnknown,
            "commit_postflight_snapshot_failed",
            include_paths,
        ))
    })?;
    if after_snapshot.conflicted || after_snapshot.operation != OperationState::Clear {
        return Err(Box::new(commit_gate_error(
            ErrorKind::OutcomeUnknown,
            "commit_postflight_state_not_clear",
            include_paths,
        )));
    }
    let after = commit_identity(repo, after_snapshot, include_paths, true).await?;
    if after.revision != evidence.after_revision
        || after.change_id.as_ref() != evidence.after_change_id.as_ref()
    {
        return Err(Box::new(commit_gate_error(
            ErrorKind::OutcomeUnknown,
            "commit_postflight_identity_changed",
            include_paths,
        )));
    }
    let changed_after = repo.changed_files_exact().await.map_err(|_| {
        Box::new(commit_gate_error(
            ErrorKind::OutcomeUnknown,
            "commit_postflight_changes_unavailable",
            include_paths,
        ))
    })?;
    if after.revision == before.revision {
        return Err(Box::new(commit_gate_error(
            ErrorKind::OutcomeUnknown,
            "commit_revision_did_not_advance",
            include_paths,
        )));
    }
    if changed_after
        .iter()
        .any(|change| change_intersects_paths(change, &invocation.commit_paths))
    {
        return Err(Box::new(commit_gate_error(
            ErrorKind::OutcomeUnknown,
            "commit_selected_paths_remain_changed",
            include_paths,
        )));
    }
    if !same_change_set(&unrelated_before, &changed_after) {
        return Err(Box::new(commit_gate_error(
            ErrorKind::OutcomeUnknown,
            "commit_unrelated_state_changed",
            include_paths,
        )));
    }

    Ok(CommitData {
        repository: repository_identity(repo, include_paths),
        before: redact_commit_identity(before, include_paths),
        after: redact_commit_identity(after, include_paths),
        included_paths: evidence
            .included_paths
            .iter()
            .map(|path| MachinePath::from_path(path, include_paths))
            .collect(),
        unrelated_changes_preserved: true,
        semantics,
    })
}

fn same_path_set(left: &[PathBuf], right: &[PathBuf]) -> bool {
    left.iter().collect::<BTreeSet<_>>() == right.iter().collect::<BTreeSet<_>>()
}

fn same_change_set(left: &[FileChange], right: &[FileChange]) -> bool {
    left.len() == right.len()
        && left.iter().all(|change| right.contains(change))
        && right.iter().all(|change| left.contains(change))
}

fn commit_semantics(kind: BackendKind) -> CommitSemantics {
    debug_assert!(matches!(kind, BackendKind::Git));
    CommitSemantics {
        selection: "exact-repo-relative-paths",
        backend_selection: "git-atomic-ref-cas",
        refs_advanced: true,
        index_may_change_for_selected_paths: true,
        unrelated_index_preserved: true,
        repository_hooks_executed: false,
        working_copy_content_mutated: false,
        push_performed: false,
        switch_performed: false,
        conflict_repair_performed: false,
    }
}

fn ensure_clear_preflight(snapshot: &RepoSnapshot, include_paths: bool) -> AgentResult<()> {
    if snapshot.conflicted || snapshot.operation != OperationState::Clear {
        return Err(Box::new(commit_gate_error(
            ErrorKind::Denied,
            "repository_state_not_clear",
            include_paths,
        )));
    }
    Ok(())
}

async fn commit_identity<R: ProcessRunner>(
    repo: &Repo<R>,
    snapshot: RepoSnapshot,
    include_paths: bool,
    postflight: bool,
) -> AgentResult<CommitIdentity> {
    let Some(revision) = snapshot.head else {
        return Err(Box::new(commit_gate_error(
            if postflight {
                ErrorKind::OutcomeUnknown
            } else {
                ErrorKind::Denied
            },
            if postflight {
                "commit_postflight_revision_unavailable"
            } else {
                "current_revision_unavailable"
            },
            include_paths,
        )));
    };
    let change_id = if let Some(jj) = repo.jj_at() {
        let change = jj.current_change().await.map_err(|_| {
            Box::new(commit_gate_error(
                if postflight {
                    ErrorKind::OutcomeUnknown
                } else {
                    ErrorKind::Backend
                },
                if postflight {
                    "commit_postflight_change_identity_unavailable"
                } else {
                    "change_identity_unavailable"
                },
                include_paths,
            ))
        })?;
        if !revision.starts_with(&change.commit_id) {
            return Err(Box::new(commit_gate_error(
                if postflight {
                    ErrorKind::OutcomeUnknown
                } else {
                    ErrorKind::Denied
                },
                if postflight {
                    "commit_postflight_identity_inconsistent"
                } else {
                    "repository_identity_changed_during_preflight"
                },
                include_paths,
            )));
        }
        Some(change.change_id)
    } else {
        None
    };
    Ok(CommitIdentity {
        revision,
        change_id,
    })
}

fn partition_preflight_changes(
    changes: &[FileChange],
    selected: &[PathBuf],
    include_paths: bool,
) -> AgentResult<Vec<FileChange>> {
    for change in changes {
        let selects_new = selected.iter().any(|path| path == &change.path);
        let selects_old = change
            .old_path
            .as_ref()
            .is_some_and(|old| selected.iter().any(|path| path == old));
        if change.old_path.is_some() && selects_new != selects_old {
            return Err(Box::new(commit_gate_error(
                ErrorKind::Denied,
                "rename_requires_old_and_new_paths",
                include_paths,
            )));
        }
    }
    if selected.iter().any(|path| {
        !changes.iter().any(|change| {
            &change.path == path || change.old_path.as_ref().is_some_and(|old| old == path)
        })
    }) {
        return Err(Box::new(commit_gate_error(
            ErrorKind::Denied,
            "selected_path_not_changed",
            include_paths,
        )));
    }
    Ok(changes
        .iter()
        .filter(|change| !change_intersects_paths(change, selected))
        .cloned()
        .collect())
}

fn change_intersects_paths(change: &FileChange, selected: &[PathBuf]) -> bool {
    selected
        .iter()
        .any(|path| path == &change.path || change.old_path.as_ref().is_some_and(|old| path == old))
}

fn commit_gate_error(kind: ErrorKind, code: &'static str, include_paths: bool) -> AgentError {
    AgentError::new(Operation::Commit.name(), kind, code, false)
        .include_machine_paths(include_paths)
}

fn redact_commit_identity(mut identity: CommitIdentity, include_paths: bool) -> CommitIdentity {
    identity.revision = redact_metadata(&identity.revision, include_paths);
    identity.change_id = identity
        .change_id
        .map(|value| redact_metadata(&value, include_paths));
    identity
}

fn open_repo(
    path: &Path,
    operation: Operation,
    include_paths: bool,
    policy: &ExecutionPolicy,
) -> AgentResult<Repo<JobRunner>> {
    Repo::discover_with(
        path,
        || {
            let git = Git::new()
                .default_timeout(policy.deadline)
                .default_cancel_on(policy.cancellation.clone())
                .default_output_budget(policy.content_budget);
            if matches!(operation, Operation::Commit | Operation::Publish) {
                git.harden()
            } else {
                git
            }
        },
        || {
            Jj::new()
                .default_timeout(policy.deadline)
                .default_cancel_on(policy.cancellation.clone())
                .default_output_budget(policy.content_budget)
        },
    )
    .map_err(|error| Box::new(map_core_error(operation, include_paths, error)))
}

async fn inspect_repo<R: ProcessRunner>(
    repo: &Repo<R>,
    remotes: Vec<vcs_core::Remote>,
    forge: Option<&dyn ForgeApi>,
    forge_remote: Option<String>,
    include_paths: bool,
) -> AgentResult<InspectData> {
    let snapshot = repo
        .snapshot()
        .await
        .map_err(|error| Box::new(map_core_error(Operation::Inspect, include_paths, error)))?;
    let change_id = if let Some(jj) = repo.jj_at() {
        Some(
            jj.current_change()
                .await
                .map_err(|error| {
                    Box::new(
                        AgentError::from_processkit(
                            Operation::Inspect.name(),
                            FailureDomain::Backend,
                            &error,
                        )
                        .include_machine_paths(include_paths),
                    )
                })?
                .change_id,
        )
    } else {
        None
    };

    let remotes: Vec<RemoteData> = remotes
        .into_iter()
        .map(|remote| RemoteData {
            name: redact_metadata(&remote.name, include_paths),
            url: redact_metadata(&remote.url, include_paths),
        })
        .collect();
    let forge = inspect_forge(forge, forge_remote.as_deref(), include_paths).await?;

    Ok(InspectData {
        repository: repository_identity(repo, include_paths),
        working_copy: working_copy(repo.kind(), snapshot, change_id, include_paths),
        remotes,
        forge,
        capabilities: repository_capabilities(),
        read_semantics: read_semantics(repo.kind()),
    })
}

async fn changes_repo<R: ProcessRunner>(
    repo: &Repo<R>,
    mode: ChangesMode,
    content_max_bytes: usize,
    include_paths: bool,
) -> AgentResult<ChangesData> {
    let files = repo
        .changed_files()
        .await
        .map_err(|error| Box::new(map_core_error(Operation::Changes, include_paths, error)))?;
    let stat = repo
        .diff_stat()
        .await
        .map_err(|error| Box::new(map_core_error(Operation::Changes, include_paths, error)))?;
    let diff =
        if mode == ChangesMode::Full {
            Some(repo.diff().await.map_err(|error| {
                Box::new(map_core_error(Operation::Changes, include_paths, error))
            })?)
        } else {
            None
        };
    let counts = count_changes(&files, stat);
    let files = files
        .into_iter()
        .map(|change| map_changed_path(change, include_paths))
        .collect();
    let diff = diff.map(|files| {
        files
            .into_iter()
            .map(|file| map_file_diff(file, include_paths))
            .collect()
    });

    Ok(ChangesData {
        repository: repository_identity(repo, include_paths),
        mode: match mode {
            ChangesMode::Summary => "summary",
            ChangesMode::Full => "full",
        },
        content_max_bytes,
        counts,
        files,
        diff,
        read_semantics: read_semantics(repo.kind()),
    })
}

fn build_forge(
    remotes: &[vcs_core::Remote],
    cwd: &Path,
    policy: &ExecutionPolicy,
) -> Option<Box<dyn ForgeApi>> {
    let kind = preferred_forge_remote(remotes)?.1;
    Some(build_forge_kind(kind, cwd, policy))
}

fn build_forge_kind(kind: ForgeKind, cwd: &Path, policy: &ExecutionPolicy) -> Box<dyn ForgeApi> {
    let timeout = policy.deadline;
    let token = &policy.cancellation;
    let budget = policy.content_budget;
    match kind {
        ForgeKind::GitHub => Box::new(Forge::from_github(
            cwd,
            GitHub::new()
                .default_timeout(timeout)
                .default_cancel_on(token.clone())
                .default_output_budget(budget),
        )),
        ForgeKind::GitLab => Box::new(Forge::from_gitlab(
            cwd,
            GitLab::new()
                .default_timeout(timeout)
                .default_cancel_on(token.clone())
                .default_output_budget(budget),
        )),
        ForgeKind::Gitea => Box::new(Forge::from_gitea(
            cwd,
            Gitea::new()
                .default_timeout(timeout)
                .default_cancel_on(token.clone())
                .default_output_budget(budget),
        )),
        _ => Box::new(Forge::<JobRunner>::from_unknown(cwd)),
    }
}

fn preferred_forge_remote(remotes: &[vcs_core::Remote]) -> Option<(&vcs_core::Remote, ForgeKind)> {
    remotes
        .iter()
        .map(|remote| {
            (
                remote,
                ForgeKind::from_remote_url(&remote.url).unwrap_or(ForgeKind::Unknown),
            )
        })
        .min_by_key(|(remote, kind)| (matches!(kind, ForgeKind::Unknown), remote.name != "origin"))
}

async fn inspect_forge(
    forge: Option<&dyn ForgeApi>,
    remote: Option<&str>,
    include_paths: bool,
) -> AgentResult<ForgeData> {
    let Some(forge) = forge else {
        return Ok(ForgeData {
            detection: "absent",
            kind: None,
            remote: remote.map(str::to_owned),
            capabilities: Fact::not_applicable(),
            auth: Fact::not_applicable(),
        });
    };
    let capabilities = match forge.capabilities().await {
        Ok(value) => Fact::known(map_forge_capabilities(value, include_paths)),
        Err(error) => {
            if let Some(error) = forge_lifecycle_error(&error, include_paths) {
                return Err(Box::new(error));
            }
            Fact::unavailable(classify_forge_error(&error))
        }
    };
    let auth = match forge.auth_info().await {
        Ok(value) => Fact::known(map_forge_auth(value, include_paths)),
        Err(error) => {
            if let Some(error) = forge_lifecycle_error(&error, include_paths) {
                return Err(Box::new(error));
            }
            Fact::unavailable(classify_forge_error(&error))
        }
    };
    Ok(ForgeData {
        detection: "detected",
        kind: Some(forge_kind(forge.kind())),
        remote: remote.map(str::to_owned),
        capabilities,
        auth,
    })
}

fn forge_lifecycle_error(error: &vcs_forge::Error, include_paths: bool) -> Option<AgentError> {
    let vcs_forge::Error::Forge(process) = error else {
        return None;
    };
    (process.is_timeout()
        || process.is_cancelled()
        || process.is_permission_denied()
        || matches!(
            process.reason(),
            processkit::ErrorReason::OutputTooLarge { .. }
        ))
    .then(|| {
        AgentError::from_processkit(Operation::Inspect.name(), FailureDomain::Forge, process)
            .include_machine_paths(include_paths)
    })
}

fn map_forge_capabilities(value: ForgeCapabilities, include_paths: bool) -> ForgeCapabilitiesData {
    ForgeCapabilitiesData {
        cli_version: value
            .version
            .map(|version| redact_metadata(&version.to_string(), include_paths)),
        cli_supported: value.supported,
        authenticated: value.authed,
        pr_create: value.pr_create,
        pr_comment: value.pr_comment,
        pr_edit: value.pr_edit,
        pr_labels: value.pr_labels,
        pr_checks: value.pr_checks,
        pr_merge: value.pr_merge,
        pr_approve: value.pr_approve,
        pr_request_changes: value.pr_request_changes,
        issue_create: value.issue_create,
        issue_close: value.issue_close,
        issue_reopen: value.issue_reopen,
        issue_comment: value.issue_comment,
        issue_labels: value.issue_labels,
        release_create: value.release_create,
        release_delete: value.release_delete,
    }
}

fn map_forge_auth(value: ForgeAuth, include_paths: bool) -> ForgeAuthData {
    ForgeAuthData {
        authenticated: value.authed,
        active_account: value
            .active_account
            .map(|account| redact_metadata(&account, include_paths)),
        accounts: value
            .accounts
            .into_iter()
            .map(|account| ForgeAccountData {
                host: redact_metadata(&account.host, include_paths),
                login: redact_metadata(&account.login, include_paths),
                active: account.active,
            })
            .collect(),
        repository_visible: value.repo_visible,
    }
}

fn classify_forge_error(error: &vcs_forge::Error) -> &'static str {
    if error.is_not_found() {
        "forge_cli_not_found"
    } else if error.is_unsupported() {
        "unsupported_capability"
    } else if error.is_unauthorized() {
        "authentication_required"
    } else if error.is_version_gated() {
        "forge_cli_too_old"
    } else if error.is_invalid_input() {
        "invalid_forge_context"
    } else {
        match error {
            vcs_forge::Error::Forge(process) if process.is_timeout() => "timeout",
            vcs_forge::Error::Forge(process) if process.is_cancelled() => "cancelled",
            _ => "forge_probe_failed",
        }
    }
}

fn repository_identity(repo: &Repo<impl ProcessRunner>, include_paths: bool) -> RepositoryIdentity {
    RepositoryIdentity {
        backend: backend_kind(repo.kind()),
        root: MachinePath::from_path(repo.root(), include_paths),
        cwd: MachinePath::from_path(repo.cwd(), include_paths),
    }
}

fn working_copy(
    kind: BackendKind,
    snapshot: RepoSnapshot,
    change_id: Option<String>,
    include_paths: bool,
) -> WorkingCopy {
    WorkingCopy {
        branch_kind: if matches!(kind, BackendKind::Git) {
            "branch"
        } else {
            "bookmark"
        },
        branch: snapshot
            .branch
            .map(|branch| redact_metadata(&branch, include_paths)),
        revision: snapshot
            .head
            .map(|revision| redact_metadata(&revision, include_paths)),
        change_id: change_id.map(|change| redact_metadata(&change, include_paths)),
        dirty: snapshot.dirty,
        tracked_changes: snapshot.tracked_changes,
        untracked: snapshot.untracked,
        total_changes: snapshot.change_count,
        conflicted: snapshot.conflicted,
        conflict_count: snapshot.conflict_count,
        operation: operation_state(snapshot.operation),
        upstream: snapshot.tracking.map(|tracking| UpstreamData {
            branch: redact_metadata(&tracking.branch, include_paths),
            ahead: tracking.ahead,
            behind: tracking.behind,
        }),
    }
}

fn redact_metadata(value: &str, include_paths: bool) -> String {
    redact_text(
        value,
        RedactionPolicy {
            include_machine_paths: include_paths,
        },
    )
}

fn count_changes(files: &[FileChange], stat: vcs_core::DiffStat) -> ChangeCounts {
    let mut counts = ChangeCounts {
        paths: files.len(),
        added: 0,
        modified: 0,
        deleted: 0,
        renamed: 0,
        files_with_line_diff: stat.files_changed,
        insertions: stat.insertions,
        deletions: stat.deletions,
    };
    for file in files {
        match file.kind {
            ChangeKind::Added => counts.added += 1,
            ChangeKind::Modified => counts.modified += 1,
            ChangeKind::Deleted => counts.deleted += 1,
            ChangeKind::Renamed => counts.renamed += 1,
            _ => {}
        }
    }
    counts
}

fn map_changed_path(change: FileChange, include_paths: bool) -> ChangedPath {
    ChangedPath {
        path: MachinePath::from_path(&change.path, include_paths),
        old_path: change
            .old_path
            .as_deref()
            .map(|path| MachinePath::from_path(path, include_paths)),
        kind: change_kind(change.kind),
    }
}

fn map_file_diff(file: FileDiff, include_paths: bool) -> StructuredFileDiff {
    let redact = |text: String| redact_metadata(&text, include_paths);
    StructuredFileDiff {
        path: MachinePath::from_path(&file.path, include_paths),
        old_path: file
            .old_path
            .as_deref()
            .map(|path| MachinePath::from_path(path, include_paths)),
        kind: change_kind(file.change),
        hunks: file
            .hunks
            .into_iter()
            .map(|hunk| StructuredHunk {
                old_start: hunk.old_start,
                old_lines: hunk.old_lines,
                new_start: hunk.new_start,
                new_lines: hunk.new_lines,
                section: redact(hunk.section),
                lines: hunk
                    .lines
                    .into_iter()
                    .map(|line| match line {
                        DiffLine::Context(text) => StructuredLine {
                            kind: "context",
                            text: redact(text),
                        },
                        DiffLine::Added(text) => StructuredLine {
                            kind: "added",
                            text: redact(text),
                        },
                        DiffLine::Removed(text) => StructuredLine {
                            kind: "removed",
                            text: redact(text),
                        },
                        _ => StructuredLine {
                            kind: "unknown",
                            text: String::new(),
                        },
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn map_core_error(operation: Operation, include_paths: bool, error: vcs_core::Error) -> AgentError {
    let mapped = match error {
        vcs_core::Error::Vcs(process) => {
            AgentError::from_processkit(operation.name(), FailureDomain::Backend, &process)
        }
        vcs_core::Error::NotARepository(path) => AgentError::new(
            operation.name(),
            ErrorKind::InvalidInput,
            "repository_not_found",
            false,
        )
        .with_detail(DetailKey::RepositoryPath, path.to_string_lossy()),
        vcs_core::Error::BareRepository(path) => AgentError::new(
            operation.name(),
            ErrorKind::Unsupported,
            "bare_repository_unsupported",
            false,
        )
        .with_detail(DetailKey::RepositoryPath, path.to_string_lossy()),
        vcs_core::Error::Unsupported(_) => AgentError::new(
            operation.name(),
            ErrorKind::Unsupported,
            "backend_capability_unsupported",
            false,
        ),
        vcs_core::Error::StaleRevision { .. } => AgentError::new(
            operation.name(),
            ErrorKind::Denied,
            "stale_expected_revision",
            false,
        ),
        vcs_core::Error::OutcomeUnknown(_) => AgentError::new(
            operation.name(),
            ErrorKind::OutcomeUnknown,
            "commit_backend_evidence_unverified",
            false,
        ),
        vcs_core::Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
            AgentError::new(
                operation.name(),
                ErrorKind::InvalidInput,
                "repository_input_invalid",
                false,
            )
        }
        _ => AgentError::new(
            operation.name(),
            ErrorKind::Backend,
            "repository_query_failed",
            false,
        ),
    };
    mapped.include_machine_paths(include_paths)
}

fn repository_capabilities() -> RepositoryCapabilities {
    RepositoryCapabilities {
        inspect: true,
        changes_summary: true,
        changes_full: true,
        lossless_status_paths: true,
        full_diff_non_utf8_paths: "git-lossless-jj-text-limited",
        raw_cli_fallback: false,
    }
}

fn read_semantics(kind: BackendKind) -> ReadSemantics {
    let jj = matches!(kind, BackendKind::Jj);
    ReadSemantics {
        refs_mutated: false,
        index_mutated: false,
        working_copy_content_mutated: false,
        working_copy_snapshot: if jj { "live-jj-snapshot" } else { "none" },
        operation_log_may_advance: jj,
    }
}

fn backend_kind(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::Git => "git",
        BackendKind::Jj => "jujutsu",
        _ => "unknown",
    }
}

fn forge_kind(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::GitHub => "github",
        ForgeKind::GitLab => "gitlab",
        ForgeKind::Gitea => "gitea",
        ForgeKind::Unknown => "unknown",
        _ => "unknown",
    }
}

fn change_kind(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
        _ => "unknown",
    }
}

fn operation_state(state: OperationState) -> &'static str {
    match state {
        OperationState::Clear => "clear",
        OperationState::Merge => "merge",
        OperationState::Rebase => "rebase",
        OperationState::ApplyMailbox => "apply_mailbox",
        OperationState::CherryPick => "cherry_pick",
        OperationState::Revert => "revert",
        OperationState::Bisect => "bisect",
        OperationState::Conflict => "conflict",
        _ => "unknown",
    }
}

#[cfg(unix)]
fn machine_path_non_utf8(path: &Path) -> MachinePath {
    use std::os::unix::ffi::OsStrExt;
    MachinePath {
        display: path.to_string_lossy().into_owned(),
        encoding: "os-bytes-hex",
        value: Some(hex(path.as_os_str().as_bytes().iter().copied())),
    }
}

#[cfg(windows)]
fn machine_path_non_utf8(path: &Path) -> MachinePath {
    use std::os::windows::ffi::OsStrExt;
    let units = path.as_os_str().encode_wide();
    let mut value = String::new();
    for unit in units {
        use std::fmt::Write;
        write!(&mut value, "{unit:04x}").expect("writing to String cannot fail");
    }
    MachinePath {
        display: path.to_string_lossy().into_owned(),
        encoding: "windows-utf16-hex",
        value: Some(value),
    }
}

#[cfg(not(any(unix, windows)))]
fn machine_path_non_utf8(path: &Path) -> MachinePath {
    MachinePath {
        display: path.to_string_lossy().into_owned(),
        encoding: "platform-native-lossy",
        value: None,
    }
}

#[cfg(unix)]
fn hex(bytes: impl Iterator<Item = u8>) -> String {
    let mut value = String::new();
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};
    use processkit::{Command, ProcessResult};
    use serde_json::json;

    struct DelayedRunner {
        inner: ScriptedRunner,
        delay: Duration,
    }

    impl ProcessRunner for DelayedRunner {
        fn output_string<'life0, 'life1, 'async_trait>(
            &'life0 self,
            command: &'life1 Command,
        ) -> std::pin::Pin<
            Box<
                dyn Future<Output = processkit::Result<ProcessResult<String>>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                tokio::time::sleep(self.delay).await;
                self.inner.output_string(command).await
            })
        }
    }

    fn inspect_invocation(include_machine_paths: bool) -> Invocation {
        Invocation {
            operation: Operation::Inspect,
            repository: Some(Path::new("/repo/private").to_path_buf()),
            changes_mode: ChangesMode::Summary,
            content_max_bytes: 8192,
            max_output_bytes: crate::cli::DEFAULT_MAX_OUTPUT_BYTES,
            include_machine_paths,
            write_intent: false,
            expected_revision: None,
            message: None,
            commit_paths: Vec::new(),
            remote: None,
            source: None,
            target: None,
            expected_remote_revision: None,
            forge: None,
            expected_account: None,
            title: None,
            body: None,
            wait_seconds: crate::cli::DEFAULT_WAIT_SECONDS,
            poll_seconds: crate::cli::DEFAULT_POLL_SECONDS,
        }
    }

    fn scripted_git_inspect(remote_output: &str, status_output: &str) -> Repo<ScriptedRunner> {
        let runner = ScriptedRunner::new()
            .on(["git", "remote", "-v"], Reply::ok(remote_output))
            .on(
                ["git", "status", "--porcelain=v2"],
                Reply::ok(status_output),
            )
            .on(
                ["git", "rev-parse", "--git-dir"],
                Reply::ok("/repo/private/.git"),
            );
        Repo::from_git("/repo/private", "/repo/private", Git::with_runner(runner))
    }

    #[tokio::test]
    async fn scripted_git_inspect_uses_typed_facade_and_redacts_remote_credentials() {
        let status = concat!(
            "# branch.oid abc123\0",
            "# branch.head main\0",
            "1 .M N... 100644 100644 100644 1 2 src/lib.rs\0",
        );
        let runner = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["git", "status", "--porcelain=v2"], Reply::ok(status))
                .on(["git", "rev-parse", "--git-dir"], Reply::ok("/repo/.git")),
        );
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&runner));
        let data = inspect_repo(
            &repo,
            vec![vcs_core::Remote::new(
                "origin",
                "https://alice:secret@example.invalid/owner/repo",
            )],
            None,
            None,
            true,
        )
        .await
        .expect("inspect");
        assert_eq!(data.repository.backend, "git");
        assert_eq!(data.working_copy.branch.as_deref(), Some("main"));
        assert_eq!(data.working_copy.total_changes, 1);
        assert!(!data.remotes[0].url.contains("secret"));
        assert!(
            runner
                .calls()
                .iter()
                .all(|call| call.program.to_string_lossy() == "git")
        );
    }

    #[tokio::test]
    async fn scripted_changes_has_distinct_summary_and_full_shapes() {
        let runner = ScriptedRunner::new()
            .on(["git", "status"], Reply::ok(" M src/lib.rs\0"))
            .on(["git", "rev-parse", "--verify"], Reply::ok("abc\n"))
            .on(["git", "diff", "--shortstat"], Reply::ok(" 1 file changed, 1 insertion(+), 1 deletion(-)"))
            .on(
                ["git", "diff"],
                Reply::ok("diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n"),
            );
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(runner));
        let full = changes_repo(&repo, ChangesMode::Full, 8192, true)
            .await
            .expect("changes");
        assert_eq!(full.mode, "full");
        assert_eq!(full.counts.paths, 1);
        assert_eq!(full.diff.as_ref().map(Vec::len), Some(1));
        assert_eq!(full.diff.unwrap()[0].hunks.len(), 1);
    }

    #[tokio::test]
    async fn scripted_full_changes_redacts_hunk_content_under_both_path_policies() {
        const DIFF: &str = concat!(
            "diff --git a/src/lib.rs b/src/lib.rs\n",
            "--- a/src/lib.rs\n",
            "+++ b/src/lib.rs\n",
            "@@ -1,3 +1,4 @@ API_TOKEN=section-secret path=C:\\Users\\section-owner\\repo\n",
            "-password=removed-secret\n",
            " remote=https://alice:uri-secret@example.invalid/repo\n",
            "+Authorization: Bearer bearer-secret\n",
            "+workspace=/home/line-owner/repo\n",
        );

        for include_paths in [false, true] {
            let runner = ScriptedRunner::new()
                .on(["git", "status"], Reply::ok(" M src/lib.rs\0"))
                .on(["git", "rev-parse", "--verify"], Reply::ok("abc\n"))
                .on(
                    ["git", "diff", "--shortstat"],
                    Reply::ok(" 1 file changed, 2 insertions(+), 1 deletion(-)"),
                )
                .on(["git", "diff"], Reply::ok(DIFF));
            let repo = Repo::from_git("/repo", "/repo", Git::with_runner(runner));
            let full = changes_repo(&repo, ChangesMode::Full, 8192, include_paths)
                .await
                .expect("changes");
            let encoded = serde_json::to_string(&full).expect("serialize full changes");

            for leaked in [
                "section-secret",
                "removed-secret",
                "alice:uri-secret",
                "bearer-secret",
            ] {
                assert!(!encoded.contains(leaked), "full diff leaked {leaked}");
            }

            let hunk = &full.diff.as_ref().expect("full diff")[0].hunks[0];
            let content = std::iter::once(hunk.section.as_str())
                .chain(hunk.lines.iter().map(|line| line.text.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            if include_paths {
                assert!(content.contains(r"C:\Users\section-owner\repo"));
                assert!(content.contains("/home/line-owner/repo"));
            } else {
                assert!(!content.contains("section-owner"));
                assert!(!content.contains("line-owner"));
                assert_eq!(content.matches("[REDACTED_PATH]").count(), 2);
            }
        }
    }

    #[tokio::test]
    async fn fired_policy_token_maps_to_structured_cancellation() {
        let token = CancellationToken::new();
        token.cancel();
        let repo = Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()).default_cancel_on(token),
        );
        let error = match changes_repo(&repo, ChangesMode::Summary, 8192, false).await {
            Err(error) => error,
            Ok(_) => panic!("fired token must cancel the first typed command"),
        };
        assert_eq!(error.kind(), ErrorKind::Cancelled);
    }

    #[tokio::test]
    async fn checked_commit_cancellation_fails_before_the_mutation() {
        let token = CancellationToken::new();
        token.cancel();
        let repo = Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()).default_cancel_on(token),
        );
        let invocation = Invocation {
            operation: Operation::Commit,
            repository: Some(PathBuf::from("/repo")),
            changes_mode: ChangesMode::Summary,
            content_max_bytes: 8192,
            max_output_bytes: crate::cli::DEFAULT_MAX_OUTPUT_BYTES,
            include_machine_paths: false,
            write_intent: true,
            expected_revision: Some("abc".to_owned()),
            message: Some("message".to_owned()),
            commit_paths: vec![PathBuf::from("selected.txt")],
            remote: None,
            source: None,
            target: None,
            expected_remote_revision: None,
            forge: None,
            expected_account: None,
            title: None,
            body: None,
            wait_seconds: crate::cli::DEFAULT_WAIT_SECONDS,
            poll_seconds: crate::cli::DEFAULT_POLL_SECONDS,
        };
        let error = match commit_repo(&repo, &invocation).await {
            Err(error) => error,
            Ok(_) => panic!("cancelled preflight cannot reach commit_paths"),
        };
        assert_eq!(error.kind(), ErrorKind::Cancelled);
    }

    #[tokio::test]
    async fn detected_but_unclassified_forge_is_structured_without_spawning() {
        let forge = Forge::<JobRunner>::from_unknown("/repo");
        let detected = inspect_forge(Some(&forge), Some("https://forge.example/repo"), false)
            .await
            .expect("unknown forge inspection");
        assert_eq!(detected.detection, "detected");
        assert_eq!(detected.kind, Some("unknown"));
        assert_eq!(detected.capabilities.status, "known");
        assert_eq!(detected.auth.status, "known");

        let absent = inspect_forge(None, None, false)
            .await
            .expect("absent forge inspection");
        assert_eq!(absent.detection, "absent");
        assert_eq!(absent.capabilities.status, "not_applicable");
    }

    #[tokio::test]
    async fn inspect_composition_distinguishes_unknown_remote_from_no_remote() {
        let status = "# branch.oid abc123\0# branch.head main\0";
        let policy = ExecutionPolicy::new(8192);
        let unknown = execute_repository(
            &inspect_invocation(false),
            &policy,
            &scripted_git_inspect(
                "origin https://forge.example.invalid/owner/repo (fetch)\n",
                status,
            ),
        )
        .await
        .expect("unknown remote remains a detected forge");
        let unknown = serde_json::to_value(unknown).expect("serialize inspect envelope");
        assert_eq!(unknown["data"]["forge"]["detection"], "detected");
        assert_eq!(unknown["data"]["forge"]["kind"], "unknown");
        assert_eq!(unknown["data"]["forge"]["capabilities"]["status"], "known");

        let absent = execute_repository(
            &inspect_invocation(false),
            &ExecutionPolicy::new(8192),
            &scripted_git_inspect("", status),
        )
        .await
        .expect("repository without remotes is inspectable");
        let absent = serde_json::to_value(absent).expect("serialize inspect envelope");
        assert_eq!(absent["data"]["forge"]["detection"], "absent");
        assert!(absent["data"]["forge"]["kind"].is_null());
        assert_eq!(
            absent["data"]["forge"]["capabilities"]["status"],
            "not_applicable"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn one_outcome_deadline_bounds_multiple_sequential_typed_calls() {
        let runner = RecordingRunner::new(DelayedRunner {
            inner: ScriptedRunner::new()
                .on(
                    ["git", "remote", "-v"],
                    Reply::ok("origin https://forge.example.invalid/repo (fetch)\n"),
                )
                .on(
                    ["git", "status", "--porcelain=v2"],
                    Reply::ok("# branch.oid abc123\0# branch.head main\0"),
                )
                .on(["git", "rev-parse", "--git-dir"], Reply::ok("/repo/.git")),
            delay: Duration::from_secs(60),
        });
        let mut policy = ExecutionPolicy::new(8192);
        policy.deadline = Duration::from_secs(100);
        let repo = Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(&runner)
                .default_timeout(Duration::from_secs(120))
                .default_cancel_on(policy.cancellation.clone()),
        );

        let error = match with_outcome_deadline(
            Operation::Inspect,
            false,
            &policy,
            execute_repository(&inspect_invocation(false), &policy, &repo),
        )
        .await
        {
            Err(error) => error,
            Ok(_) => panic!("two 60-second calls must not receive independent 100-second budgets"),
        };

        assert_eq!(error.kind(), ErrorKind::Timeout);
        assert_eq!(
            runner.calls().len(),
            2,
            "the third typed call must not start"
        );
    }

    #[tokio::test]
    async fn inspect_redacts_secret_shaped_metadata_with_or_without_machine_paths() {
        let status = concat!(
            "# branch.oid token=revision-secret\0",
            "# branch.head token=branch-secret\0",
            "# branch.upstream token=upstream-secret\0",
            "# branch.ab +0 -0\0",
        );
        let remotes =
            "token=remote-secret https://example.invalid/repo?token=remote-url-secret (fetch)\n";

        for include_paths in [false, true] {
            let envelope = execute_repository(
                &inspect_invocation(include_paths),
                &ExecutionPolicy::new(8192),
                &scripted_git_inspect(remotes, status),
            )
            .await
            .expect("secret-shaped metadata is redacted, not rejected");
            let value = serde_json::to_value(&envelope).expect("serialize inspect envelope");
            assert_eq!(
                value["data"]["repository"]["root"]["encoding"],
                if include_paths { "utf-8" } else { "redacted" }
            );
            let encoded = serde_json::to_string(&value).expect("encode inspect envelope");
            for leaked in [
                "revision-secret",
                "branch-secret",
                "upstream-secret",
                "remote-secret",
                "remote-url-secret",
            ] {
                assert!(!encoded.contains(leaked), "machine output leaked {leaked}");
            }
        }

        for include_paths in [false, true] {
            let auth = ForgeAuth::unknown()
                .active_account("token=active-account-secret")
                .accounts(vec![vcs_forge::ForgeAccount::new(
                    "token=host-secret",
                    "token=login-secret",
                )]);
            let encoded = serde_json::to_string(&map_forge_auth(auth, include_paths))
                .expect("serialize forge auth");
            for leaked in ["active-account-secret", "host-secret", "login-secret"] {
                assert!(!encoded.contains(leaked), "forge metadata leaked {leaked}");
            }
        }
    }

    #[test]
    fn known_remote_selects_the_typed_forge_without_probing_it() {
        let policy = ExecutionPolicy::new(8192);
        let forge = build_forge(
            &[vcs_core::Remote::new(
                "origin",
                "https://github.com/owner/repo.git",
            )],
            Path::new("/repo"),
            &policy,
        )
        .expect("known GitHub remote");
        assert_eq!(forge.kind(), ForgeKind::GitHub);
    }

    #[test]
    fn machine_paths_are_redacted_by_default() {
        let value = MachinePath::from_path(Path::new("/repo/private"), false);
        assert_eq!(value.encoding, "redacted");
        assert!(value.value.is_none());
    }

    #[test]
    fn checked_commit_preflight_requires_each_exact_changed_path_and_preserves_unrelated_set() {
        let changes = vec![
            FileChange::new("selected.txt", ChangeKind::Modified),
            FileChange::new("unrelated.txt", ChangeKind::Added),
        ];
        let unrelated =
            partition_preflight_changes(&changes, &[PathBuf::from("selected.txt")], false)
                .expect("selected changed path passes preflight");
        assert_eq!(unrelated, vec![changes[1].clone()]);

        let error = partition_preflight_changes(&changes, &[PathBuf::from("missing.txt")], false)
            .expect_err("an unchanged path cannot be reported as included");
        assert_eq!(error.kind(), ErrorKind::Denied);
    }

    #[test]
    fn checked_commit_preflight_refuses_half_of_a_rename() {
        let changes = vec![FileChange::new("new.txt", ChangeKind::Renamed).old_path("old.txt")];
        let error = partition_preflight_changes(&changes, &[PathBuf::from("new.txt")], false)
            .expect_err("a one-sided rename path is ambiguous");
        assert_eq!(error.kind(), ErrorKind::Denied);

        assert!(
            partition_preflight_changes(
                &changes,
                &[PathBuf::from("old.txt"), PathBuf::from("new.txt")],
                false,
            )
            .expect("both rename endpoints are exact")
            .is_empty()
        );
    }

    #[test]
    fn checked_commit_observed_path_proof_rejects_extra_or_missing_paths() {
        let selected = vec![PathBuf::from("old.txt"), PathBuf::from("new.txt")];
        assert!(same_path_set(
            &[PathBuf::from("new.txt"), PathBuf::from("old.txt")],
            &selected,
        ));
        assert!(!same_path_set(&[PathBuf::from("new.txt")], &selected));
        assert!(!same_path_set(
            &[
                PathBuf::from("old.txt"),
                PathBuf::from("new.txt"),
                PathBuf::from("extra.txt"),
            ],
            &selected,
        ));
    }

    #[test]
    fn checked_commit_postflight_requires_exact_unrelated_change_set() {
        let before = vec![FileChange::new("unrelated.txt", ChangeKind::Modified)];
        assert!(same_change_set(&before, &before));
        assert!(!same_change_set(
            &before,
            &[
                before[0].clone(),
                FileChange::new("new-unrelated.txt", ChangeKind::Added),
            ],
        ));
        assert!(!same_change_set(&before, &[]));
        assert!(!same_change_set(
            &before,
            &[FileChange::new("unrelated.txt", ChangeKind::Deleted)],
        ));
    }

    #[test]
    fn checked_commit_backend_unknown_maps_to_exit_43() {
        let error = map_core_error(
            Operation::Commit,
            false,
            vcs_core::Error::OutcomeUnknown("unobservable atomic ref update".into()),
        );
        assert_eq!(error.kind(), ErrorKind::OutcomeUnknown);
        assert_eq!(ErrorKind::OutcomeUnknown.exit_code(), 43);
    }

    #[test]
    fn checked_commit_preflight_refuses_conflict_and_in_progress_state() {
        for snapshot in [
            RepoSnapshot::new().conflicted(),
            RepoSnapshot::new().operation(OperationState::Rebase),
        ] {
            let error = ensure_clear_preflight(&snapshot, false)
                .expect_err("a checked mutation requires a clear repository");
            assert_eq!(error.kind(), ErrorKind::Denied);
        }
    }

    fn workflow_run(json: &str) -> WorkflowRun {
        serde_json::from_str(json).expect("valid workflow-run fixture")
    }

    #[test]
    fn exact_ci_selection_rejects_recent_mismatch_and_duplicate_workflow() {
        let expected = "0123456789abcdef0123456789abcdef01234567";
        let recent_other = workflow_run(
            r#"{"databaseId":1,"workflowName":"CI","headSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"completed","conclusion":"success"}"#,
        );
        assert!(
            select_exact_ci_runs(vec![recent_other], expected, Operation::CiStatus, false)
                .expect("mismatched runs are ignored")
                .is_empty(),
            "a recent branch run for another SHA must not satisfy exact CI"
        );

        let first = workflow_run(&format!(
            r#"{{"databaseId":2,"workflowName":"CI","headSha":"{expected}","status":"completed","conclusion":"success"}}"#
        ));
        let rerun = workflow_run(&format!(
            r#"{{"databaseId":3,"workflowName":"CI","headSha":"{expected}","status":"completed","conclusion":"success"}}"#
        ));
        let error = select_exact_ci_runs(vec![first, rerun], expected, Operation::CiStatus, false)
            .expect_err("two runs for one workflow/SHA are ambiguous");
        assert_eq!(error.kind(), ErrorKind::Denied);
    }

    #[test]
    fn exact_ci_requires_completed_success_not_pending_or_recent_success() {
        let expected = "0123456789abcdef0123456789abcdef01234567";
        let pending = workflow_run(&format!(
            r#"{{"databaseId":4,"workflowName":"CI","headSha":"{expected}","status":"in_progress","conclusion":""}}"#
        ));
        let exact = select_exact_ci_runs(vec![pending], expected, Operation::CiStatus, false)
            .expect("one exact pending run is unambiguous");
        assert!(!exact.iter().all(ci_run_terminal));
        assert!(!exact.iter().all(|run| run.conclusion == "success"));
    }

    #[test]
    fn ci_wait_post_watch_revision_drift_serializes_as_ci_wait_unknown() {
        let expected = "0123456789abcdef0123456789abcdef01234567";
        let completed = workflow_run(
            r#"{"databaseId":9,"workflowName":"CI","headSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"completed","conclusion":"success"}"#,
        );
        let error = ensure_ci_run_revision_unchanged(&completed, expected, false)
            .expect_err("post-watch revision drift must fail closed");
        let output = crate::contract::render(*error, crate::cli::DEFAULT_MAX_OUTPUT_BYTES);
        let text = String::from_utf8(output.stdout).expect("machine envelope is UTF-8");
        let value: serde_json::Value =
            serde_json::from_str(&text).expect("machine envelope is JSON");

        assert_eq!(value["operation"], "ci_wait");
        assert_eq!(value["error"]["kind"], "outcome_unknown");
        assert_eq!(value["error"]["code"], "ci_run_revision_changed");
        assert_eq!(value["error"]["details"]["checkpoint"], "ci_postflight");
        assert_eq!(value["error"]["details"]["revision"], expected);
        assert!(
            !text.contains("\"publish\""),
            "CI drift envelope must not carry a publish discriminator: {text}"
        );
    }

    fn pull_request(state: &str, revision: &str, cross_repository: Option<bool>) -> PullRequest {
        serde_json::from_value(json!({
            "number": 42,
            "title": "checked publish",
            "state": state,
            "headRefName": "feature",
            "headRefOid": revision,
            "baseRefName": "main",
            "isCrossRepository": cross_repository,
            "url": "https://github.com/owner/repo/pull/42"
        }))
        .expect("pull request fixture")
    }

    #[test]
    fn github_remote_identity_parser_accepts_exact_repository_shapes_only() {
        let expected = GitHubRepositoryIdentity {
            owner: "owner".into(),
            name: "repo".into(),
        };
        for remote in [
            "https://github.com/owner/repo.git",
            "ssh://git@github.com/owner/repo.git",
            "git@github.com:owner/repo.git",
            "github.com/owner/repo",
        ] {
            assert_eq!(github_repository_identity(remote), Some(expected.clone()));
        }
        for remote in [
            "https://github.com/owner",
            "https://github.com/owner/repo/extra",
            "https://github.com/owner/repo.git?redirect=other",
            "C:\\owner\\repo",
        ] {
            assert_eq!(github_repository_identity(remote), None, "{remote}");
        }
    }

    #[tokio::test]
    async fn verified_github_client_scrubs_ambient_repo_and_checks_origin_identity() {
        let rec = RecordingRunner::replying(Reply::ok(
            r#"{"name":"repo","owner":{"login":"owner"},"description":null,"url":"https://github.com/owner/repo","isPrivate":false,"defaultBranchRef":{"name":"main"}}"#,
        ));
        let policy = ExecutionPolicy::new(8192);
        let github = configured_github_client(
            GitHub::with_runner(&rec),
            "https://github.com/owner/repo.git",
            &policy,
            Operation::Publish,
            false,
        )
        .expect("verified host configuration");
        verify_github_repository(
            &github,
            Path::new("/repo"),
            &GitHubRepositoryIdentity {
                owner: "owner".into(),
                name: "repo".into(),
            },
            Operation::Publish,
            false,
        )
        .await
        .expect("matching repository");

        let call = rec.only_call();
        assert!(
            call.envs
                .iter()
                .any(|(key, value)| { key.to_str() == Some("GH_REPO") && value.is_none() })
        );
        assert!(call.env_is("GH_HOST", "github.com"));
        assert_eq!(
            call.args_str(),
            [
                "repo",
                "view",
                "--json",
                "name,owner,description,url,isPrivate,defaultBranchRef"
            ]
        );
    }

    #[tokio::test]
    async fn verified_github_client_rejects_ambient_repository_mismatch() {
        let rec = RecordingRunner::replying(Reply::ok(
            r#"{"name":"other","owner":{"login":"attacker"},"description":null,"url":"https://github.com/attacker/other","isPrivate":false,"defaultBranchRef":{"name":"main"}}"#,
        ));
        let policy = ExecutionPolicy::new(8192);
        let github = configured_github_client(
            GitHub::with_runner(&rec),
            "https://github.com/owner/repo.git",
            &policy,
            Operation::Publish,
            false,
        )
        .expect("verified host configuration");
        let error = verify_github_repository(
            &github,
            Path::new("/repo"),
            &GitHubRepositoryIdentity {
                owner: "owner".into(),
                name: "repo".into(),
            },
            Operation::Publish,
            false,
        )
        .await
        .expect_err("a redirected repository must fail closed");
        assert_eq!(error.kind(), ErrorKind::Denied);
        assert_eq!(error.code(), "forge_repository_identity_mismatch");
    }

    #[test]
    fn change_request_recovery_requires_open_same_repo_exact_revision() {
        let revision = "0123456789abcdef0123456789abcdef01234567";
        let ignored = vec![
            pull_request("CLOSED", revision, Some(false)),
            pull_request("MERGED", revision, Some(false)),
            pull_request("OPEN", revision, Some(true)),
        ];
        assert!(
            select_verified_change_request(
                ignored,
                "feature",
                "main",
                revision,
                "preflight",
                false,
            )
            .expect("closed, merged, and fork PRs are not recovery evidence")
            .is_none()
        );

        let exact = select_verified_change_request(
            vec![pull_request("OPEN", revision, Some(false))],
            "feature",
            "main",
            revision,
            "after_push",
            false,
        )
        .expect("same-repository exact revision")
        .expect("one exact PR");
        assert_eq!(exact.number, 42);

        let mismatch = select_verified_change_request(
            vec![pull_request(
                "OPEN",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                Some(false),
            )],
            "feature",
            "main",
            revision,
            "after_push",
            false,
        )
        .expect_err("a branch-name match at another revision must fail closed");
        assert_eq!(mismatch.kind(), ErrorKind::Denied);
        assert_eq!(mismatch.code(), "change_request_revision_mismatch");

        for candidate in [
            pull_request("OPEN", revision, None),
            pull_request("OPEN", "", Some(false)),
        ] {
            let error = select_verified_change_request(
                vec![candidate],
                "feature",
                "main",
                revision,
                "after_push",
                false,
            )
            .expect_err("missing identity proof must be structured unsupported");
            assert_eq!(error.kind(), ErrorKind::Unsupported);
        }
    }

    #[tokio::test]
    async fn publish_rejects_jujutsu_before_any_typed_mutation() {
        let repo = Repo::from_jj("/repo", "/repo", Jj::with_runner(ScriptedRunner::new()));
        let invocation = Invocation {
            operation: Operation::Publish,
            repository: Some(PathBuf::from("/repo")),
            changes_mode: ChangesMode::Summary,
            content_max_bytes: 8192,
            max_output_bytes: crate::cli::DEFAULT_MAX_OUTPUT_BYTES,
            include_machine_paths: false,
            write_intent: true,
            expected_revision: Some("0123456789abcdef0123456789abcdef01234567".into()),
            message: None,
            commit_paths: Vec::new(),
            remote: Some("origin".into()),
            source: Some("feature".into()),
            target: Some("main".into()),
            expected_remote_revision: Some("absent".into()),
            forge: Some("github".into()),
            expected_account: Some("agent".into()),
            title: Some("title".into()),
            body: Some(String::new()),
            wait_seconds: crate::cli::DEFAULT_WAIT_SECONDS,
            poll_seconds: crate::cli::DEFAULT_POLL_SECONDS,
        };
        let error = match publish_repo(&repo, &invocation, &ExecutionPolicy::new(8192)).await {
            Err(error) => error,
            Ok(_) => panic!("Jujutsu lacks an exact-source checked push primitive"),
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert_eq!(error.code(), "jujutsu_exact_push_unsupported");
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_uses_lossless_os_byte_encoding() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        use std::path::PathBuf;

        let path = PathBuf::from(OsString::from_vec(b"caf\xff.txt".to_vec()));
        let value = MachinePath::from_path(&path, true);
        assert_eq!(value.encoding, "os-bytes-hex");
        assert_eq!(value.value.as_deref(), Some("636166ff2e747874"));
    }

    #[cfg(windows)]
    #[test]
    fn non_unicode_windows_path_uses_lossless_utf16_encoding() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        use std::path::PathBuf;

        let path = PathBuf::from(OsString::from_wide(&[0x0043, 0x003a, 0x005c, 0xd800]));
        let value = MachinePath::from_path(&path, true);
        assert_eq!(value.encoding, "windows-utf16-hex");
        assert_eq!(value.value.as_deref(), Some("0043003a005cd800"));
    }
}
