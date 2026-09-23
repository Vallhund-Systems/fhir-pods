//! A `QuestionnaireResponse` against its `Questionnaire`.
//!
//! The response names its questionnaire by canonical; once that is in hand
//! (a contained resource, a bundle entry, or one the host loaded into the
//! [`Validator`](crate::Validator)), every response item is held to its
//! definition the way validator_cli 6.10.4's `QuestionnaireValidator` holds
//! it — same rules, same order, same words — so the two agree on the
//! fhir-test-cases questionnaire suite:
//!
//! - every item's `linkId` names a definition, at the level it sits (a
//!   definition found elsewhere is "in the wrong place"), in definition
//!   order;
//! - a required question owes an answer when it is enabled; a required
//!   group owes sub-items; an item that is not enabled (its `enableWhen`
//!   fails against the answers given) must carry no answer;
//! - one answer unless the definition repeats, and the `minOccurs` /
//!   `maxOccurs` extensions when present; a group or display item carries
//!   no answers; a non-group carries no `.item` outside its answers;
//! - the answer's value is of the definition's type (a `choice` takes a
//!   Coding, or a date / time / integer / string among the options), and
//!   within the definition's bounds: `minValue` / `maxValue`,
//!   `maxDecimalPlaces`, `minLength` / `maxLength`, `regex`, an
//!   attachment's `mimeType` / `maxSize`, a reference's
//!   `referenceResource`, a quantity's unit options or SDC min/max;
//! - a coded answer is one of the `answerOption`s (an `optionExclusive`
//!   one alone) or a member of the `answerValueSet`, through the same
//!   terminology provider every other binding uses.
//!
//! Only a `completed` or `amended` response is judged, as the reference
//! validator judges it; an in-progress one is not held to its definition.
//! What no questionnaire can be found for is said (a warning), never
//! guessed.
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::json::Value;
use crate::terminology::{Membership, TerminologyProvider};
use crate::validate::{IssueType, Report, Severity};

// Every finding here is a `BusinessRule` (the reference validator's own
// coding for most of them): the wire's `Required` is located at the
// element's parent, and these are located where the reference validator
// locates them — the response item, its answer, or its value.

const EXT: &str = "http://hl7.org/fhir/StructureDefinition/";
const SDC: &str = "http://hl7.org/fhir/uv/sdc/StructureDefinition/";

/// A parsed `Questionnaire`: what the checker reads of it.
#[derive(Debug, Clone)]
pub struct Questionnaire {
    pub url: Option<String>,
    pub version: Option<String>,
    effective_start: Option<String>,
    effective_end: Option<String>,
    /// Contained value sets an `answerValueSet` may name (`#id`): each
    /// enumerated (system, code) pair, or `None` when the set's compose is
    /// not an enumeration the pod can decide.
    contained_sets: Vec<ContainedSet>,
    items: Vec<Item>,
}

/// A contained ValueSet: its id, its name, and its (system, code) pairs
/// when the compose is an enumeration the pod can decide.
type ContainedSet = (String, Option<String>, Option<Vec<(String, String)>>);

#[derive(Debug, Clone, Default)]
struct Item {
    link_id: String,
    ty: String,
    required: bool,
    repeats: bool,
    text: Option<String>,
    max_length: Option<i64>,
    enable_when: Vec<EnableWhen>,
    /// `enableBehavior = any`; `all` otherwise.
    enable_any: bool,
    options: Vec<AnswerOption>,
    answer_value_set: Option<String>,
    /// R5's `answerConstraint`, carried in R4 by its cross-version extension:
    /// `optionsOnly` (the default), `optionsOrType`, `optionsOrString`.
    constraint: Option<String>,
    // extensions, each as the reference validator reads it
    min_value: Option<(String, String)>,
    max_value: Option<(String, String)>,
    max_decimal_places: Option<i64>,
    min_length: Option<i64>,
    regex: Option<String>,
    entry_format: Option<String>,
    mime_types: Vec<String>,
    max_size: Option<i64>,
    min_occurs: Option<i64>,
    max_occurs: Option<i64>,
    reference_resources: Vec<String>,
    unit_options: Vec<(String, String)>,
    unit_value_set: Option<String>,
    sdc_min_quantity: Option<Quantity>,
    sdc_max_quantity: Option<Quantity>,
    items: Vec<Item>,
}

#[derive(Debug, Clone)]
struct EnableWhen {
    question: String,
    operator: String,
    /// The `answer[x]` key (`answerBoolean`, `answerCoding` …) and its value.
    answer: Option<(String, Answer)>,
}

#[derive(Debug, Clone)]
struct AnswerOption {
    /// `valueCoding`, `valueString`, `valueInteger`, `valueDate`, `valueTime`, `valueReference`.
    key: String,
    value: Answer,
    exclusive: bool,
}

