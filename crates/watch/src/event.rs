//! The typed events and the **pure** snapshot-diff that derives them.
//!
//! The watcher re-queries repo state on each filesystem change and diffs the new
//! state against the old; [`diff`] turns a (previous, next) pair into the list of
//! [`RepoEvent`]s that changed. It's pure data in, pure data out — no filesystem,
//! no process, no async — so the load-bearing logic is hermetically unit-tested.

use std::collections::BTreeSet;

use vcs_core::{OperationState, RepoSnapshot};

/// One typed change to a repository's observable state, derived by diffing two
/// consecutive [`RepoSnapshot`]s (plus the branch set).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RepoEvent {
    /// The working-copy commit moved (a commit, checkout, reset, `jj` op, …).
    /// `from`/`to` are the full object ids; `None` on an unborn git repo.
    HeadMoved {
        /// The previous HEAD/`@` object id.
        from: Option<String>,
        /// The new HEAD/`@` object id.
        to: Option<String>,
    },
    /// The *current* branch (git) / bookmark (jj) changed — a switch/checkout, or
    /// going (in)to a detached/unset state (`None`).
    BranchSwitched {
        /// The previously checked-out branch/bookmark.
        from: Option<String>,
        /// The newly checked-out branch/bookmark.
        to: Option<String>,
    },
    /// A local branch/bookmark appeared.
    BranchCreated {
        /// The new branch/bookmark name.
        name: String,
    },
    /// A local branch/bookmark was removed.
    BranchDeleted {
        /// The removed branch/bookmark name.
        name: String,
    },
    /// The working-copy dirtiness or change count changed (an edit was staged,
    /// committed, stashed, snapshotted, …).
    WorkingCopyChanged {
        /// Whether the working copy now has uncommitted changes.
        dirty: bool,
        /// The new count of changed paths.
        change_count: usize,
    },
    /// The upstream tracking branch changed (git only; always absent on jj).
    UpstreamChanged {
        /// The new upstream tracking branch, or `None` when unset.
        upstream: Option<String>,
    },
    /// The ahead/behind counts versus the upstream changed (git only).
    AheadBehindChanged {
        /// Commits ahead of the upstream now; `None` when there's no upstream **or**
        /// the upstream is set but uncountable (gone/unfetched).
        ahead: Option<usize>,
        /// Commits behind the upstream now; `None` when uncountable (see `ahead`).
        behind: Option<usize>,
    },
    /// The in-progress **operation** changed — a git merge, rebase, or `am` started
    /// or finished. A transition to/from [`OperationState::Conflict`] (jj's conflict
    /// marker) is **not** reported here: `vcs-core` derives jj's `operation` and
    /// `conflicted` from the same bit, so [`ConflictChanged`](RepoEvent::ConflictChanged)
    /// already signals it on both backends. So this event fires only on git, and
    /// `from`/`to` are `Clear`/`Merge`/`Rebase`/`ApplyMailbox`.
    OperationChanged {
        /// The previous operation state.
        from: OperationState,
        /// The new operation state.
        to: OperationState,
    },
    /// Whether the working copy has an unresolved conflict changed.
    ConflictChanged {
        /// Whether the working copy is now conflicted.
        conflicted: bool,
    },
}

/// A batch of changes observed in one settled re-query: the **new full
/// [`RepoSnapshot`]** (ready to render a prompt/status line) plus the typed
/// [`RepoEvent`]s that produced it. A [`RepoWatcher`](crate::RepoWatcher) only
/// yields a `RepoChange` when at least one event fired.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RepoChange {
    /// The repository state after the change.
    pub snapshot: RepoSnapshot,
    /// The typed deltas from the previous state (never empty).
    pub events: Vec<RepoEvent>,
}

/// The observable state the watcher diffs across re-queries: the snapshot's
/// fields (mirrored so this is constructible in-crate — `RepoSnapshot` is
/// `#[non_exhaustive]`) plus the full local-branch set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchState {
    head: Option<String>,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: Option<usize>,
    behind: Option<usize>,
    dirty: bool,
    change_count: usize,
    conflicted: bool,
    operation: OperationState,
    branches: Vec<String>,
}

