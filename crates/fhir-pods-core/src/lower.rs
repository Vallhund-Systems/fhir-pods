//! Lowering — build a conformant resource from its *variable* data only.
//!
//! A compiled profile already fixes most of a resource (the codings, the
//! category, the UCUM system, the required slices). The host supplies the
//! slots the profile leaves open — `status`, `subject`, `effectiveDateTime`,
//! `valueQuantity.value` … — and gets back the full resource together with
//! its validation, so a lowered resource is never trusted unverified.
//!
//! Inputs are a JSON object keyed by slot path (see [`Slot`]): a labelled,
//! resource-relative path (`component:SystolicBP.valueQuantity.value`). A
//! value is coerced by the slot's type — a string into a `Reference`, a
//! number into a `Quantity`, `{system, code}` into a `CodeableConcept` — or
//! taken as given when it already has the type's JSON shape. Unknown keys,
//! wrong kinds and values that collide with a fixed value are issues in the
//! returned validation, never dropped silently.
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::compiled::{CompiledProfile, Max, Slot};
use crate::json::Value;
use crate::validate::{IssueType, Report, Severity};
use crate::wire::escape;
use crate::{Validation, Validator};

/// An owned JSON tree, built up then serialised.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
    Array(Vec<Node>),
    Object(Vec<(String, Node)>),
}

impl Node {
    fn from_value(v: &Value<'_>) -> Node {
        match v {
            Value::Null => Node::Null,
            Value::Bool(b) => Node::Bool(*b),
            Value::Number(n) => Node::Number((*n).into()),
            Value::Str(s) => Node::Str(s.to_string()),
            Value::Array(xs) => Node::Array(xs.iter().map(Node::from_value).collect()),
            Value::Object(kvs) => Node::Object(kvs.iter().map(|(k, v)| (k.to_string(), Node::from_value(v))).collect()),
        }
    }

    fn write(&self, out: &mut String) {
        match self {
            Node::Null => out.push_str("null"),
            Node::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Node::Number(n) => out.push_str(n),
            Node::Str(s) => {
                out.push('"');
                escape(out, s);
                out.push('"');
            }
            Node::Array(xs) => {
                out.push('[');
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    x.write(out);
                }
                out.push(']');
            }
            Node::Object(kvs) => {
                out.push('{');
                for (i, (k, v)) in kvs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    escape(out, k);
                    out.push_str("\":");
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

    fn member_mut(&mut self, key: &str) -> Option<&mut Node> {
        match self {
            Node::Object(kvs) => kvs.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn member_or_insert(&mut self, key: &str, make: Node) -> &mut Node {
        if let Node::Object(kvs) = self {
            if let Some(i) = kvs.iter().position(|(k, _)| k == key) {
                return &mut kvs[i].1;
            }
            kvs.push((key.into(), make));
            return &mut kvs.last_mut().unwrap().1;
        }
        // a scalar where an object was expected: replace it (an input collision is reported separately)
        *self = Node::Object(alloc::vec![(key.into(), make)]);
        self.member_mut(key).unwrap()
    }
}

/// The outcome: the resource as JSON text, and its validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lowered {
    pub resource: String,
    pub validation: Validation,
}

/// Why lowering could not even start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LowerError {
    /// No compiled profile with that canonical URL.
    UnknownProfile,
    /// The inputs are not a JSON object.
    InputsNotAnObject,
}

impl core::fmt::Display for LowerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            LowerError::UnknownProfile => "no compiled profile with that URL",
            LowerError::InputsNotAnObject => "inputs must be a JSON object",
        })
    }
}

impl core::error::Error for LowerError {}

/// A slot addressed by an input key: the resolved key and its type (for a
/// choice slot named by one of its concrete keys, that key's type).
struct Target<'p> {
    key: &'p str,
    ty: &'p str,
}

/// Members any resource may carry that no profile lists as a slot: taken as
/// given — the `id`, `contained` resources, narrative, extensions.
static PASS_THROUGH: &[Slot] = &[
    Slot { path: "id", ty: "id", choices: &[], min: 0, max: Max::Bounded(1), must_support: false },
    Slot { path: "contained", ty: "Resource", choices: &[], min: 0, max: Max::Star, must_support: false },
    Slot { path: "text", ty: "Narrative", choices: &[], min: 0, max: Max::Bounded(1), must_support: false },
    Slot { path: "extension", ty: "Extension", choices: &[], min: 0, max: Max::Star, must_support: false },
    Slot { path: "modifierExtension", ty: "Extension", choices: &[], min: 0, max: Max::Star, must_support: false },
    Slot { path: "language", ty: "code", choices: &[], min: 0, max: Max::Bounded(1), must_support: false },
    Slot { path: "implicitRules", ty: "uri", choices: &[], min: 0, max: Max::Bounded(1), must_support: false },
];

