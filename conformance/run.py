#!/usr/bin/env python3
"""Differential conformance test.

Runs our validator and the HL7 reference validator (`validator_cli.jar` — the
ecosystem's oracle; "correct" means bug-for-bug agreement with it) over the
corpus and compares verdicts, then flagged elements. The oracle runs once for
the whole corpus (one JVM; it emits a Bundle of OperationOutcomes, each tagged
with its source file). It needs Java 11+ — set JAVA_HOME (or JAVA); our side
runs without it.

    python3 conformance/run.py
"""
import json
import os
import re
import subprocess
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HERE = os.path.join(ROOT, "conformance")
CORPUS = os.path.join(HERE, "corpus")
JAR = os.path.join(HERE, "validator_cli.jar")
# Pinned: the oracle is part of the evidence, so its version is part of the record.
JAR_VERSION = "6.10.4"
JAR_URL = f"https://github.com/hapifhir/org.hl7.fhir.core/releases/download/{JAR_VERSION}/validator_cli.jar"
IGS = ["hl7.fhir.au.core#2.0.0", "hl7.fhir.us.core#6.1.0", "hl7.fhir.uv.ipa#1.1.0", "hl7.fhir.uv.ips#2.0.1", "hl7.fhir.au.ps#1.0.0"]  # keep in step with make_corpus.IGS
FILE_EXT = "operationoutcome-file"


def java_bin():
    """The JVM: $JAVA (an explicit binary), else $JAVA_HOME/bin/java, else PATH."""
    if os.environ.get("JAVA"):
        return os.environ["JAVA"]
    home = os.environ.get("JAVA_HOME")
    if home:
        return os.path.join(home, "bin", "java")
    return "java"


JAVA = java_bin()


def jvm_starts():
    """A real JVM has to start — macOS's /usr/bin/java stub exists but has no runtime."""
    try:
        return subprocess.run([JAVA, "-version"], capture_output=True).returncode == 0
    except OSError:
        return False


def error_paths(outcome):
    """Element paths of error/fatal issues, normalized so ours and the oracle's
    are comparable: strip `[n]` indices and `:slice` labels, rewrite the
    oracle's `.ofType(T)` choice syntax to `[x]`."""
    paths = set()
    for i in outcome.get("issue", []):
        if i.get("severity") not in ("error", "fatal"):
            continue
        for e in i.get("expression") or i.get("location") or []:
            e = re.sub(r"\.ofType\([^)]*\)", "[x]", e)
            e = re.sub(r"\[\d+\]", "", e)
            e = re.sub(r":[^.\[\]]+", "", e)
            paths.add(e)
    return paths


def summarize(outcome):
    bad = [i for i in outcome.get("issue", []) if i.get("severity") in ("error", "fatal")]
    detail = [
        f"{i.get('severity')}: {(i.get('details') or {}).get('text') or i.get('diagnostics')} @ {i.get('expression') or i.get('location')}"
        for i in bad
    ]
    return ("invalid" if bad else "valid"), detail, error_paths(outcome)


def ours(probe, path):
    """Our verdict and our OperationOutcome's error paths (apples to apples).
    A crash, a foreign exit code, or a report that disagrees with the exit
    code is an execution failure ("error"), never a verdict."""
    r = subprocess.run([probe, "--outcome", path], capture_output=True, text=True)
    try:
        o = json.loads(r.stdout)
    except ValueError:
        o = None
    if r.returncode not in (0, 1) or not isinstance(o, dict) or o.get("resourceType") != "OperationOutcome":
        return "error", set()
    has_errors = any(i.get("severity") in ("error", "fatal") for i in o.get("issue", []))
    if has_errors != (r.returncode == 1):
        return "error", set()
    return ("invalid" if has_errors else "valid"), error_paths(o)


def oracle_batch(paths):
    """One JVM for the whole corpus, terminology disabled (-tx n/a) — our same
    offline posture. Returns {file basename: (verdict, detail, error_paths)}."""
    out = os.path.join(CORPUS, "_oracle.bundle.json")
    if os.path.exists(out):
        os.remove(out)  # never read a previous run's evidence
    proc = subprocess.run(
        [JAVA, "-jar", JAR, *paths, "-version", "4.0", *[a for ig in IGS for a in ("-ig", ig)], "-tx", "n/a",
         # A CLI policy lint, not conformance: published IG examples use example.org
         # URLs and are valid by publication. Off, so the oracle scores profiles only.
         "-allow-example-urls", "true", "-output", out],
        capture_output=True, text=True,
    )
    results = {}
    # the validator exits 0 (all valid) or 1 (some invalid); anything else — a
    # signal, a JVM failure — means its output, if any, is not evidence
    if proc.returncode not in (0, 1):
        print(f"!! the oracle did not run to completion (exit {proc.returncode}):\n{proc.stderr[-2000:]}", file=sys.stderr)
        return results
    try:
        b = json.load(open(out))
    except (OSError, ValueError):
        print(f"!! the oracle produced no output (exit {proc.returncode}):\n{proc.stderr[-2000:]}", file=sys.stderr)
        return results
    if b.get("resourceType") == "Bundle":
        entries = [e.get("resource", {}) for e in b.get("entry", [])]
    else:
        entries = [b]
    for o in entries:
        if o.get("resourceType") != "OperationOutcome":
            continue  # not a verdict
        src = next((x.get("valueString") for x in o.get("extension", []) if x.get("url", "").endswith(FILE_EXT)), None)
        key = os.path.basename(src) if src else (os.path.basename(paths[0]) if len(paths) == 1 else None)
        if key:
            results[key] = summarize(o)
    return results


