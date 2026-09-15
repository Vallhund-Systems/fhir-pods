//! # fhir-pods
//!
//! FHIR is an intermediate representation for health data with a constraint
//! system clamping it: resources are the IR; profiles, terminology bindings,
//! and FHIRPath invariants are the constraints; validation is the verifier pass.
//!
//! Every incumbent ships that verifier as a *server*. A verifier is stateless,
//! CPU-bound, pure compute — a **library**. `fhir-pods-core` is that library:
//! no I/O, no telemetry, no licensed terminology, source-agnostic.
//!
//! Profiles are **generated from the packages' shipped `snapshot.element`** into
//! the compiled IR (`compiled`), then run by the generic `engine` against a
//! `document::Document` port. See `docs/domain-model.md`.
#![forbid(unsafe_code)]

use validate::{Issue, Severity};

/// Compiled profiles generated from FHIR packages (`tools/gen_profile.py`).
pub mod generated {
    pub mod au_core_patient;
}

/// Probe a FHIR resource and return a JSON report.
pub fn probe(input: &[u8]) -> String {
    let fields = match json::scan_top_level(input) {
        Ok(f) => f,
        Err(e) => return parse_error_report(e),
    };
    match fields.scalar("resourceType") {
        None if !fields.has("resourceType") => {
            let issue = Issue {
                severity: Severity::Error,
                path: "resourceType".into(),
                message: "resourceType is required".into(),
            };
            render_report("Unknown", None, &[issue], false)
        }
        None => render_report("Unknown", None, &[], true),
        Some("Observation") => {
            let report = au_core::validate_observation(&fields);
            render_report("Observation", Some("AU Core Observation"), &report.issues, report.is_valid())
        }
        Some("Patient") => match document::TreeDoc::parse(input) {
            Ok(doc) => {
                let profile = &generated::au_core_patient::PROFILE;
                let report = engine::validate(profile, &doc);
                render_report("Patient", Some(profile.name), &report.issues, report.is_valid())
            }
            Err(e) => parse_error_report(e),
        },
        Some(rt) => render_report(rt, None, &[], true),
    }
}

