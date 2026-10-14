//! De-identification: a copy of a resource under a **policy** — a rule set
//! applied to a typed walk, so what a node *is* (`HumanName`, `Address`,
//! `date`…) decides, never its shape or its name alone. The built-in policy
//! ([`Policy::safe_harbor`], the default) is modelled on HIPAA Safe Harbor's
//! identifier list, by FHIR type: names, contact details, addresses below
//! state, dates to the year, ages over 89 on an `Age`, narrative, notes,
//! attachments, signatures and device serials go; identifiers and references
//! become pseudonyms keyed by a host-supplied salt, consistently.
//!
//! **No compliance standard is claimed.** The policy covers the fields its
//! rules name, in the resource it is given. It says nothing about
//! re-identification risk from what remains, from the host's other data, or
//! from how the copy is used; it is not an expert determination and not a
//! certification. [`Deidentified::remaining`] is the built-in policy's own
//! check over the copy, not a verdict on either. The salt is required (see
//! [`PolicyError::MissingSalt`]): without one the operation refuses rather
//! than emit unkeyed pseudonyms. An age the pod cannot recognise — an
//! integer a host uses for one — collapses only where a rule names the
//! element ([`Action::CollapseAge`]).
//!
//! Where the resource's profile requires an element that is removed, a
//! `data-absent-reason: masked` stub stands in, so the copy stays conformant
//! wherever the profile permits.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::document::Document;
use crate::document::Parsed;
use crate::json::Value;
use crate::node::Node;
use crate::typing;
use crate::{Validation, Validator};

/// What a rule does to the element it targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Keep,
    /// Drop the element — or, where the profile requires it, [`Action::Mask`].
    Remove,
    /// Replace with a `data-absent-reason: masked` stub.
    Mask,
    /// A date keeps its year only; an `instant` becomes the first instant
    /// of its year (`YYYY-01-01T00:00:00Z`) — the same information.
    Year,
    /// A string is replaced by a keyed hash of itself (references keep
    /// their type; contained references keep their `#`).
    Pseudonym,
    /// An age over 89 reads as 90 (what the walk does to an `Age`; not a
    /// rule target).
    Clamp,
    /// The host declares this element an age and asks for Safe Harbor's
    /// aggregate: over 89 collapses. The pod cannot find ages by itself —
    /// nothing says which integer is one — so this is a rule target and
    /// only a rule target. An integer or decimal becomes `90`; an `Age`,
    /// `Duration`, `Count` or `Distance` says it exactly, `{"value": 90,
    /// "comparator": ">="}` in its own unit (probed); a plain `Quantity`
    /// takes the bare value, because many `Quantity` elements are
    /// constrained to `SimpleQuantity`, which forbids a comparator, and the
    /// element-type table cannot tell them apart
    /// ([`comparator_allowed`]). Anything else is left alone and named in
    /// `residual`, so a rule that cannot bite is never silent.
    CollapseAge,
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Action::Keep => "keep",
            Action::Remove => "remove",
            Action::Mask => "mask",
            Action::Year => "year",
            Action::Pseudonym => "pseudonym",
            Action::Clamp => "clamp",
            Action::CollapseAge => "collapse-age",
        }
    }

    pub fn parse(s: &str) -> Option<Action> {
        Some(match s {
            "keep" => Action::Keep,
            "remove" => Action::Remove,
            "mask" => Action::Mask,
            "year" => Action::Year,
            "pseudonym" => Action::Pseudonym,
            "clamp" => Action::Clamp,
            "collapse-age" => Action::CollapseAge,
            _ => return None,
        })
    }
}

/// A rule targets a FHIR type (`HumanName`), a member of a type
/// (`Address.city`) or an element path in a resource (`Patient.birthDate`).
/// The most specific target wins; among equals, the last rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub target: String,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Keys the pseudonyms. Keep it secret and stable: the same salt maps
    /// the same identifier to the same pseudonym across resources.
    pub salt: Vec<u8>,
    /// The current year, when known — what "over 89" is measured against.
    pub today_year: Option<i64>,
    pub rules: Vec<Rule>,
}

impl Policy {
    /// No rules: everything is kept.
    pub fn empty(salt: &[u8]) -> Self {
        Policy { salt: salt.to_vec(), today_year: None, rules: Vec::new() }
    }

    /// The built-in policy: rules modelled on HIPAA Safe Harbor's identifier
    /// list, by FHIR type. A rule set, not a compliance determination (see
    /// the module docs).
    pub fn safe_harbor(salt: &[u8]) -> Self {
        let mut p = Policy::empty(salt);
        for (target, action) in [
            ("HumanName", Action::Remove),
            ("ContactPoint", Action::Remove),
            ("Address.line", Action::Remove),
            ("Address.city", Action::Remove),
            ("Address.district", Action::Remove),
            ("Address.postalCode", Action::Remove),
            ("Address.text", Action::Remove),
            ("Address.period", Action::Remove),
            ("Identifier.value", Action::Pseudonym),
            ("Identifier.period", Action::Remove),
            ("Reference.reference", Action::Pseudonym),
            ("Reference.display", Action::Remove),
            ("Narrative", Action::Remove),
            ("Annotation", Action::Remove),
            ("Attachment", Action::Remove),
            ("Signature", Action::Remove),
            ("date", Action::Year),
            ("dateTime", Action::Year),
            ("instant", Action::Year),
            ("Meta.lastUpdated", Action::Remove),
            ("Bundle.entry.fullUrl", Action::Pseudonym),
            ("id", Action::Pseudonym),
            ("Meta.versionId", Action::Remove),
            ("Device.serialNumber", Action::Remove),
            ("Device.udiCarrier", Action::Remove),
            // free text goes — a concept's text, a note, an extension's
            // payload, an organisation's name — except the few strings that
            // are not free text: a code's display, a unit, a state, a country
            ("string", Action::Remove),
            ("markdown", Action::Remove),
            ("Coding.display", Action::Keep),
            ("Coding.version", Action::Keep),
            ("Quantity.unit", Action::Keep),
            ("SimpleQuantity.unit", Action::Keep),
            ("Age.unit", Action::Keep),
            ("Duration.unit", Action::Keep),
            ("Count.unit", Action::Keep),
            ("Distance.unit", Action::Keep),
            ("MoneyQuantity.unit", Action::Keep),
            ("Address.state", Action::Keep),
            ("Address.country", Action::Keep),
        ] {
            p = p.rule(target, action);
        }
        p
    }

