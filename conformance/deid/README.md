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

The recorded run has **10 passes and 4 failures**:

- `extension-free-text` and `codeable-concept-text` retain identifying
  witnesses while the core's checker returns an empty list (planning F018).
- `extension-human-name` and its Bundle counterpart remove the names but
  leave URL-only extensions that fail `ext-1` (planning F019).

These are intentional failing regression probes awaiting core decisions/fixes.
They are not suppressed as expected failures. A marker-free output only
establishes the fate of the planted witnesses; this small corpus cannot
certify de-identification of arbitrary data. Recorded evidence includes input
hashes, executable hash, revision and a dirty-tree flag. The committed evidence
was captured on 2026-09-16 in an isolated v0.5.2 checkout with these new tools
overlaid, excluding the concurrent audit implementation.
