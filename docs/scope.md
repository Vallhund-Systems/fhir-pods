# Runtime scope and verification evidence

`fhir-pods` is a compliance runtime: it lifts and lowers resources, applies
policies, assembles documents, constructs audit records, and converts HL7 v2.
These operations share a verifier. This document details that verifier's
coverage and the limits of reference-engine comparisons; the
[operation overview](../README.md#current-capabilities) and
[architecture](architecture.md#operation-results-and-coverage) describe the
rest of the runtime's result contracts.

`conformance/run.py` records **1,521/1,521** verdict agreements at v0.10.0
and after the subsequent core fixes, across seven pinned IGs. It includes
published examples and probes, lifted/re-lowered and de-identified examples,
assembled documents, audit records, and converted v2 messages. Wider lanes
below deliberately retain reviewed differences and exclusions.

Verdict agreement tests conformance of the resources presented to the
engines. Transformation fidelity, content preservation, policy coverage,
and host requirements also need operation-specific checks. See the
[changelog](../CHANGELOG.md) for each release and unreleased follow-up.

## The claim

For every fixture in the corpus (each IG's own published examples, plus broken
variants), our verdict equals the HL7 reference validator's
(`validator_cli.jar`, run with `-tx n/a` and `-allow-example-urls true`):

- **Verdict** — *valid* (no `error`/`fatal` issue) or *invalid*, and
- **Where** — the set of element paths carrying `error`-level issues, after
  normalising the two locator styles (ours: `Observation.component:diastolic`;
  theirs: FHIRPath with indices). A difference here is printed as
  `issue paths differ (info)`. Path agreement is diagnostic information,
  separate from the scored valid/invalid verdict.

Warnings and informational issues are **not** scored. Both sides emit them;
they differ in volume and kind, and the differences are all in the areas below.

## What the reference validator checks that we do not (yet)

These families describe known coverage differences. Counts in this table
are historical observations from the v0.6.1 corpus, not a current inventory;
the current regression lanes track individual cases and evidence hashes.

| Family | Historical count | What it is | Our stance |
|---|---|---|---|
| **Terminology without a server** | ~700 | "Unable to validate code … running without terminology services", "ValueSet … not found", "Could not confirm … extensible value set", "CodeSystem … doesn't provide any codes" | We bundle 126 value sets (gender, status codes, vitals UCUM, FHIR / HL7 v2 / v3 sets, …) and check `required` bindings against those; everything else is a `warning: no bundled terminology for <vs>`. The `TerminologyProvider` seam is where a host plugs NCTS / VSAC / a local cache in. SNOMED is never bundled (licence). |
| **Display-name checking** | 1 error-level in a probe | "Wrong Display Name 'Laboratory' for …#vital-signs" | Not done — needs code-system content we don't ship (and for SNOMED, cannot). This can produce a path difference even when another issue makes both verdicts invalid. |
| **Best-practice recommendations** | 82 + 12 | "all observations should have a performer" (`obs-…` best practice), `dom-6` narrative | `dom-6` we evaluate as a warning (it is an ordinary invariant). The performer recommendation is validator policy, not a profile constraint; not emitted. |
| **Multiple matching profiles** | 32 | The validator had to pick among candidate profiles (its own warning) | A resource that declares several profiles is validated against **every** one we have compiled (findings shared by two profiles are reported once; the report carries `profiles`). The oracle's warning here is about its own choice among *implicit* candidates, not declared ones. |
| **Deprecation / advisories** | 5 + 2 | Deprecated extensions and concepts, "UCUM annotations like {T-Score} can be misleading" | Advisory, not conformance; not emitted. |
| **Reference target types** | (none in corpus) | `subject` must reference a Patient | The reference validator does **not** enforce this offline either (`heartrate-subject-wrong-type` expects *valid* as a guard). Not enforced. |
| **Default profiles** | — | An undeclared profile means the base resource | Same: no profile claimed; the base resource's own rules (shape, its required elements, one key per choice, `dom-3` / `dom-6`) still apply. A jurisdiction default is opt-in (`Validator::with_default_profile`). |
| **Attachment fetching** | 1 | "Attachment content cannot be checked … error accessing <url>" | Never: a library does no I/O. |
| **must-support** | 0 | Validators do not check must-support | Not an error by spec. Exposed separately: `must_support_gaps(resource)` (and `probe --must-support`) lists the selected profile's must-support elements the resource leaves empty where their parent is present — what a client certifying against the IG must populate when it has the data. Never in the validation report. |

## The base spec itself: every R4 example (`conformance/base.py`)

This lane now fails on new or changed disagreements, unusable evidence and
changes to its hashed input inventory. Reviewed differences and two explicit
oracle exclusions are listed in `conformance/base-triage.json`; exclusions
never count as agreement. Oracle reuse requires matching input, outcome and
configuration hashes. See `conformance/README.md` for commands and evidence.

Beyond the seven IGs' examples, the widest test of the base spec's
rules *that we check on an unprofiled resource* — shape, cardinality,
datatype lexical rules, dom-* and bdl-* invariants, extension placement
and content, codes against the bundled code systems — is FHIR R4's own
example set: `hl7.fhir.r4.examples` 4.0.1, 5,292 instances across every
resource type, almost all declaring no profile, validated as declared by
us and by validator_cli, and compared. It is not a test of the rules an
unprofiled resource does not get from us (see "What we skip, never guess"
below: no resource-level invariants beyond dom-*/bdl-*, no bindings) —
on those the examples are clean, and agreement says only that. It runs
apart from the release gate (parity, not cleanliness: the spec's own
examples are not all valid, and several are conformance resources the
reference validator holds to rules of its own kind). The latest baseline
compares **5,290** of the **5,292** selected inputs: **4,811** verdicts agree,
**479** are reviewed lenient differences, and none are stricter. The other
**two** inputs are explicit oracle exclusions and are never scored as
agreement. The reviewed differences fall into these families:

- **Definition resolution** (the largest, on conformance resources —
  CodeSystem, ValueSet, ConceptMap, StructureDefinition, Measure): "No
  definition could be found for URL value …", "The source code … is not
  valid in the code system …", ValueSet filter properties, SearchParameter
  types, the nominated work group's publisher, CQL libraries, `sdf-*`.
  The reference validator resolves these against the packages it has
  loaded and the definitions' own content; a library that ships no
  definitions and does no I/O cannot, and says nothing.
- **Narrative hyperlinks** ("Hyperlink … does not resolve") — the xhtml's
  `<a href>` targets checked against the resource and the bundle: not done
  (the unprofiled lane records the same exclusion).
- **Codes against a version of a code system we do not carry.** Since
  v0.9.1 the base code systems' content is bundled (R4 core's and the HL7
  terminology package's `complete` systems — THO 7.3.0 at v0.9.2 — codes
  only) and a Coding or coded unit is held to it. The two packages carry
  different versions of many systems and a validator may have either
  loaded, so a code is unknown only when no version we carry has it, and
  a system any copy leaves open (`fragment`, `example`, `not-present`) is
  not bundled at all. The reference validator, holding an example to the
  one version it loaded, rejects a handful more (`partial` in
  `immunization-subpotent-reason`, gone from the newer copy;
  `service-type`, `example` in R4 core). A code system newer than the
  bundled THO can add codes the pod will call unknown until the table is
  regenerated: the trade an offline check makes.
- **Eleven files, seven small rules.** `sdf-4` (an invariant of
  StructureDefinition — no `sdf-*` runs here, only dom-* and bdl-*);
  closed sub-extension sets on a complex extension (`hla-genotyping-…`:
  the definition's slicing is `open`, the reference validator treats the
  set as closed — we follow the definition); `Attachment.hash` against
  `data`; a `#fragment` canonical whose contained target does not exist;
  in-bundle resolution of relative references (`Can't find
  'Practitioner/example' in the bundle`); an `<img src="#…">` fragment;
  MeasureReport-to-Measure group checks. Resource `id` and `code` lexical
  rules were in this list until v0.9.2, which added them.

What the lane taught us, taken in v0.9.2 (from a fresh review of
v0.9.1): an extension's placement is matched the three ways the reference
validator matches it — the literal path from the resource with content
references uncollapsed, the collapsed backbone path, and the path from
the enclosing datatype's definition (`Dosage.site`,
`StructureDefinition.snapshot.element.binding.valueSet`); a choice spelt
the R5 way in a context (`Parameters.parameter.value`) matches R4's
`value[x]`; contexts the extensions package marks for later FHIR versions
are left out; a code system whose `caseSensitive` is absent accepts any
case; resource `id` and `code` lexical rules. Taken in v0.9.1: every HL7 extension compiled
(contexts checked for all; `CanonicalResource` met by R4's canonical
resources, `regex` allowed on `ElementDefinition.type` as the reference
validator allows it); the base code systems' codes; a code under
`urn:ietf:rfc:3986` is a URI. Taken in v0.9.0: a canonical must be absolute
unless it is a fragment; a narrative must say something (txt-2: text beyond
whitespace, tags, comments and `&nbsp;`); `base64Binary` is the alphabet
with padding at the end only; a `urn:uuid:` is a lowercase UUID and a
`urn:oid:` an OID; a transaction-response's or batch-response's entries
need no `fullUrl`.

