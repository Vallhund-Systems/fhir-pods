import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from host_profile_sources import precheck, record_sources, source_identities, stage_packages

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

CP = "https://profiles.example.invalid/StructureDefinition/probe-contactpoint"
PATIENT = "https://profiles.example.invalid/StructureDefinition/probe-patient"


def host_patient(types):
    return {"resourceType": "StructureDefinition", "url": PATIENT, "kind": "resource", "type": "Patient",
            "derivation": "constraint", "snapshot": {"element": [
                {"id": "Patient", "path": "Patient"},
                {"id": "Patient.telecom", "path": "Patient.telecom", "type": [{"code": "ContactPoint", "profile": types}]}]}}


def host_contactpoint(snapshot=True):
    sd = {"resourceType": "StructureDefinition", "url": CP, "kind": "complex-type", "type": "ContactPoint",
          "derivation": "constraint", "differential": {"element": [{"id": "ContactPoint", "path": "ContactPoint"}]}}
    if snapshot:
        sd["snapshot"] = {"element": [{"id": "ContactPoint", "path": "ContactPoint"}]}
    return sd


class HostDatatypeProfiles(unittest.TestCase):
    """A host datatype profile an element is held to alone is compiled
    (gen_profile.ig_datatype_profile), so the host guards cover it too."""
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.package = Path(self.temp.name) / "package"
        self.package.mkdir()
    def tearDown(self):
        self.temp.cleanup()
    def write(self, patient_types, cp):
        (self.package / "patient.json").write_text(json.dumps(host_patient(patient_types)))
        (self.package / "contactpoint.json").write_text(json.dumps(cp))
    def test_held_alone_is_attested(self):
        self.write([CP], host_contactpoint())
        ids = source_identities([self.package])
        self.assertEqual(set(ids), {PATIENT, CP})
        self.assertEqual(ids[CP], hashlib.sha256((self.package / "contactpoint.json").read_bytes()).hexdigest())
    def test_beside_its_base_it_is_not_compiled_and_not_attested(self):
        self.write(["http://hl7.org/fhir/StructureDefinition/ContactPoint", CP], host_contactpoint())
        self.assertEqual(set(source_identities([self.package])), {PATIENT})
    def test_held_alone_without_a_snapshot_is_refused_by_name(self):
        self.write([CP], host_contactpoint(snapshot=False))
        with self.assertRaisesRegex(SystemExit, "contactpoint.json"):
            precheck(set(), [self.package])
        # unreferenced, the same definition is nothing the generator compiles
        self.write(["http://hl7.org/fhir/StructureDefinition/ContactPoint", CP], host_contactpoint(snapshot=False))
        precheck(set(), [self.package])
    def test_a_staged_package_is_refused_at_the_hosts_own_path(self):
        self.write([CP], host_contactpoint(snapshot=False))
        staged = stage_packages(Path(self.temp.name) / "staged", [self.package])
        with self.assertRaises(SystemExit) as refused:
            precheck(set(), staged, [self.package])
        self.assertIn(str(self.package / "contactpoint.json"), str(refused.exception))
        self.assertNotIn(str(staged[0]), str(refused.exception))
    def test_held_alone_reusing_a_shipped_canonical_is_refused(self):
        self.write([CP], host_contactpoint())
        with self.assertRaisesRegex(SystemExit, "redefines shipped canonicals"):
            precheck({CP}, [self.package])
        precheck(set(), [self.package])

if __name__ == "__main__":
    unittest.main()
