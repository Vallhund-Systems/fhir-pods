# Domain model

The compliance runtime performs several operations over the same health-data
definitions. Its shared language includes resources, profiles, slots,
policies, terminology, and findings. Validation is one domain service and
also a dependency of extraction, construction, and transformation.

## Shared language

| Term | Meaning |
| --- | --- |
| **Resource** | A FHIR instance read, constructed, transformed, or checked by an operation |
| **Profile** | A StructureDefinition constraining a base resource or datatype |
| **Element / path** | A typed node and its location, including choices and labelled slices |
| **Cardinality** | Per-parent minimum and maximum counts |
| **Binding** | A coded element's relationship to a ValueSet |
| **Slice / discriminator** | A named subset of a repeating element and the rules that select it |
| **Fixed / pattern** | Required exact content or required matching content |
| **Invariant** | A FHIRPath constraint with an identifier and severity |
| **Slot** | A profile's variable input, with path, type, cardinality, and choice information |
| **Skeleton** | Resource content derived from the profile's fixed values, patterns, and required slices |
| **Policy / rule** | De-identification actions selected by type, type member, or element path |
| **Issue / report** | A finding and its collection; `OperationOutcome` is the FHIR wire representation |
| **TerminologyProvider** | Membership and display answers supplied independently of the operations |

## Operation contexts

| Context | Inputs and outputs | Implementation |
| --- | --- | --- |
| Compilation | Package definitions → immutable profiles, type data, terminology tables, and conversion maps | `tools/` and generated tables |
| Lift / lower | Resource ↔ profile slot inputs, with validation and extraction coverage | `lift`, `lower` |
| Verification | Typed document plus applicable definitions → findings | `engine`, `path`, `validate` |
| De-identification | Resource plus host policy/context → transformed resource, applied actions, findings | `deidentify` |
| Document assembly | Patient resources plus author/time/context → IPS or AU PS Bundle and placement report | `assemble` |
| Audit construction | Host event context → IHE BALP AuditEvent; general lineage uses base-profile lowering | `audit`, `lower` |
| HL7 v2 conversion | ER7 message plus conversion options → FHIR message Bundle and coverage report | `v2` |
| Terminology | Value set, system, and code → membership; coding → display when available | `terminology`, bundled tables |
| Delivery | Operation inputs/results ↔ Rust, C, JavaScript, native mobile, or CLI representation | `wire`, FFI, pods, CLI |

`CompiledProfile` is shared immutable reference data. Its constraint tables
serve verification; its skeleton and slots serve construction and extraction.
Typed traversal also lets policies act on what a field is, rather than only
on its spelling. See [compiled.rs](../crates/fhir-pods-core/src/compiled.rs)
for the exact representation.

## Host data and runtime values

The host owns the lifecycle of its application records. Within the runtime,
resources are operation inputs and outputs, profiles are reference data,
and issues are result values. No repository or application aggregate lives
inside the core.

The host's map connects its fields to profile slot paths. A lifted result
contains `inputs`, `validation`, `missing`, and `extras`; uncovered fields
require an explicit host decision before lowering those inputs again.
The runtime does not infer a host's domain model or invent missing clinical
values.

The shared Rust context is currently named `Validator`. It parses invariant
expressions once and exposes the operation family. Reusing that context
avoids reparsing invariants for every call; it does not create a stored
resource lifecycle.

## Typed document access

The verifier and FHIRPath evaluator use the `Document` port. Its traversal
operations expose child counts, scalars, instances, choices, descendants,
types, and node identity. The `Parsed` / `TreeDoc` adapter reads the JSON
value tree while preserving enclosing-resource context and primitive
extension carriers.

This separation keeps rule evaluation independent of JSON representation.
The current adapter materializes a tree; a future parser could use a
different representation while preserving the port's behavior. Output
construction uses owned nodes where operations need to add, remove, or
rewrite content.

## Results are part of the model

Each operation reports both its output and the information needed to assess
it. `Validation` describes applied profiles and findings. Lift has coverage
fields; documents report placement and omissions; v2 reports skipped,
unplaced, and unmapped input; de-identification records applied actions,
remaining checks, and retained free-text sites.

A successful function call means the operation produced a result. A valid
report means no error or fatal finding was emitted. Neither discards
warnings or operation-specific coverage information. The host uses those
values to decide whether its boundary requirements have been met.

An operation that cannot start has a separate error value. Rust exposes
`Result`; the C ABI serializes an error; JS wrappers throw it. Validation
failures on a produced resource remain ordinary result data.

## Terminology and external context

The core depends on terminology answers through a provider interface, not
on a particular service. Bundled content, loaded expansions, and host
providers can be composed. Unknown answers remain visible.

The host supplies policy choices, salts, time, actors, sources, and any
external terminology. Network lookup and persistence happen outside the
core. This keeps the same operation usable on a device, in a browser, and
in a server process.

## Bounded execution

The parsers have depth limits, profile inheritance is bounded, and the
core's tests include seeded malformed-input and operation checks. Separate
libFuzzer targets exercise parsers, validation, lowering, and
de-identification. These mechanisms and their tests support the runtime's
use on caller-supplied input; they are not a proof that arbitrary execution
is infallible.

See [architecture](architecture.md) for delivery and compilation,
[scope](scope.md) for supported constraints, and [history](history.md) for
the implementation sequence.
