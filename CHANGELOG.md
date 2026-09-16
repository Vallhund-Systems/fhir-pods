# Changelog

## 0.5.0 — 2026-09-16

**Documents.** ABI 7: `fhir_pods_document(kind, inputs)`; wasm
`document(kind, inputs)`, Expo `document(kind, inputs)`; core
`Validator::document` / `assemble::{assemble, Kind}`.

- An IPS (2.0.1) or AU Patient Summary (1.0.0) `Bundle` from a patient and
  their resources: sections read from the kind's compiled Composition
  profile (the section slices' LOINC codes), each resource placed by what
  it is, a plain xhtml table per section generated from what the resource
  carries, references rewritten to the entries they name (`Type/id`, a
  host's own `fullUrl`s when resources arrive as Bundle entries,
  `urn:uuid:id`), resources nothing reaches left out and reported
  (`omitted`), a required section with nothing in it marked `emptyReason:
  unavailable` (no clinical fact is invented), deterministic entry UUIDs
  (the same inputs make the same document), and the document validated
  against the kind's Bundle profile on the way out.
- IG #4: `hl7.fhir.uv.ips` 2.0.1 and `hl7.fhir.au.ps` 1.0.0 in the
  registry (218 profiles), with `hl7.fhir.uv.extensions.r4` 5.3.0 for the
  extensions their slices inherit — a dependency package now contributes
  only the datatype profiles a compiled profile references.
- Proof: the IGs' twelve document examples, and every one of them taken
  apart and assembled again by the pod, in front of the reference
  validator (`examples/document_corpus.rs`, run by the harness). 967/967
  across corpus, lowered, de-identified and assembled fixtures.

**Verifier**
- **Bundle entries are discriminated** by their resource's type
  (`Discriminator::ResourceType`) and by conformance to a profile
  (`Discriminator::Profile` — a validation pass inside the same bundle);
  a slice's sub-element may have to conform to one of some profiles
  (`Slice.profiles`), checked when every one is compiled.
- **Slicings declared inside a slice** (`section:sectionProblems.entry`)
  are compiled and evaluated one level down (`Slice.slicings`); their
  entries are discriminated by what a reference resolves to
  (`ResolvedType`, `ResolvedProfile`). Three levels deep stays deferred.
- **`resolve()`, `&`, `is()`, `tail()` and string `contains()`** in FHIRPath
  — the Bundle invariants (`bdl-8`, `bdl-11`, `bdl-12`, `bdl-ips-1`) are
  evaluated now, not skipped — `resolve()` inside a document
  Bundle (by `fullUrl`, `Type/id`, `#contained`), never fetched.
- **An element matching more than one slice is an error**, as the reference
  has it. This is why four published IPS / AU PS examples are expected
  invalid in the corpus: without a terminology server the two pregnancy
  profiles are indistinguishable, and both validators say so.
- **`pattern[x]` / `fixed[x]` objects are enforced** on plain elements and
  slice sub-elements (`ElementConstraint.patterns`): every instance must
  carry the pattern's leaves, one instance of a repeating element carrying
  each group. They were discriminators only — a laboratory Observation
  passed IPS's tobacco-use profile and matched six entry slices.
- **The base spec's minimums inside datatypes** (`Narrative.status`,
  `Narrative.div`, `Extension.url`, `Annotation.text`…) are checked, profile
  or no profile, from the element-type table's new `required` flag.
- **Document rules**: the Composition's references must resolve inside the
  bundle, and every entry must be reachable from the Composition by
  following references forward or backward — the reference validator's
  errors, now ours.
- Lowering's slots also include the optional elements a profile's own
  invariants reach (`ips-2`: value, dataAbsentReason, hasMember,
  component), so a resource a constraint requires one of can be lowered.
- De-identification: masked stubs are typed (`Narrative` → a generated
  "Masked" narrative, `Annotation` → its text), `instant`s keep their year
  as `YYYY-01-01T00:00:00Z`, Bundle entries are top-level resources with
  `fullUrl`s pseudonymised consistently with the references to them, and
  hex tokens (UUIDs) stay hex.
- Corpus: `Bundle.signature` verification is out of scope (one AU PS
  example skipped, with the reason); the reference validator does not fail
  a document over a reference that does not resolve inside it (a probe
  guards against over-enforcing).
