//! HL7 v2 → FHIR: a parsed message converted to a message `Bundle`
//! through the compiled `hl7.fhir.uv.v2mappings` maps, validated on the way
//! out.
//!
//! The message map for the structure (`ORU_R01`) says which segment fills
//! which resource; each segment map says which field lands where; each
//! datatype map says how a `CX` becomes an `Identifier`; each vocabulary
//! table says which FHIR code a v2 code is. A row the maps state but the
//! pod cannot execute (a narrative condition, a prose assignment, a target
//! the engine does not take) is applied never and *reported* whenever the
//! message carries data for it, so nothing a message says is dropped
//! unnamed. Beyond the maps, the converter has a few rules of its own, all
//! structural and all stated here:
//!
//! - a numbered instance (`DiagnosticReport[1]`) is shared along the
//!   structure: a segment in a group and one in a group inside it fill the
//!   same instance (`Converter::open`);
//! - a resource with a `subject` / `patient` / `beneficiary` and none set
//!   refers to the nearest Patient in the structure; one with an `encounter`
//!   and none set, to the nearest Encounter;
//! - a DiagnosticReport's `result` is the observations of its order group;
//! - the Bundle is a `message` whose first entry is the MessageHeader, with
//!   deterministic `urn:uuid` entries: the same message gives the same bundle;
//! - an identifier under `urn:ietf:rfc:3986` whose value is a bare OID or
//!   UUID is written as the URN the IG spells for the same value elsewhere
//!   (`urn_identifiers`);
//! - a coding-system name resolves to its uri from the standard names of
//!   table 0396 and the host's [`Options::code_systems`], else the row is
//!   reported ([`code_system_uri`]);
//! - a time stated without a zone is truncated to its date and reported,
//!   unless the host gives [`Options::timezone`];
//! - a list item a failed reference leaves with only the map's literals
//!   (`location[1].status` without its location) is dropped and reported
//!   (`prune_fixed_only`).
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::compiled::*;
use super::{generated, Message, ParseError, Rep, Segment};
use crate::node::Node;
use crate::terminology::TerminologyProvider;
use crate::{typing, Validation, Validator};

/// Conversion options (JSON: `{"timezone": "+10:00", "codeSystems":
/// {"L": "http://example.org/local-codes"}}`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// The zone a v2 timestamp without one is in — FHIR requires a zone
    /// once a time is stated; without this, such a timestamp is truncated
    /// to its date and reported.
    pub timezone: Option<String>,
    /// Coding-system names (`CWE.3`) the host knows the uri of, beyond the
    /// standard names of HL7 table 0396 the pod resolves itself.
    pub code_systems: Vec<(String, String)>,
}

/// A FHIR zone: `Z` or `±HH:MM`, the hour within ±14.
fn is_zone(s: &str) -> bool {
    if s == "Z" {
        return true;
    }
    let b = s.as_bytes();
    b.len() == 6
        && matches!(b[0], b'+' | b'-')
        && b[1..3].iter().all(u8::is_ascii_digit)
        && b[3] == b':'
        && b[4..6].iter().all(u8::is_ascii_digit)
        && s[1..3].parse::<u8>().is_ok_and(|h| h <= 14)
        && s[4..6].parse::<u8>().is_ok_and(|m| m < 60)
}

/// The uri of a coding-system name as HL7 table 0396 (and the FHIR
/// terminology registry) states it, for the names in general use;
/// `HL7nnnn` names the v2 table `nnnn`. A name not here is the host's to
/// add through [`Options::code_systems`], else the row is reported.
pub fn code_system_uri(name: &str) -> Option<String> {
    let known: &[(&str, &str)] = &[
        ("LN", "http://loinc.org"),
        ("SCT", "http://snomed.info/sct"),
        ("SNM", "http://snomed.info/sct"),
        ("UCUM", "http://unitsofmeasure.org"),
        ("ICD10", "http://hl7.org/fhir/sid/icd-10"),
        ("I10", "http://hl7.org/fhir/sid/icd-10"),
        ("ICD10CM", "http://hl7.org/fhir/sid/icd-10-cm"),
        ("I10C", "http://hl7.org/fhir/sid/icd-10-cm"),
        ("ICD10AM", "http://hl7.org/fhir/sid/icd-10-am"),
        ("I9C", "http://hl7.org/fhir/sid/icd-9-cm"),
        ("ICD9CM", "http://hl7.org/fhir/sid/icd-9-cm"),
        ("CPT", "http://www.ama-assn.org/go/cpt"),
        ("C4", "http://www.ama-assn.org/go/cpt"),
        ("CVX", "http://hl7.org/fhir/sid/cvx"),
        ("MVX", "http://hl7.org/fhir/sid/mvx"),
        ("NDC", "http://hl7.org/fhir/sid/ndc"),
        ("RXNORM", "http://www.nlm.nih.gov/research/umls/rxnorm"),
        ("ATC", "http://www.whocc.no/atc"),
        ("NUCC", "http://nucc.org/provider-taxonomy"),
        ("MDC", "urn:iso:std:iso:11073:10101"),
        ("ISO3166_1", "urn:iso:std:iso:3166"),
        ("ISO3166_2", "urn:iso:std:iso:3166:-2"),
        ("ISO4217", "urn:iso:std:iso:4217"),
    ];
    if let Some((_, uri)) = known.iter().find(|(n, _)| *n == name) {
        return Some((*uri).into());
    }
    let table = name.strip_prefix("HL7")?;
    if table.len() == 4 && table.bytes().all(|b| b.is_ascii_digit()) {
        return Some(alloc::format!("http://terminology.hl7.org/CodeSystem/v2-{table}"));
    }
    None
}

impl Options {
    pub fn from_json(bytes: &[u8]) -> Result<Options, ConvertError> {
        if bytes.iter().all(|b| b.is_ascii_whitespace()) {
            return Ok(Options::default());
        }
        let v = crate::json::parse(bytes).map_err(|_| ConvertError::BadOptions)?;
        let crate::json::Value::Object(kvs) = &v else { return Err(ConvertError::BadOptions) };
        let mut o = Options::default();
        for (k, v) in kvs {
            match (k.as_ref(), v) {
                ("timezone", crate::json::Value::Str(s)) => {
                    if !is_zone(s) {
                        return Err(ConvertError::BadOptions);
                    }
                    o.timezone = Some(s.to_string())
                }
                ("timezone", crate::json::Value::Null) => {}
                ("codeSystems", crate::json::Value::Object(m)) => {
                    for (name, uri) in m {
                        let crate::json::Value::Str(u) = uri else { return Err(ConvertError::BadOptions) };
                        if name.is_empty() || u.is_empty() || u.contains(char::is_whitespace) {
                            return Err(ConvertError::BadOptions);
                        }
                        o.code_systems.push((name.to_string(), u.to_string()));
                    }
                }
                ("codeSystems", crate::json::Value::Null) => {}
                _ => return Err(ConvertError::BadOptions),
            }
        }
        Ok(o)
    }
}

/// What the maps stated for data the message carried, and the pod did not
/// do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    /// Where in the message (`PID[1]-12`, `OBX[3]-5`).
    pub at: String,
    /// The FHIR target the map names.
    pub target: String,
    pub why: String,
}

/// The outcome: the message Bundle as JSON, its validation, what was
/// skipped, and the segments the structure did not place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub bundle: String,
    pub validation: Validation,
    pub skipped: Vec<Skip>,
    /// Segments the message structure has no row for (`ZPI[1]`), left out.
    pub unplaced: Vec<String>,
    /// Valued fields of placed segments that no map row names at all
    /// (`PID[1]-2`): the IG says nothing about them, so nothing was done —
    /// what a host must look at itself to know nothing was lost.
    pub unmapped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertError {
    Parse(ParseError),
    /// MSH-9 names no structure, or one the pod has no map for.
    UnknownStructure(String),
    BadOptions,
}

impl core::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ConvertError::Parse(e) => write!(f, "{e}"),
            ConvertError::UnknownStructure(s) if s.is_empty() => f.write_str("MSH-9 names no message structure"),
            ConvertError::UnknownStructure(s) => write!(f, "no map for message structure '{s}'"),
            ConvertError::BadOptions => f.write_str(
                "options must be a JSON object with an optional \"timezone\" (Z or ±HH:MM) and \"codeSystems\" (name → uri)",
            ),
        }
    }
}

impl core::error::Error for ConvertError {}

impl From<ParseError> for ConvertError {
    fn from(e: ParseError) -> Self {
        ConvertError::Parse(e)
    }
}

