//! Integration tests for `vcs-github`. Ignored by default (require the `gh`
//! binary). The repo/pr/issue commands need network + authentication and are
//! not exercised here — their JSON parsing is covered by the hermetic unit
//! tests in `src/parse.rs` and the scripted-runner tests in `src/lib.rs`. Run
//! with `cargo test -p vcs-github -- --ignored`.

use vcs_github::{GitHub, GitHubApi, GitHubHost};

/// Whether `gh` is on PATH (a successful `--version` spawn).
async fn gh_present() -> bool {
    GitHub::new().version().await.is_ok()
}

#[tokio::test]
#[ignore = "requires the gh binary"]
async fn version_mentions_gh() {
    let v = GitHub::new()
        .version()
        .await
        .expect("gh should be installed");
    assert!(v.to_lowercase().contains("gh"), "unexpected: {v}");
}

// The real `gh --version` banner must parse into a version at/above the crate
// floor. This is the "modern real binary" arm of the version-gate check the
// scheduled-drift lane runs (the hermetic unit tests in `src/lib.rs` cover the
// minimum and unrecognisable arms): if a future `gh` reshapes its `--version`
// output so the shared parser can't read it, `capabilities()` returns
// `ErrorReason::Parse` and this fails, flagging the drift.
#[tokio::test]
#[ignore = "requires the gh binary"]
async fn capability_version_gate_real_binary() {
    if !gh_present().await {
        eprintln!("skipping: gh not installed");
        return;
    }
    let caps = GitHub::new().capabilities().await.expect("gh capabilities");
    assert!(
        caps.is_supported(),
        "the installed gh ({}) is below vcs-github's supported floor",
        caps.version
    );
}

#[tokio::test]
#[ignore = "requires the gh binary"]
async fn auth_status_does_not_error() {
    // Reports the bool whether or not the user is logged in; must not error.
    let _authed = GitHub::new()
        .auth_status()
        .await
        .expect("auth_status should not error");
}

#[tokio::test]
#[ignore = "requires the gh binary"]
async fn auth_status_for_host_does_not_error() {
    // The host-scoped probe reports the bool for one host (`gh auth status
    // --hostname github.com`) whether or not the user is logged in; it must not
    // error, just like the unscoped `auth_status`.
    let _authed = GitHub::new()
        .auth_status_for(&GitHubHost::github_com())
        .await
        .expect("auth_status_for should not error");
}

// Drift detection for the one command this crate reads as *text*: `gh auth
// status` has no `--json`, so a reworded report would silently degrade
// `auth_info` to "unknown". The skip below is positively identified — gh itself
// reporting no session — so a *recognised* session whose accounts we cannot read
// fails hard instead of masquerading as an environment gap.
#[tokio::test]
#[ignore = "requires the gh binary"]
async fn auth_info_recognises_the_live_report_format() {
    if !gh_present().await {
        eprintln!("skipping: gh not installed");
        return;
    }
    let auth = GitHub::new()
        .auth_info()
        .await
        .expect("auth_info should not error");
    if !auth.authed {
        // gh answered "no session" — nothing was printed to recognise.
        eprintln!("skipping: gh is not authenticated");
        return;
    }
    assert!(
        !auth.is_unknown(),
        "gh reports a session but no account line was recognised — \
         `gh auth status` output has drifted from the shapes this crate parses"
    );
    // Not `active().is_some()`: a machine logged in to several *hosts* leaves the
    // active account legitimately unresolved (see `GitHubAuth::active`).
}

#[tokio::test]
#[ignore = "requires the gh binary"]
async fn repo_visible_does_not_error() {
    if !gh_present().await {
        eprintln!("skipping: gh not installed");
        return;
    }
    // Reports the bool whether or not this checkout's repository is visible to
    // the active account (and whether or not gh is logged in); must not error.
    let _visible = GitHub::new()
        .repo_visible(std::path::Path::new("."))
        .await
        .expect("repo_visible should not error");
}

