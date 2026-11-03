//! `resolve()` on a reference nothing resolves, and the host's resource
//! store (`Validator::load_resource`). Every expectation here was probed
//! against validator_cli 6.10.4 offline (`-tx n/a`; `conformance/probes/`
//! `resolve-*`): an unresolvable reference is the empty collection, and
//! the invariant evaluates on it.
mod common;

use fhir_pods_core::document::{Document, Parsed};
use fhir_pods_core::path::{check_in, parse, Verdict};
use fhir_pods_core::{wire, Validator};

const HR: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";

fn team(member: &str, contained: &str) -> String {
    format!(
        r#"{{"resourceType":"CareTeam","id":"t1",{contained}"participant":[{{"member":{{"reference":"{member}"}},"onBehalfOf":{{"display":"An organisation"}}}}]}}"#
    )
}

fn has(v: &fhir_pods_core::Validation, needle: &str) -> bool {
    v.report.issues.iter().any(|i| i.message.contains(needle))
}

/// The probed shapes: a relative `Type/id`, an absolute URL, a `urn:uuid:`,
/// a `/_history/` version and a `#local` naming no contained resource —
/// alone, and inside a Bundle that does not hold the target. Each resolves
/// to nothing, and nothing is evaluated as the empty collection (rsv-1
/// `exists()` is false, rsv-2 `empty()` is true), never skipped.
#[test]
fn a_reference_nothing_resolves_is_the_empty_collection() {
    let exists = parse("subject.resolve().exists()").unwrap();
    let empty = parse("subject.resolve().empty()").unwrap();
    for reference in [
        "Patient/p1",
        "https://records.example.invalid/fhir/Patient/p1",
        "urn:uuid:5b2f3a4e-1c9d-4e7a-9b2f-0a1b2c3d4e5f",
        "Patient/p1/_history/2",
        "#missing",
    ] {
        let basic =
            format!(r#"{{"resourceType":"Basic","code":{{"text":"probe"}},"subject":{{"reference":"{reference}"}}}}"#);
        let parsed = Parsed::parse(basic.as_bytes()).unwrap();
        let doc = parsed.doc();
        assert_eq!(check_in(&exists, &doc, None, Some(&doc), None), Verdict::Fail, "{reference}");
        assert_eq!(check_in(&empty, &doc, None, Some(&doc), None), Verdict::Pass, "{reference}");
        let bundle = format!(
            r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"fullUrl":"https://records.example.invalid/fhir/Basic/b1","resource":{basic}}}]}}"#
        );
        let parsed_bundle = Parsed::parse(bundle.as_bytes()).unwrap();
        let b = parsed_bundle.doc();
        let entry = &b.instances("Bundle.entry.resource")[0];
        assert_eq!(check_in(&exists, entry, None, Some(entry), Some(&b)), Verdict::Fail, "{reference} in a Bundle");
    }
}

