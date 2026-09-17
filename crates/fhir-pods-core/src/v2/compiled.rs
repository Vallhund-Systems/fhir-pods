//! The intermediate representation of the HL7 v2-to-FHIR mappings the
//! generator (`tools/gen_v2.py`) compiles from `hl7.fhir.uv.v2mappings`:
//! datatype maps, segment maps, message maps and vocabulary tables — every
//! row either executable or skipped with its reason.

/// One mapping row: a v2 source (a field of the segment, or a component
/// of the datatype) to a FHIR target path, with the target's type, the
/// resource type it references (`assigner(Organization)`), a fixed value
/// the row assigns, the vocabulary table it maps through, and the
/// condition under which it applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The source as the map writes it (`PID-7`, `OBX-5.1`, `XPN.1`).
    pub src: &'static str,
    /// The source's v2 datatype as the map states it (`XPN`, `CX`, `DTM`,
    /// `varies`), which picks the datatype map.
    pub src_ty: &'static str,
    /// Field number (0 for a datatype-relative source).
    pub field: u16,
    /// Component number (0 = the whole field).
    pub component: u16,
    /// Subcomponent number (0 = the whole component).
    pub sub: u16,
    /// The FHIR path, resource- or datatype-relative: `name[1].given[2]`,
    /// `[1].family`, `$value`, `identifier[n]`.
    pub target: &'static str,
    /// The target's FHIR type as the map states it (`HumanName`, `string`,
    /// `Reference`, `code`).
    pub ty: &'static str,
    /// The resource type a reference target points at (`Organization`).
    pub reference: Option<&'static str>,
    /// A literal the row assigns instead of the source's value.
    pub fixed: Option<&'static str>,
    /// A value the row builds from literals and components (`"+"+XTN.5+" "+XTN.6`).
    pub concat: Option<&'static [Part]>,
    pub via: Via,
    pub cond: Cond,
}

/// One piece of a concatenation: a literal, or a component's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Lit(&'static str),
    Ref(Ref),
}

/// How a value is mapped on the way: through a vocabulary table, or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    None,
    /// A `TABLES` entry, by id.
    Table(&'static str),
    /// The map says a mapping exists but does not name it: the value is
    /// carried as given.
    Unspecified,
    /// A coding-system name (`LN`, `SCT`, `HL70078`) to its uri: the IG
    /// says a vocabulary table gives it and ships none, so the converter
    /// resolves the standard names of table 0396 and what the host adds.
    CodeSystem,
}

/// When a row applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    Always,
    When(&'static CondExpr),
    /// The row is not executed: the reason, for the converter's report.
    Skipped(&'static str),
}

/// A source the conditions name: a field of a segment (`PID-7`, `OBX-5.1`)
/// or a component of the datatype being mapped (`XPN.12`, name = the
/// datatype).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ref {
    pub name: &'static str,
    pub field: u16,
    pub component: u16,
    pub sub: u16,
}

/// The computable subset of the maps' conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondExpr {
    Valued(Ref),
    NotValued(Ref),
    Equals(Ref, &'static str),
    NotEquals(Ref, &'static str),
    In(Ref, &'static [&'static str]),
    NotIn(Ref, &'static [&'static str]),
    /// More than n repetitions.
    CountGt(Ref, u32),
    /// At most n repetitions.
    CountLe(Ref, u32),
    CountEq(Ref, u32),
    /// The value's length exceeds n characters.
    LengthGt(Ref, u32),
    And(&'static CondExpr, &'static CondExpr),
    Or(&'static CondExpr, &'static CondExpr),
    Not(&'static CondExpr),
}

/// A v2 datatype to a FHIR datatype (`XPN` → `HumanName`), possibly
/// qualified (`cx-mother-to-relatedperson`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatatypeMap {
    pub id: &'static str,
    pub v2: &'static str,
    pub qualifier: &'static str,
    /// The FHIR type, lower-cased as the map id has it (`humanname`).
    pub fhir: &'static str,
    pub rows: &'static [Row],
}

/// A v2 segment to a FHIR resource (`PID` → `Patient`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentMap {
    pub id: &'static str,
    pub segment: &'static str,
    pub qualifier: &'static str,
    /// The resource type, lower-cased as the map id has it (`patient`).
    pub resource: &'static str,
    pub rows: &'static [Row],
}

/// Which resource instance a message row's segment contributes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instance {
    /// A new resource per segment occurrence.
    Each,
    /// The k-th instance of that resource type within the nearest
    /// repeating group: rows of the same number share it.
    Numbered(u32),
}

/// One row of a message map: a segment at its place in the message
/// structure, and the resource it contributes to through a segment map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgRow {
    /// The structural path (`ORU_R01.PATIENT_RESULT.PATIENT.PID`).
    pub path: &'static str,
    /// The enclosing group (`ORU_R01.PATIENT_RESULT.PATIENT`).
    pub group: &'static str,
    pub segment: &'static str,
    pub min: i32,
    /// `-1` for unbounded.
    pub max: i32,
    pub resource: &'static str,
    pub instance: Instance,
    /// Where in the resource the segment's rows land (`note` for
    /// `Observation[2].note`); empty when the resource itself.
    pub at: &'static str,
    /// The segment map that fills the resource.
    pub map: Option<&'static str>,
    pub cond: Cond,
}

/// A v2 message structure (`ORU_R01`) to the resources of a Bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageMap {
    pub id: &'static str,
    pub name: &'static str,
    pub rows: &'static [MsgRow],
}

/// A vocabulary table: v2 code → FHIR code, with the target system per pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableMap {
    pub id: &'static str,
    pub source: &'static str,
    pub target: &'static str,
    pub pairs: &'static [(&'static str, &'static str, &'static str)],
}
