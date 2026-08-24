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


def _string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValidationError(f"{label} must be a non-empty string")
    return value


def _integer(value: Any, label: str) -> int:
    # bool is an int subclass in Python, but is not a call count.
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValidationError(f"{label} must be a non-negative integer")
    return value


def _boolean(value: Any, label: str) -> bool:
    if not isinstance(value, bool):
        raise ValidationError(f"{label} must be boolean")
    return value


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


def validate_files(corpus_path: Path, results_path: Path | None, baseline_path: Path | None) -> dict[str, Any]:
    corpus_by_id = validate_corpus(load_json(corpus_path))
    checked_results: list[dict[str, Any]] = []
    if results_path is not None:
        checked_results = validate_results(corpus_by_id, load_json(results_path))
    baseline = None
    if baseline_path is not None:
        baseline = validate_baseline(load_json(baseline_path))
    return {"corpus_cases": len(corpus_by_id), "result_cases": len(checked_results), "baseline_status": baseline["status"] if baseline else None}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path(__file__).resolve().parents[2]
    parser.add_argument("--corpus", type=Path, default=root / "docs/agent-interface/corpus.v1.json")
    parser.add_argument("--results", type=Path)
    parser.add_argument("--baseline", type=Path, default=root / "docs/agent-interface/baseline-mcp.v1.json")
    args = parser.parse_args(argv)
    try:
        summary = validate_files(args.corpus, args.results, args.baseline)
    except ValidationError as exc:
        print(f"agent-interface validation failed: {exc}", file=sys.stderr)
        return 1
    print(json.dumps({"valid": True, **summary}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
