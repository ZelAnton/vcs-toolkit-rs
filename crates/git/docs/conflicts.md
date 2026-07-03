# Conflict resolution guide

`vcs-git` and `vcs-jj` each ship a typed model of conflict markers:
`vcs_git::conflict` and `vcs_jj::conflict`. These are **pure parsers over a
file's content** — no subprocess, no repo handle, nothing to mock. You feed in
the marker soup a merge left behind, you get back structured regions, and you
can re-render or resolve to a chosen side. That makes them the primitive for
programmatic conflict resolution: the clients (next section) fetch the bytes,
these modules reason about them.

Two invariants hold across both modules — they are property-tested and fuzzed,
not aspirational:

- **Byte-exact round-trip.** `render(parse(x)?) == x` for any input that
  parses, including CRLF, custom marker sizes, multibyte text, and a conflict
  at EOF with no trailing newline. Lines are kept *with* their endings (the
  last line of a file may have none), and the verbatim marker lines are stored
  alongside the parsed data — so rendering never reconstructs a marker, it
  replays it.
- **Never panics.** The grammars slice on marker-run lengths against hostile
  input (a real conflicted file from a `git`/`jj` you don't control); arbitrary
  bytes parse-or-error, never abort.

The split is deliberate, not duplication: git and jj materialize conflicts with
*different grammars*. jj configured with `ui.conflict-marker-style = "git"`
emits git's grammar (with jj's labels) — parse those with `vcs_git::conflict`.
jj's native `diff`/`snapshot` styles live in `vcs_jj::conflict`. This asymmetry
is documented in both modules, not an oversight.

## git conflicts (`vcs_git::conflict`)

### The marker model

One grammar covers git's three `merge.conflictStyle`s — `merge` (2-way),
`diff3`, and `zdiff3` (same markers as diff3; the common affixes are already
hoisted outside the region by git):

```text
<<<<<<< HEAD            // ours-side marker + label
ours line
||||||| 0b025ce        // base marker + label — diff3/zdiff3 only
base line
=======                // separator
theirs line
>>>>>>> feature        // theirs-side marker + label
```

Marker length is **variable** — 7 by default, more if `merge.conflictMarkerSize`
raised it — and detected per region (`marker_len`). A line counts as a marker
only when the run is followed by a space + label or ends the line, so a line of
literal `=======` inside content with no following space is not mistaken for a
separator.

### Types

```rust,ignore
pub enum ResolutionSide { Ours, Base, Theirs }   // Base is diff3/zdiff3 only

pub enum ConflictSegment {
    Text(Vec<String>),                 // lines outside any conflict, verbatim
    Conflict(Box<ConflictRegion>),     // boxed — far larger than a text run
}

#[non_exhaustive]
pub struct ConflictRegion {
    pub ours_label: String,            // after `<<<<<<<` (e.g. "HEAD"); "" if absent
    pub base_label: Option<String>,    // after `|||||||`; None for 2-way
    pub theirs_label: String,          // after `>>>>>>>` (e.g. branch name)
    pub ours: Vec<String>,             // `<<<<<<<`-side lines
    pub base: Option<Vec<String>>,     // base lines (diff3/zdiff3); None for 2-way
    pub theirs: Vec<String>,           // `>>>>>>>`-side lines
    pub marker_len: usize,             // 7 unless merge.conflictMarkerSize raised it
    // plus private verbatim marker lines, for byte-exact rendering
}
```

`ConflictRegion` is `#[non_exhaustive]` — match it with `..` and construct it
only via `parse_conflicts`. The label/line vectors are public so you can
inspect a region; the marker lines themselves are private, which is *why* a
hand-built region can't exist and `render` can stay byte-exact.

### Functions

```rust,ignore
pub fn has_conflict_markers(content: &str) -> bool;
pub fn parse_conflicts(content: &str) -> Result<Vec<ConflictSegment>>;
pub fn render(segments: &[ConflictSegment]) -> String;
pub fn resolve(segments: &[ConflictSegment], side: ResolutionSide) -> Result<String>;
```

`has_conflict_markers` is a cheap pre-check (any line that looks like a
`<<<<<<<` start) before committing to a full parse. `parse_conflicts` errors
with `Error::Parse` only on a genuinely malformed **region** — a `<<<<<<<`-opened
region missing its `=======` separator or `>>>>>>>` terminator. A `=======` /
`>>>>>>>` run *outside* any region (a Markdown/RST underline, a divider, a quoted
email) is kept as ordinary text, not an error, so a file with marker-like content
parses cleanly. `resolve` errors when you ask for `Base` on a 2-way `merge`-style
conflict that records none.

### Worked examples

Detect, then parse:

```rust,ignore
# use vcs_git::conflict::{has_conflict_markers, parse_conflicts};
# fn demo(content: &str) -> Result<(), processkit::Error> {
if has_conflict_markers(content) {
    let segments = parse_conflicts(content)?;   // Vec<ConflictSegment>
    // ... inspect / resolve ...
}
# Ok(()) }
```

