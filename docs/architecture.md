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

- **The compiled path (this repo).** A `build.rs` ingests an Implementation
  Guide package and emits Rust: typed resource structs, and validators where the
  profile's cardinalities, must-support flags, and binding strengths are baked
  into straight-line code. A thin `derive`/attribute macro
  (`#[fhir(profile = "…")]`) gives the ergonomic surface that wires a host type
  to the generated conformance. Result: monomorphized checks, no runtime
  interpretation, and a tiny `.wasm`.
- **The interpreted path (pure TS, and most JS FHIR libs).** Ship the
  StructureDefinitions as data and tree-walk them at runtime on every
  validation. Larger, slower, and — the real problem — undifferentiated from
  what already exists.

Precise division of labor: **`build.rs` codegen for spec ingestion** (proc-macros
are the wrong tool for reading large external files — it hurts incremental
compilation), **a small `proc-macro` derive for ergonomics**. "Gut said proc
macros" was pointing at the right thing: compile-time generation of the verifier.

## Modules (`fhir-pods-core`)

| module | role |
|---|---|
| `model` | resource representation (raw JSON now → generated typed model) |
| `validate` | the verifier: `Validator` → `Report` of `Issue`s |
| `path` | FHIRPath engine — evaluates invariants |
| `terminology` | pluggable `TerminologyProvider` for binding checks (ships no licensed content) |
| `lower` | source-agnostic lowering passes: arbitrary input → FHIR resource |
| `audit` | FHIR-native audit records (`AuditEvent` / `Provenance`) |

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
- **Android:** build `.so` per ABI with `cargo-ndk` into `jniLibs`; a Kotlin
  wrapper calls the C ABI.
- **JS surface:** a synchronous **JSI** call — validate a JSON string, get a
  report, no bridge serialization. Start on the Expo Modules API; **Nitro
  Modules / hand-written JSI** is the zero-copy upgrade if marshaling ever shows
  up in a profile.

The C ABI already in `fhir-pods-wasm` is the shared FFI surface — the native pod
compiles it to a staticlib instead of `wasm32`. (Worth generalizing the crate
name `fhir-pods-wasm` → `fhir-pods-ffi` once the native target lands.)

**Boundary (the wall):** the expo pod is a **standalone Expo module any app can
install** — published or vendored as a release artifact, never source-linked into
a host app's tree. A consumer depends on it the way it would any third-party
native module; the pod never imports the host, and the host never imports pod
source. Same separation as a vendored SDK release.

## WASM boundary (`fhir-pods-wasm`)

A **lean C ABI**, not wasm-bindgen: host passes a UTF-8 JSON resource as
`(ptr, len)`; the pod returns a JSON report as `(ptr, len)`; memory is managed
explicitly via `pod_alloc` / `pod_free`. wasm-bindgen (or hand-written JS) can
wrap this for ergonomic TypeScript later — but the core never pays the glue cost.
`unsafe` is confined to this shim; the core forbids it.

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
  id. (This is why `audit::AuditRecord` carries `trace_id`.)

**Net:** traces alone — no. A dedicated, unsampled, durable audit stream (FHIR
`AuditEvent`), *correlated* with OTel traces by id, with OTel as the collection
plumbing — yes, and that is the elegant answer.

## Non-goals

- Not an enterprise FHIR server. The datacenter is the incumbents' fortified
  ground; JVM heaviness is a non-issue there and their conformance maturity is
  the moat. Attack the form factor their runtime can't reach.
- Not a terminology distributor. Providers are pluggable; licensed content
  (SNOMED CT) is never bundled.
- Not a place for any single application's domain mappings.
