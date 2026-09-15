# Incumbent APIs — the design decision

**Principle: conform on the wire, improve on the ergonomics.** The wire shapes
are FHIR *spec*, not any vendor's API, so conforming is mandatory and stable
(R4→R5). Everything a developer's hands actually touch — setup, result shape,
threading, deployment target — is where the incumbents are painful, and that's
the wedge. This doc records what's fixed, what we improve, the resulting API, and
the two genuinely open decisions.

Research basis: the FHIR spec (`$validate`, `OperationOutcome`, terminology ops,
StructureDefinition), the three SDK incumbents (HAPI/Java, Firely/.NET,
Medplum/TS), the managed servers (Aidbox, Google, Azure), the terminology
licensing regimes, the conformance oracle, and AU Core. Sources inline.

---

## 1. What's fixed — conform (non-negotiable)

These are spec-defined; a validator that diverges isn't trusted.

- **The `$validate` contract.** `POST [base]/{Type}[/{id}]/$validate`, body is a
  `Parameters` with `resource` (0..1), `mode` (`create`|`update`|`delete`),
  `profile` (canonical). HTTP is **always 200**; pass/fail lives in the returned
  **`OperationOutcome`**. (Some servers deviate to 400/422 — we follow the spec.)
- **`OperationOutcome.issue`** — the one result shape. Per issue:
  `severity` ∈ `fatal|error|warning|information` (**1..1**), `code` ∈ the
  **IssueType** vocabulary (**1..1, required**), `details` (CodeableConcept/text),
  `diagnostics` (string), `expression` (**FHIRPath** locator; `location` is
  deprecated). We emit `expression`.
- **IssueType codes** we'll actually produce: `structure`, `required`, `value`,
  `invariant`, `code-invalid`, `extension`, `business-rule`, `not-supported`.
- **Terminology op contracts** — `$validate-code` → `Parameters{result: boolean
  (1..1), message, display}` (R5 adds an `issues` OperationOutcome); `$expand`,
  `$lookup`, `$subsumes` shapes as specified.
- **StructureDefinition / ElementDefinition is the source of truth** — `min`/`max`
  (cardinality), `type` + `targetProfile`, `mustSupport`, `fixed[x]`/`pattern[x]`,
  `binding.strength` (`required`|`extensible`|`preferred`|`example`) + `valueSet`,
  `constraint{key, severity, human, expression}` (FHIRPath invariants), slicing +
  `discriminator`. Binding strength decides error-vs-warning.
- **Correctness oracle:** HL7's reference validator, `validator_cli.jar`
  (`org.hl7.fhir.core`). "Correct" = **bug-for-bug agreement** with it; our test
  strategy is differential testing against its `OperationOutcome` output.

Spec: hl7.org/fhir/resource-operation-validate.html · operationoutcome.html ·
valueset-operation-validate-code.html · profiling.html. Oracle:
github.com/hapifhir/org.hl7.fhir.core.

---

## 2. What we improve — ergonomics (the wedge)

Ranked by how loudly the ecosystem complains. Each is an incumbent pain with a
concrete answer we're structurally positioned to give.

| # | Incumbent pain | Our answer |
|---|---|---|
| 1 | **Startup/warmup**: HAPI ~20 s / ~1 GB to load core conformance on first validate; Firely "download a package + remember `CachedResolver` or it's slow" | **Precompiled conformance baked into the binary** (the codegen thesis) → ms validation, tiny footprint, zero warmup |
| 2 | **No browser/edge/on-device** except Medplum — and Medplum's client-side validation is the *validation-light* one (weak terminology, throws) | Rust → **WASM + native** gives HAPI/Firely-grade rigor client-side and on device — the unoccupied square |
| 3 | **Result shape**: HAPI `SingleValidationMessage`, Firely `OperationOutcome`, Medplum **throws on first error** | **Always return a complete typed `Report`** (never throw to signal validation state); `to_operation_outcome()` on demand |
| 4 | **Verbose/fragile wiring**: `ValidationSupportChain(...)`, resolver+terminology+cache+package-source dance, easy footguns | `Validator::new()` with **sane offline defaults** + one `.validate(&resource)` call; `.add_profile(url)` |
| 5 | **Thread-safety**: Firely's validator historically **isn't** (races under load) | `Send + Sync` by construction |
| 6 | **Terminology = network dependency**: `$validate-code` round-trips to tx.fhir.org; offline, correctness silently degrades; **patient codes leave the device** | **Bundle LOINC + UCUM offline**; SNOMED via a pluggable licensed backend (§4) |
| 7 | **IG/package plumbing reinvented** everywhere (fetch `.tgz`, resolve deps, snapshots) | Done **once at build time** in codegen; the shipped binary fetches nothing |
| 8 | **Per-vendor config sprawl, no portable primitive** — Google `enabledImplementationGuides`+import, Azure upload SDs + pre-expanded ValueSets + header, Aidbox mid-migration | **The library *is* the portable "validate instance X against IG Y" primitive**, decoupled from any server |

