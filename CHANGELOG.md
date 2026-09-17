# Changelog

## 0.6.1 — 2026-09-16

The review lane's F018–F024 (planning repository), all taken; every fix
found by their corpora or by the de-identified copies in front of the
oracle. Oracle: **1191/1191**. No ABI change (ABI 8).

**De-identification**
- **Free text goes under Safe Harbor** (F018): every `string` / `markdown`
  leaf — a concept's `text`, a note, an organisation's name, an
  extension's payload — except a coding's display and version, a
  quantity's unit, an address's state and country, and an element's own
  `id` / `linkId` (structural keys, not text). A host that keeps a
  free-text site by rule (`{"target": "CodeableConcept.text", "action":
  "keep"}`) gets every kept site back in a new **`residual`** list on the
  result (`{path, what: "free text kept"}`), so an empty `remaining` never
  reads as "all free text gone". The README's claim is what is done.
- **What is required is masked, never dropped**: what the base resource
  itself requires (`Observation.code`, `Encounter.class`) — at the root
  of this resource and of every nested one — and what every applied
  profile requires *through its slices* (`AuditEvent.agent:client.who`,
  `Composition.section:sectionProblems.text`); a masked element carries a
  mask for each required child (`Practitioner.name` → `_family`), and
  the checker knows such a stub. De-identified copies still conformant:
  287 of 368 (0.6.0: 247 of 372).
- **An extension emptied by what went goes with it** (F019): a URL-only
  extension is not an extension (`ext-1`); a compiled extension that lost
  a sub-extension its profile requires (a `translation` without its
  `content`) goes whole; a primitive's carrier emptied of its only
  extension goes too, never an empty `{}` behind.

**Documents** (F020)
- **A terminology provider reaches assembly**: `Validator::document_with`,
  `fhir_pods_document_tx`, wasm / Expo `document` under the `terminology`
  option, and the *loaded* table — which never reached document
  validation before. The IPS pregnancy profiles are told apart only by
  two LOINC sets the pod does not ship (bundling them parts us from the
  offline oracle on four published examples); supplied through the seam
  they validate, as `tests/document.rs` shows with the IG's own
  enumerations in the table format.

**Verifier**
- **An extension is held to its own compiled profile by url wherever it
  sits** — on a coding's display, inside a primitive's carrier — unless an
  applied profile already reaches it through a slice. Found by the
  de-identified IPS Immunization example (a `translation` missing its
  `content` passed).
- **The base's required elements inside backbones**
  (`QuestionnaireResponse.item.linkId`) apply when no profile does — found
  by the de-identified US Core QuestionnaireResponse examples.
- **An empty extension carrier (`"_gender": {}`) is an element with
  nothing in it** (`ele-1`), as the reference has it.
