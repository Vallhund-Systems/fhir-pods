//! # fhir-pods — compliance runtime
//!
//! An embeddable runtime for health applications to speak compliance at their
//! edges: lift and lower FHIR, validate resources, apply de-identification
//! policies, assemble patient summaries, construct audit records, and convert
//! HL7 v2 messages. The host supplies its domain mappings and boundary context.
//!
//! FHIR resources are the intermediate representation; profiles, terminology
//! bindings, and FHIRPath invariants constrain it. Compiled definitions drive
//! extraction and construction as well as the verifier pass.
//!
//! [`Validator`] is the shared operation context: it caches invariant syntax
//! trees and provides the lift/lower, validation, transformation, document,
//! audit, and v2 methods. Outputs carry validation and operation-specific
//! coverage information. Failure to start an operation is a separate error.
//!
//! [`Validation::to_operation_outcome`] emits FHIR's `OperationOutcome` wire
//! representation. Delivery bindings expose the operation family through a
//! shared C ABI, WebAssembly, Expo, and a CLI.
//!
//! The core uses `no_std` with allocation, has no external dependencies, and
//! forbids unsafe code. It performs no I/O; the host owns policy selection,
//! external terminology, consent, access, storage, transport, and decisions
//! about incomplete coverage. See `docs/architecture.md` and `docs/scope.md`.
#![no_std]
#![forbid(unsafe_code)]

// `no_std` + `alloc`: the core allocates (the value tree, the issue list) but
// never touches the OS — no I/O, no threads, no clock. The compiler enforces
// the thesis: a `std::fs` call here is a build error, not a code-review catch.
#[macro_use]
extern crate alloc;

use alloc::{string::String, vec::Vec};
use compiled::CompiledProfile;
use document::Document;
use validate::{IssueType, Report, Severity};

/// Compiled profiles generated from FHIR packages (`tools/gen_all.py`), with a
/// `REGISTRY` keyed by canonical URL.
pub mod generated;

#[cfg(feature = "testcase-profiles")]
#[doc(hidden)]
pub mod generated_testcases {
    include!("generated_testcases/mod.rs");
}

/// A host's own profiles, compiled at build time (`scripts/gen-host-profiles.sh`).
#[cfg(feature = "host-profiles")]
#[doc(hidden)]
pub mod generated_host {
    include!("generated_host/mod.rs");
}

#[cfg(feature = "testcase-profiles")]
const TESTCASE_REGISTRY: &[&CompiledProfile] = generated_testcases::REGISTRY;
#[cfg(all(feature = "host-profiles", not(feature = "testcase-profiles")))]
const TESTCASE_REGISTRY: &[&CompiledProfile] = &[];
#[cfg(feature = "host-profiles")]
const HOST_REGISTRY: &[&CompiledProfile] = generated_host::REGISTRY;
#[cfg(all(feature = "testcase-profiles", not(feature = "host-profiles")))]
const HOST_REGISTRY: &[&CompiledProfile] = &[];

/// The shipped registry, plus the isolated test context and the host's own
/// profiles in an opt-in build — the shipped profiles first, so a host
/// profile never shadows a shipped canonical (the generator refuses one).
#[cfg(not(any(feature = "testcase-profiles", feature = "host-profiles")))]
pub use generated::REGISTRY;

#[cfg(any(feature = "testcase-profiles", feature = "host-profiles"))]
pub static REGISTRY: &[&CompiledProfile] = &{
    const N: usize = generated::REGISTRY.len() + TESTCASE_REGISTRY.len() + HOST_REGISTRY.len();
    let mut profiles = [generated::REGISTRY[0]; N];
    let mut n = 0;
    let parts: [&[&CompiledProfile]; 3] = [generated::REGISTRY, TESTCASE_REGISTRY, HOST_REGISTRY];
    let mut p = 0;
    while p < parts.len() {
        let part = parts[p];
        let mut i = 0;
        while i < part.len() {
            profiles[n] = part[i];
            n += 1;
            i += 1;
        }
        p += 1;
    }
    profiles
};

