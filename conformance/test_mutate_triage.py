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


if __name__ == "__main__":
    unittest.main()
