# Roadmap — the pod family

The verifier was the first pod, not the product. "FHIR at the edges" needs a
small family of pure functions behind the one C ABI, none of them a server:

```
              lower                validate               assemble
host record ───────► FHIR resource ─────────► verdict ───────────► document (IPS / AU PS)
                       │                                              │
                       └──── de-identify ────► training-safe copy     └──► share / export
```

Every pod keeps the rules the verifier established: `no_std` core, zero
dependencies, no `unsafe`, deterministic, fuzzed, bug-for-bug against a
reference where one exists, skip-never-guess, no licensed terminology
shipped. Domain-specific maps (a host's own records → FHIR) live in the host,
never here; the pods know FHIR, the host knows its data.

## 0.3 — Lowering (the transformer) — shipped

**What.** Build a conformant resource from the *variable* data only. A
compiled profile already fixes most of a resource — the LOINC + SNOMED
codings, the category, the UCUM system, the slices — so the host should not
have to know any of it:

```json
fhir_pods_lower("…/au-core-heartrate", {
  "subject": "Patient/1", "effectiveDateTime": "2026-09-16T08:00:00+10:00",
  "value": 72, "status": "final"
})
→ { "resource": { …the full Observation, fixed parts filled… },
    "validation": { "valid": true, "issues": [] } }
```

**How.** The generator emits, per profile, a *skeleton* (the JSON implied by
every fixed / pattern value and every required slice) and its *input slots*
(the required and must-support leaves that carry no fixed value, each with
its FHIR type and, for a choice, its allowed keys). The engine merges the
inputs into the skeleton, coerces scalars by type (a number into a
`Quantity.value`, a string into a `dateTime`, a reference string into
`Reference.reference`), and validates the result with the same `Validator`.
Unknown input keys and type mismatches are issues in the returned
validation, never silently dropped. Inputs that need terminology (a code
with a display) go through the seam.

**ABI.** `fhir_pods_lower(profile_url, inputs_json)` → `{resource,
validation}`; `fhir_pods_inputs(profile_url)` → the slot list, so a host can
generate its own typed bindings. Pods: `lower(profileUrl, inputs)` in JS /
Swift / Kotlin.

**Proof.** Every publisher-valid corpus example is re-lowered from the
values at its slots (`examples/lower_corpus.rs`, run by the harness) and the
result is put in front of the reference validator alongside the corpus.

## 0.3.1 — Terminology, wired for real — shipped

- **Bundled base sets, generated.** Every value set the registry binds with
  `required` strength that expands offline from the packages' own compose
  (the FHIR and HL7 v2/v3 sets: 59 sets, 311 codes) — `tools/gen_valuesets.py`.
  Licensed or hosted-elsewhere sets (SNOMED, LOINC, VSAC, NCTS) are skipped,
  never approximated, and named in the generated file.
- **Compiled expansions**: `tools/compile_valueset.py` turns `$expand`
  responses into a line table; `terminology::Expansions` reads it; the ABI
  loads one (`fhir_pods_terminology_load`) for every call after.
- **Ask, then answer**: `terminology_needs` / `fhir_pods_terminology_needs`
  lists what the bundled sets and the table could not decide — the loop a
  host runs against NCTS / Ontoserver (`$validate-code` or `$expand`), with
  the reference calls in the wasm and Expo READMEs.
- Lowering asks the seam for displays (the bundled code systems' displays
  for the codings the registry fixes; a table's displays for the rest); the
  verifier asks it for membership.

## 0.5 — Documents (IPS / AU PS)

**What.** `fhir_pods_document(kind, resources[])` → a document `Bundle`:
`Composition` with the standard sections (problems, medications, allergies,
results, vital signs, procedures, immunizations, plan of care…), each
resource in its section, narrative generated (a plain xhtml table per
section, since `Composition` and `dom-6` want it), validated against the
IPS / AU PS Bundle profile.

**Needs.** IG #4 — `hl7.fhir.uv.ips` + `hl7.fhir.au.ps` — in the registry,
and two engine features: `Bundle.entry` slicing by *profile* (a slice whose
discriminator is "conforms to profile X" — the verifier already validates
against a profile, so the discriminator is a validation pass), and
intra-document reference checks (`Composition.section.entry` must resolve
inside the Bundle — the `resolve()` we could not do for a lone resource is
well-defined inside a document).

**Proof.** The IGs' own document examples, plus assembled documents from the
registry's examples, against the oracle.

## 0.4 — De-identification — shipped

**What.** `fhir_pods_deidentify(resource, policy)` → a copy safe to leave
the patient's device for analysis, training or support: Safe Harbor by
default (names, contact details, addresses below state, exact dates → year,
ages over 89, narrative, notes, attachments, signatures, device serials
removed; identifiers and references pseudonymised with a host-supplied salt
through HMAC-SHA256, consistently across resources). Policy is data — a
base plus rules by type, type member or element path, most specific wins —
and the walk is typed, so what a node *is* decides. Where the resource's
profile requires an element that is removed, a `data-absent-reason: masked`
stub stands in, so the copy stays conformant wherever the profile permits;
an element emptied by what was removed inside it goes too; an element the
base spec does not define cannot be typed and is removed, recorded.

**Proof.** A checker (`deidentify::check`) finds nothing in the copy —
identifying types other than masked stubs, address members below state,
dates finer than a year, instants, reference displays, ages over 89 — and
every publisher-valid corpus example is de-identified and put in front of
the reference validator with our verdict on the copy's conformance
(`examples/deid_corpus.rs`, run by the harness). Fuzzed: a seeded arm and a
libFuzzer target assert the check holds for every copy produced.

**Not covered by the default policy**, by design: free text in `string`
elements other than narrative and notes (`Observation.valueString`, a
`CodeableConcept.text`) — a host names those in its policy; dates the
profile needs to the day (vital signs' `effective[x]`, invariant vs-1) are
a conformance finding the host decides on (keep the one date, or accept
the finding).

## 0.6 — Audit and provenance

`AuditEvent` and `Provenance` built through the lowering engine from the
base spec's own definitions (the registry gains the base resources it needs
as "profiles"), so a host can emit correct audit records for every read and
write of a record — the design in `architecture.md` (audit is data, traces
are plumbing, correlated by id).

## Alongside, host-side (not pods)

- **SMART on FHIR client** — standalone launch + PKCE, patient-access
  scopes, `.well-known/smart-configuration`; the on-ramp for "bring your
  records in". Fetch → validate (pod) → lift into the host's model.
- **Lifting** — the reverse map, FHIR → host model, gated by the verifier.
- **HL7 v2** — only on demand: a v2 parser is small and pod-shaped; the
  `ORU^R01` / `ADT` maps are integration work per source system.

## Sequence

lowering → terminology → de-identification → documents (+ IG #4) → audit
(the first three shipped 0.3.0 → 0.4.0).
Lowering first because it is what makes the verifier pay for itself; de-id
before documents because it is small and it is a policy commitment a host
can point at. Each pod ships as a minor version with its own release notes,
tests, fuzz targets and, where a reference exists, an oracle.