Iterate regions, printing each side:

```rust,ignore
# use vcs_git::conflict::{parse_conflicts, ConflictSegment};
# fn demo(content: &str) -> Result<(), processkit::Error> {
for segment in parse_conflicts(content)? {
    let ConflictSegment::Conflict(region) = segment else { continue };
    println!("<<< {} | >>> {}", region.ours_label, region.theirs_label);
    print!("ours:   {}", region.ours.concat());     // lines keep their endings
    print!("theirs: {}", region.theirs.concat());
    if let Some(base) = &region.base {               // diff3/zdiff3 only
        print!("base:   {}", base.concat());
    }
}
# Ok(()) }
```

Resolve every region to ours:

```rust,ignore
# use vcs_git::conflict::{parse_conflicts, resolve, ResolutionSide};
# fn demo(content: &str) -> Result<(), processkit::Error> {
let segments = parse_conflicts(content)?;
let resolved = resolve(&segments, ResolutionSide::Ours)?;   // String, write it back
// resolve(&segments, ResolutionSide::Base) errors on 2-way `merge` style.
# Ok(()) }
```

The round-trip invariant, made concrete:

```rust,ignore
# use vcs_git::conflict::{parse_conflicts, render};
# fn demo(content: &str) -> Result<(), processkit::Error> {
let segments = parse_conflicts(content)?;
assert_eq!(render(&segments), content);   // byte-for-byte, CRLF and EOF included
# Ok(()) }
```

## jj conflicts (`vcs_jj::conflict`)

jj's **materialized** markers are not git's. A region is delimited by a counter
(`conflict N of M`) and contains one or more *sections*:

```text
<<<<<<< conflict 1 of 1
%%%%%%% diff from: rnxsupvw 638ae425 "base"        // DIFF section
\\\\\\\        to: ozvltnxm 92f2b14f "side-a"
-line 2                                            // old (base) text
+main line 2                                       // new (side) text
+++++++ xyrusolp ad268d1f "side-b"                 // SNAPSHOT section (a side)
feature line 2
>>>>>>> conflict 1 of 1 ends
```

Two section styles, set by `ui.conflict-marker-style`:

- **`diff`** (the jj 0.38 default) — one side rendered as a unified diff
  *from the base*. A `%%%%%%%` line opens it (`diff from:` label), followed by a
  `\\\\\\\` continuation line (`to:` label), then `-`/`+`/` `-prefixed lines.
  The side's content is the diff's **new** text (`+`/` `); the base is its
  **old** text (`-`/` `).
- **`snapshot`** — every side and the base rendered verbatim. A `+++++++` line
  opens a side (`Snapshot`); a `-------` line opens the base (`Base`).

Both styles can coexist in one region (the diff example above mixes a `%%%%%%%`
side with a `+++++++` side). Section/end markers must match the region's opening
run length — jj lengthens *all* of a file's markers together when content
contains marker-like runs, so a shorter run is content, not a marker.

### Types

```rust,ignore
#[non_exhaustive]
pub enum JjConflictSection {
    Diff {                             // `%%%%%%%` — one side as a unified diff
        from_label: String,            //   the `diff from:` label (base's ids/desc)
        to_label: String,              //   the `to:` label (this side's ids/desc)
        lines: Vec<String>,            //   raw diff lines, verbatim
    },
    Snapshot { label: String, lines: Vec<String> },  // `+++++++` — a side, verbatim
    Base     { label: String, lines: Vec<String> },  // `-------` — the base, verbatim
}

#[non_exhaustive]
pub struct JjConflictRegion {
    pub number: u32,                   // the `N` of `conflict N of M`
    pub total: u32,                    // the `M`
    pub sections: Vec<JjConflictSection>,  // in file order
    // plus private verbatim marker lines, for byte-exact rendering
}

pub enum JjConflictSegment {
    Text(Vec<String>),
    Conflict(Box<JjConflictRegion>),
}

