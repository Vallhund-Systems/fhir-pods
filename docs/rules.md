# Rule ids

Every finding the pod reports carries a stable rule id — `issues[].id` in the
pods' JSON report, `issue.details.coding` (system `urn:fhir-pods:rule`) in an
`OperationOutcome` — so a host asserts on *which* rule fired, never on
message text. The contract:

- an id never changes meaning;
- a new kind of finding gets a new id;
- an id that falls out of use stays listed here as retired;
- an invariant's id is `inv.<key>` — `inv.cmp-1`, `inv.que-7`, `inv.dom-6` —
  for every key the base definitions and the compiled profiles carry;
- `other.<type>` is the fallback for a finding no family claims — a defect
  of the table, which the core's tests hold to zero over the release
  corpus, the mutation files and the gut-path scenario.

The ids are derived from the finding's type and message family in
`crates/fhir-pods-core/src/rules.rs`; the table there is the source of this
page (`tools/gen_rules_doc.py`) and a test holds the two together — both
ways, so neither can carry an id the other does not.

## Severities

Every finding carries one of four severities — `issues[].severity` in the
pods' report, `issue.severity` in the `OperationOutcome` — and the tables
below say which a given id can arrive at:

| Severity | What it means |
| --- | --- |
| `fatal` | Something could not be checked at all, and the report says which. Two findings are fatal: the input is not JSON — there is no document, so that finding stands alone — and the Questionnaire a response is held to has an item with no `type`, so the response cannot be judged against that item (the Questionnaire's own findings stand beside it). |
| `error` | The resource does not conform. |
| `warning` | Something a host may need to act on that is not a conformance failure: a check that could not be performed (no bundled terminology, an expression outside the supported core), a declared profile this build has not compiled, an invariant the definitions grade best-practice. |
| `information` | Said for the record, never a defect: a profile invariant skipped as unsupported, no Questionnaire named at all, no option list to validate an answer against. |

**`valid` means no `error` and no `fatal`** — `Report::is_valid`,
`"valid": true` in the pods' report, exit 0 from the CLI. A `warning` or an
`information` finding never changes the verdict, however much it matters:
a resource whose declared profile was never compiled here is *valid* by
default, with `profile.unknown` naming it (the reference validator's own
severity), and a host that will not accept that asks for
`unknownProfile: "error"` rather than filtering the issue out
(`docs/scope.md`). A `fatal` finding makes `valid` false and says that
something was not checked: for `parse.json` nothing at all was (no bytes
parsed, no document, that one finding in the report — so reading its short
issue list as "almost conformant" would be wrong), and for
`questionnaire.definition` the response could not be judged against the
item whose definition is broken. Both were probed against validator_cli
6.10.4 (2026-10-14): it reports `fatal` for the same two inputs.

## The input

| Id | Severity | Meaning |
| --- | --- | --- |
| `parse.json` | fatal | the input is not JSON — the bytes could not be parsed at all, so nothing else was checked (path `(document)`) |
| `parse.resource-type` | error | no resourceType, or not a string, or not an R4 resource type |

## Declared profiles

| Id | Severity | Meaning |
| --- | --- | --- |
| `profile.unknown` | warning — error where the host sets `unknownProfile` | a declared profile is not compiled here, so it was not checked |
| `profile.wrong-type` | error | a declared profile applies to another resource type |

## Structure — the shape walk against the base definitions

| Id | Severity | Meaning |
| --- | --- | --- |
| `structure.unknown-element` | error | an element the base does not define |
| `structure.json-type` | error | a value of the wrong JSON type for its FHIR type: a boolean given as a string, a date as a number, a Reference as a primitive, an object where the type takes a value — or a primitive's `_` carrier given as an object where the element repeats |
| `structure.array` | error | a repeating element given as a single value, a single element given as an array, or an empty array |
| `structure.not-a-resource` | error | a contained or entry resource that is not a FHIR resource |
| `structure.carrier` | error | a primitive's `_name` carrier that is malformed, empty, or on Resource.id |
| `structure.choice` | error | two keys for one choice element, or a choice type the profile does not allow |
| `structure.required` | error | a required element is missing (base or profile cardinality) |
| `structure.max` | error | an element repeats beyond its maximum |
| `structure.slice` | error | a slice problem: an element matching more than one slice, a required slice absent, a slice not checked |
| `structure.entry-type` | error | a bundle entry of a resource type, or matching no profile, the entry slicing allows |

## Values

| Id | Severity | Meaning |
| --- | --- | --- |
| `value.invalid` | error | a primitive that is not a valid value of its type (date, uri, code, decimal …) |
| `value.fixed` | error | a value differs from the one the profile fixes |
| `value.pattern` | error | an element does not carry the pattern the profile requires |
| `value.identifier` | error | an Identifier.system or .value that breaks the base rules |
| `value.extension-url` | error | an Extension.url that is not absolute |
| `value.language` | error | an invalid displayLanguage |
| `value.url-unknown` | error | a uri under a known definition space that names no definition |

## Bindings and codes

| Id | Severity | Meaning |
| --- | --- | --- |
| `binding.not-member` | error | a code is not in the required value set |
| `binding.no-code` | error | no code provided where a required binding wants one |
| `binding.unknown` | warning | membership could not be decided (no bundled terminology) |

## Code systems

| Id | Severity | Meaning |
| --- | --- | --- |
| `code.unknown` | error | a code is not in its code system |

## References

| Id | Severity | Meaning |
| --- | --- | --- |
| `reference.target-type` | error | a reference's declared type or resolved target is not among the element's targets |
| `reference.target-profile` | error | a resolved target does not conform to any profile the element allows |
| `reference.shape` | error | a relative reference that is not Type/id |

## narrative

| Id | Severity | Meaning |
| --- | --- | --- |
| `narrative.parse` | error | the narrative is not well-formed XHTML |
| `narrative.root` | error | the narrative's root is not a div in the XHTML namespace |
| `narrative.element` | error | an element the narrative does not allow, or one out of its place (a li outside a list, a block inside a p) |
| `narrative.attribute` | error | an attribute the narrative does not allow on that element |
| `narrative.entity` | error | an entity other than XML's own |
| `narrative.link` | error | a hyperlink that is empty, carries an invalid character, does not resolve, or has active content |
| `narrative.external-link` | error | a narrative hyperlink or image source outside the resource, where the host refuses one (`externalNarrativeLinks`) |

## Extensions

| Id | Severity | Meaning |
| --- | --- | --- |
| `extension.context` | error | an extension used where its definition does not allow it |
| `extension.unknown` | error | an extension whose definition could not be found |

## Invariants

| Id | Severity | Meaning |
| --- | --- | --- |
| `inv.<key>` | error — warning for an invariant the definitions grade best-practice (`dom-6`) | a FHIRPath invariant failed: `inv.cmp-1`, `inv.que-7`, `inv.dom-6` … |
| `inv.other` | as the invariant's own | an invariant whose key the generated table does not list (a defect of the generator, not of the finding) |

## Bundles and documents

| Id | Severity | Meaning |
| --- | --- | --- |
| `bundle.full-url` | error | an entry's fullUrl is missing or does not match its resource |
| `bundle.unreachable` | error | a document entry not reachable from the Composition |
| `bundle.unresolved` | error | a reference that is not in the document (a document is self-contained) |

## Questionnaires

| Id | Severity | Meaning |
| --- | --- | --- |
| `questionnaire.unresolved` | information when none is named, or the canonical is an `example.org` url; warning when a named one could not be resolved | the Questionnaire could not be resolved, so no validation was performed against it: the response named none, or named one this module has not been given (a contained one and a bundle entry are found without loading) |
| `questionnaire.version-mismatch` | warning | the response names a version of a loaded Questionnaire other than the one loaded; not validated against it |
| `questionnaire.linkid-unknown` | error | a response item the Questionnaire does not define |
| `questionnaire.wrong-place` | error | a response item defined elsewhere in the Questionnaire |
| `questionnaire.out-of-order` | error | response items out of the Questionnaire's order |
| `questionnaire.duplicate-item` | error | more than one response item for a non-repeating definition |
| `questionnaire.required-answer` | error | no answer for a required item or group |
| `questionnaire.disabled-answer` | error | an answer to an item enableWhen disables |
| `questionnaire.item-shape` | error | items or answers where the item type forbids them: a `display` item with nested items or an answer, a non-group item with `item` outside its answers, a `question` item with answers it cannot hold |
| `questionnaire.answer-type` | error | an answer of the wrong type for the item |
| `questionnaire.answer-not-in-set` | error | a coded or constrained answer outside the permitted values |
| `questionnaire.answer-no-system` | warning | a coded answer with a code but no system, the code in the answer's contained value set, where that set's compose has one include or the set has a url: a code with no system has no defined meaning and cannot be validated (no code at all, a code the set does not hold, or a set with no url and two or more includes, is `questionnaire.answer-not-in-set`) |
| `questionnaire.answer-exclusive` | error | an exclusive option selected with others |
| `questionnaire.answer-count` | error | fewer or more answers than the item allows |
| `questionnaire.answer-range` | error | an answer outside its minimum, maximum, length or precision |
| `questionnaire.answer-content` | error — warning for a new line in an answer | an answer whose content is not allowed (new lines, format or regex, mime type, attachment size, reference type) |
| `questionnaire.answer-text` | error | answer text that does not match the definition |
| `questionnaire.authored` | warning | authored outside the permitted period |
| `questionnaire.definition` | fatal for an item with no type (nothing below it can be checked); error for the enableWhen and option-list defects; information where no option list is given at all | a defect in the Questionnaire the response is held to, not in the response: an item with no `type`, an `enableWhen` that targets itself, an unknown or inner linkId, an option list with no option of the answer's kind, or no option list to validate against |
| `questionnaire.value-set` | warning | a value set the Questionnaire names could not be found |

## Lowering inputs

| Id | Severity | Meaning |
| --- | --- | --- |
| `lower.input` | error | a lowering input that is unknown, of the wrong shape, or fixed by the profile |

## What was not checked

| Id | Severity | Meaning |
| --- | --- | --- |
| `not-supported.fhirpath` | information for a profile invariant, warning for a datatype one | an invariant outside the supported FHIRPath core was not evaluated |
| `not-supported.regex` | warning | a regex outside the supported core was not evaluated |
| `not-supported.extension-context` | warning | an extension context outside the supported FHIRPath core |
| `not-supported.slice` | information or warning | a slice that could not be checked |

## Fallback

| Id | Severity | Meaning |
| --- | --- | --- |
| `other.<type>` | as the finding's own | a finding no family above claims (a defect of this table) |

## Gate mode

`fhir-pods gate [--questionnaire Q.json]… [--terminology T.txt] FILE…` validates
every file, prints one JSON report — per file its verdict and error-level
findings by id, and the counts by id over the run — and exits 1 when any
file has an error. That is what runs on a vendor's deliverables and in CI.
