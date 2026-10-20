# Changelog

## Unreleased

**The datatype profile R4 puts on an element is applied.** R4 types 55 base
elements `{"code": "Quantity", "profile": [".../SimpleQuantity"]}` —
`Observation.referenceRange.low`, `Range.low` / `.high` wherever a Range sits,
`SampledData.origin`, `Dosage.doseAndRate.doseQuantity`,
`MedicationRequest.dispenseRequest.quantity`, `Claim.item.quantity` … — and
the element-type table recorded the type code only, so the walk read them as
a plain `Quantity` and accepted a `comparator` there (the lenient gap 0.22.0
declared in `docs/scope.md`). `tools/gen_typemap.py` now emits, beside the
specialization tables, the package's datatype profiles the walk can apply
whole (`DATATYPE_PROFILES`: `SimpleQuantity`, `MoneyQuantity`; the third,
`elementdefinition-de`, slices its extensions and is listed as not
applicable) and every base element whose type names one (`ELEMENT_PROFILES`
— **55**, counted from every `type[].profile` in the package, all
`SimpleQuantity`; a profile the walk cannot apply whole is a generator error,
never dropped). A regeneration on a clean tree is byte-identical. Where the
walk meets such an element it applies the differential: `comparator`
(max 0, present through its `_comparator` carrier alone too) is
`structure.max` at the element, and the profile's invariant is
**`inv.sqty-1`** — new ids in the invariant table with **`inv.mqty-1`**,
which nothing can raise yet, since no R4 base element names `MoneyQuantity`.
Both are errors, as measured: sixteen new probes (`conformance/probes/sq-*`)
put a comparator on fourteen such elements — a resource's own, a datatype's,
a repeating one, inside a contained resource, a Bundle entry, under a
declared profile, through the carrier alone — and validator_cli 6.10.4
answers every one with `Quantity.comparator: max allowed = 0, but found 1
(from …/SimpleQuantity|4.0.1)` and `Constraint failed: sqty-1`, both errors
at the element, and passes both controls without one. The pod before called
thirteen of the fourteen valid (the fourteenth failed only for its carrier's
missing code).

A compiled profile can name such a profile where the base does not, and the
compiler dropped it without a trace: `hl7.fhir.uv.extensions.r4`
`medicationdispense-quantityRemaining` types its `Extension.value[x]`
`Quantity` / `SimpleQuantity`, compiled as `types: ["Quantity"]`, "0
deferred" — a comparator there two errors to the oracle (max 0 and `sqty-1`
at the extension's value) and valid to the pod. `ElementConstraint` now
carries `datatype_profiles` — a base datatype profile the profile puts on a
type of the element where the element's base definition does not (one the
base names stays the walk's, so nothing is said twice) — and the engine
applies it whole with the same ids, a choice by its concrete key, a slice's
elements too; the generator stops on one it cannot apply whole. In the
shipped registry that element is the only one (two more probes,
`sq-extension-value-*`; probes now **55/55**). Every generated constraint
gains the field, which grows the wasm artefact from 3,102,273 to 3,219,080
bytes. An IG's own datatype profile on an element outside a slice is still
not applied: measured, `NzAddress` and `NzContactpoint`'s `max 1` extension
slices are errors to the oracle and pass here (lenient), while an address
breaking every `au-address` rule passes on both sides, AU Core allowing
`Address` or `au-address` (`docs/scope.md`). The `collapse-age` rule still writes
the bare value on every `Quantity` (by type), and its copy now passes the
walk that refuses the comparator.

This is a **verdict-changing** change — stricter on a comparator where the
base says SimpleQuantity — and **not an ABI change** (ABI 14, no entrypoint
or shape moved). Lanes re-measured: release gate **1,796/1,796** (1,778 + the
eighteen probes), reasons 0 untriaged on 1,796 (3 triaged), published R4
examples **5,155/5,287**, 132 lenient, 0 stricter, 3 order-dependent oracle
cases unscored (the base lane as it stands since the lane corrections below;
5,156/5,290 and 134 before them) — the pod's verdict on all
5,292 inputs is unchanged, and **no `base-triage.json` entry retires**: no
R4 example puts a comparator on a SimpleQuantity element, and the oracle
reports no SimpleQuantity finding on any of them, so the expected triage
movement does not exist (one fresh run here failed four shared-key
StructureDefinitions on intermittent oracle drift, the pod's verdict
unchanged on each; the next fresh run had none);
narrative 42/42; questionnaire 74/81 (the lane exits 1 on one stale review, the same at
0.22.0 core); supporting 9/11; validator test cases Java 64/84, Firely 14/20;
ad-hoc profiles Java 12/13, Firely 7/7; v2 samples 77/77; Synthea 14/14;
the instrument-path scenario (`conformance/instrument.py`) 71/71 (`--chain`)
and 40/40 (`--host-profile`); mutation lane 19,067/19,067, 0 untriaged. 281 tests.

Corrections to the released 0.22.0 documentation and lanes; the shipped code
and artifacts are unchanged.

