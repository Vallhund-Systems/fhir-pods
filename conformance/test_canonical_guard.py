"""The shared-canonical guard: a url given two definitions is refused."""
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import canonical_guard  # noqa: E402

URL = "https://example.org/probe/ValueSet/x"


def response(include):
    vs = {"resourceType": "ValueSet", "id": "vs", "url": URL, "status": "active", "compose": {"include": include}}
    return {"resourceType": "QuestionnaireResponse", "status": "completed", "contained": [vs]}


class SharedCanonicals(unittest.TestCase):
    def write(self, directory, name, resource):
        path = Path(directory) / name
        path.write_text(json.dumps(resource))
        return path

    def test_two_composes_under_one_url_are_refused(self):
        with tempfile.TemporaryDirectory() as d:
            a = self.write(d, "a.json", response([{"system": "s", "concept": [{"code": "y"}]}]))
            b = self.write(d, "b.json", response([{"system": "s", "concept": [{"code": "y"}]}, {"system": "t", "concept": [{"code": "y"}]}]))
            self.assertEqual(canonical_guard.shared_canonicals([a, b]), {("ValueSet", URL): ["a.json", "b.json"]})
            with self.assertRaises(SystemExit):
                canonical_guard.refuse_shared_canonicals([a, b])

    def test_the_same_definition_twice_and_distinct_urls_pass(self):
        with tempfile.TemporaryDirectory() as d:
            same = [{"system": "s", "concept": [{"code": "y"}]}]
            a = self.write(d, "a.json", response(same))
            b = self.write(d, "b.json", {**response(same), "id": "other"})
            other = response([{"system": "t"}])
            other["contained"][0]["url"] = URL + "-2"
            c = self.write(d, "c.json", other)
            self.assertEqual(canonical_guard.shared_canonicals([a, b, c]), {})

    def test_a_code_system_with_two_contents_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            cs = {"resourceType": "CodeSystem", "url": "https://example.org/cs", "status": "active", "content": "complete"}
            a = self.write(d, "a.json", {**cs, "concept": [{"code": "y"}]})
            b = self.write(d, "b.json", {"resourceType": "Bundle", "type": "collection",
                                         "entry": [{"resource": {**cs, "concept": [{"code": "n"}]}}]})
            self.assertIn(("CodeSystem", "https://example.org/cs"), canonical_guard.shared_canonicals([a, b]))

    def test_the_committed_probe_lanes_share_no_canonical(self):
        here = Path(__file__).resolve().parent
        for lane in ("coded-answers", "probes"):
            files = [p for p in (here / lane).rglob("*.json") if p.name not in ("manifest.json", "evidence.json")]
            self.assertEqual(canonical_guard.shared_canonicals(files), {}, lane)


if __name__ == "__main__":
    unittest.main()