// Read-only, auth-gated checks against this very repository (it has real
// Actions runs and releases). Skipped politely when gh isn't authenticated.
// PR-scoped reads (`pr_checks`, `pr_feedback`) have NO live coverage — the
// repo has no PRs; their parsing is covered hermetically.

/// `Some(client)` when gh is installed AND authenticated; `None` → skip.
async fn authed() -> Option<GitHub> {
    let gh = GitHub::new();
    match gh.auth_status().await {
        Ok(true) => Some(gh),
        _ => {
            eprintln!("skipping: gh not authenticated");
            None
        }
    }
}

#[tokio::test]
#[ignore = "requires the gh binary, auth, and network"]
async fn run_list_and_view_round_trip() {
    let Some(gh) = authed().await else { return };
    let dir = std::path::Path::new(".");

    let runs = gh.run_list(dir, 3, None).await.expect("run_list");
    assert!(!runs.is_empty(), "this repo has Actions runs");
    let first = &runs[0];
    assert!(first.database_id > 0);
    assert!(!first.workflow_name.is_empty(), "got {first:?}");
    assert!(!first.url.is_empty());

    let viewed = gh.run_view(dir, first.database_id).await.expect("run_view");
    assert_eq!(viewed.database_id, first.database_id);
    assert_eq!(viewed.workflow_name, first.workflow_name);
}

#[tokio::test]
#[ignore = "requires the gh binary, auth, and network"]
async fn release_list_and_view_round_trip() {
    let Some(gh) = authed().await else { return };
    let dir = std::path::Path::new(".");

    let releases = gh.release_list(dir).await.expect("release_list");
    let released_tag = releases
        .first()
        .map(|release| release.tag_name.clone())
        .expect("this repo has releases");

    let release = gh
        .release_view(dir, &released_tag)
        .await
        .expect("release_view");
    assert_eq!(release.tag_name, released_tag);
    // `release_view` fetches body/url, so both are `Some` and non-empty (the lean
    // `release_list` leaves them `None`).
    assert!(
        release.body.as_deref().is_some_and(|b| !b.is_empty()),
        "release notes were curated"
    );
    assert!(release.url.as_deref().is_some_and(|u| !u.is_empty()));
}

// --- Cassette recording ------------------------------------------------
//
// The two tests below are not part of the ordinary suite: they drive a real,
// authenticated `gh` against this very repository and (re)write the
// human-readable JSON cassettes `src/lib.rs`'s hermetic unit tests replay
// (`release_view_requests_view_fields`, `run_list_and_view_replay_recorded_cassette`).
// See CONTRIBUTING.md, "Updating a `gh` CLI cassette", for when/how to run
// these and how a cassette diff should read on review.
//
// `processkit`'s `record` feature is enabled unconditionally for this crate's
// dev/test profile (see `[dev-dependencies]` in Cargo.toml), so no extra
// `--features` flag is needed here — just `--ignored` to opt into the ones
// that spawn a real `gh`.
//
// Run with: `cargo test -p vcs-github -- --ignored record_`
mod record {
    use super::*;
    use processkit::testing::{CassetteField, RecordReplayRunner, Reply, ScriptedRunner};
    use processkit::{JobRunner, ProcessRunner};
    use proptest::prelude::*;
    use std::path::PathBuf;
    use vcs_cli_support::logging::redact_value;

    const SCRUBBED_CWD: &str = "<cwd>";
    const SCRUBBED_PATH: &str = "<path>";

    /// Make every field safe to persist while preserving the full shape of gh's
    /// JSON output. The hook is deliberately stateless: processkit may call it
    /// concurrently, and the same deterministic function is used for record and
    /// replay lookup keys.
    fn scrub_gh_cassette_field(field: CassetteField, text: &str) -> String {
        if matches!(field, CassetteField::Cwd) && is_absolute_path(text) {
            return SCRUBBED_CWD.to_owned();
        }

        scrub_absolute_paths(&redact_value(text))
    }

