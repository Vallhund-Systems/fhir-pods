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

Commit zero — architecture and module skeleton. Nothing implemented yet; the
core builds with zero external dependencies. Design, conformance target, and the
audit-surface stance are in [`docs/architecture.md`](docs/architecture.md).

## Layout

- `crates/fhir-pods-core` — the Rust verifier + lowering core (zero deps, no `unsafe`).
- `crates/fhir-pods-wasm` — the lean C-ABI WASM boundary (JSON in, report out).
- `docs/architecture.md` — the design.

## License

MIT © 2026 Justin Grosvenor. See [`LICENSE`](LICENSE).
