//! # fhir-pods
//!
//! FHIR is an intermediate representation for health data with a constraint
//! system clamping it: resources are the IR; profiles, terminology bindings,
//! and FHIRPath invariants are the constraints; validation is the verifier pass.
//!
//! Every incumbent ships that verifier as a *server*. A verifier is stateless,
//! CPU-bound, pure compute — a **library**. `fhir-pods-core` is that library:
//! no I/O, no telemetry, no licensed terminology, source-agnostic, `no_std`.
//!
//! Two layers, per the API decision (see `docs/incumbent-apis.md`):
//! * **ergonomic** — [`validate_bytes`] returns a typed [`Validation`] with a
//!   complete issue list; it never throws to signal validation state.
//! * **wire** — [`Validation::to_operation_outcome`] emits the spec's
//!   `OperationOutcome` at the boundary, so anything downstream sees a
//!   standard FHIR validator.
//!
//! Profiles are generated from the packages' shipped `snapshot.element` into
//! the compiled IR (`compiled`), then run by the generic `engine` against a
//! `document::Document` port. See `docs/domain-model.md`.
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
    let (gaps, nested) = parsed.structure();
    for (path, kind, message) in gaps {
        report.push(Severity::Error, kind, &path, message);
    }
    let bundle = (rt == "Bundle").then_some(&doc);
    if let Some(b) = bundle {
        engine::check_document(b, &mut report);
    }
    let defaults = cache.map(|v| v.defaults.as_slice()).unwrap_or(&[]);
    let profiles = validate_resource(rt, &doc, "", tx, cache, bundle, defaults, &mut report);
    // every resource nested in this one (Bundle entries, contained resources,
    // Parameters) is held to its own declared profiles, in this bundle's scope
    for (label, nested) in nested {
        let Some(nrt) = nested.scalars("$this.resourceType").first().copied() else { continue };
        let Some(nrt) = typing::known_type(nrt).filter(|t| typemap::RESOURCES.binary_search(t).is_ok()) else {
            continue;
        };
        validate_resource(nrt, &nested, &label, tx, cache, bundle, &[], &mut report);
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
    report: &mut Report,
) -> Vec<&'static CompiledProfile> {
    let at = |path: &str| if label.is_empty() { String::from(path) } else { format!("{label}.{path}") };
    for url in doc.scalars("$this.meta.profile") {
        let base = url.split('|').next().unwrap_or(url);
        match generated::REGISTRY.iter().find(|p| p.url == base) {
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
    let evaluated = engine::Evaluated::default();
    let mut issues = Vec::new();
    if profiles.is_empty() {
        // the base's own rules for a domain resource: dom-3 (contained
        // resources referenced) and dom-6 (narrative); dom-2/4/5 are in the walk
        if !matches!(rt, "Bundle" | "Binary" | "Parameters") {
            issues.extend(engine::domain_invariants(doc, rt, cache, bundle).issues);
        }
    }
    for profile in &profiles {
        issues.extend(engine::validate_at_with(profile, doc, doc, tx, 0, cache, bundle, Some(&evaluated)).issues);
    }
    for mut issue in issues {
        if !label.is_empty() {
            issue.path = at(&issue.path);
        }
        if !report.issues.contains(&issue) {
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
            generated::REGISTRY.iter().find(|p| p.resource_type == resource_type && p.url == url).copied()
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
                    Some(b) => match generated::REGISTRY.iter().find(|q| q.url == b) {
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
                    c.scalars("$this.code").contains(&code) && c.scalars("$this.system").contains(&"http://loinc.org")
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

/// [`Validator::lower`] without a validator (the invariants are re-parsed).
pub fn lower_bytes(url: &str, inputs: &[u8]) -> Result<lower::Lowered, lower::LowerError> {
    let url = url.split('|').next().unwrap_or(url);
    let profile = generated::REGISTRY.iter().find(|p| p.url == url).ok_or(lower::LowerError::UnknownProfile)?;
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

pub mod json;
pub mod lower;

pub mod document;

pub mod compiled;

/// A validator that has parsed the registry's invariant expressions once.
/// Build one per process (about half a millisecond) and reuse it: the free
/// functions [`validate_bytes`] / [`validate_bytes_with`] re-parse every
/// invariant on every call and cost roughly twice as much.
pub struct Validator {
    /// Parsed ASTs keyed by the expression's `'static` address — the registry
    /// shares expression strings, so this is small and the lookup is a binary
    /// search on an integer.
    invariants: Vec<(usize, Option<path::Expr>)>,
    /// Profiles applied to resources that declare none, per resource type —
    /// a jurisdiction's "this is what a Patient means here". Empty by default:
    /// the spec's behaviour, and the reference validator's.
    defaults: Vec<(&'static str, &'static CompiledProfile)>,
}

impl Default for Validator {
    fn default() -> Self {
        Self::new()
    }
}

impl Validator {
    pub fn new() -> Self {
        let mut keys: Vec<(usize, &'static str)> = Vec::new();
        for (_, _, _, expr) in typemap::DOMAIN_INVARIANTS {
            keys.push((expr.as_ptr() as usize, expr));
        }
        for p in generated::REGISTRY {
            let slice_invariants = p.slicings.iter().flat_map(|s| s.slices.iter()).flat_map(|sl| sl.invariants.iter());
            for inv in p.invariants.iter().chain(slice_invariants) {
                keys.push((inv.expression.as_ptr() as usize, inv.expression));
            }
        }
        keys.sort_unstable_by_key(|k| k.0);
        keys.dedup_by_key(|k| k.0);
        Self { invariants: keys.into_iter().map(|(k, e)| (k, path::parse(e).ok())).collect(), defaults: Vec::new() }
    }

    /// Apply `url`'s profile to every `resource_type` resource that declares
    /// no profile of its own (e.g. AU Core Patient for a bare `Patient`).
    /// Unknown URLs are ignored: nothing is ever guessed.
    pub fn with_default_profile(mut self, url: &str) -> Self {
        if let Some(p) = generated::REGISTRY.iter().find(|p| p.url == url) {
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

    /// Build a conformant resource for the profile at `url` from its variable
    /// inputs (a JSON object keyed by slot path), and validate it. See
    /// [`lower`](crate::lower).
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
        let url = url.split('|').next().unwrap_or(url);
        let profile = generated::REGISTRY.iter().find(|p| p.url == url).ok_or(lower::LowerError::UnknownProfile)?;
        let parsed = json::parse(inputs).map_err(|_| lower::LowerError::InputsNotAnObject)?;
        lower::lower_with(profile, &parsed, Some(self), tx)
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

pub mod validate;

pub mod path;

pub mod temporal;

pub mod regex;

pub mod terminology;
