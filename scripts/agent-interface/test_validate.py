#!/usr/bin/env python3
"""Cross-layer parity tests for the vcs-agent v1 machine fixtures."""

from __future__ import annotations

import contextlib
import copy
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any, Callable


SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

import record  # noqa: E402
import validate  # noqa: E402


Mutation = tuple[str, dict[str, Any]]


def _fixture(name: str) -> dict[str, Any]:
    return validate.load_json(ROOT / "crates" / "agent" / "tests" / "fixtures" / name)


def _mutated(name: str, mutate: Callable[[dict[str, Any]], None]) -> Mutation:
    if name.startswith("inspect"):
        source = "inspect-success-git.v1.json"
    elif name.startswith("commit"):
        source = "commit-success-git.v1.json"
    elif name.startswith("publish"):
        source = "publish-success-git.v1.json"
    elif name.startswith("ci-status"):
        source = "ci-status-success-github.v1.json"
    elif name.startswith("ci-wait"):
        source = "ci-wait-success-github.v1.json"
    else:
        source = "changes-full-jj.v1.json"
    value = copy.deepcopy(_fixture(source))
    mutate(value)
    return name, value


def schema_invalid_mutations() -> list[Mutation]:
    return [
        _mutated("inspect-missing-revision", lambda value: value["data"]["working_copy"].pop("revision")),
        _mutated(
            "inspect-invalid-forge-capability",
            lambda value: value["data"]["forge"]["capabilities"]["value"].__setitem__("cli_supported", "yes"),
        ),
        _mutated(
            "inspect-invalid-forge-auth",
            lambda value: value["data"]["forge"]["auth"]["value"].__setitem__(
                "accounts", [{"host": 42, "login": "agent", "active": True}]
            ),
        ),
        _mutated("changes-missing-count", lambda value: value["data"]["counts"].pop("insertions")),
        _mutated("changes-missing-old-path", lambda value: value["data"]["files"][0].pop("old_path")),
        _mutated("changes-missing-hunk-start", lambda value: value["data"]["diff"][0]["hunks"][0].pop("old_start")),
        _mutated(
            "changes-invalid-line-text",
            lambda value: value["data"]["diff"][0]["hunks"][0]["lines"][0].__setitem__("text", 7),
        ),
        _mutated("commit-empty-paths", lambda value: value["data"].__setitem__("included_paths", [])),
        _mutated(
            "commit-same-revision",
            lambda value: value["data"]["after"].__setitem__("revision", value["data"]["before"]["revision"]),
        ),
        _mutated(
            "commit-claims-push",
            lambda value: value["data"]["semantics"].__setitem__("push_performed", True),
        ),
        _mutated(
            "commit-hides-unrelated-loss",
            lambda value: value["data"].__setitem__("unrelated_changes_preserved", False),
        ),
        _mutated(
            "publish-remote-revision-mismatch",
            lambda value: value["data"].__setitem__("remote_revision", "different"),
        ),
        _mutated(
            "publish-unverified-push",
            lambda value: value["data"]["push"].__setitem__("verified", False),
        ),
        _mutated(
            "publish-invented-retry-state",
            lambda value: value["data"]["push"].__setitem__("state", "already_present"),
        ),
        _mutated(
            "ci-status-revision-mismatch",
            lambda value: value["data"]["runs"][0].__setitem__("revision", "different"),
        ),
        _mutated(
            "ci-status-successful-pending",
            lambda value: value["data"]["runs"][0].__setitem__("status", "in_progress"),
        ),
        _mutated(
            "ci-wait-missing-watchdog",
            lambda value: value["data"]["wait"].pop("inactivity_watchdog"),
        ),
    ]


def invalid_operation_mutations() -> list[Mutation]:
    invalid_identifiers = {
        "operation-uppercase": "FutureOperation",
        "operation-leading-digit": "2future_operation",
        "operation-leading-hyphen": "-future_operation",
        "operation-dot": "future.operation",
        "operation-slash": "future/operation",
        "operation-space": "future operation",
    }
    mutations = []
    for name, operation in invalid_identifiers.items():
        value = copy.deepcopy(_fixture("future-operation-success.v1.json"))
        value["operation"] = operation
        mutations.append((name, value))
    return mutations


def invalid_mutations() -> list[Mutation]:
    return [*schema_invalid_mutations(), *invalid_operation_mutations()]


