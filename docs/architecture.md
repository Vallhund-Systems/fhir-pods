# fhir-pods — architecture

## Thesis

FHIR is an intermediate representation for health data with a constraint system
clamping it:

- **resources** are the IR (`Patient`, `Observation`, `Condition`, …),
- **profiles** (StructureDefinitions), **terminology bindings** (ValueSets), and
  **FHIRPath invariants** are the constraints,
- **validation** is the verifier pass.

The verifier is stateless, CPU-bound, pure compute — a library, not a server.
`fhir-pods` ships it as a Rust core with a lean WASM boundary so it runs wherever
the data is.

## Why Rust, not pure TypeScript — conformance is codegen

FHIR publishes its definitions as machine-readable artifacts: base resources,
StructureDefinitions (including jurisdictional profiles like AU Core), and
ValueSets are themselves FHIR resources. So the types *and* the validators for a
given profile are **derivable from the spec**.

- **The compiled path (this repo).** `tools/gen_all.py` reads the packages'
  shipped `snapshot.element` and emits Rust — one `CompiledProfile` per
  profile (cardinality, fixed values, bindings, slicing, invariants,
  must-support) as `'static` data, plus an element-type table from the base
  spec — committed under `src/generated/` and `src/typemap.rs`. The engine
  walks that data; nothing is interpreted from StructureDefinitions at run
  time and nothing is fetched. Result: a small `.wasm`, no warm-up.
- **The interpreted path (pure TS, and most JS FHIR libs).** Ship the
  StructureDefinitions as data and tree-walk them at runtime on every
  validation. Larger, slower, and — the real problem — undifferentiated from
  what already exists.

Division of labor: **a generator for spec ingestion** (a Python script over
the packages, run by `scripts/gen-profiles.sh`, with committed output — no
`build.rs`, no proc-macros: reading large external packages at build time
hurts incremental compilation and hides the IR), **hand-written Rust for the
engine**.

## Modules (`fhir-pods-core`)

| module | role |
|---|---|
| `json` | zero-copy top-level scan + borrowed value tree, depth-bounded |
| `document` | the `Document` port and the tree adapter (`Parsed` / `TreeDoc`), typed traversal |
| `typing` + `typemap` | the element-type table and its lookup (a node learns its FHIR type as it is reached) |
| `compiled` | the profile IR the generator emits (`CompiledProfile`, slicing, invariants, must-support) |
| `generated` | the compiled profiles + `REGISTRY` (committed output of `tools/gen_all.py`) |
| `engine` | the verifier: `CompiledProfile` × `Document` → `Report` |
| `path` | the FHIRPath engine — evaluates invariants |
| `regex` | a linear-time regex engine for FHIRPath `matches()` |
| `terminology` + `value_sets` | the `TerminologyProvider` seam and the bundled base-FHIR sets (ships no licensed content) |
| `validate` | `Severity`, `IssueType`, `Issue`, `Report` |
| `wire` | the anti-corruption layer: `OperationOutcome` and the `Validation` JSON |

## Conformance target

Be **correct on one profile before widening**: **AU Core** (the CSIRO/Sparked
Australian FHIR profiles) is the first target, validated against the published
suites (**Inferno**, **Touchstone**). Do not chase HAPI's full surface — the win
is form factor + correctness on a narrow, real profile, not breadth.

## Delivery — pods (one core, bindings per target)

The core is target-general; each *pod* is a thin binding over the same lean
C ABI. "Runs wherever the data is" is delivered by the right binding per target,
not one artifact everywhere:

| Target | Pod | Mechanism |
|---|---|---|
| Browser / edge / server | **wasm pod** | C ABI → `wasm32` → tiny JS wrapper |
| iOS / Android (Expo app) | **expo pod** | C ABI → native staticlib (iOS XCFramework `.a` / Android `.so`) → Expo Modules API module |

The phone target is **native, not WASM.** Hermes has no WASM runtime, so running
the verifier as WASM inside React Native means a WebView or a bundled interpreter
— slow and heavy, the opposite of the point. On device you compile the *same
core* to a native staticlib and bridge it:

