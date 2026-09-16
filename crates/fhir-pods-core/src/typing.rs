//! Typed traversal: a JSON node learns its FHIR type from the table as it is
//! reached — root `resourceType`, then element by element (choice keys,
//! backbone paths, content references, contained resources). What lets
//! `descendants().as(canonical)`, `value.ofType(Quantity)` and `$this is
//! dateTime` work on arbitrary nodes without the document carrying types.
//! Under a resource type the table doesn't cover, a node has no type and
//! type-dependent FHIRPath is skipped, never guessed.

use crate::typemap::{TABLE, TYPES};

/// A resource or datatype name the table knows.
pub fn known_type(name: &str) -> Option<&'static str> {
    TYPES.binary_search(&name).ok().map(|i| TYPES[i])
}

/// `p.cmp(&format!("{ctx}.{key}"))` without building the string.
fn cmp_joined(p: &str, ctx: &str, key: &str) -> core::cmp::Ordering {
    use core::cmp::Ordering::*;
    let pb = p.as_bytes();
    let mut i = 0;
    for part in [ctx.as_bytes(), b".", key.as_bytes()] {
        for &x in part {
            match pb.get(i) {
                None => return Less,
                Some(&y) if y != x => return y.cmp(&x),
                _ => {}
            }
            i += 1;
        }
    }
    if pb.len() > i {
        Greater
    } else {
        Equal
    }
}

fn lookup2(ctx: &str, key: &str) -> Option<&'static (&'static str, &'static str)> {
    TABLE.binary_search_by(|(p, _)| cmp_joined(p, ctx, key)).ok().map(|i| &TABLE[i])
}

/// A child's typing: `ctx` is what its own children resolve against (a
/// type name, or a backbone path like `Observation.component`); `ty` is
/// its FHIR type name for `is` / `as` / `ofType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Child {
    pub ctx: &'static str,
    pub ty: &'static str,
}

/// The typing of element `key` under `ctx`. `Child { ty: "Resource" }`
/// means "whatever the child's own `resourceType` says" (contained,
/// Bundle.entry.resource) — the caller resolves it from the node.
pub fn child(ctx: &'static str, key: &str) -> Option<Child> {
    match key {
        "id" => {
            let ty = if ctx.contains('.') || crate::typemap::RESOURCES.binary_search(&ctx).is_err() {
                "string"
            } else {
                "id"
            };
            return Some(Child { ctx: ty, ty });
        }
        "extension" | "modifierExtension" => return Some(Child { ctx: "Extension", ty: "Extension" }),
        k if k.starts_with('_') => return Some(Child { ctx: "Element", ty: "Element" }),
        _ => {}
    }
    let (path, ty) = match lookup2(ctx, key) {
        Some((p, t)) => (*p, *t),
        None => choice(ctx, key)?,
    };
    Some(resolve(path, ty))
}

/// `valueQuantity` under `Observation` → the `Observation.value[x]` entry
/// with `Quantity` among its types.
fn choice(ctx: &str, key: &str) -> Option<(&'static str, &'static str)> {
    let start = TABLE.partition_point(|(p, _)| cmp_joined(p, ctx, "") == core::cmp::Ordering::Less);
    for (path, tys) in &TABLE[start..] {
        let Some(rest) = path.strip_prefix(ctx).and_then(|r| r.strip_prefix('.')) else { break };
        let Some(stem) = rest.strip_suffix("[x]") else { continue };
        if stem.contains('.') {
            continue;
        }
        let Some(suffix) = key.strip_prefix(stem) else { continue };
        if suffix.is_empty() {
            continue;
        }
        for t in tys.split('|') {
            let (first, tail) = t.split_at(1);
            if suffix.len() == t.len() && suffix[..1].eq_ignore_ascii_case(first) && &suffix[1..] == tail {
                return Some((path, t));
            }
        }
    }
    None
}

fn resolve(path: &'static str, ty: &'static str) -> Child {
    if let Some(cref) = ty.strip_prefix('#') {
        return Child { ctx: cref, ty: "BackboneElement" };
    }
    // a choice list that was not narrowed (path given with `[x]`): untypable
    let ty = ty.split('|').next().unwrap_or(ty);
    match ty {
        "BackboneElement" | "Element" => Child { ctx: path, ty },
        "Resource" | "DomainResource" => Child { ctx: "Resource", ty: "Resource" },
        t => Child { ctx: known_type(t).unwrap_or(t), ty: t },
    }
}
