//! Lifting — read a resource back into the profile's *variable* data: the
//! reverse of lowering.
//!
//! A compiled profile fixes most of a resource; the rest is its slots. Lift
//! reads the value at every slot out of a resource — the same slot vocabulary
//! lowering takes — so a host's map from FHIR to its own model is a map from
//! slot paths to fields, and nothing else. What the profile fixes is not
//! lifted (it is the profile's, not the record's); what no slot, fixed leaf
//! or pass-through member covers is listed in `extras`, so nothing in the
//! record is silently left behind; a required slot the record lacks is
//! listed in `missing`. The resource is validated against the profile on the
//! way in — a lifted record is never trusted unverified — and the verdict
//! comes back with the inputs.
//!
//! `lower(lift(r))` is a resource that validates like `r` did: the
//! conformance harness holds every corpus example to that.
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::compiled::{CompiledProfile, Discriminator, Slice};
use crate::document::{Document, Parsed, TreeDoc};
use crate::json::Value;
use crate::node::Node;
use crate::terminology::TerminologyProvider;
use crate::{generated, typing, Validation, Validator};

/// The outcome: the inputs (a JSON object keyed by slot path, as lowering
/// takes them), the resource's validation against the profile, the required
/// slots it lacked, and the element paths no slot covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lifted {
    pub inputs: String,
    pub validation: Validation,
    /// Required slots (`min >= 1`) the resource has no value for.
    pub missing: Vec<&'static str>,
    /// Element paths present in the resource that no slot, fixed leaf or
    /// pass-through member covers — not lifted, named.
    pub extras: Vec<String>,
}

/// Why lifting could not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiftError {
    /// No compiled profile with that canonical URL.
    UnknownProfile,
    /// No URL given and the resource declares no compiled profile.
    NoProfileDeclared,
    /// The input is not a FHIR resource (a JSON object with a `resourceType`).
    NotAResource,
    /// The profile is for another resource type.
    WrongResourceType,
}

impl core::fmt::Display for LiftError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            LiftError::UnknownProfile => "no compiled profile with that URL",
            LiftError::NoProfileDeclared => "no profile given and the resource declares none the registry has compiled",
            LiftError::NotAResource => "input is not a FHIR resource",
            LiftError::WrongResourceType => "the profile is for another resource type",
        })
    }
}

impl core::error::Error for LiftError {}

/// Members any resource may carry that no slot lists: lifted as given.
const PASS_THROUGH: &[&str] =
    &["id", "text", "contained", "extension", "modifierExtension", "language", "implicitRules"];

/// Members that are the profile's or the wire's, never the record's.
const NOT_DATA: &[&str] = &["resourceType", "meta"];

/// Does the base spec make `element` (unlabelled, resource-relative) an array?
fn is_repeating(resource_type: &'static str, element: &str) -> bool {
    let mut ctx = Some(resource_type);
    let segs: Vec<&str> = element.split('.').collect();
    for (i, seg) in segs.iter().enumerate() {
        let Some(c) = ctx else { return false };
        if i + 1 == segs.len() {
            return typing::repeats(c, seg);
        }
        ctx = typing::child(c, seg).map(|ch| ch.ctx);
    }
    false
}

/// The compiled slice for `element` (unlabelled, resource-relative) named `label`.
fn find_slice<'p>(profile: &'p CompiledProfile, element: &str, label: &str) -> Option<&'p Slice> {
    let full = alloc::format!("{}.{}", profile.resource_type, element);
    profile.slicings.iter().find(|s| s.path == full).and_then(|s| s.slices.iter().find(|sl| sl.name == label))
}

/// Does an instance belong to the slice, by the profile's own discriminators?
fn in_slice(slice: &Slice, inst: &TreeDoc<'_, '_>) -> bool {
    slice.discriminators.iter().all(|d| match d {
        Discriminator::Value { path, value } => inst.scalars(path).contains(value),
        Discriminator::Pattern { path, pairs } => inst.instances(path).iter().any(|n| {
            pairs.iter().all(|(rel, v)| {
                let p = if rel.is_empty() { "$this".to_string() } else { alloc::format!("$this.{rel}") };
                n.scalars(&p).contains(v)
            })
        }),
        Discriminator::Type { .. } => true,
        Discriminator::ResourceType { path, ty } => {
            inst.instances(path).iter().any(|r| r.scalars("$this.resourceType") == [*ty])
        }
        // a Bundle entry or a section entry discriminated by profile or by
        // what a reference resolves to: documents are assembled, not lifted
        Discriminator::Profile { .. } | Discriminator::ResolvedType { .. } | Discriminator::ResolvedProfile { .. } => {
            false
        }
    })
}