    pub fn rule(mut self, target: &str, action: Action) -> Self {
        self.rules.push(Rule { target: target.into(), action });
        self
    }

    pub fn today(mut self, year: i64) -> Self {
        self.today_year = Some(year);
        self
    }

    /// From JSON: `{"salt": "…", "today": "2026-09-16", "base": "safe-harbor"
    /// | "none", "rules": [{"target": "Patient.address.city", "action": "keep"}]}`.
    /// Rules apply after the base's, so they override it.
    pub fn from_json(bytes: &[u8]) -> Result<Policy, PolicyError> {
        let v = crate::json::parse(bytes).map_err(|_| PolicyError::NotAnObject)?;
        let Value::Object(members) = &v else { return Err(PolicyError::NotAnObject) };
        let member = |k: &str| members.iter().find(|(m, _)| m.as_ref() == k).map(|(_, v)| v);
        let salt = match member("salt") {
            Some(Value::Str(s)) if !s.is_empty() => s.as_bytes().to_vec(),
            _ => return Err(PolicyError::MissingSalt),
        };
        let mut p = match member("base") {
            None => Policy::safe_harbor(&salt),
            Some(Value::Str(b)) if b.as_ref() == "safe-harbor" => Policy::safe_harbor(&salt),
            Some(Value::Str(b)) if b.as_ref() == "none" => Policy::empty(&salt),
            _ => return Err(PolicyError::UnknownBase),
        };
        if let Some(t) = member("today") {
            let Value::Str(t) = t else { return Err(PolicyError::BadToday) };
            let year = crate::temporal::parse(t).filter(|t| t.has_date).and_then(|_| t[..4].parse::<i64>().ok());
            p.today_year = Some(year.ok_or(PolicyError::BadToday)?);
        }
        if let Some(rules) = member("rules") {
            let Value::Array(rules) = rules else { return Err(PolicyError::BadRule(0)) };
            for (i, r) in rules.iter().enumerate() {
                let Value::Object(kv) = r else { return Err(PolicyError::BadRule(i)) };
                let get = |k: &str| kv.iter().find(|(m, _)| m.as_ref() == k).map(|(_, v)| v);
                let (Some(Value::Str(target)), Some(Value::Str(action))) = (get("target"), get("action")) else {
                    return Err(PolicyError::BadRule(i));
                };
                let action = Action::parse(action).ok_or(PolicyError::BadRule(i))?;
                p = p.rule(target, action);
            }
        }
        Ok(p)
    }

    /// The action for a node: by its element path in the resource, then its
    /// member-of-type name, then its type — the most specific rule wins.
    fn action_for(&self, path: &str, member: Option<&str>, ty: Option<&str>) -> Option<Action> {
        let find = |target: &str| self.rules.iter().rev().find(|r| r.target == target).map(|r| r.action);
        find(path).or_else(|| member.and_then(find)).or_else(|| ty.and_then(find))
    }

    /// A rule naming this element itself — by its path or its member-of-type
    /// name — and not merely its type: what overrides a structural default.
    fn explicit_action(&self, path: &str, member: Option<&str>) -> Option<Action> {
        let find = |target: &str| self.rules.iter().rev().find(|r| r.target == target).map(|r| r.action);
        find(path).or_else(|| member.and_then(find))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyError {
    NotAnObject,
    MissingSalt,
    UnknownBase,
    BadToday,
    /// The rule at this index lacks a `target` / `action`, or the action is unknown.
    BadRule(usize),
}

impl core::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PolicyError::NotAnObject => f.write_str("policy must be a JSON object"),
            PolicyError::MissingSalt => f.write_str("policy needs a non-empty salt"),
            PolicyError::UnknownBase => f.write_str("policy base must be safe-harbor or none"),
            PolicyError::BadToday => f.write_str("policy today must be a date"),
            PolicyError::BadRule(i) => write!(f, "policy rule {i} needs a target and a known action"),
        }
    }
}

impl core::error::Error for PolicyError {}

/// Why de-identification could not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeidError {
    /// Not a JSON object with a string `resourceType`.
    NotAResource,
    /// A resource type the base spec does not define: nothing can be typed,
    /// so nothing is guessed.
    UnknownResourceType,
}

impl core::fmt::Display for DeidError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            DeidError::NotAResource => "input is not a FHIR resource",
            DeidError::UnknownResourceType => "resourceType is not one the base spec defines",
        })
    }
}

impl core::error::Error for DeidError {}

/// One thing the walk did, for the host's audit trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub path: String,
    pub action: Action,
}

/// Something identifying that survived (see [`check`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub what: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deidentified {
    pub resource: String,
    pub applied: Vec<Applied>,
    /// The copy's conformance to the profiles it declares.
    pub validation: Validation,
    /// What [`check`] still finds in the copy — empty under the built-in
    /// policy. The policy's own check, never a compliance verdict.
    pub remaining: Vec<Finding>,
    /// Free text a policy rule kept in place — a `CodeableConcept.text` the
    /// host chose to keep, a `Dosage.text` — named so an empty `remaining`
    /// never reads as "all free text gone" — and a [`Action::CollapseAge`]
    /// rule that found no age to collapse. Empty under the built-in policy
    /// alone; the host judges these, or drops the rule.
    pub residual: Vec<Finding>,
}

const DAR: &str = "http://hl7.org/fhir/StructureDefinition/data-absent-reason";

const MASKED_DIV: &str = "<div xmlns=\"http://www.w3.org/1999/xhtml\">Masked</div>";