## The reference validator test suite (`conformance/testcases.py`)

At v0.10.0 the JSON-only lane pins `FHIR/fhir-test-cases` at
`591528170b6b31609b7682dc67c37f45edfe33ff`. It inventories all 492 R4 JSON
cases; 184 R4 XML instances remain outside the lane. The base comparison
scores **84 cases: 57 agree with stored Java expectations**, and **14 of
20 agree with stored Firely expectations**. These are versioned upstream
expectations, not new runs of both engines. Manifest modules/options exclude
407 cases; one more manifest entry names an absent upstream input. Neither
exclusions nor unavailable inputs count as agreement.

The **25 lenient cases** remain scored differences. Every case is named and
hashed in [`testcases-triage.json`](../conformance/testcases-triage.json).
Their reasons extend the limitations above: Attachment size/hash; example-URL
policy; uncompiled Questionnaire/RiskAssessment/Period invariants; external
profile, extension and Coding.system definition resolution; terminology
displays; absolute Bundle fullUrls, singleton link relations, search-result
types/identities, and contained-ID uniqueness; the one-megabyte Element.id
string limit; and StructureDefinition snapshot/differential validation.
None is removed merely because its verdict differs.

Two cases are stricter than the **stored** Java result, with no Firely result:
`base64-ws-spaces` and `obs-vital-signs-mdc`. Fresh validator_cli 6.10.4 runs
reject both. The latter is agreement for different reasons: Java resolves
an external PHD profile while the pod applies its compiled vital-sign binding.
This does not establish diagnostic parity. There are **zero cases stricter
than both stored engines**, and **zero unresolved stricter cases** after the
pinned Java checks; missing Firely outcomes are never counted as agreement.

