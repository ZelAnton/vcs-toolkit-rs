# CLI command coverage index

The per-crate guides document the typed surface **from the method outward**
("here's `pr_merge`, here's what it runs"). This page inverts that: **from the
CLI command inward** — "I know `git rebase --onto` / `jj parallelize` / `gh
api`; is it covered by a typed method, or do I need the escape hatch?" Each
table row is one typed method and the exact subcommand/flags it runs, sourced
from the crate's trait definition (`GitApi`/`JjApi`/`GitHubApi`/`GitLabApi`/
`GiteaApi` in `crates/*/src/lib.rs`) — the same source the per-crate guides
document, cross-checked directly against the trait so a method the prose guide
hasn't caught up to yet still shows up here.

This index doubles as a **map of the untyped surface**: everything a wrapper's
`run`/`run_raw` escape hatch reaches but no typed method models yet is a
candidate for a future typed method — see [Extending
vcs-toolkit-rs](extending.md#1-adding-a-typed-method-to-a-cli-wrapper).

## How to read this

- **Runs** — the argv the method builds, elided to the load-bearing
  flags (see the linked guide/trait doc comment for the full contract:
  option types, error classification, argv-injection guards).
- **Not modeled** sections per wrapper list commands **consciously left
  untyped** — reachable only through that wrapper's `run`/`run_raw` (or the
  inherent `run_args`/`run_raw_args`) escape hatch. Each wrapper's CLI has far
  more surface than any table below or its "not modeled" list enumerates in
  full (git alone ships well over a hundred subcommands); the lists name the
  ones a consumer is most likely to look for. **Anything not in a table above
  it is, by definition, unmodeled** — go to the escape hatch.
- A method already reachable through a facade (`vcs-core`'s `Repo`,
  `vcs-forge`'s `Forge`) is not repeated here — this index is the wrapper-level
  wiring the facades dispatch to; see [Facade escape-hatch
  routers](#facade-escape-hatch-routers) for how a facade caller drops back to
  the wrapper level.

## git (`vcs-git` — the `git` binary)

Guide: [vcs-git](../crates/git/docs/git.md). Trait: `GitApi`
(`crates/git/src/lib.rs`).

### Status, log, branches, revisions

| Method | Runs | Notes |
|---|---|---|
| `status` | `status --porcelain=v1 -z` | parsed `Vec<StatusEntry>` |
| `status_text` | `status --porcelain=v1` | raw text |
| `status_tracked` | `status --porcelain=v1 -z --untracked-files=no` | tracked-only dirtiness |
| `branch_status` | `status --porcelain=v2 --branch -z` | combined branch + WT snapshot |
| `conflicted_files` | `diff --name-only --diff-filter=U -z` | repo-relative, lossless paths |
| `current_branch` | `symbolic-ref --quiet --short HEAD` | `None` only when detached |
| `branches` | `branch` | current one flagged |
| `log` | `log <revspec> --` | mirrors `JjApi::log` |
| `log_paths` | `--literal-pathspecs log <revspec> -n <max> -- <paths>` | scoped to paths; non-empty required |
| `rev_parse` | `rev-parse --verify <rev>` | full hash |
| `rev_parse_short` | `rev-parse --short <rev>` | abbreviated hash |
| `resolve_commit` | `rev-parse --verify <rev>^{commit}` | peels annotated tags |
| `is_unborn` | `rev-parse --verify -q HEAD` | fresh repo, no commits |
| `common_dir` | `rev-parse --git-common-dir` | stable across worktrees |
| `git_dir` | `rev-parse --git-dir` | this worktree's git dir |
| `is_merged` | `branch --merged <base>` | via `MergeCheck` |
| `branch_exists` | `show-ref --verify --quiet refs/heads/<name>` | |
| `remote_branch_exists` | `ls-remote origin refs/heads/<name>` | fully-qualified ref, 10s timeout |
| `remote_head_branch` | `symbolic-ref refs/remotes/origin/HEAD` | `None` when unset |
| `remote_url` | `remote get-url <remote>` | |
| `upstream` | `symbolic-ref --quiet --short HEAD` then `rev-parse --abbrev-ref --symbolic-full-name @{u}` | `None` on no upstream; error on detached |
| `remote_branches` | `ls-remote --heads <remote>` | no fetch |
| `rev_list_count` | `rev-list --count <range>` | |
| `is_rebase_in_progress` | probes `rebase-merge`/`rebase-apply` under the git dir | excludes an `am` in progress |
| `is_merge_in_progress` | probes `MERGE_HEAD` under the git dir | |
| `is_am_in_progress` | probes `rebase-apply/applying` | distinct from a rebase |
| `is_cherry_pick_in_progress` | probes `CHERRY_PICK_HEAD` | |
| `is_revert_in_progress` | probes `REVERT_HEAD` | |
| `is_bisect_in_progress` | probes `BISECT_LOG` | ended with `bisect reset`, no `--continue` |

### Staging & committing

| Method | Runs | Notes |
|---|---|---|
| `add` | `--literal-pathspecs add -- <paths>` | large sets go via `--pathspec-from-file` stdin |
| `commit` | `commit -m <message>` | staged index |
| `commit_paths` | `--literal-pathspecs commit [--amend] -m <message> --only -- <paths>` | via `CommitPaths` |
| `last_commit_message` | `log -1 --format=%B` | full message |
| `staged_is_empty` | `diff --cached --quiet` | exit-code mapped |
| `init` | `init` | |

### Checkout, worktrees, tags, clone, config, show

| Method | Runs | Notes |
|---|---|---|
| `checkout` | `checkout <target>` | via `CheckoutTarget` |
| `checkout_detach` | `checkout --detach <commit>` | |
| `create_branch` | `branch <name>` | no switch |
| `set_upstream` | `branch --set-upstream-to=<upstream> <branch>` | |
| `delete_branch` | `branch -d` (`-D` if forced) | via `BranchDelete` |
| `rename_branch` | `branch -m <old> <new>` | |
| `worktree_list` | `worktree list --porcelain` | |
| `worktree_add` | `worktree add [-b <branch>] [--no-checkout] <path> [<commitish>]` | via `WorktreeAdd` |
| `worktree_remove` | `worktree remove [--force] <path>` | via `WorktreeRemove` |
| `worktree_move` | `worktree move <from> <to>` | |
| `worktree_prune` | `worktree prune` | |
| `clone_repo` | `clone <url> <dest>` + flags | via `CloneSpec`; dirless, absolute `dest` |
| `tag_create` | `tag <name> [<rev>]` | lightweight |
| `tag_create_annotated` | `tag -a <name> -m <message> [<rev>]` | via `AnnotatedTag` |
| `tag_list` | `tag --list` | |
| `tag_delete` | `tag -d <name>` | |
| `show_file` | `show <rev>:<path>` | lossy decode, verbatim bytes |
| `config_get` | `config --get <key>` | `None` when unset; multi-valued key errors |
| `config_set` | `config -- <key> <value>` | trusted-input sink — see the trait doc comment |
| `remote_add` | `remote add <name> <url>` | |
| `remote_set_url` | `remote set-url <name> <url>` | |
| `blame` | `blame --line-porcelain [<rev>] -- <path>` | |

### Diff

| Method | Runs | Notes |
|---|---|---|
| `diff` | layered on `diff_text` | parsed `Vec<FileDiff>` |
| `diff_text` | `diff <spec> --no-color --no-ext-diff -M` | verbatim, incl. trailing blank context |
| `diff_is_empty` | `diff --quiet` | tracked files only |
| `diff_range_is_empty` | `diff --quiet <range>` | |
| `diff_stat` | `diff --shortstat <range>` | |

### Fetch, push, merge, rebase, sequencer, stash

| Method | Runs | Notes |
|---|---|---|
| `fetch` | `fetch --quiet` | prompt-off, retried 3× |
| `fetch_from` | `fetch --quiet <remote>` | same retry |
| `fetch_branch` | `fetch --quiet origin refs/heads/<b>:refs/remotes/origin/<b>` | same retry |
| `push` | `push [-u] <remote> <refspec>` | via `GitPush` |
| `merge_squash` | `merge --squash <branch>` | |
| `merge_commit` | `merge [--no-ff] [-m <msg> \| --no-edit] <branch>` | via `MergeCommit` |
| `merge_no_commit` | `merge --no-commit [--squash \| --no-ff] <branch>` | via `MergeNoCommit`; dry-run pattern |
| `merge_abort` | `merge --abort` | |
| `merge_continue` | `commit --no-edit` | editor suppressed |
| `reset_merge` | `reset --merge` | squash-safe undo |
| `reset_hard` | `reset --hard <rev>` | destructive |
| `rebase` | `rebase <onto>` | editor suppressed |
| `rebase_abort` | `rebase --abort` | |
| `rebase_continue` | `rebase --continue` | editor suppressed |
| `rebase_skip` | `rebase --skip` | mainly the `apply` backend's "nothing to commit" stop |
| `am_abort` | `am --abort` | restores pre-`am` HEAD |
| `am_continue` | `am --continue` | editor suppressed; can stop again on the next patch |
| `cherry_pick` | `cherry-pick <rev>` | conflict via `is_merge_conflict` |
| `cherry_pick_abort` | `cherry-pick --abort` | |
| `cherry_pick_continue` | `cherry-pick --continue` | editor suppressed |
| `revert` | `revert --no-edit <rev>` | |
| `revert_abort` | `revert --abort` | |
| `revert_continue` | `revert --continue` | editor suppressed |
| `bisect_reset` | `bisect reset` | ends a bisect session; no `--continue` |
| `stash_push` | `stash push [--include-untracked]` | via `StashPush` |
| `stash_pop` | `stash pop` | |
| `stash_list` | `stash list -z --format=%gd%x1f%H%x1f%gs` | parsed `Vec<StashEntry>`, most-recent first |
| `stash_apply` | `stash apply stash@{<index>}` | applies without dropping |
| `stash_drop` | `stash drop stash@{<index>}` | drops without applying |
| `clean` | `clean -n\|-f [-d] [-x\|-X]` | via `Clean`; refused before spawning unless `dry_run`/`force` is set, `dry_run` wins if both are |

### Discovery & raw escape hatches

| Method | Runs |
|---|---|
| `version` | `--version` |
| `capabilities` | `--version`, parsed (`git ≥ 2.31` floor) |
| `run` | `git <args>` in the process cwd (client) or the bound `dir` (`GitAt`) |
| `run_raw` | like `run`, never errors on non-zero exit |

Inherent (not on `GitApi`, so not mockable, but present on `Git`/`GitAt`):
`run_args`/`run_raw_args` (`&[&str]`, skip the `Vec<String>` allocation),
`switch_with_stash` (composed: `stash push -u` → `checkout` → `stash pop`), and
`blocking::worktree_remove` for a `Drop` guard. See [Raw escape
hatches](../crates/git/docs/git.md#raw-escape-hatches).

### git — not modeled (examples) → escape hatch

`add -p`/interactive staging, `am`/`apply` (patch application other than the
in-progress-am probes above), `archive`, `bundle`, `describe`,
`difftool`/`mergetool`, `fsck`, `gc`, `grep`, `ls-files`/`ls-tree`,
`merge-base`, `mv`/`rm` (path staging goes through `add`), `notes`, `reflog`,
`replace`, `reset` (soft/mixed — only `--hard`/`--merge` are typed),
`send-email`, `shortlog`, `sparse-checkout`, `submodule`, `subtree`,
`verify-commit`/`verify-tag`. Reach any of these through `run`/`run_raw`.

## jj (`vcs-jj` — the `jj` binary)

Guide: [vcs-jj](../crates/jj/docs/jj.md). Trait: `JjApi`
(`crates/jj/src/lib.rs`).

### Status, log, describe, bookmarks

| Method | Runs | Notes |
|---|---|---|
| `status` | `diff -r @ --summary` | snapshots the WC first |
| `status_ignoring_working_copy` | adds `--ignore-working-copy` | read-only twin of `status` |
| `status_text` | `status` (human text) | |
| `log` | `log` | up to `max`, newest first |
| `log_paths` | `log -r <revset> <filesets>` | non-empty filesets required |
| `current_change` | `log -r @` | reduced to one `Change` |
| `current_bookmark` | `log -r @ --no-graph --limit 1 -T <bookmarks-template>` | local bookmark on `@`, if exactly one; `None` when no bookmark |
| `trunk` | `log -r trunk() --no-graph --limit 1 -T <bookmarks-template>` | trunk bookmark; `None` when unresolved |
| `describe` | `describe -m` | on `@` |
| `describe_rev` | `describe -r <revset> -m` | arbitrary revision |
| `new_change` | `new -m` | on top of the WC |
| `new_child` | `new <parent>` | undescribed child |
| `bookmarks` | `bookmark list` | snapshots the WC first |
| `bookmarks_ignoring_working_copy` | adds `--ignore-working-copy` | read-only twin |
| `bookmarks_all` | `bookmark list -a` | local + remote-tracking |
| `reachable_bookmarks` | `log -r 'heads(::@ & bookmarks())'` | snapshots the WC first |
| `reachable_bookmarks_ignoring_working_copy` | adds `--ignore-working-copy` | read-only twin |
| `bookmark_track` | `bookmark track <name>@<remote>` | |
| `bookmark_set` | `bookmark set <name> -r <revision>` | |
| `bookmark_create` | `bookmark create <name> -r <rev>` | |
| `bookmark_rename` | `bookmark rename <old> <new>` | |
| `bookmark_delete` | `bookmark delete <name>` | |
| `bookmark_move` | `bookmark move <name> --to <rev> [--allow-backwards]` | via `BookmarkMove` |

### Diff, query, conflicts, files

| Method | Runs | Notes |
|---|---|---|
| `diff` | layered on `diff_text` | parsed `Vec<FileDiff>` |
| `diff_text` | `diff -r <spec> --git` | verbatim |
| `diff_summary` | `diff -r <from>..<to> --summary` | per-file |
| `diff_stat` | `diff -r <revset> --stat` | |
| `commit_count` | `log -r <revset> --no-graph` | one id per line |
| `is_conflicted` | template query on the revset | |
| `has_workingcopy_conflict` | `is_conflicted(dir, "@")` | |
| `resolve_list` | `resolve --list -r <revset>` | lossless paths |
| `template_query` | `log -r <revset> --no-graph [--limit n] -T <template>` | snapshots the WC first |
| `template_query_ignoring_working_copy` | adds `--ignore-working-copy` | read-only twin |
| `description` | (template query) | trimmed, newest commit of a multi-commit revset |
| `evolog` | `evolog -r <revset>` | newest predecessor first |
| `file_annotate` | `file annotate <path> [-r <revset>]` | plain path, not a fileset |
| `file_show` | `file show -r <revset> root-file:"<path>"` | lossy decode, verbatim bytes |

### Rebase, squash/split, merging, sparse

| Method | Runs | Notes |
|---|---|---|
| `rebase` | `rebase -d <onto>` (jj's default `-b @`) | whole descendant closure — not git's `rebase` semantics |
| `rebase_branch` | `rebase -b <branch> -d <dest>` | explicit branch |
| `edit` | `edit <rev>` | moves the WC |
| `squash_into` | `squash --into <rev> [--use-destination-message]` | via `SquashInto` |
| `commit_paths` | `commit -m <message> <filesets>` | non-empty filesets required |
| `squash_paths` | `squash --from <from> --into <into> [--use-destination-message] <filesets>` | via `SquashPaths` |
| `split_paths` | `split -m <message> <filesets>` | non-empty filesets required (else hangs on the interactive editor) |
| `absorb` | `absorb [--from <revset>] [<filesets>]` | empty filesets absorbs everything |
| `sparse_set` | `sparse set --clear --add <p>…` | empty list clears the WC |
| `new_merge` | `new -m <msg> <p1> <p2> …` | multiple parents |
| `duplicate` | `duplicate <revset>` | |
| `abandon` | `abandon <revset>` | |

### Git integration, workspaces, operation log

| Method | Runs | Notes |
|---|---|---|
| `git_fetch` | `git fetch` | retried 3× |
| `git_fetch_from` | `git fetch --remote <remote>` | same retry |
| `git_fetch_branch` | `git fetch --remote origin -b <branch>` | same retry |
| `git_push` | `git push [-b <bookmark>]` | |
| `git_import` | `git import` | colocated-repo sync |
| `git_clone` | `git clone <url> <dest> --colocate\|--no-colocate` | via `GitClone`; dirless, absolute `dest` |
| `remote_add` | `git remote add <name> <url>` | flag-injection-guarded positionals |
| `remote_list` | `git remote list` | parsed `Vec<Remote>`; no template/JSON form, pinned display-format parser |
| `remote_remove` | `git remote remove <name>` | also forgets the remote's bookmarks |
| `remote_rename` | `git remote rename <old> <new>` | |
| `remote_set_url` | `git remote set-url <name> <url>` | errors if `name` doesn't exist |
| `workspace_list` | `workspace list` | |
| `workspace_root` | `workspace root [--name <name>]` | |
| `workspace_add` | `workspace add --name <name> -r <base> <path>` | via `WorkspaceAdd` |
| `workspace_forget` | `workspace forget <name>` | |
| `op_head` | `op log --no-graph --limit 1` | capture before a risky sequence |
| `op_log` | `op log --no-graph --limit n` | newest first |
| `op_restore` | `op restore <id>` | |
| `op_undo` | `op undo` | |

### Discovery & raw escape hatches

| Method | Runs |
|---|---|
| `root` | `root` |
| `version` | `--version` |
| `capabilities` | `--version`, parsed (`jj ≥ 0.38` floor) |
| `run` | `jj <args>` in the process cwd (client) or the bound `dir` (`JjAt`); **unguarded** |
| `run_raw` | like `run`, never errors on non-zero exit; **unguarded** |

Inherent (not on `JjApi`): `run_args`/`run_raw_args` (`&[&str]`), and
`transaction(dir, f)` — op-log-rollback wrapper around capture (`op_head`) +
run + rollback (`op restore`) on `Err`. See [`transaction` — op-log
rollback](../crates/jj/docs/jj.md#transaction--op-log-rollback) and [Raw escape
hatches](../crates/jj/docs/jj.md#raw-escape-hatches).

### jj — not modeled (examples) → escape hatch

`backout`, `bookmark forget` (only `delete` is typed), `config` (`list`/`get`/
`set`/`edit`), `debug`, `file chmod`/`file track`/`file untrack`, `fix`,
`git init`, `interdiff`, `next`/`prev`, `resolve` (interactive; only `resolve
--list` via `resolve_list`), `simplify-parents`, `util`. Reach any of these
through `run`/`run_raw` — note the trait doc comment's warning that
`run`/`run_raw` are **unguarded**: jj's `--config`/`--config-toml` and
user-defined aliases can reach code execution, so never forward untrusted
argv there.

## gh (`vcs-github` — the GitHub CLI)

Guide: [vcs-github](../crates/github/docs/github.md). Trait: `GitHubApi`
(`crates/github/src/lib.rs`).

| Method | Runs | Notes |
|---|---|---|
| `auth_status` | `auth status` | exit code only; unscoped across hosts |
| `auth_status_for` | `auth status --hostname <host>` | scoped to a `GitHubHost` |
| `repo_view` | `repo view --json …` | |
| `api` | `api <endpoint>` | raw REST/GraphQL body; flag-guarded endpoint |
| `pr_list` | `pr list --limit 100 --json …` | open PRs, ≤100 |
| `pr_list_for_branch` | `pr list --head <head> --base <base> --state all --limit 100 --json …` | any state |
| `pr_view` | `pr view <n> --json …` | |
| `pr_create` | `pr create` | via `PrCreate`; returns URL |
| `pr_merge` | `pr merge <n> --merge\|--squash\|--rebase [--auto] [--delete-branch]` | via `PrMerge` |
| `pr_mark_ready` | `pr ready <n>` | |
| `pr_close` | `pr close <n> [--delete-branch]` | via `PrClose` |
| `pr_checkout` | `pr checkout <n>` | mutates the working copy |
| `pr_checks` | `pr checks <n> --json …` | branch on `CheckRun::bucket` |
| `pr_review` | `pr review <n> --approve\|--request-changes\|--comment [--body <body>]` | via `ReviewAction` |
| `pr_comment` | `pr comment <n> --body <body>` | returns comment URL |
| `pr_edit` | `pr edit <n> [--title <title>] [--body <body>]` | via `PrEdit`; ≥1 field required |
| `pr_feedback` | `pr view <n> --json reviews,comments` | |
| `pr_diff` | `pr diff <n> --color never` | parsed `Vec<FileDiff>` |
| `issue_list` | `issue list --limit 100 --json …` | ≤100 |
| `issue_view` | `issue view <n> --json …` | |
| `issue_create` | `issue create --title <t> --body <b>` | returns issue URL |
| `issue_close` | `issue close <n>` | |
| `issue_reopen` | `issue reopen <n>` | |
| `issue_comment` | `issue comment <n> --body <body>` | returns comment URL |
| `run_list` | `run list --limit <n> [--branch <b>] --json …` | Actions runs |
| `run_view` | `run view <id> --json …` | id is `WorkflowRun::database_id` |
| `run_watch` | `run watch <id>`, then `run view <id>` | **blocks** until the run finishes |
| `release_list` | `release list --limit 100 --json …` | `body`/`url` not fetched |
| `release_view` | `release view <tag> --json …` | fills `body`/`url` |
| `release_create` | `release create <tag> [--title] [--notes] [--draft] [--prerelease]` | via `ReleaseCreate`; returns URL |
| `release_delete` | `release delete <tag> --yes` | release only, not the git tag |
| `version` | `--version` | |
| `capabilities` | `--version`, parsed (`gh ≥ 2.0` floor) | |
| `run` | `gh <args>` in the process cwd (client) or the bound `dir` (`GitHubAt`) | |
| `run_raw` | like `run`, never errors on non-zero exit | |

Inherent (not on `GitHubApi`): `run_args`/`run_raw_args` (`&[&str]`). See [Raw
escape hatches](../crates/github/docs/github.md#raw-escape-hatches).

### gh — not modeled (examples) → escape hatch

`browse`, `cache`, `codespace`, `extension`, `gist`, `label`, `org`, `project`,
`pr lock`/`reopen`/`status`, `repo clone`/`create`/`fork`/`edit`/`sync`/`list`,
`ruleset`, `search`, `secret`, `ssh-key`, `variable`, `workflow` (`list`/`view`/
`run`/`enable`/`disable`). Reach any of these through `run`/`run_raw`, or
`api` for a raw REST/GraphQL call.

## glab (`vcs-gitlab` — the GitLab CLI)

Guide: [vcs-gitlab](../crates/gitlab/docs/gitlab.md). Trait: `GitLabApi`
(`crates/gitlab/src/lib.rs`). The surface is **deliberately lean** — auth,
project view, and the MR lifecycle — mirroring `vcs-github`'s shape, not its
breadth.

| Method | Runs | Notes |
|---|---|---|
| `auth_status` | `auth status` | exit code only; see the glab#911 caveat in the guide |
| `repo_view` | `repo view --output json` | |
| `api` | `api <endpoint>` | raw REST/GraphQL body; flag-guarded endpoint |
| `mr_list` | `mr list --per-page 100 --output json` | ≤100 |
| `mr_view` | `mr view <number> --output json` | `number` is GitLab's `iid` |
| `mr_create` | `mr create --title … --description … [--source-branch …] [--target-branch …] --yes` | via `MrCreate`; returns URL |
| `mr_merge` | `mr merge <id> --yes --auto-merge=false [--squash\|--rebase]` | via `MrMerge` |
| `mr_mark_ready` | `mr update <id> --ready` | |
| `mr_close` | `mr close <id>` | |
| `mr_checkout` | `mr checkout <id>` | mutates the working copy |
| `mr_comment` | `mr note <id> -m <message>` | returns command output |
| `mr_edit` | `mr update <id> [--title <title>] [--description <body>] --yes` | via `MrEdit`; ≥1 field required |
| `mr_approve` | `mr approve <id>` | GitLab's approve/revoke review model (no "request changes") |
| `mr_revoke` | `mr revoke <id>` | withdraws an approval |
| `mr_checks` | `mr view <id> --output json` (reads `head_pipeline.status`) | bucketed `CiStatus` |
| `mr_diff` | `mr diff <id> --color never` | parsed `Vec<FileDiff>` |
| `issue_list` | `issue list --per-page 100 --output json` | ≤100 |
| `issue_view` | `issue view <number> --output json` | |
| `issue_create` | `issue create --title … --description … --yes` | returns issue URL |
| `issue_close` | `issue close <id>` | |
| `issue_reopen` | `issue reopen <id>` | |
| `issue_comment` | `issue note <id> -m <body>` | returns command output; dash-sentinel-guarded body |
| `release_list` | `release list --per-page 100 --output json` | ≤100 |
| `release_view` | `release view <tag> --output json` | |
| `release_create` | `release create <tag> [--name …] [--notes …]` | via `ReleaseCreate`; no draft/prerelease (`Unsupported`) |
| `release_delete` | `release delete <tag> --yes` | release only, not the git tag |
| `version` | `--version` | |
| `capabilities` | `--version`, parsed | |
| `run` | `glab <args>` in the process cwd (client) or the bound `dir` (`GitLabAt`) | |
| `run_raw` | like `run`, never errors on non-zero exit | |

Inherent (not on `GitLabApi`): `run_args`/`run_raw_args` (`&[&str]`). See
[Escape hatch](../crates/gitlab/docs/gitlab.md#escape-hatch).

### glab — not modeled (examples) → escape hatch

`alias`, `ci` (`status`/`view`/`trace`/`run`/`lint`), `incident`, `label`,
`mr rebase`/`subscribe`/`todo`, `release upload`, `repo archive`/`clone`/
`create`/`fork`/`mirror`/`transfer`, `schedule`, `snippet`, `ssh-key`, `token`,
`user`, `variable`, `webhook`. Reach any of these through `run`/`run_raw`, or
`api` for a raw REST/GraphQL call.

## tea (`vcs-gitea` — the Gitea/Forgejo CLI)

Guide: [vcs-gitea](../crates/gitea/docs/gitea.md). Trait: `GiteaApi`
(`crates/gitea/src/lib.rs`). The **narrowest** of the three forge wrappers —
`tea` itself has no single-PR `view`, no current-repo view, no draft toggle, no
PR-checks command, and no single-release view; see [What `tea` does **not**
do](../crates/gitea/docs/gitea.md#what-tea-does-not-do).

| Method | Runs | Notes |
|---|---|---|
| `auth_status` | `login list --output json`, non-empty | `tea` has no per-instance auth status |
| `pr_list` | `pr list --output json` | ≤~50 (Gitea server page cap) |
| `pr_view` | `pr list --state all` (paged) + filter | synthesized — `tea` has no single-PR view |
| `pr_create` | `pr create --title … --description … [--head …] [--base …]` | via `PrCreate`; returns tea's text output, **not** a URL |
| `pr_merge` | `pr merge <number> --style merge\|rebase\|squash` | via `PrMerge`; no `auto`/`delete_branch` (`Unsupported`) |
| `pr_close` | `pr close <number>` | |
| `pr_checkout` | `pr checkout <number>` | mutates the working copy |
| `pr_comment` | `comment <number> <body>` | shared with issues; flag-guarded body |
| `pr_edit` | `pr edit <number> [--title …] [--description …]` | via `PrEdit`; ≥1 field required |
| `pr_approve` | `pr approve <number>` | |
| `pr_reject` | `pr reject <number> <reason>` | required reason; flag-guarded |
| `issue_list` | `issues list --output json` | ≤~50 |
| `issue_view` | `issues <number> --output json` | first-class single-issue view (unlike `pr_view`) |
| `issue_create` | `issues create --title … --description …` | returns text output |
| `issue_close` | `issues close <index>` | |
| `issue_reopen` | `issues reopen <index>` | |
| `issue_comment` | `comment <index> <body>` | shared with PRs; flag-guarded body |
| `release_list` | `releases list --output json` | ≤~50 |
| `release_create` | `releases create --tag <tag> [--title …] [--note …] [--draft] [--prerelease]` | via `ReleaseCreate` |
| `release_delete` | `releases delete <tag>` | flag-guarded tag |
| `version` | `--version` | |
| `capabilities` | `--version`, parsed (`tea ≥ 0.9` floor) | |
| `run` | `tea <args>` in the process cwd (client) or the bound `dir` (`GiteaAt`) | |
| `run_raw` | like `run`, never errors on non-zero exit | |

There is intentionally **no** `repo_view`, `pr_mark_ready`, `pr_checks`, or
`release_view` on `GiteaApi` — `tea` has no equivalent command; the
[`vcs-forge`](../crates/forge/docs/forge.md) facade reports these
`Error::Unsupported` for the Gitea backend. Inherent (not on `GiteaApi`):
`run_args`/`run_raw_args` (`&[&str]`). See [Escape
hatch](../crates/gitea/docs/gitea.md#escape-hatch).

### tea — not modeled (examples) → escape hatch

`admin`, `issues comment`/`labels`, `label`, `login add`/`edit`/`delete`
(only `login list`, internally, via `auth_status`), `milestone`,
`notification`, `organization`, `releases assets`, `repos create`/`list`/
`delete`, `times`, `whoami`. Reach any of these through `run`/`run_raw` —
e.g. flipping a Gitea draft (a `WIP:` title prefix) via `pr edit`.

## Facade escape-hatch routers

`vcs-core`'s `Repo` and `vcs-forge`'s `Forge` cover only the **portable
intersection** across backends/forges; both re-export the wrapper crates so
dropping to a wrapper-level method (any row above) never needs an extra
dependency:

- **`vcs-core`** — `Repo::git()` / `Repo::jj()` (the raw client, still
  `dir`-taking) and `Repo::git_at()` / `Repo::jj_at()` (the cwd-bound view,
  `None` for the other backend). See [Escape hatches to the underlying
  client](../crates/core/docs/core.md#escape-hatches-to-the-underlying-client).
- **`vcs-forge`** — the wrapper client directly (`GitHub::new().run_list(dir)…`),
  or the wrapper's `api`/`run` for anything beyond that. See [When to drop to
  the wrapped client (the escape
  hatch)](../crates/forge/docs/forge.md#when-to-drop-to-the-wrapped-client-the-escape-hatch).

A facade operation marked `Unsupported` on a given backend (e.g. a Gitea
release-by-tag view) has **no** wrapper method to drop to either — the CLI
itself can't do it; go through the forge's REST API (`api`) or your own HTTP
client, as the forge table above notes.

## Keeping this index current

A new typed method changes what a row in this index should say. When adding
one (see [Extending vcs-toolkit-rs, step
1](extending.md#1-adding-a-typed-method-to-a-cli-wrapper)), add or update the
row in the matching wrapper's table above — and drop it from that wrapper's
"not modeled" list if it was mentioned there.

## See also

- [Documentation guide map](README.md) — the full guide set this index cross-references.
- [Extending vcs-toolkit-rs](extending.md) — the contributor workflow this index's upkeep step belongs to.
- [vcs-core](../crates/core/docs/core.md) / [vcs-forge](../crates/forge/docs/forge.md) — the facade escape hatches this index links to.
