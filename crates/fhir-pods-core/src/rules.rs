//! Stable rule ids: one machine-readable name per kind of finding, frozen
//! across releases, so a host asserts on *which* rule fired rather than on
//! message text. An id never changes meaning; a new kind of finding gets a
//! new id; an id that falls out of use stays listed as retired.
//!
//! The id is derived where an issue is recorded, from its type and the
//! family its message belongs to — the messages are themselves stable (most
//! mirror the reference validator's wording), and the mapping below is the
//! contract. An invariant's id is `inv.<key>` (`inv.cmp-1`, `inv.que-7`).
//! A finding no family claims is `other.<type>` — a defect of this table,
//! which the core's tests hold to zero over the release corpus, the
//! mutation files and the gut-path scenario.

use crate::validate::IssueType;

/// Every id, with what it means. Kept in id order within each family.
pub static RULES: &[(&str, &str)] = &[
    // --- the resource itself ---
    ("parse.json", "the input is not JSON"),
    ("parse.resource-type", "no resourceType, or not a string, or not an R4 resource type"),
    ("profile.unknown", "a declared profile is not compiled here, so it was not checked"),
    ("profile.wrong-type", "a declared profile applies to another resource type"),
    // --- structure: the shape walk against the base definitions ---
    ("structure.unknown-element", "an element the base does not define"),
    ("structure.json-type", "a value of the wrong JSON type for its FHIR type"),
    ("structure.array", "a repeating element given as a single value, a single element given as an array, or an empty array"),
    ("structure.not-a-resource", "a contained or entry resource that is not a FHIR resource"),
    ("structure.carrier", "a primitive's `_name` carrier that is malformed, empty, or on Resource.id"),
    ("structure.choice", "two keys for one choice element, or a choice type the profile does not allow"),
    ("structure.required", "a required element is missing (base or profile cardinality)"),
    ("structure.max", "an element repeats beyond its maximum"),
    ("structure.slice", "a slice problem: an element matching more than one slice, a required slice absent, a slice not checked"),
    ("structure.entry-type", "a bundle entry of a resource type, or matching no profile, the entry slicing allows"),
    // --- values ---
    ("value.invalid", "a primitive that is not a valid value of its type (date, uri, code, decimal …)"),
    ("value.fixed", "a value differs from the one the profile fixes"),
    ("value.pattern", "an element does not carry the pattern the profile requires"),
    ("value.identifier", "an Identifier.system or .value that breaks the base rules"),
    ("value.extension-url", "an Extension.url that is not absolute"),
    ("value.language", "an invalid displayLanguage"),
    ("value.url-unknown", "a uri under a known definition space that names no definition"),
    // --- bindings and codes ---
    ("binding.not-member", "a code is not in the required value set"),
    ("binding.no-code", "no code provided where a required binding wants one"),
    ("binding.unknown", "membership could not be decided (no bundled terminology)"),
    ("code.unknown", "a code is not in its code system"),
    // --- references ---
    ("reference.target-type", "a reference's declared type or resolved target is not among the element's targets"),
    ("reference.target-profile", "a resolved target does not conform to any profile the element allows"),
    ("reference.shape", "a relative reference that is not Type/id"),
    // --- narratives (txt-1 is `inv.txt-1`; the parser's findings beside it) ---
    ("narrative.parse", "the narrative is not well-formed XHTML"),
    ("narrative.root", "the narrative's root is not a div in the XHTML namespace"),
    ("narrative.element", "an element the narrative does not allow, or one out of its place (a li outside a list, a block inside a p)"),
    ("narrative.attribute", "an attribute the narrative does not allow on that element"),
    ("narrative.entity", "an entity other than XML's own"),
    ("narrative.link", "a hyperlink that is empty, carries an invalid character, does not resolve, or has active content"),
    // --- extensions ---
    ("extension.context", "an extension used where its definition does not allow it"),
    ("extension.unknown", "an extension whose definition could not be found"),
    // --- invariants (the key is the id's tail) ---
    ("inv.<key>", "a FHIRPath invariant failed: `inv.cmp-1`, `inv.que-7`, `inv.dom-6` …"),
    ("inv.other", "an invariant whose key the generated table does not list (a defect of the generator, not of the finding)"),
    // --- bundles and documents ---
    ("bundle.full-url", "an entry's fullUrl is missing or does not match its resource"),
    ("bundle.unreachable", "a document entry not reachable from the Composition"),
    ("bundle.unresolved", "a reference that is not in the document (a document is self-contained)"),
    // --- questionnaires ---
    ("questionnaire.unresolved", "the Questionnaire could not be resolved, so no validation was performed"),
    ("questionnaire.version-mismatch", "the response names a version of a loaded Questionnaire other than the one loaded; not validated against it"),
    ("questionnaire.linkid-unknown", "a response item the Questionnaire does not define"),
    ("questionnaire.wrong-place", "a response item defined elsewhere in the Questionnaire"),
    ("questionnaire.out-of-order", "response items out of the Questionnaire's order"),
    ("questionnaire.duplicate-item", "more than one response item for a non-repeating definition"),
    ("questionnaire.required-answer", "no answer for a required item or group"),
    ("questionnaire.disabled-answer", "an answer to an item enableWhen disables"),
    ("questionnaire.item-shape", "items or answers where the item type forbids them"),
    ("questionnaire.answer-type", "an answer of the wrong type for the item"),
    ("questionnaire.answer-not-in-set", "a coded or constrained answer outside the permitted values"),
    ("questionnaire.answer-exclusive", "an exclusive option selected with others"),
    ("questionnaire.answer-count", "fewer or more answers than the item allows"),
    ("questionnaire.answer-range", "an answer outside its minimum, maximum, length or precision"),
    ("questionnaire.answer-content", "an answer whose content is not allowed (new lines, format or regex, mime type, attachment size, reference type)"),
    ("questionnaire.answer-text", "answer text that does not match the definition"),
    ("questionnaire.authored", "authored outside the permitted period"),
    ("questionnaire.definition", "a defect in the Questionnaire definition itself (enableWhen targets, option lists, item types)"),
    ("questionnaire.value-set", "a value set the Questionnaire names could not be found"),
    // --- lowering inputs ---
    ("lower.input", "a lowering input that is unknown, of the wrong shape, or fixed by the profile"),
    // --- what was not checked ---
    ("not-supported.fhirpath", "an invariant outside the supported FHIRPath core was not evaluated"),
    ("not-supported.regex", "a regex outside the supported core was not evaluated"),
    ("not-supported.extension-context", "an extension context outside the supported FHIRPath core"),
    ("not-supported.slice", "a slice that could not be checked"),
    // --- fallback ---
    ("other.<type>", "a finding no family above claims (a defect of this table)"),
];

