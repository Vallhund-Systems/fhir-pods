//! The host's resource store: resources a host loaded
//! ([`crate::Validator::load_resource`]) for `resolve()` and the
//! reference-target checks to find, after the contained resources and the
//! document Bundle — never fetched. The host does any retrieval with its own
//! authorisation and consent and hands the runtime the bytes.
//!
//! A stored resource is found by the rules a Bundle entry is
//! ([`crate::path::resolve_reference`]): its `fullUrl`; a relative `Type/id`
//! against a `fullUrl` ending so or the resource's own type and id; a
//! `/_history/version` suffix selecting the matching `meta.versionId`; the
//! first match in load order, as the first entry of a Bundle wins.
//!
//! Each resource's JSON is held once. Loading is indexed (`fullUrl`, and the
//! id a reference could name it by), so a load costs its own size, not the
//! store's. A validation parses only the stored resources a string in the
//! input could name — and, transitively, those a string in one of them could
//! name — and none at all when no string in the input names one.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::json::{self, Value};

/// One stored resource and the identities it is found by.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Stored {
    /// The `fullUrl` it was loaded under (a Bundle entry's), if any.
    pub(crate) full_url: Option<String>,
    pub(crate) resource_type: String,
    pub(crate) id: Option<String>,
    pub(crate) version: Option<String>,
    /// The resource, re-serialised: the one copy of its content.
    pub(crate) json: String,
}

impl Stored {
    /// What a reference could name it by, beside its `fullUrl`: its own id
    /// and the last segment of its `fullUrl` (a relative `Type/id` matches
    /// either, so every candidate is under one of them).
    fn ids(&self) -> impl Iterator<Item = &str> {
        let tail = self.full_url.as_deref().and_then(|f| f.rsplit_once('/')).map(|(_, t)| t);
        self.id.as_deref().into_iter().chain(tail.filter(|t| Some(*t) != self.id.as_deref()))
    }
}

/// The resources a host loaded. Cloning is cheap: entries are shared.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResourceStore {
    /// In load order; a replaced entry keeps its place.
    entries: Vec<Arc<Stored>>,
    /// `fullUrl` → slot: an entry with a `fullUrl` is identified by it alone.
    by_full: BTreeMap<String, usize>,
    /// (type, id, versionId) → slot, for entries loaded without a `fullUrl`.
    by_identity: BTreeMap<(String, String, String), usize>,
    /// id → slots, in load order: every entry a relative reference could name.
    by_id: BTreeMap<String, Vec<usize>>,
}

impl ResourceStore {
    /// Load one resource, or every entry resource of a Bundle (each under
    /// its `fullUrl`). `false` — and the store unchanged — when the bytes are
    /// not JSON, not an object with a FHIR R4 `resourceType`, or a Bundle
    /// whose `entry` is not an array of objects or whose entry `resource` is
    /// not a resource. An entry with no `resource`, and a resource with
    /// neither an `id` nor a `fullUrl` (nothing could name it), is not stored.
    pub(crate) fn load(&mut self, bytes: &[u8]) -> bool {
        let Ok(value) = json::parse(bytes) else { return false };
        let Some(rt) = resource_type(&value) else { return false };
        let mut incoming = Vec::new();
        if rt == "Bundle" {
            let entries = match member(&value, "entry") {
                None => &[][..],
                Some(Value::Array(xs)) => xs.as_slice(),
                Some(_) => return false,
            };
            for entry in entries {
                if !matches!(entry, Value::Object(_)) {
                    return false;
                }
                let Some(resource) = member(entry, "resource") else { continue };
                let Some(rt) = resource_type(resource) else { return false };
                let full_url = member(entry, "fullUrl").and_then(string);
                incoming.push(stored(rt, resource, full_url));
            }
        } else {
            incoming.push(stored(rt, &value, None));
        }
        for next in incoming {
            if next.full_url.is_some() || next.id.is_some() {
                self.put(next);
            }
        }
        true
    }

    /// Replace the entry `next` identifies, or append it. Identity is the
    /// `fullUrl` when the entry has one; an entry without one is identified
    /// by its type, id and `meta.versionId` (absent counts as one value), and
    /// replaces only another entry without a `fullUrl`.
    fn put(&mut self, next: Stored) {
        let existing = match &next.full_url {
            Some(full) => self.by_full.get(full).copied(),
            None => self.by_identity.get(&identity_key(&next)).copied(),
        };
        let slot = match existing {
            Some(slot) => {
                let old = self.entries[slot].clone();
                for id in old.ids() {
                    if let Some(slots) = self.by_id.get_mut(id) {
                        slots.retain(|s| *s != slot);
                    }
                }
                self.entries[slot] = Arc::new(next);
                slot
            }
            None => {
                self.entries.push(Arc::new(next));
                self.entries.len() - 1
            }
        };
        let entry = self.entries[slot].clone();
        match &entry.full_url {
            Some(full) => {
                self.by_full.insert(full.clone(), slot);
            }
            None => {
                self.by_identity.insert(identity_key(&entry), slot);
            }
        }
        for id in entry.ids() {
            let slots = self.by_id.entry(String::from(id)).or_default();
            if let Err(at) = slots.binary_search(&slot) {
                slots.insert(at, slot);
            }
        }
    }

