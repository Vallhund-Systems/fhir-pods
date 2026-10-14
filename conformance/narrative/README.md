# Narrative probes

Thirty-four synthetic `Narrative.div` cases whose verdict a reader would not
guess: what validator_cli 6.10.4 tolerates where the text ends mid-markup, and
what it does — and does not — check about an image's source. A host's report
(2026-10-14) asserted three of these the other way round, so the lane records
the probe rather than the reasoning:

- An **unclosed `<p>` inside a closed root `<div>`** is **refused** on both
  sides (`Found "</div>" expecting "</p>"`, with `txt-1` and `txt-2`): a
  closing tag that does not match is a parse failure, and the failure leaves
  no document, so the narrative also has no content.
- An **element left open where the text ends** — the root `<div>` included —
  **passes**. So does a **start tag the text ends inside**: the element is kept
  with the attributes read so far, which is why a truncated root keeps its
  `xmlns` (namespace fine, `txt-2` fails) while a bare `<div` has no namespace
  (both findings). The one truncation refused is a `/` that no `>` follows,
  and a **closing** tag the text ends inside (`Unexpected termination of html
  source`).
- An **external `<img src="http://…">` passes**, and so do `https:`, a bare
  path, `file:`, `urn:`, `cid:` — and `javascript:`. The hyperlink scheme
  rules (active content, "not hyperlinkable") are checked on `<a href>` only;
  an image's source is checked for an empty value, invalid characters, a
  `data:` URL's comma and a `#fragment` that must resolve, nothing else.
  A host that wants a narrative reference out of the resource refused asks for
  it: `options({"externalNarrativeLinks": "refuse"})` (`docs/scope.md`).

```sh
JAVA_HOME=/opt/homebrew/opt/openjdk@17 CARGO_BUILD_JOBS=2 python3 conformance/narrative/run.py
python3 conformance/narrative/run.py --oracle-only
```

One JVM, one temporary oracle output, no shared cache. The run verifies the
JAR's version and every result identity, compares the verdict three ways
(manifest expectation, oracle, pod) and writes `evidence.json` with each
fixture's narrative, its hash, the oracle's diagnostics and the pod's report.
A missing result, a nonzero oracle exit or any disagreement fails the command.

Verdict parity is what this lane gates. The pod's exact messages and rule ids
for these cases are pinned by the core's narrative test
(`crates/fhir-pods-core/tests/validate.rs`).
