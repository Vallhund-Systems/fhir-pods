# fhir-pods — architecture

## Compliance at the application's edges

`fhir-pods` is an embeddable compliance runtime. Its operations connect an
application's data to the formats, profiles, and policies required at a
boundary: lift, lower, validate, de-identify, assemble documents, construct
audit records, and convert HL7 v2 messages.

FHIR resources serve as an intermediate representation. Profiles, terminology
bindings, and FHIRPath invariants constrain that representation. The same
compiled definitions drive extraction and construction as well as the
verifier pass. A host maps its domain model to profile slots and chooses
which operations to run; the runtime returns outputs and findings.

The current Rust operation context is named `Validator`. It caches parsed
invariant expressions and supplies the shared validation path used by
`lift`, `lower`, `deidentify`, `document`, `audit`, and `v2`. The product name
covers that full surface; API names remain as implemented.

## Compile definitions, execute operations

FHIR publishes machine-readable definitions. Generators consume pinned
package snapshots and emit immutable tables committed with the core:

- `tools/gen_all.py` and `tools/gen_profile.py` compile profiles into
  `CompiledProfile`: element constraints, slicing, invariants, extension
  contexts, must-support paths, construction skeletons, and input slots.
- The type-table generator describes every R4 resource and datatype so
  traversal, validation, and transformations can interpret each node.
- Terminology generators compile the bundled value sets and code membership
  tables. `tools/compile_valueset.py` prepares host-supplied expansions.
- `tools/gen_v2.py` compiles the published v2-to-FHIR maps and identifies
  rows the converter cannot execute.

This is a compiled-data design. Generic Rust engines apply the tables to
runtime inputs. Profile compilation prepares the rules; evaluating a
particular resource remains runtime work. The core parses JSON into an
allocated tree with borrowed strings and evaluates cached FHIRPath syntax
trees. It does not fetch or interpret StructureDefinition JSON on each call.

`scripts/gen-profiles.sh` drives registry regeneration explicitly. New IG
support is a generation and conformance task, not a runtime package download.

## Modules (`fhir-pods-core`)

| Module | Role |
| --- | --- |
| `compiled`, `generated` | Profile representation and the compiled registry |
| `json`, `node` | Input parsing and owned output construction |
| `document`, `typing`, `typemap` | Typed document traversal, including nested resources and primitive carriers |
| `engine`, `path`, `regex` | Constraint verification, FHIRPath evaluation, and `matches()` support |
| `lower`, `lift` | Construct resources from profile slots and extract slots from resources |
| `deidentify` | Typed policy transformations, pseudonyms, applied actions, and residual findings |
| `assemble` | IPS / AU Patient Summary assembly, section placement, narrative, and references |
| `audit` | Map host event context to IHE BALP profiles and lower audit resources |
| `v2` | ER7 parsing, compiled conversion maps, message Bundle construction, and coverage reporting |
| `terminology`, `value_sets`, `code_systems_table` | Bundled membership data, expansion tables, providers, and outstanding terminology needs |
| `validate`, `wire` | Findings, validation reports, and FHIR `OperationOutcome` serialization |

The core is `no_std` with `alloc`, zero external dependencies, and
`#![forbid(unsafe_code)]`. It owns no filesystem, network connection, clock,
or application datastore. The host supplies the context needed by each call.

## Operation results and coverage

Construction and transformation return data together with validation. An
output can be useful for inspection even when it fails a constraint; the
host decides whether to accept it. Failure to start an operation is separate
from findings on an output.

Coverage is operation-specific:

- Lift returns missing required slots and uncovered element paths (`extras`).
  Lowering lifted inputs preserves what the slot model covers; arbitrary
  input data is not guaranteed to survive a round trip.
- De-identification returns applied actions, remaining checker findings,
  and sites a host rule left in place (`residual`). The built-in policy is a
  rule set modelled on HIPAA Safe Harbor's identifier list; its checks
  describe that transformation's coverage and claim no compliance
  determination. The host owns the release decision.
- Document assembly reports placed and omitted resources.
- HL7 v2 conversion reports skipped map rows with reasons, unplaced
  segments, and valued fields absent from the maps.
- Verification reports unknown profiles, unsupported expressions, and
  unresolved terminology. `valid` means no error or fatal finding; warnings
  can still identify requirements that have not been checked.

The [scope document](scope.md) details verification coverage. The
[roadmap](roadmap.md) records shipped operations and remaining work.

## Delivery — one core, several surfaces

| Target | Surface | Mechanism |
| --- | --- | --- |
| Browser / edge / server JS | WASM pod | C ABI compiled to `wasm32`, wrapped by dependency-free JavaScript |
| iOS / Android | Expo pod | Native XCFramework / shared libraries, Swift or Kotlin wrapper, Expo Modules API |
| Native applications | Rust / C | Direct core calls or `fhir-pods-ffi` |
| Local scripts and pipelines | CLI | Rust core plus filesystem/stdin/stdout handling in the CLI |

Expo uses the native core on both platforms. Swift calls the C header;
Kotlin reaches the same operation family through the FFI crate's Android
JNI shim. The package vendors the native artifacts. Hosts consume the pod
as a standalone dependency; application mappings stay in the host.

