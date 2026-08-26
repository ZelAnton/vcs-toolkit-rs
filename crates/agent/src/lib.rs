//! Transport-neutral, bounded outcome application services shared by the
//! `vcs-agent` CLI and structured transports such as `vcs-mcp`.
//!
//! Transport adapters parse a request and render [`OutcomeResult`]. Repository
//! discovery, preflight, policy, error mapping, evidence collection, deadlines,
//! cancellation, credential isolation, and content budgets stay in this crate.

pub mod app;
pub mod cli;
pub mod contract;
mod redaction;

use app::{ExecutionPolicy, OutcomeExecutionContext, execute, execute_with_context};
use cli::Invocation;
use contract::{RenderedOutput, render};
use processkit::ProcessRunner;

/// The common application-service entry point used by every transport.
pub struct OutcomeServices;

impl OutcomeServices {
    /// Execute one typed outcome request and produce the bounded v1 envelope.
    ///
    /// Failures are encoded in the same machine contract as successes; callers
    /// must not substitute transport-specific error mapping or truncation.
    pub async fn execute(request: &Invocation, policy: &ExecutionPolicy) -> RenderedOutput {
        match execute(request, policy).await {
            Ok(success) => render(success, request.max_output_bytes),
            Err(error) => render(*error, request.max_output_bytes),
        }
    }

    /// Execute against clients supplied by the transport. This is the MCP path:
    /// it preserves injected runners, credentials and client hardening instead
    /// of rediscovering the repository and rebuilding ambient clients.
    pub async fn execute_in<R: ProcessRunner>(
        request: &Invocation,
        policy: &ExecutionPolicy,
        context: &OutcomeExecutionContext<R>,
    ) -> RenderedOutput {
        match execute_with_context(request, policy, context).await {
            Ok(success) => render(success, request.max_output_bytes),
            Err(error) => render(*error, request.max_output_bytes),
        }
    }
}
