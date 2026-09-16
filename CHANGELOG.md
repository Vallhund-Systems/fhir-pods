# Changelog

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