impl WatchState {
    /// Mirror a [`RepoSnapshot`] (reading its public fields) plus the branch list.
    pub(crate) fn from_snapshot(snapshot: &RepoSnapshot, branches: Vec<String>) -> Self {
        WatchState {
            head: snapshot.head.clone(),
            branch: snapshot.branch.clone(),
            // Flatten the bundled tracking back into the watcher's per-field deltas
            // so `UpstreamChanged` / `AheadBehindChanged` stay distinct signals.
            // `and_then`: the count is `None` for either "no upstream" or an upstream
            // that's set-but-uncountable (M17) — both read as "no count" for the delta,
            // and a set→gone transition still flips `ahead`/`behind` so the event fires.
            upstream: snapshot.tracking.as_ref().map(|t| t.branch.clone()),
            ahead: snapshot.tracking.as_ref().and_then(|t| t.ahead),
            behind: snapshot.tracking.as_ref().and_then(|t| t.behind),
            dirty: snapshot.dirty,
            change_count: snapshot.change_count,
            conflicted: snapshot.conflicted,
            operation: snapshot.operation,
            branches,
        }
    }
}

/// Diff two consecutive states into the events that changed. Pure; the order is
/// stable (head, branch switch, created, deleted, working copy, upstream,
/// ahead/behind, operation, conflict — created/deleted names sorted).
pub(crate) fn diff(prev: &WatchState, next: &WatchState) -> Vec<RepoEvent> {
    let mut events = Vec::new();

    if prev.head != next.head {
        events.push(RepoEvent::HeadMoved {
            from: prev.head.clone(),
            to: next.head.clone(),
        });
    }
    if prev.branch != next.branch {
        events.push(RepoEvent::BranchSwitched {
            from: prev.branch.clone(),
            to: next.branch.clone(),
        });
    }

    // Branch-set delta (sorted for deterministic output, regardless of the
    // order git/jj listed them in).
    let before: BTreeSet<&str> = prev.branches.iter().map(String::as_str).collect();
    let after: BTreeSet<&str> = next.branches.iter().map(String::as_str).collect();
    for name in after.difference(&before) {
        events.push(RepoEvent::BranchCreated {
            name: (*name).to_string(),
        });
    }
    for name in before.difference(&after) {
        events.push(RepoEvent::BranchDeleted {
            name: (*name).to_string(),
        });
    }

    if prev.dirty != next.dirty || prev.change_count != next.change_count {
        events.push(RepoEvent::WorkingCopyChanged {
            dirty: next.dirty,
            change_count: next.change_count,
        });
    }
    if prev.upstream != next.upstream {
        events.push(RepoEvent::UpstreamChanged {
            upstream: next.upstream.clone(),
        });
    }
    if prev.ahead != next.ahead || prev.behind != next.behind {
        events.push(RepoEvent::AheadBehindChanged {
            ahead: next.ahead,
            behind: next.behind,
        });
    }
    // Only the git merge/rebase lifecycle: a transition to/from `Conflict` (jj's
    // conflict marker, which tracks the same bit as `conflicted`) is left to
    // `ConflictChanged` so a jj conflict isn't double-signalled.
    if prev.operation != next.operation
        && prev.operation != OperationState::Conflict
        && next.operation != OperationState::Conflict
    {
        events.push(RepoEvent::OperationChanged {
            from: prev.operation,
            to: next.operation,
        });
    }
    if prev.conflicted != next.conflicted {
        events.push(RepoEvent::ConflictChanged {
            conflicted: next.conflicted,
        });
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clean baseline state on `main` at one commit, no branches.
    fn base() -> WatchState {
        WatchState {
            head: Some("aaaa".into()),
            branch: Some("main".into()),
            upstream: None,
            ahead: None,
            behind: None,
            dirty: false,
            change_count: 0,
            conflicted: false,
            operation: OperationState::Clear,
            branches: vec!["main".into()],
        }
    }

    #[test]
    fn identical_states_yield_no_events() {
        assert!(diff(&base(), &base()).is_empty());
    }

    #[test]
    fn head_move_is_detected() {
        let mut next = base();
        next.head = Some("bbbb".into());
        assert_eq!(
            diff(&base(), &next),
            vec![RepoEvent::HeadMoved {
                from: Some("aaaa".into()),
                to: Some("bbbb".into()),
            }]
        );
    }

    #[test]
    fn branch_switch_is_detected() {
        let mut next = base();
        next.branch = Some("feature".into());
        assert_eq!(
            diff(&base(), &next),
            vec![RepoEvent::BranchSwitched {
                from: Some("main".into()),
                to: Some("feature".into()),
            }]
        );
        // Detaching maps to `to: None`.
        let mut detached = base();
        detached.branch = None;
        assert_eq!(
            diff(&base(), &detached),
            vec![RepoEvent::BranchSwitched {
                from: Some("main".into()),
                to: None,
            }]
        );
    }

    #[test]
    fn branch_create_and_delete_are_sorted_and_paired() {
        let mut next = base();
        // main stays; add feat-b and feat-a, drop nothing.
        next.branches = vec!["main".into(), "feat-b".into(), "feat-a".into()];
        assert_eq!(
            diff(&base(), &next),
            vec![
                RepoEvent::BranchCreated {
                    name: "feat-a".into()
                },
                RepoEvent::BranchCreated {
                    name: "feat-b".into()
                },
            ],
            "created names come out sorted"
        );

        // Deleting `main`, keeping nothing.
        let mut emptied = base();
        emptied.branches = vec![];
        assert_eq!(
            diff(&base(), &emptied),
            vec![RepoEvent::BranchDeleted {
                name: "main".into()
            }]
        );
    }

    #[test]
    fn working_copy_change_fires_on_dirty_or_count() {
        let mut dirtied = base();
        dirtied.dirty = true;
        dirtied.change_count = 3;
        assert_eq!(
            diff(&base(), &dirtied),
            vec![RepoEvent::WorkingCopyChanged {
                dirty: true,
                change_count: 3,
            }]
        );
        // A count change while already dirty still fires (e.g. 1 → 2 edits).
        let mut one = base();
        one.dirty = true;
        one.change_count = 1;
        let mut two = base();
        two.dirty = true;
        two.change_count = 2;
        assert_eq!(
            diff(&one, &two),
            vec![RepoEvent::WorkingCopyChanged {
                dirty: true,
                change_count: 2,
            }]
        );
    }

    #[test]
    fn upstream_and_ahead_behind_are_separate_events() {
        let mut next = base();
        next.upstream = Some("origin/main".into());
        next.ahead = Some(2);
        next.behind = Some(0);
        assert_eq!(
            diff(&base(), &next),
            vec![
                RepoEvent::UpstreamChanged {
                    upstream: Some("origin/main".into()),
                },
                RepoEvent::AheadBehindChanged {
                    ahead: Some(2),
                    behind: Some(0),
                },
            ]
        );
    }

    #[test]
    fn operation_and_conflict_transitions_are_detected() {
        let mut merging = base();
        merging.operation = OperationState::Merge;
        assert_eq!(
            diff(&base(), &merging),
            vec![RepoEvent::OperationChanged {
                from: OperationState::Clear,
                to: OperationState::Merge,
            }]
        );

        let mut conflicted = base();
        conflicted.conflicted = true;
        assert_eq!(
            diff(&base(), &conflicted),
            vec![RepoEvent::ConflictChanged { conflicted: true }]
        );
    }

    // jj derives `operation` and `conflicted` from the same bit, so a conflict
    // appearing flips BOTH (Clear→Conflict and false→true). The redundant
    // `OperationChanged` is suppressed — only `ConflictChanged` is emitted.
    #[test]
    fn jj_conflict_emits_only_conflict_changed_not_operation() {
        let mut next = base();
        next.operation = OperationState::Conflict;
        next.conflicted = true;
        assert_eq!(
            diff(&base(), &next),
            vec![RepoEvent::ConflictChanged { conflicted: true }],
            "Clear→Conflict must not also emit OperationChanged"
        );
        // …and clearing it the same way.
        let mut cleared = base();
        cleared.operation = OperationState::Clear;
        cleared.conflicted = false;
        let mut from = base();
        from.operation = OperationState::Conflict;
        from.conflicted = true;
        assert_eq!(
            diff(&from, &cleared),
            vec![RepoEvent::ConflictChanged { conflicted: false }]
        );
    }

    // A git merge with conflicts is two *distinct* facts: a merge started AND it
    // conflicts — both fire (the Merge endpoint isn't `Conflict`, so it's kept).
    #[test]
    fn git_merge_with_conflict_emits_both_operation_and_conflict() {
        let mut next = base();
        next.operation = OperationState::Merge;
        next.conflicted = true;
        assert_eq!(
            diff(&base(), &next),
            vec![
                RepoEvent::OperationChanged {
                    from: OperationState::Clear,
                    to: OperationState::Merge,
                },
                RepoEvent::ConflictChanged { conflicted: true },
            ]
        );
    }

    // A realistic "commit" burst: HEAD moves, the working copy goes clean — two
    // events from one diff, in the documented order.
    #[test]
    fn multiple_changes_emit_in_stable_order() {
        let mut prev = base();
        prev.dirty = true;
        prev.change_count = 2;
        let mut next = base(); // clean again, new head
        next.head = Some("cccc".into());
        assert_eq!(
            diff(&prev, &next),
            vec![
                RepoEvent::HeadMoved {
                    from: Some("aaaa".into()),
                    to: Some("cccc".into()),
                },
                RepoEvent::WorkingCopyChanged {
                    dirty: false,
                    change_count: 0,
                },
            ]
        );
    }
}
