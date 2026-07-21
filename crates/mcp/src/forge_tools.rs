//! The `forge_*` tools over the `vcs_forge::Forge` facade (reads + gated
//! mutations), as one `#[tool_router]` impl block whose router is combined with
//! the repo router in `VcsMcpServer::from_handles`. The tool methods stay
//! inherent `pub` methods on [`VcsMcpServer`], so their public paths
//! (`VcsMcpServer::forge_pr_create`, …) are unchanged.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ErrorData, tool, tool_router};

use crate::VcsMcpServer;
use crate::output::{forge_err, ok_json};
use crate::params::*;

#[tool_router(router = forge_tool_router, vis = "pub(crate)")]
impl VcsMcpServer {
    // --- forge: read -------------------------------------------------------

    #[tool(
        description = "Whether the forge CLI reports an authenticated session.",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_auth_status(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.auth_status().await.map_err(forge_err)?)
    }

    #[tool(
        description = "The repository/project on the configured forge (Unsupported on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_repo_view(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.repo_view().await.map_err(forge_err)?)
    }

    #[tool(
        description = "Open pull/merge requests on the configured forge (up to 100; ~50 on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_pr_list(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.pr_list().await.map_err(forge_err)?)
    }

    #[tool(
        description = "A single pull/merge request by number.",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_pr_view(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.pr_view(p.number).await.map_err(forge_err)?)
    }

    #[tool(
        description = "The PR/MR's coarse CI status (Unsupported on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_pr_checks(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.pr_checks(p.number).await.map_err(forge_err)?)
    }

    #[tool(
        description = "The PR/MR's diff, one file entry per changed file (Unsupported on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_pr_diff(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.pr_diff(p.number).await.map_err(forge_err)?)
    }

    #[tool(
        description = "Open issues on the configured forge (up to 100; ~50 on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_issue_list(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.issue_list().await.map_err(forge_err)?)
    }

    #[tool(
        description = "A single issue by number, with body and URL filled.",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_issue_view(
        &self,
        Parameters(p): Parameters<IssueNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(
            &self
                .forge()?
                .issue_view(p.number)
                .await
                .map_err(forge_err)?,
        )
    }

    #[tool(
        description = "Releases on the configured forge, newest first (up to 100; ~50 on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_release_list(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.release_list().await.map_err(forge_err)?)
    }

    #[tool(
        description = "A single release by tag (Unsupported on Gitea — filter forge_release_list instead).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_release_view(
        &self,
        Parameters(p): Parameters<ReleaseTagParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(
            &self
                .forge()?
                .release_view(&p.tag)
                .await
                .map_err(forge_err)?,
        )
    }

    // --- forge: mutations (gated) -----------------------------------------

    #[tool(
        description = "Open an issue, returning the CLI's output (the URL on success). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_issue_create(
        &self,
        Parameters(p): Parameters<IssueCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_issue_create")?;
        // No MCP-layer argv guard on `title`/`body`: every backend passes both in
        // a flag-VALUE slot (`--title`/`--body`/`--description`), so a leading `-`
        // is safe — uniform with `forge_pr_comment`/`forge_pr_edit` (T-013). Any
        // genuine bare-positional slot is guarded in its own wrapper.
        let out = self
            .forge()?
            .issue_create(vcs_forge::IssueCreate::new(p.title, p.body))
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "output": out }))
    }

    #[tool(
        description = "Open a pull/merge request, returning the CLI's output (the URL on success). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_create(
        &self,
        Parameters(p): Parameters<PrCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_create")?;
        // No MCP-layer argv guard on `title`/`body`: both ride in flag-VALUE slots
        // on every backend (`--title`/`--body`/`--description`), so a leading `-`
        // is safe — uniform with `forge_pr_comment`/`forge_pr_edit` (T-013).
        let mut spec = vcs_forge::PrCreate::new(p.title, p.body);
        if let Some(source) = p.source {
            spec = spec.source(source);
        }
        if let Some(target) = p.target {
            spec = spec.target(target);
        }
        let out = self.forge()?.pr_create(spec).await.map_err(forge_err)?;
        ok_json(&serde_json::json!({ "output": out }))
    }

    #[tool(
        description = "Merge a pull/merge request with a strategy (merge|squash|rebase). Optional `auto` (merge once requirements are met) and `delete_branch` are GitHub-only — GitLab/Gitea reject them as unsupported rather than merging without them. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_merge(
        &self,
        Parameters(p): Parameters<PrMergeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // With `delete_branch`, `gh pr merge --delete-branch` deletes the local
        // branch and switches the checkout to the default branch — a local
        // working-copy mutation that races `repo_*` mutations the same way they
        // race each other. Take the lock unconditionally (rather than only when
        // `delete_branch` is set) to keep this simple and avoid any race in a
        // conditional-lock branch (see the `write_lock` field comment).
        let _write = self.begin_repo_write("forge_pr_merge").await?;
        let mut merge = vcs_forge::PrMerge::new(p.strategy.into());
        if p.auto {
            merge = merge.auto();
        }
        if p.delete_branch {
            merge = merge.delete_branch();
        }
        self.forge()?
            .pr_merge(p.number, merge)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "merged": p.number }))
    }

    #[tool(
        description = "Close a pull/merge request without merging. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_close(
        &self,
        Parameters(p): Parameters<PrCloseParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_close")?;
        let mut spec = vcs_forge::PrClose::new(p.number);
        if p.delete_branch {
            spec = spec.delete_branch();
        }
        self.forge()?.pr_close(spec).await.map_err(forge_err)?;
        ok_json(&serde_json::json!({ "closed": p.number }))
    }

    #[tool(
        description = "Mark a draft pull/merge request as ready for review. Requires write access (--allow-write, or --allow-tools naming this tool). `Unsupported` on Gitea (`tea` has no ready command).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_mark_ready(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_mark_ready")?;
        self.forge()?
            .pr_mark_ready(p.number)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "ready": p.number }))
    }

    #[tool(
        description = "Post a comment to an existing pull/merge request, returning the CLI's output. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_comment(
        &self,
        Parameters(p): Parameters<PrCommentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_comment")?;
        // No MCP-layer argv guard on `body`: argv-injection safety is a
        // wrapper-layer concern, and only the wrapper knows which argv slot a
        // value lands in. GitHub (`gh pr comment --body <body>`) and GitLab
        // (`glab mr note -m <body>`) put the body in a flag-VALUE slot, where a
        // leading `-` is safe and typical for Markdown (a `- item` bullet list or
        // `---` rule); Gitea's `tea comment <n> <body>` is the one bare positional,
        // and the Gitea wrapper already guards it with `reject_flag_like`. A blanket
        // leading-`-` refusal here wrongly rejected legitimate Markdown on
        // GitHub/GitLab (T-013). (An empty body is still rejected by the facade
        // itself, before any spawn.)
        let out = self
            .forge()?
            .pr_comment(p.number, &p.body)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "output": out }))
    }

    #[tool(
        description = "Edit a pull/merge request's title and/or body. At least one of `title` or `body` must be set; both absent is rejected up front as an invalid-params error. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_edit(
        &self,
        Parameters(p): Parameters<PrEditParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_edit")?;
        // No MCP-layer argv guard on `title`/`body` (see `forge_pr_comment`): every
        // backend passes both in a flag-VALUE slot (`gh`/`tea` `--title`/`--body`/
        // `--description`, `glab mr update --title`/`--description`), so a leading
        // `-` is safe here — refusing it wrongly rejected legitimate Markdown
        // titles/bodies (T-013). The facade still rejects both-`None` with
        // `InvalidInput` before spawning — a backstop the MCP tool surfaces as
        // `invalid_params`.
        let mut edit = vcs_forge::PrEdit::new();
        if let Some(title) = p.title {
            edit = edit.title(title);
        }
        if let Some(body) = p.body {
            edit = edit.body(body);
        }
        self.forge()?
            .pr_edit(p.number, edit)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "edited": p.number }))
    }

    #[tool(
        description = "Submit an approving review on a pull/merge request (gh pr review --approve / glab mr approve / tea pr approve). Supported on all three forges. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_approve(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_approve")?;
        // A remote review action (no local working-copy mutation), so `require_write`
        // rather than the repo write lock — uniform with `forge_pr_comment`.
        self.forge()?
            .pr_approve(p.number)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "approved": p.number }))
    }

    #[tool(
        description = "Submit a \"request changes\" review with a required body/reason (gh pr review --request-changes --body / tea pr reject). `Unsupported` on GitLab, whose review model is approve/revoke with no request-changes action. An empty body is rejected up front as invalid params. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_request_changes(
        &self,
        Parameters(p): Parameters<PrRequestChangesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_request_changes")?;
        // No MCP-layer argv guard on `body` (see `forge_pr_comment`): GitHub puts it
        // in a flag-VALUE slot (`--body`), and the Gitea wrapper guards its bare
        // positional itself. The facade also rejects an empty body — and reports
        // GitLab `Unsupported` — before any spawn, surfaced here as invalid params.
        self.forge()?
            .pr_request_changes(p.number, &p.body)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "requested_changes": p.number }))
    }

    #[tool(
        description = "Check out a pull/merge request's branch into the local working copy (gh pr checkout / glab mr checkout / tea pr checkout). Mutates the working copy — the head/source branch is fetched and switched to. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_checkout(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // Unlike most forge tools, this one locally mutates the working copy (the
        // head/source branch is fetched and switched to), so it races `repo_*`
        // mutations the same way they race each other — gate it through the same
        // per-repo write lock (see the `write_lock` field comment).
        let _write = self.begin_repo_write("forge_pr_checkout").await?;
        self.forge()?
            .pr_checkout(p.number)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "checked_out": p.number }))
    }

    #[tool(
        description = "Create a release, returning the CLI's output (the URL on success). `draft` and `prerelease` are GitHub/Gitea-only — GitLab rejects them as unsupported (`invalid_params`) rather than creating without them. Asset uploads are not supported here. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_release_create(
        &self,
        Parameters(p): Parameters<ReleaseCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_release_create")?;
        // A remote mutation (creates a release on the forge), not a local
        // working-copy change, so `require_write` rather than the repo write lock —
        // uniform with `forge_issue_create`/`forge_pr_create`. No MCP-layer argv
        // guard on `tag`/`title`/`notes`: the bare-positional `<tag>` is guarded in
        // each wrapper (gh/glab `reject_flag_like`, tea takes it as a `--tag` flag),
        // and title/notes ride in flag-VALUE slots.
        let mut spec = vcs_forge::ReleaseCreate::new(p.tag);
        if let Some(title) = p.title {
            spec = spec.title(title);
        }
        if let Some(notes) = p.notes {
            spec = spec.notes(notes);
        }
        if p.draft {
            spec = spec.draft();
        }
        if p.prerelease {
            spec = spec.prerelease();
        }
        let out = self
            .forge()?
            .release_create(spec)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "output": out }))
    }

    #[tool(
        description = "Delete a release by its Git tag (gh release delete / glab release delete / tea releases delete). Deletes the release only, not the underlying git tag. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_release_delete(
        &self,
        Parameters(p): Parameters<ReleaseTagParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_release_delete")?;
        // A remote mutation, so `require_write` rather than the repo write lock. The
        // bare-positional `<tag>` is guarded in each wrapper (`reject_flag_like`).
        self.forge()?
            .release_delete(&p.tag)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "deleted": p.tag }))
    }

    #[tool(
        description = "The forge's identity and flat capability map (read-only). Returns `{ kind, capabilities: { pr_create, pr_comment, pr_edit, pr_checks, pr_merge, pr_approve, pr_request_changes, issue_create, release_create, release_delete, version, supported, authed } }` for the configured forge. `version` is the installed CLI's `{major,minor,patch}` (or null if unknown/unrecognisable) and `supported` whether it meets the CLI's declared version floor; the per-op flags are the intersection of \"the CLI ships the command\", `supported`, and `authed`. `pr_request_changes` is always false for GitLab (its review model is approve/revoke). Note: for GitLab, `authed` is best-effort (`glab auth status` can report authed when it is not); a real API call is the sure test.",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_info(&self) -> Result<CallToolResult, ErrorData> {
        let forge = self.forge()?;
        let kind = forge.kind();
        let capabilities = forge.capabilities().await.map_err(forge_err)?;
        ok_json(&serde_json::json!({
            "kind": kind.as_str(),
            "capabilities": capabilities,
        }))
    }
}
