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

**Recommendation: (b) — convert packages → FHIRSchema at build time, codegen from
that**, keeping the *source of truth* as the published FHIR packages. FHIRSchema
is literally "pre-compiled FHIR for validation," which is our whole thesis; it's
the one place to improve *structurally*, not just ergonomically. Risk to weigh:
FHIRSchema is younger than raw StructureDefinitions and we must still prove
bug-for-bug agreement with the reference validator — so FHIRSchema is an internal
IR, never an excuse to diverge from spec.

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
