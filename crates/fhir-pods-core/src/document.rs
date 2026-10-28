//! The `Document` port: how the engine reads a resource, independent of the
//! JSON representation. `TreeDoc` is today's adapter — a cheap view that can be
//! re-rooted at any node, which is what slicing needs.

use alloc::vec::Vec;

use crate::json::{self, Value};
use crate::terminology::TerminologyProvider;

pub trait Document: Sized + Clone {
    /// For each instance of the element's parent, how many children exist
    /// at the last path segment. Empty when no parent exists — a child's
    /// cardinality is only evaluated where its parent is present.
    fn child_counts(&self, path: &str) -> Vec<usize>;
    /// All scalar string values at `path` (across every parent).
    fn scalars(&self, path: &str) -> Vec<&str>;
    /// Sub-documents rooted at each node `path` resolves to.
    fn instances(&self, path: &str) -> Vec<Self>;
    /// The direct children under one key (arrays expanded) — `instances("$this.key")`
    /// without building or re-splitting a path.
    fn child_instances(&self, key: &str) -> Vec<Self> {
        let mut p = alloc::string::String::from("$this.");
        p.push_str(key);
        self.instances(&p)
    }
    /// This node's scalar value as text, if it is a scalar.
    fn as_scalar(&self) -> Option<&str>;
    /// This node's value if it is a JSON number (its literal text).
    fn as_number(&self) -> Option<&str>;
    /// This node's value if it is a JSON boolean.
    fn as_bool(&self) -> Option<bool>;
    /// Structural equality with another node (FHIRPath `=` on complex
    /// types, `intersect`).
    fn same(&self, other: &Self) -> bool;
    /// Every direct child node (array members expanded) — `descendants()`.
    fn all_children(&self) -> Vec<Self>;
    /// This node's FHIR type name, when the adapter can tell.
    fn fhir_type(&self) -> Option<&'static str>;
    /// The children of a choice element (`value` → `valueQuantity` /
    /// `valueString` / …), each typed by its concrete key.
    fn choice_children(&self, stem: &str) -> Vec<Self>;
    /// The concrete keys present for a choice element (`valueQuantity`, …).
    fn choice_keys(&self, stem: &str) -> Vec<&str>;
    /// Does this resource pass the base spec's shape rules on its own (no
    /// unknown element, no unresolvable extension, every datatype's rules)?
    /// What "conforms to a profile" includes for the reference validator,
    /// beyond the profile's own constraints. `true` where unknown.
    fn shape_ok(&self, tx: &dyn crate::terminology::TerminologyProvider) -> bool {
        let _ = tx;
        true
    }
    /// A stable identity for this node within one validation (its address),
    /// `0` where the document has none — what a memo keys on.
    fn identity(&self) -> usize {
        0
    }
}

/// Owner of a parsed resource; hands out `TreeDoc` views.
#[derive(Debug)]
pub struct Parsed<'a> {
    value: Value<'a>,
}

impl<'a> Parsed<'a> {
    pub fn parse(input: &'a [u8]) -> Result<Self, json::ParseError> {
        Ok(Self { value: json::parse(input)? })
    }
    /// The whole parsed tree.
    pub fn value(&self) -> &Value<'a> {
        &self.value
    }
    pub fn doc(&self) -> TreeDoc<'_, 'a> {
        TreeDoc { root: &self.value, ctx: resource_ctx(&self.value), carrier: None }
    }
    /// What the base spec says about the shape of every element, profile or
    /// no profile: an element the type does not define, a value of the wrong
    /// JSON kind for its FHIR type, a repeating element that is not an array
    /// (or a single one that is), a date / time that does not parse, a
    /// datatype missing what it requires (`Narrative.status`,
    /// `Extension.url`). Each finding is `(element path, issue type, message)`.
    pub fn structure_gaps(&self) -> Vec<Gap> {
        self.structure().0
    }

    /// The shape walk, once: its findings, and every resource nested inside
    /// this one — `Bundle.entry.resource`, `contained`,
    /// `Parameters.parameter.resource`, recursively — with the indexed
    /// element path it sits at (`Bundle.entry[2].resource`), each as a
    /// document of its own so its declared profiles can be validated.
    pub fn structure(&self) -> (Vec<Gap>, Vec<(alloc::string::String, TreeDoc<'_, 'a>)>) {
        let (gaps, _, nested, _) = self.structure_full(&crate::terminology::Bundled);
        (gaps, nested)
    }

    /// [`structure`](Self::structure), plus the findings that are warnings
    /// (a datatype invariant the base spec grades so), plus every extension
    /// instance whose `url` names a compiled extension profile, at its
    /// label, with that profile — each is validated against it, wherever it
    /// sits.
    #[allow(clippy::type_complexity)]
    pub fn structure_full(
        &self,
        tx: &dyn crate::terminology::TerminologyProvider,
    ) -> (
        Vec<Gap>,
        Vec<Gap>,
        Vec<(alloc::string::String, TreeDoc<'_, 'a>)>,
        Vec<(alloc::string::String, TreeDoc<'_, 'a>, &'static crate::compiled::CompiledProfile)>,
    ) {
        self.structure_full_with(tx, &crate::options::Options::default())
    }

    /// [`structure_full`](Self::structure_full) under the host's options —
    /// what a narrative's references are held to.
    #[allow(clippy::type_complexity)]
    pub fn structure_full_with(
        &self,
        tx: &dyn crate::terminology::TerminologyProvider,
        options: &crate::options::Options,
    ) -> (
        Vec<Gap>,
        Vec<Gap>,
        Vec<(alloc::string::String, TreeDoc<'_, 'a>)>,
        Vec<(alloc::string::String, TreeDoc<'_, 'a>, &'static crate::compiled::CompiledProfile)>,
    ) {
        let Some(rt) = resource_ctx(&self.value) else { return (Vec::new(), Vec::new(), Vec::new(), Vec::new()) };
        let mut w = Walk {
            root: rt,
            tx,
            resource_keys: Vec::new(),
            gaps: Vec::new(),
            warnings: Vec::new(),
            invariants: Vec::new(),
            profile_invariants: Vec::new(),
            nested: Vec::new(),
            extensions: Vec::new(),
            ctxs: Vec::new(),
            nodes: Vec::new(),
            segs: Vec::new(),
            profiled: Vec::new(),
            profiles: Vec::new(),
            contained: Vec::new(),
            refuse_external_links: options.refuse_external_narrative_links(),
        };
        structure_in(&self.value, rt, rt, &mut w);
        let nested = w
            .nested
            .into_iter()
            .filter_map(|(label, v)| resource_ctx(v).map(|rt| (label, TreeDoc::at(v, Some(rt)))))
            .collect();
        let extensions =
            w.extensions.into_iter().map(|(label, v, p)| (label, TreeDoc::at(v, Some("Extension")), p)).collect();
        (w.gaps, w.warnings, nested, extensions)
    }

    /// A top-level member by key (the JSON type tag lives here).
    pub fn top_member(&self, key: &str) -> Option<&Value<'a>> {
        match &self.value {
            Value::Object(kvs) => kvs.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// One structural finding: element path, issue type, message.
pub type Gap = (alloc::string::String, crate::validate::IssueType, alloc::string::String);

/// R4's primitive types: what a choice key's suffix may name besides a
/// complex type (`valueDateTime`, `deceasedBoolean`).
const PRIMITIVE_TYPES: &[&str] = &[
    "base64Binary",
    "boolean",
    "canonical",
    "code",
    "date",
    "dateTime",
    "decimal",
    "id",
    "instant",
    "integer",
    "markdown",
    "oid",
    "positiveInt",
    "string",
    "time",
    "unsignedInt",
    "uri",
    "url",
    "uuid",
    "xhtml",
];

fn is_primitive(ty: &str) -> bool {
    ty.chars().next().is_some_and(|c| c.is_ascii_lowercase())
}

fn json_kind(v: &Value<'_>) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// The JSON kind a FHIR primitive is represented as.
fn expected_kind(ty: &str) -> &'static str {
    match ty {
        "boolean" => "boolean",
        "integer" | "positiveInt" | "unsignedInt" | "decimal" => "number",
        _ => "string",
    }
}

/// Is `v` a well-formed value of the temporal / numeric primitive `ty`?
fn primitive_value_ok(ty: &str, v: &Value<'_>) -> bool {
    match (ty, v) {
        ("date", Value::Str(s)) => crate::temporal::parse(s).is_some_and(|t| t.has_date && t.precision <= 3),
        // a dateTime with a time has seconds and a zone; an instant has a date too; a time has neither date nor zone
        ("dateTime", Value::Str(s)) => crate::temporal::parse(s)
            .is_some_and(|t| t.has_date && (t.precision <= 3 || (t.precision >= 6 && t.has_tz))),
        ("instant", Value::Str(s)) => {
            crate::temporal::parse(s).is_some_and(|t| t.has_date && t.precision >= 6 && t.has_tz)
        }
        ("time", Value::Str(s)) => {
            crate::temporal::parse(s).is_some_and(|t| !t.has_date && t.precision >= 6 && !t.has_tz)
        }
        // FHIR integers are 32-bit
        ("integer", Value::Number(n)) => n.parse::<i32>().is_ok(),
        ("positiveInt", Value::Number(n)) => n.parse::<i32>().is_ok_and(|i| i >= 1),
        ("unsignedInt", Value::Number(n)) => n.parse::<i32>().is_ok_and(|i| i >= 0),
        // base64: the alphabet, padding at the end only; line breaks are
        // tolerated (as the reference validator has it), other whitespace not
        ("base64Binary", Value::Str(s)) => {
            let joined: alloc::string::String = s.chars().filter(|c| !matches!(c, '\r' | '\n')).collect();
            let b = joined.as_bytes();
            !b.is_empty()
                && b.len().is_multiple_of(4)
                && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'='))
                && b.iter()
                    .position(|c| *c == b'=')
                    .is_none_or(|i| i >= b.len() - 2 && b[i..].iter().all(|c| *c == b'='))
        }
        // a urn:uuid is a lowercase UUID; a urn:oid is an OID
        ("uri" | "url" | "canonical" | "oid" | "uuid", Value::Str(s))
            if s.starts_with("urn:uuid:") || s.starts_with("urn:oid:") =>
        {
            urn_ok(s) && (ty != "canonical" || !s.is_empty())
        }
        // a canonical is absolute (a scheme) unless it is a fragment reference
        ("canonical", Value::Str(s)) => {
            !s.is_empty()
                && !s.chars().any(char::is_whitespace)
                && (s.starts_with('#')
                    || s.split('|').next().is_some_and(|u| {
                        let scheme = u.find(':').map(|i| &u[..i]).unwrap_or("");
                        !scheme.is_empty()
                            && scheme.as_bytes()[0].is_ascii_alphabetic()
                            && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
                    }))
        }
        // a uri / url / oid / uuid carries no whitespace (the reference
        // validator: "URI values cannot have whitespace") and is not empty
        ("uri" | "url" | "oid" | "uuid", Value::Str(s)) => !s.is_empty() && !s.chars().any(char::is_whitespace),
        // `[A-Za-z0-9\-\.]{1,64}` — a resource's or element's id
        ("id", Value::Str(s)) => {
            !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
        }
        // `[^\s]+(\s[^\s]+)*` — a code has no leading, trailing or doubled whitespace
        ("code", Value::Str(s)) => {
            let ws = |c: char| c.is_whitespace() || c == '\u{a0}';
            !s.is_empty()
                && !s.starts_with(ws)
                && !s.ends_with(ws)
                && !s.chars().zip(s.chars().skip(1)).any(|(a, b)| ws(a) && ws(b))
        }
        // R4 JSON: a string primitive is never empty
        (_, Value::Str(s)) => !s.is_empty(),
        _ => true,
    }
}

/// `urn:uuid:` names a lowercase UUID, `urn:oid:` an OID (arcs of digits, no
/// leading zeros, at least two, the first 0–2) — as the reference validator
/// holds every uri to.
fn urn_ok(s: &str) -> bool {
    if let Some(u) = s.strip_prefix("urn:uuid:") {
        let b = u.as_bytes();
        return b.len() == 36
            && b.iter().enumerate().all(|(i, c)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    *c == b'-'
                } else {
                    c.is_ascii_digit() || (b'a'..=b'f').contains(c)
                }
            });
    }
    if let Some(o) = s.strip_prefix("urn:oid:") {
        let arcs: Vec<&str> = o.split('.').collect();
        return arcs.len() >= 2
            && arcs.iter().all(|a| {
                !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()) && (a.len() == 1 || !a.starts_with('0'))
            })
            && matches!(arcs[0], "0" | "1" | "2");
    }
    true
}

