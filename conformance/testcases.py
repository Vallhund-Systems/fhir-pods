#!/usr/bin/env python3
"""Pinned FHIR validator test cases, compared with their stored Java/Firely outcomes.

The default base lane needs no JVM. Profile preparation/comparison is opt-in.
Fetched inputs, diagnostics and generated definitions stay in ignored caches.
Reviewed differences are matched by input/outcome hashes and both verdicts;
new differences and execution/evidence failures fail the command.
"""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

import run

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
SUITE = HERE / "fhir-test-cases"
EVIDENCE = HERE / "corpus-testcases"
REPO = "https://github.com/FHIR/fhir-test-cases.git"
PIN = "591528170b6b31609b7682dc67c37f45edfe33ff"
TRIAGE = HERE / "testcases-triage.json"

OUT_OF_SCOPE_MODULES = {
    "tx": "terminology-service tests",
    "tx-advanced": "ValueSet parameter-expression processing",
    "matchetype": "Java matchetype feature",
    "cdshooks": "CDS Hooks logical models",
    "dsig": "digital signatures",
    "shc": "SMART Health Cards",
    "json5": "JSON5 syntax",
    "xver": "cross-version extensions",
    "api": "validator API",
    "security": "validator CLI policy checks",
    "package-versioning": "package version resolution",
    "fmt": "format detection",
    "measure": "CQL and Measure logic",
    "logical": "logical models",
    "scoring": "validator scoring policy",
}
OUT_OF_SCOPE_KEYS = {
    "logical": "logical model",
    "use-test": "validator test mode",
    "fetcher": "external resource fetcher",
    "packages": "packages not carried by the pod",
    "supporting": "external supporting resources",
    "supporting5": "R5 supporting resources",
    "tx-dependent": "terminology service",
    "profile": "ad-hoc profile (separate profile lane)",
    "profiles": "ad-hoc profiles (separate profile lane)",
    "validateContains": "configured contained-resource policy",
    "validateReference": "configured reference-validation policy",
    "assumeValidRestReferences": "configured reference-validation policy",
    "r5-bundle-relative-reference-policy": "R5 reference policy in R4",
    "noHtmlInMarkdown": "configured markdown policy",
    "fetch-canonicals": "canonical fetching and automatic profile selection",
    "advisor": "validator advisor",
    "launch-context": "questionnaire launch context",
    "security-checks": "validator CLI policy checks",
    "fhir-validator-cli-options": "validator CLI options",
    "allowed-extension-domain": "configured extension-domain policy",
    "no-experimental-content": "configured publication policy",
    "for-publication": "configured publication policy",
    "ips": "validator IPS mode",
    "allow-comments": "JSON comment mode",
    "package-map": "package resolution mapping",
    "new-xver-mode": "cross-version processing",
}
CARRIED = set(run.IGS) | {"hl7.fhir.uv.extensions.r4#5.3.0", "hl7.terminology.r4#7.3.0", "hl7.fhir.r4.core#4.0.1"}
SEVERITIES = {"error", "fatal", "warning", "information"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def json_hash(value):
    return digest(json.dumps(value, sort_keys=True, separators=(",", ":")).encode())


def git(*args, cwd=ROOT):
    return subprocess.check_output(["git", *args], cwd=cwd, text=True).strip()


def fetch():
    if not SUITE.exists():
        subprocess.run(["git", "init", "-q", str(SUITE)], check=True)
        subprocess.run(["git", "-C", str(SUITE), "remote", "add", "origin", REPO], check=True)
        subprocess.run(["git", "-C", str(SUITE), "sparse-checkout", "init", "--cone"], check=True)
        subprocess.run(["git", "-C", str(SUITE), "sparse-checkout", "set", "validator"], check=True)
        subprocess.run(["git", "-C", str(SUITE), "fetch", "--depth", "1", "--filter=blob:none", "origin", PIN], check=True)
        subprocess.run(["git", "-C", str(SUITE), "checkout", "--detach", "FETCH_HEAD"], check=True)
    if git("rev-parse", "HEAD", cwd=SUITE) != PIN:
        raise ValueError(f"suite checkout must be {PIN}; preserve other revisions in a separate cache")
    if git("status", "--porcelain", "--untracked-files=all", cwd=SUITE):
        raise ValueError("suite has modified or untracked files; cannot attribute results to its pinned revision")


def suite_file(relative, outcomes=False):
    base = SUITE / "validator"
    path = (base / ("outcomes" if outcomes else "") / relative).resolve()
    if not path.is_relative_to(base.resolve()):
        raise ValueError("manifest path is outside validator/")
    return path


def read_outcome(reference):
    if isinstance(reference, str):
        return json.loads(suite_file(reference, outcomes=True).read_text())
    if isinstance(reference, dict) and isinstance(reference.get("outcome"), dict):
        return reference["outcome"]
    raise ValueError("Java expectation is neither an outcome file nor an inline outcome")


def java_verdict(outcome):
    if not isinstance(outcome, dict) or outcome.get("resourceType") != "OperationOutcome":
        raise ValueError("Java expectation is not an OperationOutcome")
    # The suite intentionally serializes a clean expected outcome without issue.
    issues = outcome.get("issue", [])
    if not isinstance(issues, list) or any(not isinstance(i, dict) or i.get("severity") not in SEVERITIES for i in issues):
        raise ValueError("malformed OperationOutcome issues")
    errors = [i for i in issues if i["severity"] in ("error", "fatal")]
    return "invalid" if errors else "valid", run.error_paths(outcome)


def firely_verdict(lines):
    if not isinstance(lines, list) or any(not isinstance(line, str) for line in lines):
        raise ValueError("Firely expectation is not a list of diagnostic strings")
    # Empty is an explicit successful outcome. Unknown lines are not success.
    if any(not line.startswith(("[ERROR]", "[WARNING]", "[INFORMATION]", "[INFO]", "[FATAL]")) for line in lines):
        raise ValueError("unrecognized Firely diagnostic severity")
    return "invalid" if any(line.startswith(("[ERROR]", "[FATAL]")) for line in lines) else "valid"


def scope(case, profiles=False):
    module = case.get("module")
    if module in OUT_OF_SCOPE_MODULES:
        return {"reason": OUT_OF_SCOPE_MODULES[module], "manifest_key": "module", "manifest_value": module}
    for key, reason in OUT_OF_SCOPE_KEYS.items():
        if profiles and key in ("profile", "profiles", "supporting"):
            continue
        value = case.get(key)
        if not value:
            continue
        if key == "packages" and isinstance(value, list) and set(value) <= CARRIED:
            continue
        return {"reason": reason, "manifest_key": key, "manifest_value": value}
    return None


def native(probe, path):
    proc = subprocess.run([str(probe), "--outcome", str(path)], capture_output=True, text=True, timeout=60)
    outcome = json.loads(proc.stdout)
    verdict, paths = java_verdict(outcome)
    if proc.returncode not in (0, 1) or (proc.returncode == 1) != (verdict == "invalid"):
        raise ValueError(f"native exit {proc.returncode} disagrees with the outcome")
    return verdict, paths, outcome


def build(features=False):
    # Separate targets prevent an optional registry from contaminating a default probe.
    target = ROOT / "target" / ("testcase-profiles" if features else "testcases")
    args = ["cargo", "build", "-q", "-j", "2", "-p", "fhir-pods-core", "--example", "probe", "--target-dir", str(target)]
    if features:
        args += ["--features", "testcase-profiles"]
    else:
        args += ["--release"]
    subprocess.run(args, cwd=ROOT, check=True, env={**os.environ, "CARGO_BUILD_JOBS": "2"})
    return target / ("debug" if features else "release") / "examples" / "probe"


def review(row, adjudications):
    decision = adjudications.get(row["name"])
    if row["ours"] == row["java"]:
        return {"status": "agreement"}
    if decision is None:
        return {"status": "unreviewed"}
    for key in ("input_sha256", "java_sha256", "firely_sha256", "ours", "java", "firely"):
        if decision.get(key) != row.get(key):
            return {"status": "stale-review", "changed": key}
    if decision.get("kind") not in ("documented-gap", "oracle-difference", "current-java-differs") or not decision.get("reason"):
        return {"status": "malformed-review"}
    return {"status": "reviewed", **{k: decision[k] for k in ("kind", "reason")},
            **({"current_java": decision["current_java"]} if "current_java" in decision else {})}


def compare(case, probe, *, profile=False, input_path=None):
    expected = case["profile"] if profile else case
    path = input_path or suite_file(case["file"])
    if not path.is_file():
        raise FileNotFoundError(f"input file is missing: {case['file']}")
    outcome = read_outcome(expected["java"])
    jv, jp = java_verdict(outcome)
    fv, fhash = None, None
    if expected.get("firely-sdk-current"):
        lines = json.loads(suite_file(expected["firely-sdk-current"], outcomes=True).read_text())
        fv, fhash = firely_verdict(lines), json_hash(lines)
    ov, op, ours_outcome = native(probe, path)
    return {"name": case["name"], "file": case["file"], "module": case.get("module"),
            "input_sha256": digest(path.read_bytes()), "java_sha256": json_hash(outcome), "firely_sha256": fhash,
            "ours": ov, "java": jv, "firely": fv, "ours_paths": sorted(op), "java_paths": sorted(jp),
            "ours_outcome": ours_outcome, "java_outcome": outcome}


def summarize(rows, skipped, errors):
    firely = [r for r in rows if r["firely"] is not None]
    strict = [r for r in rows if r["ours"] == "invalid" and r["java"] == "valid"]
    unresolved_strict = [r for r in strict if r["firely"] != "invalid" and r["review"].get("current_java") != "invalid"]
    return {"compared": len(rows), "java_agree": sum(r["ours"] == r["java"] for r in rows),
            "firely_compared": len(firely), "firely_agree": sum(r["ours"] == r["firely"] for r in firely),
            "stricter_than_stored_java": len(strict),
            "stricter_than_both_stored_engines": sum(r["firely"] == "valid" for r in strict),
            "unresolved_stricter": len(unresolved_strict),
            "lenient": sum(r["ours"] == "valid" and r["java"] == "invalid" for r in rows),
            "unreviewed": sum(r["review"]["status"] not in ("agreement", "reviewed") for r in rows),
            "profile_changes_verdict": sum("default_ours" in r and r["default_ours"] != r["ours"] for r in rows),
            "out_of_scope": len(skipped), "out_of_scope_reasons": dict(Counter(r["reason"] for r in skipped)),
            "evidence_errors": len(errors)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--filter", help="case-name substring")
    parser.add_argument("--module", help="manifest module")
    parser.add_argument("--profiles", action="store_true", help="run the prepared ad-hoc profile lane")
    parser.add_argument("--prepare-profiles", action="store_true", help="prepare ignored snapshots using pinned Java (one JVM)")
    args = parser.parse_args(argv)
    fetch()
    EVIDENCE.mkdir(exist_ok=True)
    manifest = json.loads(suite_file("manifest.json").read_text())["test-cases"]
    r4 = [t for t in manifest if str(t.get("version", "")).split(".")[:2] == ["4", "0"]]
    selected = [t for t in r4 if t["file"].endswith(".json") and (not args.filter or args.filter in t["name"])
                and (not args.module or args.module == t.get("module"))]
    if not selected:
        parser.error("no matching R4 JSON cases")
    triage = json.loads(TRIAGE.read_text()) if TRIAGE.exists() else {"suite_commit": PIN, "base": {}, "profiles": {}}
    if triage.get("suite_commit") != PIN:
        raise ValueError("triage revision does not match pinned suite")
    if args.profiles or args.prepare_profiles:
        import testcase_profiles
        return testcase_profiles.main(args, selected, triage)
    probe = build()
    rows, skipped, errors = [], [], []
    for case in selected:
        why = scope(case)
        if why:
            skipped.append({"name": case["name"], "file": case["file"], **why})
            continue
        unavailable = triage.get("unavailable", {}).get(case["name"])
        if (unavailable and unavailable.get("case_sha256") == json_hash(case)
                and unavailable.get("file") == case["file"] and not suite_file(case["file"]).exists()):
            skipped.append({"name": case["name"], "file": case["file"], "manifest_key": "file",
                            "manifest_value": case["file"], "reason": unavailable["reason"], "unavailable": True})
            continue
        try:
            row = compare(case, probe)
            row["review"] = review(row, triage.get("base", {}))
            rows.append(row)
        except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as ex:
            errors.append({"name": case["name"], "file": case["file"], "error": str(ex)})
    return finish("base", args, manifest, r4, selected, rows, skipped, errors, probe)


def finish(lane, args, manifest, r4, selected, rows, skipped, errors, probe=None):
    summary = summarize(rows, skipped, errors)
    evidence = {"suite": REPO, "suite_commit": PIN, "manifest_sha256": digest(suite_file("manifest.json").read_bytes()),
                "source_commit": git("rev-parse", "HEAD"), "source_dirty": bool(git("status", "--porcelain", "--untracked-files=no")),
                "lane": lane, "selection": {"filter": args.filter, "module": args.module},
                "inventory": {"all": len(manifest), "r4": len(r4), "r4_json": sum(t["file"].endswith(".json") for t in r4),
                              "r4_xml_not_run": sum(t["file"].endswith(".xml") for t in r4), "selected": len(selected)},
                "summary": summary, "results": rows, "out_of_scope": skipped, "errors": errors}
    if probe:
        evidence["probe_sha256"] = digest(Path(probe).read_bytes())
    (EVIDENCE / f"{lane}-evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(f"{lane}: {summary['compared']} compared; Java {summary['java_agree']}/{summary['compared']}; "
          f"Firely {summary['firely_agree']}/{summary['firely_compared']}")
    print(f"  stored-Java stricter: {summary['stricter_than_stored_java']}; stricter than both stored engines: "
          f"{summary['stricter_than_both_stored_engines']}; unresolved stricter: {summary['unresolved_stricter']}")
    print(f"  lenient: {summary['lenient']}; unreviewed: {summary['unreviewed']}; evidence errors: {len(errors)}")
    for reason, count in sorted(summary["out_of_scope_reasons"].items()):
        print(f"  excluded {count}: {reason}")
    for row in rows:
        if row["ours"] != row["java"]:
            print(f"  {row['name']}: {row['ours']}/{row['java']} firely={row['firely']} [{row['review']['status']}]")
    for row in errors:
        print(f"  evidence error: {row['name']}: {row['error']}")
    return int(bool(errors or summary["unreviewed"] or summary["unresolved_stricter"]))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as exc:
        sys.exit(f"testcases: {exc}")
