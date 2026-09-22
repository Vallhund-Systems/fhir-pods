//! The export proof. One synthetic patient record, written against the base
//! R4 definitions the way a host's transformer would (base Patient, the base
//! vital-sign profiles, base Condition / AllergyIntolerance /
//! MedicationStatement / Immunization, a step count no guide profiles),
//! exported for every region: each re-profiled resource, each region's
//! document and the audit event are written as `export-<region>-<id>.json`
//! with a manifest carrying OUR verdict, so the conformance harness puts the
//! exporter's every output in front of the reference validator — parity,
//! not cleanliness: the IPS documents carry the offline pregnancy-profile
//! ambiguity both validators report without a terminology server.
//!
//!     cargo run -p fhir-pods-core --example export_corpus -- conformance/corpus
use std::{env, fs, process};

use fhir_pods_core::export::REGIONS;
use fhir_pods_core::{wire, Validator};

const BASE: &str = "http://hl7.org/fhir/StructureDefinition/";

/// (profile, inputs) — every resource a patient app might hold, as base inputs.
const RECORD: &[(&str, &str)] = &[
    (
        "Patient",
        r#"{"id":"u1","identifier":[{"system":"urn:ietf:rfc:3986","value":"urn:uuid:5b3a2b6e-1c3f-4b8e-9a9e-0c1d2e3f4a5b"}],
            "name":[{"family":"Citizen","given":["Alex"]}],"gender":"female","birthDate":"1988-04-12"}"#,
    ),
    (
        "bp",
        r#"{"id":"bp1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T08:00:00+10:00",
            "component:SystolicBP.valueQuantity.value":121,"component:SystolicBP.valueQuantity.unit":"mmHg",
            "component:DiastolicBP.valueQuantity.value":79,"component:DiastolicBP.valueQuantity.unit":"mmHg"}"#,
    ),
    (
        "heartrate",
        r#"{"id":"hr1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T08:00:00+10:00",
            "valueQuantity.value":72,"valueQuantity.unit":"beats/minute"}"#,
    ),
    (
        "bodyweight",
        r#"{"id":"wt1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T07:30:00+10:00",
            "valueQuantity.value":64.2,"valueQuantity.unit":"kg","valueQuantity.code":"kg"}"#,
    ),
    (
        "bodytemp",
        r#"{"id":"temp1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T07:30:00+10:00",
            "valueQuantity.value":36.7,"valueQuantity.unit":"Cel","valueQuantity.code":"Cel"}"#,
    ),
    (
        "oxygensat",
        r#"{"id":"spo2","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T07:30:00+10:00",
            "valueQuantity.value":98,"valueQuantity.unit":"%"}"#,
    ),
    (
        "Condition",
        r#"{"id":"c1","subject":"Patient/u1","clinicalStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-clinical","code":"active"}]},
            "verificationStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-ver-status","code":"confirmed"}]},
            "category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-category","code":"problem-list-item"}]}],
            "code":{"coding":[{"system":"http://snomed.info/sct","code":"38341003"}],"text":"Hypertension"},
            "onsetDateTime":"2024-01-01"}"#,
    ),
    (
        "AllergyIntolerance",
        r#"{"id":"a1","patient":"Patient/u1","clinicalStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/allergyintolerance-clinical","code":"active"}]},
            "verificationStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/allergyintolerance-verification","code":"confirmed"}]},
            "code":{"coding":[{"system":"http://snomed.info/sct","code":"91936005"}],"text":"Penicillin allergy"}}"#,
    ),
    (
        "MedicationStatement",
        r#"{"id":"m1","status":"active","subject":"Patient/u1","effectiveDateTime":"2026-09-01",
            "medicationCodeableConcept":{"coding":[{"system":"http://snomed.info/sct","code":"318420003"}],"text":"Amlodipine 5 mg tablet"},
            "dosage":[{"text":"1 tablet daily"}]}"#,
    ),
    (
        "Immunization",
        r#"{"id":"i1","status":"completed","patient":"Patient/u1","occurrenceDateTime":"2026-04-02",
            "vaccineCode":{"coding":[{"system":"http://snomed.info/sct","code":"1181000221105"}],"text":"Influenza vaccine"},
            "primarySource":true}"#,
    ),
    (
        "Observation",
        r#"{"id":"steps1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20",
            "code":{"coding":[{"system":"http://loinc.org","code":"55423-8"}],"text":"Steps"},
            "category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"activity"}]}],
            "valueQuantity":{"value":8421,"unit":"steps","system":"http://unitsofmeasure.org","code":"{steps}"}}"#,
    ),
];

const AUDIT: &str = r#"{"recorded":"2026-09-21T00:00:00Z","client":{"who":"Device/app-7","network":"10.0.0.7"},
    "server":{"who":{"display":"api.example.org"},"network":"https://api.example.org/r4"},
    "user":{"who":"Patient/u1","name":"Alex Citizen"},"source":{"observer":"Device/app-7","site":"app"},"transaction":"req-1"}"#;

fn main() {
    let dir = env::args().nth(1).unwrap_or_else(|| "conformance/corpus".into());
    let validator = Validator::new();
    let mut resources = Vec::new();
    for (profile, inputs) in RECORD {
        let l =
            validator.lower(&format!("{BASE}{profile}"), inputs.as_bytes()).expect("the base definition is compiled");
        if !l.validation.is_valid() {
            eprintln!("base {profile}: {}", wire::report(&l.validation));
            process::exit(1);
        }
        resources.push(l.resource);
    }
    let inputs = format!(
        r#"{{"resources":[{}],"author":"Patient/u1","date":"2026-09-21T10:00:00+10:00","title":"My health record","audit":{AUDIT}}}"#,
        resources.join(",")
    );

    let mut manifest_lines = Vec::new();
    let mut unprofiled = 0;
    let mut record = |name: &str, profile: &str, resource: &str, valid: bool| {
        fs::write(format!("{dir}/{name}.json"), format!("{resource}\n")).unwrap();
        manifest_lines.push(format!(
            "{{\"file\": \"{name}.json\", \"profile\": \"{profile}\", \"expect\": {valid}, \"ig\": \"export\"}}"
        ));
        println!(
            "{:<44} {}",
            format!("{name}.json"),
            if valid { "ok" } else { "not valid on our side (parity expected)" }
        );
    };
    for region in REGIONS {
        let e = validator.export(region.name, inputs.as_bytes()).expect("the export starts");
        for r in &e.resources {
            let profile = r.profile.map(|p| p.url).unwrap_or("(as given)");
            if r.profile.is_none() {
                unprofiled += 1;
                println!("{:>8} {}: {}", region.name, r.resource_type, r.note.as_deref().unwrap_or(""));
            }
            let id = r.id.as_deref().unwrap_or("");
            record(&format!("export-{}-{id}", region.name), profile, &r.resource, r.validation.is_valid());
        }
        let d = e.document.as_ref().expect("every region has a document");
        record(
            &format!("export-{}-document", region.name),
            region.document.unwrap().name(),
            &d.document,
            d.validation.is_valid(),
        );
        let a = e.audit.as_ref().expect("the audit was asked for");
        record(&format!("export-{}-audit", region.name), "patient-read", &a.resource, a.validation.is_valid());
    }
    fs::write(format!("{dir}/export-manifest.json"), format!("[\n{}\n]\n", manifest_lines.join(",\n"))).unwrap();
    println!("export records: {}; {unprofiled} exported as given", manifest_lines.len());
}