/// A value as an answer, an option or an enableWhen carries it.
#[derive(Debug, Clone, PartialEq)]
enum Answer {
    Text(String),
    Coding { system: String, code: String },
    Quantity(Quantity),
    Reference(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Quantity {
    value: String,
    unit: String,
    system: String,
    code: String,
}

// --- reading a Questionnaire --------------------------------------------------

fn member<'v, 'a>(v: &'v Value<'a>, key: &str) -> Option<&'v Value<'a>> {
    match v {
        Value::Object(kvs) => kvs.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v),
        _ => None,
    }
}

fn text<'v>(v: &'v Value<'_>, key: &str) -> Option<&'v str> {
    match member(v, key)? {
        Value::Str(s) => Some(s.as_ref()),
        Value::Number(n) => Some(n),
        Value::Bool(true) => Some("true"),
        Value::Bool(false) => Some("false"),
        _ => None,
    }
}

fn items_of<'v, 'a>(v: &'v Value<'a>, key: &str) -> Vec<&'v Value<'a>> {
    match member(v, key) {
        Some(Value::Array(xs)) => xs.iter().collect(),
        Some(x @ Value::Object(_)) => alloc::vec![x],
        _ => Vec::new(),
    }
}

/// The `value[x]` / `answer[x]` member of an object: its key and its value.
fn choice<'v, 'a>(v: &'v Value<'a>, stem: &str) -> Option<(&'v str, &'v Value<'a>)> {
    match v {
        Value::Object(kvs) => kvs.iter().find_map(|(k, x)| {
            let k = k.as_ref();
            (k.len() > stem.len() && k.starts_with(stem) && k.as_bytes()[stem.len()].is_ascii_uppercase())
                .then_some((k, x))
        }),
        _ => None,
    }
}

fn answer_of(key: &str, v: &Value<'_>) -> Option<Answer> {
    Some(match key.trim_start_matches("value").trim_start_matches("answer") {
        "Coding" => Answer::Coding {
            system: text(v, "system").unwrap_or("").to_string(),
            code: text(v, "code").unwrap_or("").to_string(),
        },
        "Quantity" => Answer::Quantity(Quantity {
            value: text(v, "value").unwrap_or("").to_string(),
            unit: text(v, "unit").unwrap_or("").to_string(),
            system: text(v, "system").unwrap_or("").to_string(),
            code: text(v, "code").unwrap_or("").to_string(),
        }),
        "Reference" => Answer::Reference(text(v, "reference").unwrap_or("").to_string()),
        "Attachment" => return None,
        _ => match v {
            Value::Str(s) => Answer::Text(s.to_string()),
            Value::Number(n) => Answer::Text(n.to_string()),
            Value::Bool(b) => Answer::Text(b.to_string()),
            _ => return None,
        },
    })
}

fn extensions<'v, 'a>(v: &'v Value<'a>, url: &str) -> Vec<&'v Value<'a>> {
    items_of(v, "extension").into_iter().filter(|e| text(e, "url") == Some(url)).collect()
}

fn ext_value<'v, 'a>(v: &'v Value<'a>, url: &str) -> Option<(&'v str, &'v Value<'a>)> {
    extensions(v, url).first().and_then(|e| choice(e, "value"))
}

fn ext_text(v: &Value<'_>, url: &str) -> Option<String> {
    match ext_value(v, url)? {
        (_, Value::Str(s)) => Some(s.to_string()),
        (_, Value::Number(n)) => Some(n.to_string()),
        (_, Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

fn ext_int(v: &Value<'_>, url: &str) -> Option<i64> {
    ext_text(v, url)?.parse().ok()
}

impl Questionnaire {
    /// Read a `Questionnaire` from its JSON; `None` when the bytes are not one.
    pub fn parse(bytes: &[u8]) -> Option<Questionnaire> {
        let v = crate::json::parse(bytes).ok()?;
        Self::from_value(&v)
    }

    pub(crate) fn from_value(v: &Value<'_>) -> Option<Questionnaire> {
        if text(v, "resourceType") != Some("Questionnaire") {
            return None;
        }
        let period = member(v, "effectivePeriod");
        let contained_sets = items_of(v, "contained")
            .into_iter()
            .filter(|c| text(c, "resourceType") == Some("ValueSet"))
            .map(|c| (text(c, "id").unwrap_or("").to_string(), text(c, "name").map(String::from), enumerate(c)))
            .collect();
        Some(Questionnaire {
            url: text(v, "url").map(String::from),
            version: text(v, "version").map(String::from),
            effective_start: period.and_then(|p| text(p, "start")).map(String::from),
            effective_end: period.and_then(|p| text(p, "end")).map(String::from),
            contained_sets,
            items: items_of(v, "item").into_iter().map(Item::read).collect(),
        })
    }

    /// Does this questionnaire answer to the canonical (`url`, or `url|version`)?
    fn matches(&self, canonical: &str) -> bool {
        let (url, version) = match canonical.split_once('|') {
            Some((u, v)) => (u, Some(v)),
            None => (canonical, None),
        };
        self.url.as_deref() == Some(url) && version.is_none_or(|v| self.version.as_deref() == Some(v))
    }
}

impl Item {
    fn read(v: &Value<'_>) -> Item {
        let mut it = Item {
            link_id: text(v, "linkId").unwrap_or("").to_string(),
            ty: text(v, "type").unwrap_or("").to_string(),
            required: text(v, "required") == Some("true"),
            repeats: text(v, "repeats") == Some("true"),
            text: text(v, "text").map(String::from),
            max_length: text(v, "maxLength").and_then(|s| s.parse().ok()),
            enable_any: text(v, "enableBehavior") == Some("any"),
            answer_value_set: text(v, "answerValueSet").map(String::from),
            constraint: ext_text(
                v,
                "http://hl7.org/fhir/5.0/StructureDefinition/extension-Questionnaire.item.answerConstraint",
            ),
            ..Item::default()
        };
        for ew in items_of(v, "enableWhen") {
            it.enable_when.push(EnableWhen {
                question: text(ew, "question").unwrap_or("").to_string(),
                operator: text(ew, "operator").unwrap_or("").to_string(),
                answer: choice(ew, "answer").and_then(|(k, x)| answer_of(k, x).map(|a| (k.to_string(), a))),
            });
        }
        for o in items_of(v, "answerOption") {
            if let Some((k, x)) = choice(o, "value") {
                if let Some(value) = answer_of(k, x) {
                    it.options.push(AnswerOption {
                        key: k.to_string(),
                        value,
                        exclusive: ext_text(o, &format!("{EXT}questionnaire-optionExclusive")).as_deref()
                            == Some("true"),
                    });
                }
            }
        }
        let typed = |url: &str| ext_value(v, url).and_then(|(k, x)| ext_scalar(x).map(|s| (k.to_string(), s)));
        it.min_value = typed(&format!("{EXT}minValue"));
        it.max_value = typed(&format!("{EXT}maxValue"));
        it.max_decimal_places = ext_int(v, &format!("{EXT}maxDecimalPlaces"));
        it.min_length = ext_int(v, &format!("{EXT}minLength"));
        it.regex = ext_text(v, &format!("{EXT}regex"));
        it.entry_format = ext_text(v, &format!("{EXT}entryFormat"));
        it.mime_types = extensions(v, &format!("{EXT}mimeType"))
            .into_iter()
            .filter_map(|e| choice(e, "value").and_then(|(_, x)| ext_scalar(x)))
            .collect();
        it.max_size = ext_int(v, &format!("{EXT}maxSize"));
        it.min_occurs = ext_int(v, &format!("{EXT}questionnaire-minOccurs"));
        it.max_occurs = ext_int(v, &format!("{EXT}questionnaire-maxOccurs"));
        it.reference_resources = extensions(v, &format!("{EXT}questionnaire-referenceResource"))
            .into_iter()
            .filter_map(|e| choice(e, "value").and_then(|(_, x)| ext_scalar(x)))
            .collect();
        it.unit_options = extensions(v, &format!("{EXT}questionnaire-unitOption"))
            .into_iter()
            .filter_map(|e| choice(e, "value"))
            .map(|(_, c)| (text(c, "system").unwrap_or("").to_string(), text(c, "code").unwrap_or("").to_string()))
            .collect();
        it.unit_value_set = ext_text(v, &format!("{EXT}questionnaire-unitValueSet"));
        let quantity = |url: &str| {
            ext_value(v, url).map(|(_, q)| Quantity {
                value: text(q, "value").unwrap_or("").to_string(),
                unit: text(q, "unit").unwrap_or("").to_string(),
                system: text(q, "system").unwrap_or("").to_string(),
                code: text(q, "code").unwrap_or("").to_string(),
            })
        };
        it.sdc_min_quantity = quantity(&format!("{SDC}sdc-questionnaire-minQuantity"));
        it.sdc_max_quantity = quantity(&format!("{SDC}sdc-questionnaire-maxQuantity"));
        it.items = items_of(v, "item").into_iter().map(Item::read).collect();
        it
    }

    /// The answer constraint in force: declared, else what the R4 type says.
    fn constraint(&self) -> &str {
        match self.constraint.as_deref() {
            Some(c) => c,
            None if self.ty == "open-choice" => "optionsOrString",
            None => "optionsOnly",
        }
    }

    fn find<'i>(items: &'i [Item], link_id: &str) -> Option<&'i Item> {
        for it in items {
            if it.link_id == link_id {
                return Some(it);
            }
            if let Some(found) = Item::find(&it.items, link_id) {
                return Some(found);
            }
        }
        None
    }
}

/// A contained ValueSet's codes, when its compose is plain enumerations
/// (an `include.concept` list per system, no filters, no nested sets).
fn enumerate(vs: &Value<'_>) -> Option<Vec<(String, String)>> {
    let compose = member(vs, "compose")?;
    if !items_of(compose, "exclude").is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for inc in items_of(compose, "include") {
        let system = text(inc, "system")?;
        let concepts = items_of(inc, "concept");
        if concepts.is_empty() || !items_of(inc, "filter").is_empty() || member(inc, "valueSet").is_some() {
            return None;
        }
        for c in concepts {
            out.push((system.to_string(), text(c, "code")?.to_string()));
        }
    }
    Some(out)
}

fn ext_scalar(x: &Value<'_>) -> Option<String> {
    match x {
        Value::Str(s) => Some(s.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

// --- the check ----------------------------------------------------------------

/// A response item beside its definition, up the tree: what `enableWhen`
/// looks through for the answers it asks about.
struct Frame<'v, 'a> {
    response: &'v Value<'a>,
}

struct Checker<'q, 'v, 'a, 't> {
    q: &'q Questionnaire,
    tx: &'t dyn TerminologyProvider,
    root: &'v Value<'a>,
    in_progress: bool,
    stack: Vec<Frame<'v, 'a>>,
}

/// Hold `qr` (a `QuestionnaireResponse`) to its questionnaire, found among
/// its contained resources, `bundle`'s entries or `loaded`.
pub(crate) fn check(
    qr: &Value<'_>,
    bundle: Option<&Value<'_>>,
    loaded: &[Questionnaire],
    tx: &dyn TerminologyProvider,
    label: &str,
    r: &mut Report,
) {
    let Some(canonical) = text(qr, "questionnaire").filter(|s| !s.is_empty()) else {
        r.push(
            Severity::Information,
            IssueType::Informational,
            label,
            "No questionnaire is identified, so no validation can be performed against the base questionnaire".into(),
        );
        return;
    };
    let owned;
    let q: &Questionnaire = if let Some(id) = canonical.strip_prefix('#') {
        match items_of(qr, "contained")
            .into_iter()
            .find(|c| text(c, "resourceType") == Some("Questionnaire") && text(c, "id") == Some(id))
            .and_then(Questionnaire::from_value)
        {
            Some(q) => {
                owned = q;
                &owned
            }
            None => return not_found(canonical, label, r),
        }
    } else if let Some(q) = loaded.iter().find(|q| q.matches(canonical)) {
        q
    } else if let Some(q) = bundle
        .into_iter()
        .flat_map(|b| items_of(b, "entry"))
        .filter_map(|e| member(e, "resource"))
        .filter(|res| text(res, "resourceType") == Some("Questionnaire"))
        .filter_map(Questionnaire::from_value)
        .find(|q| q.matches(canonical))
    {
        owned = q;
        &owned
    } else {
        return not_found(canonical, label, r);
    };
    // an in-progress response is judged like any other, except that a
    // missing required answer is a warning there, not an error
    let in_progress = !matches!(text(qr, "status").unwrap_or(""), "completed" | "amended");
    // the authoring date against the questionnaire's effective period
    if let Some(authored) = text(qr, "authored") {
        if let (Some(a), Some(end)) = (crate::temporal::parse(authored), q.effective_end.as_deref()) {
            if crate::temporal::parse(end).is_some_and(|e| a.millis > e.millis) {
                r.push(
                    Severity::Warning,
                    IssueType::BusinessRule,
                    &format!("{label}.authored"),
                    format!("The authored date '{authored}' is after the end of the permitted period defined in the Questionnaire ('{end}')"),
                );
            }
        }
        if let (Some(a), Some(start)) = (crate::temporal::parse(authored), q.effective_start.as_deref()) {
            if crate::temporal::parse(start).is_some_and(|s| a.millis < s.millis) {
                r.push(
                    Severity::Warning,
                    IssueType::BusinessRule,
                    &format!("{label}.authored"),
                    format!("The authored date '{authored}' is before the start of the permitted period defined in the Questionnaire ('{start}')"),
                );
            }
        }
    }
    let mut c = Checker { q, tx, root: qr, in_progress, stack: Vec::new() };
    c.items(&q.items, qr, label, r);
}

fn not_found(canonical: &str, label: &str, r: &mut Report) {
    let example = canonical.starts_with("http://example.org") || canonical.starts_with("https://example.org");
    r.push(
        if example { Severity::Information } else { Severity::Warning },
        IssueType::NotSupported,
        label,
        format!("The questionnaire '{canonical}' could not be resolved, so no validation can be performed against the base questionnaire"),
    );
}

impl<'q, 'v, 'a, 't> Checker<'q, 'v, 'a, 't> {
    /// The response items under `parent` against the definitions `defs`.
    fn items(&mut self, defs: &'q [Item], parent: &'v Value<'a>, label: &str, r: &mut Report) {
        let responses = items_of(parent, "item");
        // by definition: which response items answer to it, in response order
        let mut by_def: Vec<Vec<(usize, &'v Value<'a>)>> = alloc::vec![Vec::new(); defs.len()];
        let mut last_index: Option<usize> = None;
        for (i, item) in responses.iter().enumerate() {
            let at = format!("{label}.item[{i}]");
            let Some(link_id) = text(item, "linkId").filter(|s| !s.is_empty()) else {
                r.push(Severity::Error, IssueType::BusinessRule, &at, "No LinkId, so can't be validated".into());
                continue;
            };
            match defs.iter().position(|d| d.link_id == link_id) {
                None => match Item::find(&self.q.items, link_id) {
                    // defined, but not at this level
                    Some(def) => {
                        r.push(
                            Severity::Error,
                            IssueType::Structure,
                            &at,
                            format!("Structural Error: item with linkid {link_id} is in the wrong place"),
                        );
                        self.item(def, Some((i, item)), label, r);
                    }
                    None => r.push(
                        Severity::Error,
                        IssueType::Structure,
                        &at,
                        format!("LinkId '{link_id}' not found in questionnaire"),
                    ),
                },
                Some(index) => {
                    if last_index.is_some_and(|last| index < last) {
                        r.push(
                            Severity::Error,
                            IssueType::Structure,
                            &at,
                            "Structural Error: items are out of order".into(),
                        );
                    }
                    last_index = Some(index);
                    by_def[index].push((i, item));
                }
            }
        }
        for (def, found) in defs.iter().zip(by_def) {
            self.definition(def, &found, label, r);
        }
    }

    /// One definition against the response items that name it (none, one, several).
    fn definition(&mut self, def: &'q Item, found: &[(usize, &'v Value<'a>)], label: &str, r: &mut Report) {
        let enabled = self.enabled(def);
        if found.is_empty() {
            // an item the response leaves out altogether — group or question
            if enabled && def.required {
                r.push(
                    self.missing_severity(),
                    IssueType::BusinessRule,
                    label,
                    format!("No response answer found for required item '{}'", def.link_id),
                );
            }
            return;
        }
        if !enabled {
            for (i, item) in found {
                if member(item, "answer").is_some() || member(item, "item").is_some() {
                    r.push(
                        Severity::Error,
                        IssueType::BusinessRule,
                        &format!("{label}.item[{i}]"),
                        format!("Item has answer, even though it is not enabled (item id = '{}')", def.link_id),
                    );
                }
            }
            return;
        }
        if found.len() > 1 && !def.repeats {
            r.push(
                Severity::Error,
                IssueType::BusinessRule,
                &format!("{label}.item[{}]", found[1].0),
                format!("Only one response item with the linkId {} allowed - found {} items", def.link_id, found.len()),
            );
        }
        for (i, item) in found {
            self.item(def, Some((*i, item)), label, r);
        }
    }

    /// One response item against its definition.
    fn item(&mut self, def: &'q Item, at: Option<(usize, &'v Value<'a>)>, parent_label: &str, r: &mut Report) {
        let Some((index, item)) = at else { return };
        let label = format!("{parent_label}.item[{index}]");
        if let Some(t) = text(item, "text") {
            if !t.is_empty() && def.text.as_deref().is_some_and(|d| d != t) {
                r.push(
                    Severity::Error,
                    IssueType::Value,
                    &label,
                    format!("If text exists, it must match the questionnaire definition for linkId {}", def.link_id),
                );
            }
        }
        let answers = items_of(item, "answer");
        if answers.is_empty() {
            // a group present carries its requirement in its sub-items, never an answer
            if self.enabled(def) && def.required && def.ty != "group" {
                r.push(
                    self.missing_severity(),
                    IssueType::BusinessRule,
                    &label,
                    format!("No response answer found for required item '{}'", def.link_id),
                );
            }
        } else if answers.len() > 1 && !def.repeats {
            r.push(
                Severity::Error,
                IssueType::BusinessRule,
                &label,
                "Only one response answer item with this linkId allowed".into(),
            );
        }
        if let Some(max) = def.max_occurs.filter(|m| *m >= 0) {
            if answers.len() as i64 > max {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    &label,
                    format!("The maximum number of answers is {max} but this has {} answers", answers.len()),
                );
            }
        }
        if let Some(min) = def.min_occurs.filter(|m| *m >= 0) {
            if (answers.len() as i64) < min {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    &label,
                    format!("The minimum number of answers is {min} but this has {} answers", answers.len()),
                );
            }
        }
        if !answers.is_empty() {
            self.stack.push(Frame { response: item });
            for (j, answer) in answers.iter().enumerate() {
                let at = format!("{label}.answer[{j}]");
                self.answer(def, answer, &answers, &at, r);
                if def.ty != "group" {
                    // items nested inside an answer
                    self.items(&def.items, answer, &at, r);
                }
            }
            self.stack.pop();
        }
        match def.ty.as_str() {
            "" => r.push(
                Severity::Fatal,
                IssueType::Structure,
                &label,
                format!("Definition for item {} does not contain a type", def.link_id),
            ),
            "display" => {
                if !items_of(item, "item").is_empty() {
                    r.push(
                        Severity::Error,
                        IssueType::Structure,
                        &label,
                        format!("Items not of type DISPLAY should not have items - linkId {}", def.link_id),
                    );
                }
            }
            "group" => {
                let sub = items_of(item, "item");
                if def.required && sub.is_empty() {
                    r.push(
                        Severity::Error,
                        IssueType::BusinessRule,
                        &label,
                        "No sub-items found for required group".into(),
                    );
                }
                self.stack.push(Frame { response: item });
                self.items(&def.items, item, &label, r);
                self.stack.pop();
            }
            _ => {
                if !items_of(item, "item").is_empty() {
                    r.push(
                        Severity::Error,
                        IssueType::Structure,
                        &label,
                        "Items not of type group should not have items outside answers (use answer.item not .item)"
                            .into(),
                    );
                }
            }
        }
    }

    /// One answer against the definition's type and bounds.
    fn answer(&mut self, def: &'q Item, answer: &'v Value<'a>, all: &[&'v Value<'a>], at: &str, r: &mut Report) {
        let expected: &[&str] = match def.ty.as_str() {
            "group" | "question" => {
                r.push(
                    Severity::Error,
                    IssueType::Structure,
                    at.rsplit_once(".answer").map(|(l, _)| l).unwrap_or(at),
                    "Items of type question should not have answers".into(),
                );
                return;
            }
            "display" => {
                r.push(
                    Severity::Error,
                    IssueType::Structure,
                    at.rsplit_once(".answer").map(|(l, _)| l).unwrap_or(at),
                    "Items of type 'display' cannot have answers".into(),
                );
                return;
            }
            "boolean" => &["boolean"],
            "decimal" => &["decimal"],
            "integer" => &["integer"],
            "date" => &["date"],
            "dateTime" => &["dateTime"],
            "time" => &["time"],
            "string" | "text" => &["string"],
            "url" => &["uri"],
            "attachment" => &["Attachment"],
            "reference" => &["Reference"],
            "quantity" => &["Quantity"],
            "choice" | "open-choice" => &["Coding", "date", "time", "integer", "string"],
            _ => return,
        };
        let Some((key, value)) = choice(answer, "value") else { return };
        let found = &key["value".len()..];
        // a key the base spec does not define is the shape walk's finding
        if !matches!(
            found,
            "Boolean"
                | "Decimal"
                | "Integer"
                | "Date"
                | "DateTime"
                | "Time"
                | "String"
                | "Uri"
                | "Attachment"
                | "Coding"
                | "Quantity"
                | "Reference"
        ) {
            return;
        }
        let found_type = match found {
            "Coding" | "Quantity" | "Attachment" | "Reference" => found.to_string(),
            other => {
                let mut s = other.to_string();
                s[..1].make_ascii_lowercase();
                s
            }
        };
        let expected_type = expected.iter().find(|t| **t == found_type).copied();
        let vat = format!("{at}.{key}");
        let Some(ty) = expected_type else {
            let msg = if expected.len() == 1 {
                format!("Answer value must be of the type {} not {found_type}", expected[0])
            } else {
                format!(
                    "Answer value must be one of the {} types {} not {found_type}",
                    expected.len(),
                    expected.join(", ")
                )
            };
            r.push(Severity::Error, IssueType::Structure, &vat, msg);
            return;
        };
        let scalar = |v: &Value<'_>| -> Option<String> {
            match v {
                Value::Str(s) => Some(s.to_string()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            }
        };
        match def.ty.as_str() {
            "decimal" => {
                if let Some(s) = scalar(value) {
                    self.decimal_bounds(def, &s, &vat, r);
                    self.primitive_options(def, "valueDecimal", &s, "decimal", &vat, r);
                }
            }
            "integer" => {
                if let Some(s) = scalar(value) {
                    self.number_bounds(def, &s, &vat, r);
                    self.primitive_options(def, "valueInteger", &s, "integer", &vat, r);
                }
            }
            "date" | "dateTime" | "time" => {
                if let Some(s) = scalar(value) {
                    self.temporal_bounds(def, &s, &vat, r);
                    self.primitive_options(def, key, &s, &found_type, &vat, r);
                }
            }
            "string" => {
                if let Some(s) = scalar(value) {
                    self.string_rules(def, &s, &vat, r);
                }
            }
            "attachment" => self.attachment(def, value, at, r),
            "reference" => self.reference(def, value, at, r),
            "quantity" => self.quantity(def, value, &vat, r),
            "choice" | "open-choice" => match ty {
                "Coding" => self.coding(def, value, all, at, r),
                // a string where the definition allows only the members of its value set
                "string" if def.constraint() == "optionsOnly" && def.options.is_empty() && def.answer_value_set.is_some() => {
                    r.push(
                        Severity::Error,
                        IssueType::BusinessRule,
                        at,
                        format!(
                            "The answer to the item {} is a string, but answerConstraint is 'optionsOnly', so the answer has to be one of the options the questionnaire specifies",
                            def.link_id
                        ),
                    )
                }
                // any string, when the definition takes one; else an option of that type
                _ => {
                    if !(ty == "string" && def.constraint() == "optionsOrString") {
                        if let Some(s) = scalar(value) {
                            self.primitive_options(def, key, &s, ty, &vat, r);
                        }
                    }
                }
            },
            _ => {}
        }
    }

    fn decimal_bounds(&self, def: &Item, s: &str, at: &str, r: &mut Report) {
        let Ok(v) = s.parse::<f64>() else { return };
        if let Some((_, min)) = &def.min_value {
            if min.parse::<f64>().is_ok_and(|m| v < m) {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The value {s} is less than the allowed minimum of {min}"),
                );
            }
        }
        if let Some((_, max)) = &def.max_value {
            if max.parse::<f64>().is_ok_and(|m| v > m) {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The value {s} is greater than the allowed maximum of {max}"),
                );
            }
        }
        if let Some(limit) = def.max_decimal_places {
            if precision(s) > limit {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The value {s} has too many decimal places (limit = {limit})"),
                );
            }
        }
    }