The current shared C ABI is **14**. It includes validation, OperationOutcome,
slots, lift/lower, de-identification, documents, audit, export, HL7 v2,
must-support inspection, discovery, host options, and terminology /
Questionnaire loading and needs. See the
[header](../crates/fhir-pods-ffi/include/fhir_pods.h) for exact signatures.

**Versioning the ABI.** `FHIR_PODS_ABI_VERSION` (the header, the FFI crate's
`ABI_VERSION`, and each wrapper's own constant) rises whenever the **exported
surface** or the **memory contract** changes — and an added entrypoint is a
change to the exported surface, because a wrapper that calls it would
otherwise accept a module that does not export it and fail at the call rather
than at instantiation. Every wrapper checks `fhir_pods_abi_version()` against
the one it was written against and refuses a mismatch, so the version is what
makes a mismatch a clean refusal — the wasm pod is handed its module and
refuses it at `instantiate`; the Expo pod *contains* its native library, so a
mismatch there means mismatched artifacts were packed or vendored, and it is
refused on import by the TypeScript layer with the native modules logging it
on creation. What does *not* move the number: a new member in a returned JSON
document, a new rule id, a reworded message, a wider accepted input. Those are
additive within an entrypoint's contract and a host that ignores them is
unaffected. Each bump is recorded in the [changelog](../CHANGELOG.md) with
what it added: 14 host options, 13 the Questionnaire store, 12 discovery, 11
export, 10 HL7 v2, 9 lift, 8 audit, 7 documents, 6 de-identification, 5
terminology, 4 lower/slots, 3 the first cut.

Inputs are UTF-8 byte buffers: JSON for resources and options, ER7 for v2,
and text for profile URLs or terminology tables. Results are JSON in
`[u32 little-endian length][UTF-8 bytes]` buffers. The host frees buffers
with `pod_free`, using the full allocated length. `pod_alloc` supplies
input storage where needed. `unsafe` stays in the FFI boundary.

The JS wrappers copy inputs and outputs across that boundary and expose
synchronous operations after initialization. They return validation
findings as data and throw when an operation cannot start.

## Terminology

`TerminologyProvider` answers value-set membership with `Member`,
`NotMember`, or `Unknown`, and can supply displays for construction.
Bundled sets are consulted first, followed by a loaded expansion table
and an optional host provider. Unknown membership is reported as a warning.

WASM accepts a synchronous JS callback. Expo accepts a native Swift/Kotlin
provider. A host needing asynchronous network lookup uses the
`terminologyNeeds` ask-then-answer loop, loads answers, and reruns the
operation. The core never performs that network request itself.

Validation, lower, lift, document, audit, and v2 have provider-aware paths.
De-identification's validation currently uses bundled terminology; a host
can separately revalidate its output with additional terminology.

## Audit and provenance

The host supplies the facts of an event: actor, action, time, source, subject,
and correlation context. `audit` constructs IHE BALP basic `AuditEvent`
resources. General `Provenance` and failed-action `AuditEvent` records use
`lower` with the base definitions.

Construction does not observe application activity or persist records. The
host decides where to emit events and supplies delivery, retention, access,
and integrity controls. A request identifier can correlate a BALP event with
application telemetry while the host maintains its audit records separately.

## Evidence and performance

The runtime is checked both through operation-specific tests and through
external validation of its inputs and outputs. The release corpus covers
published IG examples, lift/lower, de-identification, documents, audit, and
v2 conversion. Wider base and profile-test lanes track reviewed differences
and exclusions. Exact counts and reproduction commands live in the
[README](../README.md#evidence-and-limits), [conformance guide](../conformance/README.md),
and [changelog](../CHANGELOG.md).

Performance evidence covers multiple operations: the
[native harness](../bench/README.md) measures validation, lowering,
de-identification, and document assembly; the
[WASM harness](../pods/wasm/bench/README.md) includes wrapper costs.
Records identify fixtures, versions, and hardware. The v0.10.0 WASM release
is 2,387,366 bytes, or 586,075 bytes with gzip-9.

Potential optimizations include validation during parsing, reusable output
buffers, and specialized generated code. These are future designs; today's
implementation uses an allocated tree and generic table-driven operations.

## Host options

Two strictness calls are the host's, and both default to what the reference
validator does: `unknownProfile` (a declared profile this build has not
compiled is a warning, or an error that fails the resource closed) and
`externalNarrativeLinks` (a narrative reference out of the resource is
accepted, or refused). They are data on the operation context
(`Validator::set_options`), persisted across calls by the pods
(`fhir_pods_options`, `options(json)`, `--options FILE`), and an options
document that does not say what it means — an unknown member or value — is
refused whole, leaving the previous options in force. Nothing here changes
*what* is checked; see [scope](scope.md#host-options).

## Host responsibilities

The host selects profiles and policies, maps its domain data, obtains
external terminology, and decides how to handle findings and incomplete
coverage. It also owns consent, authorization, persistence, transport,
credential management, and audit delivery. Regional requirements need
explicit implementation and evidence; choosing a profile does not configure
an application's entire compliance posture.
