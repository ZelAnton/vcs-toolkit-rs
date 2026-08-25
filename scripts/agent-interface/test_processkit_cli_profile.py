#!/usr/bin/env python3
"""Negative and gating tests for the ProcessKit-CLI interoperability profile."""

from __future__ import annotations

import contextlib
import copy
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

import processkit_cli_profile as harness  # noqa: E402
import record  # noqa: E402
import validate  # noqa: E402


PROFILE_PATH = ROOT / "docs" / "agent-interface" / "processkit-cli-profile.v1.json"
EVIDENCE_PATH = ROOT / "docs" / "agent-interface" / "fixtures" / "processkit-cli-evidence.v1.json"
MACHINE_FIXTURES = ROOT / "crates" / "agent" / "tests" / "fixtures"


class ProcessKitCliProfileTests(unittest.TestCase):
    def setUp(self) -> None:
        self.profile = validate.load_json(PROFILE_PATH)
        self.evidence = validate.load_json(EVIDENCE_PATH)

    def test_committed_profile_and_evidence_are_synchronized(self) -> None:
        validated = validate.validate_processkit_cli_profile(self.profile)
        checked = validate.validate_processkit_cli_evidence(self.evidence, validated, MACHINE_FIXTURES)
        self.assertEqual(len(checked["scenarios"]), 6)

    def test_profile_rejects_exact_required_surface_drift(self) -> None:
        mutations = []

        missing = copy.deepcopy(self.profile)
        missing["preflight"]["required_surface"].remove("run:--capture-max-bytes")
        mutations.append(("removal", missing))

        substituted = copy.deepcopy(self.profile)
        substituted["preflight"]["required_surface"][10] = "run:--unrelated"
        mutations.append(("substitution", substituted))

        duplicated = copy.deepcopy(self.profile)
        duplicated["preflight"]["required_surface"].append("run:--capture-max-bytes")
        mutations.append(("duplication", duplicated))

        added = copy.deepcopy(self.profile)
        added["preflight"]["required_surface"].append("run:--unrelated")
        mutations.append(("addition", added))

        reordered = copy.deepcopy(self.profile)
        reordered["preflight"]["required_surface"][9:11] = reversed(
            reordered["preflight"]["required_surface"][9:11]
        )
        mutations.append(("reordering", reordered))

        for label, mutated in mutations:
            with self.subTest(label=label), self.assertRaisesRegex(validate.ValidationError, "required_surface"):
                validate.validate_processkit_cli_profile(mutated)

    def test_profile_rejects_weakened_containment_claim(self) -> None:
        mutated = copy.deepcopy(self.profile)
        mutated["lifecycle"]["containment"]["claim"] = "inner-membership-proven"
        with self.assertRaisesRegex(validate.ValidationError, "containment claim"):
            validate.validate_processkit_cli_profile(mutated)

    def test_evidence_rejects_timeout_without_timeout_event(self) -> None:
        mutated = copy.deepcopy(self.evidence)
        timeout = next(item for item in mutated["scenarios"] if item["id"] == "timeout")
        timeout["events"] = [item for item in timeout["events"] if item["event"] != "timeout"]
        with self.assertRaisesRegex(validate.ValidationError, "overall timeout"):
            validate.validate_processkit_cli_evidence(mutated, self.profile, MACHINE_FIXTURES)

    def test_evidence_rejects_every_scenario_classification_mutation(self) -> None:
        for scenario in self.evidence["scenarios"]:
            terminal = next(item for item in scenario["events"] if item["event"] == "runner_exit")
            mutations = {
                "command_exit_code": scenario["command_exit_code"] + 1,
                "runner_exit.code": terminal["code"] + 1,
                "runner_exit.source": "contradictory",
                "runner_exit.child_code": 0 if terminal["child_code"] is None else terminal["child_code"] + 1,
            }
            for field, value in mutations.items():
                mutated = copy.deepcopy(self.evidence)
                target = next(item for item in mutated["scenarios"] if item["id"] == scenario["id"])
                if field == "command_exit_code":
                    target[field] = value
                else:
                    target_terminal = next(item for item in target["events"] if item["event"] == "runner_exit")
                    target_terminal[field.removeprefix("runner_exit.")] = value
                with self.subTest(scenario=scenario["id"], field=field), self.assertRaisesRegex(
                    validate.ValidationError, "classification"
                ):
                    validate.validate_processkit_cli_evidence(mutated, self.profile, MACHINE_FIXTURES)

    def test_evidence_rejects_classification_type_confusion(self) -> None:
        invalid_values = {
            "schema_version": (True, "1", None),
            "code": (False, "0", None),
            "child_code": (False, "0", None),
            "event": (False, 0, None),
            "source": (False, 0, None),
        }
        for scenario_id in ("agent-success", "bounded-capture", "nested-containment"):
            for field, values in invalid_values.items():
                for value in values:
                    mutated = copy.deepcopy(self.evidence)
                    scenario = next(item for item in mutated["scenarios"] if item["id"] == scenario_id)
                    terminal = next(item for item in scenario["events"] if item["event"] == "runner_exit")
                    terminal[field] = value
                    with self.subTest(scenario=scenario_id, field=field, value=value), self.assertRaisesRegex(
                        validate.ValidationError, f"{field}|classification"
                    ):
                        validate.validate_processkit_cli_evidence(mutated, self.profile, MACHINE_FIXTURES)

    def test_evidence_rejects_non_terminal_lifecycle_type_confusion(self) -> None:
        mutations = (
            ("timeout", "timeout", "reason", False),
            ("control-cancel", "cancelled", "source", None),
            ("bounded-capture", "output_captured", "stdout.bytes", False),
            ("bounded-capture", "output_captured", "stderr.truncated", 1),
            ("nested-containment", "cleanup_finished", "remaining", False),
            ("nested-containment", "cleanup_finished", "read_error", 0),
        )
        for scenario_id, event_name, field_path, value in mutations:
            mutated = copy.deepcopy(self.evidence)
            scenario = next(item for item in mutated["scenarios"] if item["id"] == scenario_id)
            event = next(item for item in scenario["events"] if item["event"] == event_name)
            target = event
            path = field_path.split(".")
            for component in path[:-1]:
                target = target[component]
            target[path[-1]] = value
            with self.subTest(scenario=scenario_id, field=field_path, value=value), self.assertRaisesRegex(
                validate.ValidationError, path[-1]
            ):
                validate.validate_processkit_cli_evidence(mutated, self.profile, MACHINE_FIXTURES)

    def test_evidence_rejects_capture_without_per_stream_truncation(self) -> None:
        mutated = copy.deepcopy(self.evidence)
        capture = next(item for item in mutated["scenarios"] if item["id"] == "bounded-capture")
        output = next(item for item in capture["events"] if item["event"] == "output_captured")
        output["stderr"]["truncated"] = False
        with self.assertRaisesRegex(validate.ValidationError, "per-stream truncation"):
            validate.validate_processkit_cli_evidence(mutated, self.profile, MACHINE_FIXTURES)

    def test_unprovided_binary_is_an_explicit_skip(self) -> None:
        stdout = io.StringIO()
        stderr = io.StringIO()
        with mock.patch.dict(os.environ, {}, clear=True), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = harness.main(["--vcs-agent", "not-built-for-skip"])
        self.assertEqual(result, 0)
        self.assertEqual(stderr.getvalue(), "")
        report = json.loads(stdout.getvalue())
        self.assertEqual(report["status"], "skipped")
        self.assertEqual(report["reason"], "processkit_cli_not_provided")

    def test_provided_but_missing_binary_is_failure_not_skip(self) -> None:
        stdout = io.StringIO()
        stderr = io.StringIO()
        with mock.patch.dict(os.environ, {}, clear=True), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = harness.main([
                "--processkit-cli", str(ROOT / "definitely-missing-processkit-cli"),
                "--vcs-agent", "irrelevant",
            ])
        self.assertEqual(result, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertEqual(json.loads(stderr.getvalue())["status"], "failed")

    def test_incompatible_probe_is_failure_not_skip(self) -> None:
        incompatible = copy.deepcopy({
            "probe_version": 1,
            "binary": "processkit-cli",
            "version": "future",
            "schema_version": 2,
            "exit_code_band": {"start": 100, "end": 119},
            "surface": [],
            "compatible": False,
            "mismatches": ["schema_version expected 1, got 2"],
        })
        completed = subprocess.CompletedProcess(["processkit-cli"], 110, json.dumps(incompatible).encode(), b"")
        with mock.patch.object(harness, "_run", return_value=completed):
            with self.assertRaisesRegex(harness.ProfileRunError, "incompatible"):
                harness._probe(Path("processkit-cli"), self.profile)

    def test_recorder_writes_nothing_for_invalid_interop_evidence(self) -> None:
        mutated = copy.deepcopy(self.evidence)
        timeout = next(item for item in mutated["scenarios"] if item["id"] == "timeout")
        timeout["events"] = [item for item in timeout["events"] if item["event"] != "timeout"]
        with tempfile.TemporaryDirectory(prefix="agent-processkit-validator-") as raw_temp:
            temp = Path(raw_temp)
            invalid_evidence = temp / "invalid-evidence.json"
            invalid_evidence.write_text(json.dumps(mutated), encoding="utf-8")
            output = temp / "recording.json"
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                result = record.main([
                    "--processkit-cli-evidence", str(invalid_evidence),
                    "--output", str(output),
                ])
            self.assertEqual(result, 1)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