fn find_slot<'p>(profile: &'p CompiledProfile, key: &'p str) -> Option<Target<'p>> {
    // `_birthDate`: the primitive-extension form of a scalar slot, placed verbatim
    if let Some(bare) = key.rsplit_once('.').and_then(|(p, l)| l.strip_prefix('_').map(|l| (p, l))) {
        let bare_key = alloc::format!("{}.{}", bare.0, bare.1);
        return profile.lowering.slots.iter().find(|s| s.path == bare_key).map(|_| Target { key, ty: "Element" });
    }
    if let Some(bare) = key.strip_prefix('_') {
        return profile.lowering.slots.iter().find(|s| s.path == bare).map(|_| Target { key, ty: "Element" });
    }
    for slot in profile.lowering.slots.iter().chain(PASS_THROUGH) {
        if slot.path == key {
            return Some(Target { key, ty: slot.ty });
        }
        if slot.ty == "choice" {
            let (prefix, _) = slot.path.rsplit_once('.').unwrap_or(("", slot.path));
            for choice in slot.choices {
                let full = if prefix.is_empty() { choice.to_string() } else { alloc::format!("{prefix}.{choice}") };
                if full == key {
                    return Some(Target { key, ty: choice_type(slot.path, choice) });
                }
            }
        }
    }
    None
}

/// `value[x]` + `valueDateTime` → `dateTime`; `valueQuantity` → `Quantity`.
fn choice_type<'a>(slot_path: &str, choice: &'a str) -> &'a str {
    let stem = slot_path.rsplit('.').next().unwrap_or(slot_path).trim_end_matches("[x]");
    let rest = &choice[stem.len()..];
    // primitives are lowercase type names; complex types keep their case
    match rest {
        "String" => "string",
        "Boolean" => "boolean",
        "Integer" => "integer",
        "Decimal" => "decimal",
        "Date" => "date",
        "DateTime" => "dateTime",
        "Time" => "time",
        "Instant" => "instant",
        "Uri" => "uri",
        "Code" => "code",
        "Canonical" => "canonical",
        "Base64Binary" => "base64Binary",
        "Markdown" => "markdown",
        "Id" => "id",
        "Oid" => "oid",
        "PositiveInt" => "positiveInt",
        "UnsignedInt" => "unsignedInt",
        _ => rest,
    }
}

/// Is the element at labelled `path` an array in the JSON, per the base spec?
/// A profile may cut a repeating element to 0..1 (`Coverage.payor`) but the
/// wire shape stays an array.
fn base_repeats(resource_type: &'static str, path: &str) -> bool {
    let mut ctx: Option<&'static str> = Some(resource_type);
    let mut segs = path.split('.').peekable();
    while let Some(seg) = segs.next() {
        let name = unlabelled(seg);
        let Some(c) = ctx else { return false };
        if segs.peek().is_none() {
            return crate::typing::repeats(c, name);
        }
        ctx = crate::typing::child(c, name).map(|ch| ch.ctx);
    }
    false
}

fn unlabelled(seg: &str) -> &str {
    seg.split(':').next().unwrap_or(seg)
}

/// Walk / create the node at a labelled path. Repeating elements become
/// arrays; a labelled segment selects (or creates) the array element that
/// belongs to that slice, so every leaf of `category:VSCat` lands in the same
/// instance; an unlabelled repeating segment takes the first element.
fn place<'n>(
    root: &'n mut Node,
    path: &str,
    resource_type: &'static str,
    instances: &mut Vec<(String, usize)>,
) -> &'n mut Node {
    let mut node = root;
    // the labelled path so far: `component:systolic.code.coding` and
    // `component:diastolic.code.coding` are different arrays
    let mut prefix = String::new();
    let mut ctx: Option<&'static str> = Some(resource_type);
    for seg in path.split('.') {
        let name = unlabelled(seg);
        let label = seg.split_once(':').map(|(_, l)| l);
        if !prefix.is_empty() {
            prefix.push('.');
        }
        prefix.push_str(seg);
        // whether this element repeats, and the typing of its children, come
        // from the base spec's element table
        let repeats = ctx.is_some_and(|c| crate::typing::repeats(c, name));
        ctx = ctx.and_then(|c| crate::typing::child(c, name).map(|ch| ch.ctx));
        if repeats {
            let arr = node.member_or_insert(name, Node::Array(Vec::new()));
            if !matches!(arr, Node::Array(_)) {
                *arr = Node::Array(Vec::new());
            }
            let Node::Array(items) = arr else { unreachable!() };
            let instance_key = if label.is_some() { prefix.clone() } else { alloc::format!("{prefix}:") };
            let idx = match instances.iter().find(|(k, _)| *k == instance_key) {
                Some((_, i)) => *i,
                None => {
                    // an unlabelled slot under a repeating element reuses the first instance
                    let i = if label.is_none() && !items.is_empty() {
                        0
                    } else {
                        items.push(Node::Object(Vec::new()));
                        items.len() - 1
                    };
                    instances.push((instance_key, i));
                    i
                }
            };
            node = &mut items[idx];
        } else {
            node = node.member_or_insert(name, Node::Object(Vec::new()));
        }
    }
    node
}

