//! Audit: an `AuditEvent` for a read, query, create, update or delete of a
//! record, with or without a known patient subject, conformant to the IHE
//! Basic Audit Log Patterns (BALP) profile for that kind of event — built
//! through the lowering engine from what a host knows at the moment it
//! happens (who asked, from where, which server answered, what was touched,
//! when), and validated on the way out: an audit record is never trusted
//! unverified.
//!
//! The design (`docs/architecture.md`): audit is data, traces are plumbing,
//! correlated by id. The correlation is the request id — BALP's
//! `transaction` entity carrying `X-Request-Id` — given as `transaction`.
//!
//! What the host supplies is host-shaped, not slot-shaped:
//!
//! ```json
//! {
//!   "recorded": "2026-09-16T10:00:00Z",
//!   "client": {"who": "Device/app-7", "network": "10.0.0.7"},
//!   "server": {"who": {"display": "fhir.example.org"}, "network": "https://fhir.example.org/r4"},
//!   "user": {"who": "Practitioner/gp1", "name": "Dr Who"},
//!   "source": {"observer": "Device/audit-collector", "site": "clinic-1"},
//!   "patient": "Patient/p1",
//!   "data": "Observation/hr1",
//!   "transaction": "3f2a…"
//! }
//! ```
//!
//! `audit("patient-read", …)` turns that into the profile's inputs: the
//! agents' fixed types and the event's type, subtype, action and outcome
//! come from the profile; a network address is classified (an IP address,
//! a URI, an email, a machine name) for its `network.type`; a `who` given
//! as `Type/id` is a reference and as any other string a display name; a
//! query is base64-encoded as the profile requires. The BALP basic profiles
//! record *successful* events — a failed action is not one of them, and
//! is lowered from the base `AuditEvent` definition instead.
//!
//! What the profile leaves to the host and the host does not say is taken
//! from the pattern's own guidance, and is overridable: the user agent's
//! participation type for a create / update / delete is `author`, and the
//! data entity's role is `Domain Resource`. Nothing else is invented: a
//! required input the host leaves out is an error before anything is built.
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::compiled::CompiledProfile;
use crate::json::Value;
use crate::lower::{self, Lowered};
use crate::node::Node;
use crate::terminology::TerminologyProvider;
use crate::{generated, Validator};

const BALP: &str = "https://profiles.ihe.net/ITI/BALP/StructureDefinition/";

/// Which BALP basic event: the RESTful action, with or without a known
/// patient subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Read,
    PatientRead,
    Query,
    PatientQuery,
    Create,
    PatientCreate,
    Update,
    PatientUpdate,
    Delete,
    PatientDelete,
}

