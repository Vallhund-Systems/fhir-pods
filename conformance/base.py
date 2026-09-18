#!/usr/bin/env python3
"""Regression gate over the pinned R4 published JSON examples.

Compare the checks the pod actually performs with validator_cli, with explicit
reviewed differences. A new or changed difference (including an improvement),
changed corpus, execution failure or unusable evidence fails the gate. Full
outcomes and caches stay in corpus-base/; committed triage contains metadata.
"""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run
from testcases import digest, json_hash, java_verdict, native

HERE = Path(run.HERE)
SRC_PKG = ("hl7.fhir.r4.examples", "4.0.1", "package-r4-examples")
PACKAGE = f"{SRC_PKG[0]}#{SRC_PKG[1]}"
SRC = HERE / SRC_PKG[2] / "package"
BASE = HERE / "corpus-base"
TRIAGE = HERE / "base-triage.json"
MAX_BYTES = 1_000_000
CHUNK = 600


def ensure_examples():
    if not SRC.is_dir():
        import make_corpus
        make_corpus.ensure_package(*SRC_PKG)


def select(limit=None, filt=None):
    out = []
    for path in sorted(SRC.glob("*.json")):
        if path.name in ("package.json", ".index.json") or path.stat().st_size > MAX_BYTES:
            continue
        data = json.loads(path.read_bytes())
        if not isinstance(data, dict) or "resourceType" not in data:
            continue
        if not filt or filt in path.name:
            out.append(path)
    return out[:limit] if limit else out


def configuration():
    return {"package": PACKAGE, "jar_version": run.JAR_VERSION,
            "jar_sha256": digest(Path(run.JAR).read_bytes()), "igs": run.IGS,
            "fhir_version": "4.0", "terminology": "n/a", "allow_example_urls": True}


def check_inventory(triage, inputs, config):
    if triage.get("format") != 1 or triage.get("configuration") != config:
        raise ValueError("triage configuration changed; review the package/JAR/IG pins")
    if triage.get("inventory") != {"count": len(inputs), "sha256": json_hash(inputs)}:
        raise ValueError("published-example inventory changed; review input bytes and selection")
    for group in ("differences", "oracle_exclusions"):
        for name, entry in triage[group].items():
            if entry.get("input_sha256") != inputs.get(name) or not entry.get("reasons"):
                raise ValueError(f"invalid {group} entry: {name}")
            if any(reason not in triage["reasons"] for reason in entry["reasons"]):
                raise ValueError(f"unknown triage reason: {name}")
    if triage["differences"].keys() & triage["oracle_exclusions"].keys():
        raise ValueError("a case cannot be both a difference and an oracle exclusion")


def read_oracle(bundle, requested):
    if not isinstance(bundle, dict):
        raise ValueError("oracle output is not an object")
    if bundle.get("resourceType") == "Bundle":
        entries = bundle.get("entry")
        if not isinstance(entries, list) or any(not isinstance(e, dict) for e in entries):
            raise ValueError("malformed oracle Bundle entries")
        outcomes = [e.get("resource") for e in entries]
    else:
        outcomes = [bundle]
    results = {}
    for outcome in outcomes:
        java_verdict(outcome)  # malformed severities are evidence failures
        extensions = outcome.get("extension", [])
        if not isinstance(extensions, list) or any(not isinstance(e, dict) for e in extensions):
            raise ValueError("malformed oracle source extensions")
        sources = [e.get("valueString") for e in extensions
                   if isinstance(e.get("url"), str) and e["url"].endswith(run.FILE_EXT)]
        if len(sources) == 1 and isinstance(sources[0], str):
            name = Path(sources[0]).name
        elif not sources and len(requested) == 1:
            name = next(iter(requested))
        else:
            raise ValueError("missing or ambiguous oracle source")
        if name not in requested or name in results:
            raise ValueError(f"unexpected or duplicate oracle result: {name}")
        results[name] = outcome
    if results.keys() != requested:
        raise ValueError(f"missing oracle outcomes: {sorted(requested - results.keys())}")
    return results