/// A resource being built: its type, its entry id, its body, and where in
/// the message structure it was opened.
struct Built {
    resource: &'static str,
    number: Option<u32>,
    scope: String,
    uuid: String,
    body: Node,
    /// A row carrying the message's data (not only a map literal) reached it.
    has_data: bool,
}

/// The list blocks of one segment application: (`extension[1]`) → index
/// in the list, and whether a data row (not only a fixed literal) reached
/// it. A `[k]` marker is the segment map's own numbering, so blocks live
/// as long as the application: two NTEs are two notes.
type Blocks = Vec<(String, usize, bool)>;

struct Converter<'a> {
    options: &'a Options,
    instances: Vec<Built>,
    skipped: Vec<Skip>,
    seq: u32,
    /// The message's own mark (a hash of its bytes): ids differ between
    /// messages and repeat for the same one.
    salt: String,
}

fn uuid(seed: &str) -> String {
    let h = crate::hash::sha256(seed.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let x = crate::hash::hex(&b);
    alloc::format!("{}-{}-{}-{}-{}", &x[..8], &x[8..12], &x[12..16], &x[16..20], &x[20..])
}

/// A v2 `DTM` (`YYYY[MM[DD[HH[MM[SS[.S+]]]]]][+/-ZZZZ]`) as a FHIR date /
/// dateTime / instant, or `None` (with the reason) when it cannot be one.
fn dtm(value: &str, ty: &str, tz_default: Option<&str>) -> Result<String, String> {
    let v = value.trim();
    let (main, zone) = match v.find(['+', '-']) {
        Some(i) => (&v[..i], Some(&v[i..])),
        None => (v, None),
    };
    let (digits, frac) = match main.find('.') {
        Some(i) => (&main[..i], Some(&main[i + 1..])),
        None => (main, None),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(alloc::format!("'{value}' is not a v2 date/time"));
    }
    let take = |a: usize, b: usize| digits.get(a..b);
    let (y, mo, d, h, mi, s) = (take(0, 4), take(4, 6), take(6, 8), take(8, 10), take(10, 12), take(12, 14));
    let Some(y) = y else { return Err(alloc::format!("'{value}' has no year")) };
    let mut date = String::from(y);
    if let Some(mo) = mo {
        date.push('-');
        date.push_str(mo);
        if let Some(d) = d {
            date.push('-');
            date.push_str(d);
        }
    }
    let zone = match zone {
        Some(z) if z.len() == 5 && z.is_ascii() && z[1..].bytes().all(|b| b.is_ascii_digit()) => {
            Some(alloc::format!("{}:{}", &z[..3], &z[3..]))
        }
        Some(_) => return Err(alloc::format!("'{value}' has a malformed zone")),
        None => tz_default.map(String::from),
    };
    match ty {
        "date" => {
            if crate::temporal::parse(&date).is_some_and(|t| t.has_date) {
                Ok(date)
            } else {
                Err(alloc::format!("'{value}' is not a calendar date"))
            }
        }
        _ => {
            let (Some(h), Some(mi)) = (h, mi) else {
                if ty == "instant" {
                    return Err(alloc::format!("'{value}' has no time, and an instant needs one"));
                }
                return Ok(date);
            };
            let Some(zone) = zone else {
                return Err(alloc::format!(
                    "'{value}' states a time without a zone and no default zone was given: truncated to {date}"
                ));
            };
            let s = s.unwrap_or("00");
            let mut out = alloc::format!("{date}T{h}:{mi}:{s}");
            if let Some(f) = frac.filter(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit())) {
                out.push('.');
                out.push_str(f);
            }
            out.push_str(&zone);
            if crate::temporal::parse(&out).is_some() {
                Ok(out)
            } else {
                Err(alloc::format!("'{value}' is not a valid date/time"))
            }
        }
    }
}

/// A segment another one's condition names (`IF OBR-2 NOT VALUED` on an
/// ORC row): the nearest one in the structure, or none.
type Lookup<'m> = dyn Fn(&str) -> Option<&'m Segment> + 'm;

fn eval_cond(e: &CondExpr, seg: &Segment, dt: Option<&Rep>, lookup: &Lookup<'_>) -> bool {
    let seg_for = |r: &Ref| -> Option<&Segment> {
        if r.name == seg.name {
            Some(seg)
        } else {
            lookup(r.name)
        }
    };
    let text = |r: &Ref| -> String {
        if r.field == 0 {
            // datatype-relative
            dt.map(|d| d.component(r.component, r.sub).to_string()).unwrap_or_default()
        } else {
            seg_for(r).map(|s| s.value(r.field, r.component, r.sub).to_string()).unwrap_or_default()
        }
    };
    let count = |r: &Ref| -> u32 {
        if r.field == 0 {
            return dt.map(|d| u32::from(d.is_valued())).unwrap_or(0);
        }
        seg_for(r)
            .and_then(|s| s.field(r.field))
            .map(|f| f.reps.iter().filter(|x| x.is_valued()).count() as u32)
            .unwrap_or(0)
    };
    let valued = |r: &Ref| -> bool {
        if r.field == 0 && r.component == 0 {
            // `IF ORC VALUED`: the segment is in scope; `IF XPN VALUED`: the value is
            lookup(r.name).is_some() || dt.is_some_and(Rep::is_valued)
        } else if r.component == 0 {
            seg_for(r).and_then(|s| s.rep(r.field)).is_some()
        } else {
            !text(r).is_empty()
        }
    };
    match e {
        CondExpr::Valued(r) => valued(r),
        CondExpr::NotValued(r) => !valued(r),
        CondExpr::Equals(r, v) => text(r) == *v,
        CondExpr::NotEquals(r, v) => text(r) != *v,
        CondExpr::In(r, vs) => vs.contains(&text(r).as_str()),
        CondExpr::NotIn(r, vs) => !vs.contains(&text(r).as_str()),
        CondExpr::CountGt(r, n) => count(r) > *n,
        CondExpr::CountLe(r, n) => count(r) <= *n,
        CondExpr::CountEq(r, n) => count(r) == *n,
        CondExpr::LengthGt(r, n) => text(r).chars().count() as u32 > *n,
        CondExpr::And(a, b) => eval_cond(a, seg, dt, lookup) && eval_cond(b, seg, dt, lookup),
        CondExpr::Or(a, b) => eval_cond(a, seg, dt, lookup) || eval_cond(b, seg, dt, lookup),
        CondExpr::Not(a) => !eval_cond(a, seg, dt, lookup),
    }
}

/// A concatenation's value (`"+"+XTN.5+" "+XTN.6`): every component it names
/// valued, else `None` — the row's conditions normally say so themselves.
fn concat_value(parts: &[Part], seg: &Segment, dt: Option<&Rep>) -> Option<String> {
    let mut out = String::new();
    for part in parts {
        match part {
            Part::Lit(s) => out.push_str(s),
            Part::Ref(r) => {
                let v = if r.field == 0 {
                    dt.map(|d| d.component(r.component, r.sub)).unwrap_or("")
                } else {
                    seg.value(r.field, r.component, r.sub)
                };
                if v.is_empty() {
                    return None;
                }
                out.push_str(v);
            }
        }
    }
    Some(out)
}

