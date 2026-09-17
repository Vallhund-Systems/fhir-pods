//! Native operation timings on fixed synthetic fixtures; --phases retains the
//! original publisher-corpus validation breakdown.
use fhir_pods_core::{
    document::{Document, Parsed},
    engine, generated, path,
    terminology::Bundled,
    validate_bytes, wire, Validator,
};
use std::time::Instant;

fn phases() {
    let hr = std::fs::read("conformance/corpus/example-Observation-heartrate-1.json").unwrap();
    let pt = std::fs::read("conformance/corpus/example-Patient-ronny-irvine.json").unwrap();
    let us = std::fs::read("conformance/corpus/us-Patient-example.json").unwrap();
    let t = Instant::now();
    let validator = Validator::new();
    println!("Validator::new(): {:.0} µs once", t.elapsed().as_secs_f64() * 1e6);
    for (name, bytes) in [("AU heart rate", &hr), ("AU patient", &pt), ("US patient", &us)] {
        let n = 5000;
        let t = Instant::now();
        for _ in 0..n {
            std::hint::black_box(validator.validate(bytes));
        }
        let cached = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let t = Instant::now();
        for _ in 0..n {
            std::hint::black_box(validate_bytes(bytes));
        }
        let total = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let t = Instant::now();
        for _ in 0..n {
            std::hint::black_box(Parsed::parse(bytes).unwrap());
        }
        let parse = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let parsed = Parsed::parse(bytes).unwrap();
        let doc = parsed.doc();
        let declared = doc.scalars("$this.meta.profile");
        let profile = generated::REGISTRY.iter().find(|p| declared.contains(&p.url)).unwrap();
        let t = Instant::now();
        for _ in 0..n {
            for inv in profile.invariants {
                std::hint::black_box(path::parse(inv.expression).ok());
            }
        }
        let inv_parse = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let t = Instant::now();
        for _ in 0..n {
            std::hint::black_box(engine::validate_with(profile, &doc, &Bundled));
        }
        let engine_all = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let v = validate_bytes(bytes);
        let t = Instant::now();
        for _ in 0..n {
            std::hint::black_box(wire::report(&v));
        }
        let wire_t = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        println!("{name:<14} {} bytes, {} invariants: Validator {cached:5.1} µs | free fn {total:5.1} = parse {parse:4.1} + engine {engine_all:5.1} (invariant parsing {inv_parse:4.1}) + wire {wire_t:3.1}",
            bytes.len(), profile.invariants.len());
    }
}

fn measure(name: &str, fixture: &str, iterations: usize, mut operation: impl FnMut()) -> fhir_pods_core::node::Node {
    use fhir_pods_core::node::Node;
    for _ in 0..100 {
        operation();
    }
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / iterations as f64);
    }
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    Node::Object(vec![
        ("operation".into(), Node::Str(name.into())),
        ("fixture".into(), Node::Str(fixture.into())),
        ("iterations_per_sample".into(), Node::Number(iterations.to_string())),
        ("warmup_iterations".into(), Node::Number("100".into())),
        ("samples_us".into(), Node::Array(samples.iter().map(|v| Node::Number(format!("{v:.3}"))).collect())),
        ("min_us".into(), Node::Number(format!("{:.3}", sorted[0]))),
        ("median_us".into(), Node::Number(format!("{:.3}", sorted[3]))),
        ("max_us".into(), Node::Number(format!("{:.3}", sorted[6]))),
    ])
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use fhir_pods_core::{deidentify::Policy, node::Node};
    use std::hint::black_box;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--phases"] {
        phases();
        return Ok(());
    }
    if args.len() > 1 {
        return Err("usage: bench [ITERATIONS | --phases]".into());
    }
    let iterations: usize = args.first().map(|s| s.parse()).transpose()?.unwrap_or(1000);
    if iterations == 0 {
        return Err("iterations must be positive".into());
    }
    let hr = include_bytes!("../../../bench/heart-rate.json");
    let inputs = include_bytes!("../../../bench/heart-rate-inputs.json");
    let patient = include_bytes!("../../../bench/patient.json");
    let document = include_bytes!("../../../bench/document-inputs.json");
    let profile = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";
    let validator = Validator::new();
    let policy = Policy::safe_harbor(b"synthetic-benchmark-salt").today(2026);
    // A fast failing/invalid path must never masquerade as a successful benchmark.
    let validation = validator.validate(hr);
    assert!(validation.is_valid(), "heart-rate fixture invalid: {}", wire::report(&validation));
    assert!(validator.lower(profile, inputs)?.validation.is_valid(), "lower fixture invalid");
    let deid = validator.deidentify(patient, &policy)?;
    assert!(deid.validation.is_valid() && deid.remaining.is_empty(), "deid fixture did not pass");
    assert!(validator.document("ips", document)?.validation.is_valid(), "document fixture invalid");
    let rows = vec![
        measure("validate", "heart-rate.json", iterations, || {
            black_box(validator.validate(black_box(hr)));
        }),
        measure("lower", "heart-rate-inputs.json", iterations, || {
            black_box(validator.lower(profile, black_box(inputs)).unwrap());
        }),
        measure("deidentify", "patient.json", iterations, || {
            black_box(validator.deidentify(black_box(patient), &policy).unwrap());
        }),
        measure("document", "document-inputs.json", iterations, || {
            black_box(validator.document("ips", black_box(document)).unwrap());
        }),
    ];
    println!(
        "{}",
        Node::Object(vec![
            ("schema".into(), Node::Number("1".into())),
            ("core_version".into(), Node::Str(env!("CARGO_PKG_VERSION").into())),
            ("validation_preflight".into(), Node::Bool(true)),
            ("measurements".into(), Node::Array(rows)),
        ])
        .to_json()
    );
    Ok(())
}
