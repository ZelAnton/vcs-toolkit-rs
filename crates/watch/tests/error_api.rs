//! Public error-boundary guard (T-055).
//!
//! A consumer classifies and source-chains a `vcs-watch` failure through
//! `vcs-watch` (and `std`) **alone** — no `notify` in scope. The filesystem-watch
//! backend is a private dependency, so its error type never appears in the public
//! API: a `notify` major bump is an internal change here, not a breaking one
//! downstream. This file models exactly what a downstream consumer can write; if
//! it compiles, that boundary holds.

use std::error::Error as _;
use std::io;

use vcs_watch::{Error, WatchError};

/// Compile-test: every method of the opaque [`WatchError`], the
/// [`Error::watch_error`] accessor, and the `Error::Notify` variant is reachable
/// with no `notify` dependency named. Called from the test below so it is
/// type-checked, but the point is that it *compiles* against `vcs-watch` alone.
fn classify(err: &Error) {
    // Reach the backend classifiers through the stable accessor — no backend
    // type needed.
    if let Some(w) = err.watch_error() {
        let _: bool = w.is_path_not_found();
        let _: bool = w.is_watch_limit();
        let _: Option<&io::Error> = w.io_error();
        let _: &[std::path::PathBuf] = w.paths();
        // The opaque wrapper is itself an `Error` and source-chains.
        let _: Option<&(dyn std::error::Error + 'static)> = w.source();
    }
    // The variant is still matchable by name; its payload stays opaque.
    if let Error::Notify(w) = err {
        let _: &WatchError = w;
    }
    // Walk the whole chain — traversing it needs no backend type either.
    let mut source = err.source();
    while let Some(e) = source {
        source = e.source();
    }
}

#[test]
fn error_is_classifiable_and_chainable_without_notify() {
    // Constructible from the public `From<std::io::Error>`: a consumer never
    // needs the backend type to obtain, inspect, or source-chain an error.
    let err = Error::from(io::Error::from(io::ErrorKind::PermissionDenied));
    assert!(
        err.watch_error().is_none(),
        "an Io error is not a watch-backend error"
    );
    assert!(err.source().is_some(), "the std source is exposed");
    classify(&err);
}