The separate profile lane covers all **53** R4 JSON `profile` entries.
It compares **10: 9 agree with Java, 7/7 with Firely where present**, with no
stricter case. The remaining disagreement, `sdoh-type-slice`, is a Java
terminology-display check that Firely and the pod do not fail. Thirty-one
cases name compiler gaps (including deep/unresolved slicing, exact complex
fixed values, Quantity/additional bindings, value bounds and datatype-profile
constraints); two are rejected by Java snapshot preparation; ten require
manifest-declared capabilities outside the lane. Two compared MIME-type
cases still report unresolved terminology. Their valid verdict is not proof
that the terminology binding was checked.

The `testcase-profiles` Cargo feature adds only the current test context to
the registry. Canonical collisions across cases are isolated, snapshots and
Rust tables stay in ignored directories, and normal/release builds include
none of them. [Lane commands and evidence rules](../conformance/README.md)
explain preparation and replay. New/changed disagreements, malformed evidence
and native execution failures fail the lane. Reviewed gaps remain visible.

This lane led to four core corrections: document references with
`/_history/version` now select the corresponding `meta.versionId`; opaque
Bundle URI identities do not undergo HTTP REST path checks; a null primitive
array entry must have content at the same index in its companion array; and
Resource.id cannot carry extensions. The compiler also accepts a
StructureDefinition whose optional resource id is absent.

## What we check that the reference validator also checks

Per-parent cardinality, fixed/pattern values (scalars; object patterns via
slice discriminators), `required` bindings on bundled sets, slicing (value /
pattern / type discriminators, closed slicing, nested-slice discriminator
resolution, slice → profile inheritance for `Identifier` and complex
`Extension` types), and FHIRPath invariants with typed traversal. Every
error-level finding either side raised on the corpus is in this set — that is
the count above.

