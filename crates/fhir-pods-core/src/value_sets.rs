//! Offline membership for the value sets we can legally bundle — generated
//! from the packages' own compose into `value_sets_table` (every FHIR / HL7
//! v2 / v3 set the registry binds). `None` for a set we don't ship — the
//! engine degrades that to a warning. SNOMED- and LOINC-bound sets are
//! intentionally absent: they resolve at runtime through the terminology seam.

use crate::value_sets_table::{DISPLAYS, ONE_LISTED_INCLUDE, SETS, WHOLE_SYSTEMS};

/// A bundled set's codes: sorted (system, code, display).
pub type Codes = &'static [(&'static str, &'static str, &'static str)];

/// Every bundled set: (URL, codes), sorted by URL.
pub fn sets() -> &'static [(&'static str, Codes)] {
    SETS
}

fn set(url: &str) -> Option<Codes> {
    let base = url.split('|').next().unwrap_or(url);
    SETS.binary_search_by(|(u, _)| u.cmp(&base)).ok().map(|i| SETS[i].1)
}

/// Is (`system`, `code`) in the bundled set at `url`? An empty `system` (a
/// bare `code` element) matches the code in any of the set's systems.
/// Is the set one the core can decide at all — a bundled expansion, or a
/// whole code system the packages ship?
pub fn knows(url: &str) -> bool {
    let base = url.split('|').next().unwrap_or(url);
    WHOLE_SYSTEMS.binary_search_by(|(u, _)| u.cmp(&base)).is_ok() || set(url).is_some()
}

/// A code given with no system against the bundled set at `url`, as the
/// reference validator places it: where the set's compose is one include
/// listing its concepts, by the listed codes; under any other compose (a
/// whole code system, several includes) never a member. `None` for a set
/// not bundled.
pub fn code_only_membership(url: &str, code: &str) -> Option<bool> {
    let base = url.split('|').next().unwrap_or(url);
    if WHOLE_SYSTEMS.binary_search_by(|(u, _)| u.cmp(&base)).is_ok() {
        return Some(false);
    }
    let codes = set(url)?;
    Some(ONE_LISTED_INCLUDE.binary_search(&base).is_ok() && codes.iter().any(|(_, c, _)| *c == code))
}

pub fn membership(url: &str, system: &str, code: &str) -> Option<bool> {
    // a set that is the whole of a system the packages do not ship: the
    // system decides; a bare code cannot be placed
    let base = url.split('|').next().unwrap_or(url);
    if let Ok(i) = WHOLE_SYSTEMS.binary_search_by(|(u, _)| u.cmp(&base)) {
        return if system.is_empty() { None } else { Some(WHOLE_SYSTEMS[i].1.contains(&system)) };
    }
    let codes = set(url)?;
    Some(if system.is_empty() {
        codes.iter().any(|(_, c, _)| *c == code)
    } else {
        codes.binary_search_by(|(s, c, _)| (*s, *c).cmp(&(system, code))).is_ok()
    })
}

/// [`membership`] for a bare code.
pub fn in_value_set(url: &str, code: &str) -> Option<bool> {
    membership(url, "", code)
}

/// The display the bundled code systems give (`system`, `code`), if any:
/// the codings the registry fixes, then the bundled sets' codes.
pub fn display(system: &str, code: &str) -> Option<&'static str> {
    if let Ok(i) = DISPLAYS.binary_search_by(|(s, c, _)| (*s, *c).cmp(&(system, code))) {
        return Some(DISPLAYS[i].2);
    }
    for (_, codes) in SETS {
        if let Ok(i) = codes.binary_search_by(|(s, c, _)| (*s, *c).cmp(&(system, code))) {
            let d = codes[i].2;
            if !d.is_empty() {
                return Some(d);
            }
        }
    }
    None
}
