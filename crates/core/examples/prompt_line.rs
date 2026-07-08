//! Builds a compact repository prompt from one snapshot.
//!
//! See `crates/core/docs/cookbook.md` — "A prompt / status-bar line in one or two spawns".
//!
//! ```no_run
//! # async fn run() -> vcs_core::Result<()> {
//! let repo = vcs_core::Repo::discover(".")?;
//! let snapshot = repo.snapshot().await?;
//! # let _ = snapshot;
//! # Ok(())
//! # }
//! ```

use vcs_core::Repo;

#[tokio::main]
async fn main() -> vcs_core::Result<()> {
    let snapshot = Repo::discover(".")?.snapshot().await?;
    let branch = snapshot.branch.as_deref().unwrap_or("(detached)");
    let dirty = if snapshot.dirty { " *" } else { "" };
    let conflicts = if snapshot.conflicted { " ⚠" } else { "" };

    println!("{branch}{dirty}{conflicts}");
    Ok(())
}
