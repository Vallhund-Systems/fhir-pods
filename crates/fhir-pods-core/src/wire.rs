//! The interop / wire context — the anti-corruption layer. Spec shapes live
//! here and nowhere else: the `OperationOutcome` resource, and the pods' own
//! JSON report.

use alloc::string::String;

use crate::validate::{IssueType, Report, Severity};
use crate::Validation;

/// `OperationOutcome.issue.severity`.
pub(crate) fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Fatal => "fatal",
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Information => "information",
    }
}

/// `OperationOutcome.issue.code` — the IssueType vocabulary.
pub(crate) fn issue_type_str(c: IssueType) -> &'static str {
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
