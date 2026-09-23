//! Export: one record, written against the base R4 definitions, spoken for
//! each region — the profiles are picked, the fixed content added, ids and
//! references kept, the document assembled, the read audited; what the
//! region demands and the record lacks is named.
mod common;

use fhir_pods_core::export::{ExportError, Region, REGIONS};
use fhir_pods_core::terminology::Expansions;
use fhir_pods_core::{wire, Validator};

/// The two IPS pregnancy sets — the IG's own LOINC enumerations, code-only —
/// as a host loads them: offline, the IPS pregnancy profiles are told apart
/// from every other Observation only by these, so an IPS document with any
/// Observation in it needs them (see `document.rs` F020).
const IPS_PREGNANCY_SETS: &str = "fhir-pods-terminology 1\n= http://hl7.org/fhir/uv/ips/ValueSet/edd-method-uv-ips\nhttp://loinc.org\t11778-8\t\nhttp://loinc.org\t11779-6\t\nhttp://loinc.org\t11780-4\t\n= http://hl7.org/fhir/uv/ips/ValueSet/pregnancies-summary-uv-ips\nhttp://loinc.org\t11636-8\t\nhttp://loinc.org\t11637-6\t\nhttp://loinc.org\t11638-4\t\nhttp://loinc.org\t11639-2\t\nhttp://loinc.org\t11640-0\t\nhttp://loinc.org\t11612-9\t\nhttp://loinc.org\t11613-7\t\nhttp://loinc.org\t11614-5\t\nhttp://loinc.org\t33065-4\t\n";

const BASE: &str = "http://hl7.org/fhir/StructureDefinition/";

/// A patient record as a host lowers it: base Patient, base vital-sign
/// profiles, base Condition, and an Observation no jurisdiction profiles
/// (a step count) — every one built through the base definitions' slots.
fn record(v: &Validator) -> Vec<String> {
    let lower = |profile: &str, inputs: &str| {
        let l = v.lower(&format!("{BASE}{profile}"), inputs.as_bytes()).unwrap();
        assert!(l.validation.is_valid(), "{profile}: {}", wire::report(&l.validation));
        l.resource
    };
    vec![
        lower(
            "Patient",
            r#"{"id":"u1","identifier":[{"system":"urn:ietf:rfc:3986","value":"urn:uuid:5b3a2b6e-1c3f-4b8e-9a9e-0c1d2e3f4a5b"}],
                "name":[{"family":"Citizen","given":["Alex"]}],"gender":"female","birthDate":"1988-04-12"}"#,
        ),
        lower(
            "bp",
            r#"{"id":"bp1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T08:00:00+10:00",
                "component:SystolicBP.valueQuantity.value":121,"component:SystolicBP.valueQuantity.unit":"mmHg",
                "component:DiastolicBP.valueQuantity.value":79,"component:DiastolicBP.valueQuantity.unit":"mmHg"}"#,
        ),
        lower(
            "heartrate",
            r#"{"id":"hr1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T08:00:00+10:00",
                "valueQuantity.value":72,"valueQuantity.unit":"beats/minute"}"#,
        ),
        lower(
            "Condition",
            r#"{"id":"c1","subject":"Patient/u1","clinicalStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-clinical","code":"active"}]},
                "category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-category","code":"problem-list-item"}]}],
                "code":{"coding":[{"system":"http://snomed.info/sct","code":"38341003"}],"text":"Hypertension"},
                "onsetDateTime":"2024-01-01"}"#,
        ),
        lower(
            "Observation",
            r#"{"id":"steps1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20",
                "code":{"coding":[{"system":"http://loinc.org","code":"55423-8"}],"text":"Steps"},
                "category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"activity"}]}],
                "valueQuantity":{"value":8421,"unit":"steps","system":"http://unitsofmeasure.org","code":"{steps}"}}"#,
        ),
    ]
}

fn inputs(resources: &[String], extra: &str) -> String {
    format!(
        r#"{{"resources":[{}],"author":"Patient/u1","date":"2026-09-21T10:00:00+10:00","title":"My health record"{extra}}}"#,
        resources.join(",")
    )
}

