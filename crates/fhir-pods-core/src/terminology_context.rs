//! Explicit resource ownership of complete terminology tables. These operational
//! bindings are not FHIR resources and never rewrite exported canonical URLs.
use crate::document::Document;
use crate::json::{self, Value};
use crate::terminology::{Bundled, Chain, Expansions, Membership, TerminologyProvider};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub const FORMAT: &str = "fhir-pods.terminology-contexts.v1";

#[derive(Debug, Clone, Default)]
pub struct ResourceContexts {
    contexts: Vec<Context>,
}
#[derive(Debug, Clone)]
struct Context {
    table: Expansions,
    resources: Vec<(String, String)>,
}

fn object<'v, 'a>(value: &'v Value<'a>, keys: &[&str]) -> Option<&'v [(alloc::borrow::Cow<'a, str>, Value<'a>)]> {
    let Value::Object(fields) = value else { return None };
    if fields.len() != keys.len()
        || keys.iter().any(|key| fields.iter().filter(|(k, _)| k.as_ref() == *key).count() != 1)
    {
        return None;
    }
    Some(fields)
}
fn member<'v, 'a>(fields: &'v [(alloc::borrow::Cow<'a, str>, Value<'a>)], key: &str) -> Option<&'v Value<'a>> {
    fields.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v)
}
fn text<'v, 'a>(fields: &'v [(alloc::borrow::Cow<'a, str>, Value<'a>)], key: &str) -> Option<&'v str> {
    match member(fields, key)? {
        Value::Str(s) => Some(s.as_ref()),
        _ => None,
    }
}

impl ResourceContexts {
    /// Parse the entire ownership installation before replacing any state. Empty
    /// contexts explicitly clear ownership while retaining the configured table.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let value = json::parse(bytes).ok()?;
        let fields = object(&value, &["format", "contexts"])?;
        if text(fields, "format")? != FORMAT {
            return None;
        }
        let Value::Array(rows) = member(fields, "contexts")? else { return None };
        let mut result = Self::default();
        let mut seen = Vec::new();
        for row in rows {
            let fields = object(row, &["terminology", "resources"])?;
            let table = Expansions::parse(text(fields, "terminology")?).ok()?;
            let Value::Array(resources) = member(fields, "resources")? else { return None };
            if resources.is_empty() {
                return None;
            }
            let mut keys = Vec::new();
            for resource in resources {
                let fields = object(resource, &["resourceType", "id"])?;
                let rt = text(fields, "resourceType")?;
                let id = text(fields, "id")?;
                if crate::typemap::RESOURCES.binary_search(&rt).is_err()
                    || id.is_empty()
                    || id.len() > 64
                    || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')
                {
                    return None;
                }
                let key = (rt.to_string(), id.to_string());
                if seen.contains(&key) {
                    return None;
                }
                seen.push(key.clone());
                keys.push(key);
            }
            result.contexts.push(Context { table, resources: keys });
        }
        Some(result)
    }
}
impl TerminologyProvider for ResourceContexts {
    fn validate_code(&self, _: &str, _: &str, _: &str) -> Membership {
        Membership::Unknown
    }
    fn resource_context(&self, rt: &str, id: &str) -> Option<&Expansions> {
        self.contexts.iter().find(|c| c.resources.iter().any(|(t, i)| t == rt && i == id)).map(|c| &c.table)
    }
}

/// A stack-borrowed provider for one resource. Bound resources consult bundled
/// standards and their own complete table only; unrelated resources retain the
/// root provider. Discovery observes unknowns without borrowing its authority.
pub(crate) struct ResourceProvider<'a> {
    root: &'a dyn TerminologyProvider,
    table: Option<&'a Expansions>,
}
impl<'a> ResourceProvider<'a> {
    pub(crate) fn new(tx: &'a dyn TerminologyProvider, rt: &str, id: Option<&str>) -> Self {
        let root = tx.context_root().unwrap_or(tx);
        Self { root, table: id.and_then(|id| root.resource_context(rt, id)) }
    }
    pub(crate) fn for_document(tx: &'a dyn TerminologyProvider, doc: &impl Document) -> Option<Self> {
        let types = doc.scalars("$this.resourceType");
        let rt = *types.first()?;
        if crate::typemap::RESOURCES.binary_search(&rt).is_err() {
            return None;
        }
        Some(Self::new(tx, rt, doc.scalars("$this.id").first().copied()))
    }
}
impl TerminologyProvider for ResourceProvider<'_> {
    fn validate_code(&self, vs: &str, system: &str, code: &str) -> Membership {
        let Some(table) = self.table else { return self.root.validate_code(vs, system, code) };
        let verdict = Chain(&[&Bundled, table]).validate_code(vs, system, code);
        if verdict == Membership::Unknown {
            self.root.record_need(vs, system, code);
        }
        verdict
    }
    fn knows_set(&self, vs: &str) -> bool {
        self.table.map_or_else(|| self.root.knows_set(vs), |table| Bundled.knows_set(vs) || table.knows_set(vs))
    }
    fn validate_code_only(&self, vs: &str, code: &str) -> Membership {
        let Some(table) = self.table else { return self.root.validate_code_only(vs, code) };
        let verdict = Chain(&[&Bundled, table]).validate_code_only(vs, code);
        if verdict == Membership::Unknown {
            self.root.record_need(vs, "", code);
        }
        verdict
    }
    fn display(&self, system: &str, code: &str) -> Option<String> {
        self.table
            .map_or_else(|| self.root.display(system, code), |table| Chain(&[&Bundled, table]).display(system, code))
    }
    fn record_need(&self, vs: &str, system: &str, code: &str) {
        self.root.record_need(vs, system, code)
    }
    fn resource_context(&self, rt: &str, id: &str) -> Option<&Expansions> {
        self.root.resource_context(rt, id)
    }
    fn context_root(&self) -> Option<&dyn TerminologyProvider> {
        Some(self.root)
    }
}
