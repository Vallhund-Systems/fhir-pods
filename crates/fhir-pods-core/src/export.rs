//! Export: a patient's record re-profiled for a jurisdiction.
//!
//! A host writes its transformer once, against the base R4 definitions —
//! the vital-sign profiles (`bp`, `heartrate`, …) and the base resources
//! compiled as lowerable profiles (`Patient`, `Condition`, …). `export` then
//! speaks the region: for every resource it picks the profile in the region's
//! implementation guide, re-profiles the resource through that profile's
//! slots (`lift` → `lower`, so the profile's fixed content — the codings a
//! jurisdiction adds, its categories and slices — comes from the profile, and
//! the record's data from the record), keeps the resource's `id` so the
//! references between resources survive, assembles the region's patient
//! summary document, and can record the read as an IHE BALP audit event.
//!
//! What the region demands and the record does not have is reported, never
//! invented: a required slot the resource lacks is `missing`; content the
//! target profile has no slot for is `extras` and is *not* carried (a
//! re-profiled resource is the profile's shape); a resource with no
//! profile in the region — or several and no way to tell — is exported as
//! given, base-validated, with the reason.
//!
//! Which profile: the resource's declared `meta.profile` when it names one
//! of the region's; otherwise the region's profiles for the resource type
//! whose *identifying* fixed content the resource carries — a fixed
//! `code.coding` (any one of the profile's, since a jurisdiction may add
//! its own coding beside LOINC's) and a fixed `category.coding` — the most
//! specific of those (a profile another candidate derives from is dropped).
//! A profile with no identifying content (a region's Patient, its generic
//! Observation) matches every resource of its type. Exactly one survivor is
//! the target; none or several is a report to the host, who can declare the
//! profile to settle it.
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::assemble::{self, Assembled};
use crate::compiled::CompiledProfile;
use crate::json::Value;
use crate::lower::{self, Lowered};
use crate::node::Node;
use crate::terminology::TerminologyProvider;
use crate::{lift, wire, Validation, Validator};

/// A jurisdiction the exporter can speak: the implementation guide whose
/// resource profiles apply (by their canonical namespace) and the patient
/// summary document it produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// The name a host asks for (`au`, `us`, `ips`).
    pub name: &'static str,
    pub title: &'static str,
    /// The implementation guide(s), for the report.
    pub guide: &'static str,
    /// The canonical URL prefixes of the region's own resource profiles
    /// (its guide, and its patient summary's own profiles where it has them).
    pub prefixes: &'static [&'static str],
    /// The patient summary document assembled for the region.
    pub document: Option<assemble::Kind>,
    /// Profiles the guide scopes, in words, to a kind of resource their
    /// structure does not identify (a "diagnostic result", a "clinical test
    /// result", a device "as performer"): never matched by type alone — a
    /// host that means one declares it. Everything else with no identifying
    /// content is the guide's general profile for the type, and matches.
    pub specialised: &'static [&'static str],
}

/// The regions compiled in. `ips` is the international floor: a region the
/// registry has no guide for is exported as `ips`, and says so.
pub static REGIONS: &[Region] = &[
    Region {
        name: "au",
        title: "Australia",
        guide: "AU Core 2.0.0 resources, AU Patient Summary 1.0.0 document",
        prefixes: &[
            "http://hl7.org.au/fhir/ps/StructureDefinition/",
            "http://hl7.org.au/fhir/core/StructureDefinition/",
        ],
        document: Some(assemble::Kind::AuPs),
        // "specialist or other diagnostic (non-imaging or non-pathology) results"
        specialised: &["http://hl7.org.au/fhir/core/StructureDefinition/au-core-diagnosticresult"],
    },
    Region {
        name: "us",
        title: "United States",
        guide: "US Core 6.1.0 resources, International Patient Summary 2.0.1 document",
        prefixes: &["http://hl7.org/fhir/us/core/StructureDefinition/"],
        document: Some(assemble::Kind::Ips),
        // "non-laboratory clinical test results" — the Simple Observation
        // profile is the guide's own catch-all ("any simple observations
        // that are not otherwise covered")
        specialised: &["http://hl7.org/fhir/us/core/StructureDefinition/us-core-observation-clinical-result"],
    },
    Region {
        name: "ips",
        title: "International",
        guide: "International Patient Summary 2.0.1 resources and document",
        prefixes: &["http://hl7.org/fhir/uv/ips/StructureDefinition/"],
        document: Some(assemble::Kind::Ips),
        // a device "as performer or observer" of a result, not the patient's device
        specialised: &["http://hl7.org/fhir/uv/ips/StructureDefinition/Device-observer-uv-ips"],
    },
];

