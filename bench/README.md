# Native operation benchmark

From the repository root, run `./scripts/bench.sh`. It builds the release
example, checks that every operation succeeds on its fixed synthetic fixture,
then appends one JSON line to `bench/results.jsonl`. Set
`FHIR_BENCH_ITERATIONS` to change the positive number of operations per sample;
pass a destination path as the script's first argument to record elsewhere.

Each operation has 100 warm-up calls and seven samples. Reported microseconds
are elapsed sample time divided by its iteration count; min/median/max summarize
those seven batch means, not individual-call latency percentiles. A cached
`Validator` and a fixed Safe Harbor policy are reused. Timings include each
operation's normal parsing, transformation and validation; report serialization,
fixture loading, process startup and `Validator::new()` are outside the timer.
Input buffers and outputs pass through `black_box`.

Fixtures cover AU Core heart-rate validation/lowering, a base Patient under
Safe Harbor, and a minimal IPS document with a patient and author. All identifiers
and names are synthetic. SNOMED entries are codes only, with no licensed display
text. These small fixtures do not represent large Bundles or every profile.
The original publisher-example phase breakdown is retained as:

```sh
cargo run --release -p fhir-pods-core --example bench -- --phases
```

That optional mode requires the ignored publisher corpus. The default operation
benchmark has no corpus, network or Java dependency.

The recorded v0.8.3 run uses 1,000 iterations per sample and has median batch
means of 77.299 µs validation, 105.382 µs lowering, 25.360 µs de-identification,
and 624.884 µs document assembly. This is a dated measurement on this machine,
not a performance guarantee or a comparison to earlier different fixtures.
The machine was shared (load averages are recorded); no controlled regression
claim is made. Every record includes the revision, dirty paths, toolchain,
platform, fixture hashes and executable/source hashes. The initial record's
tracked edit is this benchmark; the core is committed `f0f1961`.
