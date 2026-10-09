#!/usr/bin/env python3
"""The gut-path lane: the synthetic instrument scenario in `gut/make_gut.py`
— a sixteen-part questionnaire with its own code systems and value sets,
responses held to it, and the derived chain (observations, flag, summary,
referral, consent, export bundle, audit record) — judged end to end three
ways: what the scenario expects, what the pod says, what validator_cli
6.10.4 says with the same definitions loaded as an extra IG folder.

    JAVA_HOME=... python3 conformance/gut.py            # generate, judge the cases
    JAVA_HOME=... python3 conformance/gut.py --chain    # + lift/lower, de-identify, export for AU; judge the derived files
    python3 conformance/gut.py --reuse-oracle            # pod side only
    python3 conformance/gut.py --mutate                  # + the mutation classes over every case

The pod is given the instrument the way a host gives it (`probe
--questionnaire`) and the local value sets as a compiled terminology table
(`--terminology`). Exit 1 on any case where the three disagree and
`gut-triage.json` does not name it.
"""
import argparse
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, "gut"))
import run  # noqa: E402
import reasons  # noqa: E402
import make_gut  # noqa: E402

ROOT = run.ROOT
OUT = make_gut.OUT
DEFS = make_gut.DEFS
CASES = make_gut.CASES
TRIAGE = os.path.join(HERE, "gut-triage.json")
QUESTIONNAIRE = os.path.join(DEFS, "Questionnaire-gut-instrument.json")
TABLE = os.path.join(OUT, "gut.terminology.txt")


def ours(probe, path, questionnaire=True):
    """The pod's verdict and error paths, the instrument and table loaded."""
    args = [probe, "--outcome", "--terminology", TABLE]
    if questionnaire:
        args += ["--questionnaire", QUESTIONNAIRE]
    r = subprocess.run(args + [path], capture_output=True, text=True)
    try:
        o = json.loads(r.stdout)
    except ValueError:
        return "error", set(), [r.stderr[-300:]]
    if r.returncode not in (0, 1) or not isinstance(o, dict) or o.get("resourceType") != "OperationOutcome":
        return "error", set(), [r.stderr[-300:]]
    bad = [i for i in o.get("issue", []) if i.get("severity") in ("error", "fatal")]
    detail = [f"{(i.get('details') or {}).get('text', '')} @ {i.get('expression')}" for i in bad]
    return ("invalid" if bad else "valid"), {reasons.normalise(p) for p in run.error_paths(o)}, detail


def chain_stage(probe, manifest, meta):
    """The rest of the path, derived from the valid cases and judged like
    them: every valid resource lifted through its base profile and lowered
    back (`lowered-*`), every valid resource de-identified (`deid-*`), and the
    record exported for the AU region (`export-au-*`: each re-profiled
    resource, the region's document, the audit event). Each derived file
    carries the pod's own verdict as its expectation."""
    derived = []
    # the derivation proofs write beside the cases, with a manifest each
    for example, manifest_name in (("lower_corpus", "lowered-manifest.json"), ("deid_corpus", "deid-manifest.json")):
        subprocess.run(["cargo", "run", "-q", "-p", "fhir-pods-core", "--example", example, "--", CASES],
                       cwd=ROOT, capture_output=True, text=True)
        try:
            for row in json.load(open(os.path.join(CASES, manifest_name))):
                src = row["file"].split("-", 1)[1]
                if not meta.get(src, {}).get("expect", True):
                    continue  # a copy of an invalid case proves nothing
                needs_q = meta.get(src, {}).get("questionnaire", True)
                # the proof's own verdict is given without the instrument; the
                # expectation is the pod's verdict with it loaded, the way a
                # host holds the copy (a de-identified response that lost a
                # required free-text answer is then invalid on both sides —
                # a host's instrument-design finding, not a disagreement)
                verdict, _, _ = ours(probe, os.path.join(CASES, row["file"]), needs_q)
                meta[row["file"]] = {"expect": verdict == "valid", "questionnaire": needs_q, "why": f"{example} of {src}"}
                derived.append(os.path.join(CASES, row["file"]))
        except (OSError, ValueError):
            print(f"!! {example} wrote no manifest", file=sys.stderr)
    # the AU export of the record a host would hold
    record = {
        "resources": [json.load(open(os.path.join(CASES, f))) for f in
                      ("patient.json", "device.json", "qr-full.json", "obs-score-1.json", "obs-stool-1.json",
                       "obs-severity-1.json", "obs-alarm-1.json", "flag-alarm.json", "servicerequest-referral.json",
                       "consent-research.json")],
        "author": "Device/gut-module",
        "date": "2026-10-09T10:00:00+10:00",
        "audit": {
            "recorded": "2026-10-09T10:05:00+10:00",
            "client": {"who": "Device/gut-module", "network": "10.0.0.7"},
            "server": {"who": {"display": "export.example.org"}, "network": "https://export.example.org/r4"},
            "user": {"who": "Patient/p1", "name": "The member"},
            "source": {"observer": "Device/gut-module", "site": "gut-module"},
            "transaction": "export-1",
        },
    }
    record_path = os.path.join(OUT, "record.json")
    json.dump(record, open(record_path, "w"))
    subprocess.run(["cargo", "build", "-q", "-p", "fhir-pods-cli"], cwd=ROOT, capture_output=True, text=True)
    cli = os.path.join(ROOT, "target", "debug", "fhir-pods")
    r = subprocess.run([cli, "export", "au", "--terminology", TABLE, record_path], capture_output=True, text=True)
    try:
        exported = json.loads(r.stdout)
    except ValueError:
        print(f"!! export au produced no JSON: {r.stderr[-400:]}", file=sys.stderr)
        return derived
    outputs = []
    for item in exported.get("resources", []):
        outputs.append((f"export-au-{item.get('resourceType', 'x')}-{item.get('id') or item.get('index')}", item))
    for key in ("document", "audit"):
        item = exported.get(key)
        if isinstance(item, dict):
            body = item.get("document") or item.get("resource")
            if isinstance(body, dict):
                outputs.append((f"export-au-{key}", {"resource": body, "validation": item.get("validation")}))
    for name, item in outputs:
        p = os.path.join(CASES, f"{name}.json")
        json.dump(item["resource"], open(p, "w"), indent=1)
        valid = (item.get("validation") or {}).get("valid")
        meta[f"{name}.json"] = {"expect": valid, "questionnaire": True, "why": "export au output; expectation is the pod's own verdict"}
        derived.append(p)
    return derived