Steal the *good* parts: HAPI's rich per-issue locators (FHIRPath **+ line/col +
stable `messageId`**), Firely's explicit `addProfile(url)` and `OperationOutcome`
interop.

---

## 3. The API shape (decided)

**Two layers — conform at the boundary, improve internally.**

```rust
// Ergonomic layer (the 90% case) — typed, never throws, complete issue list.
let v = Validator::new();                 // sane offline defaults, no wiring
let report = v.validate(&resource);       // -> Report (all issues, always)
report.is_valid();                        // no Error/Fatal issues
v.add_profile("http://hl7.org.au/fhir/core/StructureDefinition/au-core-observation");

// Wire layer (at a boundary only) — spec interop.
let outcome_json = report.to_operation_outcome();   // FHIR OperationOutcome
```

**`Issue` (final shape), and how it maps to `OperationOutcome.issue`:**

| `Issue` field | → `OperationOutcome.issue` | Notes |
|---|---|---|
| `severity: Severity` | `severity` | **add `Fatal`** → `fatal\|error\|warning\|information` |
| `code: IssueType` | `code` | **new field** — required by the wire (`required`, `value`, `invariant`, `code-invalid`, …) |
| `expression: String` | `expression[]` | already a FHIRPath in our `path` |
| `message: String` | `details.text` / `diagnostics` | human message |
| `message_id: Option<String>` | (HAPI parity) | **new** — stable machine id for suppression/dedupe |
| `line: Option<u32>`, `col: Option<u32>` | (HAPI parity) | **new** — source location for editor/CI use |

So our current `Issue{severity, path, message}` grows by: `Fatal` severity, a
required `code: IssueType`, and optional `message_id`/`line`/`col`. Small,
mechanical — not a redesign. That's what "conform on the wire" costs.

**Implemented (2026-09-15):** `validate_bytes() -> Validation` (typed, never
throws) and `to_operation_outcome()` (the spec resource) — with `Fatal` and the
required `code`. `message_id` / `line` / `col` are deferred (the tree adapter
carries no source positions yet).

---

## 4. Terminology architecture (forced by licensing)

The licensing facts are hard constraints, not preferences:

- **UCUM** — most permissive. Embeddable royalty-free (retain notice). **Full
  offline unit-syntax validation ships in the crate.** (ucum.org/license)
- **LOINC** — embeddable royalty-free (retain notice, don't modify the table,
  translations assign to Regenstrief). **LOINC codes + displays ship in the
  crate.** (loinc.org/license)
- **SNOMED CT** — **cannot** be embedded/redistributed in a globally-shipped
  library. Affiliate licensing; free only in Member countries (AU is one); fees +
  annual MLDS elsewhere; sublicensing obligation to end users. → **SNOMED is a
  pluggable, license-gated backend, never a bundled table.** (snomed.org/licensing)

Design that falls out:

```rust
pub trait TerminologyProvider {  // already stubbed in core
    fn validate_code(&self, value_set_url: &str, system: &str, code: &str) -> bool;
}
```

- **Default (offline, in-crate):** structural + FHIRPath + **LOINC/UCUM** checks,
  fully offline, no patient codes leave the device.
- **SNOMED-bound elements:** resolved by a pluggable provider — for AU, the
  **NCTS / Ontoserver** (`tx.dev.hl7.org.au/fhir`; SNOMED CT-AU + AMT + LOINC,
  free for AU end use), or a user-supplied licensed release, or tx.fhir.org under
  its terms.
