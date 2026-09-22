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
use crate::{typing, Validation, Validator};

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
    /// pass-through member covers — not lifted, named. An item of a
    /// repeating element that no slice claims and no slot takes is named
    /// with its index (`component[2]`, `code.coding[1]`).
    pub extras: Vec<String>,
    /// Values the resource carries at leaves the profile fixes, that
    /// disagree with the fixed value: the record is not this profile's
    /// kind as it stands (a blood pressure in kPa under a profile that
    /// fixes mm[Hg]) — lowering the inputs would replace the value, so a
    /// caller re-profiling must not.
    pub conflicts: Vec<Conflict>,
}

impl Lifted {
    /// Shared result shape for the C ABI and CLI, including fixed-value conflicts.
    pub fn to_json(&self) -> String {
        let missing = Node::Array(self.missing.iter().map(|p| Node::Str((*p).into())).collect());
        let extras = Node::Array(self.extras.iter().cloned().map(Node::Str).collect());
        let conflicts = Node::Array(
            self.conflicts
                .iter()
                .map(|c| {
                    Node::Object(alloc::vec![
                        ("path".into(), Node::Str(c.path.into())),
                        ("given".into(), Node::Str(c.given.clone())),
                        ("fixed".into(), Node::Str(c.fixed.into())),
                    ])
                })
                .collect(),
        );
        alloc::format!(
            "{{\"inputs\":{},\"validation\":{},\"missing\":{},\"extras\":{},\"conflicts\":{}}}",
            self.inputs,
            crate::wire::report(&self.validation),
            missing.to_json(),
            extras.to_json(),
            conflicts.to_json()
        )
    }
}

