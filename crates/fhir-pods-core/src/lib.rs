//! # fhir-pods
//!
//! FHIR is an intermediate representation for health data with a constraint
//! system clamping it: resources are the IR; profiles, terminology bindings,
//! and FHIRPath invariants are the constraints; validation is the verifier pass.
//!
//! Every incumbent ships that verifier as a *server* (JVM, .NET, or a cloud
//! endpoint) because the tooling grew up on enterprise runtimes. But a verifier
//! is stateless, CPU-bound, pure compute: the textbook shape of a **library**,
//! not a server. `fhir-pods-core` is that library — no I/O, no telemetry, no
//! licensed terminology, source-agnostic.
//!
//! Profiles are **generated from the packages' shipped `snapshot.element`** into
//! the compiled IR (`compiled`), then run by a generic `engine`. See
//! `docs/incumbent-apis.md` and `tools/gen_profile.py`.
#![forbid(unsafe_code)]

use validate::{Issue, Severity};

/// Compiled profiles generated from FHIR packages (`tools/gen_profile.py`).
pub mod generated {
    pub mod au_core_patient;
}

/// Probe a FHIR resource and return a JSON report. Single pass over the input:
/// [`json::scan_top_level`] captures the top-level shape (borrowing the buffer,
/// skipping nested structures — no DOM), then the matching profile validator
/// runs as straight-line checks.
pub fn probe(input: &[u8]) -> String {
    match json::scan_top_level(input) {
        Err(e) => {
            let issue = Issue {
                severity: Severity::Error,
                path: "(document)".into(),
                message: format!("could not parse JSON at byte {}: {}", e.at, e.msg),
            };
            render_report("Unknown", None, &[issue], false)
        }
        Ok(fields) => match fields.scalar("resourceType") {
            None if !fields.has("resourceType") => {
                let issue = Issue {
                    severity: Severity::Error,
                    path: "resourceType".into(),
                    message: "resourceType is required".into(),
                };
                render_report("Unknown", None, &[issue], false)
            }
            // resourceType present but non-string — treated as unknown.
            None => render_report("Unknown", None, &[], true),
            Some("Observation") => {
                let report = au_core::validate_observation(&fields);
                render_report("Observation", Some("AU Core Observation"), &report.issues, report.is_valid())
            }
            Some("Patient") => {
                let profile = &generated::au_core_patient::PROFILE;
                let report = engine::validate(profile, &fields);
                render_report("Patient", Some(profile.name), &report.issues, report.is_valid())
            }
            // No profile bound for this resource type yet.
            Some(rt) => render_report(rt, None, &[], true),
        },
    }
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Information => "information",
    }
}

fn render_report(resource_type: &str, profile: Option<&str>, issues: &[Issue], valid: bool) -> String {
    let mut out = String::with_capacity(128);
    out.push_str("{\"resourceType\":\"");
    push_json_escaped(&mut out, resource_type);
    out.push('"');
    if let Some(p) = profile {
        out.push_str(",\"profile\":\"");
        push_json_escaped(&mut out, p);
        out.push('"');
    }
    out.push_str(",\"valid\":");
    out.push_str(if valid { "true" } else { "false" });
    out.push_str(",\"issues\":[");
    for (i, issue) in issues.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"severity\":\"");
        out.push_str(severity_str(issue.severity));
        out.push_str("\",\"path\":\"");
        push_json_escaped(&mut out, &issue.path);
        out.push_str("\",\"message\":\"");
        push_json_escaped(&mut out, &issue.message);
        out.push_str("\"}");
    }
    out.push_str("]}");
    out
}

/// Append `s` to `out`, escaping the characters JSON requires.
fn push_json_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
}

/// A minimal, single-pass JSON scanner — enough to walk a resource's top-level
/// members without materializing a DOM. It records every top-level key and the
/// scalar (string) values, and skips the rest (nested objects/arrays/strings are
/// skipped, so a nested `code` never satisfies a top-level `code`). Strings are
/// borrowed from the input, not copied.
pub mod json {
    #[derive(Debug)]
    pub struct ParseError {
        pub at: usize,
        pub msg: &'static str,
    }

