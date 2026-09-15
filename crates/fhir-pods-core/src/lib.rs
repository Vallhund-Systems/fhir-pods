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

/// Minimal resource model. Commit-zero placeholder: a resource is carried as its
/// raw JSON until the generated typed model + parsing land (see the architecture
/// doc — the typed resources are code-generated from the FHIR StructureDefinitions).
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

/// The verifier. Conformance = profile (StructureDefinition) constraints +
/// terminology bindings + FHIRPath invariants, evaluated against a resource.
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
        /// FHIRPath location, e.g. `Observation.value[x]`.
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
/// Grammar + evaluator to follow.
pub mod path {
    #[derive(Debug, Default)]
    pub struct Engine;

    impl Engine {
        pub fn new() -> Self {
            Self
        }
        // pub fn eval(&self, expr: &str, ctx: &Resource) -> Result<Value, Error>;
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
/// The `trace_id` field encodes the design stance: the audit record is the
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
        /// Who acted (the agent).
        pub actor: String,
        /// What they did.
        pub action: Action,
        /// Which resource / patient — the PHI reference the audit must name.
        pub subject_ref: String,
        /// The result.
        pub outcome: Outcome,
        /// When, ISO-8601.
        pub recorded: String,
        /// Correlation to the trace that carried the request, if any.
        pub trace_id: Option<String>,
    }

    impl AuditRecord {
        /// Build a FHIR `AuditEvent` resource from this record. Commit-zero stub.
        pub fn to_audit_event(&self) -> Resource {
            Resource::new("AuditEvent", "{}")
        }
    }
}