- The instrument-path scenario lane (previously named for a host's use case)
  is renamed; `conformance/instrument.py`, its generator
  (`conformance/instrument/make_instrument.py`), corpus
  (`conformance/instrument/out/`) and triage (`instrument-triage.json`)
  follow. It is described by its FHIR shape — one synthetic sixteen-part
  questionnaire instrument with its own code systems and value sets, its
  derived chain, lift/lower, de-identification and a regional export — as one
  scenario among the lanes, and its fixtures are neutral placeholders
  (`example.org/instrument` canonicals, a 1–7 ordinal grade, generic parts,
  condition codes and safety-screen items; no real instrument). Re-judged by
  the oracle: 71/71 with `--chain`, 7/7 more with `--host-profile`, and the
  same one triaged residue (the oracle's extra code-system finding on
  `qr-answer-not-in-set`).
- Every lane re-run at 0.22.0 (ABI 14; `dist/0.22.0/fhir_pods-0.22.0.wasm`
  sha256 `77ab6c73…7679`, the wasm the Node tests load) and the README lane
  table rewritten from the measurement, each row stamped at 0.22.0, with rows
  added for the probe and narrative lanes. Release corpus **1,778/1,778**,
  three triaged residues; probes **37/37**; narrative **42/42**; base
  **5,155/5,287** (132 lenient, every one reviewed; 0 stricter; 2 excluded;
  3 order-dependent oracle cases unscored); validator base tests **64/84** Java,
  **14/20** Firely (18 lenient; 2 stricter than stored Java, 0 than both);
  questionnaire **74/81** verdicts, **73/81** every error (7
  lenient, 0 stricter, 15 excluded); supporting **9/11** of 11 compared (2
  lenient, 0 stricter, 149 excluded); ad-hoc profiles **12/13** Java, **7/7**
  Firely (1 lenient, 0 stricter, 40 excluded); v2 samples **77/77** verdicts,
  **74/77** every error, 0 stricter; Synthea **14/14** (4.39 s, 6.2 MB/s on a
  loaded machine); mutation **19,067/19,067** (0 lenient, 0 stricter, 0
  untriaged); instrument path **71/71** + **7/7** host-profile. 277 Rust
  tests, 22 wasm Node tests, 25 Python harness checks, clippy clean.
- The README had carried older figures: validator base tests 63/84 and 13/20
  (`pat-dob-ext` agrees with both engines since 0.18.0 refused extensions no
  package defines), ad-hoc profiles 10 compared and 9/10 (binding-
  discriminated slices in 0.14.0 and nested slicing in 0.14.2 brought
  `res-inv-example-good`, `res-inv-example-bad` and
  `AuditEvent-slice23-second` out of the compiler-gap exclusions, all three
  agreeing), the release corpus at 1,766, and test counts of 239 and 18.
  Measured against each tag's tree; no verdict moved in 0.22.0.
- The questionnaire lane exits 1 on `nested-questionnaire-nested-valueset`,
  and has since 0.19.4. validator_cli 6.10.4 (with `-tx n/a` and with its
  default terminology server) reports that response's nine answer codings
  without a system as warnings — "A code with no system has no defined
  meaning…", twice each — beside the one questionnaireAdaptive-extension
  error; the pod reports them as nine errors, `… in the system 'null' is not
  in the options value set (ValueSet[null])`. The 0.19.4 rule (a coded answer
  with no system is an error) came from a case with a display and no code
  and over-reaches to a code without a system: on those findings the pod is
  stricter than the oracle, a pod defect, and the stored Java outcome is
  current. `ValueSet[null]` names nothing, although the contained ValueSet
  has a url. The lane fails until the pod is fixed. `scripts/release.sh`
  runs no conformance lane, and the released 0.19.4 line "every lane as
  0.19.3" was true of the figures but not of this lane's exit status.
- `conformance/testcases.py` fails on a reviewed difference that no longer
  differs (as `questionnaire.py` already did). Seven such entries in
  `testcases-triage.json` recorded `ours: valid` with gap reasons no longer
  true and are removed: `questionnaire-enableWhen-dw`,
  `q-enablewhen-me-wrong`, `mni-patientOverview-bundle-example1b`,
  `pat-dob-ext`, `nested-questionnaire-nested-valueset`, `encounter-period`,
  `ext-ccuk`. Figures unchanged.
- Some base-lane oracle outcomes are decided by validator_cli's own
  iteration order. 6.10.4 checks a definition's constraint key against the
  first base structure carrying it (`findInvariantInBase` over
  `ContextUtilities.allBaseStructures()`, a `java.util.HashSet` of proxies
  without hashCode/equals: identity-hash order, different in each JVM), and
  R4's base structures give `inv-1` three expressions — Parameters, Task,
  TestReport. Validating any of them, or a definition with its own `inv-1`
  (the `allergyintolerance-substanceExposureRisk` extension), yields no
  finding or "The constraint key 'inv-1' already exists at the location …"
  naming any one of the others (`StructureDefinition-Event`, a logical model,
  never reaches that check: it fails `sdf-4` alone, an agreement); batch
  composition plays no part, and judging one definition per JVM changes
  nothing. `conformance/base.py` derives those
  partners from the R4 core package and accepts exactly the outcomes they
  explain: Parameters, Task and TestReport are order-dependent oracle cases
  (`oracle_order_dependent` in `base-triage.json`, never scored), and the
  allergy extension stays a reviewed lenient difference whose message may name
  any partner and must carry its recorded FHIRPath error. Order-dependent rows
  carry `agree: null` in the evidence. Compared 5,290 → 5,287,
  agreements 5,156 → 5,155 (Parameters leaves them), lenient 134 → 132.
  A fresh run also compares every outcome with the previous
  `oracle-cache.json`, ignoring the source-file extension (an absolute path,
  so another checkout's outcomes compare), and reports a failure caused only
  by the oracle moving as drift, exit 2; exit 1 is the pod or the evidence.
  `BASE_ORACLE_TIMEOUT` raises the per-batch bound (900 s) on a loaded
  machine.
- The README's Synthea row stated planned work; it now states the measured
  limit (one oracle JVM over the 14 bundles; larger cohorts not measured).

## 0.22.0 — 2026-10-14

**A host's second field report, probed.** Three independent readers drove the
packed 0.21.0 pod with a compiled host profile and reported five things. Each
was taken to validator_cli 6.10.4 before it was taken to the code, and the
oracle contradicted part of the report: an unclosed `<p>` inside a closed root
`<div>` is refused on both sides, and it was never accepted here. What the
probes did find is a new lane and three corrections.

*The narrative, where its text stops.* `conformance/narrative/` holds 42
synthetic `Narrative.div` cases — the narrative, the oracle's own diagnostics,
the pod's report — and fails on any disagreement. 42/42 agree; six did not
before:

- the text ending inside a **closing** tag was accepted, where the oracle
  reports `Unexpected termination of html source` (with `txt-1` and `txt-2`,
  a parse failure leaving no document);
- the text ending inside a **start** tag was refused, where the oracle keeps
  the element with the attributes read so far and ends the document there —
  so `…</p><p` and `…<p class="x` pass, a truncated root keeps its `xmlns`
  and reports only `txt-2`, and a bare `<div` has no namespace and reports
  both. The one truncation it does refuse is a `/` that no `>` follows;
- an **unterminated quoted attribute value** was then treated as that same
  truncation, and it is not one: the parser takes the value to the tag's own
  `>` and carries on after it, so `<p class="x>Real content.</p></div>` is a
  paragraph with text and nothing is reported (four forms probed — either
  quote, root closed or not, first attribute or not). Only a value that
  reaches the end of the narrative truncates;
- a bare `#` was dropped before resolution, so `<img src="#">` passed; the
  oracle reports `Image source '#' … does not resolve` for an image and
  accepts `<a href="#">`, which is the document itself;
- and a `>` **inside a quoted attribute value** did not end the start tag.
  0.20.1 read the value as ending there but let the tag run on to the real
  `>`; the oracle ends the tag at the first `>` either way, so
  `<img src="http://e.org/a>b.png" alt="a"/>` keeps `http://e.org/a`, has its
  `/>` read as text, and stays open until `</div>` meets `</img>` —
  `Malformed XHTML: Found "</div>" expecting "</img>"`, where the pod said
  valid. The benign twin (`<p title="a>b">text</p>`, which agrees both ways)
  is in the lane beside it, and this one rule subsumes the unterminated value
  above;
- a character that can begin no attribute name reported `unexpected non-end
  of element null::p`, where the oracle says `Unable to read attribute on
  <p>`.

*An image's source is not a hyperlink.* The report expected an external
`<img src="http://…">` to be refused. The oracle accepts it — and `https:`,
a bare path, `file:`, `urn:`, `cid:` and `javascript:` besides: the scheme
rules (active content, "not hyperlinkable") are checked on `<a href>` only,
an image's `src` only for an empty value, invalid characters, a `data:`
URL's comma and a `#fragment` that must resolve. Each of those probed, each
recorded, and the default stays the oracle's. `docs/scope.md` gains a
**Narrative** section saying so.

**Strictness is the host's call — `fhir_pods_options` (ABI 14).** The report's
sharpest point: `profile.unknown` is a warning and the resource stays valid,
so a host that has not compiled a profile it declares *fails open* unless it
remembers to filter for the warning. The warning stays the default (D004: it
is the oracle's own severity, and every lane measures that pod), and a host
can now ask for the other thing:

```json
{ "unknownProfile": "warning" | "error", "externalNarrativeLinks": "allow" | "refuse" }
```

`"error"` reports the same finding, with the same id, at `error` — fail
closed. `"refuse"` makes a narrative reference that leaves the resource an
error (`narrative.external-link`, the one new id), which is where the field
report's external image lands. An options document that does not say what it
means is **refused whole** — an unknown member, a value no option takes, or a
member given twice, which one review asked for on the same reasoning — and
the options already in force stay: a typo in a strictness setting must never
read as "off", and nor must a stale line a host forgot to delete. An empty
document restores the defaults.

One entry per delivery, the shape `loadTerminology` / `loadQuestionnaire`
established: `fhir_pods_options(json)` on the C ABI, `options(json)` in the
wasm and Expo pods (`HostOptions` in both typings), `--options FILE` on the
CLI's `validate` and `gate`, `Validator::set_options` / `with_options` in the
core. The pods persist them for every call that follows — validate, lower,
lift, export, document, v2 — and a Questionnaire load carries them through.

**De-identification says what it does.** The wrapper's comment promised "a
Safe Harbor copy (by default)" — a compliance claim the code does not make
and cannot. What `deidentify` does is apply a **policy object** — a rule set
— to a typed walk and return the copy with a trail; the built-in policy is
modelled on HIPAA Safe Harbor's identifier list, by FHIR type. The wording is
corrected wherever it appeared (the wasm wrapper and both pods' typings and
READMEs, the C header, the FFI and core docs, the main README, the CLI guide,
`docs/architecture.md`) and `docs/scope.md` gains a **De-identification**
section stating the claim and its limits: every element the rules name is
transformed as the rule says, `applied` lists every change, `remaining` is
the policy's own check over the copy — and nothing about re-identification
risk from what remains, the host's other data, or how the copy is used is
claimed. It is not an expert determination and not a certification. Two
limits now said rather than discovered: the **salt is required** (the call
refuses without a non-empty one rather than emit unkeyed pseudonyms), and an
age is only aggregated where the pod can know it is one.

**`collapse-age`, a policy action (the host names the element).** The report
expected ages ≥ 90 to collapse. The walk already aggregates an `Age`-typed
element over 89 — the type says what it is — but nothing says which
*integer* is an age, so the pod never guesses. A rule now names it:

```json
{"target": "QuestionnaireResponse.item.answer.valueInteger", "action": "collapse-age"}
```

Over 89 then collapses: an integer or decimal to `90` (a primitive carries no
comparator, and the host that named the element knows what 90 means there),
and a quantity whose type *always* bears a comparator — `Age`, `Duration`,
`Count`, `Distance` — to `{"value": 90, "comparator": ">="}` with the
90-equivalent in its own unit (each probed;
`conformance/probes/age-collapsed-*`, the probe lane now 37/37).

A plain `Quantity` takes the bare `90`, and the review that found this is
worth recording: the base definitions constrain **55** `Quantity` elements to
`SimpleQuantity` — `Observation.referenceRange.low`,
`MedicationAdministration.dosage.dose`, `Claim.item.quantity` — where
`comparator` has max 0 and `sqty-1` refuses it (`MoneyQuantity` constrains no
base element in R4, so it is prospective), and R4 writes that as
`{"code": "Quantity", "profile": [".../SimpleQuantity"]}`, so the pod's
element-type table (type *codes*) cannot tell those from a free `Quantity`.
Writing the comparator there produced a copy the oracle rejects while the pod
called it valid. The writer is now type-safe, and the leniency behind it — the
pod does not apply a datatype profile the base puts on an element — is
declared in `docs/scope.md` with the probe, since closing it needs
element→profile data in the generated tables and a walk that applies it.

A rule naming an element no age can be read from — a `CodeableConcept`, a
quantity in kilograms — leaves it alone and names it in `residual`, so a rule
that cannot bite is never silent, and the walk's own `Age` clamp is subsumed
where both would fire.

**`fatal`, said out loud — and a severity for every rule id.** The pod emits
`severity: "fatal"` and nothing told a host so. `docs/rules.md` now opens with
the severity set (`fatal` / `error` / `warning` / `information`), what `valid`
means in each one's presence, and **when `fatal` is emitted**: exactly two
findings, `parse.json` (the input is not JSON — there is no document, so that
finding is the whole report, and a short issue list there must not read as
"almost conformant") and `questionnaire.definition` where a held-to
Questionnaire has an item with no `type` (the response cannot be judged
against that item; the Questionnaire's own findings stand beside it). Both
probed against validator_cli 6.10.4: it reports `fatal` for the same two
inputs. The C header carries the same paragraph, and so do both pods'
typings, where a host reading the packed pod actually looks.

The rule table gains a **Severity** column — every one of the 67 ids, read
from its push site and, where the lanes produce it, *measured*: a new test
scans every file the release corpus, the mutation lane, the gut cases and the
two probe lanes have on disk, and fails if an id ever arrives at a severity
the page does not list for it. So `profile.unknown` is "warning — error
where the host sets `unknownProfile`", `questionnaire.unresolved` is
"information when none
is named … warning when a named one could not be resolved",
`not-supported.fhirpath` is "information for a profile invariant, warning for
a datatype one", `questionnaire.answer-content` is "error — warning for a new
line in an answer".

**Why five ids read as undocumented, and what changed.** The host reported
`questionnaire.unresolved`, `questionnaire.definition`,
`questionnaire.item-shape`, `structure.json-type` and `parse.json` as having
no entry in the table. All five *were* in `docs/rules.md` at 0.21.0 — with a
one-line meaning, no severity and no statement of what produces them, and in
a file that does not travel with the packed pod. Each now carries its
severity and its condition (what `structure.json-type` counts as a wrong
kind, including a `_` carrier given as an object; what `item-shape` forbids;
that `questionnaire.definition` is a defect in the *Questionnaire*, not in
the response), and the forward-only documentation test is now **both ways**:
every id in the table is on the page with its severity and meaning verbatim,
and every id the page lists is in the table — a hand-edited row or a stale
regeneration fails.

**ABI 14**, and the policy written down (`docs/architecture.md`): the number
rises whenever the exported surface or the memory contract changes, an added
entrypoint included, because a wrapper that calls one must not be handed a
module that does not export it. A new member in a returned document, a new
rule id, a reworded message do not move it — 0.20.0 added `issues[].id` at
ABI 13 for exactly that reason. The architecture doc said "ABI 10" and is
corrected to 14 with the whole ladder — and its claim that every wrapper
refuses a mismatched module is now true of the Expo pod as well: it contains
its native library rather than being handed one, so a mismatch means
mismatched artifacts were packed, and `src/index.ts` refuses it on import
while the Swift and Kotlin modules log it on creation. The shared Node/Chrome
harness and the C smoke exercise the new entry; the browser and Expo device
evidence still reads ABI 13 and wants re-recording from a packed build.

Lanes, all green and all unchanged but for the two new probes: release gate
**1,778/1,778** (1,776 + the two age probes), base **5,156/5,290** (134
lenient, 0 stricter, 0 failures), probes **37/37**, narrative **42/42**,
277 tests, clippy clean, 22 wasm Node tests, the C ABI smoke and the no-std
build. Released as 0.22.0 (ABI 14); the pods' packed artifacts carry it.

## 0.21.0 — 2026-10-10

**Host profiles, as a build.** A host's own profiles — the gut module's
score `Observation`, say — compile into the pod the way the shipped registry
does: `scripts/gen-host-profiles.sh <package dir>` writes
`src/generated_host` (a stub registry until then), `--features
host-profiles` merges it after the shipped profiles (the pods' build scripts
take `FEATURES=host-profiles`), and a canonical the shipped registry already
holds is refused rather than shadowed. The host package is primary — every
resource, `Identifier` and complex `Extension` profile in it is compiled —
and the pinned packages are what it derives from and slices by; a
dependency profile the generator re-emits for a host slice is pruned, the
shipped copy serving. Proven end to end in the gut-path lane
(`conformance/gut.py --host-profile`): a synthetic gut-score profile (code
pattern, integer value, member, device, derivation) with a snapshot built
from R4's Observation, compiled into a feature build in its own target dir,
seven cases judged against validator_cli given the same definition, the
stub restored afterwards. 7/7 agree.

A fresh review of the first cut shaped the generator: the registry is now
written only once the whole package checks out (generated into a scratch
dir, moved into place), the refusals mirror the generator's own rule
(a constraint with a snapshot on a resource, an `Identifier` or a complex
`Extension` — a base-resource copy in the package is not refused, a
differential-only definition is, by name, since it would compile to
nothing), provenance names the package rather than its path, the host
profiles' invariant keys get stable rule ids (`tools/gen_rules.py` scans
the host registry too, and `--reset` restores the shipped set), a release
ignores an inherited `FEATURES` and checks the pods' passthrough, and the
lane resets the stub whatever fails. `rust-version = "1.83"` is declared
(the registry's const reads).

Regions: the Gulf stays on the IPS floor — no FHIR package for NPHIES,
Malaffi or NABIDH is published on packages.fhir.org or Simplifier as of
today (`docs/scope.md`). ABI 13 unchanged.

## 0.20.1 — 2026-10-09

**The narrative is checked.** `txt-1` — the narrative holds to the basic
HTML formatting elements — was the one invariant on every resource the pod
skipped, because `htmlChecks()` was outside the FHIRPath core. It is in the
core now, and the walk checks every `Narrative.div` the way validator_cli
6.10.4 does — every rule below probed (some 2,500 narratives, including
every HTML entity name), none read from its code:

- a `div` root, and every element in the XHTML namespace (declared on it
  or an ancestor; a prefix nobody declared is its error namespace);
- the narrative's element list and the attributes each element allows
  (a bare attribute is taken, then judged by name);
- the content model — a `li` in a `ul`|`ol`, a `td` in a `tr`, no text in
  a table or list, no `a` inside an `a`, a paragraph's invalid elements at
  any depth, and a block structure inside a *text element* (`p`, `span`,
  `b`, `h1`…) reported against every text element above it;
- character references — XML's five decode; a name the reference parser
  knows from HTML (`&nbsp;`, `&copy;` … 2,116 names, generated by
  `tools/gen_xhtml_entities.py`) is an invalid entity; any other name, a
  reference without its `;`, and a numeric reference out of Unicode's
  range are parse failures; a numeric reference that does not parse as a
  Java `int` passes, as it does there;
- hyperlinks, on the decoded value: not empty, no invalid character (each
  counted, each named once), a `#fragment` resolves to an `id` or an `a
  name` (an image's only to an `id`), a `data:` URL carries data, and the
  scheme — exactly as written, so `javascript:` is active content and
  `JavaScript:` is not — is neither active content nor `urn:`/`cid:`;
- the reference parser's own tolerances: unquoted and repeated attributes,
  a doctype (to its first `>`), an XML declaration, a BOM, comments, CDATA
  (not narrative text), an element left open at the end, text or a second
  root after the root (ignored, and not content), and its nesting limit of
  500 — the pod's parser is iterative, so that depth costs no stack.

A parse failure stands alone with `txt-1` and `txt-2` (there is no
document to check); otherwise each finding is an error of its own, with
`txt-1` beside the element, attribute and parse findings only, one
finding per distinct message, as the reference validator reports them.
`txt-2` reads from the same parse: decoded text beyond a Java trim or an
image, anywhere under the root. Six new rule ids: `narrative.parse`,
`narrative.root`, `narrative.element`, `narrative.attribute`,
`narrative.entity`, `narrative.link`. A profile restating `txt-1`/`txt-2`
(US Core CarePlan does) no longer reports them twice. Twenty-five THO
code systems and value sets whose narratives link to anchors they do not
carry moved from the base lane's lenient column to agreement.

Found by the gut-path review: a clinician's summary document carries a
narrative per section, and the pod had nothing to say about any of them.
A fresh review of the first cut found the recursion, an entity-encoded
`javascript:` href slipping past the scheme check, and the duplicated
profile invariant — all three closed before the tag, and every oracle
question it raised answered by probe rather than by assumption. Lanes:
1776/1776; base 5,156/5,290 (134 lenient, 0 stricter); mutation 0
untriaged; gut 71/71 + 710 derived; 268 tests.

## 0.20.0 — 2026-10-09

**Stable rule ids, and a gate.** Every finding now carries a machine-
readable id — `issues[].id` in the pods' JSON report, `issue.details.coding`
under system `urn:fhir-pods:rule` in an `OperationOutcome` — so a host
asserts on *which* rule fired, never on message text: `binding.no-code`,
`reference.target-type`, `questionnaire.required-answer`, `inv.cmp-1` …
59 families and one id per invariant key (361 of them); the contract is
`docs/rules.md` (an id never changes meaning; a new kind of finding gets a
new id; `other.<type>` is the fallback and a defect). The id is derived
where the finding is recorded, from its type and message family
(`src/rules.rs`), and a test holds every finding on the release corpus,
the 19,067 mutation files and the gut-path scenario to zero `other.*`.

The CLI gains `gate`: `fhir-pods gate [--questionnaire Q.json]…
[--terminology T.txt] FILE…` validates every file and prints one report —
each file's verdict and error-level findings by id, the counts by id over
the run — exiting 1 when any file has an error. That is what runs on a
vendor's deliverables and in CI. `--questionnaire` (repeatable) is new on
`validate` too: the instrument a response names, loaded the way a host
loads it.

The JSON report and the `OperationOutcome` gain a field each; nothing is
removed and the C ABI is unchanged (ABI 13). The wasm and Expo typings
carry `id`. 
Three gut-path gaps closed in the same release:

- *A vendor's value sets into the pod without a terminology server.*
  `tools/compile_valueset.py` now compiles a `ValueSet` that carries no
  expansion from its `compose` when — and only when — the set is enumerated
  there: every include lists its concepts, or names a system whose
  `CodeSystem` is given on the command line with `content: complete`; an
  enumerated exclude is applied; a filter, an intensional include or a
  system not given complete is refused with the reason. The gut-path
  scenario now compiles its table through it.
- *De-identifying a QuestionnaireResponse keeps its answers.* Safe Harbor
  removes a free-text answer's value; the answer was then empty and
  dropped, and a required item had no answer — an invalid copy on both
  validators. The value is now masked in place: a value-less answer with a
  data-absent-reason carrier (`_valueString`), which validator_cli 6.10.4
  passes against the instrument (probed; the dropped form it refuses).
- *A response naming another version of a loaded questionnaire* is now
  said — a warning (`questionnaire.version-mismatch`) a host pinning
  instrument versions can gate on — and not judged against it; the
  reference validator resolves by url alone, and a host that wants that
  loads the version it means.

A fresh review of the rule table found seven questionnaire messages and
three invariant keys (`sdf-8a`, a profile's `pvc2` and
`hasMemberOrComponent`) that fell through to `other.*`, and one clause that
claimed any message mentioning a fullUrl; all fixed before the tag, the
invariant table now generated from the keys the core reports rather than
a shape, and each corner pinned as a test sample. Lanes as 0.19.4; 267 tests.

## 0.19.4 — 2026-10-09

**Two new lanes — a mutation lane and a gut-path lane — and the nine
findings they made.** Three releases in a row had shown the same thing: a
corpus of publisher-valid examples can only show where the pod is stricter
than the reference validator, never where it is more lenient. The mutation
lane (`conformance/mutate.py`) derives every release-corpus original under
nine fixed mutations — drop an element, blank or strip a coding, wrong
JSON type, empty backbone, bad enum, null primitive, empty string, bad or
bare reference, orphan extension — 19,067 derived files, every one put
before validator_cli 6.10.4, lenient cases first. The gut-path lane
(`conformance/gut.py`) is a synthetic sixteen-part clinical instrument
with its own code systems and value sets, responses held to it, and the
chain a host builds around it — derived observations, an alarm `Flag`, a
clinician-summary `Composition`, a referral, research-export `Consent`,
the export `Bundle`, the `AuditEvent` — judged three ways (the scenario's
expectation, the pod, the oracle with the same definitions loaded as an IG
folder), then lifted and lowered, de-identified, and exported for the AU
region, every derived file judged again. Both lanes close at **0
untriaged disagreements**.

What they found, each probed against the oracle and fixed:

- *A profile's undecidable set hid the base's.* `Observation.status` of
  `mutated` under an IPS profile was valid: the profile binds `status` to
  a subset the pod cannot expand offline, and the base's required set was
  never consulted. A profile only narrows a required base set, so where
  the profile's verdict cannot be decided the base's now is — on `code`,
  `Coding` and `CodeableConcept` alike.
- *`Reference.type` was never checked.* It is held to the element's
  targets — the base's where no compiled profile applies, the profile's
  under one — by bare type name (a canonical is wrong there), and only
  beside a `reference` (an identifier-only reference's `type` is not
  judged). Base targets are a new generated table.
- *A binding beneath a choice was not judged.* IHE BALP's `ihe-otherId`
  binds `Extension.value[x].type`; the table cannot type a path through a
  choice, so the instance's shape now decides (codings or text — never a
  `Quantity`'s value and unit).
- *Vital-sign profiles are implied by the oracle's code table, not the
  magic code alone:* `3141-9` is a body weight, `8306-3` a body height,
  `8331-1` a body temperature, `55284-4` a blood pressure … — each code
  probed; the aliases are listed in the source.
- *THO value sets were not bundled.* `sex-parameter-for-clinical-use` and
  its kin are now compiled from `hl7.terminology.r4`; 33 sets remain on
  the seam.
- *A code-less coding was an error even where the set is undecidable*
  (0.19.3 over-reached): the oracle says nothing there, so neither does
  the pod beyond "not checked"; where the set is known, the error stands.
- *An empty string beside a carrier with content* (`"family": ""` with a
  `_family` translation extension) is a value absent, not a value wrong;
  a required binding still wants its code.
- *A relative reference is `Type/id`* inside a Bundle (`1` alone is the
  oracle's format error); a type the base does not define passes and is
  not held to a document's contents.
- *A coded answer with no system* names nothing a questionnaire's value
  set could hold — the oracle's terminology error, not a pass — and three
  questionnaire labels moved to where the oracle places them (the item
  holding a stray or duplicate, not the stray).
- *A resolved reference in a Bundle is held to its element's base targets
  by the target's own type.* The gut-path chain's first referral named a
  `Flag` as its reason; the oracle refused the AU document ("Invalid
  Resource target type. Found Flag, but expected one of ([Condition,
  Observation, DiagnosticReport, DocumentReference])") and the pod did not.
  Resolved as the oracle resolves: by `fullUrl`, a relative `Type/id` only
  against a RESTful one — among `urn:uuid` entries it names nothing and is
  not judged (probed). A resource declaring a compiled profile is held to
  the profile's targets by the engine as before.

Every lane as 0.19.3 — release corpus 1,776/1,776, no untriaged error
difference, base examples 5,130/5,290 with 0 stricter, questionnaire 74/81
and 73/81; the mutation lane 19,067/19,067 on verdict; the gut-path lane
71/71 — 262 tests. ABI 13 unchanged.

## 0.19.3 — 2026-10-06

**A content-referenced backbone's invariants hold at every depth; a
required-bound concept whose codings carry no code is an error; a required
repeating element that empties under de-identification stays an array.**
Three findings from the review lane's missing-content probes, none of them
visible to a published example.

- *Invariants beneath `contentReference`.* A section's section, an item's
  item, an answer's item, a component's referenceRange are the same
  backbone again, and validator_cli evaluates its invariants there —
  `cmp-1` on an empty inner section, `que-1 … que-13` and `qrs-1` on
  nested items and their children (`que-7` on a nested `enableWhen`),
  `obs-3` on a component's range. The pod had evaluated them where the
  base defines the backbone only (and said so in scope). The engine now
  follows the base table's content references to every copy the document
  holds — beneath the backbone however deep, or elsewhere in the resource
  — and reports a failure at the path it fails (`Composition.section.section`,
  `Observation.component.referenceRange`), profile or none. A copy
  carries the backbone's invariants *and* its own, as the oracle has it: a
  `TestScript` test operation with no target fails `tst-7` (the setup
  operation's) and `tst-8` together, a teardown's `tst-7` and `tst-9`,
  never a sibling copy's. Regression tests: empty inner and innermost
  sections, bare and under AU Base Composition; nested `que-1` and
  `que-7`; `obs-3` on a component; the three `TestScript` phases; the
  valid twins clean. Eleven probes against the oracle.
- *`answer is Boolean`.* `que-7` (an `exists` condition's answer must be a
  boolean) failed every `answerBoolean` — at any depth — because the
  FHIRPath `is` took a bare `Boolean` as a System type no FHIR element is.
  The reference engine takes it for a FHIR `boolean` (probed: `que-7`
  passes an `answerBoolean` and fails an `answerString`), while `is
  DateTime` on a `dateTime` stays false (re-probed through `us-core-1`,
  which still never fires). The one name is now matched that way; a
  questionnaire with a required `exists` answer validates as written.
- *A `CodeableConcept` whose codings have no `code`.* 0.19.2 made a concept
  with no coding at all an error; one whose Coding objects carry a system
  and a display but no code slipped through. The oracle rejects the parent
  concept ("None of the codings provided are in the value set") while
  accepting a standalone required-bound `Coding` with only a system
  (`Encounter.class`), so the rule is the concept's, not the Coding's. Both
  the base walk and the profile engine now say so; the standalone Coding
  stays unjudged. Oracle-backed probes, bare and under AU Core Condition,
  are in the conformance lane (review-agent, F091).
- *De-identification of a required element that repeats.* A
  `Composition.author` list of display-only references loses every display
  as free text, empties, and — being required — is masked. The mask was a
  bare object where the element is an array, so the copy was not R4 JSON
  and the pod itself rejected it. The emptied-element path now builds the
  same stub the explicit mask does: an array of one where the source was an
  array, with the profiles' required children masked inside. Regression
  test: the de-identified Composition carries `author: [stub]` and
  validates.

Every lane as 0.19.2 — release corpus 1,776/1,776 (ten review probes
joined it), no untriaged error difference, base examples 5,130/5,290 with
0 stricter, questionnaire 74/81 and 73/81 — 253 tests. ABI 13 unchanged;
the review lane's C abuse harness now covers `fhir_pods_questionnaire_load`
(F092) and the Expo artifact was exercised on iOS 26.5 and Android 15
(24/24 checks on each).

## 0.19.2 — 2026-09-30

**A required-bound `CodeableConcept` with no coding is an error.** A host
exporting allergies of unknown clinical status — `clinicalStatus` carrying
only a data-absent-reason extension — found the pod said valid where
validator_cli and a second validator said invalid. Probed against the
pinned oracle (6.10.4, `-tx n/a`), bare and under AU Core: a concept with
no coding at all — text only, an extension, nothing — is *No code
provided, and a code is required from the value set* on a required
binding; a coded one is fine. The pod had assumed, in both the base walk
and the profile engine, that the reference validator only warns there;
it never had, and no published example carries such a concept, so no
lane could say so. Both paths now report the oracle's error. The profile
engine judges it only where the element *is* a `CodeableConcept` (the
profile's own type, or the base table's): a `Quantity` bound to UCUM
(`bp`'s `component.value[x]`) carries the same binding kind and no
codings, and is judged by its system and code, never by their absence —
the first cut got that wrong and the export tests and base lane caught
it before it left the tree. A coding without a code is still not judged.
Regression test: DAR / text / coded × bare / AU Core. Every lane as
0.19.1 — release corpus 1,766/1,766, no untriaged error difference, base
examples 5,130/5,290 with 0 stricter, questionnaire 74/81 and 73/81 (no
published example has such a concept, so no lane moved); 251 tests. ABI
13 unchanged.

## 0.19.1 — 2026-09-29

**A loaded Questionnaire now holds a response on every path of the C ABI,
not only `fhir_pods_validate`.** A host's report against the 0.19.0 wasm
wrapper: `loadQuestionnaire()` returned true, `validate()` held the
response to the form, but `lower()` and `lift()` said the very canonical
just loaded could not be resolved — a string answer to a required integer
question, or no answer at all, came out `valid: true` through lowering
and lifting. The cause was in `crates/fhir-pods-ffi`: the loaded forms
lived in a store beside the process's one immutable `Validator`, and only
the validate entry points passed that store in; lower, lift, export,
document and v2 called the validator bare. Now the ABI holds one
replaceable validator snapshot that carries the loaded forms: a load
clones the current validator (its parsed invariants included — `Validator`
is `Clone`, and `clear_questionnaires` joins the core API) and replaces it
whole under the write lock, so every operation of a call sees one
consistent store, an exact `url|version` is selected, a version not loaded
is unresolved, clearing and reloading move the verdict, and a wasm
instance's store is its own. ABI 13 unchanged. Regression evidence at
every layer the report asked for: an FFI test over the C entry points
(`a_loaded_questionnaire_holds_lower_and_lift_as_it_holds_validate` —
wrong type, missing required answer, the valid control, through lower,
lift and validate alike; version replacement; clear; refused garbage),
the C smoke (`smoke.c`: a lowered response held to the loaded form, then
cleared), and the wasm smoke test (the reporter's own repro, plus a second
instance). The core's verdicts are untouched (the only core change is
`Clone` and `clear_questionnaires`), so the lanes stand as 0.19.0; 250
tests.

## 0.19.0 — 2026-09-29

**A form and its answers are lowerable.** The base `Questionnaire` and
`QuestionnaireResponse` definitions join `BASE_RESOURCES`
(`tools/gen_all.py`) and are compiled as lowerable profiles — the gap
0.16.0 left: the pod held a response to its questionnaire but gave a host
no slots to build either from. `slots(…/QuestionnaireResponse)` now lists
`questionnaire`, `status`, `subject`, `encounter`, `authored`, `author`,
`source`, `item` (given whole: the items are the recursive backbone the
base spec defines by `contentReference`, so nested groups ride inside the
one slot, and come back whole from `lift`); `slots(…/Questionnaire)` the
form's metadata and its `item`. A lowered response is held to its
questionnaire on the way out like any other (`with_questionnaire`, a
contained one, a bundle entry) — a skipped required question is the
form's finding, not lowering's. And a rule the change itself exposed
(found in review before it shipped): a profile's snapshot enumerates a
`contentReference` backbone once, at the depth the base defines it, so a
resource declaring a compiled profile lost the base's required elements
and bindings beneath the deeper copies — a lowered response whose nested
item had no `linkId` came out valid. The walk now keeps the base's checks
beneath a content reference whether or not a profile applies
(`through_content_reference`), for every such backbone (an item's item, a
section's section), and the declared base profile asks exactly what the
bare check asks — a test holds the two error lists equal. Registry 1,147
→ 1,149; ABI 13 unchanged. Three tests (lower; lift both back to the same
inputs; the equality). Every lane as 0.18.0 — release corpus 1,766/1,766, no untriaged
error difference, base examples 5,130/5,290 with 0 stricter, questionnaire
74/81 verdicts and 73/81 on every error; 249 tests.

## 0.18.0 — 2026-09-23

**A Synthea cohort, at scale** (`conformance/synthea.py`). Synthea v4.0.0
(pinned jar, sha256-checked, fetched into a gitignored cache) generates a
seeded cohort — every patient a transaction Bundle of hundreds of US Core
6.1 resources, plus the hospital and practitioner bundles — and every
bundle goes before the pod and validator_cli, compared on the verdict and
on every error-level issue, the pod's time per bundle recorded. Nothing is
committed; nothing is a real person. Verdicts and every error agree on the
whole cohort (10 patients: 14 bundles, 27.3 MB, 9,482 entries; 14/14
verdicts, 14/14 on every error-level issue; the pod 3.96 s in all, 6.9
MB/s, the largest bundle 9.7 MB in 1.48 s). A 50-patient cohort (58
bundles, 231 MB) is beyond the oracle, not the pod: validator_cli in one
process passed 10 GB resident after an hour without finishing and was
stopped — a larger cohort wants the oracle run in batches, which the lane
does not yet do. Two rules it taught:

- **Any extension no package defines is refused**, wherever it is named —
  the reference validator's `allowUnknownExtension` lets only `example.org`,
  `acme`, `nema.org` and the tools extensions through, and now so does the
  pod (0.17.0 refused only HL7's own space). Synthea's
  `synthetichealth.github.io` extensions make every cohort bundle invalid
  for both validators, and the lane says so.
- **A reference target conforms to a profile only when the resource is
  also sound on its own** — the oracle's whole verdict on the target, not
  the profile's constraints alone; a target that need only be a base type
  is judged by type alone (the base definitions are compiled for lowering,
  never held against a target).

And where the time went: a reference was resolved by a walk over every
entry, and the same Patient validated once per reference to it. A
per-validation memo (`engine::Memo` — entries indexed once, each target's
conformance to each profile remembered) took the largest bundle (9.7 MB,
3,000 entries) from 13.7 s to 1.5 s, the ten-patient cohort from 22 s to
4 s (6.9 MB/s). Base examples 5,130/5,290, 0 stricter; all other lanes as
0.17.0; 246 tests.

## 0.17.0 — 2026-09-23

**Two more corpora, and what they found.**

- **Supporting-resource lane** (`conformance/supporting.py`): fhir-test-cases'
  R4 JSON cases that come with `supporting` definitions. A supporting
  StructureDefinition goes through the ad-hoc profile lane's own machinery
  (Java snapshot, isolated compile, feature build — reused, not edited); a
  supporting ValueSet / CodeSystem that enumerates is compiled into a
  terminology table the probe loads (`probe --terminology`). 11 of 160
  compare (the rest are the questionnaire lane's, need a package, an IG, a
  validator option, a filter, or a snapshot Java cannot make); 9/11 agree,
  0 stricter; two reviewed (`supporting-triage.json`).
- **HL7 v2 sample lane** (`conformance/v2samples.py`,
  `examples/v2_samples.rs`): Microsoft FHIR-Converter's 139 synthetic v2
  messages (MIT, pinned, fetched into a gitignored cache) through the
  converter — 77 convert, 62 name structures the maps do not cover — and
  every bundle before validator_cli. Verdicts 77/77, every error-level
  issue 74/77, 0 stricter; three reviewed (a cross-version extension URL
  the oracle rejects by element id). All 77 bundles are invalid on both
  sides, and the lane says why (an ADT's Provenance with no target, XTN
  telecoms with no system, sample codes outside their sets) — reported,
  not hidden.
- **Converter defects the samples exposed, fixed:** `NM → Quantity` wrote
  the number bare (`$value` on a complex type is its `value`);
  `deceasedBoolean` from table 0136 came out as the string "false";
  `MessageHeader.source` was missing whenever MSH-24 was valued (the IG's
  `MSH-3 → source[1]` names no datatype map — the backbone's own
  `HD → messageheader-source` is attached now — and `MSH-24 → source`,
  a string into a backbone, lands on the child the row's own comment
  names); an extension `url` with the literal in the row's comment, not
  its assignment, was written with the source's value (a date); an HL7
  table THO does not publish (0296) got a made-up THO uri; the IG's
  literal misspellings (`us-ssn.`, `v5.0`, `https://` on cross-version
  extensions, `extension-subscriberId` without its resource,
  `www.hl7.org/fhir/extension-condition-asserteddate`) are corrected as
  spelling by the generator, each counted.
- **Verifier rules the oracle showed us:** `Identifier.system` must be an
  absolute URI; a top-level `Extension.url` must be absolute; an extension
  named under HL7's space that no package defines is refused (a
  well-formed cross-version one is taken on trust); a `Coding.system`
  under `terminology.hl7.org` that THO does not define is an error; a
  resource `language` is a BCP-47 tag; a Bundle entry's fullUrl "looks
  RESTful" by validator_cli's own regex (`BundleValidator.urlRegex`,
  mirrored — its type list is STU3-era and spells `Condition` as
  "Condition (aka Problem)", so a Condition fullUrl is never judged, here
  as there); `resolve()` of a `#contained` reference in a Bundle-level
  constraint finds the entry's contained resource.
- **A FHIRPath engine bug:** `countMax` was read as a choice of `count`,
  so `count.exists()` was true — `tim-8` never fired. A choice key is the
  stem plus a FHIR type name, nothing else.

Verdicts: base examples **5,130/5,290**, 0 stricter (311 reviewed
leniencies closed by the new rules and dropped from `base-triage.json`);
release corpus 1,766/1,766 by the reasons lane (3 triaged); test cases
Java 63/84; questionnaire 74/81; 246 tests.

## 0.16.0 — 2026-09-22

**QuestionnaireResponse against its Questionnaire** (ABI 13). A response
naming a questionnaire — contained, a bundle entry, or one the host
loaded (`Validator::with_questionnaire` / `load_questionnaire`, C
`fhir_pods_questionnaire_load`, wasm and Expo `loadQuestionnaire`) — is
held to it as validator_cli 6.10.4's `QuestionnaireValidator` holds it,
its rules read from the bytecode and its words kept (`src/questionnaire.rs`,
`docs/scope.md`): items defined, in place and in order; required answers
(a warning while in-progress) and required groups; `enableWhen` with the
answers found up the response tree; one answer unless the definition
repeats, `minOccurs` / `maxOccurs`; the answer's type and bounds
(`minValue` / `maxValue`, `maxDecimalPlaces`, `minLength` / `maxLength`,
`regex`, attachment `mimeType` / `maxSize`, `referenceResource`, quantity
units and the SDC min/max quantity); `answerOption` per type with
`optionExclusive`; `answerValueSet` membership through a value set
contained in the questionnaire or the terminology provider, under R5's
`answerConstraint` (cross-version extension). A questionnaire that cannot
be found is a warning, and nothing is judged. A `Questionnaire`'s own
`enableWhen` wiring is checked. New lane `conformance/questionnaire.py`
over fhir-test-cases' `questionnaire` module: 81 of 96 R4 JSON cases
compare, 74/81 verdicts agree, 73/81 agree on every error-level issue,
0 stricter; eight reviewed differences in `questionnaire-triage.json`
(terminology server, display names, attachment integrity, an SDC package).
Other lanes unchanged (gate 1,766/1,766 by the reasons lane; base
4,819/5,290; test cases 62/84); 246 tests; wasm 2,945,757 bytes.

## 0.15.0 — 2026-09-22

**Every error, both ways.** A verdict lane proves two validators agree on
valid/invalid; it cannot see an error only one of them raises on a file
both reject. `conformance/reasons.py` reads the oracle's full output beside
ours and lists, per file, each error-level issue one side raises at an
element where the other raises none (paths to one spelling, messages
matched by family, a parent/child locus only when the messages agree);
it gates on `reasons-triage.json`, which names the accepted residue path
by path and goes stale loudly. On the corpus that lane found, and this
release fixes:

- **The base definitions, profile or none.** Every datatype's own
  invariants are evaluated on every object of the type wherever it sits
  (`cpt-2` on a ContactPoint in a Patient, an extension, a datatype in a
  datatype; `per-1`, `qty-3`, `rng-2`, `rat-1`, `att-1`, `tim-*` …), and
  every `required` binding the base places on a coded element — 366 of
  them, `Patient.gender`, `Observation.status`, `ContactPoint.system`,
  `Quantity.comparator`, `Narrative.status`, `Timing.repeat.periodUnit` —
  is checked by the shape walk, with the reference validator's wording;
  a code present only through its `_carrier` still owes one ("No code
  provided, and a code is required from the value set"). A Patient with
  `gender: "banana"` was valid before this release. The value-set table
  grew from 146 to 289 sets to hold what the base binds; a profile
  snapshot that merely restates a datatype's invariant or binding is no
  longer compiled twice. Sixteen fixtures pin the family.
- **A `uri` / `url` in HL7's own space must name a definition.** Under
  `http://hl7.org/fhir`, `http://terminology.hl7.org/` or
  `http://fhir.org/guides`, a value the packages do not define is an
  error (`http://hl7.org/fhir/sid/us-ssn.`), as validator_cli grades it
  — an unknown URL anywhere else is taken on trust. The 7,983 canonicals
  and NamingSystem ids ship as sorted 32-bit hashes (`known_urls.rs`,
  generated).
- **Reference targets, as the oracle judges them.** A Reference element's
  `targetProfile`s are enforced on a target that can be reached: a bundle
  entry by conformance ("Unable to find a profile match for … among
  choices"), a `#contained` resource by type only. A bundle entry's
  allowed-profile match (`Bundle.entry.resource` typed with profiles)
  applies the resource's own declared profiles too — one it fails sinks
  every candidate ("The entry resource did not match any of the allowed
  profiles"). `ref-1` resolves against the ids of the nearest resource
  that is not itself contained (`%rootResource`), never a nested
  contained's. `dom-3` is reported per orphaned contained resource.
- **Every resource's own invariants** (`org-1`, `pat-1`, `obs-6` …, 170)
  on an unprofiled resource; `uri` / `url` / `oid` / `uuid` / `canonical`
  reject whitespace; a required-gap and a carrier's extension are located
  where the reference validator locates them
  (`name[0].family.extension[0]`); two keys for one choice are one
  finding, not also a count.
- **Lift covers a decorated member whole**: `gender`, `name`, `birthDate`
  carrying an extension are the record's, so the NZ export no longer
  writes an empty Patient; `carry_back` creates the target array it needs.
- Export preserves unextracted Quantity qualifiers, extensions and fixed
  primitive carriers where its output shape permits, naming the rest in
  `extras`. Copying a primitive and its carrier no longer duplicates a key.
- Patient consistency accepts versioned relative references and rejects
  recognizable foreign Patient ids in absolute references. Unresolved
  authorities and logical references receive explicit host-resolution notes.
- Lift results include `conflicts` through C, CLI and JavaScript, using the
  same core serializer. ABI remains 12.
- C and browser checks cover ABI 12 operations, discovery, export and
  terminology callbacks, including reentrancy. Lowering fixtures include
  Ratio and inventory only profiles accepted by the resource lookup.

- **Invariants rewritten as validator_cli rewrites them.** R4 published a
  handful of constraint expressions wrong as written, and the reference
  validator fixes them before evaluating (`FHIRPathExpressionFixer`):
  `ras-2` (a choice cast empty when the element is absent), `con-3`,
  `tim-9`, `que-12`, `pd-1`'s bare `telecom or endpoint`, every canonical
  resource's `*-0` name regex (anchored, and only when a name exists),
  `opd-3`, `eld-19` / `eld-20`. `path::parse_invariant(key, expression)`
  applies the same table, so an unprofiled RiskAssessment with no
  probability is valid here as it is there.
- The URL check exempts what the oracle exempts: a canonical resource's
  own `url`, a bundle entry's `fullUrl` and request `url`, a coding's
  `system`, an attachment's `url`, an endpoint's address, a mapping's uri
  … (its `isExemptPathForUrlChecking`), knows the FHIRPath system types
  and mapping spaces it knows, and drops a `#fragment` before looking.

Residue, each with its reason in `reasons-triage.json`: a wrong display
name (displays are not bundled), a profile declared for another resource
type (one error here, element by element there), a second choice key the
oracle discards unread. Verdicts: gate 1,766/1,766 (16 new fixtures); base
examples 4,819/5,290, 0 stricter (eight reviewed leniencies now agree —
dropped from `base-triage.json`); test-case lane Java 62/84 (57 before),
Firely 14/20, 20 lenient (25 before), 0 stricter; 239 tests.

## 0.14.2 — 2026-09-22

**Slicings nest to any depth** (29 → 21 deferred). The compiler's slice
collector walks an element id level by level
(`medication[x]:medicationCodeableConcept.coding:amt.extension:medicationClass.url`)
instead of stopping at two, recording each level's slice, its sub-rules,
its nested slicings and — for every enclosing slice — the scalar fixed
values an outer `value` discriminator resolves to. AU Core's and AU PS's
`medicationClass` extension slices on the AMT / PBS medication codings are
enforced: their value type, binding and 0..1 cardinality reach an instance
three levels down. The 21 slices still deferred are, every one, a defect of
the published guide (`docs/scope.md`). No other profile changed. Verdicts:
gate 1,750/1,750; base examples 4,811/5,290, 0 stricter; test-case lane
unchanged; 239 tests.

## 0.14.1 — 2026-09-22

**More discriminator shapes compiled** (51 → 29 deferred): the `required`
binding a slice inherits from its datatype profile (AU Core Patient's
`identifier:dva` — `type` bound to DVA entitlements, then the fixed DVA
system enforced); the nearest bound ancestor of a part-path discriminator
(`coding.system` under a bound `coding` or a bound CodeableConcept slice —
CA Condition's `HealthConcernCode`); a nested slice's own binding (AU's
`medication[x]:medicationCodeableConcept.coding:amt/pbs` inside the type
slice); `resolve()` to a base vital-sign profile the registry carries
(AU's `supportingInformation:bodyHeight/bodyWeight` — a second body weight
now exceeds the slice; IPS `DiagnosticReport.result:observation-results`);
and a Bundle entry whose `resource` is typed as the resource itself (the
AU PS and IPS `entry:consent`, `device`, `careteam`, `clinicalimpression`,
`documentreference`, `immunizationrecommendation` — findings now carry
the slice's label). What remains deferred is, in every case, a defect of
the published guide (`docs/scope.md` names each). Verdicts: gate
1,750/1,750; base examples 4,811/5,290, 0 stricter; test-case lane
unchanged; 238 tests.

## 0.14.0 — 2026-09-22

**Slices discriminated by a binding are compiled.** A slice with nothing
fixed where its `value`/`pattern` discriminator points, but a `required`
binding there, becomes `Discriminator::Member { path, binding }`: the
instance belongs to the slice when its coded value is a member of the set
— through the bundled sets, a whole-system set, or the host's terminology
provider. 30 of the 81 deferred slices are now enforced: AU Core's and AU
PS's AMT / PBS medication codings and AMT / AIR vaccine codings, US Core's
`category:us-core` on Condition, DiagnosticReport, DocumentReference,
MedicationRequest, ServiceRequest, Clinical Result and Simple Observation
and its `screening-assessment` slices, IPS's absent-or-unknown procedure,
BALP's `subtype:anyRead / anySearch / anyUpdate` and `DataSources`. An
undecidable set (AMT is SNOMED, the seam) leaves the instance unassigned:
neither claimed nor refused, a warning at the sliced element, a required
slice's minimum not counted against while an instance stands undecided,
and a closed slicing not failed for it. `lift` claims an instance for a
`Member` slice only when the provider says member.

**Whole-system value sets.** A set that is every code of one or more
systems the packages do not ship (AU's `pbs-item`; BCP-47 languages, MIME
types) is bundled as `WHOLE_SYSTEMS`: membership by system alone for a
Coding; a bare `code` stays undecided. 146 sets bundled (+11: the US Core
category sets, BALP's), 6 whole-system, 35 at the seam.

Consequences: 51 slices remain deferred (families in `docs/scope.md`);
the BALP `subtype` codings gain their `display` from the now-bundled
sets, so the CLI's frozen audit fixtures were refreshed (README there
says exactly what changed); an unlabelled slot's minimum is met by
instances its slices claimed (`subtype` beside `subtype:anyRead`).
Verdicts: gate 1,750/1,750; base examples 4,811/5,290, 0 stricter;
test-case lane unchanged; 237 tests.

## 0.13.1 — 2026-09-22

**Deferred slices are reported.** The compiled profile carries the slices
the compiler could not discriminate (`CompiledProfile::deferred`: element,
slice, why — 81 across the registry; `docs/scope.md` lists the families),
and validation emits an information `not-supported` issue whenever a
resource reaches such an element ("slice 'dva' of Patient.identifier in
AU Core Patient was not checked: its discriminator (pattern on type) is
outside the compiler's support"). Verdicts do not change; what was
quietly unchecked is now in the report. An `OperationOutcome` for a clean
resource that reaches one carries that issue in place of "All OK".

The base-examples lane pins its own oracle IG list (`base.py` `BASE_IGS`,
the seven guides its recorded verdicts were taken with) instead of
following `run.IGS`, which 0.13.0 widened for the release corpus; the
lane reruns clean at 4,811/5,290, 0 stricter.

## 0.13.0 — 2026-09-22

**Three more regions.** `export` speaks `uk` (UK Core STU2 2.1.0), `ca`
(CA Baseline 1.2.0) and `nz` (NZ Base 3.1.0), each with the IPS document
(none has a patient summary of its own); `profiles()` lists them. UK Core
has one general Observation profile and no vital-sign ones, so a blood
pressure goes out as `UKCore-Observation` with the base `bp` declared
beside it for the document (`also`); CA Baseline's general Observation
takes vitals and its laboratory profile identifies by category; NZ Base
has no Observation profile, so the document's `vitalsigns` family applies.
Specialised (never by content): UK Core's Lab variants, CA Baseline's
implantable Device and Provider-Registry practitioner profiles. Registry
992 → 1,147 (155 profiles, 84 of them extensions); UK Core's 146 examples
join the release corpus as base resources (its convention is no
`meta.profile`), NZ Base's 5 profiled ones as profiled; CA Baseline ships
no examples and is proven by the export lane alone. The export corpus is
now 78 outputs (six regions).

Two CA Baseline profiles are `unevaluable`: validator_cli 6.10.4 errors on
every Condition ("Slicing cannot be evaluated: Unable to resolve
discriminator … 'binding.valueSet'") and Immunization ("Could not match
any of the 2 discriminators") under them, and the compiler defers the same
slices — `export` never declares them (the IPS document's Condition and
Immunization apply; a host that declares one is told why it was not
applied). Ten UK Core examples are skipped from the gate with reasons
(seven use extensions or a code system the STU2 package does not ship —
the definition-resolution family; three fail the oracle's display-name
check, which needs code-system display content the pods do not bundle).
Release gate **1,750/1,750** (1,560 + 141 UK Core, 5 NZ Base, 39 more
export outputs); base examples 4,811/5,290, 0 stricter; test-case lane
unchanged; 236 tests; wasm 2.69 MB / 650 KB gz (+250 KB for the three
guides).

Selection: a single lineage takes its most specific member only when the
leaf adds no identifying content its root lacks (CA Baseline's laboratory
Observation derives from its general one and fixes a category: identity
decides). Found on the way: CA Baseline's AllergyIntolerance slices
`code.coding` by system, closed, with the SNOMED slice fixed to "no known
allergy" — a SNOMED penicillin coding contradicts it and goes out as given
with the reason.

The package loaders (`gen_all.py`, `gen_valuesets.py`, `gen_codesystems.py`)
find resources by `resourceType`, not file name (UK Core and CA Baseline
name their files their own way); `make_corpus.py` takes `examples/` as
well as `example/`. Generated files no longer carry the package list in
their header (it lives in `mod.rs`), so adding a guide does not rewrite
every file.

Carries the review agent's independent verification of 0.12.1 (`662e857`,
`8704ac0`: Quantity qualifiers and carriers preserved, absolute and
versioned patient references judged, lift conflicts across the C ABI and
CLI, ABI 12 C/browser evidence).

## 0.12.1 — 2026-09-21

The Expo module's podspec and `build.gradle` carried a stale `0.5.1`
version string (a host's `Podfile.lock` said `FhirPodsExpo (0.5.1)` for
0.12.0's binaries); both now read `pods/expo/package.json`. Found by the
ABI 12 simulator exercise: 17 checks on iPhone 17 and Pixel 8, identical
outputs, incl. export for `au`/`us`, the refused contradiction and the
refused foreign patient. No core change.

## 0.12.0 — 2026-09-21

**What a fresh review of 0.11.0's exporter found, taken** (F074–F087 in
planning). The exporter's claims — nothing invented, nothing silently
dropped — were not true as implemented; they are now:
- A record value that disagrees with a leaf the target profile fixes (a
  blood pressure in kPa; the profile fixes `mm[Hg]`) was silently
  overwritten by the profile's value. `lift` now reports `conflicts`
  (single-valued places and a slice's own instance only — a pattern on a
  repeating element is one instance's, which lowering adds), and `export`
  refuses to re-profile such a resource: out as given, the contradiction
  in `note`.
- Content no slot took was dropped, and much of it was not even in
  `extras` (a third `component`, a local coding beside LOINC's, a second
  category, `meta.security`/`tag`/`source`, a note, a performer, an
  issued instant, `code.text`, an AllergyIntolerance's criticality…).
  `lift` now names an item of a sliced or fixed repeating element no
  slice claims (`component[2]`, `code.coding[1]`) and lifts `meta` (all
  but `profile`) as the record's; `lower` keeps that `meta`; `export`
  carries every extra back where the profile's shape allows and lists it
  as `carried`, leaving only what sits beneath a repeating element the
  profile rebuilt in `extras`. A contained performer no longer orphans
  (`dom-3`); the IPS tobacco-use and pregnancy profiles keep the value
  they have no slot for.
- US Core Smoking Status matched every social-history observation (its
  only compiled identity is the category; the code binding is
  extensible) and tied with US Core's pregnancy/occupation/sexual-
  orientation profiles: `specialised` now means never by content, and a
  survivor whose identity is a strict subset of another's is dropped.
- AU Core / AU PS Smoking Status doubled the LOINC coding and lost the
  profile's SNOMED one: the generator no longer emits a spare unlabelled
  slot for a container whose own pattern supplies an instance
  (`gen_profile.py`).
- Another patient's observation went into the summary, valid; two
  Patients picked one silently: a record is one patient's —
  `ExportError::SeveralPatients` / `NotThisPatient(index, reference)`.
- `_effectiveDateTime` (a choice slot's carrier) was refused as "not an
  input"; `lower` resolves carriers of a choice's concrete keys.
- `audit` without a document failed for want of `data`: the patient's
  record (`Patient/<id>`) is what was read.
- `also` went silently missing when the document's candidates tied; a
  target that is or derives from one of them satisfies the document, and
  an unsettled requirement is a note. A declared profile the region does
  not have is noted as not applied. The README no longer claims an unknown
  region falls back to `ips` (it is refused; `profiles()` lists them).

**Discovery** (ABI 12): `fhir_pods_profiles()` / wasm `profiles()` / Expo
`profiles()` / CLI `profiles` — the regions `export` speaks and every
compiled profile (`url`, `name`, `resourceType`, `kind`, `base`).
`slots()` shows per slot what the profile fixes beside or beneath it
(`fixed`) and a composite datatype slot's members from the base element
table (`parts`: a `HumanName`'s `family`, `given`…), rendered once in the
core (`CompiledProfile::slots_json`) for the C ABI and the CLI alike.
A Quantity's `unit` is optional where the profile fixes the UCUM `code`:
lowering writes the code as the unit when none is given (the profile's
own content, not a guess) and keeps a host's display unit when one is —
the unit/`mm[Hg]` footgun closed.

Proof: 228 workspace tests (12 export, 3 discovery), 18 wasm tests; release
gate and base examples as recorded in the planning report.

## 0.11.0 — 2026-09-21

**Export** (ABI 11): `fhir_pods_export(region, inputs)` (+ `_tx`, `_host_tx`,
the JNI shim); wasm `export`, Expo `exportRecord`, CLI `export`. A host
writes its transformer once, against the base R4 definitions, and the
runtime speaks the jurisdiction: for every resource the region's profile is
chosen (a declared `meta.profile` of the region's; else the sibling profile
whose fixed `code`/`category` — or a `required` binding the terminology
provider decides — the resource carries, most specific; a type with one
profile lineage takes it; a profile its guide scopes in words beyond its
structure is `specialised` and reached only by declaration), the resource is
re-profiled `lift` → `lower` through it (the jurisdiction's codings and
slices from the profile, the data from the record, `id` kept so references
survive), the region's patient summary is assembled, and the read is
recorded as a BALP `patient-read` when `audit` is given. What the region
requires and the record lacks is `missing`; content no slot takes is
`extras`; a resource nothing profiles goes out as given with a `note` —
nothing invented. Where the region's document asks its entries for a second
profile (the IPS's Condition beside US Core's) it is declared alongside and
verified (`also`, `alsoMissing`). Regions: `au` (AU Core + AU Patient
Summary — its own profiles, derived from AU Core, where the document names
them), `us` (US Core + IPS document), `ips` (the international floor). The
selection rules are written out in `docs/scope.md`.

**Base patient-record resources compiled as lowerable profiles** — `Patient`,
`Observation`, `Condition`, `AllergyIntolerance`, `MedicationStatement`,
`MedicationRequest`, `Medication`, `Immunization`, `Procedure`, `Device`,
`DeviceUseStatement`, `Practitioner`, `Organization` join `AuditEvent` and
`Provenance` (`tools/gen_all.py` `BASE_RESOURCES`): the region-neutral
vocabulary a host lowers into before `export`. Registry 979 → 992.

**Proof.** `examples/export_corpus.rs` in the release gate: one base record
(a Patient, five vital signs, a Condition, an AllergyIntolerance, a
MedicationStatement, an Immunization, a step count) exported for every
region — 39 resources, documents and audit events, each with our own
verdict — and the oracle agrees on every one: **1,560/1,560** (was 1,521).
The two IPS documents are invalid on both sides offline (the pregnancy
profiles are told apart only by LOINC sets the pod does not ship; a loaded
table resolves them — `export_with(tx)` / `loadTerminology`). 6 core
tests, 2 CLI tests; 219 workspace tests.

Carries the unreleased corrections below.

## Unreleased (now in 0.11.0)

- Reframe the README, architecture, domain model, roadmap, pod guides, and
  package descriptions around the compliance runtime and its full operation
  family. Refresh coverage and delivery evidence, document host
  responsibilities and round-trip limits, and retain the existing API names.
- CLI `slots` uses the shared resource-profile lookup and rejects extension
  profiles. The CLI package version now matches 0.10.0.
- Published R4 examples are a regression gate: pinned input inventory,
  explicit reviewed differences, strict outcome/process checks, and oracle
  caches bound to input/configuration/outcome hashes. New or changed
  differences fail, including stale exceptions after improvements. The two
  known unavailable oracle inputs remain explicitly unscored. Fresh Java
  definition-validation changes were reviewed individually; totals remain
  4,811 agreements, 479 lenient differences and two exclusions.
- Extension-only primitives receive their computable profile constraints:
  conversion without a value yields an empty result, and an empty profile
  constraint fails. The AU Core heart-rate date-carrier case now reports
  `vs-1`. Date-bearing and effectivePeriod controls pass. Compatibility guards
  preserve the pinned oracle's optional-fullUrl and non-dateTime behavior.
- Extension contexts evaluate FHIRPath node selection from the containing
  resource and check the actual owner. Nested contexts check the named parent
  URL, including its primitive value; unknown expressions report unsupported
  coverage. Choice navigation preserves primitive carriers. The R5-only
  `ExtendedContactDetail` alternative cannot select an R4 resource; Java 6.10.4
  crashes on that negative witness, so no oracle verdict is claimed for it.

Validation: 211 workspace tests, 18 Python harness guards, formatting,
all-target clippy and no_std pass; release gate 1,521/1,521. Seventeen focused
cases agree with fresh Java; one Java execution failure remains unscored.
All 70 lowering checks pass, with 68 unchanged emitted resources agreeing
with their stored Java outcomes. Both T043 lanes retain their reviewed
baselines. Released in 0.11.0.

## 0.10.0 — 2026-09-18

**The reference validator's test cases are a reproducible lane.**
`conformance/testcases.py` pins the upstream suite and compares original R4
JSON inputs with stored Java and Firely outcomes. Inline Java outcomes are
supported, missing/invalid evidence fails, and scope exclusions come from
manifest modules/options. Reviewed differences are tied to fixture/outcome
hashes and verdicts; a new or changed difference fails the run.

Base cases: **57/84** agree with stored Java; **14/20** with stored Firely.
All 25 lenient cases have named reasons. Two cases differ from stored Java
in the stricter direction, but validator_cli 6.10.4 rejects both; the PHD case
is agreement for different diagnostic reasons. There are zero unresolved
stricter cases and zero cases stricter than both stored engines. Of the 492
R4 JSON entries, 407 have manifest-declared scope exclusions and one names a
missing upstream input. The 184 R4 XML instances are not run.

**Ad-hoc profiles exercise the compiler without changing the shipped registry.**
An opt-in `testcase-profiles` Cargo feature adds one isolated test context.
Pinned Java snapshot preparation and generated Rust stay in ignored caches;
different definitions with the same canonical URL are isolated. Of 53 profile
cases, **10** compare: **9/10** agree with Java, **7/7** with Firely where
present. The difference is a Java terminology-display check. Thirty-one cases
name compiler gaps, two fail Java snapshot preparation, and ten are outside
manifest-declared scope. Two compared MIME-type cases retain unresolved
terminology warnings. These exclusions/warnings are not conformance proof.
Normal builds and release artifacts contain no test profiles.

**Four verifier corrections:** versioned document references select the
matching `meta.versionId`; non-HTTP Bundle identities are not treated as REST
resource paths; primitive-array null placeholders require paired content at
the same index; Resource.id cannot carry extensions. The profile generator
also accepts StructureDefinitions without their optional resource id, using
the canonical to name their Rust module.

Validation: **204 Rust workspace tests**, **10 harness guards**, formatting,
all-target clippy and the bare-metal no_std build pass. Release gate
**1521/1521**; published base examples **4811/5292**, with zero stricter
verdicts, 479 documented leniencies and two oracle failures excluded from
agreement. See `conformance/README.md` and `docs/scope.md` for commands,
reviewed differences and limitations. ABI remains 10; CLI version is unchanged.

## 0.9.2 — 2026-09-17

**What a fresh review of 0.9.1 found, fixed** (planning F062–F073). Six
places where the pod rejected what the reference validator accepts, none
reached by the corpora:

- **Code systems.** An absent `CodeSystem.caseSensitive` means *any case*
  (the spec's Postel rule and the reference validator's default), not
  case-sensitive as 0.9.1 had it — 424 R4-core systems, `y`/`n` in
  `v2-0136` among them. The bundled terminology package is now **THO
  7.3.0** (0.9.1 pinned 6.2.0 and called it what the validator loads; the
  gate's IGs depend on 7.x and the validator resolves the latest, so 41
  codes it accepts were unknown here — `observation-category` `symptom`).
  A system's `url|version` is keyed bare (six R4 copies were dead rows); a
  case-insensitive system's table is lowercased at generation; a code
  carrying a line break is unknown outright (it matched across two
  entries); `example` content leaves a system open, and the docs say so.
  1,212 systems, 24,724 codes.
- **Extension placement** is matched the three ways the reference
  validator matches it: the literal path from the resource with content
  references uncollapsed (`ValueSet.expansion.contains.designation`,
  `StructureDefinition.snapshot.element.binding.valueSet` — 0.9.1 cut the
  path at any dotted context, so three extensions were unusable anywhere
  in R4), the collapsed backbone path, and the path from the enclosing
  datatype's definition (`Dosage.site`, `ElementDefinition.code` — an
  off-by-one had the owner counting as its own enclosure). A context spelt
  the R5 way (`Parameters.parameter.value`) matches `value[x]`; contexts
  the package marks for FHIR ≥ 4.3 are left out. `lower`, `lift` and
  `slots` refuse an extension's profile (`resource_profile`): it is
  compiled to check placement and content, not to build an `Extension` as
  if it were a resource.
- **Resource `id`** (`[A-Za-z0-9\-\.]{1,64}`) and **`code`** (no leading,
  trailing or doubled whitespace) lexical rules — two of the eleven
  base-lane files the docs had not accounted for; the other seven small
  rules are now named in `docs/scope.md`, and the base-lane claim is
  restated as what it is: parity on the rules an unprofiled resource gets
  from us, not "invariants and bindings" at large.

**HL7 v2.** `gen_v2.py` now reads the R4 types and **skips a row whose
target names no element of the type it lands on** — 68 rows: the IG's
typos (`extension[2].uri`, `coding.value`, `valueSampedData`), R5
elements, a bare choice stem — where 0.9.1 wrote the keys and produced
non-FHIR (`Patient.address.extension[{"uri":"H"}]` for every address type;
the README's "compiled as skipped" was not true of them). An extension
block's placement is judged on the element it sits on, resolved through
the same tables (`alternate-codes` at an Appointment root and
`annotationType` on an Appointment extension are refused, 17 rows now).
A value key the map misspells against the extension's definition
(`valueCode` for the string `iso21090-ADXP-censusTract`) is respelt to
the definition's type. After conversion, an extension left with only its
url (its value row skipped) is dropped and counted in `skipped`, and a
repeating primitive's `_name` carrier is padded to its values' length.

Base lane: **4,811 of 5,292**, none stricter. Gate **1521/1521**. Tests
200. wasm ~2.3 MiB (~570 KiB gzipped).

## 0.9.1 — 2026-09-17

**Broader validation, three ways.** (1) **AU Base 6.0.0** is a primary IG:
its 123 published examples join the release gate and every AU Base
profile is compiled. (2) **Every HL7 extension** is compiled — the 701 of
`hl7.fhir.uv.extensions.r4` 5.3.0 — so where an extension may sit is
checked for all of them, not only the ones the IGs' slices reach (the
registry goes from 259 to 979 profiles). The extensions package is written
across versions and names R5's abstract parents: a `CanonicalResource`
context is met by R4's canonical resources; `regex` is allowed on
`ElementDefinition.type` as well as `ElementDefinition`, as the reference
validator allows it (FHIR-13328). (3) **The base code systems' content** is
bundled — every `complete` CodeSystem of R4 core and of the HL7 terminology
package (1,148 systems, 20,985 codes; no displays; SNOMED and LOINC never)
— and a Coding, or a Quantity's coded unit, whose system is one of them is
held to its codes ("Unknown code … in the CodeSystem …", an error). The
two packages carry different versions of many systems and a validator may
have either loaded, so a code is unknown only when *no* version we carry
has it; a system any copy leaves open is not bundled. A code under
`urn:ietf:rfc:3986` must be a URI.

**Found by the new checks, in our own house.** The v2 IG maps CX.3 (the
check-digit scheme) onto `namingsystem-checkDigit`, an extension defined
for `NamingSystem` only; the converter had been placing it on identifiers,
which no validator accepts. `gen_v2.py` now skips an extension block whose
definition cannot sit where the map puts it, with the reason (14 rows,
all this one). Two synthetic fixtures carried a contact-role code that is
not in table 0131 (`NK`; the code is `N`) and an IN1 one pipe short, which
put a date of birth in the insurance relationship; both corrected.

Base lane (`conformance/base.py`): **4,809 of 5,292** agree (was 4,796),
none stricter than the oracle; the "extension contexts we do not compile"
family is gone. The wasm grows to ~2.3 MiB (~560 KiB gzipped) for the 720
extension profiles and the code systems.

Tests: 195 (workspace). Oracle (release gate): **1521/1521** — the seven
IGs' examples, probes, and the v2 conversions.

## 0.9.0 — 2026-09-17

**Broader validation: the base spec's own examples.** A new lane,
`conformance/base.py`, puts every published example of FHIR R4 itself
(`hl7.fhir.r4.examples` 4.0.1: 5,292 instances across every resource type,
almost all unprofiled) before us and the reference validator and compares —
the widest test of the base spec's shape rules, datatype rules, invariants
and bindings. It runs apart from the release gate (parity, not cleanliness:
the spec's own examples are not all valid) and its evidence goes to a
gitignored `conformance/corpus-base/`. At this release: **4,796 of
5,292 agree**; every remaining difference is a rule we do not check and say
so (definition resolution and the conformance resources' own rules,
narrative hyperlinks, codes against code-system content, contexts of
extensions we do not compile — see `docs/scope.md`), none a wrong verdict
on a rule we do. The validator itself gives no verdict on two ValueSets.

**Verifier**, taken from the lane: a canonical must be absolute unless it is
a fragment reference; a narrative must say something (txt-2: text beyond
whitespace, tags, comments and `&nbsp;`); `base64Binary` is the alphabet
with padding at the end only, line breaks tolerated; a `urn:uuid:` is a
lowercase UUID and a `urn:oid:` an OID, wherever a uri stands; a
transaction-response's or batch-response's entries need no `fullUrl`
(0.8.3's rule was one step too wide).

Tests: 177. Oracle (release gate): **1206/1206**.

## 0.8.6 — 2026-09-17

**HL7 v2: what the IG says nothing about is named too** (F050). The
conversion result carries `unmapped`: every valued field of a placed
segment that no map row names at all (`PID[1]-2`), beside `skipped` (rows
the maps state and the pod could not do) and `unplaced` (segments the
structure has no place for) — so a host can audit data loss end to end.
Across the C ABI, wasm and Expo pods (an added JSON field; no ABI change).
A malformed segment name (`Z"Z`) no longer refuses the whole message: the
segment is parsed by what stands before the first field separator and
reported unplaced, as a receiver would ignore it; only a line with no name
at all is a parse error.

Tests: 175. Oracle: **1206/1206**.

## 0.8.5 — 2026-09-17

**Bundles validate in linear time** (F057, seen by two reviews). Two
quadratic steps, both per entry: the FHIRPath `isDistinct()` / `distinct()`
behind `bdl-7` compared every fullUrl with every other — a collection of
plain strings is now sorted once (same answers; mixed collections keep the
pairwise path); and every nested resource's issues were checked for
duplicates against the whole report — a duplicate can only come from the
resource's own profiles, so the check is local now. A collection Bundle of
8,000 Observations: ~4.5 s → 0.67 s (release, one 2026 Apple M-series
laptop; 1k 0.11 s, 4k 0.34 s). No stated ceiling: memory is the input plus
the report.

Tests: 175. Oracle: **1206/1206**.

## 0.8.4 — 2026-09-17

**Verifier: extension contexts on recursive backbones** (F051, found by
the review agent's unprofiled lane). The owner's definition path is built
from the nearest backbone context, whose content-reference cycles the type
table already collapses (`Questionnaire.item` for an item three deep), so
`ordinalValue` on a nested item's `answerOption` — and any context-declared
extension on a recursive backbone (Questionnaire and QuestionnaireResponse
items, Composition sections, Consent provisions) — is allowed where its
definition says. US Core's PRAPARE Questionnaire agrees with the oracle
again. Docs: the wasm size and timing claims in `docs/architecture.md`
and `pods/wasm/README.md` dated to v0.8.3 (F057).

Tests: 174. Oracle: **1206/1206**.

## 0.8.3 — 2026-09-17

**The reviewer's F032–F035 taken.**

- **De-identification** (F032): a backbone's own required children count
  (`Observation.component.code`, `QuestionnaireResponse.item.linkId`): a
  Remove that would empty one becomes a Mask, as it always did at the
  resource root — the copy stays conformant.
- **Verifier** (F033): a Bundle entry that carries a resource names it with
  a `fullUrl` — an error without one ("Bundle entry missing fullUrl", as
  the reference validator has it), except a transaction's or batch's create
  (`request.method` POST), whose resource has no identity yet.
- **Lifting** (F034, F035): a slot behind an optional gate nothing opened
  (`agent:user.who` when no user was given) is not missing; a labelled slot
  whose slice the profile does not compile is met by the element itself
  (`subtype:anyRead` by `subtype`); a fixed leaf behind an optional gate
  that the record carries and no slot input opens (an audit record's
  `action`) is lifted as itself, so `lower(lift(r))` gives `r` back for
  every BALP kind, with nothing missing and nothing extra.

Tests: 174. Oracle: **1206/1206** (every lowered copy still clean: 350 of 351, the one dom-3 case as before).

## 0.8.2 — 2026-09-17

**Verifier: extension contexts** (T041, found by the v2 bundles in front of
the oracle in 0.8.0). Every compiled extension profile now carries its
definition's `context` (`CompiledProfile::contexts`), and the shape walk
holds an extension to it wherever it sits: the owner answers by its type,
its path from the resource (`Patient.birthDate`, `Patient.deceased[x]`),
its path from the datatype (`ContactPoint.value`) and the base types
(`Element`, `Resource`, `DomainResource`, `BackboneElement`, `*`); a
`fhirpath` context is not evaluated (allowed), an `extension` context is
met by sitting inside an extension. Out of place is an error with the
reference validator's wording. The walk now keeps the (context, key) of
every object on the way down, so a nested resource's extensions are judged
from that resource, and a primitive's carrier (`_birthDate`) from the
primitive. Tests: 171. Oracle: **1206/1206** (unchanged: every published example sits its extensions where they belong).

## 0.8.1 — 2026-09-17

**HL7 v2, reviewed.** Three independent reviews of 0.8.0 (the converter,
the generator against the ConceptMap JSON, the ABI and docs) filed
F036–F050 in the planning repo; this release takes them. The row
compilation was faithful; the engine around it was not.

- **The structure walk is a tree** (F036, F037): the message structure as
  segments and groups with their cardinalities — the groups' from the
  standard's abstract message syntax, compiled in (`Group`,
  `MessageMap::groups`, `GROUP_CARDINALITY` in `tools/gen_v2.py`) —
  matched greedily: the next expected segment, another occurrence of the
  last, or the start of a group that may still occur; what matches nothing
  closes the group and is tried a level up; what matches nowhere is
  `unplaced`. Two orders are two reports with their own results; two
  patients keep their own; an order's NTE after OBR (a segment the IG has
  no row for; `STRUCTURE_SUPPLEMENT`) is consumed where it belongs and
  reported, never an Observation. `v2::convert::structure_tree(name)`
  renders what the walk expects.
- **A condition naming another segment reads that segment** (F039, F043):
  `IF OBR-2 NOT VALUED` on an ORC row finds the OBR of the same order (the
  nearest in the structure: this occurrence, an enclosing one, or a
  once-only group beside it); `IF ORC VALUED` is the segment's presence; a
  condition on the field's own components (`CWE.3` of SCH-7) sees them.
  `ORC.1` for a segment's field is read as `ORC-1` (a stated correction).
- **OBX-2 types the value** (F045): `varies` is the type OBX-2 names, `TS`
  is DTM before v2.6 (conditions naming DTM admit it); a repeating OBX-5
  goes through the component map the IG's own comment names (F040), one
  component per repetition, the code on each.
- **Placement** (F041, F042): an extension on a primitive lives in its
  `_name` carrier (the birth time keeps the birth date); a value already
  in place is never overwritten by a container (reported instead); an
  unmarked repeating container is the map's first item, so an XCN is one
  name.
- **Ids come from the message** (F044): every entry's `urn:uuid` and the
  Bundle id are seeded from a hash of the message bytes — different
  messages, different ids; the same message, the same bundle.
- **Options are validated** (F046): `timezone` is `Z` or `±HH:MM` within
  ±14; a coding-system name and uri are non-empty. A trigger event without
  MSH-9.3 finds its structure through HL7 table 0354 (`ADT^A04` converts
  with the ADT_A01 map).
- **No panics from message bytes** (F038): a multibyte character in an
  `\X…\` escape or a zone, and multibyte encoding characters in MSH-2.
- **Rows the pod cannot judge are skipped, and say so** (F049): a
  narrative that reads as a condition ("If …") beside a computable one
  gates the row; a remark does not. Concatenations naming a datatype the
  row does not carry are refused. The condition grammar takes no-space
  lists, `IF` inside parentheses, shared predicates over any number of
  references (`AND` or `OR`), `X NOT "v"` and single-quoted strings: 15
  conditions remain outside it, down from 28.
- **Smaller** (F047, F048, F050): a bare `code` target takes a table pair
  from the table's own target system only (XTN.3 `CP` no longer yields
  `ContactPoint.system: mobile`); a composite source (PL, XCN) for a
  primitive target goes through a datatype map or is reported, never
  copied as text; a resource only the map's literals reached (an `Account`
  from a PID) is left out and reported; `ISO3166_2` is
  `urn:iso:std:iso:3166:-2`, and `ISO` / `HL7` are no coding systems;
  identical skips are reported once; the fuzz target exercises the options.

Generator: 2055 rows, 1500 executed (21 concatenations, 5 extension blocks
moved, 17 OBX rows bound to the component map), 555 skipped with a reason
(309 target forms, 136 narrative conditions, 56 prose assignments, 18
ambiguous segment maps, 15 outside the grammar, 10 with no segment map, 7
source forms, 2 literals for a reference, one concatenation naming a
datatype its row does not carry, one order-note row the IG lacks). Tests: 170 (13 for v2: two
orders, two patients, typed and repeating values, whole names and birth
times, ids, cross-segment conditions, hostile bytes, trigger events).
Oracle: **1206/1206** (1201 + five v2 bundles: three valid on both
sides, two invalid on both sides for the same reasons). `.wasm`: 1.48 MiB (362 KiB gzipped).

Known, not yet taken: extension contexts in the verifier (T041); a valued
field no map row names is silent (F050); DG1 stays a narrative condition;
datatype maps yielding several resources (`PL`).

## 0.8.0 — 2026-09-17

**HL7 v2.** ABI 10: `fhir_pods_v2(message, options)` (+ `_tx`); wasm
`v2(message, options?)`, Expo `v2(message, options?)`; core `Validator::v2`
/ `v2::{Message, convert::{Options, Converted, Skip}}`.

- An ER7 message → a FHIR message `Bundle` through the published
  `hl7.fhir.uv.v2mappings` 1.0.0 maps, compiled into the pod by
  `tools/gen_v2.py`: 105 datatype maps, 75 segment maps, 13 message
  structures (ADT_A01 / A02 / A05 / A06 / A09 / A11 / A17, MDM_T02, OML_O21,
  ORM_O01, ORU_R01, SIU_S12, VXU_V04) and 70 vocabulary tables — 2054 rows,
  1517 executed (22 of them concatenation formulas), 537 skipped with their
  reason (309 target forms the engine does not take, 106 narrative-only
  conditions, 56 prose assignments, 28 conditions outside the grammar, 19
  ambiguous segment maps, 10 with no segment map, 7 source forms, 2
  literals assigned to a reference). A skipped row is applied never and
  *reported* whenever the message carries data for it (`skipped`: where in
  the message, which target, why); a segment the structure has no place for
  is `unplaced`. The Bundle is validated on the way out: what FHIR requires
  and the message does not carry (an ADT's Provenance with no agent, a
  Coverage with no status) is named in `validation`, never invented.
- The parser: separators from MSH-1/2, repetitions, components,
  subcomponents, the standard escapes (`\F\ \S\ \T\ \R\ \E\ \Xdd\ \.br\`).
  MLLP framing is the transport's.
- The converter's rules beyond the maps — structural, and all stated in
  `v2/convert.rs`: a numbered instance (`DiagnosticReport[1]`) is shared
  along the structure (ORC in COMMON_ORDER and OBR in ORDER_OBSERVATION fill
  one report); a resource with a `subject` / `patient` / `beneficiary` and
  none set refers to the nearest Patient in the structure, `encounter`
  likewise; a DiagnosticReport's `result` is the observations of its order
  group; the Bundle is a `message` with the MessageHeader first and
  deterministic `urn:uuid` entries (the same message gives the same
  bundle); an identifier under `urn:ietf:rfc:3986` whose value is a bare
  OID or UUID is written as the URN the IG spells for the same value
  elsewhere; a coding-system name (`LN`, `SCT`, `UCUM`, `HL7nnnn`) resolves
  to its uri from the standard names of table 0396 and the host's
  `codeSystems`, else the row is reported; a time stated without a zone is
  truncated to its date and reported unless the host gives `timezone`; a
  list item a failed reference leaves with only the map's literals
  (`location[1].status` without its location) is dropped and reported.
- The generator reads the maps as published and corrects nothing but
  spelling: a `value[x]` key in the maps' case (`valuedateTime`), a
  datatype's own name prefixed to its targets (`Quantity.code`), an
  extension block placed a level above the element its definition allows
  (`contactpoint-local` belongs on `ContactPoint.value`: `_value.extension`,
  from the extensions package's contexts), and the one narrative that
  instructs rather than conditions ("If mapped to FHIR, use Code System and
  Display"). Two IG rows that assign a literal to a reference are refused.
- Proof: `examples/v2_corpus.rs` converts four synthetic messages (ORU^R01
  with two orders, ADT^A01, VXU^V04, OML^O21) and records each bundle's own
  verdict for the oracle; fuzz target `v2`; the Node, C and Expo smokes.

**Verifier**, found by the v2 bundles in front of the oracle: **an
identifier under `urn:ietf:rfc:3986` must carry a URI** (a scheme) — an
error, as the reference validator has it. Not yet checked, and now known
from the same run: where an extension may sit (its definition's context).

Oracle: **1205/1205** (1201 + the four v2 bundles: two valid on both sides,
two invalid on both sides, for the same reasons). Tests: 162. `.wasm`:
1.47 MB (358 KB gzipped) — the maps are ~300 KB of it.

## 0.7.0 — 2026-09-16

**Lifting.** ABI 9: `fhir_pods_lift(profile, resource)` (+ `_tx`); wasm
`lift(profileUrl, resource)`, Expo `lift(profileUrl, resource)`; core
`Validator::lift` / `lift::{lift, Lifted, LiftError}`.

- The reverse of lowering: a resource read back into the profile's slots
  — the same vocabulary `lower` takes, so `lower(lift(r))` gives `r` back
  — with `missing` (required slots the resource lacks) and `extras` (every
  element path no slot, fixed leaf or pass-through member covers), so a
  host's map from FHIR to its own model is a map from slot paths to fields
  and nothing in a record is left behind unnamed. An empty URL takes the
  most specific compiled profile the resource declares; a profile the
  resource does not declare is still applied and its verdict returned.
- The extractor the lowering proof used since 0.3.0 is this operation,
  promoted: `examples/lower_corpus.rs` now lifts every corpus example
  through the public API and lowers it again for the reference validator —
  350 of 350 lowered resources validate, as before, now through `lift`.

**Verifier**, found by the lifted copies in front of the oracle: **a local
reference (`#id`) must name a contained resource** of the resource it sits
in, or one enclosing it (`ref-1`) — an error, as the reference validator
has it; two probes.

Oracle: **1201/1201** (1199 + the two probes). Tests: 149. `.wasm`:
1.15 MB (282 KB gzipped).

## 0.6.4 — 2026-09-16

The review lane's F030 and F031 (planning repository). Oracle unchanged:
**1199/1199**.

- **A rule that names an element's `id` or a `linkId` is honoured** (F030):
  the structural default (keep) yields to an explicit path or member rule
  (`{"target": "QuestionnaireResponse.item.linkId", "action": "mask"}`);
  only the type-level `string` rule does not reach them. Before, such a
  rule was silently ignored with `applied: []`.
- **A `linkId` kept by default is not reported as residual** (F031): it is
  a key, like an element's `id`; `residual` is empty under Safe Harbor
  alone, as the field says.

F029 (the CLI drops `residual`) is the command-line pod's own file and
lane.

## 0.6.3 — 2026-09-16

**FHIRPath, against the official R4 suite** (planning F007 / T023). Of the
eleven disagreements the review lane's catalogue found, ten were the
engine's and are fixed; one is kept on purpose. Oracle unchanged:
**1199/1199** — no registry invariant turned on any of this.

- **A literal is a System value, a FHIR element never is.** `1 is Integer`,
  `true is Boolean`, `1.is(System.Integer)` hold; `Patient.active is
  System.Boolean` does not (the suite's `testType14`), so the mapping that
  made a FHIR `boolean` a `System.Boolean` is gone. Unqualified names on
  elements still match as written (`$this is DateTime` on a `dateTime`
  stays false, as before).
- **`as` / `ofType` name the exact type**: `Patient.gender.ofType(string)`
  is empty though a `code` is a `string` to `is` (which keeps the
  hierarchy). An unknown type name (`as(string1)`) is an error, never an
  empty result.
- **A bare resource-type name is the item itself when that is its type**
  — inside `iif`, `where` and the rest, not only at the invariant's root —
  so `iif(Patient.name.exists(), …)` takes the right branch.
- Kept: `as` on more than one item filters rather than errors. The suite
  wants an error; the reference validator's own `dom-3`
  (`descendants().as(canonical)`) needs the filter, and that is the engine
  we agree with. One disagreement, stated.

The catalogue's adapter asserts every string output `is System.String`,
which the corrected semantics make false for FHIR elements; ten of its
cases now read as failures for that reason alone (a note to the review
lane, whose T021 is exactly that adapter).

## 0.6.2 — 2026-09-16

The review lane's F025–F028 (planning repository), all taken. Oracle:
**1199/1199**. No ABI change (ABI 8).

- **Unprofiled Bundles owe their identity and the base Bundle rules**
  (F026): a REST-shaped `fullUrl` must end in the resource's type and id
  (a `urn:` is its own identity), and `Bundle`'s own invariants — `bdl-1`
  (`total` only on a searchset or history), `bdl-3` (a transaction entry
  has its request), `bdl-7` (one fullUrl once, unless the versions
  differ), `bdl-9`…`bdl-12` — apply when no profile does, from a generated
  table like `DomainResource`'s. Eight probes, both sides.
- **`tools/compile_valueset.py` refuses a partial expansion** (F025): a
  paged one (`offset`, `count`, or a `total` beyond what it carries) is
  compiled only when every page is given and they add up; a limited
  expansion (`limitedExpansion`, `too-costly`) is refused. A table is
  complete for the sets it lists, so completeness is checked, never
  inferred from a response parsing. The review lane's four fixtures
  behave as they should.
- **Docs say what the code does** (F027): the roadmap's lowering example
  and ABI symbol (`fhir_pods_slots`), the header's ABI comment, the scope
  note's headline and its FHIRPath list (`resolve()` and `&` are
  supported), the wasm README's inventory (259 profiles, six IGs, 1.1 MB),
  the Expo README's simulator status labelled as the v0.1.0 / ABI 3
  measurement it is, and the README's and architecture note's timing as a
  dated measurement.
- **The terminology request example encodes its parameters** (F028):
  `URLSearchParams`, a status check, and the answer cached only when the
  server gave one.

Tests: 141.

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
