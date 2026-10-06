# WASM validation measurements

From the repository root, build the artifact and run:

```sh
bash scripts/build-wasm.sh
node pods/wasm/bench/run.mjs --iterations 2000 --output /tmp/fhir-pods-bench.jsonl
```

Requires Node 22+. `--wasm /path/to/artifact.wasm` selects another artifact.
The script emits one dated JSON line and optionally appends it to `--output`.
`measurements.jsonl` retains the dated ABI 7 and ABI 12 development runs and
appends the latest clean ABI 13 run. Each names its exact revision, dirty-tree
state, artifact SHA-256 and fixture hashes; current entries also hash runner
and wrapper sources. Measurements describe their named commits and artifacts,
independent of release tags.

Each fixture is checked before measurement, warmed for 200 calls, then measured
in five rounds. The report includes all samples and median/min/max microseconds
per call, artifact bytes and gzip level-9 bytes, ABI version, runtime, hardware,
and source revision. Validation covers two valid resources, a profile violation,
and malformed JSON.

Timing includes the JavaScript wrapper, UTF-8 copies, WASM validation, output
copying and JSON parsing. It excludes file loading, warmup and terminology
callbacks. Instantiation time is reported separately. These are Node/V8 results;
the browser smoke test does not measure browser throughput. There is no
performance pass threshold, and a dirty-tree revision alone cannot reproduce
the exact binary: compare its hash and build configuration too.
