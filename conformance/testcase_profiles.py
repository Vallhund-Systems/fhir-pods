"""Prepare and compile one isolated ad-hoc profile context at a time.

Only --prepare-profiles starts Java. Stored snapshots are keyed by the pinned
suite, exact inputs, and validator JAR. Definitions with identical canonicals
in different tests never share a generated registry. No snapshots ship.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET

import testcases as lane

PREPARED = lane.EVIDENCE / "profiles-prepared.json"
GENERATED = lane.ROOT / "crates/fhir-pods-core/src/generated_testcases"


def definition(path):
    if path.suffix == ".json":
        return json.loads(path.read_text())
    root = ET.parse(path).getroot()
    def primitive(key):
        node = root.find("{http://hl7.org/fhir}" + key)
        return node.get("value") if node is not None else None
    return {"resourceType": root.tag.rsplit("}", 1)[-1], "url": primitive("url"), "type": primitive("type")}


def context(case):
    profile = case["profile"]
    config = {**case, **{k: v for k, v in profile.items() if k not in ("source", "java", "firely-sdk-current")}}
    why = lane.scope(config, profiles=True)
    if why:
        return None, why
    source = profile.get("source", "")
    if not source or "://" in source or not lane.suite_file(source).is_file():
        return None, {"reason": "profile source is not a local suite definition", "manifest_key": "profile.source", "manifest_value": source}
    names = sorted(set([source, *case.get("profiles", []), *case.get("supporting", []), *profile.get("supporting", [])]))
    files, canonicals = [], {}
    for name in names:
        path = lane.suite_file(name)
        if not path.is_file():
            raise ValueError(f"missing profile dependency: {name}")
        resource = definition(path)
        if resource.get("resourceType") not in ("StructureDefinition", "ValueSet", "CodeSystem"):
            return None, {"reason": "profile context requires external supporting instances", "manifest_key": "supporting", "manifest_value": name}
        hashed = lane.digest(path.read_bytes())
        url = resource.get("url")
        if url in canonicals and canonicals[url] != hashed:
            return None, {"reason": "conflicting definitions within one profile context", "manifest_key": "profile", "manifest_value": url}
        canonicals[url] = hashed
        files.append({"file": name, "sha256": hashed, "resource_type": resource["resourceType"], "url": url})
    return {"source": source, "files": files}, None


def prepare(cases):
    jar = Path(lane.run.JAR)
    if not jar.is_file():
        raise ValueError("profile preparation needs the pinned conformance/validator_cli.jar")
    java = os.environ.get("JAVA") or str(Path(os.environ.get("JAVA_HOME", "/opt/homebrew/opt/openjdk@17")) / "bin/java")
    banner = subprocess.run([java, "-jar", str(jar), "-help"], capture_output=True, text=True, check=True)
    if "Version 6.10.4" not in banner.stdout + banner.stderr:
        raise ValueError("snapshot preparation requires validator_cli 6.10.4")
    jar_hash = lane.digest(jar.read_bytes())
    prior = json.loads(PREPARED.read_text()) if PREPARED.exists() else {}
    cached = prior.get("contexts", {}) if prior.get("suite_commit") == lane.PIN and prior.get("jar_sha256") == jar_hash else {}
    result = {"suite_commit": lane.PIN, "jar_version": "6.10.4", "jar_sha256": jar_hash, "cases": {}, "contexts": cached}
    for case in cases:
        config, why = context(case)
        if why:
            result["cases"][case["name"]] = {"status": "excluded", **why}
            continue
        key = lane.json_hash(config)
        result["cases"][case["name"]] = {"status": "prepared", "context": key}
        old = cached.get(key)
        if old and all(Path(p).is_file() and lane.digest(Path(p).read_bytes()) == sha for p, sha in old.get("outputs", {}).items()):
            continue
        folder = lane.EVIDENCE / "profile-contexts" / key
        inputs = folder / "input"
        if inputs.exists():
            shutil.rmtree(inputs)
        inputs.mkdir(parents=True)
        sources, mapping = [], {}
        for i, file in enumerate(config["files"]):
            target = inputs / f"{i}-{Path(file['file']).name}"
            shutil.copyfile(lane.suite_file(file["file"]), target)
            mapping[file["file"]] = str(target)
            if file["resource_type"] == "StructureDefinition":
                sources.append(str(target))
        cmd = [java, "-Xmx6g", "-jar", str(jar), "snapshot", *sources, "-version", "4.0.1", "-tx", "n/a",
               "-ig", str(inputs), "-outputSuffix", "snapshot.json"]
        print(f"  snapshot: {case['name']} ({len(sources)} definitions)", flush=True)
        with (folder / "snapshot.log").open("w") as log:
            proc = subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, timeout=240)
        outputs = {str(Path(s + ".snapshot.json")): lane.digest(Path(s + ".snapshot.json").read_bytes())
                   for s in sources if Path(s + ".snapshot.json").is_file()}
        record = {"configuration": config, "inputs": mapping, "outputs": outputs, "command": cmd, "exit": proc.returncode,
                  "log_sha256": lane.digest((folder / "snapshot.log").read_bytes())}
        if proc.returncode != 0 or len(outputs) != len(sources):
            log_text = (folder / "snapshot.log").read_text()
            if "Exception generating snapshot" not in log_text:
                raise ValueError(f"snapshot process failed for {case['name']}; inspect {folder / 'snapshot.log'}")
            record["status"] = "snapshot-rejected"
            record["reason"] = "validator_cli 6.10.4 cannot generate this differential's snapshot"
        else:
            record["status"] = "ready"
        result["contexts"][key] = record
        PREPARED.write_text(json.dumps(result, indent=2) + "\n")
    PREPARED.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Prepared {len(result['contexts'])} contexts for {len(cases)} cases; definitions remain in ignored caches.")
    return 0


def compiler_gaps(sd, load):
    # Compare only semantics our compiler actually represents. Unsupported
    # constructs are tied to the manifest's explicit profile and element IDs.
    from gen_profile import build
    gaps = [{"reason": reason, "element": element} for reason, element in build(sd, load).deferred
            if reason not in ("mustSupport only", "mustSupport / non-required binding only")]
    snapshot = {e.get("id") or e["path"]: e for e in sd.get("snapshot", {}).get("element", [])}
    for element in sd.get("differential", {}).get("element", []):
        where = element.get("id") or element.get("path")
        for key in element:
            if key.startswith(("minValue", "maxValue")) or key in ("maxLength", "mustHaveValue", "valueAlternatives"):
                gaps.append({"reason": f"compiler does not represent {key}", "element": where})
            if key.startswith("fixed") and isinstance(element[key], (dict, list)):
                gaps.append({"reason": "exact object fixed-value equality (not a subset pattern)", "element": where})
        binding = element.get("binding", {})
        if binding.get("strength") == "required":
            kinds = {t.get("code") for t in snapshot.get(where, element).get("type", [])}
            if "Quantity" in kinds:
                gaps.append({"reason": "required Quantity unit binding", "element": where})
        if any("additional-binding" in x.get("url", "") or "binding.additional" in x.get("url", "")
               for x in binding.get("extension", [])):
            gaps.append({"reason": "additional terminology bindings", "element": where})
        for ty in element.get("type", []):
            if ty.get("profile") and ":" not in str(where):
                gaps.append({"reason": "non-sliced element datatype/resource profile enforcement", "element": where})
    return gaps


def compile_context(record):
    sys.path.insert(0, str(lane.ROOT / "tools"))
    import gen_profile
    definitions = {}
    snapshots = {}
    for name, input_file in record["inputs"].items():
        output = Path(input_file + ".snapshot.json")
        if output.is_file():
            sd = json.loads(output.read_text())
            if not sd.get("snapshot", {}).get("element"):
                raise ValueError(f"empty generated snapshot for {name}")
            definitions[sd["url"]] = sd
            snapshots[name] = sd
    source = record["configuration"]["source"]
    root = snapshots[source]
    r4 = lane.HERE / "package-r4/package"
    if not r4.is_dir():
        raise ValueError("missing pinned R4 package; run conformance/make_corpus.py")
    index = {}
    for path in r4.glob("StructureDefinition-*.json"):
        sd = json.loads(path.read_text())
        index[sd["url"]] = sd
    def load(url):
        return definitions.get(url) or index.get(url)
    gaps = []
    for sd in definitions.values():
        if sd.get("kind") != "resource" and sd.get("type") not in ("Identifier", "Extension"):
            gaps.append({"reason": "registry compiler does not select this datatype profile", "element": sd.get("url")})
        gaps.extend(compiler_gaps(sd, load))
    if gaps:
        return None, gaps
    # Each context has its own package; no other test's same-URL variant is visible.
    folder = Path(record["inputs"][source]).parent.parent
    package = folder / "package"
    package.mkdir(exist_ok=True)
    for i, sd in enumerate(definitions.values()):
        (package / f"StructureDefinition-{i}.json").write_text(json.dumps(sd) + "\n")
    if GENERATED.exists():
        shutil.rmtree(GENERATED)
    subprocess.run([sys.executable, str(lane.ROOT / "tools/gen_all.py"), str(GENERATED),
                    f"FHIR/fhir-test-cases {lane.PIN}; isolated conformance context", str(package), str(r4)],
                   check=True, stdout=subprocess.DEVNULL)
    mod = GENERATED / "mod.rs"
    mod.write_text(mod.read_text().replace("//!", "//"))
    # A duplicate canonical would let the shipped entry shadow this context.
    # Verify actual emitted profiles, including transitive dependencies.
    def urls(folder):
        found = set()
        for path in folder.glob("*.rs"):
            match = re.search(r'^\s+url: "([^"]+)"', path.read_text(), re.MULTILINE)
            if match:
                found.add(match[1])
        return found
    emitted = urls(GENERATED)
    overlap = emitted & urls(GENERATED.parent / "generated")
    if overlap or root["url"] not in emitted:
        raise ValueError(f"test registry is missing its root or shadows shipped canonicals: {sorted(overlap)}")
    return root["url"], []


def main(args, selected, triage):
    cases = [case for case in selected if isinstance(case.get("profile"), dict)]
    if not cases:
        raise ValueError("no matching ad-hoc profile cases")
    if args.prepare_profiles:
        return prepare(cases)
    if not PREPARED.is_file():
        raise ValueError("prepare profiles first: python3 conformance/testcases.py --prepare-profiles")
    prepared = json.loads(PREPARED.read_text())
    if prepared.get("suite_commit") != lane.PIN:
        raise ValueError("profile snapshots are from another suite revision")
    default_probe = lane.build()
    rows, skipped, errors = [], [], []
    for case in cases:
        config, why = context(case)
        if why:
            skipped.append({"name": case["name"], "file": case["file"], **why})
            continue
        key = lane.json_hash(config)
        record = prepared["contexts"].get(key)
        if not record:
            errors.append({"name": case["name"], "error": "missing prepared context; rerun --prepare-profiles"})
            continue
        if any(not Path(p).is_file() or lane.digest(Path(p).read_bytes()) != sha for p, sha in record["outputs"].items()):
            errors.append({"name": case["name"], "error": "snapshot content changed since preparation"})
            continue
        if record["status"] != "ready":
            skipped.append({"name": case["name"], "file": case["file"], "reason": record["reason"],
                            "manifest_key": "profile", "manifest_value": case["profile"]["source"], "context": key})
            continue
        try:
            url, gaps = compile_context(record)
            if gaps:
                skipped.append({"name": case["name"], "file": case["file"], "reason": "profile compiler gaps",
                                "manifest_key": "profile", "manifest_value": case["profile"]["source"], "gaps": gaps})
                continue
            probe = lane.build(features=True)
            instance = json.loads(lane.suite_file(case["file"]).read_text())
            profiles = instance.setdefault("meta", {}).setdefault("profile", [])
            if url not in profiles:
                profiles.append(url)
            path = lane.EVIDENCE / "profile-contexts" / key / "instances" / (lane.json_hash(case) + ".json")
            path.parent.mkdir(exist_ok=True)
            path.write_text(json.dumps(instance) + "\n")
            row = lane.compare(case, probe, profile=True, input_path=path)
            default_verdict, _, default_outcome = lane.native(default_probe, path)
            row.update(original_input_sha256=lane.digest(lane.suite_file(case["file"]).read_bytes()),
                       context=key, profile=url, probe_sha256=lane.digest(probe.read_bytes()),
                       default_ours=default_verdict, default_outcome=default_outcome,
                       default_probe_sha256=lane.digest(default_probe.read_bytes()))
            row["review"] = lane.review(row, triage.get("profiles", {}))
            rows.append(row)
            print(f"  profile: {case['name']}: {row['ours']}/{row['java']}", flush=True)
        except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
            errors.append({"name": case["name"], "error": str(exc)})
    manifest = json.loads(lane.suite_file("manifest.json").read_text())["test-cases"]
    r4 = [c for c in manifest if str(c.get("version", "")).split(".")[:2] == ["4", "0"]]
    return lane.finish("profiles", args, manifest, r4, cases, rows, skipped, errors)