/// The definitions the packages hold under HL7's own URL spaces (generated).
pub(crate) mod known_urls;
/// The element-type table generated from the base FHIR package
/// (`tools/gen_typemap.py`): `Type.element` → its FHIR type(s).
mod typemap;

pub mod typing;

/// The ergonomic result: what was validated, against which profile, and every
/// finding. Never a thrown error — validation state is data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    pub resource_type: String,
    /// The profile that was applied, if any (its human title; the first when
    /// several were).
    pub profile: Option<&'static str>,
    /// Every profile applied — a resource may declare more than one, and
    /// must conform to all of them.
    pub profiles: Vec<&'static str>,
    pub report: Report,
}

impl Validation {
    pub fn is_valid(&self) -> bool {
        self.report.is_valid()
    }

    /// The spec resource — a FHIR `OperationOutcome` — for the wire.
    pub fn to_operation_outcome(&self) -> String {
        wire::operation_outcome(&self.report)
    }
}

/// Validate a resource. Profile selection follows the spec: a declared
/// `meta.profile` picks the generated profile; otherwise the resource type's
/// default applies.
pub fn validate_bytes(input: &[u8]) -> Validation {
    validate_bytes_with(input, &terminology::Bundled)
}

/// [`validate_bytes`] with a host terminology provider behind the bundled sets.
pub fn validate_bytes_with(input: &[u8], tx: &dyn terminology::TerminologyProvider) -> Validation {
    validate_bytes_impl(input, tx, None)
}

fn validate_bytes_impl(
    input: &[u8],
    tx: &dyn terminology::TerminologyProvider,
    cache: Option<&Validator>,
) -> Validation {
    validate_bytes_in(input, tx, cache, &[])
}