/// The items an element's value stands for: the array's items when it
/// repeats, the value itself otherwise — or the shape error.
fn items_of<'x, 'a>(value: &'x Value<'a>, repeats: bool, path: &str) -> Result<Vec<&'x Value<'a>>, Gap> {
    use crate::validate::IssueType::Structure;
    match value {
        // R4 JSON: an array is never empty
        Value::Array(xs) if repeats && xs.is_empty() => {
            Err((path.into(), Structure, alloc::format!("{path} is an empty array (leave the element out instead)")))
        }
        Value::Array(xs) if repeats => Ok(xs.iter().collect()),
        Value::Array(_) => Err((path.into(), Structure, alloc::format!("{path} is a single element, not an array"))),
        _ if repeats => Err((path.into(), Structure, alloc::format!("{path} repeats: expected an array"))),
        x => Ok(alloc::vec![x]),
    }
}

/// The compiled profiles this resource declares (for its type).
fn compiled_profiles_of(
    kvs: &[(alloc::borrow::Cow<'_, str>, Value<'_>)],
) -> Vec<&'static crate::compiled::CompiledProfile> {
    let rt = kvs.iter().find(|(k, _)| k.as_ref() == "resourceType").and_then(|(_, v)| match v {
        Value::Str(s) => Some(s.as_ref()),
        _ => None,
    });
    let Some((_, Value::Object(meta))) = kvs.iter().find(|(k, _)| k.as_ref() == "meta") else { return Vec::new() };
    let Some((_, Value::Array(profiles))) = meta.iter().find(|(k, _)| k.as_ref() == "profile") else {
        return Vec::new();
    };
    profiles
        .iter()
        .filter_map(|p| match p {
            Value::Str(u) => {
                let base = u.split('|').next().unwrap_or(u);
                crate::REGISTRY.iter().find(|c| c.url == base && Some(c.resource_type) == rt).copied()
            }
            _ => None,
        })
        .collect()
}

/// Does this resource declare a profile the registry has compiled (for its type)?
#[allow(dead_code)]
fn declares_compiled_profile(kvs: &[(alloc::borrow::Cow<'_, str>, Value<'_>)]) -> bool {
    let rt = kvs.iter().find(|(k, _)| k.as_ref() == "resourceType").and_then(|(_, v)| match v {
        Value::Str(s) => Some(s.as_ref()),
        _ => None,
    });
    let Some((_, Value::Object(meta))) = kvs.iter().find(|(k, _)| k.as_ref() == "meta") else { return false };
    let Some((_, Value::Array(profiles))) = meta.iter().find(|(k, _)| k.as_ref() == "profile") else { return false };
    profiles.iter().any(|p| match p {
        Value::Str(u) => {
            let base = u.split('|').next().unwrap_or(u);
            crate::REGISTRY.iter().any(|c| c.url == base && Some(c.resource_type) == rt)
        }
        _ => false,
    })
}

/// The shape walk's state: the findings, every nested resource found (with
/// the indexed element path it sits at), and the path segments so far —
/// `(key, index)` for a repeating element — rendered for a finding or nested
/// resource, rather than for every successfully traversed value.
struct Walk<'v, 'a, 't> {
    root: &'static str,
    /// Who answers the base's required bindings (the host's provider, else
    /// the bundled sets).
    tx: &'t dyn crate::terminology::TerminologyProvider,
    resource_keys: Vec<Option<(&'v str, &'v str)>>,
    gaps: Vec<Gap>,
    /// Findings graded warning by the base spec (eld-20 …).
    warnings: Vec<Gap>,
    /// `typemap::DATATYPE_INVARIANTS`, parsed on first use in this walk
    /// (`None` = not yet; `Some(Err)` = outside the supported FHIRPath core).
    invariants: Vec<Option<Result<crate::path::Expr, ()>>>,
    /// The invariants of `typemap::DATATYPE_PROFILES`, parsed on first use
    /// in this walk, keyed by (profile, invariant) index.
    profile_invariants: Vec<((u8, u8), Result<crate::path::Expr, ()>)>,
    nested: Vec<(alloc::string::String, &'v Value<'a>)>,
    /// Extension instances whose `url` names a compiled extension profile,
    /// with the label they sit at: validated against it by the caller.
    extensions: Vec<(alloc::string::String, &'v Value<'a>, &'static crate::compiled::CompiledProfile)>,
    segs: Vec<(&'v str, Option<usize>)>,
    /// Whether the resource being walked (innermost) declares a compiled
    /// profile — which then restates the base's required elements itself.
    profiled: Vec<bool>,
    /// The compiled profiles the resource being walked (innermost) declares:
    /// an extension one of them reaches through a slice is theirs to check.
    profiles: Vec<Vec<&'static crate::compiled::CompiledProfile>>,
    /// The ids of the contained resources of each resource being walked,
    /// innermost last, with whether that resource is itself contained: what
    /// a local reference (`#id`) must name (ref-1) is the contained of the
    /// nearest resource that is not — `%rootResource`'s, as the invariant
    /// reads — never of a resource nested inside a contained one.
    contained: Vec<(bool, Vec<alloc::string::String>, Option<alloc::string::String>)>,
    /// Every object on the way down (root first): its typing context (a
    /// resource type, a datatype, a backbone path, `Element` for a
    /// primitive's carrier) and the JSON key it was reached by — what an
    /// extension's context is judged against.
    ctxs: Vec<(&'static str, &'v str)>,
    /// Actual nodes corresponding to ctxs, so a FHIRPath context must
    /// select this owner rather than an equal-looking sibling elsewhere.
    nodes: Vec<&'v Value<'a>>,
    /// The host asked for a narrative reference out of the resource to be
    /// refused (`externalNarrativeLinks`); off is the oracle's behaviour.
    refuse_external_links: bool,
}

/// Does a profile reach an extension of this url through a slice (an
/// inherited extension profile, or a profile discriminator)?
fn profile_reaches(slicings: &[crate::compiled::Slicing], url: &str) -> bool {
    slicings.iter().any(|sl| {
        sl.slices.iter().any(|s| {
            s.profile == Some(url)
                || s.profiles.iter().any(|(_, urls)| urls.contains(&url))
                || s.discriminators
                    .iter()
                    .any(|d| matches!(d, crate::compiled::Discriminator::Profile { url: u, .. } if *u == url))
                || profile_reaches(s.slicings, url)
        })
    })
}

impl<'v, 'a, 't> Walk<'v, 'a, 't> {
    fn provider(&self) -> crate::terminology_context::ResourceProvider<'_> {
        let (rt, id) = self.resource_keys.last().copied().flatten().map_or(("", None), |(rt, id)| (rt, Some(id)));
        crate::terminology_context::ResourceProvider::new(self.tx, rt, id)
    }
    fn label(&self, root: &str) -> alloc::string::String {
        let mut s = alloc::string::String::from(root);
        for (key, index) in &self.segs {
            s.push('.');
            s.push_str(key);
            if let Some(i) = index {
                s.push('[');
                s.push_str(&alloc::format!("{i}"));
                s.push(']');
            }
        }
        s
    }
}

/// A `uri` / `url` value under `http://hl7.org/fhir`, `http://terminology.hl7.org/`
/// or `http://fhir.org/guides` that is no definition the packages hold —
/// `http://hl7.org/fhir/sid/us-ssn.` — as the reference validator judges it.
/// An extension's `url` is judged as an extension definition instead, and
/// `Meta.source` is only ever a hint there.
fn unknown_in_known_space<'v, 'a>(ty: &str, x: &'v Value<'a>, ctx: &str, bare: &str) -> Option<&'v str> {
    if !matches!(ty, "uri" | "url") || url_check_exempt(ctx, bare) {
        return None;
    }
    match x {
        Value::Str(url) if crate::known_urls::in_known_space(url) && !crate::known_urls::known_definition(url) => {
            Some(url.as_ref())
        }
        _ => None,
    }
}

