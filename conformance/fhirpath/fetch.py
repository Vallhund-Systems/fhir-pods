#!/usr/bin/env python3
"""Fetch a pinned official R4 FHIRPath suite; prepare JSONL for the Rust runner.

Explicit developer command only: never called by cargo/build.rs. XML fixtures
are converted using the pinned R4 4.0.1 StructureDefinitions already installed
by conformance/make_corpus.py, preserving array cardinality and primitive types.
"""
import concurrent.futures
import hashlib
import json
from pathlib import Path
import urllib.request
import xml.etree.ElementTree as ET

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
REVISION = "87fe5070c0d7c13d84acecfab9c5cc1b4ec2ba47"
BASE = f"https://raw.githubusercontent.com/FHIR/fhir-test-cases/{REVISION}/"
CACHE = HERE / ".cache"


def fetch(path):
    target = CACHE / path
    if not target.exists():
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(urllib.request.urlopen(BASE + path, timeout=60).read())
    return target.read_bytes()


def sha(data):
    return hashlib.sha256(data).hexdigest()


class FhirXml:
    """A fixture adapter, not a general FHIR XML validator. Unknown shapes fail."""
    def __init__(self):
        self.elements = {}
        self.primitive = set()
        package = ROOT / "conformance/package-r4/package"
        info = json.loads((package / "package.json").read_text())
        if info["version"] != "4.0.1":
            raise ValueError("Expected hl7.fhir.r4.core 4.0.1")
        self.package_hash = hashlib.sha256()
        for path in sorted(package.glob("StructureDefinition-*.json")):
            raw = path.read_bytes()
            sd = json.loads(raw)
            if sd.get("derivation") == "constraint":
                continue
            self.package_hash.update(raw)
            if sd.get("kind") == "primitive-type":
                self.primitive.add(sd["type"])
            for e in sd.get("snapshot", {}).get("element", []):
                self.elements[e["path"]] = e

    @staticmethod
    def tag(node):
        return node.tag.rsplit("}", 1)[-1]

    def element(self, context, key):
        direct = self.elements.get(context + "." + key)
        if direct:
            return direct, (direct.get("type") or [{}])[0].get("code")
        for path, definition in self.elements.items():
            if not path.startswith(context + ".") or not path.endswith("[x]"):
                continue
            stem = path[len(context) + 1:-3]
            if "." in stem:
                continue
            for t in definition.get("type", []):
                ty = t["code"]
                if key == stem + ty[0].upper() + ty[1:]:
                    return definition, ty
        raise ValueError(f"No R4 element metadata for {context}.{key}")

    def object(self, node, context, resource=False):
        out = {"resourceType": self.tag(node)} if resource else {}
        if "id" in node.attrib:
            out["id"] = node.attrib["id"]
        if "url" in node.attrib:
            out["url"] = node.attrib["url"]
        groups = {}
        for child in node:
            groups.setdefault(self.tag(child), []).append(child)
        for key, children in groups.items():
            definition, ty = self.element(context, key)
            many = definition.get("max") != "1"
            if not many and len(children) != 1:
                raise ValueError(f"Repeated singleton {context}.{key}")
            values, carriers = [], []
            for child in children:
                if ty == "xhtml":
                    value, carrier = ET.tostring(child, encoding="unicode"), None
                elif ty == "Resource":
                    if len(child) != 1:
                        raise ValueError("Expected one wrapped resource")
                    value, carrier = self.object(child[0], self.tag(child[0]), True), None
                elif ty in self.primitive:
                    raw = child.get("value")
                    value = raw
                    if raw is not None and ty == "boolean":
                        if raw not in ("true", "false"):
                            raise ValueError("Invalid fixture boolean")
                        value = raw == "true"
                    elif raw is not None and ty in ("integer", "unsignedInt", "positiveInt"):
                        value = int(raw)
                    elif raw is not None and ty == "decimal":
                        value = float(raw)
                    carrier = self.object(child, "Element") or None
                else:
                    ref = definition.get("contentReference", "")
                    child_context = ref[1:] if ref.startswith("#") else definition["path"] if ty in ("BackboneElement", "Element") else ty
                    if not child_context:
                        raise ValueError(f"Missing context for {context}.{key}")
                    value, carrier = self.object(child, child_context), None
                values.append(value)
                carriers.append(carrier)
            if any(value is not None for value in values):
                out[key] = values if many else values[0]
            if any(carrier is not None for carrier in carriers):
                out["_" + key] = carriers if many else carriers[0]
        return out


def main():
    suite_path = "r4/fhirpath/tests-fhir-r4.xml"
    suite = fetch(suite_path)
    tree = ET.fromstring(suite)
    tests = list(tree.iter("test"))
    paths = ["r4/" + name for name in sorted({t.get("inputfile") for t in tests if t.get("inputfile")})]
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        resources = dict(zip(paths, pool.map(fetch, paths)))
    assets = {suite_path: sha(suite), **{path: sha(raw) for path, raw in resources.items()}}
    lock = {"repository": "https://github.com/FHIR/fhir-test-cases", "revision": REVISION, "assets": assets}
    lock_path = HERE / "sources.lock.json"
    if lock_path.exists() and json.loads(lock_path.read_text()) != lock:
        raise SystemExit("Upstream/cache hashes do not match sources.lock.json")
    converter = FhirXml()
    inputs = {}
    for path, raw in resources.items():
        if path.endswith(".json"):
            inputs[path[3:]] = json.loads(raw)
        else:
            xml = ET.fromstring(raw)
            inputs[path[3:]] = converter.object(xml, converter.tag(xml), True)
    cases = []
    def visit(node, group):
        if node.tag == "group":
            group = "/".join(filter(None, [group, node.get("name", "unnamed")]))
        if node.tag == "test":
            expression = node.find("expression")
            if expression is None:
                raise ValueError("Test missing expression")
            cases.append({
                "id": f"{len(cases) + 1:04d}:{group}/{node.get('name')}", "name": node.get("name"), "group": group,
                "expression": expression.text or "", "invalid": expression.get("invalid", ""),
                "mode": node.get("mode", expression.get("mode", "")), "predicate": node.get("predicate") == "true",
                "ordered": node.get("ordered") != "false", "version": node.get("version"),
                "inputfile": node.get("inputfile"), "input": inputs.get(node.get("inputfile")),
                "expected": [{"type": o.get("type", ""), "value": o.text or ""} for o in node.findall("output")],
            })
        for child in node:
            visit(child, group)
    visit(tree, "")
    (HERE / "cases.jsonl").write_text("".join(json.dumps(case) + "\n" for case in cases))
    lock_path.write_text(json.dumps(lock, indent=2) + "\n")
    (CACHE / "preparation.json").write_text(json.dumps({"base_package": "hl7.fhir.r4.core#4.0.1", "base_definitions_sha256": converter.package_hash.hexdigest(), "cases_sha256": sha((HERE / "cases.jsonl").read_bytes()), "cases": len(cases)}, indent=2) + "\n")
    print(f"Prepared {len(cases)} official cases and {len(inputs)} input resources at {REVISION}")


if __name__ == "__main__":
    main()
