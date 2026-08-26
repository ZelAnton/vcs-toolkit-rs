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

use app::{ExecutionPolicy, execute};
use cli::Invocation;
use contract::{RenderedOutput, render};

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
}