    fn number_bounds(&self, def: &Item, s: &str, at: &str, r: &mut Report) {
        let Ok(v) = s.parse::<i64>() else { return };
        if let Some((_, min)) = &def.min_value {
            if min.parse::<i64>().is_ok_and(|m| v < m) {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The value {s} is less than the allowed minimum of {min}"),
                );
            }
        }
        if let Some((_, max)) = &def.max_value {
            if max.parse::<i64>().is_ok_and(|m| v > m) {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The value {s} is greater than the allowed maximum of {max}"),
                );
            }
        }
    }

    fn temporal_bounds(&self, def: &Item, s: &str, at: &str, r: &mut Report) {
        let Some(v) = crate::temporal::parse(s) else { return };
        // a partial date is taken at its start on both sides, as the reference
        // validator's DateTimeType compares them
        if let Some((_, min)) = &def.min_value {
            if crate::temporal::parse(min).is_some_and(|m| v.millis < m.millis) {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The value {s} is less than the allowed minimum of {min}"),
                );
            }
        }
        if let Some((_, max)) = &def.max_value {
            if crate::temporal::parse(max).is_some_and(|m| v.millis > m.millis) {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The value {s} is greater than the allowed maximum of {max}"),
                );
            }
        }
    }

    fn string_rules(&self, def: &Item, s: &str, at: &str, r: &mut Report) {
        let len = s.chars().count() as i64;
        if let Some(min) = def.min_length.filter(|m| *m >= 0) {
            if len < min {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The answer '{s}' is shorter then the required minimum length of {min}"),
                );
            }
        }
        if let Some(max) = def.max_length.filter(|m| *m >= 0) {
            if len > max {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The answer '{s}' is longer then the allowed maximum length of {max}"),
                );
            }
        }
        if let Some(re) = &def.regex {
            match crate::regex::Regex::new(re) {
                Ok(rx) if !rx.is_match(s) => r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    match &def.entry_format {
                        Some(f) => {
                            format!("The answer '{s}' does not conform to the expected format '{f}' (regex '{re}')")
                        }
                        None => format!("The answer '{s}' does not conform to the required regex '{re}'"),
                    },
                ),
                Ok(_) => {}
                Err(_) => r.push(
                    Severity::Warning,
                    IssueType::NotSupported,
                    at,
                    format!("regex '{re}' not evaluated: outside the supported regex core"),
                ),
            }
        }
        if s.contains('\r') || s.contains('\n') {
            r.push(
                Severity::Warning,
                IssueType::BusinessRule,
                at,
                "The answer should not contain new line characters".into(),
            );
        }
        if let Some(vs) = &def.answer_value_set {
            match self.membership(vs, "", s).0 {
                Membership::Member => {}
                Membership::NotMember => r.push(
                    Severity::Error,
                    IssueType::CodeInvalid,
                    at,
                    format!("The value '{s}' is not in the list of allowed values (value set '{vs}')"),
                ),
                Membership::Unknown => r.push(
                    Severity::Warning,
                    IssueType::NotSupported,
                    at,
                    format!("The value set '{vs}' could not be found"),
                ),
            }
        }
        self.primitive_options(def, "valueString", s, "string", at, r);
    }

    /// A primitive answer among the definition's `answerOption`s of its kind.
    fn primitive_options(&self, def: &Item, key: &str, s: &str, kind: &str, at: &str, r: &mut Report) {
        if def.options.is_empty() {
            // nothing to hold it to — said, not assumed (the reference validator's hint)
            if matches!(def.ty.as_str(), "choice" | "open-choice") {
                r.push(
                    Severity::Information,
                    IssueType::Informational,
                    at.rsplit_once(".value").map(|(a, _)| a).unwrap_or(at),
                    format!("Cannot validate {kind} answer option because no option list is provided"),
                );
            }
            return;
        }
        let of_kind: Vec<&AnswerOption> = def.options.iter().filter(|o| o.key == key).collect();
        if of_kind.is_empty() {
            r.push(
                Severity::Error,
                IssueType::Invariant,
                at,
                format!("Option list has no option values of type {kind}"),
            );
            return;
        }
        if !of_kind.iter().any(|o| matches!(&o.value, Answer::Text(t) if t == s)) {
            let what = match kind {
                "decimal" => "value",
                other => other,
            };
            r.push(
                Severity::Error,
                IssueType::Invariant,
                at,
                format!("The {what} {s} is not in the set of permitted values"),
            );
        }
    }

    fn coding(&self, def: &Item, value: &Value<'_>, all: &[&Value<'_>], at: &str, r: &mut Report) {
        let system = text(value, "system").unwrap_or("");
        let code = text(value, "code").unwrap_or("");
        if !def.options.is_empty() {
            let codings: Vec<&AnswerOption> = def.options.iter().filter(|o| o.key == "valueCoding").collect();
            if codings.is_empty() {
                r.push(
                    Severity::Error,
                    IssueType::Invariant,
                    at,
                    "Option list has no option values of type coding".into(),
                );
                return;
            }
            let matched = codings.iter().find(|o| {
                matches!(&o.value, Answer::Coding { system: s, code: c } if c == code && (s.is_empty() || s == system))
            });
            match matched {
                Some(o) => {
                    if o.exclusive && all.len() > 1 {
                        r.push(
                            Severity::Error,
                            IssueType::BusinessRule,
                            at,
                            format!("Selected answer {system}#{code} is an exclusive option - can't select anything else at the same time"),
                        );
                    }
                }
                None => r.push(
                    Severity::Error,
                    IssueType::Invariant,
                    at,
                    format!("The code {system}::{code} is not in the set of permitted values"),
                ),
            }
        } else if let Some(vs) = &def.answer_value_set {
            // any coding at all is allowed under optionsOrType
            if code.is_empty() || system.is_empty() || def.constraint() == "optionsOrType" {
                return;
            }
            let (m, name) = self.membership(vs, system, code);
            match m {
                Membership::Member => {}
                Membership::NotMember => r.push(
                    Severity::Error,
                    IssueType::CodeInvalid,
                    at,
                    format!(
                        "The code '{code}' in the system '{system}' is not in the options value set ({name}) specified by the questionnaire. Terminology Error: The provided code '{system}#{code}' was not found in the value set '{vs}'"
                    ),
                ),
                Membership::Unknown => r.push(
                    Severity::Warning,
                    IssueType::NotSupported,
                    at,
                    format!("no bundled terminology for {vs}; '{code}' not checked"),
                ),
            }
        } else {
            r.push(
                Severity::Information,
                IssueType::Informational,
                at,
                "Cannot validate options because no option or options are provided".into(),
            );
        }
    }

    fn attachment(&self, def: &Item, value: &Value<'_>, at: &str, r: &mut Report) {
        let content_type = text(value, "contentType");
        if !def.mime_types.is_empty() {
            match content_type {
                None => r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    "A mime type is required for this answer".into(),
                ),
                Some(ct) => {
                    if !def.mime_types.iter().any(|allowed| mime_matches(allowed, ct)) {
                        r.push(
                            Severity::Error,
                            IssueType::BusinessRule,
                            at,
                            format!(
                                "The mime type {ct} is not valid for this answer (allowed = {})",
                                def.mime_types.join(", ")
                            ),
                        );
                    }
                }
            }
        }
        if let Some(max) = def.max_size.filter(|m| *m > 0) {
            // the stated size, else the decoded length of the data
            let size = text(value, "size").and_then(|s| s.parse::<i64>().ok()).or_else(|| {
                text(value, "data").map(|d| {
                    let d = d.trim_end_matches('=');
                    (d.len() as i64 * 3) / 4
                })
            });
            if let Some(size) = size {
                if size > max {
                    r.push(
                        Severity::Error,
                        IssueType::BusinessRule,
                        at,
                        format!("The attachment is too large (allowed = {max}, found = {size})"),
                    );
                }
            }
        }
    }

    fn reference(&self, def: &Item, value: &Value<'_>, at: &str, r: &mut Report) {
        if def.reference_resources.is_empty() {
            return;
        }
        let reference = text(value, "reference").unwrap_or("");
        let rt = text(value, "type").map(String::from).or_else(|| {
            let path = reference.split(['?', '#']).next().unwrap_or("");
            let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            // `Type/id` or `…/Type/id/_history/v`
            let i = segs
                .iter()
                .rposition(|s| *s == "_history")
                .map(|h| h.saturating_sub(2))
                .unwrap_or(segs.len().saturating_sub(2));
            segs.get(i).filter(|s| s.chars().next().is_some_and(|c| c.is_ascii_uppercase())).map(|s| s.to_string())
        });
        match rt {
            None => r.push(
                Severity::Error,
                IssueType::BusinessRule,
                at,
                "Unable to determine the resource type, so the resource type can't be checked".into(),
            ),
            Some(rt) if !def.reference_resources.contains(&rt) => r.push(
                Severity::Error,
                IssueType::BusinessRule,
                at,
                format!(
                    "The resource type '{rt}' in the reference {reference} is not allowed (allowed = {})",
                    def.reference_resources.join(", ")
                ),
            ),
            _ => {}
        }
    }

    fn quantity(&self, def: &Item, value: &Value<'_>, at: &str, r: &mut Report) {
        let q = Quantity {
            value: text(value, "value").unwrap_or("").to_string(),
            unit: text(value, "unit").unwrap_or("").to_string(),
            system: text(value, "system").unwrap_or("").to_string(),
            code: text(value, "code").unwrap_or("").to_string(),
        };
        let display = |q: &Quantity| {
            let unit = if !q.unit.is_empty() { q.unit.clone() } else { q.code.clone() };
            if unit.is_empty() {
                q.value.clone()
            } else {
                format!("{} {unit}", q.value)
            }
        };
        let from_ext = |b: &Option<(String, String)>| -> Option<Quantity> {
            // a `minValue` / `maxValue` given as a bare number bounds the value alone
            b.as_ref().filter(|(k, _)| k != "valueQuantity").map(|(_, v)| Quantity { value: v.clone(), ..q.clone() })
        };
        let min_bound = def.sdc_min_quantity.clone().or_else(|| from_ext(&def.min_value));
        let max_bound = def.sdc_max_quantity.clone().or_else(|| from_ext(&def.max_value));
        for (bound, word, is_min) in [(&min_bound, "minimum", true), (&max_bound, "maximum", false)] {
            let Some(b) = bound else { continue };
            let formal = |x: &Quantity| x.system == "http://unitsofmeasure.org" && !x.code.is_empty();
            if !(formal(&q) && formal(b)) {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!(
                        "The quantity {} cannot be compared to the allowed {word} of {} because no formal units are specified",
                        display(&q),
                        display(b)
                    ),
                );
                continue;
            }
            if q.code != b.code {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The quantity {} cannot be compared to the allowed {word} of {}", display(&q), display(b)),
                );
                continue;
            }
            if let (Ok(v), Ok(bv)) = (q.value.parse::<f64>(), b.value.parse::<f64>()) {
                if (is_min && v < bv) || (!is_min && v > bv) {
                    r.push(
                        Severity::Error,
                        IssueType::BusinessRule,
                        at,
                        format!(
                            "The quantity {} is {} than the allowed {word} of {}",
                            display(&q),
                            if is_min { "less" } else { "greater" },
                            display(b)
                        ),
                    );
                }
            }
        }
        if let Some(limit) = def.max_decimal_places {
            if precision(&q.value) > limit {
                r.push(
                    Severity::Error,
                    IssueType::BusinessRule,
                    at,
                    format!("The value {} has too many decimal places (limit = {limit})", q.value),
                );
            }
        }
        if !def.unit_options.is_empty()
            && !def.unit_options.iter().any(|(s, c)| *c == q.code && (s.is_empty() || *s == q.system))
        {
            r.push(
                Severity::Error,
                IssueType::BusinessRule,
                at,
                format!(
                    "The value {} has a unit that is not valid (allowed = {})",
                    display(&q),
                    def.unit_options.iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>().join(", ")
                ),
            );
        }
        if let Some(vs) = &def.unit_value_set {
            match self.tx.validate_code(vs, &q.system, &q.code) {
                Membership::Member => {}
                Membership::NotMember => r.push(
                    Severity::Error,
                    IssueType::CodeInvalid,
                    at,
                    format!("The value {} has a unit that is not in the unit value set '{vs}'", display(&q)),
                ),
                Membership::Unknown => r.push(
                    Severity::Warning,
                    IssueType::NotSupported,
                    at,
                    format!("The unit value set '{vs}' could not be found"),
                ),
            }
        }
    }

    /// Membership in an answer's value set: a set contained in the
    /// questionnaire (`#id`) decides from its enumeration; any other through
    /// the terminology provider. With the name the reference validator prints.
    fn membership(&self, vs: &str, system: &str, code: &str) -> (Membership, String) {
        if let Some(id) = vs.strip_prefix('#') {
            return match self.q.contained_sets.iter().find(|(i, _, _)| i == id) {
                Some((_, name, Some(codes))) => (
                    if codes.iter().any(|(s, c)| c == code && (s == system || system.is_empty())) {
                        Membership::Member
                    } else {
                        Membership::NotMember
                    },
                    name.clone().unwrap_or_else(|| "ValueSet[null]".into()),
                ),
                _ => (Membership::Unknown, "ValueSet[null]".into()),
            };
        }
        (self.tx.validate_code(vs, system, code), vs.rsplit('/').next().unwrap_or(vs).to_string())
    }

    fn missing_severity(&self) -> Severity {
        if self.in_progress {
            Severity::Warning
        } else {
            Severity::Error
        }
    }

    // --- enableWhen -----------------------------------------------------------

    /// Is the definition enabled, given the answers in the response so far?
    fn enabled(&self, def: &Item) -> bool {
        if def.enable_when.is_empty() {
            return true;
        }
        let results = def.enable_when.iter().map(|ew| self.condition(ew));
        if def.enable_any {
            results.into_iter().any(|x| x)
        } else {
            results.into_iter().all(|x| x)
        }
    }

    fn condition(&self, ew: &EnableWhen) -> bool {
        let answers = self.answers_to(&ew.question);
        let Some((key, expected)) = &ew.answer else { return false };
        if ew.operator == "exists" {
            let wants = matches!(expected, Answer::Text(t) if t == "true");
            return answers.is_empty() != wants;
        }
        answers.iter().any(|a| {
            let Some((ak, av)) = choice(a, "value") else { return false };
            let Some(actual) = answer_of(ak, av) else { return false };
            compare(&ew.operator, &actual, expected, key)
        })
    }

    /// The answers the response gives to `link_id`: looked for under each
    /// enclosing item, innermost first, then the whole response.
    fn answers_to(&self, link_id: &str) -> Vec<&'v Value<'a>> {
        for frame in self.stack.iter().rev() {
            let found = find_answers(frame.response, link_id);
            if !found.is_empty() {
                return found;
            }
        }
        find_answers(self.root, link_id)
    }
}

