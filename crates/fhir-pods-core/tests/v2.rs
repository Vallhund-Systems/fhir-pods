mod common;

use fhir_pods_core::v2::convert::{ConvertError, Options};
use fhir_pods_core::v2::{Message, ParseError, Separators};
use fhir_pods_core::{wire, Validator};

const ORU: &str = "MSH|^~\\&|LAB|ACME^1.2.3^ISO|EHR|CLINIC|20260916100000+1000||ORU^R01^ORU_R01|MSG0001|P|2.5.1\r\
PID|1||MRN12345^^^ACME^MR||Doe^Jane^A||19700615|F|||12 High St^^Springfield^VIC^3000^AU||^PRN^PH^^^^0400000000\r\
PV1|1|O|WARD1^BED2^^ACME||||1234^Who^Doctor^^^Dr^^^ACME&1.2.3&ISO\r\
ORC|RE|PLC001^EHR|FIL001^LAB\r\
OBR|1|PLC001^EHR|FIL001^LAB|24331-1^Lipid panel^LN|||20260916090000+1000||||||||||||||||||F\r\
OBX|1|NM|2093-3^Cholesterol^LN||5.2|mmol/L^millimole per liter^UCUM|<5.5|N|||F|||20260916093000+1000\r\
OBX|2|ST|11475-1^Microorganism^LN||No growth||||||F\r\
NTE|1||Fasting sample\r";

#[test]
fn er7_parses_separators_repetitions_components_subcomponents_and_escapes() {
    let m = Message::parse(ORU.as_bytes()).unwrap();
    assert_eq!(m.separators, Separators::default());
    assert_eq!(m.segments.len(), 8);
    let msh = &m.segments[0];
    assert_eq!(msh.value(1, 0, 0), "|");
    assert_eq!(msh.value(2, 0, 0), "^~\\&");
    assert_eq!(msh.value(3, 0, 0), "LAB");
    assert_eq!(msh.value(4, 1, 0), "ACME");
    assert_eq!(msh.value(4, 2, 0), "1.2.3");
    assert_eq!(msh.value(9, 1, 0), "ORU");
    assert_eq!(msh.value(9, 2, 0), "R01");
    assert_eq!(m.structure().as_deref(), Some("ORU_R01"));
    let pid = m.segments_named("PID").next().unwrap();
    assert_eq!(pid.value(3, 1, 0), "MRN12345");
    assert_eq!(pid.value(3, 4, 0), "ACME");
    assert_eq!(pid.value(5, 1, 0), "Doe");
    assert_eq!(pid.value(5, 2, 0), "Jane");
    assert_eq!(pid.value(11, 4, 0), "VIC");
    let pv1 = m.segments_named("PV1").next().unwrap();
    assert_eq!(pv1.value(7, 9, 1), "ACME");
    assert_eq!(pv1.value(7, 9, 2), "1.2.3");
    // repetitions and escapes
    let m2 = Message::parse(
        b"MSH|^~\\&|A|B|C|D|20260101||ADT^A01|1|P|2.5\rPID|1||1^^^X~2^^^Y||Smith\\F\\Jones^A\\T\\B^\\E\\^\\X41\\",
    )
    .unwrap();
    let pid = m2.segments_named("PID").next().unwrap();
    assert_eq!(pid.field(3).unwrap().reps.len(), 2);
    assert_eq!(pid.field(3).unwrap().reps[1].component(4, 0), "Y");
    assert_eq!(pid.value(5, 1, 0), "Smith|Jones");
    assert_eq!(pid.value(5, 2, 0), "A&B");
    assert_eq!(pid.value(5, 3, 0), "\\");
    assert_eq!(pid.value(5, 4, 0), "A");
    // errors are named
    assert_eq!(Message::parse(b"PID|1").unwrap_err(), ParseError::NoHeader);
    // a malformed segment name is a segment the structure will not place, not a refusal
    assert_eq!(Message::parse(b"MSH|^~\\&|A\rXY").unwrap().segments[1].name, "XY");
    assert_eq!(Message::parse(b"MSH|^~\\&|A\r|x").unwrap_err(), ParseError::BadSegment(2));
}

