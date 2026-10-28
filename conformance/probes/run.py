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
# The guide the oracle loads for a probe that names none; a probe of another
# guide's profile carries its own `ig` (NZ Base's NzPatient, CA Baseline's).
DEFAULT_IG = "hl7.fhir.au.core#2.0.0"
OPTIONS = ["-version", "4.0", "-tx", "n/a", "-allow-example-urls", "true"]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def oracle_signature(issues):
    """What a reviewed difference is bound to on the oracle's side: its
    verdict and every error it reported (severity, code, text, location),
    canonically ordered — a changed oracle answer voids the review."""
    errors = sorted(
        [i["severity"], i.get("code") or "", i["text"], sorted(i["paths"])]
        for i in issues if i["severity"] in ("fatal", "error")
    )
    canonical = json.dumps({"valid": not errors, "errors": errors}, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode()).hexdigest()


def judge(item, sha256, oracle_valid, signature, local_valid):
    """`(status, note)` for one probe: `agree`, `reviewed` (a recorded
    difference whose input hash, oracle signature and local verdict all
    still match) or `differ`. A review that no longer matches what it
    reviewed, or that reviews an agreement, is a failure too: a difference
    is accepted for the exact input and oracle answer it was reviewed on."""
    if oracle_valid != item["expect"]:
        return "differ", "oracle verdict is not the expected one"
    reviewed = item.get("reviewed")
    if local_valid is None or local_valid == oracle_valid:
        return ("differ", "a reviewed difference that no longer differs") if reviewed and local_valid is not None else ("agree", None)
    if not reviewed:
        return "differ", "local and oracle verdicts differ"
    stale = [k for k, have in (("sha256", sha256), ("oracle", signature), ("local_valid", local_valid)) if reviewed.get(k) != have]
    if stale or not reviewed.get("reason"):
        return "differ", "stale review: " + ", ".join(stale or ["reason"])
    return "reviewed", reviewed["reason"]


def run_oracle(java, jar, ig, paths):
    """One JVM per guide: every probe of `ig` in one invocation, a fresh
    output file, each outcome keyed by its file name."""
    options = [*OPTIONS[:2], "-ig", ig, *OPTIONS[2:]]
    with tempfile.TemporaryDirectory(prefix="fhir-pods-probes-") as temporary:
        output = Path(temporary) / "outcomes.json"
        command = [java, "-jar", str(jar), *[str(p) for p in paths], *options, "-output", str(output)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=300)
        if result.returncode != 0:
            raise SystemExit(f"Oracle execution failed (exit {result.returncode})\n{result.stdout[-3000:]}\n{result.stderr[-3000:]}")
        if not output.exists():
            raise SystemExit(f"Oracle produced no fresh output (exit {result.returncode})\n{result.stdout[-3000:]}\n{result.stderr[-3000:]}")
        bundle = json.loads(output.read_text())
    if len(paths) == 1 and bundle.get("resourceType") == "OperationOutcome":
        bundle = {"resourceType": "Bundle", "entry": [{"resource": bundle}]}
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
    if set(outcomes) != {p.name for p in paths}:
        raise SystemExit(f"Missing, duplicate or unexpected oracle results for {ig}")
    return outcomes, result.returncode


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
    by_ig = {}
    for item in manifest:
        by_ig.setdefault(item.get("ig", DEFAULT_IG), []).append(HERE / item["file"])
    outcomes, exits = {}, {}
    for ig, paths in by_ig.items():
        found, exits[ig] = run_oracle(java, jar, ig, paths)
        outcomes.update(found)
    if set(outcomes) != {item["file"] for item in manifest} or len(manifest) != len(outcomes):
        raise SystemExit("Missing, duplicate or unexpected oracle results")
    records = []
    tally = {"agree": 0, "reviewed": 0, "differ": 0}
    for item in manifest:
        path = HERE / item["file"]
        if item["file"] not in outcomes:
            raise SystemExit(f"Missing oracle result: {item['file']}")
        outcome = outcomes[item["file"]]
        issues = [{"severity": i["severity"], "code": i.get("code"), "text": i.get("details", {}).get("text") or i.get("diagnostics", ""), "paths": i.get("expression") or i.get("location", [])} for i in outcome["issue"]]
        oracle_valid = not any(i["severity"] in ("fatal", "error") for i in issues)
        sha256, signature = digest(path), oracle_signature(issues)
        record = {**item, "ig": item.get("ig", DEFAULT_IG), "sha256": sha256, "oracle_valid": oracle_valid,
                  "oracle_signature": signature, "oracle_issues": issues}
        local_valid = None
        if not args.oracle_only:
            probe = subprocess.run([str(ROOT / "target/debug/examples/probe"), str(path)], capture_output=True, text=True, timeout=15)
            local = json.loads(probe.stdout)
            if probe.returncode not in (0, 1) or not isinstance(local.get("valid"), bool):
                raise SystemExit(f"Probe execution failure: {item['file']}")
            record["local"] = local
            local_valid = local["valid"]
        status, note = judge(item, sha256, oracle_valid, signature, local_valid)
        tally[status] += 1
        record["status"] = status
        record["agrees"] = status != "differ"
        if note:
            record["note"] = note
        records.append(record)
        print(f"{ {'agree': 'ok', 'reviewed': 'rev', 'differ': 'DIFF'}[status]:4} {item['file']}: oracle={oracle_valid} expected={item['expect']}"
              + (f" local={local_valid}" if local_valid is not None else "") + (f" ({note})" if note else ""))
    failures = tally["differ"]
    report = {
        "recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT, text=True)),
        "oracle_version": VERSION, "oracle_sha256": digest(jar), "oracle_options": OPTIONS,
        "oracle_igs": sorted(by_ig), "oracle_exit_codes": exits,
        "note": "Terminology service disabled; the oracle may still resolve unknown profile URLs. One oracle run per guide (`ig`). Verdict parity only; a reviewed difference counts only while its input hash, oracle signature and local verdict match the review. Issues retained for inspection.",
        "counts": {"total": len(records), **tally},
        "probes": records,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["counts"]))
    raise SystemExit(1 if failures else 0)


if __name__ == "__main__":
    main()
