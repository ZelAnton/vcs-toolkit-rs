use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use processkit::{CancellationToken, JobRunner, ProcessRunner};
use serde::Serialize;
use vcs_cli_support::OutputBudget;
use vcs_core::vcs_git::{DiffLine, Git};
use vcs_core::vcs_jj::Jj;
use vcs_core::{BackendKind, ChangeKind, FileChange, FileDiff, OperationState, Repo, RepoSnapshot};
use vcs_forge::{
    Forge, ForgeApi, ForgeAuth, ForgeCapabilities, ForgeKind, vcs_gitea::Gitea, vcs_github::GitHub,
    vcs_gitlab::GitLab,
};

use crate::cli::{ChangesMode, Invocation, Operation};
use crate::contract::{
    AgentError, AgentResult, CONTRACT_VERSION, DetailKey, ErrorDescriptor, ErrorKind, ExitBand,
    FailureDomain, Fallback, MachineEnvelope,
};
use crate::redaction::{RedactionPolicy, redact_text};

const DEFAULT_DEADLINE: Duration = Duration::from_secs(120);

/// Policy carried across every outcome implementation and projected onto every
/// typed client. No outcome owns a second process-launch path.
pub(crate) struct ExecutionPolicy {
    pub(crate) cancellation: CancellationToken,
    pub(crate) deadline: Duration,
    pub(crate) content_budget: OutputBudget,
}

impl ExecutionPolicy {
    pub(crate) fn new(content_max_bytes: usize) -> Self {
        Self {
            cancellation: CancellationToken::new(),
            deadline: DEFAULT_DEADLINE,
            content_budget: OutputBudget::bytes(content_max_bytes),
        }
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
    working_copy_content_mutated: bool,
    push_performed: bool,
    switch_performed: bool,
    conflict_repair_performed: bool,
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

pub(crate) async fn execute(
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
        Operation::Inspect | Operation::Changes | Operation::Commit => {
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
        _ => Err(Box::new(
            AgentError::new(
                invocation.operation.name(),
                ErrorKind::Unsupported,
                "outcome_not_implemented",
                false,
            )
            .with_fallback(Fallback::raw_cli("outcome_not_implemented")),
        )),
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
                supported: vec!["probe", "inspect", "changes", "commit"],
                reserved: vec!["publish", "ci status", "ci wait"],
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
    if matches!(repo.kind(), BackendKind::Jj)
        && invocation
            .commit_paths
            .iter()
            .any(|path| path.to_str().is_none())
    {
        return Err(Box::new(commit_gate_error(
            ErrorKind::Unsupported,
            "jujutsu_non_utf8_path_unsupported",
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
        .changed_files()
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

    repo.commit_paths(&invocation.commit_paths, message)
        .await
        .map_err(|error| Box::new(map_core_error(Operation::Commit, include_paths, error)))?;

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
    let changed_after = repo.changed_files().await.map_err(|_| {
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
    if unrelated_before
        .iter()
        .any(|before_change| !changed_after.contains(before_change))
    {
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
        included_paths,
        unrelated_changes_preserved: true,
        semantics,
    })
}

fn commit_semantics(kind: BackendKind) -> CommitSemantics {
    CommitSemantics {
        selection: "exact-repo-relative-paths",
        backend_selection: if matches!(kind, BackendKind::Git) {
            "git-commit-only"
        } else {
            "jujutsu-exact-filesets"
        },
        refs_advanced: true,
        index_may_change_for_selected_paths: matches!(kind, BackendKind::Git),
        unrelated_index_preserved: true,
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
            Git::new()
                .default_timeout(policy.deadline)
                .default_cancel_on(policy.cancellation.clone())
                .default_output_budget(policy.content_budget)
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
    let timeout = policy.deadline;
    let token = &policy.cancellation;
    let budget = policy.content_budget;
    match kind {
        ForgeKind::GitHub => Some(Box::new(Forge::from_github(
            cwd,
            GitHub::new()
                .default_timeout(timeout)
                .default_cancel_on(token.clone())
                .default_output_budget(budget),
        ))),
        ForgeKind::GitLab => Some(Box::new(Forge::from_gitlab(
            cwd,
            GitLab::new()
                .default_timeout(timeout)
                .default_cancel_on(token.clone())
                .default_output_budget(budget),
        ))),
        ForgeKind::Gitea => Some(Box::new(Forge::from_gitea(
            cwd,
            Gitea::new()
                .default_timeout(timeout)
                .default_cancel_on(token.clone())
                .default_output_budget(budget),
        ))),
        _ => Some(Box::new(Forge::<JobRunner>::from_unknown(cwd))),
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