/// The base's required binding on the element `key` of an object in typing
/// context `ctx`, if any — and whether it is the walk's to check here: a
/// datatype's binding always (no snapshot restates `ContactPoint.system`
/// unless it expanded the type, and those are compiled out); a resource's
/// own only where no compiled profile applies, since a profile's snapshot
/// restates every one of them.
fn base_binding(ctx: &str, key: &str, w: &Walk<'_, '_, '_>) -> Option<&'static crate::compiled::Binding> {
    let (_, _, b) = crate::typemap::BASE_BINDINGS.iter().find(|(c, k, _)| *c == ctx && *k == key)?;
    let owner = ctx.split('.').next().unwrap_or(ctx);
    let datatype = crate::typemap::RESOURCES.binary_search(&owner).is_err();
    (datatype || w.profiled.last() == Some(&false) || through_content_reference(w)).then_some(b)
}

/// Is the object being walked reached through a `contentReference` — a
/// `Questionnaire.item` inside an item, a `Composition.section` inside a
/// section — anywhere since the resource being walked? A profile's snapshot
/// enumerates such a backbone once, at the depth the base defines it, and
/// the compiled profile checks it there; what the base requires beneath the
/// deeper copies is the walk's to check, profile or none.
fn through_content_reference(w: &Walk<'_, '_, '_>) -> bool {
    let resource = w.ctxs.iter().rposition(|(c, _)| crate::typemap::RESOURCES.binary_search(c).is_ok()).unwrap_or(0);
    w.ctxs[resource..].windows(2).any(|pair| {
        let ((parent, _), (ctx, key)) = (pair[0], pair[1]);
        // a backbone whose context is not `parent.key`: the walk followed a reference
        ctx.contains('.') && ctx.strip_prefix(parent).and_then(|rest| rest.strip_prefix('.')) != Some(key)
    })
}

/// One membership answer, reported as the reference validator words it.
fn report_base_binding(
    m: crate::terminology::Membership,
    b: &crate::compiled::Binding,
    what: &str,
    label: alloc::string::String,
    w: &mut Walk<'_, '_, '_>,
) {
    use crate::validate::IssueType::{CodeInvalid, NotSupported};
    match m {
        crate::terminology::Membership::Member => {}
        crate::terminology::Membership::NotMember => w.gaps.push((
            label,
            CodeInvalid,
            if what == crate::engine::CODINGS {
                crate::engine::no_coding_in_set(b.value_set)
            } else {
                alloc::format!(
                    "The value provided ('{what}') was not found in the value set '{}', and a code is required from this value set",
                    b.value_set
                )
            },
        )),
        crate::terminology::Membership::Unknown => w.warnings.push((
            label,
            NotSupported,
            alloc::format!("no bundled terminology for {}; '{what}' not checked", b.value_set),
        )),
    }
}

/// A `Coding` / `CodeableConcept` object under a base required binding
/// (`Condition.clinicalStatus`): its codings decide membership. A concept
/// without any usable code fails the required binding; a standalone Coding
/// without a code remains unjudged, matching validator_cli 6.10.4.
fn base_binding_object<'v, 'a>(v: &'v Value<'a>, ctx: &'static str, w: &mut Walk<'v, 'a, '_>) {
    use crate::compiled::BindingKind;
    if !matches!(ctx, "Coding" | "CodeableConcept") || w.ctxs.len() < 2 {
        return;
    }
    let (parent_ctx, key) = (w.ctxs[w.ctxs.len() - 2].0, w.ctxs[w.ctxs.len() - 1].1);
    let Some(b) = base_binding(parent_ctx, key, w) else { return };
    if !matches!((&b.kind, ctx), (BindingKind::Coding, "Coding") | (BindingKind::CodeableConcept, "CodeableConcept")) {
        return;
    }
    // a concept with no coding at all is not a member of anything: the
    // reference validator errors here for a required binding — whether the
    // concept carries text, an extension (a data-absent-reason) or nothing
    // (6.10.4, probed on `AllergyIntolerance.clinicalStatus`, bare and under
    // a profile).
    if ctx == "CodeableConcept" && !has_coding(v) {
        let label = w.label(w.root);
        w.gaps.push((
            label,
            crate::validate::IssueType::CodeInvalid,
            alloc::format!("No code provided, and a code is required from the value set {}", b.value_set),
        ));
        return;
    }
    let doc = TreeDoc::at(v, Some(ctx));
    let (m, seen) = crate::engine::membership_of(b, &doc, &w.provider());
    if seen {
        let label = w.label(w.root);
        report_base_binding(m, b, crate::engine::CODINGS, label, w);
    } else if ctx == "CodeableConcept" {
        // Coding objects exist, but none carries a code. The Coding itself is
        // permitted to be partial; the parent concept still has no member of
        // its required value set — where the set can be decided at all
        // (the reference validator says nothing for a set it cannot expand
        // offline, and neither do we beyond "not checked").
        let label = w.label(w.root);
        let m = if w.provider().validate_code(b.value_set, "", "") != crate::terminology::Membership::Unknown {
            crate::terminology::Membership::NotMember
        } else {
            crate::terminology::Membership::Unknown
        };
        report_base_binding(m, b, crate::engine::CODINGS, label, w);
    }
}

/// Does a `CodeableConcept` object carry at least one coding?
fn has_coding(v: &Value<'_>) -> bool {
    match v {
        Value::Object(kvs) => {
            kvs.iter().any(|(k, x)| k.as_ref() == "coding" && matches!(x, Value::Array(xs) if !xs.is_empty()))
        }
        _ => false,
    }
}

/// The elements the reference validator does not put through its
/// definition-resolution check (6.10.4, `isExemptPathForUrlChecking` and
/// `isCanonicalURLElement`, plus the elements its own bundle, extension,
/// terminology and `Meta.source` checks judge instead): an extension's
/// `url`, a canonical resource's own `url`, a bundle entry's `fullUrl` and
/// request `url`, a coding's `system`, an attachment's `url`, an endpoint's
/// address, a mapping's uri, a message source's endpoint, …
fn url_check_exempt(ctx: &str, bare: &str) -> bool {
    matches!(
        (ctx, bare),
        ("Extension", "url")
            | ("Meta", "source")
            | ("Coding", "system")
            | ("Bundle.entry", "fullUrl")
            | ("Bundle.entry.request", "url")
            | ("Attachment", "url")
            | ("Endpoint", "address")
            | ("CapabilityStatement.implementation", "url")
            | ("StructureDefinition", "type")
            | ("ImplementationGuide.dependsOn", "uri")
            | ("StructureDefinition.mapping", "uri")
            | ("MessageHeader.source", "endpoint")
            | ("Requirements.statement", "satisfiedBy")
    ) || (ctx == "ImplementationGuide.definition.page" && (bare.starts_with("source") || bare.starts_with("name")))
        || (ctx == "ElementDefinition" && (bare.starts_with("fixed") || bare.starts_with("pattern")))
        || (bare == "url" && CANONICAL_RESOURCES.binary_search(&ctx).is_ok())
}

/// An absolute URL as the reference validator sees one (`Utilities.isAbsoluteUrl`):
/// a scheme of lowercase token characters before a colon — `http`, `https`,
/// `urn`, `file`, `mailto`, … — and no whitespace.
fn absolute_url(s: &str) -> bool {
    let Some((scheme, _)) = s.split_once(':') else { return false };
    !scheme.is_empty()
        && scheme.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'))
        && scheme.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && !s.chars().any(char::is_whitespace)
}

/// An extension URL the reference validator lets pass unresolved
/// (`allowUnknownExtension`): example and DICOM domains, the tools
/// extensions, run with example URLs allowed as the corpus runs it.
fn example_url(url: &str) -> bool {
    url.contains("example.org")
        || url.contains("acme.com")
        || url.contains("acme.org")
        || url.contains("nema.org")
        || url.starts_with("http://hl7.org/fhir/tools/StructureDefinition/")
        || url == "http://hl7.org/fhir/StructureDefinition/structuredefinition-expression"
}

/// `http://hl7.org/fhir/<version>/StructureDefinition/extension-<Type>.<element>`:
/// an element of another FHIR version carried as an extension.
fn xver_extension(url: &str) -> bool {
    url.strip_prefix("http://hl7.org/fhir/").and_then(|rest| rest.split_once('/')).is_some_and(|(version, tail)| {
        version.len() >= 3
            && version.chars().all(|c| c.is_ascii_digit() || c == '.')
            && tail.starts_with("StructureDefinition/extension-")
    })
}

/// A BCP-47 language tag as the reference validator accepts one: a primary
/// subtag of two to eight lowercase letters, then subtags of one to eight
/// alphanumerics, `-`-separated.
fn language_tag(s: &str) -> bool {
    let mut parts = s.split('-');
    let Some(primary) = parts.next() else { return false };
    (2..=8).contains(&primary.len())
        && primary.chars().all(|c| c.is_ascii_lowercase())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn structure_in<'v, 'a>(v: &'v Value<'a>, ctx: &'static str, path: &str, w: &mut Walk<'v, 'a, '_>) {
    structure_at(v, ctx, path, "", false, true, w)
}

/// `contained`: this object is a contained resource (or inside one), which
/// the base spec holds to `dom-2` / `dom-4` / `dom-5`. `needs_content`: ele-1
/// applies to this object on its own — false for a primitive's `_name`
/// carrier whose value is present (the element has its value).
fn structure_at<'v, 'a>(
    v: &'v Value<'a>,
    ctx: &'static str,
    path: &str,
    key: &'v str,
    contained: bool,
    needs_content: bool,
    w: &mut Walk<'v, 'a, '_>,
) {
    let resource = crate::typemap::RESOURCES.binary_search(&ctx).is_ok();
    if resource {
        let key = if let Value::Object(fields) = v {
            fields.iter().find(|(key, _)| key.as_ref() == "id").and_then(|(_, value)| match value {
                Value::Str(id) => Some((ctx, id.as_ref())),
                _ => None,
            })
        } else {
            None
        };
        w.resource_keys.push(key);
    }
    w.ctxs.push((ctx, key));
    w.nodes.push(v);
    datatype_invariants(v, ctx, w);
    datatype_profile(v, w);
    base_binding_object(v, ctx, w);
    structure_object(v, ctx, path, contained, needs_content, w);
    w.nodes.pop();
    w.ctxs.pop();
    if resource {
        w.resource_keys.pop();
    }
}

/// A datatype's own invariants (cpt-2, per-1, qty-3, rng-2, tim-* …) on
/// this object, whatever it is reached through — a resource's element, an
/// extension's value, a datatype inside a datatype. The reference validator
/// judges every element by its type; a profile's snapshot restating one
/// (UK Core expands `Patient.telecom`) is not compiled a second time.
fn datatype_invariants<'v, 'a>(v: &'v Value<'a>, ctx: &'static str, w: &mut Walk<'v, 'a, '_>) {
    use crate::typemap::DATATYPE_INVARIANTS;
    use crate::validate::IssueType::{Invariant, NotSupported};
    if !matches!(v, Value::Object(_)) || !DATATYPE_INVARIANTS.iter().any(|(c, ..)| *c == ctx) {
        return;
    }
    if w.invariants.is_empty() {
        w.invariants.resize_with(DATATYPE_INVARIANTS.len(), || None);
    }
    let owner = w.nodes.len() - 1;
    let resource = (0..=owner).rev().find(|&i| resource_ctx(w.nodes[i]).is_some());
    let root = resource.map(|i| TreeDoc::at(w.nodes[i], Some(w.ctxs[i].0)));
    let bundle = (0..=owner).rev().find(|&i| w.ctxs[i].0 == "Bundle").map(|i| TreeDoc::at(w.nodes[i], Some("Bundle")));
    let this_type = if ctx.contains('.') { "Element" } else { ctx };
    let here = TreeDoc::at(v, Some(ctx));
    let label = w.label(w.root);
    for (n, (c, key, is_error, human, expression)) in DATATYPE_INVARIANTS.iter().enumerate() {
        if *c != ctx {
            continue;
        }
        if w.invariants[n].is_none() {
            w.invariants[n] = Some(crate::path::parse_invariant(key, expression).map_err(|_| ()));
        }
        let label = label.clone();
        match w.invariants[n].as_ref().and_then(|r| r.as_ref().ok()) {
            None => w.warnings.push((
                label,
                NotSupported,
                alloc::format!("{key} not evaluated: outside the supported FHIRPath core"),
            )),
            Some(expr) => {
                match crate::path::check_constraint_in(expr, &here, Some(this_type), root.as_ref(), bundle.as_ref()) {
                    crate::path::Verdict::Pass => {}
                    crate::path::Verdict::Fail => {
                        let finding = (label, Invariant, alloc::format!("{key}: {human}"));
                        if *is_error {
                            w.gaps.push(finding)
                        } else {
                            w.warnings.push(finding)
                        }
                    }
                    crate::path::Verdict::Unsupported => w.warnings.push((
                        label,
                        NotSupported,
                        alloc::format!("{key} not evaluated: outside the supported FHIRPath core"),
                    )),
                }
            }
        }
    }
}

/// The datatype profile the base definitions put on the element this object
/// was reached by, applied whole (`typemap::ELEMENT_PROFILES`): R4 types
/// `Observation.referenceRange.low`, `Dosage.doseAndRate.doseQuantity`,
/// `Range.low` and 52 more `{"code": "Quantity", "profile":
/// [".../SimpleQuantity"]}`, so the object is walked as the `Quantity` it is
/// and then held to the profile's differential — every element it
/// prohibits (`comparator`, max 0; present through its `_` carrier alone
/// too) and its own invariants (`sqty-1`). Wherever the element sits: a
/// resource's own, a datatype's (`Range`, `Dosage`), inside a contained
/// resource or a Bundle entry, profile or none — as the reference validator
/// judges every element by its declared type.
fn datatype_profile<'v, 'a>(v: &'v Value<'a>, w: &mut Walk<'v, 'a, '_>) {
    use crate::typemap::{DATATYPE_PROFILES, ELEMENT_PROFILES};
    use crate::validate::IssueType::{Invariant, NotSupported, Structure};
    let (Value::Object(kvs), Some(owner)) = (v, w.ctxs.len().checked_sub(2)) else { return };
    let (parent, key) = (w.ctxs[owner].0, w.ctxs[owner + 1].1);
    let Ok(i) = ELEMENT_PROFILES.binary_search_by(|(c, k, _)| (*c, *k).cmp(&(parent, key))) else { return };
    let p = ELEMENT_PROFILES[i].2;
    let (name, url, ty, prohibited, invariants) = DATATYPE_PROFILES[p as usize];
    let label = w.label(w.root);
    for element in prohibited {
        if kvs.iter().any(|(k, _)| k.strip_prefix('_').unwrap_or(k) == *element) {
            // located at the profiled element itself, as the reference
            // validator locates it (`Observation.referenceRange[0].low`)
            w.gaps.push((
                label.clone(),
                Structure,
                alloc::format!(
                    "{label}.{element} exceeds max 0 (found 1): {label} is a {name} ({url}), which prohibits {ty}.{element}"
                ),
            ));
        }
    }
    if invariants.is_empty() {
        return;
    }
    let this = owner + 1;
    let resource = (0..=this).rev().find(|&i| resource_ctx(w.nodes[i]).is_some());
    let root = resource.map(|i| TreeDoc::at(w.nodes[i], Some(w.ctxs[i].0)));
    let bundle = (0..=this).rev().find(|&i| w.ctxs[i].0 == "Bundle").map(|i| TreeDoc::at(w.nodes[i], Some("Bundle")));
    let here = TreeDoc::at(v, Some(ty));
    for (n, (inv, is_error, human, expression)) in invariants.iter().enumerate() {
        let at = (p, n as u8);
        let cached = match w.profile_invariants.iter().position(|(k, _)| *k == at) {
            Some(j) => j,
            None => {
                w.profile_invariants.push((at, crate::path::parse_invariant(inv, expression).map_err(|_| ())));
                w.profile_invariants.len() - 1
            }
        };
        let verdict = match &w.profile_invariants[cached].1 {
            Ok(expr) => crate::path::check_constraint_in(expr, &here, Some(ty), root.as_ref(), bundle.as_ref()),
            Err(()) => crate::path::Verdict::Unsupported,
        };
        match verdict {
            crate::path::Verdict::Pass => {}
            crate::path::Verdict::Fail => {
                let finding = (label.clone(), Invariant, alloc::format!("{inv}: {human} ({label} is a {name}, {url})"));
                if *is_error {
                    w.gaps.push(finding)
                } else {
                    w.warnings.push(finding)
                }
            }
            crate::path::Verdict::Unsupported => w.warnings.push((
                label.clone(),
                NotSupported,
                alloc::format!("{inv} not evaluated: outside the supported FHIRPath core"),
            )),
        }
    }
}

/// The element paths an extension's owner answers to: the owner's type, the
/// path from its resource (`Patient.telecom.value`) and from its datatype
/// (`ContactPoint.value`), with the base types everything is.
fn owner_names(w: &Walk<'_, '_, '_>) -> Vec<alloc::string::String> {
    use alloc::string::{String, ToString};
    let n = w.ctxs.len();
    let mut names: Vec<String> = alloc::vec![String::from("*"), String::from("Element")];
    if n < 2 {
        return names;
    }
    let owner = n - 2;
    let (owner_ctx, owner_key) = w.ctxs[owner];
    let is_resource = |c: &str| crate::typemap::RESOURCES.binary_search(&c).is_ok();
    // the owner's own type: what the element it was reached by is typed as
    let owner_type: &str = if owner >= 1 && !owner_key.is_empty() {
        let bare = owner_key.strip_prefix('_').unwrap_or(owner_key);
        crate::typing::child(w.ctxs[owner - 1].0, bare).map(|c| c.ty).unwrap_or(owner_ctx)
    } else {
        owner_ctx
    };
    if owner_ctx.contains('.') {
        names.push(String::from("BackboneElement"));
    } else {
        names.push(owner_type.to_string());
    }
    if is_resource(owner_ctx) {
        names.push(String::from("Resource"));
        if !matches!(owner_ctx, "Bundle" | "Parameters" | "Binary") {
            names.push(String::from("DomainResource"));
        }
    }
    // the extensions package is written across versions and names R5's
    // abstract parents; in R4 they are the resources a canonical identifies
    if CANONICAL_RESOURCES.binary_search(&owner_ctx).is_ok() {
        names.push(String::from("CanonicalResource"));
        names.push(String::from("MetadataResource"));
    }
    // the path from the nearest resource, and from the nearest datatype
    let path_from = |start: usize| -> String {
        let mut p = String::from(w.ctxs[start].0);
        for k in start + 1..=owner {
            let key = w.ctxs[k].1;
            let name = key.strip_prefix('_').unwrap_or(key);
            let choice = crate::typing::child(w.ctxs[k - 1].0, name).and_then(|c| c.choice);
            p.push('.');
            match choice {
                Some(stem) => {
                    p.push_str(stem);
                    p.push_str("[x]");
                }
                None => p.push_str(name),
            }
        }
        p
    };
    let is_datatype =
        |c: &str| !c.contains('.') && !is_resource(c) && c != "Element" && crate::typing::known_type(c).is_some();
    // a backbone of a resource (`Questionnaire.item`), as against a backbone
    // of a datatype (`ElementDefinition.binding`, `Timing.repeat`)
    let is_resource_backbone = |c: &str| c.split('.').next().is_some_and(is_resource) && c.contains('.');
    // the reference validator matches an element context three ways, and so
    // do we: (1) the literal path from the nearest resource, content
    // references uncollapsed (`Questionnaire.item.item`,
    // `StructureDefinition.snapshot.element.binding.valueSet`); (2) the
    // path from the nearest resource backbone, whose context is its
    // definition path with content-reference cycles collapsed
    // (`Questionnaire.item` for an item three deep); (3) the path from the
    // nearest enclosing datatype's definition (`Dosage.site`,
    // `ElementDefinition.code`) — the datatype the owner sits in, not the
    // owner itself when the owner is one
    if let Some(r) = (0..=owner).rev().find(|&i| is_resource(w.ctxs[i].0)) {
        names.push(path_from(r));
    }
    if let Some(r) = (0..=owner).rev().find(|&i| is_resource_backbone(w.ctxs[i].0)) {
        names.push(path_from(r));
    }
    if let Some(d) = (0..owner).rev().find(|&i| is_datatype(w.ctxs[i].0)) {
        names.push(path_from(d));
    }
    let mut unique: Vec<String> = Vec::new();
    for n in names {
        if !unique.contains(&n) {
            unique.push(n);
        }
    }
    unique
}

/// Does a string start with a URI scheme (`scheme:`)?
fn has_scheme(v: &str) -> bool {
    let scheme_end = v.find(':').unwrap_or(0);
    scheme_end > 0
        && v.as_bytes()[0].is_ascii_alphabetic()
        && v[..scheme_end].bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
}

/// R4's canonical resources, sorted (R5's `CanonicalResource`).
const CANONICAL_RESOURCES: &[&str] = &[
    "ActivityDefinition",
    "CapabilityStatement",
    "ChargeItemDefinition",
    "CodeSystem",
    "CompartmentDefinition",
    "ConceptMap",
    "EffectEvidenceSynthesis",
    "EventDefinition",
    "Evidence",
    "EvidenceVariable",
    "ExampleScenario",
    "GraphDefinition",
    "ImplementationGuide",
    "Library",
    "Measure",
    "MessageDefinition",
    "NamingSystem",
    "OperationDefinition",
    "PlanDefinition",
    "Questionnaire",
    "ResearchDefinition",
    "ResearchElementDefinition",
    "RiskEvidenceSynthesis",
    "SearchParameter",
    "StructureDefinition",
    "StructureMap",
    "TerminologyCapabilities",
    "TestScript",
    "ValueSet",
];

/// A context either selects the owner, rules it out, or cannot be checked.
/// Multiple contexts are alternatives; one successful selection is enough.
fn context_allows(
    p: &crate::compiled::CompiledProfile,
    names: &[alloc::string::String],
    w: &Walk<'_, '_, '_>,
) -> Result<bool, crate::path::Unsupported> {
    let Some(owner) = w.nodes.len().checked_sub(2) else { return Ok(false) };
    let resource = (0..=owner).rev().find(|&i| resource_ctx(w.nodes[i]).is_some());
    let bundle = (0..=owner).rev().find(|&i| w.ctxs[i].0 == "Bundle").map(|i| TreeDoc::at(w.nodes[i], Some("Bundle")));
    let url_at = |i: usize| match w.nodes[i] {
        Value::Object(members) => members.iter().find_map(|(key, value)| match value {
            Value::Str(url) if key.as_ref() == "url" => Some(url.as_ref()),
            _ => None,
        }),
        _ => None,
    };
    // An extension context also permits extensions on the parent's value,
    // as in cqf-libraryAlias on cqf-library.valueCanonical.
    let parent_extension = if w.ctxs[owner].0 == "Extension" {
        Some(owner)
    } else if owner > 0
        && w.ctxs[owner - 1].0 == "Extension"
        && w.ctxs[owner].1.trim_start_matches('_').starts_with("value")
    {
        Some(owner - 1)
    } else {
        None
    };
    let mut unsupported = false;
    for (kind, expression) in p.contexts {
        let allowed = match *kind {
            // The package also spells a choice the R5 way (`...value`).
            "element" => Ok(names.iter().any(|n| n == expression || n.strip_suffix("[x]") == Some(expression))),
            "extension" => {
                if let Some((canonical, code)) = expression.split_once('#') {
                    Ok(parent_extension.is_some_and(|i| {
                        i > 0
                            && w.ctxs[i - 1].0 == "Extension"
                            && url_at(i) == Some(code)
                            && url_at(i - 1) == Some(canonical)
                    }))
                } else {
                    Ok(parent_extension.is_some_and(|i| url_at(i) == Some(*expression)))
                }
            }
            "fhirpath" => match resource {
                Some(i) => crate::path::parse(expression).and_then(|expr| {
                    crate::path::context_nodes(&expr, &TreeDoc::at(w.nodes[i], Some(w.ctxs[i].0)), bundle.as_ref())
                        .map(|nodes| nodes.iter().any(|node| node.is_at(w.nodes[owner])))
                }),
                None => Err(crate::path::Unsupported),
            },
            _ => Err(crate::path::Unsupported),
        };
        match allowed {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(_) => unsupported = true,
        }
    }
    if unsupported {
        Err(crate::path::Unsupported)
    } else {
        Ok(false)
    }
}

fn structure_object<'v, 'a>(
    v: &'v Value<'a>,
    ctx: &'static str,
    path: &str,
    contained: bool,
    needs_content: bool,
    w: &mut Walk<'v, 'a, '_>,
) {
    use crate::validate::IssueType::{Invariant, Required, Structure, Value as ValueIssue};
    let Value::Object(kvs) = v else { return };
    let is_resource = crate::typemap::RESOURCES.binary_search(&ctx).is_ok();
    // a choice element (`medication[x]`) is present under any of its concrete keys
    let has = |name: &str| match name.strip_suffix("[x]") {
        Some(stem) => kvs.iter().any(|(k, _)| {
            let k = k.strip_prefix('_').unwrap_or(k);
            k.strip_prefix(stem).is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
        }),
        None => kvs.iter().any(|(k, _)| k.as_ref() == name || k.strip_prefix('_') == Some(name)),
    };
    // the missing element's own path (the outcome locates it at its parent,
    // as the reference validator does)
    let at = |child: &str| {
        if path.is_empty() {
            alloc::string::String::from(child)
        } else {
            alloc::format!("{path}.{child}")
        }
    };
    for child in crate::typing::required_children(ctx) {
        if !has(child) {
            w.gaps.push((at(child), Required, alloc::format!("{ctx}.{child} is required (min 1, found 0)")));
        }
    }
    // a resource with no compiled profile declared is held to what the base
    // resource itself requires — at its root and inside its backbones; a
    // profile restates those and reports them
    if is_resource {
        let declared = compiled_profiles_of(kvs);
        w.profiled.push(!declared.is_empty());
        w.profiles.push(declared);
        let ids = match kvs.iter().find(|(k, _)| k.as_ref() == "contained") {
            Some((_, Value::Array(xs))) => xs
                .iter()
                .filter_map(|x| match x {
                    Value::Object(c) => c.iter().find(|(k, _)| k.as_ref() == "id").and_then(|(_, v)| match v {
                        Value::Str(s) => Some(alloc::string::String::from(s.as_ref())),
                        _ => None,
                    }),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        // the resource's own id: a narrative fragment resolves to it (probed)
        let own = kvs.iter().find(|(k, _)| k.as_ref() == "id").and_then(|(_, v)| match v {
            Value::Str(s) => Some(alloc::string::String::from(s.as_ref())),
            _ => None,
        });
        w.contained.push((contained, ids, own));
        if w.profiled.last() == Some(&false) {
            for child in crate::typing::required_resource_children(ctx) {
                if !has(child) {
                    w.gaps.push((at(child), Required, alloc::format!("{ctx}.{child} is required (min 1, found 0)")));
                }
            }
        }
    } else if ctx.contains('.') && (w.profiled.last() == Some(&false) || through_content_reference(w)) {
        for child in crate::typing::required_backbone_children(ctx) {
            if !has(child) {
                w.gaps.push((at(child), Required, alloc::format!("{ctx}.{child} is required (min 1, found 0)")));
            }
        }
    }
    // a choice element carries one of its keys, never two: collected as the
    // keys are typed below, reported after
    let mut stems: Vec<(&'static str, Vec<&str>)> = Vec::new();
    // ele-1: an element has a value or children — `{}` (or only an `id`) is neither
    if needs_content && !is_resource && !kvs.iter().any(|(k, _)| k.as_ref() != "id") {
        w.gaps.push((path.into(), Invariant, alloc::format!("ele-1: {path} must have a value or children")));
    }
    // ref-1: a local reference names a contained resource of the resource it
    // sits in (or of one enclosing it)
    if ctx == "Reference" {
        // a relative reference is `Type/id` (`/_history/v` allowed): the
        // reference validator's format rule, applied where it resolves —
        // inside a Bundle (a standalone resource's bare id passes it). It
        // names no type list: a type it does not know passes here and
        // resolves nowhere
        if let (Some((_, Value::Str(r))), "Bundle") = (kvs.iter().find(|(k, _)| k.as_ref() == "reference"), w.root) {
            // a conditional reference (`Patient?identifier=…`, a transaction's)
            // is not a relative URL and is not held to the shape
            let relative = !r.starts_with('#') && !r.contains(':') && !r.contains('?');
            let shaped = r
                .split_once('/')
                .is_some_and(|(ty, rest)| !ty.is_empty() && !rest.is_empty() && !rest.starts_with('/'));
            if relative && !shaped {
                w.gaps.push((
                    alloc::format!("{path}.reference"),
                    crate::validate::IssueType::Value,
                    alloc::format!("Relative URLs must be of the format [ResourceName]/[id]. Encountered {r}"),
                ));
            }
        }
        // a declared `type` is held to the element's base targets where no
        // compiled profile applies (a profile's own targets are the engine's
        // to check), as the reference validator holds it — by bare type name
        // only (a canonical there is "not a valid Target" to it)
        // … and only beside a `reference`: a type on an identifier-only or
        // display-only reference is not judged (probed, 6.10.4)
        let unprofiled = w.profiles.last().is_none_or(|p| p.is_empty());
        let referenced = kvs.iter().any(|(k, v)| k.as_ref() == "reference" && matches!(v, Value::Str(_)));
        if let (Some((_, Value::Str(t))), true) =
            (kvs.iter().find(|(k, _)| k.as_ref() == "type"), unprofiled && referenced)
        {
            if let Some(targets) = w.ctxs.len().checked_sub(2).and_then(|i| {
                let ((parent, _), (_, key)) = (w.ctxs[i], w.ctxs[i + 1]);
                let at = alloc::format!("{parent}.{key}");
                crate::typemap::REFERENCE_TARGETS.iter().find(|(p, _)| *p == at).map(|(_, t)| *t)
            }) {
                if !targets.split('|').any(|x| x == t.as_ref()) {
                    w.gaps.push((
                        path.into(),
                        crate::validate::IssueType::Structure,
                        alloc::format!(
                            "Invalid Resource target type. Found {t}, but expected one of ({})",
                            targets.replace('|', ", ")
                        ),
                    ));
                }
            }
        }
        if let Some((_, Value::Str(r))) = kvs.iter().find(|(k, _)| k.as_ref() == "reference") {
            if let Some(local) = r.strip_prefix('#') {
                let root_ids =
                    w.contained.iter().rev().find(|(inside, _, _)| !inside).map(|(_, ids, _)| ids.as_slice());
                if !local.is_empty() && !root_ids.is_some_and(|ids| ids.iter().any(|id| id == local)) {
                    w.gaps.push((
                        alloc::format!("{path}.reference"),
                        Invariant,
                        alloc::format!("ref-1: reference '{r}' names no contained resource of {}", w.root),
                    ));
                }
            }
        }
    }
    // a Coding (or a Quantity's coded unit) whose system is a base code
    // system the pod carries is held to its codes, as the reference validator
    // holds it
    if matches!(ctx, "Coding" | "Quantity" | "SimpleQuantity" | "Age" | "Duration" | "Count" | "Distance") {
        let system = kvs.iter().find(|(k, _)| k.as_ref() == "system");
        let code = kvs.iter().find(|(k, _)| k.as_ref() == "code");
        if let (Some((_, Value::Str(system))), Some((_, Value::Str(code)))) = (system, code) {
            let bare = system.split('|').next().unwrap_or(system);
            // the "code system" of URIs: a code in it is a URI
            if bare == "urn:ietf:rfc:3986" && !has_scheme(code) {
                w.gaps.push((
                    alloc::format!("{path}.code"),
                    crate::validate::IssueType::CodeInvalid,
                    alloc::format!("Unknown code '{code}' in the CodeSystem 'urn:ietf:rfc:3986'"),
                ));
            }
            // a code system named under THO's own space that THO does not
            // define (`v2-0296`, a table whose codes are BCP-47 tags): the
            // reference validator holds a coding to a definition there
            if ctx == "Coding"
                && bare.starts_with("http://terminology.hl7.org/")
                && !crate::known_urls::known_definition(bare)
                && crate::code_systems_table::CODE_SYSTEMS.binary_search_by(|(u, _, _)| (*u).cmp(bare)).is_err()
            {
                w.gaps.push((
                    alloc::format!("{path}.system"),
                    ValueIssue,
                    alloc::format!("No definition could be found for URL value '{bare}'"),
                ));
            }
            if let Ok(i) = crate::code_systems_table::CODE_SYSTEMS.binary_search_by(|(u, _, _)| (*u).cmp(bare)) {
                let (_, case_sensitive, codes) = crate::code_systems_table::CODE_SYSTEMS[i];
                // the table is line-delimited: a code carrying a line break
                // (lexically a code, never a concept) would match across two
                // entries, so it is unknown outright; a case-insensitive
                // system's table is lowercased already
                let needle = alloc::format!("\n{code}\n");
                let known = !code.contains(['\n', '\r'])
                    && if case_sensitive { codes.contains(&needle) } else { codes.contains(&needle.to_lowercase()) };
                if !known {
                    w.gaps.push((
                        alloc::format!("{path}.code"),
                        crate::validate::IssueType::CodeInvalid,
                        alloc::format!("Unknown code '{code}' in the CodeSystem '{bare}'"),
                    ));
                }
            }
        }
    }
    // the narrative's xhtml, as the reference validator holds it: txt-1
    // (only the elements and attributes a narrative allows, well-formed,
    // a div in the XHTML namespace) with the parser's own findings beside
    // it, and txt-2 (text beyond whitespace, tags and comments — an image
    // counts, an entity counts, CDATA does not)
    if ctx == "Narrative" {
        if let Some((_, Value::Str(div))) = kvs.iter().find(|(k, _)| k.as_ref() == "div") {
            // a `#fragment` resolves to the resource's own id and its contained resources' ids too
            let known: Vec<&str> = w
                .contained
                .last()
                .map(|(_, ids, own)| ids.iter().map(|s| s.as_str()).chain(own.as_deref()).collect())
                .unwrap_or_default();
            let crate::xhtml::Inspection { findings, has_text } =
                crate::xhtml::inspect(div, &alloc::format!("{path}.div"), &known, w.refuse_external_links);
            if findings.iter().any(|f| f.txt1) {
                w.gaps.push((
                    alloc::format!("{path}.div"),
                    Invariant,
                    alloc::string::String::from("txt-1: The narrative SHALL contain only the basic html formatting elements and attributes described in chapters 7-11 (except section 4 of chapter 9) and 15 of the HTML 4.0 standard, <a> elements (either name or href), images and internally contained style attributes"),
                ));
            }
            for f in findings {
                w.gaps.push((alloc::format!("{path}.div"), Structure, f.message));
            }
            if !has_text {
                w.gaps.push((
                    alloc::format!("{path}.div"),
                    Invariant,
                    alloc::string::String::from("txt-2: The narrative SHALL have some non-whitespace content"),
                ));
            }
        }
    }
    // an identifier whose system is `urn:ietf:rfc:3986` carries a URI: its
    // value must have a scheme (the reference validator's rule, an error)
    if ctx == "Identifier" {
        let system = kvs.iter().find(|(k, _)| k.as_ref() == "system");
        if matches!(system, Some((_, Value::Str(s))) if s.as_ref() == "urn:ietf:rfc:3986") {
            if let Some((_, Value::Str(v))) = kvs.iter().find(|(k, _)| k.as_ref() == "value") {
                if !has_scheme(v) {
                    w.gaps.push((
                        alloc::format!("{path}.value"),
                        ValueIssue,
                        alloc::format!(
                            "if identifier.system is 'urn:ietf:rfc:3986', then the identifier.value must be a full URI (e.g. start with a scheme), not '{v}'"
                        ),
                    ));
                }
            }
        }
    }
    // an extension whose url names a compiled extension profile is held to
    // it — and to where its definition says it may sit
    if ctx == "Extension" {
        if let Some((_, Value::Str(url))) = kvs.iter().find(|(k, _)| k.as_ref() == "url") {
            let base = url.split('|').next().unwrap_or(url);
            if let Some(p) = crate::REGISTRY.iter().find(|p| p.resource_type == "Extension" && p.url == base) {
                let names = owner_names(w);
                let allowed = if p.contexts.is_empty() { Ok(true) } else { context_allows(p, &names, w) };
                if allowed == Ok(false) {
                    let here: Vec<&str> = names.iter().map(|s| s.as_str()).filter(|n| *n != "*").collect();
                    let allowed: Vec<&str> = p.contexts.iter().map(|(_, e)| *e).collect();
                    w.gaps.push((
                        path.into(),
                        Structure,
                        alloc::format!(
                            "The extension {base} is not allowed to be used at this point (this element is [{}]; allowed = [{}])",
                            here.join(", "),
                            allowed.join(", ")
                        ),
                    ));
                } else if allowed.is_err() {
                    w.gaps.push((
                        path.into(),
                        crate::validate::IssueType::NotSupported,
                        alloc::format!(
                            "Extension context for {base} not evaluated: outside the supported FHIRPath core"
                        ),
                    ));
                }
                let reached = w.profiles.last().is_some_and(|ps| ps.iter().any(|p| profile_reaches(p.slicings, base)));
                if !reached {
                    let label = w.label(w.root);
                    w.extensions.push((label, v, p));
                }
            } else if w.ctxs.len() >= 2
                && w.ctxs[w.ctxs.len() - 2].0 != "Extension"
                && absolute_url(base)
                && !crate::known_urls::known_definition(base)
                && !xver_extension(base)
                && !example_url(base)
            {
                // an extension no package defines: the reference validator
                // refuses it wherever it is named (an `example.org` one
                // excepted, as it is run; a cross-version extension names an
                // element of another FHIR version the pod does not carry, and
                // is not judged)
                w.gaps.push((
                    path.into(),
                    Structure,
                    alloc::format!("The extension {base} could not be found so is not allowed here"),
                ));
            }
        }
    }
    // a resource's language is a BCP-47 tag: a lowercase primary subtag
    if is_resource {
        if let Some((_, Value::Str(lang))) = kvs.iter().find(|(k, _)| k.as_ref() == "language") {
            if !language_tag(lang) {
                w.gaps.push((
                    alloc::format!("{path}.language"),
                    ValueIssue,
                    alloc::format!("Invalid displayLanguage: '{lang}'"),
                ));
            }
        }
    }
    // an identifier's system is an absolute URI: a namespace, not a name
    if ctx == "Identifier" {
        if let Some((_, Value::Str(system))) = kvs.iter().find(|(k, _)| k.as_ref() == "system") {
            if !absolute_url(system) {
                w.gaps.push((
                    path.into(),
                    ValueIssue,
                    alloc::string::String::from(
                        "Identifier.system must be an absolute reference, not a local reference",
                    ),
                ));
            }
        }
    }
    // an extension's url is absolute — except a sub-extension's, which names
    // it within its parent
    if ctx == "Extension" && w.ctxs.len() >= 2 && w.ctxs[w.ctxs.len() - 2].0 != "Extension" {
        if let Some((_, Value::Str(url))) = kvs.iter().find(|(k, _)| k.as_ref() == "url") {
            if !absolute_url(url) {
                w.gaps.push((
                    path.into(),
                    ValueIssue,
                    alloc::string::String::from("Extension.url must be an absolute URL"),
                ));
            }
        }
    }
    // ext-1: an extension carries a value or nested extensions, not both
    if ctx == "Extension" {
        // a value given only through its `_value…` carrier is still the value element
        let has_value = kvs
            .iter()
            .any(|(k, _)| k.strip_prefix('_').unwrap_or(k).starts_with("value") && k.trim_start_matches('_').len() > 5);
        if has_value == has("extension") {
            w.gaps.push((
                path.into(),
                Invariant,
                alloc::format!("ext-1: {path} must have either extensions or a value, not both"),
            ));
        }
    }
    // a contained resource: no nesting, no versioning, no security labels
    if contained && is_resource {
        if has("contained") {
            w.gaps.push((
                path.into(),
                Invariant,
                alloc::format!("dom-2: {path} is contained and must not itself contain resources"),
            ));
        }
        if let Some((_, Value::Object(meta))) = kvs.iter().find(|(k, _)| k.as_ref() == "meta") {
            for (k, rule) in [("versionId", "dom-4"), ("lastUpdated", "dom-4"), ("security", "dom-5")] {
                if meta.iter().any(|(m, _)| m.as_ref() == k) {
                    w.gaps.push((
                        path.into(),
                        Invariant,
                        alloc::format!("{rule}: {path} is contained and must not have meta.{k}"),
                    ));
                }
            }
        }
    }
    for (key, value) in kvs {
        if key.as_ref() == "resourceType" && is_resource {
            continue;
        }
        let bare = key.strip_prefix('_').unwrap_or(key);
        let child_path = alloc::format!("{path}.{bare}");
        let Some(ch) = crate::typing::child(ctx, bare) else {
            w.gaps.push((child_path.clone(), Structure, alloc::format!("{child_path} is not an element of {ctx}")));
            continue;
        };
        if let Some(stem) = ch.choice {
            match stems.iter_mut().find(|(s, _)| *s == stem) {
                Some((_, keys)) => {
                    if !keys.contains(&bare) {
                        keys.push(bare)
                    }
                }
                None => stems.push((stem, alloc::vec![bare])),
            }
        }
        let repeats = crate::typing::repeats(ctx, bare);
        // a primitive's extension carrier: `_name`, an Element (an array of them, nulls as placeholders)
        if key.starts_with('_') {
            if !is_primitive(ch.ty) {
                w.gaps.push((
                    child_path.clone(),
                    Structure,
                    alloc::format!("{child_path} is not a primitive: no `_` carrier"),
                ));
                continue;
            }
            let items = match items_of(value, repeats, &child_path) {
                Ok(items) => items,
                Err(gap) => {
                    w.gaps.push(gap);
                    continue;
                }
            };
            let primitive = kvs.iter().find(|(k, _)| k.as_ref() == bare).map(|(_, v)| v);
            for (i, x) in items.into_iter().enumerate() {
                let paired = match primitive {
                    Some(Value::Array(values)) => values.get(i).is_some_and(|v| !matches!(v, Value::Null)),
                    Some(v) => !matches!(v, Value::Null),
                    None => false,
                };
                // a code present through its carrier alone (a data-absent
                // reason) still owes the code its required binding asks for
                if !paired && ch.ty == "code" && matches!(x, Value::Object(m) if !m.is_empty()) {
                    if let Some(b) = base_binding(ctx, bare, w) {
                        w.segs.push((bare, repeats.then_some(i)));
                        let label = w.label(w.root);
                        w.segs.pop();
                        w.gaps.push((
                            label,
                            crate::validate::IssueType::CodeInvalid,
                            alloc::format!(
                                "No code provided, and a code is required from the value set {}",
                                b.value_set
                            ),
                        ));
                    }
                }
                match x {
                    Value::Object(m) if m.is_empty() => w.gaps.push((
                        child_path.clone(),
                        Invariant,
                        alloc::format!("ele-1: {child_path}'s extension carrier must have some content"),
                    )),
                    Value::Object(m) => {
                        if is_resource && bare == "id" && m.iter().any(|(k, _)| k.as_ref() == "extension") {
                            w.gaps.push((
                                child_path.clone(),
                                Structure,
                                alloc::format!("{child_path}: Resource.id cannot carry extensions"),
                            ));
                        }
                        // the carrier's content is the primitive's: labelled
                        // under it (`name[0].family.extension[0]`)
                        w.segs.push((bare, repeats.then_some(i)));
                        structure_at(x, "Element", &child_path, key, contained, !paired, w);
                        w.segs.pop();
                    }
                    Value::Null if repeats && paired => {}
                    other => w.gaps.push((
                        child_path.clone(),
                        Structure,
                        alloc::format!(
                            "{child_path}: a primitive's extension carrier is a JSON object, found {}",
                            json_kind(other)
                        ),
                    )),
                }
            }
            continue;
        }
        let items = match items_of(value, repeats, &child_path) {
            Ok(items) => items,
            Err(gap) => {
                w.gaps.push(gap);
                continue;
            }
        };
        for (i, x) in items.into_iter().enumerate() {
            w.segs.push((bare, repeats.then_some(i)));
            if ch.ty == "Resource" {
                match resource_ctx(x) {
                    Some(rt) if matches!(x, Value::Object(_)) => {
                        // a resource of its own: validated against what it declares, in
                        // the outer's scope, at the label the walk is at
                        let label = w.label(w.root);
                        w.nested.push((label, x));
                        // deeper labels read as the issues do: `…resource.Bundle.entry[1]…`
                        w.segs.push((rt, None));
                        structure_at(x, rt, &child_path, bare, contained || bare == "contained", true, w);
                        w.segs.pop();
                    }
                    _ => w.gaps.push((
                        child_path.clone(),
                        Structure,
                        alloc::format!("{child_path} is not a FHIR resource"),
                    )),
                }
            } else if is_primitive(ch.ty) {
                if matches!(x, Value::Null) && repeats {
                    let carrier_key = alloc::format!("_{bare}");
                    let carrier = kvs.iter().find(|(k, _)| k.as_ref() == carrier_key);
                    let paired = matches!(carrier, Some((_, Value::Array(values))) if matches!(values.get(i), Some(Value::Object(m)) if !m.is_empty()));
                    if paired {
                        w.segs.pop();
                        continue; // a placeholder beside its carrier at this index
                    }
                }
                let expected = expected_kind(ch.ty);
                // an empty string beside a carrier with content (`"family": ""`
                // with `_family.extension`) is a value absent, not a value
                // wrong: the reference validator passes it on the carrier's
                // content, as ele-1 does
                let carried = matches!(x, Value::Str(s) if s.is_empty()) && {
                    let carrier_key = alloc::format!("_{bare}");
                    kvs.iter().any(|(k, v)| {
                        k.as_ref() == carrier_key
                            && match v {
                                Value::Object(m) => !m.is_empty(),
                                Value::Array(values) => {
                                    matches!(values.get(i), Some(Value::Object(m)) if !m.is_empty())
                                }
                                _ => false,
                            }
                    })
                };
                if json_kind(x) != expected {
                    let label = w.label(w.root);
                    w.gaps.push((
                        label.clone(),
                        ValueIssue,
                        alloc::format!("{label}: a {} is a JSON {expected}, found {}", ch.ty, json_kind(x)),
                    ));
                } else if carried {
                    // nothing to hold the value to; the carrier is walked as
                    // an element below — but a required binding still wants
                    // its code, as the reference validator says
                    if let (true, Some(b)) = (ch.ty == "code", base_binding(ctx, bare, w)) {
                        let label = w.label(w.root);
                        w.gaps.push((
                            label,
                            crate::validate::IssueType::CodeInvalid,
                            alloc::format!(
                                "No code provided, and a code is required from the value set {}",
                                b.value_set
                            ),
                        ));
                    }
                } else if !(primitive_value_ok(ch.ty, x) || (ctx == "Binary" && bare == "data")) {
                    // `Binary.data` is the content itself, not held to the base64
                    // lexical check the reference validator applies to Attachment.data
                    let label = w.label(w.root);
                    w.gaps.push((label.clone(), ValueIssue, alloc::format!("{label} is not a valid {}", ch.ty)));
                } else if let (Value::Str(code), "code", Some(b)) = (x, ch.ty, base_binding(ctx, bare, w)) {
                    // the base's required binding on a code: what the
                    // reference validator holds every resource to
                    let m = w.provider().validate_code(b.value_set, "", code);
                    let label = w.label(w.root);
                    report_base_binding(m, b, code, label, w);
                } else if let Some(url) = unknown_in_known_space(ch.ty, x, ctx, bare) {
                    // a uri in HL7's own spaces that names no definition the
                    // packages hold: a typo, not a local system (the reference
                    // validator takes an unknown URL anywhere else on trust)
                    let label = w.label(w.root);
                    w.gaps.push((
                        label,
                        ValueIssue,
                        alloc::format!("No definition could be found for URL value '{url}'"),
                    ));
                }
            } else {
                match x {
                    Value::Object(_) => structure_at(x, ch.ctx, &child_path, bare, contained, true, w),
                    other => {
                        let label = w.label(w.root);
                        w.gaps.push((
                            label.clone(),
                            ValueIssue,
                            alloc::format!("{label}: a {} is a JSON object, found {}", ch.ty, json_kind(other)),
                        ));
                    }
                }
            }
            w.segs.pop();
        }
    }
    for (stem, keys) in stems.iter().filter(|(_, keys)| keys.len() > 1) {
        w.gaps.push((
            alloc::format!("{path}.{stem}[x]"),
            Structure,
            alloc::format!("{path}.{stem}[x] allows one choice; found {}", keys.join(", ")),
        ));
    }
    if is_resource {
        w.profiled.pop();
        w.profiles.pop();
        w.contained.pop();
    }
}

/// The typing context of a resource node: its `resourceType`, if the
/// element-type table knows it.
fn resource_ctx(v: &Value<'_>) -> Option<&'static str> {
    match v {
        Value::Object(kvs) => kvs.iter().find(|(k, _)| k.as_ref() == "resourceType").and_then(|(_, v)| match v {
            Value::Str(s) => crate::typing::known_type(s),
            _ => None,
        }),
        _ => None,
    }
}

/// The value-tree adapter: a view rooted at any node, carrying the typing
/// context it was reached through (`None` when unknown) and, for a
/// primitive, its `_name` carrier — where the primitive's `extension` and
/// `id` live in the JSON representation.
#[derive(Debug, Clone, Copy)]
pub struct TreeDoc<'v, 'a> {
    root: &'v Value<'a>,
    ctx: Option<&'static str>,
    carrier: Option<&'v Value<'a>>,
}

impl<'v, 'a> TreeDoc<'v, 'a> {
    /// The JSON node this view is rooted at.
    pub fn value(&self) -> &'v Value<'a> {
        self.root
    }

    pub(crate) fn at(root: &'v Value<'a>, ctx: Option<&'static str>) -> Self {
        TreeDoc { root, ctx, carrier: None }
    }

    fn is_at(&self, node: &Value<'_>) -> bool {
        core::ptr::eq(self.root, node) || self.carrier.is_some_and(|carrier| core::ptr::eq(carrier, node))
    }

    /// The node whose members are this element's children: the node itself,
    /// or a primitive's carrier.
    fn container(&self) -> Option<&'v Value<'a>> {
        match self.root {
            Value::Object(_) | Value::Array(_) => Some(self.root),
            _ => self.carrier,
        }
    }

    /// The typing context of a child reached by `key`; a `Resource` child
    /// takes its own `resourceType`.
    fn child_ctx(ctx: Option<&'static str>, key: &str, child: &'v Value<'a>) -> Option<&'static str> {
        let ch = crate::typing::child(ctx?, key)?;
        if ch.ty == "Resource" {
            resource_ctx(child)
        } else {
            Some(ch.ctx)
        }
    }
}

impl<'v, 'a> Document for TreeDoc<'v, 'a> {
    fn identity(&self) -> usize {
        self.root as *const Value<'a> as usize
    }
    fn shape_ok(&self, tx: &dyn crate::terminology::TerminologyProvider) -> bool {
        let Some(rt) = resource_ctx(self.root) else { return true };
        let mut w = Walk {
            root: rt,
            tx,
            resource_keys: Vec::new(),
            gaps: Vec::new(),
            warnings: Vec::new(),
            invariants: Vec::new(),
            profile_invariants: Vec::new(),
            nested: Vec::new(),
            extensions: Vec::new(),
            ctxs: Vec::new(),
            nodes: Vec::new(),
            segs: Vec::new(),
            profiled: Vec::new(),
            profiles: Vec::new(),
            contained: Vec::new(),
            refuse_external_links: false,
        };
        structure_in(self.root, rt, rt, &mut w);
        !w.gaps.iter().any(|(_, kind, _)| *kind != crate::validate::IssueType::NotSupported)
    }

    fn child_counts(&self, path: &str) -> Vec<usize> {
        let Some((parent, last)) = path.rsplit_once('.') else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let Some(root) = self.container() else { return out };
        json::resolve_containers(root, parent, &mut |p| {
            let mut n = 0usize;
            json::for_each_child(p, last, &mut |_| n += 1);
            out.push(n);
        });
        out
    }

    fn scalars(&self, path: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let Some(root) = self.container() else { return out };
        json::resolve_each(root, path, &mut |v| match v {
            Value::Str(s) => out.push(s.as_ref()),
            // a fixed / pattern value on a boolean or a number compares as text
            Value::Bool(true) => out.push("true"),
            Value::Bool(false) => out.push("false"),
            Value::Number(n) => out.push(n),
            _ => {}
        });
        out
    }

    fn instances(&self, path: &str) -> Vec<Self> {
        // Type the result by folding the path's segments; a `[x]` segment
        // or an unknown element leaves the result untyped.
        let mut ctx = self.ctx;
        let mut first = true;
        for seg in path.split('.') {
            let skip = seg == "$this" || (first && Some(seg) == self.ctx);
            first = false;
            if skip {
                continue;
            }
            ctx = match ctx {
                Some(c) if !seg.ends_with("[x]") => crate::typing::child(c, seg).map(|ch| ch.ctx),
                _ => None,
            };
        }
        let mut out = Vec::new();
        let Some(root) = self.container() else { return out };
        // a one-segment path (`$this.key`) keeps the primitive's carrier; a
        // deeper one descends through carriers on the way
        match path.split_once('.').map(|(_, rest)| rest) {
            Some(key) if !key.contains('.') => json::for_each_child_ext(root, key, &mut |n, carrier| {
                out.push(TreeDoc { root: n, ctx: if ctx == Some("Resource") { resource_ctx(n) } else { ctx }, carrier })
            }),
            _ => json::resolve_each(root, path, &mut |n| {
                out.push(TreeDoc::at(n, if ctx == Some("Resource") { resource_ctx(n) } else { ctx }))
            }),
        }
        out
    }

    fn child_instances(&self, key: &str) -> Vec<Self> {
        let ctx = match self.ctx {
            Some(c) if !key.ends_with("[x]") => crate::typing::child(c, key).map(|ch| ch.ctx),
            _ => None,
        };
        let mut out = Vec::new();
        let Some(root) = self.container() else { return out };
        json::for_each_child_ext(root, key, &mut |n, carrier| {
            out.push(TreeDoc { root: n, ctx: if ctx == Some("Resource") { resource_ctx(n) } else { ctx }, carrier })
        });
        out
    }

    fn as_scalar(&self) -> Option<&str> {
        match self.root {
            Value::Str(s) => Some(s.as_ref()),
            Value::Number(s) => Some(*s),
            Value::Bool(b) => Some(if *b { "true" } else { "false" }),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self.root {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn as_number(&self) -> Option<&str> {
        match self.root {
            Value::Number(s) => Some(*s),
            _ => None,
        }
    }

    fn same(&self, other: &Self) -> bool {
        self.root == other.root
    }

    fn all_children(&self) -> Vec<Self> {
        let mut out = Vec::new();
        let ctx = self.ctx;
        let mut push = |key: &str, v: &'v Value<'a>| match v {
            Value::Array(xs) => out.extend(xs.iter().map(|x| TreeDoc::at(x, Self::child_ctx(ctx, key, x)))),
            v => out.push(TreeDoc::at(v, Self::child_ctx(ctx, key, v))),
        };
        match self.container() {
            // `resourceType` is the JSON type tag, not an element
            Some(Value::Object(kvs)) => {
                kvs.iter().filter(|(k, _)| k.as_ref() != "resourceType").for_each(|(k, v)| push(k, v))
            }
            Some(Value::Array(xs)) => xs.iter().for_each(|v| push("", v)),
            _ => {}
        }
        out
    }

    fn fhir_type(&self) -> Option<&'static str> {
        self.ctx.map(|c| if c.contains('.') { "BackboneElement" } else { c })
    }

    fn choice_keys(&self, stem: &str) -> Vec<&str> {
        let Some(Value::Object(kvs)) = self.container() else { return Vec::new() };
        let mut keys = Vec::new();
        for (key, _) in kvs {
            let key = key.strip_prefix('_').unwrap_or(key);
            // a choice key is the stem plus a FHIR type name (`valueQuantity`,
            // `deceasedBoolean`) — `countMax` is not a choice of `count`
            let is_choice = key.strip_prefix(stem).is_some_and(|rest| {
                rest.starts_with(|c: char| c.is_ascii_uppercase())
                    && (crate::typing::known_type(rest).is_some() || {
                        let mut lower = alloc::string::String::from(rest);
                        lower[..1].make_ascii_lowercase();
                        PRIMITIVE_TYPES.contains(&lower.as_str())
                    })
            });
            if is_choice && !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys
    }

    fn choice_children(&self, stem: &str) -> Vec<Self> {
        self.choice_keys(stem).into_iter().flat_map(|key| self.child_instances(key)).collect()
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;

    fn profile(contexts: &'static [(&'static str, &'static str)]) -> crate::compiled::CompiledProfile {
        crate::compiled::CompiledProfile {
            url: "https://example.org/test",
            base: None,
            name: "Test",
            resource_type: "Extension",
            elements: &[],
            slicings: &[],
            invariants: &[],
            must_support: &[],
            contexts,
            references: &[],
            deferred: &[],
            lowering: &crate::generated::cqf_libraryalias::LOWERING,
        }
    }

    #[test]
    fn extension_context_requires_nodes_and_reports_unsupported_queries() {
        let parsed = Parsed::parse(
            br#"{"resourceType":"Patient","extension":[{"url":"https://example.org/test","valueString":"test"}]}"#,
        )
        .unwrap();
        let extension = parsed.doc().child_instances("extension")[0];
        let w = Walk {
            root: "Patient",
            tx: &crate::terminology::Bundled,
            resource_keys: Vec::new(),
            gaps: Vec::new(),
            warnings: Vec::new(),
            invariants: Vec::new(),
            profile_invariants: Vec::new(),
            nested: Vec::new(),
            extensions: Vec::new(),
            segs: Vec::new(),
            profiled: Vec::new(),
            profiles: Vec::new(),
            contained: Vec::new(),
            refuse_external_links: false,
            ctxs: alloc::vec![("Patient", ""), ("Extension", "extension")],
            nodes: alloc::vec![parsed.value(), extension.value()],
        };
        let names = owner_names(&w);
        assert_eq!(context_allows(&profile(&[("fhirpath", "true")]), &names, &w), Ok(false));
        assert_eq!(
            context_allows(&profile(&[("fhirpath", "unknownFunction()")]), &names, &w),
            Err(crate::path::Unsupported)
        );
        assert_eq!(
            context_allows(&profile(&[("fhirpath", "unknownFunction()"), ("element", "Patient")]), &names, &w),
            Ok(true)
        );
    }

    #[test]
    fn extension_context_fragment_requires_the_named_complex_parent_and_child() {
        let parsed = Parsed::parse(br#"{"resourceType":"Patient","extension":[{"url":"https://example.org/parent","extension":[{"url":"part","extension":[{"url":"https://example.org/test","valueString":"test"}]}]}]}"#).unwrap();
        let parent = parsed.doc().child_instances("extension")[0];
        let part = parent.child_instances("extension")[0];
        let child = part.child_instances("extension")[0];
        let w = Walk {
            root: "Patient",
            tx: &crate::terminology::Bundled,
            resource_keys: Vec::new(),
            gaps: Vec::new(),
            warnings: Vec::new(),
            invariants: Vec::new(),
            profile_invariants: Vec::new(),
            nested: Vec::new(),
            extensions: Vec::new(),
            segs: Vec::new(),
            profiled: Vec::new(),
            profiles: Vec::new(),
            contained: Vec::new(),
            refuse_external_links: false,
            ctxs: alloc::vec![
                ("Patient", ""),
                ("Extension", "extension"),
                ("Extension", "extension"),
                ("Extension", "extension")
            ],
            nodes: alloc::vec![parsed.value(), parent.value(), part.value(), child.value()],
        };
        let names = owner_names(&w);
        for (expression, expected) in [
            ("https://example.org/parent#part", true),
            ("https://example.org/parent#other", false),
            ("https://example.org/other#part", false),
        ] {
            let mut p = profile(&[]);
            // These are separate static context lists, matching compiled definitions.
            p.contexts = match expression {
                "https://example.org/parent#part" => &[("extension", "https://example.org/parent#part")],
                "https://example.org/parent#other" => &[("extension", "https://example.org/parent#other")],
                _ => &[("extension", "https://example.org/other#part")],
            };
            assert_eq!(context_allows(&p, &names, &w), Ok(expected));
        }
    }
}

#[cfg(test)]
mod datatype_profile_tests {
    use crate::typemap::{DATATYPE_PROFILES, ELEMENT_PROFILES};

    /// The table the walk binary-searches is sorted, names a profile that
    /// exists, and every row is an element the type table reads as the
    /// datatype the profile constrains — so each row is one the walk meets.
    #[test]
    fn every_element_profile_row_is_reachable_and_sorted() {
        assert!(ELEMENT_PROFILES.windows(2).all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1)), "sorted, no duplicates");
        for (ctx, key, i) in ELEMENT_PROFILES {
            let (name, _, ty, _, _) = DATATYPE_PROFILES[*i as usize];
            let child = crate::typing::child(ctx, key);
            assert_eq!(child.map(|c| c.ty), Some(ty), "{ctx}.{key} is a {ty} (for {name})");
        }
    }

    /// R4 puts `SimpleQuantity` on 55 base elements and `MoneyQuantity` on
    /// none; both are carried, the second for an element that names it.
    #[test]
    fn simple_quantity_on_55_elements_money_quantity_on_none() {
        let index = |n: &str| DATATYPE_PROFILES.iter().position(|(name, ..)| *name == n).unwrap() as u8;
        let (simple, money) = (index("SimpleQuantity"), index("MoneyQuantity"));
        assert_eq!(ELEMENT_PROFILES.iter().filter(|(_, _, i)| *i == simple).count(), 55);
        assert_eq!(ELEMENT_PROFILES.iter().filter(|(_, _, i)| *i == money).count(), 0);
        let (_, url, _, prohibited, invariants) = DATATYPE_PROFILES[simple as usize];
        assert_eq!(url, "http://hl7.org/fhir/StructureDefinition/SimpleQuantity|4.0.1");
        assert_eq!(prohibited, &["comparator"]);
        assert_eq!(invariants.iter().map(|(k, ..)| *k).collect::<alloc::vec::Vec<_>>(), ["sqty-1"]);
    }
}
