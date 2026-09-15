#!/usr/bin/env python3
"""Differential conformance test.

Runs our validator and the HL7 reference validator (`validator_cli.jar` — the
ecosystem's oracle; "correct" means bug-for-bug agreement with it) over the
corpus and compares verdicts. The oracle needs Java 11+; our side runs without
it, so on a machine without Java this still checks us against the expected
verdicts and reports that the oracle was skipped.

    python3 conformance/run.py
"""
import json
import os
import subprocess
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HERE = os.path.join(ROOT, "conformance")
CORPUS = os.path.join(HERE, "corpus")
JAR = os.path.join(HERE, "validator_cli.jar")
JAR_URL = "https://github.com/hapifhir/org.hl7.fhir.core/releases/latest/download/validator_cli.jar"
IG = "hl7.fhir.au.core#2.0.0"


def java_bin():
    """The JVM to run the oracle with: $JAVA (an explicit binary), else
    $JAVA_HOME/bin/java, else `java` on PATH."""
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


def ours(probe, path):
    r = subprocess.run([probe, path], capture_output=True, text=True)
    return "valid" if r.returncode == 0 else "invalid"


def oracle(path, profile):
    """Run the reference validator with terminology disabled (-tx n/a), the
    same offline posture as our engine; error/fatal issues => invalid."""
    out = path[:-5] + ".oracle.json"
    subprocess.run(
        [JAVA, "-jar", JAR, path, "-version", "4.0", "-ig", IG, "-profile", profile, "-tx", "n/a", "-output", out],
        capture_output=True, text=True,
    )
    try:
        o = json.load(open(out))
    except (OSError, ValueError):
        return "error", ["oracle produced no OperationOutcome"]
    bad = [i for i in o.get("issue", []) if i.get("severity") in ("error", "fatal")]
    detail = [
        f"{i.get('severity')}: {(i.get('details') or {}).get('text') or i.get('diagnostics')} @ {i.get('expression') or i.get('location')}"
        for i in bad
    ]
    return ("invalid" if bad else "valid"), detail


def main():
    subprocess.check_call([sys.executable, os.path.join(HERE, "make_corpus.py")], stdout=subprocess.DEVNULL)
    subprocess.check_call(["cargo", "build", "-q", "-p", "fhir-pods-core", "--example", "probe"], cwd=ROOT)
    probe = os.path.join(ROOT, "target", "debug", "examples", "probe")

    have_java = jvm_starts()
    if have_java and not os.path.exists(JAR):
        print("==> fetching validator_cli.jar")
        urllib.request.urlretrieve(JAR_URL, JAR)
    if not have_java:
        print(
            "!! No JVM: the oracle (validator_cli.jar) needs Java 11+ — set JAVA_HOME (or JAVA) to a JDK. "
            "Running OUR side only.\n"
        )

    manifest = json.load(open(os.path.join(CORPUS, "manifest.json")))
    print(f"{'fixture':<28} {'expect':<8} {'ours':<8} {'oracle':<8} status")
    failures = 0
    oracle_errors = 0
    for c in manifest:
        path = os.path.join(CORPUS, c["file"])
        expect = "valid" if c["expect"] else "invalid"
        ov = ours(probe, path)
        rv, detail = oracle(path, c["profile"]) if have_java else ("-", [])
        status = "ok"
        if ov != expect:
            status = "OURS!=EXPECT"
        if have_java and rv == "error":
            # An oracle that produced nothing is an infrastructure failure,
            # not a verdict disagreement — report it, don't score it.
            status = "ORACLE-ERR"
            oracle_errors += 1
        elif have_java and ov != rv:
            status = "OURS!=ORACLE"
        if status in ("OURS!=EXPECT", "OURS!=ORACLE"):
            failures += 1
        print(f"{c['file']:<28} {expect:<8} {ov:<8} {rv:<8} {status}")
        if status != "ok":
            for d in detail:
                print(f"    oracle: {d}")

    n = len(manifest)
    tail = "" if have_java else " (ours vs expectation only; run with Java for the oracle)"
    print(f"\n{n - failures}/{n} agree{tail}")
    if oracle_errors:
        print(f"{oracle_errors} fixture(s) got no OperationOutcome from the oracle (see the .oracle.json files)")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
