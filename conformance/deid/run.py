#!/usr/bin/env python3
"""Run independent synthetic marker checks alongside the core privacy checker."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)
    elif isinstance(value, dict):
        for key, item in value.items():
            yield key
            yield from strings(item)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=HERE / "evidence.json")
    args = parser.parse_args()
    manifest = json.loads((HERE / "manifest.json").read_text())
    ids = [case["id"] for case in manifest["cases"]]
    if not ids or len(ids) != len(set(ids)):
        raise SystemExit("Empty manifest or duplicate case identities")
    requests = []
    for case in manifest["cases"]:
        resource = json.loads((HERE / case["file"]).read_text())
        for marker in case["markers"]:
            if not any(marker in value for value in strings(resource)):
                raise SystemExit(f"Missing input witness {marker}: {case['id']}")
        requests.append({"id": case["id"], "resource": resource, "policy": case.get("policy", manifest["policy"])})
    subprocess.run(["cargo", "build", "-q", "-p", "fhir-pods-core", "--example", "deid_probes"], cwd=ROOT, check=True)
    executable = ROOT / "target/debug/examples/deid_probes"
    binary_sha256 = hashlib.sha256(executable.read_bytes()).hexdigest()
    request_text = "".join(json.dumps(item, ensure_ascii=False) + "\n" for item in requests)
    run = subprocess.run([str(executable)], input=request_text, capture_output=True, text=True, timeout=60, check=True)
    repeat = subprocess.run([str(executable)], input=request_text, capture_output=True, text=True, timeout=60, check=True)
    if repeat.stdout != run.stdout:
        raise SystemExit("Repeated execution with identical inputs and salt was not deterministic")
    if hashlib.sha256(executable.read_bytes()).hexdigest() != binary_sha256:
        raise SystemExit("Probe executable changed during the run")
    results = [json.loads(line) for line in run.stdout.splitlines()]
    if [r.get("id") for r in results] != [r["id"] for r in requests]:
        raise SystemExit("Missing, reordered or duplicate probe responses")
    groups = {}
    records = []
    for case, result in zip(manifest["cases"], results):
        failures = []
        leaks = []
        if "error" in result:
            failures.append("operation-rejected: " + result["error"])
        else:
            output = result["resource"]
            leaks = [marker for marker in case["markers"] if any(marker in value for value in strings(output))]
            if not result["inputValidation"]["valid"]:
                failures.append("invalid-input-fixture")
            if not result["validation"]["valid"]:
                failures.append("invalid-output")
            if result["remaining"] != result["checkAfter"]:
                failures.append("checker-metadata-disagreement")
            if case.get("retain_control"):
                if leaks != case["markers"] or not result["remaining"]:
                    failures.append("retention-control-undetected")
            else:
                if leaks:
                    failures.append("identifying-marker-survives")
                if result["remaining"]:
                    failures.append("core-checker-reports-identifiers")
            if case.get("contained_reference"):
                if output["subject"]["reference"] != "#" + output["contained"][0]["id"]:
                    failures.append("contained-reference-broken")
            if "age_value" in case:
                if output["onsetAge"]["value"] != case["age_value"]:
                    failures.append("incorrect-age-clamp")
                if any(a["action"] == "clamp" for a in result["applied"]) != case["clamp_recorded"]:
                    failures.append("missing-or-unexpected-clamp-record")
            if group := case.get("same_identifier_group"):
                value = output["identifier"][0]["value"]
                if groups.setdefault(group, value) != value:
                    failures.append("member-order-changes-identity")
        record = {**case, "sha256": hashlib.sha256((HERE / case["file"]).read_bytes()).hexdigest(),
                  "surviving_markers": leaks, "failures": failures, "pass": not failures, "result": result}
        records.append(record)
        print(f"{'FAIL' if failures else 'ok':4} {case['id']}: {', '.join(failures) or 'checks pass'}")
    failed = sum(not r["pass"] for r in records)
    report = {"recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True)),
              "binary_sha256": binary_sha256, "deterministic_repeat": True,
              "policy": manifest["policy"], "counts": {"total": len(records), "pass": len(records) - failed, "fail": failed}, "cases": records}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(report["counts"]))
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()