/// The id for a finding of `code` with `message`.
pub fn rule_id(code: IssueType, message: &str) -> &'static str {
    let m = message;
    let has = |s: &str| m.contains(s);
    let ends = |s: &str| message.ends_with(s);
    let starts = |s: &str| m.starts_with(s);
    // an invariant: `key: human` — the key as the generated table lists it
    // (`cmp-1`, `que-7`, `us-core-1`, `sdf-8a`, a profile's `pvc2`), or one
    // shaped like a key the table missed
    if let Some((key, _)) = m.split_once(": ") {
        if let Ok(i) = INVARIANT_IDS.binary_search_by(|(k, _)| k.cmp(&key)) {
            if ends("not evaluated: outside the supported FHIRPath core") {
                return "not-supported.fhirpath";
            }
            return INVARIANT_IDS[i].1;
        }
        if is_invariant_key(key) {
            return if ends("not evaluated: outside the supported FHIRPath core") {
                "not-supported.fhirpath"
            } else {
                "inv.other"
            };
        }
    }
    // anchored at the end: a narrative finding quotes narrative text, which
    // may say anything
    if ends("not evaluated: outside the supported FHIRPath core") {
        return if starts("Extension context") { "not-supported.extension-context" } else { "not-supported.fhirpath" };
    }
    if ends("not evaluated: outside the supported regex core") {
        return "not-supported.regex";
    }
    if starts("Error parsing XHTML") {
        return "narrative.parse";
    }
    if starts("Wrong name on the XHTML") || starts("Wrong namespace on the XHTML") {
        return "narrative.root";
    }
    if starts("Invalid element name") || starts("Elements of type") || starts("Found block structures") {
        return "narrative.element";
    }
    if starts("Invalid attribute name") {
        return "narrative.attribute";
    }
    if starts("Invalid entity in the XHTML") {
        return "narrative.entity";
    }
    if starts("Hyperlink") || starts("The URL is not valid") || starts("Image source") {
        return "narrative.link";
    }
    if starts("could not parse JSON") {
        return "parse.json";
    }
    if starts("resourceType") || has("is not a FHIR R4 resource type") {
        return "parse.resource-type";
    }
    if starts("declared profile") {
        return if has("is not compiled here") { "profile.unknown" } else { "profile.wrong-type" };
    }
    if has("is not an element of") {
        return "structure.unknown-element";
    }
    if has(" is a JSON ") || has("extension carrier is a JSON object") {
        return "structure.json-type";
    }
    if has("is not a FHIR resource") {
        return "structure.not-a-resource";
    }
    if has("is a single element, not an array") || has("repeats: expected an array") || has("is an empty array") {
        return "structure.array";
    }
    if has("could not be assigned to a slice") {
        return "not-supported.slice";
    }
    if has("matching no slice") {
        return "structure.slice";
    }
    if starts("The entry resource did not match any of the allowed profiles") {
        return "structure.entry-type";
    }
    if has("no `_` carrier")
        || has("extension carrier must have some content")
        || has("Resource.id cannot carry extensions")
    {
        return "structure.carrier";
    }
    if has("allows one choice") || has("allows the types") {
        return "structure.choice";
    }
    if has("is required (min") {
        return "structure.required";
    }
    if has("exceeds max") {
        return "structure.max";
    }
    if has("matching more than one slice") || has("was not checked:") && has("slice '") {
        return if has("was not checked") { "not-supported.slice" } else { "structure.slice" };
    }
    if has("is not one of the resource types this entry allows") {
        return "structure.entry-type";
    }
    if has("No code provided, and a code is required") {
        return "binding.no-code";
    }
    // a questionnaire answer outside its permitted values, before the general bindings
    if has("specified by the questionnaire")
        || has("is not in the set of permitted values")
        || has("not in the list of allowed values")
        || has("has a unit that is not in the unit value set")
    {
        return "questionnaire.answer-not-in-set";
    }
    if has("is not in the bound value set") || has("was not found in the value set") {
        return "binding.not-member";
    }
    if starts("no bundled terminology") {
        return "binding.unknown";
    }
    if starts("Unknown code") {
        return "code.unknown";
    }
    if has("Invalid Resource target type") || has("not one of the types it allows") {
        return "reference.target-type";
    }
    if starts("Unable to find a profile match") {
        return "reference.target-profile";
    }
    if starts("Relative URLs must be") {
        return "reference.shape";
    }
    if has("is not in the bundle") {
        return "bundle.unresolved";
    }
    if has("is not reachable from the Composition") {
        return "bundle.unreachable";
    }
    if starts("Bundle entry missing fullUrl") || starts("The fullUrl '") {
        return "bundle.full-url";
    }
    if has("must be '") && has("(found '") {
        return "value.fixed";
    }
    if has("does not carry the profile's pattern") {
        return "value.pattern";
    }
    if has("is not a valid ") {
        return "value.invalid";
    }
    if starts("Identifier.system") || starts("if identifier.system") {
        return "value.identifier";
    }
    if starts("Extension.url must be") {
        return "value.extension-url";
    }
    if starts("Invalid displayLanguage") {
        return "value.language";
    }
    if starts("No definition could be found for URL") {
        return "value.url-unknown";
    }
    if has("is not allowed to be used at this point") {
        return "extension.context";
    }
    if has("could not be found so is not allowed here") {
        return "extension.unknown";
    }
    // questionnaires
    if starts("The questionnaire '") && has("is loaded; the response was not validated") {
        return "questionnaire.version-mismatch";
    }
    if starts("The questionnaire '") || starts("No questionnaire is identified") {
        return "questionnaire.unresolved";
    }
    if starts("LinkId '") {
        return "questionnaire.linkid-unknown";
    }
    if starts("Structural Error: item with linkid") {
        return "questionnaire.wrong-place";
    }
    if starts("Structural Error: items are out of order") {
        return "questionnaire.out-of-order";
    }
    if starts("Only one response") {
        return "questionnaire.duplicate-item";
    }
    if starts("No response answer found for required") || starts("No sub-items found for required group") {
        return "questionnaire.required-answer";
    }
    if starts("Item has answer, even though it is not enabled") {
        return "questionnaire.disabled-answer";
    }
    if starts("Items not of type") || starts("Items of type") {
        return "questionnaire.item-shape";
    }
    if starts("Answer value must be") || starts("Unable to determine the resource type") {
        return "questionnaire.answer-type";
    }
    if has("answerConstraint is 'optionsOnly'") {
        return "questionnaire.answer-not-in-set";
    }
    if starts("No LinkId") {
        return "questionnaire.linkid-unknown";
    }
    if has("is an exclusive option") {
        return "questionnaire.answer-exclusive";
    }
    if starts("The minimum number of answers") || starts("The maximum number of answers") {
        return "questionnaire.answer-count";
    }
    if starts("The value ") && (has("greater than") || has("less than") || has("decimal places"))
        || starts("The answer '") && (has("longer then") || has("shorter then"))
        || starts("The quantity ")
    {
        return "questionnaire.answer-range";
    }
    if starts("The answer should not contain")
        || starts("A mime type is required")
        || starts("The mime type ")
        || starts("The attachment is too large")
        || starts("The resource type '")
        || (starts("The answer '") && has("does not conform"))
        || (starts("The value ") && has("has a unit that is not valid"))
    {
        return "questionnaire.answer-content";
    }
    if starts("If text exists") {
        return "questionnaire.answer-text";
    }
    if starts("The authored date") {
        return "questionnaire.authored";
    }
    if starts("The value set '") || starts("The unit value set '") {
        return "questionnaire.value-set";
    }
    if starts("Questions with an enableWhen")
        || starts("Target for this question")
        || starts("Unable to find an item with the linkId")
        || starts("Definition for item")
        || starts("Option list has no")
        || starts("Cannot validate")
    {
        return "questionnaire.definition";
    }
    if starts("'")
        && (has("expects") || has("is not an input of") || has("takes one object") || has("must be an object"))
        || (has("is fixed to") && has("by the profile"))
    {
        return "lower.input";
    }
    match code {
        IssueType::Structure => "other.structure",
        IssueType::Required => "other.required",
        IssueType::Value => "other.value",
        IssueType::Invariant => "other.invariant",
        IssueType::CodeInvalid => "other.code-invalid",
        IssueType::NotSupported => "other.not-supported",
        IssueType::BusinessRule => "other.business-rule",
        IssueType::Informational => "other.informational",
    }
}

/// Shaped like an invariant key the table does not list: lowercase, digits
/// and dashes, at least one dash, never a space (`cmp-1`, `val-audit-source`).
fn is_invariant_key(key: &str) -> bool {
    key.len() <= 64
        && key.contains('-')
        && key.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

include!("rules_invariants.rs");
