//! Validate a FHIR resource file and print a report.
//!
//!     cargo run -p fhir-pods-core --example probe -- [--outcome] path/to/resource.json
//!
//! Default output is the pods' JSON report; `--outcome` prints the spec's
//! `OperationOutcome` instead. Exits 1 when the resource fails validation, so
//! shells and the conformance harness can branch on the verdict.
use std::{env, fs, process};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let outcome = args.iter().any(|a| a == "--outcome");
    let Some(path) = args.iter().find(|a| !a.starts_with("--")).cloned() else {
        eprintln!("usage: probe [--outcome] <resource.json>");
        process::exit(2);
    };
    let bytes = fs::read(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        process::exit(2);
    });
    let v = fhir_pods_core::validate_bytes(&bytes);
    println!("{}", if outcome { v.to_operation_outcome() } else { fhir_pods_core::wire::report(&v) });
    if !v.is_valid() {
        process::exit(1);
    }
}
