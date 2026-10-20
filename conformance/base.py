#!/usr/bin/env python3
"""Regression gate over the pinned R4 published JSON examples.

Compare the checks the pod actually performs with validator_cli, with explicit
reviewed differences. A new or changed difference (including an improvement),
changed corpus, execution failure or unusable evidence fails the gate. Full
outcomes and caches stay in corpus-base/; committed triage contains metadata.

A few oracle outcomes depend on validator_cli's own identity-hash iteration
order over the base structures (see `key_partners`): those cases accept
exactly the outcomes that mechanism allows and are never scored. A fresh run
reports an outcome that moved since the previous one as oracle drift (exit 2);
exit 1 is the pod or the evidence.
"""
import argparse
from collections import Counter
import json
import os
import re
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run
from testcases import digest, json_hash, java_verdict, native

HERE = Path(run.HERE)
R4_CORE = HERE / "package-r4" / "package"
SRC_PKG = ("hl7.fhir.r4.examples", "4.0.1", "package-r4-examples")
PACKAGE = f"{SRC_PKG[0]}#{SRC_PKG[1]}"
SRC = HERE / SRC_PKG[2] / "package"
BASE = HERE / "corpus-base"
TRIAGE = HERE / "base-triage.json"
MAX_BYTES = 1_000_000
CHUNK = 600
# One JVM batch's bound. The default suits a quiet machine (a batch loads its
# packages in under a minute and validates in a few); on a loaded one,
# package loading alone has been measured at 18 minutes, so it can be raised.
BATCH_TIMEOUT = int(os.environ.get("BASE_ORACLE_TIMEOUT", "900"))


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


# The guides the oracle loads for this lane: the base examples declare no
# regional profile, so the list is pinned here rather than following
# `run.IGS` (which grows with `export`'s regions) — a change to it would
# stale the recorded oracle verdicts and triage, by design.
BASE_IGS = [
    "hl7.fhir.au.core#2.0.0", "hl7.fhir.us.core#6.1.0", "hl7.fhir.uv.ipa#1.1.0", "hl7.fhir.uv.ips#2.0.1",
    "hl7.fhir.au.ps#1.0.0", "ihe.iti.balp#1.1.4", "hl7.fhir.au.base#6.0.0",
]


def configuration():
    return {"package": PACKAGE, "jar_version": run.JAR_VERSION,
            "jar_sha256": digest(Path(run.JAR).read_bytes()), "igs": BASE_IGS,
            "fhir_version": "4.0", "terminology": "n/a", "allow_example_urls": True}


def check_inventory(triage, inputs, config):
    if triage.get("format") != 1 or triage.get("configuration") != config:
        raise ValueError("triage configuration changed; review the package/JAR/IG pins")
    if triage.get("inventory") != {"count": len(inputs), "sha256": json_hash(inputs)}:
        raise ValueError("published-example inventory changed; review input bytes and selection")
    for group in ("differences", "oracle_exclusions", "oracle_order_dependent"):
        for name, entry in triage.get(group, {}).items():
            if entry.get("input_sha256") != inputs.get(name) or not entry.get("reasons"):
                raise ValueError(f"invalid {group} entry: {name}")
            if any(reason not in triage["reasons"] for reason in entry["reasons"]):
                raise ValueError(f"unknown triage reason: {name}")
    groups = [set(triage.get(g, {})) for g in ("differences", "oracle_exclusions", "oracle_order_dependent")]
    if any(groups[i] & groups[j] for i in range(3) for j in range(i + 1, 3)):
        raise ValueError("a case can be in only one of differences, oracle exclusions and order-dependent oracle cases")


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
            *[arg for ig in BASE_IGS for arg in ("-ig", ig)], "-tx", "n/a",
            "-allow-example-urls", "true", "-output", str(output)]
    proc = subprocess.run(args, capture_output=True, text=True, timeout=BATCH_TIMEOUT)
    (BASE / f"oracle-{batch}.log").write_text(proc.stdout + proc.stderr)
    if proc.returncode not in (0, 1):
        raise ValueError(f"oracle batch {batch} execution failed: exit {proc.returncode}")
    return read_oracle(json.loads(output.read_bytes()), {p.name for p in paths})


def _constraints(data):
    for part in ("snapshot", "differential"):
        for element in (data.get(part) or {}).get("element", []):
            for constraint in element.get("constraint") or []:
                if constraint.get("key") and constraint.get("expression"):
                    yield constraint["key"], constraint["expression"]


