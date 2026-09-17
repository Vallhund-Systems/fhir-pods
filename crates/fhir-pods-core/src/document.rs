//! The `Document` port: how the engine reads a resource, independent of the
//! JSON representation. `TreeDoc` is today's adapter — a cheap view that can be
//! re-rooted at any node, which is what slicing needs.

use alloc::vec::Vec;

use crate::json::{self, Value};

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
        let (gaps, nested, _) = self.structure_full();
        (gaps, nested)
    }

    /// [`structure`](Self::structure), plus every extension instance whose
    /// `url` names a compiled extension profile, at its label, with that
    /// profile — each is validated against it, wherever it sits.
    #[allow(clippy::type_complexity)]
    pub fn structure_full(
        &self,
    ) -> (
        Vec<Gap>,
        Vec<(alloc::string::String, TreeDoc<'_, 'a>)>,
        Vec<(alloc::string::String, TreeDoc<'_, 'a>, &'static crate::compiled::CompiledProfile)>,
    ) {
        let Some(rt) = resource_ctx(&self.value) else { return (Vec::new(), Vec::new(), Vec::new()) };
        let mut w = Walk {
            root: rt,
            gaps: Vec::new(),
            nested: Vec::new(),
            extensions: Vec::new(),
            ctxs: Vec::new(),
            segs: Vec::new(),
            profiled: Vec::new(),
            profiles: Vec::new(),
            contained: Vec::new(),
        };
        structure_in(&self.value, rt, rt, &mut w);
        let nested = w
            .nested
            .into_iter()
            .filter_map(|(label, v)| resource_ctx(v).map(|rt| (label, TreeDoc::at(v, Some(rt)))))
            .collect();
        let extensions =
            w.extensions.into_iter().map(|(label, v, p)| (label, TreeDoc::at(v, Some("Extension")), p)).collect();
        (w.gaps, nested, extensions)
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
        // R4 JSON: a string primitive is never empty
        (_, Value::Str(s)) => !s.is_empty(),
        _ => true,
    }
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
                crate::generated::REGISTRY.iter().find(|c| c.url == base && Some(c.resource_type) == rt).copied()
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
            crate::generated::REGISTRY.iter().any(|c| c.url == base && Some(c.resource_type) == rt)
        }
        _ => false,
    })
}

