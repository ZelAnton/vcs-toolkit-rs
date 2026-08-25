#!/usr/bin/env python3
"""Cross-layer negative tests for the vcs-agent v1 machine fixtures."""

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
    source = "inspect-success-git.v1.json" if name.startswith("inspect") else "changes-full-jj.v1.json"
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
    ]


class MachineFixtureValidationTests(unittest.TestCase):
    def test_python_validator_rejects_schema_invalid_success_payloads(self) -> None:
        for name, value in schema_invalid_mutations():
            with self.subTest(name=name):
                with self.assertRaises(validate.ValidationError):
                    validate.validate_machine_envelope(value, name)

    def test_recorder_writes_nothing_for_schema_invalid_success_payloads(self) -> None:
        corpus = ROOT / "docs" / "agent-interface" / "corpus.v1.json"
        results = ROOT / "docs" / "agent-interface" / "fixtures" / "results.v1.json"
        baseline = ROOT / "docs" / "agent-interface" / "baseline-mcp.v1.json"

        for name, value in schema_invalid_mutations():
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


if __name__ == "__main__":
    unittest.main()