def base_structure_keys(core=R4_CORE):
    """Every constraint key the R4 base structures carry: key -> {url: expression}."""
    keys = {}
    for path in sorted(core.glob("StructureDefinition-*.json")):
        data = json.loads(path.read_bytes())
        if data.get("resourceType") == "StructureDefinition" and data.get("derivation") == "specialization":
            for key, expression in _constraints(data):
                keys.setdefault(key, {})[data["url"]] = expression
    return keys


def key_partners(path, base_keys):
    """The base structures validator_cli 6.10.4 may name for this definition.

    `StructureDefinitionValidator.findInvariantInBase` looks in the
    definition's own base, then walks `ContextUtilities.allBaseStructures()`
    and takes the FIRST base structure carrying the key; a different
    expression there is `ED_INVARIANT_KEY_ALREADY_USED` ("The constraint key
    'k' already exists at the location 'url|4.0.1' …"). That iteration is
    over a `java.util.HashSet` of resource proxies that do not override
    hashCode/equals, so its order is the JVM's identity-hash order and varies
    per run (judging one definition per JVM changes nothing). The outcome is
    therefore: no such error, or one naming any base structure other than the
    definition itself that carries the key with another expression — derived
    here, not enumerated from what runs happened to show."""
    data = json.loads(path.read_bytes())
    own = data.get("url")
    return {(key, url) for key, expression in _constraints(data)
            for url, other in base_keys.get(key, {}).items() if url != own and other != expression}


KEY_USED = re.compile(r"The constraint key '([^']+)' already exists at the location '([^'|]+)(?:\|[^']*)?' with a different expression")


def order_dependent_outcome_ok(errors, partners, allowed=()):
    """Every error is the key-reuse finding naming a derived partner, or one
    of the case's own recorded, order-independent errors."""
    partners = {tuple(p) for p in partners}
    for text in errors:
        m = KEY_USED.match(text)
        if m and (m.group(1), m.group(2)) in partners:
            continue
        if any(text.startswith(prefix) for prefix in allowed):
            continue
        return False
    return True


def outcome_identity(outcome):
    """An outcome's hash without the oracle's source-file extension (an
    absolute path), so two checkouts' outcomes compare."""
    if isinstance(outcome, dict):
        outcome = {**outcome, "extension": [e for e in outcome.get("extension", [])
                                            if not (isinstance(e, dict) and str(e.get("url", "")).endswith(run.FILE_EXT))]}
    return json_hash(outcome)


def oracle_errors(outcome):
    return [((i.get("details") or {}).get("text") or i.get("diagnostics") or "")
            for i in outcome.get("issue", []) if isinstance(i, dict) and i.get("severity") in ("error", "fatal")]


def oracle_drift(previous, config, oracle):
    """Outcomes that differ from the previous full oracle cache (same configuration)."""
    if not isinstance(previous, dict) or previous.get("configuration") != config:
        return None  # no comparable previous run
    drift = {}
    for name, outcome in oracle.items():
        row = (previous.get("results") or {}).get(name)
        if isinstance(row, dict) and outcome_identity(row.get("outcome")) != outcome_identity(outcome):
            try:
                before = java_verdict(row.get("outcome"))[0]
            except (TypeError, ValueError, AttributeError):
                before = None
            drift[name] = {"previous_outcome_identity": outcome_identity(row.get("outcome")),
                           "outcome_identity": outcome_identity(outcome), "previous_oracle": before}
    return drift