## What we skip, never guess

- FHIRPath outside the supported core: `htmlChecks()` (xhtml validity),
  `memberOf`, equivalence `~`, decimal arithmetic. (`resolve()` resolves
  inside the resource and its bundle — `#id`, `fullUrl`, `Type/id` — and
  string concatenation `&` is supported, since 0.5.0.)
  Every one of the registry's invariant expressions parses and, on a typed
  resource, evaluates — except the two above (`txt-1`/`txt-2`, `ctm-1`).
  Type tests and casts (`is`, `as`, `ofType`) resolve against the element's
  FHIR type — the profile's declared type for the invariant's own element,
  the element-type table for everything else (root `resourceType`, then key
  by key through choice elements, backbone paths, content references and
  contained resources) — never from the value's shape. The table covers
  every R4 resource type and datatype; a node under a `resourceType` the
  spec doesn't define has no type, and such an expression is skipped — and
  reported as an `information` issue naming the invariant.
- **A Bundle entry with a resource carries a `fullUrl`** (0.8.3), as the
  reference validator requires — except a transaction's or batch's create,
  whose resource has no identity yet.
- **The base spec's shape rules apply to every resource, profile or no
  profile:** an element the type does not define is an error; a primitive
  must be the JSON kind its FHIR type is represented as (a `boolean` a JSON
  boolean, an `integer` / `decimal` a number, everything else a string) and
  a `date` / `dateTime` / `instant` / `time` must parse with the precision
  and time zone the type demands; a repeating element must be an array and
  a single one must not; a complex element must be an object; a datatype's
  own required elements must be there. A `resourceType` the base spec does
  not define is an error; so are an empty array or an empty string, an
  integer outside 32 bits, a date that is not a calendar date, an element
  with neither value nor children (`ele-1`), an extension with both a
  value and nested extensions (`ext-1`), and a contained resource that
  itself contains, is versioned or carries security labels (`dom-2`,
  `dom-4`, `dom-5`), and a choice element (`value[x]`) given under two of
  its keys. A `dateTime` with a time has seconds and a zone, an `instant`
  has a date, a `time` has no zone, an offset stops at ±14:00, and year
  0000 is outside the grammar. Not checked: the regular expressions on
  `id`, `code`, `uri` and friends, and the `_name` / `name` array pairing.
- **An extension whose `url` names a compiled extension profile is held to
  it wherever it sits** — on a coding's display, inside a primitive's
  carrier — unless an applied profile already reaches it through a slice.
  The base's required elements inside a resource's backbones
  (`QuestionnaireResponse.item.linkId`) apply when no profile does.
- **A resource nested in another — a Bundle entry, a contained resource, a
  `Parameters` resource, however deep — is validated against the profiles
  it declares, in the outer bundle's scope,** its findings prefixed with the
  indexed path it sits at (`Bundle.entry[1].resource.Patient.identifier`).
  A declared profile compiled for another resource type is an error.
- **A declared profile the registry has not compiled is a warning**
  (`not-supported`, naming the profile) — the reference validator's own
  severity for a profile it cannot resolve — so a claim the verifier could
  not check is visible in the report, never silent. A host that treats an
  unverified claim as a failure filters that issue code. A resource with no
  compiled profile declared is held to the base resource: the shape rules,
  the resource's own required elements (`Encounter.class`,
  `Observation.status`), and `DomainResource`'s invariants (`dom-3`
  unreferenced contained, `dom-6` narrative as a warning) — `profile` is
  empty and nothing more is claimed.
