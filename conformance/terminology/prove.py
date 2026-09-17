#!/usr/bin/env python3
"""Verify the compiled table through the native ABI and actual browser WASM.

  python3 conformance/terminology/prove.py --wasm dist/0.8.3/fhir_pods-0.8.3.wasm --output /tmp/fhir-tx-proof

Requires Python, Cargo, Node 22+ and Chrome (override its executable with CHROME).
No terminology-server access is needed: this tests the checked-in finite table.
A valid final report has one unresolved binding before loading, none afterward.
An entered-in-error report must then fail that binding, proving non-membership
is checked too. Clearing the table must restore the unresolved binding.
"""
import argparse
import ctypes
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
VS = "http://hl7.org/fhir/uv/ips/ValueSet/diagnostics-report-status-uv-ips|2.0.1"


def fixtures():
    positive = {"resourceType": "DiagnosticReport", "id": "synthetic-report",
                "meta": {"profile": ["http://hl7.org/fhir/uv/ips/StructureDefinition/DiagnosticReport-uv-ips"]},
                "status": "final", "code": {"text": "Synthetic report"},
                "subject": {"reference": "Patient/synthetic"}, "effectiveDateTime": "2026-09-17T10:00:00Z"}
    return {"positive": positive, "negative": {**positive, "status": "entered-in-error"}}


class Native:
    def __init__(self, path):
        self.lib = ctypes.CDLL(str(path))
        self.lib.pod_free.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
        self.lib.pod_free.restype = None
        self.lib.fhir_pods_abi_version.restype = ctypes.c_uint32
        for name in ["validate", "terminology_needs", "terminology_load"]:
            fn = getattr(self.lib, "fhir_pods_" + name)
            fn.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
            fn.restype = ctypes.c_int32 if name == "terminology_load" else ctypes.c_void_p

    def call(self, name, value):
        raw = value.encode() if isinstance(value, str) else json.dumps(value).encode()
        buffer = ctypes.create_string_buffer(raw)
        pointer = getattr(self.lib, "fhir_pods_" + name)(buffer, len(raw))
        if name == "terminology_load":
            return pointer == 1
        if not pointer:
            raise RuntimeError("null output from " + name)
        length = int.from_bytes(ctypes.string_at(pointer, 4), "little")
        try:
            return json.loads(ctypes.string_at(pointer + 4, length))
        finally:
            self.lib.pod_free(pointer, 4 + length)


def native_report(pod, cases, table):
    def observe():
        return {name: {"validation": pod.call("validate", value),
                       "needs": pod.call("terminology_needs", value)["needs"]} for name, value in cases.items()}
    assert pod.call("terminology_load", ""), "table clear failed"
    report = {"abi": pod.lib.fhir_pods_abi_version(), "before": observe()}
    try:
        report["loaded"] = pod.call("terminology_load", table)
        report["after"] = observe()
    finally:
        report["cleared"] = pod.call("terminology_load", "")
    report["reset"] = observe()
    return report


def checks(report):
    expected = lambda needs: len(needs) == 1 and needs[0].get("valueSet") == VS
    return {"abi": report["abi"] == 10,
            "load": report["loaded"] is True,
            "clear": report["cleared"] is True,
            "before_positive": report["before"]["positive"]["validation"]["valid"] and expected(report["before"]["positive"]["needs"]),
            "before_negative_unresolved": expected(report["before"]["negative"]["needs"]),
            "after_positive": report["after"]["positive"]["validation"]["valid"] and report["after"]["positive"]["needs"] == [],
            "after_negative": not report["after"]["negative"]["validation"]["valid"] and report["after"]["negative"]["needs"] == [],
            "reset": report["reset"] == report["before"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wasm", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    wasm = args.wasm.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    subprocess.run(["cargo", "build", "-p", "fhir-pods-ffi"], cwd=ROOT, check=True)
    library = ROOT / ("target/debug/libfhir_pods_ffi.dylib" if sys.platform == "darwin" else "target/debug/libfhir_pods_ffi.so")
    cases = fixtures()
    inputs = args.output.resolve() / "fixtures.json"
    inputs.write_text(json.dumps(cases, indent=2) + "\n")
    table_path = HERE / "diagnostics-status.txt"
    native = native_report(Native(library), cases, table_path.read_text())
    browser = json.loads(subprocess.check_output(["node", str(HERE / "browser.mjs"), str(wasm), str(inputs)], text=True))
    native_checks, browser_checks = checks(native), checks(browser)
    sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    report = {"recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "dirty_paths": subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).splitlines(),
              "sha256": {"library": sha(library), "wasm": sha(wasm), "wrapper": sha(ROOT / "pods/wasm/src/index.js"), "table": sha(table_path), "fixtures": sha(inputs)},
              "native": native, "browser": browser, "native_checks": native_checks, "browser_checks": browser_checks,
              "native_browser_equal": all(native[key] == browser[key] for key in native)}
    report["pass"] = all(native_checks.values()) and all(browser_checks.values()) and report["native_browser_equal"]
    (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ["native_checks", "browser_checks", "native_browser_equal", "pass"]}, indent=2))
    return 0 if report["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
