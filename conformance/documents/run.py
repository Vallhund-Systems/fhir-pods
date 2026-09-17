#!/usr/bin/env python3
"""Assemble both summary kinds, inspect their graphs, and run the pinned oracle.

Fails on failed expectations, oracle disagreement or incomplete evidence.
Negative-input rejection is recorded separately from a validation verdict.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
VERSION = "6.10.4"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def references(value):
    if isinstance(value, list):
        for item in value:
            yield from references(item)
    elif isinstance(value, dict):
        for key, item in value.items():
            if key == "reference" and isinstance(item, str):
                yield item
            else:
                yield from references(item)


def outcomes_from(bundle, expected):
    if bundle.get("resourceType") != "Bundle":
        raise ValueError("Expected a Bundle of OperationOutcomes")
    outcomes = {}
    for entry in bundle.get("entry", []):
        outcome = entry.get("resource", {})
        issues = outcome.get("issue")
        if outcome.get("resourceType") != "OperationOutcome" or not isinstance(issues, list) or not issues:
            raise ValueError("Malformed reference-validator outcome")
        if any(i.get("severity") not in ("information", "warning", "error", "fatal") for i in issues):
            raise ValueError("Missing or unknown issue severity")
        source = [e.get("valueString") for e in outcome.get("extension", []) if e.get("url", "").endswith("operationoutcome-file")]
        if len(source) != 1 or not isinstance(source[0], str) or Path(source[0]).name in outcomes:
            raise ValueError("Missing or duplicate oracle source identity")
        outcomes[Path(source[0]).name] = outcome
    if set(outcomes) != set(expected):
        raise ValueError("Missing or unexpected oracle results")
    return outcomes


def check_graph(case, result):
    errors = []
    document = result["document"]
    entries = document.get("entry", [])
    urls = [entry.get("fullUrl") for entry in entries]
    if not urls or any(not isinstance(url, str) for url in urls) or len(urls) != len(set(urls)):
        return ["missing-or-duplicate-entry-identity"]
    if entries[0]["resource"]["resourceType"] != "Composition":
        errors.append("composition-not-first")
    composition = entries[0]["resource"]
    if "preserve_reference" in case and case["preserve_reference"] not in set(references(document)):
        errors.append("absolute-reference-rebound")
    unresolved = [ref for ref in references(document) if ref not in urls and not ref.startswith("#")]
    allowed = {case.get("unresolved"), case.get("preserve_reference")}
    if any(ref not in allowed for ref in unresolved):
        errors.append("unexpected-unresolved-reference")
    if "unresolved" in case and case["unresolved"] not in unresolved:
        errors.append("missing-unresolved-reference-witness")
    if sorted(result["omitted"]) != sorted(case.get("omitted", [])):
        errors.append("incorrect-omitted-report")
    for entry in entries:
        r = entry["resource"]
        if f"{r['resourceType']}/{r.get('id', '')}" in case.get("omitted", []):
            errors.append("omitted-resource-still-in-document")
    if "section" in case:
        targets = {e["fullUrl"] for e in entries if e["resource"]["resourceType"] == case["placed_type"]}
        placed = {p["fullUrl"] for p in result["placed"] if p["section"] == case["section"]}
        section_targets = {ref for s in composition.get("section", []) if any(c.get("code") == case["section"] for c in s.get("code", {}).get("coding", [])) for ref in references(s.get("entry", []))}
        if not targets or not targets <= placed or not targets <= section_targets:
            errors.append("wrong-or-missing-section-placement")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=HERE / "evidence.json")
    args = parser.parse_args()
    manifest = json.loads((HERE / "manifest.json").read_text())
    ids = [case["id"] for case in manifest["cases"]]
    if not ids or len(ids) != len(set(ids)) or len(manifest["kinds"]) != len(set(manifest["kinds"])):
        raise SystemExit("Empty manifest or duplicate case/kind identities")
    jar = ROOT / "conformance/validator_cli.jar"
    with zipfile.ZipFile(jar) as archive:
        metadata = archive.read("META-INF/maven/ca.uhn.hapi.fhir/org.hl7.fhir.validation.cli/pom.properties").decode()
    if f"version={VERSION}" not in metadata.splitlines():
        raise SystemExit(f"Expected reference validator {VERSION}")
    java = os.environ.get("JAVA") or (str(Path(os.environ["JAVA_HOME"]) / "bin/java") if os.environ.get("JAVA_HOME") else "java")
    subprocess.run(["cargo", "build", "-q", "-p", "fhir-pods-core", "--example", "document_probes"], cwd=ROOT, check=True)
    executable = ROOT / "target/debug/examples/document_probes"
    binary_sha256 = digest(executable)
    diagnostic_table = HERE / "diagnostic-terminology.txt"
    requests = [{"id": f"{case['id']}-{kind}", "kind": kind, "inputs": json.loads((HERE / case["file"]).read_text()), "terminology": diagnostic_table.read_text()}
                for kind in manifest["kinds"] for case in manifest["cases"]]
    text = "".join(json.dumps(request) + "\n" for request in requests)
    run = subprocess.run([str(executable)], input=text, capture_output=True, text=True, timeout=60, check=True)
    repeat = subprocess.run([str(executable)], input=text, capture_output=True, text=True, timeout=60, check=True)
    if repeat.stdout != run.stdout:
        raise SystemExit("Identical assembly inputs produced different results")
    if digest(executable) != binary_sha256:
        raise SystemExit("Probe executable changed during the run")
    results = [json.loads(line) for line in run.stdout.splitlines()]
    if [r.get("id") for r in results] != [r["id"] for r in requests]:
        raise SystemExit("Missing, reordered or duplicate native results")
    options = ["-version", "4.0", "-ig", "hl7.fhir.uv.ips#2.0.1", "-ig", "hl7.fhir.au.ps#1.0.0",
               "-ig", "hl7.fhir.au.core#2.0.0", "-tx", "n/a", "-allow-example-urls", "true"]
    with tempfile.TemporaryDirectory(prefix="fhir-pods-documents-") as temporary:
        directory = Path(temporary)
        files = []
        for result in results:
            if "document" in result:
                path = directory / (result["id"] + ".json")
                path.write_text(json.dumps(result["document"]))
                files.append(path)
        output = directory / "outcomes.json"
        oracle = subprocess.run([java, "-jar", str(jar), *map(str, files), *options, "-output", str(output)], capture_output=True, text=True, timeout=600)
        if oracle.returncode != 0 or not output.is_file():
            raise SystemExit(f"Oracle execution failure ({oracle.returncode})\n{oracle.stdout[-4000:]}\n{oracle.stderr[-4000:]}")
        outcomes = outcomes_from(json.loads(output.read_text()), [p.name for p in files])
    records = []
    by_id = {result["id"]: result for result in results}
    for request, result in zip(requests, results):
        case = next(c for c in manifest["cases"] if request["id"] == f"{c['id']}-{request['kind']}")
        failures = []
        record = {"id": request["id"], "kind": request["kind"], "input": case,
                  "input_sha256": digest(HERE / case["file"]), "local": result}
        if "expect_error" in case:
            if result.get("error") != case["expect_error"]:
                failures.append("incorrect-rejection")
        elif "error" in result:
            failures.append("unexpected-operation-rejection")
        else:
            failures += check_graph(case, result)
            if not isinstance(result.get("validation", {}).get("valid"), bool):
                raise SystemExit("Missing native boolean validation verdict")
            outcome = outcomes[request["id"] + ".json"]
            issues = [{"severity": i["severity"], "code": i.get("code"), "text": i.get("details", {}).get("text") or i.get("diagnostics", ""),
                       "paths": i.get("expression") or i.get("location", [])} for i in outcome["issue"]]
            oracle_valid = not any(i["severity"] in ("fatal", "error") for i in issues)
            record.update(oracle_valid=oracle_valid, oracle_issues=issues)
            local_valid = result["validation"]["valid"]
            if result["validationWithTerminology"]["valid"] != case["expect_valid"]:
                failures.append("diagnostic-terminology-expectation")
            if local_valid != case["expect_valid"]:
                failures.append("local-expectation")
            if oracle_valid != case["expect_valid"]:
                failures.append("oracle-expectation")
            if local_valid != oracle_valid:
                failures.append("oracle-disagreement")
            if prior := case.get("identity_differs_from"):
                other = by_id[prior + "-" + request["kind"]]["document"]
                current = result["document"]
                if current["identifier"] == other["identifier"] or current["entry"][0]["fullUrl"] == other["entry"][0]["fullUrl"]:
                    failures.append("content-identity-collision")
        record.update(failures=failures, passed=not failures)
        records.append(record)
        print(f"{'FAIL' if failures else 'ok':4} {request['id']}: {', '.join(failures) or 'checks pass'}")
    failed = sum(not r["passed"] for r in records)
    report = {"recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True)),
              "binary_sha256": binary_sha256, "deterministic_repeat": True,
              "diagnostic_terminology_sha256": digest(diagnostic_table),
              "oracle_version": VERSION, "oracle_sha256": digest(jar), "oracle_options": options, "oracle_exit_code": oracle.returncode,
              "note": "Terminology service disabled. Native rejections have no assembled artifact to submit to the oracle. Full documents and issues retained; privacy fixtures are synthetic.",
              "counts": {"total": len(records), "pass": len(records) - failed, "fail": failed, "oracle_compared": len(outcomes),
                         "oracle_agree": sum(r["oracle_valid"] == r["local"]["validation"]["valid"] for r in records if "oracle_valid" in r),
                         "diagnostic_expectation_pass": sum(r["local"]["validationWithTerminology"]["valid"] == r["input"]["expect_valid"] for r in records if "document" in r["local"])}, "cases": records}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["counts"]))
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()
