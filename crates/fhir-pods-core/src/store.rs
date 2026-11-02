//! The host's resource store: resources a host loaded
//! ([`crate::Validator::load_resource`]) for `resolve()` and the
//! reference-target check to find, beside the contained resources and the
//! document Bundle — never fetched. The host does any retrieval with its own
//! authorisation and consent and hands the runtime the bytes.
//!
//! Stored resources are held as JSON and searched as the entries of one
//! Bundle, by the rules a document Bundle's entries are searched by
//! ([`crate::path::resolve_reference`]): the entry's `fullUrl`; a relative
//! `Type/id` against a `fullUrl` ending so or the resource's own type and id;
//! a `/_history/version` suffix selecting the matching `meta.versionId`. The
//! first match in load order wins, as the first entry of a Bundle does.

use alloc::string::String;
use alloc::vec::Vec;

use crate::json::{self, Value};

/// One stored resource and the identities it is found by.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stored {
    /// The `fullUrl` it was loaded under (a Bundle entry's), if any.
    full_url: Option<String>,
    resource_type: String,
    id: Option<String>,
    version: Option<String>,
    /// The resource, re-serialised.
    json: String,
}

impl Stored {
    /// A second load of this identity replaces this entry: the same
    /// `fullUrl`, or the same `Type/id` with the same `meta.versionId` (both
    /// absent counts as the same) — two versions of one resource are two
    /// entries, so `/_history/version` can select either.
    fn same_identity(&self, other: &Stored) -> bool {
        if self.full_url.is_some() && self.full_url == other.full_url {
            return true;
        }
        self.id.is_some()
            && self.resource_type == other.resource_type
            && self.id == other.id
            && self.version == other.version
    }
}

/// The resources a host loaded, and the Bundle they are searched as.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResourceStore {
    entries: Vec<Stored>,
    /// `{"resourceType":"Bundle","type":"collection","entry":[…]}` of every
    /// entry, rebuilt on each load; empty when the store is.
    bundle: String,
}

impl ResourceStore {
    /// Load one resource, or every entry resource of a Bundle (each under
    /// its `fullUrl`). `false` — and the store unchanged — when the bytes are
    /// not JSON, not an object with a FHIR R4 `resourceType`, or a Bundle
    /// whose entry `resource` is not one. An entry with no `resource`, and a
    /// resource with neither an `id` nor a `fullUrl` (nothing could name
    /// it), is not stored.
    pub(crate) fn load(&mut self, bytes: &[u8]) -> bool {
        let Ok(value) = json::parse(bytes) else { return false };
        let Some(rt) = resource_type(&value) else { return false };
        let mut incoming = Vec::new();
        if rt == "Bundle" {
            for entry in member(&value, "entry").map_or(&[][..], array) {
                let Some(resource) = member(entry, "resource") else { continue };
                let Some(rt) = resource_type(resource) else { return false };
                let full_url = member(entry, "fullUrl").and_then(string);
                incoming.push(stored(rt, resource, full_url));
            }
        } else {
            incoming.push(stored(rt, &value, None));
        }
        for next in incoming {
            if next.full_url.is_none() && next.id.is_none() {
                continue;
            }
            match self.entries.iter_mut().find(|k| k.same_identity(&next)) {
                Some(slot) => *slot = next,
                None => self.entries.push(next),
            }
        }
        self.rebuild();
        true
    }

    /// Forget every stored resource.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.bundle.clear();
    }

    /// How many resources are stored.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The stored resources as one Bundle's JSON, `None` when there are none.
    pub(crate) fn bundle(&self) -> Option<&str> {
        (!self.bundle.is_empty()).then_some(self.bundle.as_str())
    }

    fn rebuild(&mut self) {
        self.bundle.clear();
        if self.entries.is_empty() {
            return;
        }
        let mut out = String::from(r#"{"resourceType":"Bundle","type":"collection","entry":["#);
        for (i, e) in self.entries.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('{');
            if let Some(full) = &e.full_url {
                out.push_str("\"fullUrl\":");
                write_str(full, &mut out);
                out.push(',');
            }
            out.push_str("\"resource\":");
            out.push_str(&e.json);
            out.push('}');
        }
        out.push_str("]}");
        self.bundle = out;
    }
}