/// [`validate_bytes_impl`] with Questionnaires beyond the validator's own —
/// what a host loaded into a shared store (the C ABI's).
fn validate_bytes_in(
    input: &[u8],
    tx: &dyn terminology::TerminologyProvider,
    cache: Option<&Validator>,
    extra: &[questionnaire::Questionnaire],
) -> Validation {
    // The scanner's contract, kept: a resource is a top-level object.
    let first = input.iter().position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r')).unwrap_or(input.len());
    if input.get(first) != Some(&b'{') {
        return parse_failure(json::ParseError { at: first, msg: "unexpected byte" });
    }
    let parsed = match document::Parsed::parse(input) {
        Ok(p) => p,
        Err(e) => return parse_failure(e),
    };
    let doc = parsed.doc();
    let rt_member = parsed.top_member("resourceType");
    let rt = match rt_member {
        None => {
            let mut report = Report::default();
            report.push(Severity::Error, IssueType::Structure, "resourceType", "resourceType is required".into());
            return Validation { resource_type: "Unknown".into(), profile: None, profiles: Vec::new(), report };
        }
        Some(json::Value::Str(s)) => s.as_ref(),
        Some(_) => {
            let mut report = Report::default();
            report.push(Severity::Error, IssueType::Structure, "resourceType", "resourceType must be a string".into());
            return Validation { resource_type: "Unknown".into(), profile: None, profiles: Vec::new(), report };
        }
    };
    let mut report = Report::default();
    // A resource type the base spec does not define cannot be checked at
    // all — and is not a FHIR resource.
    let rt: &'static str = match typing::known_type(rt).filter(|t| typemap::RESOURCES.binary_search(t).is_ok()) {
        Some(t) => t,
        None => {
            report.push(
                Severity::Error,
                IssueType::Structure,
                "resourceType",
                format!("'{rt}' is not a FHIR R4 resource type"),
            );
            return Validation { resource_type: rt.into(), profile: None, profiles: Vec::new(), report };
        }
    };
    // The base spec's shape rules, profile or no profile: what every element
    // is, its JSON kind, whether it repeats, what a datatype requires.
    let (gaps, warnings, nested, extensions) = parsed.structure_full(tx);
    for (path, kind, message) in gaps {
        let severity = if kind == IssueType::NotSupported { Severity::Warning } else { Severity::Error };
        report.push(severity, kind, &path, message);
    }
    for (path, kind, message) in warnings {
        report.push(Severity::Warning, kind, &path, message);
    }
    let bundle = (rt == "Bundle").then_some(&doc);
    if let Some(b) = bundle {
        engine::check_document(b, &mut report);
        engine::check_reference_targets_base(b, &mut report);
    }
    let defaults = cache.map(|v| v.defaults.as_slice()).unwrap_or(&[]);
    // one memo for the whole document: entries indexed once, each target's
    // conformance to each profile remembered
    let memo = engine::Memo::new();
    let profiles = validate_resource(rt, &doc, "", tx, cache, bundle, defaults, &memo, &mut report);
    // a QuestionnaireResponse against its Questionnaire (a contained one, a
    // bundle entry, or one loaded into the validator); a Questionnaire's own
    // enableWhen wiring
    let own = cache.map(|v| v.questionnaires.as_slice()).unwrap_or(&[]);
    let joined: Vec<questionnaire::Questionnaire>;
    let questionnaires: &[questionnaire::Questionnaire] = if extra.is_empty() {
        own
    } else {
        joined = own.iter().chain(extra).cloned().collect();
        &joined
    };
    let bundle_value = bundle.map(|b| b.value());
    let by_kind = |rt: &str, value: &json::Value<'_>, label: &str, report: &mut Report| match rt {
        "QuestionnaireResponse" => questionnaire::check(value, bundle_value, questionnaires, tx, label, report),
        "Questionnaire" => questionnaire::check_questionnaire(value, label, report),
        _ => {}
    };
    by_kind(rt, parsed.value(), rt, &mut report);
    // every resource nested in this one (Bundle entries, contained resources,
    // Parameters) is held to its own declared profiles, in this bundle's scope
    for (label, nested) in nested {
        let Some(nrt) = nested.scalars("$this.resourceType").first().copied() else { continue };
        let Some(nrt) = typing::known_type(nrt).filter(|t| typemap::RESOURCES.binary_search(t).is_ok()) else {
            continue;
        };
        validate_resource(nrt, &nested, &label, tx, cache, bundle, &[], &memo, &mut report);
        by_kind(nrt, nested.value(), &format!("{label}.{nrt}"), &mut report);
    }
    // every extension whose url names a compiled extension profile is held
    // to it, wherever it sits (a translation on a coding's display…)
    for (label, ext, profile) in extensions {
        let evaluated = engine::Evaluated::new(Some(&memo));
        for mut issue in engine::validate_at_with(profile, &ext, &ext, tx, 0, cache, bundle, Some(&evaluated)).issues {
            issue.path = format!("{label}.{}", issue.path);
            if !report.issues.contains(&issue) {
                report.issues.push(issue);
            }
        }
    }
    // No declared profile we have compiled (and no default configured on the
    // validator): the base resource applies — the shape walk and the base's
    // own invariants above are what it says — and nothing more is claimed.
    if profiles.is_empty() {
        return Validation { resource_type: rt.into(), profile: None, profiles: Vec::new(), report };
    }
    Validation {
        resource_type: rt.into(),
        profile: Some(profiles[0].name),
        profiles: profiles.iter().map(|p| p.name).collect(),
        report,
    }
}

