# CLI command coverage index

The per-crate guides document the typed surface **from the method outward**
("here's `pr_merge`, here's what it runs"). This page inverts that: **from the
CLI command inward** — "I know `git rebase --onto` / `jj parallelize` / `gh
api`; is it covered by a typed method, or do I need the escape hatch?" Each
table row is one typed method and the exact subcommand/flags it runs, sourced
from the crate's trait definition (`GitApi`/`JjApi`/`GitHubApi`/`GitLabApi`/
`GiteaApi` in `crates/*/src/lib.rs`) or, when labelled `Forge`, the portable
`Forge` facade. The public methods are literally enumerated against these
tables so a method the prose guide hasn't caught up to yet still shows up here.

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
| `remote_branch_revision` | `ls-remote <remote> refs/heads/<name>` | exact advertised object id or absence; failures stay errors |
| `remote_head_branch` | `symbolic-ref refs/remotes/origin/HEAD` | `None` when unset |
| `remote_url` | `remote get-url <remote>` | |
| `remote_list` | `remote -v` | parsed `Vec<Remote>`; one fetch-URL row per remote |
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
| `sparse_checkout_set` | `sparse-checkout set --cone\|--no-cone -- <values>` | via `SparseCheckoutSet`; cone by default, non-cone explicit |
| `sparse_checkout_list` | `sparse-checkout list` | parsed directories/patterns in git order |
| `sparse_checkout_disable` | `sparse-checkout disable` | repopulates the worktree |
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
| `diff_between` | layered on `diff_text_between` | explicit `from` → `to` trees |
| `diff_text_between` | `diff <from> <to> --no-color --no-ext-diff -M --src-prefix=a/ --dst-prefix=b/ --` | independently validated endpoints |
| `diff_is_empty` | `diff --quiet` | tracked files only |
| `diff_range_is_empty` | `diff --quiet <range>` | |
| `diff_stat` | `diff --shortstat <range>` | |

### Fetch, push, merge, rebase, sequencer, stash

On a **hardened** client (`Git::hardened()`) every network verb below — plus
`remote_branches`/`remote_branch_exists` (`ls-remote`) and `submodule_update` —
first spawns `config --get core.sshCommand` and, when that returns a value,
`config --global --get core.sshCommand`, refusing the operation if the two differ
(the second read is cached per client, and both are skipped by the
`with_ssh_command` / `trust_repo_ssh_command` opt-ins). `clone_repo` is exempt. A
plain `Git::new()` client spawns neither. See [Security &
hardening](https://docs.rs/vcs-git/latest/vcs_git/guide/security/).

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
| `stash_list` | `stash list -z --format=%gd%x0a%H%x0a%gs` | parsed `Vec<StashEntry>`, most-recent first |
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
`send-email`, `shortlog`, `submodule`, `subtree`,
`verify-commit`/`verify-tag`. Reach any of these through `run`/`run_raw`.

The sparse-checkout rows above are deliberately Git-only: `vcs-jj` keeps its
workspace-centric `sparse_set` surface, and `vcs-core` does not flatten either
backend's sparse model into a facade operation.

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
| `bookmark_forget` | `bookmark forget <name>` | inverse of `bookmark_track`; local only |
| `bookmark_untrack` | `bookmark untrack <name> --remote <remote>` | inverse of `bookmark_track`; non-deprecated `--remote` flag |
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
| `diff_between` | layered on `diff_text_between` | explicit `from` → `to` trees |
| `diff_text_between` | `diff --from <from> --to <to> --git` | separate endpoint flags; jj ≥ 0.38 |
| `diff_summary` | `diff -r <from>..<to> --summary` | per-file |
| `diff_stat` | `diff -r <revset> --stat` | |
| `commit_count` | `log -r <revset> --no-graph` | one id per line |
| `is_conflicted` | template query on the revset | |
| `has_workingcopy_conflict` | `is_conflicted(dir, "@")` | |
| `resolve_list` | `file list -r <revset> -T 'if(conflict, path ++ "\0")'` | lossless paths |
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
| `revert` | `revert -r <revset> --onto @` | undo-by-new-change; no `JjCapabilities` gate (`revert` is the only verb `≥ 0.38`) |

### Git integration, workspaces, operation log

| Method | Runs | Notes |
|---|---|---|
| `git_fetch` | `git fetch` | retried 3× |
| `git_fetch_from` | `git fetch --remote <remote>` | same retry |
| `git_fetch_branch` | `git fetch --remote origin -b <branch>` | same retry |
| `git_push` | `git push [-b <bookmark>]` | |
| `git_import` | `git import` | colocated-repo sync |
| `git_clone` | `git clone <url> <dest> --colocate\|--no-colocate` | via `GitClone`; dirless, absolute `dest` |
| `config_get` | `config get <key>` | `None` when unset (exit 1); other non-zero exit errors |
| `config_set` | `config set --repo -- <key> <value>` | trusted-input sink — see the trait doc comment |
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
| `op_log` | `op log --no-graph --limit n --at-op=@ --ignore-working-copy` | newest first; no snapshot or op-head reconciliation |
| `op_restore` | `op restore <id>` | |
| `op_undo` | `undo` | top-level form works across the supported jj 0.38+ range |

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

`config` (`list`/`edit`; only `get`/`set` are typed), `debug`, `file chmod`/
`file track`/`file untrack`, `fix`, `git init`, `interdiff`, `next`/`prev`,
`resolve` (interactive; `resolve_list` reads conflicted paths via `file list`
with a conflict template instead),
`simplify-parents`, `util`. (`backout` was jj's older, since-removed name for
`revert`, which is now typed — see [`revert`](#rebase-squashsplit-merging-sparse)
above.) Reach any of these through `run`/`run_raw` — note the trait doc
comment's warning that `run`/`run_raw` are **unguarded**: jj's `--config`/
`--config-toml` and user-defined aliases can reach code execution, so never
forward untrusted argv there.

## gh (`vcs-github` — the GitHub CLI)

Guide: [vcs-github](../crates/github/docs/github.md). Trait: `GitHubApi`
(`crates/github/src/lib.rs`).

| Method | Runs | Notes |
|---|---|---|
| `auth_status` | `auth status` | exit code only; unscoped across hosts |
| `auth_status_for` | `auth status --hostname <host>` | scoped to a `GitHubHost` |
| `auth_info` | `auth status` | same run, read as text: which account gh acts as + every login; unrecognised output degrades to "unknown", never an error |
| `repo_visible` | `repo view --json name` | exit code only: is this repo visible to the active account |
| `Forge::auth_info` | `auth status`, then `repo view --json name` **only when a session exists** | active account + every login + repo visibility; no second spawn when unauthenticated |
| `repo_view` | `repo view --json …` | |
| `api` | `api <endpoint>` | raw REST/GraphQL body; flag-guarded endpoint |
| `pr_list` | `pr list --state open --limit 100 --json …` | compatibility default: open PRs, ≤100; includes exact `headRefOid` and `isCrossRepository` |
| `pr_list_with` | `pr list --state open\|closed\|merged\|all --limit <n> --json …` | via `PrList`; zero rejected before spawn; includes exact head/repository identity |
| `pr_list_for_source_branch` | `pr list --head <source_branch> --state all --limit 100 --json …` | any state; source branch only; includes exact head/repository identity |
| `pr_list_for_branch` | `pr list --head <head> --base <base> --state all --limit 100 --json …` | any state; includes exact head/repository identity |
| `Forge::pr_for_branch` | `pr list --head <source_branch> --state all --limit 100 --json …` | any state; independent of target |
| `pr_view` | `pr view <n> --json …` | includes exact `headRefOid` and `isCrossRepository` |
| `pr_create` | `pr create … [--label <name> …]` | via `PrCreate`; returns URL |
| `pr_add_labels` / `pr_remove_labels` | `pr edit <n> --add-label\|--remove-label <name> …` | repeated flag-value pairs; empty sets rejected |
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
| `issue_list` | `issue list --state open --limit 100 --json …` | compatibility default: open issues, ≤100 |
| `issue_list_with` | `issue list --state open\|closed\|all --limit <n> --json …` | via `IssueList`; zero rejected before spawn |
| `issue_view` | `issue view <n> --json …` | |
| `issue_create` / `issue_create_with` | `issue create --title <t> --body <b> [--label <name> …]` | compatibility strings or extensible `IssueCreate`; returns issue URL |
| `issue_add_labels` / `issue_remove_labels` | `issue edit <n> --add-label\|--remove-label <name> …` | repeated flag-value pairs; empty sets rejected |
| `issue_close` | `issue close <n>` | |
| `issue_reopen` | `issue reopen <n>` | |
| `issue_comment` | `issue comment <n> --body <body>` | returns comment URL |
| `workflow_list` | `workflow list --limit 50 --json id,name,path,state` | active workflows |
| `workflow_list_with` | `workflow list --limit <n> [--all] --json id,name,path,state` | via `WorkflowList`; zero rejected before spawn |
| `workflow_view` | `workflow list --limit 2147483647 --all --json id,name,path,state` | resolves id/name/filename/path; current `workflow view` has no JSON mode, so no human output is scraped |
| `run_list` | `run list --limit <n> [--branch <b>] --json …` | Actions runs include exact `headSha` |
| `run_view` | `run view <id> --json …` | id is `WorkflowRun::database_id`; includes exact `headSha` |
| `run_watch` | `run watch <id>`, then `run view <id>` | **blocks** until the run finishes |
| `workflow_dispatch` | `workflow run <workflow> [--ref <ref>] [--raw-field key=value …]` | via `WorkflowDispatch`; returns `()` (dispatch is async, 204) |
| `run_rerun` | `run rerun <id> [--failed]` | via `RerunScope::{All,FailedOnly}` |
| `run_cancel` | `run cancel <id>` | requests cancellation of an in-progress run |
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
`ruleset`, `search`, `secret`, `ssh-key`, `variable`, `workflow enable`/`disable`
(`workflow list`/`view` and `workflow run` are modeled). Reach any
of these through `run`/`run_raw`, or `api` for a raw REST/GraphQL call.

## glab (`vcs-gitlab` — the GitLab CLI)

Guide: [vcs-gitlab](../crates/gitlab/docs/gitlab.md). Trait: `GitLabApi`
(`crates/gitlab/src/lib.rs`). The surface is **deliberately lean** — auth,
project view, and the MR lifecycle — mirroring `vcs-github`'s shape, not its
breadth.

| Method | Runs | Notes |
|---|---|---|
| `auth_status` | `auth status` | exit code only; see the glab#911 caveat in the guide |
| `Forge::auth_info` | — | no account report modelled for `glab`: reports "unknown", spawning nothing |
| `repo_view` | `repo view --output json` | |
| `api` | `api <endpoint>` | raw REST/GraphQL body; flag-guarded endpoint |
| `mr_list` | `mr list --per-page 100 --output json` | compatibility default: open MRs, ≤100 |
| `mr_list_with` | `mr list [--closed\|--merged\|--all] --per-page <n> --output json` | via `MrList`; no state flag means open; zero rejected before spawn |
| `mr_list_for_source_branch` | `mr list --source-branch <source_branch> --all --per-page 100 --output json` | any state; source branch only |
| `Forge::pr_for_branch` | `mr list --source-branch <source_branch> --all --per-page 100 --output json` | any state; independent of target |
| `mr_view` | `mr view <number> --output json` | `number` is GitLab's `iid` |
| `mr_create` | `mr create --title … --description … [--source-branch …] [--target-branch …] --yes` | via `MrCreate`; returns URL |
| `mr_merge` | `mr merge <id> --yes --auto-merge=false [--squash\|--rebase]` | via `MrMerge` |
| `mr_mark_ready` | `mr update <id> --ready` | |
| `mr_close` | `mr close <id>` | |
| `mr_checkout` | `mr checkout <id>` | mutates the working copy |
| `mr_comment` | `mr note <id> -m <message>` | returns command output |
| `mr_edit` | `mr update <id> [--title <title>] [--description <body>] --yes` | via `MrEdit`; ≥1 field required |
| `mr_add_labels` / `mr_remove_labels` | `mr update <id> --label\|--unlabel <name> … --yes` | repeated flag-value pairs; empty sets rejected |
| `mr_approve` | `mr approve <id>` | GitLab's approve/revoke review model (no "request changes") |
| `mr_revoke` | `mr revoke <id>` | withdraws an approval |
| `mr_checks` | `mr view <id> --output json` (reads `head_pipeline.status`) | bucketed `CiStatus` |
| `mr_diff` | `mr diff <id> --color never` | parsed `Vec<FileDiff>` |
| `issue_list` | `issue list --per-page 100 --output json` | compatibility default: open issues, ≤100 |
| `issue_list_with` | `issue list [--closed\|--all] --per-page <n> --output json` | via `IssueList`; no state flag means open; zero rejected before spawn |
| `issue_view` | `issue view <number> --output json` | |
| `issue_create` / `issue_create_with` | `issue create --title … --description … [--label <name> …] --yes` | compatibility strings or extensible `IssueCreate`; returns issue URL |
| `issue_add_labels` / `issue_remove_labels` | `issue update <id> --label\|--unlabel <name> …` | repeated flag-value pairs; empty sets rejected |
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
| `auth_status` | `login list --output csv`, non-empty | `tea` has no per-instance auth status |
| `Forge::auth_info` | — | no account report modelled for `tea`: reports "unknown", spawning nothing |
| `pr_list` | `pr list --state open --limit 100 --fields … --output csv` | compatibility default; ≤~50 (Gitea server page cap) |
| `pr_list_with` | `pr list --state open\|closed\|all --limit <n> --fields … --output csv` | `merged` is `Unsupported` before spawn |
| `Forge::pr_for_branch` | Unsupported | `tea` has no source-branch filter |
| `pr_view` | `pr list --state all --page <n> --output csv` (paged) + filter | synthesized — `tea` has no single-PR view |
| `pr_create` | `pr create --title … --description … [--head …] [--base …] [--labels a,b]` | via `PrCreate`; returns tea's text output, **not** a URL |
| `pr_add_labels` / `pr_remove_labels` | **Unsupported** | `tea 0.9.2` has no PR edit command; creation labels remain supported. |
| `pr_merge` | `pr merge <number> --style merge\|rebase\|squash` | via `PrMerge`; no `auto`/`delete_branch` (`Unsupported`) |
| `pr_close` | `pr close <number>` | |
| `pr_checkout` | `pr checkout <number>` | mutates the working copy |
| `pr_comment` | `comment <number> <body>` | shared with issues; flag-guarded body |
| `pr_edit` | **Unsupported** (`tea` has no `pr edit` subcommand) | Use the Gitea REST API to edit title or description. |
| `pr_approve` | `pr approve <number>` | |
| `pr_reject` | `pr reject <number> <reason>` | required reason; flag-guarded |
| `issue_list` | `issues list --state open --limit 100 --fields … --output csv` | compatibility default; ≤~50 |
| `issue_list_with` | `issues list --state open\|closed\|all --limit <n> --fields … --output csv` | zero rejected before spawn |
| `issue_view` | `issues list --state all --page <n> --output csv` (paged) + filter | synthesized because the bare-index view is Markdown |
| `issue_create` / `issue_create_with` | `issues create --title … --description … [--labels a,b]` | compatibility strings or extensible `IssueCreate`; returns text output |
| `issue_add_labels` / `issue_remove_labels` | **Unsupported** | `tea 0.9.2` has no issue edit command; creation labels remain supported. |
| `issue_close` | `issues close <index>` | |
| `issue_reopen` | `issues reopen <index>` | |
| `issue_comment` | `comment <index> <body>` | shared with PRs; flag-guarded body |
| `release_list` | `releases list --limit 100 --output csv` | ≤~50 |
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
`delete`, `times`, `whoami`. Reach any of these through `run`/`run_raw`; editing
a Gitea PR title/description (including its `WIP:` draft prefix) instead requires
the Gitea REST API because `tea` has no `pr edit` subcommand.

## Facade escape-hatch routers

`vcs-core`'s `Repo` and `vcs-forge`'s `Forge` cover only the **portable
intersection** across backends/forges; both re-export the wrapper crates so
dropping to a wrapper-level method (any row above) never needs an extra
dependency:

- **`vcs-core`** — `Repo::git()` / `Repo::jj()` (the raw client, still
  `dir`-taking) and `Repo::git_at()` / `Repo::jj_at()` (the cwd-bound view,
  `None` for the other backend). Its portable `Repo::remotes()` wraps either
  `GitApi::remote_list` or `JjApi::remote_list` and returns a facade-owned
  name/URL DTO. Its portable `Repo::clone(backend, url, dest, spec)` — the one
  associated constructor, since there is no handle yet — wraps either
  `GitApi::clone_repo` or `JjApi::git_clone` under a unified `CloneSpec` (git's
  `branch`/`depth`/`bare`, jj's `colocate`), structurally rejecting a
  cross-backend option with `Error::Unsupported`. Its portable
  `Repo::mark_resolved(paths)` — the finishing half of a programmatic conflict
  resolution, after the markers have been rewritten in the working copy — wraps
  `GitApi::add` on git (staging is what clears an unmerged `UU` index entry) and
  is a deliberate **no-op** on jj, which has no index: the working-copy content
  *is* the resolution, recorded by the next snapshotting `jj` command. See [Escape
  hatches to the underlying
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
