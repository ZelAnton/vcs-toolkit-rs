#!/usr/bin/env python3
"""Create a deterministic, offline evaluation recording from result fixtures."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from validate import ValidationError, load_json, validate_baseline, validate_corpus, validate_results


def _ratio(numerator: int, denominator: int) -> dict[str, int]:
    return {"numerator": numerator, "denominator": denominator}


def _recorded_calls(calls: dict[str, int]) -> dict[str, int]:
    """Keep the standard route channels explicit, including zeroes."""
    recorded = {
        "preferred_interface": calls["preferred_interface"],
        "fallback_interface": calls.get("fallback_interface", 0),
        "raw_cli": calls["raw_cli"],
        "total": calls["total"],
    }
    recorded.update({key: value for key, value in calls.items() if key not in recorded})
    return recorded


def make_recording(corpus: Any, results: Any, baseline: Any | None = None) -> dict[str, Any]:
    corpus_by_id = validate_corpus(corpus)
    checked = validate_results(corpus_by_id, results)
    baseline_value = validate_baseline(baseline) if baseline is not None else None
    by_id = {result["case_id"]: result for result in checked}
    ordered = [by_id[case_id] for case_id in corpus_by_id if case_id in by_id]
    positive = [case for case in corpus_by_id.values() if case["expected"]["selection"] == "preferred"]
    negative = [case for case in corpus_by_id.values() if case["expected"]["selection"] == "none"]
    preferred_selected = sum(1 for case in positive if by_id.get(case["case_id"], {}).get("selection", {}).get("preferred_interface_selected"))
    false_activation = sum(1 for case in negative if by_id.get(case["case_id"], {}).get("selection", {}).get("false_activation"))
    raw_bypass = sum(1 for result in ordered if result["selection"]["raw_cli_bypass"])
    total_calls = sum(result["calls"]["total"] for result in ordered)
    exact_verified = sum(1 for result in ordered if result["revision"]["exact_revision_verified"])
    terminal_verified = sum(1 for result in ordered if result["revision"]["terminal_ci"]["verified"])
    recording: dict[str, Any] = {
        "schema_version": "agent-interface.recording.v1",
        "corpus_version": corpus["corpus_version"],
        "source": "offline-fixture",
        "cases": [
            {
                "case_id": result["case_id"],
                "outcome_status": result["outcome"]["status"],
                "selected_interface": result["selection"]["selected_interface"],
                "fallback_reason": result["selection"]["fallback_reason"],
                "preferred_interface_selected": result["selection"]["preferred_interface_selected"],
                "false_activation": result["selection"]["false_activation"],
                "raw_cli_bypass": result["selection"]["raw_cli_bypass"],
                "calls": _recorded_calls(result["calls"]),
                "unrelated_changes_preserved": result["workspace"]["unrelated_changes_preserved"],
                "revision": {
                    "before": result["revision"].get("before"),
                    "after": result["revision"].get("after"),
                    "published": result["revision"].get("published"),
                    "exact_revision_verified": result["revision"]["exact_revision_verified"],
                    "terminal_ci": {
                        "verified": result["revision"]["terminal_ci"]["verified"],
                        "revision": result["revision"]["terminal_ci"].get("revision"),
                        "conclusion": result["revision"]["terminal_ci"].get("conclusion"),
                    },
                },
            }
            for result in ordered
        ],
        "metrics": {
            "preferred_interface_selection_rate": _ratio(preferred_selected, len(positive)),
            "false_activation_rate": _ratio(false_activation, len(negative)),
            "raw_cli_bypass_rate": _ratio(raw_bypass, len(ordered)),
            "total_calls": total_calls,
            "exact_revision_verified": exact_verified,
            "terminal_ci_verified": terminal_verified,
            "unrelated_state_preserved": sum(1 for result in ordered if result["workspace"]["unrelated_changes_preserved"]),
        },
    }
    if baseline_value is not None:
        recording["baseline"] = {
            "interface": baseline_value["interface"],
            "status": baseline_value["status"],
            "metrics": baseline_value["metrics"],
        }
    return recording


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path(__file__).resolve().parents[2]
    parser.add_argument("--corpus", type=Path, default=root / "docs/agent-interface/corpus.v1.json")
    parser.add_argument("--results", type=Path, default=root / "docs/agent-interface/fixtures/results.v1.json")
    parser.add_argument("--baseline", type=Path, default=root / "docs/agent-interface/baseline-mcp.v1.json")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        recording = make_recording(load_json(args.corpus), load_json(args.results), load_json(args.baseline))
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(recording, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    except (ValidationError, OSError) as exc:
        print(f"agent-interface recording failed: {exc}", file=sys.stderr)
        return 1
    print(json.dumps({"written": str(args.output), "cases": len(recording["cases"])}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
