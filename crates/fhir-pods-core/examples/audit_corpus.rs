//! The audit proof. One `AuditEvent` per BALP basic kind, built by the audit
//! surface from synthetic host inputs, and a `Provenance` and a failed-action
//! `AuditEvent` lowered from the base definitions — each written as
//! `audit-<kind>.json` with a manifest carrying our verdict, so the
//! conformance harness can put every record in front of the reference
//! validator.
//!
//!     cargo run -p fhir-pods-core --example audit_corpus -- conformance/corpus
use std::{env, fs, process};

use fhir_pods_core::audit::Kind;
use fhir_pods_core::{wire, Validator};

const BASE: &str = r#"{
  "recorded": "2026-09-16T10:00:00Z",
  "client": {"who": "Device/app-7", "network": "10.0.0.7"},
  "server": {"who": {"display": "fhir.example.org"}, "network": "https://fhir.example.org/r4"},
  "user": {"who": "Practitioner/gp1", "name": "Dr Who"},
  "source": {"observer": "Device/audit-collector", "site": "clinic-1"},
  "data": "Observation/hr1",
  "transaction": "3f2a1c9e-7b1d-4f0e-9a8c-1d2e3f4a5b6c"
}"#;

fn inputs(kind: Kind, with_user: bool) -> String {
    let mut s = BASE.to_string();
    if kind.has_patient() {
        s = s.replace(r#""data": "Observation/hr1","#, r#""data": "Observation/hr1", "patient": "Patient/p1","#);
    }
    if kind.is_query() {
        s = s.replace(r#""data": "Observation/hr1","#, r#""query": "GET /r4/Observation?patient=p1&_count=10","#);
    }
    if !with_user {
        s = s.replace(
            r#"  "user": {"who": "Practitioner/gp1", "name": "Dr Who"},
"#,
            "",
        );
    }
    s
}

fn main() {
    let dir = env::args().nth(1).unwrap_or_else(|| "conformance/corpus".into());
    let validator = Validator::new();
    let mut manifest_lines = Vec::new();
    let mut failed = 0;
    let mut record = |name: &str, profile: &str, resource: &str, valid: bool| {
        fs::write(format!("{dir}/{name}.json"), format!("{resource}\n")).unwrap();
        manifest_lines.push(format!(
            "{{\"file\": \"{name}.json\", \"profile\": \"{profile}\", \"expect\": true, \"ig\": \"audit\"}}"
        ));
        println!("{:<44} {}", format!("{name}.json"), if valid { "ok" } else { "NOT VALID on our side" });
        if !valid {
            failed += 1;
        }
    };
    for kind in Kind::ALL {
        for with_user in [true, false] {
            let a = validator
                .audit(kind.name(), inputs(kind, with_user).as_bytes())
                .expect("the audit surface builds every kind");
            if !a.validation.is_valid() {
                eprintln!("{}", wire::report(&a.validation));
            }
            let name = format!("audit-{}{}", kind.name(), if with_user { "" } else { "-no-user" });
            record(&name, kind.url(), &a.resource, a.validation.is_valid());
        }
    }
    let provenance = validator
        .lower(
            "http://hl7.org/fhir/StructureDefinition/Provenance",
            br#"{"target":["Observation/hr1"],"recorded":"2026-09-16T10:00:00Z","agent":[{"type":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/provenance-participant-type","code":"author"}]},"who":{"reference":"Practitioner/gp1"}}]}"#,
        )
        .expect("the base Provenance is compiled");
    record(
        "audit-provenance",
        "http://hl7.org/fhir/StructureDefinition/Provenance",
        &provenance.resource,
        provenance.validation.is_valid(),
    );
    let failed_read = validator
        .lower(
            "http://hl7.org/fhir/StructureDefinition/AuditEvent",
            br#"{"type":{"system":"http://terminology.hl7.org/CodeSystem/audit-event-type","code":"rest"},"subtype":[{"system":"http://hl7.org/fhir/restful-interaction","code":"read"}],"action":"R","recorded":"2026-09-16T10:00:00Z","outcome":"4","outcomeDesc":"not found","agent":[{"who":{"reference":"Device/app-7"},"requestor":true}],"source":{"observer":{"reference":"Device/audit-collector"}}}"#,
        )
        .expect("the base AuditEvent is compiled");
    record(
        "audit-failed-read",
        "http://hl7.org/fhir/StructureDefinition/AuditEvent",
        &failed_read.resource,
        failed_read.validation.is_valid(),
    );
    fs::write(format!("{dir}/audit-manifest.json"), format!("[\n{}\n]\n", manifest_lines.join(",\n"))).unwrap();
    println!("audit records: {}; {failed} not valid on our side", manifest_lines.len());
    if failed > 0 {
        process::exit(1);
    }
}