/// The FHIR type of the element at `path` under `ctx`, per the base spec.
fn element_type(ctx: &'static str, path: &str) -> Option<&'static str> {
    let mut c: &'static str = ctx;
    let mut ty = None;
    for seg in path.split('.') {
        let name = seg.split('[').next().unwrap_or(seg);
        let ch = typing::child(c, name)?;
        c = ch.ctx;
        ty = Some(ch.ty);
    }
    ty
}

/// The type a row's target has: the base spec's word for the element
/// first (a map may say `CodeableConcept` of `manifestation.text`, or
/// `quantity`, `name`, nothing at all), the row's own when the spec has
/// none.
fn target_type(ctx: &'static str, row: &Row) -> &'static str {
    if let Some(t) = element_type(ctx, row.target) {
        return t;
    }
    if is_primitive(row.ty) {
        return row.ty;
    }
    typing::known_type(row.ty).unwrap_or("string")
}

/// The block a failed reference empties: the deepest list item on the way
/// to the target (`location[1]` of `location[1].location`), keyed as
/// [`place`] keys it.
fn marked_prefix(ctx: &'static str, target: &str) -> Option<String> {
    let mut prefix = String::new();
    let mut found = None;
    let mut c: &'static str = ctx;
    let segs: Vec<&str> = target.split('.').collect();
    for seg in &segs[..segs.len().saturating_sub(1)] {
        let name = seg.split('[').next().unwrap_or(seg);
        if !prefix.is_empty() {
            prefix.push('.');
        }
        if typing::repeats(c, name) && !seg.contains('[') {
            prefix.push_str(name);
            prefix.push_str("[1]");
            found = Some(prefix.clone());
        } else {
            prefix.push_str(seg);
            if seg.contains('[') && !seg.ends_with("[n]") {
                found = Some(prefix.clone());
            }
        }
        c = typing::child(c, name).map(|ch| ch.ctx).unwrap_or("Element");
    }
    found
}

/// A v2 type whose whole value is one text (as opposed to a composite such
/// as PL or XCN, which a primitive target takes only through a datatype map).
fn is_simple_v2(ty: &str) -> bool {
    matches!(
        ty,
        "ST" | "ID" | "IS" | "NM" | "SI" | "DT" | "DTM" | "TM" | "TX" | "FT" | "GTS" | "varies" | "SNM" | "NUL" | ""
    )
}

fn is_primitive(ty: &str) -> bool {
    matches!(
        ty,
        "string"
            | "code"
            | "uri"
            | "url"
            | "canonical"
            | "id"
            | "markdown"
            | "boolean"
            | "integer"
            | "positiveInt"
            | "unsignedInt"
            | "decimal"
            | "date"
            | "dateTime"
            | "instant"
            | "time"
            | "base64Binary"
            | "oid"
            | "uuid"
    )
}

/// The datatype map for a v2 type to a FHIR type, the unqualified one
/// first.
fn datatype_map(v2: &str, fhir: &str) -> Option<&'static DatatypeMap> {
    let fhir_lc: String = fhir.to_ascii_lowercase();
    let find = |f: &str| {
        generated::DATATYPES
            .iter()
            .find(|m| m.v2.eq_ignore_ascii_case(v2) && m.fhir == f && m.qualifier.is_empty())
            .or_else(|| generated::DATATYPES.iter().find(|m| m.v2.eq_ignore_ascii_case(v2) && m.fhir == f))
    };
    // a map to `uri` serves every uri-shaped primitive, one to `string` every text one
    let kin = match fhir_lc.as_str() {
        "url" | "canonical" | "oid" | "uuid" => Some("uri"),
        "markdown" | "id" => Some("string"),
        _ => None,
    };
    find(&fhir_lc).or_else(|| kin.and_then(find))
}

fn table(id: &str) -> Option<&'static TableMap> {
    generated::TABLES.iter().find(|t| t.id == id)
}

/// Place `node` at `path` under `root` (`identifier[2].type.coding.code`),
/// making arrays where the base spec says the element repeats. A `[k]`
/// marker names a block: the first row to reach it appends an item and the
/// block remembers the index; later rows with the same marker fill that
/// item. A path whose last element repeats and has no marker appends.
/// `append_all`: the node is a list of items to append one by one.
fn place(
    root: &mut Node,
    blocks: &mut Blocks,
    ctx: &'static str,
    path: &str,
    node: Node,
    data: bool,
) -> Result<(), String> {
    let segs: Vec<&str> = path.split('.').collect();
    let mut cur: &mut Node = root;
    let mut c: &'static str = ctx;
    let mut prefix = String::new();
    for (i, seg) in segs.iter().enumerate() {
        let (mut name, marker) = match seg.find('[') {
            Some(j) => (&seg[..j], Some(seg[j + 1..].trim_end_matches(']'))),
            None => (*seg, None),
        };
        let last = i + 1 == segs.len();
        let child = typing::child(c, name);
        // an extension on a primitive lives in the primitive's `_name` carrier
        let carrier: String;
        if !last && child.is_some_and(|ch| is_primitive(ch.ty)) && segs[i + 1].starts_with("extension") {
            carrier = alloc::format!("_{name}");
            name = &carrier;
        }
        let repeats = typing::repeats(c, name);
        let child_ctx = if name.starts_with('_') { "Element" } else { child.map(|ch| ch.ctx).unwrap_or("Element") };
        if !prefix.is_empty() {
            prefix.push('.');
        }
        // an unmarked repeating container (`extension.url`, `name` then
        // `name[1].given`) is the map's first item: the same block as `name[1]`
        let key: String = if repeats && marker.is_none() { alloc::format!("{name}[1]") } else { (*seg).into() };
        prefix.push_str(&key);
        if last {
            if repeats {
                let arr = cur.member_or_insert(name, Node::Array(Vec::new()));
                if !matches!(arr, Node::Array(_)) {
                    return Err(alloc::format!("{prefix} already holds a value"));
                }
                let Node::Array(items) = arr else { unreachable!() };
                let register = |blocks: &mut Blocks, at: usize| {
                    if marker.is_none() && !blocks.iter().any(|(p, _, _)| *p == prefix) {
                        blocks.push((prefix.clone(), at, data));
                    }
                };
                match (marker, node) {
                    // a block of several items (a repeating field): appended, the block at the first
                    (Some(k), Node::Array(xs)) if k != "n" => {
                        match blocks.iter_mut().find(|(p, _, _)| *p == prefix) {
                            Some((_, _, d)) => *d |= data,
                            None => blocks.push((prefix.clone(), items.len(), data)),
                        }
                        items.extend(xs);
                    }
                    (Some(k), node) if k != "n" => match blocks.iter_mut().find(|(p, _, _)| *p == prefix) {
                        Some((_, idx, d)) => {
                            *d |= data;
                            merge(&mut items[*idx], node)
                        }
                        None => {
                            items.push(node);
                            blocks.push((prefix.clone(), items.len() - 1, data));
                        }
                    },
                    (_, Node::Array(xs)) => {
                        if !xs.is_empty() {
                            register(blocks, items.len());
                        }
                        items.extend(xs)
                    }
                    (_, node) => {
                        register(blocks, items.len());
                        items.push(node)
                    }
                }
            } else {
                let slot = cur.member_or_insert(name, Node::Null);
                let compatible =
                    matches!(slot, Node::Null) || matches!((&*slot, &node), (Node::Object(_), Node::Object(_)));
                if !compatible {
                    return Err(alloc::format!("{prefix} already holds a value"));
                }
                merge(slot, node);
            }
            return Ok(());
        }
        if repeats {
            let arr = cur.member_or_insert(name, Node::Array(Vec::new()));
            if !matches!(arr, Node::Array(_)) {
                return Err(alloc::format!("{prefix} already holds a value"));
            }
            let Node::Array(items) = arr else { unreachable!() };
            let idx = match blocks.iter_mut().find(|(p, _, _)| *p == prefix) {
                Some((_, idx, d)) => {
                    *d |= data;
                    *idx
                }
                None => {
                    items.push(Node::Object(Vec::new()));
                    blocks.push((prefix.clone(), items.len() - 1, data));
                    items.len() - 1
                }
            };
            cur = &mut items[idx];
        } else {
            cur = cur.member_or_insert(name, Node::Object(Vec::new()));
            if !matches!(cur, Node::Object(_)) {
                return Err(alloc::format!("{prefix} already holds a value"));
            }
        }
        c = child_ctx;
    }
    Ok(())
}

/// The node a block-qualified path names (`location[1]` → that item).
fn node_at<'n>(root: &'n mut Node, path: &str, blocks: &Blocks) -> Option<&'n mut Node> {
    let mut cur = root;
    let mut prefix = String::new();
    for seg in path.split('.') {
        let name = seg.split('[').next().unwrap_or(seg);
        if !prefix.is_empty() {
            prefix.push('.');
        }
        prefix.push_str(seg);
        let Node::Object(kvs) = cur else { return None };
        let (_, child) = kvs.iter_mut().find(|(k, _)| k == name)?;
        cur = match child {
            Node::Array(items) => {
                let idx = blocks.iter().find(|(p, _, _)| *p == prefix).map(|(_, i, _)| *i)?;
                items.get_mut(idx)?
            }
            other => other,
        };
    }
    Some(cur)
}

/// Merge `node` into `slot`: objects member by member, anything else replaced.
fn merge(slot: &mut Node, node: Node) {
    match (slot, node) {
        (Node::Object(a), Node::Object(b)) => {
            for (k, v) in b {
                match a.iter_mut().find(|(ak, _)| *ak == k) {
                    Some((_, av)) => merge(av, v),
                    None => a.push((k, v)),
                }
            }
        }
        (s, n) => *s = n,
    }
}

impl<'a> Converter<'a> {
    fn skip(&mut self, at: String, target: &str, why: String) {
        self.skipped.push(Skip { at, target: target.into(), why });
    }

