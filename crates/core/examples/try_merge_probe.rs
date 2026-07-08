//! Probes whether a branch can be merged without leaving a merge in progress.
//!
//! See `crates/core/docs/cookbook.md` — "Probe a merge for conflicts".
//!
//! ```no_run
//! # async fn run(repo: &vcs_core::Repo) -> vcs_core::Result<()> {
//! let outcome = repo.try_merge("feature").await?;
//! # let _ = outcome;
//! # Ok(())
//! # }
//! ```

use vcs_core::{MergeProbe, Repo};

#[tokio::main]
async fn main() -> vcs_core::Result<()> {
    let repo = Repo::discover(".")?;

    match repo.try_merge("feature").await? {
        MergeProbe::Clean => println!("feature merges cleanly"),
        MergeProbe::Conflicts(paths) => println!("conflicts: {}", paths.join(", ")),
        _ => println!("merge result is not recognized by this version"),
    }

    Ok(())
}