def judge(probe, paths, meta, oracle):
    rows = []
    for p in paths:
        key = os.path.basename(p)
        o = oracle.get(key)
        if not o:
            rows.append({"file": key, "kind": "error", "note": "no oracle verdict"})
            continue
        ov, odetail, opaths = o[0], o[1], {reasons.normalise(x) for x in o[2]}
        nv, npaths, ndetail = ours(probe, p, meta.get(key, {}).get("questionnaire", True))
        # one spelling for a choice on both sides (`value[x]` / `valueString`)
        opaths = {x for x in opaths if not any(reasons.same(x, y) for y in npaths)} | (opaths & npaths)
        npaths = {y for y in npaths if not any(reasons.same(x, y) for x in opaths)} | (opaths & npaths)
        expect = meta.get(key, {}).get("expect")
        expected = None if expect is None else ("valid" if expect else "invalid")
        if nv == "error" or ov == "error":
            kind = "error"
        elif nv != ov:
            kind = "lenient" if nv == "valid" else "stricter"
        elif expected is not None and expected != ov:
            kind = "expectation"  # both validators agree; the scenario was wrong about them
        elif nv == "invalid" and (opaths - npaths):
            kind = "missed"
        else:
            kind = "agree"
        rows.append({"file": key, "kind": kind, "expected": expected, "ours": nv, "oracle": ov,
                     "missed": sorted(opaths - npaths), "extra": sorted(npaths - opaths),
                     "oracle_detail": [x[:200] for x in odetail[:4]], "our_detail": [x[:200] for x in ndetail[:4]]})
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--reuse-oracle", action="store_true")
    ap.add_argument("--mutate", action="store_true")
    ap.add_argument("--chain", action="store_true",
                    help="also derive the chain: lowered copies, de-identified copies, the AU export")
    ap.add_argument("--filter")
    a = ap.parse_args()
    make_gut.main()
    subprocess.check_call(["cargo", "build", "-q", "-p", "fhir-pods-core", "--example", "probe"], cwd=ROOT)
    probe = os.path.join(ROOT, "target", "debug", "examples", "probe")
    manifest = json.load(open(os.path.join(CASES, "manifest.json")))
    meta = {c["file"]: c for c in manifest}
    paths = [os.path.join(CASES, c["file"]) for c in manifest if not a.filter or a.filter in c["file"]]
    if a.mutate:
        import mutate
        mdir = os.path.join(OUT, "mutations")
        os.makedirs(mdir, exist_ok=True)
        for f in os.listdir(mdir):
            os.remove(os.path.join(mdir, f))
        for c in manifest:
            if not c["expect"]:
                continue  # mutate what is valid: the oracle says what each mutation costs
            doc = json.load(open(os.path.join(CASES, c["file"])))
            for cls, fn in mutate.CLASSES.items():
                for label, mutated in fn(doc, 6):
                    name = f"{c['file'][:-5]}__{label}.json"
                    json.dump(mutated, open(os.path.join(mdir, name), "w"), separators=(",", ":"))
                    meta[name] = {"questionnaire": c["questionnaire"]}
                    paths.append(os.path.join(mdir, name))
    if a.chain:
        paths += chain_stage(probe, manifest, meta)
    cache = os.path.join(OUT, "_oracle.json")
    if a.reuse_oracle and os.path.exists(cache):
        oracle = json.load(open(cache))
    else:
        run.IGS = run.IGS + [DEFS]  # the scenario's definitions, as an IG folder
        run.CORPUS = OUT
        oracle = {}
        for i in range(0, len(paths), 800):
            oracle.update({k: [v[0], v[1], sorted(v[2])] for k, v in run.oracle_batch(paths[i:i + 800]).items()})
        json.dump(oracle, open(cache, "w"))
    rows = judge(probe, paths, meta, oracle)
    triage = json.load(open(TRIAGE)) if os.path.exists(TRIAGE) else {}
    counts = {}
    for r in rows:
        counts[r["kind"]] = counts.get(r["kind"], 0) + 1
    print("gut-path: " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
    ungated = 0
    for r in rows:
        if r["kind"] == "agree":
            continue
        accepted = triage.get(r["file"])
        tag = "triaged" if accepted else "UNTRIAGED"
        if not accepted and r["kind"] != "missed":
            ungated += 1
        print(f"  {r['kind']:11} {tag:9} {r['file']}  expected={r.get('expected')} ours={r.get('ours')} oracle={r.get('oracle')}")
        for d in r.get("oracle_detail", [])[:3]:
            print(f"               oracle: {d}")
        for d in r.get("our_detail", [])[:3]:
            print(f"               ours  : {d}")
        if r.get("missed"):
            print(f"               missed: {r['missed']}")
    json.dump({"oracle": run.JAR_VERSION, "rows": rows}, open(os.path.join(OUT, "evidence.json"), "w"), indent=1)
    print(f"{ungated} untriaged disagreement(s)")
    sys.exit(1 if ungated else 0)


if __name__ == "__main__":
    main()