    /// A primitive from v2 text by FHIR type.
    fn primitive(&mut self, at: &str, target: &str, ty: &str, text: &str) -> Option<Node> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        match ty {
            "date" | "dateTime" | "instant" => match dtm(t, ty, self.options.timezone.as_deref()) {
                Ok(v) => Some(Node::Str(v)),
                Err(why) => {
                    // a time without a zone: the date is kept, the loss reported
                    if let Some(date) = why.rsplit("truncated to ").nth(0).filter(|_| why.contains("truncated to ")) {
                        self.skip(at.into(), target, why.clone());
                        return Some(Node::Str(date.into()));
                    }
                    self.skip(at.into(), target, why);
                    None
                }
            },
            "time" => {
                let d: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
                if d.len() >= 4 {
                    let s = d.get(4..6).unwrap_or("00");
                    Some(Node::Str(alloc::format!("{}:{}:{}", &d[..2], &d[2..4], s)))
                } else {
                    self.skip(at.into(), target, alloc::format!("'{t}' is not a time"));
                    None
                }
            }
            "boolean" => match t {
                "Y" | "true" | "TRUE" | "T" => Some(Node::Bool(true)),
                "N" | "false" | "FALSE" | "F" => Some(Node::Bool(false)),
                _ => {
                    self.skip(at.into(), target, alloc::format!("'{t}' is not a boolean"));
                    None
                }
            },
            "integer" | "positiveInt" | "unsignedInt" | "decimal" => {
                if crate::json::parse(t.as_bytes()).is_ok_and(|v| matches!(v, crate::json::Value::Number(_))) {
                    Some(Node::Number(t.into()))
                } else {
                    self.skip(at.into(), target, alloc::format!("'{t}' is not a number"));
                    None
                }
            }
            _ => Some(Node::Str(t.into())),
        }
    }

    /// A v2 code through a vocabulary table, for a `code` target (the FHIR
    /// code) or a coded target (a Coding).
    fn via_table(&mut self, at: &str, target: &str, ty: &str, id: &str, text: &str) -> Option<Node> {
        let Some(t) = table(id) else {
            self.skip(at.into(), target, alloc::format!("table {id} is not compiled"));
            return None;
        };
        // a bare code target takes the pair from the table's own target system;
        // a coded target keeps whichever system the pair names
        let pair = if is_primitive(ty) {
            t.pairs.iter().find(|(from, _, system)| *from == text && *system == t.target)
        } else {
            t.pairs.iter().find(|(from, _, _)| *from == text)
        };
        match pair {
            Some((_, to, system)) => {
                if is_primitive(ty) {
                    Some(Node::Str((*to).into()))
                } else {
                    let coding = Node::Object(alloc::vec![
                        ("system".into(), Node::Str((*system).into())),
                        ("code".into(), Node::Str((*to).into())),
                    ]);
                    if ty.eq_ignore_ascii_case("Coding") {
                        Some(coding)
                    } else {
                        Some(Node::Object(alloc::vec![("coding".into(), Node::Array(alloc::vec![coding]))]))
                    }
                }
            }
            None => {
                if is_primitive(ty) {
                    self.skip(at.into(), target, alloc::format!("code '{text}' is not in {id}"));
                    None
                } else {
                    // the v2 coding itself, as the message stated it
                    let coding = Node::Object(alloc::vec![
                        ("system".into(), Node::Str(t.source.into())),
                        ("code".into(), Node::Str(text.into())),
                    ]);
                    if ty.eq_ignore_ascii_case("Coding") {
                        Some(coding)
                    } else {
                        Some(Node::Object(alloc::vec![("coding".into(), Node::Array(alloc::vec![coding]))]))
                    }
                }
            }
        }
    }

    /// A FHIR datatype value from a v2 repetition through a datatype map.
    fn datatype(
        &mut self,
        at: &str,
        map: &'static DatatypeMap,
        rep: &Rep,
        seg: &Segment,
        lookup: &Lookup<'_>,
    ) -> Option<Node> {
        let ctx = type_named(map.fhir).unwrap_or("Element");
        let mut body = Node::Object(Vec::new());
        let mut blocks = Vec::new();
        let mut any = false;
        for row in map.rows {
            let at_row = alloc::format!("{at}.{}", row.component);
            let source_text = rep.component(row.component, row.sub);
            match row.cond {
                Cond::Skipped(why) => {
                    if !source_text.is_empty() {
                        self.skip(at_row, row.target, why.into());
                    }
                    continue;
                }
                Cond::When(e) => {
                    if !eval_cond(e, seg, Some(rep), lookup) {
                        continue;
                    }
                }
                Cond::Always => {}
            }
            let ty: &'static str = if row.target == "$value" {
                if is_primitive(row.ty) {
                    row.ty
                } else {
                    typing::known_type(row.ty).unwrap_or("string")
                }
            } else {
                target_type(ctx, row)
            };
            let node = if let Some(fixed) = row.fixed {
                if source_text.is_empty() {
                    continue;
                }
                Some(Node::Str(fixed.into()))
            } else if let Some(parts) = row.concat {
                if source_text.is_empty() {
                    continue;
                }
                match concat_value(parts, seg, Some(rep)) {
                    Some(text) => self.primitive(&at_row, row.target, ty, &text),
                    None => None,
                }
            } else if source_text.is_empty() {
                None
            } else if row.target == "$value" {
                self.primitive(&at_row, row.target, ty, source_text)
            } else if let Via::Table(id) = row.via {
                self.via_table(&at_row, row.target, ty, id, source_text)
            } else if row.via == Via::CodeSystem {
                self.code_system(&at_row, row.target, source_text)
            } else if is_primitive(ty) {
                self.primitive(&at_row, row.target, ty, source_text)
            } else if let Some(inner) = datatype_map(row.src_ty, ty) {
                // a component with a datatype of its own: its subcomponents are its components
                let sub_rep = Rep {
                    components: rep
                        .components
                        .get(row.component.max(1) as usize - 1)
                        .map(|subs| subs.iter().map(|s| alloc::vec![s.clone()]).collect())
                        .unwrap_or_default(),
                };
                self.datatype(&at_row, inner, &sub_rep, seg, lookup)
            } else {
                self.skip(at_row.clone(), row.target, alloc::format!("no datatype map {} to {ty}", row.src_ty));
                None
            };
            let Some(node) = node else { continue };
            if row.target == "$value" {
                return Some(node);
            }
            if let Err(why) = place(&mut body, &mut blocks, ctx, row.target, node, row.fixed.is_none()) {
                self.skip(at_row, row.target, why);
                continue;
            }
            any = true;
        }
        if !any {
            return None;
        }
        Some(body)
    }

    /// A coding-system name to its uri: the standard names, then the host's.
    fn code_system(&mut self, at: &str, target: &str, name: &str) -> Option<Node> {
        if let Some((_, uri)) = self.options.code_systems.iter().find(|(n, _)| n == name) {
            return Some(Node::Str(uri.clone()));
        }
        match code_system_uri(name) {
            Some(uri) => Some(Node::Str(uri)),
            None => {
                self.skip(at.into(), target, alloc::format!("coding system '{name}' has no known uri"));
                None
            }
        }
    }

    /// Apply a segment map to a resource instance.
    fn apply_segment(
        &mut self,
        seg: &Segment,
        seg_at: &str,
        map: &'static SegmentMap,
        inst: usize,
        lookup: &Lookup<'_>,
    ) {
        let ctx: &'static str = self.instances[inst].resource;
        let mut blocks: Blocks = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        // `component[n]`: one item per repetition of the repeating field the
        // map spreads over it; a row from a field that does not repeat
        // (`component[n].code` from OBX-3) reaches every item
        let mut n_items: Vec<(String, usize)> = Vec::new();
        for row in map.rows {
            if matches!(row.cond, Cond::Skipped(_)) || row.field == 0 {
                continue;
            }
            let Some(prefix) = row.target.split('.').next().filter(|s| s.ends_with("[n]")) else { continue };
            let reps = seg.field(row.field).map(|f| f.reps.iter().filter(|r| r.is_valued()).count()).unwrap_or(0);
            match n_items.iter_mut().find(|(p, _)| p == prefix) {
                Some((_, n)) => *n = (*n).max(reps),
                None => n_items.push((prefix.into(), reps)),
            }
        }
        for row in map.rows {
            // a container row (`MSH-3 → source[1]`, no type): its parts are rows of their own
            if row.ty.is_empty()
                && row.fixed.is_none()
                && row.reference.is_none()
                && !matches!(row.cond, Cond::Skipped(_))
            {
                continue;
            }
            let ty: &'static str = target_type(ctx, row);
            let at = if row.field == 0 { seg_at.to_string() } else { alloc::format!("{seg_at}-{}", row.field) };
            let field = if row.field == 0 { None } else { seg.field(row.field) };
            let valued = row.field == 0 || field.is_some_and(|f| f.reps.iter().any(Rep::is_valued));
            // a condition on the field's own components (`CWE.3` of SCH-7) sees its first repetition
            let own = field.and_then(|f| f.reps.iter().find(|r| r.is_valued()));
            // `varies` (OBX-5) is the type OBX-2 names; `TS` is DTM before v2.6
            let src_ty: &str = if row.src_ty == "varies" && seg.name == "OBX" {
                match seg.value(2, 0, 0) {
                    "TS" => "DTM",
                    "" => row.src_ty,
                    t => t,
                }
            } else {
                row.src_ty
            };
            match row.cond {
                Cond::Skipped(why) => {
                    if valued && row.field != 0 {
                        self.skip(at, row.target, why.into());
                    }
                    continue;
                }
                Cond::When(e) => {
                    if !eval_cond(e, seg, own, lookup) {
                        continue;
                    }
                }
                Cond::Always => {}
            }
            if !valued {
                continue;
            }
            // the reps this row reads: all of a repeating field, or the first
            let reps: Vec<&Rep> = match field {
                Some(f) => f.reps.iter().filter(|r| r.is_valued()).collect(),
                None => Vec::new(),
            };
            let mut nodes: Vec<Node> = Vec::new();
            if row.fixed.is_none() && valued {
                self.instances[inst].has_data = true;
            }
            if let Some(fixed) = row.fixed {
                nodes.push(Node::Str(fixed.into()));
            } else if let Some(parts) = row.concat {
                if let Some(text) = concat_value(parts, seg, reps.first().copied()) {
                    if let Some(n) = self.primitive(&at, row.target, ty, &text) {
                        nodes.push(n);
                    }
                }
            } else if let Some(refty) = row.reference {
                // a referenced resource built from the field by its datatype map
                let Some(dmap) = datatype_map(src_ty, refty) else {
                    let _ = ty;
                    self.skip(at, row.target, alloc::format!("no datatype map {src_ty} to {refty}"));
                    continue;
                };
                for (i, rep) in reps.iter().enumerate() {
                    let at_rep = if reps.len() > 1 { alloc::format!("{at}[{}]", i + 1) } else { at.clone() };
                    let Some(body) = self.datatype(&at_rep, dmap, rep, seg, lookup) else {
                        if let Some(p) = marked_prefix(ctx, row.target) {
                            failed.push(p);
                        }
                        continue;
                    };
                    let rt = typing::known_type(refty).unwrap_or("Basic");
                    let id = self.open(rt, None, &self.instances[inst].scope.clone());
                    self.instances[id].has_data = true;
                    if let Node::Object(kvs) = &mut self.instances[id].body {
                        if let Node::Object(b) = body {
                            kvs.extend(b);
                        }
                    }
                    let url = self.instances[id].uuid.clone();
                    nodes.push(Node::Object(alloc::vec![("reference".into(), Node::Str(url))]));
                }
            } else {
                for (i, rep) in reps.iter().enumerate() {
                    let at_rep = if reps.len() > 1 { alloc::format!("{at}[{}]", i + 1) } else { at.clone() };
                    let text = rep.component(row.component, row.sub);
                    if row.component != 0 && text.is_empty() {
                        continue;
                    }
                    let node = if let Via::Table(id) = row.via {
                        self.via_table(&at_rep, row.target, ty, id, text)
                    } else if row.via == Via::CodeSystem {
                        self.code_system(&at_rep, row.target, text)
                    } else if is_primitive(ty) && (row.component != 0 || is_simple_v2(src_ty)) {
                        self.primitive(&at_rep, row.target, ty, text)
                    } else if let Some(dmap) = datatype_map(src_ty, ty) {
                        self.datatype(&at_rep, dmap, rep, seg, lookup)
                    } else {
                        self.skip(at_rep, row.target, alloc::format!("no datatype map {src_ty} to {ty}"));
                        None
                    };
                    if let Some(n) = node {
                        nodes.push(n);
                    }
                }
            }
            if nodes.is_empty() {
                continue;
            }
            // where repetitions go: the target itself when it repeats, else
            // the nearest unmarked repeating element above it (`AL1-5` ×2 →
            // `reaction.manifestation[2].text`); nowhere = reported
            let segs: Vec<&str> = row.target.split('.').collect();
            let (last_repeats, spread_at) = {
                let mut c: &'static str = ctx;
                let mut rep = false;
                let mut spread = None;
                for (i, s) in segs.iter().enumerate() {
                    let name = s.split('[').next().unwrap_or(s);
                    if i + 1 == segs.len() {
                        rep = typing::repeats(c, name);
                    } else {
                        if typing::repeats(c, name) && !s.contains('[') {
                            spread = Some(i);
                        }
                        c = typing::child(c, name).map(|ch| ch.ctx).unwrap_or("Element");
                    }
                }
                (rep, spread)
            };
            if last_repeats {
                if let Err(why) = place(
                    &mut self.instances[inst].body,
                    &mut blocks,
                    ctx,
                    row.target,
                    Node::Array(nodes),
                    row.fixed.is_none(),
                ) {
                    self.skip(at.clone(), row.target, why);
                }
                continue;
            }
            // `X[n]` at the head: item k of the map's spread
            let each = segs.first().filter(|s| s.ends_with("[n]")).and_then(|p| n_items.iter().find(|(q, _)| q == p));
            let item_path = |k: usize| -> String {
                let mut parts: Vec<String> = segs.iter().map(|s| (*s).into()).collect();
                if k > 0 {
                    parts[0] = alloc::format!("{}[r{}]", segs[0].trim_end_matches("[n]"), k + 1);
                }
                parts.join(".")
            };
            let count = nodes.len();
            if let Some((_, n)) = each {
                if count == 1 && *n > 1 {
                    // one value for every item
                    let node = nodes.into_iter().next().unwrap();
                    for k in 0..*n {
                        if let Err(why) = place(
                            &mut self.instances[inst].body,
                            &mut blocks,
                            ctx,
                            &item_path(k),
                            node.clone(),
                            row.fixed.is_none(),
                        ) {
                            self.skip(at.clone(), row.target, why);
                        }
                    }
                    continue;
                }
                for (k, node) in nodes.into_iter().enumerate() {
                    if let Err(why) = place(
                        &mut self.instances[inst].body,
                        &mut blocks,
                        ctx,
                        &item_path(k),
                        node,
                        row.fixed.is_none(),
                    ) {
                        self.skip(at.clone(), row.target, why);
                    }
                }
                continue;
            }
            for (i, node) in nodes.into_iter().enumerate() {
                let path: String = if i == 0 {
                    row.target.into()
                } else {
                    match spread_at {
                        Some(j) => {
                            let mut parts: Vec<String> = segs.iter().map(|s| (*s).into()).collect();
                            parts[j] = alloc::format!("{}[r{}]", segs[j], i + 1);
                            parts.join(".")
                        }
                        None => {
                            self.skip(
                                alloc::format!("{at}[{}]", i + 1),
                                row.target,
                                alloc::format!("repetition {} of {count}: the target does not repeat", i + 1),
                            );
                            continue;
                        }
                    }
                };
                if let Err(why) =
                    place(&mut self.instances[inst].body, &mut blocks, ctx, &path, node, row.fixed.is_none())
                {
                    self.skip(at.clone(), row.target, why);
                }
            }
        }
        prune_fixed_only(&mut self.instances[inst].body, &mut blocks, &failed, seg_at, &mut self.skipped);
    }

    /// Open a resource instance. A numbered one (`DiagnosticReport[1]`) is
    /// shared along the structure: a segment in a group and one in a
    /// group inside it fill the same instance (ORC in COMMON_ORDER and OBR
    /// in ORDER_OBSERVATION both say `DiagnosticReport[1]`), which then
    /// belongs to the outer group.
    fn open(&mut self, resource: &'static str, number: Option<u32>, scope: &str) -> usize {
        if let Some(k) = number {
            let related = |a: &str, b: &str| {
                a == b || a.starts_with(&alloc::format!("{b}.")) || b.starts_with(&alloc::format!("{a}."))
            };
            if let Some(i) = self
                .instances
                .iter()
                .position(|x| x.resource == resource && x.number == Some(k) && related(&x.scope, scope))
            {
                if self.instances[i].scope.len() > scope.len() {
                    self.instances[i].scope = scope.into();
                }
                return i;
            }
        }
        self.seq += 1;
        let uuid = alloc::format!(
            "urn:uuid:{}",
            uuid(&alloc::format!("v2:{}:{}:{resource}:{scope}:{}", self.salt, self.seq, number.unwrap_or(0)))
        );
        self.instances.push(Built {
            resource,
            number,
            scope: scope.into(),
            uuid,
            body: Node::Object(alloc::vec![("resourceType".into(), Node::Str(resource.into()))]),
            has_data: false,
        });
        self.instances.len() - 1
    }
}

