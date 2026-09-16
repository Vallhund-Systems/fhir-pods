# fhir-pods

An embeddable FHIR verifier — a small Rust core that **validates** FHIR against
compiled profiles, behind one lean C ABI, so the check runs wherever the data
already is: in the app, in the browser, at the edge, on device.

## Why

FHIR is, structurally, an intermediate representation for health data with a
constraint system clamping it — resources are the IR; profiles, terminology
bindings, and FHIRPath invariants are the constraints; validation is the
verifier pass.

Every incumbent ships that verifier as a **server** (JVM, .NET, or a cloud
endpoint), because the tooling grew up on enterprise runtimes. That runtime is
the constraint: a multi-hundred-MB floor, cold starts, no way to run inside a
TypeScript app, a browser, or a phone. But a verifier is stateless, CPU-bound,
pure compute — the textbook shape of a **library**, not a server.

`fhir-pods` returns the verifier to its correct form factor.

## Why Rust and not pure TypeScript

Because **conformance is a codegen problem**. FHIR's definitions
(StructureDefinitions, ValueSets) are machine-readable, so the typed models and
their validators can be *generated from the spec at compile time* — yielding
monomorphized, straight-line verifier code and a tiny, fast `.wasm`. A pure-TS
implementation interprets StructureDefinitions at runtime: it ships the spec and
tree-walks it on every check — slower, larger, and indistinguishable from the
existing JS FHIR helpers. The compile-time path is the differentiator, and it's
Rust-shaped. (See [`docs/architecture.md`](docs/architecture.md).)

## Scope

- **Library, not a server.** No I/O, no daemon, no datastore.
- **Correct on a narrow profile first.** Target **AU Core** and pass the
  published conformance suites (Inferno / Touchstone) before widening.
- **Ships no licensed terminology.** SNOMED CT carries affiliate licensing in
  many jurisdictions; terminology providers are pluggable.

## Status

- **AU Core 2.0.0, US Core 6.1.0, IPA 1.1.0** — 168 compiled profiles (86
  resource profiles plus the `Identifier` and complex `Extension` datatypes
  slices inherit).
- **Lowering:** build a conformant resource from its variable data only —
  the profile supplies the codings, categories, units and slices; the host
  supplies `status`, `subject`, `effectiveDateTime`, `valueQuantity.value`…
  and gets the resource back validated. Proven by re-lowering every corpus
  example from its slot values: 262/262 valid.
- **Checks:** per-parent cardinality, fixed values, required bindings, slicing
  with profile inheritance, every declared profile applied, must-support gaps
  (as information), and a FHIRPath engine with typed traversal and a
  linear-time `matches()` that evaluates every invariant in the registry
  except `htmlChecks()` and `resolve()` — skipped, never guessed.
- **Agreement with the HL7 reference validator: 294/294** corpus instances
  (the three IGs' own 253 published examples + 41 probes). What that
  covers and excludes: [`docs/scope.md`](docs/scope.md).
- **Terminology:** every licence-free set the registry binds is bundled
  (generated from the packages); the rest arrives through a host seam — a
  compiled expansion table loaded at run time, an ask-then-answer loop
  (`terminology_needs`), or a synchronous provider — never as shipped content.
- **Delivery:** a ~715 KB `.wasm` (198 KB gzipped) with a dependency-free JS
  wrapper, and an Expo module (iOS XCFramework + Android jniLibs) — each
  measured at tens of microseconds per profiled resource.
- Zero dependencies, `no_std`, `#![forbid(unsafe_code)]`, fuzzed.
- **Boundary:** the pods know FHIR; a host knows its data. Domain-specific
  maps (a host's records → FHIR) live in the host, never here.
- **De-identification:** a Safe Harbor copy under a data policy — typed
  walk, pseudonyms keyed by the host's salt, masked stubs where a profile
  requires what was removed, a checker that proves the copy clean.
- **Next:** documents (IPS / AU PS), audit — the pod family in
  [`docs/roadmap.md`](docs/roadmap.md).

Design: [`docs/architecture.md`](docs/architecture.md) ·
decisions: [`docs/incumbent-apis.md`](docs/incumbent-apis.md) ·
how it got here: [`docs/history.md`](docs/history.md).

## Use

- Browser / edge / Node: [`pods/wasm`](pods/wasm/README.md) — `instantiate(bytes)` → `validate(json)`.
- iOS / Android (Expo): [`pods/expo`](pods/expo/README.md) — `validate(json)`, native terminology provider.
- Rust: `let v = fhir_pods_core::Validator::new(); v.validate(bytes) -> Validation`,
  `v.lower(profile_url, inputs) -> Lowered` (the free functions re-parse invariants per call);
  the C ABI is `crates/fhir-pods-ffi/include/fhir_pods.h`.

## Layout

- `crates/fhir-pods-core` — the Rust verifier core (zero deps, no `unsafe`, `no_std`).
- `crates/fhir-pods-ffi` — the one lean C ABI, compiled per target (wasm32 / native staticlib).
- `pods/wasm` — the wasm pod: a ~715 KB `.wasm` (198 KB gzipped) + a zero-dependency JS wrapper.
- `pods/expo` — the expo pod: the core as an Expo native module (iOS XCFramework + Android jniLibs).
- `scripts/` — `gen-profiles.sh`, `build-wasm.sh`, `build-ios-xcframework.sh`, `build-android-jnilibs.sh`, `smoke-c.sh`, `check-no-std.sh`, `release.sh`.
- `docs/architecture.md` — the design, delivery model, audit surface, and perf budget.

## Testing

```bash
cargo test -p fhir-pods-core                 # unit tests + the seeded fuzz harness (50k iterations)
FUZZ_ITERS=1000000 FUZZ_SEED=42 cargo test -p fhir-pods-core --test fuzz   # longer / different seed
./scripts/check-no-std.sh                    # bare-metal build: nothing in the core touches std
python3 conformance/run.py                   # differential test vs the HL7 reference validator (AU Core, US Core, IPA)
./scripts/gen-profiles.sh                    # regenerate the compiled profiles from the pinned packages
cargo run -p fhir-pods-core --example probe -- [--outcome|--must-support] resource.json   # one resource, by hand
./scripts/smoke-c.sh                         # the C ABI, driven from C against the host staticlib
./scripts/build-wasm.sh && (cd pods/wasm && npm test)   # the wasm pod, end to end through the JS wrapper
```

Two fuzz layers. The **seeded harness** (`crates/fhir-pods-core/tests/fuzz.rs`)
runs under plain `cargo test` with no dependencies: deterministic under
`FUZZ_SEED`, so any failure is a seed you paste back to replay. It feeds byte
soup, mutated documents, FHIR-shaped resources with real declared profiles, and
random FHIRPath — holding *never panics, never hangs, deterministic, well-formed
wire output*. The **libFuzzer targets** (`fuzz/`) run on nightly:
`cd fuzz && cargo +nightly fuzz run validate_bytes` (also `json_parse`,
`fhirpath_parse`). Both parsers are depth-bounded, so hostile nesting is
a `ParseError`, never a stack overflow.

## License

MIT © 2026 Justin Grosvenor. See [`LICENSE`](LICENSE).
