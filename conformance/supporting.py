#!/usr/bin/env python3
"""The supporting-resource lane: fhir-test-cases' R4 JSON cases that come
with `supporting` definitions — a StructureDefinition the instance declares
or an extension it uses, a CodeSystem / ValueSet its codes are held to.

Each kind is given to the pod the way a host would give it:

  - StructureDefinitions are snapshotted by the pinned validator_cli
    (`testcase_profiles.prepare`, one JVM at a time), compiled into an
    isolated test registry (`testcase_profiles.compile_context`, the
    `testcase-profiles` feature build) — the same mechanism the ad-hoc
    profile lane uses, reused, never edited;
  - ValueSets and CodeSystems are compiled into a terminology table
    (`tools/compile_valueset.py`'s line format) when they are enumerations
    the pod can decide — a value set composed of listed concepts or of whole
    code systems given beside it — and loaded with `probe --terminology`.

Then the instance itself, unchanged, is validated and compared with the
stored Java outcome on the verdict and on every error-level issue
(`reasons.py`'s matcher). Cases in the questionnaire lane (supporting
Questionnaires only), the ad-hoc profile lane (`profile` key), or needing a
package, an ImplementationGuide, a validator option or a definition the pod
cannot decide (a filter, a supplement) are excluded with the reason. Exit 1
on any difference not named in `supporting-triage.json`, path-exact.

    python3 conformance/supporting.py [--filter SUBSTRING] [--prepare]

`--prepare` runs the Java snapshot step (needed once per suite pin / JAR).
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import reasons  # noqa: E402
import testcase_profiles as tp  # noqa: E402
import testcases as tc  # noqa: E402

EVIDENCE = HERE / "corpus-supporting"
TRIAGE = HERE / "supporting-triage.json"
PREPARED = EVIDENCE / "prepared.json"

OUT_OF_SCOPE = {
    "launch-context": "questionnaire launch context",
    "use-test": "validator test mode",
    "validateReference": "configured reference-validation policy",
    "assumeValidRestReferences": "configured reference-validation policy",
    "logical": "logical model",
    "tx-dependent": "terminology service",
    "package-map": "package resolution mapping",
}


def read(path):
    return json.loads(tc.suite_file(path).read_text())


def classify(case):
    """Sort the case's supporting files: definitions to compile, terminology
    to table, or a reason the case is out of reach."""
    if isinstance(case.get("profile"), dict):
        return None, {"reason": "ad-hoc profile lane (testcases.py --profiles)", "manifest_key": "profile"}
    if case.get("module") in tc.OUT_OF_SCOPE_MODULES:
        return None, {"reason": tc.OUT_OF_SCOPE_MODULES[case["module"]], "manifest_key": "module", "manifest_value": case["module"]}
    for key, reason in OUT_OF_SCOPE.items():
        if case.get(key):
            return None, {"reason": reason, "manifest_key": key, "manifest_value": case[key]}
    packages = [p for p in case.get("packages", []) if p not in tc.CARRIED]
    if packages:
        return None, {"reason": "packages not carried by the pod", "manifest_key": "packages", "manifest_value": packages}
    sds, terms, questionnaires = [], [], []
    for s in case["supporting"]:
        path = tc.suite_file(s)
        if not path.is_file():
            return None, {"reason": "supporting file missing from the suite", "manifest_key": "supporting", "manifest_value": s}
        if not s.endswith(".json"):
            return None, {"reason": "supporting definition is not JSON", "manifest_key": "supporting", "manifest_value": s}
        try:
            rt = json.loads(path.read_text()).get("resourceType")
        except ValueError:
            return None, {"reason": "supporting file is not JSON", "manifest_key": "supporting", "manifest_value": s}
        if rt == "StructureDefinition":
            sds.append(s)
        elif rt in ("ValueSet", "CodeSystem"):
            terms.append(s)
        elif rt == "Questionnaire":
            questionnaires.append(s)
        else:
            return None, {"reason": f"supporting {rt} is an instance the pod does not take", "manifest_key": "supporting", "manifest_value": s}
    if not sds and not terms:
        return None, {"reason": "questionnaire lane (questionnaire.py)", "manifest_key": "supporting"}
    return {"sds": sds, "terms": terms, "questionnaires": questionnaires}, None


def root_definition(case, sds):
    """The definition the instance answers to: one it declares, else the
    extension it uses, else the one for its type, else the first."""
    instance = read(case["file"])
    defs = [(s, read(s)) for s in sds]
    declared = set(instance.get("meta", {}).get("profile", []))
    for s, sd in defs:
        if sd.get("url") in declared:
            return s
    text = json.dumps(instance)
    for s, sd in defs:
        if sd.get("type") == "Extension" and sd.get("url") and sd["url"] in text:
            return s
    for s, sd in defs:
        if sd.get("type") == instance.get("resourceType"):
            return s
    return sds[0]


def compile_terminology(terms):
    """A table for the value sets among `terms` that enumerate from listed
    concepts or from whole code systems given beside them; `None` with the
    reason when one cannot be decided."""
    systems, sets = {}, []
    for s in terms:
        d = read(s)
        if d["resourceType"] == "CodeSystem":
            if d.get("content", "complete") != "complete" or d.get("supplements"):
                return None, f"code system {d.get('url')} is not a complete, standalone system"
            codes = []

            def walk(concepts):
                for c in concepts:
                    codes.append((c["code"], c.get("display", "")))
                    walk(c.get("concept", []))

            walk(d.get("concept", []))
            systems[d.get("url")] = codes
        else:
            sets.append(d)
    lines = ["fhir-pods-terminology 1"]
    for vs in sets:
        compose = vs.get("compose") or {}
        if compose.get("exclude"):
            return None, f"value set {vs.get('url')} has an exclude"
        members = []
        for inc in compose.get("include", []):
            if inc.get("filter") or inc.get("valueSet"):
                return None, f"value set {vs.get('url')} is defined by a filter or another set"
            system = inc.get("system")
            if inc.get("concept"):
                members += [(system, c["code"], c.get("display", "")) for c in inc["concept"]]
            elif system in systems:
                members += [(system, code, display) for code, display in systems[system]]
            else:
                return None, f"value set {vs.get('url')} includes the whole of {system}, which is not given"
        lines.append(f"= {vs.get('url')}")
        lines += [f"{s}\t{c}\t{d}" for s, c, d in members]
    if not sets:
        return None, "supporting code systems bind no value set the pod is given"
    return "\n".join(lines) + "\n", None


def native(probe, path, questionnaires, table):
    args = [str(probe), "--outcome"]
    for q in questionnaires:
        args += ["--questionnaire", str(tc.suite_file(q))]
    if table:
        args += ["--terminology", str(table)]
    proc = subprocess.run(args + [str(path)], capture_output=True, text=True, timeout=120)
    if proc.returncode not in (0, 1):
        raise ValueError(f"native exit {proc.returncode}: {proc.stderr.strip()[:200]}")
    outcome = json.loads(proc.stdout)
    verdict, paths = tc.java_verdict(outcome)
    return verdict, paths, outcome


def differences(ours, theirs):
    oe, te = reasons.issues(ours, ("error", "fatal")), reasons.issues(theirs, ("error", "fatal"))
    missed = [{"path": p, "message": t[:200]} for p, _, t in te if not any(reasons.same(p, q, t, t2) for q, _, t2 in oe)]
    extra = [{"path": p, "message": t[:200]} for p, _, t in oe if not any(reasons.same(p, q, t, t2) for q, _, t2 in te)]
    return missed, extra


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--filter", help="case-name substring")
    parser.add_argument("--prepare", action="store_true", help="snapshot the supporting definitions with pinned Java")
    args = parser.parse_args(argv)
    tc.fetch()
    EVIDENCE.mkdir(exist_ok=True)
    tp.PREPARED = PREPARED  # this lane's own snapshot cache
    manifest = json.loads(tc.suite_file("manifest.json").read_text())["test-cases"]
    cases = [t for t in manifest if t.get("supporting") and str(t.get("version", "")).split(".")[:2] == ["4", "0"]
             and t["file"].endswith(".json") and (not args.filter or args.filter in t["name"])]
    plans, skipped = [], []
    for case in cases:
        plan, why = classify(case)
        if why:
            skipped.append({"name": case["name"], "file": case["file"], **why})
            continue
        synthetic = None
        if plan["sds"]:
            # the ad-hoc profile lane's context: a root definition plus the rest
            synthetic = {**case, "profile": {"source": root_definition(case, plan["sds"]), "java": case["java"],
                                             "supporting": [s for s in case["supporting"] if s in plan["sds"] or s in plan["terms"]]}}
            config, why = tp.context(synthetic)
            if why:
                skipped.append({"name": case["name"], "file": case["file"], **why})
                continue
            plan["context"] = config
        plans.append((case, synthetic, plan))
    if args.prepare:
        return tp.prepare([s for _, s, _ in plans if s])
    prepared = json.loads(PREPARED.read_text()) if PREPARED.exists() else {}
    if prepared.get("suite_commit") != tc.PIN and any(s for _, s, _ in plans):
        raise ValueError("prepare first: python3 conformance/supporting.py --prepare")
    triage = json.loads(TRIAGE.read_text()) if TRIAGE.exists() else {}
    default_probe = tc.build()
    rows, errors, ungated = [], [], []
    for case, synthetic, plan in plans:
        try:
            table = None
            if plan["terms"]:
                text, why = compile_terminology(plan["terms"])
                if why:
                    skipped.append({"name": case["name"], "file": case["file"], "reason": f"terminology the pod cannot decide: {why}",
                                    "manifest_key": "supporting", "manifest_value": plan["terms"]})
                    continue
                table = EVIDENCE / "tables" / (tc.json_hash(case) + ".txt")
                table.parent.mkdir(exist_ok=True)
                table.write_text(text)
            probe = default_probe
            if synthetic:
                key = tc.json_hash(plan["context"])
                record = prepared.get("contexts", {}).get(key)
                if not record:
                    errors.append({"name": case["name"], "error": "missing prepared context; rerun --prepare"})
                    continue
                if record["status"] != "ready":
                    skipped.append({"name": case["name"], "file": case["file"], "reason": record["reason"],
                                    "manifest_key": "supporting", "manifest_value": plan["sds"]})
                    continue
                url, gaps = tp.compile_context(record)
                if gaps:
                    skipped.append({"name": case["name"], "file": case["file"], "reason": "profile compiler gaps",
                                    "manifest_key": "supporting", "manifest_value": plan["sds"], "gaps": gaps})
                    continue
                probe = tc.build(features=True)
            path = tc.suite_file(case["file"])
            java = tc.read_outcome(case["java"])
            jv, jp = tc.java_verdict(java)
            ov, op, ours = native(probe, path, plan["questionnaires"], table)
            missed, extra = differences(ours, java)
            row = {"name": case["name"], "file": case["file"], "module": case.get("module"), "supporting": case["supporting"],
                   "mechanism": ("definitions" if synthetic else "") + ("+" if synthetic and table else "") + ("terminology" if table else ""),
                   "input_sha256": tc.digest(path.read_bytes()), "java_sha256": tc.json_hash(java),
                   "ours": ov, "java": jv, "ours_paths": sorted(op), "java_paths": sorted(jp), "missed": missed, "extra": extra}
            t = triage.get(case["name"]) or {}
            differs = ov != jv or missed or extra
            if differs:
                covered = (t.get("ours", ov) == ov and t.get("java", jv) == jv
                           and all(m["path"] in t.get("missed", []) for m in missed)
                           and all(x["path"] in t.get("extra", []) for x in extra))
                row["review"] = "reviewed" if t and covered else ("stale-review" if t else "unreviewed")
            else:
                row["review"] = "stale-review" if t else "agreement"
            if row["review"] not in ("agreement", "reviewed"):
                ungated.append(row)
            rows.append(row)
            print(f"  {case['name']}: {ov}/{jv} [{row['review']}] ({row['mechanism']})", flush=True)
        except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as ex:
            errors.append({"name": case["name"], "file": case["file"], "error": str(ex)})
    agree = sum(1 for r in rows if r["ours"] == r["java"])
    exact = sum(1 for r in rows if r["ours"] == r["java"] and not r["missed"] and not r["extra"])
    evidence = {"suite": tc.REPO, "suite_commit": tc.PIN, "lane": "supporting",
                "source_commit": tc.git("rev-parse", "HEAD"),
                "source_dirty": bool(tc.git("status", "--porcelain", "--untracked-files=no")),
                "summary": {"cases": len(cases), "compared": len(rows), "verdicts_agree": agree, "errors_agree": exact,
                            "out_of_scope": len(skipped), "errors": len(errors), "ungated": len(ungated)},
                "results": rows, "out_of_scope": skipped, "errors": errors}
    (EVIDENCE / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(f"supporting: {len(rows)} compared of {len(cases)}; verdicts agree {agree}/{len(rows)}; every error agrees "
          f"{exact}/{len(rows)}; {len(skipped)} out of scope; {len(errors)} evidence errors")
    counts = {}
    for s in skipped:
        counts[s["reason"]] = counts.get(s["reason"], 0) + 1
    for reason, n in sorted(counts.items(), key=lambda kv: -kv[1]):
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
        sys.exit(f"supporting: {exc}")