#[test]
fn an_oru_r01_becomes_a_message_bundle_with_the_patient_the_report_and_its_observations() {
    let v = Validator::new();
    let c = v.v2(ORU.as_bytes(), &Options::default()).unwrap();
    let b = &c.bundle;
    for s in &c.skipped {
        eprintln!("skipped {} -> {}: {}", s.at, s.target, s.why);
    }
    eprintln!("{}", wire::report(&c.validation));
    eprintln!("unplaced {:?}", c.unplaced);
    assert!(b.starts_with(r#"{"resourceType":"Bundle","id":""#) && b.contains(r#""type":"message""#), "{b}");
    assert!(b.contains(r#""resourceType":"MessageHeader""#), "{b}");
    // the sending facility's ISO universal id under urn:ietf:rfc:3986 is the URN the IG spells for it
    assert!(b.contains(r#"{"value":"urn:oid:1.2.3","type":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v2-0301","code":"ISO"}]},"system":"urn:ietf:rfc:3986"}"#), "{b}");
    // the phone's local number extension sits on ContactPoint.value, where its definition allows it
    assert!(b.contains(r#""_value":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/contactpoint-local","valueString":"0400000000"}]}"#), "{b}");
    // the patient from PID: identifier, name, birth date, gender through the table, address
    assert!(b.contains(r#""resourceType":"Patient""#), "{b}");
    assert!(b.contains(r#""value":"MRN12345""#), "{b}");
    assert!(b.contains(r#""family":"Doe""#) && b.contains(r#""given":["Jane","A"]"#), "{b}");
    assert!(b.contains(r#""birthDate":"1970-06-15""#), "{b}");
    assert!(b.contains(r#""gender":"female""#), "{b}");
    assert!(b.contains(r#""city":"Springfield""#) && b.contains(r#""state":"VIC""#), "{b}");
    // the observations from OBX: a Quantity for NM, a string for ST, LOINC codes, status through the table
    assert!(b.contains(r#""code":"2093-3","display":"Cholesterol","system":"http://loinc.org""#), "{b}");
    assert!(
        b.contains(
            r#""valueQuantity":{"value":5.2,"code":"mmol/L","unit":"millimole per liter","system":"http://unitsofmeasure.org"}"#
        ),
        "{b}"
    );
    assert!(b.contains(r#""interpretation":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v3-ObservationInterpretation","code":"N"}]}]"#), "{b}");
    // the note from NTE, on the observation it follows
    assert!(b.contains(r#""valueString":"No growth","status":"final","note":[{"text":"Fasting sample"}]"#), "{b}");
    assert!(b.contains(r#""valueString":"No growth""#), "{b}");
    assert!(b.contains(r#""status":"final""#), "{b}");
    assert!(b.contains(r#""effectiveDateTime":"2026-09-16T09:30:00+10:00""#), "{b}");
    // one report from ORC and OBR together (identifiers from ORC, code and status from OBR), its results the
    // two observations, everything about the patient
    assert_eq!(b.matches(r#""resourceType":"DiagnosticReport""#).count(), 1, "{b}");
    assert!(b.contains(r#""value":"PLC001","type":{"coding":[{"code":"PLAC","system":"http://terminology.hl7.org/CodeSystem/v2-0203"}]}"#), "{b}");
    assert!(b.contains(r#""code":{"coding":[{"code":"24331-1","display":"Lipid panel","system":"http://loinc.org"}]},"effectiveDateTime":"2026-09-16T09:00:00+10:00","status":"final","subject":{"reference":"urn:uuid:"#), "{b}");
    assert_eq!(b.matches(r#""result":[{"reference":"urn:uuid:"#).count(), 1, "{b}");
    // the encounter from PV1: class through the table, the attending practitioner; the location the
    // message did not name (no bed) is not invented, and the map's literal for it is reported
    assert!(b.contains(r#""class":{"system":"http://terminology.hl7.org/CodeSystem/v3-ActCode","code":"AMB"}"#), "{b}");
    assert!(b.contains(r#""individual":{"reference":"urn:uuid:"#) && b.contains(r#""family":"Who""#), "{b}");
    assert!(!b.contains("location"), "{b}");
    assert!(
        c.skipped.iter().any(|s| s.at == "PV1[1]" && s.target == "location[1]" && s.why.contains("literals")),
        "{:?}",
        c.skipped
    );
    assert!(b.matches(r#""subject":{"reference":"urn:uuid:"#).count() >= 3, "{b}");
    assert!(c.unplaced.is_empty(), "{:?}", c.unplaced);
    // validated on the way out; what the maps could not do is named, never silent
    let report = wire::report(&c.validation);
    assert!(c.validation.is_valid(), "{report}\n{b}");
    for s in &c.skipped {
        assert!(!s.why.is_empty() && !s.at.is_empty());
    }
    // the same message, the same bundle
    assert_eq!(v.v2(ORU.as_bytes(), &Options::default()).unwrap().bundle, c.bundle);
}

#[test]
fn a_time_without_a_zone_is_the_hosts_to_say() {
    let v = Validator::new();
    let msg = ORU.replace("20260916093000+1000", "20260916093000");
    let c = v.v2(msg.as_bytes(), &Options::default()).unwrap();
    assert!(c.bundle.contains(r#""effectiveDateTime":"2026-09-16""#), "{}", c.bundle);
    assert!(c.skipped.iter().any(|s| s.why.contains("without a zone")), "{:?}", c.skipped);
    let c = v.v2(msg.as_bytes(), &Options { timezone: Some("+10:00".into()), ..Options::default() }).unwrap();
    assert!(c.bundle.contains(r#""effectiveDateTime":"2026-09-16T09:30:00+10:00""#), "{}", c.bundle);
}

#[test]
fn what_cannot_start_is_an_error() {
    let v = Validator::new();
    assert_eq!(v.v2(b"not a message", &Options::default()).unwrap_err(), ConvertError::Parse(ParseError::NoHeader));
    assert_eq!(
        v.v2(b"MSH|^~\\&|A|B|C|D|20260101||ZZZ^Z99|1|P|2.5\r", &Options::default()).unwrap_err(),
        ConvertError::UnknownStructure("ZZZ_Z99".into())
    );
    assert_eq!(Options::from_json(br#"{"timezone": 7}"#).unwrap_err(), ConvertError::BadOptions);
    assert_eq!(Options::from_json(b"").unwrap(), Options::default());
}

/// An admission (ADT^A01): the patient, next of kin, the visit with its bed,
/// allergies and coverage — and a Z segment the structure has no place for.
const ADT: &str = "MSH|^~\\&|ADT1|GOOD HEALTH HOSPITAL|GHH LAB, INC.|GOOD HEALTH HOSPITAL|20260916103000+1000||ADT^A01^ADT_A01|MSG00001|P|2.8\r\
EVN|A01|20260916102000+1000||||20260916101500+1000\r\
PID|1||PATID1234^^^GOOD HEALTH HOSPITAL^MR||Everyman^Adam^A^III||19610615|M||2106-3^White^HL70005|2222 Home Street^^Greensboro^NC^27401-1020^US|GL|(555) 555-2004|(555)555-2004||S||PATID12345001^2^M10|123456789\r\
NK1|1|Nuclear^Nelda^W|SPO^Spouse^HL70063||(555) 555-2004||NK^Next of kin^HL70131\r\
PV1|1|I|2000^2012^01^GOOD HEALTH HOSPITAL||||004777^Attend^Aaron^A|||SUR||||ADM|A0|||||||||||||||||||||||||||||20260916101500+1000\r\
AL1|1||^Penicillin||Produces hives~Rash\r\
AL1|2||^Cat dander|SV|Anaphylaxis\r\
IN1|1|PLAN001^^^GH^PLAN|INS001|Good Health Insurance||||||||||||Everyman^Adam^A|SEL|19610615\r\
ZPI|custom\r";

#[test]
fn an_adt_a01_becomes_an_admission_with_what_the_message_carries_and_names_what_it_does_not() {
    let v = Validator::new();
    let c = v.v2(ADT.as_bytes(), &Options::default()).unwrap();
    let b = &c.bundle;
    for s in &c.skipped {
        eprintln!("skipped {} -> {}: {}", s.at, s.target, s.why);
    }
    eprintln!("{}", wire::report(&c.validation));
    for e in b.split(r#"{"fullUrl":"#).skip(1) {
        eprintln!("entry: {}", &e[..e.len().min(900)]);
    }
    assert!(
        b.contains(r#""eventCoding":{"code":"A01","system":"http://terminology.hl7.org/CodeSystem/v2-0003"}"#),
        "{b}"
    );
    // the patient: identifiers, name with suffix, marital status through the table, the next of kin as a contact
    assert!(b.contains(r#""family":"Everyman","given":["Adam","A"],"suffix":["III"]"#), "{b}");
    assert!(b.contains(r#""maritalStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v3-MaritalStatus","code":"S"}]}"#), "{b}");
    assert!(b.contains(r#""contact":[{"name":{"family":"Nuclear","given":["Nelda","W"]}"#), "{b}");
    // the next of kin as a RelatedPerson of the patient, relationship through the table
    assert!(b.contains(r#""resourceType":"RelatedPerson""#) && b.contains(r#""code":"SPS"#), "{b}");
    // the visit: inpatient, its bed a Location with its physical type, the attending practitioner, the admit time
    assert!(b.contains(r#""class":{"system":"http://terminology.hl7.org/CodeSystem/v3-ActCode","code":"IMP"}"#), "{b}");
    assert!(
        b.contains(r#""location":[{"location":{"reference":"urn:uuid:"#) && b.contains(r#""status":"active"}]"#),
        "{b}"
    );
    assert!(b.contains(r#""resourceType":"Location""#) && b.contains(r#""identifier":[{"value":"01"}],"mode":"instance","physicalType":{"coding":[{"code":"bd","system":"http://terminology.hl7.org/CodeSystem/location-physical-type"}]}"#), "{b}");
    assert!(b.contains(r#""period":{"start":"2026-09-16T10:15:00+10:00"}"#), "{b}");
    // allergies: each AL1 its own AllergyIntolerance, every reaction kept, the patient's
    assert_eq!(b.matches(r#""resourceType":"AllergyIntolerance""#).count(), 2, "{b}");
    assert!(b.contains(r#""manifestation":[{"text":"Produces hives"},{"text":"Rash"}]"#), "{b}");
    assert!(b.contains(r#""clinicalStatus":{"coding":[{"code":"active","system":"http://terminology.hl7.org/CodeSystem/allergyintolerance-clinical"}]}"#), "{b}");
    assert_eq!(b.matches(r#""patient":{"reference":"urn:uuid:"#).count(), 3, "{b}");
    // coverage: the payor an Organization, the relationship through the table (to v3 RoleCode, as the IG maps it)
    assert!(b.contains(r#""resourceType":"Coverage""#) && b.contains(r#""payor":[{"reference":"urn:uuid:"#), "{b}");
    assert!(b.contains(r#""relationship":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v3-RoleCode","code":"ONESELF"}]}"#), "{b}");
    // the Z segment is named, not silently dropped; so is a valued field no map row names
    assert_eq!(c.unplaced, vec!["ZPI[1]".to_string()]);
    eprintln!("unmapped {:?}", c.unmapped);
    assert!(c.unmapped.iter().all(|u| u.contains('[') && u.contains('-')), "{:?}", c.unmapped);
    // what FHIR requires and the message does not carry is not invented: the validation names it
    let report = wire::report(&c.validation);
    assert!(!c.validation.is_valid(), "{report}");
    let errors: Vec<&str> = c
        .validation
        .report
        .issues
        .iter()
        .filter(|i| i.severity == fhir_pods_core::validate::Severity::Error)
        .map(|i| i.message.as_str())
        .collect();
    assert_eq!(
        errors,
        vec![
            "Provenance.agent is required (min 1, found 0)",
            "Provenance.target is required (min 1, found 0)",
            "Coverage.status is required (min 1, found 0)",
        ],
        "{report}"
    );
}

/// Two orders, four results, notes on two of them, and one patient — the
/// shape the first review found collapsed (F036/F037).
const ORU_TWO_ORDERS: &str = "MSH|^~\\&|LAB|ACME|EHR|CLINIC|20260916100000+1000||ORU^R01^ORU_R01|MSG0002|P|2.5.1\r\
PID|1||MRN12345^^^ACME^MR||Doe^Jane||19700615|F\r\
ORC|RE|PLC-A1^EHR|FIL-A1^LAB\r\
OBR|1|PLC-A1^EHR|FIL-A1^LAB|24331-1^Lipid panel^LN|||20260916090000+1000||||||||||||||||||F\r\
NTE|1||Order note A1\r\
OBX|1|NM|2093-3^Cholesterol^LN||5.2|mmol/L^^UCUM|||||F|||20260916093000+1000\r\
NTE|1||Chol note\r\
NTE|2||Chol note two\r\
OBX|2|NM|2571-8^Triglycerides^LN||1.1|mmol/L^^UCUM|||||F|||20260916093000+1000\r\
ORC|RE|PLC-A2^EHR|FIL-A2^LAB\r\
OBR|2|PLC-A2^EHR|FIL-A2^LAB|58410-2^CBC panel^LN|||20260916090000+1000||||||||||||||||||F\r\
OBX|1|NM|718-7^Hemoglobin^LN||140|g/L^^UCUM|||||F|||20260916094000+1000\r\
OBX|2|ST|11475-1^Microorganism^LN||No growth||||||F\r\
NTE|1||Micro note\r";

fn entries(bundle: &str) -> Vec<serde_like::Entry> {
    serde_like::entries(bundle)
}

/// A hand-rolled reader for the bits of the bundle the tests look at, so
/// the assertions read as facts about resources rather than as substrings.
mod serde_like {
    use fhir_pods_core::json::{parse, Value};

    pub struct Entry {
        pub full_url: String,
        pub resource_type: String,
        pub json: String,
    }

    pub fn entries(bundle: &str) -> Vec<Entry> {
        let v = parse(bundle.as_bytes()).unwrap();
        let Value::Object(kvs) = &v else { panic!() };
        let entry = kvs.iter().find(|(k, _)| k.as_ref() == "entry").map(|(_, v)| v).unwrap();
        let Value::Array(items) = entry else { panic!() };
        items
            .iter()
            .map(|e| {
                let Value::Object(ekvs) = e else { panic!() };
                let full_url = match ekvs.iter().find(|(k, _)| k.as_ref() == "fullUrl") {
                    Some((_, Value::Str(s))) => s.to_string(),
                    _ => String::new(),
                };
                let res = ekvs.iter().find(|(k, _)| k.as_ref() == "resource").map(|(_, v)| v).unwrap();
                let Value::Object(rkvs) = res else { panic!() };
                let resource_type = match rkvs.iter().find(|(k, _)| k.as_ref() == "resourceType") {
                    Some((_, Value::Str(s))) => s.to_string(),
                    _ => String::new(),
                };
                Entry { full_url, resource_type, json: fhir_pods_core::node::Node::from_value(res).to_json() }
            })
            .collect()
    }
}

#[test]
fn two_orders_stay_two_reports_with_their_own_results_and_notes() {
    let v = Validator::new();
    let c = v.v2(ORU_TWO_ORDERS.as_bytes(), &Options::default()).unwrap();
    for sk in &c.skipped {
        eprintln!("skipped {} -> {}: {}", sk.at, sk.target, sk.why);
    }
    eprintln!("unplaced {:?}", c.unplaced);
    let es = entries(&c.bundle);
    let reports: Vec<&serde_like::Entry> = es.iter().filter(|e| e.resource_type == "DiagnosticReport").collect();
    let obs: Vec<&serde_like::Entry> = es.iter().filter(|e| e.resource_type == "Observation").collect();
    assert_eq!(reports.len(), 2, "{}", c.bundle);
    assert_eq!(obs.len(), 4, "{}", c.bundle);
    // each report's results are its own group's observations, in order
    let ids: Vec<&str> = obs.iter().map(|e| e.full_url.as_str()).collect();
    assert!(
        reports[0].json.contains(r#""value":"PLC-A1""#) && reports[0].json.contains("Lipid panel"),
        "{}",
        reports[0].json
    );
    assert!(
        reports[0].json.contains(&format!(r#""result":[{{"reference":"{}"}},{{"reference":"{}"}}]"#, ids[0], ids[1])),
        "{}",
        reports[0].json
    );
    assert!(
        reports[1].json.contains(r#""value":"PLC-A2""#) && reports[1].json.contains("CBC panel"),
        "{}",
        reports[1].json
    );
    assert!(
        reports[1].json.contains(&format!(r#""result":[{{"reference":"{}"}},{{"reference":"{}"}}]"#, ids[2], ids[3])),
        "{}",
        reports[1].json
    );
    // every result keeps its own code and value
    assert!(obs[0].json.contains("2093-3") && obs[0].json.contains(r#""value":5.2"#), "{}", obs[0].json);
    assert!(obs[1].json.contains("2571-8") && obs[1].json.contains(r#""value":1.1"#), "{}", obs[1].json);
    assert!(obs[2].json.contains("718-7") && obs[2].json.contains(r#""value":140"#), "{}", obs[2].json);
    assert!(obs[3].json.contains("11475-1") && obs[3].json.contains(r#""valueString":"No growth""#), "{}", obs[3].json);
    // notes sit on the observation they follow; two NTEs are two notes; the order's note is named, not invented
    assert!(obs[0].json.contains(r#""note":[{"text":"Chol note"},{"text":"Chol note two"}]"#), "{}", obs[0].json);
    assert!(!obs[1].json.contains("note"), "{}", obs[1].json);
    assert!(obs[3].json.contains(r#""note":[{"text":"Micro note"}]"#), "{}", obs[3].json);
    assert!(c.skipped.iter().any(|s| s.at == "NTE[1]" && s.why.contains("order's notes")), "{:?}", c.skipped);
    assert!(c.unplaced.is_empty(), "{:?}", c.unplaced);
    assert!(c.validation.is_valid(), "{}", wire::report(&c.validation));
}

#[test]
fn two_patients_keep_their_own_results() {
    let v = Validator::new();
    let msg = format!(
        "{}PID|2||MRN99999^^^ACME^MR||Roe^Richard||19800101|M\r\
ORC|RE|PLC-B1^EHR|FIL-B1^LAB\r\
OBR|1|PLC-B1^EHR|FIL-B1^LAB|24331-1^Lipid panel^LN|||20260916090000+1000||||||||||||||||||F\r\
OBX|1|NM|2093-3^Cholesterol^LN||6.9|mmol/L^^UCUM|||||F|||20260916093000+1000\r",
        ORU_TWO_ORDERS
    );
    let c = v.v2(msg.as_bytes(), &Options::default()).unwrap();
    let es = entries(&c.bundle);
    let patients: Vec<&serde_like::Entry> = es.iter().filter(|e| e.resource_type == "Patient").collect();
    assert_eq!(patients.len(), 2, "{}", c.bundle);
    let (jane, richard) = (&patients[0].full_url, &patients[1].full_url);
    let obs: Vec<&serde_like::Entry> = es.iter().filter(|e| e.resource_type == "Observation").collect();
    assert_eq!(obs.len(), 5);
    for o in &obs[..4] {
        assert!(o.json.contains(&format!(r#""subject":{{"reference":"{jane}"}}"#)), "{}", o.json);
    }
    assert!(
        obs[4].json.contains(&format!(r#""subject":{{"reference":"{richard}"}}"#))
            && obs[4].json.contains(r#""value":6.9"#),
        "{}",
        obs[4].json
    );
    let reports: Vec<&serde_like::Entry> = es.iter().filter(|e| e.resource_type == "DiagnosticReport").collect();
    assert_eq!(reports.len(), 3);
    assert!(reports[2].json.contains(&format!(r#""subject":{{"reference":"{richard}"}}"#)), "{}", reports[2].json);
    assert!(c.validation.is_valid(), "{}", wire::report(&c.validation));
}

#[test]
fn typed_values_follow_obx_2_and_repeating_values_become_components() {
    let v = Validator::new();
    let base = ORU.split("OBX|1|").next().unwrap();
    let msg = format!(
        "{base}OBX|1|CWE|32624-9^Blood group^LN||112144000^Blood group A^SCT||||||F\r\
OBX|2|TS|11778-8^Delivery date^LN||20260901||||||F\r\
OBX|3|ST|11475-1^Microorganism^LN||first~second||||||F\r\
OBX|4|DT|21112-8^Birth date^LN||20260901||||||F\r"
    );
    let c = v.v2(msg.as_bytes(), &Options::default()).unwrap();
    for sk in &c.skipped {
        eprintln!("skipped {} -> {}: {}", sk.at, sk.target, sk.why);
    }
    eprintln!("unplaced {:?}", c.unplaced);
    let es = entries(&c.bundle);
    let obs: Vec<&serde_like::Entry> = es.iter().filter(|e| e.resource_type == "Observation").collect();
    assert_eq!(obs.len(), 4, "{}", c.bundle);
    assert!(obs[0].json.contains(r#""valueCodeableConcept":{"coding":[{"code":"112144000","display":"Blood group A","system":"http://snomed.info/sct"}]}"#), "{}", obs[0].json);
    assert!(obs[1].json.contains(r#""valueDateTime":"2026-09-01""#), "{}", obs[1].json);
    assert!(
        obs[2].json.contains(r#""component":[{"code":{"coding":[{"code":"11475-1""#)
            && obs[2].json.contains(r#""valueString":"first"}"#)
            && obs[2].json.contains(r#""valueString":"second"}"#)
            && !obs[2].json.contains(r#""valueString":"first","#),
        "{}",
        obs[2].json
    );
    assert!(obs[3].json.contains(r#""valueDateTime":"2026-09-01""#), "{}", obs[3].json);
    assert!(c.validation.is_valid(), "{}", wire::report(&c.validation));
}

#[test]
fn names_stay_whole_and_a_birth_time_keeps_the_birth_date() {
    let v = Validator::new();
    let msg = ORU.replace("|19700615|F", "|19700615103000+1000|F");
    let c = v.v2(msg.as_bytes(), &Options::default()).unwrap();
    let es = entries(&c.bundle);
    let pract = es.iter().find(|e| e.resource_type == "Practitioner").unwrap();
    assert!(pract.json.contains(r#""name":[{"family":"Who","given":["Doctor"],"prefix":["Dr"]}]"#), "{}", pract.json);
    let patient = es.iter().find(|e| e.resource_type == "Patient").unwrap();
    assert!(patient.json.contains(r#""birthDate":"1970-06-15","_birthDate":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/patient-birthTime","valueDateTime":"1970-06-15T10:30:00+10:00"}]}"#), "{}", patient.json);
    assert!(c.validation.is_valid(), "{}", wire::report(&c.validation));
}

#[test]
fn different_messages_get_different_ids_and_the_same_message_the_same() {
    let v = Validator::new();
    let a = v.v2(ORU.as_bytes(), &Options::default()).unwrap().bundle;
    let b = v.v2(ORU.replace("MSG0001", "MSG0002").as_bytes(), &Options::default()).unwrap().bundle;
    let again = v.v2(ORU.as_bytes(), &Options::default()).unwrap().bundle;
    assert_eq!(a, again);
    let id = |s: &str| s.split(r#""id":""#).nth(1).unwrap().split('"').next().unwrap().to_string();
    assert_ne!(id(&a), id(&b));
    let urls = |s: &str| s.matches("urn:uuid:").count();
    assert!(urls(&a) > 5 && !b.contains(a.split("urn:uuid:").nth(1).unwrap().split('"').next().unwrap()), "{a}\n{b}");
}

#[test]
fn a_cross_segment_condition_reads_the_segment_it_names() {
    // an order: ORC-2 is the placer number, and OBR-2 is empty — the IG puts the
    // identifier on the ServiceRequest from ORC when OBR-2 is not valued
    let v = Validator::new();
    let msg = "MSH|^~\\&|EHR|CLINIC|LAB|ACME|20260916120000+1000||OML^O21^OML_O21|MSG0003|P|2.5.1\r\
PID|1||MRN12345^^^ACME^MR||Doe^Jane^A||19700615|F\r\
ORC|NW|PLC003^EHR|||||||20260916115500+1000|1234^Who^Doctor\r\
OBR|1|||24331-1^Lipid panel^LN\r";
    let c = v.v2(msg.as_bytes(), &Options::default()).unwrap();
    for sk in &c.skipped {
        eprintln!("skipped {} -> {}: {}", sk.at, sk.target, sk.why);
    }
    let es = entries(&c.bundle);
    let sr = es.iter().find(|e| e.resource_type == "ServiceRequest").unwrap();
    assert!(sr.json.contains(r#""identifier":[{"value":"PLC003","type":{"coding":[{"code":"PLAC","system":"http://terminology.hl7.org/CodeSystem/v2-0203"}]}}]"#), "{}", sr.json);
    assert!(sr.json.contains(r#""authoredOn":"2026-09-16T11:55:00+10:00""#), "{}", sr.json);
}

#[test]
fn hostile_bytes_are_errors_or_skips_never_panics() {
    let v = Validator::new();
    // a multibyte character inside an escape, and inside a zone
    let esc = "MSH|^~\\&|A|B|C|D|20260101||ADT^A01|1|P|2.5\rPID|1||1||Sm\\X€1\\ith\r";
    assert!(v.v2(esc.as_bytes(), &Options::default()).is_ok());
    let zone = ORU.replace("20260916093000+1000", "20260916093000+1é0");
    let c = v.v2(zone.as_bytes(), &Options::default()).unwrap();
    assert!(c.skipped.iter().any(|s| s.why.contains("malformed zone")), "{:?}", c.skipped);
    // multibyte encoding characters
    let enc =
        "MSH|αβγδ|LAB|ACME|EHR|CLINIC|20260916100000+1000||ORUαR01αORU_R01|MSG0001|P|2.5.1\rPID|1||MRN1||DoeαJane\r";
    let c = v.v2(enc.as_bytes(), &Options::default()).unwrap();
    assert!(c.bundle.contains(r#""endpoint":"LAB""#), "{}", c.bundle);
    // options are validated at the boundary
    for bad in [
        r#"{"timezone":"+1000"}"#,
        r#"{"timezone":"AEST"}"#,
        r#"{"timezone":""}"#,
        r#"{"timezone":"+15:00"}"#,
        r#"{"codeSystems":{"L":""}}"#,
    ] {
        assert_eq!(Options::from_json(bad.as_bytes()).unwrap_err(), ConvertError::BadOptions, "{bad}");
    }
    assert!(Options::from_json(br#"{"timezone":"Z"}"#).is_ok());
}

#[test]
fn a_trigger_event_finds_its_structure() {
    let v = Validator::new();
    let msg = ADT.replace("ADT^A01^ADT_A01", "ADT^A04");
    let c = v.v2(msg.as_bytes(), &Options::default()).unwrap();
    assert!(c.bundle.contains(r#""eventCoding":{"code":"A04""#), "{}", c.bundle);
    assert!(c.bundle.contains(r#""resourceType":"Encounter""#));
    let msg = ADT.replace("ADT^A01^ADT_A01", "ADT^A99");
    assert_eq!(
        v.v2(msg.as_bytes(), &Options::default()).unwrap_err(),
        ConvertError::UnknownStructure("ADT_A99".into())
    );
}
