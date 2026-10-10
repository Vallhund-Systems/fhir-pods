//! The validation engine — the domain service. Pure: a compiled profile and a
//! document in, a report out.

use alloc::vec::Vec;

use alloc::string::String;

use crate::compiled::{
    Binding, BindingKind, CompiledProfile, Discriminator, ElementConstraint, InvariantSeverity, Max, ProfileInvariant,
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
pub(crate) struct Evaluated<'m, D> {
    done: core::cell::RefCell<Vec<(&'static str, &'static str)>>,
    /// What one validation of a whole document remembers across its resources.
    memo: Option<&'m Memo<D>>,
}

impl<'m, D> Evaluated<'m, D> {
    pub(crate) fn new(memo: Option<&'m Memo<D>>) -> Self {
        Evaluated { done: core::cell::RefCell::new(Vec::new()), memo }
    }
}

/// What one validation of a document remembers across its resources: the
/// bundle's entries indexed by identity (a reference is resolved by lookup,
/// not by a walk over every entry), and each target's conformance to each
/// profile it was held to (a Patient referenced by a thousand Observations
/// is validated once). A memo lives for one validation; nothing outlives it.
/// A bundle entry as the memo indexes it: fullUrl, resource type, id,
/// versionId, and the resource.
type Entry<D> = (String, String, String, String, D);

pub(crate) struct Memo<D> {
    /// The bundle's entries, once built.
    entries: core::cell::RefCell<Option<Vec<Entry<D>>>>,
    /// (target identity, profile address, valid?)
    conformance: core::cell::RefCell<Vec<(usize, usize, bool)>>,
}

impl<D> Memo<D> {
    pub(crate) fn new() -> Self {
        Memo { entries: core::cell::RefCell::new(None), conformance: core::cell::RefCell::new(Vec::new()) }
    }
}