fn parse_error_report(e: json::ParseError) -> String {
    let issue = Issue {
        severity: Severity::Error,
        path: "(document)".into(),
        message: format!("could not parse JSON at byte {}: {}", e.at, e.msg),
    };
    render_report("Unknown", None, &[issue], false)
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

/// JSON, two ways. `scan_top_level` is the zero-copy single pass over a
/// resource's top-level members (no DOM). `parse` builds a borrowed value tree —
/// the correctness-first adapter behind the `Document` port; a fused
/// validate-on-parse adapter replaces it later without touching the domain.
pub mod json {
    #[derive(Debug)]
    pub struct ParseError {
        pub at: usize,
        pub msg: &'static str,
    }

    /// The top-level shape captured in one pass; strings borrow the input.
    #[derive(Default, Debug)]
    pub struct TopLevel<'a> {
        pub present: Vec<&'a str>,
        pub scalars: Vec<(&'a str, &'a str)>,
    }

    impl<'a> TopLevel<'a> {
        pub fn has(&self, key: &str) -> bool {
            self.present.iter().any(|k| *k == key)
        }
        pub fn scalar(&self, key: &str) -> Option<&'a str> {
            self.scalars.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
        }
        pub fn has_value_x(&self) -> bool {
            self.present.iter().any(|k| k.len() > 5 && k.starts_with("value"))
        }
    }

    /// A borrowed JSON value tree. Strings and numbers point into the input.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value<'a> {
        Null,
        Bool(bool),
        Number(&'a str),
        Str(&'a str),
        Array(Vec<Value<'a>>),
        Object(Vec<(&'a str, Value<'a>)>),
    }

    /// Parse a whole document into a value tree.
    pub fn parse(input: &[u8]) -> Result<Value<'_>, ParseError> {
        let mut p = Parser { b: input, i: 0 };
        p.parse_value()
    }

    /// Resolve a dotted element path against a resource. The first segment is
    /// the resource type (skipped). Arrays expand; `[x]` matches a choice suffix
    /// (`value[x]` → `valueQuantity`).
    pub fn resolve<'v, 'a>(root: &'v Value<'a>, path: &str) -> Vec<&'v Value<'a>> {
        let mut current: Vec<&'v Value<'a>> = vec![root];
        for seg in path.split('.').skip(1) {
            let mut next = Vec::new();
            for node in current {
                children(node, seg, &mut next);
            }
            current = next;
        }
        current
    }

    /// Push the children of `node` matching `seg` (expanding arrays).
    pub fn children<'v, 'a>(node: &'v Value<'a>, seg: &str, out: &mut Vec<&'v Value<'a>>) {
        match node {
            Value::Object(members) => {
                for (k, v) in members {
                    if key_matches(k, seg) {
                        match v {
                            Value::Array(items) => out.extend(items.iter()),
                            other => out.push(other),
                        }
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    children(item, seg, out);
                }
            }
            _ => {}
        }
    }

    fn key_matches(key: &str, seg: &str) -> bool {
        match seg.strip_suffix("[x]") {
            Some(prefix) => {
                key.len() > prefix.len()
                    && key.starts_with(prefix)
                    && key[prefix.len()..].chars().next().is_some_and(|c| c.is_ascii_uppercase())
            }
            None => key == seg,
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

        fn literal(&mut self, lit: &[u8]) -> Result<(), ParseError> {
            if self.b.get(self.i..).is_some_and(|s| s.starts_with(lit)) {
                self.i += lit.len();
                Ok(())
            } else {
                Err(ParseError { at: self.i, msg: "bad literal" })
            }
        }

        fn parse_number(&mut self) -> Result<Value<'a>, ParseError> {
            let b = self.b;
            let start = self.i;
            while let Some(c) = self.byte() {
                if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E') {
                    self.i += 1;
                } else {
                    break;
                }
            }
            if self.i == start {
                return Err(ParseError { at: start, msg: "expected value" });
            }
            let s = core::str::from_utf8(&b[start..self.i])
                .map_err(|_| ParseError { at: start, msg: "invalid utf-8" })?;
            Ok(Value::Number(s))
        }

        fn parse_value(&mut self) -> Result<Value<'a>, ParseError> {
            self.skip_ws();
            match self.byte() {
                Some(b'{') => self.parse_object(),
                Some(b'[') => self.parse_array(),
                Some(b'"') => Ok(Value::Str(self.string_raw()?)),
                Some(b't') => {
                    self.literal(b"true")?;
                    Ok(Value::Bool(true))
                }
                Some(b'f') => {
                    self.literal(b"false")?;
                    Ok(Value::Bool(false))
                }
                Some(b'n') => {
                    self.literal(b"null")?;
                    Ok(Value::Null)
                }
                Some(_) => self.parse_number(),
                None => Err(ParseError { at: self.i, msg: "unexpected end" }),
            }
        }

        fn parse_object(&mut self) -> Result<Value<'a>, ParseError> {
            self.expect(b'{')?;
            let mut members = Vec::new();
            self.skip_ws();
            if self.byte() == Some(b'}') {
                self.i += 1;
                return Ok(Value::Object(members));
            }
            loop {
                self.skip_ws();
                let key = self.string_raw()?;
                self.skip_ws();
                self.expect(b':')?;
                let val = self.parse_value()?;
                members.push((key, val));
                self.skip_ws();
                match self.byte() {
                    Some(b',') => self.i += 1,
                    Some(b'}') => {
                        self.i += 1;
                        return Ok(Value::Object(members));
                    }
                    _ => return Err(ParseError { at: self.i, msg: "expected ',' or '}'" }),
                }
            }
        }

        fn parse_array(&mut self) -> Result<Value<'a>, ParseError> {
            self.expect(b'[')?;
            let mut items = Vec::new();
            self.skip_ws();
            if self.byte() == Some(b']') {
                self.i += 1;
                return Ok(Value::Array(items));
            }
            loop {
                let val = self.parse_value()?;
                items.push(val);
                self.skip_ws();
                match self.byte() {
                    Some(b',') => self.i += 1,
                    Some(b']') => {
                        self.i += 1;
                        return Ok(Value::Array(items));
                    }
                    _ => return Err(ParseError { at: self.i, msg: "expected ',' or ']'" }),
                }
            }
        }

        fn skip_value(&mut self) -> Result<(), ParseError> {
            self.parse_value().map(|_| ())
        }
    }

    /// Scan a resource's top-level object in a single pass (no DOM).
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

/// The `Document` port: how the engine reads a resource, independent of the
/// JSON representation. `TreeDoc` is today's adapter (a parsed value tree); a
/// fused single-pass adapter is the later performance move — same port.
pub mod document {
    use crate::json::{self, Value};

    pub trait Document {
        /// For each instance of the element's parent, how many children exist
        /// at the last path segment. Empty when no parent exists — a child's
        /// cardinality is only evaluated when its parent is present.
        fn child_counts(&self, path: &str) -> Vec<usize>;
        /// All scalar string values at `path` (across every parent).
        fn scalars(&self, path: &str) -> Vec<&str>;
    }

