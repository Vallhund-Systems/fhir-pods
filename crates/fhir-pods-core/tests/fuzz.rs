//! Seeded, zero-dependency fuzzing that runs under `cargo test`.
//!
//! The core has two hand-written parsers fed untrusted bytes and runs
//! `no_std` on device under `panic = "abort"` — so "never panics, never hangs,
//! on any input" is a property to hold, not hope for. This harness holds it
//! deterministically: same `FUZZ_SEED`, same inputs, so a failure prints the
//! seed and iteration to replay.
//!
//!     FUZZ_ITERS=1000000 FUZZ_SEED=42 cargo test -p fhir-pods-core --test fuzz
//!
//! For coverage-guided runs, see `fuzz/` (libFuzzer, needs nightly).
use std::panic::{catch_unwind, AssertUnwindSafe};

use fhir_pods_core::{document, json, path, validate_bytes, wire};

/// xorshift64* — small, fast, deterministic; not for anything but fuzzing.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

// ---- generators ------------------------------------------------------------

fn byte_soup(rng: &mut Rng) -> Vec<u8> {
    let n = rng.below(256);
    (0..n).map(|_| rng.next() as u8).collect()
}

/// A few random edits to a valid document: flips, structural inserts,
/// deletions, truncation.
fn mutate(rng: &mut Rng, base: &str) -> Vec<u8> {
    let mut b = base.as_bytes().to_vec();
    for _ in 0..(1 + rng.below(8)) {
        if b.is_empty() {
            break;
        }
        let i = rng.below(b.len());
        match rng.below(4) {
            0 => b[i] = rng.next() as u8,
            1 => b.insert(i, *rng.pick(b"{}[]\":,\\ _0aZ")),
            2 => {
                b.remove(i);
            }
            _ => b.truncate(i),
        }
    }
    b
}

const KEYS: &[&str] = &[
    "status", "code", "coding", "system", "value", "valueQuantity", "valueString", "component", "identifier",
    "type", "name", "given", "family", "use", "gender", "birthDate", "_birthDate", "extension", "url",
    "subject", "reference", "category", "text", "deceasedBoolean", "deceasedDateTime", "dataAbsentReason",
    "communication", "language", "link", "other", "unit", "display", "effectiveDateTime", "hasMember",
    "meta", "profile", "id", "active", "resourceType",
];
const STRS: &[&str] = &[
    "final", "preliminary", "banana", "male", "female", "official", "bogus", "http://loinc.org",
    "http://snomed.info/sct", "8867-4", "364075005", "85354-9", "8480-6", "8462-4", "vital-signs",
    "http://terminology.hl7.org/CodeSystem/observation-category", "NI", "MC",
    "http://terminology.hl7.org/CodeSystem/v2-0203", "http://ns.electronichealth.net.au/id/hi/ihi/1.0",
    "8003608833357361", "/min", "mm[Hg]", "kg", "http://unitsofmeasure.org", "Patient/1", "Device/1",
    "1970-01-01", "2026-09-15T10:00:00Z", "", "a } \" tricky", "\u{1F600}", "\\", "\\u00",
];
const TYPES: &[&str] = &[
    "Patient", "Observation", "Encounter", "Condition", "AllergyIntolerance", "MedicationRequest", "Bundle", "Nope",
];
const PROFILES: &[&str] = &[
    "http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient",
    "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate",
    "http://hl7.org.au/fhir/core/StructureDefinition/au-core-bloodpressure",
    "http://hl7.org.au/fhir/core/StructureDefinition/au-core-condition",
    "http://hl7.org.au/fhir/core/StructureDefinition/au-core-medicationrequest",
    "http://hl7.org.au/fhir/core/StructureDefinition/au-core-encounter",
];

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn value(rng: &mut Rng, out: &mut String, depth: usize) {
    let arms = if depth == 0 { 5 } else { 7 };
    match rng.below(arms) {
        0 => json_str(out, *rng.pick(STRS)),
        1 => out.push_str(&format!("{}", rng.next() % 1000)),
        2 => out.push_str(if rng.chance(50) { "true" } else { "false" }),
        3 => out.push_str("null"),
        4 => out.push_str(&format!("{}.{}", rng.next() % 200, rng.next() % 10)),
        5 => object(rng, out, depth - 1),
        _ => {
            out.push('[');
            let n = rng.below(4);
            for i in 0..n {
                if i > 0 {
                    out.push(',');
                }
                value(rng, out, depth - 1);
            }
            out.push(']');
        }
    }
}

