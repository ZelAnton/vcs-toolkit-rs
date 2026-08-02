//! Prints typed repository-change events through the optional stream interface.
//!
//! See `crates/watch/docs/watch.md` for the full watcher guide.

use tokio_stream::StreamExt;
use vcs_core::Repo;
use vcs_watch::{RepoEvent, RepoWatcher};

#[tokio::main(flavor = "current_thread")]
async fn main() -> vcs_watch::Result<()> {
    let repo = Repo::discover(".")?;
    let mut watcher = RepoWatcher::watch(repo).await?;

    while let Some(change) = watcher.next().await {
        for event in &change.events {
            println!("{}: {event:?}", event_name(event));
        }
        println!("snapshot: {:#?}", change.snapshot);
    }

    Ok(())
}

fn event_name(event: &RepoEvent) -> &'static str {
    match event {
        RepoEvent::HeadMoved { .. } => "head moved",
        RepoEvent::BranchSwitched { .. } => "branch switched",
        RepoEvent::BranchCreated { .. } => "branch created",
        RepoEvent::BranchDeleted { .. } => "branch deleted",
        RepoEvent::WorkingCopyChanged { .. } => "working copy changed",
        RepoEvent::UpstreamChanged { .. } => "upstream changed",
        RepoEvent::AheadBehindChanged { .. } => "ahead/behind changed",
        RepoEvent::OperationChanged { .. } => "operation changed",
        RepoEvent::ConflictChanged { .. } => "conflict changed",
        _ => "unknown repository event",
    }
}
