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

/// Typed traversal: a JSON node learns its FHIR type from the table as it is
/// reached — root `resourceType`, then element by element (choice keys,
/// backbone paths, content references, contained resources). What lets
/// `descendants().as(canonical)`, `value.ofType(Quantity)` and `$this is
/// dateTime` work on arbitrary nodes without the document carrying types.
/// Under a resource type the table doesn't cover, a node has no type and
/// type-dependent FHIRPath is skipped, never guessed.
pub mod typing {
    use crate::typemap::{TABLE, TYPES};

    /// A resource or datatype name the table knows.
    pub fn known_type(name: &str) -> Option<&'static str> {
        TYPES.binary_search(&name).ok().map(|i| TYPES[i])
    }

    /// `p.cmp(&format!("{ctx}.{key}"))` without building the string.
    fn cmp_joined(p: &str, ctx: &str, key: &str) -> core::cmp::Ordering {
        use core::cmp::Ordering::*;
        let pb = p.as_bytes();
        let mut i = 0;
        for part in [ctx.as_bytes(), b".", key.as_bytes()] {
            for &x in part {
                match pb.get(i) {
                    None => return Less,
                    Some(&y) if y != x => return y.cmp(&x),
                    _ => {}
                }
                i += 1;
            }
        }
        if pb.len() > i {
            Greater
        } else {
            Equal
        }
    }

    fn lookup2(ctx: &str, key: &str) -> Option<&'static (&'static str, &'static str)> {
        TABLE.binary_search_by(|(p, _)| cmp_joined(p, ctx, key)).ok().map(|i| &TABLE[i])
    }

    /// A child's typing: `ctx` is what its own children resolve against (a
    /// type name, or a backbone path like `Observation.component`); `ty` is
    /// its FHIR type name for `is` / `as` / `ofType`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Child {
        pub ctx: &'static str,
        pub ty: &'static str,
    }

    /// The typing of element `key` under `ctx`. `Child { ty: "Resource" }`
    /// means "whatever the child's own `resourceType` says" (contained,
    /// Bundle.entry.resource) — the caller resolves it from the node.
    pub fn child(ctx: &'static str, key: &str) -> Option<Child> {
        match key {
            "id" => {
                let ty = if ctx.contains('.') || crate::typemap::RESOURCES.binary_search(&ctx).is_err() {
                    "string"
                } else {
                    "id"
                };
                return Some(Child { ctx: ty, ty });
            }
            "extension" | "modifierExtension" => return Some(Child { ctx: "Extension", ty: "Extension" }),
            k if k.starts_with('_') => return Some(Child { ctx: "Element", ty: "Element" }),
            _ => {}
        }
        let (path, ty) = match lookup2(ctx, key) {
            Some((p, t)) => (*p, *t),
            None => choice(ctx, key)?,
        };
        Some(resolve(path, ty))
    }

    /// `valueQuantity` under `Observation` → the `Observation.value[x]` entry
    /// with `Quantity` among its types.
    fn choice(ctx: &str, key: &str) -> Option<(&'static str, &'static str)> {
        let start = TABLE.partition_point(|(p, _)| cmp_joined(p, ctx, "") == core::cmp::Ordering::Less);
        for (path, tys) in &TABLE[start..] {
            let Some(rest) = path.strip_prefix(ctx).and_then(|r| r.strip_prefix('.')) else { break };
            let Some(stem) = rest.strip_suffix("[x]") else { continue };
            if stem.contains('.') {
                continue;
            }
            let Some(suffix) = key.strip_prefix(stem) else { continue };
            if suffix.is_empty() {
                continue;
            }
            for t in tys.split('|') {
                let (first, tail) = t.split_at(1);
                if suffix.len() == t.len() && suffix[..1].eq_ignore_ascii_case(first) && &suffix[1..] == tail {
                    return Some((path, t));
                }
            }
        }
        None
    }

    fn resolve(path: &'static str, ty: &'static str) -> Child {
        if let Some(cref) = ty.strip_prefix('#') {
            return Child { ctx: cref, ty: "BackboneElement" };
        }
        // a choice list that was not narrowed (path given with `[x]`): untypable
        let ty = ty.split('|').next().unwrap_or(ty);
        match ty {
            "BackboneElement" | "Element" => Child { ctx: path, ty },
            "Resource" | "DomainResource" => Child { ctx: "Resource", ty: "Resource" },
            t => Child { ctx: known_type(t).unwrap_or(t), ty: t },
        }
    }
}

/// The ergonomic result: what was validated, against which profile, and every
/// finding. Never a thrown error — validation state is data.
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

/// The pods' JSON report — the ergonomic shape, stable for the FFI.
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
    ///
    /// The ergonomic `Issue.path` keeps the precise, labelled location.
    pub fn expression(path: &str, code: IssueType) -> String {
        let located =
            if code == IssueType::Required { path.rsplit_once('.').map_or(path, |(parent, _)| parent) } else { path };
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
        if v.profiles.len() > 1 {
            out.push_str(",\"profiles\":[");
            for (i, p) in v.profiles.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('"');
                escape(&mut out, p);
                out.push('"');
            }
            out.push(']');
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
        let mut start = 0;
        for (i, b) in s.bytes().enumerate() {
            let rep = match b {
                b'"' => "\\\"",
                b'\\' => "\\\\",
                b if b < 0x20 => " ",
                _ => continue,
            };
            out.push_str(&s[start..i]);
            out.push_str(rep);
            start = i + 1;
        }
        out.push_str(&s[start..]);
    }
}

/// JSON: `parse` builds a borrowed value tree (strings decoded only when they
/// carry escapes) — the correctness-first adapter behind the `Document` port.
pub mod json {
    use alloc::{borrow::Cow, string::String, vec::Vec};

    #[derive(Debug)]
    pub struct ParseError {
        pub at: usize,
        pub msg: &'static str,
    }