/// A record value that disagrees with what the profile fixes there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The labelled path of the fixed leaf.
    pub path: &'static str,
    /// What the record carries there, as text.
    pub given: String,
    /// What the profile fixes.
    pub fixed: &'static str,
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
/// `meta.profile` is the profile's; the rest of `meta` (security labels,
/// tags, source, version) is the record's and is lifted under `meta`.
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
    if let Some(Value::Object(meta)) = doc.instances("$this.meta").first().map(|m| m.value()) {
        let rest: Vec<(String, Node)> = meta
            .iter()
            .filter(|(k, _)| k.as_ref() != "profile")
            .map(|(k, v)| (k.to_string(), Node::from_value(v)))
            .collect();
        if !rest.is_empty() {
            inputs.push((String::from("meta"), Node::Object(rest)));
        }
    }
    // the optional gates (`agent:user`, `action`): a slice or element lowering
    // opens only when an input reaches into it
    let gate_of = |path: &str| -> Option<&'static str> {
        let mut prefix = String::new();
        let mut gate = None;
        for seg in path.split('.') {
            if !prefix.is_empty() {
                prefix.push('.');
            }
            prefix.push_str(seg);
            if let Some((p, 0)) = profile.lowering.slices.iter().find(|(p, _)| *p == prefix) {
                gate = Some(*p);
            }
        }
        gate
    };
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
            // Only a slot actually extracted whole covers its subtree. A
            // composite represented by child slots can still carry other
            // content (Quantity.comparator, extensions, primitive carriers).
            covered.push(unlabelled(key));
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
            // a labelled slot whose slice the profile does not compile (the
            // lowering table outstrips the slicing) is met by the element itself
            let label_unresolved = slot.path.split('.').find_map(|seg| seg.split_once(':')).is_some_and(|(base, _)| {
                let element = unlabelled(slot.path.split(':').next().unwrap_or(base));
                let label = slot.path.split(':').nth(1).map(|r| r.split('.').next().unwrap_or(r)).unwrap_or("");
                find_slice(profile, &element, label).is_none()
            });
            if label_unresolved && keys.iter().any(|k| extract(doc, profile, &unlabelled(k), &[]).is_some()) {
                continue;
            }
            // a slot behind an optional gate (`agent:user.who` when no user was
            // given) is required only once the gate is open
            if let Some(gate) = gate_of(slot.path) {
                let reached = inputs.iter().any(|(k, _)| k == gate || k.starts_with(&alloc::format!("{gate}.")));
                if !reached {
                    continue;
                }
            }
            missing.push(slot.path);
        }
    }

    // a fixed leaf behind an optional gate that the record carries, when no
    // slot input opens that gate (`action` of an audit record): lifted as
    // itself, so lowering the inputs opens the gate again
    for (path, value) in profile.lowering.fixed {
        let Some(gate) = gate_of(path) else { continue };
        // a gate with slots of its own (an NPI identifier's value) is opened by
        // those; only a gate holding nothing but fixed leaves needs the leaf
        let has_slots =
            profile.lowering.slots.iter().any(|s| s.path == gate || s.path.starts_with(&alloc::format!("{gate}.")));
        let reached = inputs.iter().any(|(k, _)| k == gate || k.starts_with(&alloc::format!("{gate}.")));
        // a slice (`identifier:NPI`, `extension:race`) is opened by what the
        // record's own instance carries; only a plain gated element applies
        if gate.contains(':') || has_slots || reached {
            continue;
        }
        if let Some(v) = extract(doc, profile, path, &[]) {
            let agrees = matches!(&v, Node::Str(s) if s == value)
                || matches!(&v, Node::Bool(b) if b.to_string() == *value)
                || matches!(&v, Node::Number(n) if n == value);
            if agrees {
                inputs.push((path.to_string(), v));
            }
        }
    }

    // the record's own values at the profile's fixed leaves, where they disagree
    let mut conflicts = Vec::new();
    for (path, value) in profile.lowering.fixed {
        // a leaf fixed under an unlabelled repeating element (a pattern on
        // `code.coding`: some coding must carry it) is one instance's; the
        // record's other instances are its own, and lowering adds the
        // profile's — only a single-valued place, or a slice's own
        // instance, can disagree
        let parent = path.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
        let last = parent.rsplit('.').next().unwrap_or("");
        if !parent.is_empty() && !last.contains(':') && is_repeating(profile.resource_type, &unlabelled(parent)) {
            continue;
        }
        if let Some(v) = extract(doc, profile, path, &[]) {
            let agrees = matches!(&v, Node::Str(s) if s == value)
                || matches!(&v, Node::Bool(b) if b.to_string() == *value)
                || matches!(&v, Node::Number(n) if n == value);
            if !agrees && !matches!(v, Node::Array(_) | Node::Object(_)) {
                conflicts.push(Conflict { path, given: v.to_json().trim_matches('"').to_string(), fixed: value });
            }
        }
    }

    // what the record carries that nothing above covers
    let mut extras = Vec::new();
    if let Value::Object(kvs) = doc.value() {
        walk_extras(kvs, "", &covered, &mut extras);
    }
    // an item of a repeating element the profile slices or fixes into that
    // no slice claims and no unlabelled slot takes: the profile's content
    // covers the element's paths, not this instance
    for element in fixed_or_sliced_arrays(profile) {
        let unlabelled_slot =
            profile.lowering.slots.iter().any(|s| unlabelled(s.path) == element && !s.path.contains(':'));
        if unlabelled_slot {
            continue;
        }
        let slices: Vec<&Slice> = profile
            .slicings
            .iter()
            .filter(|sl| sl.path == alloc::format!("{}.{element}", profile.resource_type))
            .flat_map(|sl| sl.slices.iter())
            .collect();
        // a slice the lowering table names but the slicing does not compile
        // (its discriminator is beyond the compiler): its instances cannot
        // be told from the record's — nothing is claimed unclaimed here
        let uncompiled_label = profile
            .lowering
            .slots
            .iter()
            .map(|s| s.path)
            .chain(profile.lowering.slices.iter().map(|(p, _)| *p))
            .filter_map(|p| p.strip_prefix(element.as_str()).and_then(|r| r.strip_prefix(':')))
            .map(|r| r.split('.').next().unwrap_or(r))
            .any(|label| !slices.iter().any(|sl| sl.name == label));
        if uncompiled_label {
            continue;
        }
        // the fixed groups the element's own pattern supplies (`code.coding`
        // with a fixed system + code: one instance is the profile's)
        let own_fixed = profile
            .lowering
            .fixed
            .iter()
            .filter(|(p, _)| p.rsplit_once('.').is_some_and(|(parent, _)| *parent == element))
            .count()
            > 0;
        let items = doc.instances(&alloc::format!("$this.{element}"));
        let mut claimed = 0usize;
        for (i, inst) in items.iter().enumerate() {
            if slices.iter().any(|sl| in_slice(sl, inst)) {
                continue;
            }
            if own_fixed && claimed == 0 {
                // the one instance the element's own fixed leaves describe
                let carries = profile.lowering.fixed.iter().all(|(p, v)| {
                    p.rsplit_once('.').is_none_or(|(parent, leaf)| {
                        parent != element || inst.scalars(&alloc::format!("$this.{leaf}")).contains(v)
                    })
                });
                if carries {
                    claimed += 1;
                    continue;
                }
            }
            extras.push(alloc::format!("{element}[{i}]"));
        }
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
    Ok(Lifted { inputs, validation, missing, extras, conflicts })
}

/// The repeating elements (unlabelled, resource-relative) the profile
/// slices or has fixed leaves directly beneath — where an instance the
/// profile does not describe is the record's and must be named.
fn fixed_or_sliced_arrays(profile: &CompiledProfile) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in profile.slicings {
        if let Some(rel) = s.path.strip_prefix(profile.resource_type).and_then(|r| r.strip_prefix('.')) {
            if is_repeating(profile.resource_type, rel) && !out.iter().any(|o| o == rel) {
                out.push(rel.to_string());
            }
        }
    }
    for (path, _) in profile.lowering.fixed {
        let Some((parent, _)) = path.rsplit_once('.') else { continue };
        let plain = unlabelled(parent);
        if plain == parent && is_repeating(profile.resource_type, &plain) && !out.contains(&plain) {
            out.push(plain);
        }
    }
    out
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
        // A fixed primitive covers its value, not its extension carrier.
        // Carriers extracted for variable slots have their own covered paths.
        let path = if prefix.is_empty() { key.to_string() } else { alloc::format!("{prefix}.{key}") };
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
            crate::resource_profile(url).ok_or(LiftError::UnknownProfile)?
        };
        lift_with(profile, resource, self, tx)
    }
}
