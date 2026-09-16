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
#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

// `no_std` + `alloc`: the core allocates (the value tree, the issue list) but
// never touches the OS — no I/O, no threads, no clock. The compiler enforces
// the thesis: a `std::fs` call here is a build error, not a code-review catch.
#[macro_use]
extern crate alloc;

use alloc::string::String;
use document::Document;
use validate::{IssueType, Report, Severity};

/// Compiled profiles generated from FHIR packages (`tools/gen_all.py`), with a
/// `REGISTRY` keyed by canonical URL.
pub mod generated;

/// The ergonomic result: what was validated, against which profile, and every
/// finding. Never a thrown error — validation state is data.
pub struct Validation {
    pub resource_type: String,
    /// The profile that was applied, if any (its human title).
    pub profile: Option<&'static str>,
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
    let fields = match json::scan_top_level(input) {
        Ok(f) => f,
        Err(e) => return parse_failure(e),
    };
    match fields.scalar("resourceType") {
        None if !fields.has("resourceType") => {
            let mut report = Report::default();
            report.push(Severity::Error, IssueType::Structure, "resourceType", "resourceType is required".into());
            Validation { resource_type: "Unknown".into(), profile: None, report }
        }
        None => Validation { resource_type: "Unknown".into(), profile: None, report: Report::default() },
        Some(rt) => match document::Parsed::parse(input) {
            Ok(parsed) => {
                let doc = parsed.doc();
                // A declared meta.profile selects the generated profile (spec behavior).
                let declared = doc.scalars("$this.meta.profile");
                let selected = generated::REGISTRY
                    .iter()
                    .find(|p| p.resource_type == rt && declared.iter().any(|u| *u == p.url))
                    .copied();
                // Resource-type defaults when nothing is declared.
                let default = match rt {
                    "Patient" => Some(&generated::au_core_patient::PROFILE),
                    _ => None,
                };
                match selected.or(default) {
                    Some(profile) => Validation {
                        resource_type: rt.into(),
                        profile: Some(profile.name),
                        report: engine::validate(profile, &doc),
                    },
                    None if rt == "Observation" => Validation {
                        resource_type: rt.into(),
                        profile: Some("AU Core Observation"),
                        report: au_core::validate_observation(&fields),
                    },
                    None => Validation { resource_type: rt.into(), profile: None, report: Report::default() },
                }
            }
            Err(e) => parse_failure(e),
        },
    }
}

/// The pods' JSON report — the ergonomic shape, stable for the FFI.
pub fn probe(input: &[u8]) -> String {
    wire::report(&validate_bytes(input))
}

fn parse_failure(e: json::ParseError) -> Validation {
    let mut report = Report::default();
    report.push(
        Severity::Fatal,
        IssueType::Structure,
        "(document)",
        format!("could not parse JSON at byte {}: {}", e.at, e.msg),
    );
    Validation { resource_type: "Unknown".into(), profile: None, report }
}

/// The interop / wire context — the anti-corruption layer. Spec shapes live
/// here and nowhere else: the `OperationOutcome` resource, and the pods' own
/// JSON report.
pub mod wire {
    use alloc::string::String;

    use crate::validate::{IssueType, Report, Severity};
    use crate::Validation;

    /// `OperationOutcome.issue.severity`.
    pub fn severity_str(s: Severity) -> &'static str {
        match s {
            Severity::Fatal => "fatal",
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Information => "information",
        }
    }

    /// `OperationOutcome.issue.code` — the IssueType vocabulary.
    pub fn issue_type_str(c: IssueType) -> &'static str {
        match c {
            IssueType::Structure => "structure",
            IssueType::Required => "required",
            IssueType::Value => "value",
            IssueType::Invariant => "invariant",
            IssueType::CodeInvalid => "code-invalid",
            IssueType::NotSupported => "not-supported",
            IssueType::BusinessRule => "business-rule",
            IssueType::Informational => "informational",
        }
    }

    /// The spec resource. `issue` is 1..*: a clean result carries the
    /// conventional single informational issue.
    pub fn operation_outcome(r: &Report) -> String {
        let mut out = String::with_capacity(160);
        out.push_str("{\"resourceType\":\"OperationOutcome\",\"issue\":[");
        if r.issues.is_empty() {
            out.push_str(
                "{\"severity\":\"information\",\"code\":\"informational\",\"details\":{\"text\":\"Validation successful\"}}",
            );
        }
        for (i, issue) in r.issues.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"severity\":\"");
            out.push_str(severity_str(issue.severity));
            out.push_str("\",\"code\":\"");
            out.push_str(issue_type_str(issue.code));
            out.push_str("\",\"details\":{\"text\":\"");
            escape(&mut out, &issue.message);
            out.push_str("\"},\"expression\":[\"");
            escape(&mut out, &expression(&issue.path, issue.code));
            out.push_str("\"]}");
        }
        out.push_str("]}");
        out
    }

    /// `OperationOutcome.issue.expression` — the location as the reference
    /// validator reports it, so issue-level diffs are apples to apples:
    /// * a **missing** element (`Required`) is located at its **parent** — there
    ///   is no node to point at (`Patient.gender` missing → `Patient`);
    /// * our slice labels become FHIRPath: `value[x]:valueQuantity` →
    ///   `value.ofType(Quantity)`, and a plain `:slice` label is dropped.
    /// The ergonomic `Issue.path` keeps the precise, labelled location.
    pub fn expression(path: &str, code: IssueType) -> String {
        let located = if code == IssueType::Required {
            path.rsplit_once('.').map_or(path, |(parent, _)| parent)
        } else {
            path
        };
        let mut out = String::with_capacity(located.len());
        for (i, seg) in located.split('.').enumerate() {
            if i > 0 {
                out.push('.');
            }
            match seg.split_once(':') {
                Some((name, key)) => match name.strip_suffix("[x]") {
                    Some(base) => {
                        out.push_str(base);
                        if let Some(ty) = key.strip_prefix(base) {
                            out.push_str(".ofType(");
                            out.push_str(ty);
                            out.push(')');
                        }
                    }
                    None => out.push_str(name),
                },
                None => out.push_str(seg),
            }
        }
        out
    }

    /// The pods' JSON report: resource type, applied profile, verdict, issues.
    pub fn report(v: &Validation) -> String {
        let mut out = String::with_capacity(160);
        out.push_str("{\"resourceType\":\"");
        escape(&mut out, &v.resource_type);
        out.push('"');
        if let Some(p) = v.profile {
            out.push_str(",\"profile\":\"");
            escape(&mut out, p);
            out.push('"');
        }
        out.push_str(",\"valid\":");
        out.push_str(if v.is_valid() { "true" } else { "false" });
        out.push_str(",\"issues\":[");
        for (i, issue) in v.report.issues.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"severity\":\"");
            out.push_str(severity_str(issue.severity));
            out.push_str("\",\"code\":\"");
            out.push_str(issue_type_str(issue.code));
            out.push_str("\",\"path\":\"");
            escape(&mut out, &issue.path);
            out.push_str("\",\"message\":\"");
            escape(&mut out, &issue.message);
            out.push_str("\"}");
        }
        out.push_str("]}");
        out
    }

    /// Append `s` to `out`, escaping the characters JSON requires.
    pub(crate) fn escape(out: &mut String, s: &str) {
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c if (c as u32) < 0x20 => out.push(' '),
                c => out.push(c),
            }
        }
    }
}

/// JSON, two ways. `scan_top_level` is the zero-copy single pass over a
/// resource's top-level members (no DOM). `parse` builds a borrowed value tree —
/// the correctness-first adapter behind the `Document` port.
pub mod json {
    use alloc::vec::Vec;

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

