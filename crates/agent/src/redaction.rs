#[derive(Clone, Copy)]
pub(crate) struct RedactionPolicy {
    pub(crate) include_machine_paths: bool,
}

pub(crate) fn redact_text(input: &str, policy: RedactionPolicy) -> String {
    let credential_safe = redact_credentialed_urls(input);
    let secret_safe = redact_secret_assignments(&credential_safe);
    if policy.include_machine_paths {
        secret_safe
    } else {
        redact_machine_paths(&secret_safe)
    }
}

fn redact_credentialed_urls(input: &str) -> String {
    let mut result = input.to_owned();
    for scheme in ["https://", "http://"] {
        let mut offset = 0;
        while let Some(scheme_start) = find_ascii_case_insensitive(&result, scheme, offset) {
            let authority_start = scheme_start + scheme.len();
            let authority_end = result[authority_start..]
                .find(['/', '?', '#', ' ', '\t', '\r', '\n'])
                .map_or(result.len(), |relative| authority_start + relative);
            let Some(relative_at) = result[authority_start..authority_end].rfind('@') else {
                offset = authority_end;
                continue;
            };
            let at = authority_start + relative_at;
            result.replace_range(authority_start..at, "[REDACTED]");
            offset = authority_start + "[REDACTED]@".len();
        }
    }
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
                if is_secret_key(key) {
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
                    } else if key.eq_ignore_ascii_case("authorization") {
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
    redact_bearer(&output)
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "token" | "password" | "passwd" | "secret" | "authorization" | "api_key" | "apikey"
    ) || key.ends_with("_token")
        || key.ends_with("_password")
        || key.ends_with("_secret")
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
