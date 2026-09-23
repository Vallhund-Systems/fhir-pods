#!/usr/bin/env python3
"""The Synthea lane: a seeded cohort of synthetic patients — every one a
transaction Bundle of hundreds of US Core 6.1 resources — validated by the
pod and by validator_cli, compared on the verdict and on every error-level
issue, with the pod's time per bundle recorded.

Synthea (Apache-2.0) generates the cohort at run time into the gitignored
`conformance/synthea/` from a pinned release jar (sha256-checked): nothing
generated is committed, and nothing in it is a real person. The cohort is
`-p N -s SEED` (plus the hospital and practitioner bundles), so it is the
same on every machine running the same jar.

Scored: **parity** (both validators agree per bundle, and on each
error-level issue); **scale** (the pod's time per bundle and per MB, and
the largest bundle it took). Exit 1 on a parity difference not named in
`synthea-triage.json`, path-exact.

    JAVA_HOME=<jdk17> python3 conformance/synthea.py [--patients N] [--reuse-oracle]
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
import reasons  # noqa: E402
import run  # noqa: E402

VERSION = "v4.0.0"
JAR_URL = f"https://github.com/synthetichealth/synthea/releases/download/{VERSION}/synthea-with-dependencies.jar"
JAR_SHA256 = "ed43c20ad40ba5c3bc724503a5af032715fe3c491620b766148e7c2361e6ecc1"
SEED = 20260923
STATE = "Massachusetts"
CACHE = HERE / "synthea"
EVIDENCE = HERE / "corpus-synthea"
TRIAGE = HERE / "synthea-triage.json"


def java():
    home = os.environ.get("JAVA_HOME", "/opt/homebrew/opt/openjdk@17")
    return os.environ.get("JAVA") or str(Path(home) / "bin/java")


def ensure_jar():
    CACHE.mkdir(exist_ok=True)
    jar = CACHE / "synthea-with-dependencies.jar"
    if not jar.is_file() or hashlib.sha256(jar.read_bytes()).hexdigest() != JAR_SHA256:
        print(f"  fetching Synthea {VERSION}…", flush=True)
        urllib.request.urlretrieve(JAR_URL, jar)
        if hashlib.sha256(jar.read_bytes()).hexdigest() != JAR_SHA256:
            raise ValueError("Synthea jar does not match its pinned sha256")
    return jar


def generate(jar, patients):
    out = CACHE / f"cohort-{patients}-{SEED}"
    fhir = out / "fhir"
    if fhir.is_dir() and len(list(fhir.glob("*.json"))) >= patients:
        return fhir
    cmd = [java(), "-jar", str(jar), "-p", str(patients), "-s", str(SEED), f"--exporter.baseDirectory={out}",
           "--exporter.fhir.export=true", "--exporter.hospital.fhir.export=true",
           "--exporter.practitioner.fhir.export=true", "--exporter.years_of_history=10", STATE]
    print(f"  generating {patients} patients (seed {SEED}, {STATE})…", flush=True)
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT, cwd=CACHE)
    return fhir


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--patients", type=int, default=10)
    parser.add_argument("--reuse-oracle", action="store_true")
    args = parser.parse_args(argv)
    jar = ensure_jar()
    fhir = generate(jar, args.patients)
    files = sorted(fhir.glob("*.json"))
    EVIDENCE.mkdir(exist_ok=True)
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "fhir-pods-core", "--example", "probe"],
                   cwd=ROOT, check=True, env={**os.environ, "CARGO_BUILD_JOBS": "2"})
    probe = ROOT / "target/release/examples/probe"
    ours = {}
    for f in files:
        started = time.perf_counter()
        proc = subprocess.run([str(probe), "--outcome", str(f)], capture_output=True, text=True)
        elapsed = time.perf_counter() - started
        o = json.loads(proc.stdout)
        b = json.load(open(f))
        ours[f.name] = {"outcome": o, "seconds": elapsed, "bytes": f.stat().st_size,
                        "entries": len(b.get("entry", [])), "type": b.get("type")}
    oracle_bundle = EVIDENCE / "_oracle.bundle.json"
    if not (args.reuse_oracle and oracle_bundle.exists()):
        run.CORPUS = str(EVIDENCE)
        print(f"  oracle over {len(files)} bundles ({sum(v['bytes'] for v in ours.values()) / 1e6:.0f} MB)…", flush=True)
        results = run.oracle_batch([str(f) for f in files])
        if len(results) != len(files):
            raise ValueError(f"the oracle judged {len(results)} of {len(files)} bundles")
    oracle = {}
    for e in json.load(open(oracle_bundle)).get("entry", []):
        o = e.get("resource", {})
        src = next((x.get("valueString") for x in o.get("extension", []) if x.get("url", "").endswith(run.FILE_EXT)), None)
        if src:
            oracle[os.path.basename(src)] = o
    triage = json.load(open(TRIAGE)) if TRIAGE.exists() else {}
    rows, ungated = [], []
    for name, mine in ours.items():
        theirs = oracle.get(name)
        if theirs is None:
            ungated.append({"file": name, "error": "no oracle outcome"})
            continue
        oe, te = reasons.issues(mine["outcome"], ("error", "fatal")), reasons.issues(theirs, ("error", "fatal"))
        ov, jv = ("invalid" if oe else "valid"), ("invalid" if te else "valid")
        missed = [{"path": p, "message": t[:200]} for p, _, t in te if not any(reasons.same(p, q, t, t2) for q, _, t2 in oe)]
        extra = [{"path": p, "message": t[:200]} for p, _, t in oe if not any(reasons.same(p, q, t, t2) for q, _, t2 in te)]
        # the cohort is regenerated per machine; triage keys by kind of bundle
        key = "hospital" if name.startswith("hospitalInformation") else "practitioner" if name.startswith("practitionerInformation") else "patient"
        t = triage.get(key) or {}
        row = {"file": name, "kind": key, "type": mine["type"], "entries": mine["entries"], "bytes": mine["bytes"],
               "seconds": round(mine["seconds"], 3), "ours": ov, "oracle": jv, "missed": missed, "extra": extra}
        if ov != jv or missed or extra:
            covered = (t.get("ours", ov) == ov and t.get("oracle", jv) == jv
                       and all(x["path"] in t.get("missed", []) for x in missed)
                       and all(x["path"] in t.get("extra", []) for x in extra))
            row["review"] = "reviewed" if t and covered else ("stale-review" if t else "unreviewed")
        else:
            row["review"] = "agreement"
        if row["review"] not in ("agreement", "reviewed"):
            ungated.append(row)
        rows.append(row)
    agree = sum(1 for r in rows if r["ours"] == r["oracle"])
    exact = sum(1 for r in rows if r["ours"] == r["oracle"] and not r["missed"] and not r["extra"])
    total_bytes = sum(r["bytes"] for r in rows)
    total_s = sum(r["seconds"] for r in rows)
    biggest = max(rows, key=lambda r: r["bytes"]) if rows else None
    evidence = {"synthea": VERSION, "jar_sha256": JAR_SHA256, "seed": SEED, "state": STATE, "patients": args.patients,
                "oracle": run.JAR_VERSION,
                "summary": {"bundles": len(rows), "verdicts_agree": agree, "errors_agree": exact,
                            "valid_both": sum(1 for r in rows if r["ours"] == "valid" and r["oracle"] == "valid"),
                            "bytes": total_bytes, "entries": sum(r["entries"] for r in rows), "seconds_ours": round(total_s, 3),
                            "mb_per_second": round(total_bytes / 1e6 / total_s, 1) if total_s else None, "ungated": len(ungated)},
                "results": rows}
    json.dump(evidence, open(EVIDENCE / "evidence.json", "w"), indent=2)
    print(f"synthea: {len(rows)} bundles ({total_bytes / 1e6:.1f} MB, {evidence['summary']['entries']} entries); "
          f"verdicts agree {agree}/{len(rows)}; every error agrees {exact}/{len(rows)}; "
          f"valid on both {evidence['summary']['valid_both']}; ours {total_s:.2f} s in all "
          f"({evidence['summary']['mb_per_second']} MB/s"
          + (f"; largest {biggest['bytes'] / 1e6:.1f} MB in {biggest['seconds']:.2f} s)" if biggest else ")"))
    for r in rows:
        if r["review"] != "agreement":
            print(f"  {r['file']}: ours {r['ours']} / oracle {r['oracle']} [{r['review']}]")
            for x in r["missed"][:8]:
                print(f"      missed  {x['path']}: {x['message'][:110]}")
            for x in r["extra"][:8]:
                print(f"      extra   {x['path']}: {x['message'][:110]}")
    return int(bool(ungated))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as exc:
        sys.exit(f"synthea: {exc}")
