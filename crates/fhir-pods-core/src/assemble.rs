//! Documents: an IPS or AU Patient Summary `Bundle` assembled from a
//! patient and the resources a host holds for them. The sections come from
//! the kind's compiled Composition profile (its section slices' LOINC
//! codes), each resource is placed by what it is, the narrative is a plain
//! table per section built from what the resource carries, every reference
//! between the resources is rewritten to the entry it names, and the
//! document is validated against the kind's Bundle profile on the way out —
//! an assembled document is never trusted unverified.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::compiled::{CompiledProfile, Discriminator};
use crate::json::Value;
use crate::node::Node;
use crate::{generated, Validation, Validator};

/// Which patient summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// International Patient Summary (`hl7.fhir.uv.ips`).
    Ips,
    /// AU Patient Summary (`hl7.fhir.au.ps`).
    AuPs,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "ips" => Some(Kind::Ips),
            "au-ps" => Some(Kind::AuPs),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Ips => "ips",
            Kind::AuPs => "au-ps",
        }
    }

    fn bundle_url(self) -> &'static str {
        match self {
            Kind::Ips => "http://hl7.org/fhir/uv/ips/StructureDefinition/Bundle-uv-ips",
            Kind::AuPs => "http://hl7.org.au/fhir/ps/StructureDefinition/au-ps-bundle",
        }
    }

    fn composition_url(self) -> &'static str {
        match self {
            Kind::Ips => "http://hl7.org/fhir/uv/ips/StructureDefinition/Composition-uv-ips",
            Kind::AuPs => "http://hl7.org.au/fhir/ps/StructureDefinition/au-ps-composition",
        }
    }

    pub(crate) fn composition(self) -> Option<&'static CompiledProfile> {
        generated::REGISTRY.iter().find(|p| p.url == self.composition_url()).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssembleError {
    /// Not `ips` or `au-ps`, or the kind's profiles are not compiled.
    UnknownKind,
    InputsNotAnObject,
    /// `resources` is missing or not an array of resource objects.
    ResourcesNotAnArray,
    /// No `Patient` among the resources.
    NoPatient,
    /// `date` (a dateTime) and `author` (a reference) are needed.
    Missing(&'static str),
}

impl core::fmt::Display for AssembleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AssembleError::UnknownKind => f.write_str("kind must be ips or au-ps"),
            AssembleError::InputsNotAnObject => f.write_str("inputs must be a JSON object"),
            AssembleError::ResourcesNotAnArray => f.write_str("inputs.resources must be an array of resources"),
            AssembleError::NoPatient => f.write_str("inputs.resources must include the Patient"),
            AssembleError::Missing(what) => write!(f, "inputs.{what} is required"),
        }
    }
}

impl core::error::Error for AssembleError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    /// The document `Bundle`, as JSON text.
    pub document: String,
    /// Which section each resource went to, by entry (`fullUrl`, section
    /// LOINC code or `""` for a supporting entry).
    pub placed: Vec<(String, String)>,
    /// Resources left out because nothing in the document reaches them
    /// (`Type/id`): a document is what the Composition links to.
    pub omitted: Vec<String>,
    pub validation: Validation,
}

/// Every `reference` string under `node`.
fn references_in(node: &Node, out: &mut Vec<String>) {
    match node {
        Node::Object(kvs) => {
            for (k, v) in kvs {
                if k == "reference" {
                    if let Node::Str(r) = v {
                        out.push(r.clone());
                    }
                } else {
                    references_in(v, out);
                }
            }
        }
        Node::Array(xs) => xs.iter().for_each(|x| references_in(x, out)),
        _ => {}
    }
}

/// A section of the kind's Composition profile: its slice name, LOINC code
/// and the title the pods give it.
struct Section {
    name: &'static str,
    code: String,
    title: &'static str,
    required: bool,
}

