#!/usr/bin/env python3
"""Recheck the coded-answer probes against the pinned reference validator and core.

A coded answer (and a Coding outside a questionnaire) with one part missing —
no system, no code, a display alone — under each way a Questionnaire names
its options: a ValueSet contained in the Questionnaire (loaded by canonical,
since a contained Questionnaire may not itself contain one), a ValueSet
contained beside the Questionnaire in the response, a ValueSet by canonical,
and inline answerOptions. The lane exists because a rule derived from one
shape (a display with no code) once reached another (a code with no system)
the reference validator only warns about: probe, never reason.

`defs/` is given to the oracle as an IG folder and to the pod the way a host
gives it: the Questionnaires loaded, the ValueSet compiled into a terminology
table (`tools/compile_valueset.py`). One JVM, one temporary directory.
Run from any directory; JAVA / JAVA_HOME select the installed JDK.

Gated: the verdict three ways (manifest, oracle, pod) and the error-level
paths both ways. A probe the pod is known to judge differently carries a
`reviewed` entry in `manifest.json` — the pod's verdict, the error paths it
misses and adds, and why — and must differ in exactly that way (a fix that
removes the difference fails until the entry goes). Every issue on both
sides is kept in `evidence.json`.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFS = HERE / "defs"
VERSION = "6.10.4"
sys.path.insert(0, str(HERE.parent))
import reasons  # noqa: E402
import run as gate  # noqa: E402


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def issues_of(outcome):
    return [
        {
            "severity": i["severity"],
            "code": i.get("code"),
            "rule": next((c.get("code") for c in (i.get("details") or {}).get("coding", []) if c.get("system") == "urn:fhir-pods:rule"), None),
            "text": (i.get("details") or {}).get("text") or i.get("diagnostics", ""),
            "paths": i.get("expression") or i.get("location", []),
        }
        for i in outcome.get("issue", [])
    ]


def errors(outcome):
    return sorted({reasons.normalise(p) for p in gate.error_paths(outcome)})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=HERE / "evidence.json")
    parser.add_argument("--oracle-only", action="store_true")
    parser.add_argument("--tx", default="n/a", help="the oracle's -tx (default n/a; the pod is always offline)")
    parser.add_argument("--write-expectations", action="store_true", help="fill manifest entries with no expectation from the oracle's verdict")
    args = parser.parse_args()
    manifest = json.loads((HERE / "manifest.json").read_text())
    files = [item["file"] for item in manifest]
    if len(set(files)) != len(files):
        raise SystemExit("Duplicate manifest entry")
    on_disk = {p.name for p in HERE.glob("*.json")} - {"manifest.json", "evidence.json"}
    if on_disk != set(files):
        raise SystemExit(f"Manifest and fixtures disagree: {sorted(on_disk ^ set(files))}")
    jar = ROOT / "conformance/validator_cli.jar"
    with zipfile.ZipFile(jar) as archive:
        metadata = archive.read("META-INF/maven/ca.uhn.hapi.fhir/org.hl7.fhir.validation.cli/pom.properties").decode()
    if f"version={VERSION}" not in metadata.splitlines():
        raise SystemExit(f"Expected reference validator {VERSION}; install the pinned JAR first")
    java = os.environ.get("JAVA") or (str(Path(os.environ["JAVA_HOME"]) / "bin/java") if os.environ.get("JAVA_HOME") else "java")
    if not args.oracle_only:
        subprocess.run(["cargo", "build", "-q", "-p", "fhir-pods-core", "--example", "probe"], cwd=ROOT, check=True)
    options = ["-version", "4.0", "-ig", "defs", "-tx", args.tx, "-allow-example-urls", "true"]
    questionnaires = sorted(DEFS.glob("Questionnaire-*.json"))
    with tempfile.TemporaryDirectory(prefix="fhir-pods-coded-answers-") as temporary:
        table = Path(temporary) / "answers.terminology.txt"
        compiled = subprocess.run([sys.executable, str(ROOT / "tools/compile_valueset.py"), *map(str, sorted(DEFS.glob("[CV]*.json")))],
                                  capture_output=True, text=True)
        if compiled.returncode != 0:
            raise SystemExit(f"compile_valueset.py refused the definitions: {compiled.stderr.strip()}")
        table.write_text(compiled.stdout)
        output = Path(temporary) / "outcomes.json"
        command = [java, "-jar", str(jar), *[str(HERE / f) for f in files],
                   *[str(DEFS) if a == "defs" else a for a in options], "-output", str(output)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=900)
        if result.returncode not in (0, 1) or not output.exists():
            raise SystemExit(f"Oracle execution failed (exit {result.returncode})\n{result.stdout[-3000:]}\n{result.stderr[-3000:]}")
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
        if set(outcomes) != set(files):
            raise SystemExit("Missing, duplicate or unexpected oracle results")
        records = []
        failures = 0
        for item in manifest:
            path = HERE / item["file"]
            outcome = outcomes[item["file"]]
            oracle_issues = issues_of(outcome)
            oracle_valid = not any(i["severity"] in ("fatal", "error") for i in oracle_issues)
            if args.write_expectations and item.get("expect") is None:
                item["expect"] = oracle_valid
            record = {**item, "sha256": digest(path), "oracle_valid": oracle_valid, "oracle_errors": errors(outcome), "oracle_issues": oracle_issues}
            agrees = oracle_valid == item["expect"]
            if not args.oracle_only:
                command = [str(ROOT / "target/debug/examples/probe"), "--outcome", "--terminology", str(table)]
                for q in questionnaires:
                    command += ["--questionnaire", str(q)]
                probe = subprocess.run([*command, str(path)], capture_output=True, text=True, timeout=15)
                local = json.loads(probe.stdout)
                if probe.returncode not in (0, 1) or local.get("resourceType") != "OperationOutcome":
                    raise SystemExit(f"Probe execution failure: {item['file']}")
                local_issues = issues_of(local)
                local_valid = not any(i["severity"] in ("fatal", "error") for i in local_issues)
                record.update(local_valid=local_valid, local_errors=errors(local), local_issues=local_issues)
                missed = sorted(set(record["oracle_errors"]) - set(record["local_errors"]))
                extra = sorted(set(record["local_errors"]) - set(record["oracle_errors"]))
                differs = local_valid != oracle_valid or missed or extra
                reviewed = item.get("reviewed")
                if reviewed:
                    # a reviewed difference must be exactly the one measured: a
                    # fix that removes it, or a new one beside it, fails until
                    # the entry is reviewed again
                    matches = (reviewed.get("local") == local_valid and sorted(reviewed.get("missed", [])) == missed
                               and sorted(reviewed.get("extra", [])) == extra)
                    record["review"] = "reviewed" if differs and matches else "stale-review"
                    agrees = agrees and differs and matches
                else:
                    record["review"] = "difference" if differs else "agreement"
                    agrees = agrees and not differs
            failures += not agrees
            record["agrees"] = agrees
            records.append(record)
            line = f"{'ok' if agrees else 'DIFF':4} {item['file']}: oracle={oracle_valid} expected={item['expect']}"
            if "local_valid" in record:
                line += f" local={record['local_valid']}" + (f" [{record['review']}]" if record["review"] != "agreement" else "")
                if record["local_errors"] != record["oracle_errors"]:
                    line += f"\n       oracle errors {record['oracle_errors']}\n       local errors  {record['local_errors']}"
            print(line)
    if args.write_expectations:
        (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    report = {
        "recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT, text=True)),
        "oracle_version": VERSION, "oracle_sha256": digest(jar), "oracle_options": options,
        "oracle_exit_code": result.returncode,
        "note": "Verdict and error-path parity; every issue on both sides retained (severity, code, the pod's rule id, text, paths).",
        "counts": {"total": len(records), "pass": len(records) - failures, "fail": failures,
                   "reviewed": sum(1 for r in records if r.get("review") == "reviewed")},
        "probes": records,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["counts"]))
    raise SystemExit(1 if failures else 0)


if __name__ == "__main__":
    main()