impl Kind {
    pub const ALL: [Kind; 10] = [
        Kind::Read,
        Kind::PatientRead,
        Kind::Query,
        Kind::PatientQuery,
        Kind::Create,
        Kind::PatientCreate,
        Kind::Update,
        Kind::PatientUpdate,
        Kind::Delete,
        Kind::PatientDelete,
    ];

    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "read" => Kind::Read,
            "patient-read" => Kind::PatientRead,
            "query" => Kind::Query,
            "patient-query" => Kind::PatientQuery,
            "create" => Kind::Create,
            "patient-create" => Kind::PatientCreate,
            "update" => Kind::Update,
            "patient-update" => Kind::PatientUpdate,
            "delete" => Kind::Delete,
            "patient-delete" => Kind::PatientDelete,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Read => "read",
            Kind::PatientRead => "patient-read",
            Kind::Query => "query",
            Kind::PatientQuery => "patient-query",
            Kind::Create => "create",
            Kind::PatientCreate => "patient-create",
            Kind::Update => "update",
            Kind::PatientUpdate => "patient-update",
            Kind::Delete => "delete",
            Kind::PatientDelete => "patient-delete",
        }
    }

    /// The BALP profile's canonical URL.
    pub fn url(self) -> &'static str {
        match self {
            Kind::Read => "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Read",
            Kind::PatientRead => "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientRead",
            Kind::Query => "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Query",
            Kind::PatientQuery => "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientQuery",
            Kind::Create => "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Create",
            Kind::PatientCreate => "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientCreate",
            Kind::Update => "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Update",
            Kind::PatientUpdate => "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientUpdate",
            Kind::Delete => "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Delete",
            Kind::PatientDelete => "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientDelete",
        }
    }

    /// Does the event carry a known patient subject?
    pub fn has_patient(self) -> bool {
        matches!(
            self,
            Kind::PatientRead | Kind::PatientQuery | Kind::PatientCreate | Kind::PatientUpdate | Kind::PatientDelete
        )
    }

    /// Is the event a query (an `entity:query` with the query, no data entity)?
    pub fn is_query(self) -> bool {
        matches!(self, Kind::Query | Kind::PatientQuery)
    }

    /// The default `subtype` — the FHIR RESTful interaction — where the
    /// profile binds it to a set rather than fixing it.
    fn default_subtype(self) -> &'static str {
        match self {
            Kind::Read | Kind::PatientRead => "read",
            Kind::Query | Kind::PatientQuery => "search-type",
            Kind::Update | Kind::PatientUpdate => "update",
            Kind::Create | Kind::PatientCreate => "create",
            Kind::Delete | Kind::PatientDelete => "delete",
        }
    }

    fn profile(self) -> Option<&'static CompiledProfile> {
        let url = self.url();
        debug_assert!(url.starts_with(BALP));
        generated::REGISTRY.iter().find(|p| p.url == url).copied()
    }
}

/// Why an audit record could not even be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditError {
    /// Not one of the kinds (`read`, `patient-read`, `query`, …).
    UnknownKind,
    /// The inputs are not a JSON object.
    InputsNotAnObject,
    /// The kind's profile is not in this registry.
    ProfileNotCompiled,
    /// A required input is absent.
    Missing(&'static str),
    /// An input that has no place in this kind of event (`patient` on a
    /// `read`, `query` on an `update`).
    Unexpected(&'static str),
    /// An input of the wrong shape (`client` not an object, `recorded` not a string).
    Malformed(&'static str),
    /// An input the audit surface does not know.
    Unknown(String),
}

impl core::fmt::Display for AuditError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AuditError::UnknownKind => f.write_str(
                "kind must be one of read, patient-read, query, patient-query, create, patient-create, update, patient-update, delete, patient-delete",
            ),
            AuditError::InputsNotAnObject => f.write_str("inputs must be a JSON object"),
            AuditError::ProfileNotCompiled => f.write_str("the kind's BALP profile is not compiled in this registry"),
            AuditError::Missing(what) => write!(f, "'{what}' is required"),
            AuditError::Unexpected(what) => write!(f, "'{what}' has no place in this kind of event"),
            AuditError::Malformed(what) => write!(f, "'{what}' is not the shape the audit surface takes"),
            AuditError::Unknown(what) => write!(f, "'{what}' is not an audit input"),
        }
    }
}

impl core::error::Error for AuditError {}

/// Standard base64 (RFC 4648 §4, with padding) — what a `base64Binary` carries.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

/// The `network.type` code for an address: `5` a URI, `4` an email address,
/// `2` an IP address (v4 or v6), `1` a machine name — deterministic, and a
/// host that knows better gives `{"address", "type"}` itself.
pub fn network_type(address: &str) -> &'static str {
    if address.contains("://") {
        return "5";
    }
    if address.contains('@') {
        return "4";
    }
    let ipv4 = address.matches('.').count() == 3 && address.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    let ipv6 = address.contains(':') && address.bytes().all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
    if ipv4 || ipv6 {
        return "2";
    }
    "1"
}

fn str_of<'v>(v: &'v Value<'_>) -> Option<&'v str> {
    match v {
        Value::Str(s) => Some(s.as_ref()),
        _ => None,
    }
}

fn member<'v, 'a>(kvs: &'v [(alloc::borrow::Cow<'a, str>, Value<'a>)], key: &str) -> Option<&'v Value<'a>> {
    kvs.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v)
}

