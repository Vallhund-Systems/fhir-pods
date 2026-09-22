//! The validation engine — the domain service. Pure: a compiled profile and a
//! document in, a report out.

use alloc::vec::Vec;

use alloc::string::String;

use crate::compiled::{
    BindingKind, CompiledProfile, Discriminator, ElementConstraint, InvariantSeverity, Max, ProfileInvariant, Slice,
    Slicing,
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
    let bundle = (doc.scalars("$this.resourceType") == ["Bundle"]).then_some(doc);
    validate_at(profile, doc, doc, tx, 0, None, bundle)
}

/// Must-support elements that are absent where their parent is present.
/// A child under an absent parent is not a gap — the parent is.
pub fn must_support_gaps<D: Document>(profile: &CompiledProfile, doc: &D) -> Vec<&'static str> {
    profile.must_support.iter().copied().filter(|path| doc.child_counts(path).contains(&0)).collect()
}

/// `root` is the resource being validated (`%resource`) — the same node
/// as `doc` at the top level, and still the resource when `doc` is a
/// slice instance being checked against an inherited profile.
/// `bundle` is the document Bundle the resource sits in, if any — the scope
/// `resolve()` and profile discriminators on entries work within.
pub(crate) fn validate_at<D: Document>(
    profile: &CompiledProfile,
    doc: &D,
    root: &D,
    tx: &dyn TerminologyProvider,
    depth: usize,
    cache: Option<&crate::Validator>,
    bundle: Option<&D>,
) -> Report {
    validate_at_with(profile, doc, root, tx, depth, cache, bundle, None)
}

/// An invariant evaluated once per resource across the profiles applied to
/// it: (expression, path) pairs already done. A resource held to AU Core's
/// heart rate, the base heart rate and the base vital signs would otherwise
/// evaluate `dom-3`'s descendants walk three times.
pub(crate) type Evaluated = core::cell::RefCell<Vec<(&'static str, &'static str)>>;

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_at_with<D: Document>(
    profile: &CompiledProfile,
    doc: &D,
    root: &D,
    tx: &dyn TerminologyProvider,
    depth: usize,
    cache: Option<&crate::Validator>,
    bundle: Option<&D>,
    evaluated: Option<&Evaluated>,
) -> Report {
    let mut r = Report::default();
    for el in profile.elements {
        check_element(el, el.path, doc, tx, &mut r);
    }
    let scope = Scope { root, bundle, tx, depth, cache, evaluated };
    for slicing in profile.slicings {
        check_slicing(slicing, doc, &scope, "", &mut r);
    }
    // a slice the compiler could not discriminate: unenforced, and said so
    // wherever the resource actually reaches the sliced element — never a
    // silently weaker verdict
    for (element, name, why) in profile.deferred {
        // outer slice labels (`Composition.section:x.entry`) are the
        // profile's naming; presence is judged on the element itself
        let plain: String =
            element.split('.').map(|seg| seg.split(':').next().unwrap_or(seg)).collect::<Vec<_>>().join(".");
        if !doc.instances(&plain).is_empty() {
            r.push(
                Severity::Information,
                IssueType::NotSupported,
                &format!("{element}:{name}"),
                format!("slice '{name}' of {element} in {} was not checked: {why}", profile.name),
            );
        }
    }
    for inv in profile.invariants {
        check_invariant(inv, inv.path, doc, &scope, &mut r);
    }
    r
}

/// What a check needs beyond the node it looks at.
struct Scope<'s, D> {
    /// `%resource`.
    root: &'s D,
    /// The document Bundle around it, if any.
    bundle: Option<&'s D>,
    tx: &'s dyn TerminologyProvider,
    depth: usize,
    cache: Option<&'s crate::Validator>,
    /// Invariants already evaluated on this resource by another profile.
    evaluated: Option<&'s Evaluated>,
}

impl<D: Document> Scope<'_, D> {
    /// Validate `target` as a resource of its own (its own `%resource`)
    /// against a registry profile, inside the same bundle.
    fn conformance(&self, profile: &CompiledProfile, target: &D) -> Report {
        validate_at(profile, target, target, self.tx, self.depth + 1, self.cache, self.bundle)
    }

    /// What the Reference `inst` points at, inside the document.
    fn resolved(&self, inst: &D) -> Option<D> {
        let reference = inst.scalars("$this.reference").into_iter().next()?;
        path::resolve_reference(reference, Some(self.root), self.bundle)
    }
}

