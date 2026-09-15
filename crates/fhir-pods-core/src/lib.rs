//! # fhir-pods
//!
//! FHIR is an intermediate representation for health data with a constraint
//! system clamping it: resources are the IR; profiles, terminology bindings,
//! and FHIRPath invariants are the constraints; validation is the verifier pass.
//!
//! Every incumbent ships that verifier as a *server* (JVM, .NET, or a cloud
//! endpoint) because the tooling grew up on enterprise runtimes — a
//! multi-hundred-MB floor, cold starts, no way to run inside a TS app, a
//! browser, or a phone. But a verifier is stateless, CPU-bound, pure compute:
//! the textbook shape of a **library**, not a server.
//!
//! `fhir-pods-core` is that library. It performs **no I/O**, speaks to **no
//! telemetry backend**, and bundles **no licensed terminology**. It is
//! source-agnostic: it lowers *arbitrary* input to FHIR; domain-specific maps
//! live in the host application, never here.
//!
//! See `docs/architecture.md` for the design, the conformance target (AU Core),
//! and why the conformance rules are generated from the spec at compile time
//! rather than interpreted at runtime.
#![forbid(unsafe_code)]

use validate::{Issue, Severity};

/// Probe a FHIR resource and return a JSON report. Single pass over the input:
/// [`json::scan_top_level`] captures the profile-relevant fields (borrowing the
/// buffer, skipping nested structures — no DOM), then the matching profile
/// validator runs as straight-line checks. This is the "almost free" path — the
/// only allocation is the report string.
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
        Ok(fields) => match fields.resource_type {
            None => {
                let issue = Issue {
                    severity: Severity::Error,
                    path: "resourceType".into(),
                    message: "resourceType is required".into(),
                };
                render_report("Unknown", None, &[issue], false)
            }
            Some("Observation") => {
                let report = au_core::validate_observation(&fields);
                render_report(
                    "Observation",
                    Some("AU Core Observation"),
                    &report.issues,
                    report.is_valid(),
                )
            }
            // No profile bound for this resource type yet — nothing to assert.
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

/// A minimal, single-pass JSON scanner — just enough to walk a resource's
/// top-level members without materializing a DOM. It captures the fields a
/// profile cares about and skips the rest (nested objects/arrays/strings are
/// skipped, so a nested `code` never satisfies a top-level `code`). Strings are
/// borrowed from the input, not copied.
///
/// Deliberately not a general JSON library: it scans the well-formed JSON FHIR
/// emits. Exotic escape decoding is deferred (codes carry no escapes); string
/// *boundaries* are always tracked correctly so skipping is sound.
pub mod json {
    /// A parse failure, located by byte offset.
    #[derive(Debug)]
    pub struct ParseError {
        pub at: usize,
        pub msg: &'static str,
    }

    /// Top-level fields captured in one pass. Strings borrow the input.
    #[derive(Default, Debug)]
    pub struct TopLevel<'a> {
        pub resource_type: Option<&'a str>,
        pub status: Option<&'a str>,
        pub has_code: bool,
        pub has_subject: bool,
        pub has_value_x: bool,
        pub has_data_absent_reason: bool,
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
                    Some(b'\\') => self.i += 2, // skip the escape and its escapee
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

        /// A string value if the next token is a string, else skip and return None.
        fn opt_string(&mut self) -> Result<Option<&'a str>, ParseError> {
            self.skip_ws();
            if self.byte() == Some(b'"') {
                Ok(Some(self.string_raw()?))
            } else {
                self.skip_value()?;
                Ok(None)
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

        /// Skip a balanced object/array, honoring strings so structural bytes
        /// inside a string don't change depth.
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
            match key {
                "resourceType" => out.resource_type = p.opt_string()?,
                "status" => out.status = p.opt_string()?,
                "code" => {
                    out.has_code = true;
                    p.skip_value()?;
                }
                "subject" => {
                    out.has_subject = true;
                    p.skip_value()?;
                }
                "dataAbsentReason" => {
                    out.has_data_absent_reason = true;
                    p.skip_value()?;
                }
                k if k.len() > 5 && k.starts_with("value") => {
                    out.has_value_x = true;
                    p.skip_value()?;
                }
                _ => p.skip_value()?,
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

/// AU Core profiles — hand-written first slices that the codegen will later
/// generate from the published StructureDefinitions. Starting with Observation.
pub mod au_core {
    use crate::json::TopLevel;
    use crate::validate::{Issue, Report, Severity};

    /// The `observation-status` value set, compiled to an O(1) match — the
    /// miniature of build-time ValueSet compilation (never an interpreted scan).
    pub fn is_observation_status(code: &str) -> bool {
        matches!(
            code,
            "registered"
                | "preliminary"
                | "final"
                | "amended"
                | "corrected"
                | "cancelled"
                | "entered-in-error"
                | "unknown"
        )
    }

    /// Validate the top-level shape of an Observation against a first slice of
    /// AU Core Observation: `status` (1..1, bound), `code` (1..1), `subject`
    /// (1..1), and value[x]-or-dataAbsentReason.
    pub fn validate_observation(fields: &TopLevel) -> Report {
        let mut r = Report::default();
        let error = |path: &str, message: &str| Issue {
            severity: Severity::Error,
            path: path.into(),
            message: message.into(),
        };

        match fields.status {
            None => r.issues.push(error("Observation.status", "status is required (1..1)")),
            Some(s) if !is_observation_status(s) => r.issues.push(Issue {
                severity: Severity::Error,
                path: "Observation.status".into(),
                message: format!("status '{s}' is not in the observation-status value set"),
            }),
            Some(_) => {}
        }
        if !fields.has_code {
            r.issues.push(error("Observation.code", "code is required (1..1)"));
        }
        if !fields.has_subject {
            r.issues.push(error("Observation.subject", "subject is required (1..1)"));
        }
        if !fields.has_value_x && !fields.has_data_absent_reason {
            r.issues.push(Issue {
                severity: Severity::Warning,
                path: "Observation".into(),
                message: "neither value[x] nor dataAbsentReason is present".into(),
            });
        }
        r
    }
}

/// Minimal resource model. Commit-zero placeholder: a resource is carried as its
/// raw JSON until the generated typed model + parsing land.
pub mod model {
    /// A FHIR resource. For now, its type tag plus the raw JSON payload.
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

/// The verifier types. Conformance = profile constraints + terminology bindings
/// + FHIRPath invariants, evaluated against a resource.
pub mod validate {
    use crate::model::Resource;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Severity {
        Error,
        Warning,
        Information,
    }

    /// A single conformance finding, located by FHIRPath.
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
        /// Conformant iff there are no `Error`-severity issues.
        pub fn is_valid(&self) -> bool {
            !self.issues.iter().any(|i| i.severity == Severity::Error)
        }
    }

    /// A constraint set over a base resource (e.g. AU Core Patient).
    #[derive(Debug, Clone)]
    pub struct Profile {
        pub url: String,
    }

    /// The verifier pass. A generated implementation evaluates `resource`
    /// against `profile` as straight-line code, not a runtime tree-walk.
    pub trait Validator {
        fn validate(&self, resource: &Resource, profile: &Profile) -> Report;
    }
}

/// FHIRPath — the expression language FHIR invariants are written in; the engine
/// that evaluates a constraint such as `value.exists() or dataAbsentReason.exists()`.
pub mod path {
    #[derive(Debug, Default)]
    pub struct Engine;

    impl Engine {
        pub fn new() -> Self {
            Self
        }
    }
}

/// Terminology binding checks: is a coded value drawn from the ValueSet its
/// element is bound to? SNOMED CT carries affiliate licensing in many
/// jurisdictions (LOINC / UCUM are free), so the provider is pluggable and the
/// core ships no licensed content.
pub mod terminology {
    pub trait TerminologyProvider {
        /// Does `code` from `system` belong to the value set at `value_set_url`?
        fn validate_code(&self, value_set_url: &str, system: &str, code: &str) -> bool;
    }
}

/// Lowering passes: map arbitrary source records into FHIR resources via a
/// declarative mapping spec. Deliberately source-agnostic — domain-specific
/// maps live in the host, never in this crate.
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

/// The audit surface — FHIR-native. In FHIR the audit trail is *data*, not a
/// byproduct of observability: `AuditEvent` (who did what to which resource,
/// when, with what outcome) and `Provenance` (a record's lineage). This crate
/// constructs and validates those resources and nothing more — no I/O, no
/// telemetry backend. The host owns transport and durable storage.
///
/// The `trace_id` field encodes the stance: the audit record is the
/// authoritative, durable, unsampled record; a sampled OpenTelemetry trace is
/// correlated context, linked by id — never the reverse. See
/// `docs/architecture.md` § Audit surface.
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

    /// The actor–action–subject–outcome–time tuple every audit regime requires.
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
        /// Build a FHIR `AuditEvent` resource from this record. Commit-zero stub.
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
        assert!(r.contains("\"valid\":false"), "{r}");
        assert!(r.contains("status is required"), "{r}");
    }

    #[test]
    fn bad_status_code_fails_binding() {
        let r = probe_str(
            r#"{"resourceType":"Observation","status":"finalized","code":{},"subject":{},"valueString":"x"}"#,
        );
        assert!(r.contains("observation-status value set"), "{r}");
    }

    #[test]
    fn nested_code_does_not_satisfy_top_level() {
        // "code" is nested inside subject — top-level code is still missing.
        let r = probe_str(
            r#"{"resourceType":"Observation","status":"final","subject":{"code":"nope","reference":"Patient/1"},"valueString":"x"}"#,
        );
        assert!(r.contains("code is required"), "{r}");
    }

    #[test]
    fn tricky_string_does_not_end_object_early() {
        // A string value containing } and an escaped quote must not break scanning.
        let r = probe_str(
            r#"{"resourceType":"Observation","note":"a } \" tricky","status":"final","code":{},"subject":{},"valueString":"x"}"#,
        );
        assert!(r.contains("\"valid\":true"), "{r}");
    }

    #[test]
    fn missing_value_and_dar_warns() {
        let r = probe_str(r#"{"resourceType":"Observation","status":"final","code":{},"subject":{}}"#);
        assert!(r.contains("dataAbsentReason"), "{r}");
    }

    #[test]
    fn nested_arrays_and_objects_are_skipped() {
        let r = probe_str(
            r#"{"resourceType":"Observation","status":"final","category":[{"coding":[{"code":"vital-signs"}]}],"code":{"coding":[{"system":"http://loinc.org","code":"8867-4"}]},"subject":{"reference":"Patient/1"},"valueQuantity":{"value":72,"unit":"beats/minute"}}"#,
        );
        assert!(r.contains("\"valid\":true"), "{r}");
    }

    #[test]
    fn non_observation_has_no_profile_bound() {
        let r = probe_str(r#"{"resourceType":"Patient","id":"1"}"#);
        assert!(r.contains("\"resourceType\":\"Patient\""), "{r}");
        assert!(r.contains("\"valid\":true"), "{r}");
    }

    #[test]
    fn garbage_reports_a_parse_error() {
        let r = probe_str("not json");
        assert!(r.contains("could not parse"), "{r}");
        assert!(r.contains("\"valid\":false"), "{r}");
    }
}
