//! The HL7 v2 sample-message proof. Every `.hl7` / `.txt` message in a
//! directory (a pinned public sample set — synthetic, never real data) is
//! converted; each bundle is written as `v2s-<name>.json` beside a manifest
//! (`v2s-manifest.json`) carrying our own verdict, what the converter said
//! it could not place or map, and the time it took — so the conformance lane
//! (`conformance/v2samples.py`) can put every bundle in front of the
//! reference validator and compare, message by message, what the two say.
//! A message the pod has no map for is recorded as refused, with the reason,
//! never converted by guesswork.
//!
//!     cargo run --release -p fhir-pods-core --example v2_samples -- <messages-dir> <out-dir>
use std::time::Instant;
use std::{env, fs, process};

use fhir_pods_core::v2::convert::{ConvertError, Options};
use fhir_pods_core::{wire, Validator};

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
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
    out
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let (Some(dir), Some(out)) = (args.first(), args.get(1)) else {
        eprintln!("usage: v2_samples <messages-dir> <out-dir>");
        process::exit(2);
    };
    fs::create_dir_all(out).unwrap();
    let validator = Validator::new();
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".hl7") || n.ends_with(".txt"))
        .collect();
    names.sort();
    let mut rows = Vec::new();
    let (mut converted, mut refused) = (0, 0);
    for name in &names {
        let bytes = fs::read(format!("{dir}/{name}")).unwrap();
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).replace(['.', ' '], "_");
        let started = Instant::now();
        let result = validator.v2(&bytes, &Options::default());
        let micros = started.elapsed().as_micros();
        match result {
            Ok(c) => {
                converted += 1;
                let file = format!("v2s-{stem}.json");
                fs::write(format!("{out}/{file}"), format!("{}\n", c.bundle)).unwrap();
                let skipped: Vec<String> =
                    c.skipped.iter().map(|s| json_str(&format!("{} -> {}: {}", s.at, s.target, s.why))).collect();
                let unplaced: Vec<String> = c.unplaced.iter().map(|u| json_str(u)).collect();
                let unmapped: Vec<String> = c.unmapped.iter().map(|u| json_str(u)).collect();
                let errors: Vec<String> = c
                    .validation
                    .report
                    .issues
                    .iter()
                    .filter(|i| matches!(i.severity, fhir_pods_core::validate::Severity::Error))
                    .map(|i| json_str(&format!("{}: {}", i.path, i.message)))
                    .collect();
                rows.push(format!(
                    "{{\"message\": {}, \"file\": {}, \"structure\": {}, \"converted\": true, \"expect\": {}, \"micros\": {micros}, \"skipped\": [{}], \"unplaced\": [{}], \"unmapped\": [{}], \"errors\": [{}]}}",
                    json_str(name),
                    json_str(&file),
                    json_str(&structure_of(&bytes)),
                    c.validation.is_valid(),
                    skipped.join(", "),
                    unplaced.join(", "),
                    unmapped.join(", "),
                    errors.join(", ")
                ));
                println!(
                    "{:<44} {} ({} skipped, {} unplaced, {} unmapped) {}",
                    name,
                    if c.validation.is_valid() { "valid  " } else { "invalid" },
                    c.skipped.len(),
                    c.unplaced.len(),
                    c.unmapped.len(),
                    if c.validation.is_valid() {
                        String::new()
                    } else {
                        wire::report(&c.validation).chars().take(160).collect()
                    }
                );
            }
            Err(e) => {
                refused += 1;
                let why = match &e {
                    ConvertError::UnknownStructure(s) if s.is_empty() => "MSH-9 names no message structure".to_string(),
                    ConvertError::UnknownStructure(s) => format!("no map for message structure '{s}'"),
                    other => format!("{other}"),
                };
                rows.push(format!(
                    "{{\"message\": {}, \"structure\": {}, \"converted\": false, \"refused\": {}, \"micros\": {micros}}}",
                    json_str(name),
                    json_str(&structure_of(&bytes)),
                    json_str(&why)
                ));
                println!("{:<44} refused: {why}", name);
            }
        }
    }
    fs::write(format!("{out}/v2s-manifest.json"), format!("[\n{}\n]\n", rows.join(",\n"))).unwrap();
    println!("{} messages: {converted} converted, {refused} refused", names.len());
}

/// MSH-9's message structure (or `type^event`) as the message states it.
fn structure_of(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let msh = text.split(['\r', '\n']).find(|l| l.starts_with("MSH")).unwrap_or("");
    let sep = msh.chars().nth(3).unwrap_or('|');
    let field = msh.split(sep).nth(8).unwrap_or("");
    field.replace('^', "_").split('_').collect::<Vec<_>>().join("_")
}
