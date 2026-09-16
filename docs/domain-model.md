# Domain model

The nested engine is where the design has to be right, so this is the DDD pass
before the grind: the ubiquitous language, the bounded contexts, the value
objects, and the one seam that matters (document access). We use the FHIR spec's
own words throughout — inventing vocabulary in a domain that already has a
precise one is how validators drift from the spec.

## Ubiquitous language

Straight from FHIR — these are the type names in the code:

| Term | Meaning |
|---|---|
| **Resource** | a FHIR instance being validated |
| **Profile** | a StructureDefinition — a constraint set over a base resource |
| **Element** | a constrained node, addressed by a dotted **path** (`Observation.value[x].unit`) |
| **Cardinality** | `min`..`max` (`max` is a bound or `*`) |
| **Binding** | a coded element's tie to a **ValueSet**, with a **strength** (`required`/`extensible`/`preferred`/`example`) |
| **Slice** / **Discriminator** | a partition of a repeating element, selected by a discriminator (`value`/`pattern`/`type`/`profile`/`exists` at a sub-path) |
| **Fixed** / **Pattern** | a value the element must equal exactly / match as a subset |
| **Invariant** | a FHIRPath constraint (`obs-6`…) with a severity |
| **Coding** | `system` + `code` (+ `display`) |
| **Issue** | one finding: `severity` + `code` (IssueType) + `expression` (FHIRPath) |
| **Report** / **OperationOutcome** | our finding collection / its spec wire form |
| **TerminologyProvider** | the seam that answers "is this code in that ValueSet" |

## Bounded contexts

Four, with clean boundaries:

1. **Compilation** (build-time) — a StructureDefinition `snapshot` → a
   `CompiledProfile`. Owned by `tools/gen_profile.py`. Language: snapshot,
   element, canonical, differential.
2. **Validation** (runtime) — `CompiledProfile` × `Document` → `Report`. Owned by
   `engine`. Language: rule, walk, cardinality, issue.
3. **Terminology** — (`system`, `code`, `ValueSet`) → membership. The seam. Owned
   by `terminology` + the bundled `value_sets`. Language: binding, expansion,
   provider.
4. **Interop / wire** — `Report` ↔ `OperationOutcome`; the `$validate` contract.
   The anti-corruption layer: spec shapes live here and nowhere else.

The **shared kernel** between Compilation and Validation is the **`CompiledProfile`
IR** — an immutable value object, generated once, embedded in the binary. It is
the published language the two contexts agree on.

## This domain is functional, not aggregate-centric

Honest DDD: a validator has **no entities** — nothing with identity and a
lifecycle. A `Resource` is transient input; a `CompiledProfile` is immutable
reference data; an `Issue` is a value. Forcing aggregate roots here would be
cargo-cult. Good DDD for *this* domain is: **precise ubiquitous language + rich
value objects + clean bounded contexts + dependency inversion** — not ceremony.
The "aggregate" is just the pure function `validate(profile, document) -> report`.

## Value objects (the Validation context)

```
ElementConstraint { path, min: u32, max: Max, fixed: Option<&str>, required_binding: Option<Binding> }
Binding           { value_set, kind: Code | Coding | CodeableConcept }
Discriminator     Value { path, value } | Type { key }
Slice             { name, discriminators, min, max, elements, profile: Option<url> }
Slicing           { path, closed, slices }
ProfileInvariant  { path, key, severity, human, expression, types }
CompiledProfile   { url, name, resource_type, elements, slicings, invariants, must_support }
Issue, Report     (severity, IssueType, path, message)
```

All immutable, all `Send + Sync`, all `'static` in generated profiles.

## The seam that matters: document access is *infrastructure*

The engine does not know how the JSON is represented. It validates against a
**`Document` port**:

```
trait Document: Clone {
    fn child_counts(&self, path) -> Vec<usize>;   // per parent instance — cardinality
    fn scalars(&self, path) -> Vec<&str>;          // fixed values, bindings, discriminators
    fn instances(&self, path) -> Vec<Self>;        // re-rooted sub-documents ($this)
    fn choice_children(&self, stem) -> Vec<Self>;  // value → valueQuantity / valueString …
    fn all_children(&self) -> Vec<Self>;           // descendants()
    fn as_scalar / as_bool / same / fhir_type      // FHIRPath's view of one node
}
```

Today's **adapter** parses the resource into a value tree and walks it. A future
adapter **fuses parse + validation into a single pass** — the "almost free"
performance thesis — and the domain never changes, because it only ever saw the
port. This is the dependency inversion that keeps validate-on-parse a swappable
infrastructure decision rather than a domain constraint. Correctness first
(tree-walk), the single-pass fusion later, behind the same port.

## The domain service: the walk

```
engine::validate(profile: &CompiledProfile, doc: &impl Document) -> Report
```

Per `ElementConstraint`: resolve its path (handling `[x]` choice + arrays),
check **cardinality**, then **fixed** / **binding**, then recurse into **slices**
(each slice re-checks its constrained sub-set). Pure, deterministic, no I/O.
Terminology binding calls out to the `TerminologyProvider` / bundled sets;
unknown sets degrade to a warning (spec behavior), SNOMED expansions are never
bundled (see the licence decision).

## Licence rule, encoded in the model

A `Fixed` on a SNOMED `Coding` carries **`system` + `code` only — never
`display`** (the code is a licence-free reference; the display is licensed
content). The generator enforces this when it emits `Fixed` values.

## The grind, in the order it happened

Document adapter → nested engine → slicing → generator → conformance oracle →
FHIRPath (three tiers) → hardening → pods → second and third IGs → the
terminology seam → typed traversal. The pass-by-pass record is
[`history.md`](history.md).

## Hardening

The core runs on device under `panic = "abort"`, fed untrusted bytes, so
"never panics, never hangs" is a *held* property, not a hope:

- **Depth-bounded parsers.** Both recursive-descent parsers (JSON, FHIRPath)
  carry a depth budget: hostile nesting is a `ParseError` / `Unsupported`,
  never a stack overflow — which is an abort, uncatchable, fatal on device.
- **Bounded inheritance.** A slice may name a profile; the recursion is capped
  (`MAX_INHERIT`) so a registry cycle can never loop.
- **Seeded fuzzing under `cargo test`** (`tests/fuzz.rs`, zero deps): byte
  soup, mutated documents, FHIR-shaped resources with real declared profiles
  (so slicing, inheritance and invariants all run), and random FHIRPath —
  asserting no panic, determinism, and well-formed wire output. Deterministic
  under `FUZZ_SEED`, so a failure is a seed to paste back and replay.
  libFuzzer targets in `fuzz/` for coverage-guided sessions (nightly).