    /// The top-level shape captured in one pass; strings borrow the input.
    #[derive(Default, Debug)]
    pub struct TopLevel<'a> {
        /// Every top-level member key, in document order.
        pub present: Vec<&'a str>,
        /// Scalar (string) member values, by key.
        pub scalars: Vec<(&'a str, &'a str)>,
    }

    impl<'a> TopLevel<'a> {
        /// Is there a top-level member with this key?
        pub fn has(&self, key: &str) -> bool {
            self.present.iter().any(|k| *k == key)
        }

        /// The scalar string value of a top-level member, if it is a string.
        pub fn scalar(&self, key: &str) -> Option<&'a str> {
            self.scalars.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
        }

        /// True if any top-level member is a `value[x]` choice (`valueQuantity`…).
        pub fn has_value_x(&self) -> bool {
            self.present.iter().any(|k| k.len() > 5 && k.starts_with("value"))
        }
    }

    struct Parser<'a> {
        b: &'a [u8],
        i: usize,
    }

    impl<'a> Parser<'a> {
        fn byte(&self) -> Option<u8> {
            self.b.get(self.i).copied()
        }

        fn skip_ws(&mut self) {
            while matches!(self.byte(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.i += 1;
            }
        }

        fn expect(&mut self, c: u8) -> Result<(), ParseError> {
            if self.byte() == Some(c) {
                self.i += 1;
                Ok(())
            } else {
                Err(ParseError { at: self.i, msg: "unexpected byte" })
            }
        }

        /// Parse a JSON string, returning the raw inner slice (borrowed). Tracks
        /// escapes only enough to find the true closing quote.
        fn string_raw(&mut self) -> Result<&'a str, ParseError> {
            self.expect(b'"')?;
            let b = self.b;
            let start = self.i;
            loop {
                match b.get(self.i).copied() {
                    None => return Err(ParseError { at: self.i, msg: "unterminated string" }),
                    Some(b'\\') => self.i += 2,
                    Some(b'"') => {
                        let s = core::str::from_utf8(&b[start..self.i])
                            .map_err(|_| ParseError { at: start, msg: "invalid utf-8" })?;
                        self.i += 1;
                        return Ok(s);
                    }
                    Some(_) => self.i += 1,
                }
            }
        }

        fn skip_value(&mut self) -> Result<(), ParseError> {
            self.skip_ws();
            match self.byte() {
                Some(b'"') => {
                    self.string_raw()?;
                    Ok(())
                }
                Some(b'{') | Some(b'[') => self.skip_balanced(),
                Some(_) => {
                    self.skip_scalar();
                    Ok(())
                }
                None => Err(ParseError { at: self.i, msg: "unexpected end" }),
            }
        }

        fn skip_scalar(&mut self) {
            while let Some(c) = self.byte() {
                if matches!(c, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                    break;
                }
                self.i += 1;
            }
        }

        fn skip_balanced(&mut self) -> Result<(), ParseError> {
            let mut depth = 0i32;
            loop {
                match self.byte() {
                    None => return Err(ParseError { at: self.i, msg: "unterminated structure" }),
                    Some(b'"') => {
                        self.string_raw()?;
                    }
                    Some(b'{') | Some(b'[') => {
                        depth += 1;
                        self.i += 1;
                    }
                    Some(b'}') | Some(b']') => {
                        depth -= 1;
                        self.i += 1;
                        if depth == 0 {
                            return Ok(());
                        }
                    }
                    Some(_) => self.i += 1,
                }
            }
        }
    }

    /// Scan a resource's top-level object in a single pass.
    pub fn scan_top_level(input: &[u8]) -> Result<TopLevel<'_>, ParseError> {
        let mut p = Parser { b: input, i: 0 };
        p.skip_ws();
        p.expect(b'{')?;
        let mut out = TopLevel::default();
        p.skip_ws();
        if p.byte() == Some(b'}') {
            return Ok(out);
        }
        loop {
            p.skip_ws();
            let key = p.string_raw()?;
            p.skip_ws();
            p.expect(b':')?;
            out.present.push(key);
            p.skip_ws();
            if p.byte() == Some(b'"') {
                let v = p.string_raw()?;
                out.scalars.push((key, v));
            } else {
                p.skip_value()?;
            }
            p.skip_ws();
            match p.byte() {
                Some(b',') => {
                    p.i += 1;
                    continue;
                }
                Some(b'}') => break,
                _ => return Err(ParseError { at: p.i, msg: "expected ',' or '}'" }),
            }
        }
        Ok(out)
    }
}

