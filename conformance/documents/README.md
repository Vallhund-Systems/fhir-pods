# Document assembly probes

```sh
# Requires the same cached IG packages and pinned validator_cli.jar as conformance/run.py.
JAVA=/path/to/java python3 conformance/documents/run.py
python3 -m unittest discover -s conformance/documents -p 'test_*.py'
```

`make_fixtures.py` generates 25 synthetic input sets. The runner exercises
each as IPS and AU PS through the public `Validator::document` API: **50
cases**, including every section-routing resource kind, the Observation
routes, no patient, outside author, orphan resource, year-only vital date,
foreign absolute reference and changed-content identity. It checks section
references against `placed`, graph integrity, omission reports and repeatability.

The native driver returns the full result as JSONL. Every assembled artifact
is then checked by reference validator **6.10.4**, FHIR R4, IPS **2.0.1**,
AU PS **1.0.0**, AU Core **2.0.0**, with `-tx n/a`. A fresh temporary directory,
JAR version check, strict process status, complete source identities and
OperationOutcome/severity checks protect the evidence. Native rejections
have no artifact for the oracle; they are counted separately. The runner
does not regenerate the shared corpus or use its cached oracle output.

The recorded default run has **45 passes / 5 failed expectations**, with
**48/48 oracle verdict agreement** and two expected native rejections. The
five failures are IPS vital signs, social history, pregnancy and laboratory,
plus AU PS pregnancy: both validators report overlapping profile slices.
The runner exits 1 rather than hiding these failures behind verdict parity.

`diagnostic-terminology.txt` contains the complete, explicitly enumerated
code-only contents of two IPS 2.0.1 value sets, `edd-method-uv-ips` and
`pregnancies-summary-uv-ips`. Source JSON hashes are in its comments; the IPS
package declares CC0-1.0. No clinical displays or network expansion are used.
Revalidating the same outputs through `validate_with` and that table makes
all five failures valid; **48/48 diagnostic verdicts meet their intended
expectation**. This diagnostic is recorded separately and never replaces the
actual `document` verdict or the oracle verdict. The default assembly API
has no terminology-provider argument. See planning F020 for this limitation.

`evidence.json` retains full documents, reports, required terminology,
input/executable/table/JAR hashes and source revision/dirty state. The committed
evidence was captured on 2026-09-16 in an isolated v0.5.2 checkout with these
new tools overlaid, excluding the concurrent audit implementation. Terminology
is disabled on the oracle: agreement alone does not establish correctness
when both sides lack a value set. Neither this corpus nor its terminology
diagnostic proves full implementation-guide conformance.
