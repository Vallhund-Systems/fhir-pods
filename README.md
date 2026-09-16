# fhir-pods

An embeddable FHIR verifier — a small Rust core that **validates** and **lowers**
FHIR, with a lean WASM boundary, so the check runs wherever the data already is:
in the app, in the browser, at the edge, on device.

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
- **Source-agnostic.** It lowers *arbitrary* input to FHIR. Domain-specific maps
  live in the host application, never in this repo.

## Status

Working verifier for **AU Core 2.0.0** (61 compiled profiles incl. the
`hl7.fhir.au.base` identifier types): per-parent cardinality, fixed values,
required bindings, slicing with profile inheritance, and a FHIRPath core
sufficient for every invariant in the registry bar four (type tests,
`%resource`, `descendants`, `intersect` — skipped, never guessed). Agrees with
the HL7 reference validator on **84/84** corpus instances (the IG's own 64
examples + 20 broken ones) with zero issue-path differences. Zero
dependencies, `no_std`, fuzzed. Design and decisions:
[`docs/architecture.md`](docs/architecture.md),
[`docs/incumbent-apis.md`](docs/incumbent-apis.md).

## Layout

- `crates/fhir-pods-core` — the Rust verifier + lowering core (zero deps, no `unsafe`).
- `crates/fhir-pods-ffi` — the one lean C ABI, compiled per target (wasm32 / native staticlib).
- `pods/expo` — the expo pod: the core as an Expo native module (iOS skeleton).
- `scripts/` — native build (iOS XCFramework; Android `cargo-ndk` next).
- `docs/architecture.md` — the design, delivery model, audit surface, and perf budget.

## Testing

```bash
cargo test -p fhir-pods-core                 # unit tests + the seeded fuzz harness (50k iterations)
FUZZ_ITERS=1000000 FUZZ_SEED=42 cargo test -p fhir-pods-core --test fuzz   # longer / different seed
./scripts/check-no-std.sh                    # bare-metal build: nothing in the core touches std
python3 conformance/run.py                   # differential test against the HL7 reference validator
```

Two fuzz layers. The **seeded harness** (`crates/fhir-pods-core/tests/fuzz.rs`)
runs under plain `cargo test` with no dependencies: deterministic under
`FUZZ_SEED`, so any failure is a seed you paste back to replay. It feeds byte
soup, mutated documents, FHIR-shaped resources with real declared profiles, and
random FHIRPath — holding *never panics, never hangs, deterministic, well-formed
wire output*. The **libFuzzer targets** (`fuzz/`, needs nightly) are for
coverage-guided sessions. Both parsers are depth-bounded, so hostile nesting is
a `ParseError`, never a stack overflow.

## License

MIT © 2026 Justin Grosvenor. See [`LICENSE`](LICENSE).
