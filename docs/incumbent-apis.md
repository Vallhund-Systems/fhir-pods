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
  elements on all 9.

  **Third pass, widened corpus (2026-09-15): 79/79.** The corpus is now derived
  from the pinned package's own **64 publisher-validated example instances**
  (every one of the 25 profiles, fetched into a gitignored cache so the
  examples' SNOMED display terms never enter the repo) plus 15 deliberately
  broken instances, validated in **one oracle JVM** (a `Bundle` of
  `OperationOutcome`s tagged by source file). Two disagreements surfaced and
  were both real:
  1. **A general engine bug — `_name` primitives.** FHIR JSON represents a
     primitive carrying only extensions as its `_name` sibling with no value
     (`"_birthDate": {"extension": […]}`). That element *is present* and
     satisfies `min`; we only looked at the value key, so we rejected a valid
     patient. Fixed in the document adapter (`json::children` resolves through
     `_name` when the value key is absent), so it holds for every primitive in
     every profile.
  2. **Bundled value sets.** `Observation.value.code` on body weight is
     required-bound to `ucum-bodyweight`; we warned instead of erroring. The four
     vitals UCUM sets are now bundled, concept lists taken verbatim from
     `hl7.fhir.r4.core#4.0.1`. Follow-up: **generate** bundled base-FHIR value
     sets from the `r4.core` package rather than hand-listing them.
  After both: same verdicts *and* same flagged elements on all 79.

  **Fourth pass, feature probes (2026-09-15): 79/83 — by design.** Four
  mutations each aimed at a feature we defer; the oracle named the rule:
  1. BP with a wrong diastolic code → `Slice 'Observation.component:DiastolicBP'
     … required, but not found`. Diagnosed on inspection — the first guess
     (object patterns) was wrong: the component slices are `value`-discriminated
     on `code.coding.code` / `.system` (scalar paths), but the *values* live
     inside a **nested** slice (`component:SystolicBP.code.coding:SBPCode.code`),
     which the generator discarded. **Fixed, generator-only:** discriminator
     values now resolve through nested slice labels. The MedicationRequest /
     Statement deferrals are *not* this: they are genuinely nested (`coding`
     sliced by `pattern $this` inside the type-slice) plus `supportingInformation`
     discriminated by `profile resolve()` — reference resolution, which nothing
     can do offline.
  2. No value and no dataAbsentReason → `vs-2` — **FHIRPath invariants**;
     `constraint[]` is not in the IR. A minimal FHIRPath core (`exists` /
     `empty` / `and` / `or` / `implies` / navigation) covers this class.
  3. An IHI failing Luhn → `inv-ihi-value-2` (from `au-ihi`) — FHIRPath with
     string/arithmetic **plus the `type.profile` choice** into the `au.base`
     profiles. The largest piece.
  4. `subject` referencing a Device → the oracle says **valid**: it does not
     enforce reference target type offline. The probe's expectation was wrong,
     not the engine — flipped to *expect valid* as a guard against
     over-enforcing what the reference doesn't.
  **Build order, by evidence: nested-label discriminators (done) → FHIRPath core
  → `type.profile` + `au.base`.**

  **Fifth pass (2026-09-15): 81/83.** The BP probe now agrees with the oracle,
  and the `subject → Device` guard agrees (both valid). The two remaining
  disagreements are exactly the two FHIRPath-invariant probes (`vs-2`, the IHI
  Luhn) — the next build is the FHIRPath core.

  **Sixth pass, FHIRPath core (2026-09-15): 82/83.** `constraint[]` is now in
  the IR (`ProfileInvariant`, one per non-sliced element incl. the root;
  `ele-1`/`ext-1` excluded as JSON-shape rules), evaluated with each instance
  of its element as `$this` by a deliberately small FHIRPath core — navigation
  with a choice-type fallback, `exists`/`empty`/`not`/`count`/`first`/
  `hasValue`/`where`, `and`/`or`/`xor`/`implies`, `=`/`!=`, literals — with the
  spec's three-valued logic (empty is unknown; only a definite `false` fails).
  Anything outside the core fails to parse or evaluates `Unsupported` and the
  invariant is **skipped, never guessed**. The `vs-2` probe flipped to
  agreement, and — the real test — the invariants the core *did* evaluate
  produced **no false positives on the 64 valid examples**. The one remaining
  disagreement is the IHI Luhn: it needs string/arithmetic FHIRPath *and* the
  `type.profile` choice into `au.base` — the next build.

  **Seventh pass, `type.profile` + `au.base` (2026-09-15): 83/84.** On
  inspection the element-level `type.profile` list is nearly a no-op (base
  `Identifier` is always among the choices), so the mechanism that matters is
  the **slicing**: `Patient.identifier` is sliced by `pattern` on `type`, and
  each slice *names an external profile* (`au-ihi`, `au-medicarecardnumber`,
  `au-dvanumber`) that carries the discriminating `type` pattern, the fixed
  `system`, and the invariants. Design — recursive and honest: a `Slice` may
  name a `profile` (canonical URL); matched instances are validated against
  that compiled profile from the registry with the instance as the root, and
  its issues are relabelled onto the slice. The generator resolves a slice's
  discriminator in order — its own scalar fixed → a nested slice's fixed → the
  extension URL → its own object pattern (flattened) → the referenced profile's
  pattern/fixed at the discriminator path (flattened) — and **defers the slice
  if none resolves**: `:ihi` and `:medicare` resolved (`NI` / `MC`); `:dva`
  stayed deferred because neither `au-core-patient` nor `au-dvanumber` carries
  a `type` pattern — nothing to resolve, nothing invented. `gen_all` now
  indexes every package for URL resolution and generates the `Identifier`
  datatype profiles from `hl7.fhir.au.base` (36) alongside the 25 AU Core
  resource profiles: 61 in the registry. A wrong-system IHI probe flipped to
  agreement (caught by `au-ihi`'s fixed system, no invariant needed); all
  five valid Patient examples still agree with slices live; the bad-Luhn IHI
  still disagrees *correctly* — its invariant parses `Unsupported` and is
  skipped. **The last gap is FHIRPath tier two** (`substring`, `toInteger`,
  arithmetic, `mod`, `iif`, `select`).
- **Pass 8 — FHIRPath tier two → 84/84 (2026-09-15).** Hardened first
  (depth-bounded parsers, bounded slice→profile inheritance, a seeded
  zero-dep fuzz harness under `cargo test` plus libFuzzer targets; 14M
  iterations clean across byte soup, hostile primitives, mutated real IG
  examples and random FHIRPath), then widened the evaluator: comparison
  (`< <= > >=`, integers and strings), integer arithmetic (`+ - * div mod`,
  checked; `/` stays out — decimals), unary minus, string `+`, and
  `substring`, `length`, `startsWith`, `toInteger`, `toString`, `select`,
  `all`, `iif`, `last`, `extension(url)`, and delimited identifiers
  (`` text.`div` ``). `$this` may now be a scalar, which is what the Luhn
  invariants need (`select(value.substring(n,1).toInteger()).select(iif($this<5,
  …))`). **53 of the 57 distinct invariant expressions in the registry now
  parse**; the four that don't (`is dateTime`, `%resource`, `descendants()`,
  `intersect`) stay skipped, as do `matches`, `memberOf`, `ofType` at
  evaluation. `patient-ihi-bad-luhn` flipped to agreement and nothing else
  moved: **84/84 with zero issue-path differences**. Side effect: `dom-6`
  (narrative) now evaluates and warns, matching the reference validator's
  best-practice warning.
- **Pass 9 — the pods (2026-09-15).** ABI 1 (`fhir_pods_validate` /
  `fhir_pods_operation_outcome`, the two API layers over one memory
  contract), driven from Rust, from C, from Swift (`swiftc` against the C
  module + host staticlib) and from JS. **wasm pod:** 170 KB (49 KB gzipped)
  with all 61 profiles, a ~40-line dependency-free wrapper, all 84 corpus
  files through it, ~30k validations/s in Node (33 µs). **expo pod:** packed
  and installed as a vendored tarball into a fresh Expo SDK 57 app —
  autolinks, builds, answers on an iPhone 17 simulator at ~22k validations/s
  (45 µs incl. `JSON.parse`), and on a Pixel 8 (Android 15) emulator at
  ~19.6k (51 µs) through a Kotlin wrapper over an Android-only JNI shim in the
  same crate (`ByteArray` in/out; the `.so` is ~560 KB stripped, 16 KB
  page-aligned, built with plain cargo + the NDK clang — no `cargo-ndk`). The
  "almost free" thesis is now a measured number on all three delivery
  targets, not a hope.
- **Pass 10 — a second IG: US Core 6.1.0 → 271/271 (2026-09-15).** The
  corpus builder, oracle runner and generator now take a list of IGs; the
  registry holds 156 profiles (AU Core 25 + US Core 49 resource profiles, plus
  the `Identifier` *and now complex `Extension`* datatype profiles from every
  package). US Core's 174 profile-declaring examples came in **agreeing
  first time** (0 false rejections); the oracle's only complaints were its
  own CLI policy lint on `example.org` URLs in published examples — switched
  off with `-allow-example-urls true` so the oracle scores conformance, not
  hygiene. Thirteen planted US Core probes steered three changes: (1) the
  slice element's *own* `patternCodeableConcept` now resolves a `pattern
  $this` discriminator (US Core's `category:us-core` / `category:laboratory`
  idiom, absent from AU Core); (2) complex `Extension` profiles are compiled
  so an extension slice inherits them (`Patient.extension:race` →
  `us-core-race`, whose `extension:text` is 1..1); (3) one expectation was
  mine, not the spec's — `MedicationRequest.requester` is must-support, not
  required, in 6.1.0; flipped to an over-enforcement guard. Result:
  **271/271, one informational path difference** (the oracle additionally
  flags a *display-name* mismatch on a code we don't bundle displays for —
  consistent with the terminology stance). `.wasm` grew 170 → 262 KB (49 →
  66 KB gzipped) for the second IG.
- **Pass 11 — scope note + the terminology seam (2026-09-15).** `docs/scope.md`
  states what 271/271 means (error-level verdict + error paths) and, family by
  family from the oracle's 868 non-error issues, what it checks that we don't:
  terminology without a server (~700), display names, best-practice
  recommendations, multiple matching profiles, advisories, attachment
  fetching. Then the seam: `TerminologyProvider` is now real —
  `Membership::{Member, NotMember, Unknown}`, `Bundled` first, a host provider
  chained behind it and asked only what the bundle can't answer; `Unknown`
  stays a warning. Bindings are compiled by kind (`code` / `Coding` /
  `CodeableConcept`, 231 across the registry — before, only `code` elements
  were bound). ABI 2 adds `fhir_pods_validate_tx` /
  `fhir_pods_operation_outcome_tx` with a C callback; wasm takes the provider
  as an import the wrapper supplies (JS closure); the expo pod takes a native
  closure (Swift) / `fun interface` (Kotlin, via JNI `call_method`) installed
  once by the app — never a JS callback, because the verifier is synchronous.
  Proven from Rust, C, Swift (swiftc), JS (node suite 8/8), and on both
  simulators. Oracle unchanged at 271/271.
- **Pass 12 — must-support as information, not conformance (2026-09-15).**
  Each compiled profile now carries its `mustSupport` element paths (plain
  elements; slice-internal ones are deferred). `must_support_gaps(resource)`
  returns the ones the resource leaves empty *where the parent is present* (a
  child under an absent parent is not a gap — the parent is). Deliberately
  outside the validation report and the FFI for now: no validator fails on
  must-support, so folding it into `issues` would break bug-for-bug agreement;
  it is the client-side completeness check (what US Core / AU Core certify a
  *sender* on), reachable from Rust and `probe --must-support`. Promote to the
  ABI when a host wants it.
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
