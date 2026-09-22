//! Discovery without the specification open: what is compiled in, and what
//! a slot wants — its fixed neighbours and, for a composite datatype, its
//! parts.
mod common;

use fhir_pods_core::json::{self, Value};
use fhir_pods_core::{catalogue_json, resource_profile, wire, Validator};

fn member<'a>(v: &'a Value<'a>, key: &str) -> Option<&'a Value<'a>> {
    match v {
        Value::Object(kvs) => kvs.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v),
        _ => None,
    }
}

fn text<'a>(v: &'a Value<'a>) -> &'a str {
    match v {
        Value::Str(s) => s.as_ref(),
        _ => panic!("not a string: {v:?}"),
    }
}

#[test]
fn the_catalogue_lists_every_region_and_profile_with_its_kind() {
    let json = catalogue_json();
    let v = json::parse(json.as_bytes()).expect("valid JSON");
    let Some(Value::Array(regions)) = member(&v, "regions") else { panic!() };
    assert_eq!(regions.iter().map(|r| text(member(r, "name").unwrap())).collect::<Vec<_>>(), ["au", "us", "ips"]);
    assert_eq!(text(member(&regions[0], "document").unwrap()), "au-ps");
    let Some(Value::Array(profiles)) = member(&v, "profiles") else { panic!() };
    assert_eq!(profiles.len(), fhir_pods_core::REGISTRY.len());
    let find = |url: &str| profiles.iter().find(|p| text(member(p, "url").unwrap()) == url).unwrap();
    let bp = find("http://hl7.org.au/fhir/core/StructureDefinition/au-core-bloodpressure");
    assert_eq!(text(member(bp, "kind").unwrap()), "resource");
    assert_eq!(text(member(bp, "resourceType").unwrap()), "Observation");
    assert_eq!(text(member(bp, "base").unwrap()), "http://hl7.org/fhir/StructureDefinition/bp");
    let ext = find("http://hl7.org/fhir/StructureDefinition/patient-birthPlace");
    assert_eq!(text(member(ext, "kind").unwrap()), "extension");
    let ihi = find("http://hl7.org.au/fhir/StructureDefinition/au-ihi");
    assert_eq!(text(member(ihi, "kind").unwrap()), "datatype");
    // the base resources a host lowers into before export are there to find
    let base = find("http://hl7.org/fhir/StructureDefinition/Patient");
    assert_eq!(text(member(base, "kind").unwrap()), "resource");
    assert_eq!(text(member(base, "base").unwrap()), "http://hl7.org/fhir/StructureDefinition/DomainResource");
}

