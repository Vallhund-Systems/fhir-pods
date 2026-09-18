#!/usr/bin/env python3
"""The reference validator's own test suite, as a lane.

`FHIR/fhir-test-cases` (Apache-2.0) is the corpus HL7's validator is
developed against: a thousand adversarial instances, each with the Java
validator's expected OperationOutcome checked in (`validator/outcomes/java/`),
and for some the Firely .NET validator's verdict too. This lane runs its R4
JSON cases through us and compares verdicts — against Java, and against
Firely where it speaks, so a difference between two engines is visible as
such and one engine's quirk is not mistaken for the spec.

It runs apart from the release gate: parity, not cleanliness. Cases that
need what the pod does not do — a terminology server, Java-only features
(matchetype, CDS Hooks logical models, signatures), packages we do not
carry, supporting resources resolved by reference — are classified out with
the module or key that says so, not counted as disagreement.

    python3 conformance/testcases.py [--filter SUBSTR] [--module NAME]

The suite is fetched once into a gitignored `conformance/fhir-test-cases/`
(sparse: the `validator/` tree only). Evidence: `conformance/corpus-testcases/`.
"""
import json
import os
import subprocess
import sys

import run

HERE = os.path.dirname(os.path.abspath(__file__))
SUITE = os.path.join(HERE, "fhir-test-cases")
EVIDENCE = os.path.join(HERE, "corpus-testcases")
REPO = "https://github.com/FHIR/fhir-test-cases.git"
# Pinned: the suite moves with the validator; the commit is part of the evidence.
PIN = "master"

# What the pod does not do, by the manifest's own vocabulary. A case in one of
# these is reported under its reason, never scored.
OUT_OF_SCOPE_MODULES = {
    "tx": "needs a terminology server",
    "matchetype": "Java's matchetype feature",
    "cdshooks": "CDS Hooks logical models",
    "dsig": "digital signatures",
    "shc": "SMART Health Cards (JWS)",
    "json5": "JSON5 syntax",
    "xver": "cross-version extensions",
    "api": "the validator's API",
    "security": "the CLI's security checks",
    "package-versioning": "package version resolution",
    "fmt": "format detection",
    "measure": "CQL / Measure logic",
}
OUT_OF_SCOPE_KEYS = {
    "logical": "a logical model, not a resource",
    "use-test": "the validator's own test mode",
    "fetcher": "a resource fetcher (references resolved by I/O)",
    "packages": "IG packages we do not carry",
    "supporting": "supporting resources resolved by reference",
    "tx-dependent": "needs a terminology server",
    "profile": "an ad-hoc profile the registry has not compiled",
    "profiles": "an ad-hoc profile the registry has not compiled",
}
# Packages the registry does carry, so a case naming only these is in scope.
CARRIED = {"hl7.fhir.us.core#6.1.0", "hl7.fhir.au.core#2.0.0", "hl7.fhir.uv.ipa#1.1.0", "hl7.fhir.uv.ips#2.0.1"}


def fetch():
    if os.path.isdir(os.path.join(SUITE, "validator")):
        return
    print(f"==> fetching {REPO} (validator/ only)")
    subprocess.check_call(["git", "clone", "--depth", "1", "--filter=blob:none", "--sparse", "--branch", PIN, REPO, SUITE])
    subprocess.check_call(["git", "-C", SUITE, "sparse-checkout", "set", "validator"])


def java_verdict(path):
    """The Java validator's verdict from its checked-in OperationOutcome."""
    try:
        o = json.load(open(path))
    except (OSError, ValueError):
        return None, set(), []
    issues = o.get("issue", []) if isinstance(o, dict) else []
    errors = [i for i in issues if i.get("severity") in ("error", "fatal")]
    details = [(i.get("details") or {}).get("text", "") for i in errors]
    return ("invalid" if errors else "valid"), run.error_paths(o), details


def firely_verdict(path):
    """Firely's verdict: a list of `[ERROR] …` / `[WARNING] …` lines."""
    try:
        lines = json.load(open(path))
    except (OSError, ValueError):
        return None
    if not isinstance(lines, list):
        return None
    return "invalid" if any(str(l).startswith("[ERROR]") for l in lines) else "valid"