/// The value at a labelled slot path inside the resource. A labelled
/// segment (`component:SystolicBP`) selects the array element the profile's
/// own slicing assigns to that slice; `sibling_slices` are the labelled
/// slices under the same element, so an unlabelled repeating slot takes
/// only the items none of them claims and a slice instance is never lifted
/// twice.
fn extract(doc: &TreeDoc<'_, '_>, profile: &CompiledProfile, path: &str, sibling_slices: &[&str]) -> Option<Node> {
    let mut nodes: Vec<TreeDoc<'_, '_>> = alloc::vec![*doc];
    let mut element = String::new();
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
            // the member itself — never the `_name` carrier the document port
            // falls back to (that is lifted under its own key)
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
            None if last && (next.len() > 1 || is_repeating(profile.resource_type, &element)) => {
                next.retain(|inst| {
                    !sibling_slices.iter().any(|sl| {
                        let l = sl.split_once(':').map(|(_, l)| l).unwrap_or("");
                        find_slice(profile, &element, l).is_some_and(|slice| in_slice(slice, inst))
                    })
                });
                if next.is_empty() {
                    return None;
                }
                return Some(Node::Array(next.iter().map(|inst| Node::from_value(inst.value())).collect()));
            }
            None => {}
        }
        nodes = next;
    }
    nodes.first().map(|n| Node::from_value(n.value()))
}

fn unlabelled(path: &str) -> String {
    path.split('.').map(|seg| seg.split(':').next().unwrap_or(seg)).collect::<Vec<_>>().join(".")
}

/// Lift `resource` (of the profile's type) into the profile's slots.
pub fn lift(profile: &'static CompiledProfile, resource: &[u8], validator: &Validator) -> Result<Lifted, LiftError> {
    lift_with(profile, resource, validator, &crate::terminology::Bundled)
}

/// [`lift`] with a terminology provider for the validation.
pub fn lift_with(
    profile: &'static CompiledProfile,
    resource: &[u8],
    validator: &Validator,
    tx: &dyn TerminologyProvider,
) -> Result<Lifted, LiftError> {
    let parsed = Parsed::parse(resource).map_err(|_| LiftError::NotAResource)?;
    let Some(Value::Str(rt)) = parsed.top_member("resourceType") else { return Err(LiftError::NotAResource) };
    if profile.resource_type != rt.as_ref() {
        return Err(LiftError::WrongResourceType);
    }
    let doc = &parsed.doc();
    let mut inputs: Vec<(String, Node)> = Vec::new();
    let mut covered: Vec<String> = Vec::new(); // unlabelled paths a slot, a fixed leaf or a pass-through covers
    let mut missing: Vec<&'static str> = Vec::new();

    for key in PASS_THROUGH {
        if let Some(v) = extract(doc, profile, key, &[]) {
            inputs.push((String::from(*key), v));
        }
        covered.push(String::from(*key));
    }
    for key in NOT_DATA {
        covered.push(String::from(*key));
    }
    for (path, _) in profile.lowering.fixed {
        let plain = unlabelled(path);
        // a fixed coding's display is the seam's (filled at lowering), not the record's
        if let Some(coding) = plain.strip_suffix(".code") {
            covered.push(alloc::format!("{coding}.display"));
        }
        covered.push(plain);
    }
    for slot in profile.lowering.slots {
        let keys: Vec<String> = if slot.ty == "choice" {
            let prefix = slot.path.rsplit_once('.').map(|(p, _)| alloc::format!("{p}.")).unwrap_or_default();
            slot.choices.iter().map(|c| alloc::format!("{prefix}{c}")).collect()
        } else {
            alloc::vec![slot.path.to_string()]
        };
        // the labelled slices under this element, from any slot inside them
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
        siblings.sort();
        siblings.dedup();
        let mut found = false;
        for key in &keys {
            covered.push(unlabelled(key));
            // a slot with slots of its own beneath it (`valueQuantity` over
            // `valueQuantity.value`) is lifted through those: the parts are
            // the record's, the rest of the container is the profile's
            let has_parts = profile
                .lowering
                .slots
                .iter()
                .any(|s| s.path.strip_prefix(key.as_str()).is_some_and(|r| r.starts_with('.')));
            if has_parts {
                found = true;
                continue;
            }
            if let Some(v) = extract(doc, profile, key, &siblings) {
                inputs.push((key.clone(), v));
                found = true;
            }
            // the primitive's extension carrier (`_birthDate`), when the resource carries one
            let (parent, leaf) = key.rsplit_once('.').unwrap_or(("", key));
            let ext_key =
                if parent.is_empty() { alloc::format!("_{leaf}") } else { alloc::format!("{parent}._{leaf}") };
            covered.push(unlabelled(&ext_key));
            if let Some(v) = extract(doc, profile, &ext_key, &[]) {
                inputs.push((ext_key, v));
                found = true;
            }
        }
        if !found && slot.min >= 1 {
            missing.push(slot.path);
        }
    }

    // what the record carries that nothing above covers
    let mut extras = Vec::new();
    if let Value::Object(kvs) = doc.value() {
        walk_extras(kvs, "", &covered, &mut extras);
    }
    extras.sort();
    extras.dedup();

    let inputs = Node::Object(inputs).to_json();
    // the verdict: the resource as it declares itself, and against this
    // profile when it does not declare it
    let mut validation = validator.validate_with(resource, tx);
    if !validation.profiles.contains(&profile.name) {
        let report = crate::engine::validate_at(profile, doc, doc, tx, 0, Some(validator), None);
        validation.profiles.push(profile.name);
        if validation.profile.is_none() {
            validation.profile = Some(profile.name);
        }
        validation.report.issues.extend(report.issues);
    }
    Ok(Lifted { inputs, validation, missing, extras })
}

