"""Failure-path guards for the published-example regression gate."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import base


class BaseGateTests(unittest.TestCase):
    def setUp(self):
        self.row = {"file": "Patient-example.json", "input_sha256": "input", "ours": "valid",
                    "oracle": "invalid", "oracle_signature": "oracle"}
        self.triage = {"differences": {self.row["file"]: {k: v for k, v in self.row.items() if k != "file"}},
                       "oracle_exclusions": {}}

    def test_new_disagreements_fail_in_both_directions(self):
        for ours, oracle in [("valid", "invalid"), ("invalid", "valid")]:
            row = dict(self.row, file="new.json", ours=ours, oracle=oracle)
            self.assertEqual(base.assess(row, self.triage), "new verdict disagreement")

    def test_reviewed_difference_cannot_mask_changed_input_outcome_or_improvement(self):
        self.assertIsNone(base.assess(self.row, self.triage))
        for field, changed in [("input_sha256", "changed"), ("oracle_signature", "changed"),
                               ("ours", "invalid"), ("oracle", "valid")]:
            self.assertIsNotNone(base.assess(dict(self.row, **{field: changed}), self.triage))

    def test_execution_failures_are_never_agreement(self):
        for ours, oracle in [("error", "error"), ("invalid", "error"), ("error", "invalid")]:
            self.assertIsNotNone(base.assess(dict(self.row, ours=ours, oracle=oracle), self.triage))

    def test_exclusions_remain_unscored_and_do_not_mask_native_failure(self):
        triage = {"differences": {}, "oracle_exclusions": {self.row["file"]: self.row}}
        row = dict(self.row, oracle="excluded")
        self.assertIsNone(base.assess(row, triage))
        for changes in [{"ours": "error"}, {"ours": "invalid"}, {"input_sha256": "changed"}, {"oracle": "valid"}]:
            self.assertIsNotNone(base.assess(dict(row, **changes), triage))

    def test_oracle_reader_rejects_partial_duplicate_foreign_and_malformed_results(self):
        def oo(name, severity="information"):
            return {"resourceType": "OperationOutcome", "issue": [{"severity": severity}],
                    "extension": [{"url": "http://example.org/operationoutcome-file", "valueString": name}]}
        def bundle(*outcomes):
            return {"resourceType": "Bundle", "entry": [{"resource": o} for o in outcomes]}
        for data in [bundle(oo("a")), bundle(oo("a"), oo("a")), bundle(oo("a"), oo("foreign")),
                     bundle(oo("a"), oo("b", "unknown")), [], {"resourceType": "Patient"},
                     bundle(oo("a"), {"resourceType": "OperationOutcome", "issue": {}})]:
            with self.assertRaises(ValueError):
                base.read_oracle(data, {"a", "b"})
        self.assertEqual(set(base.read_oracle(bundle(oo("a"), oo("b")), {"a", "b"})), {"a", "b"})

    def test_cache_checks_configuration_input_bytes_and_outcome_hash(self):
        outcome = {"resourceType": "OperationOutcome", "issue": []}
        config = {"jar_sha256": "jar", "igs": ["ig#1"]}
        cache = {"format": 1, "configuration": config, "results": {"a": {
            "input_sha256": "input", "outcome_sha256": base.json_hash(outcome), "outcome": outcome}}}
        self.assertEqual(base.cached_outcomes(cache, config, {"a": "input"}), {"a": outcome})
        for cfg, inputs in [({"jar_sha256": "new"}, {"a": "input"}), (config, {"a": "new"}), (config, {"b": "input"})]:
            with self.assertRaises(ValueError):
                base.cached_outcomes(cache, cfg, inputs)
        corrupt = copy.deepcopy(cache)
        corrupt["results"]["a"]["outcome"]["issue"] = [{"severity": "error"}]
        with self.assertRaises(ValueError):
            base.cached_outcomes(corrupt, config, {"a": "input"})

    def test_changed_inventory_and_dangling_exceptions_fail(self):
        inputs = {"a": "hash"}
        triage = {"format": 1, "configuration": {}, "inventory": {"count": 1, "sha256": base.json_hash(inputs)},
                  "differences": {"a": {"input_sha256": "hash", "reasons": ["known"]}},
                  "oracle_exclusions": {}, "reasons": {"known": "A reviewed reason"}}
        base.check_inventory(triage, inputs, {})
        for changed in [{"a": "changed"}, {}, {"a": "hash", "b": "new"}]:
            with self.assertRaises(ValueError):
                base.check_inventory(triage, changed, {})
        triage["differences"]["missing"] = triage["differences"]["a"]
        with self.assertRaises(ValueError):
            base.check_inventory(triage, inputs, {})

    def test_empty_selection_fails_without_replacing_full_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            full = root / "evidence.json"
            full.write_text('"prior full evidence"')
            triage = root / "triage.json"
            triage.write_text('{}')
            with patch.object(base, "BASE", root), patch.object(base, "TRIAGE", triage), \
                 patch.object(base, "ensure_examples"), patch.object(base, "configuration", return_value={}), \
                 patch.object(base, "check_inventory"), patch.object(base, "select", return_value=[]):
                self.assertEqual(base.main(["--filter", "absent"]), 1)
            self.assertEqual(json.loads(full.read_text()), "prior full evidence")
            self.assertIn("matched no examples", (root / "evidence-subset.json").read_text())


if __name__ == "__main__":
    unittest.main()