def main(args):
    filt = args[args.index("--filter") + 1] if "--filter" in args else None
    module = args[args.index("--module") + 1] if "--module" in args else None
    fetch()
    commit = subprocess.check_output(["git", "-C", SUITE, "rev-parse", "HEAD"], text=True).strip()
    vdir = os.path.join(SUITE, "validator")
    manifest = json.load(open(os.path.join(vdir, "manifest.json")))["test-cases"]
    subprocess.check_call(["cargo", "build", "-q", "--release", "-p", "fhir-pods-core", "--example", "probe"], cwd=run.ROOT)
    probe = os.path.join(run.ROOT, "target", "release", "examples", "probe")
    os.makedirs(EVIDENCE, exist_ok=True)

    rows, skipped, agree, compared = [], {}, 0, 0
    for t in manifest:
        name, f = t.get("name", ""), str(t.get("file", ""))
        if not str(t.get("version") or "").startswith("4.0") or not f.endswith(".json"):
            continue
        if filt and filt not in name:
            continue
        if module and t.get("module") != module:
            continue
        why = OUT_OF_SCOPE_MODULES.get(t.get("module"))
        if why is None:
            for k, w in OUT_OF_SCOPE_KEYS.items():
                if t.get(k):
                    if k == "packages" and set(t["packages"]) <= CARRIED:
                        continue
                    why = w
                    break
        if why:
            skipped.setdefault(why, []).append(name)
            continue
        path = os.path.join(vdir, f)
        if not os.path.exists(path) or not t.get("java"):
            skipped.setdefault("no file or no Java outcome", []).append(name)
            continue
        jv, jpaths, jdetails = java_verdict(os.path.join(vdir, "outcomes", t["java"]))
        if jv is None:
            skipped.setdefault("no file or no Java outcome", []).append(name)
            continue
        fv = firely_verdict(os.path.join(vdir, "outcomes", t["firely-sdk-current"])) if t.get("firely-sdk-current") else None
        ov, opaths = run.ours(probe, path)
        compared += 1
        same = ov == jv and ov != "error"
        agree += same
        rows.append({
            "name": name, "file": f, "module": t.get("module"), "ours": ov, "java": jv, "firely": fv, "agree": same,
            "ours_paths": sorted(opaths), "java_paths": sorted(jpaths), "java_detail": "; ".join(d for d in jdetails if d)[:600],
            "explanation": t.get("explanation") or t.get("description") or "",
        })

    print(f"\ntest cases: {compared} compared at {commit[:10]}; {agree}/{compared} agree with the Java validator")
    both = [r for r in rows if r["firely"]]
    if both:
        fj = sum(1 for r in both if r["firely"] == r["java"])
        fo = sum(1 for r in both if r["firely"] == r["ours"])
        print(f"   Firely speaks on {len(both)}: agrees with Java on {fj}, with us on {fo}")
    for kind, label in (("invalid", "ours=invalid java=valid"), ("valid", "ours=valid java=invalid")):
        bad = [r for r in rows if not r["agree"] and r["ours"] == kind]
        print(f"  {label}: {len(bad)}")
        for r in bad:
            side = f"  firely={r['firely']}" if r["firely"] else ""
            print(f"    {r['name']:<50} [{r['module']}]{side}  {r['java_detail'][:110]}")
    errs = [r for r in rows if r["ours"] == "error"]
    if errs:
        print(f"  ours=error: {len(errs)}  " + ", ".join(r["name"] for r in errs))
    print("  out of scope, by reason:")
    for why, names in sorted(skipped.items(), key=lambda kv: -len(kv[1])):
        print(f"    {len(names):>4}  {why}")
    with open(os.path.join(EVIDENCE, "evidence.json"), "w") as fh:
        json.dump({"suite": REPO, "commit": commit, "compared": compared, "agree": agree, "results": rows,
                   "out_of_scope": skipped}, fh, indent=1)


if __name__ == "__main__":
    main(sys.argv[1:])