fn object(rng: &mut Rng, out: &mut String, depth: usize) {
    out.push('{');
    let n = rng.below(6);
    for i in 0..n {
        if i > 0 {
            out.push(',');
        }
        json_str(out, *rng.pick(KEYS));
        out.push(':');
        value(rng, out, depth);
    }
    out.push('}');
}

/// A FHIR-shaped resource: a real resource type, often a real declared
/// profile (so slicing, inheritance and invariants all run), random members
/// from a FHIR-ish vocabulary, nested a few levels.
fn resource(rng: &mut Rng) -> String {
    let mut out = String::from("{\"resourceType\":");
    json_str(&mut out, *rng.pick(TYPES));
    if rng.chance(60) {
        out.push_str(",\"meta\":{\"profile\":[");
        json_str(&mut out, *rng.pick(PROFILES));
        out.push_str("]}");
    }
    for _ in 0..(1 + rng.below(8)) {
        out.push(',');
        json_str(&mut out, *rng.pick(KEYS));
        out.push(':');
        value(rng, &mut out, 3);
    }
    out.push('}');
    out
}

const EXPR_TOKS: &[&str] = &[
    "value", "status", "code", "coding", "system", "component", "dataAbsentReason", "exists()", "empty()",
    "not()", "count()", "first()", "hasValue()", "where(", "exists(", "and", "or", "xor", "implies", "=",
    "!=", "<", "<=", ">", "(", ")", ".", ",", "'final'", "'x'", "''", "0", "1", "42", "99999999999999999999",
    "$this", "true", "false", "%resource", "matches(", "substring(", "toInteger()", "'unterminated", " ",
];

fn expression(rng: &mut Rng) -> String {
    let n = 1 + rng.below(12);
    let sep = if rng.chance(50) { "" } else { " " };
    (0..n).map(|_| *rng.pick(EXPR_TOKS)).collect::<Vec<_>>().join(sep)
}

// ---- properties ------------------------------------------------------------

fn check_validate(input: &[u8], seed: u64, iter: u64, kind: &str) {
    let r = catch_unwind(AssertUnwindSafe(|| {
        let v = validate_bytes(input);
        let outcome = v.to_operation_outcome();
        // deterministic
        assert_eq!(outcome, validate_bytes(input).to_operation_outcome(), "non-deterministic");
        // both wire forms are well-formed JSON (round-trip through our own parser)
        json::parse(outcome.as_bytes()).expect("OperationOutcome must be well-formed JSON");
        json::parse(wire::report(&v).as_bytes()).expect("report must be well-formed JSON");
    }));
    if r.is_err() {
        panic!(
            "fuzz failure: kind={kind} seed={seed} iter={iter}\ninput={:?}",
            String::from_utf8_lossy(input)
        );
    }
}

fn check_path(expr: &str, doc: &str, seed: u64, iter: u64) {
    let r = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(e) = path::parse(expr) {
            if let Ok(parsed) = document::Parsed::parse(doc.as_bytes()) {
                let _ = path::check(&e, &parsed.doc());
            }
        }
    }));
    if r.is_err() {
        panic!("fuzz failure: kind=fhirpath seed={seed} iter={iter}\nexpr={expr:?}\ndoc={doc:?}");
    }
}

#[test]
fn seeded_fuzz_never_panics() {
    let seed = env_u64("FUZZ_SEED", 20260915);
    let iters = env_u64("FUZZ_ITERS", 50_000);
    let mut rng = Rng::new(seed);
    for iter in 0..iters {
        match iter % 4 {
            0 => check_validate(&byte_soup(&mut rng), seed, iter, "bytes"),
            1 => {
                let base = resource(&mut rng);
                check_validate(&mutate(&mut rng, &base), seed, iter, "mutated");
            }
            2 => {
                let doc = resource(&mut rng);
                check_validate(doc.as_bytes(), seed, iter, "structured");
            }
            _ => {
                let expr = expression(&mut rng);
                let doc = resource(&mut rng);
                check_path(&expr, &doc, seed, iter);
            }
        }
    }
}

#[test]
fn hostile_nesting_is_an_error_not_a_crash() {
    let deep = "[".repeat(100_000);
    assert!(json::parse(deep.as_bytes()).is_err());
    assert!(json::scan_top_level(format!("{{\"a\":{deep}").as_bytes()).is_err());
    assert!(path::parse(&"(".repeat(100_000)).is_err());
    assert!(path::parse(&"where(".repeat(100_000)).is_err());
    // …and through the public entrypoint: a fatal parse issue, not an abort.
    let v = validate_bytes(format!("{{\"resourceType\":\"Patient\",\"x\":{deep}").as_bytes());
    assert!(!v.is_valid());
    assert!(v.to_operation_outcome().contains("nesting too deep"));
}
