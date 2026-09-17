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
        let mut out = Vec::new();
        if let Some(rt) = resource_ctx(&self.value) {
            structure_in(&self.value, rt, rt, &mut out);
        }
        out
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
        ("dateTime", Value::Str(s)) => {
            crate::temporal::parse(s).is_some_and(|t| t.has_date && (t.precision <= 3 || t.has_tz))
        }
        ("instant", Value::Str(s)) => crate::temporal::parse(s).is_some_and(|t| t.precision >= 6 && t.has_tz),
        ("time", Value::Str(s)) => crate::temporal::parse(s).is_some_and(|t| !t.has_date && t.precision >= 6),
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

fn structure_in(v: &Value<'_>, ctx: &'static str, path: &str, out: &mut Vec<Gap>) {
    structure_at(v, ctx, path, false, true, out)
}

/// `contained`: this object is a contained resource (or inside one), which
/// the base spec holds to `dom-2` / `dom-4` / `dom-5`. `needs_content`: ele-1
/// applies to this object on its own — false for a primitive's `_name`
/// carrier whose value is present (the element has its value).
fn structure_at(
    v: &Value<'_>,
    ctx: &'static str,
    path: &str,
    contained: bool,
    needs_content: bool,
    out: &mut Vec<Gap>,
) {
    use crate::validate::IssueType::{Invariant, Required, Structure, Value as ValueIssue};
    let Value::Object(kvs) = v else { return };
    let is_resource = crate::typemap::RESOURCES.binary_search(&ctx).is_ok();
    let has = |name: &str| kvs.iter().any(|(k, _)| k.as_ref() == name || k.strip_prefix('_') == Some(name));
    for child in crate::typing::required_children(ctx) {
        if !has(child) {
            out.push((path.into(), Required, alloc::format!("{ctx}.{child} is required (min 1, found 0)")));
        }
    }
    // ele-1: an element has a value or children — `{}` (or only an `id`) is neither
    if needs_content && !is_resource && !kvs.iter().any(|(k, _)| k.as_ref() != "id") {
        out.push((path.into(), Invariant, alloc::format!("ele-1: {path} must have a value or children")));
    }
    // ext-1: an extension carries a value or nested extensions, not both
    if ctx == "Extension" {
        let has_value = kvs.iter().any(|(k, _)| k.starts_with("value") && k.len() > 5);
        if has_value == has("extension") {
            out.push((
                path.into(),
                Invariant,
                alloc::format!("ext-1: {path} must have either extensions or a value, not both"),
            ));
        }
    }
    // a contained resource: no nesting, no versioning, no security labels
    if contained && is_resource {
        if has("contained") {
            out.push((
                path.into(),
                Invariant,
                alloc::format!("dom-2: {path} is contained and must not itself contain resources"),
            ));
        }
        if let Some((_, Value::Object(meta))) = kvs.iter().find(|(k, _)| k.as_ref() == "meta") {
            for (k, rule) in [("versionId", "dom-4"), ("lastUpdated", "dom-4"), ("security", "dom-5")] {
                if meta.iter().any(|(m, _)| m.as_ref() == k) {
                    out.push((
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
            out.push((child_path.clone(), Structure, alloc::format!("{child_path} is not an element of {ctx}")));
            continue;
        };
        let repeats = crate::typing::repeats(ctx, bare);
        // a primitive's extension carrier: `_name`, an Element (an array of them, nulls as placeholders)
        if key.starts_with('_') {
            if !is_primitive(ch.ty) {
                out.push((
                    child_path.clone(),
                    Structure,
                    alloc::format!("{child_path} is not a primitive: no `_` carrier"),
                ));
                continue;
            }
            let items = match items_of(value, repeats, &child_path) {
                Ok(items) => items,
                Err(gap) => {
                    out.push(gap);
                    continue;
                }
            };
            for x in items {
                match x {
                    Value::Object(_) => {
                        let paired = kvs.iter().any(|(k, _)| k.as_ref() == bare);
                        structure_at(x, "Element", &child_path, contained, !paired, out)
                    }
                    Value::Null if repeats => {}
                    other => out.push((
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
                out.push(gap);
                continue;
            }
        };
        for x in items {
            if ch.ty == "Resource" {
                match resource_ctx(x) {
                    Some(rt) if matches!(x, Value::Object(_)) => {
                        structure_at(x, rt, &child_path, contained || bare == "contained", true, out)
                    }
                    _ => {
                        out.push((child_path.clone(), Structure, alloc::format!("{child_path} is not a FHIR resource")))
                    }
                }
            } else if is_primitive(ch.ty) {
                if matches!(x, Value::Null) && repeats && has(&alloc::format!("_{bare}")) {
                    continue; // a placeholder beside its carrier
                }
                let expected = expected_kind(ch.ty);
                if json_kind(x) != expected {
                    out.push((
                        child_path.clone(),
                        ValueIssue,
                        alloc::format!("{child_path}: a {} is a JSON {expected}, found {}", ch.ty, json_kind(x)),
                    ));
                } else if !primitive_value_ok(ch.ty, x) {
                    out.push((child_path.clone(), ValueIssue, alloc::format!("{child_path} is not a valid {}", ch.ty)));
                }
            } else {
                match x {
                    Value::Object(_) => structure_at(x, ch.ctx, &child_path, contained, true, out),
                    other => out.push((
                        child_path.clone(),
                        ValueIssue,
                        alloc::format!("{child_path}: a {} is a JSON object, found {}", ch.ty, json_kind(other)),
                    )),
                }
            }
        }
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

    fn at(root: &'v Value<'a>, ctx: Option<&'static str>) -> Self {
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
        json::resolve_each(root, path, &mut |v| {
            if let Value::Str(s) = v {
                out.push(s.as_ref())
            }
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
