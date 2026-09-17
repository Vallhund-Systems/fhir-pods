#!/usr/bin/env python3
"""The base-spec lane: every published example of FHIR R4 itself
(`hl7.fhir.r4.examples` 4.0.1, ~5,000 instances across every resource type)
validated as it declares itself — almost all declare no profile, so this is
the widest test there is of the base spec's shape rules, datatype minimums,
`dom-*` / resource invariants and bindings — by us and by the reference
validator, and compared. Parity, not cleanliness: the spec's own examples
are not all valid, and what matters is that both sides say the same. Runs
apart from the release gate (`run.py`); the evidence goes to
conformance/corpus-base/ (gitignored) and the disagreements are the work list.

    JAVA_HOME=/opt/homebrew/opt/openjdk@17 python3 conformance/base.py [--limit N] [--filter SUBSTR] [--reuse-oracle]
"""
import glob
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run  # noqa: E402  (the harness's runners and oracle)

HERE = run.HERE
SRC_PKG = ("hl7.fhir.r4.examples", "4.0.1", "package-r4-examples")
SRC = os.path.join(HERE, SRC_PKG[2], "package")
BASE = os.path.join(HERE, "corpus-base")
MAX_BYTES = 1_000_000  # the spec's own definition bundles (StructureDefinitions, expansions) are not examples
CHUNK = 600


def ensure_examples():
    if os.path.isdir(SRC):
        return
    sys.path.insert(0, HERE)
    import make_corpus
    make_corpus.ensure_package(*SRC_PKG)


def select(limit=None, filt=None):
    out = []
    for path in sorted(glob.glob(os.path.join(SRC, "*.json"))):
        name = os.path.basename(path)
        if name in ("package.json", ".index.json") or os.path.getsize(path) > MAX_BYTES:
            continue
        if filt and filt not in name:
            continue
        try:
            d = json.load(open(path))
        except ValueError:
            continue
        if not isinstance(d, dict) or "resourceType" not in d:
            continue
        out.append(path)
    return out[:limit] if limit else out


def oracle_split(chunk):
    """The oracle over a chunk; when it dies (one file it cannot take), halve
    the chunk and try again, down to the single file that is at fault."""
    got = run.oracle_batch(chunk)
    if got or len(chunk) == 1:
        if not got:
            print(f"   oracle: no verdict for {os.path.basename(chunk[0])}", flush=True)
        return got
    mid = len(chunk) // 2
    out = oracle_split(chunk[:mid])
    out.update(oracle_split(chunk[mid:]))
    return out


def main():
    args = sys.argv[1:]
    limit = int(args[args.index("--limit") + 1]) if "--limit" in args else None
    filt = args[args.index("--filter") + 1] if "--filter" in args else None
    ensure_examples()
    os.makedirs(BASE, exist_ok=True)
    run.CORPUS = BASE  # the oracle's output bundle lands beside the evidence
    paths = select(limit, filt)
    print(f"base lane: {len(paths)} examples from {SRC_PKG[0]}#{SRC_PKG[1]}")
    subprocess.check_call(["cargo", "build", "-q", "--release", "-p", "fhir-pods-core", "--example", "probe"], cwd=run.ROOT)
    probe = os.path.join(run.ROOT, "target", "release", "examples", "probe")
    ours = {}
    for i, p in enumerate(paths):
        v, epaths = run.ours(probe, p)
        ours[os.path.basename(p)] = (v, sorted(epaths))
        if (i + 1) % 500 == 0:
            print(f"   ours: {i + 1}/{len(paths)}", flush=True)
    oracle = {}
    prior = os.path.join(BASE, "evidence.json")
    if "--reuse-oracle" in args and os.path.exists(prior):
        # the oracle's verdicts do not move between our own iterations
        for r in json.load(open(prior))["results"]:
            if r["oracle"] == "error":
                continue  # asked again
            oracle[r["file"]] = (r["oracle"], [r["oracle_detail"]] if r["oracle_detail"] else [], set(r["oracle_paths"]))
        print(f"   oracle: {len(oracle)} verdicts reused from {prior}")
    for i in range(0, len(paths), CHUNK):
        chunk = [p for p in paths[i:i + CHUNK] if os.path.basename(p) not in oracle]
        if not chunk:
            continue
        print(f"   oracle: {i + 1}..{i + len(chunk)} of {len(paths)}", flush=True)
        oracle.update(oracle_split(chunk))
    rows = []
    agree = 0
    for p in paths:
        name = os.path.basename(p)
        ov, opaths = ours[name]
        rv, rdetail, rpaths = oracle.get(name, ("error", "no oracle verdict", set()))
        same = (ov == rv) and ov != "error"
        agree += same
        rows.append({"file": name, "ours": ov, "oracle": rv, "agree": same, "ours_paths": list(opaths),
                     "oracle_paths": sorted(rpaths), "oracle_detail": "; ".join(rdetail)})
    with open(os.path.join(BASE, "evidence.json"), "w") as fh:
        json.dump({"package": f"{SRC_PKG[0]}#{SRC_PKG[1]}", "oracle": run.JAR_VERSION, "compared": len(paths),
                   "agree": agree, "results": rows}, fh, indent=1)
    from collections import Counter
    kinds = Counter((r["ours"], r["oracle"]) for r in rows if not r["agree"])
    print(f"\n{agree}/{len(paths)} agree")
    for (o, r), n in kinds.most_common():
        print(f"  ours={o} oracle={r}: {n}")
    for r in [r for r in rows if not r["agree"]][:60]:
        print(f"  {r['file']:<60} ours={r['ours']:<8} oracle={r['oracle']:<8} {r['oracle_detail'][:110]}")


if __name__ == "__main__":
    main()
