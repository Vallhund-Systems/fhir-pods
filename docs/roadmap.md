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

## 0.3.x — Terminology, wired for real

- A reference host provider for **NCTS / Ontoserver** (`$validate-code`,
  `$lookup` for displays) in the wasm README and the Expo README — free to
  operate in Australia; the pod never ships the content.
- **Compiled expansions**: `tools/compile_valueset.py` turns a ValueSet
  expansion the host is licensed for into a compact lookup table; a
  `TerminologyProvider` that reads it, so a device answers the bindings
  offline. LOINC / UCUM displays for the codes the registry fixes are
  bundled (licence-free).
- Lowering asks the seam for displays; the verifier asks it for membership.

## 0.4 — Documents (IPS / AU PS)

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

## 0.4.x — De-identification

**What.** `fhir_pods_deidentify(resource, policy)` → a copy safe to leave
the patient's device for analysis, training or support: Safe Harbor by
default (identifiers, names, addresses beyond state, contact details, exact
dates → year, ages over 89 → 90+, device serials, free-text narrative and
notes removed or flagged), references pseudonymised consistently with a
host-supplied salt. Policy is data (which elements, which treatment), typed
traversal already knows what every node *is* (`HumanName`, `Address`,
`ContactPoint`, `Identifier`, `date`/`dateTime`), so the pass is a walk.

**Proof.** A checker: after de-identification, no node of an identifying
type survives outside the allow-list; dates carry only a year; the result
still validates against its profile where the profile permits.

## 0.5 — Audit and provenance

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

lowering → terminology → de-identification → documents (+ IG #4) → audit.
Lowering first because it is what makes the verifier pay for itself; de-id
before documents because it is small and it is a policy commitment a host
can point at. Each pod ships as a minor version with its own release notes,
tests, fuzz targets and, where a reference exists, an oracle.