impl<D: Document> Memo<D> {
    fn entries<'a>(&'a self, bundle: &D) -> core::cell::Ref<'a, Vec<Entry<D>>> {
        if self.entries.borrow().is_none() {
            let mut out = Vec::new();
            for entry in bundle.child_instances("entry") {
                let full = entry.scalars("$this.fullUrl").first().map(|s| String::from(*s)).unwrap_or_default();
                let Some(res) = entry.child_instances("resource").into_iter().next() else { continue };
                let rt = res.scalars("$this.resourceType").first().map(|s| String::from(*s)).unwrap_or_default();
                let id = res.scalars("$this.id").first().map(|s| String::from(*s)).unwrap_or_default();
                let version = res.scalars("$this.meta.versionId").first().map(|s| String::from(*s)).unwrap_or_default();
                out.push((full, rt, id, version, res));
            }
            *self.entries.borrow_mut() = Some(out);
        }
        core::cell::Ref::map(self.entries.borrow(), |e| e.as_ref().expect("built above"))
    }

    /// [`path::resolve_reference`] by index: the same answer, without the walk.
    fn resolve(&self, reference: &str, bundle: &D) -> Option<D> {
        let (identity, version) = match reference.rsplit_once("/_history/") {
            Some((identity, version)) if !version.is_empty() && !version.contains('/') => (identity, Some(version)),
            Some(_) => return None,
            None => (reference, None),
        };
        let relative = identity.split_once('/').filter(|(ty, id)| !ty.is_empty() && !id.contains('/'));
        let entries = self.entries(bundle);
        for (full, rt, id, v, res) in entries.iter() {
            if version.is_some_and(|want| v != want) {
                continue;
            }
            if full == identity {
                return Some(res.clone());
            }
            if let Some((ty, rid)) = relative {
                let ends = full.strip_suffix(rid).and_then(|s| s.strip_suffix('/')).is_some_and(|s| s.ends_with(ty));
                if ends || (rt == ty && id == rid) {
                    return Some(res.clone());
                }
            }
        }
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_at_with<D: Document>(
    profile: &CompiledProfile,
    doc: &D,
    root: &D,
    tx: &dyn TerminologyProvider,
    depth: usize,
    cache: Option<&crate::Validator>,
    bundle: Option<&D>,
    evaluated: Option<&Evaluated<'_, D>>,
) -> Report {
    let mut r = Report::default();
    for el in profile.elements {
        check_element(el, el.path, doc, tx, &mut r);
    }
    let scope = Scope { root, bundle, tx, depth, cache, evaluated };
    for slicing in profile.slicings {
        check_slicing(slicing, doc, &scope, "", &mut r);
    }
    // a reference must point at what its element allows: judged where the
    // target is in reach (a bundle entry, a contained resource) and at
    // least one allowed profile is compiled
    for (path, targets) in profile.references {
        check_reference_targets(path, targets, doc, &scope, &mut r);
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

/// The reference validator's rule for a Reference element with target
/// profiles: a reference that resolves inside the document must point at a
/// resource of an allowed type that conforms to one of the allowed profiles
/// the registry has compiled. A base resource canonical is a type check
/// alone; targets none of which are compiled, or a reference out of reach,
/// are not judged.
fn check_reference_targets<D: Document>(
    path: &str,
    targets: &[&'static str],
    doc: &D,
    scope: &Scope<'_, D>,
    r: &mut Report,
) {
    const BASE: &str = "http://hl7.org/fhir/StructureDefinition/";
    // (resource type, compiled profile or None for a base canonical)
    let allowed: Vec<(&'static str, Option<&'static CompiledProfile>)> = targets
        .iter()
        .filter_map(|url| {
            let compiled: Option<&'static CompiledProfile> = crate::REGISTRY.iter().find(|p| p.url == *url).copied();
            if let Some(ty) = url.strip_prefix(BASE) {
                // a base resource type as the target: the type is the whole
                // test (the base definition is compiled for lowering, but a
                // target is never held to it beyond being that type)
                return match ty {
                    "Resource" | "DomainResource" => None,
                    t => Some(compiled.map(|p| (p.resource_type, None)).unwrap_or((t, None))),
                };
            }
            compiled.map(|p| (p.resource_type, Some(p)))
        })
        .collect();
    if allowed.is_empty() || scope.depth >= MAX_INHERIT {
        return;
    }
    // `Resource` among the targets: anything goes
    if targets.iter().any(|u| u.ends_with("/StructureDefinition/Resource")) {
        return;
    }
    for (i, inst) in doc.instances(path).iter().enumerate() {
        // a declared `type` outside the profile's targets is wrong before
        // any resolution — and so is a canonical where a type name belongs;
        // judged only beside a `reference` (probed, 6.10.4)
        if let Some(t) = inst.scalars("$this.type").into_iter().next() {
            let referenced = !inst.scalars("$this.reference").is_empty();
            if referenced && !allowed.iter().any(|(ty, _)| *ty == t) {
                r.push(
                    Severity::Error,
                    IssueType::Structure,
                    &format!("{path}[{i}]"),
                    format!(
                        "Invalid Resource target type. Found {t}, but expected one of ({})",
                        allowed.iter().map(|(ty, _)| *ty).collect::<Vec<_>>().join(", ")
                    ),
                );
            }
        }
        let Some(reference) = inst.scalars("$this.reference").into_iter().next() else { continue };
        let Some(target) = scope.resolved(inst) else { continue };
        let rt = target.scalars("$this.resourceType");
        let Some(rt) = rt.first().copied() else { continue };
        let of_type: Vec<&(&'static str, Option<&'static CompiledProfile>)> =
            allowed.iter().filter(|(t, _)| *t == rt).collect();
        let label = format!("{path}[{i}]");
        if of_type.is_empty() {
            r.push(
                Severity::Error,
                IssueType::Structure,
                &label,
                format!(
                    "{path} points at a {rt}, not one of the types it allows ({})",
                    allowed.iter().map(|(t, _)| *t).collect::<Vec<_>>().join(", ")
                ),
            );
            continue;
        }
        // a base type among the allowed of this type: the type is enough —
        // and so is it for a contained resource (`#id`): the reference
        // validator judges a contained target by type, a bundle entry by
        // conformance (its 6.10.4 behaviour on the corpus; matched, not
        // second-guessed)
        if of_type.iter().any(|(_, p)| p.is_none()) || reference.starts_with('#') {
            continue;
        }
        // a target conforms when the profile's own constraints hold and the
        // resource is sound on its own (the reference validator's whole
        // verdict on the target, not the profile's alone)
        let conforms = of_type.iter().any(|(_, p)| p.is_some_and(|p| scope.conforms(p, &target)));
        if !conforms {
            r.push(
                Severity::Error,
                IssueType::Structure,
                &label,
                format!(
                    "Unable to find a profile match for {reference} among choices: {}",
                    of_type.iter().filter_map(|(_, p)| p.map(|p| p.url)).collect::<Vec<_>>().join(", ")
                ),
            );
        }
    }
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
    evaluated: Option<&'s Evaluated<'s, D>>,
}

impl<D: Document> Scope<'_, D> {
    fn memo(&self) -> Option<&Memo<D>> {
        self.evaluated.and_then(|e| e.memo)
    }

    /// Validate `target` as a resource of its own (its own `%resource`)
    /// against a registry profile, inside the same bundle.
    fn conformance(&self, profile: &CompiledProfile, target: &D) -> Report {
        let fresh = Evaluated::new(self.memo());
        validate_at_with(profile, target, target, self.tx, self.depth + 1, self.cache, self.bundle, Some(&fresh))
    }

    /// Does `target` conform to `profile` — its constraints and the
    /// resource's own shape — remembered per target for the validation.
    fn conforms(&self, profile: &CompiledProfile, target: &D) -> bool {
        let key = (target.identity(), profile as *const CompiledProfile as usize);
        if key.0 != 0 {
            if let Some(memo) = self.memo() {
                if let Some((_, _, ok)) = memo.conformance.borrow().iter().find(|(t, p, _)| (*t, *p) == key) {
                    return *ok;
                }
            }
        }
        let ok = self.conformance(profile, target).is_valid() && target.shape_ok(self.tx);
        if key.0 != 0 {
            if let Some(memo) = self.memo() {
                memo.conformance.borrow_mut().push((key.0, key.1, ok));
            }
        }
        ok
    }

    /// What the Reference `inst` points at, inside the document.
    fn resolved(&self, inst: &D) -> Option<D> {
        let reference = inst.scalars("$this.reference").into_iter().next()?;
        if !reference.starts_with('#') {
            if let (Some(memo), Some(bundle)) = (self.memo(), self.bundle) {
                return memo.resolve(reference, bundle);
            }
        }
        path::resolve_reference(reference, Some(self.root), self.bundle)
    }
}

/// Evaluate one invariant on every instance of its path under `doc`,
/// reporting at `label`. Outside the supported FHIRPath core: skipped,
/// never guessed — and said so, once per invariant, so a silent gap never
/// reads as a pass.
/// dom-3, as the reference validator reports it: one finding per contained
/// resource that nothing in its container refers to and that does not
/// refer to its container, located at the contained resource — not one
/// FHIRPath verdict at the root.
fn contained_orphans<D: Document>(label: &str, doc: &D, scope: &Scope<'_, D>, r: &mut Report) {
    for (i, contained) in doc.instances("$this.contained").iter().enumerate() {
        let Some(id) = contained.scalars("$this.id").into_iter().next() else { continue };
        if id.chars().any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))) {
            continue; // not an id; the shape walk has said so
        }
        let referenced = format!(
            "(descendants().reference | descendants().as(canonical) | descendants().as(uri) | descendants().as(url)).where($this = '#{id}').exists()"
        );
        let refers_back = "descendants().where(reference = '#').exists()";
        let verdict = |expr: &str, ctx: &D| match path::parse_constraint(expr) {
            Ok(e) => path::check_constraint_in(&e, ctx, None, Some(scope.root), scope.bundle),
            Err(_) => Verdict::Unsupported,
        };
        // a narrative link to it (`<a href="#id">`, `<img src="#id">`) is a
        // reference too, from the resource's narrative or a section's (probed)
        let linked = || {
            ["$this.text.div", "$this.section.text.div", "$this.section.section.text.div"]
                .iter()
                .flat_map(|sel| doc.scalars(sel))
                .any(|div| crate::xhtml::fragments(div).iter().any(|f| *f == id))
        };
        if let (Verdict::Fail, Verdict::Fail) = (verdict(&referenced, doc), verdict(refers_back, contained)) {
            if linked() {
                continue;
            }
            r.push(
                Severity::Error,
                IssueType::Invariant,
                &format!("{label}.contained[{i}]"),
                format!(
                    "dom-3: The contained resource '{id}' is not referenced to from elsewhere in the containing resource nor does it refer to the containing resource"
                ),
            );
        }
    }
}