/// Validate one resource (the root, or one nested in it, at `label`) against
/// the profiles it declares: unknown ones are warned about, a compiled one
/// for another resource type is an error, the rest are applied; with none
/// applied, `DomainResource`'s own invariants stand in. Returns the profiles
/// applied.
#[allow(clippy::too_many_arguments)]
fn validate_resource<D: Document>(
    rt: &'static str,
    doc: &D,
    label: &str,
    tx: &dyn terminology::TerminologyProvider,
    cache: Option<&Validator>,
    bundle: Option<&D>,
    defaults: &[(&'static str, &'static CompiledProfile)],
    memo: &engine::Memo<D>,
    report: &mut Report,
) -> Vec<&'static CompiledProfile> {
    let at = |path: &str| if label.is_empty() { String::from(path) } else { format!("{label}.{path}") };
    for url in doc.scalars("$this.meta.profile") {
        let base = url.split('|').next().unwrap_or(url);
        match crate::REGISTRY.iter().find(|p| p.url == base) {
            None => report.push(
                Severity::Warning,
                IssueType::NotSupported,
                &at("meta.profile"),
                format!("declared profile '{url}' is not compiled here, so it was not checked"),
            ),
            Some(p) if p.resource_type != rt => report.push(
                Severity::Error,
                IssueType::Structure,
                &at("meta.profile"),
                format!("declared profile '{url}' applies to {}, not to a {rt}", p.resource_type),
            ),
            Some(_) => {}
        }
    }
    let profiles = select_profiles(rt, doc, defaults);
    let evaluated = engine::Evaluated::new(Some(memo));
    let mut issues = Vec::new();
    if profiles.is_empty() {
        // the base's own rules for a domain resource: dom-3 (contained
        // resources referenced) and dom-6 (narrative); dom-2/4/5 are in the walk
        if rt == "Bundle" {
            issues.extend(engine::bundle_invariants(doc, cache, bundle).issues);
        } else if !matches!(rt, "Binary" | "Parameters") {
            issues.extend(engine::domain_invariants(doc, rt, cache, bundle).issues);
            // and the resource type's own (org-1, pat-1, cmp-1 …)
            issues.extend(engine::resource_invariants(doc, rt, cache, bundle).issues);
        }
    }
    for profile in &profiles {
        issues.extend(engine::validate_at_with(profile, doc, doc, tx, 0, cache, bundle, Some(&evaluated)).issues);
    }
    // a duplicate can only come from this resource's own profiles (the same
    // invariant reached through two of them), so the check stays local: a
    // bundle of thousands of entries must not pay for every earlier entry
    let start = report.issues.len();
    for mut issue in issues {
        if !label.is_empty() {
            issue.path = at(&issue.path);
        }
        if !report.issues[start..].contains(&issue) {
            report.issues.push(issue);
        }
    }
    profiles
}

/// Profile selection, as the spec has it: every declared `meta.profile` we
/// have compiled applies (in declaration order; a `|version` suffix is
/// accepted — the registry pins one version per IG); when none is declared,
/// the validator's configured default for the resource type, if any.
fn select_profiles(
    resource_type: &str,
    doc: &impl Document,
    defaults: &[(&'static str, &'static CompiledProfile)],
) -> Vec<&'static CompiledProfile> {
    let mut out: Vec<&'static CompiledProfile> = doc
        .scalars("$this.meta.profile")
        .iter()
        .filter_map(|url| {
            let url = url.split('|').next().unwrap_or(url);
            crate::REGISTRY.iter().find(|p| p.resource_type == resource_type && p.url == url).copied()
        })
        .collect();
    out.dedup_by_key(|p| p.url);
    if out.is_empty() {
        out.extend(defaults.iter().filter(|(rt, _)| *rt == resource_type).map(|(_, p)| *p));
    }
    // The spec's own rule, and the reference validator's: an Observation
    // that carries a base vital-sign profile's LOINC code (or, for the
    // generic profile, the vital-signs category) conforms to that profile.
    if resource_type == "Observation" {
        // does an applied profile derive from `url` (its snapshot already carries it)?
        let derives_from = |mut from: &'static CompiledProfile, url: &str| -> bool {
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
        };
        let mut implied: Vec<&'static CompiledProfile> = Vec::new();
        for p in generated::IMPLIED {
            if out.iter().any(|q| q.url == p.url) {
                continue;
            }
            // the profile's fixed LOINC code, whatever the slice label on `coding`
            let unlabelled = |path: &str| -> String {
                path.split('.').map(|seg| seg.split(':').next().unwrap_or(seg)).collect::<Vec<_>>().join(".")
            };
            let fixed =
                |leaf: &str| p.lowering.fixed.iter().find(|(path, _)| unlabelled(path) == leaf).map(|(_, v)| *v);
            let loinc = fixed("code.coding.system") == Some("http://loinc.org");
            let carries = match fixed("code.coding.code").filter(|_| loinc) {
                Some(code) => doc.instances("$this.code.coding").iter().any(|c| {
                    c.scalars("$this.system").contains(&"http://loinc.org")
                        && c.scalars("$this.code").iter().any(|got| *got == code || implied_alias(p.url, got))
                }),
                None => doc.instances("$this.category.coding").iter().any(|c| {
                    c.scalars("$this.code").contains(&"vital-signs")
                        && c.scalars("$this.system")
                            .contains(&"http://terminology.hl7.org/CodeSystem/observation-category")
                }),
            };
            if carries {
                implied.push(p);
            }
        }
        // the most specific only: a base profile an applied (or implied)
        // profile derives from is already carried by that profile's snapshot
        let all: Vec<&'static CompiledProfile> = out.iter().chain(implied.iter()).copied().collect();
        out.extend(implied.iter().filter(|p| !all.iter().any(|q| derives_from(q, p.url))).copied());
    }
    out
}