    fn is_absolute_path(value: &str) -> bool {
        let bytes = value.as_bytes();
        PathBuf::from(value).is_absolute()
            || value.starts_with('/')
            || value.starts_with("\\\\")
            || value.starts_with("//")
            || (bytes.len() >= 3 && bytes[1] == b':' && matches!(bytes[2], b'/' | b'\\'))
    }

    /// Replace portable absolute-path spellings embedded in output/arguments.
    ///
    /// This intentionally handles both Windows and Unix spellings regardless of
    /// the host doing the recording, so a cassette cannot retain a path merely
    /// because it was recorded on the other platform. Quoted JSON strings may
    /// contain spaces; unquoted argv values are delimited at whitespace.
    fn scrub_absolute_paths(value: &str) -> String {
        let bytes = value.as_bytes();
        let mut out = String::with_capacity(value.len());
        let mut index = 0;
        while index < bytes.len() {
            if is_absolute_path_start(bytes, index) {
                let quoted = index > 0 && bytes[index - 1] == b'"';
                let end = path_end(bytes, index, quoted);
                out.push_str(SCRUBBED_PATH);
                index = end;
            } else {
                let ch = value[index..]
                    .chars()
                    .next()
                    .expect("index always points into the string");
                out.push(ch);
                index += ch.len_utf8();
            }
        }
        out
    }

