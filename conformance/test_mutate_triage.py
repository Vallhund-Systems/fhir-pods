"""The mutation lane's triage binds an entry to one file as it stands."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mutate  # noqa: E402


class Accepts(unittest.TestCase):
    row = {"file": "f.json", "ours": "valid", "oracle": "invalid", "sha256": "a" * 64,
           "oracle_signature": mutate.oracle_signature(["error: x @ ['P']", "error: y @ ['Q']"])}

    def test_exact_entry_accepts(self):
        self.assertTrue(mutate.accepts(dict(self.row), self.row))

    def test_signature_is_order_free(self):
        self.assertEqual(mutate.oracle_signature(["b", "a"]), mutate.oracle_signature(["a", "b"]))

    def test_any_moved_part_refuses(self):
        for key, value in (("sha256", "b" * 64), ("oracle_signature", mutate.oracle_signature(["error: z"])),
                           ("ours", "invalid"), ("oracle", "valid")):
            entry = dict(self.row, **{key: value})
            self.assertFalse(mutate.accepts(entry, self.row), key)

    def test_no_entry_refuses(self):
        self.assertFalse(mutate.accepts(None, self.row))


class FamilyLines(unittest.TestCase):
    rows = [{"file": f"f{i}.json", "missed": []} for i in range(3)]

    def test_a_partly_accepted_family_lists_only_the_files_owed_a_review(self):
        lines, owed = mutate.family_lines("lenient", "fam", self.rows, self.rows[:2], True, 12)
        self.assertEqual(owed, 1)
        self.assertEqual(lines, [
            "  lenient     2  triaged   fam",
            "  lenient     1  UNTRIAGED fam",
            "             f2.json",
        ])

    def test_a_wholly_accepted_family_is_one_triaged_line(self):
        lines, owed = mutate.family_lines("lenient", "fam", self.rows, list(self.rows), True, 12)
        self.assertEqual((lines, owed), (["  lenient     3  triaged   fam"], 0))

    def test_an_unaccepted_family_lists_every_file(self):
        lines, owed = mutate.family_lines("stricter", "fam", self.rows, [], True, 2)
        self.assertEqual(owed, 3)
        self.assertEqual(lines, ["  stricter    3  UNTRIAGED fam", "             f0.json", "             f1.json"])

    def test_an_ungated_kind_is_reported_and_owes_nothing(self):
        lines, owed = mutate.family_lines("missed", "fam", self.rows[:1], [], False, 12)
        self.assertEqual((lines, owed), (["  missed      1  reported  fam", "             f0.json missed []"], 0))


if __name__ == "__main__":
    unittest.main()