/// Drop the list items a failed reference left with only the map's
/// literals (`location[1].status` without its location): a block under a
/// failed reference's marker with no data of its own and none in a block
/// above or below it is no data of the message — removed, and reported.
/// Empty carriers left behind go with it.
fn prune_fixed_only(body: &mut Node, blocks: &mut Blocks, failed: &[String], at: &str, skipped: &mut Vec<Skip>) {
    if failed.is_empty() {
        return;
    }
    let has_data = |p: &str, blocks: &Blocks| -> bool {
        blocks.iter().any(|(q, _, d)| {
            *d && (q == p || q.starts_with(&alloc::format!("{p}.")) || p.starts_with(&alloc::format!("{q}.")))
        })
    };
    let under_failed = |p: &str| failed.iter().any(|f| p == f || p.starts_with(&alloc::format!("{f}.")));
    let mut fixed_only: Vec<(String, usize)> = blocks
        .iter()
        .filter(|(p, _, _)| under_failed(p) && !has_data(p, blocks))
        .map(|(p, i, _)| (p.clone(), *i))
        .collect();
    // outermost first: an inner block goes with its outer one
    fixed_only.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then(b.1.cmp(&a.1)));
    let mut done: Vec<String> = Vec::new();
    for (prefix, idx) in fixed_only {
        if done.iter().any(|d| prefix.starts_with(&alloc::format!("{d}."))) {
            continue;
        }
        let list_path = prefix.rsplit_once('.').map(|(_, l)| l).unwrap_or(&prefix);
        let list_name = list_path.split('[').next().unwrap_or(list_path);
        let parent_path = prefix.rsplit_once('.').map(|(p, _)| p);
        let parent: Option<&mut Node> = match parent_path {
            None => Some(&mut *body),
            Some(pp) => node_at(body, pp, blocks),
        };
        if let Some(Node::Object(kvs)) = parent {
            if let Some((_, Node::Array(items))) = kvs.iter_mut().find(|(k, _)| k == list_name) {
                if idx < items.len() {
                    items.remove(idx);
                }
            }
            kvs.retain(|(_, v)| !matches!(v, Node::Array(xs) if xs.is_empty()));
        }
        skipped.push(Skip {
            at: at.into(),
            target: prefix.clone(),
            why: "nothing of the message reached it: only the map's literals".into(),
        });
        // later items of the same list moved down one
        let list_prefix = alloc::format!("{}[", prefix.split('[').next().unwrap_or(&prefix));
        for (p, i, _) in blocks.iter_mut() {
            if p.starts_with(&list_prefix) && !p[list_prefix.len()..].contains('.') && *i > idx {
                *i -= 1;
            }
        }
        blocks.retain(|(p, _, _)| *p != prefix && !p.starts_with(&alloc::format!("{prefix}.")));
        done.push(prefix);
    }
    drop_empty(body);
}

