# fhir-pods

A small Rust library that lets an application **speak FHIR at its edges** —
verify a resource against compiled profiles, build one from the variable data
only, hand out a de-identified copy under a policy, assemble a patient summary
document — behind one lean C ABI, so every one of those runs wherever the
data already is: in the app, in the browser, at the edge, on device. Each
operation says which profiles and policies it applied, what it found, and
what it could not check; the host owns its data, its transport and the
decision to proceed.

## What it does

| Operation | In | Out |
| --- | --- | --- |
| **Verify** | a resource (JSON) | the findings against the base spec's shape and resource rules and against the profiles it declares — and the profiles every nested resource declares — as a report or a FHIR `OperationOutcome`; must-support gaps as information; what could not be checked, named |
| **Lower** | a profile URL and the variable inputs (`status`, `subject`, `valueQuantity.value`…) | the conformant resource — the profile supplies the codings, categories, units and slices — validated on the way out |
| **De-identify** | a resource and a policy (Safe Harbor by default) | a copy with names, contacts, addresses below state, exact dates and free text gone, identifiers pseudonymised under the host's salt, masked where the profile requires an element; an audit trail; what a Safe Harbor check still finds |
| **Document** | a patient and their resources | an IPS or AU Patient Summary `Bundle`: sections from the profile, narrative generated, references rewritten, validated |
| **Audit** | what a host knows when a record is read, queried, created, updated or deleted: who asked, from where, which server, on whose behalf, which resource or patient, the request id | an `AuditEvent` conformant to the IHE Basic Audit Log Patterns profile for that kind of event, validated; `Provenance` and a failed action's `AuditEvent` lowered from the base definitions |
| **Terminology** | the bindings the bundled sets cannot decide | answered by a compiled expansion table the host loads, by an ask-then-answer loop against a terminology server, or by a synchronous provider — never by shipped licensed content |

The pod family and what comes next: [`docs/roadmap.md`](docs/roadmap.md).

## Why a library

FHIR is, structurally, an intermediate representation for health data with a
constraint system clamping it — resources are the IR; profiles, terminology
bindings and FHIRPath invariants are the constraints; validation is the
verifier pass, and everything else an application does at its boundary
(construct, transform, assemble, record) is defined by the same constraints.

The incumbents ship that verifier as a **server** (JVM, .NET, a cloud
endpoint), because the tooling grew up on enterprise runtimes. That runtime
is the constraint: a multi-hundred-MB floor, cold starts, no way to run inside
a TypeScript app, a browser or a phone. But the work is stateless, CPU-bound,
pure compute — the textbook shape of a library. `fhir-pods` returns it to that
form factor, and keeps the rest of the boundary work in the same place.

## Why Rust

Because **conformance is a compile-time problem**. FHIR's definitions are
machine-readable, so the profiles are compiled *from the packages* into
`'static` constraint tables (cardinality, fixed values, patterns, bindings,
slicing, invariants, lowering slots) that a small generic engine walks — a
compiled-data design, not a runtime tree-walk over StructureDefinition JSON,
and not generated straight-line code either (what is measured, what is
implemented and what is a target are three different sentences in
[`docs/architecture.md`](docs/architecture.md)). The core is `no_std` with
zero dependencies and no `unsafe`, which is what lets one C ABI carry it to
wasm, iOS and Android unchanged.

## How "correct" is established

Correct means **agreeing with HL7's reference validator**, fixture by fixture,
with terminology switched off on both sides so profiles are what is scored.
`conformance/run.py` builds the corpus from the pinned packages, runs
validator_cli 6.10.4 over it, and compares verdicts:

- **1187/1187** at v0.6.0 — the six IGs' published examples and 60-odd
  probes, every corpus resource re-lowered from its own slot values, every
  one de-identified, the twelve document examples taken apart and assembled
  again, an audit record of every kind from the audit surface, and the
  review lane's adversarial probes. A run in which either
  side crashed, returned a foreign exit code or a report that disagrees
  with its own exit code counts as a failure, never as agreement. Every
  engine change is proven this way before it ships; the record is in the
  changelog.
- **Skip, never guess.** Anything outside the supported FHIRPath core, an
  unresolvable slice, a set the bundle cannot decide, a profile the registry
  has not compiled — reported as such, never invented. What is checked and
  what is not: [`docs/scope.md`](docs/scope.md).
- **Fuzzed** two ways: a seeded harness under plain `cargo test`, and
  libFuzzer targets for the validator, the parsers, lowering and
  de-identification.

## What is compiled in

259 profiles from the pinned packages (`scripts/gen-profiles.sh`
regenerates): **AU Core 2.0.0**, **US Core 6.1.0**, **IPA 1.1.0**, **IPS
2.0.1**, **AU Patient Summary 1.0.0**, **IHE Basic Audit Log Patterns
1.1.4**, the AU Base 6.0.0 resource and datatype profiles they derive from
or slice into, the extension profiles their slices inherit, the base spec's
vital-sign profiles, which an Observation is held to by its LOINC code as
the spec requires, and the base spec's own `AuditEvent` and `Provenance`
definitions, so a host can lower and declare them directly. The element-
type table covers every R4 resource and datatype, so a contained resource of
any type is typed. Every licence-free value set the registry binds is bundled
(72 sets); SNOMED CT, LOINC, VSAC and NCTS-hosted sets go through the
terminology seam.