fn stored(resource_type: &str, resource: &Value<'_>, full_url: Option<&str>) -> Stored {
    let scalar = |v: Option<&Value<'_>>| v.and_then(string).map(String::from);
    let mut json = String::new();
    write(resource, &mut json);
    Stored {
        full_url: full_url.filter(|u| !u.is_empty()).map(String::from),
        resource_type: String::from(resource_type),
        id: scalar(member(resource, "id")).filter(|id| !id.is_empty()),
        version: scalar(member(resource, "meta").and_then(|m| member(m, "versionId"))),
        json,
    }
}

/// The `resourceType` of an object, when it names an R4 resource type.
fn resource_type<'v>(v: &'v Value<'_>) -> Option<&'v str> {
    let rt = member(v, "resourceType").and_then(string)?;
    crate::typing::known_type(rt).filter(|t| crate::typemap::RESOURCES.binary_search(t).is_ok())?;
    Some(rt)
}

fn member<'v, 'a>(v: &'v Value<'a>, key: &str) -> Option<&'v Value<'a>> {
    match v {
        Value::Object(kvs) => kvs.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v),
        _ => None,
    }
}

fn array<'v, 'a>(v: &'v Value<'a>) -> &'v [Value<'a>] {
    match v {
        Value::Array(xs) => xs.as_slice(),
        _ => &[],
    }
}

fn string<'v>(v: &'v Value<'_>) -> Option<&'v str> {
    match v {
        Value::Str(s) => Some(s.as_ref()),
        _ => None,
    }
}

/// JSON text for a parsed value: numbers as their literal text, strings
/// re-escaped, members in their order.
fn write(v: &Value<'_>, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(n),
        Value::Str(s) => write_str(s, out),
        Value::Array(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(x, out);
            }
            out.push(']');
        }
        Value::Object(kvs) => {
            out.push('{');
            for (i, (k, x)) in kvs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_str(k, out);
                out.push(':');
                write(x, out);
            }
            out.push('}');
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resource_and_a_bundle_load_and_replace_by_identity() {
        let mut s = ResourceStore::default();
        assert!(s.load(br#"{"resourceType":"Patient","id":"p1","name":[{"text":"A \"quoted\" name"}]}"#));
        assert!(s.load(br#"{"resourceType":"Patient","id":"p1","gender":"other"}"#));
        assert_eq!(s.len(), 1, "same Type/id, no version: replaced");
        assert!(s.load(br#"{"resourceType":"Patient","id":"p1","meta":{"versionId":"2"}}"#));
        assert_eq!(s.len(), 2, "another version is another entry");
        assert!(s.load(
            br#"{"resourceType":"Bundle","type":"collection","entry":[
                {"fullUrl":"urn:uuid:0b6e3f2a-1111-4c2d-8e9f-000000000001","resource":{"resourceType":"Practitioner"}},
                {"request":{"method":"DELETE","url":"Patient/x"}},
                {"fullUrl":"https://records.example.invalid/fhir/Organization/o1","resource":{"resourceType":"Organization","id":"o1"}}]}"#
        ));
        assert_eq!(s.len(), 4);
        assert!(s.load(
            br#"{"resourceType":"Bundle","entry":[{"fullUrl":"urn:uuid:0b6e3f2a-1111-4c2d-8e9f-000000000001","resource":{"resourceType":"Device"}}]}"#
        ));
        assert_eq!(s.len(), 4, "the same fullUrl: replaced");
        let bundle = s.bundle().expect("held");
        assert!(json::parse(bundle.as_bytes()).is_ok(), "{bundle}");
        assert!(bundle.contains(r#""gender":"other""#) && !bundle.contains("quoted"), "{bundle}");
        assert!(bundle.contains(r#""resourceType":"Device""#) && !bundle.contains("Practitioner"), "{bundle}");
    }

    #[test]
    fn what_is_not_a_resource_leaves_the_store_unchanged() {
        let mut s = ResourceStore::default();
        assert!(s.load(br#"{"resourceType":"Patient","id":"p1"}"#));
        for bad in [
            &b"not json"[..],
            br#"[]"#,
            br#"{"id":"p1"}"#,
            br#"{"resourceType":"Spaceship","id":"s1"}"#,
            br#"{"resourceType":"Bundle","entry":[{"resource":{"resourceType":"Nope"}}]}"#,
        ] {
            assert!(!s.load(bad));
            assert_eq!(s.len(), 1);
        }
        assert!(s.load(br#"{"resourceType":"Patient"}"#), "a resource nothing can name is taken and not stored");
        assert_eq!(s.len(), 1);
        s.clear();
        assert!(s.bundle().is_none());
    }
}