/// Coerce an input for a slot of type `ty`; `None` when the kind is wrong.
fn coerce(ty: &str, v: &Value<'_>) -> Option<Node> {
    let is_primitive_str = |v: &Value<'_>| matches!(v, Value::Str(_));
    Some(match ty {
        "boolean" => match v {
            Value::Bool(_) => Node::from_value(v),
            _ => return None,
        },
        "integer" | "positiveInt" | "unsignedInt" | "decimal" => match v {
            Value::Number(_) => Node::from_value(v),
            _ => return None,
        },
        "date" | "dateTime" | "instant" | "time" => match v {
            Value::Str(s) if crate::temporal::parse(s).is_some() => Node::from_value(v),
            _ => return None,
        },
        "string" | "code" | "id" | "markdown" | "uri" | "url" | "canonical" | "oid" | "uuid" | "base64Binary" => {
            if is_primitive_str(v) {
                Node::from_value(v)
            } else {
                return None;
            }
        }
        "Reference" => match v {
            Value::Str(s) => Node::Object(alloc::vec![("reference".into(), Node::Str(s.to_string()))]),
            Value::Object(_) => Node::from_value(v),
            _ => return None,
        },
        "Quantity" | "SimpleQuantity" | "Age" | "Duration" | "Count" | "Distance" | "MoneyQuantity" => match v {
            Value::Number(_) => Node::Object(alloc::vec![("value".into(), Node::from_value(v))]),
            Value::Object(_) => Node::from_value(v),
            _ => return None,
        },
        "CodeableConcept" => match v {
            Value::Str(s) => Node::Object(alloc::vec![("text".into(), Node::Str(s.to_string()))]),
            Value::Object(kvs) => {
                // a coding shorthand {system, code[, display]} becomes one coding
                if kvs.iter().any(|(k, _)| k == "code") && !kvs.iter().any(|(k, _)| k == "coding" || k == "text") {
                    Node::Object(alloc::vec![("coding".into(), Node::Array(alloc::vec![Node::from_value(v)]))])
                } else {
                    Node::from_value(v)
                }
            }
            _ => return None,
        },
        "choice" => return None,
        // a primitive's extension carrier: an object, placed as given
        "Element" => match v {
            Value::Object(_) => Node::from_value(v),
            _ => return None,
        },
        _ => match v {
            Value::Object(_) | Value::Array(_) => Node::from_value(v),
            _ => return None,
        },
    })
}