/// What stands in for a required element that went: a `data-absent-reason:
/// masked` extension — or, for the datatypes whose own elements are
/// required, the smallest valid value that says so (a `Narrative` needs a
/// status and a div; an `Annotation` its text).
fn masked_stub_for(ty: &str) -> Node {
    match ty {
        "Narrative" => Node::Object(alloc::vec![
            ("status".into(), Node::Str("generated".into())),
            ("div".into(), Node::Str(MASKED_DIV.into())),
        ]),
        "Annotation" => Node::Object(alloc::vec![("text".into(), Node::Str("Masked".into()))]),
        _ => masked_stub(),
    }
}

fn masked_stub() -> Node {
    Node::Object(alloc::vec![(
        "extension".into(),
        Node::Array(alloc::vec![Node::Object(alloc::vec![
            ("url".into(), Node::Str(DAR.into())),
            ("valueCode".into(), Node::Str("masked".into())),
        ])])
    )])
}

struct Walker<'p> {
    policy: &'p Policy,
    /// Element paths the resource's profiles require (`min >= 1`).
    required: Vec<String>,
    /// What the base resource itself requires of its root (`Observation.code`),
    /// profile or no profile: an element that goes is masked, never dropped.
    base_required: Vec<&'static str>,
    applied: Vec<Applied>,
    residual: Vec<Finding>,
}