/// Evaluate one invariant on every instance of its path under `doc`,
/// reporting at `label`. Outside the supported FHIRPath core: skipped,
/// never guessed — and said so, once per invariant, so a silent gap never
/// reads as a pass.
fn check_invariant<D: Document>(inv: &ProfileInvariant, label: &str, doc: &D, scope: &Scope<'_, D>, r: &mut Report) {
    if let Some(done) = scope.evaluated {
        let mut done = done.borrow_mut();
        if done.iter().any(|(e, p)| *e == inv.expression && *p == inv.path) {
            return;
        }
        done.push((inv.expression, inv.path));
    }
    let owned;
    let expr: &path::Expr = match scope.cache {
        Some(v) => match v.expr(inv.expression) {
            Some(e) => e,
            None => return not_evaluated(inv, label, r),
        },
        None => match path::parse_constraint(inv.expression) {
            Ok(e) => {
                owned = e;
                &owned
            }
            Err(_) => return not_evaluated(inv, label, r),
        },
    };
    let mut skipped = false;
    for (ctx, this_type) in invariant_contexts(inv, doc) {
        match path::check_constraint_in(expr, &ctx, this_type, Some(scope.root), scope.bundle) {
            Verdict::Fail => {
                let severity = match inv.severity {
                    InvariantSeverity::Error => Severity::Error,
                    InvariantSeverity::Warning => Severity::Warning,
                };
                r.push(severity, IssueType::Invariant, label, format!("{}: {}", inv.key, inv.human));
            }
            Verdict::Unsupported => skipped = true,
            Verdict::Pass => {}
        }
    }
    if skipped {
        not_evaluated(inv, label, r);
    }
}