/// The compiled profile IR — what `tools/gen_profile.py` emits from a package's
/// `snapshot.element`. Our own representation (FHIRSchema was rejected as a
/// dependency — see the decision doc); tree-shaped, arrays-first ideas borrowed
/// as prior art only.
pub mod compiled {
    /// One enforced constraint. Today: top-level cardinality + required binding.
    pub struct ElementRule {
        /// Dotted path, e.g. `Patient.gender`.
        pub path: &'static str,
        /// Minimum cardinality (>=1 means mandatory).
        pub min: u32,
        /// A required-strength ValueSet binding on a top-level code, if any.
        pub required_binding: Option<&'static str>,
    }

    /// A profile compiled to embeddable straight-line rules.
    pub struct CompiledProfile {
        pub url: &'static str,
        pub name: &'static str,
        pub resource_type: &'static str,
        pub rules: &'static [ElementRule],
        /// Count of constraints not yet enforced (nested / slicing / fixed / MS).
        pub deferred: u32,
    }
}

/// The generic validation engine: run a [`compiled::CompiledProfile`] against a
/// scanned resource. Enforces the top-level rules the generator emitted;
/// deeper rules are the profile's `deferred` count until the nested engine lands.
pub mod engine {
    use crate::compiled::CompiledProfile;
    use crate::json::TopLevel;
    use crate::validate::{Issue, Report, Severity};

    pub fn validate(profile: &CompiledProfile, top: &TopLevel) -> Report {
        let mut r = Report::default();
        for rule in profile.rules {
            let field = rule.path.rsplit('.').next().unwrap_or(rule.path);
            if rule.min >= 1 && !top.has(field) {
                r.issues.push(Issue {
                    severity: Severity::Error,
                    path: rule.path.into(),
                    message: format!("{} is required (min {})", rule.path, rule.min),
                });
                continue;
            }
            if let Some(vs) = rule.required_binding {
                if let Some(code) = top.scalar(field) {
                    match crate::value_sets::in_value_set(vs, code) {
                        Some(false) => r.issues.push(Issue {
                            severity: Severity::Error,
                            path: rule.path.into(),
                            message: format!("'{code}' is not in the bound value set"),
                        }),
                        None => r.issues.push(Issue {
                            severity: Severity::Warning,
                            path: rule.path.into(),
                            message: format!("no bundled terminology for {vs}; '{code}' not checked"),
                        }),
                        Some(true) => {}
                    }
                }
            }
        }
        r
    }
}

/// Offline membership for the small base-FHIR value sets we can legally bundle.
/// Returns `None` for a set we don't ship — the engine degrades that to a
/// warning (the reference validator's no-terminology behavior). SNOMED-bound
/// sets are intentionally absent: they resolve against a licensed seam at runtime.
pub mod value_sets {
    pub fn in_value_set(url: &str, code: &str) -> Option<bool> {
        let base = url.split('|').next().unwrap_or(url);
        match base {
            "http://hl7.org/fhir/ValueSet/administrative-gender" => {
                Some(matches!(code, "male" | "female" | "other" | "unknown"))
            }
            "http://hl7.org/fhir/ValueSet/observation-status" => Some(matches!(
                code,
                "registered" | "preliminary" | "final" | "amended" | "corrected"
                    | "cancelled" | "entered-in-error" | "unknown"
            )),
            _ => None,
        }
    }
}

