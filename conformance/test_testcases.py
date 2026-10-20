"""Evidence and classification guards for the published test-case lane."""
import json
from pathlib import Path
import tempfile
import unittest
import subprocess
import sys
from unittest.mock import patch
from types import SimpleNamespace

import testcases as lane
import testcase_profiles as profiles


class EvidenceTests(unittest.TestCase):
    def test_empty_stored_outcome_is_valid_but_foreign_json_is_not(self):
        self.assertEqual(lane.java_verdict({"resourceType": "OperationOutcome"})[0], "valid")
        for value in [None, {}, [], {"resourceType": "Patient"},
                      {"resourceType": "OperationOutcome", "issue": None},
                      {"resourceType": "OperationOutcome", "issue": [{}]}]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                lane.java_verdict(value)

    def test_fatal_is_invalid_and_paths_use_the_gate_normalization(self):
        value = {"resourceType": "OperationOutcome", "issue": [{"severity": "fatal", "expression": ["Patient.name[2].given[0]"]}]}
        verdict, paths = lane.java_verdict(value)
        self.assertEqual(verdict, "invalid")
        self.assertEqual(paths, {"Patient.name.given"})

    def test_inline_and_file_outcomes_share_validation(self):
        value = {"resourceType": "OperationOutcome", "issue": []}
        self.assertEqual(lane.read_outcome({"outcome": value}), value)
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "outcome.json"
            path.write_text(json.dumps(value))
            with patch.object(lane, "suite_file", return_value=path):
                self.assertEqual(lane.read_outcome("java/example.json"), value)
        with self.assertRaises(ValueError):
            lane.read_outcome({"errorCount": 0})

    def test_firely_absence_is_not_a_valid_result(self):
        self.assertEqual(lane.firely_verdict([]), "valid")
        self.assertEqual(lane.firely_verdict(["[ERROR] synthetic"]), "invalid")
        for value in [None, {}, [42], ["unrecognized"]]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                lane.firely_verdict(value)

    def test_manifest_flags_are_classified_without_reading_the_verdict(self):
        self.assertIsNone(lane.scope({"module": "general", "use-test": False}))
        self.assertEqual(lane.scope({"validateContains": "IGNORE"})["manifest_key"], "validateContains")
        self.assertIsNone(lane.scope({"packages": ["hl7.fhir.au.base#6.0.0"]}))
        self.assertEqual(lane.scope({"packages": ["not.carried#1"]})["manifest_key"], "packages")

    def test_review_cannot_hide_changed_inputs_or_verdicts(self):
        row = {"name": "case", "input_sha256": "input", "java_sha256": "java", "firely_sha256": None,
               "ours": "valid", "java": "invalid", "firely": None}
        decision = {**row, "kind": "documented-gap", "reason": "uncompiled resource invariant"}
        self.assertEqual(lane.review(row, {"case": decision})["status"], "reviewed")
        for key, value in [("input_sha256", "new"), ("java_sha256", "new"), ("firely", "valid")]:
            with self.subTest(key=key):
                self.assertEqual(lane.review({**row, key: value}, {"case": decision})["status"], "stale-review")
        self.assertEqual(lane.review(row, {})["status"], "unreviewed")

    def test_a_triaged_case_that_now_agrees_fails_the_lane(self):
        row = {"name": "case", "input_sha256": "input", "java_sha256": "java", "firely_sha256": None,
               "ours": "invalid", "java": "invalid", "firely": None}
        decision = {**row, "ours": "valid", "kind": "documented-gap", "reason": "a gap since closed"}
        self.assertEqual(lane.review(row, {"case": decision})["status"], "stale-review")
        self.assertEqual(lane.review(row, {})["status"], "agreement")
        counts = lane.summarize([{**row, "review": lane.review(row, {"case": decision})}], [], [])
        self.assertEqual(counts["unreviewed"], 1)

    def test_missing_firely_is_not_two_engine_agreement(self):
        row = {"ours": "invalid", "java": "valid", "firely": None, "review": {"status": "unreviewed"}}
        counts = lane.summarize([row], [], [])
        self.assertEqual(counts["firely_compared"], 0)
        self.assertEqual(counts["stricter_than_both_stored_engines"], 0)
        self.assertEqual(counts["unresolved_stricter"], 1)

    def test_probe_failure_and_exit_mismatch_are_never_verdicts(self):
        outcome = json.dumps({"resourceType": "OperationOutcome"})
        for code, stdout in [(-9, outcome), (2, outcome), (1, outcome), (0, "")]:
            proc = SimpleNamespace(returncode=code, stdout=stdout)
            with self.subTest(code=code), patch.object(lane.subprocess, "run", return_value=proc), self.assertRaises(ValueError):
                lane.native(Path("probe"), Path("fixture"))

    def test_profile_compiler_accepts_an_optional_resource_id(self):
        with tempfile.TemporaryDirectory() as folder:
            package = Path(folder) / "package"
            package.mkdir()
            sd = {"resourceType": "StructureDefinition", "url": "https://example.org/StructureDefinition/no-id",
                  "type": "Patient", "kind": "resource", "derivation": "constraint",
                  "snapshot": {"element": [{"id": "Patient", "path": "Patient", "min": 0, "max": "*"}]}}
            (package / "StructureDefinition-no-id.json").write_text(json.dumps(sd))
            output = Path(folder) / "generated"
            subprocess.run([sys.executable, str(lane.ROOT / "tools/gen_all.py"), str(output), "synthetic test", str(package)],
                           check=True, capture_output=True)
            text = (output / "no_id.rs").read_text()
            self.assertIn('name: "https://example.org/StructureDefinition/no-id"', text)
            self.assertIn("&no_id::PROFILE", (output / "mod.rs").read_text())

    def test_profile_gaps_are_named_even_when_the_differential_omits_types(self):
        sys.path.insert(0, str(lane.ROOT / "tools"))
        element = {"id": "Observation.value[x]", "path": "Observation.value[x]", "min": 0, "max": "1",
                   "type": [{"code": "Quantity"}], "binding": {"strength": "required", "valueSet": "https://example.org/vs"}}
        sd = {"type": "Observation", "snapshot": {"element": [element]}, "differential": {"element": [
            {k: v for k, v in element.items() if k != "type"}]}}
        gaps = profiles.compiler_gaps(sd, lambda _: None)
        self.assertIn("required Quantity unit binding", [gap["reason"] for gap in gaps])
        element["binding"] = {"strength": "example", "extension": [{
            "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-ElementDefinition.binding.additional",
            "extension": [{"url": "purpose", "valueCode": "required"}]}]}
        sd["differential"]["element"] = [element]
        self.assertIn("additional terminology bindings", [gap["reason"] for gap in profiles.compiler_gaps(sd, lambda _: None)])


if __name__ == "__main__":
    unittest.main()