fn find_answers<'v, 'a>(node: &'v Value<'a>, link_id: &str) -> Vec<&'v Value<'a>> {
    let mut out = Vec::new();
    for item in items_of(node, "item") {
        if text(item, "linkId") == Some(link_id) {
            out.extend(items_of(item, "answer"));
        }
        out.extend(find_answers(item, link_id));
        for answer in items_of(item, "answer") {
            out.extend(find_answers(answer, link_id));
        }
    }
    out
}

/// `actual <op> expected`, typed by the enableWhen's `answer[x]` key.
fn compare(op: &str, actual: &Answer, expected: &Answer, key: &str) -> bool {
    use core::cmp::Ordering;
    let ord: Option<Ordering> = match (actual, expected) {
        (Answer::Coding { system: s1, code: c1 }, Answer::Coding { system: s2, code: c2 }) => {
            return match op {
                "=" => c1 == c2 && (s2.is_empty() || s1 == s2),
                "!=" => !(c1 == c2 && (s2.is_empty() || s1 == s2)),
                _ => false,
            };
        }
        (Answer::Quantity(a), Answer::Quantity(b)) => {
            if !b.code.is_empty() && a.code != b.code {
                return op == "!=";
            }
            a.value.parse::<f64>().ok().zip(b.value.parse::<f64>().ok()).and_then(|(x, y)| x.partial_cmp(&y))
        }
        (Answer::Reference(a), Answer::Reference(b)) => Some(a.cmp(b)),
        (Answer::Text(a), Answer::Text(b)) => match key {
            "answerInteger" | "answerDecimal" => {
                a.parse::<f64>().ok().zip(b.parse::<f64>().ok()).and_then(|(x, y)| x.partial_cmp(&y))
            }
            "answerDate" | "answerDateTime" | "answerTime" => {
                crate::temporal::parse(a).zip(crate::temporal::parse(b)).map(|(x, y)| x.millis.cmp(&y.millis))
            }
            _ => Some(a.cmp(b)),
        },
        _ => None,
    };
    let Some(ord) = ord else { return op == "!=" };
    match op {
        "=" => ord == Ordering::Equal,
        "!=" => ord != Ordering::Equal,
        ">" => ord == Ordering::Greater,
        "<" => ord == Ordering::Less,
        ">=" => ord != Ordering::Less,
        "<=" => ord != Ordering::Greater,
        _ => false,
    }
}