fn not_evaluated(inv: &ProfileInvariant, label: &str, r: &mut Report) {
    r.push(
        Severity::Information,
        IssueType::NotSupported,
        label,
        format!("{} not evaluated: outside the supported FHIRPath core", inv.key),
    );
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
fn check_element<D: Document>(
    el: &ElementConstraint,
    label: &str,
    doc: &D,
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
    // a choice element only takes the types the profile allows
    if let Some((parent, stem)) = el.path.rsplit_once('.').and_then(|(p, l)| l.strip_suffix("[x]").map(|s| (p, s))) {
        if !el.types.is_empty() {
            for inst in doc.instances(parent) {
                for key in inst.choice_keys(stem) {
                    let suffix = &key[stem.len()..];
                    let allowed = el.types.iter().any(|t| {
                        suffix.len() == t.len() && suffix[..1].eq_ignore_ascii_case(&t[..1]) && suffix[1..] == t[1..]
                    });
                    if !allowed {
                        r.push(
                            Severity::Error,
                            IssueType::Structure,
                            label,
                            format!("{label} allows the types [{}]; found {key}", el.types.join(", ")),
                        );
                    }
                }
            }
        }
    }
    if !el.patterns.is_empty() {
        for inst in doc.instances(el.path) {
            for (anchor, pairs) in el.patterns {
                let carries = |n: &D| {
                    pairs.iter().all(|(rel, value)| {
                        let mut p = String::from("$this");
                        if !rel.is_empty() {
                            p.push('.');
                            p.push_str(rel);
                        }
                        n.scalars(&p).contains(value)
                    })
                };
                let ok = if anchor.is_empty() {
                    carries(&inst)
                } else {
                    let mut p = String::from("$this.");
                    p.push_str(anchor);
                    inst.instances(&p).iter().any(carries)
                };
                if !ok {
                    let wanted = pairs.iter().map(|(rel, v)| format!("{rel}={v}")).collect::<Vec<_>>().join(", ");
                    r.push(
                        Severity::Error,
                        IssueType::Value,
                        label,
                        format!(
                            "{label} does not carry the profile's pattern ({}{wanted})",
                            if anchor.is_empty() { String::new() } else { format!("{anchor}: ") }
                        ),
                    );
                }
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
        Membership::NotMember => {
            r.push(Severity::Error, IssueType::CodeInvalid, label, format!("'{what}' is not in the bound value set"))
        }
        Membership::Unknown => r.push(
            Severity::Warning,
            IssueType::NotSupported,
            label,
            format!("no bundled terminology for {vs}; '{what}' not checked"),
        ),
    }
}

/// Does this instance belong to the slice? Value/pattern discriminators must
/// all match; type slices are selected by key in `check_slicing`; a Bundle
/// entry is discriminated by its resource's type, or by conformance to a
/// profile — a validation pass within the same bundle.
fn matches<D: Document>(slice: &Slice, inst: &D, scope: &Scope<'_, D>) -> bool {
    slice.discriminators.iter().all(|d| match d {
        Discriminator::ResourceType { path, ty } => {
            inst.instances(path).iter().any(|res| res.scalars("$this.resourceType") == [*ty])
        }
        Discriminator::Profile { path, url } => {
            let Some(profile) = crate::REGISTRY.iter().find(|p| p.url == *url) else { return false };
            scope.depth < MAX_INHERIT
                && inst.instances(path).iter().any(|res| {
                    res.scalars("$this.resourceType") == [profile.resource_type]
                        && scope.conformance(profile, res).is_valid()
                })
        }
        Discriminator::ResolvedType { types } => scope
            .resolved(inst)
            .is_some_and(|target| target.scalars("$this.resourceType").iter().any(|t| types.contains(t))),
        Discriminator::ResolvedProfile { urls } => {
            scope.depth < MAX_INHERIT
                && scope.resolved(inst).is_some_and(|target| {
                    urls.iter().filter_map(|u| crate::REGISTRY.iter().find(|p| p.url == *u)).any(|p| {
                        target.scalars("$this.resourceType") == [p.resource_type]
                            && scope.conformance(p, &target).is_valid()
                    })
                })
        }
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

/// `Bundle`'s own invariants (bdl-1 … bdl-12) on a Bundle no compiled
/// profile applies to.
pub(crate) fn bundle_invariants<D: Document>(doc: &D, cache: Option<&crate::Validator>, bundle: Option<&D>) -> Report {
    let mut r = Report::default();
    let scope = Scope { root: doc, bundle, tx: &Bundled, depth: 0, cache, evaluated: None };
    for (path, key, is_error, human, expression) in crate::typemap::BUNDLE_INVARIANTS {
        let inv = ProfileInvariant {
            path,
            key,
            severity: if *is_error { InvariantSeverity::Error } else { InvariantSeverity::Warning },
            human,
            expression,
            types: &[],
        };
        check_invariant(&inv, path, doc, &scope, &mut r);
    }
    r
}

/// `DomainResource`'s own invariants on a resource no compiled profile
/// applies to — the ones the shape walk does not already enforce.
pub(crate) fn domain_invariants<D: Document>(
    doc: &D,
    rt: &'static str,
    cache: Option<&crate::Validator>,
    bundle: Option<&D>,
) -> Report {
    let mut r = Report::default();
    let scope = Scope { root: doc, bundle, tx: &Bundled, depth: 0, cache, evaluated: None };
    for (key, is_error, human, expression) in crate::typemap::DOMAIN_INVARIANTS {
        if matches!(*key, "dom-2" | "dom-4" | "dom-5") {
            continue;
        }
        let inv = ProfileInvariant {
            path: rt,
            key,
            severity: if *is_error { InvariantSeverity::Error } else { InvariantSeverity::Warning },
            human,
            expression,
            types: &[],
        };
        check_invariant(&inv, rt, doc, &scope, &mut r);
    }
    r
}

/// Every `reference` under `node`, with the path it sits at.
fn references_in<D: Document>(node: &D, path: &str, out: &mut Vec<(String, String)>) {
    for r in node.scalars("$this.reference") {
        out.push((String::from(path), String::from(r)));
    }
    for child in node.all_children() {
        references_in(&child, path, out);
    }
}

/// What a document Bundle owes beyond its profile, as the reference
/// validator has it: the Composition's references resolve inside the
/// bundle, and every entry is reachable from the Composition by following
/// references forward or backward.
pub(crate) fn check_document<D: Document>(bundle: &D, r: &mut Report) {
    // a REST-shaped fullUrl names the resource it carries: its last two
    // segments are the resource's type and id. Other URI schemes carry an
    // opaque identity, rather than a FHIR REST resource URL.
    let bundle_type = bundle.scalars("$this.type").first().copied().unwrap_or("");
    for (i, entry) in bundle.instances("$this.entry").iter().enumerate() {
        let Some(full) = entry.scalars("$this.fullUrl").first().copied() else {
            // an entry carrying a resource names it — except a transaction's or
            // batch's create, whose resource has no identity yet
            let has_resource = !entry.scalars("$this.resource.resourceType").is_empty();
            let creates = matches!(bundle_type, "transaction" | "batch")
                && entry.scalars("$this.request.method").first().copied() == Some("POST");
            // a response's entries answer requests; their resources need no identity of their own
            let responds = matches!(bundle_type, "transaction-response" | "batch-response");
            if has_resource && !creates && !responds {
                r.push(
                    Severity::Error,
                    IssueType::Required,
                    &format!("Bundle.entry[{i}]"),
                    String::from("Bundle entry missing fullUrl"),
                );
            }
            continue;
        };
        if !full
            .split_once("://")
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        {
            continue;
        }
        let rt = entry.scalars("$this.resource.resourceType").first().copied();
        let id = entry.scalars("$this.resource.id").first().copied();
        if let (Some(rt), Some(id)) = (rt, id) {
            let mut segs = full.trim_end_matches('/').rsplit('/');
            let (last, before) = (segs.next(), segs.next());
            if last != Some(id) || before != Some(rt) {
                r.push(
                    Severity::Error,
                    IssueType::Invariant,
                    &format!("Bundle.entry[{i}].fullUrl"),
                    format!("fullUrl '{full}' does not end in the resource's type and id ({rt}/{id})"),
                );
            }
        }
    }
    if bundle.scalars("$this.type") != ["document"] {
        return;
    }
    let entries = bundle.child_instances("entry");
    let resources: Vec<Option<D>> = entries.iter().map(|e| e.child_instances("resource").into_iter().next()).collect();
    let Some(Some(composition)) = resources.first() else { return };
    let index_of = |reference: &str| -> Option<usize> {
        let target = path::resolve_reference(reference, Some(composition), Some(bundle))?;
        resources.iter().position(|res| res.as_ref().is_some_and(|x| x.same(&target)))
    };
    // the Composition's own references
    let mut refs = Vec::new();
    references_in(composition, "Bundle.entry[0].resource", &mut refs);
    for (path, reference) in &refs {
        if !reference.starts_with('#') && index_of(reference).is_none() {
            r.push(
                Severity::Error,
                IssueType::Structure,
                path,
                format!("reference '{reference}' is not in the bundle (a document is self-contained)"),
            );
        }
    }
    // reachability: forward and backward links, to a fixed point
    let links: Vec<Vec<usize>> = resources
        .iter()
        .map(|res| {
            let mut out = Vec::new();
            if let Some(res) = res {
                references_in(res, "", &mut out);
            }
            out.iter().filter_map(|(_, reference)| index_of(reference)).collect()
        })
        .collect();
    let mut reachable = vec![false; resources.len()];
    reachable[0] = true;
    loop {
        let mut grew = false;
        for i in 0..resources.len() {
            if reachable[i] {
                for &j in &links[i] {
                    if !reachable[j] {
                        reachable[j] = true;
                        grew = true;
                    }
                }
            } else if links[i].iter().any(|&j| reachable[j]) {
                reachable[i] = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    for (i, ok) in reachable.iter().enumerate() {
        if !ok {
            r.push(
                Severity::Error,
                IssueType::Structure,
                &format!("Bundle.entry[{i}]"),
                format!("Bundle.entry[{i}] is not reachable from the Composition by following references"),
            );
        }
    }
}

/// Re-label a profile's findings under the element they were checked at.
fn forward(rep: Report, profile: &CompiledProfile, label: &str, r: &mut Report) {
    for issue in rep.issues {
        let rel =
            issue.path.strip_prefix(profile.resource_type).map(|s| s.trim_start_matches('.')).unwrap_or(&issue.path);
        let path = if rel.is_empty() { String::from(label) } else { format!("{label}.{rel}") };
        r.push(issue.severity, issue.code, &path, issue.message);
    }
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
fn check_slicing<D: Document>(slicing: &Slicing, doc: &D, scope: &Scope<'_, D>, prefix: &str, r: &mut Report) {
    let Some((parent_path, last)) = slicing.path.rsplit_once('.') else {
        return;
    };
    let (root, tx, depth, cache) = (scope.root, scope.tx, scope.depth, scope.cache);
    // the reported path: absolute at the top, `Outer:slice.inner` when nested
    let base = if prefix.is_empty() {
        String::from(slicing.path)
    } else {
        format!("{prefix}.{}", slicing.path.trim_start_matches("$this."))
    };
    for parent in doc.instances(parent_path) {
        let all = parent.child_instances(last);
        let mut matched = 0usize;
        // which slices each instance matched: more than one is an error
        let mut hits: Vec<Vec<&str>> = vec![Vec::new(); all.len()];
        let mut label = String::new();
        for slice in slicing.slices {
            label.clear();
            label.push_str(&base);
            label.push(':');
            label.push_str(slice.name);
            let insts: Vec<_> = match type_key(slice) {
                Some(key) => parent.child_instances(key),
                None => all
                    .iter()
                    .enumerate()
                    .filter(|(i, inst)| {
                        let hit = matches(slice, *inst, scope);
                        if hit {
                            hits[*i].push(slice.name);
                        }
                        hit
                    })
                    .map(|(_, inst)| inst.clone())
                    .collect(),
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
                for inv in slice.invariants {
                    label.truncate(base_len);
                    if let Some(rel) = inv.path.strip_prefix("$this.") {
                        label.push('.');
                        label.push_str(rel);
                    }
                    check_invariant(inv, &label, inst, scope, r);
                }
                label.truncate(base_len);
                if let Some(url) = slice.profile.filter(|_| depth < MAX_INHERIT) {
                    if let Some(profile) = crate::REGISTRY.iter().find(|p| p.url == url) {
                        let rep = validate_at(profile, inst, root, tx, depth + 1, cache, scope.bundle);
                        forward(rep, profile, &label, r);
                    }
                }
                // slicings declared inside this slice, on this instance
                for nested in slice.slicings {
                    label.truncate(base_len);
                    check_slicing(nested, inst, scope, &label, r);
                }
                // a sub-element that must conform to one of some profiles (a
                // Bundle entry's resource): checked only when every one is compiled
                for (rel, urls) in slice.profiles {
                    let known: Vec<&CompiledProfile> =
                        urls.iter().filter_map(|u| crate::REGISTRY.iter().find(|p| p.url == *u).copied()).collect();
                    if known.len() != urls.len() || depth >= MAX_INHERIT {
                        continue;
                    }
                    label.truncate(base_len);
                    label.push('.');
                    label.push_str(rel.trim_start_matches("$this."));
                    for target in inst.instances(rel) {
                        let reports: Vec<(&CompiledProfile, Report)> = known
                            .iter()
                            .filter(|p| target.scalars("$this.resourceType") == [p.resource_type])
                            .map(|p| (*p, scope.conformance(p, &target)))
                            .collect();
                        if reports.iter().any(|(_, rep)| rep.is_valid()) {
                            continue;
                        }
                        match reports.as_slice() {
                            [(profile, rep)] => forward(rep.clone(), profile, &label, r),
                            [] => r.push(
                                Severity::Error,
                                IssueType::Structure,
                                &label,
                                format!("{label} is not one of the resource types this entry allows"),
                            ),
                            many => r.push(
                                Severity::Error,
                                IssueType::Structure,
                                &label,
                                format!(
                                    "{label} conforms to none of [{}]",
                                    many.iter().map(|(p, _)| p.name).collect::<Vec<_>>().join(", ")
                                ),
                            ),
                        }
                    }
                }
            }
        }
        for names in hits.iter().filter(|h| h.len() > 1) {
            r.push(
                Severity::Error,
                IssueType::Structure,
                &base,
                format!("{base} has an element matching more than one slice ({})", names.join(", ")),
            );
        }
        if slicing.closed && matched < all.len() {
            r.push(
                Severity::Error,
                IssueType::Structure,
                &base,
                format!("{base} has {} element(s) matching no slice (closed slicing)", all.len() - matched),
            );
        }
    }
}