def drift_only(row, error, triage, drift):
    """An assessment failure caused by the oracle moving, not by the pod: the
    oracle's outcome for the file changed since the previous run, and the pod
    still says what was reviewed (or what the oracle said before)."""
    moved = (drift or {}).get(row["file"])
    if not moved or row["ours"] not in ("valid", "invalid"):
        return False
    expected = triage["differences"].get(row["file"])
    if expected:
        return expected.get("ours") == row["ours"] and error.startswith("reviewed difference changed (oracle")
    return error == "new verdict disagreement" and moved["previous_oracle"] == row["ours"]


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
    order = triage.get("oracle_order_dependent", {}).get(name)
    if order:
        # the oracle's own iteration order decides the verdict: any outcome
        # the mechanism allows is accepted, and the case is never scored
        if row["input_sha256"] != order["input_sha256"] or row["ours"] != order["ours"]:
            return "changed order-dependent oracle case"
        if not order_dependent_outcome_ok(row.get("oracle_errors", []), row.get("key_partners", set()),
                                          order.get("allowed_errors", [])):
            return "order-dependent oracle case gave an outcome its key partners do not explain"
        return None
    expected = triage["differences"].get(name)
    if expected:
        for key in ("input_sha256", "ours", "oracle"):
            if row.get(key) != expected.get(key):
                return f"reviewed difference changed ({key}); review before updating triage"
        if "order_dependent" in expected:
            # the verdict is stable, the message names an order-dependent partner
            if not order_dependent_outcome_ok(row.get("oracle_errors", []), row.get("key_partners", set()),
                                              expected["order_dependent"].get("allowed_errors", [])):
                return "reviewed difference changed (oracle errors); review before updating triage"
        elif row.get("oracle_signature") != expected.get("oracle_signature"):
            return "reviewed difference changed (oracle_signature); review before updating triage"
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
    evidence = {"format": 1, "package": PACKAGE, "selection": vars(args), "results": [], "errors": [], "drift_errors": []}
    drift = None
    triage = {}
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
            previous_path = BASE / "oracle-cache.json"
            if not partial and previous_path.exists():
                drift = oracle_drift(json.loads(previous_path.read_bytes()), config, oracle)
            cache_path.write_text(json.dumps({"format": 1, "configuration": config,
                "results": {name: {"input_sha256": inputs[name], "outcome_sha256": json_hash(outcome),
                                   "outcome": outcome} for name, outcome in oracle.items()}}, indent=1) + "\n")
            evidence["oracle_cache_sha256"] = digest(cache_path.read_bytes())
        evidence["oracle_drift"] = drift
        base_keys = base_structure_keys()
        order_cases = {name: key_partners(SRC / name, base_keys) for name in
                       set(triage.get("oracle_order_dependent", {})) | {n for n, d in triage["differences"].items()
                                                                        if "order_dependent" in d}}
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
                           oracle_outcome_sha256=outcome_identity(outcome))
                if name in order_cases:
                    row["oracle_errors"] = oracle_errors(outcome)
            if name in order_cases:
                row["key_partners"] = sorted([key, url] for key, url in order_cases[name])
            expected = (triage["differences"].get(name) or triage["oracle_exclusions"].get(name)
                        or triage.get("oracle_order_dependent", {}).get(name))
            if expected:
                row["reasons"] = expected["reasons"]
            try:
                verdict, epaths, result = native(probe, path)
                row.update(ours=verdict, ours_paths=sorted(epaths), ours_outcome_sha256=json_hash(result))
            except (OSError, ValueError, subprocess.TimeoutExpired) as error:
                row["native_error"] = str(error)
            row["agree"] = row["ours"] == row["oracle"] and row["ours"] in ("valid", "invalid")
            error = assess(row, triage)
            if error and drift_only(row, error, triage, drift):
                evidence["drift_errors"].append({"file": name, "error": error, **drift[name]})
            elif error:
                evidence["errors"].append({"file": name, "error": error})
            evidence["results"].append(row)
            if (i + 1) % 500 == 0:
                print(f"native: {i + 1}/{len(paths)}", flush=True)
        if {p.name: digest(p.read_bytes()) for p in all_paths} != inputs:
            raise ValueError("input bytes changed during the run")
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        evidence["errors"].append({"error": str(error)})
    rows = evidence["results"]
    order_names = set(triage.get("oracle_order_dependent", {})) if isinstance(triage, dict) else set()
    scored_rows = [r for r in rows if r["file"] not in order_names]
    kinds = Counter((r["ours"], r["oracle"]) for r in scored_rows)
    evidence["summary"] = {"selected": len(rows), "compared": sum(r["oracle"] in ("valid", "invalid") for r in scored_rows),
        "agree": sum(r["agree"] for r in scored_rows), "oracle_excluded": sum(r["oracle"] == "excluded" for r in rows),
        "oracle_order_dependent": len(rows) - len(scored_rows),
        "lenient": kinds[("valid", "invalid")], "stricter": kinds[("invalid", "valid")], "failures": len(evidence["errors"]),
        "oracle_drift": len(drift) if drift is not None else None, "drift_failures": len(evidence["drift_errors"])}
    evidence_path.write_text(json.dumps(evidence, indent=1) + "\n")
    print(json.dumps(evidence["summary"]))
    for error in evidence["errors"][:30]:
        print(error, file=sys.stderr)
    for error in evidence["drift_errors"][:30]:
        print("oracle drift:", error, file=sys.stderr)
    # 1: the pod or the evidence; 2: only the oracle moved since the previous
    # fresh run (review it; `--reuse-oracle` stays the pod-side gate)
    return 1 if evidence["errors"] else (2 if evidence["drift_errors"] else 0)


if __name__ == "__main__":
    sys.exit(main())
