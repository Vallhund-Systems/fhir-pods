# Runtime scope and verification evidence

`fhir-pods` is a compliance runtime: it lifts and lowers resources, applies
policies, assembles documents, constructs audit records, and converts HL7 v2.
These operations share a verifier. This document details that verifier's
coverage and the limits of reference-engine comparisons; the
[operation overview](../README.md#current-capabilities) and
[architecture](architecture.md#operation-results-and-coverage) describe the
rest of the runtime's result contracts.

`conformance/run.py` records **1,766/1,766** verdict agreements at v0.15.0
(1,521/1,521 at v0.10.0), across ten pinned IGs. It includes
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

## Beyond the verdict: every error, both ways (`conformance/reasons.py`)

Agreeing on a verdict is not agreeing on *why*: a file both validators
reject can hide an error only one of them raises. Since 0.15.0 a second lane
reads the oracle's full output (`_oracle.bundle.json`, no JVM) beside ours
and, for every file, lists **each error-level issue the oracle raises at an
element where we raise none**, and **each we raise where it raises none** —
paths normalised to one spelling, and a message matched by family (the two
validators word one finding differently) so that a parent/child locus counts
only when the messages say the same thing. It exits non-zero on any
difference not named, path by path, in `conformance/reasons-triage.json`,
and on a triaged file that no longer differs.

On the 1,766-file corpus the residue is three cases, each with its reason
in that file: a wrong display name (below), a profile declared for another
resource type (the oracle applies it element by element; we refuse it in
one error), and a second choice key the oracle discards unread while we
also judge its format. Everything else the lane found on the way — datatype
invariants never evaluated on unprofiled resources, the base's required
bindings never checked without a profile, `ref-1` resolving against a
nested contained's ids, a `#contained` target judged by profile where the
oracle judges by type, a bundle entry's allowed-profile match ignoring the
resource's own declared profiles, a code owed by an element present only
through its carrier, an HL7-space URL that names nothing — is fixed, and
sixteen fixtures now pin the binding family the corpus never covered.

The lane also tallies, on files valid for both, the oracle's warning
families we do not emit (terminology it cannot check without a server,
definitions it cannot resolve, its choice among matching profiles, the
performer recommendation …). Those are not gated; they are the table below.

## What the reference validator checks that we do not (yet)

These families describe known coverage differences. Counts in this table
are historical observations from the v0.6.1 corpus, not a current inventory;
the current regression lanes track individual cases and evidence hashes.

| Family | Historical count | What it is | Our stance |
|---|---|---|---|
| **Terminology without a server** | ~700 | "Unable to validate code … running without terminology services", "ValueSet … not found", "Could not confirm … extensible value set", "CodeSystem … doesn't provide any codes" | We bundle 289 value sets (every set the compiled profiles bind `required`, and every set the base definitions bind `required` — gender, status codes, contact-point systems, units of time, vitals UCUM, FHIR / HL7 v2 / v3 sets, …) and check `required` bindings against those, profile or none; everything else is a `warning: no bundled terminology for <vs>`. The `TerminologyProvider` seam is where a host plugs NCTS / VSAC / a local cache in. SNOMED is never bundled (licence). |
| **Display-name checking** | 1 error-level in a probe | "Wrong Display Name 'Laboratory' for …#vital-signs" | Not done — needs the code systems' display strings (509 KB for the 1,212 systems we hold whole, and for SNOMED, never). The one corpus case is triaged in `reasons-triage.json`; the verdict agrees. |
| **Best-practice recommendations** | 82 + 12 | "all observations should have a performer" (`obs-…` best practice), `dom-6` narrative | `dom-6` we evaluate as a warning (it is an ordinary invariant). The performer recommendation is validator policy, not a profile constraint; not emitted. |
| **Multiple matching profiles** | 32 | The validator had to pick among candidate profiles (its own warning) | A resource that declares several profiles is validated against **every** one we have compiled (findings shared by two profiles are reported once; the report carries `profiles`). The oracle's warning here is about its own choice among *implicit* candidates, not declared ones. |
| **Deprecation / advisories** | 5 + 2 | Deprecated extensions and concepts, "UCUM annotations like {T-Score} can be misleading" | Advisory, not conformance; not emitted. |
| **Reference target types** | (none in corpus) | `subject` must reference a Patient | Enforced as the reference validator enforces it (0.15.0): a target it can reach — a bundle entry, a contained resource — is held to the element's `targetProfile`s (a bundle entry by conformance, "Unable to find a profile match for … among choices"; a `#contained` resource by type only, as the oracle judges it). A `Patient/x` nobody can resolve is not judged by either side (`heartrate-subject-wrong-type` expects *valid* as a guard). |
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
compares **5,290** of the **5,292** selected inputs: **4,819** verdicts agree,
**471** are reviewed lenient differences, and none are stricter (0.15.0's
base-definition checks moved eight reviewed leniencies — ConceptMap targets,
an ImplementationGuide rendering, the FiveWs / Definition / Event / Request
logical models' invariants, a hierarchical ValueSet's expansion systems —
into agreement). The other
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

The **20 lenient cases** remain scored differences (25 before 0.15.0: the
base's required bindings and datatype invariants closed five). Every case is named and
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

Profile or no profile, the shape walk holds every resource to the base
definitions themselves (0.15.0): every resource's own invariants (`org-1`,
`pat-1`, `obs-6` …, 170 of them); every datatype's own invariants on every
object of the type wherever it sits — `cpt-2` on a ContactPoint, `per-1`,
`qty-3`, `rng-2`, `rat-1`, `att-1`, `tim-*` on a Timing's repeat, inside an
extension's value or a datatype in a datatype; every `required` binding the
base places on a coded element (366 of them: `Patient.gender`,
`Observation.status`, `ContactPoint.system`, `Quantity.comparator`,
`Narrative.status`, `Timing.repeat.periodUnit` …), a code present only
through its `_carrier` still owing one ("No code provided, and a code is
required from the value set"); `ref-1` against the ids of the nearest
resource that is not itself contained (`%rootResource`, as the invariant
reads); `dom-3` per orphaned contained resource; and, for a `uri` / `url`
value under `http://hl7.org/fhir`, `http://terminology.hl7.org/` or
`http://fhir.org/guides`, that it names a definition the packages hold
(`http://hl7.org/fhir/sid/us-ssn.` does not; 7,983 canonicals and
NamingSystem ids as 32-bit hashes, `src/known_urls.rs`) — the reference
validator's own rule, which takes an unknown URL anywhere else on trust. A
profile snapshot that merely restates a datatype's invariant or binding is
not compiled a second time.

Rules the mutation lane added (0.19.4), each probed against the oracle:
`Reference.type` is held to the element's targets — the base's where no
compiled profile applies (a generated table of every base Reference
element's targets), the profile's under one — by bare type name and only
beside a `reference`; a relative reference inside a Bundle is `Type/id`
(a type the base does not define passes the format and resolves nowhere,
so a document is not held to it); inside a Bundle, a reference that
resolves to an entry — by `fullUrl`, a relative `Type/id` only against a
RESTful one — is held to the element's base targets by the target's own
type; an empty string beside a carrier with
content is a value absent, not a value wrong, though a required binding
still wants its code; where a profile's required set cannot be decided
offline, the base's own required set on the same element decides (a
profile only narrows it); a coding with no code fails a required binding
only where the set can be decided — the oracle says nothing where it
cannot; a binding beneath a choice (`Extension.value[x].type`) is typed by
the instance's shape; and the vital-sign profiles are implied by the
reference validator's own code table (`3141-9` is a body weight, `8306-3`
a height, `8331-1` a temperature, `55284-4` and `8480-6` a blood pressure,
`59408-5` an oxygen saturation, `8287-5` a head circumference), not by the
magic code alone. Value sets defined in `hl7.terminology.r4` are bundled
alongside the guides' own.

## Two adversarial lanes: mutation (`conformance/mutate.py`) and the gut path (`conformance/gut.py`)

A corpus of publisher-valid examples can only show where the pod is
*stricter* than the reference validator; three releases in a row were fixed
on host reports of leniency no lane could see. The mutation lane derives
every release-corpus original under nine fixed mutations — drop one
top-level element; a coding with its code removed, a concept reduced to
text; a primitive of the wrong JSON type; an array of objects reduced to
`[{}]`; a `status` / `gender` / `use` / `intent` / `mode` / `kind` / `type`
set outside its set; a top-level primitive set to `null`; a string set
empty; a reference to a type the base does not define, and a bare id; an
extension with a url and nothing else, and one with no url — and puts every
derived file (19,067 at 0.19.4) before validator_cli 6.10.4, one JVM per
class, the oracle's verdicts cached per class. It reports, lenient first:
where the pod says valid and the oracle invalid, the reverse, and where both
say invalid but the pod misses an error path the oracle reports (after the
reasons lane's own path spelling and with the original's recorded
differences subtracted). Accepted differences are named in
`mutate-triage.json`; the lane exits 1 while an unaccepted lenient or
stricter case remains. Its first run found nine defects (0.19.4, above).

The gut-path lane is the end-to-end proof for the shape a clinical-module
host builds: a synthetic sixteen-part instrument (`conformance/gut/make_gut.py`
— nested groups, `enableWhen`, required and repeating groups, coded answers
from its own code system and value sets, integer and decimal scales, a 1–7
stool-form choice, an eleven-item boolean safety screen; nothing in it is
copied from any real instrument), complete and minimal responses and
thirteen single-defect ones, the derived observations, an alarm `Flag`, a
clinician-summary `Composition`, a referral `ServiceRequest`, the research
export `Consent` and `Bundle`, and the `AuditEvent`. Every case is judged
three ways — the scenario's expectation, the pod (given the instrument the
way a host gives it, and the value sets as a compiled terminology table),
and the oracle with the same definitions loaded as an IG folder — then
`--chain` lifts every valid resource through its base profile and lowers
it back, de-identifies it, and exports the record for the AU region (every
re-profiled resource, the AU PS document, the BALP audit event), and judges
each derived file the same way. One reviewed difference is named in
`gut-triage.json`: the oracle also reports a vendor code as unknown in the
CodeSystem it has loaded, which the pod — given the value sets as a table,
not the code system — cannot. A host's finding the chain surfaces: a
response de-identified under Safe Harbor loses free-text answers, and a
required free-text item then makes the copy invalid against its instrument
on both sides — instrument design, not validation.

## Export: which profile, and what is never invented

`export` (0.11.0) picks, per resource, the region's profile it goes out
through. The rules are structural and closed, so a host can predict them:

- A declared `meta.profile` of the region's own settles it; where the region
  has no profile for the type, a declared one its document asks for does. A
  declared base profile (the `bp` a host lowered into) is the resource's
  identity, never a jurisdiction's profile.
- A type with one profile lineage in the region (a Patient, an
  AllergyIntolerance — AU Core's and AU PS's derived from it) takes the most
  specific: there is nothing to tell apart — unless the leaf adds
  identifying content its root lacks (CA Baseline's laboratory Observation
  fixes a category its general Observation does not), in which case it is
  a sibling in all but lineage and identity decides.
- Among sibling profiles of a type, the ones whose *identifying* content the
  resource carries survive: a fixed `code.coding` (any one of the profile's,
  so AU's SNOMED beside LOINC is not required of the record) or fixed
  `category.coding`, or a `required` binding on `code`/`category` the
  terminology provider decides as a member — a set it cannot decide
  identifies nothing. Identified profiles beat unidentified ones; a profile
  another survivor derives from is dropped.
- A profile its guide scopes in words to a kind of resource its compiled
  structure does not identify (AU Core Diagnostic Result — "specialist or
  other diagnostic results"; US Core Observation Clinical Result —
  "clinical test results"; US Core Smoking Status, whose only compiled
  identity is the social-history category because its code binding is
  extensible; the IPS Device as performer) is never matched by content: it
  is listed as `specialised` on the region and reached only by declaration.
  US Core's Simple Observation ("any simple observations not otherwise
  covered") is the guide's own catch-all and matches. In UK Core the "Lab"
  Observation, DiagnosticReport and ServiceRequest variants are
  specialised (no fixed category); in CA Baseline the implantable Device
  and the Provider-Registry Practitioner/PractitionerRole profiles are.
- Among survivors, a profile whose identity is a strict subset of another
  survivor's (a category alone beside a category and a code) is the less
  specific one and is dropped, as is one another survivor derives from.
- Nothing of the region's identifies → what its document asks of such an
  entry (the base `vitalsigns` family for the IPS, which has no vital-sign
  profiles of its own) → else the resource goes out as given, base-validated,
  with the reason in `note`. Several fit and none declared → likewise, naming
  them.
- Where the region's document asks its entries for a profile the target does
  not derive from (the IPS's Condition beside US Core's), that profile is
  declared alongside and verified: `also`, `alsoMissing`.

The re-profiling is `lift` through the target then `lower`, then what no
slot took carried back: the profile's fixed content is the profile's, the
record's slots are the record's, `id` and `meta` (all but `profile`) are
carried so references and security labels survive. A required slot the
record lacks is `missing`; content no slot takes is carried as given where
the profile's shape allows — a top-level member, an item of a repeating
element no slice claims (`component[2]`, `code.coding[1]`), a leaf under
single-valued parents (`code.text`) — and listed as `carried`; what cannot
be (beneath a repeating element the profile rebuilt, `component.interpretation`)
is `extras`. Nothing is invented, and nothing is replaced: a record value
at a leaf the profile fixes that disagrees with it (`conflicts` from
`lift`: a kPa pressure where the profile fixes mm[Hg]) stops the
re-profiling — the resource goes out as given with the contradiction in
`note`. Only a single-valued place or a slice's own instance can conflict;
a pattern on a repeating element (`code.coding` must contain SNOMED's) is
one instance's, which lowering adds. `notes` carry what else the host
should know: a declared profile the region does not have (not applied), a
document requirement several profiles fit (none declared), or unresolved
patient identity. A second `Patient`, or a top-level `subject`/`patient`
reference naming a different recognizable Patient id, rejects the export.
Same-id relative references may include `/_history/<version>`. Absolute
same-id references, opaque or logical references, and a missing record
Patient id produce an unresolved-identity note for the host. Matching an
absolute URL's suffix does not establish server identity; nested references
are outside this consistency check. The document's entry profiles may
come from another guide — the AU Patient Summary names IPS profiles for
its pregnancy and device entries — and are taken as the document asks. The oracle lane (`examples/export_corpus.rs`, 39 files: one base
record for every region, each resource, the document, the audit event)
agrees with the reference validator on every verdict; the two IPS documents
are invalid on both sides offline, for the pregnancy-profile ambiguity the
IPS's LOINC sets resolve when loaded.

## Deferred slices are said, not silent

21 slices across the registry have discriminators the compiler cannot
resolve, and every one is a defect of its guide as published: a `value`
discriminator on a code the profile neither fixes nor binds as required
(AU's `category:specificDiscipline` / `anatomicRegionOfInterest`; CA
Baseline's `NonDrugAllergen`, `LabOrder` and `@default` slices; CA's
Immunization `Generic`/`Tradename` — the reference validator errors on
those too), a discriminator on `binding.valueSet` (CA Condition), a
sub-extension whose `url` the published definition never fixes
(QuantityConfidenceInterval), and BALP's SAML `otherId` slices on
`value.ofType(Identifier).type`. (0.14.0 compiled slices whose
discriminator points at a `required` binding as `Member` discriminators —
decided by membership through the bundled sets, a whole-system set or the
host's provider, undecided never an error; 0.14.1 added the binding
inherited from a slice's datatype profile (AU's DVA number), the nearest
bound ancestor of a part-path discriminator, `resolve()` to a base
vital-sign profile the registry carries (AU's `supportingInformation:
bodyHeight/bodyWeight`, IPS's `DiagnosticReport.result`), and Bundle
entries whose `resource` is typed as the resource itself (the AU PS and
IPS `entry:consent/device/careteam/…`); 0.14.2 made slicings nest to any
depth (AU's `medicationClass` extension inside the AMT/PBS coding slice
inside the medication type slice): 60 of the 81 once deferred.)
Each remaining one is compiled as `deferred` (element, slice,
why) and not enforced. Since 0.13.1 a resource
that reaches the sliced element gets an **information** issue
(`not-supported`): "slice 'dva' of Patient.identifier in AU Core Patient
was not checked: its discriminator (pattern on type) is outside the
compiler's support" — so a `valid` verdict is never quietly weaker than it
looks, and a host can see exactly where the reference validator would look
harder. The mustSupport-only and non-required-binding-only element
constraints (610) are not slices and carry nothing to enforce; they
surface through `mustSupportGaps`.

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
- **What a Bundle entry's `fullUrl` owes** (0.17.0): one that "looks like
  a RESTful server URL" must end with the resource's own type and id —
  and "looks like" is validator_cli's own regex (`BundleValidator.urlRegex`),
  mirrored as it is: an STU3-era type list in which `Condition` is spelt
  "Condition (aka Problem)", so a Condition fullUrl is never judged, and
  host segments admit only `[A-Za-z0-9.:%$]`. Nothing else about the
  fullUrl is judged (F026's stricter rule is gone).
- **Names that must be names** (0.17.0): `Identifier.system` is an absolute
  URI (a scheme of lowercase token characters, as `Utilities.isAbsoluteUrl`
  has it); a top-level `Extension.url` is absolute; a resource's `language`
  is a BCP-47 tag; an extension that no package defines is refused wherever
  it is named ("could not be found so is not allowed here" — the reference
  validator's `allowUnknownExtension` lets only `example.org`, `acme`,
  `nema.org` and the tools extensions pass, and so does the pod), a
  well-formed cross-version extension (`http://hl7.org/fhir/5.0/
  StructureDefinition/extension-Type.element`) taken on trust since the pod
  carries no other version's element ids; a `Coding.system` under
  `terminology.hl7.org` that THO does not define is an error. A reference
  target that must conform to a profile conforms only when the resource is
  also sound on its own (the oracle's whole verdict on the target); a target
  that need only be a base type is judged by type alone.
- **A `QuestionnaireResponse` is held to its `Questionnaire`** (0.16.0,
  `src/questionnaire.rs`) the way validator_cli's `QuestionnaireValidator`
  holds it — its rules read from the bytecode, its words kept: every item's
  `linkId` defined at the level it sits and in definition order; a required
  question answered when enabled (a warning while the response is
  in-progress), a required group with sub-items; nothing answered where
  `enableWhen` disables (`exists`, `=`, `!=`, `<`, `>`, `<=`, `>=`, `any` /
  `all`, the answers found up the response tree); one answer unless the
  definition repeats, `minOccurs` / `maxOccurs`; the answer's type, and its
  `minValue` / `maxValue`, `maxDecimalPlaces`, `minLength` / `maxLength`,
  `regex` (+ `entryFormat`), attachment `mimeType` / `maxSize`,
  `referenceResource`, quantity `unitOption` / `unitValueSet` and the SDC
  min/max quantity; `answerOption` membership per type with
  `optionExclusive`, `answerValueSet` membership through a value set
  contained in the questionnaire (enumerations) or the terminology
  provider, under R5's `answerConstraint` carried by its cross-version
  extension. The questionnaire is found as a contained resource, a bundle
  entry, or one the host loaded (`Validator::with_questionnaire`,
  `fhir_pods_questionnaire_load`, `loadQuestionnaire`); one that cannot be
  found is a warning (an `example.org` one, information) and nothing is
  judged. A `Questionnaire`'s own `enableWhen` wiring is checked (a target
  that exists, is not the question itself, not an inner one). The lane:
  fhir-test-cases' `questionnaire` module, `conformance/questionnaire.py`
  — 81 of the 96 R4 JSON cases (15 need an SDC package, a launch context,
  a reference-policy option, or a supporting ValueSet), 74/81 verdicts
  agree, 73/81 agree on every error-level issue, 0 stricter; the eight
  reviewed differences (`questionnaire-triage.json`) are the stored
  outcomes' terminology server (SNOMED, a filter-defined jurisdiction set),
  display names, attachment size integrity, and an SDC extension. Not
  taken: SDC `enableWhenExpression` / variables / launch contexts, the
  derivation checks on a `Questionnaire`, `answerExpression`. Both base
  definitions are compiled as lowerable profiles (0.19.0): a host lowers a
  `Questionnaire` from its slots and a `QuestionnaireResponse` from its
  (`questionnaire`, `status`, `subject`, `authored`, `item` given whole —
  the items are the recursive backbone the base spec defines by
  `contentReference`, so nested groups ride inside the one slot), lifts
  either back to the same inputs, and the lowered response is held to its
  questionnaire on the way out like any other. The declared base profile
  asks exactly what the bare base check asks: a profile's snapshot
  enumerates a `contentReference` backbone once, at the depth the base
  defines it, so beneath the deeper copies (an item's item, a section's
  section) the walk keeps the base's required elements and bindings
  whether or not a profile applies, and the invariants on a backbone or
  beneath it (cmp-1 on a section, que-1 … que-13 and qrs-1 on an item and
  its `enableWhen`, obs-3 on a component's `referenceRange`) are evaluated
  on every copy the document holds, beneath the backbone or elsewhere in
  the resource, and reported at the path they fail (0.19.3).
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

## Scale: a Synthea cohort (`conformance/synthea.py`)

Synthea (Apache-2.0) generates, at run time and from a pinned release jar,
a seeded cohort of synthetic patients — each a transaction Bundle of
hundreds of US Core 6.1 resources — plus the hospital and practitioner
bundles. Every bundle goes before the pod and validator_cli and is compared
on the verdict and on every error-level issue, with the pod's time per
bundle recorded. Nothing generated is committed; nothing in it is a real
person. What it found (0.18.0): the reference validator refuses any
extension it cannot resolve (Synthea's own `synthetichealth.github.io`
ones), so every cohort bundle is invalid for both — and a reference target's
conformance includes the target's own soundness, so every reference to
that Patient fails its profile on both sides. And where the time went:
resolving each reference by a walk over the whole bundle, and validating
the same Patient once per reference to it — a per-validation memo (entries
indexed once, conformance remembered per target and profile) took the
largest bundle from 13.7 s to 1.5 s.

## Reproducing

```bash
JAVA_HOME=/opt/homebrew/opt/openjdk@17 python3 conformance/run.py
```

The oracle runs once per corpus in one JVM; its full output (all severities)
lands in `conformance/corpus/_oracle.bundle.json` for inspection — and is
what `python3 conformance/reasons.py` reads, no JVM, to put every error
side by side (evidence in `conformance/corpus-reasons/evidence.json`).