/// The further LOINC codes the reference validator takes as implying a
/// vital-sign profile, beyond the profile's own magic code: its code table,
/// probed one code at a time against 6.10.4 (an Observation in the
/// vital-signs category carrying only that code is held to the profile).
/// Not implied there, and so not here: 8334-1, 8335-2, 8462-4, 8478-0.
fn implied_alias(profile: &str, code: &str) -> bool {
    const ALIASES: &[(&str, &[&str])] = &[
        ("/bodyheight", &["3137-7", "8306-3", "8308-9"]),
        ("/bodyweight", &["3141-9"]),
        ("/bodytemp", &["8328-7", "8331-1", "8332-9"]),
        ("/oxygensat", &["59408-5"]),
        ("/bp", &["55284-4", "8480-6"]),
        ("/headcircum", &["8287-5"]),
    ];
    ALIASES.iter().any(|(suffix, codes)| profile.ends_with(suffix) && codes.contains(&code))
}

/// The compiled profile a host may lower into, lift from or ask the slots
/// of: found by canonical (a `|version` ignored), and a *resource* profile —
/// an extension's profile is compiled so its placement and content can be
/// checked, not so a host can build an `Extension` as if it were a resource.
pub fn resource_profile(url: &str) -> Option<&'static compiled::CompiledProfile> {
    let url = url.split('|').next().unwrap_or(url);
    crate::REGISTRY.iter().find(|p| p.url == url).filter(|p| p.resource_type != "Extension").copied()
}

/// What is compiled in, for a host to discover without the specification:
/// `{"regions": [{"name", "title", "guide", "document"}], "profiles":
/// [{"url", "name", "resourceType", "kind", "base"}]}` — every registry
/// profile (`kind`: `resource` a host may lower into, lift from or export
/// through; `datatype` a slice inherits; `extension`), and the regions
/// `export` speaks.
pub fn catalogue_json() -> String {
    use wire::escape;
    let mut out = String::from("{\"regions\":[");
    for (i, r) in export::REGIONS.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":\"");
        out.push_str(r.name);
        out.push_str("\",\"title\":\"");
        escape(&mut out, r.title);
        out.push_str("\",\"guide\":\"");
        escape(&mut out, r.guide);
        out.push_str("\",\"document\":");
        match r.document {
            Some(k) => {
                out.push('"');
                out.push_str(k.name());
                out.push('"');
            }
            None => out.push_str("null"),
        }
        out.push('}');
    }
    out.push_str("],\"profiles\":[");
    for (i, p) in crate::REGISTRY.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"url\":\"");
        escape(&mut out, p.url);
        out.push_str("\",\"name\":\"");
        escape(&mut out, p.name);
        out.push_str("\",\"resourceType\":\"");
        escape(&mut out, p.resource_type);
        out.push_str("\",\"kind\":\"");
        out.push_str(if p.resource_type == "Extension" {
            "extension"
        } else if typemap::RESOURCES.binary_search(&p.resource_type).is_ok() {
            "resource"
        } else {
            "datatype"
        });
        out.push_str("\",\"base\":");
        match p.base {
            Some(b) => {
                out.push('"');
                escape(&mut out, b);
                out.push('"');
            }
            None => out.push_str("null"),
        }
        out.push('}');
    }
    out.push_str("]}");
    out
}

