//! The document proof. Every document example in the corpus (IPS and AU
//! Patient Summary bundles) is taken apart — its entries minus the
//! Composition, the Composition's author, date, title and custodian — and
//! assembled again, written as `doc-<name>.json` with a manifest carrying
//! our verdict on the result, so the conformance harness can put every
//! assembled document in front of the reference validator.
//!
//!     cargo run -p fhir-pods-core --example document_corpus -- conformance/corpus
use std::{env, fs, process};

use fhir_pods_core::document::{Document, Parsed};
use fhir_pods_core::json::Value;
use fhir_pods_core::node::Node;
use fhir_pods_core::{wire, Validator};

fn main() {
    let dir = env::args().nth(1).unwrap_or_else(|| "conformance/corpus".into());
    let validator = Validator::new();
    let (mut n, mut valid) = (0, 0);
    let mut manifest_lines = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&dir).expect("corpus dir").flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".json")
            || name.starts_with('_')
            || name.ends_with("manifest.json")
            || name.starts_with("lowered-")
            || name.starts_with("deid-")
            || name.starts_with("doc-")
        {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let Ok(parsed) = Parsed::parse(&bytes) else { continue };
        let doc = parsed.doc();
        if doc.scalars("$this.resourceType") != ["Bundle"] || doc.scalars("$this.type") != ["document"] {
            continue;
        }
        let declared = doc.scalars("$this.meta.profile");
        let kind = if declared.iter().any(|u| u.starts_with("http://hl7.org.au/fhir/ps/")) {
            "au-ps"
        } else if declared.iter().any(|u| u.starts_with("http://hl7.org/fhir/uv/ips/")) {
            "ips"
        } else {
            continue;
        };
        // take the document apart
        let Value::Object(top) = parsed.value() else { continue };
        let member = |kvs: &[(std::borrow::Cow<'_, str>, Value<'_>)], k: &str| {
            kvs.iter().find(|(m, _)| m.as_ref() == k).map(|(_, v)| Node::from_value(v))
        };
        let Some(Node::Array(entry_nodes)) = member(top, "entry") else { continue };
        let mut resources: Vec<Node> = Vec::new();
        let mut composition: Option<Node> = None;
        for e in &entry_nodes {
            if let Some(r) = e.member("resource") {
                if r.member("resourceType").and_then(Node::as_str) == Some("Composition") && composition.is_none() {
                    composition = Some(r.clone());
                } else {
                    // as the host would hand them over: the entry, fullUrl and all
                    resources.push(Node::Object(vec![
                        ("fullUrl".into(), e.member("fullUrl").cloned().unwrap_or(Node::Null)),
                        ("resource".into(), r.clone()),
                    ]));
                }
            }
        }
        let Some(comp) = composition else { continue };
        let text = |n: &Node, k: &str| n.member(k).and_then(Node::as_str).map(String::from);
        let reference =
            |n: Option<&Node>| n.and_then(|r| r.member("reference")).and_then(Node::as_str).map(String::from);
        let authors: Vec<Node> = match comp.member("author") {
            Some(Node::Array(xs)) => xs.iter().filter_map(|x| reference(Some(x))).map(Node::Str).collect(),
            _ => Vec::new(),
        };
        let Some(date) = text(&comp, "date") else { continue };
        if authors.is_empty() {
            continue;
        }
        let mut inputs = Node::Object(vec![
            ("resources".into(), Node::Array(resources)),
            ("author".into(), Node::Array(authors)),
            ("date".into(), Node::Str(date)),
        ]);
        if let Some(t) = text(&comp, "title") {
            inputs.member_or_insert("title", Node::Str(t));
        }
        if let Some(c) = reference(comp.member("custodian")) {
            inputs.member_or_insert("custodian", Node::Str(c));
        }
        if let Some(ts) = member(top, "timestamp").and_then(|t| t.as_str().map(String::from)) {
            inputs.member_or_insert("timestamp", Node::Str(ts));
        }
        let assembled = match validator.document(kind, inputs.to_json().as_bytes()) {
            Ok(a) => a,
            Err(e) => {
                println!("{:<60} could not assemble: {e}", format!("doc-{name}"));
                process::exit(1);
            }
        };
        n += 1;
        let ok = assembled.validation.is_valid();
        if ok {
            valid += 1;
        }
        fs::write(path.with_file_name(format!("doc-{name}")), format!("{}\n", assembled.document)).unwrap();
        manifest_lines.push(format!(
            "{{\"file\": \"doc-{name}\", \"profile\": \"{kind}\", \"expect\": {ok}, \"ig\": \"document\"}}"
        ));
        let status = if ok { "conformant".to_string() } else { wire::report(&assembled.validation) };
        println!("{:<60} {status} (omitted {})", format!("doc-{name}"), assembled.omitted.len());
    }
    fs::write(format!("{dir}/doc-manifest.json"), format!("[\n{}\n]\n", manifest_lines.join(",\n"))).unwrap();
    println!("assembled {n} documents; {valid} conformant");
}
