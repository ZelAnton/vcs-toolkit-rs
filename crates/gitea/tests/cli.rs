//! Integration tests for `vcs-gitea`. Ignored by default (require the `tea`
//! binary). Run with `cargo test -p vcs-gitea -- --ignored`.
//!
//! `tea` is rarely pre-installed, so each test **skips gracefully** (prints and
//! returns) when the binary is absent, rather than failing — CI installs it
//! best-effort.
//!
//! The list/view tests below are the **definitive check** of the `tea --output csv`
//! DSV contract our positional parsers model (the hermetic unit tests can only confirm
//! the parser against *assumed* fixtures). They run a real `tea … list` and assert the
//! output is a real table our parser accepts — **not** a format mismatch. A format
//! mismatch is a hard **failure**, the exact bug class this crate's re-model fixed:
//! that means either our parser diverged from tea's real output (`Error::Parse`) or tea
//! rejected the requested `--output` format (an `unknown output type` diagnostic — how
//! `tea` 0.9.x reported the old, unsupported `--output json`, with exit 0). Only a
//! genuine **environment** problem (no Gitea repo, not authenticated, network) is a
//! skip. So they need a live, authenticated Gitea repo to be meaningful: point
//! `VCS_GITEA_TEST_REPO` at one (defaults to the cwd). The weekly `gitea-live` lane in
//! `.github/workflows/scheduled-cli-drift.yml` stands up a one-shot Gitea, logs `tea`
//! in, and points `VCS_GITEA_TEST_REPO` at a seeded repo, so these run live there
//! (alongside the `vcs-forge` facade lifecycle suite) — run them against a real `tea`
//! locally too, the same way.

use std::path::PathBuf;

use vcs_gitea::{Gitea, GiteaApi};

/// Whether `tea` is on PATH (a successful `--version` spawn).
async fn tea_present() -> bool {
    Gitea::new().version().await.is_ok()
}

/// The repo dir the list/view tests run against (`VCS_GITEA_TEST_REPO`, else cwd).
fn test_repo() -> PathBuf {
    std::env::var_os("VCS_GITEA_TEST_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"))
}

/// Whether an error is a **format-contract** signal (a hard failure) rather than an
/// environment skip. Two shapes count as drift: our parser rejecting tea's real output
/// (`Error::Parse`), and tea rejecting the requested `--output` format — the `unknown
/// output type` diagnostic, which on `tea` 0.9.x arrived with exit 0 and so used to be
/// swallowed as a silent empty list. Everything else (no repo, no login, network) is a
/// genuine environment error we skip on.
fn is_format_drift(err: &processkit::Error) -> bool {
    matches!(err, processkit::Error::Parse { .. })
        || err.to_string().contains("unknown output type")
}

/// Fail on a format-contract violation this suite hunts (a parser/`--output` mismatch);
/// treat only a genuine environment error (the dir is not a Gitea repo, no login,
/// network) as a skip. This is the un-masked gate: a format drift is **never** quietly
/// skipped, even when tea reports it via a non-`Parse` error.
fn assert_output_contract<T>(label: &str, result: processkit::Result<T>) {
    match result {
        Ok(_) => {} // tea produced a real table and our parser accepted it.
        Err(err) if is_format_drift(&err) => {
            panic!("{label}: tea output did not match the parser (contract drift): {err}");
        }
        Err(other) => eprintln!("skipping {label}: {other}"),
    }
}

/// Whether a release's `published_at` cell looks like tea's machine-readable timestamp
/// (RFC3339, e.g. `2023-07-26T13:02:36Z`) or is empty (an unpublished draft), and **not**
/// like a `Status` keyword (`released`/`draft`/`prerelease`).
///
/// `release_list` is the one read op with **no `--fields`** pin — tea's release-table
/// column order is intrinsic (pinned in `src/parse.rs` to tea's
/// `modules/print/release.go::ReleasesList`), so a same-typed `Published At`<->`Status`
/// transposition in a future `tea` would parse with no `Error::Parse`/`unknown output
/// type` and thus slip past [`is_format_drift`]. This value-shape check makes that
/// specific swap catchable against a real `tea`: for `--output csv` (machine-readable),
/// tea's `FormatTime` emits an RFC3339 stamp for a published release and `""` for a
/// draft — never a bare status word.
fn release_published_at_is_timestamp_or_empty(published_at: &str) -> bool {
    published_at.is_empty()
        || (published_at.chars().any(|c| c.is_ascii_digit())
            && published_at.chars().any(|c| c == '-' || c == ':'))
}

