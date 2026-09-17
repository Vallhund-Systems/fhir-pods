//! The de-identification proof. Every publisher-valid corpus example is
//! de-identified under Safe Harbor and written as `deid-<name>.json` with a
//! manifest carrying *our* verdict on the copy, so the conformance harness
//! can put every copy in front of the reference validator: the checker must
//! find nothing, and the copy's conformance must be what we say it is.
//!
//!     cargo run -p fhir-pods-core --example deid_corpus -- conformance/corpus
use std::{env, fs, process};

use fhir_pods_core::deidentify::Policy;
use fhir_pods_core::document::{Document, Parsed};
use fhir_pods_core::{generated, Validator};

fn main() {
    let dir = env::args().nth(1).unwrap_or_else(|| "conformance/corpus".into());
    let validator = Validator::new();
    let policy = Policy::safe_harbor(b"corpus-salt").today(2026);
    let (mut n, mut clean, mut valid) = (0, 0, 0);
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
            || name.starts_with("audit-")
        {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let Ok(parsed) = Parsed::parse(&bytes) else { continue };
        let doc = parsed.doc();
        let declared = doc.scalars("$this.meta.profile");
        let Some(profile) =
            generated::REGISTRY.iter().find(|p| declared.iter().any(|u| u.split('|').next() == Some(p.url)))
        else {
            continue;
        };
        if !validator.validate(&bytes).is_valid() {
            continue;
        }
        let d = validator.deidentify(&bytes, &policy).expect("a registry example is a known resource");
        n += 1;
        let remaining = d.remaining.len();
        if remaining == 0 {
            clean += 1;
        }
        let ok = d.validation.is_valid();
        if ok {
            valid += 1;
        }
        fs::write(path.with_file_name(format!("deid-{name}")), format!("{}\n", d.resource)).unwrap();
        manifest_lines.push(format!(
            "{{\"file\": \"deid-{name}\", \"profile\": \"{}\", \"expect\": {ok}, \"ig\": \"deid\"}}",
            profile.url
        ));
        let status = if remaining == 0 {
            format!("clean, {}", if ok { "conformant" } else { "not conformant" })
        } else {
            format!("{remaining} remaining: {:?}", d.remaining)
        };
        println!("{:<60} {status}", format!("deid-{name}"));
    }
    fs::write(format!("{dir}/deid-manifest.json"), format!("[\n{}\n]\n", manifest_lines.join(",\n"))).unwrap();
    println!("de-identified {n} examples; {clean} clean for the checker; {valid} still conformant");
    if clean != n {
        process::exit(1);
    }
}