#[test]
fn the_regions_are_the_compiled_guides_and_ips_is_the_floor() {
    assert_eq!(REGIONS.iter().map(|r| r.name).collect::<Vec<_>>(), ["au", "us", "uk", "ca", "nz", "ips"]);
    assert!(Region::parse("au").is_some() && Region::parse("in").is_none());
    for r in REGIONS {
        assert!(!r.candidates("Patient").is_empty(), "{}: a region without a Patient profile", r.name);
        assert!(r.candidates("NutritionOrder").is_empty());
    }
    let v = Validator::new();
    assert_eq!(v.export("in", b"{}").unwrap_err(), ExportError::UnknownRegion);
    assert_eq!(v.export("au", b"[]").unwrap_err(), ExportError::InputsNotAnObject);
    assert_eq!(v.export("au", br#"{"resources":[{"id":"x"}]}"#).unwrap_err(), ExportError::ResourcesNotAnArray);
    assert_eq!(
        v.export("au", br#"{"resources":[{"resourceType":"Widget"}]}"#).unwrap_err(),
        ExportError::NotAResource(0)
    );
}

#[test]
fn one_base_record_is_re_profiled_for_australia_with_the_profiles_fixed_content_added() {
    let v = Validator::new();
    let record = record(&v);
    let e = v.export("au", inputs(&record, "").as_bytes()).unwrap();
    let by_id = |id: &str| e.resources.iter().find(|r| r.id.as_deref() == Some(id)).unwrap();
    // every resource the region profiles went through its profile
    // the AU Patient Summary's own Patient profile (it derives from AU Core Patient)
    assert_eq!(by_id("u1").profile.unwrap().name, "AU PS Patient");
    assert!(by_id("u1").also.is_none());
    assert_eq!(by_id("bp1").profile.unwrap().name, "AU Core Blood Pressure");
    assert_eq!(by_id("hr1").profile.unwrap().name, "AU Core Heart Rate");
    assert_eq!(by_id("c1").profile.unwrap().name, "AU PS Condition");
    // the jurisdiction's own codings came from the profile, not the record
    let bp = &by_id("bp1").resource;
    assert!(bp.contains("271649006") && bp.contains("75367002"), "AU adds SNOMED beside LOINC: {bp}");
    assert!(bp.contains(r#""id":"bp1""#) && bp.contains(r#""reference":"Patient/u1""#), "{bp}");
    assert!(bp.contains("au-core-bloodpressure"), "{bp}");
    for r in &e.resources {
        if r.profile.is_some() {
            assert!(r.validation.is_valid(), "{}: {}", r.resource_type, wire::report(&r.validation));
            assert!(r.missing.is_empty() && r.extras.is_empty(), "{}: {:?} {:?}", r.resource_type, r.missing, r.extras);
        }
    }
    // the step count has no AU Core profile: exported as given, and said so
    let steps = by_id("steps1");
    assert!(steps.profile.is_none());
    let note = steps.note.as_deref().unwrap();
    assert!(
        note.starts_with("no Australia profile for this Observation") && note.ends_with("exported as given"),
        "{note}"
    );
    assert!(steps.validation.is_valid(), "{}", wire::report(&steps.validation));
    assert!(steps.resource.contains("55423-8"));
    // the AU Patient Summary was assembled from the re-profiled resources
    let d = e.document.as_ref().unwrap();
    assert!(d.validation.is_valid(), "{}", wire::report(&d.validation));
    assert!(d.document.contains("au-ps-bundle") && d.document.contains("271649006"), "{}", &d.document[..300]);
    let sections: Vec<&str> = d.placed.iter().map(|(_, s)| s.as_str()).collect();
    assert!(sections.contains(&"8716-3"), "vital signs placed: {sections:?}");
    assert!(sections.contains(&"11450-4"), "problems placed: {sections:?}");
    assert!(e.audit.is_none());
    assert!(e.is_valid());
}

#[test]
fn the_same_record_speaks_us_core_and_the_international_summary() {
    let v = Validator::new();
    let record = record(&v);
    let table = Expansions::parse(IPS_PREGNANCY_SETS).unwrap();
    for (region, patient, bp, condition) in [
        (
            "us",
            "US Core Patient Profile",
            "US Core Blood Pressure Profile",
            "US Core Condition Problems and Health Concerns Profile",
        ),
        ("ips", "Patient (IPS)", "Observation Blood Pressure Profile", "Condition (IPS)"),
    ] {
        let e = v.export_with(region, inputs(&record, "").as_bytes(), &table).unwrap();
        let by_id = |id: &str| e.resources.iter().find(|r| r.id.as_deref() == Some(id)).unwrap();
        assert_eq!(by_id("u1").profile.map(|p| p.name), Some(patient), "{region}");
        assert_eq!(by_id("c1").profile.map(|p| p.name), Some(condition), "{region}");
        // the IPS has no vital-sign profiles of its own: its document asks for
        // the base `vitalsigns`, and a blood pressure is the base `bp`
        assert_eq!(by_id("bp1").profile.map(|p| p.name), Some(bp), "{region}");
        let d = e.document.as_ref().unwrap();
        assert!(d.validation.is_valid(), "{region} document: {}", wire::report(&d.validation));
        assert!(d.document.contains("Bundle-uv-ips"));
        for r in &e.resources {
            assert!(r.validation.is_valid(), "{region} {}: {}", r.resource_type, wire::report(&r.validation));
        }
    }
    // US Core's Condition is not the IPS's: the document asks for the IPS
    // one too, so it is declared beside US Core's and both are verified
    let e = v.export_with("us", inputs(&record, "").as_bytes(), &table).unwrap();
    // a step count is what US Core's Simple Observation is for ("any simple
    // observations not otherwise covered"); its Clinical Result profile,
    // scoped in words to clinical test results, is not matched by type alone
    let steps = e.resources.iter().find(|r| r.id.as_deref() == Some("steps1")).unwrap();
    assert_eq!(steps.profile.map(|p| p.name), Some("US Core Simple Observation Profile"), "{:?}", steps.note);
    assert!(steps.also.is_none());
    let c = e.resources.iter().find(|r| r.id.as_deref() == Some("c1")).unwrap();
    assert_eq!(c.also.map(|p| p.name), Some("Condition (IPS)"));
    assert!(c.also_missing.is_empty(), "{:?}", c.also_missing);
    assert!(
        c.resource.contains("us-core-condition-problems-health-concerns") && c.resource.contains("Condition-uv-ips")
    );
    // offline — without the IPS pregnancy sets — the IPS document cannot
    // tell its pregnancy entries from other Observations, and says so
    let offline = v.export("ips", inputs(&record, "").as_bytes()).unwrap();
    let d = offline.document.as_ref().unwrap();
    assert!(!d.validation.is_valid());
    assert!(d
        .validation
        .report
        .issues
        .iter()
        .any(|i| i.message.contains("more than one slice") && i.message.contains("pregnancy")));
}

#[test]
fn what_the_region_demands_and_the_record_lacks_is_missing_not_invented() {
    let v = Validator::new();
    // a Patient with a name and nothing else: AU Core wants identifier, gender, birthDate
    let patient = v.lower(&format!("{BASE}Patient"), br#"{"id":"u2","name":[{"family":"Citizen"}]}"#).unwrap().resource;
    let e = v.export("au", format!(r#"{{"resources":[{patient}],"document":false}}"#).as_bytes()).unwrap();
    let p = &e.resources[0];
    assert_eq!(p.profile.unwrap().name, "AU PS Patient");
    assert_eq!(p.missing, ["identifier", "gender", "birthDate"]);
    assert!(!p.validation.is_valid());
    assert!(!p.resource.contains("identifier"), "nothing invented: {}", p.resource);
    assert!(e.document.is_none());
    // an IPS document needs a birthDate; the international floor asks for less than AU
    let e = v.export("ips", format!(r#"{{"resources":[{patient}],"document":false}}"#).as_bytes()).unwrap();
    assert_eq!(e.resources[0].missing, ["birthDate"]);
}

#[test]
fn a_declared_profile_settles_the_choice_and_content_no_slot_takes_is_named() {
    let v = Validator::new();
    // an Observation that identifies with no US Core profile by content, but declares one
    let obs = r#"{"resourceType":"Observation","id":"o1","meta":{"profile":["http://hl7.org/fhir/us/core/StructureDefinition/us-core-observation-lab"]},
        "status":"final","category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"laboratory"}]}],
        "code":{"coding":[{"system":"http://loinc.org","code":"2339-0"}]},"subject":{"reference":"Patient/u1"},
        "effectiveDateTime":"2026-09-20T08:00:00+10:00","valueQuantity":{"value":5.4,"unit":"mmol/L","system":"http://unitsofmeasure.org","code":"mmol/L"},
        "bodySite":{"text":"finger"}}"#;
    let e = v.export("us", format!(r#"{{"resources":[{obs}],"document":false}}"#).as_bytes()).unwrap();
    let o = &e.resources[0];
    assert_eq!(o.profile.unwrap().url, "http://hl7.org/fhir/us/core/StructureDefinition/us-core-observation-lab");
    // the region's document (the IPS) asks its Results entries for the IPS
    // laboratory profile too: declared beside US Core's, verified, and what
    // it requires that the record lacks is named — the IPS wants a performer
    assert_eq!(o.also.map(|p| p.name), Some("Observation Results - Laboratory/Pathology (IPS)"));
    assert_eq!(o.also_missing, ["performer"]);
    assert!(
        o.resource.contains("us-core-observation-lab")
            && o.resource.contains("Observation-results-laboratory-pathology-uv-ips")
    );
    assert!(!o.validation.is_valid());
    assert!(
        o.validation.report.issues.iter().any(|i| i.path == "Observation.performer"),
        "{}",
        wire::report(&o.validation)
    );
    // no US Core lab slot takes bodySite: carried as given, named as carried, verified with the rest
    assert_eq!(o.carried, ["bodySite"]);
    assert!(o.extras.is_empty(), "{:?}", o.extras);
    assert!(o.resource.contains(r#""bodySite":{"text":"finger"}"#), "{}", o.resource);
    // and the same glucose without the declaration: laboratory category identifies it
    let undeclared = obs.replace(
        r#""meta":{"profile":["http://hl7.org/fhir/us/core/StructureDefinition/us-core-observation-lab"]},"#,
        "",
    );
    let e = v.export("us", format!(r#"{{"resources":[{undeclared}],"document":false}}"#).as_bytes()).unwrap();
    let o = &e.resources[0];
    assert_eq!(o.profile.map(|p| p.name), Some("US Core Laboratory Result Observation Profile"), "{:?}", o.note);
}

#[test]
fn the_export_is_audited_as_a_patient_read_of_the_document() {
    let v = Validator::new();
    let record = record(&v);
    let audit = r#","audit":{"recorded":"2026-09-21T00:00:00Z","client":{"who":"Device/app-7","network":"10.0.0.7"},
        "server":{"who":{"display":"api.example.org"},"network":"https://api.example.org/r4"},
        "user":{"who":"Patient/u1","name":"Alex Citizen"},"source":{"observer":"Device/app-7","site":"app"},"transaction":"req-1"}"#;
    let e = v.export("au", inputs(&record, audit).as_bytes()).unwrap();
    let a = e.audit.as_ref().unwrap();
    assert!(a.validation.is_valid(), "{}", wire::report(&a.validation));
    let doc_id = e.document.as_ref().unwrap().document.split(r#""id":""#).nth(1).unwrap().split('"').next().unwrap();
    assert!(a.resource.contains(&format!(r#""reference":"Bundle/{doc_id}""#)), "{}", a.resource);
    assert!(a.resource.contains(r#""reference":"Patient/u1""#), "{}", a.resource);
    assert!(a.resource.contains("req-1"));
    assert!(e.is_valid());
    // the JSON the pods hand out carries all of it
    let j = e.to_json();
    assert!(j.starts_with(r#"{"region":{"name":"au","title":"Australia""#), "{}", &j[..120]);
    assert!(j.contains(r#""profile":{"url":"http://hl7.org.au/fhir/core/StructureDefinition/au-core-bloodpressure","name":"AU Core Blood Pressure"}"#));
    assert!(j.contains(r#""document":{"kind":"au-ps","document":{"resourceType":"Bundle""#));
    assert!(j.contains(r#""audit":{"resource":{"resourceType":"AuditEvent""#));
    // an audit that cannot be built is the export's error, not a silent gap
    let bad = r#","audit":{"recorded":"2026-09-21T00:00:00Z"}"#;
    assert!(matches!(v.export("au", inputs(&record, bad).as_bytes()).unwrap_err(), ExportError::Audit(_)));
}

/// What the fresh review of v0.11.0 found: nothing invented, nothing
/// silently dropped, nobody else's data.
#[test]
fn a_record_that_contradicts_the_profiles_fixed_content_goes_out_as_given_and_says_why() {
    let v = Validator::new();
    // a blood pressure in kPa: the base bp fixes mm[Hg], so lowering it is
    // refused by the base profile itself — hand-write the resource instead
    let kpa = r#"{"resourceType":"Observation","id":"bp-kpa","status":"final",
        "category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs"}]}],
        "code":{"coding":[{"system":"http://loinc.org","code":"85354-9"}]},"subject":{"reference":"Patient/u1"},
        "effectiveDateTime":"2026-09-20T08:00:00+10:00",
        "component":[{"code":{"coding":[{"system":"http://loinc.org","code":"8480-6"}]},"valueQuantity":{"value":16.1,"unit":"kPa","system":"http://unitsofmeasure.org","code":"kPa"}},
                     {"code":{"coding":[{"system":"http://loinc.org","code":"8462-4"}]},"valueQuantity":{"value":10.5,"unit":"kPa","system":"http://unitsofmeasure.org","code":"kPa"}}]}"#;
    for region in ["au", "us", "ips"] {
        let e = v.export(region, format!(r#"{{"resources":[{kpa}],"document":false}}"#).as_bytes()).unwrap();
        let r = &e.resources[0];
        assert!(r.profile.is_none(), "{region}: re-profiled a kPa pressure as {:?}", r.profile.map(|p| p.name));
        let note = r.note.as_deref().unwrap();
        assert!(
            note.starts_with("not re-profiled:") && note.contains("kPa") && note.contains("mm[Hg]"),
            "{region}: {note}"
        );
        assert!(r.resource.contains(r#""code":"kPa""#) && !r.resource.contains("mm[Hg]"), "{region}: {}", r.resource);
    }
}

#[test]
fn what_no_slot_takes_is_carried_not_dropped_and_what_cannot_be_is_named() {
    let v = Validator::new();
    let table = Expansions::parse(IPS_PREGNANCY_SETS).unwrap();
    // a base bp with a third component (mean arterial pressure), a local
    // coding beside LOINC, a second category, security labels, a note, an
    // issued instant, a performer and a text on the code
    let bp = v
        .lower(
            &format!("{BASE}bp"),
            br#"{"id":"bp1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T08:00:00+10:00",
            "component:SystolicBP.valueQuantity.value":121,"component:DiastolicBP.valueQuantity.value":79}"#,
        )
        .unwrap()
        .resource;
    let rich = bp
        .replace(
            r#""component":["#,
            r#""component":[{"code":{"coding":[{"system":"http://loinc.org","code":"8478-0"}]},"valueQuantity":{"value":93,"unit":"mm[Hg]","system":"http://unitsofmeasure.org","code":"mm[Hg]"}},"#,
        )
        .replace(
            r#""meta":{"profile":["http://hl7.org/fhir/StructureDefinition/bp"]}"#,
            r#""meta":{"profile":["http://hl7.org/fhir/StructureDefinition/bp"],"security":[{"system":"http://terminology.hl7.org/CodeSystem/v3-Confidentiality","code":"R"}],"source":"http://example.org/app-7"},"note":[{"text":"seated"}],"issued":"2026-09-20T08:05:00+10:00","performer":[{"reference":"Patient/u1"}]"#,
        )
        .replace(
            r#""code":{"coding":[{"system":"http://loinc.org","code":"85354-9"}]}}"#,
            r#""code":{"coding":[{"system":"http://loinc.org","code":"85354-9"},{"system":"http://example.org/local","code":"BP"}],"text":"Blood pressure"}}"#,
        );
    assert!(v.validate(rich.as_bytes()).is_valid(), "{}", wire::report(&v.validate(rich.as_bytes())));
    for region in ["au", "us", "ips"] {
        let e =
            v.export_with(region, format!(r#"{{"resources":[{rich}],"document":false}}"#).as_bytes(), &table).unwrap();
        let r = &e.resources[0];
        assert!(r.profile.is_some(), "{region}: {:?}", r.note);
        let out = &r.resource;
        for kept in [
            "8478-0",
            r#""code":"R""#,
            "http://example.org/app-7",
            r#""text":"seated""#,
            "2026-09-20T08:05:00+10:00",
            "http://example.org/local",
            r#""text":"Blood pressure""#,
        ] {
            assert!(out.contains(kept), "{region} lost {kept}: {out}");
        }
        for c in ["component[0]", "code.coding[1]", "note", "issued", "performer", "code.text"] {
            assert!(r.carried.iter().any(|x| x == c), "{region} carried {:?}, extras {:?}", r.carried, r.extras);
        }
        assert!(r.extras.is_empty(), "{region}: {:?}", r.extras);
        assert!(r.validation.is_valid(), "{region}: {}", wire::report(&r.validation));
    }
}

#[test]
fn a_smoking_status_keeps_its_value_and_is_not_every_social_history_observation() {
    let v = Validator::new();
    let smoking = r#"{"resourceType":"Observation","id":"s1","status":"final",
        "category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"social-history"}]}],
        "code":{"coding":[{"system":"http://loinc.org","code":"72166-2"}]},"subject":{"reference":"Patient/u1"},
        "effectiveDateTime":"2026-09-20T08:00:00+10:00","valueCodeableConcept":{"coding":[{"system":"http://snomed.info/sct","code":"266919005"}]}}"#;
    // the IPS's tobacco-use profile: the value is no slot of it, and is carried
    let e = v.export("ips", format!(r#"{{"resources":[{smoking}],"document":false}}"#).as_bytes()).unwrap();
    let r = &e.resources[0];
    assert_eq!(r.profile.map(|p| p.name), Some("Observation Social History - Tobacco Use (IPS)"));
    assert!(r.resource.contains("266919005") && r.resource.contains("social-history"), "{}", r.resource);
    assert!(r.validation.is_valid(), "{}", wire::report(&r.validation));
    // AU: the profile's own SNOMED coding joins LOINC's, nothing doubled
    let e = v.export("au", format!(r#"{{"resources":[{smoking}],"document":false}}"#).as_bytes()).unwrap();
    let r = &e.resources[0];
    assert_eq!(r.profile.map(|p| p.name), Some("AU PS Smoking Status"), "{:?}", r.note);
    assert_eq!(r.resource.matches("72166-2").count(), 1, "{}", r.resource);
    assert!(r.resource.contains("1747861000168109"), "{}", r.resource);
    assert!(r.validation.is_valid(), "{}", wire::report(&r.validation));
    // US Core's smoking-status profile is any social-history observation in
    // structure: never taken by content. A care-experience preference with
    // that category is not a smoking status; a pregnancy status is its own
    let preference = smoking.replace("72166-2", "95541-9");
    let e = v.export("us", format!(r#"{{"resources":[{preference}],"document":false}}"#).as_bytes()).unwrap();
    assert_ne!(e.resources[0].profile.map(|p| p.name), Some("US Core Smoking Status Observation Profile"));
    let pregnancy = smoking.replace("72166-2", "82810-3").replace("266919005", "77386006");
    let e = v.export("us", format!(r#"{{"resources":[{pregnancy}],"document":false}}"#).as_bytes()).unwrap();
    assert_eq!(
        e.resources[0].profile.map(|p| p.name),
        Some("US Core Observation Pregnancy Status Profile"),
        "{:?}",
        e.resources[0].note
    );
    // and declared, US Core Smoking Status is honoured
    let declared = smoking.replace(
        r#""id":"s1","#,
        r#""id":"s1","meta":{"profile":["http://hl7.org/fhir/us/core/StructureDefinition/us-core-smokingstatus"]},"#,
    );
    let e = v.export("us", format!(r#"{{"resources":[{declared}],"document":false}}"#).as_bytes()).unwrap();
    assert_eq!(e.resources[0].profile.map(|p| p.name), Some("US Core Smoking Status Observation Profile"));
}

#[test]
fn nobody_elses_data_leaves_in_a_record_and_a_record_is_one_patients() {
    let v = Validator::new();
    let record = record(&v);
    let other = record[2]
        .replace(r#""id":"hr1""#, r#""id":"hr2""#)
        .replace(r#""reference":"Patient/u1""#, r#""reference":"Patient/OTHER""#);
    let mut with_other = record.clone();
    with_other.push(other);
    assert_eq!(
        v.export("au", inputs(&with_other, "").as_bytes()).unwrap_err(),
        ExportError::NotThisPatient(5, "Patient/OTHER".into())
    );
    let second = record[0].replace(r#""id":"u1""#, r#""id":"u2""#);
    let mut two = record.clone();
    two.push(second);
    assert_eq!(v.export("au", inputs(&two, "").as_bytes()).unwrap_err(), ExportError::SeveralPatients);
}

#[test]
fn a_primitive_carrier_on_a_choice_and_a_contained_performer_survive_re_profiling() {
    let v = Validator::new();
    let bp = v
        .lower(
            &format!("{BASE}bp"),
            br#"{"id":"bp1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T08:00:00+10:00",
            "component:SystolicBP.valueQuantity.value":121,"component:DiastolicBP.valueQuantity.value":79}"#,
        )
        .unwrap()
        .resource
        .replace(
            r#""effectiveDateTime":"2026-09-20T08:00:00+10:00""#,
            r##""effectiveDateTime":"2026-09-20T08:00:00+10:00","_effectiveDateTime":{"extension":[{"url":"http://example.org/ext/precision","valueString":"approx"}]},"contained":[{"resourceType":"Practitioner","id":"pr","name":[{"family":"Who"}]}],"performer":[{"reference":"#pr"}]"##,
        );
    for region in ["au", "us", "ips"] {
        let e = v.export(region, format!(r#"{{"resources":[{bp}],"document":false}}"#).as_bytes()).unwrap();
        let r = &e.resources[0];
        assert!(r.profile.is_some(), "{region}: {:?}", r.note);
        assert!(r.resource.contains(r#""_effectiveDateTime":{"extension""#), "{region}: {}", r.resource);
        assert!(r.resource.contains(r##""reference":"#pr""##), "{region}: {}", r.resource);
        assert!(r.validation.is_valid(), "{region}: {}", wire::report(&r.validation));
    }
}

#[test]
fn the_audit_of_a_resources_only_export_reads_the_patient_and_a_document_requirement_is_satisfied_by_the_target() {
    let v = Validator::new();
    let record = record(&v);
    let audit = r#","audit":{"recorded":"2026-09-21T00:00:00Z","client":{"who":"Device/app-7","network":"10.0.0.7"},
        "server":{"who":{"display":"api.example.org"},"network":"https://api.example.org/r4"},
        "source":{"observer":"Device/app-7","site":"app"},"transaction":"req-1"},"document":false"#;
    let e = v.export("au", inputs(&record, audit).as_bytes()).unwrap();
    let a = e.audit.as_ref().unwrap();
    assert!(a.validation.is_valid(), "{}", wire::report(&a.validation));
    assert_eq!(a.resource.matches(r#""reference":"Patient/u1""#).count(), 2, "patient and data: {}", a.resource);
    // a US blood pressure satisfies the IPS document's `vitalsigns` entry
    // requirement through its own lineage: nothing else declared, no note
    let e = v.export("us", inputs(&record, r#","document":false"#).as_bytes()).unwrap();
    let bp = e.resources.iter().find(|r| r.id.as_deref() == Some("bp1")).unwrap();
    assert_eq!(bp.profile.map(|p| p.name), Some("US Core Blood Pressure Profile"));
    assert!(bp.also.is_none() && bp.notes.is_empty(), "{:?}", bp.notes);
    // a declared profile of another region is not applied, and said
    let foreign = record[2].replace(
        "http://hl7.org/fhir/StructureDefinition/heartrate",
        "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate",
    );
    let e = v.export("us", format!(r#"{{"resources":[{foreign}],"document":false}}"#).as_bytes()).unwrap();
    assert_eq!(e.resources[0].profile.map(|p| p.name), Some("US Core Heart Rate Profile"));
    assert!(
        e.resources[0].notes.iter().any(|n| n.contains("au-core-heartrate") && n.contains("not applied")),
        "{:?}",
        e.resources[0].notes
    );
}

#[test]
fn the_same_record_speaks_the_united_kingdom_canada_and_new_zealand() {
    let v = Validator::new();
    let record = record(&v);
    let table = Expansions::parse(IPS_PREGNANCY_SETS).unwrap();
    // (region, Patient, blood pressure, `also` on the blood pressure, Condition)
    for (region, patient, bp, bp_also, condition) in [
        // UK Core has one general Observation profile and no vital-sign ones: the
        // IPS document's `vitalsigns` requirement is met by the base `bp` beside it
        (
            "uk",
            "UK Core Patient",
            "UK Core Observation",
            Some("Observation Blood Pressure Profile"),
            "UK Core Condition",
        ),
        // CA Baseline: the general Observation (its laboratory profile identifies by category);
        // its Condition profile is one the reference validator cannot evaluate (a
        // binding.valueSet discriminator), so the IPS document's Condition applies
        (
            "ca",
            "Patient Profile",
            "Observation Profile (General Use)",
            Some("Observation Blood Pressure Profile"),
            "Condition (IPS)",
        ),
        // NZ Base has no Observation profile: the document's `vitalsigns` family — a `bp`
        ("nz", "NZ Patient", "Observation Blood Pressure Profile", None, "Condition resource for use in NZ"),
    ] {
        let e = v.export_with(region, inputs(&record, "").as_bytes(), &table).unwrap();
        let by_id = |id: &str| e.resources.iter().find(|r| r.id.as_deref() == Some(id)).unwrap();
        assert_eq!(by_id("u1").profile.map(|p| p.name), Some(patient), "{region}: {:?}", by_id("u1").note);
        assert_eq!(by_id("bp1").profile.map(|p| p.name), Some(bp), "{region}: {:?}", by_id("bp1").note);
        assert_eq!(by_id("bp1").also.map(|p| p.name), bp_also, "{region}: {:?}", by_id("bp1").notes);
        assert_eq!(by_id("c1").profile.map(|p| p.name), Some(condition), "{region}: {:?}", by_id("c1").note);
        let d = e.document.as_ref().unwrap();
        assert!(d.document.contains("Bundle-uv-ips"));
        for r in &e.resources {
            assert!(r.validation.is_valid(), "{region} {}: {}", r.resource_type, wire::report(&r.validation));
        }
        // a profile with no slots of its own (NZ Patient decorates the base
        // with extensions) carries the record whole — nothing is dropped
        let patient = by_id("u1");
        for kept in ["Citizen", "1988-04-12", "female", "urn:uuid:5b3a2b6e"] {
            assert!(patient.resource.contains(kept), "{region} lost {kept}: {}", patient.resource);
        }
        assert!(patient.extras.is_empty(), "{region}: {:?}", patient.extras);
    }
}