/// Remove empty objects and arrays a pruning left behind (`type: {}`).
fn drop_empty(node: &mut Node) -> bool {
    match node {
        Node::Object(kvs) => {
            kvs.retain_mut(|(_, v)| !drop_empty(v));
            kvs.is_empty()
        }
        Node::Array(xs) => {
            xs.retain_mut(|v| !drop_empty(v));
            xs.is_empty()
        }
        _ => false,
    }
}

/// See the identifier rule in [`convert_with`]: `1.2.3` → `urn:oid:1.2.3`,
/// a UUID → `urn:uuid:…`, anything else left as written.
fn urn_identifiers(node: &mut Node) {
    match node {
        Node::Object(kvs) => {
            let rfc = kvs.iter().any(|(k, v)| k == "system" && matches!(v, Node::Str(s) if s == "urn:ietf:rfc:3986"));
            if rfc {
                if let Some((_, Node::Str(v))) = kvs.iter_mut().find(|(k, _)| k == "value") {
                    let is_oid = !v.is_empty()
                        && v.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                        && !v.starts_with('.')
                        && !v.ends_with('.')
                        && !v.contains("..");
                    let is_uuid = v.len() == 36
                        && v.bytes().enumerate().all(|(i, b)| {
                            if matches!(i, 8 | 13 | 18 | 23) {
                                b == b'-'
                            } else {
                                b.is_ascii_hexdigit()
                            }
                        });
                    if is_oid {
                        *v = alloc::format!("urn:oid:{v}");
                    } else if is_uuid {
                        *v = alloc::format!("urn:uuid:{}", v.to_ascii_lowercase());
                    }
                }
            }
            for (_, v) in kvs.iter_mut() {
                urn_identifiers(v);
            }
        }
        Node::Array(xs) => {
            for x in xs.iter_mut() {
                urn_identifiers(x);
            }
        }
        _ => {}
    }
}

/// HL7 table 0354: the trigger events each compiled message structure
/// serves, for a message that does not name its structure in MSH-9.3.
const TRIGGERS: &[(&str, &[&str], &str)] = &[
    ("ADT", &["A01", "A04", "A08", "A13"], "ADT_A01"),
    ("ADT", &["A02"], "ADT_A02"),
    ("ADT", &["A05", "A14", "A28", "A31"], "ADT_A05"),
    ("ADT", &["A06", "A07"], "ADT_A06"),
    ("ADT", &["A09", "A10"], "ADT_A09"),
    ("ADT", &["A11"], "ADT_A11"),
    ("ADT", &["A17"], "ADT_A17"),
    ("MDM", &["T02", "T04", "T06", "T08", "T10"], "MDM_T02"),
    ("OML", &["O21"], "OML_O21"),
    ("ORM", &["O01"], "ORM_O01"),
    ("ORU", &["R01"], "ORU_R01"),
    (
        "SIU",
        &["S12", "S13", "S14", "S15", "S16", "S17", "S18", "S19", "S20", "S21", "S22", "S23", "S24", "S26"],
        "SIU_S12",
    ),
    ("VXU", &["V04"], "VXU_V04"),
];

/// The message structure to convert with: MSH-9.3 when it names one the pod
/// compiles, else the structure table 0354 gives the trigger event, else
/// `code_trigger` (which then names no map, and says so).
fn structure_of(message: &Message) -> String {
    let compiled = |s: &str| generated::MESSAGES.iter().any(|m| m.name == s);
    let Some(msh) = message.segments.first().filter(|s| s.name == "MSH") else { return String::new() };
    let Some(rep) = msh.rep(9) else { return String::new() };
    let (code, trigger, named) = (rep.component(1, 0), rep.component(2, 0), rep.component(3, 0));
    if !named.is_empty() && compiled(named) {
        return named.into();
    }
    if let Some((_, _, st)) = TRIGGERS.iter().find(|(c, ts, _)| *c == code && ts.contains(&trigger)) {
        return (*st).into();
    }
    message.structure().unwrap_or_default()
}

/// A node of the message structure: a segment (with its rows) or a group.
struct GNode {
    name: String,
    is_group: bool,
    min: i32,
    max: i32,
    children: Vec<GNode>,
    /// Indices into the message map's rows (a segment node).
    rows: Vec<usize>,
    /// The segment names that can begin this node.
    first_set: Vec<String>,
}

