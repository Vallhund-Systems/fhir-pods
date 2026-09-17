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
        ("Patient.active", [("boolean", "true")], False, "pass"),
        ("Patient.gender", [("code", "unknown")], False, "pass"),
        ("'unknown'", [("code", "unknown")], False, "fail"),
        ("Patient.id", [("id", "synthetic")], False, "pass"),
        ("'synthetic'", [("id", "synthetic")], False, "fail"),
        ("Patient.birthDate", [("date", "@2000-01-01")], False, "pass"),
        ("Patient.birthDate", [("date", "@2001-01-01")], False, "fail"),
        ("'2000-01-01'", [("date", "@2000-01-01")], False, "fail"),
        ("Patient.deceasedBoolean", [("boolean", "true")], False, "fail"),
        ("1", [("decimal", "1")], False, "unsupported"),
        ("1.syntheticUnknown()", [("integer", "1")], False, "unsupported"),
    ]
    fixture = {"resourceType": "Patient", "id": "synthetic", "birthDate": "2000-01-01", "active": True, "gender": "unknown",
               "name": [{"given": ["Same", "Other", "Same"]}],
               "_deceasedBoolean": {"extension": [{"url": "http://hl7.org/fhir/StructureDefinition/data-absent-reason", "valueCode": "unknown"}]}}
    cases = [{"id": str(i), "expression": expr, "input": fixture,
              "expected": [{"type": ty, "value": value} for ty, value in expected],
              "ordered": True, "predicate": predicate}
             for i, (expr, expected, predicate, _) in enumerate(checks)]
    expected_statuses = [c[3] for c in checks]
    for values, status in [("Other Same Same", "pass"), ("Other Other Same", "fail"), ("Same Other", "fail")]:
        cases.append({"id": str(len(cases)), "expression": "Patient.name.given", "input": fixture,
                      "expected": [{"type": "string", "value": value} for value in values.split()],
                      "ordered": False, "predicate": False})
        expected_statuses.append(status)
    cases.append({"id": str(len(cases)), "expression": "Patient.name.given", "input": fixture,
                  "expected": [{"type": "string", "value": "Same"}, {"type": "code", "value": "Other"}],
                  "ordered": False, "predicate": False})
    expected_statuses.append("unsupported")
    result = subprocess.run([str(executable)], input="".join(json.dumps(c) + "\n" for c in cases), text=True, capture_output=True, check=True, timeout=15)
    outcomes = [json.loads(line) for line in result.stdout.splitlines()]
    if [(o["id"], o["status"]) for o in outcomes] != list(zip(map(str, range(len(cases))), expected_statuses)):
        raise SystemExit(f"Comparison adapter self-check failed: {outcomes}")
    # Equal status does not establish why a case is unsupported.
    if outcomes[20]["reason"] != "output-comparison-not-exposed: decimal" or outcomes[21]["reason"] != "engine-evaluation-unsupported":
        raise SystemExit("Comparison adapter cannot separate evaluation from comparison limits")
    return len(cases)


def category(case, outcome):
    """Report the stage that blocked comparison, without guessing error types."""
    if outcome["status"] != "unsupported":
        return "matched" if outcome["status"] == "pass" else "disagreement"
    reason = outcome["reason"]
    if reason.startswith("parser-rejected"):
        return "api-error-ambiguity" if case.get("invalid") else "engine-parse"
    if reason.startswith("execution-rejected"):
        return "api-error-ambiguity"
    if reason == "static-semantic-checking-not-exposed":
        return "api-static-semantics"
    if reason == "resource-free-context-variables-not-representable":
        return "api-context"
    if reason == "engine-evaluation-unsupported":
        return "engine-evaluation"
    return "adapter-comparison"


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
    adapter_checks = check_adapter(executable)
    result = subprocess.run([str(executable)], input=raw, capture_output=True, timeout=120, check=True)
    outcomes = [json.loads(line) for line in result.stdout.splitlines()]
    if [r["id"] for r in outcomes] != [c["id"] for c in cases]:
        raise SystemExit("Missing, reordered or unexpected runner results")
    counts = Counter(r["status"] for r in outcomes)
    groups = defaultdict(Counter)
    records = []
    for case, outcome in zip(cases, outcomes):
        if outcome.get("status") not in ("pass", "fail", "unsupported") or not isinstance(outcome.get("reason"), str):
            raise SystemExit("Malformed runner result")
        groups[case["group"]][outcome["status"]] += 1
        records.append({**{k: v for k, v in case.items() if k != "input"}, **outcome, "category": category(case, outcome)})
    report = {
        "recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "tracked_source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT, text=True)),
        "engine_sha256": hashlib.sha256((ROOT / "crates/fhir-pods-core/src/path.rs").read_bytes()).hexdigest(),
        "adapter_sha256": hashlib.sha256((ROOT / "crates/fhir-pods-core/examples/fhirpath_suite.rs").read_bytes()).hexdigest(),
        "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "binary_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
        "adapter_checks": adapter_checks,
        "upstream": json.loads((HERE / "sources.lock.json").read_text()), "preparation": preparation,
        "comparison": "Public invariant API assertions: cardinality, FHIR/System primitive type and value, order or unordered multiplicity. Adds exact code/id type and date precision/string comparison. Primitive carriers without values cannot pass through invariant emptiness. Predicate cases use explicit empty-to-false conversion. Unsupported stages separate original-expression parse/evaluation, adapter comparison and unavailable API semantics; exact error categories, decimal/dateTime/Quantity/complex output comparisons and static semantics are not claimed.",
        "counts": {"total": len(records), **counts},
        "unsupported_by_category": dict(Counter(r["category"] for r in records if r["status"] == "unsupported")),
        "groups": dict(groups), "cases": records,
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
