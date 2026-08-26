//! Intent-oriented MCP adapters over `vcs-agent`'s common outcome services.

use std::path::PathBuf;
use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ErrorData, tool, tool_router};
use vcs_agent::OutcomeServices;
use vcs_agent::app::ExecutionPolicy;
use vcs_agent::cli::{
    ChangesMode, DEFAULT_CONTENT_MAX_BYTES, DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_POLL_SECONDS,
    DEFAULT_WAIT_SECONDS, Invocation, Operation,
};

use crate::VcsMcpServer;
use crate::params::*;

pub(crate) const OUTCOME_FORGE_TOOLS: &[&str] =
    &["outcome_publish", "outcome_ci_status", "outcome_ci_wait"];

#[tool_router(router = outcome_tool_router, vis = "pub(crate)")]
impl VcsMcpServer {
    #[tool(
        description = "Use this when you need one policy-consistent repository/remote/forge preflight before choosing another tool. Returns the same bounded inspect outcome as vcs-agent, including capabilities and evidence. Do not use this for raw history or file content.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn outcome_inspect(&self) -> Result<CallToolResult, ErrorData> {
        self.run_outcome(self.outcome_request(Operation::Inspect), None)
            .await
    }

    #[tool(
        description = "Use this when you need an outcome-oriented change summary or bounded structured diff before a commit. Returns the same counts, paths, read semantics, and content-budget behavior as vcs-agent. Do not use this to mutate files or repository state.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn outcome_changes(
        &self,
        Parameters(p): Parameters<OutcomeChangesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut request = self.outcome_request(Operation::Changes);
        request.changes_mode = match p.mode.unwrap_or(OutcomeChangesModeArg::Summary) {
            OutcomeChangesModeArg::Summary => ChangesMode::Summary,
            OutcomeChangesModeArg::Full => ChangesMode::Full,
        };
        self.run_outcome(request, None).await
    }

    #[tool(
        description = "Use this when the exact expected revision and exact repo-relative paths are known and you intend one checked commit. The common service performs preflight, preserves unrelated changes, serializes the mutation, and returns revision evidence. Do not use this without explicit write approval or for an all-files commit.",
        annotations(destructive_hint = true)
    )]
    pub async fn outcome_commit(
        &self,
        Parameters(p): Parameters<OutcomeCommitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("outcome_commit").await?;
        nonempty(&p.expected_revision, "expected_revision")?;
        nonempty(&p.message, "message")?;
        if p.paths.is_empty() {
            return Err(ErrorData::invalid_params(
                "paths must contain at least one exact repo-relative path".to_string(),
                None,
            ));
        }
        let mut request = self.outcome_request(Operation::Commit);
        request.write_intent = true;
        request.expected_revision = Some(p.expected_revision);
        request.message = Some(p.message);
        request.commit_paths = p.paths.into_iter().map(PathBuf::from).collect();
        self.run_outcome(request, None).await
    }

    #[tool(
        description = "Use this when you intend to publish one exact revision and create or recover its pull/merge request under a verified forge account. The common service owns push preflight, retry-safe outcome recovery, evidence, and policy. Do not use this for an unchecked push or without explicit write approval.",
        annotations(destructive_hint = true)
    )]
    pub async fn outcome_publish(
        &self,
        Parameters(p): Parameters<OutcomePublishParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("outcome_publish").await?;
        for (value, field) in [
            (&p.expected_revision, "expected_revision"),
            (&p.expected_remote_revision, "expected_remote_revision"),
            (&p.remote, "remote"),
            (&p.source, "source"),
            (&p.target, "target"),
            (&p.forge, "forge"),
            (&p.expected_account, "expected_account"),
            (&p.title, "title"),
        ] {
            nonempty(value, field)?;
        }
        checked_forge(&p.forge)?;
        let mut request = self.outcome_request(Operation::Publish);
        request.write_intent = true;
        request.expected_revision = Some(p.expected_revision);
        request.expected_remote_revision = Some(p.expected_remote_revision);
        request.remote = Some(p.remote);
        request.source = Some(p.source);
        request.target = Some(p.target);
        request.forge = Some(p.forge);
        request.expected_account = Some(p.expected_account);
        request.title = Some(p.title);
        request.body = Some(p.body);
        self.run_outcome(request, None).await
    }

    #[tool(
        description = "Use this when you need CI evidence tied to one exact published revision. The common service refuses stale or mismatched runs. Do not use this for branch-only best-effort status or when no forge is configured.",
        annotations(read_only_hint = true)
    )]
    pub async fn outcome_ci_status(
        &self,
        Parameters(p): Parameters<OutcomeCiStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        checked_ci(&p)?;
        let mut request = self.outcome_request(Operation::CiStatus);
        request.forge = Some(p.forge);
        request.source = Some(p.source);
        request.expected_revision = Some(p.expected_revision);
        self.run_outcome(request, None).await
    }

    #[tool(
        description = "Use this when you need terminal CI evidence for one exact published revision within one aggregate deadline. The common service polls with bounded diagnostics and refuses revision drift. Do not use this as an unbounded monitor or when no forge is configured.",
        annotations(read_only_hint = true)
    )]
    pub async fn outcome_ci_wait(
        &self,
        Parameters(p): Parameters<OutcomeCiWaitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        checked_forge(&p.forge)?;
        nonempty(&p.source, "source")?;
        nonempty(&p.expected_revision, "expected_revision")?;
        let wait = p.wait_seconds.unwrap_or(DEFAULT_WAIT_SECONDS);
        let poll = p.poll_seconds.unwrap_or(DEFAULT_POLL_SECONDS);
        if wait == 0 || poll == 0 || poll > wait {
            return Err(ErrorData::invalid_params(
                "wait_seconds and poll_seconds must be positive, with poll_seconds <= wait_seconds"
                    .to_string(),
                None,
            ));
        }
        let mut request = self.outcome_request(Operation::CiWait);
        request.forge = Some(p.forge);
        request.source = Some(p.source);
        request.expected_revision = Some(p.expected_revision);
        request.wait_seconds = wait;
        request.poll_seconds = poll;
        self.run_outcome(request, Some(Duration::from_secs(wait)))
            .await
    }

    fn outcome_request(&self, operation: Operation) -> Invocation {
        let budget = self
            .content_budget
            .max_bytes()
            .unwrap_or(DEFAULT_CONTENT_MAX_BYTES);
        Invocation {
            operation,
            repository: Some(self.repo.root().to_path_buf()),
            changes_mode: ChangesMode::Summary,
            content_max_bytes: budget,
            max_output_bytes: self
                .content_budget
                .max_bytes()
                .unwrap_or(DEFAULT_MAX_OUTPUT_BYTES),
            include_machine_paths: false,
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
            wait_seconds: DEFAULT_WAIT_SECONDS,
            poll_seconds: DEFAULT_POLL_SECONDS,
        }
    }

    async fn run_outcome(
        &self,
        request: Invocation,
        deadline: Option<Duration>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut policy = ExecutionPolicy::new(request.content_max_bytes);
        if let Some(deadline) = deadline {
            policy = policy.with_deadline(deadline);
        }
        let output = OutcomeServices::execute(&request, &policy).await;
        let text = String::from_utf8(output.stdout).map_err(|_| {
            ErrorData::internal_error("outcome service returned non-UTF-8 JSON".to_string(), None)
        })?;
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }
}

fn nonempty(value: &str, field: &str) -> Result<(), ErrorData> {
    if value.trim().is_empty() {
        Err(ErrorData::invalid_params(
            format!("{field} must not be empty"),
            None,
        ))
    } else {
        Ok(())
    }
}

fn checked_forge(value: &str) -> Result<(), ErrorData> {
    nonempty(value, "forge")?;
    if matches!(value, "github" | "gitlab" | "gitea") {
        Ok(())
    } else {
        Err(ErrorData::invalid_params(
            "forge must be github, gitlab, or gitea".to_string(),
            None,
        ))
    }
}

fn checked_ci(p: &OutcomeCiStatusParams) -> Result<(), ErrorData> {
    checked_forge(&p.forge)?;
    nonempty(&p.source, "source")?;
    nonempty(&p.expected_revision, "expected_revision")
}
