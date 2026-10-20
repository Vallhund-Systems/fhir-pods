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
    /// A `pattern[x]` / `fixed[x]` object, as leaf groups: for each
    /// (anchor, pairs), some ONE instance at `anchor` (relative; `""` is
    /// the element itself) must carry every pair. A `patternCodeableConcept`
    /// with one coding is one group anchored at `coding`.
    pub patterns: &'static [(&'static str, &'static [(&'static str, &'static str)])],
    /// A base datatype profile the profile puts on one of the element's
    /// types where the base definition of the element does not (an
    /// extension's `value[x]` typed `Quantity` with `SimpleQuantity`):
    /// (type code, canonical), each applied whole from
    /// `typemap::DATATYPE_PROFILES`. One the base already names is the
    /// shape walk's (`typemap::ELEMENT_PROFILES`), and is not listed here.
    pub datatype_profiles: &'static [(&'static str, &'static str)],
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
    /// The resource at `path` (`$this.resource` of a Bundle entry) is of
    /// this resource type.
    ResourceType { path: &'static str, ty: &'static str },
    /// The resource at `path` conforms to the registry profile at `url` —
    /// the discriminator is a validation pass.
    Profile { path: &'static str, url: &'static str },
    /// The slice instance is a Reference whose target (resolved inside the
    /// document) is of one of these resource types.
    ResolvedType { types: &'static [&'static str] },
    /// … whose target conforms to one of these registry profiles.
    ResolvedProfile { urls: &'static [&'static str] },
    /// The coded value at `path` (`$this` for the instance itself) is a
    /// member of the binding's value set — a slice with nothing fixed and a
    /// `required` binding where its `value`/`pattern` discriminator points
    /// (AU Core's AMT and PBS codings, US Core's category slices). Decided
    /// through the terminology provider; a set the provider cannot decide
    /// leaves the instance unassigned — neither a match nor a mismatch —
    /// and the report says so.
    Member { path: &'static str, binding: Binding },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Slice {
    pub name: &'static str,
    pub discriminators: &'static [Discriminator],
    pub min: u32,
    pub max: Max,
    /// Constraints on the slice's sub-elements, paths relative as `$this.…`.
    pub elements: &'static [ElementConstraint],
    /// Invariants declared on the slice itself (`$this`) or its sub-elements
    /// (`$this.…`) — evaluated on the slice's instances only.
    pub invariants: &'static [ProfileInvariant],
    /// Profiles a sub-element must conform to one of (`$this.resource` of a
    /// Bundle entry). Checked only when every listed profile is compiled;
    /// a list naming one that is not is skipped, never guessed.
    pub profiles: &'static [(&'static str, &'static [&'static str])],
    /// Slicings declared inside this slice (`section:sectionProblems.entry`),
    /// paths relative as `$this.…`, evaluated on the slice's instances.
    pub slicings: &'static [Slicing],
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
    /// The profile this one derives from (`baseDefinition`, versionless):
    /// its snapshot carries the base's constraints, so a base profile need
    /// not be applied alongside it.
    pub base: Option<&'static str>,
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
    /// Where an extension may sit (`StructureDefinition.context`): the kind
    /// (`element`, `fhirpath`, `extension`) and the expression — an element
    /// path (`Patient.birthDate`), a type (`ContactPoint`, `Element`), `*`,
    /// or for `extension` the url of the extension it nests in. Empty for
    /// anything but an extension profile.
    pub contexts: &'static [(&'static str, &'static str)],
    /// Reference elements and the profiles their targets must conform to
    /// (`ElementDefinition.type.targetProfile`): (absolute element path,
    /// canonical urls). Checked when the reference resolves inside the
    /// document — a bundle entry, a contained resource — against every
    /// target the registry has compiled (a base resource canonical is a
    /// type check); a reference nothing here can resolve, or targets none
    /// of which are compiled, are not judged. The reference validator's
    /// "Unable to find a profile match for … among choices".
    pub references: &'static [(&'static str, &'static [&'static str])],
    /// Slices the compiler could not discriminate (a discriminator on a
    /// binding rather than a fixed value, on what a reference resolves to
    /// beyond the registry, on `$this` with no type; a slicing nested three
    /// levels deep): (the sliced element's absolute path, the slice's name,
    /// why). Not enforced — and a resource that reaches the element is told
    /// so with an information issue, so `valid` is never quietly weaker than
    /// it looks. See `docs/scope.md`.
    pub deferred: &'static [(&'static str, &'static str, &'static str)],
}

impl CompiledProfile {
    /// The pods' `slots` JSON: `{"slots": [{"path", "type", "choices",
    /// "min", "max", "mustSupport", "fixed": [{"path", "value"}], "parts":
    /// [{"name", "type", "repeats", "required"}]}]}` — `fixed` is what the
    /// profile fixes beside the slot under the same parent (the UCUM `code`
    /// beside a Quantity's `unit`), `parts` the members of a composite
    /// datatype slot (a `HumanName`'s `family`, `given`…) from the base
    /// element table, so a host can fill either without the specification.
    pub fn slots_json(&self) -> alloc::string::String {
        use crate::wire::escape;
        use alloc::string::ToString;
        let mut out = alloc::string::String::from("{\"slots\":[");
        for (i, s) in self.lowering.slots.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"path\":\"");
            escape(&mut out, s.path);
            out.push_str("\",\"type\":\"");
            escape(&mut out, s.ty);
            out.push_str("\",\"choices\":[");
            for (j, c) in s.choices.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                out.push('"');
                escape(&mut out, c);
                out.push('"');
            }
            out.push_str("],\"min\":");
            out.push_str(&s.min.to_string());
            out.push_str(",\"max\":");
            match s.max {
                Max::Bounded(n) => out.push_str(&n.to_string()),
                Max::Star => out.push_str("null"),
            }
            out.push_str(",\"mustSupport\":");
            out.push_str(if s.must_support { "true" } else { "false" });
            // the fixed leaves beside this slot: same parent, or — for a
            // composite slot — beneath it
            out.push_str(",\"fixed\":[");
            let parent = s.path.rsplit_once('.').map(|(p, _)| p);
            let mut first = true;
            for (path, value) in self.lowering.fixed {
                let beside = parent.is_some_and(|p| path.rsplit_once('.').is_some_and(|(q, _)| q == p));
                let beneath = path.strip_prefix(s.path).is_some_and(|r| r.starts_with('.'));
                if !(beside || beneath) {
                    continue;
                }
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str("{\"path\":\"");
                escape(&mut out, path);
                out.push_str("\",\"value\":\"");
                escape(&mut out, value);
                out.push_str("\"}");
            }
            out.push_str("],\"parts\":[");
            for (j, (name, codes, repeats, required)) in crate::typing::members(s.ty).iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                out.push_str("{\"name\":\"");
                escape(&mut out, name);
                out.push_str("\",\"type\":\"");
                escape(&mut out, codes.trim_start_matches(['#', '~']));
                out.push_str("\",\"repeats\":");
                out.push_str(if *repeats { "true" } else { "false" });
                out.push_str(",\"required\":");
                out.push_str(if *required { "true" } else { "false" });
                out.push('}');
            }
            out.push_str("]}");
        }
        out.push_str("]}");
        out
    }
}