pub enum JjResolution {
    Side(usize),                       // N-th side, 0-based, file order
    Base,                              // the recorded base
}
```

`JjConflictRegion` carries two materializers — they apply the recorded diff so
you don't reason about `-`/`+` prefixes yourself:

```rust,ignore
impl JjConflictRegion {
    pub fn sides(&self) -> Vec<Vec<String>>;   // each side's content, file order
    pub fn base(&self) -> Option<Vec<String>>; // the base, when one is recorded
}
```

`sides()` returns one entry per side: a `Diff` section contributes its applied
**new** text, a `Snapshot` its verbatim lines, and a `Base` section is *not* a
side (skipped). `base()` finds the first base it can: a `Diff` section's applied
**old** text, or a `Snapshot`-style `-------` section's lines — `None` if the
region records neither.

### Functions

```rust,ignore
pub fn has_conflict_markers(content: &str) -> bool;
pub fn parse_conflicts(content: &str) -> Result<Vec<JjConflictSegment>>;
pub fn render(segments: &[JjConflictSegment]) -> String;
pub fn resolve(segments: &[JjConflictSegment], resolution: JjResolution) -> Result<String>;
```

`has_conflict_markers` looks for a `<<<<<<<` line whose label parses as
`conflict N of M` — git-style markers are *not* jj's and won't match.
`parse_conflicts` errors with `Error::Parse` on an unterminated region, content
before the first section marker, or a `git`-style file (the error tells you to
use `vcs_git::conflict`). `resolve` errors when the requested `Side(i)` doesn't
exist or `Base` is requested on a region with no base — the message names the
conflict number and its side count.

### Worked examples

Parse, then materialize each side and the base — no prefix-stripping by hand:

```rust,ignore
# use vcs_jj::conflict::{parse_conflicts, JjConflictSegment};
# fn demo(content: &str) -> Result<(), processkit::Error> {
for segment in parse_conflicts(content)? {
    let JjConflictSegment::Conflict(region) = segment else { continue };
    println!("conflict {} of {}", region.number, region.total);
    for (i, side) in region.sides().iter().enumerate() {
        print!("side {i}: {}", side.concat());   // diff applied, prefixes gone
    }
    if let Some(base) = region.base() {
        print!("base:   {}", base.concat());
    }
}
# Ok(()) }
```

Resolve to the first side, or to the base:

```rust,ignore
# use vcs_jj::conflict::{parse_conflicts, resolve, JjResolution};
# fn demo(content: &str) -> Result<(), processkit::Error> {
let segments = parse_conflicts(content)?;
let theirs = resolve(&segments, JjResolution::Side(0))?;   // first side, file order
let base   = resolve(&segments, JjResolution::Base)?;      // errors if none recorded
// resolve(&segments, JjResolution::Side(99)) errors: that side doesn't exist.
# Ok(()) }
```

Round-trip — exact even with a conflict at EOF and no trailing newline:

```rust,ignore
# use vcs_jj::conflict::{parse_conflicts, render};
# fn demo(content: &str) -> Result<(), processkit::Error> {
let segments = parse_conflicts(content)?;
assert_eq!(render(&segments), content);   // byte-for-byte
# Ok(()) }
```

## Pairing with the clients

These modules parse content — they don't fetch it. Get the bytes from the
client crate, then hand them over.

git: list the unmerged paths, read each file's working-tree content (the worktree
holds the conflict markers), and parse:

```rust,ignore
# use std::{fs, path::Path};
# use vcs_git::{Git, GitApi, conflict};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
for path in git.conflicted_files(repo).await? {          // Vec<String>, unmerged paths
    let content = fs::read_to_string(repo.join(&path)).unwrap();
    if conflict::has_conflict_markers(&content) {
        let segments = conflict::parse_conflicts(&content)?;
        let resolved = conflict::resolve(&segments, conflict::ResolutionSide::Ours)?;
        fs::write(repo.join(&path), resolved).unwrap();   // then `git add` it
    }
}
# Ok(()) }
```

`git.show_file(repo, rev, path)` fetches a *specific revision's* blob when you
want a stage rather than the worktree.

jj: list conflicted paths in a revset, materialize each, and parse:

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi, conflict};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
for path in jj.resolve_list(repo, "@").await? {           // Vec<String> of paths
    let content = jj.file_show(repo, "@", &path).await?;  // materialized markers
    let segments = conflict::parse_conflicts(&content)?;
    let resolved = conflict::resolve(&segments, conflict::JjResolution::Side(0))?;
    // ... write `resolved` back via your file-update path ...
}
# Ok(()) }
```

For the full client surface — handles, mocking, error shapes — see the per-crate
guides: [vcs-git guide](https://docs.rs/vcs-git/latest/vcs_git/guide/) and [vcs-jj guide](https://docs.rs/vcs-jj/latest/vcs_jj/guide/).

## Robustness

Both parsers are built to survive content they didn't author. The grammars only
ever slice on detected marker-run lengths, and every slice is guarded — so
`parse_conflicts` on arbitrary bytes returns `Ok`/`Err`, never panics. The jj
side additionally exercises `sides()`, `base()`, and `render` on whatever parsed,
so the diff materializer (`apply_diff`) is in the panic-free guarantee too.

The round-trip is the load-bearing invariant and is pinned three ways:

- **Unit tests** on captured-verbatim samples (jj 0.38 `diff` and `snapshot`
  output; git `merge`/`diff3`, CRLF, wide markers, EOF-without-newline).
- **`proptest`** generators that draw from each tool's marker vocabulary with
  variable counters/marker lengths and multibyte text, asserting
  `render(parse(x)?) == x` for everything that parses.
- **`cargo-fuzz`** targets in `fuzz/` for continuous coverage beyond the
  proptest corpus.

See the [workspace README](https://github.com/ZelAnton/vcs-toolkit-rs#readme) for how to run the build, tests, and
fuzz targets.

## See also

- [vcs-git guide](https://docs.rs/vcs-git/latest/vcs_git/guide/)
- [vcs-jj guide](https://docs.rs/vcs-jj/latest/vcs_jj/guide/)
- [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/)
