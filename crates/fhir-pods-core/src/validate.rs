//! The verifier types — the finding vocabulary, aligned to `OperationOutcome`.

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
    /// The labelled location (`Observation.value[x]:valueQuantity.code`);
    /// `wire::expression` derives the FHIRPath `OperationOutcome.issue.expression` from it.
    pub path: String,
    pub message: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