/// Build the resource for `profile` from `inputs`, and validate it.
pub fn lower(
    profile: &CompiledProfile,
    inputs: &Value<'_>,
    validator: Option<&Validator>,
) -> Result<Lowered, LowerError> {
    let Value::Object(members) = inputs else { return Err(LowerError::InputsNotAnObject) };
    let l = profile.lowering;
    let mut issues = Report::default();
    let mut root = Node::Object(alloc::vec![
        ("resourceType".into(), Node::Str(profile.resource_type.into())),
        (
            "meta".into(),
            Node::Object(alloc::vec![("profile".into(), Node::Array(alloc::vec![Node::Str(profile.url.into())]))])
        ),
    ]);
    let mut instances: Vec<(String, usize)> = Vec::new();

    // Which slices are live: the required ones, and any optional one an input reaches into.
    let key_reaches = |slice: &str| {
        members
            .iter()
            .any(|(k, _)| k.as_ref() == slice || (k.starts_with(slice) && k.as_bytes().get(slice.len()) == Some(&b'.')))
    };
    let active = |path: &str| -> bool {
        let mut prefix = String::new();
        for seg in path.split('.') {
            if !prefix.is_empty() {
                prefix.push('.');
            }
            prefix.push_str(seg);
            if let Some((_, min)) = l.slices.iter().find(|(p, _)| *p == prefix) {
                if *min == 0 && !key_reaches(&prefix) {
                    return false;
                }
            }
        }
        true
    };

    // 1. the host's inputs
    for (key, value) in members {
        let Some(target) = find_slot(profile, key) else {
            issues.push(
                Severity::Error,
                IssueType::Structure,
                key,
                alloc::format!("'{key}' is not an input of {}", profile.name),
            );
            continue;
        };
        let repeats = base_repeats(profile.resource_type, target.key);
        let node = match (repeats, value) {
            (true, Value::Array(xs)) => {
                let mut items = Vec::new();
                for x in xs {
                    match coerce(target.ty, x) {
                        Some(n) => items.push(n),
                        None => {
                            issues.push(
                                Severity::Error,
                                IssueType::Value,
                                key,
                                alloc::format!("'{key}' expects {} values", target.ty),
                            );
                            continue;
                        }
                    }
                }
                Node::Array(items)
            }
            (true, v) => match coerce(target.ty, v) {
                Some(n) => Node::Array(alloc::vec![n]),
                None => {
                    issues.push(
                        Severity::Error,
                        IssueType::Value,
                        key,
                        alloc::format!("'{key}' expects a {}", target.ty),
                    );
                    continue;
                }
            },
            (false, Value::Array(xs)) if xs.len() == 1 => match coerce(target.ty, &xs[0]) {
                Some(n) => n,
                None => {
                    issues.push(
                        Severity::Error,
                        IssueType::Value,
                        key,
                        alloc::format!("'{key}' expects a {}", target.ty),
                    );
                    continue;
                }
            },
            (false, v) => match coerce(target.ty, v) {
                Some(n) => n,
                None => {
                    issues.push(
                        Severity::Error,
                        IssueType::Value,
                        key,
                        alloc::format!("'{key}' expects a {}", target.ty),
                    );
                    continue;
                }
            },
        };
        let (parent, leaf) = match target.key.rsplit_once('.') {
            Some((p, l)) => (Some(p), l),
            None => (None, target.key),
        };
        if leaf.contains(':') {
            // a slice given as a whole (`identifier:ihi`): merge into that
            // slice's instance, so the profile's fixed leaves join it
            let instance = place(&mut root, target.key, profile.resource_type, &mut instances);
            let given = match node {
                Node::Array(mut xs) if xs.len() == 1 => xs.remove(0),
                Node::Array(_) => {
                    issues.push(
                        Severity::Error,
                        IssueType::Structure,
                        key,
                        alloc::format!("'{key}' takes one object (a slice instance)"),
                    );
                    continue;
                }
                n => n,
            };
            match given {
                Node::Object(kvs) => {
                    for (k, v) in kvs {
                        let target = instance.member_or_insert(&k, Node::Null);
                        *target = v;
                    }
                }
                _ => issues.push(Severity::Error, IssueType::Value, key, alloc::format!("'{key}' expects an object")),
            }
            continue;
        }
        let holder = match parent {
            Some(p) => place(&mut root, p, profile.resource_type, &mut instances),
            None => &mut root,
        };
        let leaf_name = unlabelled(leaf);
        match (repeats, node) {
            // repeating slots accumulate: `identifier` and `identifier:ihi` can both contribute
            (true, Node::Array(items)) => {
                let arr = holder.member_or_insert(leaf_name, Node::Array(Vec::new()));
                match arr {
                    Node::Array(existing) => existing.extend(items),
                    other => *other = Node::Array(items),
                }
            }
            (_, node) => {
                let target = holder.member_or_insert(leaf_name, Node::Null);
                *target = node;
            }
        }
    }

    // 2. the profile's fixed leaves, where their slice is live; a host value
    //    that disagrees with a fixed value is an error, not overwritten
    for (path, value) in l.fixed {
        if !active(path) {
            continue;
        }
        let (parent, leaf) = path.rsplit_once('.').unwrap_or(("", path));
        let holder =
            if parent.is_empty() { &mut root } else { place(&mut root, parent, profile.resource_type, &mut instances) };
        let leaf_name = unlabelled(leaf);
        match holder.member_mut(leaf_name) {
            Some(Node::Str(existing)) if existing == value => {}
            Some(existing) => {
                let mut text = String::new();
                existing.write(&mut text);
                issues.push(
                    Severity::Error,
                    IssueType::Value,
                    path,
                    alloc::format!("'{path}' is fixed to '{value}' by the profile (input gave {text})"),
                );
            }
            None => {
                if let Node::Object(kvs) = holder {
                    kvs.push((leaf_name.into(), Node::Str((*value).into())));
                }
            }
        }
    }

    let mut resource = String::new();
    root.write(&mut resource);
    let mut validation = match validator {
        Some(v) => v.validate(resource.as_bytes()),
        None => crate::validate_bytes(resource.as_bytes()),
    };
    // input problems come first; the conformance findings follow
    let conformance = core::mem::take(&mut validation.report.issues);
    validation.report.issues = issues.issues;
    validation.report.issues.extend(conformance);
    Ok(Lowered { resource, validation })
}