impl Walker<'_> {
    /// Must this element stay (masked) when its content goes? By the
    /// profiles applied, or — at a resource's root — by the base resource.
    fn is_required(&self, child_path: &str, path: &str, key: &str) -> bool {
        self.required.iter().any(|r| r == child_path)
            || (!path.contains('.') && self.base_required.contains(&key))
            || (path.contains('.') && typing::required_backbone_children(path).contains(&key))
    }

    /// The direct children the profiles require under a masked element
    /// (`Practitioner.name` → `family`): each gets its own mask, so the stub
    /// is as conformant as a masked element can be.
    fn required_children(&self, child_path: &str) -> Vec<String> {
        let prefix = alloc::format!("{child_path}.");
        let mut out: Vec<String> = self
            .required
            .iter()
            .filter_map(|r| r.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('.'))
            .map(String::from)
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// A pseudonym with the shape of the value: every digit becomes a digit,
    /// every ASCII letter a letter of the same case, everything else stays —
    /// so a 10-digit NPI stays 10 digits, a URI keeps its scheme and
    /// punctuation, a `maxLength` still holds. The characters come from
    /// HMAC-SHA256 over the salt, the domain and the value (a counter extends
    /// the stream for long values). A value whose digits pass the Luhn check
    /// (an IHI, an NPI) gets a pseudonym that passes it too.
    fn pseudonym(&self, domain: &str, value: &str) -> String {
        let mut msg = String::from(domain);
        msg.push(':');
        msg.push_str(value);
        let mut out = String::with_capacity(value.len());
        let mut stream: Vec<u8> = Vec::new();
        let mut counter = 0u32;
        let mut next = |stream: &mut Vec<u8>| -> u8 {
            if stream.is_empty() {
                let mut m = msg.clone();
                m.push(':');
                m.push_str(&counter.to_string());
                counter += 1;
                let mut block = crate::hash::hmac_sha256(&self.policy.salt, m.as_bytes()).to_vec();
                block.reverse();
                *stream = block;
            }
            stream.pop().unwrap_or(0)
        };
        // a hex token (a UUID, a hash) stays hex
        let hex =
            value.chars().all(|c| c.is_ascii_hexdigit() || c == '-') && value.chars().any(|c| c.is_ascii_alphabetic());
        let letters = if hex { 6 } else { 26 };
        for c in value.chars() {
            out.push(match c {
                '0'..='9' => (b'0' + next(&mut stream) % 10) as char,
                'a'..='z' => (b'a' + next(&mut stream) % letters) as char,
                'A'..='Z' => (b'A' + next(&mut stream) % letters) as char,
                // anything beyond ASCII letters and digits is content, not shape: it goes too
                c if !c.is_ascii() => (b'a' + next(&mut stream) % 26) as char,
                c => c,
            });
        }
        luhn::preserve(value, &mut out);
        out
    }

    /// The one pseudonym an id gets wherever it appears — as a resource's
    /// `id`, in `Type/id`, `#id`, `urn:uuid:id` or a Bundle entry's `fullUrl`.
    fn pseudonym_id(&self, id: &str) -> String {
        self.pseudonym("id", id)
    }

    /// `Patient/123` → `Patient/<h>`; `#med` → `#<h>`; `urn:uuid:x` →
    /// `urn:uuid:<h>`; an absolute URL keeps its base; anything else is
    /// hashed whole.
    fn pseudonym_reference(&self, r: &str) -> String {
        if let Some(local) = r.strip_prefix('#') {
            let mut s = String::from("#");
            s.push_str(&self.pseudonym_id(local));
            return s;
        }
        if let Some(uuid) = r.strip_prefix("urn:uuid:") {
            let mut s = String::from("urn:uuid:");
            s.push_str(&self.pseudonym_id(uuid));
            return s;
        }
        let (r, _history) = r.split_once("/_history/").unwrap_or((r, ""));
        let mut segs: Vec<&str> = r.rsplitn(3, '/').collect();
        segs.reverse();
        // […/Type/id] — the type is the segment before the id
        if segs.len() >= 2 {
            let id = segs[segs.len() - 1];
            let ty = segs[segs.len() - 2];
            if typing::known_type(ty).is_some() {
                let base = &r[..r.len() - id.len()];
                let mut s = String::from(base);
                s.push_str(&self.pseudonym_id(id));
                return s;
            }
        }
        self.pseudonym("reference", r)
    }

    fn note(&mut self, path: &str, action: Action) {
        self.applied.push(Applied { path: path.into(), action });
    }

    /// Rewrite the members of an object whose typing context is `ctx`
    /// (a type name or a backbone path) at element path `path`.
    fn walk_object(&mut self, kvs: &mut Vec<(String, Node)>, ctx: &'static str, path: &str) {
        let is_type = !ctx.contains('.');
        let mut out: Vec<(String, Node)> = Vec::with_capacity(kvs.len());
        let mut dropped: Vec<String> = Vec::new(); // keys whose `_key` carrier goes too
        let mut masked: Vec<String> = Vec::new();
        // an Identifier's system, read from the original object whatever the member order
        let identifier_system: Option<String> = if ctx == "Identifier" {
            kvs.iter().find(|(k, _)| k == "system").and_then(|(_, v)| v.as_str().map(String::from))
        } else {
            None
        };
        let taken = core::mem::take(kvs);
        // carriers are handled with their primitive: pull them out first
        let mut carriers: Vec<(String, Node)> = Vec::new();
        let mut members: Vec<(String, Node)> = Vec::new();
        for (k, v) in taken {
            if k.starts_with('_') && k.len() > 1 {
                carriers.push((k, v));
            } else {
                members.push((k, v));
            }
        }
        for (key, mut value) in members {
            if key == "resourceType" {
                out.push((key, value));
                continue;
            }
            let child_path = alloc::format!("{path}.{key}");
            let child = typing::child(ctx, &key);
            let Some(child) = child else {
                // an element the base spec does not define: it cannot be typed, so it goes
                self.note(&child_path, Action::Remove);
                dropped.push(key);
                continue;
            };
            let member = if is_type { Some(alloc::format!("{ctx}.{key}")) } else { None };
            let ty = child.ty;
            // an element's own `id`, or a `linkId`, is a structural key, not
            // content: it stays unless a rule names it (a resource's `id` is
            // typed `id` and has its own rule); the type-level `string` rule
            // does not reach it
            let structural = (key == "id" || key == "linkId") && ty == "string";
            let action = match self.policy.explicit_action(&child_path, member.as_deref()) {
                Some(a) => a,
                None if structural => Action::Keep,
                None => self.policy.action_for(&child_path, member.as_deref(), Some(ty)).unwrap_or(Action::Keep),
            };
            let action = match action {
                Action::Remove if self.is_required(&child_path, path, &key) => Action::Mask,
                // a QuestionnaireResponse answer's value is masked, never
                // dropped: an answer with no value is no answer, and a
                // required item then has none — the reference validator
                // passes the value-less answer with its carrier (probed)
                Action::Remove if ctx == "QuestionnaireResponse.item.answer" && key.starts_with("value") => {
                    Action::Mask
                }
                // clamping is what the walk does to an Age on the way through; as a rule it means keep
                Action::Clamp => Action::Keep,
                a => a,
            };
            match action {
                // the host named this element an age: de-identify what is
                // inside it as a kept element, then collapse the value
                Action::CollapseAge => {
                    self.keep(&mut value, child, &child_path);
                    let (mut collapsed, mut unreadable) = (false, false);
                    let mut apply = |n: &mut Node| match collapse_age(n, ty) {
                        Some(true) => collapsed = true,
                        Some(false) => {} // an age under the threshold: as given
                        None => unreadable = true,
                    };
                    match &mut value {
                        Node::Array(xs) => xs.iter_mut().for_each(&mut apply),
                        v => apply(v),
                    }
                    if collapsed {
                        // the walk's own `Age` clamp, where it ran, is
                        // subsumed by the collapse at this very path
                        self.applied.retain(|a| !(a.path == child_path && a.action == Action::Clamp));
                        self.note(&child_path, Action::CollapseAge);
                    }
                    if unreadable {
                        // the rule named something no age can be read from:
                        // said, never silently skipped
                        self.residual.push(Finding {
                            path: child_path.clone(),
                            what: "age not collapsed: not an integer, a decimal, or a quantity in a unit of time"
                                .into(),
                        });
                    }
                    out.push((key, value));
                }
                Action::Keep | Action::Clamp => {
                    if is_free_text(ctx, &key, ty) {
                        self.residual.push(Finding { path: child_path.clone(), what: "free text kept".into() });
                    }
                    self.keep(&mut value, child, &child_path);
                    // an element emptied by what was removed inside it goes too
                    let was_array = matches!(value, Node::Array(_));
                    if prune_empty(&mut value) {
                        if self.is_required(&child_path, path, &key) {
                            // masked exactly as if the rule had named it: the
                            // same stub, in an array where the element repeats
                            self.note(&child_path, Action::Mask);
                            out.push((key, self.masked_value(ty, child.ctx, &child_path, was_array)));
                        } else {
                            self.note(&child_path, Action::Remove);
                            dropped.push(key);
                        }
                    } else {
                        out.push((key, value));
                    }
                }
                Action::Remove => {
                    self.note(&child_path, Action::Remove);
                    dropped.push(key);
                }
                Action::Mask => {
                    self.note(&child_path, Action::Mask);
                    let stub = self.masked_value(ty, child.ctx, &child_path, matches!(value, Node::Array(_)));
                    if is_primitive_type(ty) {
                        // a primitive is masked through its carrier: no value, the extension on `_key`
                        masked.push(key.clone());
                        out.push((alloc::format!("_{key}"), stub));
                    } else {
                        out.push((key, stub));
                    }
                }
                Action::Year => {
                    let mut all_ok = true;
                    let instant = ty == "instant";
                    let mut apply = |n: &mut Node| match n {
                        Node::Str(s) => match year_of(s) {
                            Some(y) if !self.over_89(&child_path, y) => {
                                *s = if instant { alloc::format!("{y:04}-01-01T00:00:00Z") } else { y.to_string() }
                            }
                            _ => all_ok = false,
                        },
                        _ => all_ok = false,
                    };
                    match &mut value {
                        Node::Array(xs) => xs.iter_mut().for_each(&mut apply),
                        v => apply(v),
                    }
                    if all_ok {
                        self.note(&child_path, Action::Year);
                        out.push((key, value));
                    } else if self.is_required(&child_path, path, &key) {
                        self.note(&child_path, Action::Mask);
                        masked.push(key.clone());
                        out.push((alloc::format!("_{key}"), masked_stub()));
                    } else {
                        self.note(&child_path, Action::Remove);
                        dropped.push(key);
                    }
                }
                Action::Pseudonym => {
                    let domain = self.pseudonym_domain(ctx, &key, path, identifier_system.as_deref());
                    let mut apply = |n: &mut Node| {
                        if let Node::Str(s) = n {
                            *s = match &domain {
                                None => self.pseudonym_reference(s),
                                Some(d) => self.pseudonym(d, s),
                            };
                        }
                    };
                    match &mut value {
                        Node::Array(xs) => xs.iter_mut().for_each(&mut apply),
                        v => apply(v),
                    }
                    self.note(&child_path, Action::Pseudonym);
                    out.push((key, value));
                }
            }
        }
        // the carriers: dropped with their primitive, replaced when masked, otherwise walked as elements
        for (ck, mut cv) in carriers {
            let bare = &ck[1..];
            if dropped.iter().any(|d| d == bare) || masked.iter().any(|m| m == bare) {
                continue;
            }
            let child_path = alloc::format!("{path}.{bare}");
            self.walk_carrier(&mut cv, &child_path);
            // a carrier emptied by what went from it (its only extension) goes too
            let emptied = match &mut cv {
                Node::Object(m) => {
                    m.retain(|(_, v)| !matches!(v, Node::Array(xs) if xs.is_empty()));
                    m.is_empty()
                }
                Node::Array(xs) => {
                    for x in xs.iter_mut() {
                        if let Node::Object(m) = x {
                            m.retain(|(_, v)| !matches!(v, Node::Array(ys) if ys.is_empty()));
                            if m.is_empty() {
                                *x = Node::Null;
                            }
                        }
                    }
                    xs.iter().all(|x| matches!(x, Node::Null))
                }
                _ => false,
            };
            if emptied {
                continue;
            }
            out.push((ck, cv));
        }
        *kvs = out;
    }

    /// Which pseudonym domain a string gets, so the same thing maps to the
    /// same pseudonym wherever it appears. `None` = it is a reference.
    fn pseudonym_domain(&self, ctx: &str, key: &str, path: &str, identifier_system: Option<&str>) -> Option<String> {
        if (ctx == "Reference" && key == "reference") || (ctx == "Bundle.entry" && key == "fullUrl") {
            return None;
        }
        if key == "id" && !path.contains('.') {
            // a resource's own id: the domain every reference to it uses
            return Some("id".into());
        }
        if ctx == "Identifier" && key == "value" {
            return Some(alloc::format!("identifier:{}", identifier_system.unwrap_or("")));
        }
        Some(alloc::format!("text:{ctx}.{key}"))
    }

    fn over_89(&self, path: &str, year: i64) -> bool {
        // only a birth date measures an age
        path.ends_with(".birthDate") && self.policy.today_year.is_some_and(|t| t - year > 89)
    }

    /// What stands in for a required element of type `ty` that went — the
    /// masked stub, with what the profiles require inside it masked too —
    /// in an array where the element repeated.
    fn masked_value(&self, ty: &str, ctx: &'static str, child_path: &str, was_array: bool) -> Node {
        let mut one = masked_stub_for(ty);
        if let Node::Object(stub) = &mut one {
            for name in self.required_children(child_path) {
                if stub.iter().any(|(k, _)| *k == name || *k == alloc::format!("_{name}")) {
                    continue;
                }
                match typing::child(ctx, &name) {
                    Some(c) if is_primitive_type(c.ty) => stub.push((alloc::format!("_{name}"), masked_stub())),
                    Some(c) => stub.push((name.clone(), masked_stub_for(c.ty))),
                    None => {}
                }
            }
        }
        if was_array {
            Node::Array(alloc::vec![one])
        } else {
            one
        }
    }

    /// Recurse into a kept element by its typing.
    fn keep(&mut self, value: &mut Node, child: typing::Child, path: &str) {
        match value {
            Node::Array(xs) => {
                for x in xs {
                    self.keep_one(x, child, path);
                }
            }
            v => self.keep_one(v, child, path),
        }
    }

    fn keep_one(&mut self, value: &mut Node, child: typing::Child, path: &str) {
        let Node::Object(kvs) = value else { return };
        if child.ty == "Resource" {
            // a contained resource, or a Bundle entry's: typed by its own
            // resourceType, in its own path space
            let rt = kvs.iter().find(|(k, _)| k == "resourceType").and_then(|(_, v)| v.as_str());
            match rt.and_then(typing::known_type) {
                Some(rt) if crate::typemap::RESOURCES.binary_search(&rt).is_ok() => {
                    self.walk_object(kvs, rt, rt);
                }
                _ => {
                    self.note(path, Action::Remove);
                    *value = masked_stub();
                }
            }
            return;
        }
        self.walk_object(kvs, child.ctx, path);
        if child.ty == "Age" && clamp_age(kvs) {
            self.note(path, Action::Clamp);
        }
        // an extension whose value (or every nested extension) went is a
        // URL and nothing else — not an extension any more (ext-1): it goes
        // with its value
        if child.ty == "Extension" && kvs.iter().all(|(k, _)| k == "url" || k == "id") {
            kvs.clear();
        }
        // an extension whose compiled profile requires a sub-extension that
        // went (a translation without its content) is not that extension any
        // more: it goes whole
        if child.ty == "Extension" && !kvs.is_empty() {
            let url = kvs.iter().find(|(k, _)| k == "url").and_then(|(_, v)| v.as_str()).unwrap_or("");
            if let Some(p) = crate::REGISTRY.iter().find(|p| p.resource_type == "Extension" && p.url == url) {
                let has_sub = |sub_url: &str| match kvs.iter().find(|(k, _)| k == "extension") {
                    Some((_, Node::Array(xs))) => {
                        xs.iter().any(|x| x.member("url").and_then(Node::as_str) == Some(sub_url))
                    }
                    _ => false,
                };
                let missing = p.slicings.iter().filter(|sl| sl.path == "Extension.extension").any(|sl| {
                    sl.slices.iter().filter(|s| s.min >= 1).any(|s| {
                        s.discriminators.iter().any(|d| {
                            matches!(d, crate::compiled::Discriminator::Value { path: "$this.url", value } if !has_sub(value))
                        })
                    })
                });
                if missing {
                    self.note(path, Action::Remove);
                    kvs.clear();
                }
            }
        }
    }

    /// A primitive's `_name` carrier: an Element whose extensions are walked.
    fn walk_carrier(&mut self, carrier: &mut Node, path: &str) {
        match carrier {
            Node::Array(xs) => {
                for x in xs {
                    if let Node::Object(kvs) = x {
                        self.walk_object(kvs, "Element", path);
                    }
                }
            }
            Node::Object(kvs) => self.walk_object(kvs, "Element", path),
            _ => {}
        }
    }
}

