//! Phase timings for one validation, release mode. Not a benchmark suite —
//! a profile to steer the hot-path audit.
use std::time::Instant;
use fhir_pods_core::{document::{Document, Parsed}, engine, generated, path, terminology::Bundled, validate_bytes, wire, Validator};

fn main() {
    let hr = std::fs::read("conformance/corpus/example-Observation-heartrate-1.json").unwrap();
    let pt = std::fs::read("conformance/corpus/example-Patient-ronny-irvine.json").unwrap();
    let us = std::fs::read("conformance/corpus/us-Patient-example.json").unwrap();
    let t = Instant::now();
    let validator = Validator::new();
    println!("Validator::new(): {:.0} µs once", t.elapsed().as_secs_f64() * 1e6);
    for (name, bytes) in [("AU heart rate", &hr), ("AU patient", &pt), ("US patient", &us)] {
        let n = 5000;
        let t = Instant::now();
        for _ in 0..n { std::hint::black_box(validator.validate(bytes)); }
        let cached = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let t = Instant::now();
        for _ in 0..n { std::hint::black_box(validate_bytes(bytes)); }
        let total = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let t = Instant::now();
        for _ in 0..n { std::hint::black_box(Parsed::parse(bytes).unwrap()); }
        let parse = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let parsed = Parsed::parse(bytes).unwrap();
        let doc = parsed.doc();
        let declared = doc.scalars("$this.meta.profile");
        let profile = generated::REGISTRY.iter().find(|p| declared.iter().any(|u| *u == p.url)).unwrap();
        let t = Instant::now();
        for _ in 0..n { for inv in profile.invariants { std::hint::black_box(path::parse(inv.expression).ok()); } }
        let inv_parse = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let t = Instant::now();
        for _ in 0..n { std::hint::black_box(engine::validate_with(profile, &doc, &Bundled)); }
        let engine_all = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let v = validate_bytes(bytes);
        let t = Instant::now();
        for _ in 0..n { std::hint::black_box(wire::report(&v)); }
        let wire_t = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        println!("{name:<14} {} bytes, {} invariants: Validator {cached:5.1} µs | free fn {total:5.1} = parse {parse:4.1} + engine {engine_all:5.1} (invariant parsing {inv_parse:4.1}) + wire {wire_t:3.1}",
            bytes.len(), profile.invariants.len());
    }
}
