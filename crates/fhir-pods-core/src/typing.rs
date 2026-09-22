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

/// The direct children a resource's backbone requires
/// (`QuestionnaireResponse.item` → `linkId`): the base's word inside a
/// resource, where a profile that applies restates it.
pub fn required_backbone_children(ctx: &str) -> alloc::vec::Vec<&'static str> {
    let (ty, rest) = split_ctx(ctx);
    let span = if rest.is_empty() || crate::typemap::RESOURCES.binary_search(&ty).is_err() {
        &[][..]
    } else {
        range_of(ty).unwrap_or(&[])
    };
    let start = span.partition_point(|(rel, _, _, _)| cmp_rel(rel, rest, "") == core::cmp::Ordering::Less);
    span[start..]
        .iter()
        .take_while(|(rel, _, _, _)| rel.strip_prefix(rest).is_some_and(|r| r.starts_with('.')))
        .filter_map(|(rel, _, _, required)| {
            let name = rel.strip_prefix(rest)?.strip_prefix('.')?;
            (*required && !name.contains('.')).then_some(name)
        })
        .collect()
}

/// The direct children a resource type itself requires (`Observation.status`,
/// `Observation.code`) — what the base resource says when no profile, which
/// restates them, applies.
pub fn required_resource_children(ty: &str) -> impl Iterator<Item = &'static str> {
    let span = if crate::typemap::RESOURCES.binary_search(&ty).is_ok() { range_of(ty).unwrap_or(&[]) } else { &[][..] };
    span.iter().filter(|(rel, _, _, required)| *required && !rel.contains('.')).map(|(rel, _, _, _)| *rel)
}

/// A datatype's direct members, in table order: name, type code(s) (`|`
/// separated for a choice; `#path` for a backbone), whether it repeats,
/// whether the base requires it. Empty for a resource or an unknown type —
/// what a host needs to write a composite slot (`HumanName`, `Identifier`,
/// `Address`) without the specification open.
pub fn members(ty: &str) -> alloc::vec::Vec<(&'static str, &'static str, bool, bool)> {
    let span = if crate::typemap::RESOURCES.binary_search(&ty).is_ok() { &[][..] } else { range_of(ty).unwrap_or(&[]) };
    span.iter()
        .filter(|(rel, _, _, _)| !rel.contains('.'))
        .map(|(rel, code, repeats, required)| (*rel, CODES[*code as usize], *repeats, *required))
        .collect()
}

/// Every type name the table knows, for a lookup by other spellings.
pub fn type_names() -> impl Iterator<Item = &'static str> {
    TYPES.iter().copied()
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
    /// The `[x]` element this key is one choice of (`valueQuantity` → `value`).
    pub choice: Option<&'static str>,
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
            return Some(Child { ctx: ty, ty, choice: None });
        }
        "extension" | "modifierExtension" => return Some(Child { ctx: "Extension", ty: "Extension", choice: None }),
        k if k.starts_with('_') => return Some(Child { ctx: "Element", ty: "Element", choice: None }),
        _ => {}
    }
    let (codes, stem) = match lookup(ctx, key) {
        Some(e) => (e.codes, None),
        None => {
            let (stem, codes) = choice(ctx, key)?;
            (codes, Some(stem))
        }
    };
    let c = resolve(codes);
    Some(Child { choice: stem, ..c })
}

/// `valueQuantity` under `Observation` → the `Observation.value[x]` entry's
/// `Quantity` alternative.
fn choice(ctx: &str, key: &str) -> Option<(&'static str, &'static str)> {
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
                return Some((stem, t));
            }
        }
    }
    None
}

fn resolve(codes: &'static str) -> Child {
    // a backbone (or content reference): the path is the children's context
    if let Some(path) = codes.strip_prefix('#') {
        return Child { ctx: path, ty: "BackboneElement", choice: None };
    }
    if let Some(path) = codes.strip_prefix('~') {
        return Child { ctx: path, ty: "Element", choice: None };
    }
    // a choice list that was not narrowed (path given with `[x]`): untypable
    let ty = codes.split('|').next().unwrap_or(codes);
    match ty {
        "Resource" | "DomainResource" => Child { ctx: "Resource", ty: "Resource", choice: None },
        t => Child { ctx: known_type(t).unwrap_or(t), ty: t, choice: None },
    }
}
