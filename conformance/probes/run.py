#!/usr/bin/env python3
"""Recheck the synthetic probes against the pinned reference validator and core.

No corpus regeneration or shared oracle output: each invocation uses a temporary
directory. Run from any directory; JAVA / JAVA_HOME select the installed JDK.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import sys
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE.parent))
import canonical_guard  # noqa: E402
VERSION = "6.10.4"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=HERE / "evidence.json")
    parser.add_argument("--oracle-only", action="store_true")
    args = parser.parse_args()
    manifest = json.loads((HERE / "manifest.json").read_text())
    # the oracle caches a url-bearing ValueSet by its url: never two definitions under one
    canonical_guard.refuse_shared_canonicals([HERE / item["file"] for item in manifest])
    jar = ROOT / "conformance/validator_cli.jar"
    with zipfile.ZipFile(jar) as archive:
        metadata = archive.read("META-INF/maven/ca.uhn.hapi.fhir/org.hl7.fhir.validation.cli/pom.properties").decode()
    if f"version={VERSION}" not in metadata.splitlines():
        raise SystemExit(f"Expected reference validator {VERSION}; install the pinned JAR first")
    java = os.environ.get("JAVA") or (str(Path(os.environ["JAVA_HOME"]) / "bin/java") if os.environ.get("JAVA_HOME") else "java")
    if not args.oracle_only:
        subprocess.run(["cargo", "build", "-q", "-p", "fhir-pods-core", "--example", "probe"], cwd=ROOT, check=True)
    options = ["-version", "4.0", "-ig", "hl7.fhir.au.core#2.0.0", "-tx", "n/a", "-allow-example-urls", "true"]
    with tempfile.TemporaryDirectory(prefix="fhir-pods-probes-") as temporary:
        output = Path(temporary) / "outcomes.json"
        command = [java, "-jar", str(jar), *[str(HERE / item["file"]) for item in manifest], *options, "-output", str(output)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=300)
        if result.returncode != 0:
            raise SystemExit(f"Oracle execution failed (exit {result.returncode})\n{result.stdout[-3000:]}\n{result.stderr[-3000:]}")
        if not output.exists():
            raise SystemExit(f"Oracle produced no fresh output (exit {result.returncode})\n{result.stdout[-3000:]}\n{result.stderr[-3000:]}")
        bundle = json.loads(output.read_text())
    if bundle.get("resourceType") != "Bundle":
        raise SystemExit("Expected a Bundle of OperationOutcomes")
    outcomes = {}
    for entry in bundle.get("entry", []):
        outcome = entry.get("resource", {})
        if outcome.get("resourceType") != "OperationOutcome" or not outcome.get("issue"):
            raise SystemExit("Malformed reference-validator outcome")
        source = next((e.get("valueString") for e in outcome.get("extension", []) if e.get("url", "").endswith("operationoutcome-file")), None)
        if not source or Path(source).name in outcomes:
            raise SystemExit("Missing or duplicate oracle source identity")
        outcomes[Path(source).name] = outcome
    if set(outcomes) != {item["file"] for item in manifest} or len(manifest) != len(outcomes):
        raise SystemExit("Missing, duplicate or unexpected oracle results")
    records = []
    failures = 0
    for item in manifest:
        path = HERE / item["file"]
        if item["file"] not in outcomes:
            raise SystemExit(f"Missing oracle result: {item['file']}")
        outcome = outcomes[item["file"]]
        issues = [{"severity": i["severity"], "code": i.get("code"), "text": i.get("details", {}).get("text") or i.get("diagnostics", ""), "paths": i.get("expression") or i.get("location", [])} for i in outcome["issue"]]
        oracle_valid = not any(i["severity"] in ("fatal", "error") for i in issues)
        record = {**item, "sha256": digest(path), "oracle_valid": oracle_valid, "oracle_issues": issues}
        agrees = oracle_valid == item["expect"]
        if not args.oracle_only:
            probe = subprocess.run([str(ROOT / "target/debug/examples/probe"), str(path)], capture_output=True, text=True, timeout=15)
            local = json.loads(probe.stdout)
            if probe.returncode not in (0, 1) or not isinstance(local.get("valid"), bool):
                raise SystemExit(f"Probe execution failure: {item['file']}")
            record["local"] = local
            agrees = agrees and local["valid"] == oracle_valid
        failures += not agrees
        record["agrees"] = agrees
        records.append(record)
        print(f"{'ok' if agrees else 'DIFF':4} {item['file']}: oracle={oracle_valid} expected={item['expect']}" + (f" local={record['local']['valid']}" if "local" in record else ""))
    report = {
        "recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT, text=True)),
        "oracle_version": VERSION, "oracle_sha256": digest(jar), "oracle_options": options,
        "oracle_exit_code": result.returncode,
        "note": "Terminology service disabled; the oracle may still resolve unknown profile URLs. Verdict parity only; issues retained for inspection.",
        "counts": {"total": len(records), "agree": len(records) - failures, "differ": failures},
        "probes": records,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["counts"]))
    raise SystemExit(1 if failures else 0)


if __name__ == "__main__":
    main()