- Speed: an invariant is evaluated once per resource across the profiles
  applied to it, and an implied base profile is skipped when an applied
  profile derives from it (the IR carries each profile's `base`) — an AU
  Core heart rate went from 120 µs (three profiles, `dom-3` three times)
  to 66 µs native; a Patient is 37 µs; ~80 µs in wasm.
- `.wasm`: 951 KB (242 KB gzipped) — the two document IGs and the assembler.

## 0.4.0 — 2026-09-16

**De-identification.** ABI 6: `fhir_pods_deidentify(resource, policy)`;
wasm `deidentify(json, policy)`, Expo `deidentify(json, policy)`; core
`Validator::deidentify` / `deidentify::{Policy, Action, check}`.

- Safe Harbor by default, as a typed walk: `HumanName`, `ContactPoint`,
  `Narrative`, `Annotation`, `Attachment`, `Signature` removed; `Address`
  kept to `state` / `country` (and `use` / `type`); `date` / `dateTime` to
  the year, `instant` removed; ages over 89 (a birth date against the
  policy's `today`; an `Age` quantity clamped to 90); `Identifier.value`,
  `Reference.reference` and every resource `id` pseudonymised by
  HMAC-SHA256 under the host's salt (references keep their type and base
  URL, contained references their `#`, and a resource's own id maps as a
  reference to it does); `Reference.display`, `Meta.versionId`,
  `Device.serialNumber` / `udiCarrier` removed.
- Policy is data: `{"salt", "today", "base": "safe-harbor" | "none",
  "rules": [{"target", "action"}]}` — a target is a type, a type member
  (`Address.city`) or an element path (`Patient.address.city`); the most
  specific wins, the last among equals. Actions: keep, remove, mask, year,
  pseudonym.
- Conformance kept where the profile permits: an element the declared
  profile requires (`min ≥ 1`) is masked with `data-absent-reason: masked`
  (a primitive through its `_name` carrier) instead of dropped; an element
  emptied by what went inside it goes; an element the base spec does not
  define is removed and recorded; an unknown `resourceType` is refused.
- Output carries the audit trail (`applied`), the copy's validation, and
  `remaining` — what `check` still finds (empty under the default policy).
- Proof: `examples/deid_corpus.rs` de-identifies every publisher-valid
  corpus example, requires the checker to find nothing, and hands every
  copy to the reference validator with our verdict on its conformance;
  fuzzed by a seeded arm and a libFuzzer target.
- Pseudonyms keep the shape of what they replace — digits for digits,
  letters for letters of the same case, punctuation as is — so an NPI stays
  ten digits, a URI keeps its scheme, a `maxLength` still holds; a value
  that passes the Luhn check (an IHI, an NPI over `80840`) gets a pseudonym
  that does too. Not preserved, by design: format rules on letters (CLIA's
  `D` in third position) — a policy rule decides those identifiers.
- `hash`: SHA-256 and HMAC-SHA256, `no_std`, tested against the published
  vectors. `node`: the owned JSON tree, shared by lowering and
  de-identification.
**Verifier**
- **FHIRPath type names are matched as written**, as the reference engine
  does: a bare name is a FHIR type, `System.X` the system type a primitive
  maps to. `$this is DateTime` on a FHIR `dateTime` is false — which is why
  US Core's `us-core-1` never fires in the reference validator, and now not
  here (89 de-identified copies disagreed before this).

- **Slice-scoped invariants are evaluated** on the slice's instances (US
  Core's NPI / CLIA / NAIC identifier checks, `us-core-16` … `19`) — they
  were dropped by the generator before, because evaluated on the whole
  path they would over-apply. The IR's `Slice` carries `invariants`.
- **The base spec's vital-sign profiles are implied**, as the reference
  validator does and the spec requires: an Observation carrying one of
  their LOINC codes (or the vital-signs category, for the generic profile)
  is validated against it whether or not it declares it. The eleven
  profiles are compiled from `hl7.fhir.r4.core` into `generated::IMPLIED`
  (registry: 179 profiles).
- `.wasm`: 713 KB (198 KB gzipped) — de-identification, the hash, and the
  eleven base vital-sign profiles.
- Generated profiles intern their shared string lists
  (`generated/intern.rs`) — smaller source; the binary was already
  deduplicated by the compiler.

## 0.3.1 — 2026-09-16

**Terminology, wired for real.** ABI 5.

- **Bundled sets are generated, not hand-listed**: every value set the
  registry binds with `required` strength that expands offline from the
  packages' own compose — the FHIR and HL7 v2/v3 sets, 59 sets / 311 codes
  (`tools/gen_valuesets.py` → `src/value_sets_table.rs`). Licensed or
  hosted-elsewhere sets (SNOMED, LOINC, VSAC, NCTS) are skipped, never
  approximated, and listed in the generated file. Displays for the codings
  the registry fixes travel with them (licence-free code systems only).
- **Compiled expansions**: `terminology::Expansions` reads the line table
  `tools/compile_valueset.py` writes from `$expand` responses; a set in the
  table is complete. ABI: `fhir_pods_terminology_load` installs one for every
  call after; wasm `loadTerminology`, Expo `loadTerminology`.
- **Ask, then answer**: `terminology_needs` (core), `fhir_pods_terminology_needs`
  (ABI), `terminologyNeeds` (pods) — the bindings neither the bundled sets
  nor the table could decide, deduplicated, for a host to resolve against
  NCTS / Ontoserver / VSAC. The `Recorder` provider behind it.
- **Displays through the seam**: `TerminologyProvider::display`; lowering
  fills `display` on a `{system, code}` input and on the profile's fixed
  codings when the seam knows one. `Validator::lower_with(url, inputs, tx)`.
- The provider chain on every entrypoint is bundled → loaded table → host
  callback; `fhir_pods_lower_tx` now lowers through the callback directly
  (displays included) instead of re-validating afterwards. wasm gains
  `fhir_pods_lower_host_tx`; Android's `lower` takes the provider.
- `.wasm`: 645 KB (168 KB gzipped) — the bundled sets, their displays and
  every choice element's type list.

**Verifier**
- **Extensions on primitives are validated.** `_birthDate.extension` and
  every other `_name` carrier were invisible to the engine: a profile's
  slicing on `Patient.birthDate.extension` never saw an instance. The
  document port now reads a primitive's children from its carrier (paired by
  position in arrays), so AU Core's date-accuracy slice, the inherited
  extension profile and its binding are checked. Oracle probe:
  `patient-birthdate-accuracy-wrong-type`.