- **Documents.** A `Bundle` is validated against its declared Bundle
  profile: entries discriminated by resource type and by conformance to a
  profile (a validation pass in the same bundle), section entries by what
  their references resolve to, an element matching more than one slice an
  error (the reference validator's rule), the Composition's references
  resolved inside the bundle, every entry reachable from the Composition.
  Not checked: `Bundle.signature` (cryptographic verification is not a
  validation concern here), slicings nested three levels deep (none in
  IPS / AU PS carry a verdict), and references that do not resolve outside
  the Composition (the reference validator warns; so would a stricter
  reading — neither fails the document).
- A scalar `pattern[x]` / `fixed[x]` on a boolean or a number is enforced
  like one on a string (BALP's `agent:user.requestor = true`); the JSON kind
  is the shape walk's concern, the value the profile's.
- `pattern[x]` and `fixed[x]` objects on an element are enforced as
  patterns: every instance carries the leaves, one instance of a repeating
  element carrying each group. A `fixed[x]` object's "nothing else" clause
  is not enforced (the reference validator is stricter there).
- The base spec's minimums inside datatypes (`Narrative.status`,
  `Extension.url`…) are checked whenever a profile applies.
- An Observation carrying a base vital-sign profile's LOINC code (or the
  vital-signs category) is validated against that base profile as well as
  what it declares — the spec's rule, and the reference validator's.
- A slice's own invariants (US Core's NPI / CLIA / NAIC checks) are
  evaluated on the slice's instances only.
- `is` / `as` / `ofType` match type names as written, like the reference
  engine: a bare name is a FHIR type (`dateTime`), a `System.` name the
  FHIRPath system type a primitive maps to (`System.DateTime`). So US Core's
  `us-core-1` (`$this is DateTime implies …`) is vacuously true on a FHIR
  `dateTime` — in the reference validator and here — and a year-only
  `effectiveDateTime` passes it while failing the vital-signs `vs-1`.
- `matches(regex)` runs on a small linear-time engine (literals, classes,
  anchors, groups, alternation, bounded quantifiers). Backreferences,
  lookaround, flags, `\b` and unicode properties are refused →
  `Unsupported` → skipped.
- Slices whose discriminator cannot be resolved from the profile (e.g.
  AU Core's `identifier:dva`, whose `type` pattern lives nowhere). Deferred.
- Bindings to value sets we do not bundle. Warned, not failed.
- **Where an extension may sit is checked** (0.8.2): a compiled extension
  profile carries its definition's `context`, and an instance whose owner
  answers to none of them — by its type, by its path from the resource
  (`Patient.birthDate`, `Observation.value[x]`), by its path from the
  datatype (`ContactPoint.value`), or by the base types (`Element`,
  `Resource`, `DomainResource`, `BackboneElement`, `*`) — is an error, as
  the reference validator has it. A `fhirpath` context selects nodes from
  the containing resource and must select the actual extension owner, not
  an equal-looking node elsewhere. An `extension` context checks the named
  parent URL, including placement on that parent's primitive value and
  `canonical#code` sub-extension contexts. An unsupported context expression
  produces a `not-supported` warning if no other context matches; it does
  not silently establish allowed placement. A root `ofType` naming a type
  absent from R4 cannot select an R4 resource. The published
  `patient-contactPriority` alternative `ofType(ExtendedContactDetail)`
  crashes Java 6.10.4 on a wrong R4 owner; that witness is recorded as an
  oracle execution failure, not claimed as verdict agreement.
- **Primitive-only extensions and invariants:** a primitive without a
  value still receives its profile constraints. Its `toString()` result is
  empty; an empty profile-constraint result fails, matching validator_cli.
  The public FHIRPath `check*` expression helpers retain their documented
  empty/unknown behavior. The R4 `bdl-8` constraint uses the pinned Java
  validator's explicit `fullUrl.exists()` guard for optional fullUrl. The
  older vital-sign date-precision expression is guarded to apply to dateTime
  only, preserving the reference validator's acceptance of effectivePeriod.
- **HL7 v2** (0.8.0): the published `hl7.fhir.uv.v2mappings` maps are
  executed as published; a row the pod cannot execute (a narrative-only
  condition, a prose formula, a target form the engine does not take, an
  ambiguous segment map) is applied never and reported per message with its
  reason, a segment the structure has no place for is named, and what FHIR
  requires that the message does not carry stays missing and is named in
  the validation. The converter's own rules beyond the maps are few,
  structural and listed at the top of `v2/convert.rs`.

A skipped check can only make us *more lenient* than the reference validator,
never stricter: the corpus proves no lenience shows up on real examples, but a
constraint we skip is a constraint we skip.

## Reproducing

```bash
JAVA_HOME=/opt/homebrew/opt/openjdk@17 python3 conformance/run.py
```

The oracle runs once per corpus in one JVM; its full output (all severities)
lands in `conformance/corpus/_oracle.bundle.json` for inspection.
