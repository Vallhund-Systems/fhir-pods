mod common;

use fhir_pods_core::audit::{base64, network_type, AuditError, Kind};
use fhir_pods_core::{wire, Validator};

const READ: &str = r#"{
  "recorded": "2026-09-16T10:00:00Z",
  "client": {"who": "Device/app-7", "network": "10.0.0.7"},
  "server": {"who": {"display": "fhir.example.org"}, "network": "https://fhir.example.org/r4"},
  "user": {"who": "Practitioner/gp1", "name": "Dr Who"},
  "source": {"observer": "Device/audit-collector", "site": "clinic-1"},
  "data": "Observation/hr1",
  "transaction": "3f2a1c9e-7b1d-4f0e-9a8c-1d2e3f4a5b6c"
}"#;

fn inputs(kind: Kind) -> String {
    let mut s = READ.to_string();
    if kind.has_patient() {
        s = s.replace(r#""data": "Observation/hr1","#, r#""data": "Observation/hr1", "patient": "Patient/p1","#);
    }
    if kind.is_query() {
        s = s.replace(r#""data": "Observation/hr1","#, r#""query": "GET /r4/Observation?patient=p1&_count=10","#);
    }
    s
}

#[test]
fn every_kind_builds_a_conformant_balp_audit_event_from_host_shaped_inputs() {
    let v = Validator::new();
    for kind in Kind::ALL {
        let a = v.audit(kind.name(), inputs(kind).as_bytes()).unwrap_or_else(|e| panic!("{}: {e}", kind.name()));
        assert!(a.validation.is_valid(), "{}: {}", kind.name(), wire::report(&a.validation));
        let r = &a.resource;
        assert!(r.contains(&format!(r#""meta":{{"profile":["{}"]}}"#, kind.url())), "{}: {r}", kind.name());
        // the profile supplies the event: type rest, the action, the outcome
        assert!(
            r.contains(r#""system":"http://terminology.hl7.org/CodeSystem/audit-event-type","code":"rest""#),
            "{r}"
        );
        assert!(r.contains(r#""outcome":"0""#), "{r}");
        let action = match kind {
            Kind::Read | Kind::PatientRead => "R",
            Kind::Query | Kind::PatientQuery => "E",
            Kind::Create | Kind::PatientCreate => "C",
            Kind::Update | Kind::PatientUpdate => "U",
            Kind::Delete | Kind::PatientDelete => "D",
        };
        assert!(r.contains(&format!(r#""action":"{action}""#)), "{}: {r}", kind.name());
        // the three agents with the profile's types, the host's who and network
        assert!(r.contains(r#""reference":"Device/app-7""#) && r.contains(r#""display":"fhir.example.org""#), "{r}");
        assert!(r.contains(r#""network":{"address":"10.0.0.7","type":"2"}"#), "{r}");
        assert!(r.contains(r#""network":{"address":"https://fhir.example.org/r4","type":"5"}"#), "{r}");
        assert!(r.contains(r#""reference":"Practitioner/gp1"},"requestor":true,"name":"Dr Who""#), "{r}");
        assert!(r.contains(r#""source":{"observer":{"reference":"Device/audit-collector"},"site":"clinic-1"}"#), "{r}");
        // the entities
        if kind.has_patient() {
            assert!(r.contains(r#""what":{"reference":"Patient/p1"}"#), "{r}");
            assert!(r.contains(r#""system":"http://terminology.hl7.org/CodeSystem/object-role","code":"1""#), "{r}");
        } else {
            assert!(!r.contains("Patient/p1"));
        }
        if kind.is_query() {
            assert!(
                r.contains(&format!(r#""query":"{}""#, base64(b"GET /r4/Observation?patient=p1&_count=10"))),
                "{r}"
            );
            assert!(r.contains(r#""code":"24""#), "{r}");
        } else {
            assert!(r.contains(r#""what":{"reference":"Observation/hr1"}"#), "{r}");
        }
        // the correlation: the request id as BALP's transaction entity
        assert!(
            r.contains(r#""what":{"identifier":{"value":"3f2a1c9e-7b1d-4f0e-9a8c-1d2e3f4a5b6c"}}"#)
                && r.contains(r#""system":"https://profiles.ihe.net/ITI/BALP/CodeSystem/BasicAuditEntityType","code":"XrequestId""#),
            "{r}"
        );
        // the same inputs make the same record
        assert_eq!(v.audit(kind.name(), inputs(kind).as_bytes()).unwrap().resource, a.resource);
    }
}

#[test]
fn the_defaults_are_the_patterns_own_and_the_host_overrides_them() {
    let v = Validator::new();
    // a read's subtype is the RESTful read; a vread is the host's to say
    let r = v.audit("read", READ.as_bytes()).unwrap().resource;
    assert!(r.contains(r#""subtype":[{"system":"http://hl7.org/fhir/restful-interaction","code":"read""#), "{r}");
    let r =
        v.audit("read", READ.replace(r#""transaction""#, r#""subtype": "vread", "transaction""#).as_bytes()).unwrap();
    assert!(r.validation.is_valid(), "{}", wire::report(&r.validation));
    assert!(r.resource.contains(r#""code":"vread""#));
    // an update's user is its author and its data a Domain Resource unless told otherwise
    let r = v.audit("update", READ.as_bytes()).unwrap();
    assert!(r.validation.is_valid(), "{}", wire::report(&r.validation));
    assert!(
        r.resource.contains(r#""system":"http://terminology.hl7.org/CodeSystem/v3-ParticipationType","code":"AUT""#),
        "{}",
        r.resource
    );
    assert!(
        r.resource.contains(r#""role":{"system":"http://terminology.hl7.org/CodeSystem/object-role","code":"4""#),
        "{}",
        r.resource
    );
    let told = READ
        .replace(r#""name": "Dr Who""#, r#""name": "Dr Who", "type": "INF""#)
        .replace(r#""data": "Observation/hr1""#, r#""data": {"what": "Observation/hr1", "role": "3"}"#);
    let r = v.audit("update", told.as_bytes()).unwrap();
    assert!(r.validation.is_valid(), "{}", wire::report(&r.validation));
    assert!(
        r.resource.contains(r#""code":"INF""#) && r.resource.contains(r#"object-role","code":"3""#),
        "{}",
        r.resource
    );
    // a create fixes its subtype: the host cannot give one
    assert_eq!(
        v.audit("create", READ.replace(r#""transaction""#, r#""subtype": "read", "transaction""#).as_bytes())
            .unwrap_err(),
        AuditError::Unexpected("subtype")
    );
    // without a user, the client and server still make a record
    let r = v
        .audit("read", READ.replace(r#""user": {"who": "Practitioner/gp1", "name": "Dr Who"},"#, "").as_bytes())
        .unwrap();
    assert!(r.validation.is_valid(), "{}", wire::report(&r.validation));
    assert!(!r.resource.contains("IRCP"));
}

#[test]
fn what_is_missing_or_out_of_place_is_an_error_before_anything_is_built() {
    let v = Validator::new();
    assert_eq!(v.audit("browse", READ.as_bytes()).unwrap_err(), AuditError::UnknownKind);
    assert_eq!(v.audit("read", b"[]").unwrap_err(), AuditError::InputsNotAnObject);
    let without = |key: &str| -> String {
        let mut lines: Vec<&str> = READ.lines().collect();
        lines.retain(|l| !l.trim_start().starts_with(&format!(r#""{key}""#)));
        lines.join("\n").replace(",\n}", "\n}")
    };
    assert_eq!(v.audit("read", without("recorded").as_bytes()).unwrap_err(), AuditError::Missing("recorded"));
    assert_eq!(v.audit("read", without("client").as_bytes()).unwrap_err(), AuditError::Missing("client"));
    assert_eq!(v.audit("read", without("source").as_bytes()).unwrap_err(), AuditError::Missing("source"));
    assert_eq!(v.audit("read", without("data").as_bytes()).unwrap_err(), AuditError::Missing("data"));
    assert_eq!(v.audit("patient-read", READ.as_bytes()).unwrap_err(), AuditError::Missing("patient"));
    assert_eq!(v.audit("query", READ.as_bytes()).unwrap_err(), AuditError::Missing("query"));
    assert_eq!(
        v.audit("query", inputs(Kind::Query).replace(r#""query""#, r#""data": "Observation/hr1", "query""#).as_bytes())
            .unwrap_err(),
        AuditError::Unexpected("data")
    );
    assert_eq!(v.audit("read", inputs(Kind::PatientRead).as_bytes()).unwrap_err(), AuditError::Unexpected("patient"));
    assert_eq!(
        v.audit("read", READ.replace(r#""network": "10.0.0.7""#, r#""network": 7"#).as_bytes()).unwrap_err(),
        AuditError::Malformed("client")
    );
    assert_eq!(
        v.audit(
            "read",
            READ.replace(r#""recorded": "2026-09-16T10:00:00Z""#, r#""recorded": "2026-09-16""#).as_bytes()
        )
        .unwrap_err(),
        AuditError::Malformed("recorded")
    );
    assert_eq!(
        v.audit("read", READ.replace(r#""transaction""#, r#""outcome": "4", "transaction""#).as_bytes()).unwrap_err(),
        AuditError::Unknown("outcome".into())
    );
    assert_eq!(
        v.audit("read", READ.replace(r#""site": "clinic-1""#, r#""where": "clinic-1""#).as_bytes()).unwrap_err(),
        AuditError::Unknown("source.where".into())
    );
}

#[test]
fn addresses_are_classified_and_base64_is_standard() {
    assert_eq!(network_type("10.0.0.7"), "2");
    assert_eq!(network_type("2001:0db8:85a3:0000:0000:8a2e:0370:7334"), "2");
    assert_eq!(network_type("https://fhir.example.org/r4"), "5");
    assert_eq!(network_type("ops@example.org"), "4");
    assert_eq!(network_type("myMachine.example.org"), "1");
    // F024: a scoped IPv6 address is an address, a mailto: or a urn: is a URI
    assert_eq!(network_type("fe80::1%en0"), "2");
    assert_eq!(network_type("::1"), "2");
    assert_eq!(network_type("mailto:reviewer@example.org"), "5");
    assert_eq!(network_type("urn:uuid:3f2a1c9e-7b1d-4f0e-9a8c-1d2e3f4a5b6c"), "5");
    assert_eq!(network_type("urn:oid:1.2.3"), "5");
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    // a host that knows better gives the access point itself
    let r = Validator::new()
        .audit(
            "read",
            READ.replace(r#""network": "10.0.0.7""#, r#""network": {"address": "10.0.0.7", "type": "1"}"#).as_bytes(),
        )
        .unwrap();
    assert!(r.resource.contains(r#""network":{"address":"10.0.0.7","type":"1"}"#));
}

#[test]
fn provenance_is_lowered_from_the_base_definition() {
    let v = Validator::new();
    // F023: every element of the base definition is an input, a Signature included
    let signed = v
        .lower(
            "http://hl7.org/fhir/StructureDefinition/Provenance",
            br#"{"target":["Observation/hr1"],"recorded":"2026-09-16T10:00:00Z","agent":[{"who":{"reference":"Practitioner/gp1"}}],"signature":[{"type":[{"system":"urn:iso-astm:E1762-95:2013","code":"1.2.840.10065.1.12.1.1"}],"when":"2026-09-16T10:00:00Z","who":{"reference":"Practitioner/gp1"}}]}"#,
        )
        .unwrap();
    assert!(signed.validation.is_valid(), "{}", wire::report(&signed.validation));
    assert!(
        signed.resource.contains(r#""signature":[{"type":[{"system":"urn:iso-astm:E1762-95:2013""#),
        "{}",
        signed.resource
    );
    let p = v
        .lower(
            "http://hl7.org/fhir/StructureDefinition/Provenance",
            br#"{"target":["Observation/hr1"],"recorded":"2026-09-16T10:00:00Z","agent":[{"type":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/provenance-participant-type","code":"author"}]},"who":{"reference":"Practitioner/gp1"}}]}"#,
        )
        .unwrap();
    assert!(p.validation.is_valid(), "{}", wire::report(&p.validation));
    assert!(p.resource.starts_with(r#"{"resourceType":"Provenance","meta":{"profile":["http://hl7.org/fhir/StructureDefinition/Provenance"]},"target":[{"reference":"Observation/hr1"}]"#), "{}", p.resource);
    // a failed action is not a BALP basic event: it is an AuditEvent from the base definition
    let a = v
        .lower(
            "http://hl7.org/fhir/StructureDefinition/AuditEvent",
            br#"{"type":{"system":"http://terminology.hl7.org/CodeSystem/audit-event-type","code":"rest"},"action":"R","recorded":"2026-09-16T10:00:00Z","outcome":"4","agent":[{"who":{"reference":"Device/app-7"},"requestor":true}],"source":{"observer":{"reference":"Device/audit-collector"}}}"#,
        )
        .unwrap();
    assert!(a.validation.is_valid(), "{}", wire::report(&a.validation));
    assert!(a.resource.contains(r#""outcome":"4""#));
}
