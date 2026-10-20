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

    def test_key_partners_are_derived_from_the_base_structures(self):
        def sd(url, key, expression, derivation="specialization"):
            return {"resourceType": "StructureDefinition", "url": url, "derivation": derivation,
                    "differential": {"element": [{"constraint": [{"key": key, "expression": expression}]}]}}
        with tempfile.TemporaryDirectory() as directory:
            core = Path(directory) / "core"
            core.mkdir()
            for name, data in {"A": sd("u/A", "inv-1", "a"), "B": sd("u/B", "inv-1", "b"), "C": sd("u/C", "inv-1", "a"),
                               "P": sd("u/P", "inv-1", "p", derivation="constraint")}.items():
                (core / f"StructureDefinition-{name}.json").write_text(json.dumps(data))
            keys = base.base_structure_keys(core)
            self.assertEqual(set(keys["inv-1"]), {"u/A", "u/B", "u/C"})
            own = Path(directory) / "A.json"
            own.write_text(json.dumps(sd("u/A", "inv-1", "a")))
            self.assertEqual(base.key_partners(own, keys), {("inv-1", "u/B")})
            ext = Path(directory) / "x.json"
            ext.write_text(json.dumps(sd("u/X", "inv-1", "x", derivation="constraint")))
            self.assertEqual(base.key_partners(ext, keys), {("inv-1", "u/A"), ("inv-1", "u/B"), ("inv-1", "u/C")})

    def test_an_order_dependent_case_accepts_exactly_what_its_partners_explain(self):
        used = "The constraint key 'inv-1' already exists at the location '{}|4.0.1' with a different expression ('x')"
        partners = [["inv-1", "u/B"]]
        self.assertTrue(base.order_dependent_outcome_ok([], partners))
        self.assertTrue(base.order_dependent_outcome_ok([used.format("u/B")], partners))
        self.assertFalse(base.order_dependent_outcome_ok([used.format("u/Z")], partners))
        self.assertFalse(base.order_dependent_outcome_ok(["something else"], partners))
        self.assertTrue(base.order_dependent_outcome_ok(["Error in constraint 'inv-1' x", used.format("u/B")], partners,
                                                        ["Error in constraint 'inv-1'"]))
        triage = {"differences": {}, "oracle_exclusions": {}, "oracle_order_dependent": {"Task.json": {
            "input_sha256": "input", "ours": "valid", "reasons": ["oracle-order-dependent"]}}}
        row = {"file": "Task.json", "input_sha256": "input", "ours": "valid", "oracle": "invalid",
               "oracle_errors": [used.format("u/B")], "key_partners": partners}
        self.assertIsNone(base.assess(row, triage))
        self.assertIsNone(base.assess(dict(row, oracle="valid", oracle_errors=[]), triage))
        self.assertIsNotNone(base.assess(dict(row, oracle_errors=[used.format("u/Z")]), triage))
        self.assertIsNotNone(base.assess(dict(row, ours="invalid"), triage))

    def test_a_reviewed_difference_with_an_order_dependent_message_needs_its_own_error(self):
        used = "The constraint key 'inv-1' already exists at the location '{}|4.0.1' with a different expression ('x')"
        own = "Error in constraint 'inv-1' with expression 'e'"
        triage = {"oracle_exclusions": {}, "differences": {"Ext.json": {
            "input_sha256": "input", "ours": "valid", "oracle": "invalid", "reasons": ["oracle-order-dependent"],
            "order_dependent": {"own_errors": [own]}}}}
        row = {"file": "Ext.json", "input_sha256": "input", "ours": "valid", "oracle": "invalid",
               "oracle_signature": "any", "key_partners": [["inv-1", "u/B"], ["inv-1", "u/C"]],
               "oracle_errors": [used.format("u/B"), own + ": FHIRPath error"]}
        self.assertIsNone(base.assess(row, triage))
        self.assertIsNone(base.assess(dict(row, oracle_errors=[used.format("u/C"), own + ": x"]), triage))
        # the own error must be there, and nothing unexplained beside it
        self.assertIsNotNone(base.assess(dict(row, oracle_errors=[used.format("u/B")]), triage))
        self.assertIsNotNone(base.assess(dict(row, oracle_errors=[used.format("u/Z"), own]), triage))
        self.assertIsNotNone(base.assess(dict(row, oracle_errors=[own, "another error"]), triage))

    def test_drift_compares_outcomes_without_their_source_path(self):
        def oo(path, issues):
            return {"resourceType": "OperationOutcome", "issue": issues,
                    "extension": [{"url": "http://hl7.org/fhir/StructureDefinition/" + base.run.FILE_EXT, "valueString": path}]}
        before = oo("/checkout-a/x.json", [])
        previous = {"configuration": {}, "results": {"x.json": {"outcome_sha256": base.json_hash(before), "outcome": before}}}
        self.assertEqual(base.oracle_drift(previous, {}, {"x.json": oo("/checkout-b/x.json", [])}), {})
        moved = base.oracle_drift(previous, {}, {"x.json": oo("/checkout-b/x.json", [{"severity": "error", "code": "invalid"}])})
        self.assertEqual(set(moved), {"x.json"})

    def test_an_outcome_identity_ignores_the_source_path(self):
        def oo(path):
            return {"resourceType": "OperationOutcome", "issue": [],
                    "extension": [{"url": "http://hl7.org/fhir/StructureDefinition/" + base.run.FILE_EXT, "valueString": path}]}
        self.assertEqual(base.outcome_identity(oo("/a/x.json")), base.outcome_identity(oo("/b/x.json")))
        self.assertNotEqual(base.json_hash(oo("/a/x.json")), base.json_hash(oo("/b/x.json")))

    def test_oracle_drift_is_told_apart_from_a_pod_change(self):
        before = {"resourceType": "OperationOutcome", "issue": []}
        after = {"resourceType": "OperationOutcome", "issue": [{"severity": "error", "code": "invalid"}]}
        previous = {"configuration": {}, "results": {"Patient-example.json": {
            "outcome_sha256": base.json_hash(before), "outcome": before}}}
        drift = base.oracle_drift(previous, {}, {"Patient-example.json": after})
        self.assertEqual(drift["Patient-example.json"]["previous_oracle"], "valid")
        self.assertIsNone(base.oracle_drift(previous, {"other": 1}, {"Patient-example.json": after}))
        changed = dict(self.row, oracle_signature="moved")
        self.assertTrue(base.drift_only(changed, "reviewed difference changed (oracle_signature); review", self.triage, drift))
        self.assertFalse(base.drift_only(changed, "reviewed difference changed (oracle_signature); review", self.triage, {}))
        self.assertFalse(base.drift_only(dict(changed, ours="invalid"), "reviewed difference changed (ours)", self.triage, drift))
        fresh = {"file": "Patient-example.json", "ours": "valid", "oracle": "invalid"}
        self.assertTrue(base.drift_only(fresh, "new verdict disagreement", {"differences": {}}, drift))


if __name__ == "__main__":
    unittest.main()
