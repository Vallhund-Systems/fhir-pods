#!/usr/bin/env python3
"""The HL7 v2 sample-message lane: a pinned public set of synthetic v2
messages through the converter, every produced bundle before the reference
validator.

The messages are Microsoft FHIR-Converter's `data/SampleData/Hl7v2` (MIT),
fetched at a pinned commit into the gitignored `conformance/v2-samples/`
— never copied into the repository. `examples/v2_samples.rs` converts each
one (a message whose structure the maps do not cover is refused, with the
reason) and records our verdict, the rows the maps could not place, the
segments and fields left over, and the conversion time. This lane then runs
validator_cli 6.10.4 once over every bundle and compares, per message, the
verdict and every error-level issue (`reasons.py`'s matcher).

Two things are scored: **parity** — the bundle the converter wrote is judged
the same by both validators — and **yield** — how much of each message
reached FHIR (reported, not gated: a map row the IG does not have is a
finding about the IG, an unplaced segment one about the converter). Exit 1
on any parity difference not named in `v2samples-triage.json`, path-exact.

    JAVA_HOME=<jdk17> python3 conformance/v2samples.py [--reuse-oracle]
"""
import argparse
import collections
import json
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
import reasons  # noqa: E402
import run  # noqa: E402

REPO = "https://github.com/microsoft/FHIR-Converter"
PIN = "70fd328e05019142f616a660cf65c6034baaa3c9"
SUBDIR = "data/SampleData/Hl7v2"
SAMPLES = HERE / "v2-samples"
EVIDENCE = HERE / "corpus-v2samples"
TRIAGE = HERE / "v2samples-triage.json"


