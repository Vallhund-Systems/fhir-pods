# Reference health app

A synthetic daily check-in that uses the compliance runtime at the app's
boundary. Change a person or pulse reading, choose a destination, inspect
what changed, and download the resulting FHIR when the example host policy
passes. It runs locally in a browser with no external dependencies.

From the repository root:

```sh
CARGO_BUILD_JOBS=2 bash scripts/build-wasm.sh
node examples/health-app/serve.mjs
```

Open **http://127.0.0.1:4173**. `PORT=4174` chooses another port.
The server binds to loopback and serves only the example and WASM wrapper/
artifact. There is no bundler, package install, account or vendor API.
Stop it with Ctrl-C. Nothing is persisted by the page, and JSON downloads
stay on the host computer.

## Try the boundary

1. Review the **Unsupported checks**. Since v0.13.1 the runtime names an
   unevaluated slice in the BALP audit, and AU also names one on Patient.
   The default policy pauses download. For this synthetic local demo, the
   checkbox can explicitly accept the listed limitations for this preview;
   the review packet records them. AU/US resource bundles can then be
   downloaded. Switch destinations: the base mapping stays the same.
2. In **AU**, choose **Missing birth date**. Base Patient permits the omission; AU
   requirements surface it. The app pauses FHIR download rather than invent
   a date. A review packet remains available.
3. Choose **Qualified reading (<)**. The qualifier is preserved in the
   export and listed as carried. Under the hood, **Lift reading** exposes
   the value in the base slots and the qualifier in `extras`.
4. Enable **Include a patient summary**. With bundled terminology only,
   unresolved memberships pause download. IPS documents also show the
   known offline pregnancy-slice ambiguity. A valid verdict alone does
   not satisfy the host policy.
5. Inspect **Host-supplied terminology** to load a table produced by
   `tools/compile_valueset.py`, then recheck. No clinical expansion table
   or sample provider that invents membership answers is bundled here.

The regions are discovered through `pod.profiles()`. With v0.13.0 they are
AU, US, UK, CA, NZ and IPS. The sample deliberately shows CA/NZ uncarried
fields and UK/CA notes about additional document profiles. Findings are
runtime output, not preset verdicts. Region support does not mean every
record can be exported without additional host work.

## Integration structure

| File | Responsibility |
| --- | --- |
| `integration.mjs` | Host-shaped model, one base R4 adapter, regional export, host decision, review packet and lift inspection |
| `app.mjs` | Form state, discovered destinations, report rendering, terminology file input and local downloads |
| `serve.mjs` | Small local static server |
| `integration.test.mjs` | Mapping, preservation and download-policy checks against the actual WASM |
| `browser-test.mjs` | Chrome interaction, downloaded JSON, desktop/mobile layout and artifact evidence |

`toBaseRecord()` knows base Patient and heart-rate profiles. It contains no
regional branches. `prepareExport()` supplies author/time and the context
for an IHE BALP patient-read audit, then calls `pod.export(region, …)`.
Resources-only output is wrapped in a collection Bundle and validated as a
whole. Document mode uses the runtime's assembled patient summary.

This example's conservative host policy pauses FHIR download for errors,
missing inputs, `extras`, any selection/identity note, omitted document
entries, unresolved terminology or an invalid/missing audit record. This
includes informational notes; the app never interprets prose as approval.
`not-supported` findings also pause download, including those with information
severity. A separate, unchecked host control can accept just the listed
non-error unsupported findings for a local demo download. The decision is
recorded in `acceptedLimitations`, never changes the runtime's report, and
clears on any input, region, document or terminology change. It cannot waive
errors, missing/lost data, unresolved terminology or selection/identity notes.
Other validation warnings/information remain visible and do not alone pause
download.
`carried` is successful preservation, not a blocker. Editing inputs clears
the current review and disables both downloads until rebuilt.

The **review JSON** is an application envelope with the original model,
base record, full export, audit, candidate Bundle and host decision. It is
not an approved exchange document. The **FHIR download** is a collection
or document Bundle, and is available only when the host decision passes.

The audit describes the local read used to prepare the preview, using
explicitly synthetic device/network context. It is neither sent nor stored;
it is not evidence that a recipient received a document. The example does
not implement consent, authentication, transport, retention, vendor-specific
clinical mapping or regulatory approval. Passing the compiled checks is
not a legal-compliance certification.

Lift inspection reads the exported observation against the same base
heart-rate profile. The returned `extras` and `conflicts` stay visible;
the demo does not silently write a partial lift back into the app model.

## Verification

```sh
node --test examples/health-app/integration.test.mjs
node examples/health-app/browser-test.mjs --output /tmp/fhir-pods-reference-app
```

Node 22+ and local Chrome are required for the browser check. On macOS the
default is the standard Chrome application; elsewhere it uses `chromium`.
Set `CHROME=/path/to/browser` to override. It uses an isolated browser
profile and an ephemeral loopback port, then cleans up both. The output
directory receives a JSON report and desktop/mobile screenshots. Evidence
records the source revision/dirty paths, source hashes, WASM hash and browser.
The browser test snapshots the WASM bytes before serving them; `--wasm PATH`
selects a pinned artifact. `FHIR_PODS_WASM=PATH` does the same for the Node
integration tests. This keeps evidence stable during concurrent core builds.