    /// Forget every stored resource.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// How many resources are stored.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Slots whose entry `reference` could name (a superset of the matches,
    /// which the Bundle-entry rules then decide among): the entry under that
    /// `fullUrl`, and for a relative `Type/id` every entry with that id.
    fn candidates(&self, reference: &str, out: &mut BTreeSet<usize>) {
        if reference.starts_with('#') {
            return;
        }
        let identity = reference.rsplit_once("/_history/").map_or(reference, |(identity, _)| identity);
        if let Some(slot) = self.by_full.get(identity) {
            out.insert(*slot);
        }
        if let Some((ty, id)) = identity.split_once('/') {
            if !ty.is_empty() && !id.contains('/') {
                if let Some(slots) = self.by_id.get(id) {
                    out.extend(slots.iter().copied());
                }
            }
        }
    }

    /// The stored resources any string in `value` could name, followed
    /// through the strings of each one found, in load order.
    pub(crate) fn reachable(&self, value: &Value<'_>) -> Vec<Arc<Stored>> {
        let mut found = BTreeSet::new();
        if self.entries.is_empty() {
            return Vec::new();
        }
        each_string(value, &mut |s| self.candidates(s, &mut found));
        let mut queue: Vec<usize> = found.iter().copied().collect();
        while let Some(slot) = queue.pop() {
            let Ok(target) = json::parse(self.entries[slot].json.as_bytes()) else { continue };
            let mut more = BTreeSet::new();
            each_string(&target, &mut |s| self.candidates(s, &mut more));
            for s in more {
                if found.insert(s) {
                    queue.push(s);
                }
            }
        }
        found.into_iter().map(|slot| self.entries[slot].clone()).collect()
    }
}

fn identity_key(s: &Stored) -> (String, String, String) {
    (s.resource_type.clone(), s.id.clone().unwrap_or_default(), s.version.clone().unwrap_or_default())
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

/// Every string value in a tree (not the keys).
fn each_string<'v>(v: &'v Value<'_>, f: &mut dyn FnMut(&'v str)) {
    match v {
        Value::Str(s) => f(s.as_ref()),
        Value::Array(xs) => xs.iter().for_each(|x| each_string(x, f)),
        Value::Object(kvs) => kvs.iter().for_each(|(_, x)| each_string(x, f)),
        _ => {}
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

    fn reach(s: &ResourceStore, json: &str) -> Vec<String> {
        let v = json::parse(json.as_bytes()).unwrap();
        s.reachable(&v).iter().map(|e| e.json.clone()).collect()
    }

    #[test]
    fn a_resource_and_a_bundle_load_and_replace_by_identity() {
        let mut s = ResourceStore::default();
        assert!(s.load(br#"{"resourceType":"Patient","id":"p1","name":[{"text":"A \"quoted\" name"}]}"#));
        assert!(s.load(br#"{"resourceType":"Patient","id":"p1","gender":"other"}"#));
        assert_eq!(s.len(), 1, "same Type/id, no version, no fullUrl: replaced");
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
        let all = reach(&s, r#"["Patient/p1","urn:uuid:0b6e3f2a-1111-4c2d-8e9f-000000000001","Organization/o1"]"#);
        assert_eq!(all.len(), 4, "{all:?}");
        assert!(all.iter().any(|j| j.contains(r#""gender":"other""#)) && !all.iter().any(|j| j.contains("quoted")));
        assert!(all.iter().any(|j| j.contains("Device")) && !all.iter().any(|j| j.contains("Practitioner")));
    }

    /// Two entries under different `fullUrl`s are two entries, whatever
    /// their type and id; an entry without a `fullUrl` never replaces one
    /// with, nor the reverse.
    #[test]
    fn a_full_url_is_the_whole_identity() {
        let mut s = ResourceStore::default();
        assert!(s.load(
            br#"{"resourceType":"Bundle","entry":[
                {"fullUrl":"https://a.example.invalid/fhir/Patient/1","resource":{"resourceType":"Patient","id":"1"}},
                {"fullUrl":"https://b.example.invalid/fhir/Patient/1","resource":{"resourceType":"Patient","id":"1","gender":"other"}}]}"#
        ));
        assert_eq!(s.len(), 2);
        assert!(s.load(br#"{"resourceType":"Patient","id":"1"}"#));
        assert_eq!(s.len(), 3);
        assert_eq!(reach(&s, r#""https://b.example.invalid/fhir/Patient/1""#).len(), 1);
        assert_eq!(reach(&s, r#""Patient/1""#).len(), 3, "every candidate a relative reference could name");
    }

    #[test]
    fn reach_follows_stored_references_and_nothing_else() {
        let mut s = ResourceStore::default();
        assert!(
            s.load(br#"{"resourceType":"Patient","id":"p1","managingOrganization":{"reference":"Organization/o1"}}"#)
        );
        assert!(s.load(br#"{"resourceType":"Organization","id":"o1"}"#));
        assert!(s.load(br#"{"resourceType":"Organization","id":"o2"}"#));
        assert_eq!(reach(&s, r#"{"subject":{"reference":"Patient/p1"}}"#).len(), 2);
        assert!(reach(&s, r##"{"subject":{"reference":"#p1"},"text":"nothing here"}"##).is_empty());
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
            br#"{"resourceType":"Bundle","entry":"notarray"}"#,
            br#"{"resourceType":"Bundle","entry":[1]}"#,
        ] {
            assert!(!s.load(bad));
            assert_eq!(s.len(), 1);
        }
        assert!(s.load(br#"{"resourceType":"Patient"}"#), "a resource nothing can name is taken and not stored");
        assert_eq!(s.len(), 1);
        s.clear();
        assert!(s.is_empty());
    }
}
