//! Lossless raw-bytes → `OsString`/`PathBuf` bridge for filesystem paths taken
//! from `git`/`jj` machine output.
//!
//! A filesystem path is *bytes*, not text: on Unix a filename can be any byte
//! sequence except `/` and NUL, so it need not be valid UTF-8. Decoding such a
//! path through [`String::from_utf8_lossy`] substitutes `U+FFFD` for the offending
//! bytes, and the resulting `String` no longer names the same file — feeding it
//! back to `add`/`commit_paths` then addresses a *different* path (or none at
//! all). These helpers preserve the exact bytes so a path read from
//! status/diff/conflict output round-trips into a mutating call unchanged.

use std::ffi::OsString;
use std::path::PathBuf;

/// Build an [`OsString`] from raw filesystem-path `bytes`, losslessly on Unix.
///
/// - **Unix:** the bytes *are* the OS path encoding, wrapped verbatim via
///   [`OsStringExt::from_vec`](std::os::unix::ffi::OsStringExt::from_vec), so a
///   filename whose bytes are not valid UTF-8 survives byte-for-byte.
/// - **Other platforms (Windows/WASI):** `git` and `jj` emit their `-z` / machine
///   path output as UTF-8 there, so the bytes are decoded as UTF-8. A genuinely
///   invalid sequence — which these tools do not produce on this path — falls back
///   to the lossy replacement, preserving the pre-existing Windows
///   `String`/`OsString` behaviour (Unicode names like `𝓁abc` still round-trip).
pub fn os_from_bytes(bytes: &[u8]) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(bytes.to_vec())
    }
    #[cfg(not(unix))]
    {
        OsString::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// [`os_from_bytes`] as a [`PathBuf`] — the path type the facade DTOs carry.
pub fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(os_from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_utf8_round_trip_on_every_platform() {
        assert_eq!(path_from_bytes(b"src/lib.rs"), PathBuf::from("src/lib.rs"));
        // A multibyte UTF-8 name decodes to the same scalar on all platforms.
        assert_eq!(
            path_from_bytes("café.txt".as_bytes()),
            PathBuf::from("café.txt")
        );
    }

    // On Unix, a non-UTF-8 filename survives byte-for-byte (the load-bearing
    // property this whole change exists for): the bytes go in and come back out
    // of the `OsString` unchanged, never substituted with U+FFFD.
    #[cfg(unix)]
    #[test]
    fn non_utf8_bytes_survive_on_unix() {
        use std::os::unix::ffi::OsStrExt;
        let raw = b"caf\xff.txt"; // 0xFF is never valid UTF-8
        let os = os_from_bytes(raw);
        assert_eq!(os.as_bytes(), raw, "the exact bytes must survive");
    }
}
