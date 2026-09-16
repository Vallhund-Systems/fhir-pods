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
    pub fn doc(&self) -> TreeDoc<'_, 'a> {
        TreeDoc { root: &self.value, ctx: resource_ctx(&self.value) }
    }
    /// A top-level member by key (the JSON type tag lives here).
    pub fn top_member(&self, key: &str) -> Option<&Value<'a>> {
        match &self.value {
            Value::Object(kvs) => kvs.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v),
            _ => None,
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
/// context it was reached through (`None` when unknown).
#[derive(Debug, Clone, Copy)]
pub struct TreeDoc<'v, 'a> {
    root: &'v Value<'a>,
    ctx: Option<&'static str>,
}

impl<'v, 'a> TreeDoc<'v, 'a> {
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
        json::resolve_each(self.root, parent, &mut |p| {
            let mut n = 0usize;
            json::for_each_child(p, last, &mut |_| n += 1);
            out.push(n);
        });
        out
    }

    fn scalars(&self, path: &str) -> Vec<&str> {
        let mut out = Vec::new();
        json::resolve_each(self.root, path, &mut |v| {
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
        json::resolve_each(self.root, path, &mut |n| {
            out.push(TreeDoc { root: n, ctx: if ctx == Some("Resource") { resource_ctx(n) } else { ctx } })
        });
        out
    }

    fn child_instances(&self, key: &str) -> Vec<Self> {
        let ctx = match self.ctx {
            Some(c) if !key.ends_with("[x]") => crate::typing::child(c, key).map(|ch| ch.ctx),
            _ => None,
        };
        let mut out = Vec::new();
        json::for_each_child(self.root, key, &mut |n| {
            out.push(TreeDoc { root: n, ctx: if ctx == Some("Resource") { resource_ctx(n) } else { ctx } })
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
            Value::Array(xs) => out.extend(xs.iter().map(|x| TreeDoc { root: x, ctx: Self::child_ctx(ctx, key, x) })),
            v => out.push(TreeDoc { root: v, ctx: Self::child_ctx(ctx, key, v) }),
        };
        match self.root {
            // `resourceType` is the JSON type tag, not an element
            Value::Object(kvs) => {
                kvs.iter().filter(|(k, _)| k.as_ref() != "resourceType").for_each(|(k, v)| push(k, v))
            }
            Value::Array(xs) => xs.iter().for_each(|v| push("", v)),
            _ => {}
        }
        out
    }

    fn fhir_type(&self) -> Option<&'static str> {
        self.ctx.map(|c| if c.contains('.') { "BackboneElement" } else { c })
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
                Value::Array(xs) => out.extend(xs.iter().map(|x| TreeDoc { root: x, ctx })),
                v => out.push(TreeDoc { root: v, ctx }),
            }
        }
        out
    }
}