/// The sections, read from the Composition profile's `section` slicing —
/// the slice's pattern discriminator carries the LOINC code.
fn sections(profile: &CompiledProfile) -> Vec<Section> {
    let mut out = Vec::new();
    let Some(slicing) = profile.slicings.iter().find(|s| s.path == "Composition.section") else { return out };
    for slice in slicing.slices {
        let code = slice.discriminators.iter().find_map(|d| match d {
            Discriminator::Pattern { pairs, .. } => {
                pairs.iter().find(|(k, _)| *k == "code").map(|(_, v)| v.to_string())
            }
            _ => None,
        });
        if let Some(code) = code {
            out.push(Section { name: slice.name, code, title: title_of(slice.name), required: slice.min >= 1 });
        }
    }
    out
}

fn title_of(slice: &'static str) -> &'static str {
    match slice {
        "sectionProblems" => "Problems",
        "sectionAllergies" => "Allergies and Intolerances",
        "sectionMedications" => "Medications",
        "sectionImmunizations" => "Immunizations",
        "sectionResults" => "Results",
        "sectionProceduresHx" => "History of Procedures",
        "sectionMedicalDevices" => "Medical Devices",
        "sectionAdvanceDirectives" => "Advance Directives",
        "sectionAlerts" => "Alerts",
        "sectionFunctionalStatus" => "Functional Status",
        "sectionPastProblems" => "History of Past Illness",
        "sectionPregnancyHx" => "Pregnancy",
        "sectionPatientStory" => "Patient Story",
        "sectionPlanOfCare" => "Plan of Care",
        "sectionSocialHistory" => "Social History",
        "sectionVitalSigns" => "Vital Signs",
        other => other,
    }
}

/// Which section a resource belongs in, by what it is; `None` for a
/// supporting resource (the patient, practitioners, organisations,
/// medications referenced from statements, devices, specimens…).
fn section_for(resource: &Node) -> Option<&'static str> {
    let rt = resource.member("resourceType").and_then(Node::as_str)?;
    let has_coding = |path: &[&str], system: &str, code: &str| -> bool {
        let mut node = resource;
        for seg in path {
            node = match node.member(seg) {
                Some(Node::Array(xs)) => match xs.first() {
                    Some(x) => x,
                    None => return false,
                },
                Some(n) => n,
                None => return false,
            };
        }
        let Some(Node::Array(codings)) = node.member("coding") else { return false };
        codings.iter().any(|c| {
            c.member("system").and_then(Node::as_str) == Some(system)
                && c.member("code").and_then(Node::as_str) == Some(code)
        })
    };
    let any_coding = |member: &str, system: &str, codes: &[&str]| -> bool {
        match resource.member(member) {
            Some(Node::Array(concepts)) => concepts.iter().any(|cc| {
                matches!(cc.member("coding"), Some(Node::Array(cs)) if cs.iter().any(|c| c.member("system").and_then(Node::as_str) == Some(system) && c.member("code").and_then(Node::as_str).is_some_and(|code| codes.contains(&code))))
            }),
            _ => false,
        }
    };
    const LOINC: &str = "http://loinc.org";
    const CATEGORY: &str = "http://terminology.hl7.org/CodeSystem/observation-category";
    Some(match rt {
        "Condition" => {
            let inactive =
                has_coding(&["clinicalStatus"], "http://terminology.hl7.org/CodeSystem/condition-clinical", "inactive")
                    || has_coding(
                        &["clinicalStatus"],
                        "http://terminology.hl7.org/CodeSystem/condition-clinical",
                        "resolved",
                    )
                    || has_coding(
                        &["clinicalStatus"],
                        "http://terminology.hl7.org/CodeSystem/condition-clinical",
                        "remission",
                    );
            if inactive {
                "sectionPastProblems"
            } else {
                "sectionProblems"
            }
        }
        "AllergyIntolerance" => "sectionAllergies",
        "MedicationStatement" | "MedicationRequest" => "sectionMedications",
        "Immunization" => "sectionImmunizations",
        "Procedure" => "sectionProceduresHx",
        "DeviceUseStatement" => "sectionMedicalDevices",
        "DiagnosticReport" => "sectionResults",
        "CarePlan" | "ImmunizationRecommendation" => "sectionPlanOfCare",
        "Flag" => "sectionAlerts",
        "Consent" => "sectionAdvanceDirectives",
        "ClinicalImpression" => "sectionFunctionalStatus",
        "Observation" => {
            if has_coding(&["code"], LOINC, "72166-2") || has_coding(&["code"], LOINC, "74013-4") {
                "sectionSocialHistory"
            } else if has_coding(&["code"], LOINC, "82810-3")
                || has_coding(&["code"], LOINC, "11778-8")
                || has_coding(&["code"], LOINC, "11779-6")
                || has_coding(&["code"], LOINC, "11780-4")
                || has_coding(&["code"], LOINC, "93857-1")
            {
                "sectionPregnancyHx"
            } else if any_coding("category", CATEGORY, &["vital-signs"]) {
                "sectionVitalSigns"
            } else if any_coding("category", CATEGORY, &["laboratory", "imaging", "procedure"]) {
                "sectionResults"
            } else {
                return None;
            }
        }
        _ => return None,
    })
}

