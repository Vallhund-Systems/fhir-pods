#!/usr/bin/env python3
"""Fetch the finite IPS diagnostic-status expansion and compile a code-only table.

Example (from repo root, after obtaining the pinned IPS package):
  python3 conformance/terminology/fetch.py \
    conformance/package-ips/package/ValueSet-diagnostics-report-status-uv-ips.json \
    /tmp/fhir-terminology-refresh

Only the public ValueSet definition is sent to tx.fhir.org; no resources or
patient data. Raw response and provenance stay in the supplied output folder.
The compiler refuses incomplete expansions. Review the resulting table before
replacing diagnostics-status.txt. The public server is needed only to refresh.
"""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import urllib.request

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
URL = "http://hl7.org/fhir/uv/ips/ValueSet/diagnostics-report-status-uv-ips"
ENDPOINT = "https://tx.fhir.org/r4/ValueSet/$expand"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("definition", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    definition = args.definition.read_bytes()
    value_set = json.loads(definition)
    if (value_set.get("resourceType"), value_set.get("url"), value_set.get("version")) != ("ValueSet", URL, "2.0.1"):
        parser.error("expected the pinned IPS 2.0.1 diagnostic-status ValueSet")
    body = json.dumps({"resourceType": "Parameters", "parameter": [{"name": "valueSet", "resource": value_set}]}).encode()
    request = urllib.request.Request(ENDPOINT, data=body, headers={"Content-Type": "application/fhir+json", "Accept": "application/fhir+json"})
    with urllib.request.urlopen(request, timeout=60) as response:
        raw = response.read()
    expanded = json.loads(raw)
    if expanded.get("url") != URL or expanded.get("expansion", {}).get("total") != 9:
        raise ValueError("unexpected expansion identity or member count")
    args.output.mkdir(parents=True, exist_ok=True)
    expansion_path = args.output / "expansion.json"
    expansion_path.write_bytes(raw)
    table = subprocess.check_output([sys.executable, str(ROOT / "tools/compile_valueset.py"), "--no-display", str(expansion_path)])
    (args.output / "diagnostics-status.txt").write_bytes(table)
    sha = lambda data: hashlib.sha256(data).hexdigest()
    evidence = {"recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(), "endpoint": ENDPOINT,
                "value_set": URL, "version": "2.0.1", "definition_sha256": sha(definition),
                "response_sha256": sha(raw), "table_sha256": sha(table), "members": 9,
                "compiler_sha256": sha((ROOT / "tools/compile_valueset.py").read_bytes())}
    (args.output / "provenance.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    main()
