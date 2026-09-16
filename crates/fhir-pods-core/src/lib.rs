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
    // No declared profile we have compiled (and no default configured on the
    // validator): the spec says the base resource applies, and we compile no
    // base resources — so there is nothing to check and nothing to claim.
    let profiles = select_profiles(rt, &doc, cache.map(|v| v.defaults.as_slice()).unwrap_or(&[]));
    if profiles.is_empty() {
        return Validation { resource_type: rt.into(), profile: None, profiles: Vec::new(), report: Report::default() };
    }
    let mut report = Report::default();
    for profile in &profiles {
        let rep = engine::validate_at(profile, &doc, &doc, tx, 0, cache);
        for issue in rep.issues {
            if !report.issues.contains(&issue) {
                report.issues.push(issue);
            }
        }
    }
    Validation {
        resource_type: rt.into(),
        profile: Some(profiles[0].name),
        profiles: profiles.iter().map(|p| p.name).collect(),
        report,
    }
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
    out
}

/// [`Validator::lower`] without a validator (the invariants are re-parsed).
pub fn lower_bytes(url: &str, inputs: &[u8]) -> Result<lower::Lowered, lower::LowerError> {
    let url = url.split('|').next().unwrap_or(url);
    let profile = generated::REGISTRY.iter().find(|p| p.url == url).ok_or(lower::LowerError::UnknownProfile)?;
    let parsed = json::parse(inputs).map_err(|_| lower::LowerError::InputsNotAnObject)?;
    lower::lower(profile, &parsed, None)
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
        for p in generated::REGISTRY {
            for inv in p.invariants {
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
        let url = url.split('|').next().unwrap_or(url);
        let profile = generated::REGISTRY.iter().find(|p| p.url == url).ok_or(lower::LowerError::UnknownProfile)?;
        let parsed = json::parse(inputs).map_err(|_| lower::LowerError::InputsNotAnObject)?;
        lower::lower(profile, &parsed, Some(self))
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

pub mod validate;

pub mod path;

pub mod temporal;

pub mod regex;

pub mod terminology;