/// A UUID (version 4 layout, deterministic) for the entry that holds
/// `resourceType/id` — the same inputs make the same document.
fn entry_uuid(seed: &str) -> String {
    let h = crate::hash::sha256(seed.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let x = crate::hash::hex(&b);
    alloc::format!("{}-{}-{}-{}-{}", &x[..8], &x[8..12], &x[12..16], &x[16..20], &x[20..])
}

fn esc(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// A short description of a coded element: its text, else a coding's
/// display, else `system|code`.
fn describe(cc: Option<&Node>) -> Option<String> {
    let cc = cc?;
    if let Some(t) = cc.member("text").and_then(Node::as_str) {
        return Some(t.into());
    }
    let Some(Node::Array(codings)) = cc.member("coding") else { return None };
    let first = codings.first()?;
    if let Some(d) = first.member("display").and_then(Node::as_str) {
        return Some(d.into());
    }
    let code = first.member("code").and_then(Node::as_str)?;
    Some(match first.member("system").and_then(Node::as_str) {
        Some(s) => alloc::format!("{s}|{code}"),
        None => code.into(),
    })
}

/// One narrative row per resource: what it is, its status, and when.
fn row(resource: &Node) -> (String, String, String) {
    let rt = resource.member("resourceType").and_then(Node::as_str).unwrap_or("");
    let what = describe(resource.member("code"))
        .or_else(|| describe(resource.member("medicationCodeableConcept")))
        .or_else(|| describe(resource.member("vaccineCode")))
        .or_else(|| {
            resource
                .member("medicationReference")
                .and_then(|r| r.member("display"))
                .and_then(Node::as_str)
                .map(String::from)
        })
        .or_else(|| {
            resource.member("device").and_then(|r| r.member("display")).and_then(Node::as_str).map(String::from)
        })
        .or_else(|| resource.member("title").and_then(Node::as_str).map(String::from))
        .unwrap_or_else(|| rt.into());
    let status = resource
        .member("status")
        .and_then(Node::as_str)
        .map(String::from)
        .or_else(|| describe(resource.member("clinicalStatus")))
        .unwrap_or_default();
    let when = [
        "effectiveDateTime",
        "effectivePeriod",
        "onsetDateTime",
        "occurrenceDateTime",
        "performedDateTime",
        "performedPeriod",
        "recordedDate",
        "authoredOn",
        "issued",
        "dateAsserted",
    ]
    .iter()
    .find_map(|k| match resource.member(k) {
        Some(Node::Str(s)) => Some(s.clone()),
        Some(Node::Object(_)) => {
            resource.member(k).and_then(|p| p.member("start")).and_then(Node::as_str).map(String::from)
        }
        _ => None,
    })
    .unwrap_or_default();
    (what, status, when)
}

fn narrative(title: &str, rows: &[(String, String, String)], empty: Option<&str>) -> Node {
    let mut div = String::from("<div xmlns=\"http://www.w3.org/1999/xhtml\"><h2>");
    esc(&mut div, title);
    div.push_str("</h2>");
    match empty {
        Some(reason) => {
            div.push_str("<p>");
            esc(&mut div, reason);
            div.push_str("</p>");
        }
        None => {
            div.push_str("<table><thead><tr><th>Item</th><th>Status</th><th>Date</th></tr></thead><tbody>");
            for (what, status, when) in rows {
                div.push_str("<tr><td>");
                esc(&mut div, what);
                div.push_str("</td><td>");
                esc(&mut div, status);
                div.push_str("</td><td>");
                esc(&mut div, when);
                div.push_str("</td></tr>");
            }
            div.push_str("</tbody></table>");
        }
    }
    div.push_str("</div>");
    Node::Object(alloc::vec![("status".into(), Node::Str("generated".into())), ("div".into(), Node::Str(div))])
}

fn reference(url: &str) -> Node {
    Node::Object(alloc::vec![("reference".into(), Node::Str(url.into()))])
}

fn codeable(system: &str, code: &str) -> Node {
    Node::Object(alloc::vec![(
        "coding".into(),
        Node::Array(alloc::vec![Node::Object(alloc::vec![
            ("system".into(), Node::Str(system.into())),
            ("code".into(), Node::Str(code.into())),
        ])])
    )])
}

/// Rewrite every `reference` under `node` that names one of the entries —
/// `Type/id`, an absolute URL ending so, or `urn:uuid:id` — to that entry's
/// `fullUrl`.
fn rewrite_references(node: &mut Node, entries: &[(String, String)]) {
    // a relative `Type/id` names the entry with that key; an absolute URL names
    // only an entry given with exactly that fullUrl (the server is part of the
    // identity — a suffix match would attach another server's patient)
    let names = |r: &str, key: &str| -> bool {
        r == key || key.split_once('/').is_some_and(|(_, id)| r.strip_prefix("urn:uuid:") == Some(id))
    };
    match node {
        Node::Object(kvs) => {
            let is_reference = kvs.iter().any(|(k, _)| k == "reference");
            for (k, v) in kvs.iter_mut() {
                if is_reference && k == "reference" {
                    if let Node::Str(r) = v {
                        if let Some((_, full)) = entries.iter().find(|(key, _)| names(r, key)) {
                            *r = full.clone();
                        }
                    }
                } else {
                    rewrite_references(v, entries);
                }
            }
        }
        Node::Array(xs) => xs.iter_mut().for_each(|x| rewrite_references(x, entries)),
        _ => {}
    }
}

/// Assemble a document of `kind` from `inputs`:
///
/// ```json
/// {"resources": [{"resourceType": "Patient", …}, {"fullUrl": "urn:uuid:…", "resource": {…}}, …],
///  "author": "Practitioner/p1",          // a reference (or a list of them) to one of the resources
///  "date": "2026-09-16T10:00:00+10:00",   // Composition.date; also the Bundle timestamp when an instant
///  "title": "Patient Summary",            // optional
///  "custodian": "Organization/o1",        // optional
///  "identifier": "urn:uuid:…",            // optional: the document's identifier (derived otherwise)
///  "timestamp": "2026-09-16T10:00:00+10:00"} // optional when date is already an instant
/// ```
pub fn assemble(kind: Kind, inputs: &Value<'_>, validator: &Validator) -> Result<Assembled, AssembleError> {
    assemble_with(kind, inputs, validator, &crate::terminology::Bundled)
}

/// [`assemble`] with a terminology provider for the document's validation:
/// the bindings the bundled sets cannot decide, and the profile slices
/// that turn on them, are answered by it.
pub fn assemble_with(
    kind: Kind,
    inputs: &Value<'_>,
    validator: &Validator,
    tx: &dyn crate::terminology::TerminologyProvider,
) -> Result<Assembled, AssembleError> {
    let composition_profile = kind.composition().ok_or(AssembleError::UnknownKind)?;
    let Value::Object(members) = inputs else { return Err(AssembleError::InputsNotAnObject) };
    let member = |k: &str| members.iter().find(|(m, _)| m.as_ref() == k).map(|(_, v)| v);
    let text = |k: &'static str| match member(k) {
        Some(Value::Str(s)) => Ok(Some(s.to_string())),
        None => Ok(None),
        Some(_) => Err(AssembleError::Missing(k)),
    };
    let Some(Value::Array(given)) = member("resources") else { return Err(AssembleError::ResourcesNotAnArray) };
    // a bare resource, or a Bundle entry `{fullUrl, resource}` whose fullUrl
    // is kept and also names the resource for references
    let mut resources: Vec<Node> = Vec::with_capacity(given.len());
    let mut given_urls: Vec<Option<String>> = Vec::with_capacity(given.len());
    for g in given {
        let n = Node::from_value(g);
        let (url, resource) = match (n.member("fullUrl").and_then(Node::as_str), n.member("resource")) {
            (Some(url), Some(res)) if n.member("resourceType").is_none() => (Some(String::from(url)), res.clone()),
            _ => (None, n),
        };
        if !matches!(resource, Node::Object(_)) || resource.member("resourceType").and_then(Node::as_str).is_none() {
            return Err(AssembleError::ResourcesNotAnArray);
        }
        resources.push(resource);
        given_urls.push(url);
    }
    let date = text("date")?.ok_or(AssembleError::Missing("date"))?;
    let authors: Vec<String> = match member("author") {
        Some(Value::Str(s)) => alloc::vec![s.to_string()],
        Some(Value::Array(xs)) if !xs.is_empty() && xs.iter().all(|x| matches!(x, Value::Str(_))) => xs
            .iter()
            .filter_map(|x| match x {
                Value::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .collect(),
        _ => return Err(AssembleError::Missing("author")),
    };
    let title = text("title")?.unwrap_or_else(|| "Patient Summary".into());
    let custodian = text("custodian")?;
    let identifier = text("identifier")?;
    let timestamp = text("timestamp")?;

    // entries: the patient first after the composition, then the rest in order
    let patient_at = resources
        .iter()
        .position(|r| r.member("resourceType").and_then(Node::as_str) == Some("Patient"))
        .ok_or(AssembleError::NoPatient)?;
    let patient = resources.remove(patient_at);
    resources.insert(0, patient);
    let patient_url_given = given_urls.remove(patient_at);
    given_urls.insert(0, patient_url_given);
    let keys: Vec<(String, String)> = resources
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let rt = r.member("resourceType").and_then(Node::as_str).unwrap_or("");
            let key = match r.member("id").and_then(Node::as_str) {
                Some(id) => alloc::format!("{rt}/{id}"),
                None => alloc::format!("{rt}/#{i}"),
            };
            let full = match &given_urls[i] {
                Some(url) => url.clone(),
                None => alloc::format!("urn:uuid:{}", entry_uuid(&alloc::format!("{}:{key}", kind.name()))),
            };
            (key, full)
        })
        .collect();
    // a reference by the given fullUrl names the entry too
    let aliases: Vec<(String, String)> = given_urls
        .iter()
        .enumerate()
        .filter_map(|(i, u)| u.as_ref().map(|u| (u.clone(), keys[i].1.clone())))
        .chain(keys.iter().cloned())
        .collect();
    for r in resources.iter_mut() {
        rewrite_references(r, &aliases);
    }
    let author_refs: Vec<Node> = authors
        .iter()
        .map(|a| {
            let mut n = reference(a);
            rewrite_references(&mut n, &aliases);
            n
        })
        .collect();
    let custodian_ref = custodian.map(|c| {
        let mut n = reference(&c);
        rewrite_references(&mut n, &aliases);
        n
    });
    let patient_url = keys[0].1.clone();

    // sections, from the profile
    let sections = sections(composition_profile);
    let mut placed: Vec<(String, String)> = Vec::new();
    let mut per_section: Vec<Vec<usize>> = sections.iter().map(|_| Vec::new()).collect();
    for (i, r) in resources.iter().enumerate().skip(1) {
        let name = section_for(r);
        let at = name.and_then(|n| sections.iter().position(|s| s.name == n));
        match at {
            Some(si) => {
                per_section[si].push(i);
                placed.push((keys[i].1.clone(), sections[si].code.clone()));
            }
            None => placed.push((keys[i].1.clone(), String::new())),
        }
    }
    let mut section_nodes = Vec::new();
    let mut summary_rows: Vec<(String, String, String)> = Vec::new();
    for (si, s) in sections.iter().enumerate() {
        let members = &per_section[si];
        if members.is_empty() && !s.required {
            continue;
        }
        let rows: Vec<(String, String, String)> = members.iter().map(|&i| row(&resources[i])).collect();
        let mut section = Node::Object(alloc::vec![
            ("title".into(), Node::Str(s.title.into())),
            ("code".into(), codeable("http://loinc.org", &s.code)),
        ]);
        if members.is_empty() {
            // a required section with nothing to put in it says so — no
            // clinical fact ("no known allergies") is ever invented
            section.member_or_insert("text", narrative(s.title, &rows, Some("No information available")));
            section.member_or_insert(
                "emptyReason",
                codeable("http://terminology.hl7.org/CodeSystem/list-empty-reason", "unavailable"),
            );
        } else {
            section.member_or_insert("text", narrative(s.title, &rows, None));
            section.member_or_insert("entry", Node::Array(members.iter().map(|&i| reference(&keys[i].1)).collect()));
        }
        summary_rows.push((s.title.into(), alloc::format!("{} item(s)", members.len()), String::new()));
        section_nodes.push(section);
    }

    // the Composition
    // the document's identity is its content: the same inputs make the same
    // document, and any difference in them makes a different one
    let content = crate::hash::hex(&crate::hash::sha256(Node::from_value(inputs).to_json().as_bytes()));
    let comp_url = alloc::format!("urn:uuid:{}", entry_uuid(&alloc::format!("{}:composition:{content}", kind.name())));
    let comp_id = comp_url.trim_start_matches("urn:uuid:").to_string();
    let mut composition = Node::Object(alloc::vec![
        ("resourceType".into(), Node::Str("Composition".into())),
        ("id".into(), Node::Str(comp_id)),
        (
            "meta".into(),
            Node::Object(alloc::vec![(
                "profile".into(),
                Node::Array(alloc::vec![Node::Str(kind.composition_url().into())])
            )])
        ),
        ("text".into(), narrative(&title, &summary_rows, None)),
        ("status".into(), Node::Str("final".into())),
        ("type".into(), codeable("http://loinc.org", "60591-5")),
        ("subject".into(), reference(&patient_url)),
        ("date".into(), Node::Str(date.clone())),
        ("author".into(), Node::Array(author_refs)),
        ("title".into(), Node::Str(title)),
    ]);
    if let Some(c) = custodian_ref {
        composition.member_or_insert("custodian", c);
    }
    composition.member_or_insert("section", Node::Array(section_nodes));

    // the Bundle
    let doc_id = identifier.unwrap_or_else(|| {
        alloc::format!("urn:uuid:{}", entry_uuid(&alloc::format!("{}:document:{content}", kind.name())))
    });
    // the Bundle's timestamp is an instant: given, or the date when it is one
    let is_instant = |s: &str| crate::temporal::parse(s).is_some_and(|t| t.precision >= 6 && t.has_tz);
    let timestamp = match timestamp {
        Some(t) if is_instant(&t) => t,
        Some(_) => return Err(AssembleError::Missing("timestamp (an instant: to the second, with a time zone)")),
        None if is_instant(&date) => date.clone(),
        None => return Err(AssembleError::Missing("timestamp (an instant, when date is not one)")),
    };
    // a document is what the Composition links to: entries nothing reaches
    // (forward or backward) are left out, and said so
    let mut reachable = vec![false; resources.len()];
    let links: Vec<Vec<usize>> = resources
        .iter()
        .map(|r| {
            let mut refs = Vec::new();
            references_in(r, &mut refs);
            refs.iter().filter_map(|x| keys.iter().position(|(_, full)| full == x)).collect()
        })
        .collect();
    let mut comp_refs = Vec::new();
    references_in(&composition, &mut comp_refs);
    for x in &comp_refs {
        if let Some(i) = keys.iter().position(|(_, full)| full == x) {
            reachable[i] = true;
        }
    }
    loop {
        let mut grew = false;
        for i in 0..resources.len() {
            if reachable[i] {
                for &j in &links[i] {
                    if !reachable[j] {
                        reachable[j] = true;
                        grew = true;
                    }
                }
            } else if links[i].iter().any(|&j| reachable[j]) {
                reachable[i] = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let omitted: Vec<String> = keys.iter().zip(&reachable).filter(|(_, ok)| !**ok).map(|(k, _)| k.0.clone()).collect();
    placed.retain(|(full, _)| keys.iter().zip(&reachable).any(|((_, f), ok)| *ok && f == full));

    let mut entries = Vec::with_capacity(resources.len() + 1);
    let entry = |full: &str, resource: Node| {
        Node::Object(alloc::vec![("fullUrl".into(), Node::Str(full.into())), ("resource".into(), resource)])
    };
    entries.push(entry(&comp_url, composition));
    for (i, r) in resources.into_iter().enumerate() {
        if reachable[i] {
            entries.push(entry(&keys[i].1, r));
        }
    }
    let bundle = Node::Object(alloc::vec![
        ("resourceType".into(), Node::Str("Bundle".into())),
        ("id".into(), Node::Str(doc_id.trim_start_matches("urn:uuid:").into())),
        (
            "meta".into(),
            Node::Object(alloc::vec![(
                "profile".into(),
                Node::Array(alloc::vec![Node::Str(kind.bundle_url().into())])
            )])
        ),
        (
            "identifier".into(),
            Node::Object(alloc::vec![
                ("system".into(), Node::Str("urn:ietf:rfc:3986".into())),
                ("value".into(), Node::Str(doc_id)),
            ])
        ),
        ("type".into(), Node::Str("document".into())),
        ("timestamp".into(), Node::Str(timestamp)),
        ("entry".into(), Node::Array(entries)),
    ]);
    let document = bundle.to_json();
    let validation = validator.validate_unstored(document.as_bytes(), tx);
    Ok(Assembled { document, placed, omitted, validation })
}

impl Validator {
    /// Assemble a patient summary document: see [`assemble`].
    pub fn document(&self, kind: &str, inputs: &[u8]) -> Result<Assembled, AssembleError> {
        self.document_with(kind, inputs, &crate::terminology::Bundled)
    }

    /// [`Validator::document`] with a terminology provider: the document's
    /// validation — and the profile slices that turn on a binding the
    /// bundled sets cannot decide — asks it.
    pub fn document_with(
        &self,
        kind: &str,
        inputs: &[u8],
        tx: &dyn crate::terminology::TerminologyProvider,
    ) -> Result<Assembled, AssembleError> {
        let kind = Kind::parse(kind).ok_or(AssembleError::UnknownKind)?;
        let parsed = crate::json::parse(inputs).map_err(|_| AssembleError::InputsNotAnObject)?;
        assemble_with(kind, &parsed, self, tx)
    }
}
