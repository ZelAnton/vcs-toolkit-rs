use std::ffi::OsString;

use vcs_cli_support::logging::redact_args;

#[derive(Clone, Copy)]
pub(crate) struct RedactionPolicy {
    pub(crate) include_machine_paths: bool,
}

pub(crate) fn redact_text(input: &str, policy: RedactionPolicy) -> String {
    let bearer_safe = redact_bearer(input);
    let assignment_safe = redact_secret_assignments(&bearer_safe);
    let secret_safe = redact_cli_values(&assignment_safe);
    if policy.include_machine_paths {
        secret_safe
    } else {
        redact_machine_paths(&secret_safe)
    }
}

/// Apply the workspace's shared fail-closed argv/value policy while retaining
/// the whitespace of a diagnostic. Agent diagnostics are free text rather than
/// an argv, but splitting only on whitespace lets the shared sequence-aware
/// primitive cover sensitive flags without inventing a second URL/token parser.
fn redact_cli_values(input: &str) -> String {
    let words: Vec<&str> = input.split_whitespace().collect();
    if words.is_empty() {
        return input.to_owned();
    }

    let args: Vec<OsString> = words.iter().map(OsString::from).collect();
    let redacted = redact_args(&args);
    debug_assert_eq!(words.len(), redacted.len());

    let mut result = String::with_capacity(input.len());
    let mut cursor = 0;
    for (word, safe) in words.into_iter().zip(redacted) {
        let relative = input[cursor..]
            .find(word)
            .expect("split word remains in the source text");
        let start = cursor + relative;
        result.push_str(&input[cursor..start]);
        result.push_str(&safe.replace("<redacted>", "[REDACTED]"));
        cursor = start + word.len();
    }
    result.push_str(&input[cursor..]);
    result
}

fn redact_secret_assignments(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let key_start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-'))
        {
            index += 1;
        }
        if index > key_start {
            let key = &input[key_start..index];
            output.push_str(key);
            let mut separator_end = index;
            while separator_end < bytes.len() && matches!(bytes[separator_end], b' ' | b'\t') {
                separator_end += 1;
            }
            if separator_end < bytes.len() && matches!(bytes[separator_end], b'=' | b':') {
                separator_end += 1;
                while separator_end < bytes.len() && matches!(bytes[separator_end], b' ' | b'\t') {
                    separator_end += 1;
                }
                output.push_str(&input[index..separator_end]);
                let normalized_key = key.trim_start_matches('-');
                if is_secret_key(normalized_key) {
                    let mut value_end = separator_end;
                    if bytes
                        .get(separator_end)
                        .is_some_and(|byte| matches!(byte, b'\'' | b'"' | b'`'))
                    {
                        let quote = bytes[separator_end];
                        value_end += 1;
                        while value_end < bytes.len() && bytes[value_end] != quote {
                            value_end += 1;
                        }
                        if value_end < bytes.len() {
                            value_end += 1;
                        }
                    } else if normalized_key.eq_ignore_ascii_case("authorization") {
                        while value_end < bytes.len()
                            && !matches!(bytes[value_end], b'\r' | b'\n' | b',' | b';')
                        {
                            value_end += 1;
                        }
                    } else {
                        while value_end < bytes.len()
                            && !matches!(
                                bytes[value_end],
                                b' ' | b'\t' | b'\r' | b'\n' | b',' | b';'
                            )
                        {
                            value_end += 1;
                        }
                    }
                    output.push_str("[REDACTED]");
                    index = value_end;
                    continue;
                }
                index = separator_end;
                continue;
            }
            continue;
        }
        let ch = input[index..]
            .chars()
            .next()
            .expect("index is on a char boundary");
        output.push(ch);
        index += ch.len_utf8();
    }
    output
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "token"
            | "password"
            | "passwd"
            | "secret"
            | "authorization"
            | "api_key"
            | "api-key"
            | "apikey"
    ) || key.ends_with("_token")
        || key.ends_with("_password")
        || key.ends_with("_secret")
        || key.ends_with("-token")
        || key.ends_with("-password")
        || key.ends_with("-secret")
}

fn redact_bearer(input: &str) -> String {
    let mut result = input.to_owned();
    let mut offset = 0;
    while let Some(start) = find_ascii_case_insensitive(&result, "bearer ", offset) {
        let value_start = start + "bearer ".len();
        let value_end = result[value_start..]
            .find(|ch: char| ch.is_ascii_whitespace() || matches!(ch, ',' | ';'))
            .map_or(result.len(), |relative| value_start + relative);
        result.replace_range(value_start..value_end, "[REDACTED]");
        offset = value_start + "[REDACTED]".len();
    }
    result
}

fn find_ascii_case_insensitive(input: &str, needle: &str, start: usize) -> Option<usize> {
    input.as_bytes()[start..]
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
        .map(|relative| start + relative)
}

fn redact_machine_paths(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for token in input.split_inclusive(|ch: char| ch.is_ascii_whitespace()) {
        let trimmed = token.trim_end_matches(|ch: char| ch.is_ascii_whitespace());
        let suffix = &token[trimmed.len()..];
        if let Some(start) = machine_path_start(trimmed) {
            result.push_str(&trimmed[..start]);
            result.push_str("[REDACTED_PATH]");
        } else {
            result.push_str(trimmed);
        }
        result.push_str(suffix);
    }
    result
}

