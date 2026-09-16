# Scope — what "agrees with the reference validator" means

`conformance/run.py` reports **288/288 agree** across AU Core 2.0.0, US Core
6.1.0 and IPA 1.1.0. This note says precisely what that claim covers and what it deliberately
does not, so nobody reads more into it than it says.

## The claim

For every fixture in the corpus (each IG's own published examples, plus broken
variants), our verdict equals the HL7 reference validator's
(`validator_cli.jar`, run with `-tx n/a` and `-allow-example-urls true`):

- **Verdict** — *valid* (no `error`/`fatal` issue) or *invalid*, and
- **Where** — the set of element paths carrying `error`-level issues, after
  normalising the two locator styles (ours: `Observation.component:diastolic`;
  theirs: FHIRPath with indices). A difference here is printed as
  `issue paths differ (info)` — one remains (below).

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
| **Attachment fetching** | 1 | "Attachment content cannot be checked … error accessing <url>" | Never: a library does no I/O. |
| **must-support** | 0 | Validators do not check must-support | Not an error by spec. Exposed separately: `must_support_gaps(resource)` (and `probe --must-support`) lists the selected profile's must-support elements the resource leaves empty where their parent is present — what a client certifying against the IG must populate when it has the data. Never in the validation report. |

## What we check that the reference validator also checks

Per-parent cardinality, fixed/pattern values (scalars; object patterns via
slice discriminators), `required` bindings on bundled sets, slicing (value /
pattern / type discriminators, closed slicing, nested-slice discriminator
resolution, slice → profile inheritance for `Identifier` and complex
`Extension` types), and FHIRPath invariants through tier two. Every
error-level finding either side raised on the corpus is in this set — that is
the 271/271.

## What we skip, never guess

- FHIRPath outside the supported core: `as(type)` / `ofType(type)` as
  functions on arbitrary nodes (JSON carries no element types; only `$this`'s
  type is known, from the profile), `memberOf`, string concatenation `&`,
  equivalence `~`, decimals. Every one of the registry's distinct invariant
  expressions now parses; `dom-3` (contained-reference integrity) is the one
  that still evaluates `Unsupported`, because it needs `as(canonical)` over
  `descendants()`. Skipped. Type tests (`$this is dateTime`, `$this as
  dateTime`) resolve against the element's type *from the profile* — never
  from the value's shape — and are unsupported on anything but `$this`.
- `matches(regex)` runs on a small linear-time engine (literals, classes,
  anchors, groups, alternation, bounded quantifiers). Backreferences,
  lookaround, flags, `\b` and unicode properties are refused →
  `Unsupported` → skipped.
- Slices whose discriminator cannot be resolved from the profile (e.g.
  AU Core's `identifier:dva`, whose `type` pattern lives nowhere). Deferred.
- Bindings to value sets we do not bundle. Warned, not failed.

A skipped check can only make us *more lenient* than the reference validator,
never stricter: the corpus proves no lenience shows up on real examples, but a
constraint we skip is a constraint we skip.

## Reproducing

```bash
JAVA_HOME=/opt/homebrew/opt/openjdk@17 python3 conformance/run.py
```

The oracle runs once per corpus in one JVM; its full output (all severities)
lands in `conformance/corpus/_oracle.bundle.json` for inspection.