impl Region {
    pub fn parse(name: &str) -> Option<&'static Region> {
        REGIONS.iter().find(|r| r.name == name)
    }

    /// The region's own profiles for a resource type.
    pub fn candidates(&self, resource_type: &str) -> Vec<&'static CompiledProfile> {
        crate::REGISTRY.iter().filter(|p| p.resource_type == resource_type && self.owns(p.url)).copied().collect()
    }

    fn owns(&self, url: &str) -> bool {
        self.prefixes.iter().any(|p| url.starts_with(p))
    }

    /// The profiles the region's patient summary requires of its entries, for
    /// a resource type: the ones its Composition's sections name, and the
    /// profiles deriving from those under the region's namespaces or the
    /// base spec's (the base `vitalsigns` family — a document names
    /// `vitalsigns`; a blood pressure is a `bp`).
    pub fn document_candidates(&self, resource_type: &str) -> Vec<&'static CompiledProfile> {
        let Some(composition) = self.document.and_then(|k| k.composition()) else { return Vec::new() };
        let mut named: Vec<&'static str> = Vec::new();
        fn walk(s: &'static crate::compiled::Slicing, out: &mut Vec<&'static str>) {
            for slice in s.slices {
                for d in slice.discriminators {
                    if let crate::compiled::Discriminator::ResolvedProfile { urls } = d {
                        out.extend(urls.iter().copied());
                    }
                }
                for (_, urls) in slice.profiles {
                    out.extend(urls.iter().copied());
                }
                for nested in slice.slicings {
                    walk(nested, out);
                }
            }
        }
        for s in composition.slicings {
            walk(s, &mut named);
        }
        crate::REGISTRY
            .iter()
            .filter(|p| p.resource_type == resource_type)
            .filter(|p| {
                named.contains(&p.url)
                    || ((self.owns(p.url) || p.url.starts_with(BASE)) && named.iter().any(|n| derives_from(p, n)))
            })
            .copied()
            .collect()
    }
}

const BASE: &str = "http://hl7.org/fhir/StructureDefinition/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    /// Not a region compiled in (see [`REGIONS`]).
    UnknownRegion,
    InputsNotAnObject,
    /// `resources` is missing or not an array of resource objects.
    ResourcesNotAnArray,
    /// A resource whose `resourceType` the base spec does not define.
    NotAResource(usize),
    /// The region's document could not be assembled (no Patient, no
    /// `date`/`author`): the host asked for one — `document: false` to skip.
    Document(assemble::AssembleError),
    /// The audit event could not be built from `audit`.
    Audit(crate::audit::AuditError),
}

impl core::fmt::Display for ExportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ExportError::UnknownRegion => {
                f.write_str("region must be one of: ")?;
                for (i, r) in REGIONS.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    f.write_str(r.name)?;
                }
                Ok(())
            }
            ExportError::InputsNotAnObject => f.write_str("inputs must be a JSON object"),
            ExportError::ResourcesNotAnArray => f.write_str("inputs.resources must be an array of resources"),
            ExportError::NotAResource(i) => write!(f, "inputs.resources[{i}] is not a FHIR R4 resource"),
            ExportError::Document(e) => write!(f, "document: {e}"),
            ExportError::Audit(e) => write!(f, "audit: {e}"),
        }
    }
}

impl core::error::Error for ExportError {}

/// One resource of the export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedResource {
    /// Its position in `inputs.resources`.
    pub index: usize,
    pub resource_type: String,
    pub id: Option<String>,
    /// The region's profile it was re-profiled through; `None` when it went
    /// out as given (see `note`).
    pub profile: Option<&'static CompiledProfile>,
    /// The resource as exported, JSON text.
    pub resource: String,
    /// The exported resource's validation: against the target profile, or
    /// the base spec (and whatever it declares) when no profile applied.
    pub validation: Validation,
    /// The profile the region's patient summary asks of an entry of this
    /// type when it is not the target and the target does not derive from
    /// it: declared beside the target and verified — see [`Selection`].
    pub also: Option<&'static CompiledProfile>,
    /// Required slots of the target profile the record has no value for.
    pub missing: Vec<&'static str>,
    /// Required slots of `also` the record has no value for.
    pub also_missing: Vec<&'static str>,
    /// Element paths of the given resource the target profile has no slot
    /// for: not carried into the export.
    pub extras: Vec<String>,
    /// Why no profile applied (none in the region for the type; several and
    /// none declared), or the given resource declared none of the region's.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    pub region: &'static Region,
    pub resources: Vec<ExportedResource>,
    /// The region's patient summary, unless `document: false`.
    pub document: Option<Assembled>,
    /// The BALP `patient-read` event for the export, when `audit` was given.
    pub audit: Option<Lowered>,
}

