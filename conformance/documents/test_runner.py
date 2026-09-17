"""Regression guards against malformed oracle evidence becoming a pass."""
import copy
import unittest

from run import check_graph, outcomes_from


def outcome(file, severity):
    return {"resource": {"resourceType": "OperationOutcome", "issue": [{"severity": severity, "code": "invalid"}],
                         "extension": [{"url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-file", "valueString": "/tmp/" + file}]}}


class EvidenceGuards(unittest.TestCase):
    def setUp(self):
        self.bundle = {"resourceType": "Bundle", "entry": [outcome("one.json", "information"), outcome("two.json", "error")]}
        self.expected = ["one.json", "two.json"]

    def test_valid_and_invalid_outcomes_are_both_evidence(self):
        self.assertEqual(set(outcomes_from(self.bundle, self.expected)), set(self.expected))

    def test_missing_duplicate_and_unexpected_records_fail(self):
        for entries in [[], self.bundle["entry"][:1], self.bundle["entry"] * 2,
                        [*self.bundle["entry"], outcome("extra.json", "warning")]]:
            with self.subTest(entries=len(entries)), self.assertRaises(ValueError):
                outcomes_from({"resourceType": "Bundle", "entry": entries}, self.expected)

    def test_non_outcomes_empty_issues_and_unknown_severity_fail(self):
        for field, value in [("resourceType", "Patient"), ("issue", []), ("issue", [{"code": "invalid"}]),
                             ("issue", [{"severity": "success"}]), ("extension", [])]:
            bundle = copy.deepcopy(self.bundle)
            bundle["entry"][0]["resource"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                outcomes_from(bundle, self.expected)

    def test_successful_graph_control_and_false_omission(self):
        result = {"document": {"entry": [{"fullUrl": "urn:uuid:one", "resource": {"resourceType": "Composition", "id": "c"}}]},
                  "omitted": [], "placed": []}
        self.assertEqual(check_graph({}, result), [])
        self.assertIn("incorrect-omitted-report", check_graph({"omitted": ["Organization/o"]}, result))
        result["document"]["entry"] *= 2
        self.assertEqual(check_graph({}, result), ["missing-or-duplicate-entry-identity"])


if __name__ == "__main__":
    unittest.main()