- **A choice element only takes the types its profile allows.** `value[x]`
  restricted to `Coding` rejects a `valueString` (structure error), as the
  reference validator does — the IR carries each choice element's `types`.

## 0.3.0 — 2026-09-16

**Lowering — the transformer.** Build a conformant resource from its
variable data only. The generator now emits, per profile, the fixed skeleton
(every fixed / pattern / inherited-profile leaf at any depth, with which
slices and optional elements gate it), the input *slots* (the required and
must-support leaves the profile leaves open, typed; choice elements list
their concrete keys) and the repeating paths. The engine merges inputs into
the skeleton with typed coercions (a string into a `Reference`, a number into
a `Quantity`, `{system, code}` into a `CodeableConcept`, a whole datatype
object as given, `_name` primitive-extension carriers verbatim), reports
unknown keys, wrong kinds and collisions with fixed values as issues, and
validates the result — a lowered resource is never trusted unverified.

- Rust: `Validator::lower(url, inputs)`, `lower_bytes`, `compiled::{Lowering, Slot}`.
- ABI 4: `fhir_pods_lower`, `fhir_pods_lower_tx`, `fhir_pods_slots`; wasm
  `pod.lower(url, inputs)` / `pod.slots(url)`; Expo `lower()` / `slots()`.
- Proof: every publisher-valid corpus example is re-lowered from the values
  at its slots and the result put in front of the reference validator
  (`lower_corpus` — the harness runs it): 262/262 lowered resources valid.
- Fuzzed: a lowering arm in the seeded harness and a libFuzzer target.
- Inputs any resource may carry — `id`, `text`, `contained`, `extension`,
  `modifierExtension`, `language`, `implicitRules` — pass through without
  being profile slots. Wire shape (array or not) comes from the base spec's
  element table, so a profile that cuts `Coverage.payor` to 0..1 still
  emits the array the JSON representation requires.

**Verifier**
- **The element-type table covers every R4 resource type** (5,471 entries
  across 187 types), not only the registry's — a contained resource or a
  Bundle entry can be any of them, and `dom-3` walks
  `descendants().as(uri)` through the contained resources. Before, an
  Endpoint contained in an Organization left `dom-3` unevaluable and the
  orphan passed silently; the oracle caught it on a lowered resource. The
  table is packed (paths relative to their type, dictionary-encoded type
  codes, the repeat flag folded in) so the full spec costs less than the
  registry subset did unpacked.