def oracle_batch(paths, batch):
    output = BASE / f"oracle-{batch}.json"
    output.unlink(missing_ok=True)
    args = [run.JAVA, "-Xmx6g", "-jar", run.JAR, *map(str, paths), "-version", "4.0",
            *[arg for ig in run.IGS for arg in ("-ig", ig)], "-tx", "n/a",
            "-allow-example-urls", "true", "-output", str(output)]
    proc = subprocess.run(args, capture_output=True, text=True, timeout=900)
    (BASE / f"oracle-{batch}.log").write_text(proc.stdout + proc.stderr)
    if proc.returncode not in (0, 1):
        raise ValueError(f"oracle batch {batch} execution failed: exit {proc.returncode}")
    return read_oracle(json.loads(output.read_bytes()), {p.name for p in paths})


def cached_outcomes(cache, config, inputs):
    if not isinstance(cache, dict) or cache.get("format") != 1 or cache.get("configuration") != config:
        raise ValueError("oracle cache is absent/stale; run without --reuse-oracle")
    entries = cache.get("results")
    if not isinstance(entries, dict):
        raise ValueError("malformed oracle cache results")
    found = {}
    for name, sha in inputs.items():
        row = entries.get(name)
        if not isinstance(row, dict) or row.get("input_sha256") != sha:
            raise ValueError(f"missing/changed cached input {name}; run without --reuse-oracle")
        outcome = row.get("outcome")
        java_verdict(outcome)
        if row.get("outcome_sha256") != json_hash(outcome):
            raise ValueError(f"changed cached oracle outcome: {name}")
        found[name] = outcome
    return found


def oracle_signature(outcome):
    verdict, detail, paths = run.summarize(outcome)
    return json_hash({"oracle": verdict, "oracle_detail": "; ".join(detail), "oracle_paths": sorted(paths)})


def assess(row, triage):
    name = row["file"]
    if row["ours"] == "error":
        return "native execution/evidence failure"
    exclusion = triage["oracle_exclusions"].get(name)
    if exclusion:
        if row["input_sha256"] != exclusion["input_sha256"] or row["ours"] != exclusion["ours"]:
            return "changed oracle-excluded case"
        if row["oracle"] != "excluded":
            return "oracle exclusion needs review"
        return None
    if row["oracle"] not in ("valid", "invalid"):
        return "oracle execution/evidence failure"
    expected = triage["differences"].get(name)
    if expected:
        for key in ("input_sha256", "ours", "oracle", "oracle_signature"):
            if row.get(key) != expected.get(key):
                return f"reviewed difference changed ({key}); review before updating triage"
    elif row["ours"] != row["oracle"]:
        return "new verdict disagreement"
    return None