fn machine_path_start(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let file_uri = file_uri_start(value);
    let windows = bytes.windows(3).enumerate().find_map(|(index, window)| {
        (window[0].is_ascii_alphabetic()
            && window[1] == b':'
            && matches!(window[2], b'\\' | b'/')
            && is_path_boundary(value, index))
        .then_some(index)
    });
    let unc = value.find(r"\\");
    let unix = value.char_indices().find_map(|(index, ch)| {
        if ch != '/' {
            return None;
        }
        let is_url_separator = value
            .as_bytes()
            .get(index.saturating_sub(1)..index + 2)
            .is_some_and(|window| window == b"://");
        (is_path_boundary(value, index) && !is_url_separator).then_some(index)
    });
    file_uri
        .into_iter()
        .chain(windows)
        .chain(unc)
        .chain(unix)
        .min()
}

fn is_path_boundary(value: &str, index: usize) -> bool {
    index == 0
        || value[..index]
            .chars()
            .next_back()
            .is_some_and(|ch| matches!(ch, '=' | ':' | '(' | '[' | '{' | '\'' | '"' | '`'))
}

fn file_uri_start(value: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(start) = find_ascii_case_insensitive(value, "file://", offset) {
        let scheme_boundary = start == 0
            || value[..start]
                .chars()
                .next_back()
                .is_some_and(|ch| !ch.is_ascii_alphanumeric() && !matches!(ch, '+' | '-' | '.'));
        if scheme_boundary {
            return Some(start);
        }
        offset = start + "file://".len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials_without_destroying_remote_identity() {
        let value = redact_text(
            "remote=HTTPS://alice:ghp_secret@example.invalid/owner/repo token=abc",
            RedactionPolicy {
                include_machine_paths: true,
            },
        );
        assert_eq!(
            value,
            "remote=HTTPS://[REDACTED]@example.invalid/owner/repo token=[REDACTED]"
        );
    }

    #[test]
    fn shared_fail_closed_policy_covers_uri_flags_and_pat_shapes() {
        let value = redact_text(
            "ssh://alice:uri-secret@example.invalid/repo --token flag-secret --password=password-secret --api-key=api-secret github_pat_PAT_SECRET",
            RedactionPolicy {
                include_machine_paths: true,
            },
        );

        for leaked in [
            "alice:uri-secret",
            "flag-secret",
            "password-secret",
            "api-secret",
            "github_pat_PAT_SECRET",
        ] {
            assert!(
                !value.contains(leaked),
                "redaction leaked {leaked}: {value}"
            );
        }
        assert!(value.contains("ssh://[REDACTED]@example.invalid/repo"));
        assert!(value.contains("--token [REDACTED]"));
        assert!(value.contains("--password=[REDACTED]"));
        assert!(value.contains("--api-key=[REDACTED]"));
    }

    #[test]
    fn redacts_windows_and_unix_machine_paths_by_default() {
        for path in [
            r"C:\Users\alice\repo",
            r"\\server\share\repo",
            "/workspaces/alice/repo",
            "/tmp/repo",
        ] {
            assert_eq!(
                redact_text(
                    path,
                    RedactionPolicy {
                        include_machine_paths: false,
                    }
                ),
                "[REDACTED_PATH]"
            );
        }
    }

    #[test]
    fn redacts_file_uris_without_touching_network_remotes() {
        let value = redact_text(
            "local=file:///workspaces/alice/repo windows=FILE:///C:/Users/alice/repo unc=file://server/share/repo https=https://example.invalid/owner/repo ssh=ssh://git@example.invalid/owner/repo scp=git@example.invalid:owner/repo",
            RedactionPolicy {
                include_machine_paths: false,
            },
        );
        assert!(!value.contains("alice"));
        assert!(!value.contains("server"));
        assert_eq!(value.matches("[REDACTED_PATH]").count(), 3);
        assert!(value.contains("https=https://example.invalid/owner/repo"));
        assert!(value.contains("ssh=ssh://git@example.invalid/owner/repo"));
        assert!(value.contains("scp=git@example.invalid:owner/repo"));
    }

    #[test]
    fn explicit_machine_path_opt_in_preserves_paths_but_not_secrets() {
        let value = redact_text(
            r"C:\Users\alice\repo password=hunter2",
            RedactionPolicy {
                include_machine_paths: true,
            },
        );
        assert!(value.contains(r"C:\Users\alice\repo"));
        assert!(!value.contains("hunter2"));
    }

    #[test]
    fn redaction_preserves_unicode_and_finds_embedded_paths_and_multiple_bearers() {
        let value = redact_text(
            "ошибка path=C:\\Users\\алиса\\repo Bearer one, bearer two GITHUB_TOKEN='very secret'",
            RedactionPolicy {
                include_machine_paths: false,
            },
        );
        assert!(value.starts_with("ошибка path=[REDACTED_PATH]"));
        assert!(!value.contains("алиса"));
        assert!(!value.contains("one"));
        assert!(!value.contains("two"));
        assert!(!value.contains("very secret"));
        assert_eq!(value.matches("[REDACTED]").count(), 3);
    }
}