- Probes for all three; the scalar reader renders booleans and numbers so
  a fixed boolean compares (0.6.0's pattern fix, completed).

**ABI and audit**
- **F021 (P1): a terminology callback may load a table while being
  asked.** The loaded table is taken as a snapshot and the lock released
  before host code runs; `fhir_pods_terminology_load` inside the callback
  no longer deadlocks native (or traps wasm) — the table answers from the
  next call on. Test: a loading callback, asked once, then not at all.
- **F022: control characters are escaped, never replaced** — `\n`, `\r`,
  `\t`, `\u00XX` — in every JSON the pods write (one writer).
- **F023: every element of a base definition is an input**: `Signature`,
  `SampledData`, `ContactDetail`, `Contributor`, `DataRequirement`,
  `Expression`, `ParameterDefinition`, `RelatedArtifact`,
  `TriggerDefinition`, `UsageContext` and `Meta` join the slot types
  (`Provenance.signature` lowers).
- **F024: addresses classified as the pattern has them**: a scoped IPv6
  address (`fe80::1%en0`) is an IP address; any scheme (`mailto:`,
  `urn:`, not only `://` forms) is a URI; the host's `{address, type}`
  still wins.

Tests: 140. `.wasm`: 1.1 MB (277 KB gzipped). The review lane's own
runners against this tree: `conformance/deid/run.py` 14/14;
`conformance/documents/run.py` 45/50 with 48/48 oracle agreement, the
five being its valid-output expectations for the pregnancy / laboratory
cases that need the seam (its `diagnostic-terminology.txt` through
`document_with` meets them).

## 0.6.0 — 2026-09-16

**Audit and provenance.** ABI 8: `fhir_pods_audit(kind, inputs)` (+ `_tx`);
wasm `audit(kind, inputs)`, Expo `audit(kind, inputs)`; core
`Validator::audit` / `audit::{audit, Kind}`.

- **IG #6: IHE Basic Audit Log Patterns (BALP) 1.1.4** — its 19 `AuditEvent`
  profiles and 2 extensions compiled; its 45 published examples in the
  corpus, each re-lowered from its slot values and de-identified as every
  other example is. And the base spec's own **`AuditEvent` and
  `Provenance`** definitions compiled as profiles (every element an input),
  so a host lowers and declares them directly. Registry: 259 profiles; 72
  bundled value sets.
- **`audit(kind, inputs)`** for the ten BALP basic events — `read`, `query`,
  `create`, `update`, `delete`, each with or without a known patient
  (`patient-read`…) — from host-shaped inputs: `recorded`, `client` and
  `server` (`who`, `network`), an optional `user`, `source`, `data` or
  `query`, `patient`, and `transaction` (the request id, the correlation
  with the trace — BALP's `X-Request-Id` entity). The profile supplies the
  event type, subtype, action, outcome and the agents' types; a network
  address is classified for its `network.type` (IP, URI, email, machine
  name — or given as `{address, type}`); a query is base64-encoded. What the
  profile leaves open and the host does not say takes the pattern's own
  guidance, overridably: the user's participation type on a
  create / update / delete is `author`, the data entity's role is
  `Domain Resource`. A required input left out, or one out of place, is an
  error before anything is built. The basic profiles record successful
  events; a failed action is an `AuditEvent` lowered from the base
  definition.
- **The lowering engine grew** for it: a fixed leaf behind an optional gate
  (`action` patterned but 0..1) is reachable by giving it, and must agree; a
  fixed value lands with its type's JSON kind (a boolean, a number), not as
  text; a sliced container asking for more instances than its required
  slices supply (`entity` 2..* with one `patient` slice) keeps its plain
  input; a fixed leaf under one slice no longer hides the same-named leaf
  under a sibling slice (`entity:patient.role` vs `entity:data.role`); a base
  definition exposes every element as a slot.
- **Verifier: boolean patterns are enforced.** `patternBoolean` was dropped
  by the generator, so BALP's `agent:user.requestor = true` — and any other
  fixed boolean — was never checked. Three probes on a published example
  (false, missing, true) hold both sides to it.

Oracle: **1187/1187** (1160 with BALP's examples, their lowered and
de-identified copies and the three probes, plus the 22 records of the
audit proof: every kind with and without a user, a `Provenance`, a failed
read). Tests: 113. `.wasm`: 1.1 MB (272 KB gzipped). Timing unchanged
against 0.5.2 back to back (81 / 78 / 152 µs).

## 0.5.2 — 2026-09-16

The second review's findings (planning repository F008–F017, with F003
rechecked), all taken. Oracle: **1022/1022** (1010 + twelve probes for the
verdict-changing fixes). No ABI change.

**Verifier**
- **Nested resources are validated** (F010): a resource inside a Bundle
  entry, `contained`, or a `Parameters` parameter — however deep — is held
  to the profiles it declares, in the outer bundle's scope, its findings
  prefixed with the indexed path it sits at
  (`Bundle.entry[1].resource.Patient.identifier`). Before, an AU Core
  Patient missing its identifier inside an unprofiled collection Bundle
  validated clean. Found by the same shape walk, so the cost is nil: timing
  is unchanged against 0.5.1 measured back to back.
- **A declared profile compiled for another resource type is an error**
  (F011), not a silent drop.
- **The temporal grammar in full** (F012): a `dateTime` with a time has
  seconds and a zone, an `instant` has a date, a `time` has no zone, an
  offset stops at ±14:00, year 0000 is outside.
- **`ext-1` counts a value given through its `_value…` carrier** (F013).
- **The base resource's own rules when no compiled profile is declared:**
  its required elements (`Encounter.class`, `Observation.status`), one key
  per choice element (`value[x]` under two keys is an error, profile or
  not), and `DomainResource`'s invariants — `dom-3` (contained resources
  referenced) as an error, `dom-6` (narrative) as a warning — from a
  generated table (`typemap::DOMAIN_INVARIANTS`). `docs/scope.md` says so;
  the README's coverage claim is narrowed to what is checked.

**De-identification**
- **Pseudonyms replace every character** (F008): a non-ASCII letter was
  copied through, so `Müller`, `北京` and their like survived in ids and
  identifier values. Every non-ASCII character now maps like a letter.