def positive_int(text):
    n = int(text)
    if n < 1:
        raise argparse.ArgumentTypeError("limit must be positive")
    return n


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--limit", type=positive_int)
    parser.add_argument("--filter")
    parser.add_argument("--reuse-oracle", action="store_true")
    args = parser.parse_args(argv)
    BASE.mkdir(exist_ok=True)
    partial = args.limit is not None or args.filter is not None
    evidence_path = BASE / ("evidence-subset.json" if partial else "evidence.json")
    evidence = {"format": 1, "package": PACKAGE, "selection": vars(args), "results": [], "errors": []}
    try:
        ensure_examples()
        all_paths = select()
        inputs = {p.name: digest(p.read_bytes()) for p in all_paths}
        config = configuration()
        triage = json.loads(TRIAGE.read_bytes())
        check_inventory(triage, inputs, config)
        paths = select(args.limit, args.filter)
        if not paths:
            raise ValueError("selection matched no examples")
        evidence.update(configuration=config, inventory=triage["inventory"], triage_sha256=digest(TRIAGE.read_bytes()),
                        source_commit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=run.ROOT, text=True).strip(),
                        source_dirty=bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=run.ROOT)))
        scored = [p for p in paths if p.name not in triage["oracle_exclusions"]]
        cache_path = BASE / ("oracle-cache-subset.json" if partial else "oracle-cache.json")
        if args.reuse_oracle:
            # Subsets reuse the full cache, never replace it with partial evidence.
            cache = json.loads((BASE / "oracle-cache.json").read_bytes())
            oracle = cached_outcomes(cache, config, {p.name: inputs[p.name] for p in scored})
            evidence["oracle_cache_sha256"] = digest((BASE / "oracle-cache.json").read_bytes())
        else:
            banner = subprocess.run([run.JAVA, "-jar", run.JAR], capture_output=True, text=True, timeout=60)
            if f"Version {run.JAR_VERSION}" not in banner.stdout + banner.stderr:
                raise ValueError("oracle version banner does not match the pinned JAR")
            oracle = {}
            for i in range(0, len(scored), CHUNK):
                chunk = scored[i:i + CHUNK]
                print(f"oracle: {i + 1}..{i + len(chunk)} / {len(scored)}", flush=True)
                oracle.update(oracle_batch(chunk, i // CHUNK))
            cache_path.write_text(json.dumps({"format": 1, "configuration": config,
                "results": {name: {"input_sha256": inputs[name], "outcome_sha256": json_hash(outcome),
                                   "outcome": outcome} for name, outcome in oracle.items()}}, indent=1) + "\n")
            evidence["oracle_cache_sha256"] = digest(cache_path.read_bytes())
        target = Path(run.ROOT) / "target" / "base-lane"
        subprocess.run(["cargo", "build", "-q", "--release", "-j", "2", "-p", "fhir-pods-core",
                        "--example", "probe", "--target-dir", str(target)], cwd=run.ROOT, check=True)
        probe = target / "release" / "examples" / "probe"
        evidence["probe_sha256"] = digest(probe.read_bytes())
        for i, path in enumerate(paths):
            name = path.name
            row = {"file": name, "input_sha256": inputs[name], "ours": "error", "oracle": "excluded", "agree": False}
            outcome = oracle.get(name)
            if outcome is not None:
                verdict, epaths = java_verdict(outcome)
                row.update(oracle=verdict, oracle_paths=sorted(epaths), oracle_signature=oracle_signature(outcome),
                           oracle_outcome_sha256=json_hash(outcome))
            expected = triage["differences"].get(name) or triage["oracle_exclusions"].get(name)
            if expected:
                row["reasons"] = expected["reasons"]
            try:
                verdict, epaths, result = native(probe, path)
                row.update(ours=verdict, ours_paths=sorted(epaths), ours_outcome_sha256=json_hash(result))
            except (OSError, ValueError, subprocess.TimeoutExpired) as error:
                row["native_error"] = str(error)
            row["agree"] = row["ours"] == row["oracle"] and row["ours"] in ("valid", "invalid")
            error = assess(row, triage)
            if error:
                evidence["errors"].append({"file": name, "error": error})
            evidence["results"].append(row)
            if (i + 1) % 500 == 0:
                print(f"native: {i + 1}/{len(paths)}", flush=True)
        if {p.name: digest(p.read_bytes()) for p in all_paths} != inputs:
            raise ValueError("input bytes changed during the run")
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        evidence["errors"].append({"error": str(error)})
    rows = evidence["results"]
    kinds = Counter((r["ours"], r["oracle"]) for r in rows)
    evidence["summary"] = {"selected": len(rows), "compared": sum(r["oracle"] in ("valid", "invalid") for r in rows),
        "agree": sum(r["agree"] for r in rows), "oracle_excluded": sum(r["oracle"] == "excluded" for r in rows),
        "lenient": kinds[("valid", "invalid")], "stricter": kinds[("invalid", "valid")], "failures": len(evidence["errors"])}
    evidence_path.write_text(json.dumps(evidence, indent=1) + "\n")
    print(json.dumps(evidence["summary"]))
    for error in evidence["errors"][:30]:
        print(error, file=sys.stderr)
    return 1 if evidence["errors"] else 0


if __name__ == "__main__":
    sys.exit(main())