// Hermetic guard (runs without `tea`, in the normal test pass) for the shape predicate
// the live release gate relies on: an RFC3339 stamp and an empty draft cell pass; a
// `Status` keyword fails — so a `Published At`<->`Status` transposition trips the live
// gate rather than parsing into a silently-mislabeled release.
#[test]
fn release_published_at_shape_predicate_distinguishes_timestamps_from_status() {
    assert!(release_published_at_is_timestamp_or_empty(
        "2023-07-26T13:02:36Z"
    ));
    assert!(release_published_at_is_timestamp_or_empty("")); // unpublished draft
    for status in ["released", "draft", "prerelease"] {
        assert!(
            !release_published_at_is_timestamp_or_empty(status),
            "{status:?} must not read as a plausible published_at"
        );
    }
}

#[tokio::test]
#[ignore = "requires the tea binary"]
async fn version_runs() {
    if !tea_present().await {
        eprintln!("skipping: tea not installed");
        return;
    }
    let v = Gitea::new().version().await.expect("tea version");
    assert!(!v.trim().is_empty(), "expected a version string");
}

// The real `tea --version` banner must parse into a version at/above the crate
// floor. This is the "modern real binary" arm of the version-gate check the
// scheduled-drift lane runs (the hermetic unit tests in `src/lib.rs` cover the
// minimum and unrecognisable arms): if a future `tea` reshapes its `--version`
// output so the shared parser can't read it, `capabilities()` returns
// `Error::Parse` and this fails, flagging the drift.
#[tokio::test]
#[ignore = "requires the tea binary"]
async fn capability_version_gate_real_binary() {
    if !tea_present().await {
        eprintln!("skipping: tea not installed");
        return;
    }
    let caps = Gitea::new().capabilities().await.expect("tea capabilities");
    assert!(
        caps.is_supported(),
        "the installed tea ({}) is below vcs-gitea's supported floor",
        caps.version
    );
}

#[tokio::test]
#[ignore = "requires the tea binary"]
async fn auth_status_does_not_error() {
    if !tea_present().await {
        eprintln!("skipping: tea not installed");
        return;
    }
    // Reports the bool whether or not a login is configured; must not error — but if
    // tea rejects `--output csv` (a format regression), that IS a failure, not a skip.
    assert_output_contract("auth_status", Gitea::new().auth_status().await);
}

// The three list shapes (PR / issue / release tables) and the paged issue view must
// deserialize from REAL `tea --output csv` without a format mismatch. These are the
// only structural validation of the DSV table contract the parsers model — point
// `VCS_GITEA_TEST_REPO` at a populated, authenticated Gitea repo.
#[tokio::test]
#[ignore = "requires the tea binary + a real Gitea repo/login"]
async fn list_outputs_match_the_parsers() {
    if !tea_present().await {
        eprintln!("skipping: tea not installed");
        return;
    }
    let tea = Gitea::new();
    let dir = test_repo();

    assert_output_contract("pr_list", tea.pr_list(&dir).await);
    assert_output_contract("issue_list", tea.issue_list(&dir).await);

    // `release_list` has no `--fields` pin, so a same-typed `Published At`<->`Status`
    // transposition would parse cleanly and slip past the format-drift gate. Beyond the
    // parser/`--output` contract, assert each real release row's `published_at` still
    // looks like a timestamp (never a `Status` word), so that specific column swap fails
    // here instead of silently mislabeling releases. Zero rows (empty repo) stays a clean
    // pass; a genuine environment error stays a skip.
    match tea.release_list(&dir).await {
        Ok(releases) => {
            for r in &releases {
                assert!(
                    release_published_at_is_timestamp_or_empty(&r.published_at),
                    "release_list: published_at {:?} (tag {:?}) is not a timestamp — tea's \
                     release column order may have drifted (Published At<->Status transposition)",
                    r.published_at,
                    r.tag,
                );
            }
        }
        Err(err) if is_format_drift(&err) => {
            panic!("release_list: tea output did not match the parser (contract drift): {err}");
        }
        Err(other) => eprintln!("skipping release_list: {other}"),
    }

    // issue_view pages the same issues list and filters by number; probe #1 (a
    // non-format-drift error is a fine skip if it simply doesn't exist).
    assert_output_contract("issue_view", tea.issue_view(&dir, 1).await);
}
