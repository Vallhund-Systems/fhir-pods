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

## 0.5 — Documents (IPS / AU PS) — shipped

**What.** `fhir_pods_document(kind, inputs)` → a document `Bundle`: a
`Composition` whose sections come from the kind's compiled Composition
profile (the section slices' LOINC codes), each resource placed by what it
is, a plain xhtml table per section generated from what the resource
carries, references rewritten to the entries they name (`Type/id`, a
host's own `fullUrl`s, `urn:uuid:id`), resources nothing reaches left out
and reported, a required section with nothing in it marked `emptyReason:
unavailable` (no clinical fact is ever invented), and the document
validated against the kind's Bundle profile on the way out. Kinds: `ips`
(IPS 2.0.1) and `au-ps` (AU Patient Summary 1.0.0).

**What it took in the verifier.** IG #4 (IPS 2.0.1 + AU PS 1.0.0, with the
extensions package for what their slices inherit; registry 218 profiles);
Bundle entries discriminated by resource type and by conformance to a
profile (a validation pass inside the same bundle); slicings declared
inside a slice (`section:sectionProblems.entry`) compiled and evaluated
one level down, their entries discriminated by what a reference resolves
to; `resolve()` and `&` in FHIRPath; the reference validator's "matches
more than one slice" error; `pattern[x]` / `fixed[x]` objects on plain
elements and slice sub-elements enforced (they were only discriminators
before — a laboratory Observation passed IPS's tobacco-use profile); the
base spec's minimums inside datatypes (`Narrative.status`,
`Extension.url`) checked profile or no profile; and the document rules the
reference applies — the Composition's references resolve in the bundle,
every entry is reachable from the Composition.

**Proof.** The IGs' twelve document examples against the oracle (four of
them the reference itself rejects without a terminology server, and so do
we: IPS's two pregnancy profiles differ only by value-set membership), and
every one of them taken apart and assembled again by the pod, the result
in front of the oracle with our verdict — `examples/document_corpus.rs`,
run by the harness. Out of scope, stated: `Bundle.signature`
verification; slicings nested three levels deep (none in these IGs carry
a verdict).

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

## 0.6 — Audit and provenance — shipped

**What.** `fhir_pods_audit(kind, inputs)` → an `AuditEvent` conformant to
the IHE Basic Audit Log Patterns profile for that kind of event — `read`,
`query`, `create`, `update`, `delete`, each with or without a known
patient — from what a host knows at the moment it happens:

```json
fhir_pods_audit("patient-read", {
  "recorded": "2026-09-16T10:00:00Z",
  "client": {"who": "Device/app-7", "network": "10.0.0.7"},
  "server": {"who": {"display": "fhir.example.org"}, "network": "https://fhir.example.org/r4"},
  "user": {"who": "Practitioner/gp1", "name": "Dr Who"},
  "source": {"observer": "Device/audit-collector", "site": "clinic-1"},
  "patient": "Patient/p1", "data": "Observation/hr1",
  "transaction": "3f2a1c9e-…"
})
→ { "resource": { …the AuditEvent, the profile's types and codes filled… },
    "validation": { "valid": true, "issues": [] } }
```

The correlation with the trace is the request id, as BALP's own
`X-Request-Id` entity — the design in `architecture.md` (audit is data,
traces are plumbing, correlated by id). `Provenance`, and a failed action's
`AuditEvent`, are lowered from the base spec's own definitions, now
compiled as profiles with every element an input.

**How.** IG #6 (BALP 1.1.4) compiled like the others; the audit surface is
a host-shaped front on the lowering engine — it names the profile, maps
the host's inputs to its slots, classifies network addresses, encodes the
query — and the engine supplies everything the profile fixes and validates
the result. Proof: every BALP example re-lowered and de-identified, and
one record per kind with and without a user put before the reference
validator (1187/1187).

**What it took.** Four lowering-engine gaps (gated fixed leaves, typed fixed
values, spare cardinality on a sliced container, sibling-slice
suppression) and one verifier gap (boolean patterns were never enforced),
all found by the new IG's corpus — the pattern every IG has followed.

## Alongside, host-side (not pods)

- **SMART on FHIR client** — standalone launch + PKCE, patient-access
  scopes, `.well-known/smart-configuration`; the on-ramp for "bring your
  records in". Fetch → validate (pod) → lift into the host's model.
- **Lifting** — the reverse map, FHIR → host model, gated by the verifier.
- **HL7 v2** — only on demand: a v2 parser is small and pod-shaped; the
  `ORU^R01` / `ADT` maps are integration work per source system.

## Sequence

lowering → terminology → de-identification → documents (+ IG #4) → audit
(+ IG #6) — all shipped, 0.3.0 → 0.6.0. Next in the core: the eleven
official FHIRPath disagreements (planning T023); then the host-side pieces
below as they are asked for.
Lowering first because it is what makes the verifier pay for itself; de-id
before documents because it is small and it is a policy commitment a host
can point at. Each pod ships as a minor version with its own release notes,
tests, fuzz targets and, where a reference exists, an oracle.