## Delivery

- **wasm** — a ~1.1 MB `.wasm` (272 KB gzipped) with a dependency-free JS
  wrapper; browser, edge runtimes, Node, Bun, Deno. [`pods/wasm`](pods/wasm/README.md).
- **Expo** — the core as a native module: iOS XCFramework + Android jniLibs,
  a native terminology provider seam. [`pods/expo`](pods/expo/README.md).
- **Rust / C** — `fhir_pods_core` directly, or the C ABI in
  `crates/fhir-pods-ffi/include/fhir_pods.h`.
- 40–70 µs per profiled resource native, about 80 µs in wasm; the same
  inputs always give the same outputs.

## Use

```rust
use fhir_pods_core::{deidentify::Policy, Validator};

let v = Validator::new();                       // parses the registry's invariants once
let report = v.validate(json);                   // Validation: profiles applied, issues
let lowered = v.lower(HEART_RATE, inputs)?;      // Lowered: resource + validation
let copy = v.deidentify(json, &Policy::safe_harbor(b"salt").today(2026))?;
let summary = v.document("au-ps", inputs)?;      // Assembled: document + placed + validation
let record = v.audit("patient-read", event)?;    // Lowered: a BALP AuditEvent + validation
let needs = v.terminology_needs(json);           // what the bundled sets could not decide
```

```js
const pod = await instantiate(wasmBytes);
pod.validate(json);                              // { resourceType, profile, valid, issues }
pod.lower(profileUrl, inputs);                   // { resource, validation }
pod.deidentify(json, { salt, today });           // { resource, applied, validation, remaining }
pod.document("ips", { resources, author, date }); // { document, placed, omitted, validation }
pod.audit("patient-read", { recorded, client, server, user, source, patient, data, transaction });
pod.loadTerminology(table); pod.terminologyNeeds(json);
```

The Expo module exports the same operations as functions. Every result is
data: a report is never thrown, and a copy or a document is never trusted
unverified.

## Boundaries

- **Library, not a server.** No I/O, no daemon, no datastore, no fetching —
  the registry is compiled in.
- **The pods know FHIR; a host knows its data.** Domain maps (a host's
  records → FHIR inputs) live in the host, never here.
- **Ships no licensed terminology.** SNOMED CT carries affiliate licensing in
  many jurisdictions; fixed SNOMED codings carry `system` + `code` only, and
  displays come through the seam.
- **Unverifiable is visible, not silent.** A declared profile the registry
  lacks, an invariant outside the FHIRPath core, a set the bundle cannot
  decide — each is an issue in the report.

## Layout

- `crates/fhir-pods-core` — the core (zero deps, no `unsafe`, `no_std`):
  `engine`, `path` (FHIRPath), `lower`, `deidentify`, `assemble`, `audit`,
  `terminology`, `generated/` (the compiled registry), `typemap` (the
  element-type table).
- `crates/fhir-pods-ffi` — the one lean C ABI, compiled per target.
- `pods/wasm`, `pods/expo` — the delivery pods.
- `tools/` — the generators, from the packages to `'static` tables:
  profiles, the type table, the value sets, and `compile_valueset.py` for a
  host's expansions.
- `conformance/` — the corpus builder, the differential harness against the
  reference validator, and `probes/` for hand-written fixtures with their
  reasons.
- `docs/` — [`architecture.md`](docs/architecture.md), [`scope.md`](docs/scope.md),
  [`roadmap.md`](docs/roadmap.md), [`incumbent-apis.md`](docs/incumbent-apis.md),
  [`history.md`](docs/history.md).

## Testing

```bash
cargo test --workspace                       # unit + integration tests and the seeded fuzz harness
FUZZ_ITERS=1000000 FUZZ_SEED=42 cargo test -p fhir-pods-core --test fuzz   # longer / different seed
./scripts/check-no-std.sh                    # bare-metal build: nothing in the core touches std
JAVA_HOME=<jdk17> python3 conformance/run.py # differential test vs the HL7 reference validator
./scripts/gen-profiles.sh                    # regenerate the registry from the pinned packages
cargo run -p fhir-pods-core --example probe -- [--outcome|--must-support] resource.json
./scripts/smoke-c.sh                         # the C ABI, driven from C against the host staticlib
./scripts/build-wasm.sh && (cd pods/wasm && npm test)   # the wasm pod through the JS wrapper
cd fuzz && cargo +nightly fuzz run validate_bytes       # also json_parse, fhirpath_parse, lower, deidentify
./scripts/release.sh                         # every artifact into dist/<version>/ with SHA256SUMS
```

## License

MIT © 2026 Justin Grosvenor. See [`LICENSE`](LICENSE).
