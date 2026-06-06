//! A semantic `major.minor.patch` version and a tolerant parser for the
//! `<tool> --version` banners that `vcs-git`/`vcs-jj` read.

/// A parsed CLI version (`major.minor.patch`). `Ord` compares numerically, so a
/// caller can gate a feature on a minimum version; `Hash` lets it key a map (e.g.
/// a per-version capability cache).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    /// Major component (`2` in `2.54.0`).
    pub major: u64,
    /// Minor component.
    pub minor: u64,
    /// Patch component (`0` when the binary reports only `major.minor`).
    pub patch: u64,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Find the first `N.N[.N…]` token in `raw` and return its leading three numeric
/// components (a missing patch reads as 0). Each component is the token's leading
/// digits, so `0-dev` or `1.windows` trailers don't break parsing — this handles
/// `git version 2.54.0.windows.1`, `jj 0.42.0`, `2.41.0-rc1`, etc.
pub fn parse_dotted_version(raw: &str) -> Option<Version> {
    for token in raw.split_whitespace() {
        let mut parts = token.split('.');
        let Some(major) = parts.next().and_then(leading_number) else {
            continue;
        };
        let Some(minor) = parts.next().and_then(leading_number) else {
            continue; // A bare number ("2") is not a version token.
        };
        let patch = parts.next().and_then(leading_number).unwrap_or(0);
        return Some(Version {
            major,
            minor,
            patch,
        });
    }
    None
}

/// The numeric prefix of `s` (`"38-dev"` → 38); `None` when it has none.
fn leading_number(s: &str) -> Option<u64> {
    let end = s.bytes().take_while(u8::is_ascii_digit).count();
    if end == 0 {
        return None;
    }
    s[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_world_shapes() {
        // The Windows build trailer (`.windows.1`) is extra dotted components
        // beyond the patch; an `-rc1` suffix rides on the patch itself.
        let v = parse_dotted_version("git version 2.54.0.windows.1").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (2, 54, 0));
        let v = parse_dotted_version("git version 2.41.0-rc1").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (2, 41, 0));
        let v = parse_dotted_version("git version 2.54").unwrap();
        assert_eq!(v.patch, 0, "missing patch defaults to 0");
        // jj's banner is `jj 0.42.0`.
        let v = parse_dotted_version("jj 0.42.0").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (0, 42, 0));
        assert!(parse_dotted_version("no digits here").is_none());
        assert!(parse_dotted_version("git version unknowable").is_none());
    }

    #[test]
    fn orders_numerically() {
        let lo = parse_dotted_version("jj 0.38.0").unwrap();
        let hi = parse_dotted_version("jj 0.40.0").unwrap();
        assert!(hi > lo);
        assert!(
            Version {
                major: 2,
                minor: 9,
                patch: 0
            } < Version {
                major: 2,
                minor: 10,
                patch: 0
            }
        );
    }

    #[test]
    fn displays_dotted() {
        let v = parse_dotted_version("git version 2.54.1").unwrap();
        assert_eq!(v.to_string(), "2.54.1");
    }
}

// `parse_dotted_version` is a pure parser over an arbitrary `<tool> --version`
// banner (a binary on the user's machine), with byte-offset slicing in
// `leading_number` — so the load-bearing invariant is "never panic, whatever the
// bytes". Lock it against future edits.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn never_panics_on_arbitrary_text(s in any::<String>()) {
            let _ = parse_dotted_version(&s);
        }

        // …and on version-ish input that reaches the digit-run slicing branches.
        #[test]
        fn never_panics_on_versionish_text(s in r"[a-z]{0,6} ?[0-9.\-+a-z]{0,20}") {
            let _ = parse_dotted_version(&s);
        }
    }
}