/// Decimal places of a numeric literal, as the reference validator counts them.
fn precision(s: &str) -> i64 {
    match s.split_once('.') {
        Some((_, frac)) => frac.split(['e', 'E']).next().unwrap_or("").len() as i64,
        None => 0,
    }
}

/// `image/*` allows `image/gif`; `text/plain;charset=utf-8` is `text/plain`.
fn mime_matches(allowed: &str, actual: &str) -> bool {
    let actual = actual.split(';').next().unwrap_or(actual).trim();
    let allowed = allowed.split(';').next().unwrap_or(allowed).trim();
    if allowed.eq_ignore_ascii_case(actual) {
        return true;
    }
    match allowed.strip_suffix("/*") {
        Some(prefix) => actual.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix)),
        None => false,
    }
}

/// A Questionnaire's own `enableWhen` wiring: every condition names an
/// item that exists and is not the question itself.
pub(crate) fn check_questionnaire(q: &Value<'_>, label: &str, r: &mut Report) {
    fn walk(items: &[&Value<'_>], all: &Value<'_>, label: &str, r: &mut Report) {
        for (i, item) in items.iter().enumerate() {
            let at = format!("{label}.item[{i}]");
            let link_id = text(item, "linkId").unwrap_or("");
            for (j, ew) in items_of(item, "enableWhen").into_iter().enumerate() {
                let ewat = format!("{at}.enableWhen[{j}]");
                match text(ew, "question") {
                    None | Some("") => r.push(
                        Severity::Error,
                        IssueType::BusinessRule,
                        &ewat,
                        "Questions with an enableWhen must have a value for the question link".into(),
                    ),
                    Some(q) if q == link_id => r.push(
                        Severity::Error,
                        IssueType::BusinessRule,
                        &ewat,
                        "Target for this question enableWhen can't reference itself".into(),
                    ),
                    Some(q) => {
                        if find_item(all, q).is_none() {
                            r.push(
                                Severity::Error,
                                IssueType::BusinessRule,
                                &ewat,
                                format!("Unable to find an item with the linkId '{q}' which is referenced in the enableWhen for '{link_id}'"),
                            );
                        } else if find_item(item, q).is_some() {
                            r.push(
                                Severity::Error,
                                IssueType::BusinessRule,
                                &ewat,
                                "Questions with an enableWhen cannot refer to an inner question for its enableWhen condition".into(),
                            );
                        }
                    }
                }
            }
            walk(&items_of(item, "item"), all, &at, r);
        }
    }
    walk(&items_of(q, "item"), q, label, r);
}

fn find_item<'v, 'a>(node: &'v Value<'a>, link_id: &str) -> Option<&'v Value<'a>> {
    for item in items_of(node, "item") {
        if text(item, "linkId") == Some(link_id) {
            return Some(item);
        }
        if let Some(found) = find_item(item, link_id) {
            return Some(found);
        }
    }
    None
}
