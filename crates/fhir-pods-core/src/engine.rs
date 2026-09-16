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
        // Outside the supported FHIRPath core: skipped, never guessed — and
        // said so, once per invariant, so a silent gap never reads as a pass.
        let owned;
        let expr: &path::Expr = match cache {
            Some(v) => match v.expr(inv.expression) {
                Some(e) => e,
                None => {
                    not_evaluated(inv, &mut r);
                    continue;
                }
            },
            None => match path::parse(inv.expression) {
                Ok(e) => {
                    owned = e;
                    &owned
                }
                Err(_) => {
                    not_evaluated(inv, &mut r);
                    continue;
                }
            },
        };
        let mut skipped = false;
        for (ctx, this_type) in invariant_contexts(inv, doc) {
            match path::check_with(expr, &ctx, this_type, Some(root)) {
                Verdict::Fail => {
                    let severity = match inv.severity {
                        InvariantSeverity::Error => Severity::Error,
                        InvariantSeverity::Warning => Severity::Warning,
                    };
                    r.push(severity, IssueType::Invariant, inv.path, format!("{}: {}", inv.key, inv.human));
                }
                Verdict::Unsupported => skipped = true,
                Verdict::Pass => {}
            }
        }
        if skipped {
            not_evaluated(inv, &mut r);
        }
    }
    r
}

fn not_evaluated(inv: &ProfileInvariant, r: &mut Report) {
    r.push(
        Severity::Information,
        IssueType::NotSupported,
        inv.path,
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
                format!("{} has {} element(s) matching no slice (closed slicing)", slicing.path, all.len() - matched),
            );
        }
    }
}