    /// The value-tree adapter.
    pub struct TreeDoc<'a> {
        root: Value<'a>,
    }

    impl<'a> TreeDoc<'a> {
        pub fn parse(input: &'a [u8]) -> Result<Self, json::ParseError> {
            Ok(Self { root: json::parse(input)? })
        }
    }

    impl Document for TreeDoc<'_> {
        fn child_counts(&self, path: &str) -> Vec<usize> {
            let Some((parent, last)) = path.rsplit_once('.') else {
                return Vec::new();
            };
            json::resolve(&self.root, parent)
                .into_iter()
                .map(|p| {
                    let mut out = Vec::new();
                    json::children(p, last, &mut out);
                    out.len()
                })
                .collect()
        }

        fn scalars(&self, path: &str) -> Vec<&str> {
            json::resolve(&self.root, path)
                .into_iter()
                .filter_map(|v| match v {
                    Value::Str(s) => Some(*s),
                    _ => None,
                })
                .collect()
        }
    }
}

/// The compiled profile IR — what `tools/gen_profile.py` emits from a package's
/// `snapshot.element`. Our own representation; immutable, embedded, `'static`.
pub mod compiled {
    /// Upper cardinality bound.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Max {
        Star,
        Bounded(u32),
    }

    /// One element's enforceable constraints. Cardinality is per parent instance.
    pub struct ElementConstraint {
        /// Dotted path, e.g. `Patient.name.use`.
        pub path: &'static str,
        pub min: u32,
        pub max: Max,
        /// A scalar the element must equal exactly (fixedUri / fixedCode / …).
        /// For a SNOMED coding this is only ever `system`/`code`, never `display`.
        pub fixed: Option<&'static str>,
        /// A required-strength ValueSet binding on a code element.
        pub required_binding: Option<&'static str>,
    }

    /// A profile compiled to embeddable rules.
    pub struct CompiledProfile {
        pub url: &'static str,
        pub name: &'static str,
        pub resource_type: &'static str,
        pub elements: &'static [ElementConstraint],
        /// Constraints not yet enforced (slices / pattern / mustSupport-only).
        pub deferred: u32,
    }
}

/// The validation engine — the domain service. Pure: a compiled profile and a
/// document in, a report out.
pub mod engine {
    use crate::compiled::{CompiledProfile, Max};
    use crate::document::Document;
    use crate::validate::{Issue, Report, Severity};