/// A `who`: `Type/id` (or a URL) is a reference, any other string a display
/// name, an object is taken as the Reference it is.
fn who(v: &Value<'_>) -> Option<Node> {
    match v {
        Value::Str(s) if s.contains('/') => {
            Some(Node::Object(alloc::vec![("reference".into(), Node::Str(s.to_string()))]))
        }
        Value::Str(s) => Some(Node::Object(alloc::vec![("display".into(), Node::Str(s.to_string()))])),
        Value::Object(_) => Some(Node::from_value(v)),
        _ => None,
    }
}

/// A network access point: a string is classified, an object taken as given.
fn network(v: &Value<'_>) -> Option<Node> {
    match v {
        Value::Str(s) => Some(Node::Object(alloc::vec![
            ("address".into(), Node::Str(s.to_string())),
            ("type".into(), Node::Str(network_type(s).into())),
        ])),
        Value::Object(_) => Some(Node::from_value(v)),
        _ => None,
    }
}

fn coding(system: &str, code: &str) -> Node {
    Node::Object(alloc::vec![("system".into(), Node::Str(system.into())), ("code".into(), Node::Str(code.into()))])
}

fn concept(system: &str, code: &str) -> Node {
    Node::Object(alloc::vec![("coding".into(), Node::Array(alloc::vec![coding(system, code)]))])
}

struct Inputs {
    slots: Vec<(String, Node)>,
    profile: &'static CompiledProfile,
}

impl Inputs {
    fn has_slot(&self, path: &str) -> bool {
        self.profile.lowering.slots.iter().any(|s| s.path == path)
    }

    fn put(&mut self, path: &str, node: Node) {
        self.slots.push((path.into(), node));
    }

    /// One of the three agents. `client` and `server` need a network access
    /// point; `user` is the person (or system) on whose behalf the client
    /// acted, the requestor.
    fn agent(&mut self, label: &'static str, v: &Value<'_>, kind: Kind) -> Result<(), AuditError> {
        let Value::Object(kvs) = v else { return Err(AuditError::Malformed(label)) };
        let is_user = label == "user";
        for (k, _) in kvs {
            match k.as_ref() {
                "who" | "network" | "requestor" => {}
                "name" | "type" | "role" | "purposeOfUse" if is_user => {}
                _ => return Err(AuditError::Unknown(alloc::format!("{label}.{k}"))),
            }
        }
        let who_node = member(kvs, "who").and_then(who).ok_or(AuditError::Missing(match label {
            "client" => "client.who",
            "server" => "server.who",
            _ => "user.who",
        }))?;
        self.put(&alloc::format!("agent:{label}.who"), who_node);
        match member(kvs, "network") {
            Some(n) => {
                let node = network(n).ok_or(AuditError::Malformed(label))?;
                self.put(&alloc::format!("agent:{label}.network"), node);
            }
            None if !is_user => {
                return Err(AuditError::Missing(if label == "client" { "client.network" } else { "server.network" }))
            }
            None => {}
        }
        let requestor = match member(kvs, "requestor") {
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(AuditError::Malformed(label)),
            // the user is the requestor (the profile says so); a client or
            // server acts on its behalf
            None => is_user,
        };
        self.put(&alloc::format!("agent:{label}.requestor"), Node::Bool(requestor));
        if is_user {
            if let Some(name) = member(kvs, "name") {
                let Some(name) = str_of(name) else { return Err(AuditError::Malformed("user.name")) };
                self.put("agent:user.name", Node::Str(name.into()));
            }
            // the user's participation in a create / update / delete is
            // theirs to say (author, custodian, informant…); the pattern's
            // own examples say author, and that is the default
            if self.has_slot("agent:user.type") {
                let node = match member(kvs, "type") {
                    Some(Value::Str(code)) => {
                        concept("http://terminology.hl7.org/CodeSystem/v3-ParticipationType", code)
                    }
                    Some(v @ Value::Object(_)) => Node::from_value(v),
                    Some(_) => return Err(AuditError::Malformed("user.type")),
                    None => concept("http://terminology.hl7.org/CodeSystem/v3-ParticipationType", "AUT"),
                };
                self.put("agent:user.type", node);
            } else if member(kvs, "type").is_some() {
                return Err(AuditError::Unexpected("user.type"));
            }
            for (key, slot) in [("role", "agent:user.role"), ("purposeOfUse", "agent:user.purposeOfUse")] {
                if let Some(v) = member(kvs, key) {
                    if !matches!(v, Value::Object(_) | Value::Array(_)) {
                        return Err(AuditError::Malformed(if key == "role" {
                            "user.role"
                        } else {
                            "user.purposeOfUse"
                        }));
                    }
                    self.put(slot, Node::from_value(v));
                }
            }
        }
        let _ = kind;
        Ok(())
    }
}

