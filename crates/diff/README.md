# vcs-diff

[![crates.io](https://img.shields.io/crates/v/vcs-diff.svg)](https://crates.io/crates/vcs-diff) [![docs.rs](https://img.shields.io/docsrs/vcs-diff)](https://docs.rs/vcs-diff) [![downloads](https://img.shields.io/crates/d/vcs-diff.svg)](https://crates.io/crates/vcs-diff)

Shared **git-format unified-diff** model and parser for the
[vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

`git diff` and `jj diff --git` emit byte-identical output, so `vcs-git` and
`vcs-jj` share one parser here instead of each carrying a copy that could
silently drift. **Dependency-free** (std only) — pure data types and pure
functions, no process execution.

```rust
use vcs_diff::{parse_diff, ChangeKind};

let diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n";
for file in parse_diff(diff) {              // Vec<FileDiff>
    let verb = match file.change {          // ChangeKind is #[non_exhaustive]
        ChangeKind::Added => "added",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
        _ => "modified",
    };
    println!("{verb} {}", file.path);
    for hunk in &file.hunks {               // Vec<Hunk> of DiffLine
        // …
    }
}
```

It also exposes [`DiffStat`] (the file/line aggregate both `--shortstat` and
`--stat` parse into) and [`Version`] + [`parse_dotted_version`] for reading a
`<tool> --version` banner.

The wrapper crates re-export these types, so `vcs_git::FileDiff`,
`vcs_git::parse_diff`, and `vcs_git::GitVersion` (an alias of `vcs_diff::Version`)
all resolve — you rarely name this crate directly.

Part of [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs); used by
`vcs-git`, `vcs-jj`, and `vcs-core`.

## License

MIT
