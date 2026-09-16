# Changelog

## 0.1.0 — 2026-09-15

First cut. One verifier core, one C ABI (version 3), three pods.

**Verifier**
- AU Core 2.0.0, US Core 6.1.0, IPA 1.1.0 — 168 compiled profiles (86
  resource profiles plus the `Identifier` and complex `Extension` datatypes
  slices inherit), generated from the packages' snapshots by
  `scripts/gen-profiles.sh`.
- Per-parent cardinality, fixed values, required bindings (`code` / `Coding`
  / `CodeableConcept`), slicing (value / pattern / type discriminators,
  closed slicing, nested-slice discriminator resolution, slice → profile
  inheritance), every declared profile applied.
- FHIRPath: navigation, boolean logic, equality/comparison, integer
  arithmetic, string and conversion functions, `where`/`select`/`all`/`iif`,
  `extension(url)`, type tests against the element's declared type,
  `%resource`, union, `in`, `descendants`, `repeat`, `intersect`,
  `isDistinct`, `allTrue` family, `matches` on a linear-time regex engine.
  Outside the core: skipped, never guessed.
- Terminology seam: bundled base-FHIR value sets first, a host provider
  behind them; SNOMED content never ships.
- Must-support gaps as information, separate from validation.
- Agrees with the HL7 reference validator (`validator_cli.jar`, `-tx n/a`)
  on 288/288 corpus instances; what that covers and excludes is in
  `docs/scope.md`.
- `no_std` + `alloc`, zero dependencies, `#![forbid(unsafe_code)]`;
  depth-bounded parsers; seeded fuzz harness under `cargo test`.

**Delivery**
- `fhir-pods-wasm` — ~330 KB `.wasm` (83 KB gzipped) + a dependency-free JS
  wrapper; ~30k validations/s in Node.
- `fhir-pods-expo` — Expo native module: iOS XCFramework (Swift over the C
  header) and Android jniLibs (Kotlin over a JNI shim), ~20k validations/s
  on simulators; terminology provider installed natively.
