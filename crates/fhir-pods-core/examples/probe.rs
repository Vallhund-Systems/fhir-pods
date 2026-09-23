//! Validate a FHIR resource file and print a report.
//!
//!     cargo run -p fhir-pods-core --example probe -- [--outcome] [--must-support] [--questionnaire q.json]… [--terminology table.txt] path/to/resource.json
//!
//! Default output is the pods' JSON report; `--outcome` prints the spec's
//! `OperationOutcome` instead; `--must-support` prints the selected profile's
//! must-support elements the resource leaves empty (information, not
//! validation); `--questionnaire FILE` (repeatable) loads a Questionnaire a
//! QuestionnaireResponse may name; `--terminology FILE` loads a compiled
//! terminology table (`tools/compile_valueset.py`'s format) consulted after
//! the bundled sets. Exits 1 when the resource fails
//! validation, so shells and the conformance harness can branch on the verdict.
use std::{env, fs, process};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let outcome = args.iter().any(|a| a == "--outcome");
    let must_support = args.iter().any(|a| a == "--must-support");
    let mut validator = fhir_pods_core::Validator::new();
    let mut questionnaires = Vec::new();
    let mut tables = Vec::new();
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--questionnaire" => {
                i += 1;
                questionnaires.push(args.get(i).cloned().unwrap_or_default());
            }
            "--terminology" => {
                i += 1;
                tables.push(args.get(i).cloned().unwrap_or_default());
            }
            a if a.starts_with("--") => {}
            a => positional.push(a.to_string()),
        }
        i += 1;
    }
    for q in &questionnaires {
        let bytes = fs::read(q).unwrap_or_else(|e| {
            eprintln!("{q}: {e}");
            process::exit(2);
        });
        if !validator.load_questionnaire(&bytes) {
            eprintln!("{q}: not a Questionnaire");
            process::exit(2);
        }
    }
    let mut loaded: Vec<fhir_pods_core::terminology::Expansions> = Vec::new();
    for t in &tables {
        let text = fs::read_to_string(t).unwrap_or_else(|e| {
            eprintln!("{t}: {e}");
            process::exit(2);
        });
        match fhir_pods_core::terminology::Expansions::parse(&text) {
            Ok(x) => loaded.push(x),
            Err(e) => {
                eprintln!("{t}: not a terminology table ({e:?})");
                process::exit(2);
            }
        }
    }
    let Some(path) = positional.first().cloned() else {
        eprintln!("usage: probe [--outcome] [--must-support] [--questionnaire q.json]… [--terminology table.txt] <resource.json>");
        process::exit(2);
    };
    let bytes = fs::read(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        process::exit(2);
    });
    if must_support {
        let gaps = fhir_pods_core::must_support_gaps(&bytes);
        let quoted: Vec<String> = gaps.iter().map(|g| format!("\"{g}\"")).collect();
        println!("{{\"mustSupportGaps\":[{}]}}", quoted.join(","));
        return;
    }
    let mut providers: Vec<&dyn fhir_pods_core::terminology::TerminologyProvider> =
        vec![&fhir_pods_core::terminology::Bundled];
    for t in &loaded {
        providers.push(t);
    }
    let chain = fhir_pods_core::terminology::Chain(&providers);
    let v = validator.validate_with(&bytes, &chain);
    println!("{}", if outcome { v.to_operation_outcome() } else { fhir_pods_core::wire::report(&v) });
    if !v.is_valid() {
        process::exit(1);
    }
}
