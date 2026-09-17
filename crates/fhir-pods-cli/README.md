# fhir-pods CLI

A local command-line pod over the Rust core, with no dependencies beyond the
core and the standard library. It performs no network calls.

Build or install from the repository root:

```sh
cargo build --release -p fhir-pods-cli
cargo install --path crates/fhir-pods-cli
```

```sh
fhir-pods validate patient.json
fhir-pods validate --outcome < patient.json
fhir-pods lower http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient inputs.json
fhir-pods lift http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient patient.json
fhir-pods lift "" < patient.json
fhir-pods deidentify --policy policy.json patient.json
fhir-pods document ips inputs.json
fhir-pods document au-ps < inputs.json
fhir-pods audit read audit-inputs.json
fhir-pods audit patient-query < audit-inputs.json
fhir-pods needs patient.json
```

Input defaults to stdin; an explicit `-` also means stdin. Each invocation
accepts one JSON input. Put `--` before a positional filename beginning with
`-`. A policy may come from stdin when the resource comes from a file. Two
inputs cannot share stdin. The CLI does not accept a salt as a command-line
option: the policy file has the same shape as the core's policy JSON:

```json
{"salt":"host-supplied-secret","today":"2026-09-16","base":"safe-harbor"}
```

Results go to stdout as one JSON object. `validate --outcome` emits a FHIR
OperationOutcome; default validation emits the pod report. `lower`,
`audit`, `deidentify`, and `document` use the same result shapes as the WASM pod,
including the output resource, validation and operation-specific findings.
De-identification includes both `remaining` (the bounded identifying-field
check) and `residual` (free text kept by the policy). A nonempty `residual`
is reported even when validation passes and `remaining` is empty; it does
not change the exit-code rules below.
`needs` emits both `needs` and `validation`, so an invalid input cannot look
successful merely because it has no terminology questions.

`lift PROFILE_URL` reads a resource into the profile's input slots for `lower`.
Pass an empty argument (`lift ""`) to select a compiled profile declared in the
resource's `meta.profile`. With an explicit URL, the resource is validated
against that profile even when it does not declare it. The result contains
`inputs` (an object, not an encoded JSON string), `validation`, `missing`
(required slots absent) and `extras` (paths not covered by slots, fixed leaves
or pass-through fields). A valid verdict can still include missing slots;
the exit code checks both fields. Nonempty `extras` do not change the exit code;
inspect them before lowering the returned `inputs`, since uncovered values are not
preserved. A missing CLI profile argument is a usage error (exit 2); an unknown
profile, absent compiled declaration with `""`, wrong resource type or malformed
resource is a rejected operation (exit 1, JSON error on stdout).

`audit KIND` builds a BALP basic AuditEvent from host inputs. Kinds are `read`,
`query`, `create`, `update`, `delete`, and the five `patient-` prefixed variants.
For example, `fhir-pods audit read` accepts this JSON on stdin:

```json
{
  "recorded": "2026-09-16T10:00:00Z",
  "client": {"who": "Device/app", "network": "10.0.0.7"},
  "server": {"who": "Device/server", "network": "https://fhir.example.org/r4"},
  "source": {"observer": "Device/audit-collector"},
  "data": "Observation/example"
}
```

For queries, replace `data` with `query` (the original query text; the core
base64-encodes it). Patient variants also require `patient`, such as
`"patient": "Patient/example"`. Optional `user` and `transaction` inputs identify
the actor and request. The result contains `resource` and `validation`.
These BALP basic events describe successful actions; failed actions can be
constructed with `lower http://hl7.org/fhir/StructureDefinition/AuditEvent`.
A missing or unknown CLI kind is a usage error (exit 2); rejected host inputs
or an invalid generated record return exit 1 with a JSON error/report.

| Exit | Meaning |
| --- | --- |
| 0 | The validation verdict is valid. De-identification additionally requires empty `remaining`; lifting requires empty `missing`. |
| 1 | Invalid result, remaining de-identification findings, missing lift slots, or input rejected by the operation. The JSON report/error is on stdout. |
| 2 | Usage, policy configuration, file/stdin read, or output failure. Usage/read/configuration errors are JSON on stderr; an output failure cannot guarantee delivery of an error. |

Warnings do not change the validation verdict. In particular, a successful
`needs` invocation can list unresolved terminology for the host to supply.
The core's de-identification checker has a bounded scope; an empty `remaining`
list is not an independent certification of privacy. Inspect all operation
results before choosing to use the output.

`--help` and `--version` return plain text. No output file is overwritten by
the CLI; use the shell's redirection when desired.