    pub fn parse(input: &[u8]) -> Result<Value<'_>, ParseError> {
        let mut p = Parser { b: input, i: 0, depth: 0 };
        p.parse_value()
    }

    /// Resolve a dotted path against a node. The first segment is the root's
    /// name (the resource type, or `$this` for a sub-document) and is skipped.
    /// Arrays expand; `[x]` matches a choice suffix (`value[x]` → `valueQuantity`).
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
    ///
    /// FHIR JSON represents a primitive that carries only extensions (no value)
    /// as its `_name` sibling alone — `"_birthDate": {"extension": […]}` with no
    /// `birthDate`. That element **is present** (it satisfies `min`), so when the
    /// value key is absent we resolve through `_name`, which also makes its
    /// `.extension` reachable. When both are present they describe one element,
    /// so only the value key is counted.
    pub fn children<'v, 'a>(node: &'v Value<'a>, seg: &str, out: &mut Vec<&'v Value<'a>>) {
        match node {
            Value::Object(members) => {
                let before = out.len();
                for (k, v) in members {
                    if key_matches(k, seg) {
                        push_expanded(v, out);
                    }
                }
                if out.len() == before && !seg.ends_with("[x]") {
                    for (k, v) in members {
                        if k.strip_prefix('_') == Some(seg) {
                            push_expanded(v, out);
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

    fn push_expanded<'v, 'a>(v: &'v Value<'a>, out: &mut Vec<&'v Value<'a>>) {
        match v {
            Value::Array(items) => out.extend(items.iter()),
            other => out.push(other),
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

    /// Recursive descent must be bounded: past this, nesting is a `ParseError`
    /// — never a stack overflow, which is an abort (not a panic) and fatal on
    /// device. Real FHIR resources are ~10–20 deep.
    const MAX_DEPTH: usize = 128;

    struct Parser<'a> {
        b: &'a [u8],
        i: usize,
        depth: usize,
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
                Some(c @ (b'{' | b'[')) => {
                    if self.depth >= MAX_DEPTH {
                        return Err(ParseError { at: self.i, msg: "nesting too deep" });
                    }
                    self.depth += 1;
                    let r = if c == b'{' { self.parse_object() } else { self.parse_array() };
                    self.depth -= 1;
                    r
                }
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
        let mut p = Parser { b: input, i: 0, depth: 0 };
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
/// JSON representation. `TreeDoc` is today's adapter — a cheap view that can be
/// re-rooted at any node, which is what slicing needs.
pub mod document {
    use alloc::vec::Vec;

    use crate::json::{self, Value};

    pub trait Document: Sized + Clone {
        /// For each instance of the element's parent, how many children exist
        /// at the last path segment. Empty when no parent exists — a child's
        /// cardinality is only evaluated where its parent is present.
        fn child_counts(&self, path: &str) -> Vec<usize>;
        /// All scalar string values at `path` (across every parent).
        fn scalars(&self, path: &str) -> Vec<&str>;
        /// Sub-documents rooted at each node `path` resolves to.
        fn instances(&self, path: &str) -> Vec<Self>;
        /// This node's scalar value as text, if it is a scalar.
        fn as_scalar(&self) -> Option<&str>;
        /// This node's value if it is a JSON boolean.
        fn as_bool(&self) -> Option<bool>;
    }

    /// Owner of a parsed resource; hands out `TreeDoc` views.
    pub struct Parsed<'a> {
        value: Value<'a>,
    }

    impl<'a> Parsed<'a> {
        pub fn parse(input: &'a [u8]) -> Result<Self, json::ParseError> {
            Ok(Self { value: json::parse(input)? })
        }
        pub fn doc(&self) -> TreeDoc<'_, 'a> {
            TreeDoc { root: &self.value }
        }
    }

    /// The value-tree adapter: a view rooted at any node.
    #[derive(Clone, Copy)]
    pub struct TreeDoc<'v, 'a> {
        root: &'v Value<'a>,
    }

    impl<'v, 'a> Document for TreeDoc<'v, 'a> {
        fn child_counts(&self, path: &str) -> Vec<usize> {
            let Some((parent, last)) = path.rsplit_once('.') else {
                return Vec::new();
            };
            json::resolve(self.root, parent)
                .into_iter()
                .map(|p| {
                    let mut out = Vec::new();
                    json::children(p, last, &mut out);
                    out.len()
                })
                .collect()
        }

        fn scalars(&self, path: &str) -> Vec<&str> {
            json::resolve(self.root, path)
                .into_iter()
                .filter_map(|v| match v {
                    Value::Str(s) => Some(*s),
                    _ => None,
                })
                .collect()
        }

        fn instances(&self, path: &str) -> Vec<Self> {
            json::resolve(self.root, path).into_iter().map(|n| TreeDoc { root: n }).collect()
        }

        fn as_scalar(&self) -> Option<&str> {
            match self.root {
                Value::Str(s) | Value::Number(s) => Some(*s),
                Value::Bool(b) => Some(if *b { "true" } else { "false" }),
                _ => None,
            }
        }

        fn as_bool(&self) -> Option<bool> {
            match self.root {
                Value::Bool(b) => Some(*b),
                _ => None,
            }
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
    /// Inside a slice, `path` is relative to the matched instance: `$this.…`.
    pub struct ElementConstraint {
        pub path: &'static str,
        pub min: u32,
        pub max: Max,
        /// A scalar the element must equal exactly (fixedUri / fixedCode / …).
        /// For a SNOMED coding this is only ever `system`/`code`, never `display`.
        pub fixed: Option<&'static str>,
        /// A required-strength ValueSet binding on a code element.
        pub required_binding: Option<&'static str>,
    }

    /// How an instance of a sliced element is assigned to a slice.
    pub enum Discriminator {
        /// The scalars at a relative path (`$this.system`) must include `value`.
        Value { path: &'static str, value: &'static str },
        /// A choice-type slice: the instance is the member under this concrete
        /// key (`valueQuantity`).
        Type { key: &'static str },
    }

    pub struct Slice {
        pub name: &'static str,
        pub discriminators: &'static [Discriminator],
        pub min: u32,
        pub max: Max,
        /// Constraints on the slice's sub-elements, paths relative as `$this.…`.
        pub elements: &'static [ElementConstraint],
        /// A profile (by canonical URL, looked up in the registry) that matched
        /// instances must also conform to — how a slice inherits an external
        /// datatype profile's constraints, e.g. `au-ihi` on `Patient.identifier:ihi`.
        pub profile: Option<&'static str>,
    }

    pub struct Slicing {
        /// The sliced element, absolute path (`Observation.code.coding`).
        pub path: &'static str,
        /// `closed`: every instance must match some slice.
        pub closed: bool,
        pub slices: &'static [Slice],
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum InvariantSeverity {
        Error,
        Warning,
    }

    /// A FHIRPath invariant (`ElementDefinition.constraint`), evaluated with each
    /// instance of `path` as `$this`. Expressions outside the supported core are
    /// skipped at runtime — never guessed.
    pub struct ProfileInvariant {
        pub path: &'static str,
        pub key: &'static str,
        pub severity: InvariantSeverity,
        pub human: &'static str,
        pub expression: &'static str,
    }

    /// A profile compiled to embeddable rules.
    pub struct CompiledProfile {
        pub url: &'static str,
        pub name: &'static str,
        pub resource_type: &'static str,
        pub elements: &'static [ElementConstraint],
        pub slicings: &'static [Slicing],
        pub invariants: &'static [ProfileInvariant],
        /// Constraints not yet enforced (nested slicing / non-scalar patterns / mustSupport-only).
        pub deferred: u32,
    }
}

/// The validation engine — the domain service. Pure: a compiled profile and a
/// document in, a report out.
pub mod engine {
    use alloc::vec::Vec;

    use crate::compiled::{CompiledProfile, Discriminator, ElementConstraint, InvariantSeverity, Max, Slice, Slicing};
    use crate::document::Document;
    use crate::path::{self, Verdict};
    use crate::validate::{IssueType, Report, Severity};

    /// How many profiles a slice may inherit through, transitively. Bounds the
    /// recursion so a registry cycle can never loop.
    const MAX_INHERIT: usize = 8;

    pub fn validate(profile: &CompiledProfile, doc: &impl Document) -> Report {
        validate_at(profile, doc, 0)
    }

    fn validate_at(profile: &CompiledProfile, doc: &impl Document, depth: usize) -> Report {
        let mut r = Report::default();
        for el in profile.elements {
            check_element(el, el.path, doc, &mut r);
        }
        for slicing in profile.slicings {
            check_slicing(slicing, doc, &mut r, depth);
        }
        for inv in profile.invariants {
            // Outside the supported FHIRPath core: skipped, never guessed.
            let Ok(expr) = path::parse(inv.expression) else { continue };
            for ctx in doc.instances(inv.path) {
                if path::check(&expr, &ctx) == Verdict::Fail {
                    let severity = match inv.severity {
                        InvariantSeverity::Error => Severity::Error,
                        InvariantSeverity::Warning => Severity::Warning,
                    };
                    r.push(severity, IssueType::Invariant, inv.path, format!("{}: {}", inv.key, inv.human));
                }
            }
        }
        r
    }

    /// Cardinality (per parent instance), fixed value, and required binding for
    /// one element. `label` is the path reported in issues — absolute at the top
    /// level, `Path:slice.sub` inside a slice.
    fn check_element(el: &ElementConstraint, label: &str, doc: &impl Document, r: &mut Report) {
        for count in doc.child_counts(el.path) {
            let count = count as u32;
            if count < el.min {
                r.push(Severity::Error, IssueType::Required, label, format!("{label} is required (min {}, found {count})", el.min));
            }
            if let Max::Bounded(m) = el.max {
                if count > m {
                    r.push(Severity::Error, IssueType::Structure, label, format!("{label} exceeds max {m} (found {count})"));
                }
            }
        }
        if el.fixed.is_none() && el.required_binding.is_none() {
            return;
        }
        for s in doc.scalars(el.path) {
            if let Some(f) = el.fixed {
                if s != f {
                    r.push(Severity::Error, IssueType::Value, label, format!("{label} must be '{f}' (found '{s}')"));
                }
            }
            if let Some(vs) = el.required_binding {
                match crate::value_sets::in_value_set(vs, s) {
                    Some(false) => r.push(Severity::Error, IssueType::CodeInvalid, label, format!("'{s}' is not in the bound value set")),
                    None => r.push(
                        Severity::Warning,
                        IssueType::NotSupported,
                        label,
                        format!("no bundled terminology for {vs}; '{s}' not checked"),
                    ),
                    Some(true) => {}
                }
            }
        }
    }

    /// Does this instance belong to the slice? Value/pattern discriminators must
    /// all match; type slices are selected by key in `check_slicing`.
    fn matches(slice: &Slice, inst: &impl Document) -> bool {
        slice.discriminators.iter().all(|d| match d {
            Discriminator::Value { path, value } => inst.scalars(path).iter().any(|s| *s == *value),
            Discriminator::Type { .. } => true,
        })
    }

    fn type_key(slice: &Slice) -> Option<&'static str> {
        slice.discriminators.iter().find_map(|d| match d {
            Discriminator::Type { key } => Some(*key),
            _ => None,
        })
    }

    /// Evaluate a slicing per parent instance: assign instances to slices, check
    /// each slice's cardinality, validate its sub-constraints against every
    /// matched instance (re-rooted as `$this`), and enforce `closed`.
    fn check_slicing(slicing: &Slicing, doc: &impl Document, r: &mut Report, depth: usize) {
        let Some((parent_path, last)) = slicing.path.rsplit_once('.') else {
            return;
        };
        for parent in doc.instances(parent_path) {
            let all = parent.instances(&format!("$this.{last}"));
            let mut matched = 0usize;
            for slice in slicing.slices {
                let label = format!("{}:{}", slicing.path, slice.name);
                let insts: Vec<_> = match type_key(slice) {
                    Some(key) => parent.instances(&format!("$this.{key}")),
                    None => all.iter().filter(|i| matches(slice, *i)).cloned().collect(),
                };
                matched += insts.len();
                let count = insts.len() as u32;
                if count < slice.min {
                    r.push(Severity::Error, IssueType::Required, &label, format!("{label} is required (min {}, found {count})", slice.min));
                }
                if let Max::Bounded(m) = slice.max {
                    if count > m {
                        r.push(Severity::Error, IssueType::Structure, &label, format!("{label} exceeds max {m} (found {count})"));
                    }
                }
                for inst in &insts {
                    for el in slice.elements {
                        let sub = format!("{label}.{}", el.path.trim_start_matches("$this."));
                        check_element(el, &sub, inst, r);
                    }
                    // A slice that names a profile: matched instances must also
                    // conform to it. Recursive — the instance becomes the root,
                    // and the profile's own element paths resolve against it.
                    if let Some(url) = slice.profile.filter(|_| depth < MAX_INHERIT) {
                        if let Some(profile) = crate::generated::REGISTRY.iter().find(|p| p.url == url) {
                            for issue in validate_at(profile, inst, depth + 1).issues {
                                let rel = issue
                                    .path
                                    .strip_prefix(profile.resource_type)
                                    .map(|s| s.trim_start_matches('.'))
                                    .unwrap_or(&issue.path);
                                let path = if rel.is_empty() { label.clone() } else { format!("{label}.{rel}") };
                                r.push(issue.severity, issue.code, &path, issue.message);
                            }
                        }
                    }
                }
            }
            if slicing.closed && matched < all.len() {
                r.push(
                    Severity::Error,
                    IssueType::Structure,
                    slicing.path,
                    format!("{} has {} element(s) matching no slice (closed slicing)", slicing.path, all.len() - matched),
                );
            }
        }
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
            "http://hl7.org/fhir/ValueSet/quantity-comparator" => {
                Some(matches!(code, "<" | "<=" | ">=" | ">"))
            }
            // The vital-signs UCUM unit sets, concept lists taken verbatim from
            // hl7.fhir.r4.core#4.0.1 (CC0). UCUM is freely embeddable.
            "http://hl7.org/fhir/ValueSet/ucum-vitals-common" => Some(matches!(
                code,
                "%" | "cm" | "[in_i]" | "kg" | "g" | "[lb_av]" | "Cel" | "[degF]" | "mm[Hg]" | "/min"
                    | "kg/m2" | "m2"
            )),
            "http://hl7.org/fhir/ValueSet/ucum-bodyweight" => Some(matches!(code, "kg" | "[lb_av]" | "g")),
            "http://hl7.org/fhir/ValueSet/ucum-bodylength" => Some(matches!(code, "cm" | "[in_i]")),
            "http://hl7.org/fhir/ValueSet/ucum-bodytemp" => Some(matches!(code, "Cel" | "[degF]")),
            _ => None,
        }
    }
}

/// Default Observation checks over the single-pass scanner, used when no
/// generated profile is declared in `meta.profile`.
pub mod au_core {
    use crate::json::TopLevel;
    use crate::validate::{IssueType, Report, Severity};

    pub fn is_observation_status(code: &str) -> bool {
        crate::value_sets::in_value_set("http://hl7.org/fhir/ValueSet/observation-status", code)
            .unwrap_or(false)
    }

    pub fn validate_observation(fields: &TopLevel) -> Report {
        let mut r = Report::default();
        match fields.scalar("status") {
            None => r.push(Severity::Error, IssueType::Required, "Observation.status", "status is required (1..1)".into()),
            Some(s) if !is_observation_status(s) => r.push(
                Severity::Error,
                IssueType::CodeInvalid,
                "Observation.status",
                format!("status '{s}' is not in the observation-status value set"),
            ),
            Some(_) => {}
        }
        if !fields.has("code") {
            r.push(Severity::Error, IssueType::Required, "Observation.code", "code is required (1..1)".into());
        }
        if !fields.has("subject") {
            r.push(Severity::Error, IssueType::Required, "Observation.subject", "subject is required (1..1)".into());
        }
        if !fields.has_value_x() && !fields.has("dataAbsentReason") {
            r.push(
                Severity::Warning,
                IssueType::Invariant,
                "Observation",
                "neither value[x] nor dataAbsentReason is present".into(),
            );
        }
        r
    }
}

pub mod model {
    use alloc::string::String;

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

/// The verifier types — the finding vocabulary, aligned to `OperationOutcome`.
pub mod validate {
    use alloc::{string::String, vec::Vec};

    use crate::model::Resource;

    /// `OperationOutcome.issue.severity`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Severity {
        Fatal,
        Error,
        Warning,
        Information,
    }

    /// `OperationOutcome.issue.code` — the IssueType vocabulary (the subset we emit).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum IssueType {
        /// Structural problem: parse failure, cardinality exceeded, closed slicing.
        Structure,
        /// A required element is missing.
        Required,
        /// An element value is invalid (e.g. does not match a fixed value).
        Value,
        /// A FHIRPath invariant failed.
        Invariant,
        /// A code is not in the bound value set.
        CodeInvalid,
        /// A check that could not be performed (e.g. no terminology for a set).
        NotSupported,
        BusinessRule,
        Informational,
    }

    #[derive(Debug, Clone)]
    pub struct Issue {
        pub severity: Severity,
        pub code: IssueType,
        /// FHIRPath-style location — `OperationOutcome.issue.expression`.
        pub path: String,
        pub message: String,
    }

    #[derive(Debug, Default)]
    pub struct Report {
        pub issues: Vec<Issue>,
    }

    impl Report {
        /// Conformant iff there are no `Fatal` or `Error` issues.
        pub fn is_valid(&self) -> bool {
            !self.issues.iter().any(|i| matches!(i.severity, Severity::Fatal | Severity::Error))
        }

        /// The spec resource — a FHIR `OperationOutcome`.
        pub fn to_operation_outcome(&self) -> String {
            crate::wire::operation_outcome(self)
        }

        pub(crate) fn push(&mut self, severity: Severity, code: IssueType, path: &str, message: String) {
            self.issues.push(Issue { severity, code, path: path.into(), message });
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

/// FHIRPath — the expression language FHIR invariants are written in. A
/// deliberately small, honest core: navigation (with a choice-type fallback so
/// `value` finds `valueQuantity`), `exists` / `empty` / `not` / `count` /
/// `first` / `hasValue` / `where`, `and` / `or` / `xor` / `implies`, `=` / `!=`,
/// string / integer / boolean literals, `$this`. Anything else fails to parse
/// or evaluates as [`Unsupported`] and the invariant is skipped — never
/// guessed. Three-valued logic per the spec: an empty result is "unknown", and
/// an invariant fails only on a definite `false`.
pub mod path {
    use alloc::{boxed::Box, string::String, vec::Vec};

    use crate::document::Document;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Lit {
        Str(String),
        Int(i64),
        Bool(bool),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Op {
        And,
        Or,
        Xor,
        Implies,
        Eq,
        Ne,
        Lt,
        Le,
        Gt,
        Ge,
        Add,
        Sub,
        Mul,
        /// Integer division (`div`). `/` yields decimals and is outside the core.
        Div,
        Mod,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum Expr {
        This,
        Lit(Lit),
        /// `base.name` — navigation.
        Nav(Box<Expr>, String),
        /// `base.name(args)` — a function call.
        Call(Box<Expr>, String, Vec<Expr>),
        Bin(Box<Expr>, Op, Box<Expr>),
    }

    /// Syntax or semantics outside the supported core.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Unsupported;

    #[derive(Debug, Clone, PartialEq)]
    enum Tok {
        Ident(String),
        Str(String),
        Int(i64),
        LParen,
        RParen,
        Dot,
        Comma,
        Eq,
        Ne,
        Lt,
        Le,
        Gt,
        Ge,
        Plus,
        Minus,
        Star,
        /// Anything we don't support (`/`, `|`, `%`, …) — makes the parse fail.
        Other,
    }

    fn tokenize(src: &str) -> Result<Vec<Tok>, Unsupported> {
        let b = src.as_bytes();
        let mut i = 0;
        let mut out = Vec::new();
        while i < b.len() {
            let c = b[i];
            match c {
                b' ' | b'\t' | b'\n' | b'\r' => i += 1,
                b'(' => {
                    out.push(Tok::LParen);
                    i += 1;
                }
                b')' => {
                    out.push(Tok::RParen);
                    i += 1;
                }
                b'.' => {
                    out.push(Tok::Dot);
                    i += 1;
                }
                b',' => {
                    out.push(Tok::Comma);
                    i += 1;
                }
                b'=' => {
                    out.push(Tok::Eq);
                    i += 1;
                }
                b'!' if b.get(i + 1) == Some(&b'=') => {
                    out.push(Tok::Ne);
                    i += 2;
                }
                b'<' | b'>' => {
                    let eq = b.get(i + 1) == Some(&b'=');
                    out.push(match (c, eq) {
                        (b'<', false) => Tok::Lt,
                        (b'<', true) => Tok::Le,
                        (_, false) => Tok::Gt,
                        (_, true) => Tok::Ge,
                    });
                    i += if eq { 2 } else { 1 };
                }
                b'+' => {
                    out.push(Tok::Plus);
                    i += 1;
                }
                b'-' => {
                    out.push(Tok::Minus);
                    i += 1;
                }
                b'*' => {
                    out.push(Tok::Star);
                    i += 1;
                }
                b'`' => {
                    // A delimited identifier: an element named like a keyword (`text.`div``).
                    let start = i + 1;
                    let mut j = start;
                    while j < b.len() && b[j] != b'`' {
                        j += 1;
                    }
                    if j >= b.len() || j == start {
                        return Err(Unsupported);
                    }
                    let s = core::str::from_utf8(&b[start..j]).map_err(|_| Unsupported)?;
                    out.push(Tok::Ident(s.into()));
                    i = j + 1;
                }
                b'\'' => {
                    let start = i + 1;
                    let mut j = start;
                    while j < b.len() && b[j] != b'\'' {
                        if b[j] == b'\\' {
                            j += 1;
                        }
                        j += 1;
                    }
                    if j >= b.len() {
                        return Err(Unsupported);
                    }
                    let s = core::str::from_utf8(&b[start..j]).map_err(|_| Unsupported)?;
                    out.push(Tok::Str(s.replace("\\'", "'")));
                    i = j + 1;
                }
                b'0'..=b'9' => {
                    let start = i;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i < b.len() && b[i] == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                        return Err(Unsupported); // decimals: not in the core
                    }
                    let n = core::str::from_utf8(&b[start..i])
                        .ok()
                        .and_then(|s| s.parse::<i64>().ok())
                        .ok_or(Unsupported)?;
                    out.push(Tok::Int(n));
                }
                b'$' | b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                    let start = i;
                    i += 1;
                    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                        i += 1;
                    }
                    let s = core::str::from_utf8(&b[start..i]).map_err(|_| Unsupported)?;
                    out.push(Tok::Ident(s.into()));
                }
                _ => {
                    out.push(Tok::Other);
                    i += 1;
                }
            }
        }
        Ok(out)
    }

    /// Expression nesting bound (parens, call arguments): past this the parse
    /// is `Unsupported` rather than a stack overflow.
    const MAX_DEPTH: usize = 64;

    struct P {
        toks: Vec<Tok>,
        i: usize,
        depth: usize,
    }

    impl P {
        fn peek(&self) -> Option<&Tok> {
            self.toks.get(self.i)
        }

        fn next(&mut self) -> Option<Tok> {
            let t = self.toks.get(self.i).cloned();
            self.i += 1;
            t
        }

        fn eat_kw(&mut self, kw: &str) -> bool {
            if matches!(self.peek(), Some(Tok::Ident(s)) if s == kw) {
                self.i += 1;
                true
            } else {
                false
            }
        }

        fn expect(&mut self, t: Tok) -> Result<(), Unsupported> {
            if self.next() == Some(t) {
                Ok(())
            } else {
                Err(Unsupported)
            }
        }

        fn args(&mut self) -> Result<Vec<Expr>, Unsupported> {
            self.expect(Tok::LParen)?;
            let mut args = Vec::new();
            if !matches!(self.peek(), Some(Tok::RParen)) {
                loop {
                    args.push(self.implies()?);
                    if matches!(self.peek(), Some(Tok::Comma)) {
                        self.i += 1;
                    } else {
                        break;
                    }
                }
            }
            self.expect(Tok::RParen)?;
            Ok(args)
        }

        // Precedence, low to high (the spec's table, minus what's outside the
        // core): implies < or/xor < and < =/!= < comparison < +/- < * div mod
        // < unary - < path/call. `implies` is the entry of every nested
        // expression (parens, call arguments), so the depth bound lives here.
        fn implies(&mut self) -> Result<Expr, Unsupported> {
            if self.depth >= MAX_DEPTH {
                return Err(Unsupported);
            }
            self.depth += 1;
            let r = self.implies_inner();
            self.depth -= 1;
            r
        }

        fn implies_inner(&mut self) -> Result<Expr, Unsupported> {
            let l = self.or()?;
            if self.eat_kw("implies") {
                let r = self.implies()?;
                return Ok(Expr::Bin(Box::new(l), Op::Implies, Box::new(r)));
            }
            Ok(l)
        }

        fn or(&mut self) -> Result<Expr, Unsupported> {
            let mut l = self.and()?;
            loop {
                let op = if self.eat_kw("or") {
                    Op::Or
                } else if self.eat_kw("xor") {
                    Op::Xor
                } else {
                    return Ok(l);
                };
                let r = self.and()?;
                l = Expr::Bin(Box::new(l), op, Box::new(r));
            }
        }

        fn and(&mut self) -> Result<Expr, Unsupported> {
            let mut l = self.eq()?;
            while self.eat_kw("and") {
                let r = self.eq()?;
                l = Expr::Bin(Box::new(l), Op::And, Box::new(r));
            }
            Ok(l)
        }

        fn eq(&mut self) -> Result<Expr, Unsupported> {
            let l = self.cmp()?;
            let op = match self.peek() {
                Some(Tok::Eq) => Op::Eq,
                Some(Tok::Ne) => Op::Ne,
                _ => return Ok(l),
            };
            self.i += 1;
            let r = self.cmp()?;
            Ok(Expr::Bin(Box::new(l), op, Box::new(r)))
        }

        fn cmp(&mut self) -> Result<Expr, Unsupported> {
            let l = self.add()?;
            let op = match self.peek() {
                Some(Tok::Lt) => Op::Lt,
                Some(Tok::Le) => Op::Le,
                Some(Tok::Gt) => Op::Gt,
                Some(Tok::Ge) => Op::Ge,
                _ => return Ok(l),
            };
            self.i += 1;
            let r = self.add()?;
            Ok(Expr::Bin(Box::new(l), op, Box::new(r)))
        }

        fn add(&mut self) -> Result<Expr, Unsupported> {
            let mut l = self.mul()?;
            loop {
                let op = match self.peek() {
                    Some(Tok::Plus) => Op::Add,
                    Some(Tok::Minus) => Op::Sub,
                    _ => return Ok(l),
                };
                self.i += 1;
                let r = self.mul()?;
                l = Expr::Bin(Box::new(l), op, Box::new(r));
            }
        }

        fn mul(&mut self) -> Result<Expr, Unsupported> {
            let mut l = self.unary()?;
            loop {
                let op = if matches!(self.peek(), Some(Tok::Star)) {
                    self.i += 1;
                    Op::Mul
                } else if self.eat_kw("div") {
                    Op::Div
                } else if self.eat_kw("mod") {
                    Op::Mod
                } else {
                    return Ok(l);
                };
                let r = self.unary()?;
                l = Expr::Bin(Box::new(l), op, Box::new(r));
            }
        }

        fn unary(&mut self) -> Result<Expr, Unsupported> {
            match self.peek() {
                Some(Tok::Minus) => {
                    self.i += 1;
                    match self.next() {
                        Some(Tok::Int(n)) => Ok(Expr::Lit(Lit::Int(n.checked_neg().ok_or(Unsupported)?))),
                        _ => Err(Unsupported),
                    }
                }
                Some(Tok::Plus) => {
                    self.i += 1;
                    match self.next() {
                        Some(Tok::Int(n)) => Ok(Expr::Lit(Lit::Int(n))),
                        _ => Err(Unsupported),
                    }
                }
                _ => self.postfix(),
            }
        }

        fn postfix(&mut self) -> Result<Expr, Unsupported> {
            let mut e = self.primary()?;
            while matches!(self.peek(), Some(Tok::Dot)) {
                self.i += 1;
                let Some(Tok::Ident(name)) = self.next() else {
                    return Err(Unsupported);
                };
                e = if matches!(self.peek(), Some(Tok::LParen)) {
                    Expr::Call(Box::new(e), name, self.args()?)
                } else {
                    Expr::Nav(Box::new(e), name)
                };
            }
            Ok(e)
        }

        fn primary(&mut self) -> Result<Expr, Unsupported> {
            match self.next() {
                Some(Tok::Str(s)) => Ok(Expr::Lit(Lit::Str(s))),
                Some(Tok::Int(n)) => Ok(Expr::Lit(Lit::Int(n))),
                Some(Tok::LParen) => {
                    let e = self.implies()?;
                    self.expect(Tok::RParen)?;
                    Ok(e)
                }
                Some(Tok::Ident(s)) => match s.as_str() {
                    "true" => Ok(Expr::Lit(Lit::Bool(true))),
                    "false" => Ok(Expr::Lit(Lit::Bool(false))),
                    "$this" => Ok(Expr::This),
                    // A bare identifier navigates from `$this`; a bare call applies to it.
                    _ if matches!(self.peek(), Some(Tok::LParen)) => {
                        Ok(Expr::Call(Box::new(Expr::This), s, self.args()?))
                    }
                    _ => Ok(Expr::Nav(Box::new(Expr::This), s)),
                },
                _ => Err(Unsupported),
            }
        }
    }

    /// Parse an expression, or `Unsupported` if it uses anything outside the core.
    pub fn parse(src: &str) -> Result<Expr, Unsupported> {
        let toks = tokenize(src)?;
        let mut p = P { toks, i: 0, depth: 0 };
        let e = p.implies()?;
        if p.i != p.toks.len() {
            return Err(Unsupported);
        }
        Ok(e)
    }

    /// One item of a FHIRPath collection.
    #[derive(Clone)]
    pub enum Item<D> {
        Node(D),
        Str(String),
        Int(i64),
        Bool(bool),
    }

    fn scalar_of<D: Document>(it: &Item<D>) -> Option<String> {
        match it {
            Item::Node(d) => d.as_scalar().map(String::from),
            Item::Str(s) => Some(s.clone()),
            Item::Int(n) => Some(format!("{n}")),
            Item::Bool(b) => Some(String::from(if *b { "true" } else { "false" })),
        }
    }

    /// A collection in boolean context: empty is unknown; a lone boolean is
    /// itself; anything else present is true.
    fn truth<D: Document>(c: &[Item<D>]) -> Option<bool> {
        match c {
            [] => None,
            [Item::Bool(b)] => Some(*b),
            [Item::Node(d)] => Some(d.as_bool().unwrap_or(true)),
            _ => Some(true),
        }
    }

    /// `where(cond)`: each item becomes `$this` for its own test.
    fn filter<D: Document>(coll: &[Item<D>], cond: &Expr) -> Result<Vec<Item<D>>, Unsupported> {
        let mut out = Vec::new();
        for it in coll {
            if truth(&eval(cond, it)?) == Some(true) {
                out.push(it.clone());
            }
        }
        Ok(out)
    }

    /// A collection that must be empty or a single item; more is unsupported
    /// (the spec raises an error — we skip rather than guess).
    fn single<D: Document>(coll: &[Item<D>]) -> Result<Option<&Item<D>>, Unsupported> {
        match coll {
            [] => Ok(None),
            [one] => Ok(Some(one)),
            _ => Err(Unsupported),
        }
    }

    /// The text of a string-valued item. Integers and booleans are not
    /// strings: string functions on them are an error in the spec.
    fn text_of<D: Document>(it: &Item<D>) -> Result<String, Unsupported> {
        match it {
            Item::Str(s) => Ok(s.clone()),
            Item::Node(d) if d.as_bool().is_none() => d.as_scalar().map(String::from).ok_or(Unsupported),
            _ => Err(Unsupported),
        }
    }

    fn int_of<D: Document>(it: &Item<D>) -> Result<i64, Unsupported> {
        match it {
            Item::Int(n) => Ok(*n),
            _ => Err(Unsupported),
        }
    }

    /// `toInteger()`: an integer stays; a boolean is 1/0; text matching
    /// `[+-]?\d+` converts; anything else is empty.
    fn to_integer<D: Document>(it: &Item<D>) -> Option<i64> {
        match it {
            Item::Int(n) => Some(*n),
            Item::Bool(b) => Some(*b as i64),
            Item::Node(d) if d.as_bool().is_some() => d.as_bool().map(|b| b as i64),
            Item::Node(d) => d.as_scalar().and_then(parse_int),
            Item::Str(s) => parse_int(s),
        }
    }

    fn parse_int(s: &str) -> Option<i64> {
        let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    }

    /// `substring(start[, length])`, in characters; a start outside the
    /// string (or a negative length) is empty, not an error.
    fn substring(s: &str, start: i64, len: Option<i64>) -> Option<String> {
        let chars: Vec<char> = s.chars().collect();
        if start < 0 || start as usize >= chars.len() {
            return None;
        }
        let start = start as usize;
        let end = match len {
            None => chars.len(),
            Some(l) if l < 0 => return None,
            Some(l) => (start + l as usize).min(chars.len()),
        };
        Some(chars[start..end].iter().collect())
    }

    fn arith(op: Op, x: i64, y: i64) -> Result<Option<i64>, Unsupported> {
        let r = match op {
            Op::Add => x.checked_add(y),
            Op::Sub => x.checked_sub(y),
            Op::Mul => x.checked_mul(y),
            // Division by zero is empty in the spec, not an error.
            Op::Div if y == 0 => return Ok(None),
            Op::Mod if y == 0 => return Ok(None),
            Op::Div => x.checked_div(y),
            Op::Mod => x.checked_rem(y),
            _ => return Err(Unsupported),
        };
        // Overflow: the spec's integers are 32-bit and overflow is an error.
        r.map(Some).ok_or(Unsupported)
    }

    fn compare<D: Document>(op: Op, a: &Item<D>, b: &Item<D>) -> Result<bool, Unsupported> {
        let ord = match (a, b) {
            (Item::Int(x), Item::Int(y)) => x.cmp(y),
            (Item::Int(_), _) | (_, Item::Int(_)) | (Item::Bool(_), _) | (_, Item::Bool(_)) => return Err(Unsupported),
            // Strings compare by code point, which is what `str`'s order is.
            _ => text_of(a)?.cmp(&text_of(b)?),
        };
        Ok(match op {
            Op::Lt => ord.is_lt(),
            Op::Le => ord.is_le(),
            Op::Gt => ord.is_gt(),
            Op::Ge => ord.is_ge(),
            _ => return Err(Unsupported),
        })
    }

    fn logic(op: Op, a: Option<bool>, b: Option<bool>) -> Option<bool> {
        match op {
            Op::And => match (a, b) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            Op::Or => match (a, b) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            Op::Xor => match (a, b) {
                (Some(x), Some(y)) => Some(x != y),
                _ => None,
            },
            Op::Implies => match (a, b) {
                (Some(false), _) => Some(true),
                (Some(true), y) => y,
                (None, Some(true)) => Some(true),
                (None, _) => None,
            },
            _ => None,
        }
    }

    /// Evaluate with `ctx` as `$this` — a node, or a scalar item when inside
    /// `select`/`where` over scalars (the Luhn invariants iterate digits).
    fn eval<D: Document>(e: &Expr, ctx: &Item<D>) -> Result<Vec<Item<D>>, Unsupported> {
        Ok(match e {
            Expr::This => vec![ctx.clone()],
            Expr::Lit(Lit::Str(s)) => vec![Item::Str(s.clone())],
            Expr::Lit(Lit::Int(n)) => vec![Item::Int(*n)],
            Expr::Lit(Lit::Bool(b)) => vec![Item::Bool(*b)],
            Expr::Nav(base, name) => {
                let mut out = Vec::new();
                for it in eval(base, ctx)? {
                    let Item::Node(d) = it else {
                        return Err(Unsupported);
                    };
                    let mut found = d.instances(&format!("$this.{name}"));
                    if found.is_empty() {
                        // Choice type: `value` reaches `valueQuantity`.
                        found = d.instances(&format!("$this.{name}[x]"));
                    }
                    out.extend(found.into_iter().map(Item::Node));
                }
                out
            }
            Expr::Call(base, name, args) => {
                let coll = eval(base, ctx)?;
                match (name.as_str(), args.len()) {
                    ("exists", 0) => vec![Item::Bool(!coll.is_empty())],
                    ("exists", 1) => vec![Item::Bool(!filter(&coll, &args[0])?.is_empty())],
                    ("empty", 0) => vec![Item::Bool(coll.is_empty())],
                    ("not", 0) => match truth(&coll) {
                        Some(b) => vec![Item::Bool(!b)],
                        None => vec![],
                    },
                    ("count", 0) => vec![Item::Int(coll.len() as i64)],
                    ("first", 0) => coll.into_iter().take(1).collect(),
                    ("hasValue", 0) => vec![Item::Bool(coll.iter().any(|it| scalar_of(it).is_some()))],
                    ("where", 1) => filter(&coll, &args[0])?,
                    ("last", 0) => coll.into_iter().last().into_iter().collect(),
                    ("select", 1) => {
                        let mut out = Vec::new();
                        for it in &coll {
                            out.extend(eval(&args[0], it)?);
                        }
                        out
                    }
                    ("all", 1) => {
                        let mut all = true;
                        for it in &coll {
                            all &= truth(&eval(&args[0], it)?) == Some(true);
                        }
                        vec![Item::Bool(all)]
                    }
                    // `extension(url)` — the extensions of each node with that URL.
                    ("extension", 1) => {
                        let url = match single(&eval(&args[0], ctx)?)? {
                            Some(u) => text_of(u)?,
                            None => return Ok(vec![]),
                        };
                        let mut out = Vec::new();
                        for it in &coll {
                            let Item::Node(d) = it else {
                                return Err(Unsupported);
                            };
                            for ext in d.instances("$this.extension") {
                                let matches = ext.instances("$this.url").iter().any(|u| u.as_scalar() == Some(url.as_str()));
                                if matches {
                                    out.push(Item::Node(ext));
                                }
                            }
                        }
                        out
                    }
                    // `iif(cond, then[, else])` — the input item is `$this` for all three.
                    ("iif", 2 | 3) => {
                        let Some(this) = single(&coll)? else {
                            return Ok(vec![]);
                        };
                        match truth(&eval(&args[0], this)?) {
                            Some(true) => eval(&args[1], this)?,
                            _ if args.len() == 3 => eval(&args[2], this)?,
                            _ => vec![],
                        }
                    }
                    ("toInteger", 0) => match single(&coll)? {
                        Some(it) => to_integer(it).map(Item::Int).into_iter().collect(),
                        None => vec![],
                    },
                    ("toString", 0) => match single(&coll)? {
                        Some(it) => vec![Item::Str(scalar_of(it).ok_or(Unsupported)?)],
                        None => vec![],
                    },
                    ("length", 0) => match single(&coll)? {
                        Some(it) => vec![Item::Int(text_of(it)?.chars().count() as i64)],
                        None => vec![],
                    },
                    ("startsWith", 1) => match single(&coll)? {
                        Some(it) => {
                            let s = text_of(it)?;
                            match single(&eval(&args[0], ctx)?)? {
                                Some(p) => vec![Item::Bool(s.starts_with(text_of(p)?.as_str()))],
                                None => vec![],
                            }
                        }
                        None => vec![],
                    },
                    ("substring", 1 | 2) => match single(&coll)? {
                        Some(it) => {
                            let s = text_of(it)?;
                            let Some(start) = single(&eval(&args[0], ctx)?)?.map(int_of).transpose()? else {
                                return Ok(vec![]);
                            };
                            let len = match args.get(1) {
                                Some(a) => match single(&eval(a, ctx)?)? {
                                    Some(n) => Some(int_of(n)?),
                                    None => return Ok(vec![]),
                                },
                                None => None,
                            };
                            substring(&s, start, len).map(Item::Str).into_iter().collect()
                        }
                        None => vec![],
                    },
                    _ => return Err(Unsupported),
                }
            }
            Expr::Bin(l, op @ (Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod), r) => {
                let (a, b) = (eval(l, ctx)?, eval(r, ctx)?);
                let (Some(a), Some(b)) = (single(&a)?, single(&b)?) else {
                    return Ok(vec![]);
                };
                match (a, b, op) {
                    (Item::Int(x), Item::Int(y), _) => arith(*op, *x, *y)?.map(Item::Int).into_iter().collect(),
                    // `+` on strings concatenates.
                    (Item::Str(_) | Item::Node(_), Item::Str(_) | Item::Node(_), Op::Add) => {
                        vec![Item::Str(text_of(a)? + &text_of(b)?)]
                    }
                    _ => return Err(Unsupported),
                }
            }
            Expr::Bin(l, op @ (Op::Lt | Op::Le | Op::Gt | Op::Ge), r) => {
                let (a, b) = (eval(l, ctx)?, eval(r, ctx)?);
                let (Some(a), Some(b)) = (single(&a)?, single(&b)?) else {
                    return Ok(vec![]);
                };
                vec![Item::Bool(compare(*op, a, b)?)]
            }
            Expr::Bin(l, op @ (Op::Eq | Op::Ne), r) => {
                let (a, b) = (eval(l, ctx)?, eval(r, ctx)?);
                if a.is_empty() || b.is_empty() {
                    return Ok(vec![]);
                }
                if a.len() != 1 || b.len() != 1 {
                    return Err(Unsupported);
                }
                match (scalar_of(&a[0]), scalar_of(&b[0])) {
                    (Some(x), Some(y)) => vec![Item::Bool(if *op == Op::Eq { x == y } else { x != y })],
                    _ => return Err(Unsupported),
                }
            }
            Expr::Bin(l, op, r) => {
                let a = truth(&eval(l, ctx)?);
                let b = truth(&eval(r, ctx)?);
                match logic(*op, a, b) {
                    Some(v) => vec![Item::Bool(v)],
                    None => vec![],
                }
            }
        })
    }

    /// The outcome of evaluating an invariant.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Verdict {
        Pass,
        Fail,
        Unsupported,
    }

    /// Evaluate an invariant with `ctx` as `$this`. Empty is unknown and passes;
    /// only a definite `false` fails.
    pub fn check<D: Document>(expr: &Expr, ctx: &D) -> Verdict {
        match eval(expr, &Item::Node(ctx.clone())) {
            Err(Unsupported) => Verdict::Unsupported,
            Ok(coll) => {
                if truth(&coll) == Some(false) {
                    Verdict::Fail
                } else {
                    Verdict::Pass
                }
            }
        }
    }
}

pub mod terminology {
    pub trait TerminologyProvider {
        fn validate_code(&self, value_set_url: &str, system: &str, code: &str) -> bool;
    }
}

pub mod lower {
    use alloc::string::String;

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
    use alloc::string::String;

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
    use super::{probe, validate_bytes};

    fn probe_str(s: &str) -> String {
        probe(s.as_bytes())
    }

    const PATIENT_ID: &str = r#""identifier":[{"system":"http://hospital.example.org/mrn","value":"1"}]"#;
    const PATIENT_OK: &str = r#"{"resourceType":"Patient","identifier":[{"system":"http://hospital.example.org/mrn","value":"1"}],"name":[{"family":"Doe","use":"official"}],"gender":"male","birthDate":"1970-01-01"}"#;

    // --- Observation, default checks (no declared profile) ---

    #[test]
    fn valid_observation_default() {
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

    // --- Patient (generated au-core-patient) ---

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
        assert!(r.contains("\"code\":\"code-invalid\""), "{r}");
    }

    #[test]
    fn primitive_with_only_extension_sibling_counts_as_present() {
        // FHIR JSON: a primitive carrying only extensions is `_birthDate` with no `birthDate`.
        let r = probe_str(&PATIENT_OK.replace(
            "\"birthDate\":\"1970-01-01\"",
            "\"_birthDate\":{\"extension\":[{\"url\":\"http://hl7.org/fhir/StructureDefinition/data-absent-reason\",\"valueCode\":\"unknown\"}]}",
        ));
        assert!(r.contains("\"valid\":true"), "{r}");
        assert!(!r.contains("birthDate is required"), "{r}");
    }

    #[test]
    fn nested_binding_on_name_use() {
        let r = probe_str(&PATIENT_OK.replace("\"official\"", "\"bogus\""));
        assert!(r.contains("Patient.name.use"), "{r}");
    }

    #[test]
    fn nested_min_fires_per_parent_instance() {
        let r = probe_str(&PATIENT_OK.replace(
            "\"birthDate\"",
            "\"communication\":[{\"language\":{\"text\":\"en\"}},{\"preferred\":true}],\"birthDate\"",
        ));
        assert_eq!(r.matches("Patient.communication.language is required").count(), 1, "{r}");
    }

    #[test]
    fn nested_min_does_not_fire_when_parent_absent() {
        let r = probe_str(PATIENT_OK);
        assert!(!r.contains("communication.language"), "{r}");
    }

    #[test]
    fn nested_max_exceeded_per_parent() {
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
        assert!(probe_str(&bad).contains("Patient.link.type"));
    }

    #[test]
    fn identifier_slice_inherits_au_ihi_profile() {
        // An NI-typed identifier is the `ihi` slice, whose profile (au-ihi)
        // fixes the system — reached through the slice's profile, recursively,
        // with the issue relabelled onto the slice.
        let ni = |system: &str| {
            PATIENT_OK.replace(
                PATIENT_ID,
                &format!(
                    r#""identifier":[{{"type":{{"coding":[{{"system":"http://terminology.hl7.org/CodeSystem/v2-0203","code":"NI"}}]}},"system":"{system}","value":"8003608833357361"}}]"#
                ),
            )
        };
        let wrong = probe_str(&ni("urn:oid:1.2.36.146.595.217.0.1"));
        assert!(wrong.contains("Patient.identifier:ihi.system"), "{wrong}");
        assert!(wrong.contains("\"valid\":false"), "{wrong}");
        let right = probe_str(&ni("http://ns.electronichealth.net.au/id/hi/ihi/1.0"));
        assert!(right.contains("\"valid\":true"), "{right}");
    }

    // --- Observation: generated au-core-heartrate, selected via meta.profile ---

    const HEARTRATE_OK: &str = r#"{"resourceType":"Observation","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate"]},"status":"final","category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs"}]}],"code":{"coding":[{"system":"http://loinc.org","code":"8867-4"},{"system":"http://snomed.info/sct","code":"364075005"}]},"subject":{"reference":"Patient/1"},"effectiveDateTime":"2026-09-15T10:00:00Z","valueQuantity":{"value":72,"unit":"beats/minute","system":"http://unitsofmeasure.org","code":"/min"}}"#;

    #[test]
    fn heartrate_valid_via_declared_profile() {
        let r = probe_str(HEARTRATE_OK);
        assert!(r.contains("\"valid\":true"), "{r}");
        assert!(r.contains("Heart"), "{r}");
    }

    #[test]
    fn heartrate_missing_snomed_slice_is_error() {
        let r = probe_str(&HEARTRATE_OK.replace(r#",{"system":"http://snomed.info/sct","code":"364075005"}"#, ""));
        assert!(r.contains("Observation.code.coding:snomedHeartRateCode is required"), "{r}");
    }

    #[test]
    fn heartrate_wrong_ucum_code_fails_slice_fixed() {
        let r = probe_str(&HEARTRATE_OK.replace("\"code\":\"/min\"", "\"code\":\"min\""));
        assert!(r.contains("Observation.value[x]:valueQuantity.code must be '/min'"), "{r}");
    }

    #[test]
    fn heartrate_closed_value_slicing_rejects_other_type() {
        let r = probe_str(&HEARTRATE_OK.replace(
            r#""valueQuantity":{"value":72,"unit":"beats/minute","system":"http://unitsofmeasure.org","code":"/min"}"#,
            r#""valueString":"72""#,
        ));
        assert!(r.contains("matching no slice"), "{r}");
    }

    // --- Observation: generated au-core-bloodpressure — component slices whose
    //     discriminator values live inside nested slices ---

    const BP_OK: &str = r#"{"resourceType":"Observation","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-bloodpressure"]},"status":"final","category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs"}]}],"code":{"coding":[{"system":"http://loinc.org","code":"85354-9"},{"system":"http://snomed.info/sct","code":"75367002"}]},"subject":{"reference":"Patient/1"},"effectiveDateTime":"2026-09-15T10:00:00Z","component":[{"code":{"coding":[{"system":"http://loinc.org","code":"8480-6"},{"system":"http://snomed.info/sct","code":"271649006"}]},"valueQuantity":{"value":120,"unit":"mmHg","system":"http://unitsofmeasure.org","code":"mm[Hg]"}},{"code":{"coding":[{"system":"http://loinc.org","code":"8462-4"},{"system":"http://snomed.info/sct","code":"271650006"}]},"valueQuantity":{"value":80,"unit":"mmHg","system":"http://unitsofmeasure.org","code":"mm[Hg]"}}]}"#;

    #[test]
    fn bloodpressure_valid_with_component_slices() {
        let r = probe_str(BP_OK);
        assert!(r.contains("\"valid\":true"), "{r}");
        assert!(r.contains("Blood Pressure"), "{r}");
    }

    #[test]
    fn bloodpressure_wrong_diastolic_code_fails_component_slice() {
        // Two components remain, so `component min 2` still holds — only the
        // slice discriminator (resolved through a nested slice) can catch it.
        let r = probe_str(&BP_OK.replace("\"code\":\"8462-4\"", "\"code\":\"8462-9\""));
        assert!(r.contains("Observation.component:DiastolicBP is required"), "{r}");
    }

    #[test]
    fn bloodpressure_component_wrong_unit_fails_slice_fixed() {
        let r = probe_str(&BP_OK.replacen("\"code\":\"mm[Hg]\"", "\"code\":\"mmHg\"", 1));
        assert!(r.contains("Observation.component:SystolicBP.value[x].code must be 'mm[Hg]'"), "{r}");
    }

    // --- FHIRPath core + invariants ---

    #[test]
    fn fhirpath_vs2_semantics() {
        use super::document::Parsed;
        use super::path::{check, parse, Verdict};
        let vs2 = parse("(component.empty() and hasMember.empty()) implies (value.exists() or dataAbsentReason.exists())").unwrap();
        let has_value = Parsed::parse(br#"{"resourceType":"Observation","valueQuantity":{"value":1}}"#).unwrap();
        assert_eq!(check(&vs2, &has_value.doc()), Verdict::Pass);
        let has_component = Parsed::parse(br#"{"resourceType":"Observation","component":[{"code":{}}]}"#).unwrap();
        assert_eq!(check(&vs2, &has_component.doc()), Verdict::Pass);
        let bare = Parsed::parse(br#"{"resourceType":"Observation","status":"final"}"#).unwrap();
        assert_eq!(check(&vs2, &bare.doc()), Verdict::Fail);
    }

    #[test]
    fn fhirpath_where_and_equality() {
        use super::document::Parsed;
        use super::path::{check, parse, Verdict};
        let e = parse("coding.where(system = 'http://loinc.org').exists()").unwrap();
        let ok = Parsed::parse(br#"{"coding":[{"system":"http://snomed.info/sct"},{"system":"http://loinc.org"}]}"#).unwrap();
        assert_eq!(check(&e, &ok.doc()), Verdict::Pass);
        let no = Parsed::parse(br#"{"coding":[{"system":"http://snomed.info/sct"}]}"#).unwrap();
        assert_eq!(check(&e, &no.doc()), Verdict::Fail);
    }

    #[test]
    fn fhirpath_unsupported_is_skipped_not_guessed() {
        use super::path::parse;
        // obs-7: %resource and intersect() are outside the core
        assert!(parse("value.empty() or component.code.where(coding.intersect(%resource.code.coding).exists()).empty()").is_err());
        // type tests, union, decimal division, regex: outside the core
        assert!(parse("$this is dateTime implies $this.toString().length() >= 10").is_err());
        assert!(parse("a | b").is_err());
        assert!(parse("1 / 2").is_err());
        assert!(parse("1.5 < 2").is_err());
        assert!(parse("-x").is_err());
    }

    fn fp(expr: &str, doc: &str) -> super::path::Verdict {
        use super::document::Parsed;
        use super::path::{check, parse};
        let e = parse(expr).unwrap_or_else(|_| panic!("should parse: {expr}"));
        let d = Parsed::parse(doc.as_bytes()).unwrap();
        check(&e, &d.doc())
    }

    #[test]
    fn fhirpath_arithmetic_and_precedence() {
        use super::path::Verdict::{Fail, Pass};
        let d = r#"{"a":"x"}"#;
        assert_eq!(fp("1 + 2 * 3 = 7", d), Pass);
        assert_eq!(fp("(1 + 2) * 3 = 9", d), Pass);
        assert_eq!(fp("7 mod 3 = 1", d), Pass);
        assert_eq!(fp("7 div 2 = 3", d), Pass);
        assert_eq!(fp("10 - 2 - 3 = 5", d), Pass); // left-assoc
        assert_eq!(fp("-3 + 5 = 2", d), Pass);
        assert_eq!(fp("2 < 3 and 3 <= 3 and 4 > 3 and 3 >= 3", d), Pass);
        assert_eq!(fp("3 < 2", d), Fail);
        assert_eq!(fp("1 + 2 > 2 and 2 * 2 = 4", d), Pass); // + binds tighter than >, = looser than <
        assert_eq!(fp("'b' > 'a'", d), Pass);
        assert_eq!(fp("'ab' + 'c' = 'abc'", d), Pass);
        // empty propagates; division by zero is empty; both pass an invariant
        assert_eq!(fp("(nope + 1) = 1", d), Pass);
        assert_eq!(fp("(1 div 0).empty()", d), Pass);
        assert_eq!(fp("(1 mod 0).empty()", d), Pass);
    }

    #[test]
    fn fhirpath_string_and_conversion_functions() {
        use super::path::Verdict::{Fail, Pass, Unsupported};
        let d = r#"{"value":"800360","n":"42","b":true,"list":["a","bb"]}"#;
        assert_eq!(fp("value.length() = 6", d), Pass);
        assert_eq!(fp("value.substring(0,3) = '800'", d), Pass);
        assert_eq!(fp("value.substring(4) = '60'", d), Pass);
        assert_eq!(fp("value.substring(9).empty()", d), Pass);
        assert_eq!(fp("value.substring(2,-1).empty()", d), Pass);
        assert_eq!(fp("value.startsWith('8003')", d), Pass);
        assert_eq!(fp("value.startsWith('9')", d), Fail);
        assert_eq!(fp("n.toInteger() + 1 = 43", d), Pass);
        assert_eq!(fp("value.toInteger() = 800360", d), Pass);
        assert_eq!(fp("'x'.toInteger().empty()", d), Pass);
        assert_eq!(fp("b.toInteger() = 1", d), Pass);
        assert_eq!(fp("n.toString() = '42'", d), Pass);
        assert_eq!(fp("(42).toString().length() = 2", d), Pass);
        assert_eq!(fp("list.last() = 'bb'", d), Pass);
        // a string function on a non-string, or on a multi-item collection: unsupported
        assert_eq!(fp("(1).length() = 1", d), Unsupported);
        assert_eq!(fp("list.length() = 1", d), Unsupported);
        assert_eq!(fp("list < 'c'", d), Unsupported);
        // an unknown function parses (it is a call) but evaluates as unsupported
        assert_eq!(fp("value.matches('^([0-9]{16})$')", d), Unsupported);
        assert_eq!(fp("value.memberOf('http://x/vs')", d), Unsupported);
    }

    #[test]
    fn fhirpath_iif_select_all_extension() {
        use super::path::Verdict::{Fail, Pass};
        let d = r#"{"digits":["1","7"],"name":[{"family":"A"},{"family":"B"}],"extension":[{"url":"http://x/dar","valueCode":"unknown"},{"url":"http://x/other"}]}"#;
        assert_eq!(fp("digits.select($this.toInteger()).select(iif($this < 5, $this * 2, $this * 2 - 9)).first() = 2", d), Pass);
        assert_eq!(fp("digits.select($this.toInteger()).select(iif($this < 5, $this * 2, $this * 2 - 9)).last() = 5", d), Pass);
        assert_eq!(fp("iif(true, 'y', 'n') = 'y'", d), Pass);
        assert_eq!(fp("iif(nope.exists(), 'y', 'n') = 'n'", d), Pass);
        assert_eq!(fp("iif(false, 'y').empty()", d), Pass);
        assert_eq!(fp("name.all(family.exists())", d), Pass);
        assert_eq!(fp("name.all(given.exists())", d), Fail);
        assert_eq!(fp("nope.all(given.exists())", d), Pass);
        assert_eq!(fp("extension('http://x/dar').exists()", d), Pass);
        assert_eq!(fp("extension('http://x/dar').value.exists()", d), Pass);
        assert_eq!(fp("extension('http://x/none').exists()", d), Fail);
        assert_eq!(fp("name.count() + digits.count() > 3", d), Pass);
        // an element named like a keyword is written delimited
        assert_eq!(fp("text.`div`.exists()", r#"{"text":{"div":"<div/>"}}"#), Pass);
    }

    #[test]
    fn ihi_luhn_invariant_fires_through_the_slice_profile() {
        let ihi = |value: &str| {
            PATIENT_OK.replace(
                PATIENT_ID,
                &format!(
                    r#""identifier":[{{"type":{{"coding":[{{"system":"http://terminology.hl7.org/CodeSystem/v2-0203","code":"NI"}}]}},"system":"http://ns.electronichealth.net.au/id/hi/ihi/1.0","value":"{value}"}}]"#
                ),
            )
        };
        let good = probe_str(&ihi("8003608833357361"));
        assert!(good.contains("\"valid\":true"), "{good}");
        let bad = probe_str(&ihi("8003608833357362"));
        assert!(bad.contains("Luhn"), "{bad}");
        assert!(bad.contains("\"valid\":false"), "{bad}");
    }

    #[test]
    fn heartrate_missing_value_and_dar_fails_vs2() {
        let r = probe_str(&HEARTRATE_OK.replace(
            r#","valueQuantity":{"value":72,"unit":"beats/minute","system":"http://unitsofmeasure.org","code":"/min"}"#,
            "",
        ));
        assert!(r.contains("vs-2"), "{r}");
        assert!(r.contains("\"valid\":false"), "{r}");
    }

    // --- wire: OperationOutcome ---

    #[test]
    fn operation_outcome_is_spec_shaped() {
        let v = validate_bytes(br#"{"resourceType":"Patient","active":true}"#);
        let o = v.to_operation_outcome();
        assert!(o.starts_with("{\"resourceType\":\"OperationOutcome\",\"issue\":["), "{o}");
        assert!(o.contains("\"severity\":\"error\",\"code\":\"required\""), "{o}");
        // A missing element is located at its parent, as the reference validator does.
        assert!(o.contains("\"expression\":[\"Patient\"]"), "{o}");
    }

    #[test]
    fn operation_outcome_success_carries_one_informational_issue() {
        // With narrative, so dom-6 (a warning) is quiet and the outcome is clean.
        let with_text = PATIENT_OK.replace(
            "\"gender\"",
            "\"text\":{\"status\":\"generated\",\"div\":\"<div xmlns=\\\"http://www.w3.org/1999/xhtml\\\"/>\"},\"gender\"",
        );
        let v = validate_bytes(with_text.as_bytes());
        assert!(v.is_valid());
        let o = v.to_operation_outcome();
        assert!(o.contains("\"severity\":\"information\",\"code\":\"informational\""), "{o}");
    }

    #[test]
    fn parse_failure_is_fatal() {
        let v = validate_bytes(b"not json");
        assert!(!v.is_valid());
        assert!(v.to_operation_outcome().contains("\"severity\":\"fatal\",\"code\":\"structure\""));
    }

    #[test]
    fn wire_expression_locates_missing_element_at_parent() {
        use super::validate::IssueType;
        use super::wire::expression;
        assert_eq!(expression("Patient.gender", IssueType::Required), "Patient");
        assert_eq!(expression("Observation.code.coding:snomedHeartRateCode", IssueType::Required), "Observation.code");
    }

    #[test]
    fn wire_expression_rewrites_slice_labels_to_fhirpath() {
        use super::validate::IssueType;
        use super::wire::expression;
        assert_eq!(
            expression("Observation.value[x]:valueQuantity.code", IssueType::Value),
            "Observation.value.ofType(Quantity).code"
        );
        assert_eq!(
            expression("Observation.category:VSCat.coding.system", IssueType::Value),
            "Observation.category.coding.system"
        );
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
