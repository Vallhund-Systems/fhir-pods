# Scope — what "agrees with the reference validator" means

`conformance/run.py` reports **1191/1191 agree** at v0.6.1 across the six
pinned IGs (AU Core 2.0.0, US Core 6.1.0, IPA 1.1.0, IPS 2.0.1, AU PS 1.0.0,
IHE BALP 1.1.4): every IG's published examples, the probes, every example
re-lowered from its slot values, every one de-identified, the documents
assembled again, and an audit record of every kind. This note says precisely
what that claim covers and what it deliberately does not, so nobody reads
more into it than it says. The count moves with every release and is in the
changelog.

## The claim

For every fixture in the corpus (each IG's own published examples, plus broken
variants), our verdict equals the HL7 reference validator's
(`validator_cli.jar`, run with `-tx n/a` and `-allow-example-urls true`):

- **Verdict** — *valid* (no `error`/`fatal` issue) or *invalid*, and
- **Where** — the set of element paths carrying `error`-level issues, after
  normalising the two locator styles (ours: `Observation.component:diastolic`;
  theirs: FHIRPath with indices). A difference here is printed as
  `issue paths differ (info)` — two remain: the display-name case below, and
  the orphan-contained probe, where the oracle also flags the dangling
  `Medication/elsewhere` reference we do not resolve.

Warnings and informational issues are **not** scored. Both sides emit them;
they differ in volume and kind, and the differences are all in the areas below.

## What the reference validator checks that we do not (yet)

From the 868 non-error issues it raised over the corpus (506 warnings, 307
informational), by family:

| Family | Their count | What it is | Our stance |
|---|---|---|---|
| **Terminology without a server** | ~700 | "Unable to validate code … running without terminology services", "ValueSet … not found", "Could not confirm … extensible value set", "CodeSystem … doesn't provide any codes" | We bundle a small set of base-FHIR value sets (gender, status codes, vitals UCUM, …) and check `required` bindings against those; everything else is a `warning: no bundled terminology for <vs>`. The `TerminologyProvider` seam is where a host plugs NCTS / VSAC / a local cache in. SNOMED is never bundled (licence). |
| **Display-name checking** | 1 error-level in a probe | "Wrong Display Name 'Laboratory' for …#vital-signs" | Not done — needs code-system content we don't ship (and for SNOMED, cannot). The one surviving `issue paths differ` is this. |
| **Best-practice recommendations** | 82 + 12 | "all observations should have a performer" (`obs-…` best practice), `dom-6` narrative | `dom-6` we evaluate as a warning (it is an ordinary invariant). The performer recommendation is validator policy, not a profile constraint; not emitted. |
| **Multiple matching profiles** | 32 | The validator had to pick among candidate profiles (its own warning) | A resource that declares several profiles is validated against **every** one we have compiled (findings shared by two profiles are reported once; the report carries `profiles`). The oracle's warning here is about its own choice among *implicit* candidates, not declared ones. |
| **Deprecation / advisories** | 5 + 2 | Deprecated extensions and concepts, "UCUM annotations like {T-Score} can be misleading" | Advisory, not conformance; not emitted. |
| **Reference target types** | (none in corpus) | `subject` must reference a Patient | The reference validator does **not** enforce this offline either (`heartrate-subject-wrong-type` expects *valid* as a guard). Not enforced. |
| **Default profiles** | — | An undeclared profile means the base resource | Same: no profile claimed; the base resource's own rules (shape, its required elements, one key per choice, `dom-3` / `dom-6`) still apply. A jurisdiction default is opt-in (`Validator::with_default_profile`). |
| **Attachment fetching** | 1 | "Attachment content cannot be checked … error accessing <url>" | Never: a library does no I/O. |
| **must-support** | 0 | Validators do not check must-support | Not an error by spec. Exposed separately: `must_support_gaps(resource)` (and `probe --must-support`) lists the selected profile's must-support elements the resource leaves empty where their parent is present — what a client certifying against the IG must populate when it has the data. Never in the validation report. |

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
  the reference validator has it. A `fhirpath` context is not evaluated
  (allowed); an `extension` context is met by sitting inside an extension.
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