    pub fn validate(profile: &CompiledProfile, doc: &impl Document) -> Report {
        let mut r = Report::default();
        let error = |path: &str, message: String| Issue { severity: Severity::Error, path: path.into(), message };

        for el in profile.elements {
            // Cardinality, evaluated per parent instance (FHIR semantics: a
            // child's min applies only where its parent exists).
            for count in doc.child_counts(el.path) {
                let count = count as u32;
                if count < el.min {
                    r.issues.push(error(el.path, format!("{} is required (min {}, found {})", el.path, el.min, count)));
                }
                if let Max::Bounded(m) = el.max {
                    if count > m {
                        r.issues.push(error(el.path, format!("{} exceeds max {} (found {})", el.path, m, count)));
                    }
                }
            }

            if el.fixed.is_none() && el.required_binding.is_none() {
                continue;
            }
            for s in doc.scalars(el.path) {
                if let Some(f) = el.fixed {
                    if s != f {
                        r.issues.push(error(el.path, format!("{} must be '{}' (found '{}')", el.path, f, s)));
                    }
                }
                if let Some(vs) = el.required_binding {
                    match crate::value_sets::in_value_set(vs, s) {
                        Some(false) => r.issues.push(error(el.path, format!("'{s}' is not in the bound value set"))),
                        None => r.issues.push(Issue {
                            severity: Severity::Warning,
                            path: el.path.into(),
                            message: format!("no bundled terminology for {vs}; '{s}' not checked"),
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
/// `None` for a set we don't ship — the engine degrades that to a warning.
/// SNOMED-bound sets are intentionally absent: they resolve at runtime against
/// a licensed seam.
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
            "http://hl7.org/fhir/ValueSet/name-use" => Some(matches!(
                code,
                "usual" | "official" | "temp" | "nickname" | "anonymous" | "old" | "maiden"
            )),
            "http://hl7.org/fhir/ValueSet/link-type" => {
                Some(matches!(code, "replaced-by" | "replaces" | "refer" | "seealso"))
            }
            _ => None,
        }
    }
}

/// Hand-written AU Core Observation checks over the single-pass scanner — the
/// reference the Observation-family codegen (post-slicing) will reproduce.
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

pub mod path {
    #[derive(Debug, Default)]
    pub struct Engine;

    impl Engine {
        pub fn new() -> Self {
            Self
        }
    }
}

pub mod terminology {
    pub trait TerminologyProvider {
        fn validate_code(&self, value_set_url: &str, system: &str, code: &str) -> bool;
    }
}

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

    const PATIENT_OK: &str = r#"{"resourceType":"Patient","identifier":[{"value":"1"}],"name":[{"family":"Doe","use":"official"}],"gender":"male","birthDate":"1970-01-01"}"#;

    // --- Observation (hand-written, single-pass scanner) ---

    #[test]
    fn valid_observation() {
        let r = probe_str(
            r#"{"resourceType":"Observation","status":"final","code":{"text":"HR"},"subject":{"reference":"Patient/1"},"valueQuantity":{"value":60}}"#,
        );
        assert!(r.contains("\"valid\":true"), "{r}");
    }

    #[test]
    fn nested_code_does_not_satisfy_top_level() {
        let r = probe_str(
            r#"{"resourceType":"Observation","status":"final","subject":{"code":"nope"},"valueString":"x"}"#,
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

    // --- Patient (generated au-core-patient, nested engine) ---

    #[test]
    fn valid_patient() {
        let r = probe_str(PATIENT_OK);
        assert!(r.contains("\"valid\":true"), "{r}");
        assert!(r.contains("AU Core Patient"), "{r}");
    }

    #[test]
    fn patient_missing_mandatory_is_error() {
        let r = probe_str(r#"{"resourceType":"Patient","active":true}"#);
        assert!(r.contains("Patient.identifier is required"), "{r}");
        assert!(r.contains("Patient.name is required"), "{r}");
        assert!(r.contains("Patient.gender is required"), "{r}");
        assert!(r.contains("Patient.birthDate is required"), "{r}");
    }

    #[test]
    fn patient_bad_gender_fails_required_binding() {
        let r = probe_str(&PATIENT_OK.replace("\"male\"", "\"banana\""));
        assert!(r.contains("not in the bound value set"), "{r}");
    }

    #[test]
    fn nested_binding_on_name_use() {
        let r = probe_str(&PATIENT_OK.replace("\"official\"", "\"bogus\""));
        assert!(r.contains("Patient.name.use"), "{r}");
        assert!(r.contains("not in the bound value set"), "{r}");
    }

    #[test]
    fn nested_min_fires_per_parent_instance() {
        // Two communications: one has a language, one does not → exactly one error.
        let r = probe_str(&PATIENT_OK.replace(
            "\"birthDate\"",
            "\"communication\":[{\"language\":{\"text\":\"en\"}},{\"preferred\":true}],\"birthDate\"",
        ));
        assert_eq!(r.matches("Patient.communication.language is required").count(), 1, "{r}");
    }

    #[test]
    fn nested_min_does_not_fire_when_parent_absent() {
        // No communication at all → the child's min is not evaluated.
        let r = probe_str(PATIENT_OK);
        assert!(!r.contains("communication.language"), "{r}");
    }

    #[test]
    fn nested_max_exceeded_per_parent() {
        // language is 0..1 per communication; two → exceeds max.
        let r = probe_str(&PATIENT_OK.replace(
            "\"birthDate\"",
            "\"communication\":[{\"language\":[{\"text\":\"en\"},{\"text\":\"fr\"}]}],\"birthDate\"",
        ));
        assert!(r.contains("Patient.communication.language exceeds max 1"), "{r}");
    }

    #[test]
    fn link_type_binding_nested() {
        let bad = PATIENT_OK.replace(
            "\"birthDate\"",
            "\"link\":[{\"other\":{\"reference\":\"Patient/2\"},\"type\":\"nope\"}],\"birthDate\"",
        );
        let r = probe_str(&bad);
        assert!(r.contains("Patient.link.type"), "{r}");
        let ok = PATIENT_OK.replace(
            "\"birthDate\"",
            "\"link\":[{\"other\":{\"reference\":\"Patient/2\"},\"type\":\"replaces\"}],\"birthDate\"",
        );
        assert!(probe_str(&ok).contains("\"valid\":true"), "{}", probe_str(&ok));
    }

    // --- shared ---

    #[test]
    fn non_profiled_resource_has_no_profile() {
        let r = probe_str(r#"{"resourceType":"Encounter","status":"finished"}"#);
        assert!(r.contains("\"valid\":true"), "{r}");
    }

    #[test]
    fn garbage_reports_a_parse_error() {
        let r = probe_str("not json");
        assert!(r.contains("could not parse"), "{r}");
    }
}