    /// A borrowed JSON value tree. Numbers point into the input; strings do
    /// too unless they carry escapes, in which case they are decoded into an
    /// owned copy (`Cow`).
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value<'a> {
        Null,
        Bool(bool),
        Number(&'a str),
        Str(Cow<'a, str>),
        Array(Vec<Value<'a>>),
        Object(Vec<(Cow<'a, str>, Value<'a>)>),
    }

    /// Parse one JSON document: a value with nothing but whitespace after it.
    pub fn parse(input: &[u8]) -> Result<Value<'_>, ParseError> {
        let mut p = Parser { b: input, i: 0, depth: 0 };
        let v = p.parse_value()?;
        p.skip_ws();
        if p.i != input.len() {
            return Err(ParseError { at: p.i, msg: "trailing characters after the document" });
        }
        Ok(v)
    }

    /// Resolve a dotted path against a node. The first segment is the root's
    /// name (the resource type, or `$this` for a sub-document) and is skipped.
    /// Arrays expand; `[x]` matches a choice suffix (`value[x]` → `valueQuantity`).
    pub fn resolve<'v, 'a>(root: &'v Value<'a>, path: &str) -> Vec<&'v Value<'a>> {
        let mut out = Vec::new();
        resolve_each(root, path, &mut |v| out.push(v));
        out
    }

    /// Visit every node `path` resolves to, depth-first, with no intermediate
    /// collections. First segment (the root's name) is skipped.
    pub fn resolve_each<'v, 'a>(root: &'v Value<'a>, path: &str, f: &mut dyn FnMut(&'v Value<'a>)) {
        match path.split_once('.') {
            None => f(root),
            Some((_, rest)) => walk(root, rest, f),
        }
    }

    fn walk<'v, 'a>(node: &'v Value<'a>, rest: &str, f: &mut dyn FnMut(&'v Value<'a>)) {
        match rest.split_once('.') {
            None => for_each_child(node, rest, f),
            Some((seg, tail)) => for_each_child(node, seg, &mut |c| walk(c, tail, f)),
        }
    }

    /// The children of `node` at `seg` (arrays expanded), in document order.
    pub fn for_each_child<'v, 'a>(node: &'v Value<'a>, seg: &str, f: &mut dyn FnMut(&'v Value<'a>)) {
        match node {
            Value::Object(members) => {
                let mut found = false;
                match seg.strip_suffix("[x]") {
                    Some(prefix) => {
                        for (k, v) in members {
                            if k.len() > prefix.len()
                                && k.starts_with(prefix)
                                && k.as_bytes()[prefix.len()].is_ascii_uppercase()
                            {
                                expand(v, f);
                            }
                        }
                    }
                    None => {
                        for (k, v) in members {
                            if k.as_ref() == seg {
                                found = true;
                                expand(v, f);
                            }
                        }
                        if !found {
                            // the `_name` form alone: its array's `null`s are placeholders, not elements
                            for (k, v) in members {
                                if k.strip_prefix('_') == Some(seg) {
                                    expand(v, &mut |x| {
                                        if !matches!(x, Value::Null) {
                                            f(x)
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    for_each_child(item, seg, f);
                }
            }
            _ => {}
        }
    }

    fn expand<'v, 'a>(v: &'v Value<'a>, f: &mut dyn FnMut(&'v Value<'a>)) {
        match v {
            Value::Array(items) => {
                for x in items {
                    f(x)
                }
            }
            other => f(other),
        }
    }

    /// Push the children of `node` matching `seg` (expanding arrays).
    pub fn children<'v, 'a>(node: &'v Value<'a>, seg: &str, out: &mut Vec<&'v Value<'a>>) {
        for_each_child(node, seg, &mut |v| out.push(v));
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

        /// A JSON string. Borrowed when it has no escapes; otherwise decoded
        /// (`\"`, `\\`, `\/`, `\b \f \n \r \t`, `\uXXXX` with surrogate
        /// pairs) into an owned copy. A lone surrogate becomes U+FFFD.
        fn string(&mut self) -> Result<Cow<'a, str>, ParseError> {
            self.expect(b'"')?;
            let b = self.b;
            let start = self.i;
            let mut owned: Option<String> = None;
            let mut run = start;
            loop {
                match b.get(self.i).copied() {
                    None => return Err(ParseError { at: self.i, msg: "unterminated string" }),
                    Some(b'"') => {
                        let tail = core::str::from_utf8(&b[run..self.i])
                            .map_err(|_| ParseError { at: run, msg: "invalid utf-8" })?;
                        self.i += 1;
                        return Ok(match owned {
                            None => Cow::Borrowed(tail),
                            Some(mut o) => {
                                o.push_str(tail);
                                Cow::Owned(o)
                            }
                        });
                    }
                    Some(b'\\') => {
                        let o = owned.get_or_insert_with(String::new);
                        o.push_str(
                            core::str::from_utf8(&b[run..self.i])
                                .map_err(|_| ParseError { at: run, msg: "invalid utf-8" })?,
                        );
                        let at = self.i;
                        let esc = b.get(self.i + 1).copied().ok_or(ParseError { at, msg: "unterminated string" })?;
                        self.i += 2;
                        match esc {
                            b'"' => o.push('"'),
                            b'\\' => o.push('\\'),
                            b'/' => o.push('/'),
                            b'b' => o.push('\u{8}'),
                            b'f' => o.push('\u{c}'),
                            b'n' => o.push('\n'),
                            b'r' => o.push('\r'),
                            b't' => o.push('\t'),
                            b'u' => {
                                let hi = self.hex4().ok_or(ParseError { at, msg: "bad \\u escape" })?;
                                let c = if (0xD800..0xDC00).contains(&hi) {
                                    // a high surrogate: needs a \uDC00–\uDFFF partner
                                    if b.get(self.i..self.i + 2) == Some(b"\\u") {
                                        let save = self.i;
                                        self.i += 2;
                                        match self.hex4() {
                                            Some(lo) if (0xDC00..0xE000).contains(&lo) => {
                                                char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                                            }
                                            _ => {
                                                self.i = save;
                                                None
                                            }
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    char::from_u32(hi)
                                };
                                o.push(c.unwrap_or('\u{FFFD}'));
                            }
                            _ => return Err(ParseError { at, msg: "bad escape" }),
                        }
                        run = self.i;
                    }
                    Some(_) => self.i += 1,
                }
            }
        }

        fn hex4(&mut self) -> Option<u32> {
            let s = self.b.get(self.i..self.i + 4)?;
            let mut v = 0u32;
            for &c in s {
                v = v * 16 + (c as char).to_digit(16)?;
            }
            self.i += 4;
            Some(v)
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
            let s =
                core::str::from_utf8(&b[start..self.i]).map_err(|_| ParseError { at: start, msg: "invalid utf-8" })?;
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
                Some(b'"') => Ok(Value::Str(self.string()?)),
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
                let key = self.string()?;
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
        /// The direct children under one key (arrays expanded) — `instances("$this.key")`
        /// without building or re-splitting a path.
        fn child_instances(&self, key: &str) -> Vec<Self> {
            let mut p = alloc::string::String::from("$this.");
            p.push_str(key);
            self.instances(&p)
        }
        /// This node's scalar value as text, if it is a scalar.
        fn as_scalar(&self) -> Option<&str>;
        /// This node's value if it is a JSON number (its literal text).
        fn as_number(&self) -> Option<&str>;
        /// This node's value if it is a JSON boolean.
        fn as_bool(&self) -> Option<bool>;
        /// Structural equality with another node (FHIRPath `=` on complex
        /// types, `intersect`).
        fn same(&self, other: &Self) -> bool;
        /// Every direct child node (array members expanded) — `descendants()`.
        fn all_children(&self) -> Vec<Self>;
        /// This node's FHIR type name, when the adapter can tell.
        fn fhir_type(&self) -> Option<&'static str>;
        /// The children of a choice element (`value` → `valueQuantity` /
        /// `valueString` / …), each typed by its concrete key.
        fn choice_children(&self, stem: &str) -> Vec<Self>;
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
            TreeDoc { root: &self.value, ctx: resource_ctx(&self.value) }
        }
        /// A top-level member by key (the JSON type tag lives here).
        pub fn top_member(&self, key: &str) -> Option<&Value<'a>> {
            match &self.value {
                Value::Object(kvs) => kvs.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v),
                _ => None,
            }
        }
    }

    /// The typing context of a resource node: its `resourceType`, if the
    /// element-type table knows it.
    fn resource_ctx(v: &Value<'_>) -> Option<&'static str> {
        match v {
            Value::Object(kvs) => kvs.iter().find(|(k, _)| k.as_ref() == "resourceType").and_then(|(_, v)| match v {
                Value::Str(s) => crate::typing::known_type(s),
                _ => None,
            }),
            _ => None,
        }
    }

    /// The value-tree adapter: a view rooted at any node, carrying the typing
    /// context it was reached through (`None` when unknown).
    #[derive(Clone, Copy)]
    pub struct TreeDoc<'v, 'a> {
        root: &'v Value<'a>,
        ctx: Option<&'static str>,
    }

    impl<'v, 'a> TreeDoc<'v, 'a> {
        /// The typing context of a child reached by `key`; a `Resource` child
        /// takes its own `resourceType`.
        fn child_ctx(ctx: Option<&'static str>, key: &str, child: &'v Value<'a>) -> Option<&'static str> {
            let ch = crate::typing::child(ctx?, key)?;
            if ch.ty == "Resource" {
                resource_ctx(child)
            } else {
                Some(ch.ctx)
            }
        }
    }

    impl<'v, 'a> Document for TreeDoc<'v, 'a> {
        fn child_counts(&self, path: &str) -> Vec<usize> {
            let Some((parent, last)) = path.rsplit_once('.') else {
                return Vec::new();
            };
            let mut out = Vec::new();
            json::resolve_each(self.root, parent, &mut |p| {
                let mut n = 0usize;
                json::for_each_child(p, last, &mut |_| n += 1);
                out.push(n);
            });
            out
        }

        fn scalars(&self, path: &str) -> Vec<&str> {
            let mut out = Vec::new();
            json::resolve_each(self.root, path, &mut |v| {
                if let Value::Str(s) = v {
                    out.push(s.as_ref())
                }
            });
            out
        }

        fn instances(&self, path: &str) -> Vec<Self> {
            // Type the result by folding the path's segments; a `[x]` segment
            // or an unknown element leaves the result untyped.
            let mut ctx = self.ctx;
            let mut first = true;
            for seg in path.split('.') {
                let skip = seg == "$this" || (first && Some(seg) == self.ctx);
                first = false;
                if skip {
                    continue;
                }
                ctx = match ctx {
                    Some(c) if !seg.ends_with("[x]") => crate::typing::child(c, seg).map(|ch| ch.ctx),
                    _ => None,
                };
            }
            let mut out = Vec::new();
            json::resolve_each(self.root, path, &mut |n| {
                out.push(TreeDoc { root: n, ctx: if ctx == Some("Resource") { resource_ctx(n) } else { ctx } })
            });
            out
        }

        fn child_instances(&self, key: &str) -> Vec<Self> {
            let ctx = match self.ctx {
                Some(c) if !key.ends_with("[x]") => crate::typing::child(c, key).map(|ch| ch.ctx),
                _ => None,
            };
            let mut out = Vec::new();
            json::for_each_child(self.root, key, &mut |n| {
                out.push(TreeDoc { root: n, ctx: if ctx == Some("Resource") { resource_ctx(n) } else { ctx } })
            });
            out
        }

        fn as_scalar(&self) -> Option<&str> {
            match self.root {
                Value::Str(s) => Some(s.as_ref()),
                Value::Number(s) => Some(*s),
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

        fn as_number(&self) -> Option<&str> {
            match self.root {
                Value::Number(s) => Some(*s),
                _ => None,
            }
        }

        fn same(&self, other: &Self) -> bool {
            self.root == other.root
        }

        fn all_children(&self) -> Vec<Self> {
            let mut out = Vec::new();
            let ctx = self.ctx;
            let mut push = |key: &str, v: &'v Value<'a>| match v {
                Value::Array(xs) => {
                    out.extend(xs.iter().map(|x| TreeDoc { root: x, ctx: Self::child_ctx(ctx, key, x) }))
                }
                v => out.push(TreeDoc { root: v, ctx: Self::child_ctx(ctx, key, v) }),
            };
            match self.root {
                // `resourceType` is the JSON type tag, not an element
                Value::Object(kvs) => {
                    kvs.iter().filter(|(k, _)| k.as_ref() != "resourceType").for_each(|(k, v)| push(k, v))
                }
                Value::Array(xs) => xs.iter().for_each(|v| push("", v)),
                _ => {}
            }
            out
        }

        fn fhir_type(&self) -> Option<&'static str> {
            self.ctx.map(|c| if c.contains('.') { "BackboneElement" } else { c })
        }

        fn choice_children(&self, stem: &str) -> Vec<Self> {
            let Value::Object(kvs) = self.root else { return Vec::new() };
            let mut out = Vec::new();
            for (k, v) in kvs {
                let Some(rest) = k.strip_prefix(stem) else { continue };
                if !rest.starts_with(|c: char| c.is_ascii_uppercase()) {
                    continue;
                }
                let ctx = Self::child_ctx(self.ctx, k, v);
                match v {
                    Value::Array(xs) => out.extend(xs.iter().map(|x| TreeDoc { root: x, ctx })),
                    v => out.push(TreeDoc { root: v, ctx }),
                }
            }
            out
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
        /// A required-strength ValueSet binding on a coded element.
        pub required_binding: Option<Binding>,
    }

    /// How a binding reads the instance: a `code` is the code itself (the
    /// set's implied system); a `Coding` is system + code; a `CodeableConcept`
    /// is a member if any of its codings is.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BindingKind {
        Code,
        Coding,
        CodeableConcept,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Binding {
        pub value_set: &'static str,
        pub kind: BindingKind,
    }

    /// How an instance of a sliced element is assigned to a slice.
    pub enum Discriminator {
        /// The scalars at a relative path (`$this.system`) must include `value`.
        Value { path: &'static str, value: &'static str },
        /// Some ONE instance at `path` (`$this.type.coding`) must carry every
        /// pair — a `pattern` on a repeating element means "contains an
        /// element matching this", not one match per leaf.
        Pattern { path: &'static str, pairs: &'static [(&'static str, &'static str)] },
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
        /// The element's FHIR type code(s): what `$this is dateTime` resolves
        /// against, and for a choice (`effective[x]`) the concrete keys to
        /// evaluate — each instance knows its own type.
        pub types: &'static [&'static str],
    }

    /// A profile compiled to embeddable rules.
    pub struct CompiledProfile {
        pub url: &'static str,
        pub name: &'static str,
        pub resource_type: &'static str,
        pub elements: &'static [ElementConstraint],
        pub slicings: &'static [Slicing],
        pub invariants: &'static [ProfileInvariant],
        /// Elements the IG flags `mustSupport`. Not a validation rule (no
        /// validator fails on it) — a conforming *client* is expected to
        /// populate them when it has the data, so gaps are reported separately
        /// as information (see [`crate::must_support_gaps`]).
        pub must_support: &'static [&'static str],
    }
}

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

/// The validation engine — the domain service. Pure: a compiled profile and a
/// document in, a report out.
pub mod engine {
    use alloc::vec::Vec;

    use alloc::string::String;

    use crate::compiled::{
        BindingKind, CompiledProfile, Discriminator, ElementConstraint, InvariantSeverity, Max, ProfileInvariant,
        Slice, Slicing,
    };
    use crate::document::Document;
    use crate::path::{self, Verdict};
    use crate::terminology::{Bundled, Membership, TerminologyProvider};
    use crate::validate::{IssueType, Report, Severity};

    /// How many profiles a slice may inherit through, transitively. Bounds the
    /// recursion so a registry cycle can never loop.
    const MAX_INHERIT: usize = 8;

    /// Validate with the bundled terminology only.
    pub fn validate(profile: &CompiledProfile, doc: &impl Document) -> Report {
        validate_with(profile, doc, &Bundled)
    }

    /// Validate with a host terminology provider answering the bindings the
    /// bundled sets can't (see [`crate::terminology::Chain`]).
    pub fn validate_with<D: Document>(profile: &CompiledProfile, doc: &D, tx: &dyn TerminologyProvider) -> Report {
        validate_at(profile, doc, doc, tx, 0, None)
    }

    /// Must-support elements that are absent where their parent is present.
    /// A child under an absent parent is not a gap — the parent is.
    pub fn must_support_gaps<D: Document>(profile: &CompiledProfile, doc: &D) -> Vec<&'static str> {
        profile.must_support.iter().copied().filter(|path| doc.child_counts(path).contains(&0)).collect()
    }

    /// `root` is the resource being validated (`%resource`) — the same node
    /// as `doc` at the top level, and still the resource when `doc` is a
    /// slice instance being checked against an inherited profile.
    pub(crate) fn validate_at<D: Document>(
        profile: &CompiledProfile,
        doc: &D,
        root: &D,
        tx: &dyn TerminologyProvider,
        depth: usize,
        cache: Option<&crate::Validator>,
    ) -> Report {
        let mut r = Report::default();
        for el in profile.elements {
            check_element(el, el.path, doc, tx, &mut r);
        }
        for slicing in profile.slicings {
            check_slicing(slicing, doc, root, tx, &mut r, depth, cache);
        }
        for inv in profile.invariants {
            // Outside the supported FHIRPath core: skipped, never guessed.
            let owned;
            let expr: &path::Expr = match cache {
                Some(v) => match v.expr(inv.expression) {
                    Some(e) => e,
                    None => continue,
                },
                None => {
                    let Ok(e) = path::parse(inv.expression) else { continue };
                    owned = e;
                    &owned
                }
            };
            for (ctx, this_type) in invariant_contexts(inv, doc) {
                if path::check_with(expr, &ctx, this_type, Some(root)) == Verdict::Fail {
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

    /// The instances an invariant applies to, each with its FHIR type. A
    /// choice element (`effective[x]`) is enumerated per concrete key
    /// (`effectiveDateTime`, `effectivePeriod`, …) so `$this is dateTime`
    /// answers per instance.
    fn invariant_contexts<'a, D: Document>(inv: &'a ProfileInvariant, doc: &D) -> Vec<(D, Option<&'a str>)> {
        match inv.path.strip_suffix("[x]") {
            Some(base) if !inv.types.is_empty() => inv
                .types
                .iter()
                .flat_map(|t| {
                    let mut key = String::from(base);
                    key.push_str(&t[..1].to_ascii_uppercase());
                    key.push_str(&t[1..]);
                    doc.instances(&key).into_iter().map(move |d| (d, Some(*t)))
                })
                .collect(),
            _ => doc.instances(inv.path).into_iter().map(|d| (d, inv.types.first().copied())).collect(),
        }
    }

    /// Cardinality (per parent instance), fixed value, and required binding for
    /// one element. `label` is the path reported in issues — absolute at the top
    /// level, `Path:slice.sub` inside a slice.
    fn check_element(
        el: &ElementConstraint,
        label: &str,
        doc: &impl Document,
        tx: &dyn TerminologyProvider,
        r: &mut Report,
    ) {
        for count in doc.child_counts(el.path) {
            let count = count as u32;
            if count < el.min {
                r.push(
                    Severity::Error,
                    IssueType::Required,
                    label,
                    format!("{label} is required (min {}, found {count})", el.min),
                );
            }
            if let Max::Bounded(m) = el.max {
                if count > m {
                    r.push(
                        Severity::Error,
                        IssueType::Structure,
                        label,
                        format!("{label} exceeds max {m} (found {count})"),
                    );
                }
            }
        }
        if let Some(f) = el.fixed {
            for s in doc.scalars(el.path) {
                if s != f {
                    r.push(Severity::Error, IssueType::Value, label, format!("{label} must be '{f}' (found '{s}')"));
                }
            }
        }
        let Some(b) = el.required_binding else { return };
        let vs = b.value_set;
        match b.kind {
            BindingKind::Code => {
                for code in doc.scalars(el.path) {
                    report_membership(tx.validate_code(vs, "", code), vs, code, label, r);
                }
            }
            BindingKind::Coding => {
                for inst in doc.instances(el.path) {
                    if let Some((system, code)) = coding_of(&inst) {
                        report_membership(tx.validate_code(vs, system, code), vs, code, label, r);
                    }
                }
            }
            BindingKind::CodeableConcept => {
                for inst in doc.instances(el.path) {
                    // A member if any coding is; unknown if none can be decided;
                    // a text-only concept is not checked (the reference validator
                    // only warns there).
                    let mut verdict = None;
                    for c in inst.instances("$this.coding") {
                        if let Some((system, code)) = coding_of(&c) {
                            match tx.validate_code(vs, system, code) {
                                Membership::Member => {
                                    verdict = Some(Membership::Member);
                                    break;
                                }
                                Membership::Unknown => verdict = Some(Membership::Unknown),
                                Membership::NotMember => verdict = verdict.or(Some(Membership::NotMember)),
                            }
                        }
                    }
                    if let Some(v) = verdict {
                        report_membership(v, vs, "the codings", label, r);
                    }
                }
            }
        }
    }

    fn coding_of(inst: &impl Document) -> Option<(&str, &str)> {
        let code = inst.scalars("$this.code").into_iter().next()?;
        let system = inst.scalars("$this.system").into_iter().next().unwrap_or("");
        Some((system, code))
    }

    fn report_membership(m: Membership, vs: &str, what: &str, label: &str, r: &mut Report) {
        match m {
            Membership::Member => {}
            Membership::NotMember => r.push(
                Severity::Error,
                IssueType::CodeInvalid,
                label,
                format!("'{what}' is not in the bound value set"),
            ),
            Membership::Unknown => r.push(
                Severity::Warning,
                IssueType::NotSupported,
                label,
                format!("no bundled terminology for {vs}; '{what}' not checked"),
            ),
        }
    }

    /// Does this instance belong to the slice? Value/pattern discriminators must
    /// all match; type slices are selected by key in `check_slicing`.
    fn matches(slice: &Slice, inst: &impl Document) -> bool {
        slice.discriminators.iter().all(|d| match d {
            Discriminator::Value { path, value } => inst.scalars(path).contains(value),
            Discriminator::Pattern { path, pairs } => inst.instances(path).iter().any(|n| {
                pairs.iter().all(|(rel, value)| {
                    let mut p = String::from("$this");
                    if !rel.is_empty() {
                        p.push('.');
                        p.push_str(rel);
                    }
                    n.scalars(&p).contains(value)
                })
            }),
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
    fn check_slicing<D: Document>(
        slicing: &Slicing,
        doc: &D,
        root: &D,
        tx: &dyn TerminologyProvider,
        r: &mut Report,
        depth: usize,
        cache: Option<&crate::Validator>,
    ) {
        let Some((parent_path, last)) = slicing.path.rsplit_once('.') else {
            return;
        };
        for parent in doc.instances(parent_path) {
            let all = parent.child_instances(last);
            let mut matched = 0usize;
            let mut label = String::new();
            for slice in slicing.slices {
                label.clear();
                label.push_str(slicing.path);
                label.push(':');
                label.push_str(slice.name);
                let insts: Vec<_> = match type_key(slice) {
                    Some(key) => parent.child_instances(key),
                    None => all.iter().filter(|i| matches(slice, *i)).cloned().collect(),
                };
                matched += insts.len();
                let count = insts.len() as u32;
                if count < slice.min {
                    r.push(
                        Severity::Error,
                        IssueType::Required,
                        &label,
                        format!("{label} is required (min {}, found {count})", slice.min),
                    );
                }
                if let Max::Bounded(m) = slice.max {
                    if count > m {
                        r.push(
                            Severity::Error,
                            IssueType::Structure,
                            &label,
                            format!("{label} exceeds max {m} (found {count})"),
                        );
                    }
                }
                let base_len = label.len();
                for inst in &insts {
                    for el in slice.elements {
                        label.truncate(base_len);
                        label.push('.');
                        label.push_str(el.path.trim_start_matches("$this."));
                        check_element(el, &label, inst, tx, r);
                    }
                    label.truncate(base_len);
                    if let Some(url) = slice.profile.filter(|_| depth < MAX_INHERIT) {
                        if let Some(profile) = crate::generated::REGISTRY.iter().find(|p| p.url == url) {
                            for issue in validate_at(profile, inst, root, tx, depth + 1, cache).issues {
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
                    format!(
                        "{} has {} element(s) matching no slice (closed slicing)",
                        slicing.path,
                        all.len() - matched
                    ),
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
                "registered"
                    | "preliminary"
                    | "final"
                    | "amended"
                    | "corrected"
                    | "cancelled"
                    | "entered-in-error"
                    | "unknown"
            )),
            "http://hl7.org/fhir/ValueSet/name-use" => {
                Some(matches!(code, "usual" | "official" | "temp" | "nickname" | "anonymous" | "old" | "maiden"))
            }
            "http://hl7.org/fhir/ValueSet/link-type" => {
                Some(matches!(code, "replaced-by" | "replaces" | "refer" | "seealso"))
            }
            "http://hl7.org/fhir/ValueSet/quantity-comparator" => Some(matches!(code, "<" | "<=" | ">=" | ">")),
            // The vital-signs UCUM unit sets, concept lists taken verbatim from
            // hl7.fhir.r4.core#4.0.1 (CC0). UCUM is freely embeddable.
            "http://hl7.org/fhir/ValueSet/ucum-vitals-common" => Some(matches!(
                code,
                "%" | "cm" | "[in_i]" | "kg" | "g" | "[lb_av]" | "Cel" | "[degF]" | "mm[Hg]" | "/min" | "kg/m2" | "m2"
            )),
            "http://hl7.org/fhir/ValueSet/ucum-bodyweight" => Some(matches!(code, "kg" | "[lb_av]" | "g")),
            "http://hl7.org/fhir/ValueSet/ucum-bodylength" => Some(matches!(code, "cm" | "[in_i]")),
            "http://hl7.org/fhir/ValueSet/ucum-bodytemp" => Some(matches!(code, "Cel" | "[degF]")),
            _ => None,
        }
    }
}

/// The verifier types — the finding vocabulary, aligned to `OperationOutcome`.
pub mod validate {
    use alloc::{string::String, vec::Vec};

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

    #[derive(Debug, Clone, PartialEq, Eq)]
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
    use crate::temporal;

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
        /// `|` — union of two collections, distinct.
        Union,
        /// `x in coll` (and `coll contains x`, reversed at parse time).
        In,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum Expr {
        This,
        /// An environment variable: `%resource`.
        Var(String),
        Lit(Lit),
        /// `base.name` — navigation.
        Nav(Box<Expr>, String),
        /// `base.name(args)` — a function call.
        Call(Box<Expr>, String, Vec<Expr>),
        Bin(Box<Expr>, Op, Box<Expr>),
        /// `e is Type` — a type test (only `$this`'s type is known: the
        /// invariant's element type, from the profile).
        Is(Box<Expr>, String),
        /// `e as Type` — `e` if it is of the type, else empty.
        As(Box<Expr>, String),
    }

    /// Syntax or semantics outside the supported core.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Unsupported;

    #[derive(Debug, Clone, PartialEq)]
    enum Tok {
        Ident(String),
        /// `%name` — an environment variable.
        Var(String),
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
        /// `|` — union.
        Pipe,
        /// Anything we don't support (`/`, `&`, `~`, …) — makes the parse fail.
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
                b'|' => {
                    out.push(Tok::Pipe);
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
                    // FHIRPath string escapes: \' \" \` \\ \/ \f \n \r \t \uXXXX
                    let mut j = i + 1;
                    let mut out_s = String::new();
                    loop {
                        let Some(&c) = b.get(j) else { return Err(Unsupported) };
                        match c {
                            b'\'' => break,
                            b'\\' => {
                                let Some(&e) = b.get(j + 1) else { return Err(Unsupported) };
                                j += 2;
                                match e {
                                    b'\'' | b'"' | b'`' | b'\\' | b'/' => out_s.push(e as char),
                                    b'f' => out_s.push('\u{c}'),
                                    b'n' => out_s.push('\n'),
                                    b'r' => out_s.push('\r'),
                                    b't' => out_s.push('\t'),
                                    b'u' => {
                                        let hex = b.get(j..j + 4).ok_or(Unsupported)?;
                                        let mut v = 0u32;
                                        for &h in hex {
                                            v = v * 16 + (h as char).to_digit(16).ok_or(Unsupported)?;
                                        }
                                        out_s.push(char::from_u32(v).ok_or(Unsupported)?);
                                        j += 4;
                                    }
                                    _ => return Err(Unsupported),
                                }
                            }
                            _ => {
                                // copy one UTF-8 scalar
                                let len = match c {
                                    0x00..=0x7F => 1,
                                    0xC0..=0xDF => 2,
                                    0xE0..=0xEF => 3,
                                    _ => 4,
                                };
                                let s = core::str::from_utf8(b.get(j..j + len).ok_or(Unsupported)?)
                                    .map_err(|_| Unsupported)?;
                                out_s.push_str(s);
                                j += len;
                            }
                        }
                    }
                    out.push(Tok::Str(out_s));
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
                    if s.starts_with('$') && s != "$this" {
                        return Err(Unsupported); // $index / $total: not in the core
                    }
                    out.push(Tok::Ident(s.into()));
                }
                b'%' => {
                    let start = i + 1;
                    let mut j = start;
                    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                        j += 1;
                    }
                    if j == start {
                        return Err(Unsupported); // `%'quoted'` variables: not in the core
                    }
                    let s = core::str::from_utf8(&b[start..j]).map_err(|_| Unsupported)?;
                    out.push(Tok::Var(s.into()));
                    i = j;
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
            let mut l = self.or()?;
            while self.eat_kw("implies") {
                let r = self.or()?;
                l = Expr::Bin(Box::new(l), Op::Implies, Box::new(r));
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
            let mut l = self.membership()?;
            while self.eat_kw("and") {
                let r = self.membership()?;
                l = Expr::Bin(Box::new(l), Op::And, Box::new(r));
            }
            Ok(l)
        }

        // `x in coll` / `coll contains x`
        fn membership(&mut self) -> Result<Expr, Unsupported> {
            let l = self.eq()?;
            if self.eat_kw("in") {
                let r = self.eq()?;
                return Ok(Expr::Bin(Box::new(l), Op::In, Box::new(r)));
            }
            if self.eat_kw("contains") {
                let r = self.eq()?;
                return Ok(Expr::Bin(Box::new(r), Op::In, Box::new(l)));
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
            let l = self.union()?;
            let op = match self.peek() {
                Some(Tok::Lt) => Op::Lt,
                Some(Tok::Le) => Op::Le,
                Some(Tok::Gt) => Op::Gt,
                Some(Tok::Ge) => Op::Ge,
                _ => return Ok(l),
            };
            self.i += 1;
            let r = self.union()?;
            Ok(Expr::Bin(Box::new(l), op, Box::new(r)))
        }

        // `a | b` — union, distinct.
        fn union(&mut self) -> Result<Expr, Unsupported> {
            let mut l = self.type_expr()?;
            while matches!(self.peek(), Some(Tok::Pipe)) {
                self.i += 1;
                let r = self.type_expr()?;
                l = Expr::Bin(Box::new(l), Op::Union, Box::new(r));
            }
            Ok(l)
        }

        // `e is Type` / `e as Type` — binds tighter than comparison, looser
        // than arithmetic (the spec's table).
        fn type_expr(&mut self) -> Result<Expr, Unsupported> {
            let e = self.add()?;
            if self.eat_kw("is") {
                return Ok(Expr::Is(Box::new(e), self.type_specifier()?));
            }
            if self.eat_kw("as") {
                return Ok(Expr::As(Box::new(e), self.type_specifier()?));
            }
            Ok(e)
        }

        /// `dateTime`, `FHIR.dateTime`, `System.String` — the last segment names the type.
        fn type_specifier(&mut self) -> Result<String, Unsupported> {
            let Some(Tok::Ident(mut t)) = self.next() else {
                return Err(Unsupported);
            };
            while matches!(self.peek(), Some(Tok::Dot)) {
                self.i += 1;
                let Some(Tok::Ident(seg)) = self.next() else {
                    return Err(Unsupported);
                };
                t = seg;
            }
            Ok(t)
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
                Some(Tok::Var(v)) => Ok(Expr::Var(v)),
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
    fn truth<D: Document>(c: &[Item<D>]) -> Result<Option<bool>, Unsupported> {
        Ok(match c {
            [] => None,
            [Item::Bool(b)] => Some(*b),
            [Item::Node(d)] => Some(d.as_bool().unwrap_or(true)),
            [_] => Some(true),
            // singleton evaluation of collections: more than one item is an error
            _ => return Err(Unsupported),
        })
    }

    /// What an expression is evaluated in besides `$this`: the type of the
    /// invariant's element (for `is`/`as` on `$this`) and the resource root
    /// (`%resource`). Inside `where`/`select`/…, `$this` is an arbitrary item
    /// whose type is unknown, so `is`/`as` there are unsupported.
    pub struct Env<'e, D> {
        pub this_type: Option<&'e str>,
        pub root: Option<&'e Item<D>>,
    }

    impl<'e, D> Env<'e, D> {
        fn inner(&self) -> Env<'e, D> {
            Env { this_type: None, root: self.root }
        }
    }

    /// `where(cond)`: each item becomes `$this` for its own test.
    fn filter<D: Document>(coll: &[Item<D>], cond: &Expr, env: &Env<'_, D>) -> Result<Vec<Item<D>>, Unsupported> {
        let inner = env.inner();
        let mut out = Vec::new();
        for it in coll {
            if truth(&eval(cond, it, &inner)?)? == Some(true) {
                out.push(it.clone());
            }
        }
        Ok(out)
    }

    /// FHIRPath `=` on two single items: integers numerically, booleans,
    /// dates / times temporally (empty when precisions or timezone presence
    /// differ), text by value, complex nodes structurally; items of different
    /// kinds are simply not equal. A decimal is outside the core.
    fn equal<D: Document>(a: &Item<D>, b: &Item<D>) -> Result<Option<bool>, Unsupported> {
        if let (Some(x), Some(y)) = (num_of(a)?, num_of(b)?) {
            return Ok(Some(x == y));
        }
        let bool_of = |it: &Item<D>| match it {
            Item::Bool(b) => Some(*b),
            Item::Node(d) => d.as_bool(),
            _ => None,
        };
        if let (Some(x), Some(y)) = (bool_of(a), bool_of(b)) {
            return Ok(Some(x == y));
        }
        if is_text(a) && is_text(b) {
            let (x, y) = (text_of(a)?, text_of(b)?);
            return Ok(match (temporal::parse(&x), temporal::parse(&y)) {
                (Some(tx), Some(ty)) => temporal::compare(&tx, &ty).map(|o| o.is_eq()),
                _ => Some(x == y),
            });
        }
        if let (Item::Node(x), Item::Node(y)) = (a, b) {
            if x.as_scalar().is_none() && y.as_scalar().is_none() {
                return Ok(Some(x.same(y)));
            }
        }
        Ok(Some(false))
    }

    /// Equality as a plain yes/no (for `in`, `union`, `intersect`, `distinct`):
    /// an undecidable comparison counts as not equal.
    fn items_equal<D: Document>(a: &Item<D>, b: &Item<D>) -> bool {
        matches!(equal(a, b), Ok(Some(true)))
    }

    /// The FHIR type of an item: the profile's declared type for `$this` when
    /// the engine supplied one, else what the document adapter knows, else
    /// the system type of a literal. Unknown → unsupported, never guessed.
    fn item_type<'a, D: Document>(it: &'a Item<D>, operand: &Expr, env: &Env<'a, D>) -> Result<&'a str, Unsupported> {
        if matches!(operand, Expr::This) {
            if let Some(t) = env.this_type {
                return Ok(t);
            }
        }
        match it {
            Item::Node(d) => d.fhir_type().ok_or(Unsupported),
            Item::Str(_) => Ok("string"),
            Item::Int(_) => Ok("integer"),
            Item::Bool(_) => Ok("boolean"),
        }
    }

    /// The type name in `as(canonical)` / `ofType(FHIR.Quantity)` — written as
    /// a bare path, so it parses as navigation; read back here.
    fn type_arg(e: &Expr) -> Option<&str> {
        match e {
            Expr::Nav(base, name) if matches!(**base, Expr::This) => Some(name),
            Expr::Nav(base, name) => match &**base {
                Expr::Nav(inner, ns) if matches!(**inner, Expr::This) && (ns == "FHIR" || ns == "System") => Some(name),
                _ => None,
            },
            _ => None,
        }
    }

    /// `dateTime` matches `dateTime` / `FHIR.dateTime`; case-insensitive on the
    /// last segment, as FHIRPath type specifiers are written both ways.
    /// FHIRPath `is`: the type itself or a supertype — FHIR's primitive
    /// specialisations (`code` is a `string`, `canonical` a `uri`, `Age` a
    /// `Quantity`), every resource a `Resource` (and, bar Bundle / Binary /
    /// Parameters, a `DomainResource`), every datatype an `Element`, and the
    /// FHIRPath System types the primitives map to (`String`, `Integer`,
    /// `DateTime`, …). Case-insensitive on the last segment, as type
    /// specifiers are written both ways.
    fn type_matches(actual: &str, spec: &str) -> bool {
        fn last(t: &str) -> &str {
            t.rsplit('.').next().unwrap_or(t)
        }
        let (actual, spec) = (last(actual), last(spec));
        let is = |t: &str| t.eq_ignore_ascii_case(spec);
        if is(actual) {
            return true;
        }
        let parents: &[&str] = match actual {
            "code" | "id" | "markdown" | "xhtml" => &["string", "String"],
            "string" | "base64Binary" => &["String"],
            "url" | "canonical" | "uuid" | "oid" => &["uri", "String"],
            "uri" => &["String"],
            "positiveInt" | "unsignedInt" => &["integer", "Integer"],
            "integer" => &["Integer"],
            "decimal" => &["Decimal"],
            "boolean" => &["Boolean"],
            "date" => &["Date"],
            "dateTime" | "instant" => &["DateTime"],
            "time" => &["Time"],
            "Age" | "Count" | "Distance" | "Duration" | "MoneyQuantity" | "SimpleQuantity" => &["Quantity", "Element"],
            "BackboneElement" => &["Element"],
            _ => &[],
        };
        if parents.iter().any(|p| is(p)) {
            return true;
        }
        if crate::typemap::RESOURCES.binary_search(&actual).is_ok() {
            return is("Resource") || (is("DomainResource") && !matches!(actual, "Bundle" | "Binary" | "Parameters"));
        }
        crate::typing::known_type(actual).is_some() && is("Element")
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
            Item::Node(d) if d.as_bool().is_none() && d.as_number().is_none() => {
                d.as_scalar().map(String::from).ok_or(Unsupported)
            }
            _ => Err(Unsupported),
        }
    }

    /// The integer an item denotes: an integer literal, or a JSON number node
    /// that is an integer. A decimal is outside the core (`Err`); a non-number
    /// is `Ok(None)`.
    fn num_of<D: Document>(it: &Item<D>) -> Result<Option<i64>, Unsupported> {
        match it {
            Item::Int(n) => Ok(Some(*n)),
            Item::Node(d) => match d.as_number() {
                Some(text) => parse_int(text).map(Some).ok_or(Unsupported),
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// Is this item text (a string literal or a JSON string node)?
    fn is_text<D: Document>(it: &Item<D>) -> bool {
        match it {
            Item::Str(_) => true,
            Item::Node(d) => d.as_bool().is_none() && d.as_number().is_none() && d.as_scalar().is_some(),
            _ => false,
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

    /// `<` `<=` `>` `>=`: integers numerically; dates / times temporally
    /// (empty when precisions or timezone presence differ); other text by
    /// code point. Mixed kinds are outside the core.
    fn compare<D: Document>(op: Op, a: &Item<D>, b: &Item<D>) -> Result<Option<bool>, Unsupported> {
        let ord = match (num_of(a)?, num_of(b)?) {
            (Some(x), Some(y)) => x.cmp(&y),
            (None, None) if is_text(a) && is_text(b) => {
                let (x, y) = (text_of(a)?, text_of(b)?);
                match (temporal::parse(&x), temporal::parse(&y)) {
                    (Some(tx), Some(ty)) => match temporal::compare(&tx, &ty) {
                        Some(o) => o,
                        None => return Ok(None),
                    },
                    // Strings compare by code point, which is what `str`'s order is.
                    _ => x.cmp(&y),
                }
            }
            _ => return Err(Unsupported),
        };
        Ok(Some(match op {
            Op::Lt => ord.is_lt(),
            Op::Le => ord.is_le(),
            Op::Gt => ord.is_gt(),
            Op::Ge => ord.is_ge(),
            _ => return Err(Unsupported),
        }))
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
    fn eval<D: Document>(e: &Expr, ctx: &Item<D>, env: &Env<'_, D>) -> Result<Vec<Item<D>>, Unsupported> {
        Ok(match e {
            Expr::This => vec![ctx.clone()],
            Expr::Var(name) => match (name.as_str(), env.root) {
                ("resource", Some(root)) => vec![root.clone()],
                _ => return Err(Unsupported),
            },
            Expr::Is(operand, ty) => {
                let coll = eval(operand, ctx, env)?;
                let Some(it) = single(&coll)? else {
                    return Ok(vec![]);
                };
                vec![Item::Bool(type_matches(item_type(it, operand, env)?, ty))]
            }
            Expr::As(operand, ty) => {
                let coll = eval(operand, ctx, env)?;
                let Some(it) = single(&coll)? else {
                    return Ok(vec![]);
                };
                if type_matches(item_type(it, operand, env)?, ty) {
                    vec![it.clone()]
                } else {
                    vec![]
                }
            }
            Expr::Lit(Lit::Str(s)) => vec![Item::Str(s.clone())],
            Expr::Lit(Lit::Int(n)) => vec![Item::Int(*n)],
            Expr::Lit(Lit::Bool(b)) => vec![Item::Bool(*b)],
            // A bare resource-type name (`QuestionnaireResponse.item`,
            // `Patient.name`) is `$this` when that is its type.
            Expr::Nav(base, name) if matches!(**base, Expr::This) && env.this_type == Some(name.as_str()) => {
                vec![ctx.clone()]
            }
            Expr::Nav(base, name) => {
                let mut out = Vec::new();
                for it in eval(base, ctx, env)? {
                    let Item::Node(d) = it else {
                        return Err(Unsupported);
                    };
                    let mut found = d.child_instances(name);
                    if found.is_empty() {
                        // Choice type: `value` reaches `valueQuantity` (typed by its key).
                        found = d.choice_children(name);
                    }
                    out.extend(found.into_iter().map(Item::Node));
                }
                out
            }
            Expr::Call(base, name, args) => {
                let coll = eval(base, ctx, env)?;
                match (name.as_str(), args.len()) {
                    ("exists", 0) => vec![Item::Bool(!coll.is_empty())],
                    ("exists", 1) => vec![Item::Bool(!filter(&coll, &args[0], env)?.is_empty())],
                    ("empty", 0) => vec![Item::Bool(coll.is_empty())],
                    ("not", 0) => match truth(&coll)? {
                        Some(b) => vec![Item::Bool(!b)],
                        None => vec![],
                    },
                    ("count", 0) => vec![Item::Int(coll.len() as i64)],
                    ("first", 0) => coll.into_iter().take(1).collect(),
                    ("hasValue", 0) => vec![Item::Bool(match coll.as_slice() {
                        [Item::Node(d)] => d.as_scalar().is_some(),
                        [_] => true,
                        _ => false,
                    })],
                    ("where", 1) => filter(&coll, &args[0], env)?,
                    ("last", 0) => coll.into_iter().last().into_iter().collect(),
                    // `single()`: the one item, empty for none; more is an error in the spec
                    ("single", 0) => single(&coll)?.cloned().into_iter().collect(),
                    ("select", 1) => {
                        let inner = env.inner();
                        let mut out = Vec::new();
                        for it in &coll {
                            out.extend(eval(&args[0], it, &inner)?);
                        }
                        out
                    }
                    ("all", 1) => {
                        let inner = env.inner();
                        let mut all = true;
                        for it in &coll {
                            all &= truth(&eval(&args[0], it, &inner)?)? == Some(true);
                        }
                        vec![Item::Bool(all)]
                    }
                    // `intersect(other)`: the items also in `other` (FHIRPath equality).
                    ("intersect", 1) => {
                        let other = eval(&args[0], ctx, env)?;
                        coll.into_iter().filter(|a| other.iter().any(|b| items_equal(a, b))).collect()
                    }
                    // `as(T)` / `ofType(T)` as functions: the items of that type.
                    // (`as` on a collection is treated as a filter, as the
                    // reference engine does in dom-3.)
                    ("as", 1) | ("ofType", 1) => {
                        let Some(ty) = type_arg(&args[0]) else {
                            return Err(Unsupported);
                        };
                        let mut out = Vec::new();
                        for it in &coll {
                            if type_matches(item_type(it, &args[0], &env.inner())?, ty) {
                                out.push(it.clone());
                            }
                        }
                        out
                    }
                    ("descendants", 0) => {
                        let mut out = Vec::new();
                        let mut frontier: Vec<Item<D>> = coll;
                        while let Some(it) = frontier.pop() {
                            if let Item::Node(d) = it {
                                for c in d.all_children() {
                                    out.push(Item::Node(c.clone()));
                                    frontier.push(Item::Node(c));
                                }
                            }
                        }
                        out
                    }
                    // `repeat(projection)`: the projection applied to the input
                    // and then to each new item, until nothing new appears.
                    ("repeat", 1) => {
                        let inner = env.inner();
                        let mut out: Vec<Item<D>> = Vec::new();
                        let mut frontier = coll;
                        while let Some(it) = frontier.pop() {
                            for n in eval(&args[0], &it, &inner)? {
                                if !out.iter().any(|o| items_equal(o, &n)) {
                                    out.push(n.clone());
                                    frontier.push(n);
                                }
                            }
                        }
                        out
                    }
                    ("allTrue", 0) | ("anyTrue", 0) | ("allFalse", 0) | ("anyFalse", 0) => {
                        let mut bools = Vec::new();
                        for it in &coll {
                            match it {
                                Item::Bool(b) => bools.push(*b),
                                Item::Node(d) => bools.push(d.as_bool().ok_or(Unsupported)?),
                                _ => return Err(Unsupported),
                            }
                        }
                        vec![Item::Bool(match name.as_str() {
                            "allTrue" => bools.iter().all(|b| *b),
                            "anyTrue" => bools.iter().any(|b| *b),
                            "allFalse" => bools.iter().all(|b| !*b),
                            _ => bools.iter().any(|b| !*b),
                        })]
                    }
                    ("isDistinct", 0) => {
                        let distinct =
                            coll.iter().enumerate().all(|(i, a)| !coll[..i].iter().any(|b| items_equal(a, b)));
                        vec![Item::Bool(distinct)]
                    }
                    ("distinct", 0) => {
                        let mut out: Vec<Item<D>> = Vec::new();
                        for it in coll {
                            if !out.iter().any(|o| items_equal(o, &it)) {
                                out.push(it);
                            }
                        }
                        out
                    }
                    // `trace(name[, projection])` logs in other engines; here it is the identity.
                    ("trace", 1 | 2) => coll,
                    ("matches", 1) => match single(&coll)? {
                        Some(it) => {
                            let s = text_of(it)?;
                            let pats = eval(&args[0], ctx, env)?;
                            let Some(pat) = single(&pats)? else {
                                return Ok(vec![]);
                            };
                            let re = crate::regex::Regex::new(&text_of(pat)?).map_err(|_| Unsupported)?;
                            vec![Item::Bool(re.is_match(&s))]
                        }
                        None => vec![],
                    },
                    // `extension(url)` — the extensions of each node with that URL.
                    ("extension", 1) => {
                        let url = match single(&eval(&args[0], ctx, env)?)? {
                            Some(u) => text_of(u)?,
                            None => return Ok(vec![]),
                        };
                        let mut out = Vec::new();
                        for it in &coll {
                            let Item::Node(d) = it else {
                                return Err(Unsupported);
                            };
                            for ext in d.child_instances("extension") {
                                let matches =
                                    ext.child_instances("url").iter().any(|u| u.as_scalar() == Some(url.as_str()));
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
                        let inner = env.inner();
                        match truth(&eval(&args[0], this, &inner)?)? {
                            Some(true) => eval(&args[1], this, &inner)?,
                            _ if args.len() == 3 => eval(&args[2], this, &inner)?,
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
                            match single(&eval(&args[0], ctx, env)?)? {
                                Some(p) => vec![Item::Bool(s.starts_with(text_of(p)?.as_str()))],
                                None => vec![],
                            }
                        }
                        None => vec![],
                    },
                    ("substring", 1 | 2) => match single(&coll)? {
                        Some(it) => {
                            let s = text_of(it)?;
                            let Some(start) = single(&eval(&args[0], ctx, env)?)?.map(int_of).transpose()? else {
                                return Ok(vec![]);
                            };
                            let len = match args.get(1) {
                                Some(a) => match single(&eval(a, ctx, env)?)? {
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
                let (a, b) = (eval(l, ctx, env)?, eval(r, ctx, env)?);
                let (Some(a), Some(b)) = (single(&a)?, single(&b)?) else {
                    return Ok(vec![]);
                };
                match (num_of(a)?, num_of(b)?) {
                    (Some(x), Some(y)) => arith(*op, x, y)?.map(Item::Int).into_iter().collect(),
                    // `+` on strings concatenates.
                    (None, None) if *op == Op::Add && is_text(a) && is_text(b) => {
                        vec![Item::Str(text_of(a)? + &text_of(b)?)]
                    }
                    _ => return Err(Unsupported),
                }
            }
            Expr::Bin(l, Op::Union, r) => {
                let mut out = eval(l, ctx, env)?;
                for it in eval(r, ctx, env)? {
                    if !out.iter().any(|o| items_equal(o, &it)) {
                        out.push(it);
                    }
                }
                // the left side is made distinct too
                let mut distinct: Vec<Item<D>> = Vec::new();
                for it in out {
                    if !distinct.iter().any(|o| items_equal(o, &it)) {
                        distinct.push(it);
                    }
                }
                distinct
            }
            Expr::Bin(l, Op::In, r) => {
                let (a, b) = (eval(l, ctx, env)?, eval(r, ctx, env)?);
                let Some(a) = single(&a)? else {
                    return Ok(vec![]);
                };
                vec![Item::Bool(b.iter().any(|x| items_equal(a, x)))]
            }
            Expr::Bin(l, op @ (Op::Lt | Op::Le | Op::Gt | Op::Ge), r) => {
                let (a, b) = (eval(l, ctx, env)?, eval(r, ctx, env)?);
                let (Some(a), Some(b)) = (single(&a)?, single(&b)?) else {
                    return Ok(vec![]);
                };
                compare(*op, a, b)?.map(Item::Bool).into_iter().collect()
            }
            Expr::Bin(l, op @ (Op::Eq | Op::Ne), r) => {
                let (a, b) = (eval(l, ctx, env)?, eval(r, ctx, env)?);
                if a.is_empty() || b.is_empty() {
                    return Ok(vec![]);
                }
                if a.len() != 1 || b.len() != 1 {
                    return Err(Unsupported);
                }
                match equal(&a[0], &b[0])? {
                    Some(eq) => vec![Item::Bool(if *op == Op::Eq { eq } else { !eq })],
                    None => vec![],
                }
            }
            Expr::Bin(l, op, r) => {
                let a = truth(&eval(l, ctx, env)?)?;
                let b = truth(&eval(r, ctx, env)?)?;
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
        check_with(expr, ctx, None, None)
    }

    /// [`check`] with the element's FHIR type (for `$this is …`) and the
    /// resource root (for `%resource`).
    pub fn check_with<D: Document>(expr: &Expr, ctx: &D, this_type: Option<&str>, root: Option<&D>) -> Verdict {
        let root_item = root.map(|r| Item::Node(r.clone()));
        let env = Env { this_type, root: root_item.as_ref() };
        match eval(expr, &Item::Node(ctx.clone()), &env).and_then(|coll| truth(&coll)) {
            Err(Unsupported) => Verdict::Unsupported,
            Ok(Some(false)) => Verdict::Fail,
            Ok(_) => Verdict::Pass,
        }
    }
}

/// FHIRPath's date / dateTime / time values, enough to compare them: a
/// partial date (`2020`, `2020-03`, `2020-03-01`), an optional time
/// (`T10:00`, `:30`, `.250`) and an optional timezone (`Z`, `+10:00`).
/// Comparison follows the spec: different precisions (seconds and
/// milliseconds counting as one), or a timezone on one side only, is empty;
/// otherwise the instants are compared with offsets normalised.
pub mod temporal {
    use core::cmp::Ordering;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Temporal {
        /// 1 year … 3 day, 4 hour, 5 minute, 6 second (milliseconds fold in).
        pub precision: u8,
        /// The instant in milliseconds, offsets applied (a partial date is
        /// taken at its start; only same-precision values are compared).
        pub millis: i64,
        pub has_date: bool,
        pub has_tz: bool,
    }

    fn digits(s: &[u8], n: usize) -> Option<i64> {
        let part = s.get(..n)?;
        if part.len() != n || !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        part.iter().try_fold(0i64, |acc, &c| acc.checked_mul(10)?.checked_add((c - b'0') as i64))
    }

    /// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }

    pub fn parse(s: &str) -> Option<Temporal> {
        let b = s.as_bytes();
        let mut i = 0;
        let (mut y, mut mo, mut d) = (1970i64, 1i64, 1i64);
        let mut precision = 0u8;
        let mut has_date = false;
        if b.first().is_some_and(u8::is_ascii_digit) {
            y = digits(b, 4)?;
            i = 4;
            precision = 1;
            has_date = true;
            if b.get(i) == Some(&b'-') {
                mo = digits(&b[i + 1..], 2)?;
                i += 3;
                precision = 2;
                if b.get(i) == Some(&b'-') {
                    d = digits(&b[i + 1..], 2)?;
                    i += 3;
                    precision = 3;
                }
            }
            if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
                return None;
            }
        }
        let (mut h, mut mi, mut sec, mut ms) = (0i64, 0i64, 0i64, 0i64);
        let mut has_tz = false;
        let mut tz = 0i64;
        let time_starts = b.get(i) == Some(&b'T') || (!has_date && b.get(i).is_some_and(u8::is_ascii_digit));
        if time_starts {
            if precision != 0 && precision < 3 {
                return None; // a time needs a full date before it
            }
            if b.get(i) == Some(&b'T') {
                i += 1;
            }
            h = digits(&b[i..], 2)?;
            if b.get(i + 2) != Some(&b':') {
                return None;
            }
            mi = digits(&b[i + 3..], 2)?;
            i += 5;
            precision = 5;
            if b.get(i) == Some(&b':') {
                sec = digits(&b[i + 1..], 2)?;
                i += 3;
                precision = 6;
                if b.get(i) == Some(&b'.') {
                    let start = i + 1;
                    let mut j = start;
                    while b.get(j).is_some_and(u8::is_ascii_digit) {
                        j += 1;
                    }
                    if j == start {
                        return None;
                    }
                    let frac = &s[start..j];
                    let mut scaled = 0i64;
                    for k in 0..3 {
                        scaled = scaled * 10 + frac.as_bytes().get(k).map_or(0, |c| (c - b'0') as i64);
                    }
                    ms = scaled;
                    i = j;
                }
            }
            if h > 23 || mi > 59 || sec > 60 {
                return None;
            }
            match b.get(i) {
                Some(b'Z') => {
                    has_tz = true;
                    i += 1;
                }
                Some(&sign @ (b'+' | b'-')) => {
                    let th = digits(&b[i + 1..], 2)?;
                    if b.get(i + 3) != Some(&b':') {
                        return None;
                    }
                    let tm = digits(&b[i + 4..], 2)?;
                    tz = (th * 60 + tm) * if sign == b'-' { -1 } else { 1 };
                    has_tz = true;
                    i += 6;
                }
                _ => {}
            }
        }
        if i != b.len() || precision == 0 {
            return None;
        }
        let days = if has_date { days_from_civil(y, mo, d) } else { 0 };
        let millis = (((days * 24 + h) * 60 + mi - tz) * 60 + sec) * 1000 + ms;
        Some(Temporal { precision, millis, has_date, has_tz })
    }

    /// `None` when the values are not comparable (different precision, or a
    /// timezone on one side only, or a date against a time).
    pub fn compare(a: &Temporal, b: &Temporal) -> Option<Ordering> {
        if a.precision != b.precision || a.has_tz != b.has_tz || a.has_date != b.has_date {
            return None;
        }
        Some(a.millis.cmp(&b.millis))
    }
}

/// A small regular-expression engine for FHIRPath `matches()`. Thompson-style
/// NFA simulation — linear in input × program, so hostile patterns and
/// inputs cannot blow up (no backtracking). Covers what invariants actually
/// use: literals, `.`, classes (`[0-9]`, `[^a-z]`, `\d \w \s`), anchors,
/// groups, alternation, and `* + ? {n} {n,} {n,m}` quantifiers (expanded,
/// bounded). Anything else (backreferences, lookaround, flags, `\b`,
/// unicode properties) is `Unsupported` — skipped, never guessed.
pub mod regex {
    use alloc::{boxed::Box, vec::Vec};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Unsupported;

    #[derive(Clone)]
    enum ClassItem {
        Ch(char),
        Range(char, char),
        Digit,
        Word,
        Space,
        NotDigit,
        NotWord,
        NotSpace,
    }

    #[derive(Clone)]
    enum Ast {
        Char(char),
        Any,
        Class(bool, Vec<ClassItem>),
        Start,
        End,
        Group(Vec<Vec<Ast>>),
        Repeat(Box<Ast>, u32, Option<u32>),
    }

    /// Bounds on the compiled program, so a pathological pattern is refused
    /// rather than expanded without limit.
    const MAX_PROGRAM: usize = 4096;
    const MAX_REPEAT: u32 = 256;

    struct Parser<'a> {
        s: &'a [char],
        i: usize,
        depth: usize,
    }

    impl Parser<'_> {
        fn peek(&self) -> Option<char> {
            self.s.get(self.i).copied()
        }

        fn alternation(&mut self) -> Result<Vec<Vec<Ast>>, Unsupported> {
            if self.depth > 32 {
                return Err(Unsupported);
            }
            self.depth += 1;
            let mut alts = alloc::vec![Vec::new()];
            while let Some(c) = self.peek() {
                match c {
                    ')' => break,
                    '|' => {
                        self.i += 1;
                        alts.push(Vec::new());
                    }
                    _ => {
                        let atom = self.atom()?;
                        let atom = self.quantified(atom)?;
                        alts.last_mut().unwrap().push(atom);
                    }
                }
            }
            self.depth -= 1;
            Ok(alts)
        }

        fn atom(&mut self) -> Result<Ast, Unsupported> {
            let c = self.peek().ok_or(Unsupported)?;
            self.i += 1;
            Ok(match c {
                '(' => {
                    if self.peek() == Some('?') {
                        // only the non-capturing form; lookaround / flags / names are not supported
                        if self.s.get(self.i + 1) == Some(&':') {
                            self.i += 2;
                        } else {
                            return Err(Unsupported);
                        }
                    }
                    let alts = self.alternation()?;
                    if self.peek() != Some(')') {
                        return Err(Unsupported);
                    }
                    self.i += 1;
                    Ast::Group(alts)
                }
                '[' => self.class()?,
                '.' => Ast::Any,
                '^' => Ast::Start,
                '$' => Ast::End,
                '\\' => self.escape(false)?,
                '*' | '+' | '?' | '{' | ')' | '|' => return Err(Unsupported),
                c => Ast::Char(c),
            })
        }

        fn escape(&mut self, in_class: bool) -> Result<Ast, Unsupported> {
            let c = self.peek().ok_or(Unsupported)?;
            self.i += 1;
            let item = |it: ClassItem| Ast::Class(false, alloc::vec![it]);
            Ok(match c {
                'd' => item(ClassItem::Digit),
                'D' => item(ClassItem::NotDigit),
                'w' => item(ClassItem::Word),
                'W' => item(ClassItem::NotWord),
                's' => item(ClassItem::Space),
                'S' => item(ClassItem::NotSpace),
                'n' => Ast::Char('\n'),
                't' => Ast::Char('\t'),
                'r' => Ast::Char('\r'),
                c if c.is_ascii_alphanumeric() => return Err(Unsupported), // \b, \1, \p{..}, …
                c => {
                    let _ = in_class;
                    Ast::Char(c)
                }
            })
        }

        fn class(&mut self) -> Result<Ast, Unsupported> {
            let neg = self.peek() == Some('^');
            if neg {
                self.i += 1;
            }
            let mut items = Vec::new();
            let mut first = true;
            loop {
                let c = self.peek().ok_or(Unsupported)?;
                if c == ']' && !first {
                    self.i += 1;
                    break;
                }
                first = false;
                self.i += 1;
                let lo = if c == '\\' {
                    match self.escape(true)? {
                        Ast::Char(ch) => ch,
                        Ast::Class(_, mut it) => {
                            items.push(it.remove(0));
                            continue;
                        }
                        _ => return Err(Unsupported),
                    }
                } else {
                    c
                };
                if self.peek() == Some('-') && self.s.get(self.i + 1).is_some_and(|&n| n != ']') {
                    self.i += 1;
                    let hi = self.peek().ok_or(Unsupported)?;
                    self.i += 1;
                    let hi = if hi == '\\' {
                        match self.escape(true)? {
                            Ast::Char(ch) => ch,
                            _ => return Err(Unsupported),
                        }
                    } else {
                        hi
                    };
                    if lo > hi {
                        return Err(Unsupported);
                    }
                    items.push(ClassItem::Range(lo, hi));
                } else {
                    items.push(ClassItem::Ch(lo));
                }
            }
            Ok(Ast::Class(neg, items))
        }

        fn quantified(&mut self, atom: Ast) -> Result<Ast, Unsupported> {
            let (min, max) = match self.peek() {
                Some('*') => (0, None),
                Some('+') => (1, None),
                Some('?') => (0, Some(1)),
                Some('{') => {
                    self.i += 1;
                    let min = self.number()?;
                    let max = if self.peek() == Some(',') {
                        self.i += 1;
                        if self.peek() == Some('}') {
                            None
                        } else {
                            Some(self.number()?)
                        }
                    } else {
                        Some(min)
                    };
                    if self.peek() != Some('}')
                        || max.is_some_and(|m| m < min)
                        || min > MAX_REPEAT
                        || max.is_some_and(|m| m > MAX_REPEAT)
                    {
                        return Err(Unsupported);
                    }
                    (min, max)
                }
                _ => return Ok(atom),
            };
            self.i += 1;
            if matches!(self.peek(), Some('?') | Some('+')) {
                self.i += 1; // lazy / possessive: same language for a yes/no match
            }
            if matches!(atom, Ast::Start | Ast::End) {
                return Err(Unsupported);
            }
            Ok(Ast::Repeat(Box::new(atom), min, max))
        }

        fn number(&mut self) -> Result<u32, Unsupported> {
            let start = self.i;
            let mut n: u32 = 0;
            while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
                n = n.checked_mul(10).and_then(|n| n.checked_add(d)).ok_or(Unsupported)?;
                self.i += 1;
            }
            if self.i == start {
                return Err(Unsupported);
            }
            Ok(n)
        }
    }

    #[derive(Clone)]
    enum Inst {
        Char(char),
        Any,
        Class(bool, Vec<ClassItem>),
        Start,
        End,
        Split(usize, usize),
        Jmp(usize),
        Match,
    }

    /// A compiled pattern.
    pub struct Regex {
        prog: Vec<Inst>,
    }

    impl Regex {
        pub fn new(pattern: &str) -> Result<Regex, Unsupported> {
            let chars: Vec<char> = pattern.chars().collect();
            let mut p = Parser { s: &chars, i: 0, depth: 0 };
            let alts = p.alternation()?;
            if p.i != chars.len() {
                return Err(Unsupported); // an unbalanced ')'
            }
            let mut prog = Vec::new();
            emit(&Ast::Group(alts), &mut prog)?;
            prog.push(Inst::Match);
            Ok(Regex { prog })
        }

        /// Does the pattern match anywhere in `s` (FHIRPath `matches` is a
        /// search; anchor with `^…$` for a full match)?
        pub fn is_match(&self, s: &str) -> bool {
            let chars: Vec<char> = s.chars().collect();
            let n = self.prog.len();
            let mut cur: Vec<usize> = Vec::new();
            let mut next: Vec<usize> = Vec::new();
            let mut seen = alloc::vec![usize::MAX; n];
            for pos in 0..=chars.len() {
                // unanchored: a fresh thread may start at every position
                add_thread(&self.prog, 0, pos, &chars, &mut cur, &mut seen);
                next.clear();
                let mut matched = false;
                for &pc in &cur {
                    match &self.prog[pc] {
                        Inst::Match => {
                            matched = true;
                            break;
                        }
                        Inst::Char(c) => {
                            if chars.get(pos) == Some(c) {
                                add_thread(&self.prog, pc + 1, pos + 1, &chars, &mut next, &mut seen);
                            }
                        }
                        Inst::Any => {
                            if pos < chars.len() {
                                add_thread(&self.prog, pc + 1, pos + 1, &chars, &mut next, &mut seen);
                            }
                        }
                        Inst::Class(neg, items) => {
                            if let Some(&c) = chars.get(pos) {
                                if class_matches(items, c) != *neg {
                                    add_thread(&self.prog, pc + 1, pos + 1, &chars, &mut next, &mut seen);
                                }
                            }
                        }
                        Inst::Start | Inst::End | Inst::Split(..) | Inst::Jmp(_) => {}
                    }
                }
                if matched {
                    return true;
                }
                core::mem::swap(&mut cur, &mut next);
                if pos == chars.len() {
                    break;
                }
            }
            false
        }
    }

    /// Follow epsilon edges from `pc` at input position `pos`, adding the
    /// consuming (or Match) states reached. `seen` is keyed by position so a
    /// state is added at most once per step: the linear-time guarantee.
    fn add_thread(prog: &[Inst], pc: usize, pos: usize, chars: &[char], list: &mut Vec<usize>, seen: &mut [usize]) {
        let mut stack = alloc::vec![pc];
        while let Some(pc) = stack.pop() {
            if seen[pc] == pos {
                continue;
            }
            seen[pc] = pos;
            match &prog[pc] {
                Inst::Jmp(t) => stack.push(*t),
                Inst::Split(a, b) => {
                    stack.push(*b);
                    stack.push(*a);
                }
                Inst::Start => {
                    if pos == 0 {
                        stack.push(pc + 1);
                    }
                }
                Inst::End => {
                    if pos == chars.len() {
                        stack.push(pc + 1);
                    }
                }
                _ => list.push(pc),
            }
        }
    }

    fn class_matches(items: &[ClassItem], c: char) -> bool {
        items.iter().any(|it| match it {
            ClassItem::Ch(x) => *x == c,
            ClassItem::Range(lo, hi) => (*lo..=*hi).contains(&c),
            ClassItem::Digit => c.is_ascii_digit(),
            ClassItem::NotDigit => !c.is_ascii_digit(),
            ClassItem::Word => c.is_alphanumeric() || c == '_',
            ClassItem::NotWord => !(c.is_alphanumeric() || c == '_'),
            ClassItem::Space => c.is_whitespace(),
            ClassItem::NotSpace => !c.is_whitespace(),
        })
    }

    fn emit(ast: &Ast, prog: &mut Vec<Inst>) -> Result<(), Unsupported> {
        if prog.len() > MAX_PROGRAM {
            return Err(Unsupported);
        }
        match ast {
            Ast::Char(c) => prog.push(Inst::Char(*c)),
            Ast::Any => prog.push(Inst::Any),
            Ast::Class(neg, items) => prog.push(Inst::Class(*neg, items.clone())),
            Ast::Start => prog.push(Inst::Start),
            Ast::End => prog.push(Inst::End),
            Ast::Group(alts) => {
                // split → alt1 → jmp end; split → alt2 → jmp end; … lastalt
                let mut jumps = Vec::new();
                for (i, alt) in alts.iter().enumerate() {
                    let last = i + 1 == alts.len();
                    let split_at = prog.len();
                    if !last {
                        prog.push(Inst::Split(split_at + 1, 0)); // patched below
                    }
                    for a in alt {
                        emit(a, prog)?;
                    }
                    if !last {
                        jumps.push(prog.len());
                        prog.push(Inst::Jmp(0)); // patched below
                        let next = prog.len();
                        prog[split_at] = Inst::Split(split_at + 1, next);
                    }
                }
                let end = prog.len();
                for j in jumps {
                    prog[j] = Inst::Jmp(end);
                }
            }
            Ast::Repeat(inner, min, max) => {
                for _ in 0..*min {
                    emit(inner, prog)?;
                }
                match max {
                    None => {
                        // loop: split(body, exit); body; jmp split
                        let split_at = prog.len();
                        prog.push(Inst::Split(split_at + 1, 0));
                        emit(inner, prog)?;
                        prog.push(Inst::Jmp(split_at));
                        let exit = prog.len();
                        prog[split_at] = Inst::Split(split_at + 1, exit);
                    }
                    Some(max) => {
                        // each optional copy: split(body, exit)
                        let mut splits = Vec::new();
                        for _ in *min..*max {
                            splits.push(prog.len());
                            prog.push(Inst::Split(prog.len() + 1, 0));
                            emit(inner, prog)?;
                        }
                        let exit = prog.len();
                        for s in splits {
                            prog[s] = Inst::Split(s + 1, exit);
                        }
                    }
                }
            }
        }
        if prog.len() > MAX_PROGRAM {
            return Err(Unsupported);
        }
        Ok(())
    }
}

/// The terminology seam. Bundled sets answer what they can; a host provider
/// (NCTS, VSAC, an on-device cache…) answers the rest. SNOMED content never
/// ships in the core — it only ever arrives through this seam.
pub mod terminology {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Membership {
        Member,
        NotMember,
        /// The provider cannot decide (unknown set, no data). The engine
        /// degrades this to a warning, never an error.
        Unknown,
    }

    pub trait TerminologyProvider {
        /// Is (`system`, `code`) in the value set `value_set` (a canonical URL,
        /// possibly `|version`)? `system` is empty for a bare `code` element —
        /// the set's implied system.
        fn validate_code(&self, value_set: &str, system: &str, code: &str) -> Membership;
    }

    /// The value sets compiled into the core (see [`crate::value_sets`]).
    pub struct Bundled;

    impl TerminologyProvider for Bundled {
        fn validate_code(&self, value_set: &str, _system: &str, code: &str) -> Membership {
            match crate::value_sets::in_value_set(value_set, code) {
                Some(true) => Membership::Member,
                Some(false) => Membership::NotMember,
                None => Membership::Unknown,
            }
        }
    }

    /// Providers in order; the first definite answer wins. Put [`Bundled`]
    /// first so the host is only asked what the core can't answer.
    pub struct Chain<'a>(pub &'a [&'a dyn TerminologyProvider]);

    impl TerminologyProvider for Chain<'_> {
        fn validate_code(&self, value_set: &str, system: &str, code: &str) -> Membership {
            for p in self.0 {
                let m = p.validate_code(value_set, system, code);
                if m != Membership::Unknown {
                    return m;
                }
            }
            Membership::Unknown
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
    const PATIENT_OK: &str = r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"identifier":[{"system":"http://hospital.example.org/mrn","value":"1"}],"name":[{"family":"Doe","use":"official"}],"gender":"male","birthDate":"1970-01-01"}"#;

    // --- no declared profile: the base resource applies, which we do not compile ---

    #[test]
    fn unprofiled_resource_is_valid_and_claims_no_profile() {
        let r = probe_str(
            r#"{"resourceType":"Observation","status":"final","code":{"text":"HR"},"valueQuantity":{"value":60}}"#,
        );
        assert!(r.contains("\"valid\":true") && !r.contains("\"profile\""), "{r}");
        // a validator may carry a jurisdiction default for a resource type
        let v = super::Validator::new()
            .with_default_profile("http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient");
        let bare = br#"{"resourceType":"Patient","active":true}"#;
        assert!(super::wire::report(&v.validate(bare)).contains("Patient.identifier is required"));
        assert!(super::wire::report(&super::validate_bytes(bare)).contains("\"valid\":true"));
        // an unknown default URL is ignored, never guessed
        let v2 = super::Validator::new().with_default_profile("http://example.org/nope");
        assert!(super::wire::report(&v2.validate(bare)).contains("\"valid\":true"));
    }

    #[test]
    fn versioned_canonical_selects_the_profile() {
        let r = probe_str(&PATIENT_OK.replace("au-core-patient\"", "au-core-patient|2.0.0\""));
        assert!(r.contains("\"profile\":\"AU Core Patient\""), "{r}");
    }

    #[test]
    fn resource_type_must_be_a_string() {
        let r = probe_str(r#"{"resourceType":5}"#);
        assert!(r.contains("resourceType must be a string") && r.contains("\"valid\":false"), "{r}");
    }

    #[test]
    fn tricky_string_does_not_end_object_early() {
        let r = probe_str(&PATIENT_OK.replace("\"gender\"", "\"note\":\"a } \\\" tricky\",\"gender\""));
        assert!(r.contains("\"valid\":true"), "{r}");
    }

    // --- JSON strings are decoded before anything compares them ---

    #[test]
    fn json_escapes_are_decoded() {
        use super::json::{parse, Value};
        let v =
            parse(br#"{"a\u006de":"http:\/\/loinc.org","q":"say \"hi\"\n","emoji":"\ud83d\ude00","lone":"\ud83d"}"#)
                .unwrap();
        let Value::Object(kvs) = v else { panic!() };
        let get = |k: &str| kvs.iter().find(|(kk, _)| kk.as_ref() == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(get("ame"), Value::Str("http://loinc.org".into()));
        assert_eq!(get("q"), Value::Str("say \"hi\"\n".into()));
        assert_eq!(get("emoji"), Value::Str("\u{1F600}".into()));
        assert_eq!(get("lone"), Value::Str("\u{FFFD}".into()));
        assert!(parse(br#""\x""#).is_err());
        assert!(parse(br#"{} trailing"#).is_err());
        assert!(parse(b"{}  \n").is_ok());
    }

    #[test]
    fn escaped_slashes_still_match_fixed_systems_and_slices() {
        let r = probe_str(&HEARTRATE_OK.replace("http://loinc.org", "http:\\/\\/loinc.org"));
        assert!(r.contains("\"valid\":true"), "{r}");
    }

    #[test]
    fn primitive_extension_nulls_are_placeholders() {
        use super::document::{Document, Parsed};
        let d = Parsed::parse(br#"{"resourceType":"Patient","name":[{"_given":[null,{"extension":[{"url":"http://x","valueString":"y"}]}]}]}"#).unwrap();
        assert_eq!(d.doc().child_counts("Patient.name.given"), vec![1]);
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
        let r = probe_str(
            r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"active":true}"#,
        );
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

    #[test]
    fn a_pattern_discriminator_needs_one_coding_carrying_every_leaf() {
        // MR under v2-0203 plus a *local* NI: no single coding is the IHI pattern
        // (v2-0203 + NI), so this is not the `ihi` slice and au-ihi must not fire.
        let split = PATIENT_OK.replace(
            PATIENT_ID,
            r#""identifier":[{"type":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v2-0203","code":"MR"},{"system":"http://example.org/local","code":"NI"}]},"system":"http://hospital.example.org/mrn","value":"1"}]"#,
        );
        let r = probe_str(&split);
        assert!(!r.contains("identifier:ihi"), "{r}");
        assert!(r.contains("\"valid\":true"), "{r}");
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
        let vs2 =
            parse("(component.empty() and hasMember.empty()) implies (value.exists() or dataAbsentReason.exists())")
                .unwrap();
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
        let ok = Parsed::parse(br#"{"coding":[{"system":"http://snomed.info/sct"},{"system":"http://loinc.org"}]}"#)
            .unwrap();
        assert_eq!(check(&e, &ok.doc()), Verdict::Pass);
        let no = Parsed::parse(br#"{"coding":[{"system":"http://snomed.info/sct"}]}"#).unwrap();
        assert_eq!(check(&e, &no.doc()), Verdict::Fail);
    }

    #[test]
    fn fhirpath_unsupported_is_skipped_not_guessed() {
        use super::path::parse;
        // %resource without a root, and type functions on untyped nodes: unsupported
        assert_eq!(
            fp("contained.where(('#'+id in (%resource.descendants().reference | %resource.descendants().as(canonical))).not()).empty()", r#"{"contained":[{"id":"x"}]}"#),
            super::path::Verdict::Unsupported
        );
        // decimal division, string concatenation `&`, equivalence `~`: outside the core
        assert!(parse("1 / 2").is_err());
        assert!(parse("a & b").is_err());
        assert!(parse("a ~ b").is_err());
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
        assert_eq!(fp("value.single() = '800360'", d), Pass);
        assert_eq!(fp("nope.single().empty()", d), Pass);
        assert_eq!(fp("list.single()", d), Unsupported);
        // a string function on a non-string, or on a multi-item collection: unsupported
        assert_eq!(fp("(1).length() = 1", d), Unsupported);
        assert_eq!(fp("list.length() = 1", d), Unsupported);
        assert_eq!(fp("list < 'c'", d), Unsupported);
        // an unknown function parses (it is a call) but evaluates as unsupported
        assert_eq!(fp("value.memberOf('http://x/vs')", d), Unsupported);
    }

    #[test]
    fn fhirpath_iif_select_all_extension() {
        use super::path::Verdict::{Fail, Pass};
        let d = r#"{"digits":["1","7"],"name":[{"family":"A"},{"family":"B"}],"extension":[{"url":"http://x/dar","valueCode":"unknown"},{"url":"http://x/other"}]}"#;
        assert_eq!(
            fp("digits.select($this.toInteger()).select(iif($this < 5, $this * 2, $this * 2 - 9)).first() = 2", d),
            Pass
        );
        assert_eq!(
            fp("digits.select($this.toInteger()).select(iif($this < 5, $this * 2, $this * 2 - 9)).last() = 5", d),
            Pass
        );
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

    // --- the terminology seam ---

    #[test]
    fn host_terminology_answers_what_the_bundled_sets_cannot() {
        use super::terminology::{Bundled, Chain, Membership, TerminologyProvider};
        use super::{validate_bytes, validate_bytes_with, wire};
        // telecom.system is bound (required) to contact-point-system, which is not bundled.
        let patient =
            PATIENT_OK.replace("\"gender\"", "\"telecom\":[{\"system\":\"phone\",\"value\":\"1\"}],\"gender\"");
        let bundled = wire::report(&validate_bytes(patient.as_bytes()));
        assert!(
            bundled.contains("no bundled terminology for http://hl7.org/fhir/ValueSet/contact-point-system"),
            "{bundled}"
        );

        struct Host(Membership, core::cell::Cell<usize>);
        impl TerminologyProvider for Host {
            fn validate_code(&self, vs: &str, system: &str, code: &str) -> Membership {
                self.1.set(self.1.get() + 1);
                assert!(
                    vs.starts_with("http://hl7.org/fhir/ValueSet/contact-point-system"),
                    "asked about {vs} {system} {code}"
                );
                self.0
            }
        }
        let yes = Host(Membership::Member, Default::default());
        let r = wire::report(&validate_bytes_with(patient.as_bytes(), &Chain(&[&Bundled, &yes])));
        assert!(!r.contains("contact-point-system"), "{r}");
        assert_eq!(yes.1.get(), 1, "asked once, only for the set the bundle lacks");

        let no = Host(Membership::NotMember, Default::default());
        let r = wire::report(&validate_bytes_with(patient.as_bytes(), &Chain(&[&Bundled, &no])));
        assert!(
            r.contains("\"path\":\"Patient.telecom.system\",\"message\":\"'phone' is not in the bound value set\""),
            "{r}"
        );
        assert!(r.contains("\"valid\":false"), "{r}");
    }

    #[test]
    fn codeable_concept_binding_is_member_if_any_coding_is() {
        use super::terminology::{Membership, TerminologyProvider};
        use super::{validate_bytes_with, wire};
        struct Only(&'static str);
        impl TerminologyProvider for Only {
            fn validate_code(&self, _vs: &str, _system: &str, code: &str) -> Membership {
                if code == self.0 {
                    Membership::Member
                } else {
                    Membership::NotMember
                }
            }
        }
        // Condition.clinicalStatus: required binding, CodeableConcept.
        let cond = |code: &str| {
            format!(
                r#"{{"resourceType":"Condition","meta":{{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-condition"]}},"clinicalStatus":{{"coding":[{{"system":"http://terminology.hl7.org/CodeSystem/condition-clinical","code":"{code}"}},{{"system":"http://example.org/other","code":"x"}}]}},"code":{{"text":"c"}},"subject":{{"reference":"Patient/1"}}}}"#
            )
        };
        let ok = wire::report(&validate_bytes_with(cond("active").as_bytes(), &Only("active")));
        assert!(!ok.contains("clinicalStatus"), "{ok}");
        let bad = wire::report(&validate_bytes_with(cond("bogus").as_bytes(), &Only("active")));
        assert!(bad.contains("Condition.clinicalStatus") && bad.contains("not in the bound value set"), "{bad}");
    }

    // --- FHIRPath tier three: type tests, %resource, intersect ---

    #[test]
    fn fhirpath_type_tests_use_the_elements_declared_type() {
        use super::document::Parsed;
        use super::path::{check, check_with, parse, Verdict};
        let e = parse("$this is dateTime implies $this.toString().length() >= 10").unwrap();
        let d = Parsed::parse(br#""2026-09""#).unwrap();
        // the element's type comes from the profile, never from the value's shape
        assert_eq!(check_with(&e, &d.doc(), Some("dateTime"), None), Verdict::Fail);
        assert_eq!(check_with(&e, &d.doc(), Some("Period"), None), Verdict::Pass);
        assert_eq!(check_with(&e, &d.doc(), Some("FHIR.dateTime"), None), Verdict::Fail);
        // unknown type: skipped, not guessed
        assert_eq!(check(&e, &d.doc()), Verdict::Unsupported);
        // vs-1: `as` yields $this or empty
        let vs1 = parse("($this as dateTime).toString().length() >= 8").unwrap();
        assert_eq!(check_with(&vs1, &d.doc(), Some("dateTime"), None), Verdict::Fail);
        assert_eq!(check_with(&vs1, &d.doc(), Some("Period"), None), Verdict::Pass); // empty >= 8 is empty → passes
                                                                                     // `is` on a node whose type the table cannot give: unsupported, not guessed
        let other = parse("a is string").unwrap();
        let untyped = Parsed::parse(br#"{"resourceType":"Nope","a":"x"}"#).unwrap();
        assert_eq!(check_with(&other, &untyped.doc(), Some("Nope"), None), Verdict::Unsupported);
    }

    #[test]
    fn fhirpath_resource_variable_and_intersect() {
        use super::document::Parsed;
        use super::path::{check, check_with, parse, Verdict};
        // obs-7: a component must not repeat the observation's own code
        let obs7 =
            parse("value.empty() or component.code.where(coding.intersect(%resource.code.coding).exists()).empty()")
                .unwrap();
        let mk = |comp_code: &str| {
            format!(
                r#"{{"resourceType":"Observation","code":{{"coding":[{{"system":"http://loinc.org","code":"85354-9"}}]}},"valueQuantity":{{"value":1}},"component":[{{"code":{{"coding":[{{"system":"http://loinc.org","code":"{comp_code}"}}]}}}}]}}"#
            )
        };
        let (ok_src, dup_src) = (mk("8480-6"), mk("85354-9"));
        let ok = Parsed::parse(ok_src.as_bytes()).unwrap();
        assert_eq!(check_with(&obs7, &ok.doc(), Some("Observation"), Some(&ok.doc())), Verdict::Pass);
        let dup = Parsed::parse(dup_src.as_bytes()).unwrap();
        assert_eq!(check_with(&obs7, &dup.doc(), Some("Observation"), Some(&dup.doc())), Verdict::Fail);
        // without a root, %resource is unsupported (skipped)
        assert_eq!(check(&obs7, &dup.doc()), Verdict::Unsupported);
        // intersect on scalars
        let d = Parsed::parse(br#"{"a":["x","y"],"b":["y","z"]}"#).unwrap();
        assert_eq!(check(&parse("a.intersect(b) = 'y'").unwrap(), &d.doc()), Verdict::Pass);
        assert_eq!(check(&parse("a.intersect(c).empty()").unwrap(), &d.doc()), Verdict::Pass);
    }

    #[test]
    fn obs7_fires_through_the_engine_on_a_profiled_observation() {
        // A valued observation with a component repeating its own code:
        // obs-7 (an Observation base invariant, reached via %resource).
        let dup = HEARTRATE_OK.replace(
            "\"effectiveDateTime\"",
            "\"component\":[{\"code\":{\"coding\":[{\"system\":\"http://loinc.org\",\"code\":\"8867-4\"}]},\"valueString\":\"x\"}],\"effectiveDateTime\"",
        );
        let r = probe_str(&dup);
        assert!(r.contains("obs-7"), "{r}");
        assert!(!probe_str(HEARTRATE_OK).contains("obs-7"));
    }

    // --- review findings: what the spec says and we now do ---

    #[test]
    fn fhirpath_string_escapes_are_decoded() {
        use super::path::Verdict::Pass;
        let d = r#"{"v":"a.b","n":"12"}"#;
        assert_eq!(fp("v.matches('a\\\\.b')", d), Pass); // FHIRPath source: 'a\\.b' → regex a\.b
        assert_eq!(fp("n.matches('^\\\\d+$')", d), Pass);
        assert_eq!(fp("'it\\'s' = 'it' + '\\'' + 's'", d), Pass); // FHIRPath source: 'it\'s'
        assert_eq!(fp("'\\u0041' = 'A'", d), Pass);
        assert!(super::path::parse("'\\q' = 'q'").is_err()); // not a FHIRPath escape: the parse is refused
                                                             // $index / $total: not in the core
        assert!(super::path::parse("$index = 0").is_err());
        assert!(super::path::parse("$total").is_err());
    }

    #[test]
    fn implies_is_left_associative_and_booleans_are_singletons() {
        use super::path::Verdict::{Fail, Pass, Unsupported};
        let d = r#"{"a":["x","y"],"one":["x"]}"#;
        assert_eq!(fp("false implies true implies false", d), Fail); // (false implies true) implies false
                                                                     // more than one item in boolean context: skipped, never guessed
        assert_eq!(fp("a.not()", d), Unsupported);
        assert_eq!(fp("a and true", d), Unsupported);
        assert_eq!(fp("one and true", d), Pass);
        assert_eq!(fp("a.hasValue()", d), Fail);
        assert_eq!(fp("one.hasValue()", d), Pass);
    }

    #[test]
    fn numbers_are_numbers() {
        use super::path::Verdict::{Fail, Pass, Unsupported};
        let d = r#"{"a":1,"b":2,"c":10,"d":9,"s":"1","x":1.5}"#;
        assert_eq!(fp("a + b = 3", d), Pass);
        assert_eq!(fp("c > d", d), Pass);
        assert_eq!(fp("a = 1", d), Pass);
        assert_eq!(fp("a = s", d), Fail); // different types are not equal
        assert_eq!(fp("1 = '1'", d), Fail);
        assert_eq!(fp("(a).length() = 1", d), Unsupported);
        assert_eq!(fp("x = 1", d), Unsupported); // decimals: outside the core
        assert_eq!(fp("a + s = 2", d), Unsupported);
    }

    #[test]
    fn dates_compare_as_instants() {
        use super::path::Verdict::{Fail, Pass};
        let d = r#"{"h":"2020-01-01T10:00:00.000Z","p":"2020-01-01T10:00:00Z","q":"2020-01-01T10:00:00+00:00","later":"2020-01-01T19:00:00+10:00","early":"2020-01-01T09:30:00Z","day":"2020-01-01","month":"2020-01"}"#;
        assert_eq!(fp("h >= p", d), Pass);
        assert_eq!(fp("h = p and p = q", d), Pass);
        assert_eq!(fp("later < early", d), Pass); // 19:00+10:00 is 09:00Z — before 09:30Z
        assert_eq!(fp("early > later", d), Pass);
        assert_eq!(fp("(day = month).empty()", d), Pass); // different precision: empty
        assert_eq!(fp("(day < h).empty()", d), Pass); // date vs dateTime with tz: empty
        assert_eq!(fp("day < '2020-01-02'", d), Pass);
        assert_eq!(fp("'b' > 'a'", d), Pass); // plain text still compares
        assert_eq!(fp("h < p", d), Fail);
    }

    #[test]
    fn type_tests_honour_the_hierarchy() {
        use super::path::Verdict::{Fail, Pass};
        let d = r#"{"resourceType":"Condition","onsetAge":{"value":1,"unit":"a"},"code":{"text":"x"},"subject":{"reference":"Patient/1"},"id":"c1"}"#;
        assert_eq!(fp("onset is Quantity", d), Pass);
        assert_eq!(fp("onset is Age", d), Pass);
        assert_eq!(fp("onset is CodeableConcept", d), Fail);
        assert_eq!(fp("$this is Resource and $this is DomainResource", r#"{"resourceType":"Condition"}"#), Pass);
        assert_eq!(fp("$this is DomainResource", r#"{"resourceType":"Bundle"}"#), super::path::Verdict::Unsupported); // Bundle is not in the table
        assert_eq!(fp("code is Element", d), Pass);
        assert_eq!(fp("id is id and id is string and id is String", d), Pass);
        assert_eq!(fp("subject.reference is string", d), Pass);
        let mr = r#"{"resourceType":"MedicationRequest","instantiatesCanonical":["http://x"],"instantiatesUri":["http://y"]}"#;
        assert_eq!(fp("descendants().ofType(uri).count() = 2", mr), Pass); // canonical is a uri
    }

    // --- regex + tier three collections ---

    #[test]
    fn regex_engine_covers_the_registrys_patterns_and_refuses_the_rest() {
        use super::regex::Regex;
        let ok = |p: &str, s: &str| Regex::new(p).unwrap().is_match(s);
        assert!(ok("^([0-9]{16})$", "8003608833357361"));
        assert!(!ok("^([0-9]{16})$", "800360883335736"));
        assert!(!ok("^([0-9]{16})$", "8003608833357361x"));
        assert!(ok("^[A-Z]{3}[0-9]{10}$", "ABC0123456789"));
        assert!(!ok("^[A-Z]{3}[0-9]{10}$", "AbC0123456789"));
        assert!(ok("^(800361)([0-9]{10})(@)(800362)([0-9]{10})$", "8003610000000000@8003620000000000"));
        assert!(ok("b+c", "aabbbcc")); // unanchored: a search
        assert!(!ok("^b+c", "aabbbcc"));
        assert!(ok("a|b", "xbx") && !ok("a|b", "xyz"));
        assert!(ok("^\\d{2}-\\w+\\s?$", "12-abc_ "));
        assert!(ok("[^a-c]", "d") && !ok("[^a-c]", "b"));
        assert!(ok("colou?r", "color") && ok("colou?r", "colour"));
        assert!(ok("^a{2,3}$", "aaa") && !ok("^a{2,3}$", "aaaa"));
        assert!(ok("^.$", "é") && !ok("^.$", "ab"));
        assert!(ok("(?:ab)+", "ababab"));
        // linear time: the classic backtracking bomb finishes instantly
        assert!(!Regex::new("^(a+)+$").unwrap().is_match(&format!("{}b", "a".repeat(5000))));
        // outside the supported syntax: refused, never guessed
        for p in ["(a)\\1", "(?=a)", "(?i)a", "\\ba", "\\p{L}", "a{300}", "(a", "a)", "*a", "[z-a]"] {
            assert!(Regex::new(p).is_err(), "{p}");
        }
    }

    #[test]
    fn fhirpath_matches_union_in_descendants_repeat() {
        use super::path::Verdict::{Fail, Pass, Unsupported};
        let d = r#"{"value":"8003608833357361","a":["x","y"],"b":["y","z"],"item":[{"linkId":"1","item":[{"linkId":"2"}]},{"linkId":"3"}]}"#;
        assert_eq!(fp("value.matches('^([0-9]{16})$')", d), Pass);
        assert_eq!(fp("value.matches('^([0-9]{15})$')", d), Fail);
        assert_eq!(fp("value.matches('(a)\\\\1')", d), Unsupported); // a backreference: refused
        assert_eq!(fp("nope.matches('x').empty()", d), Pass);
        assert_eq!(fp("(a | b).count() = 3", d), Pass);
        assert_eq!(fp("'y' in a and 'y' in b and ('q' in a).not()", d), Pass);
        assert_eq!(fp("a contains 'x'", d), Pass);
        assert_eq!(fp("descendants().linkId.count() = 3", d), Pass);
        assert_eq!(fp("repeat(item).linkId.count() = 3", d), Pass);
        assert_eq!(fp("repeat(item).linkId.isDistinct()", d), Pass);
        assert_eq!(fp("(a | b | a).isDistinct()", d), Pass);
        assert_eq!(fp("a.trace('t').count() = 2", d), Pass);
        assert_eq!(fp("(true | false).anyTrue() and (true | false).allTrue().not() and (false).allFalse()", d), Pass);
        assert_eq!(fp("nope.allTrue()", d), Pass); // vacuous
                                                   // sdcqr-2's shape: every group's items have distinct linkIds
        assert_eq!(fp("repeat(item).select(item.linkId.isDistinct()).allTrue()", d), Pass);
    }

    #[test]
    fn ihi_format_invariant_fires_through_the_slice_profile() {
        let short = PATIENT_OK.replace(
            PATIENT_ID,
            r#""identifier":[{"type":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v2-0203","code":"NI"}]},"system":"http://ns.electronichealth.net.au/id/hi/ihi/1.0","value":"800360883335736"}]"#,
        );
        let r = probe_str(&short);
        assert!(r.contains("16 digit"), "{r}");
        assert!(r.contains("\"valid\":false"), "{r}");
    }

    #[test]
    fn questionnaireresponse_repeat_invariant_is_live() {
        use super::path::parse;
        // the registry's own sdcqr-2 must now parse
        let expr = super::generated::REGISTRY
            .iter()
            .flat_map(|p| p.invariants.iter())
            .find(|i| i.key == "sdcqr-2")
            .map(|i| i.expression)
            .expect("sdcqr-2 in the registry");
        assert!(parse(expr).is_ok(), "{expr}");
    }

    // --- typed traversal + dom-3 ---

    #[test]
    fn nodes_know_their_fhir_type_from_the_table() {
        use super::document::{Document, Parsed};
        let src = br#"{"resourceType":"Observation","valueQuantity":{"value":1},"component":[{"code":{"coding":[{"system":"x"}]}}],"contained":[{"resourceType":"Patient","name":[{"family":"D"}]}],"referenceRange":[{"low":{"value":1}}]}"#;
        let d = Parsed::parse(src).unwrap();
        let doc = d.doc();
        assert_eq!(doc.fhir_type(), Some("Observation"));
        let value = doc.instances("$this.valueQuantity");
        assert_eq!(value[0].fhir_type(), Some("Quantity"));
        assert_eq!(value[0].instances("$this.value")[0].fhir_type(), Some("decimal"));
        let comp = doc.instances("$this.component");
        assert_eq!(comp[0].fhir_type(), Some("BackboneElement"));
        assert_eq!(comp[0].instances("$this.code.coding.system")[0].fhir_type(), Some("uri"));
        let contained = doc.instances("$this.contained");
        assert_eq!(contained[0].fhir_type(), Some("Patient"));
        assert_eq!(contained[0].instances("$this.name")[0].fhir_type(), Some("HumanName"));
        // content reference
        assert_eq!(doc.instances("$this.referenceRange.low")[0].fhir_type(), Some("Quantity"));
        // untyped: a resource the table doesn't cover, or a [x] path
        assert_eq!(Parsed::parse(br#"{"resourceType":"Nope","a":1}"#).unwrap().doc().fhir_type(), None);
        assert_eq!(doc.instances("$this.value[x]")[0].fhir_type(), None);
        // descendants carry types
        let kinds: Vec<_> = doc.all_children().iter().map(|c| c.fhir_type()).collect();
        assert!(
            kinds.contains(&Some("Quantity"))
                && kinds.contains(&Some("Patient"))
                && kinds.contains(&Some("BackboneElement")),
            "{kinds:?}"
        );
    }

    #[test]
    fn fhirpath_type_functions_on_typed_nodes() {
        use super::path::Verdict::{Fail, Pass, Unsupported};
        let d = r#"{"resourceType":"MedicationRequest","instantiatesCanonical":["http://x/plan"],"instantiatesUri":["http://x/u"],"subject":{"reference":"Patient/1"},"medicationCodeableConcept":{"text":"m"}}"#;
        assert_eq!(fp("descendants().as(canonical).count() = 1", d), Pass);
        assert_eq!(fp("descendants().ofType(FHIR.uri).count() = 2", d), Pass); // canonical is a uri
        assert_eq!(fp("medication.ofType(CodeableConcept).exists()", d), Pass);
        assert_eq!(fp("medication is CodeableConcept", d), Pass);
        assert_eq!(fp("medication is Reference", d), Fail);
        assert_eq!(fp("subject is Reference and subject.reference is string", d), Pass);
        // untyped resource: skipped, not guessed
        assert_eq!(fp("descendants().as(canonical).empty()", r#"{"resourceType":"Nope","a":"b"}"#), Unsupported);
    }

    #[test]
    fn dom3_contained_resources_must_be_referenced() {
        use super::path::Verdict::{Fail, Pass};
        let mk = |reference: &str| {
            format!(
                r#"{{"resourceType":"MedicationRequest","contained":[{{"resourceType":"Medication","id":"med","code":{{"text":"x"}}}}],"status":"active","intent":"order","medicationReference":{{"reference":"{reference}"}},"subject":{{"reference":"Patient/1"}}}}"#
            )
        };
        let dom3 = super::generated::REGISTRY
            .iter()
            .flat_map(|p| p.invariants.iter())
            .find(|i| i.key == "dom-3")
            .unwrap()
            .expression;
        use super::document::Parsed;
        use super::path::{check_with, parse};
        let e = parse(dom3).unwrap();
        let (ok_src, bad_src) = (mk("#med"), mk("Medication/other"));
        let ok = Parsed::parse(ok_src.as_bytes()).unwrap();
        assert_eq!(check_with(&e, &ok.doc(), Some("MedicationRequest"), Some(&ok.doc())), Pass);
        let bad = Parsed::parse(bad_src.as_bytes()).unwrap();
        assert_eq!(check_with(&e, &bad.doc(), Some("MedicationRequest"), Some(&bad.doc())), Fail);
        // through the engine, on a profiled resource
        let profiled = bad_src.replace("\"resourceType\":\"MedicationRequest\",", "\"resourceType\":\"MedicationRequest\",\"meta\":{\"profile\":[\"http://hl7.org/fhir/us/core/StructureDefinition/us-core-medicationrequest\"]},");
        let r = probe_str(&profiled);
        assert!(r.contains("dom-3"), "{r}");
    }

    // --- several declared profiles ---

    #[test]
    fn every_declared_profile_applies() {
        let both = PATIENT_OK.replace(
            "\"resourceType\":\"Patient\",",
            "\"resourceType\":\"Patient\",\"meta\":{\"profile\":[\"http://hl7.org/fhir/us/core/StructureDefinition/us-core-patient\",\"http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient\"]},",
        );
        let r = probe_str(&both);
        assert!(r.contains("\"profile\":\"US Core Patient Profile\""), "{r}");
        assert!(r.contains("\"profiles\":[\"US Core Patient Profile\",\"AU Core Patient\"]"), "{r}");
        // an AU-only finding shows up alongside the US ones
        let gender_bad = probe_str(&both.replace("\"male\"", "\"banana\""));
        assert_eq!(
            gender_bad.matches("is not in the bound value set").count(),
            1,
            "shared finding reported once: {gender_bad}"
        );
        let single = probe_str(PATIENT_OK);
        assert!(!single.contains("\"profiles\""), "{single}");
    }

    // --- must-support (information, not conformance) ---

    #[test]
    fn must_support_gaps_are_reported_where_the_parent_is_present() {
        use super::must_support_gaps;
        let us = r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org/fhir/us/core/StructureDefinition/us-core-patient"]},"identifier":[{"system":"http://h.example.org/mrn","value":"1"}],"name":[{"family":"D"}],"gender":"male"}"#;
        let gaps = must_support_gaps(us.as_bytes());
        // absent must-support elements at the top level… (US Core flags
        // telecom's children MS but not telecom itself, so it is not a gap)
        assert!(!gaps.contains(&"Patient.telecom"), "{gaps:?}");
        assert!(gaps.contains(&"Patient.birthDate"), "{gaps:?}");
        assert!(gaps.contains(&"Patient.address"), "{gaps:?}");
        // …and under a present parent — but not under an absent one
        assert!(gaps.contains(&"Patient.name.given"), "{gaps:?}");
        assert!(!gaps.contains(&"Patient.address.line"), "{gaps:?}");
        assert!(!gaps.contains(&"Patient.telecom.system"), "{gaps:?}");
        // populated ones are not gaps
        assert!(!gaps.contains(&"Patient.identifier.value"), "{gaps:?}");
        // it is not validation: the same resource validates independently
        assert!(must_support_gaps(b"not json").is_empty());
        assert!(must_support_gaps(br#"{"resourceType":"Encounter"}"#).is_empty());
    }

    // --- wire: OperationOutcome ---

    #[test]
    fn operation_outcome_is_spec_shaped() {
        let v = validate_bytes(br#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"active":true}"#);
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
