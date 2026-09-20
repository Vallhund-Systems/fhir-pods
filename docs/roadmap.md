# Roadmap — the compliance runtime

The product is an embeddable runtime that lets an app speak compliance at
its edges. Its operations share compiled health-data definitions, typed
traversal, terminology, and validation. WASM, Expo, Rust/C, and the CLI are
delivery surfaces for that operation family.

## Shipped capabilities

| Capability | Available since | Current behavior |
| --- | --- | --- |
| Verification | Initial releases | Base R4 structure and supported rules, compiled profiles, extension contexts, terminology, OperationOutcome, and separate must-support inspection |
| Lowering and slot inspection | 0.3 | Build a resource from variable inputs plus the profile's fixed content; return validation |
| Terminology integration | 0.3.1 | Bundled sets, host-loaded expansions, synchronous providers, and an unresolved-needs loop |
| De-identification | 0.4 | Typed policy transformations, pseudonyms, required-element masking, applied actions, remaining findings, and retained free-text reporting |
| Patient summary documents | 0.5 | IPS / AU PS sections, narrative, references, placement/omission reporting, and output validation |
| Audit and provenance | 0.6 | IHE BALP event construction; base AuditEvent and Provenance lowering |
| Lifting | 0.7 | Extract profile slots with validation, missing inputs, and uncovered paths |
| HL7 v2 | 0.8 | Compiled ER7-to-FHIR message conversion with skipped, unplaced, and unmapped input reporting |
| Wider base coverage | 0.9 | Published R4 examples, compiled extension definitions and contexts, and base code-system membership |
| Published validator test lanes | 0.10 | Pinned upstream cases, stored Java/Firely comparisons, and isolated ad-hoc profile compilation |

These rows describe the current behavior of capabilities introduced in each
release; follow-up corrections are recorded in the [changelog](../CHANGELOG.md).
The [README](../README.md#current-capabilities) is the operation overview.

## The latest source work

The changes after v0.10.0 make the published-base lane a regression gate with
hashed evidence and individually reviewed differences. They also correct
primitive-carrier invariant evaluation, extension-owner context checks, and
CLI slot lookup. The CLI package version is aligned at 0.10.0. These changes
are currently unreleased; they retain ABI 10.

The earlier official FHIRPath review is complete for its recorded batch;
its fixes and remaining intentional deviation are in the
[FHIRPath lane](../conformance/fhirpath/README.md). It is no longer the next
unstarted feature.

## Remaining work

Expansion should follow concrete boundary requirements and evidence. The
current areas are:

- **Constraint and compiler coverage.** Resolve reviewed leniencies in the
  published base and validator-test lanes, extend unsupported expressions
  and profile constructs, and keep every new or changed difference visible.
- **Transformation fidelity.** Broaden focused lift/lower, de-identification,
  and v2 content checks. Output validation alone cannot prove the intended
  clinical mapping. Known v2 gaps include multi-resource datatype maps and
  narrative conditions that require interpretation beyond compiled rules.
- **Delivery evidence.** Refresh C ABI coverage, browser measurements, and
  Expo simulator checks for the current operation surface and artifacts.
  Existing measurements remain tied to the versions actually exercised.
- **More inputs and guides.** Add fixtures and implementation guides when
  they exercise needed behavior, with explicit compiler support, terminology
  requirements, and differential evidence before claiming coverage.

The extended fuzz campaign remains deferred. Existing bounded seeded checks
run under `cargo test`; separate libFuzzer targets are available. Neither is
presented as completion of that larger campaign.

## Boundaries owned by the host

The runtime supplies building blocks for an application's compliance
requirements. The host still supplies:

- Domain-field mappings to and from profile slots, and decisions about
  lifted `missing` / `extras` or transformed content.
- Profile and policy selection, including regional and destination-specific
  requirements. The current compiled registry is listed in the README;
  jurisdiction discovery and automatic rule selection are not implemented.
- Consent, authorization, credentials, persistence, and transport. A SMART
  on FHIR client can fetch records and pass them to lift/validate.
- External terminology access and any source-specific v2 dialect handling.
- Audit event context, emission, delivery, and retention.

Future boundary operations should reuse the shared definitions and reporting
model. Each addition needs a concrete input/output contract, coverage fields,
and evidence appropriate to the operation.