/// A kept string the policy did not judge: free text by any name except the
/// few strings that are not (a code's display, a unit, a state or country).
fn is_free_text(ctx: &str, key: &str, ty: &str) -> bool {
    if !matches!(ty, "string" | "markdown") || key == "id" || key == "linkId" {
        return false;
    }
    !matches!(
        (ctx, key),
        ("Coding", "display" | "version")
            | ("Quantity" | "SimpleQuantity" | "Age" | "Duration" | "Count" | "Distance" | "MoneyQuantity", "unit")
            | ("Address", "state" | "country")
            | ("Meta", _)
    )
}

/// Drop empty objects from an array; `true` when nothing is left of the value.
fn prune_empty(value: &mut Node) -> bool {
    match value {
        Node::Object(kvs) => kvs.is_empty(),
        Node::Array(xs) => {
            xs.retain(|x| !matches!(x, Node::Object(kvs) if kvs.is_empty()));
            xs.is_empty()
        }
        _ => false,
    }
}

/// The Luhn check digit (IHI, Medicare provider, NPI — the latter over
/// `80840` + the number). A pseudonym of a value that passes keeps passing.
mod luhn {
    use alloc::string::{String, ToString};

    fn digits(s: &str) -> Option<impl Iterator<Item = u32> + '_> {
        let clean = s.chars().filter(|c| !matches!(c, ' ' | '-'));
        if clean.clone().count() < 2 || !clean.clone().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(clean.map(|c| c as u32 - '0' as u32))
    }

    fn sum_from_right(ds: impl Iterator<Item = u32>, skip_last: bool) -> u32 {
        let v: alloc::vec::Vec<u32> = ds.collect();
        let end = if skip_last { v.len() - 1 } else { v.len() };
        v[..end]
            .iter()
            .rev()
            .enumerate()
            .map(|(i, &d)| {
                // the digit next to the check digit is doubled
                let doubled = if skip_last { i % 2 == 0 } else { i % 2 == 1 };
                if doubled {
                    let x = d * 2;
                    if x > 9 {
                        x - 9
                    } else {
                        x
                    }
                } else {
                    d
                }
            })
            .sum()
    }

    fn passes(s: &str, prefix: &str) -> Option<bool> {
        let mut full = String::from(prefix);
        full.push_str(s);
        let ds = digits(&full)?;
        Some(sum_from_right(ds, false) % 10 == 0)
    }

    fn fix(out: &mut String, prefix: &str) {
        let mut full = String::from(prefix);
        full.push_str(out);
        let Some(ds) = digits(&full) else { return };
        let check = (10 - sum_from_right(ds, true) % 10) % 10;
        if let Some(i) = out.rfind(|c: char| c.is_ascii_digit()) {
            out.replace_range(i..i + 1, &check.to_string());
        }
    }

    /// If `original` passes the Luhn check (plain, or as an NPI), make `out` pass it too.
    pub fn preserve(original: &str, out: &mut String) {
        for prefix in ["", "80840"] {
            if prefix == "80840" && original.chars().filter(|c| c.is_ascii_digit()).count() != 10 {
                continue;
            }
            if passes(original, prefix) == Some(true) {
                fix(out, prefix);
                return;
            }
        }
    }
}

