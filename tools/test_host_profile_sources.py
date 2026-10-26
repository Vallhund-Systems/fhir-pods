import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from host_profile_sources import record_sources, source_identities, stage_packages

URL = "https://profiles.example.invalid/StructureDefinition/score"

def definition(value):
    return {"resourceType":"StructureDefinition","url":URL,"kind":"resource","type":"Observation","derivation":"constraint","snapshot":{"element":[{"path":"Observation.valueInteger","fixedInteger":value}]}}

class HostProfileSources(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.package = self.root / "package"
        self.package.mkdir()
        self.source = self.package / "profile.json"
        self.source.write_text(json.dumps(definition(3), indent=2) + "\n")
        self.generated = self.root / "generated"
        self.generated.mkdir()
        (self.generated / "mod.rs").write_text("// previous generated registry\n")
        (self.generated / "score.rs").write_text(f'    url: "{URL}",\n')
    def tearDown(self):
        self.temp.cleanup()
    def test_staged_bytes_are_both_compiled_input_and_recorded_identity(self):
        original = self.source.read_bytes()
        staged = stage_packages(self.root / "staged", [self.package])
        self.source.write_text(json.dumps(definition(999)))
        record_sources(self.generated, staged)
        output = (self.generated / "mod.rs").read_text()
        self.assertIn(hashlib.sha256(original).hexdigest(), output)
        self.assertNotIn(hashlib.sha256(self.source.read_bytes()).hexdigest(), output)
        self.assertEqual((staged[0] / self.source.name).read_bytes(), original)
    def test_same_url_changed_constraints_change_the_source_identity(self):
        before = source_identities([self.package])[URL]
        self.source.write_text(json.dumps(definition(999), indent=2)+"\n")
        self.assertNotEqual(source_identities([self.package])[URL], before)
    def test_raw_formatting_is_part_of_the_file_identity(self):
        before = source_identities([self.package])[URL]
        self.source.write_text(json.dumps(definition(3)))
        self.assertNotEqual(source_identities([self.package])[URL], before)
    def test_duplicate_canonicals_refuse_without_replacing_the_registry(self):
        (self.package / "duplicate.json").write_bytes(self.source.read_bytes())
        before=(self.generated / "mod.rs").read_bytes()
        with self.assertRaisesRegex(ValueError,"duplicate host profile canonical"):
            record_sources(self.generated,[self.package])
        self.assertEqual((self.generated / "mod.rs").read_bytes(),before)
    def test_uncompiled_source_is_not_attested(self):
        (self.generated / "score.rs").unlink()
        before=(self.generated / "mod.rs").read_bytes()
        with self.assertRaisesRegex(ValueError,"source was not compiled"):
            record_sources(self.generated,[self.package])
        self.assertEqual((self.generated / "mod.rs").read_bytes(),before)

if __name__ == "__main__":
    unittest.main()
