# Independent de-identification probes

```sh
python3 conformance/deid/run.py
```

The runner builds `deid_probes`, passes JSONL requests through the public
`Validator::deidentify` API, and writes `evidence.json`. It exits 1 if any
expectation fails. Process failures and malformed/missing responses fail the
run; they are never counted as successful privacy checks.

All 14 fixtures are synthetic. The default policy uses a fixed test salt and
date for reproducibility. `make_fixtures.py` regenerates the JSON and manifest;
it does not fetch anything. Never use the test salt for real data.

The checks combine the core's independent `deidentify::check` call with a scan
of every output JSON string for explicit identifying witnesses. Each witness
must exist in its input. A deliberate `base: none` control retains a name;
both detectors must catch it. Additional checks cover contained reference
integrity, Unicode identifiers, member-order stability, age units, recorded
clamps and exact repeatability. Input and output validation reports are kept.

The current recorded run has **14 passes and no failures**, using the v0.6.4
core with the corrected Bundle fixture. The previous v0.5.2 run recorded
10 passes / 4 failures (planning F018/F019); those text-removal and extension
pruning regressions now pass. Bundle entry fullUrls match the resources' IDs,
and the repeated Patient/Observation examples have distinct entry identities.
The generator preserves those relationships. The corrected Bundle and its
de-identified copy also pass reference validator 6.10.4 in review T038.

No expectation is suppressed as an expected failure. A marker-free output only
establishes the fate of the planted witnesses; this small corpus cannot
certify de-identification of arbitrary data. Recorded evidence includes input
hashes, executable hash, revision and a dirty-tree flag. The committed evidence
was captured on 2026-09-16 in an isolated v0.6.4 checkout with the review
lane's CLI, comparison adapter and fixture changes overlaid. Concurrent engine
work is excluded. This driver's historical report does not serialize the core's
new `residual` field; separate CLI and ABI/WASM checks cover that reporting
contract. The positive retention control is deliberate, not a production policy.
