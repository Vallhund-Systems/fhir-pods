# Browser smoke test

Exercises the JavaScript wrapper in Chrome with 29 synthetic ABI 12 checks:
validation/outcomes, discovery/slots, lift/lower and fixed conflicts,
de-identification, documents, audit, v2, export preservation and patient
references, terminology table replacement/clear, and provider callbacks.
Reentrant callbacks load a table and validate within the active operation.
The same operation checks run under Node in `test/operations.test.mjs`.

All eight WASM provider routes are invoked. Validate, outcome, lower, lift,
document and export must actually request a membership answer; the audit and
v2 fixtures exercise routing without requiring an unresolved binding.
Synthetic membership answers are fixture controls, not clinical expansions.

From the repository root:

```sh
bash scripts/build-wasm.sh
node pods/wasm/browser/run.mjs
```

The automated runner requires Node 22+ and Chrome. It starts a temporary local
HTTP server and a fresh headless Chrome profile, prints a dated JSON report,
and exits nonzero on failure. On macOS it uses the standard Chrome application;
elsewhere it uses `chromium` on PATH. Set `CHROME=/path/to/browser` to override.
No browser package or network service is needed after building the artifact.

For an interactive run:

```sh
python3 -m http.server 8080 --bind 127.0.0.1 --directory pods/wasm
```

Open `http://127.0.0.1:8080/browser/index.html`. The page runs on load and the
button repeats it. `window.fhirPodsSmoke` exposes the result to browser tooling.
This is a smoke check of these operations; it does not establish compatibility
with browsers that have not been run. `measurement.json` records the 2026-09-22 run after v0.12.1, including the
exact source revision, source hashes, artifact SHA-256 and browser version.
That commit is unreleased; these are not measurements of the v0.12.1 tag.
