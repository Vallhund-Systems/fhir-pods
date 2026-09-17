#!/usr/bin/env bash
# Run the release benchmark and append one dated, revision-labelled JSON line.
set -euo pipefail
BENCH_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$BENCH_ROOT"
cargo build --release -p fhir-pods-core --example bench
python3 - "$BENCH_ROOT" "${1:-bench/results.jsonl}" "${FHIR_BENCH_ITERATIONS:-1000}" <<'PY'
import datetime, hashlib, json, os, pathlib, platform, subprocess, sys
root, output = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
def command(*args):
    return subprocess.check_output(args, cwd=root, text=True).strip()
start = datetime.datetime.now(datetime.timezone.utc).isoformat()
result = json.loads(command(str(root/'target/release/examples/bench'), sys.argv[3]))
result.update(recorded_at=start, revision=command('git','rev-parse','HEAD'),
              dirty_paths=command('git','status','--porcelain').splitlines(),
              rustc=command('rustc','--version'), platform=platform.platform(),
              architecture=platform.machine(), cpu_count=os.cpu_count(),
              load_average=list(os.getloadavg()),
              fixture_sha256={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((root/'bench').glob('*.json'))},
              example_sha256=hashlib.sha256((root/'crates/fhir-pods-core/examples/bench.rs').read_bytes()).hexdigest(),
              binary_sha256=hashlib.sha256((root/'target/release/examples/bench').read_bytes()).hexdigest())
output.parent.mkdir(parents=True, exist_ok=True)
with output.open('a') as stream: stream.write(json.dumps(result, separators=(',',':'))+'\n')
print(json.dumps(result, indent=2))
PY