def fetch():
    """The sample set at its pin, as a sparse checkout kept beside the messages."""
    clone = SAMPLES / ".repo"
    if not (clone / ".git").exists():
        SAMPLES.mkdir(exist_ok=True)
        subprocess.run(["git", "clone", "-q", "--filter=blob:none", "--no-checkout", REPO, str(clone)], check=True)
        subprocess.run(["git", "-C", str(clone), "sparse-checkout", "set", "--no-cone", f"{SUBDIR}/*"], check=True)
    head = subprocess.run(["git", "-C", str(clone), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    if head != PIN or not (clone / SUBDIR).is_dir():
        subprocess.run(["git", "-C", str(clone), "fetch", "-q", "--depth", "1", "origin", PIN], check=True)
        subprocess.run(["git", "-C", str(clone), "checkout", "-q", "--detach", PIN], check=True)
    files = sorted((clone / SUBDIR).glob("*.hl7"))
    if not files:
        raise ValueError("no sample messages after checkout")
    return files


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reuse-oracle", action="store_true", help="read the last oracle bundle instead of running Java")
    args = parser.parse_args(argv)
    files = fetch()
    out = EVIDENCE / "bundles"
    if out.exists():
        for old in out.glob("*.json"):
            old.unlink()
    out.mkdir(parents=True, exist_ok=True)
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "fhir-pods-core", "--example", "v2_samples"],
                   cwd=ROOT, check=True, env={**os.environ, "CARGO_BUILD_JOBS": "2"})
    subprocess.run([str(ROOT / "target/release/examples/v2_samples"), str(files[0].parent), str(out)],
                   check=True, stdout=subprocess.DEVNULL)
    manifest = json.load(open(out / "v2s-manifest.json"))
    converted = [m for m in manifest if m["converted"]]
    refused = [m for m in manifest if not m["converted"]]
    # the oracle, once, over every bundle
    oracle_bundle = EVIDENCE / "_oracle.bundle.json"
    if args.reuse_oracle and oracle_bundle.exists():
        pass
    else:
        run.CORPUS = str(EVIDENCE)  # where run.oracle_batch writes its bundle
        results = run.oracle_batch([str(out / m["file"]) for m in converted])
        if len(results) != len(converted):
            raise ValueError(f"the oracle judged {len(results)} of {len(converted)} bundles")
    oracle = {}
    for e in json.load(open(oracle_bundle)).get("entry", []):
        o = e.get("resource", {})
        src = next((x.get("valueString") for x in o.get("extension", []) if x.get("url", "").endswith(run.FILE_EXT)), None)
        if src:
            oracle[os.path.basename(src)] = o
    probe = ROOT / "target/release/examples/probe"
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "fhir-pods-core", "--example", "probe"],
                   cwd=ROOT, check=True, env={**os.environ, "CARGO_BUILD_JOBS": "2"})
    triage = json.load(open(TRIAGE)) if TRIAGE.exists() else {}
    rows, ungated = [], []
    for m in converted:
        theirs = oracle.get(m["file"])
        if theirs is None:
            ungated.append({"message": m["message"], "error": "no oracle outcome"})
            continue
        ours = json.loads(subprocess.run([str(probe), "--outcome", str(out / m["file"])], capture_output=True, text=True).stdout)
        oe, te = reasons.issues(ours, ("error", "fatal")), reasons.issues(theirs, ("error", "fatal"))
        ov, jv = ("invalid" if oe else "valid"), ("invalid" if te else "valid")
        missed = [{"path": p, "message": t[:200]} for p, _, t in te if not any(reasons.same(p, q, t, t2) for q, _, t2 in oe)]
        extra = [{"path": p, "message": t[:200]} for p, _, t in oe if not any(reasons.same(p, q, t, t2) for q, _, t2 in te)]
        row = {**m, "ours": ov, "oracle": jv, "missed": missed, "extra": extra}
        t = triage.get(m["message"]) or {}
        if ov != jv or missed or extra:
            covered = (t.get("ours", ov) == ov and t.get("oracle", jv) == jv
                       and all(x["path"] in t.get("missed", []) for x in missed)
                       and all(x["path"] in t.get("extra", []) for x in extra))
            row["review"] = "reviewed" if t and covered else ("stale-review" if t else "unreviewed")
        else:
            row["review"] = "stale-review" if t else "agreement"
        if row["review"] not in ("agreement", "reviewed"):
            ungated.append(row)
        rows.append(row)
    agree = sum(1 for r in rows if r["ours"] == r["oracle"])
    exact = sum(1 for r in rows if r["ours"] == r["oracle"] and not r["missed"] and not r["extra"])
    # yield: what the converter left behind, by family
    families = collections.Counter()
    for m in converted:
        for s in m["skipped"]:
            families["skipped: " + s.split(": ", 1)[-1][:70]] += 1
    refused_by = collections.Counter(m["structure"] for m in refused)
    ours_errors = collections.Counter()
    for m in converted:
        for e in m["errors"]:
            ours_errors[reasons.family(e.split(": ", 1)[-1])] += 1
    evidence = {"samples": REPO, "samples_commit": PIN, "subdir": SUBDIR, "oracle": run.JAR_VERSION,
                "summary": {"messages": len(manifest), "converted": len(converted), "refused": len(refused),
                            "verdicts_agree": agree, "errors_agree": exact, "valid_ours": sum(1 for m in converted if m["expect"]),
                            "micros_total": sum(m["micros"] for m in manifest), "ungated": len(ungated)},
                "refused_by_structure": dict(refused_by.most_common()),
                "our_error_families": dict(ours_errors.most_common()),
                "skipped_families": dict(families.most_common(40)),
                "results": rows, "refused": refused}
    json.dump(evidence, open(EVIDENCE / "evidence.json", "w"), indent=2)
    print(f"v2 samples: {len(manifest)} messages, {len(converted)} converted, {len(refused)} refused; "
          f"verdicts agree {agree}/{len(rows)}; every error agrees {exact}/{len(rows)}; "
          f"{evidence['summary']['valid_ours']} bundles valid; {evidence['summary']['micros_total'] / 1000:.0f} ms to convert all")
    print("  refused structures:", ", ".join(f"{k or '(none)'}×{v}" for k, v in refused_by.most_common()))
    print("  our error families on the bundles:")
    for k, v in ours_errors.most_common(12):
        print(f"    {v:4}  {k}")
    for r in rows:
        if r["review"] != "agreement":
            print(f"  {r['message']}: ours {r['ours']} / oracle {r['oracle']} [{r['review']}]")
            for x in r["missed"]:
                print(f"      missed  {x['path']}: {x['message'][:110]}")
            for x in r["extra"]:
                print(f"      extra   {x['path']}: {x['message'][:110]}")
    return int(bool(ungated))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as exc:
        sys.exit(f"v2samples: {exc}")