/// Years per unit of an `Age` quantity (UCUM code or a plain unit word);
/// `None` for a unit the policy does not interpret.
fn years_per(unit: &str) -> Option<f64> {
    Some(match unit {
        "a" | "yr" | "year" | "years" => 1.0,
        "mo" | "month" | "months" => 1.0 / 12.0,
        "wk" | "week" | "weeks" => 7.0 / 365.25,
        "d" | "day" | "days" => 1.0 / 365.25,
        _ => return None,
    })
}

/// The years an `Age` quantity states, when its unit is one the policy reads.
fn age_years(kvs: &[(String, Node)]) -> Option<f64> {
    let value = kvs.iter().find(|(k, _)| k == "value").and_then(|(_, v)| match v {
        Node::Number(n) => n.parse::<f64>().ok(),
        _ => None,
    })?;
    let unit = kvs
        .iter()
        .find(|(k, _)| k == "code")
        .or_else(|| kvs.iter().find(|(k, _)| k == "unit"))
        .and_then(|(_, v)| v.as_str())?;
    Some(value * years_per(unit)?)
}

/// The host named an element an age (`Action::CollapseAge`): over 89 reads
/// as Safe Harbor's "90 or above". An integer or decimal becomes `90` — a
/// primitive carries no comparator, and the host that named the element
/// knows what 90 now means there; a quantity whose type always bears a
/// comparator says it exactly, `comparator: ">="` with the 90-equivalent in
/// its own unit (see [`comparator_allowed`] for which types, and why a plain
/// `Quantity` is not one of them). `Some(false)` = an age at or under 89,
/// left as given; `None` = no age can be read from this node at all.
fn collapse_age(node: &mut Node, ty: &str) -> Option<bool> {
    if let Node::Number(n) = node {
        if n.parse::<f64>().ok()? <= 89.0 {
            return Some(false);
        }
        *node = Node::Number("90".to_string());
        return Some(true);
    }
    let Node::Object(kvs) = node else { return None };
    if !matches!(ty, "Quantity" | "SimpleQuantity" | "Age" | "Duration" | "Count" | "Distance" | "MoneyQuantity") {
        return None;
    }
    if age_years(kvs)? <= 89.0 {
        return Some(false);
    }
    let unit = kvs
        .iter()
        .find(|(k, _)| k == "code")
        .or_else(|| kvs.iter().find(|(k, _)| k == "unit"))
        .and_then(|(_, v)| v.as_str())
        .unwrap_or("a");
    let ninety = (90.0 / years_per(unit).unwrap_or(1.0) + 0.5) as i64;
    kvs.retain(|(k, _)| k != "comparator");
    let mut at = kvs.len();
    for (i, (k, v)) in kvs.iter_mut().enumerate() {
        if k == "value" {
            *v = Node::Number(ninety.to_string());
            at = i + 1;
        }
    }
    if comparator_allowed(ty) {
        kvs.insert(at, ("comparator".to_string(), Node::Str(">=".to_string())));
    }
    Some(true)
}