/// Build the `AuditEvent` for `kind` from the host's inputs and validate it.
pub fn audit(kind: Kind, inputs: &Value<'_>, validator: Option<&Validator>) -> Result<Lowered, AuditError> {
    audit_with(kind, inputs, validator, &crate::terminology::Bundled)
}

/// [`audit`] with a terminology provider for the validation and for displays.
pub fn audit_with(
    kind: Kind,
    inputs: &Value<'_>,
    validator: Option<&Validator>,
    tx: &dyn TerminologyProvider,
) -> Result<Lowered, AuditError> {
    let Value::Object(kvs) = inputs else { return Err(AuditError::InputsNotAnObject) };
    let profile = kind.profile().ok_or(AuditError::ProfileNotCompiled)?;
    let mut ins = Inputs { slots: Vec::new(), profile };

    for (k, _) in kvs {
        match k.as_ref() {
            "recorded" | "client" | "server" | "user" | "source" | "data" | "patient" | "query" | "transaction"
            | "subtype" | "id" | "text" | "extension" => {}
            other => return Err(AuditError::Unknown(other.into())),
        }
    }

    // when
    let recorded = member(kvs, "recorded").ok_or(AuditError::Missing("recorded"))?;
    match recorded {
        Value::Str(s) if crate::temporal::parse(s).is_some_and(|t| t.has_date && t.precision >= 6 && t.has_tz) => {
            ins.put("recorded", Node::Str(s.to_string()));
        }
        _ => return Err(AuditError::Malformed("recorded")),
    }

    // who: the client that asked, the server that answered, the user on whose behalf
    ins.agent("client", member(kvs, "client").ok_or(AuditError::Missing("client"))?, kind)?;
    ins.agent("server", member(kvs, "server").ok_or(AuditError::Missing("server"))?, kind)?;
    if let Some(user) = member(kvs, "user") {
        ins.agent("user", user, kind)?;
    }

    // where it was observed
    let source = member(kvs, "source").ok_or(AuditError::Missing("source"))?;
    let Value::Object(skvs) = source else { return Err(AuditError::Malformed("source")) };
    let mut source_node: Vec<(String, Node)> = Vec::new();
    for (k, v) in skvs {
        match k.as_ref() {
            "observer" => {
                source_node.push(("observer".into(), who(v).ok_or(AuditError::Malformed("source.observer"))?))
            }
            "site" => source_node
                .push(("site".into(), Node::Str(str_of(v).ok_or(AuditError::Malformed("source.site"))?.into()))),
            "type" => match v {
                Value::Array(_) => source_node.push(("type".into(), Node::from_value(v))),
                Value::Object(_) => source_node.push(("type".into(), Node::Array(alloc::vec![Node::from_value(v)]))),
                _ => return Err(AuditError::Malformed("source.type")),
            },
            other => return Err(AuditError::Unknown(alloc::format!("source.{other}"))),
        }
    }
    if !source_node.iter().any(|(k, _)| k == "observer") {
        return Err(AuditError::Missing("source.observer"));
    }
    ins.put("source", Node::Object(source_node));

    // what was touched
    match member(kvs, "patient") {
        Some(p) if kind.has_patient() => {
            ins.put("entity:patient.what", who(p).ok_or(AuditError::Malformed("patient"))?)
        }
        Some(_) => return Err(AuditError::Unexpected("patient")),
        None if kind.has_patient() => return Err(AuditError::Missing("patient")),
        None => {}
    }
    match member(kvs, "query") {
        Some(q) if kind.is_query() => {
            let Some(q) = str_of(q) else { return Err(AuditError::Malformed("query")) };
            ins.put("entity:query.query", Node::Str(base64(q.as_bytes())));
        }
        Some(_) => return Err(AuditError::Unexpected("query")),
        None if kind.is_query() => return Err(AuditError::Missing("query")),
        None => {}
    }
    match member(kvs, "data") {
        Some(_) if kind.is_query() => return Err(AuditError::Unexpected("data")),
        Some(Value::Object(dkvs)) if member(dkvs, "what").is_some() => {
            // `{"what": …, "role": …}`
            for (k, v) in dkvs {
                match k.as_ref() {
                    "what" => ins.put("entity:data.what", who(v).ok_or(AuditError::Malformed("data.what"))?),
                    "role" if ins.has_slot("entity:data.role") => match v {
                        Value::Str(code) => ins
                            .put("entity:data.role", coding("http://terminology.hl7.org/CodeSystem/object-role", code)),
                        Value::Object(_) => ins.put("entity:data.role", Node::from_value(v)),
                        _ => return Err(AuditError::Malformed("data.role")),
                    },
                    "role" => return Err(AuditError::Unexpected("data.role")),
                    other => return Err(AuditError::Unknown(alloc::format!("data.{other}"))),
                }
            }
        }
        Some(d) => ins.put("entity:data.what", who(d).ok_or(AuditError::Malformed("data"))?),
        None if !kind.is_query() => return Err(AuditError::Missing("data")),
        None => {}
    }
    // the data entity's role where the profile asks for one: what the
    // pattern's own examples give a resource — a Domain Resource
    if ins.has_slot("entity:data.role") && !ins.slots.iter().any(|(k, _)| k == "entity:data.role") {
        ins.put("entity:data.role", coding("http://terminology.hl7.org/CodeSystem/object-role", "4"));
    }

    // the correlation: the request id, as BALP's transaction entity
    if let Some(t) = member(kvs, "transaction") {
        let Some(t) = str_of(t) else { return Err(AuditError::Malformed("transaction")) };
        ins.put(
            "entity:transaction.what",
            Node::Object(alloc::vec![(
                "identifier".into(),
                Node::Object(alloc::vec![("value".into(), Node::Str(t.into()))])
            )]),
        );
    }

    // the action the profile patterns (`R`, `E`, `C`, `U`, `D`) sits behind an
    // optional element: it is given, as the profile's own value, so it lands
    if let Some((_, action)) = profile.lowering.fixed.iter().find(|(p, _)| *p == "action") {
        ins.put("action", Node::Str((*action).into()));
    }

    // the RESTful interaction, where the profile binds rather than fixes it
    match member(kvs, "subtype") {
        Some(_) if !ins.has_slot("subtype") => return Err(AuditError::Unexpected("subtype")),
        Some(Value::Str(code)) => ins.put("subtype", coding("http://hl7.org/fhir/restful-interaction", code)),
        Some(v @ Value::Object(_)) => ins.put("subtype", Node::from_value(v)),
        Some(_) => return Err(AuditError::Malformed("subtype")),
        None if ins.has_slot("subtype") => {
            ins.put("subtype", coding("http://hl7.org/fhir/restful-interaction", kind.default_subtype()))
        }
        None => {}
    }

    // what any resource may carry, as given
    for key in ["id", "text", "extension"] {
        if let Some(v) = member(kvs, key) {
            ins.put(key, Node::from_value(v));
        }
    }

    let text = Node::Object(ins.slots).to_json();
    let parsed = crate::json::parse(text.as_bytes()).map_err(|_| AuditError::InputsNotAnObject)?;
    lower::lower_with(profile, &parsed, validator, tx).map_err(|e| match e {
        lower::LowerError::UnknownProfile => AuditError::ProfileNotCompiled,
        lower::LowerError::InputsNotAnObject => AuditError::InputsNotAnObject,
    })
}
