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
page (`tools/gen_rules_doc.py`) and a test holds the two together.

## The input

| Id | Meaning |
| --- | --- |
| `parse.json` | the input is not JSON |
| `parse.resource-type` | no resourceType, or not a string, or not an R4 resource type |

## Declared profiles

| Id | Meaning |
| --- | --- |
| `profile.unknown` | a declared profile is not compiled here, so it was not checked |
| `profile.wrong-type` | a declared profile applies to another resource type |

## Structure — the shape walk against the base definitions

| Id | Meaning |
| --- | --- |
| `structure.unknown-element` | an element the base does not define |
| `structure.json-type` | a value of the wrong JSON type for its FHIR type |
| `structure.array` | a repeating element given as a single value, a single element given as an array, or an empty array |
| `structure.not-a-resource` | a contained or entry resource that is not a FHIR resource |
| `structure.carrier` | a primitive's `_name` carrier that is malformed, empty, or on Resource.id |
| `structure.choice` | two keys for one choice element, or a choice type the profile does not allow |
| `structure.required` | a required element is missing (base or profile cardinality) |
| `structure.max` | an element repeats beyond its maximum |
| `structure.slice` | a slice problem: an element matching more than one slice, a required slice absent, a slice not checked |
| `structure.entry-type` | a bundle entry of a resource type, or matching no profile, the entry slicing allows |

## Values

| Id | Meaning |
| --- | --- |
| `value.invalid` | a primitive that is not a valid value of its type (date, uri, code, decimal …) |
| `value.fixed` | a value differs from the one the profile fixes |
| `value.pattern` | an element does not carry the pattern the profile requires |
| `value.identifier` | an Identifier.system or .value that breaks the base rules |
| `value.extension-url` | an Extension.url that is not absolute |
| `value.language` | an invalid displayLanguage |
| `value.url-unknown` | a uri under a known definition space that names no definition |

## Bindings and codes

| Id | Meaning |
| --- | --- |
| `binding.not-member` | a code is not in the required value set |
| `binding.no-code` | no code provided where a required binding wants one |
| `binding.unknown` | membership could not be decided (no bundled terminology) |

## Code systems

| Id | Meaning |
| --- | --- |
| `code.unknown` | a code is not in its code system |

## References

| Id | Meaning |
| --- | --- |
| `reference.target-type` | a reference's declared type or resolved target is not among the element's targets |
| `reference.target-profile` | a resolved target does not conform to any profile the element allows |
| `reference.shape` | a relative reference that is not Type/id |

## narrative

| Id | Meaning |
| --- | --- |
| `narrative.parse` | the narrative is not well-formed XHTML |
| `narrative.root` | the narrative's root is not a div in the XHTML namespace |
| `narrative.element` | an element the narrative does not allow, or one out of its place (a li outside a list, a block inside a p) |
| `narrative.attribute` | an attribute the narrative does not allow on that element |
| `narrative.entity` | an entity other than XML's own |
| `narrative.link` | a hyperlink that is empty, carries an invalid character, does not resolve, or has active content |
| `narrative.external-link` | a narrative hyperlink or image source outside the resource, where the host refuses one (`externalNarrativeLinks`) |

## Extensions

| Id | Meaning |
| --- | --- |
| `extension.context` | an extension used where its definition does not allow it |
| `extension.unknown` | an extension whose definition could not be found |

## Invariants

| Id | Meaning |
| --- | --- |
| `inv.<key>` | a FHIRPath invariant failed: `inv.cmp-1`, `inv.que-7`, `inv.dom-6` … |
| `inv.other` | an invariant whose key the generated table does not list (a defect of the generator, not of the finding) |

## Bundles and documents

| Id | Meaning |
| --- | --- |
| `bundle.full-url` | an entry's fullUrl is missing or does not match its resource |
| `bundle.unreachable` | a document entry not reachable from the Composition |
| `bundle.unresolved` | a reference that is not in the document (a document is self-contained) |

## Questionnaires

| Id | Meaning |
| --- | --- |
| `questionnaire.unresolved` | the Questionnaire could not be resolved, so no validation was performed |
| `questionnaire.version-mismatch` | the response names a version of a loaded Questionnaire other than the one loaded; not validated against it |
| `questionnaire.linkid-unknown` | a response item the Questionnaire does not define |
| `questionnaire.wrong-place` | a response item defined elsewhere in the Questionnaire |
| `questionnaire.out-of-order` | response items out of the Questionnaire's order |
| `questionnaire.duplicate-item` | more than one response item for a non-repeating definition |
| `questionnaire.required-answer` | no answer for a required item or group |
| `questionnaire.disabled-answer` | an answer to an item enableWhen disables |
| `questionnaire.item-shape` | items or answers where the item type forbids them |
| `questionnaire.answer-type` | an answer of the wrong type for the item |
| `questionnaire.answer-not-in-set` | a coded or constrained answer outside the permitted values |
| `questionnaire.answer-exclusive` | an exclusive option selected with others |
| `questionnaire.answer-count` | fewer or more answers than the item allows |
| `questionnaire.answer-range` | an answer outside its minimum, maximum, length or precision |
| `questionnaire.answer-content` | an answer whose content is not allowed (new lines, format or regex, mime type, attachment size, reference type) |
| `questionnaire.answer-text` | answer text that does not match the definition |
| `questionnaire.authored` | authored outside the permitted period |
| `questionnaire.definition` | a defect in the Questionnaire definition itself (enableWhen targets, option lists, item types) |
| `questionnaire.value-set` | a value set the Questionnaire names could not be found |

## Lowering inputs

| Id | Meaning |
| --- | --- |
| `lower.input` | a lowering input that is unknown, of the wrong shape, or fixed by the profile |

## What was not checked

| Id | Meaning |
| --- | --- |
| `not-supported.fhirpath` | an invariant outside the supported FHIRPath core was not evaluated |
| `not-supported.regex` | a regex outside the supported core was not evaluated |
| `not-supported.extension-context` | an extension context outside the supported FHIRPath core |
| `not-supported.slice` | a slice that could not be checked |

## Fallback

| Id | Meaning |
| --- | --- |
| `other.<type>` | a finding no family above claims (a defect of this table) |

## Gate mode

`fhir-pods gate [--questionnaire Q.json]… [--terminology T.txt] FILE…` validates
every file, prints one JSON report — per file its verdict and error-level
findings by id, and the counts by id over the run — and exits 1 when any
file has an error. That is what runs on a vendor's deliverables and in CI.