/// May a `comparator` be written on an element of this type? Only where
/// *every* element of the type carries one. `Age`, `Duration`, `Count` and
/// `Distance` do (probed: validator_cli 6.10.4 accepts `comparator: ">="` on
/// `Condition.onsetAge`, `Encounter.length`, `Task.input.valueCount` and
/// `valueDistance`). A plain `Quantity` does **not**, however free its own
/// definition is: the base definitions constrain many `Quantity` elements to
/// `SimpleQuantity` — `Observation.referenceRange.low`,
/// `MedicationAdministration.dosage.dose`, `Claim.item.quantity` — where
/// `comparator` has max 0 and `sqty-1` refuses it, and the element-type table
/// records the type code (`Quantity`) and not that profile, so the pod cannot
/// tell the two apart. It therefore writes the bare value there, exactly as
/// the walk's own `Age` clamp has always done. `SimpleQuantity` and
/// `MoneyQuantity` named outright by a compiled profile are refused the same
/// way (`sqty-1`, `mtqy-1`). See `docs/scope.md`.
fn comparator_allowed(ty: &str) -> bool {
    matches!(ty, "Age" | "Duration" | "Count" | "Distance")
}

/// Ages over 89 read as 90, in the quantity's own unit.
fn clamp_age(kvs: &mut Vec<(String, Node)>) -> bool {
    let Some(years) = age_years(kvs) else { return false };
    if years <= 89.0 {
        return false;
    }
    let unit = kvs
        .iter()
        .find(|(k, _)| k == "code")
        .or_else(|| kvs.iter().find(|(k, _)| k == "unit"))
        .and_then(|(_, v)| v.as_str())
        .unwrap_or("a");
    let ninety = (90.0 / years_per(unit).unwrap_or(1.0) + 0.5) as i64;
    kvs.retain(|(k, _)| k != "comparator");
    for (k, v) in kvs.iter_mut() {
        if k == "value" {
            *v = Node::Number(ninety.to_string());
        }
    }
    true
}

fn is_primitive_type(ty: &str) -> bool {
    ty.chars().next().is_some_and(|c| c.is_ascii_lowercase())
}

/// The year of a `date` / `dateTime` value, when it is one.
fn year_of(s: &str) -> Option<i64> {
    let t = crate::temporal::parse(s)?;
    if !t.has_date {
        return None;
    }
    s.get(..4)?.parse().ok()
}

/// The typed-walk de-identification of `input` under `policy`; the copy is
/// then validated with `validator` and checked.
pub fn deidentify(input: &[u8], policy: &Policy, validator: &Validator) -> Result<Deidentified, DeidError> {
    let parsed = Parsed::parse(input).map_err(|_| DeidError::NotAResource)?;
    let Some(Value::Str(rt)) = parsed.top_member("resourceType") else { return Err(DeidError::NotAResource) };
    let rt = typing::known_type(rt).ok_or(DeidError::UnknownResourceType)?;
    if crate::typemap::RESOURCES.binary_search(&rt).is_err() {
        return Err(DeidError::UnknownResourceType);
    }
    let doc = parsed.doc();
    // what the profiles require — of this resource and of every resource
    // nested in it (a Bundle's entries, contained resources), each in its
    // own path space: an element that goes is masked, never dropped
    // (slice labels dropped: `AuditEvent.agent:client.who` is required of
    // the `agent` instance the walk sees as `AuditEvent.agent.who`)
    let unlabel = |path: &str| -> String {
        path.split('.').map(|seg| seg.split(':').next().unwrap_or(seg)).collect::<Vec<_>>().join(".")
    };
    // a slice's own required sub-elements (`agent:client` requires `$this.who`)
    fn slice_required(slicings: &[crate::compiled::Slicing], unlabel: &dyn Fn(&str) -> String, out: &mut Vec<String>) {
        for slicing in slicings {
            for slice in slicing.slices {
                for e in slice.elements.iter().filter(|e| e.min >= 1) {
                    if let Some(rest) = e.path.strip_prefix("$this.") {
                        out.push(unlabel(&alloc::format!("{}.{rest}", slicing.path)));
                    }
                }
                slice_required(slice.slicings, unlabel, out);
            }
        }
    }
    let paths_for = |rt: &str, d: &crate::document::TreeDoc<'_, '_>| -> Vec<String> {
        let mut out = Vec::new();
        for p in crate::select_profiles(rt, d, &validator.defaults) {
            out.extend(p.elements.iter().filter(|e| e.min >= 1).map(|e| unlabel(e.path)));
            slice_required(p.slicings, &unlabel, &mut out);
        }
        out
    };
    let mut required: Vec<String> = paths_for(rt, &doc);
    let (_, nested) = parsed.structure();
    for (_, nd) in &nested {
        if let Some(nrt) = nd.scalars("$this.resourceType").first().and_then(|t| typing::known_type(t)) {
            required.extend(paths_for(nrt, nd));
        }
    }
    let mut root = Node::from_value(parsed.value());
    // every resource root in the document — this one, contained ones, entries —
    // owes its own base what it requires; the union is what the walk masks
    let mut types: Vec<&str> = doc.scalars("$this.resourceType");
    types.extend(doc.scalars("$this.contained.resourceType"));
    types.extend(doc.scalars("$this.entry.resource.resourceType"));
    let mut base_required: Vec<&'static str> = Vec::new();
    for t in types.into_iter().filter_map(typing::known_type) {
        base_required.extend(typing::required_resource_children(t));
    }
    let mut walker = Walker { policy, required, base_required, applied: Vec::new(), residual: Vec::new() };
    if let Node::Object(kvs) = &mut root {
        walker.walk_object(kvs, rt, rt);
    }
    let resource = root.to_json();
    let validation = validator.validate(resource.as_bytes());
    let remaining = check(resource.as_bytes());
    Ok(Deidentified { resource, applied: walker.applied, validation, remaining, residual: walker.residual })
}

