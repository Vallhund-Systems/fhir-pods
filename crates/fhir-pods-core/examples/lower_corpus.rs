//! The lowering proof. For every corpus example that declares a registry
//! profile, read the values at the profile's slots out of the example, lower
//! them, and write the result as `lowered-<name>.json` next to it — so the
//! conformance harness can put every lowered resource in front of the
//! reference validator. Prints one line per example: whether our own
//! validation of the lowered resource is clean.
//!
//!     cargo run -p fhir-pods-core --example lower_corpus -- conformance/corpus
use std::{env, fs, process};

use fhir_pods_core::compiled::{CompiledProfile, Discriminator, Slice};
use fhir_pods_core::document::{Document, Parsed, TreeDoc};
use fhir_pods_core::json::Value;
use fhir_pods_core::{generated, wire, Validator};

/// The value at a labelled slot path inside an example, as JSON text: an
/// unlabelled segment is a plain member; a labelled one picks the array
/// element whose fixed leaves (from the profile) match the slice.
/// The value at a labelled slot path inside an example, as JSON text. A
/// labelled segment (`component:SystolicBP`) selects the array element the
/// profile's own slicing assigns to that slice — the engine's discriminators,
/// not a guess. `sibling_slices` are the labelled slices under the same
/// element: an unlabelled repeating slot takes every item that belongs to
/// none of them, so a slice instance is never lowered twice.
fn extract(doc: &TreeDoc<'_, '_>, profile: &CompiledProfile, path: &str, sibling_slices: &[&str]) -> Option<String> {
    let mut nodes: Vec<TreeDoc<'_, '_>> = vec![*doc];
    let mut element = String::new(); // the unlabelled element path so far, for slicing lookup
    let segs: Vec<&str> = path.split('.').collect();
    for (i, seg) in segs.iter().enumerate() {
        let (name, label) = seg.split_once(':').map(|(n, l)| (n, Some(l))).unwrap_or((seg, None));
        if !element.is_empty() {
            element.push('.');
        }
        element.push_str(name);
        let last = i + 1 == segs.len();
        let mut next: Vec<TreeDoc<'_, '_>> = Vec::new();
        for n in &nodes {
            // only the member itself — never the `_name` extension carrier the
            // document port falls back to (that is extracted under its own key)
            let present = matches!(n.value(), Value::Object(kvs) if kvs.iter().any(|(k, _)| k.as_ref() == name));
            if present {
                next.extend(n.child_instances(name));
            }
        }
        if next.is_empty() {
            return None;
        }
        match label {
            Some(l) => {
                let slice = find_slice(profile, &element, l)?;
                next.retain(|inst| in_slice(slice, inst));
                if next.is_empty() {
                    return None;
                }
            }
            None if last && (next.len() > 1 || is_repeating(profile, &element)) => {
                // an unlabelled repeating slot: every item no sibling slice claims
                next.retain(|inst| {
                    !sibling_slices.iter().any(|sl| {
                        let l = sl.split_once(':').map(|(_, l)| l).unwrap_or("");
                        find_slice(profile, &element, l).is_some_and(|slice| in_slice(slice, inst))
                    })
                });
                if next.is_empty() {
                    return None;
                }
                let mut out = String::from("[");
                for (k, inst) in next.iter().enumerate() {
                    if k > 0 {
                        out.push(',');
                    }
                    write(inst.value(), &mut out);
                }
                out.push(']');
                return Some(out);
            }
            None => {}
        }
        nodes = next;
    }
    let mut out = String::new();
    write(nodes.first()?.value(), &mut out);
    Some(out)
}

/// Does the base spec make `element` (unlabelled, resource-relative) an array?
fn is_repeating(profile: &CompiledProfile, element: &str) -> bool {
    let mut ctx = Some(profile.resource_type);
    let segs: Vec<&str> = element.split('.').collect();
    for (i, seg) in segs.iter().enumerate() {
        let Some(c) = ctx else { return false };
        if i + 1 == segs.len() {
            return fhir_pods_core::typing::repeats(c, seg);
        }
        ctx = fhir_pods_core::typing::child(c, seg).map(|ch| ch.ctx);
    }
    false
}

/// The compiled slice for `element` (unlabelled, resource-relative) named `label`.
fn find_slice<'p>(profile: &'p CompiledProfile, element: &str, label: &str) -> Option<&'p Slice> {
    let full = format!("{}.{}", profile.resource_type, element);
    profile.slicings.iter().find(|s| s.path == full).and_then(|s| s.slices.iter().find(|sl| sl.name == label))
}

fn in_slice(slice: &Slice, inst: &TreeDoc<'_, '_>) -> bool {
    slice.discriminators.iter().all(|d| match d {
        Discriminator::Value { path, value } => inst.scalars(path).contains(value),
        Discriminator::Pattern { path, pairs } => inst.instances(path).iter().any(|n| {
            pairs.iter().all(|(rel, v)| {
                let p = if rel.is_empty() { "$this".to_string() } else { format!("$this.{rel}") };
                n.scalars(&p).contains(v)
            })
        }),
        Discriminator::Type { .. } => true,
        Discriminator::ResourceType { path, ty } => {
            inst.instances(path).iter().any(|r| r.scalars("$this.resourceType") == [*ty])
        }
        // a Bundle entry or a section entry discriminated by profile or by what a
        // reference resolves to: documents are assembled, not lowered
        Discriminator::Profile { .. } | Discriminator::ResolvedType { .. } | Discriminator::ResolvedProfile { .. } => {
            false
        }
    })
}

