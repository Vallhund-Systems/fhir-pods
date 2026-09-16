//! Typed traversal: a JSON node learns its FHIR type from the table as it is
//! reached — root `resourceType`, then element by element (choice keys,
//! backbone paths, content references, contained resources). What lets
//! `descendants().as(canonical)`, `value.ofType(Quantity)` and `$this is
//! dateTime` work on arbitrary nodes without the document carrying types.
//! Under a resource type the table doesn't cover, a node has no type and
//! type-dependent FHIRPath is skipped, never guessed.
//!
//! The table (`typemap`) is packed: elements are grouped by their type and
//! keyed by the path *relative* to it, so a lookup is two small binary
//! searches (the type, then the element within it) and no string is built.

use crate::typemap::{CODES, RANGES, TABLE, TYPES};

/// One element of the table: its type codes and whether it repeats.
#[derive(Clone, Copy)]
struct Entry {
    codes: &'static str,
    repeats: bool,
}

/// `ctx` split into its type and the backbone path under it:
/// `Observation.component` → (`Observation`, `component`); `Quantity` → (`Quantity`, ``).
fn split_ctx(ctx: &str) -> (&str, &str) {
    ctx.split_once('.').unwrap_or((ctx, ""))
}

/// The table's span for type `ty`.
fn range_of(ty: &str) -> Option<&'static [(&'static str, u16, bool, bool)]> {
    let i = TYPES.binary_search(&ty).ok()?;
    let (start, end) = RANGES[i];
    Some(&TABLE[start as usize..end as usize])
}

/// `rel.cmp(&format!("{rest}.{key}"))` (or `rel.cmp(key)` when `rest` is
/// empty) without building the string.
fn cmp_rel(rel: &str, rest: &str, key: &str) -> core::cmp::Ordering {
    use core::cmp::Ordering::*;
    let rb = rel.as_bytes();
    let mut i = 0;
    let parts: [&[u8]; 3] =
        if rest.is_empty() { [b"", b"", key.as_bytes()] } else { [rest.as_bytes(), b".", key.as_bytes()] };
    for part in parts {
        for &x in part {
            match rb.get(i) {
                None => return Less,
                Some(&y) if y != x => return y.cmp(&x),
                _ => {}
            }
            i += 1;
        }
    }
    if rb.len() > i {
        Greater
    } else {
        Equal
    }
}

fn lookup(ctx: &str, key: &str) -> Option<Entry> {
    let (ty, rest) = split_ctx(ctx);
    let span = range_of(ty)?;
    let i = span.binary_search_by(|(rel, _, _, _)| cmp_rel(rel, rest, key)).ok()?;
    let (_, code, repeats, _) = span[i];
    Some(Entry { codes: CODES[code as usize], repeats })
}

/// Does element `key` under `ctx` repeat (base cardinality `max` ≠ 1)? An
/// element the table doesn't know is taken as single.
pub fn repeats(ctx: &str, key: &str) -> bool {
    // a slice label never changes the element; a primitive's extension
    // carrier (`_name`) has the shape of the primitive it carries
    let key = key.split(':').next().unwrap_or(key);
    let key = key.strip_prefix('_').unwrap_or(key);
    if matches!(key, "extension" | "modifierExtension" | "contained") {
        return true;
    }
    // (a concrete choice key resolves to its `[x]` entry: choices never repeat)
    lookup(ctx, key).is_some_and(|e| e.repeats)
}

/// The direct children a complex datatype requires (`Narrative.status`,
/// `Extension.url`, `Annotation.text`): what every instance of the type
/// must carry, profile or no profile. Empty for a resource type — a
/// resource's required elements come with its profile.
pub fn required_children(ty: &str) -> impl Iterator<Item = &'static str> {
    let span = if crate::typemap::RESOURCES.binary_search(&ty).is_ok() { &[][..] } else { range_of(ty).unwrap_or(&[]) };
    span.iter().filter(|(rel, _, _, required)| *required && !rel.contains('.')).map(|(rel, _, _, _)| *rel)
}

/// A resource or datatype name the table knows.
pub fn known_type(name: &str) -> Option<&'static str> {
    TYPES.binary_search(&name).ok().map(|i| TYPES[i])
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
    let codes = match lookup(ctx, key) {
        Some(e) => e.codes,
        None => choice(ctx, key)?,
    };
    Some(resolve(codes))
}

/// `valueQuantity` under `Observation` → the `Observation.value[x]` entry's
/// `Quantity` alternative.
fn choice(ctx: &str, key: &str) -> Option<&'static str> {
    let (ty, rest) = split_ctx(ctx);
    let span = range_of(ty)?;
    // the entries directly under `rest`: those sorting at or after `rest.`
    let start = span.partition_point(|(rel, _, _, _)| cmp_rel(rel, rest, "") == core::cmp::Ordering::Less);
    for (rel, code, _, _) in &span[start..] {
        let name = if rest.is_empty() {
            *rel
        } else {
            let Some(r) = rel.strip_prefix(rest).and_then(|r| r.strip_prefix('.')) else { break };
            r
        };
        let Some(stem) = name.strip_suffix("[x]") else { continue };
        if stem.contains('.') {
            continue;
        }
        let Some(suffix) = key.strip_prefix(stem) else { continue };
        if suffix.is_empty() {
            continue;
        }
        for t in CODES[*code as usize].split('|') {
            let (first, tail) = t.split_at(1);
            if suffix.len() == t.len() && suffix[..1].eq_ignore_ascii_case(first) && &suffix[1..] == tail {
                return Some(t);
            }
        }
    }
    None
}

fn resolve(codes: &'static str) -> Child {
    // a backbone (or content reference): the path is the children's context
    if let Some(path) = codes.strip_prefix('#') {
        return Child { ctx: path, ty: "BackboneElement" };
    }
    if let Some(path) = codes.strip_prefix('~') {
        return Child { ctx: path, ty: "Element" };
    }
    // a choice list that was not narrowed (path given with `[x]`): untypable
    let ty = codes.split('|').next().unwrap_or(codes);
    match ty {
        "Resource" | "DomainResource" => Child { ctx: "Resource", ty: "Resource" },
        t => Child { ctx: known_type(t).unwrap_or(t), ty: t },
    }
}
