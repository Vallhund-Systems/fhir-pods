//! The compiled profile IR — what `tools/gen_profile.py` emits from a package's
//! `snapshot.element`. Our own representation; immutable, embedded, `'static`.

/// Upper cardinality bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Max {
    Star,
    Bounded(u32),
}

/// One element's enforceable constraints. Cardinality is per parent instance.
/// Inside a slice, `path` is relative to the matched instance: `$this.…`.
#[derive(Debug, PartialEq, Eq)]
pub struct ElementConstraint {
    pub path: &'static str,
    pub min: u32,
    pub max: Max,
    /// A scalar the element must equal exactly (fixedUri / fixedCode / …).
    /// For a SNOMED coding this is only ever `system`/`code`, never `display`.
    pub fixed: Option<&'static str>,
    /// A required-strength ValueSet binding on a coded element.
    pub required_binding: Option<Binding>,
    /// For a choice element (`value[x]`): the types it allows. A concrete
    /// key of any other type is a structure error.
    pub types: &'static [&'static str],
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
#[derive(Debug, PartialEq, Eq)]
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

#[derive(Debug, PartialEq, Eq)]
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

#[derive(Debug, PartialEq, Eq)]
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
#[derive(Debug, PartialEq, Eq)]
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
/// One variable input of a profile: an element the host supplies when
/// lowering — everything the profile does not fix. `path` is the labelled
/// resource-relative path (`component:SystolicBP.valueQuantity`, `subject`);
/// `ty` the FHIR type the value is coerced to; `choices` the concrete keys a
/// choice element accepts (`valueQuantity`, `valueString`, …) when the path
/// ends in `[x]`.
#[derive(Debug, PartialEq, Eq)]
pub struct Slot {
    pub path: &'static str,
    pub ty: &'static str,
    pub choices: &'static [&'static str],
    pub min: u32,
    pub max: Max,
    pub must_support: bool,
}

/// What the generator knows about building a resource for this profile:
/// the fixed leaves (labelled paths → scalar, from every fixed / pattern /
/// inherited-profile value at any depth), which slices are required (and
/// so instantiated by default) versus on demand, and the input slots. Which
/// elements repeat comes from the base spec's element table (`typing`).
#[derive(Debug, PartialEq, Eq)]
pub struct Lowering {
    pub fixed: &'static [(&'static str, &'static str)],
    pub slices: &'static [(&'static str, u32)],
    pub slots: &'static [Slot],
}

#[derive(Debug, PartialEq, Eq)]
pub struct CompiledProfile {
    pub url: &'static str,
    pub name: &'static str,
    pub resource_type: &'static str,
    pub elements: &'static [ElementConstraint],
    pub slicings: &'static [Slicing],
    pub invariants: &'static [ProfileInvariant],
    /// How to build a resource for this profile from its variable inputs.
    pub lowering: &'static Lowering,
    /// Elements the IG flags `mustSupport`. Not a validation rule (no
    /// validator fails on it) — a conforming *client* is expected to
    /// populate them when it has the data, so gaps are reported separately
    /// as information (see [`crate::must_support_gaps`]).
    pub must_support: &'static [&'static str],
}