fn check_invariant<D: Document>(inv: &ProfileInvariant, label: &str, doc: &D, scope: &Scope<'_, D>, r: &mut Report) {
    // the walk checks every Narrative.div for txt-1 and txt-2 (a profile
    // restating them would report each twice — and R4's txt-2 expression is
    // `htmlChecks()`, which would fail a narrative full of text)
    if inv.key == "txt-1" || inv.key == "txt-2" {
        return;
    }
    if inv.key == "dom-3" {
        if let Some(done) = scope.evaluated {
            let mut done = done.done.borrow_mut();
            if done.iter().any(|(e, p)| *e == inv.expression && *p == inv.path) {
                return;
            }
            done.push((inv.expression, inv.path));
        }
        return contained_orphans(label, doc, scope, r);
    }
    if let Some(done) = scope.evaluated {
        let mut done = done.done.borrow_mut();
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
        None => match path::parse_invariant(inv.key, inv.expression) {
            Ok(e) => {
                owned = e;
                &owned
            }
            Err(_) => return not_evaluated(inv, label, r),
        },
    };
    let mut skipped = false;
    for (ctx, this_type, at) in invariant_contexts(inv, doc) {
        match path::check_constraint_in(expr, &ctx, this_type, Some(scope.root), scope.bundle) {
            Verdict::Fail => {
                let severity = match inv.severity {
                    InvariantSeverity::Error => Severity::Error,
                    InvariantSeverity::Warning => Severity::Warning,
                };
                // a deeper copy of the backbone is reported where it is
                let label = at.as_deref().unwrap_or(label);
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
fn invariant_contexts<'a, D: Document>(
    inv: &'a ProfileInvariant,
    doc: &D,
) -> Vec<(D, Option<&'a str>, Option<String>)> {
    match inv.path.strip_suffix("[x]") {
        Some(base) if !inv.types.is_empty() => inv
            .types
            .iter()
            .flat_map(|t| {
                let mut key = String::from(base);
                key.push_str(&t[..1].to_ascii_uppercase());
                key.push_str(&t[1..]);
                doc.instances(&key).into_iter().map(move |d| (d, Some(*t), None))
            })
            .collect(),
        _ => {
            let ty = inv.types.first().copied();
            let mut out = Vec::new();
            for at in invariant_paths(inv.path, doc) {
                let label = (at != inv.path).then(|| at.clone());
                out.extend(doc.instances(&at).into_iter().map(|d| (d, ty, label.clone())));
            }
            out
        }
    }
}

/// Every path an invariant defined at `path` applies to: the path itself,
/// and the path again beneath each copy of a backbone it lies on or under
/// that the base defines by `contentReference` — a section's section, an
/// item's item, an answer's item, a component's referenceRange — however
/// deep the document nests them. The reference validator evaluates an
/// invariant on every copy; so does this. A copy counts only where the
/// document has an instance of it, which bounds the walk by the document.
fn invariant_paths<D: Document>(path: &str, doc: &D) -> Vec<String> {
    let mut out = alloc::vec![String::from(path)];
    let segs: Vec<&str> = path.split('.').collect();
    let mut ctx = crate::typing::known_type(segs[0]);
    for k in 1..=segs.len() {
        if k > 1 {
            ctx = ctx.and_then(|c| crate::typing::child(c, segs[k - 1]).map(|ch| ch.ctx));
        }
        let Some(backbone) = ctx else { break };
        if !backbone.contains('.') {
            continue;
        }
        let rest = segs[k..].join(".");
        for copy in backbone_copies(backbone, &segs[..k].join("."), doc) {
            let full = if rest.is_empty() { copy } else { format!("{copy}.{rest}") };
            // a copy carries the backbone's invariants and its own: a test
            // operation fails tst-7 (the setup operation's) and tst-8 (its
            // own) together, as the reference validator reports it
            if !out.contains(&full) {
                out.push(full);
            }
        }
    }
    out
}

/// The paths at which the document holds a copy of `backbone` other than
/// the one at `at`: beneath `at` where the reference lies under the
/// backbone (a section's section, and that section's section …), or where
/// the base put the reference when it lies elsewhere (a component's
/// referenceRange) — the latter only from the backbone's own definition,
/// never from one copy to its siblings. A copy counts only where an
/// instance exists, which bounds the descent by the document.
fn backbone_copies<D: Document>(backbone: &str, at: &str, doc: &D) -> Vec<String> {
    let refs = crate::typing::content_references_to(backbone);
    let mut copies: Vec<String> = alloc::vec![String::from(at)];
    let mut i = 0;
    while i < copies.len() {
        let here = copies[i].clone();
        i += 1;
        for q in &refs {
            let copy = match q.strip_prefix(backbone).and_then(|s| s.strip_prefix('.')) {
                Some(beneath) => format!("{here}.{beneath}"),
                None if at == backbone => q.clone(),
                None => continue,
            };
            if !copies.contains(&copy) && !doc.instances(&copy).is_empty() {
                copies.push(copy);
            }
        }
    }
    copies.remove(0);
    copies
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
        // a choice element never repeats: two keys for one `[x]` is the shape
        // walk's finding ("allows one choice"), reported once, not a count
        if let Max::Bounded(m) = el.max {
            if count > m && !el.path.ends_with("[x]") {
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
            let codes = doc.scalars(el.path);
            for code in &codes {
                let m = under_base(tx.validate_code(vs, "", code), &b, el.path, |set| tx.validate_code(set, "", code));
                report_membership(m, vs, code, label, r);
            }
            // an element present without a value (a `_name` carrier alone —
            // a data-absent-reason) still owes the code the binding requires
            let present: usize = doc.child_counts(el.path).iter().sum();
            for _ in codes.len()..present {
                r.push(
                    Severity::Error,
                    IssueType::CodeInvalid,
                    label,
                    String::from("No code provided, and a code is required from the value set"),
                );
            }
        }
        BindingKind::Coding => {
            for inst in doc.instances(el.path) {
                if let Some((system, code)) = coding_of(&inst) {
                    let m = under_base(tx.validate_code(vs, system, code), &b, el.path, |set| {
                        tx.validate_code(set, system, code)
                    });
                    report_membership(m, vs, code, label, r);
                }
            }
        }
        BindingKind::CodeableConcept => {
            for inst in doc.instances(el.path) {
                // A member if any coding is; unknown if none can be decided.
                // No coding at all — text only, a data-absent-reason, nothing
                // — is an error for a required binding, as the reference
                // validator reports it (6.10.4). One or more Coding objects
                // with no code also fail the parent concept's required
                // binding; a standalone Coding with no code is not judged.
                let codings = inst.instances("$this.coding");
                let codeable_concept = is_codeable_concept(el).unwrap_or_else(|| concept_shaped(&inst));
                if codings.is_empty() && codeable_concept {
                    r.push(
                        Severity::Error,
                        IssueType::CodeInvalid,
                        label,
                        format!("No code provided, and a code is required from the value set {vs}"),
                    );
                    continue;
                }
                let concept = |set: &str| -> Option<Membership> {
                    let mut verdict = None;
                    for c in &codings {
                        if let Some((system, code)) = coding_of(c) {
                            match tx.validate_code(set, system, code) {
                                Membership::Member => return Some(Membership::Member),
                                Membership::Unknown => verdict = Some(Membership::Unknown),
                                Membership::NotMember => verdict = verdict.or(Some(Membership::NotMember)),
                            }
                        }
                    }
                    verdict
                };
                let base = base_binding_under(&b, el.path);
                let mut verdict = concept(vs);
                if !matches!(verdict, Some(Membership::Member | Membership::NotMember)) {
                    if let Some(base) = base {
                        if let v @ Some(Membership::Member | Membership::NotMember) = concept(base.value_set) {
                            verdict = v;
                        }
                    }
                }
                if let Some(v) = verdict {
                    report_membership(v, vs, "the codings", label, r);
                } else if codeable_concept {
                    // Coding objects exist, none with a code: no member of the
                    // set — where the set can be decided at all. Where it
                    // cannot (a LOINC-built set, offline) the reference
                    // validator says nothing, and neither do we beyond the
                    // usual "not checked".
                    let judged = decidable(tx, vs) || base.is_some_and(|b| decidable(tx, b.value_set));
                    let m = if judged { Membership::NotMember } else { Membership::Unknown };
                    report_membership(m, vs, "the codings", label, r);
                }
            }
        }
    }
}

/// Is the element a `CodeableConcept` — by the profile's own type list, or
/// the base element table where the profile leaves it open? A `Quantity`
/// bound to UCUM (`bp`'s `component.value[x]`) carries the same binding
/// kind but no codings, and is judged by its own system and code, never by
/// their absence; a choice the profile leaves open is not decided here.
fn is_codeable_concept(el: &ElementConstraint) -> Option<bool> {
    if !el.types.is_empty() {
        return Some(el.types == ["CodeableConcept"]);
    }
    let mut segs = el.path.split('.');
    let mut ctx = segs.next()?;
    let mut ty = None;
    for seg in segs {
        let name = seg.split(':').next().unwrap_or(seg);
        if name.ends_with("[x]") {
            // a path through a choice (`Extension.value[x].type`) cannot be
            // typed from the table: the instance decides
            return None;
        }
        let ch = crate::typing::child(ctx, name)?;
        ctx = ch.ctx;
        ty = Some(ch.ty);
    }
    Some(ty == Some("CodeableConcept"))
}

/// Where the table cannot type the element, the instance can: a
/// `CodeableConcept` carries codings or text, and never a `Quantity`'s
/// value, unit, comparator or code scalars (`bp`'s `component.value[x]`
/// carries the same binding kind; a Quantity of system and code alone is
/// still a Quantity). An object with neither is not judged.
fn concept_shaped<D: Document>(inst: &D) -> bool {
    let concept = !inst.instances("$this.coding").is_empty() || !inst.scalars("$this.text").is_empty();
    let quantity =
        ["$this.value", "$this.unit", "$this.comparator", "$this.code"].iter().any(|p| !inst.scalars(p).is_empty());
    concept && !quantity
}

/// Is the coded value at `path` a member of the set? `Code`: the scalar;
/// `Coding`: its system + code; `CodeableConcept`: any coding decides
/// membership, none decided is unknown.
/// Membership of the coded object `inst` itself (a `Coding` or
/// `CodeableConcept` reached by the shape walk) in the binding's set, and
/// whether any coding was there to decide it.
pub(crate) fn membership_of<D: Document>(b: &Binding, inst: &D, tx: &dyn TerminologyProvider) -> (Membership, bool) {
    membership_seen(b, inst, "$this", tx)
}

pub(crate) fn membership_at<D: Document>(
    b: &Binding,
    inst: &D,
    path: &str,
    tx: &dyn TerminologyProvider,
) -> Membership {
    match membership_seen(b, inst, path, tx) {
        // nothing coded there: not this slice's, decidedly
        (_, false) => Membership::NotMember,
        (m, true) => m,
    }
}

fn membership_seen<D: Document>(b: &Binding, inst: &D, path: &str, tx: &dyn TerminologyProvider) -> (Membership, bool) {
    let vs = b.value_set;
    let mut verdict = Membership::Unknown;
    let mut seen = false;
    let fold = |verdict: &mut Membership, m: Membership| match m {
        Membership::Member => *verdict = Membership::Member,
        Membership::NotMember if !matches!(*verdict, Membership::Member) => *verdict = Membership::NotMember,
        _ => {}
    };
    match b.kind {
        BindingKind::Code => {
            for code in inst.scalars(path) {
                seen = true;
                fold(&mut verdict, tx.validate_code(vs, "", code));
            }
        }
        BindingKind::Coding => {
            for c in inst.instances(path) {
                if let Some((system, code)) = coding_of(&c) {
                    seen = true;
                    fold(&mut verdict, tx.validate_code(vs, system, code));
                }
            }
        }
        BindingKind::CodeableConcept => {
            let mut unknown = false;
            for cc in inst.instances(path) {
                for c in cc.instances("$this.coding") {
                    if let Some((system, code)) = coding_of(&c) {
                        seen = true;
                        match tx.validate_code(vs, system, code) {
                            Membership::Member => return (Membership::Member, true),
                            Membership::Unknown => unknown = true,
                            Membership::NotMember => fold(&mut verdict, Membership::NotMember),
                        }
                    }
                }
            }
            if unknown {
                return (Membership::Unknown, true);
            }
        }
    }
    (verdict, seen)
}

/// Can the provider decide membership in `set` at all? A set it carries
/// answers every (system, code) — including none; a set it does not know
/// answers nothing.
fn decidable(tx: &dyn TerminologyProvider, set: &str) -> bool {
    tx.knows_set(set)
}

/// The base's own required binding on the element at `path`, of the same
/// kind as the profile's and to a different set. A profile only narrows a
/// required base set, so what the base refuses the profile refuses too —
/// and the reference validator reports both. Where the profile's set
/// cannot be decided offline (an IPS status subset), the base's still can.
fn base_binding_under(b: &crate::compiled::Binding, path: &str) -> Option<&'static crate::compiled::Binding> {
    let (parent, key) = path.rsplit_once('.')?;
    let mut segs = parent.split('.');
    let mut ctx = crate::typing::known_type(segs.next()?)?;
    for s in segs {
        if s.ends_with("[x]") {
            // a choice folds to its first type in the table — not the
            // instance's; the base beneath a choice is not looked up
            return None;
        }
        ctx = crate::typing::child(ctx, s)?.ctx;
    }
    let (_, _, base) = crate::typemap::BASE_BINDINGS.iter().find(|(c, k, _)| *c == ctx && *k == key)?;
    let same_set = base.value_set.split('|').next() == b.value_set.split('|').next();
    (base.kind == b.kind && !same_set).then_some(base)
}

/// A profile verdict that could not be decided, judged by the base's set
/// where there is one.
fn under_base(
    m: Membership,
    b: &crate::compiled::Binding,
    path: &str,
    judge: impl Fn(&str) -> Membership,
) -> Membership {
    if m != Membership::Unknown {
        return m;
    }
    match base_binding_under(b, path) {
        Some(base) => judge(base.value_set),
        None => m,
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
/// Whether an instance belongs to a slice: yes, no, or — a `Member`
/// discriminator whose set the provider cannot decide — undecided.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Belongs {
    Yes,
    No,
    Undecided(&'static str),
}

fn matches<D: Document>(slice: &Slice, inst: &D, scope: &Scope<'_, D>) -> Belongs {
    let mut undecided = None;
    for d in slice.discriminators {
        if let Discriminator::Member { path, binding } = d {
            match membership_at(binding, inst, path, scope.tx) {
                Membership::Member => {}
                Membership::NotMember => return Belongs::No,
                Membership::Unknown => undecided = Some(binding.value_set),
            }
        } else if !matches_one(d, inst, scope) {
            return Belongs::No;
        }
    }
    match undecided {
        Some(vs) => Belongs::Undecided(vs),
        None => Belongs::Yes,
    }
}

fn matches_one<D: Document>(d: &Discriminator, inst: &D, scope: &Scope<'_, D>) -> bool {
    {
        match d {
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
            // decided in `matches`, three ways
            Discriminator::Member { .. } => true,
        }
    }
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
/// A resource's own invariants (org-1, pat-1, cmp-1 …) on a resource no
/// compiled profile applies to — a profile's snapshot restates them, so
/// only the unprofiled case reads the table.
pub(crate) fn resource_invariants<D: Document>(
    doc: &D,
    rt: &'static str,
    cache: Option<&crate::Validator>,
    bundle: Option<&D>,
) -> Report {
    let mut r = Report::default();
    let scope = Scope { root: doc, bundle, tx: &Bundled, depth: 0, cache, evaluated: None };
    for (ty, path, key, is_error, human, expression, types) in crate::typemap::RESOURCE_INVARIANTS {
        if *ty != rt {
            continue;
        }
        let inv = ProfileInvariant {
            path,
            key,
            severity: if *is_error { InvariantSeverity::Error } else { InvariantSeverity::Warning },
            human,
            expression,
            types,
        };
        check_invariant(&inv, path, doc, &scope, &mut r);
    }
    r
}

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

/// The resource types the reference validator's RESTful-URL regex names
/// (`BundleValidator.urlRegex`, 6.10.4) — an STU3-era list, kept as it is:
/// `Condition` is spelled "Condition (aka Problem)" there and so never
/// matches, and the R4-only types are absent. Verbatim, so the same fullUrls
/// are judged.
const RESTFUL_TYPES: &[&str] = &[
    "Account",
    "ActivityDefinition",
    "AllergyIntolerance",
    "AdverseEvent",
    "Appointment",
    "AppointmentResponse",
    "AuditEvent",
    "Basic",
    "Binary",
    "BodySite",
    "Bundle",
    "CapabilityStatement",
    "CarePlan",
    "CareTeam",
    "ChargeItem",
    "Claim",
    "ClaimResponse",
    "ClinicalImpression",
    "CodeSystem",
    "Communication",
    "CommunicationRequest",
    "CompartmentDefinition",
    "Composition",
    "ConceptMap",
    "Condition (aka Problem)",
    "Consent",
    "Contract",
    "Coverage",
    "DataElement",
    "DetectedIssue",
    "Device",
    "DeviceComponent",
    "DeviceMetric",
    "DeviceRequest",
    "DeviceUseStatement",
    "DiagnosticReport",
    "DocumentManifest",
    "DocumentReference",
    "EligibilityRequest",
    "EligibilityResponse",
    "Encounter",
    "Endpoint",
    "EnrollmentRequest",
    "EnrollmentResponse",
    "EpisodeOfCare",
    "ExpansionProfile",
    "ExplanationOfBenefit",
    "FamilyMemberHistory",
    "Flag",
    "Goal",
    "GraphDefinition",
    "Group",
    "GuidanceResponse",
    "HealthcareService",
    "ImagingManifest",
    "ImagingStudy",
    "Immunization",
    "ImmunizationRecommendation",
    "ImplementationGuide",
    "Library",
    "Linkage",
    "List",
    "Location",
    "Measure",
    "MeasureReport",
    "Media",
    "Medication",
    "MedicationAdministration",
    "MedicationDispense",
    "MedicationRequest",
    "MedicationStatement",
    "MessageDefinition",
    "MessageHeader",
    "NamingSystem",
    "NutritionOrder",
    "Observation",
    "OperationDefinition",
    "OperationOutcome",
    "Organization",
    "Parameters",
    "Patient",
    "PaymentNotice",
    "PaymentReconciliation",
    "Person",
    "PlanDefinition",
    "Practitioner",
    "PractitionerRole",
    "Procedure",
    "ProcedureRequest",
    "ProcessRequest",
    "ProcessResponse",
    "Provenance",
    "Questionnaire",
    "QuestionnaireResponse",
    "ReferralRequest",
    "RelatedPerson",
    "RequestGroup",
    "ResearchStudy",
    "ResearchSubject",
    "RiskAssessment",
    "Schedule",
    "SearchParameter",
    "Sequence",
    "ServiceDefinition",
    "Slot",
    "Specimen",
    "StructureDefinition",
    "StructureMap",
    "Subscription",
    "Substance",
    "SupplyDelivery",
    "SupplyRequest",
    "Task",
    "TestScript",
    "TestReport",
    "ValueSet",
    "VisionPrescription",
];

/// `((http|https)://([A-Za-z0-9\.\:\%\$]*/)*)?(<type>)/[A-Za-z0-9\-\.]{1,64}(/_history/[A-Za-z0-9\-\.]{1,64})?`,
/// the whole string — the reference validator's test for a fullUrl that
/// looks like a RESTful server URL; the type and id it names.
fn restful_shape(full: &str) -> Option<(&str, &str)> {
    let id_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '.';
    let rest = match full.split_once("://") {
        Some((scheme, rest)) if scheme == "http" || scheme == "https" => {
            // any number of host / path segments of the permitted characters
            let mut rest = rest;
            while let Some((seg, tail)) = rest.split_once('/') {
                if !seg.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '%' | '$')) {
                    break;
                }
                // the segment before `Type/id` may itself be the type: try both readings
                if let Some(found) = restful_tail(rest, id_char) {
                    return Some(found);
                }
                rest = tail;
            }
            rest
        }
        Some(_) => return None,
        None => full,
    };
    restful_tail(rest, id_char)
}

fn restful_tail(rest: &str, id_char: impl Fn(char) -> bool) -> Option<(&str, &str)> {
    let (ty, after) = rest.split_once('/')?;
    if !RESTFUL_TYPES.contains(&ty) {
        return None;
    }
    let (id, history) = match after.split_once('/') {
        Some((id, h)) => (id, Some(h)),
        None => (after, None),
    };
    let ok_id = |s: &str| !s.is_empty() && s.len() <= 64 && s.chars().all(&id_char);
    if !ok_id(id) {
        return None;
    }
    match history {
        None => Some((ty, id)),
        Some(h) => h.strip_prefix("_history/").filter(|v| ok_id(v)).map(|_| (ty, id)),
    }
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

/// Inside any Bundle, a reference that resolves to an entry is held to its
/// element's base targets by the target's own type — the reference
/// validator's "Invalid Resource target type", resolved as it resolves: an
/// absolute or `urn:` reference by its `fullUrl`, a relative `Type/id` only
/// against a RESTful `fullUrl` ending so (among `urn:uuid` entries it names
/// nothing and is not judged). A resource declaring a compiled profile is
/// the profile's to hold to its own targets.
pub(crate) fn check_reference_targets_base<D: Document>(bundle: &D, r: &mut Report) {
    let entries: Vec<(Option<String>, Option<D>)> = bundle
        .instances("$this.entry")
        .into_iter()
        .map(|e| {
            (
                e.scalars("$this.fullUrl").first().map(|s| String::from(*s)),
                e.child_instances("resource").into_iter().next(),
            )
        })
        .collect();
    let resolve = |reference: &str| -> Option<&'static str> {
        let identity = reference.rsplit_once("/_history/").map(|(i, _)| i).unwrap_or(reference);
        let relative = !identity.contains(':');
        if relative && !identity.contains('/') {
            return None; // a bare id names no entry (the format rule reports it)
        }
        entries
            .iter()
            .find(|(full, _)| {
                full.as_deref().is_some_and(|f| {
                    f == identity || (relative && f.ends_with(identity) && f[..f.len() - identity.len()].ends_with('/'))
                })
            })
            .and_then(|(_, res)| res.as_ref())
            .and_then(|res| res.scalars("$this.resourceType").first().copied())
            .and_then(|rt| crate::typemap::RESOURCES.binary_search(&rt).ok().map(|k| crate::typemap::RESOURCES[k]))
    };
    for (i, (_, res)) in entries.iter().enumerate() {
        let Some(res) = res else { continue };
        let Some(rt) = res.scalars("$this.resourceType").first().copied() else { continue };
        let profiled = res
            .scalars("$this.meta.profile")
            .iter()
            .any(|u| crate::REGISTRY.iter().any(|p| p.url == u.split('|').next().unwrap_or(u)));
        if profiled {
            continue;
        }
        for (path, targets) in crate::typemap::REFERENCE_TARGETS {
            if !(path.starts_with(rt) && path.as_bytes().get(rt.len()) == Some(&b'.')) {
                continue;
            }
            for (j, inst) in res.instances(path).iter().enumerate() {
                let Some(reference) = inst.scalars("$this.reference").first().copied() else { continue };
                if reference.starts_with('#') {
                    continue;
                }
                let Some(target) = resolve(reference) else { continue };
                if !targets.split('|').any(|t| t == target) {
                    r.push(
                        Severity::Error,
                        IssueType::Structure,
                        &format!("Bundle.entry[{i}].resource.{path}[{j}]"),
                        format!(
                            "Invalid Resource target type. Found {target}, but expected one of ([{}])",
                            targets.replace('|', ", ")
                        ),
                    );
                }
            }
        }
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
        // a fullUrl that looks like a RESTful server URL must end with the
        // resource's own type and id — "looks like" being the reference
        // validator's own regex (`BundleValidator.urlRegex`), mirrored
        let Some((url_type, url_id)) = restful_shape(full) else { continue };
        let rt = entry.scalars("$this.resource.resourceType").first().copied();
        let id = entry.scalars("$this.resource.id").first().copied();
        let Some(rt) = rt else { continue };
        match id {
            None => r.push(
                Severity::Error,
                IssueType::Invariant,
                &format!("Bundle.entry[{i}].fullUrl"),
                format!("The fullUrl '{full}' looks like a RESTful server URL, but the resource has no id"),
            ),
            Some(id) if url_type != rt || url_id != id => r.push(
                Severity::Error,
                IssueType::Invariant,
                &format!("Bundle.entry[{i}].fullUrl"),
                format!(
                    "The fullUrl '{full}' looks like a RESTful server URL, so it must end with the correct type and id (/{rt}/{id})"
                ),
            ),
            _ => {}
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
    // a relative reference whose type the base does not define resolves
    // nowhere and is not held to the bundle (the reference validator passes it)
    let resolvable = |reference: &str| {
        reference.contains(':')
            || reference.split_once('/').is_some_and(|(ty, _)| crate::typemap::RESOURCES.binary_search(&ty).is_ok())
    };
    for (path, reference) in &refs {
        if !reference.starts_with('#') && resolvable(reference) && index_of(reference).is_none() {
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
        // instances no slice could claim or refuse: a `Member` discriminator
        // whose set the provider cannot decide — (instance, set)
        let mut undecided: Vec<(usize, &'static str)> = Vec::new();
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
                    .filter(|(i, inst)| match matches(slice, *inst, scope) {
                        Belongs::Yes => {
                            hits[*i].push(slice.name);
                            true
                        }
                        Belongs::Undecided(vs) => {
                            if !undecided.iter().any(|(j, _)| j == i) {
                                undecided.push((*i, vs));
                            }
                            false
                        }
                        Belongs::No => false,
                    })
                    .map(|(_, inst)| inst.clone())
                    .collect(),
            };
            matched += insts.len();
            let count = insts.len() as u32;
            // an undecided instance may be this slice's: its minimum is not
            // an error while that stands (the warning below says so)
            let could_be_here =
                slice.discriminators.iter().any(|d| matches!(d, Discriminator::Member { .. })) && !undecided.is_empty();
            if count < slice.min && !could_be_here {
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
                        // the reference validator judges each candidate with
                        // the resource's own declared profiles applied as
                        // well: a declared profile it fails sinks every
                        // candidate, and the entry "did not match any of the
                        // allowed profiles" (those failures are reported on
                        // the entry itself, by the nested pass)
                        let declared_ok = target.scalars("$this.meta.profile").iter().all(|u| {
                            let u = u.split('|').next().unwrap_or(u);
                            match crate::REGISTRY.iter().find(|p| p.url == u) {
                                Some(d) => scope.conformance(d, &target).is_valid(),
                                None => true,
                            }
                        });
                        let any_valid = reports.iter().any(|(_, rep)| rep.is_valid()) && target.shape_ok(scope.tx);
                        let _ = &any_valid;
                        match reports.as_slice() {
                            [(profile, rep)] if !rep.is_valid() => forward(rep.clone(), profile, &label, r),
                            [_] => {}
                            [] => r.push(
                                Severity::Error,
                                IssueType::Structure,
                                &label,
                                format!("{label} is not one of the resource types this entry allows"),
                            ),
                            many if !(any_valid && declared_ok) => r.push(
                                Severity::Error,
                                IssueType::Structure,
                                &label,
                                format!(
                                    "The entry resource did not match any of the allowed profiles (Type {}: {})",
                                    many[0].0.resource_type,
                                    many.iter().map(|(p, _)| p.url).collect::<Vec<_>>().join(", ")
                                ),
                            ),
                            _ => {}
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
        // an instance a slice matched is decided, whatever another slice
        // could not say about it
        undecided.retain(|(i, _)| hits[*i].is_empty());
        for (i, vs) in &undecided {
            r.push(
                Severity::Warning,
                IssueType::NotSupported,
                &base,
                format!(
                    "{base}[{i}] could not be assigned to a slice: no bundled terminology for {vs}; membership not checked"
                ),
            );
        }
        if slicing.closed && matched + undecided.len() < all.len() {
            r.push(
                Severity::Error,
                IssueType::Structure,
                &base,
                format!(
                    "{base} has {} element(s) matching no slice (closed slicing)",
                    all.len() - matched - undecided.len()
                ),
            );
        }
    }
}