- **An identifier's pseudonym domain is its `system` read from the
  original** (F014), whatever the member order; before, `value` before
  `system` fell into the empty domain.
- **The age clamp reads the unit** (F015): months, weeks, days and years
  (UCUM code or unit word) are compared in years and clamped in the
  quantity's own unit (1100 months → 1080); recorded in `applied` as a
  `clamp` action (new `Action::Clamp`, also accepted in a policy). The
  checker reads the unit the same way, and accepts exactly 90 — Safe
  Harbor's own aggregate — which it used to flag.

**Documents**
- **An absolute reference names only an entry with exactly that fullUrl**
  (F009): `http://other.example.org/fhir/Patient/p1` no longer attaches to
  this bundle's `Patient/p1` by suffix. Relative `Type/id` and `urn:uuid`
  matching unchanged.
- **The Composition id and Bundle identifier derive from every input**
  (F016): the seed is the SHA-256 of the canonical inputs, so two documents
  for the same patient on the same date with different content have
  different identities; the same inputs still make the same document.

**Conformance harness** (F017, F003 rechecked)
- Our side: a crash, an exit code other than 0/1, output that is not an
  `OperationOutcome`, or a report whose severities disagree with the exit
  code is `LOCAL-ERR` — a failure, never a verdict.
- The oracle's side: an exit code other than 0/1 discards its output; an
  entry that is not an `OperationOutcome` is not a verdict; the jar on disk
  is checked against the pinned version before it runs.

Tests: 107 (nine new, one named per finding). `.wasm`: 990 KB (251 KB
gzipped).

## 0.5.1 — 2026-09-16

The first review's findings (planning repository F001–F004), all taken.

**Verifier**
- **The base spec's shape rules, profile or no profile** (F001): an
  element the type does not define, a primitive of the wrong JSON kind for
  its FHIR type, a repeating element that is not an array (or a single one
  that is), a complex element that is not an object, a `date` / `dateTime`
  / `instant` / `time` that does not parse as its type demands, a
  contained or entry resource that is not one — all errors, from a typed
  walk over the element-type table (`Parsed::structure_gaps`). Before,
  `active: "not-a-boolean"`, `birthDate: 123` and an unknown element all
  validated clean against AU Core Patient. Also from the base spec, profile
  or no profile: `ele-1` (an element has a value or children), `ext-1` (an
  extension has a value or nested extensions, not both), `dom-2` / `dom-4`
  / `dom-5` on contained resources, no empty arrays or empty strings,
  32-bit integers, real calendar dates. Nine corpus probes of our own and
  the reviewer's first twenty-seven (`conformance/probes/`) all agree with
  the reference validator; four of theirs found the last of those gaps.
- **Coverage is never silent** (F002): a `resourceType` the base spec does
  not define is an error, and a declared profile the registry has not
  compiled is a warning naming it (`not-supported`) — the reference
  validator's severity for a profile it cannot resolve, and a probe holds
  both sides to it. A resource with no declared profile is held to the
  shape rules above and claims nothing else. Report shape unchanged.
- A bare `time` (`08:00:00`) parses: the temporal parser took a leading
  digit for a date, so every `Timing.repeat.timeOfDay` in the corpus was
  invalid the moment the walk checked times.
- Dependency packages now also contribute the resource profiles a compiled
  profile derives from (AU Base's `au-patient`, `au-diagnosticresult`…),
  so an example that declares the base alongside the derived profile is
  checked against both, as the reference does. Registry: 226 profiles.

`.wasm`: 982 KB (249 KB gzipped). Timing unchanged within noise (40 µs
Patient, 66 µs heart rate native).

**Conformance harness** (F003): a fixture the oracle returned nothing for
is a failure, not agreement; the oracle's previous output is removed before
it runs and a run with no output is reported with its stderr; the validator
jar is pinned (6.10.4) and named in the summary, which now states how many
fixtures were actually compared. A probe lane: hand-written fixtures in
`conformance/probes/` with a manifest carrying each one's expected verdict
and reason are scored like every other fixture.

**Documentation** (F004, T004): the README leads with what the library is —
an application speaking FHIR at its edges: verify, build, de-identify,
assemble — rather than the verifier alone; `docs/architecture.md`'s
performance section separates what is measured, what is implemented and
what is a target. `docs/scope.md` states the shape rules and the coverage
semantics.

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