fn build_tree(map: &'static MessageMap) -> GNode {
    let mut root = GNode {
        name: map.name.into(),
        is_group: true,
        min: 1,
        max: 1,
        children: Vec::new(),
        rows: Vec::new(),
        first_set: Vec::new(),
    };
    for (i, row) in map.rows.iter().enumerate() {
        let mut node = &mut root;
        let mut path = String::new();
        for part in row.group.split('.').skip(1) {
            if !path.is_empty() {
                path.push('.');
            }
            path.push_str(part);
            let full = alloc::format!("{}.{path}", map.name);
            let pos = match node.children.iter().position(|c| c.is_group && c.name == part) {
                Some(p) => p,
                None => {
                    let (min, max) =
                        map.groups.iter().find(|g| g.path == full).map(|g| (g.min, g.max)).unwrap_or((0, -1));
                    node.children.push(GNode {
                        name: part.into(),
                        is_group: true,
                        min,
                        max,
                        children: Vec::new(),
                        rows: Vec::new(),
                        first_set: Vec::new(),
                    });
                    node.children.len() - 1
                }
            };
            node = &mut node.children[pos];
        }
        let pos = match node.children.iter().position(|c| !c.is_group && c.name == row.segment) {
            Some(p) => p,
            None => {
                node.children.push(GNode {
                    name: row.segment.into(),
                    is_group: false,
                    min: row.min,
                    max: row.max,
                    children: Vec::new(),
                    rows: Vec::new(),
                    first_set: alloc::vec![row.segment.into()],
                });
                node.children.len() - 1
            }
        };
        node.children[pos].rows.push(i);
    }
    fn first_sets(node: &mut GNode) {
        if !node.is_group {
            return;
        }
        for child in node.children.iter_mut() {
            first_sets(child);
        }
        let mut set: Vec<String> = Vec::new();
        for child in node.children.iter() {
            for f in &child.first_set {
                if !set.contains(f) {
                    set.push(f.clone());
                }
            }
            if child.min >= 1 {
                break;
            }
        }
        node.first_set = set;
    }
    first_sets(&mut root);
    root
}

/// The message structure the pod walks for `name` (`ORU_R01`), one line per
/// segment or group with its cardinality and the segments that can begin
/// it — for a host or a reviewer to see what the walk expects.
pub fn structure_tree(name: &str) -> Option<String> {
    let map = generated::MESSAGES.iter().find(|m| m.name == name)?;
    fn render(n: &GNode, depth: usize, out: &mut String) {
        for _ in 0..depth {
            out.push_str("  ");
        }
        let max = if n.max < 0 { String::from("*") } else { alloc::format!("{}", n.max) };
        if n.is_group {
            out.push_str(&alloc::format!("{} [{}..{}] begins with {}\n", n.name, n.min, max, n.first_set.join(" | ")));
        } else {
            out.push_str(&alloc::format!("{} [{}..{}] rows {}\n", n.name, n.min, max, n.rows.len()));
        }
        for c in &n.children {
            render(c, depth + 1, out);
        }
    }
    let mut out = String::new();
    render(&build_tree(map), 0, &mut out);
    Some(out)
}

/// The type the maps spell in lower case (`humanname`, `codeableconcept`).
fn type_named(lower: &str) -> Option<&'static str> {
    typing::type_names().find(|t| t.eq_ignore_ascii_case(lower))
}

/// The FHIR resource type the maps spell in lower case (`diagnosticreport`).
fn resource_type_of(lower: &str) -> Option<&'static str> {
    crate::typemap::RESOURCES.iter().copied().find(|r| r.eq_ignore_ascii_case(lower))
}

/// Convert `input` (ER7) to a message Bundle.
pub fn convert(input: &[u8], options: &Options, validator: &Validator) -> Result<Converted, ConvertError> {
    convert_with(input, options, validator, &crate::terminology::Bundled)
}

