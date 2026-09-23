#!/usr/bin/env python3
"""The questionnaire lane: fhir-test-cases' `questionnaire` module, R4 JSON.

Each case is a QuestionnaireResponse (or a Questionnaire) with the
Questionnaire it answers to as a "supporting" file, and the Java validator's
stored outcome. The pod is given the supporting Questionnaires the way a host
gives them (`probe --questionnaire FILE`), and compared with Java twice:

  - the verdict (valid / invalid), as every other lane scores it;
  - every error-level issue, both ways (paths to one spelling, messages by
    family — `reasons.py`'s comparison), because agreeing on a verdict is
    not agreeing on why.

Cases that need what the pod does not carry are excluded with the reason
(an SDC package, a launch context, a validator policy option, a supporting
resource that is not a Questionnaire). Exit 1 on any difference not named
in `questionnaire-triage.json` (a reviewed reason per case, path-exact),
and on a triaged case that no longer differs.

    python3 conformance/questionnaire.py [--filter SUBSTRING]
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import reasons  # noqa: E402
import testcases as tc  # noqa: E402

EVIDENCE = HERE / "corpus-questionnaire"
TRIAGE = HERE / "questionnaire-triage.json"

# manifest keys that put a case outside what the pod is given
OUT_OF_SCOPE = {
    "launch-context": "questionnaire launch context",
    "use-test": "validator test mode",
    "validateReference": "configured reference-validation policy",
    "assumeValidRestReferences": "configured reference-validation policy",
    "logical": "logical model",
    "tx-dependent": "terminology service",
}


def scope(case):
    for key, reason in OUT_OF_SCOPE.items():
        if case.get(key):
            return {"reason": reason, "manifest_key": key, "manifest_value": case[key]}
    packages = [p for p in case.get("packages", []) if p not in tc.CARRIED]
    if packages:
        return {"reason": "packages not carried by the pod", "manifest_key": "packages", "manifest_value": packages}
    for s in case.get("supporting", []):
        path = tc.suite_file(s)
        if not path.is_file():
            return {"reason": "supporting file missing from the suite", "manifest_key": "supporting", "manifest_value": s}
        try:
            rt = json.loads(path.read_text()).get("resourceType")
        except ValueError:
            rt = None
        if rt != "Questionnaire":
            return {"reason": f"supporting {rt or 'non-JSON'} (only Questionnaires are given to the pod)",
                    "manifest_key": "supporting", "manifest_value": s}
    return None


def native(probe, path, supporting):
    args = [str(probe), "--outcome"]
    for s in supporting:
        args += ["--questionnaire", str(tc.suite_file(s))]
    proc = subprocess.run(args + [str(path)], capture_output=True, text=True, timeout=60)
    if proc.returncode not in (0, 1):
        raise ValueError(f"native exit {proc.returncode}: {proc.stderr.strip()[:200]}")
    outcome = json.loads(proc.stdout)
    verdict, paths = tc.java_verdict(outcome)
    if (proc.returncode == 1) != (verdict == "invalid"):
        raise ValueError(f"native exit {proc.returncode} disagrees with the outcome")
    return verdict, paths, outcome


def differences(ours, theirs):
    """Error-level issues one side raises where the other raises none."""
    oe, te = reasons.issues(ours, ("error", "fatal")), reasons.issues(theirs, ("error", "fatal"))
    missed = [{"path": p, "message": t[:200]} for p, _, t in te if not any(reasons.same(p, q, t, t2) for q, _, t2 in oe)]
    extra = [{"path": p, "message": t[:200]} for p, _, t in oe if not any(reasons.same(p, q, t, t2) for q, _, t2 in te)]
    return missed, extra


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--filter", help="case-name substring")
    args = parser.parse_args(argv)
    tc.fetch()
    EVIDENCE.mkdir(exist_ok=True)
    manifest = json.loads(tc.suite_file("manifest.json").read_text())["test-cases"]
    cases = [t for t in manifest if t.get("module") == "questionnaire"
             and str(t.get("version", "")).split(".")[:2] == ["4", "0"] and t["file"].endswith(".json")
             and (not args.filter or args.filter in t["name"])]
    if not cases:
        parser.error("no matching cases")
    triage = json.loads(TRIAGE.read_text()) if TRIAGE.exists() else {}
    probe = tc.build()
    rows, skipped, errors, ungated = [], [], [], []
    for case in cases:
        why = scope(case)
        if why:
            skipped.append({"name": case["name"], "file": case["file"], **why})
            continue
        try:
            path = tc.suite_file(case["file"])
            java = tc.read_outcome(case["java"])
            jv, jp = tc.java_verdict(java)
            ov, op, ours = native(probe, path, case.get("supporting", []))
            missed, extra = differences(ours, java)
            row = {"name": case["name"], "file": case["file"], "supporting": case.get("supporting", []),
                   "input_sha256": tc.digest(path.read_bytes()), "java_sha256": tc.json_hash(java),
                   "ours": ov, "java": jv, "ours_paths": sorted(op), "java_paths": sorted(jp),
                   "missed": missed, "extra": extra}
            t = triage.get(case["name"]) or {}
            accepted_missed, accepted_extra = t.get("missed", []), t.get("extra", [])
            differs = ov != jv or missed or extra
            if differs:
                covered = (t.get("ours", ov) == ov and t.get("java", jv) == jv
                           and all(m["path"] in accepted_missed for m in missed)
                           and all(x["path"] in accepted_extra for x in extra))
                row["review"] = "reviewed" if t and covered else ("stale-review" if t else "unreviewed")
                if row["review"] != "reviewed":
                    ungated.append(row)
            else:
                row["review"] = "agreement"
                if t:
                    row["review"] = "stale-review"
                    ungated.append(row)
            rows.append(row)
        except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as ex:
            errors.append({"name": case["name"], "file": case["file"], "error": str(ex)})
    agree = sum(1 for r in rows if r["ours"] == r["java"])
    exact = sum(1 for r in rows if not r["missed"] and not r["extra"] and r["ours"] == r["java"])
    evidence = {"suite": tc.REPO, "suite_commit": tc.PIN, "lane": "questionnaire",
                "source_commit": tc.git("rev-parse", "HEAD"),
                "source_dirty": bool(tc.git("status", "--porcelain", "--untracked-files=no")),
                "probe_sha256": tc.digest(Path(probe).read_bytes()),
                "summary": {"cases": len(cases), "compared": len(rows), "verdicts_agree": agree,
                            "errors_agree": exact, "out_of_scope": len(skipped), "errors": len(errors),
                            "ungated": len(ungated)},
                "results": rows, "out_of_scope": skipped, "errors": errors}
    (EVIDENCE / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(f"questionnaire: {len(rows)} compared of {len(cases)}; verdicts agree {agree}/{len(rows)}; "
          f"every error agrees {exact}/{len(rows)}; {len(skipped)} out of scope; {len(errors)} evidence errors")
    reasons_out = {}
    for s in skipped:
        reasons_out[s["reason"]] = reasons_out.get(s["reason"], 0) + 1
    for reason, n in sorted(reasons_out.items()):
        print(f"  excluded {n}: {reason}")
    for r in rows:
        if r["review"] != "agreement":
            print(f"  {r['name']}: ours {r['ours']} / java {r['java']} [{r['review']}]")
            for m in r["missed"]:
                print(f"      missed  {m['path']}: {m['message'][:110]}")
            for x in r["extra"]:
                print(f"      extra   {x['path']}: {x['message'][:110]}")
    for e in errors:
        print(f"  evidence error: {e['name']}: {e['error']}")
    return int(bool(errors or ungated))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as exc:
        sys.exit(f"questionnaire: {exc}")