/// What identifies a profile: for each anchor (`code.coding`,
/// `category.coding`), the coding groups it fixes — `(system, code)` pairs,
/// one group per fixed coding — and the value sets a `required` binding on
/// the anchor's element names. A resource identifies with the profile when,
/// for every anchor that has any, it carries one of the fixed codings or a
/// member of one of the sets (a set the provider cannot decide identifies
/// nothing: not knowing is not matching).
struct Identity {
    anchor: &'static str,
    codings: Vec<(Option<&'static str>, &'static str)>,
    sets: Vec<&'static str>,
}

fn identifying(p: &CompiledProfile) -> Vec<Identity> {
    const ANCHORS: [(&str, &str); 2] = [("code.coding", "code"), ("category.coding", "category")];
    let mut out: Vec<Identity> = Vec::new();
    for (anchor, element) in ANCHORS {
        // the fixed leaves under the anchor, grouped by their labelled parent
        // (`code.coding:loinc`, `code.coding:snomed` are two groups)
        let mut groups: Vec<(&str, Option<&'static str>, Option<&'static str>)> = Vec::new();
        for (path, value) in p.lowering.fixed {
            let Some((parent, leaf)) = path.rsplit_once('.') else { continue };
            if unlabelled(parent) != anchor || parent.starts_with("component") {
                continue;
            }
            let g = match groups.iter().position(|(k, _, _)| *k == parent) {
                Some(i) => i,
                None => {
                    groups.push((parent, None, None));
                    groups.len() - 1
                }
            };
            match leaf {
                "system" => groups[g].1 = Some(value),
                "code" => groups[g].2 = Some(value),
                _ => {}
            }
        }
        let codings: Vec<(Option<&'static str>, &'static str)> =
            groups.iter().filter_map(|(_, system, code)| code.map(|c| (*system, c))).collect();
        let element_path = format!("{}.{element}", p.resource_type);
        let sets: Vec<&'static str> = p
            .elements
            .iter()
            .filter(|e| e.path == element_path)
            .filter_map(|e| e.required_binding.map(|b| b.value_set))
            .collect();
        if !codings.is_empty() || !sets.is_empty() {
            out.push(Identity { anchor, codings, sets });
        }
    }
    out
}

/// The codings at `anchor` in the resource.
fn codings_at<'r>(resource: &'r Node, anchor: &str) -> Vec<&'r Node> {
    let mut at: Vec<&Node> = alloc::vec![resource];
    for seg in anchor.split('.') {
        let mut next = Vec::new();
        for n in at {
            match n.member(seg) {
                Some(Node::Array(xs)) => next.extend(xs.iter()),
                Some(x) => next.push(x),
                None => {}
            }
        }
        at = next;
    }
    at
}

fn identifies(resource: &Node, id: &Identity, tx: &dyn TerminologyProvider) -> bool {
    codings_at(resource, id.anchor).iter().any(|c| {
        let system = c.member("system").and_then(Node::as_str);
        let Some(code) = c.member("code").and_then(Node::as_str) else { return false };
        id.codings.iter().any(|(s, k)| *k == code && s.is_none_or(|s| Some(s) == system))
            || id.sets.iter().any(|set| {
                matches!(tx.validate_code(set, system.unwrap_or(""), code), crate::terminology::Membership::Member)
            })
    })
}

fn unlabelled(path: &str) -> String {
    path.split('.').map(|seg| seg.split(':').next().unwrap_or(seg)).collect::<Vec<_>>().join(".")
}

