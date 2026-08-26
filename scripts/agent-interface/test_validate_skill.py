#!/usr/bin/env python3

from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import validate_skill  # noqa: E402


class SkillDocumentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.root = SCRIPT_DIR.parents[1]
        cls.skill = cls.root / "skills/vcs-agent/SKILL.md"
        cls.contract = validate_skill.load_json(
            cls.root / "skills/vcs-agent/references/contract.v1.json"
        )
        cls.profile = validate_skill.load_json(
            cls.root / "docs/agent-interface/processkit-cli-profile.v1.json"
        )

    def test_committed_skill_matches_declared_contracts(self) -> None:
        _, metadata = validate_skill.validate_documents(
            self.skill, self.contract, self.profile
        )
        self.assertEqual(metadata["name"], "vcs-agent")

    def test_rejects_extra_fallback_ground(self) -> None:
        contract = copy.deepcopy(self.contract)
        contract["fallback_reasons"].append("any_failure")
        with self.assertRaisesRegex(
            validate_skill.SkillValidationError, "three allowed grounds"
        ):
            validate_skill.validate_documents(self.skill, contract, self.profile)

    def test_rejects_processkit_threshold_drift(self) -> None:
        contract = copy.deepcopy(self.contract)
        contract["processkit_cli_profile"]["minimum_expected_duration_seconds"] = 1
        with self.assertRaisesRegex(
            validate_skill.SkillValidationError, "60 seconds"
        ):
            validate_skill.validate_documents(self.skill, contract, self.profile)

    def test_rejects_metadata_that_would_activate_for_file_editing(self) -> None:
        contract = copy.deepcopy(self.contract)
        contract["metadata"]["description"] = "Use for every source edit."
        with self.assertRaisesRegex(
            validate_skill.SkillValidationError, "metadata differs"
        ):
            validate_skill.validate_documents(self.skill, contract, self.profile)


if __name__ == "__main__":
    unittest.main()
