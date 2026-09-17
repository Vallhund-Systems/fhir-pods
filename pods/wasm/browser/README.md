# Browser smoke test

Exercises the published JavaScript wrapper in a real browser: valid and invalid
validation, AU Core Patient lowering, de-identification, and IPS document
assembly followed by validation. All patient data is synthetic.

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
with browsers that have not been run. See `measurement.json` for the recorded run.
