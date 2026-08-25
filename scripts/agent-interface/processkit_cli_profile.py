#!/usr/bin/env python3
"""Exercise the gated vcs-agent/ProcessKit-CLI binary interoperability profile.

The harness is intentionally shell-free and depends only on public executable
surfaces.  An absent, unprovided ProcessKit-CLI is a documented skip.  Once a
path is provided, every probe mismatch, launch failure, or evidence violation is
a hard failure; an incompatible binary is never reclassified as unavailable.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path
from typing import Any


SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

import validate  # noqa: E402


class ProfileRunError(RuntimeError):
    """A supplied binary failed or contradicted the committed profile."""


def _json_bytes(value: Any) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def _resolve_executable(raw: str, label: str) -> Path:
    candidate = Path(raw).expanduser()
    if candidate.is_file():
        return candidate.resolve()
    if os.name == "nt" and not candidate.suffix and candidate.with_suffix(".exe").is_file():
        return candidate.with_suffix(".exe").resolve()
    discovered = shutil.which(raw)
    if discovered:
        return Path(discovered).resolve()
    raise ProfileRunError(f"{label} was provided but is not an executable file: {raw}")


def _provided_processkit_cli(argument: str | None) -> tuple[Path | None, str | None]:
    if argument:
        return _resolve_executable(argument, "--processkit-cli"), "argument"
    environment = os.environ.get("PROCESSKIT_CLI_BIN")
    if environment:
        return _resolve_executable(environment, "PROCESSKIT_CLI_BIN"), "environment"
    return None, None


def _run(argv: list[str], *, timeout: float = 60.0) -> subprocess.CompletedProcess[bytes]:
    try:
        return subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise ProfileRunError(f"could not execute public binary surface: {exc}") from exc


def _decode_json(raw: bytes, label: str) -> dict[str, Any]:
    try:
        value = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ProfileRunError(f"{label} is not one UTF-8 JSON document: {exc}") from exc
    if not isinstance(value, dict):
        raise ProfileRunError(f"{label} must be a JSON object")
    return value


def _probe(processkit_cli: Path, profile: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any], str]:
    preflight = profile["preflight"]
    band = preflight["exit_code_band"]
    argv = [
        str(processkit_cli),
        "probe",
        "--json",
        "--require-schema-version",
        str(preflight["schema_version"]),
        "--require-exit-code-band",
        f"{band['start']}-{band['end']}",
    ]
    for surface in preflight["required_surface"]:
        argv.extend(["--require-surface", surface])
    completed = _run(argv)
    report = _decode_json(completed.stdout, "processkit-cli probe report")
    if completed.returncode == preflight["incompatible_exit_code"]:
        raise ProfileRunError("provided ProcessKit-CLI is incompatible with the committed profile")
    if completed.returncode != preflight["compatible_exit_code"]:
        raise ProfileRunError(f"ProcessKit-CLI probe failed with exit code {completed.returncode}")
    if report.get("compatible") is not True or report.get("mismatches") != []:
        raise ProfileRunError("ProcessKit-CLI probe exited successfully without a compatible empty-mismatch report")
    if report.get("binary") != "processkit-cli" or not isinstance(report.get("version"), str):
        raise ProfileRunError("ProcessKit-CLI probe did not identify a versioned processkit-cli binary")
    expected = {
        "probe_version": preflight["probe_version"],
        "schema_version": preflight["schema_version"],
        "exit_code_band": band,
    }
    for key, value in expected.items():
        if report.get(key) != value:
            raise ProfileRunError(f"ProcessKit-CLI probe {key} differs from the committed profile")
    if not set(preflight["required_surface"]).issubset(set(report.get("surface", []))):
        raise ProfileRunError("ProcessKit-CLI compatible report omitted a required surface token")

    schema_result = _run([str(processkit_cli), "probe", "--json", "--print-schema"])
    if schema_result.returncode != 0:
        raise ProfileRunError(f"ProcessKit-CLI schema probe failed with exit code {schema_result.returncode}")
    schema = _decode_json(schema_result.stdout, "ProcessKit-CLI lifecycle schema")
    _validate_lifecycle_schema(schema, profile)
    schema_hash = hashlib.sha256(schema_result.stdout).hexdigest()
    return report, schema, schema_hash


def _validate_lifecycle_schema(schema: dict[str, Any], profile: dict[str, Any]) -> None:
    if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
        raise ProfileRunError("ProcessKit-CLI lifecycle schema is not Draft 2020-12")
    definitions = schema.get("$defs")
    if not isinstance(definitions, dict):
        raise ProfileRunError("ProcessKit-CLI lifecycle schema has no $defs object")
    runner_exit = definitions.get("runnerExit")
    output_captured = definitions.get("outputCaptured")
    if not isinstance(runner_exit, dict) or not isinstance(output_captured, dict):
        raise ProfileRunError("ProcessKit-CLI lifecycle schema lacks runnerExit/outputCaptured definitions")
    runner_required = set(runner_exit.get("required", []))
    if not {"schema_version", "time", "event", "code", "source", "child_code"}.issubset(runner_required):
        raise ProfileRunError("ProcessKit-CLI runnerExit schema lacks required terminal fields")
    sources = runner_exit.get("properties", {}).get("source", {}).get("enum", [])
    lifecycle = profile["lifecycle"]
    required_sources = {lifecycle["child_exit_source"], lifecycle["timeout"]["source"], lifecycle["control_cancel"]["source"]}
    if not required_sources.issubset(set(sources)):
        raise ProfileRunError("ProcessKit-CLI runnerExit source taxonomy is incompatible")
    if not {"schema_version", "time", "event", "stdout", "stderr"}.issubset(set(output_captured.get("required", []))):
        raise ProfileRunError("ProcessKit-CLI outputCaptured schema lacks required per-stream evidence")


def _schema_type_matches(value: Any, expected: str) -> bool:
    return {
        "null": value is None,
        "boolean": isinstance(value, bool),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "number": isinstance(value, (int, float)) and not isinstance(value, bool),
        "string": isinstance(value, str),
        "array": isinstance(value, list),
        "object": isinstance(value, dict),
    }.get(expected, False)


def _validate_schema_value(value: Any, node: dict[str, Any], root: dict[str, Any], label: str) -> None:
    reference = node.get("$ref")
    if reference is not None:
        prefix = "#/$defs/"
        if not isinstance(reference, str) or not reference.startswith(prefix):
            raise ProfileRunError(f"{label}: lifecycle schema uses an unsupported reference")
        target = root.get("$defs", {}).get(reference[len(prefix):])
        if not isinstance(target, dict):
            raise ProfileRunError(f"{label}: lifecycle schema reference is unresolved")
        _validate_schema_value(value, target, root, label)
        return
    for keyword in ("oneOf", "anyOf"):
        branches = node.get(keyword)
        if branches is not None:
            matches = 0
            for branch in branches:
                try:
                    _validate_schema_value(value, branch, root, label)
                except ProfileRunError:
                    continue
                matches += 1
            if matches < 1 or (keyword == "oneOf" and matches != 1):
                raise ProfileRunError(f"{label}: value does not satisfy lifecycle schema {keyword}")
            return
    expected_type = node.get("type")
    if expected_type is not None:
        allowed = expected_type if isinstance(expected_type, list) else [expected_type]
        if not all(isinstance(item, str) for item in allowed) or not any(_schema_type_matches(value, item) for item in allowed):
            raise ProfileRunError(f"{label}: value has the wrong lifecycle schema type")
    if "const" in node and value != node["const"]:
        raise ProfileRunError(f"{label}: value differs from lifecycle schema const")
    if "enum" in node and value not in node["enum"]:
        raise ProfileRunError(f"{label}: value is outside lifecycle schema enum")
    if isinstance(value, dict):
        required = node.get("required", [])
        missing = [field for field in required if field not in value]
        if missing:
            raise ProfileRunError(f"{label}: lifecycle record lacks required fields: {', '.join(missing)}")
        properties = node.get("properties", {})
        if node.get("additionalProperties") is False:
            extra = sorted(set(value) - set(properties))
            if extra:
                raise ProfileRunError(f"{label}: lifecycle record has undeclared fields: {', '.join(extra)}")
        for key, child in value.items():
            child_schema = properties.get(key)
            if isinstance(child_schema, dict):
                _validate_schema_value(child, child_schema, root, f"{label}.{key}")
    if isinstance(value, list) and isinstance(node.get("items"), dict):
        for index, child in enumerate(value):
            _validate_schema_value(child, node["items"], root, f"{label}[{index}]")
    if isinstance(value, int) and not isinstance(value, bool):
        if "minimum" in node and value < node["minimum"]:
            raise ProfileRunError(f"{label}: integer is below lifecycle schema minimum")
        if "maximum" in node and value > node["maximum"]:
            raise ProfileRunError(f"{label}: integer is above lifecycle schema maximum")
    if isinstance(value, str):
        if "minLength" in node and len(value) < node["minLength"]:
            raise ProfileRunError(f"{label}: string is shorter than lifecycle schema minimum")
        if "pattern" in node and re.search(node["pattern"], value) is None:
            raise ProfileRunError(f"{label}: string does not match lifecycle schema pattern")


def _validate_runtime_records(scenarios: list[dict[str, Any]], schema: dict[str, Any]) -> None:
    definitions = schema["$defs"]
    for scenario in scenarios:
        label = scenario["id"]
        _validate_schema_value(scenario["terminal_event"], definitions["runnerExit"], schema, f"{label}.runner_exit")
        _validate_schema_value(scenario["capture_event"], definitions["outputCaptured"], schema, f"{label}.output_captured")


def _read_events(path: Path) -> list[dict[str, Any]]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        raise ProfileRunError(f"cannot read lifecycle stream: {exc}") from exc
    events = []
    for index, line in enumerate(lines, start=1):
        if not line.strip():
            continue
        events.append(_decode_json(line.encode("utf-8"), f"lifecycle line {index}"))
    if not events:
        raise ProfileRunError("lifecycle stream is empty")
    return events


def _terminal(events: list[dict[str, Any]], profile: dict[str, Any]) -> dict[str, Any]:
    expected_version = profile["lifecycle"]["schema_version"]
    for event in events:
        if event.get("schema_version") != expected_version or not isinstance(event.get("event"), str):
            raise ProfileRunError("lifecycle stream contains a non-v1 or untagged event")
    terminals = [event for event in events if event["event"] == profile["lifecycle"]["terminal_event"]]
    if len(terminals) != 1:
        raise ProfileRunError("lifecycle stream must contain exactly one runner_exit record")
    terminal = terminals[0]
    for field in ("code", "source", "child_code"):
        if field not in terminal:
            raise ProfileRunError(f"runner_exit is missing {field}")
    return terminal


def _captured_event(events: list[dict[str, Any]], profile: dict[str, Any]) -> dict[str, Any]:
    matches = [event for event in events if event.get("event") == profile["lifecycle"]["bounded_capture"]["event"]]
    if len(matches) != 1:
        raise ProfileRunError("captured run must contain exactly one output_captured event")
    capture = matches[0]
    for stream in ("stdout", "stderr"):
        info = capture.get(stream)
        if not isinstance(info, dict) or not isinstance(info.get("bytes"), int) or not isinstance(info.get("truncated"), bool):
            raise ProfileRunError(f"output_captured.{stream} lacks byte/truncation evidence")
    return capture


def _run_foreground(
    processkit_cli: Path,
    child: list[str],
    directory: Path,
    profile: dict[str, Any],
    *,
    timeout_value: str = "30s",
    capture_bytes: str = "1m",
) -> tuple[subprocess.CompletedProcess[bytes], list[dict[str, Any]], dict[str, Any], dict[str, Any], Path]:
    events_path = directory / "events.jsonl"
    capture_dir = directory / "capture"
    capture_dir.mkdir(parents=True)
    run_id = f"vcs-agent-profile-{uuid.uuid4().hex}"
    argv = [
        str(processkit_cli), "run",
        "--run-id", run_id,
        "--cwd", str(ROOT),
        "--jsonl", str(events_path),
        "--capture-dir", str(capture_dir),
        "--capture-max-bytes", capture_bytes,
        "--no-echo",
        "--timeout", timeout_value,
        "--grace", "0",
        "--",
        *child,
    ]
    completed = _run(argv, timeout=45.0)
    events = _read_events(events_path)
    terminal = _terminal(events, profile)
    captured = _captured_event(events, profile)
    return completed, events, terminal, captured, capture_dir


def _agent_scenario(
    scenario_id: str,
    processkit_cli: Path,
    vcs_agent: Path,
    child_args: list[str],
    expected_exit: int,
    expected_operation: str,
    expected_status: str,
    directory: Path,
    profile: dict[str, Any],
) -> dict[str, Any]:
    completed, events, terminal, captured, capture_dir = _run_foreground(
        processkit_cli, [str(vcs_agent), *child_args], directory, profile
    )
    if completed.returncode != expected_exit:
        raise ProfileRunError(f"{scenario_id}: runner returned {completed.returncode}, expected faithful child exit {expected_exit}")
    if terminal.get("code") != expected_exit or terminal.get("child_code") != expected_exit or terminal.get("source") != profile["lifecycle"]["child_exit_source"]:
        raise ProfileRunError(f"{scenario_id}: runner_exit did not preserve the child classification")
    outcome = _decode_json((capture_dir / "stdout.log").read_bytes(), f"{scenario_id} child stdout")
    validate.validate_machine_envelope(outcome, f"{scenario_id} child outcome")
    if outcome.get("contract_version") != profile["vcs_agent"]["contract_version"]:
        raise ProfileRunError(f"{scenario_id}: child contract version drifted")
    if outcome.get("operation") != expected_operation or outcome.get("status") != expected_status:
        raise ProfileRunError(f"{scenario_id}: child emitted an unexpected operation/status")
    if expected_status == "error" and outcome.get("error", {}).get("exit_code") != expected_exit:
        raise ProfileRunError(f"{scenario_id}: structured child error disagrees with process exit")
    return {
        "id": scenario_id,
        "status": "passed",
        "command_exit_code": completed.returncode,
        "events": events,
        "terminal_event": terminal,
        "capture_event": captured,
        "child_outcome": outcome,
    }


def _timeout_scenario(processkit_cli: Path, directory: Path, profile: dict[str, Any]) -> dict[str, Any]:
    completed, events, terminal, captured, _ = _run_foreground(
        processkit_cli,
        [sys.executable, str(Path(__file__).resolve()), "--_child", "sleep", "10"],
        directory,
        profile,
        timeout_value="250ms",
    )
    expected = profile["lifecycle"]["timeout"]
    if completed.returncode != expected["code"] or terminal.get("code") != expected["code"] or terminal.get("source") != expected["source"] or terminal.get("child_code") is not None:
        raise ProfileRunError("timeout: runner-imposed exit classification was not faithful")
    if not any(event.get("event") == expected["evidence_event"] and event.get("reason") == "overall" for event in events):
        raise ProfileRunError("timeout: lifecycle stream lacks the overall timeout record")
    return {
        "id": "timeout", "status": "passed", "command_exit_code": completed.returncode,
        "events": events, "terminal_event": terminal, "capture_event": captured, "child_outcome": None,
    }


def _wait_for_terminal(path: Path, profile: dict[str, Any], deadline: float = 10.0) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    expires = time.monotonic() + deadline
    last_error: Exception | None = None
    while time.monotonic() < expires:
        if path.is_file():
            try:
                events = _read_events(path)
                return events, _terminal(events, profile)
            except ProfileRunError as exc:
                last_error = exc
        time.sleep(0.05)
    raise ProfileRunError(f"detached run did not publish terminal lifecycle evidence: {last_error}")


def _cancel_scenario(processkit_cli: Path, directory: Path, profile: dict[str, Any]) -> dict[str, Any]:
    events_path = directory / "events.jsonl"
    capture_dir = directory / "capture"
    capture_dir.mkdir(parents=True)
    run_id = f"vcs-agent-profile-{uuid.uuid4().hex}"
    started = _run([
        str(processkit_cli), "run", "--detach", "--run-id", run_id,
        "--cwd", str(ROOT), "--jsonl", str(events_path), "--capture-dir", str(capture_dir),
        "--capture-max-bytes", "1m", "--no-echo", "--grace", "0", "--",
        sys.executable, str(Path(__file__).resolve()), "--_child", "sleep", "30",
    ])
    if started.returncode != 0:
        raise ProfileRunError(f"control-cancel: detached start failed with {started.returncode}")
    cancel_completed: subprocess.CompletedProcess[bytes] | None = None
    wait_completed: subprocess.CompletedProcess[bytes] | None = None
    try:
        cancel_completed = _run([str(processkit_cli), "cancel", "--run-id", run_id])
        if cancel_completed.returncode != 0:
            raise ProfileRunError(f"control-cancel: public cancel failed with {cancel_completed.returncode}")
        wait_completed = _run([str(processkit_cli), "wait", "--run-id", run_id, "--timeout", "10s"], timeout=15.0)
        if wait_completed.returncode != 0:
            raise ProfileRunError(f"control-cancel: public wait failed with {wait_completed.returncode}")
        events, terminal = _wait_for_terminal(events_path, profile)
    finally:
        if not events_path.is_file() or not any('"event":"runner_exit"' in line.replace(" ", "") for line in events_path.read_text(encoding="utf-8", errors="ignore").splitlines()):
            _run([str(processkit_cli), "cancel", "--run-id", run_id])
            _run([str(processkit_cli), "wait", "--run-id", run_id, "--timeout", "10s"], timeout=15.0)
    expected = profile["lifecycle"]["control_cancel"]
    if terminal.get("code") != expected["code"] or terminal.get("source") != expected["source"] or terminal.get("child_code") is not None:
        raise ProfileRunError("control-cancel: terminal classification drifted")
    if not any(event.get("event") == expected["evidence_event"] and event.get("source") == "control_cancel" for event in events):
        raise ProfileRunError("control-cancel: lifecycle stream lacks control cancellation evidence")
    captured = _captured_event(events, profile)
    return {
        "id": "control-cancel", "status": "passed", "command_exit_code": started.returncode,
        "cancel_exit_code": cancel_completed.returncode if cancel_completed else None,
        "wait_exit_code": wait_completed.returncode if wait_completed else None,
        "events": events, "terminal_event": terminal, "capture_event": captured, "child_outcome": None,
    }


def _capture_scenario(processkit_cli: Path, directory: Path, profile: dict[str, Any]) -> dict[str, Any]:
    completed, events, terminal, captured, capture_dir = _run_foreground(
        processkit_cli,
        [sys.executable, str(Path(__file__).resolve()), "--_child", "emit", "4096"],
        directory,
        profile,
        capture_bytes="1024",
    )
    if completed.returncode != 0 or terminal.get("code") != 0 or terminal.get("source") != "child_exit" or terminal.get("child_code") != 0:
        raise ProfileRunError("bounded-capture: truncation changed child exit classification")
    for stream in ("stdout", "stderr"):
        info = captured[stream]
        if info.get("bytes", 0) < 4096 or info.get("truncated") is not True:
            raise ProfileRunError(f"bounded-capture: {stream} did not disclose truncation")
        if (capture_dir / f"{stream}.log").stat().st_size != 1024:
            raise ProfileRunError(f"bounded-capture: {stream} file did not respect its byte ceiling")
    return {
        "id": "bounded-capture", "status": "passed", "command_exit_code": completed.returncode,
        "events": events, "terminal_event": terminal, "capture_event": captured, "child_outcome": None,
    }


def _nested_scenario(processkit_cli: Path, vcs_agent: Path, repository: Path, directory: Path, profile: dict[str, Any]) -> dict[str, Any]:
    scenario = _agent_scenario(
        "nested-containment", processkit_cli, vcs_agent,
        ["inspect", "--repo", str(repository), "--max-output-bytes", "1048576"],
        0, "inspect", "success", directory, profile,
    )
    events = scenario["events"]
    cleanup = [event for event in events if event.get("event") == profile["lifecycle"]["containment"]["cleanup_event"]]
    started = [event for event in events if event.get("event") == profile["lifecycle"]["containment"]["start_event"]]
    if len(started) != 1 or any(field not in started[0] for field in profile["lifecycle"]["containment"]["observed_fields"]):
        raise ProfileRunError("nested-containment: outer start event lacks mechanism evidence")
    if len(cleanup) != 1 or cleanup[0].get("remaining") != 0 or cleanup[0].get("read_error") is not False:
        raise ProfileRunError("nested-containment: outer cleanup was not confirmed empty")
    scenario["claim"] = profile["lifecycle"]["containment"]["claim"]
    return scenario


def run_profile(processkit_cli: Path, vcs_agent: Path, repository: Path, profile: dict[str, Any]) -> dict[str, Any]:
    probe, schema, schema_hash = _probe(processkit_cli, profile)
    with tempfile.TemporaryDirectory(prefix="vcs-agent-processkit-profile-") as raw_temp:
        temporary = Path(raw_temp)
        scenarios = [
            _agent_scenario("agent-success", processkit_cli, vcs_agent, ["probe"], 0, "probe", "success", temporary / "agent-success", profile),
            _agent_scenario("agent-structured-failure", processkit_cli, vcs_agent, ["__processkit_profile_unknown__"], 10, "unknown", "error", temporary / "agent-failure", profile),
            _timeout_scenario(processkit_cli, temporary / "timeout", profile),
            _cancel_scenario(processkit_cli, temporary / "cancel", profile),
            _capture_scenario(processkit_cli, temporary / "capture", profile),
            _nested_scenario(processkit_cli, vcs_agent, repository, temporary / "nested", profile),
        ]
        _validate_runtime_records(scenarios, schema)
    return {
        "evidence_version": "vcs-agent.processkit-cli.runtime-evidence/v1",
        "profile_version": profile["profile_version"],
        "status": "passed",
        "probe": probe,
        "lifecycle_schema_sha256": schema_hash,
        "scenarios": scenarios,
    }


def _write_evidence(path: Path, evidence: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
    temporary.write_bytes(_json_bytes(evidence))
    temporary.replace(path)


def _child_main(argv: list[str]) -> int:
    if len(argv) != 2 or argv[0] not in {"sleep", "emit"}:
        return 2
    try:
        amount = int(argv[1])
    except ValueError:
        return 2
    if argv[0] == "sleep":
        time.sleep(amount)
        return 0
    payload = b"x" * amount
    os.write(sys.stdout.fileno(), payload)
    os.write(sys.stderr.fileno(), payload)
    return 0


def main(argv: list[str] | None = None) -> int:
    raw_args = list(sys.argv[1:] if argv is None else argv)
    if raw_args[:1] == ["--_child"]:
        return _child_main(raw_args[1:])

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--processkit-cli", help="Explicit provided ProcessKit-CLI binary (or set PROCESSKIT_CLI_BIN)")
    parser.add_argument("--vcs-agent", type=Path, required=True, help="Built vcs-agent executable")
    parser.add_argument("--repo", type=Path, default=ROOT, help="Repository used by the nested inspect scenario")
    parser.add_argument("--profile", type=Path, default=ROOT / "docs/agent-interface/processkit-cli-profile.v1.json")
    parser.add_argument("--evidence-output", type=Path, help="Write full validated runtime evidence only after all scenarios pass")
    args = parser.parse_args(raw_args)
    try:
        profile = validate.validate_processkit_cli_profile(validate.load_json(args.profile))
        processkit_cli, provided_by = _provided_processkit_cli(args.processkit_cli)
        if processkit_cli is None:
            print(json.dumps({
                "profile_version": profile["profile_version"],
                "status": profile["gating"]["unavailable_status"],
                "reason": "processkit_cli_not_provided",
                "environment_variable": profile["gating"]["environment_variable"],
            }, sort_keys=True))
            return 0
        vcs_agent = _resolve_executable(str(args.vcs_agent), "--vcs-agent")
        repository = args.repo.resolve(strict=True)
        evidence = run_profile(processkit_cli, vcs_agent, repository, profile)
        evidence["provided_by"] = provided_by
        if args.evidence_output is not None:
            _write_evidence(args.evidence_output, evidence)
        print(json.dumps({
            "profile_version": profile["profile_version"],
            "status": "passed",
            "processkit_cli_version": evidence["probe"].get("version"),
            "scenario_count": len(evidence["scenarios"]),
            "evidence_output": str(args.evidence_output) if args.evidence_output else None,
        }, sort_keys=True))
        return 0
    except (ProfileRunError, validate.ValidationError, OSError) as exc:
        print(json.dumps({"status": "failed", "reason": str(exc)}, sort_keys=True), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
