//! Probe a FHIR resource file and print the JSON report.
//!
//!     cargo run -p fhir-pods-core --example probe -- path/to/resource.json
//!
//! Exits 1 when the resource fails validation, so shells and the conformance
//! harness can branch on the verdict.
use std::{env, fs, process};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: probe <resource.json>");
        process::exit(2);
    });
    let bytes = fs::read(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        process::exit(2);
    });
    let report = fhir_pods_core::probe(&bytes);
    println!("{report}");
    if report.contains("\"valid\":false") {
        process::exit(1);
    }
}