/// [`Validator::lower`] without a validator (the invariants are re-parsed).
pub fn lower_bytes(url: &str, inputs: &[u8]) -> Result<lower::Lowered, lower::LowerError> {
    let profile = resource_profile(url).ok_or(lower::LowerError::UnknownProfile)?;
    let parsed = json::parse(inputs).map_err(|_| lower::LowerError::InputsNotAnObject)?;
    lower::lower_with(profile, &parsed, None, &terminology::Bundled)
}

/// What a host must resolve for `input` to get a verdict free of terminology
/// warnings: every binding the bundled sets could not decide, deduplicated.
/// Resolve them (`$validate-code`, or `$expand` and compile the sets — see
/// [`terminology::Expansions`]) and validate again with the answers.
pub fn terminology_needs(input: &[u8]) -> Vec<terminology::Need> {
    let rec = terminology::Recorder::new();
    validate_bytes_with(input, &terminology::Chain(&[&terminology::Bundled, &rec]));
    rec.needs()
}

/// `wire::report(&validate_bytes(input))` — the `Validation` JSON, as text.
pub fn probe(input: &[u8]) -> String {
    wire::report(&validate_bytes(input))
}

/// The selected profile's must-support elements this resource leaves empty
/// (where their parent is present). Not conformance — no validator fails on
/// must-support — but what a client certifying against the IG must populate
/// when it has the data. Empty for unparseable input or no applicable profile.
pub fn must_support_gaps(input: &[u8]) -> Vec<&'static str> {
    let Ok(parsed) = document::Parsed::parse(input) else { return Vec::new() };
    let Some(json::Value::Str(rt)) = parsed.top_member("resourceType") else { return Vec::new() };
    let doc = parsed.doc();
    let mut gaps = Vec::new();
    for profile in select_profiles(rt, &doc, &[]) {
        for g in engine::must_support_gaps(profile, &doc) {
            if !gaps.contains(&g) {
                gaps.push(g);
            }
        }
    }
    gaps
}

fn parse_failure(e: json::ParseError) -> Validation {
    let mut report = Report::default();
    report.push(
        Severity::Fatal,
        IssueType::Structure,
        "(document)",
        format!("could not parse JSON at byte {}: {}", e.at, e.msg),
    );
    Validation { resource_type: "Unknown".into(), profile: None, profiles: Vec::new(), report }
}

pub mod wire;

/// An owned JSON tree (lowering builds one; de-identification rewrites one).
pub mod node;

/// SHA-256 / HMAC-SHA256, for pseudonyms.
pub mod hash;

/// De-identification under a data policy (Safe Harbor by default).
pub mod deidentify;

/// Documents: IPS / AU Patient Summary bundles assembled from resources.
pub mod assemble;

pub mod audit;

/// Export: a record re-profiled for a jurisdiction, its summary document and audit event.
pub mod export;

pub mod json;
pub mod lower;

pub mod lift;

pub mod v2;

pub mod document;
/// A QuestionnaireResponse against its Questionnaire.
pub mod questionnaire;

pub mod compiled;

/// Shared context for the compliance runtime's operations, retaining the
/// `Validator` API name. Parses the registry's invariant expressions once for
/// reuse by validation, lift/lower, de-identification, documents, audit, and v2.
/// The free [`validate_bytes`] / [`validate_bytes_with`] functions parse
/// invariant expressions on each call. `Clone` copies the parsed
/// expressions (a host that holds one behind a lock clones it to load a
/// Questionnaire into a fresh, consistent one).
#[derive(Clone)]
pub struct Validator {
    /// Parsed ASTs keyed by the expression's `'static` address — the registry
    /// shares expression strings, so this is small and the lookup is a binary
    /// search on an integer.
    invariants: Vec<(usize, Option<path::Expr>)>,
    /// Profiles applied to resources that declare none, per resource type —
    /// a jurisdiction's "this is what a Patient means here". Empty by default:
    /// the spec's behaviour, and the reference validator's.
    defaults: Vec<(&'static str, &'static CompiledProfile)>,
    /// Questionnaires the host loaded: what a `QuestionnaireResponse` that
    /// names one by canonical is held to (a contained one or a bundle entry
    /// is found without loading).
    questionnaires: Vec<questionnaire::Questionnaire>,
}

