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
    assert_eq!(Message::parse(b"MSH|^~\\&|A\rXY").unwrap_err(), ParseError::BadSegment(2));
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
    // the Z segment is named, not silently dropped
    assert_eq!(c.unplaced, vec!["ZPI[1]".to_string()]);
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