/// Hand-written AU Core Observation checks (the reference the Observation-family
/// codegen will reproduce and exceed). Uses the same scanner as the engine.
pub mod au_core {
    use crate::json::TopLevel;
    use crate::validate::{Issue, Report, Severity};

    pub fn is_observation_status(code: &str) -> bool {
        crate::value_sets::in_value_set("http://hl7.org/fhir/ValueSet/observation-status", code)
            .unwrap_or(false)
    }

    pub fn validate_observation(fields: &TopLevel) -> Report {
        let mut r = Report::default();
        let error = |path: &str, message: &str| Issue {
            severity: Severity::Error,
            path: path.into(),
            message: message.into(),
        };
        match fields.scalar("status") {
            None => r.issues.push(error("Observation.status", "status is required (1..1)")),
            Some(s) if !is_observation_status(s) => r.issues.push(Issue {
                severity: Severity::Error,
                path: "Observation.status".into(),
                message: format!("status '{s}' is not in the observation-status value set"),
            }),
            Some(_) => {}
        }
        if !fields.has("code") {
            r.issues.push(error("Observation.code", "code is required (1..1)"));
        }
        if !fields.has("subject") {
            r.issues.push(error("Observation.subject", "subject is required (1..1)"));
        }
        if !fields.has_value_x() && !fields.has("dataAbsentReason") {
            r.issues.push(Issue {
                severity: Severity::Warning,
                path: "Observation".into(),
                message: "neither value[x] nor dataAbsentReason is present".into(),
            });
        }
        r
    }
}

/// Minimal resource model — carried as raw JSON until the generated typed model.
pub mod model {
    #[derive(Debug, Clone)]
    pub struct Resource {
        pub resource_type: String,
        pub raw: String,
    }

    impl Resource {
        pub fn new(resource_type: impl Into<String>, raw: impl Into<String>) -> Self {
            Self { resource_type: resource_type.into(), raw: raw.into() }
        }
    }
}

/// The verifier types.
pub mod validate {
    use crate::model::Resource;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Severity {
        Error,
        Warning,
        Information,
    }

    #[derive(Debug, Clone)]
    pub struct Issue {
        pub severity: Severity,
        pub path: String,
        pub message: String,
    }

    #[derive(Debug, Default)]
    pub struct Report {
        pub issues: Vec<Issue>,
    }

    impl Report {
        pub fn is_valid(&self) -> bool {
            !self.issues.iter().any(|i| i.severity == Severity::Error)
        }
    }

    #[derive(Debug, Clone)]
    pub struct Profile {
        pub url: String,
    }

    pub trait Validator {
        fn validate(&self, resource: &Resource, profile: &Profile) -> Report;
    }
}

/// FHIRPath — the expression language FHIR invariants are written in.
pub mod path {
    #[derive(Debug, Default)]
    pub struct Engine;

    impl Engine {
        pub fn new() -> Self {
            Self
        }
    }
}

/// Terminology binding checks against a pluggable provider (ships no licensed content).
pub mod terminology {
    pub trait TerminologyProvider {
        fn validate_code(&self, value_set_url: &str, system: &str, code: &str) -> bool;
    }
}

/// Lowering passes: arbitrary source records → FHIR resources (source-agnostic).
pub mod lower {
    use crate::model::Resource;

    #[derive(Debug, Clone)]
    pub struct Mapping {
        pub target_type: String,
    }

    pub trait Lower<T> {
        fn lower(&self, source: &T, mapping: &Mapping) -> Resource;
    }
}

/// The audit surface — FHIR-native `AuditEvent` / `Provenance`. No I/O.
pub mod audit {
    use crate::model::Resource;

