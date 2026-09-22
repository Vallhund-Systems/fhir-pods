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
//! target profile has no slot for is carried where the output shape allows
//! it, or reported in `extras` when it cannot be carried; a resource with no
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
    /// compiled structure does not identify (a "diagnostic result", a
    /// "clinical test result", "smoking status" whose code binding is only
    /// extensible, a device "as performer"): never matched by content — a
    /// host that means one declares it. Everything else with no identifying
    /// content is the guide's general profile for the type, and matches.
    pub specialised: &'static [&'static str],
}

/// The regions compiled in. A host can explicitly select `ips` as the
/// international floor. Unknown region names are rejected.
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
        // "non-laboratory clinical test results"; "smoking status" (its
        // structure is any social-history observation: the code binding
        // is extensible, so nothing compiled identifies it). The Simple
        // Observation profile is the guide's own catch-all ("any simple
        // observations that are not otherwise covered") and stays.
        specialised: &[
            "http://hl7.org/fhir/us/core/StructureDefinition/us-core-observation-clinical-result",
            "http://hl7.org/fhir/us/core/StructureDefinition/us-core-smokingstatus",
        ],
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
    /// More than one `Patient` among the resources: a record is one patient's.
    SeveralPatients,
    /// A resource whose `subject` / `patient` names a Patient other than the
    /// record's (relative or absolute, optionally versioned, at
    /// `resources[index]`). Unresolved identity is reported in `notes`.
    NotThisPatient(usize, String),
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
            ExportError::SeveralPatients => {
                f.write_str("inputs.resources holds more than one Patient; a record is one patient's")
            }
            ExportError::NotThisPatient(i, r) => {
                write!(f, "inputs.resources[{i}] is about '{r}', not this record's Patient; not exported")
            }
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
    /// for and the export could not carry (beneath a repeating element the
    /// profile rebuilt): left out, named.
    pub extras: Vec<String>,
    /// Element paths the target profile has no slot for that the export
    /// carried as given (a top-level member, an item of a repeating element
    /// no slice claims, a leaf under single-valued parents) — verified
    /// with the rest.
    pub carried: Vec<String>,
    /// Why no profile applied (none in the region for the type; several and
    /// none declared; the record contradicts what the profile fixes).
    pub note: Option<String>,
    /// What a host should know about the choice: a declared profile that
    /// was not applied, a document requirement that could not be settled,
    /// or a patient reference whose identity the host must resolve.
    pub notes: Vec<String>,
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
/// the target does not already satisfy — that one too, declared beside it
/// and verified. `notes` are what a host should know about the choice: a
/// declared profile that was ignored, a document requirement that could
/// not be settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub target: &'static CompiledProfile,
    pub also: Option<&'static CompiledProfile>,
    pub notes: Vec<String>,
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
    let matching: Vec<(&'static CompiledProfile, Vec<&'static str>)> = candidates
        .iter()
        .filter_map(|p| {
            if region.specialised.contains(&p.url) {
                return None;
            }
            let idents = identifying(p);
            idents
                .iter()
                .all(|id| identifies(resource, id, tx))
                .then(|| (*p, idents.iter().map(|id| id.anchor).collect::<Vec<_>>()))
        })
        .collect();
    let any_identified = matching.iter().any(|(_, anchors)| !anchors.is_empty());
    matching
        .iter()
        .filter(|(p, anchors)| {
            let identified = !anchors.is_empty();
            // a profile another survivor derives from, or whose identity is
            // a strict subset of another survivor's (a category alone
            // beside a category and a code), is the less specific one
            let outdone = matching.iter().any(|(q, qa)| {
                q.url != p.url
                    && (derives_from(q, p.url) || (anchors.iter().all(|a| qa.contains(a)) && qa.len() > anchors.len()))
            });
            (identified || !any_identified) && !outdone
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
    // what the document asks of an entry of this type: the candidates the
    // resource identifies with; settled when one, or when the target is
    // (or derives from) one of them
    let for_document = matching(region, &document, resource, tx);
    let mut notes: Vec<String> = Vec::new();
    // declared profiles the region does not know: said, not silently passed over
    if let Some(Node::Array(urls)) = resource.member("meta").and_then(|m| m.member("profile")) {
        for u in urls.iter().filter_map(Node::as_str) {
            let bare = u.split('|').next().unwrap_or(u);
            let known = own.iter().chain(document.iter()).any(|p| p.url == bare);
            if !known && !bare.starts_with(BASE) {
                notes.push(format!("declared profile '{bare}' is not one of {}'s; not applied", region.title));
            }
        }
    }
    let with_document = |target: &'static CompiledProfile, mut notes: Vec<String>| {
        let satisfied = for_document.iter().any(|d| d.url == target.url || derives_from(target, d.url));
        let also = match for_document.as_slice() {
            _ if satisfied => None,
            [one] => Some(*one),
            [] => None,
            several => {
                notes.push(format!(
                    "the {} document asks its {rt} entries for one of {}; none settled, so none declared",
                    region.title,
                    names(several)
                ));
                None
            }
        };
        Selection { target, also, notes }
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
        return Ok(with_document(p, notes));
    }
    if own.is_empty() {
        if let Some(p) = declared(&document) {
            return Ok(Selection { target: p, also: None, notes });
        }
    }
    match matching(region, &own, resource, tx).as_slice() {
        [one] => Ok(with_document(one, notes)),
        // nothing of the region's own identifies with it: what its document
        // asks of such an entry is the profile (a `bp` for the IPS, whose
        // sections name the base `vitalsigns` and which has no vitals
        // profiles of its own; an IPS profile the AU summary names for a
        // section); failing that, none
        [] => match for_document.as_slice() {
            [d] => Ok(Selection { target: d, also: None, notes }),
            _ => {
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

// Recognize a REST patient identity without discarding an absolute URL's
// authority. Absolute same-id references still need host resolution.
fn patient_id(reference: &str) -> Option<(&str, bool)> {
    if reference.contains(['?', '#']) {
        return None;
    }
    let (path, absolute) =
        if let Some(rest) = reference.strip_prefix("https://").or_else(|| reference.strip_prefix("http://")) {
            let (authority, path) = rest.split_once('/')?;
            if authority.is_empty() {
                return None;
            }
            (path, true)
        } else {
            (reference, false)
        };
    let parts: Vec<&str> = path.split('/').collect();
    let tail = match parts.as_slice() {
        [rest @ .., "_history", version] if !version.is_empty() => rest,
        _ => parts.as_slice(),
    };
    match tail {
        [prefix @ .., "Patient", id] if !id.is_empty() && (absolute || prefix.is_empty()) => Some((id, absolute)),
        _ => None,
    }
}

fn patient_notes(node: &Node, record_id: Option<&str>, index: usize) -> Result<Vec<String>, ExportError> {
    let mut notes = Vec::new();
    for key in ["subject", "patient"] {
        let Some(subject) = node.member(key) else { continue };
        let reference = subject.member("reference").and_then(Node::as_str);
        if let (Some(record_id), Some(reference)) = (record_id, reference) {
            if let Some((id, absolute)) = patient_id(reference) {
                if id != record_id {
                    return Err(ExportError::NotThisPatient(index, reference.into()));
                }
                if !absolute {
                    continue;
                }
            }
        }
        notes.push(format!(
            "patient identity not established for {key}{}; the host must resolve this reference against the record's Patient",
            reference.map(|r| format!(" '{r}'")).unwrap_or_default()
        ));
    }
    Ok(notes)
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

    // one patient: the record's Patient, and every resource about someone
    let patient_ids: Vec<String> = given
        .iter()
        .filter_map(|g| {
            let n = Node::from_value(g);
            (n.member("resourceType").and_then(Node::as_str) == Some("Patient"))
                .then(|| n.member("id").and_then(Node::as_str).map(String::from).unwrap_or_default())
        })
        .collect();
    if patient_ids.len() > 1 {
        return Err(ExportError::SeveralPatients);
    }
    let record_patient_id = patient_ids.first().filter(|id| !id.is_empty()).map(String::as_str);

    let mut resources: Vec<ExportedResource> = Vec::with_capacity(given.len());
    for (index, g) in given.iter().enumerate() {
        let node = Node::from_value(g);
        let rt = match node.member("resourceType").and_then(Node::as_str) {
            Some(rt) if matches!(node, Node::Object(_)) => rt.to_string(),
            _ => return Err(ExportError::ResourcesNotAnArray),
        };
        let id = node.member("id").and_then(Node::as_str).map(String::from);
        let identity_notes = patient_notes(&node, record_patient_id, index)?;
        let bytes = node.to_json();
        let as_given =
            |note: String, notes: Vec<String>, resources: &mut Vec<ExportedResource>| -> Result<(), ExportError> {
                let validation = validator.validate_with(bytes.as_bytes(), tx);
                if validation.report.issues.iter().any(|i| i.path == "resourceType") {
                    return Err(ExportError::NotAResource(index));
                }
                resources.push(ExportedResource {
                    index,
                    resource_type: rt.clone(),
                    id: id.clone(),
                    profile: None,
                    also: None,
                    resource: bytes.clone(),
                    validation,
                    missing: Vec::new(),
                    also_missing: Vec::new(),
                    extras: Vec::new(),
                    carried: Vec::new(),
                    note: Some(note),
                    notes,
                });
                Ok(())
            };
        match select(region, &node, tx) {
            Ok(Selection { target, also, mut notes }) => {
                notes.extend(identity_notes);
                // the profile was chosen by the resource's own type, so a
                // lift that cannot start means the resource is not one
                let lifted = lift::lift_with(target, bytes.as_bytes(), validator, tx)
                    .map_err(|_| ExportError::NotAResource(index))?;
                // the record contradicts what the profile fixes: re-profiling
                // would replace its values — it goes out as given, and says why
                if !lifted.conflicts.is_empty() {
                    let why = lifted
                        .conflicts
                        .iter()
                        .map(|c| format!("{} is '{}' where {} fixes '{}'", c.path, c.given, target.name, c.fixed))
                        .collect::<Vec<_>>()
                        .join("; ");
                    as_given(format!("not re-profiled: {why}; exported as given"), notes, &mut resources)?;
                    continue;
                }
                let parsed =
                    crate::json::parse(lifted.inputs.as_bytes()).map_err(|_| ExportError::NotAResource(index))?;
                let lowered = lower::lower_with(target, &parsed, Some(validator), tx)
                    .map_err(|_| ExportError::NotAResource(index))?;
                let mut out = crate::json::parse(lowered.resource.as_bytes())
                    .map(|v| Node::from_value(&v))
                    .map_err(|_| ExportError::NotAResource(index))?;
                // what no slot took, carried back where the profile's shape
                // allows it — the record's, verified with the rest
                let (carried, extras) = carry_back(&node, &mut out, &lifted.extras);
                if let Some(second) = also {
                    let profiles = out
                        .member_or_insert("meta", Node::Object(Vec::new()))
                        .member_or_insert("profile", Node::Array(Vec::new()));
                    if let Node::Array(urls) = profiles {
                        urls.push(Node::Str(second.url.into()));
                    }
                }
                let resource = out.to_json();
                let validation = validator.validate_with(resource.as_bytes(), tx);
                let also_missing = match also {
                    Some(second) => lift::lift_with(second, resource.as_bytes(), validator, tx)
                        .map(|l| l.missing)
                        .unwrap_or_default(),
                    None => Vec::new(),
                };
                if let Some(second) = also {
                    notes.push(format!(
                        "the {} document asks its {rt} entries for {}: declared beside {} and verified",
                        region.title, second.name, target.name
                    ));
                }
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
                    extras,
                    carried,
                    note: None,
                    notes,
                });
            }
            Err(why) => as_given(format!("{why}; exported as given"), identity_notes, &mut resources)?,
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
            // what was read: the document; without one, the patient's record
            let data = document
                .as_ref()
                .and_then(|d| crate::json::parse(d.document.as_bytes()).ok().map(|v| Node::from_value(&v)))
                .and_then(|b| b.member("id").and_then(Node::as_str).map(|id| format!("Bundle/{id}")))
                .or_else(|| patient.clone());
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

/// Carry the given resource's `extras` into the lowered one where its
/// shape allows: a top-level member the lowered resource lacks; an item of
/// a repeating element no slice claimed (`component[2]`), appended; a leaf
/// under single-valued parents that exist in both (`code.text`). Anything
/// beneath a repeating element the profile rebuilt is left out and named.
/// Returns (carried, left out).
fn carry_back(given: &Node, out: &mut Node, extras: &[String]) -> (Vec<String>, Vec<String>) {
    let mut carried = Vec::new();
    let mut left = Vec::new();
    for path in extras {
        // `component[2]`: an unclaimed item of a repeating element
        if let Some((element, idx)) = path.rsplit_once('[').and_then(|(e, i)| i.strip_suffix(']').map(|i| (e, i))) {
            let Ok(i) = idx.parse::<usize>() else {
                left.push(path.clone());
                continue;
            };
            let item = element.split('.').try_fold(given, |n, seg| n.member(seg)).and_then(|n| match n {
                Node::Array(xs) => xs.get(i),
                _ => None,
            });
            let target = element.split('.').try_fold(&mut *out, |n, seg| n.member_mut(seg));
            match (item, target) {
                (Some(item), Some(Node::Array(xs))) => {
                    xs.push(item.clone());
                    carried.push(path.clone());
                }
                _ => left.push(path.clone()),
            }
            continue;
        }
        // a leaf whose every ancestor is a single object on both sides
        let segs: Vec<&str> = path.split('.').collect();
        let (parents, leaf) = segs.split_at(segs.len() - 1);
        let from = parents.iter().try_fold(given, |n, seg| match n.member(seg) {
            Some(m @ Node::Object(_)) => Some(m),
            _ => None,
        });
        let to = parents.iter().try_fold(&mut *out, |n, seg| match n.member_mut(seg) {
            Some(m @ Node::Object(_)) => Some(m),
            _ => None,
        });
        match (from, to) {
            (Some(from), Some(Node::Object(kvs))) if !kvs.iter().any(|(k, _)| k == leaf[0]) => {
                for key in [leaf[0].to_string(), format!("_{}", leaf[0])] {
                    if let Some(v) = from.member(&key).filter(|_| !kvs.iter().any(|(k, _)| k == &key)) {
                        kvs.push((key, v.clone()));
                    }
                }
                carried.push(path.clone());
            }
            _ => left.push(path.clone()),
        }
    }
    (carried, left)
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
            s.push_str("],\"carried\":[");
            for (j, e) in r.carried.iter().enumerate() {
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
            s.push_str(",\"notes\":[");
            for (j, n) in r.notes.iter().enumerate() {
                if j > 0 {
                    s.push(',');
                }
                s.push('"');
                wire::escape(&mut s, n);
                s.push('"');
            }
            s.push_str("]}");
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
