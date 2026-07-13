# Extending vcs-toolkit-rs

This is the contributor workflow for a new capability: validate the real
contract, add the typed implementation, prove it hermetically, document it in
the owning crate guide, and update that crate's `[Unreleased]` changelog entry.
Use the [guide map](README.md) to select the right layer: wrappers own one CLI,
`vcs-core` unifies git/jj, `vcs-forge` unifies forges, and `vcs-mcp` is the MCP
adapter over the facades.

## 1. Adding a typed method to a CLI wrapper

Use this for [vcs-git](../crates/git/docs/git.md),
[vcs-jj](../crates/jj/docs/jj.md),
[vcs-github](../crates/github/docs/github.md),
[vcs-gitlab](../crates/gitlab/docs/gitlab.md), or
[vcs-gitea](../crates/gitea/docs/gitea.md). The wrapper owns argv, exit-code
semantics, and parsing; neither a facade nor MCP should recreate them.

### Validate the CLI before designing the API

Run the installed binary against a disposable real repository/project. Record
its version, complete argv, whether each input is a flag or positional, stdout,
stderr, JSON field names/nullability, and meaningful exit codes. Test omitted
options, empty input, and a positional beginning with `-`. Do not infer `glab`
or `tea` behavior from a similarly named `gh` command.

[`GitHubApi::pr_view`](../crates/github/src/lib.rs) is a small real example:
`gh pr view <number> --json <fields>`. Its typed number and shared JSON parser
come directly from the observed contract.

```rust,ignore
async fn pr_view(&self, dir: &Path, number: u64) -> Result<PullRequest> {
    let n = number.to_string();
    self.core
        .try_parse(
            self.core.command_in(dir, ["pr", "view", n.as_str(), "--json", PR_FIELDS]),
            |s| vcs_cli_support::json::from_json(BINARY, s),
        )
        .await
}
```

Model ordinary failures, normal predicate exit codes, and a failing command
that still emits JSON separately. Those are three different public contracts.

### Guard positional argv and choose a usable signature

Use a validating newtype when one exists: `vcs-git`'s `RefName` and `RevSpec`
keep flag-like refs and revisions out of argv. For an untyped bare positional,
guard it before creating the command:

```rust,ignore
use vcs_cli_support::reject_flag_like;

fn checked_remote(remote: &str) -> Result<()> {
    reject_flag_like("git", "remote name", remote)
}
```

The guard rejects empty/whitespace values, leading `-`, and an interior NUL.
Apply it to every caller-controlled *bare positional*. Do not reject leading
dashes blindly at the MCP or facade layer: a value after `--title` or `--body`
is a flag-value slot, where Markdown such as `- item` is valid. Only the wrapper
knows the final argv layout. See [Security & hardening](../crates/git/docs/security.md).

Follow the [builder/spec rule](../CONTRIBUTING.md): use a builder/spec for
**two or more options, or any bare boolean**. Named setters make a request
unambiguous and keep optional flags absent unless selected.

```rust,ignore
let create = PrCreate::new("Add extending guide", "Contributor workflow")
    .head("docs/extending")
    .base("main");
let merge = PrMerge::squash().auto().delete_branch();
```

[`PrCreate`](../crates/github/src/lib.rs) and
[`PrMerge`](../crates/github/src/lib.rs) are the existing patterns; avoid an
ambiguous call such as `pr_close(number, true)`.

### Test argv, parsing, and failure paths without a process

Inject `ScriptedRunner` to exercise the real command builder and parser, and
wrap it with `RecordingRunner` when exact argv/cwd/env matters. The complete
recipes are in [the testing guide](../crates/testkit/docs/testing.md).

```rust,ignore
use processkit::testing::{RecordingRunner, Reply};
use std::path::Path;
use vcs_github::{GitHub, GitHubApi};

# async fn test() {
let rec = RecordingRunner::replying(Reply::ok(
    r#"{"number":7,"title":"Add docs","state":"OPEN"}"#,
));
let gh = GitHub::with_runner(&rec);
assert_eq!(gh.pr_view(Path::new("/repo"), 7).await.unwrap().number, 7);
assert_eq!(rec.only_call().args_str(), ["pr", "view", "7", "--json", PR_FIELDS]);
# }
```

