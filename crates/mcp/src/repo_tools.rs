//! The `repo_*` tools over the `vcs_core::Repo` facade (reads + gated
//! mutations), as one `#[tool_router]` impl block whose router is combined with
//! the forge router in `VcsMcpServer::from_handles`. The tool methods stay
//! inherent `pub` methods on [`VcsMcpServer`], so their public paths
//! (`VcsMcpServer::repo_commit`, …) are unchanged.

use std::path::Path;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ErrorData, tool, tool_router};
use vcs_core::BranchDelete;

use crate::VcsMcpServer;
use crate::output::{RepoInfo, core_err, ok_json};
use crate::params::*;

#[tool_router(router = repo_tool_router, vis = "pub(crate)")]
impl VcsMcpServer {
    // --- repo: read --------------------------------------------------------

    // NOTE (T-068): NOT `read_only_hint = true`. On jj this tool runs a plain,
    // working-copy-**snapshotting** jj command that records an op-log operation, so
    // asserting `readOnlyHint` ("does not modify its environment") would violate the
    // MCP contract. It is classified as the honest, backend-agnostic truth instead —
    // non-destructive + idempotent (the op-log snapshot is append-only/recoverable
    // and changes no tracked content, refs, or bookmarks) — the same classification
    // `repo_try_merge` uses. See the `jj_snapshotting_read_tools_*` tests and the
    // Safety model note in `docs/mcp.md`. It stays callable without a write gate (an
    // op-log snapshot is not a content/ref mutation).
    #[tool(
        description = "A batched snapshot of the repo state: branch, upstream, ahead/behind, HEAD, dirtiness, change count, conflict, and operation state. Read query; on jj it snapshots the working copy (records a reversible op-log operation), so it is annotated non-destructive rather than readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_snapshot(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.snapshot().await.map_err(core_err)?)
    }

    // T-068: `read_only_hint = true` is correct here (unlike the other repo_* reads).
    // `repo_info` spawns NO backend command at all — it reads the backend kind and the
    // root/cwd paths the facade cached at construction, and the forge kind — so it can
    // never snapshot a jj working copy or record an op-log operation. The read-only
    // guarantee holds on both backends. Pinned by `truly_read_only_tools_keep_read_only_hint`.
    #[tool(
        description = "Which backend (git/jj), the repository root, the working directory, and the configured forge (if any).",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_info(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&RepoInfo {
            backend: self.repo.kind().as_str(),
            root: self.repo.root(),
            cwd: self.repo.cwd(),
            forge: self.forge.as_ref().map(|f| f.kind().as_str()),
        })
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate).
    #[tool(
        description = "The working-copy changes (added/modified/deleted/renamed paths). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_status(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.changed_files().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate).
    #[tool(
        description = "Aggregate insertion/deletion/file counts for the working copy. Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_diff_stat(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.diff_stat().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate).
    #[tool(
        description = "The full parsed working-copy diff (per-file hunks/lines) — same scope as repo_diff_stat: git working tree vs HEAD (excludes untracked files), jj @ vs its parent (includes newly-added files). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_diff(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.diff().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate). A plain `jj log`
    // snapshots the working copy first, exactly like the other repo_* reads.
    #[tool(
        description = "Recent history: up to `max` commits reachable from `revspec_or_revset` (a git revspec, e.g. \"HEAD\", or a jj revset, e.g. \"@\"), most-recent-first. `author`/`date` are null on jj — its typed log doesn't currently surface authorship or a timestamp. Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_log(
        &self,
        Parameters(p): Parameters<LogParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(
            &self
                .repo
                .log(&p.revspec_or_revset, p.max)
                .await
                .map_err(core_err)?,
        )
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate). A plain `jj file show`
    // snapshots the working copy first, exactly like the other repo_* reads.
    #[tool(
        description = "The content of a file at a revision (a git revspec, e.g. \"HEAD\", or a jj revset, e.g. \"@-\"). Returns the file's bytes verbatim (including any trailing newline). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_show_file(
        &self,
        Parameters(p): Parameters<ShowFileParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(
            &self
                .repo
                .show_file(&p.rev, &p.path)
                .await
                .map_err(core_err)?,
        )
    }

    #[tool(
        description = "Per-line attribution for a repo-relative file, optionally at a git revspec or jj revset. Each line has id, line, and content; author/date are null on jj because its typed annotation exposes no author or timestamp. Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_annotate(
        &self,
        Parameters(p): Parameters<AnnotateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(
            &self
                .repo
                .annotate(&p.path, p.rev.as_deref())
                .await
                .map_err(core_err)?,
        )
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate).
    #[tool(
        description = "Local branch (git) / bookmark (jj) names. Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_branches(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.local_branches().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate).
    #[tool(
        description = "The current branch/bookmark (null when detached/unset). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_current_branch(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.current_branch().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate). `jj resolve --list`
    // snapshots the working copy first, exactly like the other repo_* reads.
    #[tool(
        description = "Paths with unresolved merge conflicts (repo-relative, '/'-separated). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_conflicts(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.conflicted_files().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate). `jj workspace list`
    // snapshots the working copy first (the per-workspace `workspace root` probes it
    // fans out are already `--ignore-working-copy`, but the top-level list is not).
    #[tool(
        description = "Attached worktrees (git) / workspaces (jj). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_worktrees(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.list_worktrees().await.map_err(core_err)?)
    }

    // NOTE: the absence of `read_only_hint = true` here is DELIBERATE — do not add
    // it back. `try_merge` materializes a real (rolled-back) trial merge, so it is
    // write-gated below; marking it read-only would re-expose it in the default
    // read-only mode and reopen the untrusted-repo filter/textconv code-exec path.
    #[tool(
        description = "Probe whether merging `source` into the current work would conflict, WITHOUT leaving a trace (the probe is always rolled back). It spawns a REAL trial merge that materializes working-tree content, so — like checkout — it is write-gated: on an untrusted repository that materialization can run repo-local `filter`/`textconv` drivers, which the hardened client does not sandbox. Enable it with `--allow-write` or `--allow-tools repo_try_merge`.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_try_merge(
        &self,
        Parameters(p): Parameters<TryMergeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_try_merge").await?;
        ok_json(&self.repo.try_merge(&p.source).await.map_err(core_err)?)
    }

    // --- repo: mutations (gated) ------------------------------------------

    #[tool(
        description = "Commit exactly the given paths with a message (git commit --only / jj commit <filesets>). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_commit(
        &self,
        Parameters(p): Parameters<CommitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_commit").await?;
        // The facade takes `PathBuf`s (lossless for a non-UTF-8 path on Unix); a tool
        // input arrives as JSON `String`s (always UTF-8), so the conversion is exact.
        let paths: Vec<std::path::PathBuf> = p.paths.iter().map(std::path::PathBuf::from).collect();
        self.repo
            .commit_paths(&paths, &p.message)
            .await
            .map_err(core_err)?;
        ok_json(&serde_json::json!({ "committed_paths": paths.len() }))
    }

    #[tool(
        description = "Switch the working copy to a branch/bookmark/revision (git checkout / jj edit). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_checkout(
        &self,
        Parameters(p): Parameters<CheckoutParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_checkout").await?;
        self.repo.checkout(&p.reference).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "checked_out": p.reference }))
    }

    #[tool(
        description = "Rebase the current line onto a branch, bookmark, or revision. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_rebase(
        &self,
        Parameters(p): Parameters<RebaseParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_rebase").await?;
        self.repo.rebase(&p.onto).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "rebased_onto": p.onto }))
    }

    #[tool(
        description = "Abort the in-progress repository operation, if any. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_abort_in_progress(&self) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_abort_in_progress").await?;
        let operation_state = self.repo.abort_in_progress().await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "operation_state": operation_state }))
    }

    #[tool(
        description = "Continue the in-progress repository operation after resolving conflicts. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_continue_in_progress(&self) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_continue_in_progress").await?;
        let operation_state = self.repo.continue_in_progress().await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "operation_state": operation_state }))
    }

    #[tool(
        description = "Start new work on top of a branch, bookmark, or revision. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_new_child(
        &self,
        Parameters(p): Parameters<NewChildParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_new_child").await?;
        self.repo.new_child(&p.reference).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "new_child_of": p.reference }))
    }

    #[tool(
        description = "Create a local branch or bookmark at the current head, without switching the working copy (git branch <name> / jj bookmark create <name> -r @). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_create_branch(
        &self,
        Parameters(p): Parameters<CreateBranchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_create_branch").await?;
        self.repo.create_branch(&p.name).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "created_branch": p.name }))
    }

    #[tool(
        description = "Delete a local branch or bookmark. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_delete_branch(
        &self,
        Parameters(p): Parameters<DeleteBranchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_delete_branch").await?;
        let deleted_branch = p.name.clone();
        let force = p.force;
        let spec = if p.force {
            BranchDelete::new(p.name).force()
        } else {
            BranchDelete::new(p.name)
        };
        self.repo.delete_branch(spec).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "deleted_branch": deleted_branch, "force": force }))
    }

    #[tool(
        description = "Rename a local branch or bookmark. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_rename_branch(
        &self,
        Parameters(p): Parameters<RenameBranchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_rename_branch").await?;
        self.repo
            .rename_branch(&p.old, &p.new)
            .await
            .map_err(core_err)?;
        ok_json(&serde_json::json!({ "renamed": { "old": p.old, "new": p.new } }))
    }

    #[tool(
        description = "Fetch from the default remote (git fetch / jj git fetch). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_fetch(&self) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_fetch").await?;
        self.repo.fetch().await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "fetched": true }))
    }

    #[tool(
        description = "Push an existing branch/bookmark to origin (git push -u origin <branch> / jj git push -b <branch>). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_push(
        &self,
        Parameters(p): Parameters<PushParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_push").await?;
        self.repo.push(&p.branch).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "pushed": p.branch }))
    }

    #[tool(
        description = "Create a worktree/workspace at `path` on a new `branch` from `base`. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_create_worktree(
        &self,
        Parameters(p): Parameters<CreateWorktreeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_create_worktree").await?;
        let spec = vcs_core::WorktreeCreate::new(p.path, p.branch).base(p.base);
        let outcome = self.repo.create_worktree(spec).await.map_err(core_err)?;
        ok_json(&outcome)
    }

    #[tool(
        description = "Remove the worktree/workspace at `path`. Without `force`, a worktree with uncommitted changes is refused; the repository's main worktree/workspace is always refused. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_remove_worktree(
        &self,
        Parameters(p): Parameters<RemoveWorktreeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_remove_worktree").await?;
        let mut spec = vcs_core::WorktreeRemove::new(Path::new(&p.path));
        if p.force {
            spec = spec.force();
        }
        self.repo.remove_worktree(spec).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "removed": p.path }))
    }
}
