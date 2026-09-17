//! HL7 v2: an ER7 message parsed into segments, fields, repetitions,
//! components and subcomponents, and converted to a FHIR message Bundle
//! through the `hl7.fhir.uv.v2mappings` maps compiled into the pod.
//!
//! The parser knows the grammar and nothing else: `MSH` carries the
//! separators (`MSH-1`) and encoding characters (`MSH-2`), every other
//! segment is a name and fields; escapes (`\F\ \S\ \T\ \R\ \E\`, `\Xdd\`)
//! are decoded on read. Framing (MLLP) is the transport's, not the pod's.
pub mod compiled;
pub mod convert;
pub mod generated;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The separators of one message (`MSH-1`, `MSH-2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Separators {
    pub field: char,
    pub component: char,
    pub repetition: char,
    pub escape: char,
    pub subcomponent: char,
}

impl Default for Separators {
    fn default() -> Self {
        Separators { field: '|', component: '^', repetition: '~', escape: '\\', subcomponent: '&' }
    }
}

/// A parsed message: segments in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub separators: Separators,
    pub segments: Vec<Segment>,
}

/// A segment: its three-letter name and its fields, `fields[0]` being
/// field 1 (`MSH-1` and `MSH-2` are the separators themselves, present as
/// fields so numbering matches the standard).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub name: String,
    pub fields: Vec<Field>,
}

/// A field: one or more repetitions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Field {
    pub reps: Vec<Rep>,
}

/// One repetition of a field: components, each a list of subcomponents
/// (already unescaped).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rep {
    pub components: Vec<Vec<String>>,
}

/// Why a message could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Not starting with `MSH` and its separators.
    NoHeader,
    /// A segment line without a three-character name.
    BadSegment(usize),
    /// Not UTF-8.
    NotText,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::NoHeader => f.write_str("message must start with MSH and its separators"),
            ParseError::BadSegment(i) => write!(f, "segment {i} has no three-character name"),
            ParseError::NotText => f.write_str("message is not UTF-8"),
        }
    }
}

impl core::error::Error for ParseError {}

impl Rep {
    /// Component `n` (1-based), subcomponent `s` (1-based, 0 = the whole
    /// component joined by nothing but its first subcomponent when it has
    /// several). Empty when absent.
    pub fn component(&self, n: u16, s: u16) -> &str {
        let Some(c) = self.components.get(n.max(1) as usize - 1) else { return "" };
        if s == 0 {
            c.first().map(String::as_str).unwrap_or("")
        } else {
            c.get(s as usize - 1).map(String::as_str).unwrap_or("")
        }
    }

    /// The repetition as text: components joined by the separators again,
    /// trailing empties dropped — what a whole field's value is.
    pub fn text(&self, sep: &Separators) -> String {
        let mut out = String::new();
        let mut comps: Vec<String> = self
            .components
            .iter()
            .map(|subs| {
                let mut s: Vec<&str> = subs.iter().map(String::as_str).collect();
                while s.len() > 1 && s.last() == Some(&"") {
                    s.pop();
                }
                s.join(&sep.subcomponent.to_string())
            })
            .collect();
        while comps.len() > 1 && comps.last().is_some_and(String::is_empty) {
            comps.pop();
        }
        for (i, c) in comps.iter().enumerate() {
            if i > 0 {
                out.push(sep.component);
            }
            out.push_str(c);
        }
        out
    }

    /// Is anything valued in this repetition?
    pub fn is_valued(&self) -> bool {
        self.components.iter().any(|c| c.iter().any(|s| !s.is_empty()))
    }
}

impl Segment {
    /// Field `n` (1-based).
    pub fn field(&self, n: u16) -> Option<&Field> {
        self.fields.get(n.max(1) as usize - 1)
    }

    /// The first repetition of field `n`, when valued.
    pub fn rep(&self, n: u16) -> Option<&Rep> {
        self.field(n).and_then(|f| f.reps.first()).filter(|r| r.is_valued())
    }

    /// Component `c` (1-based; 0 = the whole field's first component) of
    /// subcomponent `s` of field `n`'s first repetition, as text.
    pub fn value(&self, n: u16, c: u16, s: u16) -> &str {
        match self.rep(n) {
            Some(r) => r.component(c, s),
            None => "",
        }
    }
}

impl Message {
    /// Parse ER7 text. Segments end at `\r`, `\n` or `\r\n`; blank lines
    /// are skipped.
    pub fn parse(input: &[u8]) -> Result<Message, ParseError> {
        let text = core::str::from_utf8(input).map_err(|_| ParseError::NotText)?;
        let text = text.trim_start_matches(['\u{feff}', ' ', '\r', '\n']);
        if !text.starts_with("MSH") || text.len() < 8 {
            return Err(ParseError::NoHeader);
        }
        let mut chars = text[3..].chars();
        let field = chars.next().ok_or(ParseError::NoHeader)?;
        let enc: Vec<char> = chars.by_ref().take(4).collect();
        if enc.len() < 4 || enc.contains(&field) {
            return Err(ParseError::NoHeader);
        }
        let sep = Separators { field, component: enc[0], repetition: enc[1], escape: enc[2], subcomponent: enc[3] };
        let mut segments = Vec::new();
        for (i, line) in text.split(['\r', '\n']).enumerate() {
            let line = line.trim_end_matches([' ']);
            if line.is_empty() {
                continue;
            }
            if line.len() < 3 || !line.is_char_boundary(3) || !line[..3].bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(ParseError::BadSegment(i + 1));
            }
            let name = String::from(&line[..3]);
            let rest = line[3..].strip_prefix(sep.field).unwrap_or(&line[3..]);
            let mut fields: Vec<Field> = Vec::new();
            if name == "MSH" {
                // MSH-1 is the field separator, MSH-2 the encoding characters: literal, never unescaped
                fields.push(Field {
                    reps: alloc::vec![Rep { components: alloc::vec![alloc::vec![String::from(sep.field)]] }],
                });
                let enc_text: String = enc.iter().collect();
                fields.push(Field { reps: alloc::vec![Rep { components: alloc::vec![alloc::vec![enc_text]] }] });
                let after = rest.get(4..).map(|r| r.strip_prefix(sep.field).unwrap_or("")).unwrap_or("");
                for raw in split_top(after, sep.field) {
                    fields.push(parse_field(raw, &sep));
                }
            } else {
                for raw in split_top(rest, sep.field) {
                    fields.push(parse_field(raw, &sep));
                }
            }
            segments.push(Segment { name, fields });
        }
        Ok(Message { separators: sep, segments })
    }

    /// Every segment with this name, in order.
    pub fn segments_named<'m>(&'m self, name: &'m str) -> impl Iterator<Item = &'m Segment> + 'm {
        self.segments.iter().filter(move |s| s.name == name)
    }

    /// The message type (`MSH-9`: `ORU^R01` → `ORU_R01`), from the
    /// message code and trigger event.
    pub fn structure(&self) -> Option<String> {
        let msh = self.segments.first().filter(|s| s.name == "MSH")?;
        let rep = msh.rep(9)?;
        let code = rep.component(1, 0);
        let trigger = rep.component(2, 0);
        let structure = rep.component(3, 0);
        if !structure.is_empty() {
            return Some(String::from(structure));
        }
        if code.is_empty() || trigger.is_empty() {
            return None;
        }
        Some(alloc::format!("{code}_{trigger}"))
    }
}

fn split_top(s: &str, sep: char) -> Vec<&str> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split(sep).collect()
}

fn parse_field(raw: &str, sep: &Separators) -> Field {
    let reps = raw
        .split(sep.repetition)
        .map(|rep| Rep {
            components: rep
                .split(sep.component)
                .map(|comp| comp.split(sep.subcomponent).map(|sub| unescape(sub, sep)).collect())
                .collect(),
        })
        .collect();
    Field { reps }
}

/// Decode the standard escapes: `\F\` `\S\` `\T\` `\R\` `\E\` are the
/// separators, `\Xdd..\` hexadecimal bytes (as UTF-8), `\.br\` a line
/// break; an escape the standard does not define is kept as written.
pub fn unescape(s: &str, sep: &Separators) -> String {
    if !s.contains(sep.escape) {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(sep.escape) {
        out.push_str(&rest[..i]);
        let after = &rest[i + sep.escape.len_utf8()..];
        let Some(j) = after.find(sep.escape) else {
            out.push_str(&rest[i..]);
            return out;
        };
        let code = &after[..j];
        match code {
            "F" => out.push(sep.field),
            "S" => out.push(sep.component),
            "T" => out.push(sep.subcomponent),
            "R" => out.push(sep.repetition),
            "E" => out.push(sep.escape),
            ".br" => out.push('\n'),
            c if c.starts_with('X') && c.len() > 1 && c.len() % 2 == 1 => {
                let bytes: Option<Vec<u8>> =
                    (1..c.len()).step_by(2).map(|k| u8::from_str_radix(&c[k..k + 2], 16).ok()).collect();
                match bytes.and_then(|b| String::from_utf8(b).ok()) {
                    Some(text) => out.push_str(&text),
                    None => {
                        out.push(sep.escape);
                        out.push_str(code);
                        out.push(sep.escape);
                    }
                }
            }
            other => {
                out.push(sep.escape);
                out.push_str(other);
                out.push(sep.escape);
            }
        }
        rest = &after[j + sep.escape.len_utf8()..];
    }
    out.push_str(rest);
    out
}
