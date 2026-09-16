//! The terminology seam. Bundled sets answer what they can; a host provider
//! (NCTS, VSAC, an on-device cache…) answers the rest. SNOMED content never
//! ships in the core — it only ever arrives through this seam.

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
#[derive(Debug, Clone, Copy, Default)]
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
#[derive(Clone, Copy)]
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