impl Default for Validator {
    fn default() -> Self {
        Self::new()
    }
}

impl Validator {
    pub fn new() -> Self {
        // keyed by the expression's address: one parse per distinct static
        // string, rewritten as the reference validator rewrites it (by its
        // invariant key where that matters — `path::parse_invariant`)
        let mut keys: Vec<(usize, &'static str, &'static str)> = Vec::new();
        for (key, _, _, expr) in typemap::DOMAIN_INVARIANTS {
            keys.push((expr.as_ptr() as usize, key, expr));
        }
        for (_, key, _, _, expr) in typemap::BUNDLE_INVARIANTS {
            keys.push((expr.as_ptr() as usize, key, expr));
        }
        for (_, _, key, _, _, expr, _) in typemap::RESOURCE_INVARIANTS {
            keys.push((expr.as_ptr() as usize, key, expr));
        }
        for p in crate::REGISTRY {
            let slice_invariants = p.slicings.iter().flat_map(|s| s.slices.iter()).flat_map(|sl| sl.invariants.iter());
            for inv in p.invariants.iter().chain(slice_invariants) {
                keys.push((inv.expression.as_ptr() as usize, inv.key, inv.expression));
            }
        }
        keys.sort_unstable_by_key(|k| k.0);
        keys.dedup_by_key(|k| k.0);
        Self {
            invariants: keys.into_iter().map(|(k, key, e)| (k, path::parse_invariant(key, e).ok())).collect(),
            defaults: Vec::new(),
            questionnaires: Vec::new(),
        }
    }

    /// Apply `url`'s profile to every `resource_type` resource that declares
    /// no profile of its own (e.g. AU Core Patient for a bare `Patient`).
    /// Load a `Questionnaire` (JSON) for responses that name it by canonical
    /// (`url`, or `url|version`). Returns whether the bytes were one; a
    /// second load of the same canonical replaces the first.
    pub fn load_questionnaire(&mut self, bytes: &[u8]) -> bool {
        let Some(q) = questionnaire::Questionnaire::parse(bytes) else { return false };
        self.questionnaires.retain(|k| !(k.url == q.url && k.version == q.version));
        self.questionnaires.push(q);
        true
    }

    /// [`load_questionnaire`](Self::load_questionnaire), builder-style; bytes
    /// that are not a Questionnaire are ignored.
    pub fn with_questionnaire(mut self, bytes: &[u8]) -> Self {
        self.load_questionnaire(bytes);
        self
    }

    /// Forget every loaded Questionnaire (a contained one or a bundle entry
    /// is still found).
    pub fn clear_questionnaires(&mut self) {
        self.questionnaires.clear();
    }

    /// Unknown URLs are ignored: nothing is ever guessed.
    pub fn with_default_profile(mut self, url: &str) -> Self {
        if let Some(p) = crate::REGISTRY.iter().find(|p| p.url == url) {
            self.defaults.push((p.resource_type, p));
        }
        self
    }

    /// The parsed expression, or `None` when it is outside the supported
    /// FHIRPath core (skipped, never guessed).
    fn expr(&self, expression: &'static str) -> Option<&path::Expr> {
        let k = expression.as_ptr() as usize;
        match self.invariants.binary_search_by_key(&k, |x| x.0) {
            Ok(i) => self.invariants[i].1.as_ref(),
            Err(_) => None,
        }
    }

    /// [`validate_bytes`], with the invariants already parsed.
    pub fn validate(&self, input: &[u8]) -> Validation {
        validate_bytes_impl(input, &terminology::Bundled, Some(self))
    }