fn write(v: &Value<'_>, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(n),
        Value::Str(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        Value::Array(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(x, out);
            }
            out.push(']');
        }
        Value::Object(kvs) => {
            out.push('{');
            for (i, (k, v)) in kvs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('"');
                out.push_str(k);
                out.push_str("\":");
                write(v, out);
            }
            out.push('}');
        }
    }
}

fn main() {
    let dir = env::args().nth(1).unwrap_or_else(|| "conformance/corpus".into());
    let validator = Validator::new();
    let (mut n, mut clean) = (0, 0);
    let mut manifest_lines = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&dir).expect("corpus dir").flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".json")
            || name.starts_with('_')
            || name.ends_with("manifest.json")
            || name.starts_with("lowered-")
            || name.starts_with("deid-")
        {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let Ok(parsed) = Parsed::parse(&bytes) else { continue };
        let doc = parsed.doc();
        let declared = doc.scalars("$this.meta.profile");
        let Some(profile) =
            generated::REGISTRY.iter().find(|p| declared.iter().any(|u| u.split('|').next() == Some(p.url)))
        else {
            continue;
        };
        // only publisher-valid examples are lowered: a broken probe's slots are broken too;
        // a document (Bundle, Composition) is assembled by the document pod, not lowered from slots
        if !validator.validate(&bytes).is_valid() || matches!(profile.resource_type, "Bundle" | "Composition") {
            continue;
        }
        let mut inputs = String::from("{");
        let mut first = true;
        // the members every resource may carry, when the example does (its id,
        // narrative, extensions)
        for key in ["id", "text", "extension", "modifierExtension", "language"] {
            if let Some(v) = extract(&doc, profile, key, &[]) {
                if !first {
                    inputs.push(',');
                }
                first = false;
                inputs.push('"');
                inputs.push_str(key);
                inputs.push_str("\":");
                inputs.push_str(&v);
            }
        }
        let slots_start = inputs.len();
        for slot in profile.lowering.slots {
            let keys: Vec<String> = if slot.ty == "choice" {
                let prefix = slot.path.rsplit_once('.').map(|(p, _)| format!("{p}.")).unwrap_or_default();
                slot.choices.iter().map(|c| format!("{prefix}{c}")).collect()
            } else {
                vec![slot.path.to_string()]
            };
            // the labelled slices under this element (`identifier:ihi`), from any slot inside them
            let mut siblings: Vec<&str> = profile
                .lowering
                .slots
                .iter()
                .filter_map(|s| {
                    let first = s.path.split('.').next()?;
                    let (base, _) = first.split_once(':')?;
                    (base == slot.path).then_some(first)
                })
                .collect();
            // (a slice with no slot of its own is lowered through the plain slot,
            // carrying its own system — so it is not "claimed")
            siblings.sort();
            siblings.dedup();
            for key in keys {
                if let Some(v) = extract(&doc, profile, &key, &siblings) {
                    if !first {
                        inputs.push(',');
                    }
                    first = false;
                    inputs.push('"');
                    inputs.push_str(&key);
                    inputs.push_str("\":");
                    inputs.push_str(&v);
                }
                // the primitive-extension sibling (`_birthDate`), when the example carries one
                let (parent, leaf) = key.rsplit_once('.').unwrap_or(("", &key));
                let ext_key = if parent.is_empty() { format!("_{leaf}") } else { format!("{parent}._{leaf}") };
                if let Some(v) = extract(&doc, profile, &ext_key, &[]) {
                    if !first {
                        inputs.push(',');
                    }
                    first = false;
                    inputs.push('"');
                    inputs.push_str(&ext_key);
                    inputs.push_str("\":");
                    inputs.push_str(&v);
                }
            }
        }
        // contained resources travel with the example only where a slot value
        // still reaches them (`#id`); one only a non-slot member referenced
        // would be an orphan in the lowered resource (dom-3)
        let contained: Vec<TreeDoc<'_, '_>> = doc.child_instances("contained");
        let reached: Vec<String> = contained
            .iter()
            .filter(|c| {
                let id = c.scalars("$this.id");
                id.first().is_some_and(|id| inputs[slots_start..].contains(&format!("\"#{id}\"")))
            })
            .map(|c| {
                let mut out = String::new();
                write(c.value(), &mut out);
                out
            })
            .collect();
        if !reached.is_empty() {
            if !first {
                inputs.push(',');
            }
            inputs.push_str("\"contained\":[");
            inputs.push_str(&reached.join(","));
            inputs.push(']');
        }
        inputs.push('}');
        let lowered = validator.lower(profile.url, inputs.as_bytes()).expect("lowerable");
        n += 1;
        let ok = lowered.validation.is_valid();
        if ok {
            clean += 1;
        }
        let out = path.with_file_name(format!("lowered-{name}"));
        fs::write(&out, format!("{}\n", lowered.resource)).unwrap();
        manifest_lines.push(format!(
            "{{\"file\": \"lowered-{name}\", \"profile\": \"{}\", \"expect\": true, \"ig\": \"lowered\"}}",
            profile.url
        ));
        let status = if ok { "ok".to_string() } else { wire::report(&lowered.validation) };
        println!("{:<60} {status}", format!("lowered-{name}"));
    }
    fs::write(format!("{dir}/lowered-manifest.json"), format!("[\n{}\n]\n", manifest_lines.join(",\n"))).unwrap();
    println!("lowered {n} examples; {clean} validate clean on our side");
    if clean != n {
        process::exit(1);
    }
}
