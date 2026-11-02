#!/usr/bin/env bash
# Bounded functional/input checks for every entrypoint in the native header.
# Optional SANITIZE=1 instruments the C caller; it does not instrument Rust.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$(mktemp -d "${TMPDIR:-/tmp}/fhir-pods-abuse.XXXXXX")"
trap 'rm -rf "$OUT"' EXIT
cd "$ROOT"
cargo build -q -p fhir-pods-ffi --release
FLAGS=(-std=c11 -Wall -Wextra -Werror -DNDEBUG)
if [[ "${SANITIZE:-0}" == 1 ]]; then
  FLAGS+=(-fsanitize=address,undefined -fno-omit-frame-pointer)
fi
LINK=()
if [[ "$(uname -s)" == Linux ]]; then LINK+=(-ldl -lpthread -lm); fi
cc "${FLAGS[@]}" -I crates/fhir-pods-ffi/include crates/fhir-pods-ffi/c/abuse.c \
  target/release/libfhir_pods_ffi.a "${LINK[@]}" -o "$OUT/abuse"
python3 - "$OUT/abuse" <<'PYTHON'
import collections
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

def git(*args):
    return subprocess.check_output(['git', *args], text=True).strip()
def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()
result = subprocess.run([sys.argv[1]], capture_output=True, timeout=60)
sys.stderr.buffer.write(result.stderr)
if result.returncode:
    raise SystemExit(f"C ABI harness failed: exit {result.returncode}")
lines = result.stdout.splitlines()
if not lines:
    raise SystemExit("No output documents were checked")
counts = collections.Counter()
for i, line in enumerate(lines):
    envelope = json.loads(line.decode('utf-8', errors='strict'))
    if not isinstance(envelope, dict) or set(envelope) != {'entry', 'result'}:
        raise SystemExit(f"Output {i} is not an entry/result object")
    value = envelope['result']
    if not isinstance(value, dict):
        raise SystemExit(f"Output {i} is not a JSON object")
    if 'error' in value and not isinstance(value['error'], str):
        raise SystemExit(f"Output {i} has a malformed operation error")
    counts[envelope['entry']] += 1
header = re.sub(r'/\*.*?\*/', '', Path('crates/fhir-pods-ffi/include/fhir_pods.h').read_text(), flags=re.S)
expected = set(re.findall(r'\b(fhir_pods_\w+|pod_alloc|pod_free)\s*\([^;]*\);', header))
if set(counts) != expected:
    raise SystemExit(f"Header coverage mismatch: missing={sorted(expected-set(counts))}, unexpected={sorted(set(counts)-expected)}")
paths = ['crates/fhir-pods-ffi/c/abuse.c', 'crates/fhir-pods-ffi/include/fhir_pods.h', 'scripts/abuse-c.sh', 'target/release/libfhir_pods_ffi.a']
report = {
    'recordedAt': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'revision': git('rev-parse', 'HEAD'),
    'dirtyPaths': git('status', '--porcelain', '--untracked-files=no').splitlines(),
    'abi': int(re.search(r'#define FHIR_PODS_ABI_VERSION (\d+)u', Path(paths[1]).read_text())[1]),
    'ndebug': True, 'cSanitizers': os.environ.get('SANITIZE') == '1',
    'rustInstrumented': False, 'timeoutSeconds': 60,
    'resultBuffers': sum(n for entry,n in counts.items() if entry not in ['pod_alloc','pod_free','fhir_pods_abi_version','fhir_pods_terminology_load','fhir_pods_questionnaire_load','fhir_pods_resource_load','fhir_pods_options']),
    'entrypoints': dict(sorted(counts.items())),
    'sha256': {path:digest(path) for path in paths},
    'resultStreamSha256': hashlib.sha256(result.stdout).hexdigest(),
    'compiler': subprocess.check_output(['cc','--version'],text=True).splitlines()[0],
    'rustc': subprocess.check_output(['rustc','--version'],text=True).strip(),
    'summary': result.stderr.decode().splitlines()[-1],
}
if os.environ.get('FHIR_ABI_REPORT'):
    Path(os.environ['FHIR_ABI_REPORT']).write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report, indent=2))
PYTHON