#[test]
fn a_slot_shows_what_the_profile_fixes_beside_it_and_the_parts_of_a_composite() {
    let bp = resource_profile("http://hl7.org.au/fhir/core/StructureDefinition/au-core-bloodpressure").unwrap();
    let json = bp.slots_json();
    let v = json::parse(json.as_bytes()).expect("valid JSON");
    let Some(Value::Array(slots)) = member(&v, "slots") else { panic!() };
    let slot = |path: &str| slots.iter().find(|s| text(member(s, "path").unwrap()) == path).unwrap();
    // the unit slot shows the UCUM code the profile fixes next to it, and is optional: lowering writes the code as the unit
    let unit = slot("component:SystolicBP.valueQuantity.unit");
    let Some(Value::Array(fixed)) = member(unit, "fixed") else { panic!() };
    let pairs: Vec<(&str, &str)> =
        fixed.iter().map(|f| (text(member(f, "path").unwrap()), text(member(f, "value").unwrap()))).collect();
    assert!(pairs.contains(&("component:SystolicBP.valueQuantity.code", "mm[Hg]")), "{pairs:?}");
    assert!(pairs.contains(&("component:SystolicBP.valueQuantity.system", "http://unitsofmeasure.org")), "{pairs:?}");
    assert!(matches!(member(unit, "min"), Some(Value::Number("0"))));
    assert!(matches!(member(unit, "parts"), Some(Value::Array(p)) if p.is_empty()), "a string has no parts");
    // a Reference slot lists what a Reference is made of
    let subject = slot("subject");
    let Some(Value::Array(parts)) = member(subject, "parts") else { panic!() };
    let names: Vec<&str> = parts.iter().map(|p| text(member(p, "name").unwrap())).collect();
    assert_eq!(names, ["display", "identifier", "reference", "type"]);
    assert!(matches!(member(subject, "fixed"), Some(Value::Array(f)) if f.is_empty()));

    // a Patient's name: the HumanName's members, with which repeat
    let patient = resource_profile("http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient").unwrap();
    let json = patient.slots_json();
    let v = json::parse(json.as_bytes()).unwrap();
    let Some(Value::Array(slots)) = member(&v, "slots") else { panic!() };
    let name = slots.iter().find(|s| text(member(s, "path").unwrap()) == "name").unwrap();
    let Some(Value::Array(parts)) = member(name, "parts") else { panic!() };
    let given = parts.iter().find(|p| text(member(p, "name").unwrap()) == "given").unwrap();
    assert_eq!(text(member(given, "type").unwrap()), "string");
    assert!(matches!(member(given, "repeats"), Some(Value::Bool(true))));
    let family = parts.iter().find(|p| text(member(p, "name").unwrap()) == "family").unwrap();
    assert!(matches!(member(family, "repeats"), Some(Value::Bool(false))));
    // an IHI identifier slice: the fixed system it carries is shown beneath it
    let ihi = slots.iter().find(|s| text(member(s, "path").unwrap()) == "identifier:ihi").unwrap();
    let Some(Value::Array(fixed)) = member(ihi, "fixed") else { panic!() };
    assert!(
        fixed.iter().any(|f| text(member(f, "value").unwrap()) == "http://ns.electronichealth.net.au/id/hi/ihi/1.0"),
        "{}",
        patient.slots_json()
    );
}

#[test]
fn a_quantity_without_a_unit_takes_the_profiles_fixed_code_as_its_unit() {
    let v = Validator::new();
    let url = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-bloodpressure";
    let no_units = br#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-20T08:00:00+10:00",
        "component:SystolicBP.valueQuantity.value":121,"component:DiastolicBP.valueQuantity.value":79}"#;
    let l = v.lower(url, no_units).unwrap();
    assert!(l.validation.is_valid(), "{}", wire::report(&l.validation));
    assert_eq!(l.resource.matches(r#""unit":"mm[Hg]""#).count(), 2, "{}", l.resource);
    // a host's own unit is kept
    let with = br#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-20T08:00:00+10:00",
        "component:SystolicBP.valueQuantity.value":121,"component:SystolicBP.valueQuantity.unit":"mmHg",
        "component:DiastolicBP.valueQuantity.value":79,"component:DiastolicBP.valueQuantity.unit":"mmHg"}"#;
    let l = v.lower(url, with).unwrap();
    assert_eq!(l.resource.matches(r#""unit":"mmHg""#).count(), 2);
    assert!(!l.resource.contains(r#""unit":"mm[Hg]""#));
    // a component the host leaves out entirely gets no quantity and no unit
    let one = br#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-20T08:00:00+10:00",
        "component:SystolicBP.valueQuantity.value":121,
        "component:DiastolicBP.dataAbsentReason":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/data-absent-reason","code":"unknown"}]}}"#;
    let l = v.lower(url, one).unwrap();
    assert_eq!(l.resource.matches(r#""unit":"mm[Hg]""#).count(), 1, "{}", l.resource);
    // and the round trip holds: lifted, the unit is an input; lowered again, the same resource
    let lifted = v.lift(url, l.resource.as_bytes()).unwrap();
    assert!(lifted.missing.is_empty(), "{:?}", lifted.missing);
    assert_eq!(v.lower(url, lifted.inputs.as_bytes()).unwrap().resource, l.resource);
}
