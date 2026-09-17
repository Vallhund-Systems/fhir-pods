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
fhir-pods deidentify --policy policy.json patient.json
fhir-pods document ips inputs.json
fhir-pods document au-ps < inputs.json
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
`deidentify`, and `document` use the same result shapes as the WASM pod,
including the output resource, validation and operation-specific findings.
`needs` emits both `needs` and `validation`, so an invalid input cannot look
successful merely because it has no terminology questions.

| Exit | Meaning |
| --- | --- |
| 0 | The validation verdict is valid. De-identification additionally requires an empty `remaining` list. |
| 1 | Invalid result, remaining de-identification findings, or input rejected by the operation. The JSON report/error is on stdout. |
| 2 | Usage, policy configuration, file/stdin read, or output failure. Usage/read/configuration errors are JSON on stderr; an output failure cannot guarantee delivery of an error. |

Warnings do not change the validation verdict. In particular, a successful
`needs` invocation can list unresolved terminology for the host to supply.
The core's de-identification checker has a bounded scope; an empty `remaining`
list is not an independent certification of privacy. Inspect all operation
results before choosing to use the output.

`--help` and `--version` return plain text. No output file is overwritten by
the CLI; use the shell's redirection when desired.