/// `ctm-1` — `onBehalfOf.exists() implies (member.resolve().iif(empty(),
/// true, ofType(Practitioner).exists()))` — on a member nothing resolves:
/// `iif`'s criterion is evaluated on the empty input, so it is `true`, as
/// the oracle has it (it was skipped here, and `#missing` failed it).
#[test]
fn ctm_1_evaluates_on_a_member_nothing_resolves() {
    let v = Validator::new();
    for member in [
        "Practitioner/r1",
        "https://records.example.invalid/fhir/Practitioner/r1",
        "urn:uuid:5b2f3a4e-1c9d-4e7a-9b2f-0a1b2c3d4e5f",
    ] {
        let r = v.validate(team(member, "").as_bytes());
        assert!(r.is_valid(), "{member}: {}", wire::report(&r));
        assert!(!has(&r, "ctm-1"), "{member}: evaluated, not skipped: {}", wire::report(&r));
    }
    // `#missing` is ref-1's error, not ctm-1's
    let r = v.validate(team("#missing", "").as_bytes());
    assert!(has(&r, "ref-1") && !has(&r, "ctm-1"), "{}", wire::report(&r));
    // what resolves is judged: a contained Patient fails it, a Practitioner passes
    let patient = v.validate(team("#p1", r#""contained":[{"resourceType":"Patient","id":"p1"}],"#).as_bytes());
    assert!(has(&patient, "ctm-1: "), "{}", wire::report(&patient));
    let practitioner =
        v.validate(team("#r1", r#""contained":[{"resourceType":"Practitioner","id":"r1"}],"#).as_bytes());
    assert!(practitioner.is_valid() && !has(&practitioner, "ctm-1"), "{}", wire::report(&practitioner));
}

/// A stored resource is found by `resolve()` by the rules a Bundle entry is:
/// `Type/id` against the resource's type and id, a `fullUrl` (exactly, or a
/// relative reference against one ending so), `/_history/version` selecting
/// `meta.versionId`.
#[test]
fn resolve_finds_a_stored_resource_by_a_bundle_entrys_rules() {
    let fails = |v: &Validator, member: &str| has(&v.validate(team(member, "").as_bytes()), "ctm-1: ");
    let mut v = Validator::new();
    assert!(!fails(&v, "Patient/p1"));
    assert!(v.load_resource(br#"{"resourceType":"Patient","id":"p1"}"#));
    assert!(fails(&v, "Patient/p1"), "by type and id");
    assert!(!fails(&v, "https://records.example.invalid/fhir/Patient/p1"), "an absolute URL needs a fullUrl");
    assert!(v.load_resource(
        br#"{"resourceType":"Bundle","type":"collection","entry":[
            {"fullUrl":"https://records.example.invalid/fhir/Patient/p2","resource":{"resourceType":"Patient","id":"p2"}},
            {"fullUrl":"urn:uuid:5b2f3a4e-1c9d-4e7a-9b2f-0a1b2c3d4e5f","resource":{"resourceType":"Patient"}}]}"#
    ));
    assert_eq!(v.resource_count(), 3);
    assert!(fails(&v, "https://records.example.invalid/fhir/Patient/p2"), "by fullUrl");
    assert!(fails(&v, "Patient/p2"), "relative, against a fullUrl ending so");
    assert!(fails(&v, "urn:uuid:5b2f3a4e-1c9d-4e7a-9b2f-0a1b2c3d4e5f"), "a urn:uuid fullUrl");
    // a version selects meta.versionId; two versions are two entries
    assert!(!fails(&v, "Patient/p3/_history/2"));
    assert!(v.load_resource(br#"{"resourceType":"Patient","id":"p3","meta":{"versionId":"2"}}"#));
    assert!(fails(&v, "Patient/p3/_history/2"));
    assert!(!fails(&v, "Patient/p3/_history/1"));
    // a local reference never reaches the store
    let local = v.validate(team("#p1", "").as_bytes());
    assert!(has(&local, "ref-1") && !has(&local, "ctm-1"), "{}", wire::report(&local));
    // a second load of the same Type/id replaces it; an empty store resolves nothing
    assert!(v.load_resource(br#"{"resourceType":"Practitioner","id":"p1"}"#));
    assert!(fails(&v, "Patient/p1"), "another type is another identity");
    assert!(v.load_resource(br#"{"resourceType":"Patient","id":"p1","gender":"other"}"#));
    assert_eq!(v.resource_count(), 5);
    v.clear_resources();
    assert!(!fails(&v, "Patient/p1"));
}

/// Precedence: contained, then the document Bundle, then the store.
#[test]
fn a_bundle_entry_comes_before_the_store() {
    let v = Validator::new().with_resource(
        br#"{"resourceType":"Bundle","entry":[{"fullUrl":"https://records.example.invalid/fhir/Practitioner/r1","resource":{"resourceType":"Patient","id":"r1"}}]}"#,
    );
    let alone = v.validate(team("Practitioner/r1", "").as_bytes());
    assert!(has(&alone, "ctm-1: "), "alone, the store's Patient answers: {}", wire::report(&alone));
    let document = format!(
        r#"{{"resourceType":"Bundle","type":"collection","entry":[
            {{"fullUrl":"https://records.example.invalid/fhir/CareTeam/t1","resource":{}}},
            {{"fullUrl":"https://records.example.invalid/fhir/Practitioner/r1","resource":{{"resourceType":"Practitioner","id":"r1"}}}}]}}"#,
        team("Practitioner/r1", "")
    );
    let r = v.validate(document.as_bytes());
    assert!(!has(&r, "ctm-1"), "the entry answers first: {}", wire::report(&r));
    let contained = v.validate(team("#r1", r#""contained":[{"resourceType":"Practitioner","id":"r1"}],"#).as_bytes());
    assert!(!has(&contained, "ctm-1"), "{}", wire::report(&contained));
}

/// A stored target is held to the element's `targetProfile`s by
/// conformance, as a Bundle entry is: AU Core heart rate's `subject` is an
/// AU Core Patient.
#[test]
fn a_stored_target_is_held_to_the_target_profile() {
    let hr = common::HEARTRATE_OK;
    let mut v = Validator::new();
    assert!(v.validate(hr.as_bytes()).is_valid(), "nothing resolves: not judged");
    assert!(v.load_resource(br#"{"resourceType":"Patient","id":"1"}"#));
    let r = v.validate(hr.as_bytes());
    assert!(!r.is_valid() && has(&r, "Unable to find a profile match for Patient/1"), "{}", wire::report(&r));
    let conforming =
        common::PATIENT_OK.replacen(r#""resourceType":"Patient","#, r#""resourceType":"Patient","id":"1","#, 1);
    assert!(v.load_resource(conforming.as_bytes()));
    let r = v.validate(hr.as_bytes());
    assert!(r.is_valid(), "{}", wire::report(&r));
    assert!(v.load_resource(br#"{"resourceType":"Organization","id":"1"}"#));
    assert!(v.validate(hr.as_bytes()).is_valid(), "Patient/1 is still the Patient");
}

/// A contained target is held to the element's `targetProfile`s by
/// conformance, as a Bundle entry and a stored resource are, and so is one
/// a contained resource names (its sibling): probed against validator_cli
/// 6.10.4 (`conformance/probes/contained-target-*`).
#[test]
fn a_contained_target_is_held_to_the_target_profile() {
    let bare = r#"{"resourceType":"Patient","id":"p1"}"#;
    let conforming = common::PATIENT_OK
        .replacen(r#""resourceType":"Patient","#, r#""resourceType":"Patient","id":"p1","#, 1)
        .replacen(r#""meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"#, "", 1);
    let contained = |patient: &str| {
        common::HEARTRATE_OK.replacen(
            r#""subject":{"reference":"Patient/1"}"#,
            &format!(r##""subject":{{"reference":"#p1"}},"contained":[{patient}]"##),
            1,
        )
    };
    let v = Validator::new();
    let r = v.validate(contained(bare).as_bytes());
    assert!(!r.is_valid() && has(&r, "Unable to find a profile match for #p1"), "{}", wire::report(&r));
    let r = v.validate(contained(&conforming).as_bytes());
    assert!(r.is_valid(), "{}", wire::report(&r));
    // a contained heart rate naming its sibling: held the same
    let nested = |patient: &str| {
        let inner = common::HEARTRATE_OK
            .replacen(r#""resourceType":"Observation","#, r#""resourceType":"Observation","id":"o1","#, 1)
            .replacen(r#""reference":"Patient/1""#, r##""reference":"#p1""##, 1);
        format!(
            r##"{{"resourceType":"List","status":"current","mode":"working","contained":[{inner},{patient}],"entry":[{{"item":{{"reference":"#o1"}}}}]}}"##
        )
    };
    let r = v.validate(nested(bare).as_bytes());
    assert!(!r.is_valid() && has(&r, "Unable to find a profile match for #p1"), "{}", wire::report(&r));
    let r = v.validate(nested(&conforming).as_bytes());
    assert!(r.is_valid(), "{}", wire::report(&r));
    // precedence unchanged: a `#local` reference never reaches the store
    let stored = Validator::new().with_resource(conforming.as_bytes());
    assert!(!stored.validate(contained(bare).as_bytes()).is_valid());
}

/// A contained target that names its own sibling (`#sp`, `#p9`) is
/// judged as contained in the resource holding both: its `#local`
/// references resolve there, for its shape (ref-1) as for its profile. And
/// an unprofiled resource's `#local` references are held to its base
/// target types. Each probed against validator_cli 6.10.4
/// (`conformance/probes/contained-target-*`).
#[test]
fn a_contained_target_names_its_siblings_in_the_container() {
    let probe = |name: &str| {
        let path = format!("{}/../../conformance/probes/{name}.json", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(path).expect("probe fixture")
    };
    let v = Validator::new();
    for (name, valid, needle) in [
        ("contained-target-us-diagnosticreport-sibling-specimen", true, ""),
        ("contained-target-us-diagnosticreport-sibling-conforming-patient", true, ""),
        ("contained-target-us-diagnosticreport-sibling-bare-patient", false, "Unable to find a profile match for #o1"),
        ("contained-target-us-encounter-condition-sibling-conforming-patient", true, ""),
        (
            "contained-target-us-encounter-condition-sibling-bare-patient",
            false,
            "Unable to find a profile match for #c1",
        ),
        ("contained-target-unprofiled-observation-medication", false, "Invalid Resource target type. Found Medication"),
        ("contained-target-unprofiled-encounter-location", false, "Invalid Resource target type. Found Location"),
        ("contained-target-unprofiled-encounter-bare-patient", true, ""),
    ] {
        let r = v.validate(&probe(name));
        assert_eq!(r.is_valid(), valid, "{name}: {}", wire::report(&r));
        assert!(needle.is_empty() || has(&r, needle), "{name}: {}", wire::report(&r));
    }
}

/// Lower and lift validate what they build and read without the store.
#[test]
fn lower_and_lift_do_not_read_the_store() {
    let v = Validator::new().with_resource(br#"{"resourceType":"Patient","id":"1"}"#);
    assert!(!v.validate(common::HEARTRATE_OK.as_bytes()).is_valid());
    let lifted = v.lift(HR, common::HEARTRATE_OK.as_bytes()).unwrap();
    assert!(lifted.validation.is_valid(), "{}", wire::report(&lifted.validation));
    let inputs = r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-16T08:00:00+10:00","valueQuantity.value":72,"valueQuantity.unit":"beats/minute"}"#;
    let lowered = v.lower(HR, inputs.as_bytes()).unwrap();
    assert!(lowered.validation.is_valid(), "{}", wire::report(&lowered.validation));
}

/// What is not a resource is refused and leaves the store as it was.
#[test]
fn a_refused_load_leaves_the_store_unchanged() {
    let mut v = Validator::new();
    assert!(v.load_resource(br#"{"resourceType":"Patient","id":"p1"}"#));
    for bad in [
        &b"{"[..],
        br#"{"resourceType":"Nope","id":"x"}"#,
        br#"{"resourceType":"Bundle","entry":[{"resource":{"id":"x"}}]}"#,
    ] {
        assert!(!v.load_resource(bad));
    }
    assert_eq!(v.resource_count(), 1);
}

/// Two stored entries under different `fullUrl`s are two entries, though
/// both are `Patient/1`: each absolute reference finds its own, in either
/// load order, as the same two entries of a document Bundle are found.
#[test]
fn entries_under_two_full_urls_are_two_entries() {
    let conforming =
        common::PATIENT_OK.replacen(r#""resourceType":"Patient","#, r#""resourceType":"Patient","id":"1","#, 1);
    let bare = r#"{"resourceType":"Patient","id":"1"}"#;
    let a = "https://a.example.invalid/fhir/Patient/1";
    let b = "https://b.example.invalid/fhir/Patient/1";
    let hr = |subject: &str| common::HEARTRATE_OK.replace("\"Patient/1\"", &format!("\"{subject}\""));
    let pair = |first: (&str, &str), second: (&str, &str)| {
        format!(
            r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"fullUrl":"{}","resource":{}}},{{"fullUrl":"{}","resource":{}}}]}}"#,
            first.0, first.1, second.0, second.1
        )
    };
    for bundle in [pair((a, bare), (b, &conforming)), pair((b, &conforming), (a, bare))] {
        let v = Validator::new().with_resource(bundle.as_bytes());
        assert_eq!(v.resource_count(), 2, "{bundle}");
        let to_a = v.validate(hr(a).as_bytes());
        assert!(has(&to_a, &format!("Unable to find a profile match for {a}")), "{}", wire::report(&to_a));
        let to_b = v.validate(hr(b).as_bytes());
        assert!(to_b.is_valid(), "{}", wire::report(&to_b));
    }
}

/// An unprofiled resource validated alone is held to its elements' base
/// target types against what the host stored, as a Bundle entry is
/// ("Invalid Resource target type", probed in a Bundle: `bn-obs-med`).
#[test]
fn a_stored_target_is_held_to_the_base_target_types() {
    let obs = r#"{"resourceType":"Observation","id":"o1","status":"final","code":{"text":"x"},"subject":{"reference":"Medication/m1"}}"#;
    let mut v = Validator::new();
    assert!(!has(&v.validate(obs.as_bytes()), "Invalid Resource target type"));
    assert!(v.load_resource(br#"{"resourceType":"Medication","id":"m1"}"#));
    let r = v.validate(obs.as_bytes());
    assert!(
        has(
            &r,
            "Invalid Resource target type. Found Medication, but expected one of ([Device, Group, Location, Patient])"
        ),
        "{}",
        wire::report(&r)
    );
    // inside a Bundle, an entry the Bundle does not hold is looked up in the store too
    let bundle = format!(
        r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"fullUrl":"https://s.example.invalid/fhir/Observation/o1","resource":{obs}}}]}}"#
    );
    let r = v.validate(bundle.as_bytes());
    assert!(has(&r, "Invalid Resource target type. Found Medication"), "{}", wire::report(&r));
    // and the Bundle's own entry comes first
    let held = format!(
        r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"fullUrl":"https://s.example.invalid/fhir/Observation/o1","resource":{obs}}},{{"fullUrl":"https://s.example.invalid/fhir/Medication/m1","resource":{{"resourceType":"Patient","id":"m1"}}}}]}}"#
    );
    assert!(!has(&v.validate(held.as_bytes()), "Invalid Resource target type"));
}

/// A stored resource's own references reach the store too: its conformance
/// is judged with what it names in reach.
#[test]
fn what_a_stored_resource_names_is_in_reach() {
    let mut v = Validator::new();
    assert!(!v.load_resource(br#"{"resourceType":"Bundle","entry":"notarray"}"#));
    assert_eq!(v.resource_count(), 0);
    assert!(v.load_resource(br#"{"resourceType":"Practitioner","id":"r1"}"#));
    assert!(v.load_resource(
        br#"{"resourceType":"Patient","id":"p1","generalPractitioner":[{"reference":"Medication/m1"}]}"#
    ));
    assert!(v.load_resource(br#"{"resourceType":"Medication","id":"m1"}"#));
    let r = v.validate(team("Patient/p1", "").as_bytes());
    assert!(has(&r, "ctm-1: "), "{}", wire::report(&r));
}