For each method, add a successful parse test; an exact argv test including flags
that must be absent; all observed exit-code cases using `Reply::fail` (and
`.with_stdout(json)` if applicable); and a flag-like positional test that
proves rejection before any runner call. Add a real-binary integration test only
for a behavior the hermetic seam cannot prove.

Finish by updating the wrapper trait and implementation, its per-crate guide,
and that crate's `CHANGELOG.md`. Crates publish independently, so a
user-visible method needs the owning crate's release note.

## 2. Adding a facade operation

Add a facade operation only when it has a useful portable meaning:
[vcs-core](../crates/core/docs/core.md) for git/jj or
[vcs-forge](../crates/forge/docs/forge.md) for GitHub/GitLab/Gitea. A facade is
the least common denominator (LCD), not one backend's CLI renamed.

### Extend dispatch and the public trait together

The concrete method dispatches across every backend and the facade trait makes
that operation injectable to downstream consumers. The
[`Forge::pr_merge`](../crates/forge/src/lib.rs) implementation is the model:

```rust,ignore
pub async fn pr_merge(&self, number: u64, merge: PrMerge) -> Result<()> {
    match &self.backend {
        Backend::GitHub(c) => github_forge::pr_merge(c, &self.cwd, number, merge).await,
        Backend::GitLab(c) => gitlab_forge::pr_merge(c, &self.cwd, number, merge).await,
        Backend::Gitea(c) => gitea_forge::pr_merge(c, &self.cwd, number, merge).await,
        Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_merge")),
    }
}

#[async_trait::async_trait]
pub trait ForgeApi: Send + Sync {
    async fn pr_merge(&self, number: u64, merge: PrMerge) -> Result<()>;
}
```

Add the trait declaration, concrete dispatch, every backend mapper, and the
`ForgeApi for Forge<R>` forwarding implementation together. Use the equivalent
`VcsRepo` pattern in [`vcs-core`](../crates/core/src/lib.rs) for git/jj.

### Make divergences explicit

Keep only fields and requirements with the same meaning on every backend; use
`Option` for information a backend cannot supply and structured `Unsupported`
for an operation/option it cannot perform. Never silently drop a requested
feature to make a call appear portable.

`PrMerge` is the real example: all forges map a merge strategy, but `auto` and
`delete_branch` are GitHub-only. GitLab and Gitea return `Unsupported` rather
than performing a different merge. Its tests in
[`vcs-forge`](../crates/forge/src/lib.rs) assert each backend's argv and that
refusal. Document equivalent support matrices, list limits, optional fields,
and `Unsupported` cases in the facade guide; link users to a wrapper when they
need a non-portable operation.

### Test through the trait/runner seam

Consumers can take `&dyn VcsRepo` or `&dyn ForgeApi` for trait-based injection.
The `mock` feature is on wrapper traits (`GitApi`, `JjApi`, `GitHubApi`,
`GitLabApi`, `GiteaApi`) and generates `Mock*Api` types. The facade traits
intentionally have no mock feature: build `Repo::from_git` / `Repo::from_jj` or
`Forge::from_github` and its siblings over `ScriptedRunner` to prove real
dispatch, argv, and parsing. See [Testing & mocking](../crates/testkit/docs/testing.md)
and [the stability guide](../crates/core/docs/stability.md).

## 3. Adding an MCP tool

An MCP tool is a policy-enforcing adapter over a typed `Repo` or `Forge`
operation. Start with [vcs-mcp](../crates/mcp/docs/mcp.md) and a neighboring
`#[tool]` method in [`VcsMcpServer`](../crates/mcp/src/lib.rs); a tool must not
assemble CLI argv or duplicate wrapper validation.

### Name, annotate, and serialize it

Use `repo_*` for repo-facade operations and `forge_*` for configured-forge
operations. The name is public MCP API and, for mutations, an `--allow-tools`
value, so it must match `WRITE_TOOLS` exactly. Give every tool a precise
description and correct metadata: reads use `read_only_hint`, mutations use
`destructive_hint`.