- **No provider present:** SNOMED binding checks degrade to `warning` (the
  reference validator's `-tx n/a` behavior) — a deliberate, spec-aligned choice,
  not silent breakage.

This is also a *feature* vs the incumbents: everyone else punts on offline
terminology; we bundle the two we legally can and make the third a clean seam.

### AU specifics + the fixed-code refinement (2026-09-15)

Two sharpenings from inspecting the real AU Core v2.0.0 profiles:

- **Fixed codes need no licence.** Much of AU Core's SNOMED usage is *fixed
  codes*, not ValueSet bindings — e.g. `au-core-heartrate` fixes
  `Observation.code.coding.code` to SNOMED `364075005`. A fixed value is an
  **exact-match structural check**, not a set-membership query, so it needs no
  terminology server and no SNOMED licence. Only extensible/required **ValueSet
  bindings** hit the seam — and many of those are small base-FHIR value sets
  (`observation-status`, `administrative-gender`) we bundle, or LOINC-based.
- **In Australia the SNOMED seam is free to operate.** AU is a SNOMED
  International Member; SNOMED CT-AU + AMT are distributed free to registered AU
  users via **NCTS / CSIRO Ontoserver** (public sandbox tx at
  `tx.dev.hl7.org.au/fhir`). So the licensed seam costs nothing to run for the AU
  market — a materially better "AU-ready" story than round-tripping every code to
  a paid tx server.

**The NCTS licence — resolved (2026-09-15).** The binding instrument is the
**Australian National Terminology Licence Agreement (ANTLA, DH-2400:2016)**, and
it settles the crux: there is **no separate "distributor" tier**. The single
national licence — taken by any SNOMED Affiliate who registers with NCTS —
*already* grants the right to **bundle SNOMED CT-AU + AMT into a product and
sub-license end users** (ANTLA cl. 2.1.3 / 2.1.5), and SNOMED CT-AU use in
Australia is **free** (cl. 7.1). So bundling is *permitted*, not prohibited — the
question is whether we want the obligations.

**Decision for fhir-pods: default to the seam; the library ships no SNOMED
content.** Becoming an Affiliate drags in real, ongoing obligations —
attribution notice on all media (cl. 8.3.1), version/date stamping (8.3.2), a
maintained **sub-licence register** (8.7), a 30-day release-update cadence (6.2),
namespace ID for extensions (3.1), ADHA notification for non-AU use (9.1.1), and
MLDS fees for non-Member deployments. Too heavy to bake into a globally
distributed library. So SNOMED CT-AU/AMT expansions + code validation resolve at
runtime against a **licensed NCTS/Ontoserver endpoint the deployer holds** —
which, for AU, is free to run. (Bundling stays *available* as a deliberate
Affiliate-route option if a product later wants fully-offline SNOMED.)

**Bundle-safe worldwide, no SNOMED dependency:** CC0 base-FHIR value sets, LOINC
(with notice + version stamp), UCUM (with notice), and **fixed SNOMED codes as
structural constants (`system` + `code`)** — a reference, not redistribution.
**Generator rule (for the fixed-code work):** emit a fixed SNOMED coding's
`system` + `code` only — **never its display string**; the code is a reference,
the display is licensed content.

**Two items need a written NCTS/ADHA ruling before any public "AU-ready"
redistribution claim:** (1) whether shipping a fixed SCTID — and especially its
display — inside a conformance resource is treated as licence-free reference
(universal FHIR practice says the code is fine and the display is the risky part,
but it's not black-letter); (2) whether one ANTLA held by the library could serve
as the sub-licence chain for many AU deployers. Primary text: ANTLA DH-2400:2016
(healthterminologies.gov.au).

---

## 5. Two open decisions

### 5a. The codegen IR — raw StructureDefinition vs FHIRSchema  ← the real fork

The expensive, divergence-prone step in every validator is **snapshot
generation** (merging `differential` → `snapshot` across the IG dependency
graph). Two ways to feed `build.rs`:

- **(a) Raw StructureDefinitions** → we implement snapshot generation ourselves.
  Maximum fidelity to source, but we re-solve the hardest, most bug-prone part.
- **(b) FHIRSchema as the IR** — an open, JSON-Schema-like, **validation-optimized
  re-expression** of StructureDefinitions (elements as named properties with
  inline types, first-class arrays, differential schemas resolved by
  `collect`/`follow`). It **eliminates runtime snapshot logic**, and there's a
  `fhir-schema-codegen` precedent. (fhir-schema.github.io/fhir-schema · Aidbox
  ships it as their fast engine.)

**Resolved (2026-09-15): (a), sharpened — no FHIRSchema dependency.** Diligence
on FHIRSchema found a **single-vendor (Health Samurai), pre-1.0, non-HL7** project:
no frozen/versioned spec (zero tags), the canonical spec repo **dormant ~18 months**
(last commit "start algo draft"), **only Aidbox** as a named production user, and
the "independent" ports written by the same contributor circle. That fails the
"is the format stable enough to build on" bar. And it turns out we don't need it:
the **published packages already ship computed `snapshot.element`** (verified on
AU Core v2.0.0 — profiles carry 86–90 elements `from snapshot`), so option (a) is
**not** the feared snapshot-merge implementation — it's **codegen straight from the
shipped snapshot into our own small IR**. We borrow FHIRSchema's good design ideas
(tree-shaped, arrays-first) as *prior art* and depend on none of its tooling.
Source of truth = the HL7 / AU Core packages; oracle = `validator_cli.jar`.
(Snapshot generation is deferred to the "only if a future package ships
differential-only" case.)

### 5b. FHIR version — R4 first

AU Core v2.0.0 is **R4**, and R4 is where the ecosystem's weight is. Build R4
first; keep the IssueType/OperationOutcome shapes R5-ready (they're stable), and
treat `$validate.profile` `uri`→`canonical` + R5's `usageContext` as additive.

---

## 6. Build-time acquisition (for the codegen build item)

- **AU Core v2.0.0**, package `hl7.fhir.au.core` (R4), deps `hl7.fhir.au.base` →
  `hl7.fhir.r4.core`.
- FHIR packages are **npm-style gzipped tarballs** on `packages.fhir.org`:
  `GET https://packages.fhir.org/hl7.fhir.au.core/2.0.0` → `.tgz`; inside, a
  `package/` dir with `package.json` (deps), `.index.json`, one JSON per
  conformance resource.
- **Recipe:** resolve the dep graph → fetch each `.tgz` → unpack → (convert to
  FHIRSchema per 5a) → codegen Rust types + validators. Precedents:
  `FHIR/fhir-package-loader`, `fhir-schema-codegen`. **Pin the exact version.**
- The `Observation` slice we hand-wrote is the reference the generated
  `au-core-observation` validator must reproduce (and then exceed:
  `au-core-diagnosticresult`, vitals variants, etc.).

---

## 7. Conformance strategy

- **Differential-test against `validator_cli.jar`** — the oracle. "Passing" for
  the *library* = our `OperationOutcome` matches its `OperationOutcome` on a
  corpus of AU Core instances (valid + deliberately broken).

  **First oracle run (2026-09-15): 9/9 verdict agreement** (`conformance/run.py`,
  JDK 17, `-tx n/a`) — heart-rate including every slicing case (missing SNOMED
  slice, wrong UCUM code, closed-slicing rejection of `valueString`, missing
  mandatory) and patient including the nested `name.use` binding. The one
  transient miss was a *fixture* defect: the reference validator **rejects
  `example.org` URLs by policy**, so a "valid" patient with an example MRN system
  failed — fixed by using a realistic AU MRN (type `MR` + OID system + assigner).
  Two real backlog items surfaced:
  1. **`type.profile` choice conformance.** AU Core `Patient.identifier` must
     conform to *one of* the AU Base identifier profiles (`au-ihi`, `au-medicare`,
     `au-medicalrecordnumber`, …, or plain `Identifier`). We don't model
     "conforms to at least one of these profiles" yet — it needs the
     `hl7.fhir.au.base` datatype profiles generated plus one engine rule.
  2. **The example-URL policy.** A best-practice check, not spec — decide whether
     to mirror it behind a policy flag for full bug-for-bug parity.
  **Second pass, with the wire layer (2026-09-15): issue-level parity, 9/9.**
  Diffing our `OperationOutcome.expression`s against the oracle's (normalized)
  first showed one convention gap, not a defect: the reference validator
  **locates a missing element at its parent** (`Patient.gender` missing →
  `Patient`; a missing slice → the sliced element's parent), and writes choice
  slices as FHIRPath (`value.ofType(Quantity)`). The wire layer now follows
  both — `wire::expression` — while the ergonomic `Issue.path` keeps the
  precise labelled location. After that: same verdicts *and* same flagged
  elements on all 9. Next widening: more profiles in the corpus.
- **Inferno** (ONC g10) and **Touchstone** are **API-behavior** harnesses
  (SMART launch, Bulk Data, search, US Core over the wire) — relevant only if we
  later ship a *server* surface, not for the embeddable validator. Note them,
  don't target them yet.

---

## Bottom line

Conform on the wire is a **small, mechanical mapping** (add `Fatal` + `code`, emit
`OperationOutcome`), because the wire is spec, not vendor. Improve on ergonomics
is **eight concrete wins** we're uniquely positioned for, anchored by the two
things only we do: **precompiled conformance in an embeddable binary** and
**offline LOINC/UCUM with a clean SNOMED seam**. The one decision that shapes the
next build item is **FHIRSchema as the codegen IR** — recommended.
