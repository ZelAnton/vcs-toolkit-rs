#!/usr/bin/env python3
"""Validate the v1 agent-interface corpus and hermetic result recordings.

This module deliberately uses only the Python standard library.  Validation is
an offline contract check: it never starts a model, contacts a forge, or reads
ambient credentials.  ``record.py`` imports the same functions so recording
and validation cannot silently disagree about the result envelope.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


EXPECTED_SCENARIOS = {
    "inspect_status",
    "changes_diff",
    "exact_path_commit",
    "publish_pr",
    "wait_ci",
    "conflict",
    "ordinary_file_search",
    "unsupported_low_level",
    "preferred_unavailable",
}
EXPECTED_SELECTIONS = {"preferred", "fallback", "none"}
EXPECTED_INTERFACES = {"vcs-agent", "mcp", "raw-cli", "none"}
MACHINE_OPERATIONS = {"probe", "inspect", "changes", "commit", "publish", "ci_status", "ci_wait", "unknown"}


class ValidationError(ValueError):
    """A human-readable contract violation."""


def load_json(path: Path) -> Any:
    try:
        with path.open("r", encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, json.JSONDecodeError) as exc:
        raise ValidationError(f"{path}: cannot read JSON: {exc}") from exc


def _object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValidationError(f"{label} must be an object")
    return value


def _require_fields(value: dict[str, Any], fields: tuple[str, ...], label: str) -> None:
    missing = [field for field in fields if field not in value]
    if missing:
        raise ValidationError(f"{label} is missing required fields: {', '.join(missing)}")


def _string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValidationError(f"{label} must be a non-empty string")
    return value


def _string_or_null(value: Any, label: str) -> None:
    if value is not None and not isinstance(value, str):
        raise ValidationError(f"{label} must be a string or null")


def _integer(value: Any, label: str) -> int:
    # bool is an int subclass in Python, but is not a call count.
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValidationError(f"{label} must be a non-negative integer")
    return value


def _integer_or_null(value: Any, label: str) -> None:
    if value is not None:
        _integer(value, label)


def _boolean(value: Any, label: str) -> bool:
    if not isinstance(value, bool):
        raise ValidationError(f"{label} must be boolean")
    return value


def _boolean_or_null(value: Any, label: str) -> None:
    if value is not None:
        _boolean(value, label)


def _machine_path(value: Any, label: str) -> None:
    path = _object(value, label)
    _require_fields(path, ("display", "encoding", "value"), label)
    display = path.get("display")
    if not isinstance(display, str):
        raise ValidationError(f"{label}.display must be a string")
    encoding = path.get("encoding")
    allowed = {"utf-8", "os-bytes-hex", "windows-utf16-hex", "platform-native-lossy", "redacted"}
    if encoding not in allowed:
        raise ValidationError(f"{label}.encoding is invalid")
    encoded = path.get("value")
    if encoding == "redacted":
        if encoded is not None:
            raise ValidationError(f"{label}.value must be null when redacted")
    elif not isinstance(encoded, str):
        raise ValidationError(f"{label}.value must be a string when not redacted")


def _read_semantics(value: Any, label: str) -> None:
    semantics = _object(value, label)
    _require_fields(
        semantics,
        (
            "refs_mutated",
            "index_mutated",
            "working_copy_content_mutated",
            "working_copy_snapshot",
            "operation_log_may_advance",
        ),
        label,
    )
    for key in ("refs_mutated", "index_mutated", "working_copy_content_mutated"):
        if semantics.get(key) is not False:
            raise ValidationError(f"{label}.{key} must be false for a read outcome")
    snapshot = semantics.get("working_copy_snapshot")
    may_advance = _boolean(semantics.get("operation_log_may_advance"), f"{label}.operation_log_may_advance")
    if snapshot == "none" and may_advance:
        raise ValidationError(f"{label}: a no-snapshot read cannot claim op-log advancement")
    if snapshot == "live-jj-snapshot" and not may_advance:
        raise ValidationError(f"{label}: live jj snapshot must disclose possible op-log advancement")
    if snapshot not in {"none", "live-jj-snapshot"}:
        raise ValidationError(f"{label}.working_copy_snapshot is invalid")


def _repository(value: Any, label: str) -> dict[str, Any]:
    repository = _object(value, label)
    _require_fields(repository, ("backend", "root", "cwd"), label)
    if repository["backend"] not in {"git", "jujutsu", "unknown"}:
        raise ValidationError(f"{label}.backend is invalid")
    _machine_path(repository["root"], f"{label}.root")
    _machine_path(repository["cwd"], f"{label}.cwd")
    return repository


def _forge_capabilities(value: Any, label: str) -> None:
    capabilities = _object(value, label)
    boolean_fields = (
        "cli_supported",
        "authenticated",
        "pr_create",
        "pr_comment",
        "pr_edit",
        "pr_labels",
        "pr_checks",
        "pr_merge",
        "pr_approve",
        "pr_request_changes",
        "issue_create",
        "issue_close",
        "issue_reopen",
        "issue_comment",
        "issue_labels",
        "release_create",
        "release_delete",
    )
    _require_fields(capabilities, ("cli_version", *boolean_fields), label)
    _string_or_null(capabilities["cli_version"], f"{label}.cli_version")
    for field in boolean_fields:
        _boolean(capabilities[field], f"{label}.{field}")


def _forge_auth(value: Any, label: str) -> None:
    auth = _object(value, label)
    _require_fields(
        auth,
        ("authenticated", "active_account", "accounts", "repository_visible"),
        label,
    )
    _boolean_or_null(auth["authenticated"], f"{label}.authenticated")
    _string_or_null(auth["active_account"], f"{label}.active_account")
    accounts = auth["accounts"]
    if not isinstance(accounts, list):
        raise ValidationError(f"{label}.accounts must be an array")
    for index, raw_account in enumerate(accounts):
        account_label = f"{label}.accounts[{index}]"
        account = _object(raw_account, account_label)
        _require_fields(account, ("host", "login", "active"), account_label)
        if not isinstance(account["host"], str):
            raise ValidationError(f"{account_label}.host must be a string")
        if not isinstance(account["login"], str):
            raise ValidationError(f"{account_label}.login must be a string")
        _boolean_or_null(account["active"], f"{account_label}.active")
    _boolean_or_null(auth["repository_visible"], f"{label}.repository_visible")


def _fact(value: Any, label: str, validate_payload: Any) -> None:
    fact = _object(value, label)
    _require_fields(fact, ("status", "reason", "value"), label)
    status = fact.get("status")
    reason = fact.get("reason")
    payload = fact.get("value")
    if status == "known":
        if reason is not None or not isinstance(payload, dict):
            raise ValidationError(f"{label}: known fact requires reason=null and an object value")
        validate_payload(payload, f"{label}.value")
    elif status == "unavailable":
        _string(reason, f"{label}.reason")
        if payload is not None:
            raise ValidationError(f"{label}: unavailable fact requires value=null")
    elif status == "not_applicable":
        if reason is not None or payload is not None:
            raise ValidationError(f"{label}: not_applicable fact requires reason=value=null")
    else:
        raise ValidationError(f"{label}.status is invalid")


def _changed_path(value: Any, label: str) -> None:
    changed = _object(value, label)
    _require_fields(changed, ("path", "old_path", "kind"), label)
    _machine_path(changed["path"], f"{label}.path")
    if changed["old_path"] is not None:
        _machine_path(changed["old_path"], f"{label}.old_path")
    if changed["kind"] not in {"added", "modified", "deleted", "renamed", "unknown"}:
        raise ValidationError(f"{label}.kind is invalid")


def _structured_diff(value: Any, label: str) -> None:
    changed = _object(value, label)
    _require_fields(changed, ("path", "old_path", "kind", "hunks"), label)
    _changed_path(
        {"path": changed["path"], "old_path": changed["old_path"], "kind": changed["kind"]},
        label,
    )
    hunks = changed["hunks"]
    if not isinstance(hunks, list):
        raise ValidationError(f"{label}.hunks must be an array")
    for hunk_index, raw_hunk in enumerate(hunks):
        hunk_label = f"{label}.hunks[{hunk_index}]"
        hunk = _object(raw_hunk, hunk_label)
        _require_fields(
            hunk,
            ("old_start", "old_lines", "new_start", "new_lines", "section", "lines"),
            hunk_label,
        )
        for field in ("old_start", "old_lines", "new_start", "new_lines"):
            _integer(hunk[field], f"{hunk_label}.{field}")
        if not isinstance(hunk["section"], str):
            raise ValidationError(f"{hunk_label}.section must be a string")
        lines = hunk["lines"]
        if not isinstance(lines, list):
            raise ValidationError(f"{hunk_label}.lines must be an array")
        for line_index, raw_line in enumerate(lines):
            line_label = f"{hunk_label}.lines[{line_index}]"
            line = _object(raw_line, line_label)
            _require_fields(line, ("kind", "text"), line_label)
            if line["kind"] not in {"context", "added", "removed", "unknown"}:
                raise ValidationError(f"{line_label}.kind is invalid")
            if not isinstance(line["text"], str):
                raise ValidationError(f"{line_label}.text must be a string")


def validate_machine_envelope(value: Any, label: str) -> dict[str, Any]:
    envelope = _object(value, label)
    _require_fields(
        envelope,
        (
            "contract_version",
            "binary_version",
            "operation",
            "status",
            "data",
            "error",
            "warnings",
            "fallback",
        ),
        label,
    )
    if envelope.get("contract_version") != "vcs-agent/v1":
        raise ValidationError(f"{label}.contract_version must be vcs-agent/v1")
    binary_version = _string(envelope.get("binary_version"), f"{label}.binary_version")
    if re.match(r"^[0-9]+\.[0-9]+\.[0-9]+", binary_version) is None:
        raise ValidationError(f"{label}.binary_version is invalid")
    operation = _string(envelope.get("operation"), f"{label}.operation")
    if operation not in MACHINE_OPERATIONS:
        raise ValidationError(f"{label}.operation is not a v1 operation")
    status = envelope.get("status")
    if status not in {"success", "error"}:
        raise ValidationError(f"{label}.status is invalid")
    warnings = envelope.get("warnings")
    if not isinstance(warnings, list) or not all(isinstance(item, str) for item in warnings):
        raise ValidationError(f"{label}.warnings must be a string array")

    if status == "error":
        if envelope.get("data") is not None:
            raise ValidationError(f"{label}: error envelope must use data=null")
        error = _object(envelope.get("error"), f"{label}.error")
        _require_fields(
            error,
            ("kind", "exit_code", "code", "message", "retryable", "details"),
            f"{label}.error",
        )
        kind = _string(error.get("kind"), f"{label}.error.kind")
        if re.fullmatch(r"[a-z][a-z0-9_]*", kind) is None:
            raise ValidationError(f"{label}.error.kind is invalid")
        exit_code = error.get("exit_code")
        if isinstance(exit_code, bool) or not isinstance(exit_code, int) or not 2 <= exit_code <= 79:
            raise ValidationError(f"{label}.error.exit_code is invalid")
        _string(error.get("code"), f"{label}.error.code")
        if not isinstance(error.get("message"), str):
            raise ValidationError(f"{label}.error.message must be a string")
        _boolean(error.get("retryable"), f"{label}.error.retryable")
        details = _object(error.get("details"), f"{label}.error.details")
        if not all(isinstance(key, str) and isinstance(item, str) for key, item in details.items()):
            raise ValidationError(f"{label}.error.details values must be strings")
        fallback = envelope.get("fallback")
        if fallback is not None:
            fallback = _object(fallback, f"{label}.fallback")
            _require_fields(fallback, ("allowed", "interface", "reason"), f"{label}.fallback")
            _boolean(fallback["allowed"], f"{label}.fallback.allowed")
            _string(fallback["interface"], f"{label}.fallback.interface")
            _string(fallback["reason"], f"{label}.fallback.reason")
        return envelope

    if envelope.get("error") is not None or envelope.get("fallback") is not None:
        raise ValidationError(f"{label}: success envelope cannot carry error/fallback")
    data = _object(envelope.get("data"), f"{label}.data")
    if operation == "inspect":
        _require_fields(
            data,
            ("repository", "working_copy", "remotes", "forge", "capabilities", "read_semantics"),
            f"{label}.data",
        )
        repository = _repository(data["repository"], f"{label}.data.repository")
        working = _object(data["working_copy"], f"{label}.data.working_copy")
        working_fields = (
            "branch_kind",
            "branch",
            "revision",
            "change_id",
            "dirty",
            "tracked_changes",
            "untracked",
            "total_changes",
            "conflicted",
            "conflict_count",
            "operation",
            "upstream",
        )
        _require_fields(working, working_fields, f"{label}.data.working_copy")
        if working["branch_kind"] not in {"branch", "bookmark"}:
            raise ValidationError(f"{label}.data.working_copy.branch_kind is invalid")
        for field in ("branch", "revision", "change_id"):
            _string_or_null(working[field], f"{label}.data.working_copy.{field}")
        _boolean(working["dirty"], f"{label}.data.working_copy.dirty")
        for field in ("tracked_changes", "untracked", "conflict_count"):
            _integer_or_null(working[field], f"{label}.data.working_copy.{field}")
        _integer(working["total_changes"], f"{label}.data.working_copy.total_changes")
        _boolean(working["conflicted"], f"{label}.data.working_copy.conflicted")
        if working["operation"] not in {
            "clear", "merge", "rebase", "apply_mailbox", "cherry_pick", "revert", "bisect", "conflict", "unknown"
        }:
            raise ValidationError(f"{label}.data.working_copy.operation is invalid")
        upstream = working["upstream"]
        if upstream is not None:
            upstream = _object(upstream, f"{label}.data.working_copy.upstream")
            _require_fields(upstream, ("branch", "ahead", "behind"), f"{label}.data.working_copy.upstream")
            if not isinstance(upstream["branch"], str):
                raise ValidationError(f"{label}.data.working_copy.upstream.branch must be a string")
            _integer_or_null(upstream["ahead"], f"{label}.data.working_copy.upstream.ahead")
            _integer_or_null(upstream["behind"], f"{label}.data.working_copy.upstream.behind")

        remotes = data["remotes"]
        if not isinstance(remotes, list):
            raise ValidationError(f"{label}.data.remotes must be an array")
        for index, raw_remote in enumerate(remotes):
            remote_label = f"{label}.data.remotes[{index}]"
            remote = _object(raw_remote, remote_label)
            _require_fields(remote, ("name", "url"), remote_label)
            if not isinstance(remote["name"], str) or not isinstance(remote["url"], str):
                raise ValidationError(f"{remote_label}.name and url must be strings")

        forge = _object(data["forge"], f"{label}.data.forge")
        _require_fields(forge, ("detection", "kind", "remote", "capabilities", "auth"), f"{label}.data.forge")
        if forge["detection"] not in {"absent", "detected"}:
            raise ValidationError(f"{label}.data.forge.detection is invalid")
        if forge["kind"] not in {"github", "gitlab", "gitea", "unknown", None}:
            raise ValidationError(f"{label}.data.forge.kind is invalid")
        _string_or_null(forge["remote"], f"{label}.data.forge.remote")
        _fact(forge["capabilities"], f"{label}.data.forge.capabilities", _forge_capabilities)
        _fact(forge["auth"], f"{label}.data.forge.auth", _forge_auth)

        capabilities = _object(data["capabilities"], f"{label}.data.capabilities")
        capability_fields = (
            "inspect", "changes_summary", "changes_full", "lossless_status_paths",
            "full_diff_non_utf8_paths", "raw_cli_fallback",
        )
        _require_fields(capabilities, capability_fields, f"{label}.data.capabilities")
        for field in ("inspect", "changes_summary", "changes_full", "lossless_status_paths"):
            if capabilities[field] is not True:
                raise ValidationError(f"{label}.data.capabilities.{field} must be true")
        if capabilities["full_diff_non_utf8_paths"] != "git-lossless-jj-text-limited":
            raise ValidationError(f"{label}.data.capabilities.full_diff_non_utf8_paths is invalid")
        if capabilities["raw_cli_fallback"] is not False:
            raise ValidationError(f"{label}.data.capabilities.raw_cli_fallback must be false")

        _read_semantics(data["read_semantics"], f"{label}.data.read_semantics")
        snapshot = data["read_semantics"]["working_copy_snapshot"]
        if (repository["backend"] == "git" and snapshot != "none") or (
            repository["backend"] == "jujutsu" and snapshot != "live-jj-snapshot"
        ):
            raise ValidationError(f"{label}: backend and read snapshot semantics disagree")
    elif operation == "changes":
        _require_fields(
            data,
            ("repository", "mode", "content_max_bytes", "counts", "files", "diff", "read_semantics"),
            f"{label}.data",
        )
        repository = _repository(data["repository"], f"{label}.data.repository")
        mode = data["mode"]
        if mode not in {"summary", "full"}:
            raise ValidationError(f"{label}.data.mode is invalid")
        content_max = _integer(data["content_max_bytes"], f"{label}.data.content_max_bytes")
        if not 1024 <= content_max <= 1048576:
            raise ValidationError(f"{label}.data.content_max_bytes is out of range")
        counts = _object(data["counts"], f"{label}.data.counts")
        count_fields = ("paths", "added", "modified", "deleted", "renamed", "files_with_line_diff", "insertions", "deletions")
        _require_fields(counts, count_fields, f"{label}.data.counts")
        for field in count_fields:
            _integer(counts[field], f"{label}.data.counts.{field}")

        files = data["files"]
        if not isinstance(files, list):
            raise ValidationError(f"{label}.data.files must be an array")
        for index, item in enumerate(files):
            _changed_path(item, f"{label}.data.files[{index}]")
        diff = data["diff"]
        if (mode == "summary" and diff is not None) or (mode == "full" and not isinstance(diff, list)):
            raise ValidationError(f"{label}.data.diff does not match mode")
        if isinstance(diff, list):
            for index, item in enumerate(diff):
                _structured_diff(item, f"{label}.data.diff[{index}]")
        _read_semantics(data["read_semantics"], f"{label}.data.read_semantics")
        snapshot = data["read_semantics"]["working_copy_snapshot"]
        if (repository.get("backend") == "git" and snapshot != "none") or (
            repository.get("backend") == "jujutsu" and snapshot != "live-jj-snapshot"
        ):
            raise ValidationError(f"{label}: backend and read snapshot semantics disagree")
    return envelope


def validate_machine_fixtures(fixtures_dir: Path) -> list[dict[str, Any]]:
    paths = sorted(fixtures_dir.glob("*.v1.json"))
    if not paths:
        raise ValidationError(f"{fixtures_dir}: no machine fixtures found")
    return [validate_machine_envelope(load_json(path), str(path)) for path in paths]


def validate_corpus(corpus: Any) -> dict[str, dict[str, Any]]:
    root = _object(corpus, "corpus")
    if root.get("schema_version") != "agent-interface.corpus.v1":
        raise ValidationError("corpus.schema_version must be agent-interface.corpus.v1")
    _string(root.get("corpus_version"), "corpus.corpus_version")
    policy = _object(root.get("selection_policy"), "corpus.selection_policy")
    if policy.get("preferred_interface") != "vcs-agent":
        raise ValidationError("selection_policy.preferred_interface must be vcs-agent")
    fallbacks = policy.get("fallback_interfaces")
    if fallbacks != ["mcp", "raw-cli"]:
        raise ValidationError("selection_policy.fallback_interfaces must be [mcp, raw-cli]")
    cases = root.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ValidationError("corpus.cases must be a non-empty array")

    seen: set[str] = set()
    by_id: dict[str, dict[str, Any]] = {}
    for index, raw_case in enumerate(cases):
        case = _object(raw_case, f"corpus.cases[{index}]")
        case_id = _string(case.get("case_id"), f"corpus.cases[{index}].case_id")
        if case_id in seen:
            raise ValidationError(f"duplicate case_id: {case_id}")
        seen.add(case_id)
        scenario = _string(case.get("scenario"), f"{case_id}.scenario")
        if scenario not in EXPECTED_SCENARIOS:
            raise ValidationError(f"{case_id}.scenario is not a known v1 scenario: {scenario}")
        _object(case.get("request"), f"{case_id}.request")
        expected = _object(case.get("expected"), f"{case_id}.expected")
        selection = expected.get("selection")
        if selection not in EXPECTED_SELECTIONS:
            raise ValidationError(f"{case_id}.expected.selection is invalid")
        _string(expected.get("operation"), f"{case_id}.expected.operation")
        fallback = _object(expected.get("fallback"), f"{case_id}.expected.fallback")
        _boolean(fallback.get("allowed"), f"{case_id}.expected.fallback.allowed")
        interfaces = fallback.get("interfaces")
        reasons = fallback.get("reasons")
        if not isinstance(interfaces, list) or not all(isinstance(item, str) for item in interfaces):
            raise ValidationError(f"{case_id}.expected.fallback.interfaces must be a string array")
        if not isinstance(reasons, list) or not all(isinstance(item, str) for item in reasons):
            raise ValidationError(f"{case_id}.expected.fallback.reasons must be a string array")
        if selection == "preferred" and fallback["allowed"]:
            raise ValidationError(f"{case_id}: preferred selection cannot allow fallback")
        if selection == "none" and fallback["allowed"]:
            raise ValidationError(f"{case_id}: negative selection cannot allow fallback")
        invariants = _object(expected.get("invariants"), f"{case_id}.expected.invariants")
        calls = _object(invariants.get("preferred_calls"), f"{case_id}.preferred_calls")
        low = _integer(calls.get("min"), f"{case_id}.preferred_calls.min")
        high = _integer(calls.get("max"), f"{case_id}.preferred_calls.max")
        if low > high:
            raise ValidationError(f"{case_id}: preferred_calls.min exceeds max")
        _integer(invariants.get("raw_cli_calls"), f"{case_id}.raw_cli_calls")
        _boolean(invariants.get("unrelated_changes_preserved"), f"{case_id}.unrelated_changes_preserved")
        if invariants.get("exact_revision_evidence") not in {"required", "not_required"}:
            raise ValidationError(f"{case_id}.exact_revision_evidence is invalid")
        if invariants.get("terminal_ci") not in {"required", "not_required"}:
            raise ValidationError(f"{case_id}.terminal_ci is invalid")
        by_id[case_id] = case

    missing = EXPECTED_SCENARIOS - {case["scenario"] for case in by_id.values()}
    if missing:
        raise ValidationError(f"corpus is missing scenarios: {', '.join(sorted(missing))}")
    return by_id


def _validate_result_shape(result: Any, label: str) -> dict[str, Any]:
    result = _object(result, label)
    if result.get("schema_version") != "agent-interface.result.v1":
        raise ValidationError(f"{label}.schema_version must be agent-interface.result.v1")
    case_id = _string(result.get("case_id"), f"{label}.case_id")
    outcome = _object(result.get("outcome"), f"{label}.outcome")
    _string(outcome.get("status"), f"{label}.outcome.status")
    selection = _object(result.get("selection"), f"{label}.selection")
    if selection.get("selected_interface") not in EXPECTED_INTERFACES:
        raise ValidationError(f"{label}.selection.selected_interface is invalid")
    for key in ("preferred_interface_selected", "false_activation", "raw_cli_bypass"):
        _boolean(selection.get(key), f"{label}.selection.{key}")
    reason = selection.get("fallback_reason")
    if reason is not None:
        _string(reason, f"{label}.selection.fallback_reason")
    calls = _object(result.get("calls"), f"{label}.calls")
    for key, value in calls.items():
        _integer(value, f"{label}.calls.{key}")
    for key in ("preferred_interface", "fallback_interface", "raw_cli", "total"):
        if key not in calls:
            raise ValidationError(f"{label}.calls.{key} is required")
    total = sum(value for key, value in calls.items() if key != "total")
    if calls["total"] != total:
        raise ValidationError(f"{label}.calls.total must equal the sum of interface calls")
    workspace = _object(result.get("workspace"), f"{label}.workspace")
    _boolean(workspace.get("unrelated_changes_preserved"), f"{label}.workspace.unrelated_changes_preserved")
    revision = _object(result.get("revision"), f"{label}.revision")
    _boolean(revision.get("exact_revision_verified"), f"{label}.revision.exact_revision_verified")
    terminal_ci = _object(revision.get("terminal_ci"), f"{label}.revision.terminal_ci")
    _boolean(terminal_ci.get("verified"), f"{label}.revision.terminal_ci.verified")
    if terminal_ci.get("conclusion") is not None:
        _string(terminal_ci["conclusion"], f"{label}.revision.terminal_ci.conclusion")
    for key in ("before", "after", "published"):
        if key in revision and revision[key] is not None:
            _string(revision[key], f"{label}.revision.{key}")
    return result


def validate_results(corpus_by_id: dict[str, dict[str, Any]], results: Any) -> list[dict[str, Any]]:
    root = _object(results, "results")
    if root.get("schema_version") != "agent-interface.results.v1":
        raise ValidationError("results.schema_version must be agent-interface.results.v1")
    raw_results = root.get("results")
    if not isinstance(raw_results, list):
        raise ValidationError("results.results must be an array")
    seen: set[str] = set()
    checked: list[dict[str, Any]] = []
    for index, raw_result in enumerate(raw_results):
        result = _validate_result_shape(raw_result, f"results.results[{index}]")
        case_id = result["case_id"]
        if case_id not in corpus_by_id:
            raise ValidationError(f"result references unknown case_id: {case_id}")
        if case_id in seen:
            raise ValidationError(f"duplicate result case_id: {case_id}")
        seen.add(case_id)
        case = corpus_by_id[case_id]
        expected = case["expected"]
        invariants = expected["invariants"]
        selection = result["selection"]
        calls = result["calls"]
        expected_selection = expected["selection"]

        # These flags are evidence derived from the actual route, not free-form
        # annotations.  Keep the relationship explicit so a recording cannot
        # claim a false activation or raw-CLI bypass that its calls do not show.
        fallback_calls = calls.get("fallback_interface", 0)
        selected_interface = selection["selected_interface"]
        if selected_interface == "vcs-agent":
            if not selection["preferred_interface_selected"]:
                raise ValidationError(f"{case_id}: vcs-agent selection must set preferred_interface_selected")
            if selection["false_activation"] or selection["raw_cli_bypass"]:
                raise ValidationError(f"{case_id}: preferred selection cannot claim false activation or raw CLI bypass")
            if selection["fallback_reason"] is not None or calls["raw_cli"] or fallback_calls:
                raise ValidationError(f"{case_id}: preferred selection has fallback or raw CLI call evidence")
            if result["outcome"]["status"] != "success":
                raise ValidationError(f"{case_id}: preferred selection must have a success outcome")
        elif selected_interface == "mcp":
            if selection["preferred_interface_selected"] or selection["false_activation"]:
                raise ValidationError(f"{case_id}: MCP fallback cannot claim preferred selection or false activation")
            if selection["raw_cli_bypass"] or calls["raw_cli"] or fallback_calls < 1:
                raise ValidationError(f"{case_id}: MCP fallback must have fallback calls and no raw CLI bypass")
            if selection["fallback_reason"] is None or result["outcome"]["status"] != "fallback":
                raise ValidationError(f"{case_id}: MCP fallback requires a classified fallback outcome")
        elif selected_interface == "raw-cli":
            if selection["preferred_interface_selected"] or selection["false_activation"]:
                raise ValidationError(f"{case_id}: raw CLI fallback cannot claim preferred selection or false activation")
            if not selection["raw_cli_bypass"] or calls["raw_cli"] < 1 or selection["fallback_reason"] is None:
                raise ValidationError(f"{case_id}: raw CLI fallback must have classified raw CLI evidence")
            if result["outcome"]["status"] != "fallback":
                raise ValidationError(f"{case_id}: raw CLI fallback must have a fallback outcome")
        else:
            if selection["preferred_interface_selected"] or selection["false_activation"] or selection["raw_cli_bypass"]:
                raise ValidationError(f"{case_id}: none selection cannot claim activation or bypass evidence")
            if selection["fallback_reason"] is not None or calls["total"] != 0:
                raise ValidationError(f"{case_id}: none selection must have no fallback reason or calls")
            if result["outcome"]["status"] != "ignored":
                raise ValidationError(f"{case_id}: none selection must have an ignored outcome")

        if expected_selection == "preferred":
            if selection["selected_interface"] != "vcs-agent" or not selection["preferred_interface_selected"]:
                raise ValidationError(f"{case_id}: preferred interface was not selected")
            if selection["fallback_reason"] is not None or selection["raw_cli_bypass"]:
                raise ValidationError(f"{case_id}: preferred result has fallback/bypass evidence")
        elif expected_selection == "none":
            if selection["selected_interface"] != "none" or selection["preferred_interface_selected"]:
                raise ValidationError(f"{case_id}: negative prompt activated an interface")
            if calls["total"] != 0 or selection["fallback_reason"] is not None or selection["raw_cli_bypass"]:
                raise ValidationError(f"{case_id}: negative prompt has call or fallback evidence")
        else:
            fallback = expected["fallback"]
            if selection["selected_interface"] not in fallback["interfaces"]:
                raise ValidationError(f"{case_id}: selected fallback is not allowed")
            if selection["fallback_reason"] not in fallback["reasons"]:
                raise ValidationError(f"{case_id}: fallback reason is not classified")
            if selection["preferred_interface_selected"]:
                raise ValidationError(f"{case_id}: fallback result claims preferred selection")
            if selection["selected_interface"] == "raw-cli" and not selection["raw_cli_bypass"]:
                raise ValidationError(f"{case_id}: raw-cli fallback must be marked as a bypass")
        preferred_calls = invariants["preferred_calls"]
        if not preferred_calls["min"] <= calls["preferred_interface"] <= preferred_calls["max"]:
            raise ValidationError(f"{case_id}: preferred call count is outside the corpus bound")
        if calls["raw_cli"] != invariants["raw_cli_calls"]:
            raise ValidationError(f"{case_id}: raw CLI call count differs from the corpus")
        if result["outcome"]["status"] != expected["outcome"]:
            raise ValidationError(f"{case_id}: outcome status differs from the corpus")
        if invariants["unrelated_changes_preserved"] and not result["workspace"]["unrelated_changes_preserved"]:
            raise ValidationError(f"{case_id}: unrelated workspace changes were not preserved")
        if invariants["exact_revision_evidence"] == "required":
            revision = result["revision"]
            if not revision["exact_revision_verified"] or not revision.get("after"):
                raise ValidationError(f"{case_id}: exact revision evidence is required")
            if expected["operation"] == "publish":
                if not revision.get("published") or revision["published"] != revision["after"]:
                    raise ValidationError(f"{case_id}: published revision must exactly match the local after revision")
        if invariants["terminal_ci"] == "required":
            terminal_ci = result["revision"]["terminal_ci"]
            if not terminal_ci["verified"] or terminal_ci.get("conclusion") != "success":
                raise ValidationError(f"{case_id}: terminal exact-revision CI evidence is required")
            expected_revision = result["revision"].get("published") or result["revision"].get("after")
            if terminal_ci.get("revision") != expected_revision:
                raise ValidationError(f"{case_id}: terminal CI must reference the exact published revision")
        checked.append(result)
    missing = sorted(set(corpus_by_id) - seen)
    if missing:
        raise ValidationError(f"results missing case IDs: {', '.join(missing)}")
    return checked


def validate_baseline(baseline: Any) -> dict[str, Any]:
    root = _object(baseline, "baseline")
    if root.get("schema_version") != "agent-interface.baseline.v1":
        raise ValidationError("baseline.schema_version must be agent-interface.baseline.v1")
    _string(root.get("interface"), "baseline.interface")
    harness = _object(root.get("harness"), "baseline.harness")
    _string(harness.get("kind"), "baseline.harness.kind")
    _string(harness.get("availability"), "baseline.harness.availability")
    _string(root.get("status"), "baseline.status")
    if root["status"] == "no_data" and root.get("metrics") is not None:
        raise ValidationError("baseline.no_data must use metrics=null, never zero-valued metrics")
    return root


def validate_files(corpus_path: Path, results_path: Path | None, baseline_path: Path | None, machine_fixtures: Path | None = None) -> dict[str, Any]:
    corpus_by_id = validate_corpus(load_json(corpus_path))
    checked_results: list[dict[str, Any]] = []
    if results_path is not None:
        checked_results = validate_results(corpus_by_id, load_json(results_path))
    baseline = None
    if baseline_path is not None:
        baseline = validate_baseline(load_json(baseline_path))
    checked_machine = validate_machine_fixtures(machine_fixtures) if machine_fixtures is not None else []
    return {"corpus_cases": len(corpus_by_id), "result_cases": len(checked_results), "machine_fixtures": len(checked_machine), "baseline_status": baseline["status"] if baseline else None}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path(__file__).resolve().parents[2]
    parser.add_argument("--corpus", type=Path, default=root / "docs/agent-interface/corpus.v1.json")
    parser.add_argument("--results", type=Path)
    parser.add_argument("--baseline", type=Path, default=root / "docs/agent-interface/baseline-mcp.v1.json")
    parser.add_argument("--machine-fixtures", type=Path, default=root / "crates/agent/tests/fixtures")
    args = parser.parse_args(argv)
    try:
        summary = validate_files(args.corpus, args.results, args.baseline, args.machine_fixtures)
    except ValidationError as exc:
        print(f"agent-interface validation failed: {exc}", file=sys.stderr)
        return 1
    print(json.dumps({"valid": True, **summary}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