    #[derive(Debug, Clone, Copy)]
    pub enum Action {
        Create,
        Read,
        Update,
        Delete,
        Execute,
    }

    #[derive(Debug, Clone, Copy)]
    pub enum Outcome {
        Success,
        MinorFailure,
        SeriousFailure,
        MajorFailure,
    }

    #[derive(Debug, Clone)]
    pub struct AuditRecord {
        pub actor: String,
        pub action: Action,
        pub subject_ref: String,
        pub outcome: Outcome,
        pub recorded: String,
        pub trace_id: Option<String>,
    }

    impl AuditRecord {
        pub fn to_audit_event(&self) -> Resource {
            Resource::new("AuditEvent", "{}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::probe;

    fn probe_str(s: &str) -> String {
        probe(s.as_bytes())
    }

    // --- Observation (hand-written) ---

    #[test]
    fn valid_observation() {
        let r = probe_str(
            r#"{"resourceType":"Observation","status":"final","code":{"text":"HR"},"subject":{"reference":"Patient/1"},"valueQuantity":{"value":60}}"#,
        );
        assert!(r.contains("\"valid\":true"), "{r}");
        assert!(r.contains("AU Core Observation"), "{r}");
    }

    #[test]
    fn missing_status_is_error() {
        let r = probe_str(r#"{"resourceType":"Observation","code":{},"subject":{},"valueString":"x"}"#);
        assert!(r.contains("status is required"), "{r}");
    }

    #[test]
    fn nested_code_does_not_satisfy_top_level() {
        let r = probe_str(
            r#"{"resourceType":"Observation","status":"final","subject":{"code":"nope","reference":"Patient/1"},"valueString":"x"}"#,
        );
        assert!(r.contains("code is required"), "{r}");
    }

    #[test]
    fn tricky_string_does_not_end_object_early() {
        let r = probe_str(
            r#"{"resourceType":"Observation","note":"a } \" tricky","status":"final","code":{},"subject":{},"valueString":"x"}"#,
        );
        assert!(r.contains("\"valid\":true"), "{r}");
    }

    // --- Patient (generated: au-core-patient) ---

    #[test]
    fn valid_patient() {
        let r = probe_str(
            r#"{"resourceType":"Patient","identifier":[{"value":"1"}],"name":[{"family":"Doe"}],"gender":"male","birthDate":"1970-01-01"}"#,
        );
        assert!(r.contains("\"valid\":true"), "{r}");
        assert!(r.contains("AU Core Patient"), "{r}");
    }

    #[test]
    fn patient_missing_mandatory_is_error() {
        // no identifier, name, gender, birthDate
        let r = probe_str(r#"{"resourceType":"Patient","active":true}"#);
        assert!(r.contains("\"valid\":false"), "{r}");
        assert!(r.contains("Patient.identifier is required"), "{r}");
        assert!(r.contains("Patient.name is required"), "{r}");
        assert!(r.contains("Patient.gender is required"), "{r}");
        assert!(r.contains("Patient.birthDate is required"), "{r}");
    }

    #[test]
    fn patient_bad_gender_fails_required_binding() {
        let r = probe_str(
            r#"{"resourceType":"Patient","identifier":[{"value":"1"}],"name":[{"family":"Doe"}],"gender":"banana","birthDate":"1970-01-01"}"#,
        );
        assert!(r.contains("not in the bound value set"), "{r}");
        assert!(r.contains("\"valid\":false"), "{r}");
    }

    // --- shared ---

    #[test]
    fn non_profiled_resource_has_no_profile() {
        let r = probe_str(r#"{"resourceType":"Encounter","status":"finished"}"#);
        assert!(r.contains("\"resourceType\":\"Encounter\""), "{r}");
        assert!(r.contains("\"valid\":true"), "{r}");
    }

    #[test]
    fn garbage_reports_a_parse_error() {
        let r = probe_str("not json");
        assert!(r.contains("could not parse"), "{r}");
        assert!(r.contains("\"valid\":false"), "{r}");
    }
}