- **iOS:** build an **XCFramework** (arm64 device + sim) from the staticlib and
  vendor it in a **CocoaPods pod** — hence the name. A Swift wrapper calls the C
  header; the **Expo Modules API** handles autolinking + the config plugin.
- **Android:** build `.so` per ABI into `jniLibs` (plain cargo pointed at the
  NDK clang; `cargo-ndk` optional); a Kotlin wrapper reaches the same two
  entrypoints through a small Android-only JNI shim in the FFI crate (the JVM
  can't call a C ABI directly).
- **JS surface:** a synchronous **JSI** call — validate a JSON string, get a
  report, no bridge serialization. Start on the Expo Modules API; **Nitro
  Modules / hand-written JSI** is the zero-copy upgrade if marshaling ever shows
  up in a profile.

The C ABI in `fhir-pods-ffi` is the shared FFI surface — the native pod
compiles it to a staticlib instead of `wasm32`. Three entrypoints —
`fhir_pods_validate` (the `Validation` JSON), `fhir_pods_operation_outcome`
(the spec wire form), `fhir_pods_must_support` (information) — plus the
`_tx` twins that take a host terminology callback. Every pod makes exactly
these calls; the ABI never changes per target.

**Boundary (the wall):** the expo pod is a **standalone Expo module any app can
install** — published or vendored as a release artifact, never source-linked into
a host app's tree. A consumer depends on it the way it would any third-party
native module; the pod never imports the host, and the host never imports pod
source. Same separation as a vendored SDK release.

## The C ABI (`fhir-pods-ffi`)

A **lean C ABI**, not wasm-bindgen: host passes a UTF-8 JSON resource as
`(ptr, len)`; the pod returns a JSON report as `(ptr, len)`; memory is managed
explicitly via `pod_alloc` / `pod_free`. wasm-bindgen (or hand-written JS) can
wrap this for ergonomic TypeScript later — but the core never pays the glue cost.
`unsafe` is confined to this shim; the core forbids it.

## The terminology seam

Bindings are checked through one trait — `terminology::TerminologyProvider`
(`validate_code(value_set, system, code) -> Member | NotMember | Unknown`).
`Bundled` answers from the value sets compiled into the core; `Chain` puts a
host provider behind it, asked only what the bundle can't decide. `Unknown`
is a warning, never an error, so an offline host degrades gracefully. Over the
C ABI the provider is a callback (`fhir_pods_validate_tx`); in wasm it is an
import the JS wrapper supplies; in the expo pod it is a native closure /
`fun interface` the app installs once — the verifier never calls back into
JS. Bindings are compiled by kind: a `code` is the code itself, a `Coding` is
system + code, a `CodeableConcept` is a member if any of its codings is.

## Audit surface

FHIR already models the audit trail as **data**, not as a byproduct of
observability:

- **`AuditEvent`** — the actor–action–subject–outcome–time record of a single
  access or mutation of PHI.
- **`Provenance`** — the lineage of a resource (which activity, which agents,
  which sources it derived from).

So "the audit surface" for a FHIR component is: emit correctly-formed
`AuditEvent` / `Provenance` at every point PHI is read or changed, and retain
them **completely, immutably, and queryably by subject**. `fhir-pods` constructs
and validates those resources; storage and transport are the host's.

### Can OpenTelemetry traces be the audit of record? No — but OTel can be the plumbing.

Traces and audit both look like "records of what happened," but their
requirements diverge in load-bearing ways:

| | Traces (OTel) | Audit |
|---|---|---|
| Completeness | **sampled** (1–10%) by design | **every** PHI event, no sampling |
| Delivery | best-effort, batched, droppable under backpressure | at-least-once, durable, guaranteed |
| Retention | days–weeks | years (e.g. HIPAA 6y), immutable |
| Integrity | mutable telemetry | append-only, ideally hash-chained / WORM |
| PII / PHI | scrubbed **out** | subject reference required **in** |
| Query axis | by trace / service | by **subject** (patient) |
| Schema | freeform attribute bag | fixed tuple (`AuditEvent`) |

The two conflicts that kill "traces alone":

1. **Sampling vs completeness.** Cranking sampling to 100% to cover audit turns
   the trace backend into an expensive, wrong-shaped audit store — and it's still
   neither durable nor immutable.
2. **PHI direction.** Traces must *not* carry PHI; audit must name the subject.
   Push subject identifiers into spans and your observability vendor (Honeycomb,
   Datadog, …) becomes a PHI processor, typically outside a BAA — the same rule
   as "no PHI in the non-BAA cache."

### The shape that works

- **Audit is a first-class domain event**, emitted intentionally at the PHI
  boundary — not inferred from a span. Model it as a FHIR `AuditEvent`.
- **Give it its own pipeline.** OTel is fine as *transport* — but use the **Logs**
  signal (or a dedicated span→audit exporter routed by the Collector), on an
  **unsampled, PHI-aware** path, into a durable append-only sink with long
  retention. Never the sampled trace pipeline.
- **Correlate, don't conflate.** Stamp the `trace_id` onto the `AuditEvent` so an
  investigation can pivot from the authoritative audit record to the rich,
  ephemeral trace. Audit is the record of truth; the trace is context, linked by
  id. (A host that emits `AuditEvent`s should carry the trace id on them.)

**Net:** traces alone — no. A dedicated, unsampled, durable audit stream (FHIR
`AuditEvent`), *correlated* with OTel traces by id, with OTel as the collection
plumbing — yes, and that is the elegant answer.

## Performance — almost free, shaped right

Structural validation should be effectively free, and that is a design
constraint, not a hope. A FHIR resource is a few KB. Three things are true
at once here, and they are kept apart:

**Measured (v0.6.1, release build, one 2026 Apple M-series laptop,
`examples/bench.rs`):** 78 µs for an AU Core Patient, 81 µs for an AU Core
heart rate, 152 µs for a US Core Patient; the `.wasm` is 1.1 MB (277 KB
gzipped). Earlier releases measured 37 / 66 µs on a quieter machine — the
absolute number is the machine's; what the releases hold to is no
regression back to back (planning T020 makes it reproducible). The conformance corpus the numbers come from is
reproducible from `conformance/run.py`.

**Current implementation:** profiles are compiled into `'static` constraint
tables (cardinality, fixed values, patterns, bindings, slicings, invariants,
lowering slots) that a *generic* engine walks; the JSON is parsed into an
allocated value tree with borrowed strings, typed as it is reached from the
element-type table; FHIRPath invariants are parsed once per process and
evaluated per resource. Nothing is interpreted from StructureDefinitions at
run time and nothing is fetched. It is a compiled-data design, not
generated straight-line code.

**Targets, not yet built:** validate-on-parse in one pass without a
materialised tree; no allocation in the hot path (borrow the input, emit
issues into a reusable buffer); constraints lowered to straight-line code
per profile. Each is a measured win to chase when the numbers above stop
being in the noise for a host — none is claimed now.

Two places "free" quietly ends, designed against from the start:

1. **The FFI boundary.** Copy the JSON in and the report out twice and the copy
   dominates for large payloads. Keep it JSI zero-copy; **batch on the native
   side** rather than crossing the bridge per resource.
2. **Terminology.** Membership of a code in a large ValueSet is the one
   non-trivial lookup. **Compile ValueSets into perfect-hash / FST tables at
   build time** so the check stays O(1); never ship an interpreted set scan.

Net: structural validation cheap, boundary zero-copy where the platform
allows it (the wasm and Expo pods copy the JSON once each way today),
terminology compiled. Measured, not asserted — and the measurement, the
implementation and the target are three different sentences.

## Non-goals

- Not an enterprise FHIR server. The datacenter is the incumbents' fortified
  ground; JVM heaviness is a non-issue there and their conformance maturity is
  the moat. Attack the form factor their runtime can't reach.
- Not a terminology distributor. Providers are pluggable; licensed content
  (SNOMED CT) is never bundled.
- Not a place for any single application's domain mappings.
