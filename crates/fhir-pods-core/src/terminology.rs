//! The terminology seam. Bundled sets answer what they can; a host provider
//! (NCTS, VSAC, an on-device cache…) answers the rest. SNOMED content never
//! ships in the core — it only ever arrives through this seam.
//!
//! Three providers come with the core: [`Bundled`] (the generated base
//! sets), [`Expansions`] (a compiled table the host loads — the offline
//! route), and [`Recorder`] (answers nothing and remembers every question —
//! how a host learns what to fetch). [`Chain`] composes them.

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

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

    /// The display for (`system`, `code`), when the provider knows one.
    /// Lowering fills a coding's `display` from here; the verifier never
    /// needs it. Default: none.
    fn display(&self, _system: &str, _code: &str) -> Option<String> {
        None
    }
}

/// The value sets compiled into the core (see [`crate::value_sets`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct Bundled;

impl TerminologyProvider for Bundled {
    fn validate_code(&self, value_set: &str, system: &str, code: &str) -> Membership {
        match crate::value_sets::membership(value_set, system, code) {
            Some(true) => Membership::Member,
            Some(false) => Membership::NotMember,
            None => Membership::Unknown,
        }
    }

    fn display(&self, system: &str, code: &str) -> Option<String> {
        crate::value_sets::display(system, code).map(String::from)
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

    fn display(&self, system: &str, code: &str) -> Option<String> {
        self.0.iter().find_map(|p| p.display(system, code))
    }
}

/// One question the bundled sets could not answer: what a host must resolve
/// (and may then compile into an [`Expansions`] table) for a verdict that
/// carries no terminology warnings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    pub value_set: String,
    pub system: String,
    pub code: String,
}

/// Answers nothing and remembers every question, deduplicated. Chain it
/// after [`Bundled`], validate, then read [`Recorder::needs`].
#[derive(Debug, Default)]
pub struct Recorder {
    asked: RefCell<Vec<Need>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything asked so far, in order of first appearance.
    pub fn needs(&self) -> Vec<Need> {
        self.asked.borrow().clone()
    }
}

impl TerminologyProvider for Recorder {
    fn validate_code(&self, value_set: &str, system: &str, code: &str) -> Membership {
        let mut asked = self.asked.borrow_mut();
        if !asked.iter().any(|n| n.value_set == value_set && n.system == system && n.code == code) {
            asked.push(Need { value_set: value_set.into(), system: system.into(), code: code.into() });
        }
        Membership::Unknown
    }
}

/// A compiled terminology table the host loads at run time — value-set
/// expansions the host is licensed for, in a line format `tools/compile_valueset.py`
/// writes from a `$expand` response:
///
/// ```text
/// fhir-pods-terminology 1
/// = http://hl7.org.au/fhir/ValueSet/… (one `=` line per value set, versionless or |version)
/// http://snomed.info/sct<TAB>364075005<TAB>Heart rate      (system, code, optional display)
/// # comments and blank lines are ignored
/// ```
///
/// A set present in the table is *complete*: a code not listed is not a
/// member. A set absent from the table is unknown. Displays are available to
/// lowering through [`TerminologyProvider::display`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Expansions {
    /// (value set URL without version, sorted [(system, code)]), sorted by URL.
    sets: Vec<(String, Vec<(String, String)>)>,
    /// (system, code, display), sorted — only the codes that carry a display.
    displays: Vec<(String, String, String)>,
}

/// Why a table could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableError {
    /// 1-based line.
    pub line: usize,
    pub msg: &'static str,
}

impl Expansions {
    /// Parse the line format. The first non-blank line must be the header.
    pub fn parse(text: &str) -> Result<Self, TableError> {
        let mut sets: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut displays: Vec<(String, String, String)> = Vec::new();
        let mut current: Option<usize> = None;
        let mut header_seen = false;
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r');
            let ln = i + 1;
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            if !header_seen {
                if line.trim() != "fhir-pods-terminology 1" {
                    return Err(TableError { line: ln, msg: "expected header `fhir-pods-terminology 1`" });
                }
                header_seen = true;
                continue;
            }
            if let Some(url) = line.strip_prefix('=') {
                let url = url.trim();
                let url = url.split('|').next().unwrap_or(url);
                if url.is_empty() {
                    return Err(TableError { line: ln, msg: "empty value set URL" });
                }
                let idx = match sets.iter().position(|(u, _)| u == url) {
                    Some(i) => i,
                    None => {
                        sets.push((url.into(), Vec::new()));
                        sets.len() - 1
                    }
                };
                current = Some(idx);
                continue;
            }
            let Some(idx) = current else {
                return Err(TableError { line: ln, msg: "a code before any `= value set` line" });
            };
            let mut parts = line.splitn(3, '\t');
            let (system, code) = (parts.next().unwrap_or("").trim(), parts.next().unwrap_or("").trim());
            if system.is_empty() || code.is_empty() {
                return Err(TableError { line: ln, msg: "expected `system<TAB>code[<TAB>display]`" });
            }
            sets[idx].1.push((system.into(), code.into()));
            if let Some(d) = parts.next().map(str::trim).filter(|d| !d.is_empty()) {
                displays.push((system.into(), code.into(), d.into()));
            }
        }
        if !header_seen {
            return Err(TableError { line: 0, msg: "empty table" });
        }
        for (_, codes) in &mut sets {
            codes.sort();
            codes.dedup();
        }
        sets.sort_by(|a, b| a.0.cmp(&b.0));
        displays.sort();
        displays.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        Ok(Self { sets, displays })
    }

    /// The value sets the table answers for.
    pub fn value_sets(&self) -> impl Iterator<Item = &str> {
        self.sets.iter().map(|(u, _)| u.as_str())
    }

    /// Codes across every set.
    pub fn len(&self) -> usize {
        self.sets.iter().map(|(_, c)| c.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl TerminologyProvider for Expansions {
    fn validate_code(&self, value_set: &str, system: &str, code: &str) -> Membership {
        let base = value_set.split('|').next().unwrap_or(value_set);
        let Ok(i) = self.sets.binary_search_by(|(u, _)| u.as_str().cmp(base)) else {
            return Membership::Unknown;
        };
        let codes = &self.sets[i].1;
        let found = if system.is_empty() {
            codes.iter().any(|(_, c)| c == code)
        } else {
            codes.binary_search_by(|(s, c)| (s.as_str(), c.as_str()).cmp(&(system, code))).is_ok()
        };
        if found {
            Membership::Member
        } else {
            Membership::NotMember
        }
    }

    fn display(&self, system: &str, code: &str) -> Option<String> {
        self.displays
            .binary_search_by(|(s, c, _)| (s.as_str(), c.as_str()).cmp(&(system, code)))
            .ok()
            .map(|i| self.displays[i].2.clone())
    }
}
