//! Offline membership for the value sets we can legally bundle — generated
//! from the packages' own compose into `value_sets_table` (every FHIR / HL7
//! v2 / v3 set the registry binds). `None` for a set we don't ship — the
//! engine degrades that to a warning. SNOMED- and LOINC-bound sets are
//! intentionally absent: they resolve at runtime through the terminology seam.

use crate::value_sets_table::{DISPLAYS, SETS, WHOLE_SYSTEMS};

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