    /// [`validate_bytes_with`], with the invariants already parsed.
    pub fn validate_with(&self, input: &[u8], tx: &dyn terminology::TerminologyProvider) -> Validation {
        validate_bytes_impl(input, tx, Some(self))
    }

    /// [`validate_with`](Self::validate_with), with Questionnaires a
    /// `QuestionnaireResponse` may name beyond the ones loaded here.
    pub fn validate_in(
        &self,
        input: &[u8],
        tx: &dyn terminology::TerminologyProvider,
        questionnaires: &[questionnaire::Questionnaire],
    ) -> Validation {
        validate_bytes_in(input, tx, Some(self), questionnaires)
    }

    /// Build a conformant resource for the profile at `url` from its variable
    /// inputs (a JSON object keyed by slot path), and validate it. See
    /// [`lower`].
    pub fn lower(&self, url: &str, inputs: &[u8]) -> Result<lower::Lowered, lower::LowerError> {
        self.lower_with(url, inputs, &terminology::Bundled)
    }

    /// [`Validator::lower`] with a terminology provider: the validation asks
    /// it about bindings, and a coding given as `{system, code}` gets its
    /// `display` from it when it knows one.
    pub fn lower_with(
        &self,
        url: &str,
        inputs: &[u8],
        tx: &dyn terminology::TerminologyProvider,
    ) -> Result<lower::Lowered, lower::LowerError> {
        let profile = resource_profile(url).ok_or(lower::LowerError::UnknownProfile)?;
        let parsed = json::parse(inputs).map_err(|_| lower::LowerError::InputsNotAnObject)?;
        lower::lower_with(profile, &parsed, Some(self), tx)
    }

    /// An `AuditEvent` for a read, query, create, update or delete of a
    /// record (`kind`: `read`, `patient-read`, `query`, `patient-query`,
    /// `create`, `patient-create`, `update`, `patient-update`, `delete`,
    /// `patient-delete`), conformant to the IHE BALP profile for it, built
    /// from host-shaped inputs and validated on the way out — see
    /// [`audit`].
    pub fn audit(&self, kind: &str, inputs: &[u8]) -> Result<lower::Lowered, audit::AuditError> {
        self.audit_with(kind, inputs, &terminology::Bundled)
    }

    /// [`Validator::audit`] with a terminology provider.
    pub fn audit_with(
        &self,
        kind: &str,
        inputs: &[u8],
        tx: &dyn terminology::TerminologyProvider,
    ) -> Result<lower::Lowered, audit::AuditError> {
        let kind = audit::Kind::parse(kind).ok_or(audit::AuditError::UnknownKind)?;
        let parsed = json::parse(inputs).map_err(|_| audit::AuditError::InputsNotAnObject)?;
        audit::audit_with(kind, &parsed, Some(self), tx)
    }

    /// [`terminology_needs`], with the invariants already parsed.
    pub fn terminology_needs(&self, input: &[u8]) -> Vec<terminology::Need> {
        let rec = terminology::Recorder::new();
        self.validate_with(input, &terminology::Chain(&[&terminology::Bundled, &rec]));
        rec.needs()
    }

    /// [`must_support_gaps`], honouring this validator's default profiles.
    pub fn must_support_gaps(&self, input: &[u8]) -> Vec<&'static str> {
        let Ok(parsed) = document::Parsed::parse(input) else { return Vec::new() };
        let Some(json::Value::Str(rt)) = parsed.top_member("resourceType") else { return Vec::new() };
        let doc = parsed.doc();
        let mut gaps = Vec::new();
        for profile in select_profiles(rt, &doc, &self.defaults) {
            for g in engine::must_support_gaps(profile, &doc) {
                if !gaps.contains(&g) {
                    gaps.push(g);
                }
            }
        }
        gaps
    }
}

pub mod engine;

pub mod value_sets;

/// The bundled value sets, generated from the packages (`tools/gen_valuesets.py`).
mod value_sets_table;

/// The base code systems' content (generated): what a Coding's code is held to.
pub mod code_systems_table;
pub mod rules;
pub mod validate;
pub mod xhtml;

pub mod path;

pub mod temporal;

pub mod regex;

pub mod terminology;