    fn is_absolute_path_start(bytes: &[u8], index: usize) -> bool {
        let boundary = index == 0
            || matches!(
                bytes[index - 1],
                b' ' | b'\t'
                    | b'\r'
                    | b'\n'
                    | b'"'
                    | b'\''
                    | b'='
                    | b'('
                    | b'['
                    | b'{'
                    | b','
                    | b';'
            );
        if !boundary {
            return false;
        }

        let drive_path = index + 2 < bytes.len()
            && bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'/' | b'\\');
        let unix_path = bytes[index] == b'/' && index + 1 < bytes.len() && bytes[index + 1] != b'/';
        let unc_path = index + 1 < bytes.len()
            && matches!(bytes[index], b'/' | b'\\')
            && bytes[index + 1] == bytes[index];
        drive_path || unix_path || unc_path
    }

    fn path_end(bytes: &[u8], start: usize, quoted: bool) -> usize {
        let mut index = start;
        while index < bytes.len() {
            if quoted {
                if bytes[index] == b'"' && !is_escaped_quote(bytes, index) {
                    break;
                }
            } else if bytes[index].is_ascii_whitespace()
                || matches!(
                    bytes[index],
                    b'"' | b'\'' | b',' | b']' | b'}' | b')' | b';'
                )
            {
                break;
            }
            index += 1;
        }
        index
    }

    fn is_escaped_quote(bytes: &[u8], index: usize) -> bool {
        let mut backslashes = 0;
        let mut cursor = index;
        while cursor > 0 && bytes[cursor - 1] == b'\\' {
            backslashes += 1;
            cursor -= 1;
        }
        backslashes % 2 == 1
    }

    #[test]
    fn scrubber_redacts_secrets_paths_and_preserves_ordinary_values() {
        let secret = "github_pat_DO_NOT_PERSIST_123456";
        assert_eq!(
            scrub_gh_cassette_field(CassetteField::Argument, secret),
            "<redacted>"
        );
        assert_eq!(
            scrub_gh_cassette_field(
                CassetteField::Stdout,
                &format!(r#"{{"token":"{secret}","cwd":"C:\\Users\\alice\\repo"}}"#)
            ),
            r#"{"token":"<redacted>","cwd":"<path>"}"#
        );
        assert_eq!(
            scrub_gh_cassette_field(CassetteField::Cwd, r"C:\Users\alice\repo"),
            SCRUBBED_CWD
        );
        assert_eq!(scrub_gh_cassette_field(CassetteField::Cwd, "."), ".");
        assert_eq!(
            scrub_gh_cassette_field(CassetteField::Argument, "--json"),
            "--json"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn scrubber_is_idempotent_and_preserves_plain_values(
            value in "[a-z0-9._-]{0,160}".prop_filter(
                "plain values do not contain known secret-shaped prefixes",
                |value| {
                    [
                        "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "glpat-",
                        "glptt-", "xoxb-", "xoxp-", "xoxa-", "xoxr-",
                    ]
                    .iter()
                    .all(|prefix| !value.contains(*prefix))
                },
            )
        ) {
            let once = scrub_gh_cassette_field(CassetteField::Stdout, &value);
            prop_assert_eq!(
                scrub_gh_cassette_field(CassetteField::Stdout, &once),
                once.clone()
            );
            prop_assert_eq!(once, value);
        }
    }

    #[tokio::test]
    async fn scrubber_is_symmetric_for_record_and_replay_without_live_gh() {
        let path =
            std::env::temp_dir().join(format!("vcs-github-t166-scrub-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let command = processkit::Command::new("gh")
            .args(["run", "view", "--token=github_pat_NOT_IN_FIXTURE"])
            .current_dir(r"C:\Users\recording\repo");
        let recorder = RecordReplayRunner::record(
            &path,
            ScriptedRunner::new().on(
                ["gh", "run", "view", "--token=github_pat_NOT_IN_FIXTURE"],
                Reply::ok(
                    r#"{"message":"github_pat_NOT_IN_FIXTURE","path":"C:\\Users\\recording\\repo"}"#,
                ),
            ),
        )
        .scrub_with(scrub_gh_cassette_field);
        let _ = recorder
            .output_string(&command)
            .await
            .expect("record scripted response");
        recorder.save().expect("save scrubbed fixture");

        let fixture = std::fs::read_to_string(&path).expect("read fixture");
        assert!(!fixture.contains("github_pat_NOT_IN_FIXTURE"));
        assert!(!fixture.contains(r"C:\Users\recording\repo"));

        let replayer = RecordReplayRunner::replay(&path)
            .expect("load scrubbed fixture")
            .scrub_with(scrub_gh_cassette_field);
        let replayed = replayer
            .output_string(&command)
            .await
            .expect("scrubbed invocation matches scrubbed key");
        assert_eq!(
            replayed.stdout(),
            r#"{"message":"<redacted>","path":"<path>"}"#
        );
        let _ = std::fs::remove_file(path);
    }

    fn cassette_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/cassettes")
            .join(name)
    }

    #[tokio::test]
    #[ignore = "records a live cassette against gh; requires network + an authenticated gh"]
    async fn record_release_round_trip() {
        let runner =
            RecordReplayRunner::record(cassette_path("release_round_trip.json"), JobRunner::new())
                .scrub_with(scrub_gh_cassette_field);
        let gh = GitHub::with_runner(&runner);
        let dir = std::path::Path::new(".");

        let releases = gh.release_list(dir).await.expect("release_list");
        let tag = releases
            .first()
            .map(|r| r.tag_name.clone())
            .expect("this repo has releases");
        gh.release_view(dir, &tag).await.expect("release_view");

        runner.save().expect("save release cassette");
    }

    #[tokio::test]
    #[ignore = "records a live cassette against gh; requires network + an authenticated gh"]
    async fn record_run_round_trip() {
        let runner =
            RecordReplayRunner::record(cassette_path("run_round_trip.json"), JobRunner::new())
                .scrub_with(scrub_gh_cassette_field);
        let gh = GitHub::with_runner(&runner);
        let dir = std::path::Path::new(".");

        let runs = gh.run_list(dir, 3, None).await.expect("run_list");
        let id = runs
            .first()
            .map(|r| r.database_id)
            .expect("this repo has Actions runs");
        gh.run_view(dir, id).await.expect("run_view");

        runner.save().expect("save run cassette");
    }
}
