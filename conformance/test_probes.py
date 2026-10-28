"""The probe lane's verdicts: agreement, a reviewed difference bound to the
exact input and oracle answer it was reviewed on, and every way a review
goes stale."""
import importlib.util
import unittest
from pathlib import Path

# loaded by path: `conformance/run.py` is also a module named `run`
_spec = importlib.util.spec_from_file_location("probes_run", Path(__file__).resolve().parent / "probes" / "run.py")
run = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(run)

ERROR = {"severity": "error", "code": "structure", "text": "max allowed = 1, but found 2", "paths": ["Patient.address[0]"]}
WARNING = {"severity": "warning", "code": "invariant", "text": "dom-6", "paths": ["Patient"]}


class Signature(unittest.TestCase):
    def test_bound_to_errors_not_warnings_or_order(self):
        a = run.oracle_signature([ERROR, WARNING])
        self.assertEqual(a, run.oracle_signature([WARNING, ERROR]))
        self.assertEqual(a, run.oracle_signature([ERROR]))
        self.assertNotEqual(a, run.oracle_signature([WARNING]))
        self.assertNotEqual(a, run.oracle_signature([{**ERROR, "text": "max allowed = 1, but found 3"}]))
        self.assertNotEqual(a, run.oracle_signature([{**ERROR, "paths": ["Patient.address[1]"]}]))


class Judge(unittest.TestCase):
    SIG = run.oracle_signature([ERROR])

    def item(self, review=True, **changed):
        item = {"file": "p.json", "expect": False}
        if review:
            item["reviewed"] = {"sha256": "h", "oracle": self.SIG, "local_valid": True, "reason": "why", **changed}
        return item

    def test_agreement(self):
        self.assertEqual(run.judge(self.item(False), "h", False, self.SIG, False), ("agree", None))
        self.assertEqual(run.judge(self.item(False), "h", False, self.SIG, None), ("agree", None))

    def test_an_unreviewed_difference_fails(self):
        self.assertEqual(run.judge(self.item(False), "h", False, self.SIG, True)[0], "differ")

    def test_a_reviewed_difference_counts_while_everything_matches(self):
        self.assertEqual(run.judge(self.item(), "h", False, self.SIG, True), ("reviewed", "why"))

    def test_a_review_goes_stale_on_any_change(self):
        self.assertEqual(run.judge(self.item(), "other", False, self.SIG, True), ("differ", "stale review: sha256"))
        other = run.oracle_signature([{**ERROR, "text": "changed"}])
        self.assertEqual(run.judge(self.item(), "h", False, other, True), ("differ", "stale review: oracle"))
        self.assertEqual(run.judge(self.item(reason=""), "h", False, self.SIG, True), ("differ", "stale review: reason"))

    def test_a_review_of_an_agreement_fails(self):
        self.assertEqual(run.judge(self.item(), "h", False, self.SIG, False)[0], "differ")

    def test_the_oracle_must_give_the_expected_verdict(self):
        self.assertEqual(run.judge(self.item(), "h", True, self.SIG, True)[0], "differ")


if __name__ == "__main__":
    unittest.main()