- **A skipped invariant is now visible**: an expression outside the
  supported FHIRPath core (or one the table can't type) adds an
  `information` / `not-supported` issue naming the invariant, once per
  resource. Validity is unchanged (errors only); a silent gap no longer
  reads as a pass.
- `.wasm`: 570 KB (152 KB gzipped) — the LOWERING tables and the full type
  table; ~29 µs mean per corpus validate in release.

## 0.2.0 — 2026-09-15

Review pass: an independent correctness review (16 findings, 8 of them
wrong verdicts), a hot-path audit, and a structure pass.

**Behaviour changes**
- **No implicit default profiles.** A resource that declares no profile we
  have compiled is valid with no profile claimed — the spec's behaviour and
  the reference validator's. The old AU Core defaults for a bare `Patient`
  and the hand-written `Observation` fallback are gone. A jurisdiction
  default is now explicit: `Validator::new().with_default_profile(url)`.
- **JSON string escapes are decoded** (`\/`, `\uXXXX` with surrogate pairs,
  `\"`, …) before anything compares them; trailing characters after the
  document are a parse error; a non-string `resourceType` is a structure
  error; `null` placeholders in a `_name` array are not counted as elements.
- **Pattern discriminators match one instance**: a `patternCodeableConcept`
  slice needs ONE coding carrying every leaf, not one coding per leaf.
- **Versioned canonicals** (`url|version`) in `meta.profile` are recognised.
- **FHIRPath**: numbers are numbers (integer arithmetic / comparison /
  equality on JSON numbers; decimals are outside the core → skipped),
  dates and times compare as instants with the spec's precision and
  timezone rules, `is`/`as`/`ofType` honour the type hierarchy (`code` is a
  `string`, `canonical` a `uri`, `Age` a `Quantity`, every resource a
  `Resource`…), string escapes are decoded, `implies` is left-associative,
  a multi-item collection in boolean context is skipped (not treated as
  true), `hasValue()` needs a single value, `$index`/`$total` are refused.
- Generator: an element carrying an object `fixed`/`pattern` keeps its
  cardinality (26 elements and 15 slice sub-elements gained their min/max).

**API**
- `Validator` — the registry's invariants parsed once (~0.6 ms); ~2× faster
  per validation than the free functions, which remain. The pods use one.
- `Validation.profiles`, `Document::child_instances` / `as_number` /
  `choice_children` (additive).

**Corpus**: 294/294 against the reference validator (six new probes).

## 0.1.1 — 2026-09-15

- **Typed traversal.** A JSON node now learns its FHIR type from a compact
  element-type table generated from the base FHIR package (`src/typemap.rs`,
  registry resource types + all datatypes; `id`/`extension` implicit): root
  `resourceType`, then element by element — choice keys, backbone paths,
  content references, contained resources. `descendants().as(canonical)`,
  `value.ofType(Quantity)`, `x is Reference` work on arbitrary nodes; under a
  resource the table doesn't cover, nodes are untyped and such expressions
  are skipped, never guessed.
- **`dom-3`** (contained resources must be referenced) now evaluates —
  every invariant expression in the registry is live except `htmlChecks()`
  and `resolve()`, which a library without a reference resolver cannot do.
- `single()`.
- libFuzzer targets exercised on nightly (`cargo fuzz`).
- `.wasm` 331 → 371 KB (96 KB gzipped) for the type table.

## 0.1.0 — 2026-09-15

First cut. One verifier core, one C ABI (version 3), two pods (wasm, Expo).

**Verifier**
- AU Core 2.0.0, US Core 6.1.0, IPA 1.1.0 — 168 compiled profiles (86
  resource profiles plus the `Identifier` and complex `Extension` datatypes
  slices inherit), generated from the packages' snapshots by
  `scripts/gen-profiles.sh`.
- Per-parent cardinality, fixed values, required bindings (`code` / `Coding`
  / `CodeableConcept`), slicing (value / pattern / type discriminators,
  closed slicing, nested-slice discriminator resolution, slice → profile
  inheritance), every declared profile applied.
- FHIRPath: navigation, boolean logic, equality/comparison, integer
  arithmetic, string and conversion functions, `where`/`select`/`all`/`iif`,
  `extension(url)`, type tests against the element's declared type,
  `%resource`, union, `in`, `descendants`, `repeat`, `intersect`,
  `isDistinct`, `allTrue` family, `matches` on a linear-time regex engine.
  Outside the core: skipped, never guessed.
- Terminology seam: bundled base-FHIR value sets first, a host provider
  behind them; SNOMED content never ships.
- Must-support gaps as information, separate from validation.
- Agrees with the HL7 reference validator (`validator_cli.jar`, `-tx n/a`)
  on 288/288 corpus instances; what that covers and excludes is in
  `docs/scope.md`.
- `no_std` + `alloc`, zero dependencies, `#![forbid(unsafe_code)]`;
  depth-bounded parsers; seeded fuzz harness under `cargo test`.

**Delivery**
- `fhir-pods-wasm` — ~330 KB `.wasm` (83 KB gzipped) + a dependency-free JS
  wrapper; ~30k validations/s in Node.
- `fhir-pods-expo` — Expo native module: iOS XCFramework (Swift over the C
  header) and Android jniLibs (Kotlin over a JNI shim), ~20k validations/s
  on simulators; terminology provider installed natively.