class MachineFixtureValidationTests(unittest.TestCase):
    def test_publish_retry_vocabulary_fixtures_reach_validator_and_recorder(self) -> None:
        names = [
            "publish-success-git.v1.json",
            "publish-success-retry-git.v1.json",
            "publish-success-discovered-git.v1.json",
            "publish-success-recovered-git.v1.json",
        ]
        for name in names:
            with self.subTest(name=name):
                value = _fixture(name)
                self.assertIs(validate.validate_machine_envelope(value, name), value)

        corpus = ROOT / "docs" / "agent-interface" / "corpus.v1.json"
        results = ROOT / "docs" / "agent-interface" / "fixtures" / "results.v1.json"
        baseline = ROOT / "docs" / "agent-interface" / "baseline-mcp.v1.json"
        source = ROOT / "crates" / "agent" / "tests" / "fixtures"
        with tempfile.TemporaryDirectory(prefix="agent-validator-") as raw_temp:
            temp = Path(raw_temp)
            fixtures = temp / "fixtures"
            fixtures.mkdir()
            for name in names:
                (fixtures / name).write_text(
                    (source / name).read_text(encoding="utf-8"), encoding="utf-8"
                )
            output = temp / "recording.json"
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                exit_code = record.main(
                    [
                        "--corpus", str(corpus),
                        "--results", str(results),
                        "--baseline", str(baseline),
                        "--machine-fixtures", str(fixtures),
                        "--output", str(output),
                    ]
                )
            self.assertEqual(exit_code, 0)
            self.assertTrue(output.exists(), "reachable retry fixtures must produce a recording")

    def test_future_operation_is_accepted_by_validator_and_recorder(self) -> None:
        future_operation = _fixture("future-operation-success.v1.json")
        self.assertIs(
            validate.validate_machine_envelope(future_operation, "future operation"),
            future_operation,
        )

        corpus = ROOT / "docs" / "agent-interface" / "corpus.v1.json"
        results = ROOT / "docs" / "agent-interface" / "fixtures" / "results.v1.json"
        baseline = ROOT / "docs" / "agent-interface" / "baseline-mcp.v1.json"
        fixture = ROOT / "crates" / "agent" / "tests" / "fixtures" / "future-operation-success.v1.json"

        with tempfile.TemporaryDirectory(prefix="agent-validator-") as raw_temp:
            temp = Path(raw_temp)
            fixtures = temp / "fixtures"
            fixtures.mkdir()
            (fixtures / fixture.name).write_text(fixture.read_text(encoding="utf-8"), encoding="utf-8")
            output = temp / "recording.json"
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                exit_code = record.main(
                    [
                        "--corpus", str(corpus),
                        "--results", str(results),
                        "--baseline", str(baseline),
                        "--machine-fixtures", str(fixtures),
                        "--output", str(output),
                    ]
                )
            self.assertEqual(exit_code, 0)
            self.assertTrue(output.exists(), "valid future operation must produce a recording")

    def test_python_validator_rejects_invalid_machine_envelopes(self) -> None:
        for name, value in invalid_mutations():
            with self.subTest(name=name):
                with self.assertRaises(validate.ValidationError):
                    validate.validate_machine_envelope(value, name)

    def test_recorder_writes_nothing_for_invalid_machine_envelopes(self) -> None:
        corpus = ROOT / "docs" / "agent-interface" / "corpus.v1.json"
        results = ROOT / "docs" / "agent-interface" / "fixtures" / "results.v1.json"
        baseline = ROOT / "docs" / "agent-interface" / "baseline-mcp.v1.json"

        for name, value in invalid_mutations():
            with self.subTest(name=name), tempfile.TemporaryDirectory(prefix="agent-validator-") as raw_temp:
                temp = Path(raw_temp)
                fixtures = temp / "fixtures"
                fixtures.mkdir()
                (fixtures / f"{name}.v1.json").write_text(
                    json.dumps(value), encoding="utf-8"
                )
                output = temp / "recording.json"
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    exit_code = record.main(
                        [
                            "--corpus", str(corpus),
                            "--results", str(results),
                            "--baseline", str(baseline),
                            "--machine-fixtures", str(fixtures),
                            "--output", str(output),
                        ]
                    )
                self.assertEqual(exit_code, 1)
                self.assertFalse(output.exists(), "invalid input must not produce a recording")


class SkillMetadataEvaluationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.corpus = validate.load_json(
            ROOT / "docs" / "agent-interface" / "corpus.v1.json"
        )
        cls.results = validate.load_json(
            ROOT / "docs" / "agent-interface" / "fixtures" / "results.v1.json"
        )
        cls.baseline = validate.load_json(
            ROOT / "docs" / "agent-interface" / "baseline-mcp.v1.json"
        )

    def test_recording_fixes_skill_selection_false_activation_and_bypass_metrics(self) -> None:
        recording = record.make_recording(self.corpus, self.results, self.baseline)
        self.assertEqual(recording["skill_metadata"], self.corpus["skill_metadata"])
        self.assertEqual(
            recording["metrics"]["preferred_interface_selection_rate"],
            {"numerator": 9, "denominator": 9},
        )
        self.assertEqual(
            recording["metrics"]["false_activation_rate"],
            {"numerator": 0, "denominator": 3},
        )
        self.assertEqual(
            recording["metrics"]["raw_cli_bypass_rate"],
            {"numerator": 1, "denominator": 14},
        )
        self.assertIsNone(recording["skill_metadata"]["unavailable_live_metrics"])

    def test_negative_metric_must_remain_first_priority(self) -> None:
        corpus = copy.deepcopy(self.corpus)
        corpus["skill_metadata"]["metric_priority"].reverse()
        with self.assertRaisesRegex(validate.ValidationError, "prioritize negative"):
            validate.validate_corpus(corpus)

    def test_unavailable_live_metrics_cannot_be_zero(self) -> None:
        corpus = copy.deepcopy(self.corpus)
        corpus["skill_metadata"]["unavailable_live_metrics"] = {
            "false_activation_rate": 0
        }
        with self.assertRaisesRegex(validate.ValidationError, "never zero"):
            validate.validate_corpus(corpus)


if __name__ == "__main__":
    unittest.main()