```rust,ignore
#[tool(
    description = "A batched snapshot of the repo state.",
    annotations(read_only_hint = true)
)]
pub async fn repo_snapshot(&self) -> Result<CallToolResult, ErrorData> {
    ok_json(&self.repo.snapshot().await.map_err(core_err)?)
}
```

Use typed serializable parameter structs, `ok_json` for DTO output, and
`core_err`/`forge_err` so unsupported and invalid facade requests become useful
MCP parameter errors.

### Write-gate all mutations before calling a facade

A mutating tool must be listed in `WRITE_TOOLS`, annotated destructive, and
checked by `WriteGate` before it does work. Normal remote forge mutations use
`require_write`:

```rust,ignore
self.require_write("forge_pr_close")?;
self.forge()?.pr_close(spec).await.map_err(forge_err)?;
```

Use `begin_repo_write("tool_name")` when it can mutate the local working copy
or race a repo mutation. It performs the same gate check and holds the per-repo
lock until completion:

```rust,ignore
let _write = self.begin_repo_write("forge_pr_merge").await?;
self.forge()?.pr_merge(p.number, merge).await.map_err(forge_err)?;
```

[`forge_pr_merge`](../crates/mcp/src/lib.rs) and `forge_pr_checkout` take this
stronger path because they can affect the local checkout; ordinary remote forge
tools use `require_write`. Do not acquire a lock before the gate check.

### Bound content output

Content tools must propagate an
[`OutputBudget`](../crates/core/src/lib.rs) to the underlying client.
`--max-output-bytes` configures the MCP clients; `repo_show_file` and
`forge_pr_diff` are the current examples. Over-budget content must return
`OutputTooLarge`, never a silently truncated or unbounded JSON response. The
existing MCP tests cover both over-budget errors and complete under-budget data.

Test routing, tool-router registration, descriptions/annotations, disabled and
selected `WriteGate` cases, backend `Unsupported`, and budget failure. Update
the MCP tool catalogue and write-gate notes in
[the MCP guide](../crates/mcp/docs/mcp.md), plus `crates/mcp/CHANGELOG.md` for a
public server change.

## 4. Where decisions and proposals belong

Keep present-tense guides separate from planning records. At repository root:

| Location | Purpose |
|---|---|
| `ideas/next-*.md` | A high-value open proposal to reconsider when the near-term roadmap drains; state the evidence, alternatives, affected APIs, and open question. |
| `ideas/later-*.md` | A more distant proposal or one gated on a concrete consumer/upstream release; name the gate and evidence needed. |
| `decisions/wont-do-*.md` | A settled decision not to add/change something; record the choice, alternatives, evidence, and consequences. |

`next` is just below the roadmap cut, not a second backlog. Use `later` when
timing or a dependency is genuinely uncertain. Use `wont-do` only after enough
evidence to stop relitigating the decision. The language-binding contract is
the durable-decision pattern: `decisions/keep-bindable.md`.

New evidence can reopen a decision: a concrete consumer, changed upstream
behavior, measured security/performance data, or a changed constraint. Do not
silently rewrite history. Open `ideas/next-*.md` or `ideas/later-*.md` linking
to the old decision, state the new evidence and exact question, then add a
status/link to the decision when reconsideration is accepted. Otherwise leave
the decision settled and record the trigger needed to revisit it.

## See also

- [Documentation guide map](README.md) — choose the crate and cross-cutting guide first.
- [CONTRIBUTING.md](../CONTRIBUTING.md) — changelog, builder/spec, formatting, and release conventions.
- [Testing & mocking](../crates/testkit/docs/testing.md) — runners, real-binary tests, and wrapper mocks.
- [Security & hardening](../crates/git/docs/security.md) — argv injection barriers and hardened Git.
- [vcs-core](../crates/core/docs/core.md), [vcs-forge](../crates/forge/docs/forge.md), and [vcs-mcp](../crates/mcp/docs/mcp.md) — the facade and MCP layers.