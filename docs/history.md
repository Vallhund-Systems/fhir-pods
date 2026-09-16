# Build history — the conformance passes

The pass-by-pass log of how the verifier reached agreement with the reference
validator, moved here from the decision record so that document states
decisions rather than narrating them. Newest at the bottom. Numbers quoted in
each pass are as they stood at the time.

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
- **Pass 13 — finishing the scope table (2026-09-15).** (1) *Every declared
profile applies*: a resource declaring several compiled profiles is
validated against all of them, shared findings reported once, `profiles`
in the report; a two-profile probe (US Core Patient + AU Core Patient)
agrees with the oracle. (2) *FHIRPath tier three*: `$this is T` / `$this
as T` resolve against the invariant element's FHIR type carried in the IR
(`ProfileInvariant.types`), with choice elements (`effective[x]`)
enumerated per concrete key so each instance knows its type — never
inferred from the value's shape; `%resource` (the root threaded through
the engine, still the resource inside inherited datatype profiles);
`intersect` with structural node equality. This turns on `vs-1`,
`au-core-obs-02` and `obs-7` (verified: a valued Observation whose
component repeats its own code now fails obs-7). **81 of 83** distinct
invariant expressions parse; the two left are `dom-3` (needs
`descendants`/union/`in`/`as()`) and the QuestionnaireResponse
`repeat`/`isDistinct` check. Oracle **272/272**, the single informational
path difference unchanged; 500k fuzz iterations clean; 42 unit tests.
- **Pass 14 — `matches()` and the rest of the collection vocabulary
(2026-09-15).** A small regex engine in the core: Thompson NFA simulation
(linear in input × program, so the classic `(a+)+$` bomb finishes
instantly), covering literals, `.`, classes, `\d\w\s`, anchors, groups,
alternation and bounded quantifiers; backreferences, lookaround, flags,
`\b`, unicode properties are refused → `Unsupported`. Plus union `|`,
`in`/`contains`, `descendants()`, `repeat()`, `isDistinct()`, `distinct()`,
`trace()` (identity), `allTrue`/`anyTrue`/`allFalse`/`anyFalse`, and a
bare resource-type name resolving to `$this` (root invariants now carry
the resource type). Every distinct invariant expression in the registry
parses; `dom-3` alone still evaluates `Unsupported` (`as(canonical)` over
untyped descendants — JSON carries no element types). A 15-digit IHI probe
agrees with the oracle: **273/273**.
- **Pass 15 — must-support on the ABI (ABI 3).** `fhir_pods_must_support` +
JNI `mustSupport`; `pod.mustSupportGaps()` (wasm), `mustSupportGaps()`
(expo), Swift/Kotlin functions. Same memory contract; information, never
part of the validation report.
- **Pass 16 — a third IG: IPA 1.1.0 (jurisdiction-neutral).** 12 profiles,
15 examples; **288/288 first try, zero generator or engine changes** — the
generality check passed. AU Base is now fetched into the repo's own
gitignored cache like every other package (`DEPS` in `make_corpus.py`),
and `scripts/gen-profiles.sh` is the one regeneration command.
Registry: 168 profiles. `.wasm` 331 KB (83 KB gzipped).
- **Pass 17 — typed traversal, dom-3, libFuzzer (2026-09-15).** JSON carries
no element types, so the last unsupported invariant needed the base spec:
`tools/gen_typemap.py` compiles a 1,170-entry element-type table (registry
resource types + all datatypes; `id`/`extension` implicit) from
`hl7.fhir.r4.core`, and the document adapter types nodes as it reaches
them — root `resourceType`, choice keys (`valueQuantity` → `Quantity`),
backbone paths, `contentReference`, contained resources by their own
`resourceType`. `is`/`as`/`ofType` then work on any node (`as` on a
collection filters, as the reference engine does in dom-3). `dom-3` is
live: an orphaned contained Medication fails, and the oracle agrees
(**289/289**). What's left is exactly what a library can't do:
`htmlChecks()` and `resolve()`. Cost: `.wasm` 331 → 371 KB. libFuzzer
targets run on nightly (installed on request).
- **Pass 18 — lowering, proved (2026-09-16).** The transformer: per profile
the generator emits the fixed skeleton, the gated slices and the typed input
slots; the engine merges a host's inputs, applies the fixed leaves where
their gates are live, and validates. Proof = every publisher-valid corpus
example re-lowered from the values at its own slots and put in front of
the oracle. First run **449/556**: two families. (1) Pattern-derived leaves
(`category.coding.code` from a `patternCodeableConcept`) landed in an
object because "which elements repeat" came from the profile snapshot,
which omits `coding` when nothing constrains it — now repetition and wire
shape come from the base spec's element table, for lowering and for the
extractor alike (`Coverage.payor` cut to 0..1 is still an array). Fixing
that exposed a placement bug: slice instances were keyed by the unlabelled
prefix, so `component:systolic.code.coding` and `component:diastolic…`
shared one array. (2) A `MedicationRequest` referencing `#med1` lowered
without its `contained` — pass-through members (`id`, `text`, `contained`,
extensions) now travel, and the extractor carries a contained resource only
where a slot value still reaches it. **556/556.** The last disagreement was
the verifier's, not lowering's: an Organization containing an Endpoint —
a type outside the registry — left `dom-3` unevaluable and the orphan
passed silently. The type table now covers every R4 resource type (5,471
entries, 187 types), packed (relative paths, dictionary-encoded codes,
repeat flag in the entry: 570 KB `.wasm`, 152 KB gzipped, versus 681 KB
unpacked), and a skipped invariant is reported as an `information` issue
rather than nothing. ~29 µs mean per corpus validate.