/// Does `from` derive (transitively) from the profile at `url`?
fn derives_from(mut from: &'static CompiledProfile, url: &str) -> bool {
    for _ in 0..16 {
        match from.base {
            Some(b) if b == url => return true,
            Some(b) => match crate::REGISTRY.iter().find(|q| q.url == b) {
                Some(q) => from = q,
                None => return false,
            },
            None => return false,
        }
    }
    false
}

/// The profile(s) a resource is exported through: the region's own, and —
/// when the region's patient summary asks its entries for a different one
/// the target does not already derive from — that one too, declared
/// beside it and verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub target: &'static CompiledProfile,
    pub also: Option<&'static CompiledProfile>,
}

/// Among `candidates`, the ones the resource identifies with, most specific
/// only: a profile another survivor derives from is dropped, and a profile
/// with no identifying content yields to any that has some.
fn matching(
    region: &Region,
    candidates: &[&'static CompiledProfile],
    resource: &Node,
    tx: &dyn TerminologyProvider,
) -> Vec<&'static CompiledProfile> {
    if let Some(only) = single_lineage(candidates) {
        return alloc::vec![only];
    }
    let matching: Vec<(&'static CompiledProfile, bool)> = candidates
        .iter()
        .filter_map(|p| {
            let idents = identifying(p);
            if idents.is_empty() && region.specialised.contains(&p.url) {
                return None;
            }
            idents.iter().all(|id| identifies(resource, id, tx)).then_some((*p, !idents.is_empty()))
        })
        .collect();
    let any_identified = matching.iter().any(|(_, identified)| *identified);
    matching
        .iter()
        .filter(|(p, identified)| {
            (*identified || !any_identified) && !matching.iter().any(|(q, _)| q.url != p.url && derives_from(q, p.url))
        })
        .map(|(p, _)| *p)
        .collect()
}

/// The most specific of `candidates` when they are one lineage — every one
/// derives from, or is, the same root — else `None`. Identity is how sibling
/// profiles of a type are told apart; a type with one lineage in the region
/// (a Patient, an AllergyIntolerance) has nothing to tell apart, and its
/// `code` binding is content the validator checks, not a discriminator.
fn single_lineage(candidates: &[&'static CompiledProfile]) -> Option<&'static CompiledProfile> {
    let leaves: Vec<&'static CompiledProfile> = candidates
        .iter()
        .filter(|p| !candidates.iter().any(|q| q.url != p.url && derives_from(q, p.url)))
        .copied()
        .collect();
    match leaves.as_slice() {
        [leaf] if candidates.iter().all(|c| c.url == leaf.url || derives_from(leaf, c.url)) => Some(leaf),
        _ => None,
    }
}

/// The profiles for a resource in `region`, or why there is none.
pub fn select(region: &Region, resource: &Node, tx: &dyn TerminologyProvider) -> Result<Selection, String> {
    let rt = resource.member("resourceType").and_then(Node::as_str).unwrap_or("");
    let own = region.candidates(rt);
    let document = region.document_candidates(rt);
    if own.is_empty() && document.is_empty() {
        return Err(format!("no {} profile for {rt}", region.title));
    }
    // what the document asks of an entry of this type, when that is settled
    let for_document: Option<&'static CompiledProfile> = match matching(region, &document, resource, tx).as_slice() {
        [one] => Some(one),
        _ => None,
    };
    let with_document = |target: &'static CompiledProfile| Selection {
        target,
        also: for_document.filter(|d| d.url != target.url && !derives_from(target, d.url)),
    };
    // a declared profile settles it: one of the region's own; or, where the
    // region has none for the type, one its document asks for (a base
    // `bp` the host lowered into is the resource's identity, not a
    // jurisdiction's profile — it never pre-empts the region's own)
    let declared = |among: &[&'static CompiledProfile]| -> Option<&'static CompiledProfile> {
        let Some(Node::Array(urls)) = resource.member("meta").and_then(|m| m.member("profile")) else { return None };
        urls.iter()
            .filter_map(Node::as_str)
            .map(|u| u.split('|').next().unwrap_or(u))
            .find_map(|u| among.iter().find(|p| p.url == u).copied())
    };
    if let Some(p) = declared(&own) {
        return Ok(with_document(p));
    }
    if own.is_empty() {
        if let Some(p) = declared(&document) {
            return Ok(Selection { target: p, also: None });
        }
    }
    match matching(region, &own, resource, tx).as_slice() {
        [one] => Ok(with_document(one)),
        // nothing of the region's own identifies with it: what its document
        // asks of such an entry is the profile (a `bp` for the IPS, whose
        // sections name the base `vitalsigns` and which has no vitals
        // profiles of its own); failing that, none
        [] => match for_document {
            Some(d) => Ok(Selection { target: d, also: None }),
            None => {
                let mut all: Vec<&'static CompiledProfile> = own.iter().chain(document.iter()).copied().collect();
                all.dedup_by_key(|p| p.url);
                Err(format!(
                    "no {} profile for this {rt}: none of the {} compiled identifies with its code or category",
                    region.title,
                    all.len()
                ))
            }
        },
        several => Err(format!(
            "several {} profiles fit this {rt} ({}): declare one in meta.profile",
            region.title,
            names(several)
        )),
    }
}

fn names(ps: &[&'static CompiledProfile]) -> String {
    ps.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")
}

/// Export the resources for `region`. Inputs:
///
/// ```json
/// {"resources": [{"resourceType": "Patient", …}, …],   // base-profiled, or any
///  "document": true,                                   // optional; false = resources only
///  "author": "Patient/u1", "date": "2026-09-21T10:00:00+10:00",  // the document's, as `document` takes them
///  "title": "…", "custodian": "…", "identifier": "…", "timestamp": "…",     // optional, as `document` takes them
///  "audit": {"recorded": "…", "client": {…}, "server": {…}, "user": {…}, "source": {…}, "transaction": "…"}}
///                                                     // optional: the BALP patient-read of this export;
///                                                     // `patient` and `data` are filled in from the export
/// ```
pub fn export(region: &'static Region, inputs: &Value<'_>, validator: &Validator) -> Result<Exported, ExportError> {
    export_with(region, inputs, validator, &crate::terminology::Bundled)
}

/// [`export`] with a terminology provider for every validation on the way.
pub fn export_with(
    region: &'static Region,
    inputs: &Value<'_>,
    validator: &Validator,
    tx: &dyn TerminologyProvider,
) -> Result<Exported, ExportError> {
    let Value::Object(members) = inputs else { return Err(ExportError::InputsNotAnObject) };
    let member = |k: &str| members.iter().find(|(m, _)| m.as_ref() == k).map(|(_, v)| v);
    let Some(Value::Array(given)) = member("resources") else { return Err(ExportError::ResourcesNotAnArray) };

    let mut resources: Vec<ExportedResource> = Vec::with_capacity(given.len());
    for (index, g) in given.iter().enumerate() {
        let node = Node::from_value(g);
        let rt = match node.member("resourceType").and_then(Node::as_str) {
            Some(rt) if matches!(node, Node::Object(_)) => rt.to_string(),
            _ => return Err(ExportError::ResourcesNotAnArray),
        };
        let id = node.member("id").and_then(Node::as_str).map(String::from);
        let bytes = node.to_json();
        match select(region, &node, tx) {
            Ok(Selection { target, also }) => {
                // the profile was chosen by the resource's own type, so a
                // lift that cannot start means the resource is not one
                let lifted = lift::lift_with(target, bytes.as_bytes(), validator, tx)
                    .map_err(|_| ExportError::NotAResource(index))?;
                let parsed =
                    crate::json::parse(lifted.inputs.as_bytes()).map_err(|_| ExportError::NotAResource(index))?;
                let lowered = lower::lower_with(target, &parsed, Some(validator), tx)
                    .map_err(|_| ExportError::NotAResource(index))?;
                let (resource, validation, also_missing) = match also {
                    None => (lowered.resource, lowered.validation, Vec::new()),
                    // declared beside the target, verified against both;
                    // what the second one requires and the record lacks
                    Some(second) => {
                        let mut node = crate::json::parse(lowered.resource.as_bytes())
                            .map(|v| Node::from_value(&v))
                            .map_err(|_| ExportError::NotAResource(index))?;
                        let profiles = node
                            .member_or_insert("meta", Node::Object(Vec::new()))
                            .member_or_insert("profile", Node::Array(Vec::new()));
                        if let Node::Array(urls) = profiles {
                            urls.push(Node::Str(second.url.into()));
                        }
                        let resource = node.to_json();
                        let validation = validator.validate_with(resource.as_bytes(), tx);
                        let also_missing = lift::lift_with(second, resource.as_bytes(), validator, tx)
                            .map(|l| l.missing)
                            .unwrap_or_default();
                        (resource, validation, also_missing)
                    }
                };
                resources.push(ExportedResource {
                    index,
                    resource_type: rt,
                    id,
                    profile: Some(target),
                    also,
                    resource,
                    validation,
                    missing: lifted.missing,
                    also_missing,
                    extras: lifted.extras,
                    note: None,
                });
            }
            Err(why) => {
                let validation = validator.validate_with(bytes.as_bytes(), tx);
                if validation.report.issues.iter().any(|i| i.path == "resourceType") {
                    return Err(ExportError::NotAResource(index));
                }
                resources.push(ExportedResource {
                    index,
                    resource_type: rt,
                    id,
                    profile: None,
                    also: None,
                    resource: bytes,
                    validation,
                    missing: Vec::new(),
                    also_missing: Vec::new(),
                    extras: Vec::new(),
                    note: Some(format!("{why}; exported as given")),
                });
            }
        }
    }

    // the region's document, from the exported resources
    let wants_document = !matches!(member("document"), Some(Value::Bool(false)));
    let document = match region.document {
        Some(kind) if wants_document => {
            let mut s = String::from("{\"resources\":[");
            for (i, r) in resources.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push_str(&r.resource);
            }
            s.push(']');
            for key in ["author", "date", "title", "custodian", "identifier", "timestamp"] {
                if let Some(v) = member(key) {
                    s.push_str(",\"");
                    s.push_str(key);
                    s.push_str("\":");
                    Node::from_value(v).write(&mut s);
                }
            }
            s.push('}');
            let parsed = crate::json::parse(s.as_bytes()).map_err(|_| ExportError::InputsNotAnObject)?;
            Some(assemble::assemble_with(kind, &parsed, validator, tx).map_err(ExportError::Document)?)
        }
        _ => None,
    };

    // the export as a BALP patient-read: what the host knows about the
    // request, plus who the record is about and what went out
    let audit = match member("audit") {
        None => None,
        Some(Value::Object(given)) => {
            let patient = resources
                .iter()
                .find(|r| r.resource_type == "Patient")
                .and_then(|r| r.id.as_ref().map(|id| format!("Patient/{id}")));
            let data = document
                .as_ref()
                .and_then(|d| crate::json::parse(d.document.as_bytes()).ok().map(|v| Node::from_value(&v)))
                .and_then(|b| b.member("id").and_then(Node::as_str).map(|id| format!("Bundle/{id}")));
            let has = |k: &str| given.iter().any(|(m, _)| m.as_ref() == k);
            let mut members: Vec<(String, Node)> = Vec::with_capacity(given.len() + 2);
            if let Some(p) = patient.filter(|_| !has("patient")) {
                members.push(("patient".into(), Node::Str(p)));
            }
            if let Some(d) = data.filter(|_| !has("data")) {
                members.push(("data".into(), Node::Str(d)));
            }
            members.extend(given.iter().map(|(k, v)| (k.to_string(), Node::from_value(v))));
            let s = Node::Object(members).to_json();
            let parsed = crate::json::parse(s.as_bytes()).map_err(|_| ExportError::InputsNotAnObject)?;
            Some(
                crate::audit::audit_with(crate::audit::Kind::PatientRead, &parsed, Some(validator), tx)
                    .map_err(ExportError::Audit)?,
            )
        }
        Some(_) => return Err(ExportError::Audit(crate::audit::AuditError::InputsNotAnObject)),
    };

    Ok(Exported { region, resources, document, audit })
}

impl Exported {
    /// The pods' JSON: `{"region": {…}, "resources": [{…}], "document": {…}|null, "audit": {…}|null}`.
    pub fn to_json(&self) -> String {
        let mut s = String::with_capacity(4096);
        s.push_str("{\"region\":{\"name\":\"");
        s.push_str(self.region.name);
        s.push_str("\",\"title\":\"");
        s.push_str(self.region.title);
        s.push_str("\",\"guide\":\"");
        s.push_str(self.region.guide);
        s.push_str("\"},\"resources\":[");
        for (i, r) in self.resources.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str("{\"index\":");
            s.push_str(&r.index.to_string());
            s.push_str(",\"resourceType\":\"");
            wire::escape(&mut s, &r.resource_type);
            s.push_str("\",\"id\":");
            match &r.id {
                Some(id) => {
                    s.push('"');
                    wire::escape(&mut s, id);
                    s.push('"');
                }
                None => s.push_str("null"),
            }
            s.push_str(",\"profile\":");
            match r.profile {
                Some(p) => {
                    s.push_str("{\"url\":\"");
                    wire::escape(&mut s, p.url);
                    s.push_str("\",\"name\":\"");
                    wire::escape(&mut s, p.name);
                    s.push_str("\"}");
                }
                None => s.push_str("null"),
            }
            s.push_str(",\"also\":");
            match r.also {
                Some(p) => {
                    s.push_str("{\"url\":\"");
                    wire::escape(&mut s, p.url);
                    s.push_str("\",\"name\":\"");
                    wire::escape(&mut s, p.name);
                    s.push_str("\"}");
                }
                None => s.push_str("null"),
            }
            s.push_str(",\"resource\":");
            s.push_str(&r.resource);
            s.push_str(",\"validation\":");
            s.push_str(&wire::report(&r.validation));
            s.push_str(",\"missing\":[");
            for (j, m) in r.missing.iter().enumerate() {
                if j > 0 {
                    s.push(',');
                }
                s.push('"');
                wire::escape(&mut s, m);
                s.push('"');
            }
            s.push_str("],\"alsoMissing\":[");
            for (j, m) in r.also_missing.iter().enumerate() {
                if j > 0 {
                    s.push(',');
                }
                s.push('"');
                wire::escape(&mut s, m);
                s.push('"');
            }
            s.push_str("],\"extras\":[");
            for (j, e) in r.extras.iter().enumerate() {
                if j > 0 {
                    s.push(',');
                }
                s.push('"');
                wire::escape(&mut s, e);
                s.push('"');
            }
            s.push_str("],\"note\":");
            match &r.note {
                Some(n) => {
                    s.push('"');
                    wire::escape(&mut s, n);
                    s.push('"');
                }
                None => s.push_str("null"),
            }
            s.push('}');
        }
        s.push_str("],\"document\":");
        match &self.document {
            Some(d) => {
                s.push_str("{\"kind\":\"");
                s.push_str(self.region.document.map(|k| k.name()).unwrap_or(""));
                s.push_str("\",\"document\":");
                s.push_str(&d.document);
                s.push_str(",\"placed\":[");
                for (i, (full, section)) in d.placed.iter().enumerate() {
                    if i > 0 {
                        s.push(',');
                    }
                    s.push_str("{\"fullUrl\":\"");
                    wire::escape(&mut s, full);
                    s.push_str("\",\"section\":\"");
                    wire::escape(&mut s, section);
                    s.push_str("\"}");
                }
                s.push_str("],\"omitted\":[");
                for (i, key) in d.omitted.iter().enumerate() {
                    if i > 0 {
                        s.push(',');
                    }
                    s.push('"');
                    wire::escape(&mut s, key);
                    s.push('"');
                }
                s.push_str("],\"validation\":");
                s.push_str(&wire::report(&d.validation));
                s.push('}');
            }
            None => s.push_str("null"),
        }
        s.push_str(",\"audit\":");
        match &self.audit {
            Some(a) => {
                s.push_str("{\"resource\":");
                s.push_str(&a.resource);
                s.push_str(",\"validation\":");
                s.push_str(&wire::report(&a.validation));
                s.push('}');
            }
            None => s.push_str("null"),
        }
        s.push('}');
        s
    }

    /// Every verdict at once: each resource, the document and the audit event.
    pub fn is_valid(&self) -> bool {
        self.resources.iter().all(|r| r.validation.is_valid())
            && self.document.as_ref().is_none_or(|d| d.validation.is_valid())
            && self.audit.as_ref().is_none_or(|a| a.validation.is_valid())
    }
}

impl Validator {
    /// Export resources for a region: see [`export`].
    pub fn export(&self, region: &str, inputs: &[u8]) -> Result<Exported, ExportError> {
        self.export_with(region, inputs, &crate::terminology::Bundled)
    }

    /// [`Validator::export`] with a terminology provider.
    pub fn export_with(
        &self,
        region: &str,
        inputs: &[u8],
        tx: &dyn TerminologyProvider,
    ) -> Result<Exported, ExportError> {
        let region = Region::parse(region).ok_or(ExportError::UnknownRegion)?;
        let parsed = crate::json::parse(inputs).map_err(|_| ExportError::InputsNotAnObject)?;
        export_with(region, &parsed, self, tx)
    }
}