/// [`convert`] with a terminology provider for the validation.
pub fn convert_with(
    input: &[u8],
    options: &Options,
    validator: &Validator,
    tx: &dyn TerminologyProvider,
) -> Result<Converted, ConvertError> {
    let message = Message::parse(input)?;
    let structure = structure_of(&message);
    let map = generated::MESSAGES
        .iter()
        .find(|m| m.name == structure)
        .ok_or_else(|| ConvertError::UnknownStructure(structure.clone()))?;
    let salt = crate::hash::hex(&crate::hash::sha256(input)[..16]);
    let mut cv = Converter { options, instances: Vec::new(), skipped: Vec::new(), seq: 0, salt };
    let mut unplaced = Vec::new();
    let mut unmapped: Vec<String> = Vec::new();

    // the structure walk: the message structure as a tree (segments and
    // groups, each with its cardinality — the groups' from the standard,
    // compiled in), matched greedily in order: a segment is the next
    // expected one, another occurrence of the last, or the start of a group
    // that may (still) occur; what matches nothing at this level closes the
    // group and is tried one level up; what matches nowhere is unplaced
    let tree = build_tree(map);
    struct Frame<'t> {
        node: &'t GNode,
        occurrence: u32,
        cursor: usize,
        counts: Vec<u32>,
    }
    struct Placed<'r> {
        seg: usize,
        rows: &'r [usize],
        scope: String,
        /// The group the segment sits in may occur once at most.
        single: bool,
    }
    let mut stack: Vec<Frame<'_>> =
        alloc::vec![Frame { node: &tree, occurrence: 1, cursor: 0, counts: alloc::vec![0; tree.children.len()] }];
    let mut placed: Vec<Placed<'_>> = Vec::new();
    let mut counts_by_name: Vec<(String, u32)> = Vec::new(); // segment name → occurrences seen, for labels
    let mut labels: Vec<String> = Vec::new();
    for (si, seg) in message.segments.iter().enumerate() {
        let n = match counts_by_name.iter_mut().find(|(s, _)| *s == seg.name) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                counts_by_name.push((seg.name.clone(), 1));
                1
            }
        };
        let seg_at = alloc::format!("{}[{n}]", seg.name);
        labels.push(seg_at.clone());
        loop {
            let depth = stack.len();
            let frame = &mut stack[depth - 1];
            let node = frame.node;
            let mut found = None;
            for ci in frame.cursor..node.children.len() {
                let child = &node.children[ci];
                let count = frame.counts[ci];
                let can_repeat = child.max < 0 || count < child.max as u32;
                let matches = if child.is_group { child.first_set.contains(&seg.name) } else { child.name == seg.name };
                if !matches {
                    continue;
                }
                if ci == frame.cursor && count > 0 && !can_repeat {
                    continue;
                }
                found = Some(ci);
                break;
            }
            match found {
                Some(ci) => {
                    frame.cursor = ci;
                    frame.counts[ci] += 1;
                    let occ = frame.counts[ci];
                    let child = &node.children[ci];
                    if child.is_group {
                        let counts = alloc::vec![0; child.children.len()];
                        stack.push(Frame { node: child, occurrence: occ, cursor: 0, counts });
                        continue;
                    }
                    let scope = stack
                        .iter()
                        .map(|f| alloc::format!("{}#{}", f.node.name, f.occurrence))
                        .collect::<Vec<_>>()
                        .join(".");
                    let single = stack.last().is_some_and(|f| f.node.max == 1);
                    placed.push(Placed { seg: si, rows: &child.rows, scope, single });
                    break;
                }
                None => {
                    if stack.len() == 1 {
                        unplaced.push(seg_at.clone());
                        break;
                    }
                    stack.pop();
                }
            }
        }
    }

    // the rows, in message order; a condition naming another segment finds
    // the nearest one in the structure: in this occurrence or an enclosing
    // one, or in a once-only group beside it (ORC in COMMON_ORDER for the
    // OBR of the same order)
    let lookup_in = |name: &str, scope: &str| -> Option<&Segment> {
        let mut best: Option<(usize, usize)> = None; // (shared prefix length, placed index)
        for (pi, p) in placed.iter().enumerate() {
            if message.segments[p.seg].name != name {
                continue;
            }
            let parent = p.scope.rsplit_once('.').map(|(a, _)| a).unwrap_or(&p.scope);
            let within = |s: &str| scope == s || scope.starts_with(&alloc::format!("{s}."));
            if !(within(&p.scope) || (p.single && within(parent))) {
                continue;
            }
            let shared = p.scope.split('.').zip(scope.split('.')).take_while(|(a, b)| a == b).count();
            if best.is_none_or(|(b, _)| shared > b) {
                best = Some((shared, pi));
            }
        }
        best.map(|(_, pi)| &message.segments[placed[pi].seg])
    };
    for p in &placed {
        let seg = &message.segments[p.seg];
        let seg_at = labels[p.seg].clone();
        let scope = p.scope.clone();
        let lookup = |name: &str| lookup_in(name, &scope);
        let mut applied: Vec<(&str, Instance)> = Vec::new();
        let mut deferred: Vec<(&str, Instance, &str)> = Vec::new();
        // the fields any map for this segment names, applicable or not
        let mut named: Vec<u16> = Vec::new();
        for &ri in p.rows {
            if let Some(smap) = map.rows[ri].map.and_then(|id| generated::SEGMENTS.iter().find(|m| m.id == id)) {
                named.extend(smap.rows.iter().map(|r| r.field).filter(|f| *f != 0));
            }
        }
        for (k, field) in seg.fields.iter().enumerate() {
            let n = k as u16 + 1;
            if seg.name == "MSH" && n <= 2 {
                continue;
            }
            if field.reps.iter().any(Rep::is_valued) && !named.contains(&n) && !p.rows.is_empty() {
                unmapped.push(alloc::format!("{seg_at}-{n}"));
            }
        }
        for &ri in p.rows {
            let row = &map.rows[ri];
            match row.cond {
                Cond::Skipped(why) => {
                    deferred.push((row.resource, row.instance, why));
                    continue;
                }
                Cond::When(e) => {
                    if !eval_cond(e, seg, None, &lookup) {
                        continue;
                    }
                }
                Cond::Always => {}
            }
            let Some(map_id) = row.map else { continue };
            let Some(smap) = generated::SEGMENTS.iter().find(|m| m.id == map_id) else { continue };
            let Some(rt) = resource_type_of(row.resource) else {
                cv.skip(seg_at.clone(), row.resource, alloc::format!("'{}' is not a resource type", row.resource));
                continue;
            };
            let number = match row.instance {
                Instance::Each => None,
                Instance::Numbered(k) => Some(k),
            };
            let inst = cv.open(rt, number, &scope);
            cv.apply_segment(seg, &seg_at, smap, inst, &lookup);
            applied.push((row.resource, row.instance));
        }
        // an alternative the pod could not judge is a skip only when no
        // sibling row for the same resource applied
        for (res, instance, why) in deferred {
            if !applied.iter().any(|(r, i)| *r == res && *i == instance) {
                let target = if res.is_empty() { seg.name.as_str() } else { res };
                cv.skip(seg_at.clone(), target, why.into());
            }
        }
    }

    // the converter's own structural rules
    let patients: Vec<(String, String)> =
        cv.instances.iter().filter(|i| i.resource == "Patient").map(|i| (i.scope.clone(), i.uuid.clone())).collect();
    let encounters: Vec<(String, String)> =
        cv.instances.iter().filter(|i| i.resource == "Encounter").map(|i| (i.scope.clone(), i.uuid.clone())).collect();
    // the nearest one in the structure: the longest shared group path (the
    // PATIENT group's patient reaches its PATIENT_RESULT's orders; a second
    // PATIENT_RESULT has its own)
    let in_scope = |set: &[(String, String)], scope: &str| -> Option<String> {
        let shared = |a: &str, b: &str| a.split('.').zip(b.split('.')).take_while(|(x, y)| x == y).count();
        let mut best: Option<(usize, &String)> = None;
        for (s, u) in set {
            let n = shared(s, scope);
            if best.is_none_or(|(b, _)| n > b) {
                best = Some((n, u));
            }
        }
        best.map(|(_, u)| u.clone())
    };
    let observations: Vec<(String, String)> = cv
        .instances
        .iter()
        .filter(|i| i.resource == "Observation" && i.number == Some(2))
        .map(|i| (i.scope.clone(), i.uuid.clone()))
        .collect();
    for inst in cv.instances.iter_mut() {
        let subject_key = match inst.resource {
            "Observation"
            | "DiagnosticReport"
            | "Encounter"
            | "Condition"
            | "Procedure"
            | "ServiceRequest"
            | "Specimen"
            | "MedicationRequest"
            | "MedicationAdministration"
            | "DocumentReference"
            | "CareTeam"
            | "EpisodeOfCare"
            | "Basic"
            | "Appointment" => Some("subject"),
            "AllergyIntolerance" | "Immunization" | "RelatedPerson" | "EpisodeOfCare2" => Some("patient"),
            "Coverage" => Some("beneficiary"),
            _ => None,
        };
        let Node::Object(kvs) = &mut inst.body else { continue };
        if let Some(key) = subject_key {
            if !kvs.iter().any(|(k, _)| k == key) {
                if let Some(p) = in_scope(&patients, &inst.scope) {
                    if inst.resource == "Appointment" {
                        // Appointment has no subject: participants carry actors; left alone
                    } else {
                        kvs.push((key.into(), Node::Object(alloc::vec![("reference".into(), Node::Str(p))])));
                    }
                }
            }
        }
        if matches!(
            inst.resource,
            "Observation" | "DiagnosticReport" | "Condition" | "Procedure" | "ServiceRequest" | "MedicationRequest"
        ) && !kvs.iter().any(|(k, _)| k == "encounter")
        {
            if let Some(e) = in_scope(&encounters, &inst.scope) {
                kvs.push(("encounter".into(), Node::Object(alloc::vec![("reference".into(), Node::Str(e))])));
            }
        }
        if inst.resource == "DiagnosticReport" && !kvs.iter().any(|(k, _)| k == "result") {
            let prefix = alloc::format!("{}.", inst.scope);
            let results: Vec<Node> = observations
                .iter()
                .filter(|(s, _)| s.starts_with(&prefix) || *s == inst.scope)
                .map(|(_, u)| Node::Object(alloc::vec![("reference".into(), Node::Str(u.clone()))]))
                .collect();
            if !results.is_empty() {
                kvs.push(("result".into(), Node::Array(results)));
            }
        }
    }

    // an identifier under `urn:ietf:rfc:3986` is a URI: a bare OID or UUID
    // the maps put there (an HD's universal ID) is written as the URN the
    // IG itself spells for the same value elsewhere (`"urn:oid:"+HD.2`)
    for inst in cv.instances.iter_mut() {
        urn_identifiers(&mut inst.body);
    }

    // the Bundle: a message, the MessageHeader first
    let mut entries: Vec<Node> = Vec::new();
    let mut bundle_fields: Vec<(String, Node)> = Vec::new();
    let mut header_first: Vec<usize> = Vec::new();
    let mut others: Vec<usize> = Vec::new();
    let mut literal_only: Vec<&'static str> = Vec::new();
    for (i, inst) in cv.instances.iter().enumerate() {
        match inst.resource {
            "Bundle" => {
                if let Node::Object(kvs) = &inst.body {
                    for (k, v) in kvs {
                        if k != "resourceType" {
                            bundle_fields.push((k.clone(), v.clone()));
                        }
                    }
                }
            }
            "MessageHeader" => header_first.push(i),
            _ => others.push(i),
        }
    }
    for i in header_first.into_iter().chain(others) {
        let inst = &cv.instances[i];
        // an instance no row filled beyond its type carries nothing: left out;
        // one only the map's literals reached (`Account.status = active` from
        // a PID) states nothing of the message: left out, and said so
        let filled = matches!(&inst.body, Node::Object(kvs) if kvs.len() > 1);
        if !filled {
            continue;
        }
        if !inst.has_data {
            literal_only.push(inst.resource);
            continue;
        }
        let mut body = inst.body.clone();
        if let Node::Object(kvs) = &mut body {
            kvs.insert(1, ("id".into(), Node::Str(inst.uuid.trim_start_matches("urn:uuid:").into())));
        }
        entries.push(Node::Object(alloc::vec![
            ("fullUrl".into(), Node::Str(inst.uuid.clone())),
            ("resource".into(), body),
        ]));
    }
    let mut bundle: Vec<(String, Node)> = alloc::vec![("resourceType".into(), Node::Str("Bundle".into()))];
    bundle.push(("id".into(), Node::Str(uuid(&alloc::format!("v2:bundle:{}", cv.salt)))));
    let mut has_type = false;
    for (k, v) in bundle_fields {
        if k == "type" {
            has_type = true;
        }
        bundle.push((k, v));
    }
    if !has_type {
        bundle.push(("type".into(), Node::Str("message".into())));
    }
    bundle.push(("entry".into(), Node::Array(entries)));
    for r in literal_only {
        cv.skip(String::from("-"), r, "nothing of the message reached it: only the map's literals".into());
    }
    // the same row reported once per place it applies, not once per attempt
    let mut skipped: Vec<Skip> = Vec::new();
    for k in cv.skipped {
        if !skipped.contains(&k) {
            skipped.push(k);
        }
    }
    let text = Node::Object(bundle).to_json();
    let validation = validator.validate_with(text.as_bytes(), tx);
    Ok(Converted { bundle: text, validation, skipped, unplaced, unmapped })
}

impl Validator {
    /// Convert an HL7 v2 message (ER7 text) to a FHIR message Bundle; see
    /// [`v2::convert`](crate::v2::convert).
    pub fn v2(&self, message: &[u8], options: &Options) -> Result<Converted, ConvertError> {
        convert(message, options, self)
    }

    /// [`Validator::v2`] with a terminology provider for the validation.
    pub fn v2_with(
        &self,
        message: &[u8],
        options: &Options,
        tx: &dyn TerminologyProvider,
    ) -> Result<Converted, ConvertError> {
        convert_with(message, options, self, tx)
    }
}