/// The shape walk's state: the findings, every nested resource found (with
/// the indexed element path it sits at), and the path segments so far —
/// `(key, index)` for a repeating element — rendered only when a nested
/// resource is found, so the walk itself never formats a path per element.
struct Walk<'v, 'a> {
    root: &'static str,
    gaps: Vec<Gap>,
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
    /// innermost last: what a local reference (`#id`) must name (ref-1).
    contained: Vec<Vec<alloc::string::String>>,
    /// Every object on the way down (root first): its typing context (a
    /// resource type, a datatype, a backbone path, `Element` for a
    /// primitive's carrier) and the JSON key it was reached by — what an
    /// extension's context is judged against.
    ctxs: Vec<(&'static str, &'v str)>,
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

impl<'v, 'a> Walk<'v, 'a> {
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

fn structure_in<'v, 'a>(v: &'v Value<'a>, ctx: &'static str, path: &str, w: &mut Walk<'v, 'a>) {
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
    w: &mut Walk<'v, 'a>,
) {
    w.ctxs.push((ctx, key));
    structure_object(v, ctx, path, contained, needs_content, w);
    w.ctxs.pop();
}

/// The element paths an extension's owner answers to: the owner's type, the
/// path from its resource (`Patient.telecom.value`) and from its datatype
/// (`ContactPoint.value`), with the base types everything is.
fn owner_names(w: &Walk<'_, '_>) -> Vec<alloc::string::String> {
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
    if let Some(r) = (0..=owner).rev().find(|&i| is_resource(w.ctxs[i].0)) {
        names.push(path_from(r));
    }
    if let Some(d) = (0..=owner).rev().find(|&i| is_datatype(w.ctxs[i].0)) {
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

/// Does an extension's definition allow it where it sits? A `fhirpath`
/// context is not evaluated (allowed); an `extension` context is met by
/// sitting inside an extension.
fn context_allows(p: &crate::compiled::CompiledProfile, names: &[alloc::string::String], owner_ctx: &str) -> bool {
    p.contexts.iter().any(|(kind, expr)| match *kind {
        "element" => names.iter().any(|n| n == expr),
        "extension" => owner_ctx == "Extension",
        _ => true,
    })
}

fn structure_object<'v, 'a>(
    v: &'v Value<'a>,
    ctx: &'static str,
    path: &str,
    contained: bool,
    needs_content: bool,
    w: &mut Walk<'v, 'a>,
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
    for child in crate::typing::required_children(ctx) {
        if !has(child) {
            w.gaps.push((path.into(), Required, alloc::format!("{ctx}.{child} is required (min 1, found 0)")));
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
        w.contained.push(ids);
        if w.profiled.last() == Some(&false) {
            for child in crate::typing::required_resource_children(ctx) {
                if !has(child) {
                    w.gaps.push((path.into(), Required, alloc::format!("{ctx}.{child} is required (min 1, found 0)")));
                }
            }
        }
    } else if ctx.contains('.') && w.profiled.last() == Some(&false) {
        for child in crate::typing::required_backbone_children(ctx) {
            if !has(child) {
                w.gaps.push((path.into(), Required, alloc::format!("{ctx}.{child} is required (min 1, found 0)")));
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
        if let Some((_, Value::Str(r))) = kvs.iter().find(|(k, _)| k.as_ref() == "reference") {
            if let Some(local) = r.strip_prefix('#') {
                if !local.is_empty() && !w.contained.iter().any(|ids| ids.iter().any(|id| id == local)) {
                    w.gaps.push((
                        alloc::format!("{path}.reference"),
                        Invariant,
                        alloc::format!("ref-1: reference '{r}' names no contained resource of {}", w.root),
                    ));
                }
            }
        }
    }
    // an identifier whose system is `urn:ietf:rfc:3986` carries a URI: its
    // value must have a scheme (the reference validator's rule, an error)
    if ctx == "Identifier" {
        let system = kvs.iter().find(|(k, _)| k.as_ref() == "system");
        if matches!(system, Some((_, Value::Str(s))) if s.as_ref() == "urn:ietf:rfc:3986") {
            if let Some((_, Value::Str(v))) = kvs.iter().find(|(k, _)| k.as_ref() == "value") {
                let scheme_end = v.find(':').unwrap_or(0);
                let has_scheme = scheme_end > 0
                    && v.as_bytes()[0].is_ascii_alphabetic()
                    && v[..scheme_end].bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'));
                if !has_scheme {
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
            if let Some(p) = crate::generated::REGISTRY.iter().find(|p| p.resource_type == "Extension" && p.url == base)
            {
                let owner_ctx = w.ctxs.len().checked_sub(2).map(|i| w.ctxs[i].0).unwrap_or("");
                let names = owner_names(w);
                if !p.contexts.is_empty() && !context_allows(p, &names, owner_ctx) {
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
                }
                let reached = w.profiles.last().is_some_and(|ps| ps.iter().any(|p| profile_reaches(p.slicings, base)));
                if !reached {
                    let label = w.label(w.root);
                    w.extensions.push((label, v, p));
                }
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
            for x in items {
                match x {
                    Value::Object(m) if m.is_empty() => w.gaps.push((
                        child_path.clone(),
                        Invariant,
                        alloc::format!("ele-1: {child_path}'s extension carrier must have some content"),
                    )),
                    Value::Object(_) => {
                        let paired = kvs.iter().any(|(k, _)| k.as_ref() == bare);
                        structure_at(x, "Element", &child_path, key, contained, !paired, w)
                    }
                    Value::Null if repeats => {}
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
                if matches!(x, Value::Null) && repeats && has(&alloc::format!("_{bare}")) {
                    w.segs.pop();
                    continue; // a placeholder beside its carrier
                }
                let expected = expected_kind(ch.ty);
                if json_kind(x) != expected {
                    w.gaps.push((
                        child_path.clone(),
                        ValueIssue,
                        alloc::format!("{child_path}: a {} is a JSON {expected}, found {}", ch.ty, json_kind(x)),
                    ));
                } else if !primitive_value_ok(ch.ty, x) {
                    w.gaps.push((
                        child_path.clone(),
                        ValueIssue,
                        alloc::format!("{child_path} is not a valid {}", ch.ty),
                    ));
                }
            } else {
                match x {
                    Value::Object(_) => structure_at(x, ch.ctx, &child_path, bare, contained, true, w),
                    other => w.gaps.push((
                        child_path.clone(),
                        ValueIssue,
                        alloc::format!("{child_path}: a {} is a JSON object, found {}", ch.ty, json_kind(other)),
                    )),
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
        kvs.iter()
            .map(|(k, _)| k.as_ref())
            .filter(|k| k.strip_prefix(stem).is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_uppercase())))
            .collect()
    }

    fn choice_children(&self, stem: &str) -> Vec<Self> {
        let Value::Object(kvs) = self.root else { return Vec::new() };
        let mut out = Vec::new();
        for (k, v) in kvs {
            let Some(rest) = k.strip_prefix(stem) else { continue };
            if !rest.starts_with(|c: char| c.is_ascii_uppercase()) {
                continue;
            }
            let ctx = Self::child_ctx(self.ctx, k, v);
            match v {
                Value::Array(xs) => out.extend(xs.iter().map(|x| TreeDoc::at(x, ctx))),
                v => out.push(TreeDoc::at(v, ctx)),
            }
        }
        out
    }
}
