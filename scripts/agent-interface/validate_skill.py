#!/usr/bin/env python3
"""Fail closed when the vcs-agent Skill drifts from executable contracts."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


class SkillValidationError(ValueError):
    pass


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SkillValidationError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict):
        raise SkillValidationError(f"{path} must contain a JSON object")
    return value


def parse_frontmatter(path: Path) -> tuple[dict[str, str], str]:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise SkillValidationError(f"cannot read {path}: {error}") from error
    lines = text.splitlines()
    if not lines or lines[0] != "---":
        raise SkillValidationError("SKILL.md must start with YAML frontmatter")
    try:
        end = lines.index("---", 1)
    except ValueError as error:
        raise SkillValidationError("SKILL.md frontmatter is not closed") from error
    metadata: dict[str, str] = {}
    for line in lines[1:end]:
        if not line.strip():
            continue
        key, separator, value = line.partition(":")
        if not separator or not key.strip() or not value.strip():
            raise SkillValidationError("SKILL.md frontmatter must use scalar key: value fields")
        metadata[key.strip()] = value.strip()
    return metadata, text


def _pairs(items: Any, label: str, key: str, first: str, last: str) -> dict[str, tuple[int, int]]:
    if not isinstance(items, list):
        raise SkillValidationError(f"{label} must be an array")
    result: dict[str, tuple[int, int]] = {}
    for item in items:
        if not isinstance(item, dict) or not isinstance(item.get(key), str):
            raise SkillValidationError(f"{label} contains an invalid entry")
        result[item[key]] = (item.get(first), item.get(last))
    return result


def validate_documents(
    skill_path: Path,
    contract: dict[str, Any],
    profile: dict[str, Any],
) -> tuple[str, dict[str, Any]]:
    metadata, skill_text = parse_frontmatter(skill_path)
    expected_metadata = contract.get("metadata")
    if metadata != expected_metadata:
        raise SkillValidationError("SKILL.md metadata differs from contract.v1.json")
    if contract.get("skill_contract_version") != "vcs-agent-skill/v1":
        raise SkillValidationError("skill contract version is not vcs-agent-skill/v1")

    fallback = contract.get("fallback_reasons")
    if fallback != [
        "structured_unsupported",
        "missing_vcs_agent",
        "exact_low_level_diagnostic",
    ]:
        raise SkillValidationError("fallback reasons differ from the three allowed grounds")

    processkit = contract.get("processkit_cli_profile")
    if not isinstance(processkit, dict):
        raise SkillValidationError("processkit_cli_profile is missing")
    if processkit.get("profile_version") != profile.get("profile_version"):
        raise SkillValidationError("ProcessKit-CLI profile version drifted")
    if processkit.get("minimum_expected_duration_seconds") != 60:
        raise SkillValidationError("ProcessKit-CLI duration threshold must remain 60 seconds")
    preflight = processkit.get("preflight")
    source_preflight = profile.get("preflight")
    if not isinstance(preflight, dict) or not isinstance(source_preflight, dict):
        raise SkillValidationError("ProcessKit-CLI preflight is missing")
    source_band = source_preflight.get("exit_code_band")
    expected_band = [source_band.get("start"), source_band.get("end")] if isinstance(source_band, dict) else None
    if preflight.get("probe_version") != source_preflight.get("probe_version"):
        raise SkillValidationError("ProcessKit-CLI probe version drifted")
    if preflight.get("schema_version") != source_preflight.get("schema_version"):
        raise SkillValidationError("ProcessKit-CLI schema version drifted")
    if preflight.get("exit_code_band") != expected_band:
        raise SkillValidationError("ProcessKit-CLI exit band drifted")
    if processkit.get("terminal_event") != profile.get("lifecycle", {}).get("terminal_event"):
        raise SkillValidationError("ProcessKit-CLI terminal event drifted")
    if contract.get("agent_contract_version") != profile.get("vcs_agent", {}).get("contract_version"):
        raise SkillValidationError("vcs-agent child contract differs from ProcessKit-CLI profile")

    for required in (
        "Run `vcs-agent probe`, then `vcs-agent inspect",
        "exact selected paths",
        "structured `unsupported`",
        "at least 60 seconds",
        "`runner_exit` record",
        "host sandbox, command rules, and approvals",
    ):
        if required not in skill_text:
            raise SkillValidationError(f"SKILL.md omits required workflow invariant: {required!r}")
    return skill_text, metadata


def _run(binary: Path, argv: list[str]) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            [str(binary), *argv],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=20,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise SkillValidationError(f"cannot run {binary}: {error}") from error


def validate_binary(binary: Path, contract: dict[str, Any]) -> dict[str, Any]:
    help_result = _run(binary, ["--help"])
    if help_result.returncode != 0:
        raise SkillValidationError("vcs-agent --help failed")
    help_text = help_result.stdout

    probe_result = _run(binary, ["probe"])
    if probe_result.returncode != 0:
        raise SkillValidationError("vcs-agent probe failed")
    try:
        probe = json.loads(probe_result.stdout)
    except json.JSONDecodeError as error:
        raise SkillValidationError("vcs-agent probe did not emit JSON") from error
    if probe.get("contract_version") != contract.get("agent_contract_version"):
        raise SkillValidationError("vcs-agent contract version drifted")
    data = probe.get("data", {})
    if data.get("commands", {}).get("supported") != contract.get("commands"):
        raise SkillValidationError("vcs-agent command set drifted")

    actual_errors = {
        item.get("kind"): item.get("exit_code")
        for item in data.get("error_kinds", [])
        if isinstance(item, dict)
    }
    if actual_errors != contract.get("error_kinds"):
        raise SkillValidationError("vcs-agent error names or exit codes drifted")
    actual_bands = _pairs(data.get("exit_bands"), "probe exit_bands", "name", "first", "last")
    expected_bands = {
        name: tuple(bounds) for name, bounds in contract.get("exit_bands", {}).items()
    }
    if actual_bands != expected_bands:
        raise SkillValidationError("vcs-agent exit bands drifted")

    for command, flags in contract.get("required_flags", {}).items():
        if command not in contract.get("commands", []):
            raise SkillValidationError(f"flags declared for unknown command {command}")
        for flag in flags:
            if flag not in help_text:
                raise SkillValidationError(f"{command} references missing flag {flag}")

    for example in contract.get("examples", []):
        result = _run(binary, example["argv"])
        try:
            envelope = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise SkillValidationError(f"example {example['id']} did not emit JSON") from error
        if envelope.get("operation") != example["operation"]:
            raise SkillValidationError(f"example {example['id']} selected the wrong operation")
        error = envelope.get("error")
        if isinstance(error, dict) and error.get("kind") == "invalid_input":
            raise SkillValidationError(f"example {example['id']} is rejected by the parser")
    return probe


def main(argv: list[str] | None = None) -> int:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument("--vcs-agent", type=Path, required=True)
    parser.add_argument("--skill", type=Path, default=root / "skills/vcs-agent/SKILL.md")
    parser.add_argument("--contract", type=Path, default=root / "skills/vcs-agent/references/contract.v1.json")
    parser.add_argument("--processkit-profile", type=Path, default=root / "docs/agent-interface/processkit-cli-profile.v1.json")
    args = parser.parse_args(argv)
    try:
        contract = load_json(args.contract)
        profile = load_json(args.processkit_profile)
        validate_documents(args.skill, contract, profile)
        probe = validate_binary(args.vcs_agent, contract)
    except SkillValidationError as error:
        print(f"skill validation failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps({
        "status": "passed",
        "skill_contract_version": contract["skill_contract_version"],
        "agent_contract_version": probe["contract_version"],
        "examples": len(contract["examples"]),
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