/// What the built-in policy's own check still objects to in `input`: identifying types
/// present (a name, a contact point, narrative, a note, an attachment) other
/// than as masked stubs, address members below state, dates finer than a
/// year, instants, reference displays, ages over 89. Pseudonymisation
/// cannot be told from the outside and is not checked.
pub fn check(input: &[u8]) -> Vec<Finding> {
    let mut out = Vec::new();
    let Ok(parsed) = Parsed::parse(input) else { return out };
    let Some(Value::Str(rt)) = parsed.top_member("resourceType") else { return out };
    let Some(rt) = typing::known_type(rt) else { return out };
    check_object(parsed.value(), rt, rt, &mut out);
    out
}

const IDENTIFYING: &[&str] = &["HumanName", "ContactPoint", "Narrative", "Annotation", "Attachment", "Signature"];
const ADDRESS_ALLOWED: &[&str] = &["use", "type", "state", "country", "id", "extension"];

fn check_object(v: &Value<'_>, ctx: &'static str, path: &str, out: &mut Vec<Finding>) {
    let Value::Object(kvs) = v else { return };
    for (key, value) in kvs {
        if key.as_ref() == "resourceType" {
            continue;
        }
        let child_path = alloc::format!("{path}.{key}");
        let Some(child) = typing::child(ctx, key) else { continue };
        let each = |f: &mut dyn FnMut(&Value<'_>)| match value {
            Value::Array(xs) => xs.iter().for_each(&mut *f),
            x => f(x),
        };
        if IDENTIFYING.contains(&child.ty) {
            each(&mut |x| {
                if !is_masked_value(x) {
                    out.push(Finding { path: child_path.clone(), what: alloc::format!("{} present", child.ty) });
                }
            });
            continue;
        }
        if ctx == "Address" && !ADDRESS_ALLOWED.contains(&key.as_ref()) {
            out.push(Finding { path: child_path.clone(), what: "address below state".into() });
            continue;
        }
        if ctx == "Reference" && key.as_ref() == "display" {
            out.push(Finding { path: child_path.clone(), what: "reference display".into() });
            continue;
        }
        match child.ty {
            "date" | "dateTime" => each(&mut |x| {
                if matches!(x, Value::Str(s) if s.len() > 4) {
                    out.push(Finding { path: child_path.clone(), what: "date finer than a year".into() });
                }
            }),
            "instant" => each(&mut |x| {
                if matches!(x, Value::Str(s) if !(s.len() == 20 && s.ends_with("-01-01T00:00:00Z"))) {
                    out.push(Finding { path: child_path.clone(), what: "instant finer than a year".into() });
                }
            }),
            _ => {}
        }
        if child.ty == "Age" {
            each(&mut |x| {
                if let Value::Object(q) = x {
                    let get = |k: &str| q.iter().find(|(m, _)| m.as_ref() == k).map(|(_, v)| v);
                    let value = get("value").and_then(|v| match v {
                        Value::Number(n) => n.parse::<f64>().ok(),
                        _ => None,
                    });
                    let unit = get("code").or_else(|| get("unit")).and_then(|v| match v {
                        Value::Str(u) => years_per(u),
                        _ => None,
                    });
                    // over 89 is a finding; exactly 90 is Safe Harbor's own aggregate
                    if let (Some(v), Some(per)) = (value, unit) {
                        let years = v * per;
                        if years > 89.0 && (years - 90.0).abs() > 1e-9 {
                            out.push(Finding { path: child_path.clone(), what: "age over 89".into() });
                        }
                    }
                }
            });
        }
        each(&mut |x| {
            if child.ty == "Resource" {
                if let Value::Object(inner) = x {
                    let rt = inner.iter().find(|(k, _)| k.as_ref() == "resourceType").and_then(|(_, v)| match v {
                        Value::Str(s) => typing::known_type(s),
                        _ => None,
                    });
                    if let Some(rt) = rt {
                        check_object(x, rt, &alloc::format!("{child_path}[{rt}]"), out);
                    }
                }
            } else {
                check_object(x, child.ctx, &child_path, out);
            }
        });
    }
    // carriers hold extensions: walk them as elements
    for (key, value) in kvs {
        if let Some(bare) = key.strip_prefix('_') {
            let child_path = alloc::format!("{path}.{bare}");
            match value {
                Value::Array(xs) => xs.iter().for_each(|x| check_object(x, "Element", &child_path, out)),
                x => check_object(x, "Element", &child_path, out),
            }
        }
    }
}

fn is_masked_value(v: &Value<'_>) -> bool {
    let Value::Object(kvs) = v else { return false };
    let member = |k: &str| kvs.iter().find(|(m, _)| m.as_ref() == k).map(|(_, v)| v);
    // a masked Narrative / Annotation stub
    if kvs.len() == 2 && matches!(member("div"), Some(Value::Str(d)) if d.as_ref() == MASKED_DIV) {
        return true;
    }
    if kvs.len() == 1 && matches!(member("text"), Some(Value::Str(t)) if t.as_ref() == "Masked") {
        return true;
    }
    // a data-absent-reason stub — possibly carrying masked required children
    // (`_family`, a nested masked element) and nothing else
    let dar = |v: &Value<'_>| matches!(v, Value::Array(xs) if xs.iter().any(|x| matches!(x, Value::Object(e) if e.iter().any(|(k, v)| k.as_ref() == "url" && matches!(v, Value::Str(u) if u.as_ref() == DAR)))));
    !kvs.is_empty()
        && kvs.iter().any(|(k, v)| k.as_ref() == "extension" && dar(v))
        && kvs.iter().all(|(k, v)| (k.as_ref() == "extension" && dar(v)) || is_masked_value(v))
}

impl Validator {
    /// De-identify a resource under `policy`: see [`deidentify`].
    pub fn deidentify(&self, input: &[u8], policy: &Policy) -> Result<Deidentified, DeidError> {
        deidentify(input, policy, self)
    }
}