def main():
    subprocess.check_call([sys.executable, os.path.join(HERE, "make_corpus.py")])
    subprocess.check_call(["cargo", "build", "-q", "-p", "fhir-pods-core", "--example", "probe"], cwd=ROOT)
    probe = os.path.join(ROOT, "target", "debug", "examples", "probe")

    have_java = jvm_starts()
    if have_java and not os.path.exists(JAR):
        print("==> fetching validator_cli.jar")
        urllib.request.urlretrieve(JAR_URL, JAR)
    if have_java:
        # the jar on disk must be the pinned version: the oracle is part of the record
        banner = subprocess.run([JAVA, "-jar", JAR], capture_output=True, text=True)
        if f"Version {JAR_VERSION}" not in (banner.stdout + banner.stderr):
            sys.exit(f"!! {JAR} is not validator_cli {JAR_VERSION}; delete it to refetch the pinned version")
    if not have_java:
        print(
            "!! No JVM: the oracle (validator_cli.jar) needs Java 11+ — set JAVA_HOME (or JAVA) to a JDK. "
            "Running OUR side only.\n"
        )

    # The lowering proof: every valid example lowered from its slot values,
    # written back as lowered-*.json for the oracle to judge.
    subprocess.check_call(
        ["cargo", "run", "-q", "-p", "fhir-pods-core", "--example", "lower_corpus", "--", CORPUS],
        cwd=ROOT, stdout=subprocess.DEVNULL,
    )
    subprocess.check_call(
        ["cargo", "run", "-q", "-p", "fhir-pods-core", "--example", "deid_corpus", "--", CORPUS],
        cwd=ROOT, stdout=subprocess.DEVNULL,
    )
    subprocess.check_call(
        ["cargo", "run", "-q", "-p", "fhir-pods-core", "--example", "document_corpus", "--", CORPUS],
        cwd=ROOT, stdout=subprocess.DEVNULL,
    )
    manifest = json.load(open(os.path.join(CORPUS, "manifest.json")))
    manifest += json.load(open(os.path.join(CORPUS, "lowered-manifest.json")))
    manifest += json.load(open(os.path.join(CORPUS, "deid-manifest.json")))
    manifest += json.load(open(os.path.join(CORPUS, "doc-manifest.json")))
    paths = [os.path.join(CORPUS, c["file"]) for c in manifest]
    oracle = oracle_batch(paths) if have_java else {}

    print(f"{'fixture':<58} {'expect':<8} {'ours':<8} {'oracle':<8} status")
    failures = 0
    oracle_errors = 0
    for c, path in zip(manifest, paths):
        expect = "valid" if c["expect"] else "invalid"
        ov, opaths = ours(probe, path)
        rv, detail, rpaths = oracle.get(c["file"], ("error", ["no OperationOutcome for this file"], set())) if have_java else ("-", [], set())
        status = "ok"
        if ov == "error":
            status = "LOCAL-ERR"  # our own run failed: a crash or a malformed report, not a verdict
        elif ov != expect:
            status = "OURS!=EXPECT"
        if have_java and rv == "error":
            status = "ORACLE-ERR"
            oracle_errors += 1
        elif have_java and ov != rv and status == "ok":
            status = "OURS!=ORACLE"
        if status != "ok":
            failures += 1  # a missing verdict on either side is not agreement
        print(f"{c['file']:<58} {expect:<8} {ov:<8} {rv:<8} {status}")
        if status != "ok":
            for d in detail[:6]:
                print(f"    oracle: {d}")
            if len(detail) > 6:
                print(f"    oracle: … {len(detail) - 6} more")
        if have_java and status == "ok" and rv == "invalid" and opaths != rpaths:
            print(f"    issue paths differ (info)  ours: {sorted(opaths)}  oracle: {sorted(rpaths)}")

    n = len(manifest)
    tail = "" if have_java else " (ours vs expectation only; run with Java for the oracle)"
    compared = n - oracle_errors
    print(f"\n{n - failures}/{n} agree{tail} — {compared} compared with the oracle (validator_cli {JAR_VERSION})")
    if oracle_errors:
        print(f"{oracle_errors} fixture(s) got no OperationOutcome from the oracle — counted as failures")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