/// Element paths under `kvs` (at `prefix`) that no covered path reaches:
/// a member is covered when a covered path equals it or lies below it, or
/// when it lies below a covered path (the whole subtree is a slot).
fn walk_extras(
    kvs: &[(alloc::borrow::Cow<'_, str>, Value<'_>)],
    prefix: &str,
    covered: &[String],
    out: &mut Vec<String>,
) {
    for (key, value) in kvs {
        let bare = key.strip_prefix('_').unwrap_or(key);
        let path = if prefix.is_empty() { String::from(bare) } else { alloc::format!("{prefix}.{bare}") };
        let exact_or_below = covered.iter().any(|c| *c == path || path.starts_with(&alloc::format!("{c}.")));
        if exact_or_below {
            continue;
        }
        let ancestor = covered.iter().any(|c| c.starts_with(&alloc::format!("{path}.")));
        if !ancestor {
            out.push(path);
            continue;
        }
        let items: Vec<&Value<'_>> = match value {
            Value::Array(xs) => xs.iter().collect(),
            v => alloc::vec![v],
        };
        for item in items {
            if let Value::Object(inner) = item {
                walk_extras(inner, &path, covered, out);
            }
        }
    }
}

impl Validator {
    /// Lift a resource into a profile's slots: `url` names a compiled
    /// profile, or is empty to take the most specific compiled profile the
    /// resource declares. See [`lift`](crate::lift).
    pub fn lift(&self, url: &str, resource: &[u8]) -> Result<Lifted, LiftError> {
        self.lift_with(url, resource, &crate::terminology::Bundled)
    }

    /// [`Validator::lift`] with a terminology provider for the validation.
    pub fn lift_with(&self, url: &str, resource: &[u8], tx: &dyn TerminologyProvider) -> Result<Lifted, LiftError> {
        let profile: &'static CompiledProfile = if url.is_empty() {
            let parsed = Parsed::parse(resource).map_err(|_| LiftError::NotAResource)?;
            let Some(Value::Str(rt)) = parsed.top_member("resourceType") else { return Err(LiftError::NotAResource) };
            let doc = parsed.doc();
            *crate::select_profiles(rt, &doc, &self.defaults).first().ok_or(LiftError::NoProfileDeclared)?
        } else {
            let url = url.split('|').next().unwrap_or(url);
            generated::REGISTRY.iter().find(|p| p.url == url).copied().ok_or(LiftError::UnknownProfile)?
        };
        lift_with(profile, resource, self, tx)
    }
}
