#!/usr/bin/env python3
"""Catalogue every pinned R4 FHIRPath case. Failing cases do not disappear.

Default is a coverage report; --strict exits nonzero for failures OR unsupported
cases. Fetch explicitly first with fetch.py; this command never uses the network.
"""
import argparse
from collections import Counter, defaultdict
import datetime
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def check_adapter(executable):
    """Check that empty results, false, wrong types/order cannot pass by accident."""
    checks = [
        ("true", [("boolean", "true")], False, "pass"),
        ("false", [("boolean", "false")], False, "pass"),
        ("true", [("boolean", "false")], False, "fail"),
        ("'true'", [("boolean", "true")], False, "fail"),
        ("Patient.missing", [], False, "pass"),
        ("Patient.missing", [("boolean", "true")], False, "fail"),
        ("1 | 2", [("integer", "1"), ("integer", "2")], False, "pass"),
        ("1 | 2", [("integer", "2"), ("integer", "1")], False, "fail"),
        ("Patient.missing", [("boolean", "false")], True, "pass"),
        ("Patient.missing", [("boolean", "true")], True, "fail"),
        ("Patient.birthDate", [("boolean", "true")], True, "pass"),
    ]
    cases = [{"id": str(i), "expression": expr, "input": {"resourceType": "Patient", "birthDate": "2000-01-01"},
              "expected": [{"type": ty, "value": value} for ty, value in expected],
              "ordered": True, "predicate": predicate}
             for i, (expr, expected, predicate, _) in enumerate(checks)]
    result = subprocess.run([str(executable)], input="".join(json.dumps(c) + "\n" for c in cases), text=True, capture_output=True, check=True, timeout=15)
    outcomes = [json.loads(line) for line in result.stdout.splitlines()]
    if [(o["id"], o["status"]) for o in outcomes] != [(str(i), c[3]) for i, c in enumerate(checks)]:
        raise SystemExit(f"Comparison adapter self-check failed: {outcomes}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--strict", action="store_true")
    parser.add_argument("--output", type=Path, default=HERE / "catalogue.json")
    args = parser.parse_args()
    raw = (HERE / "cases.jsonl").read_bytes()
    cases = [json.loads(line) for line in raw.splitlines()]
    preparation = json.loads((HERE / ".cache/preparation.json").read_text())
    if hashlib.sha256(raw).hexdigest() != preparation["cases_sha256"]:
        raise SystemExit("Prepared cases changed; rerun fetch.py")
    subprocess.run(["cargo", "build", "-q", "-p", "fhir-pods-core", "--example", "fhirpath_suite"], cwd=ROOT, check=True)
    executable = ROOT / "target/debug/examples/fhirpath_suite"
    check_adapter(executable)
    result = subprocess.run([str(executable)], input=raw, capture_output=True, timeout=120, check=True)
    outcomes = [json.loads(line) for line in result.stdout.splitlines()]
    if [r["id"] for r in outcomes] != [c["id"] for c in cases]:
        raise SystemExit("Missing, reordered or unexpected runner results")
    counts = Counter(r["status"] for r in outcomes)
    groups = defaultdict(Counter)
    records = []
    for case, outcome in zip(cases, outcomes):
        groups[case["group"]][outcome["status"]] += 1
        records.append({**{k: v for k, v in case.items() if k != "input"}, **outcome})
    report = {
        "recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "tracked_source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT, text=True)),
        "engine_sha256": hashlib.sha256((ROOT / "crates/fhir-pods-core/src/path.rs").read_bytes()).hexdigest(),
        "upstream": json.loads((HERE / "sources.lock.json").read_text()), "preparation": preparation,
        "comparison": "Public invariant API assertions: ordered cardinality, primitive system type and value. Predicate cases use boolean conversion with an explicit empty-to-false guard. Unsupported includes engine omissions AND adapter/API limits; exact error categories, decimal/date/Quantity outputs, FHIR primitive subtypes, unordered output and static semantics are not claimed.",
        "counts": {"total": len(records), **counts}, "groups": dict(groups), "cases": records,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["counts"]))
    for reason, count in Counter(r["reason"] for r in outcomes if r["status"] != "pass").most_common():
        print(f"{count:4} {reason}")
    if args.strict and counts["pass"] != len(records):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
