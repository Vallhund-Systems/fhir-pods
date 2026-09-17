mod common;
use common::*;

// --- FHIRPath core + invariants ---

#[test]
fn fhirpath_vs2_semantics() {
    use fhir_pods_core::document::Parsed;
    use fhir_pods_core::path::{check, parse, Verdict};
    let vs2 = parse("(component.empty() and hasMember.empty()) implies (value.exists() or dataAbsentReason.exists())")
        .unwrap();
    let has_value = Parsed::parse(br#"{"resourceType":"Observation","valueQuantity":{"value":1}}"#).unwrap();
    assert_eq!(check(&vs2, &has_value.doc()), Verdict::Pass);
    let has_component = Parsed::parse(br#"{"resourceType":"Observation","component":[{"code":{}}]}"#).unwrap();
    assert_eq!(check(&vs2, &has_component.doc()), Verdict::Pass);
    let bare = Parsed::parse(br#"{"resourceType":"Observation","status":"final"}"#).unwrap();
    assert_eq!(check(&vs2, &bare.doc()), Verdict::Fail);
}

#[test]
fn fhirpath_where_and_equality() {
    use fhir_pods_core::document::Parsed;
    use fhir_pods_core::path::{check, parse, Verdict};
    let e = parse("coding.where(system = 'http://loinc.org').exists()").unwrap();
    let ok =
        Parsed::parse(br#"{"coding":[{"system":"http://snomed.info/sct"},{"system":"http://loinc.org"}]}"#).unwrap();
    assert_eq!(check(&e, &ok.doc()), Verdict::Pass);
    let no = Parsed::parse(br#"{"coding":[{"system":"http://snomed.info/sct"}]}"#).unwrap();
    assert_eq!(check(&e, &no.doc()), Verdict::Fail);
}

#[test]
fn fhirpath_unsupported_is_skipped_not_guessed() {
    use fhir_pods_core::path::parse;
    // %resource without a root, and type functions on untyped nodes: unsupported
    assert_eq!(
        fp("contained.where(('#'+id in (%resource.descendants().reference | %resource.descendants().as(canonical))).not()).empty()", r#"{"contained":[{"id":"x"}]}"#),
        fhir_pods_core::path::Verdict::Unsupported
    );
    // decimal division, string concatenation `&`, equivalence `~`: outside the core
    assert!(parse("1 / 2").is_err());
    assert!(parse("a ~ b").is_err());
    assert!(parse("a ~ b").is_err());
    assert!(parse("1.5 < 2").is_err());
    assert!(parse("-x").is_err());
}

#[test]
fn fhirpath_arithmetic_and_precedence() {
    use fhir_pods_core::path::Verdict::{Fail, Pass};
    let d = r#"{"a":"x"}"#;
    assert_eq!(fp("1 + 2 * 3 = 7", d), Pass);
    assert_eq!(fp("(1 + 2) * 3 = 9", d), Pass);
    assert_eq!(fp("7 mod 3 = 1", d), Pass);
    assert_eq!(fp("7 div 2 = 3", d), Pass);
    assert_eq!(fp("10 - 2 - 3 = 5", d), Pass); // left-assoc
    assert_eq!(fp("-3 + 5 = 2", d), Pass);
    assert_eq!(fp("2 < 3 and 3 <= 3 and 4 > 3 and 3 >= 3", d), Pass);
    assert_eq!(fp("3 < 2", d), Fail);
    assert_eq!(fp("1 + 2 > 2 and 2 * 2 = 4", d), Pass); // + binds tighter than >, = looser than <
    assert_eq!(fp("'b' > 'a'", d), Pass);
    assert_eq!(fp("'ab' + 'c' = 'abc'", d), Pass);
    // empty propagates; division by zero is empty; both pass an invariant
    assert_eq!(fp("(nope + 1) = 1", d), Pass);
    assert_eq!(fp("(1 div 0).empty()", d), Pass);
    assert_eq!(fp("(1 mod 0).empty()", d), Pass);
}

#[test]
fn fhirpath_string_and_conversion_functions() {
    use fhir_pods_core::path::Verdict::{Fail, Pass, Unsupported};
    let d = r#"{"value":"800360","n":"42","b":true,"list":["a","bb"]}"#;
    assert_eq!(fp("value.length() = 6", d), Pass);
    assert_eq!(fp("value.substring(0,3) = '800'", d), Pass);
    assert_eq!(fp("value.substring(4) = '60'", d), Pass);
    assert_eq!(fp("value.substring(9).empty()", d), Pass);
    assert_eq!(fp("value.substring(2,-1).empty()", d), Pass);
    assert_eq!(fp("value.startsWith('8003')", d), Pass);
    assert_eq!(fp("value.startsWith('9')", d), Fail);
    assert_eq!(fp("n.toInteger() + 1 = 43", d), Pass);
    assert_eq!(fp("value.toInteger() = 800360", d), Pass);
    assert_eq!(fp("'x'.toInteger().empty()", d), Pass);
    assert_eq!(fp("b.toInteger() = 1", d), Pass);
    assert_eq!(fp("n.toString() = '42'", d), Pass);
    assert_eq!(fp("(42).toString().length() = 2", d), Pass);
    assert_eq!(fp("list.last() = 'bb'", d), Pass);
    assert_eq!(fp("value.single() = '800360'", d), Pass);
    assert_eq!(fp("nope.single().empty()", d), Pass);
    assert_eq!(fp("list.single()", d), Unsupported);
    // a string function on a non-string, or on a multi-item collection: unsupported
    assert_eq!(fp("(1).length() = 1", d), Unsupported);
    assert_eq!(fp("list.length() = 1", d), Unsupported);
    assert_eq!(fp("list < 'c'", d), Unsupported);
    // an unknown function parses (it is a call) but evaluates as unsupported
    assert_eq!(fp("value.memberOf('http://x/vs')", d), Unsupported);
}

#[test]
fn fhirpath_iif_select_all_extension() {
    use fhir_pods_core::path::Verdict::{Fail, Pass};
    let d = r#"{"digits":["1","7"],"name":[{"family":"A"},{"family":"B"}],"extension":[{"url":"http://x/dar","valueCode":"unknown"},{"url":"http://x/other"}]}"#;
    assert_eq!(
        fp("digits.select($this.toInteger()).select(iif($this < 5, $this * 2, $this * 2 - 9)).first() = 2", d),
        Pass
    );
    assert_eq!(
        fp("digits.select($this.toInteger()).select(iif($this < 5, $this * 2, $this * 2 - 9)).last() = 5", d),
        Pass
    );
    assert_eq!(fp("iif(true, 'y', 'n') = 'y'", d), Pass);
    assert_eq!(fp("iif(nope.exists(), 'y', 'n') = 'n'", d), Pass);
    assert_eq!(fp("iif(false, 'y').empty()", d), Pass);
    assert_eq!(fp("name.all(family.exists())", d), Pass);
    assert_eq!(fp("name.all(given.exists())", d), Fail);
    assert_eq!(fp("nope.all(given.exists())", d), Pass);
    assert_eq!(fp("extension('http://x/dar').exists()", d), Pass);
    assert_eq!(fp("extension('http://x/dar').value.exists()", d), Pass);
    assert_eq!(fp("extension('http://x/none').exists()", d), Fail);
    assert_eq!(fp("name.count() + digits.count() > 3", d), Pass);
    // an element named like a keyword is written delimited
    assert_eq!(fp("text.`div`.exists()", r#"{"text":{"div":"<div/>"}}"#), Pass);
}

#[test]
fn ihi_luhn_invariant_fires_through_the_slice_profile() {
    let ihi = |value: &str| {
        PATIENT_OK.replace(
            PATIENT_ID,
            &format!(
                r#""identifier":[{{"type":{{"coding":[{{"system":"http://terminology.hl7.org/CodeSystem/v2-0203","code":"NI"}}]}},"system":"http://ns.electronichealth.net.au/id/hi/ihi/1.0","value":"{value}"}}]"#
            ),
        )
    };
    let good = probe_str(&ihi("8003608833357361"));
    assert!(good.contains("\"valid\":true"), "{good}");
    let bad = probe_str(&ihi("8003608833357362"));
    assert!(bad.contains("Luhn"), "{bad}");
    assert!(bad.contains("\"valid\":false"), "{bad}");
}

#[test]
fn heartrate_missing_value_and_dar_fails_vs2() {
    let r = probe_str(&HEARTRATE_OK.replace(
        r#","valueQuantity":{"value":72,"unit":"beats/minute","system":"http://unitsofmeasure.org","code":"/min"}"#,
        "",
    ));
    assert!(r.contains("vs-2"), "{r}");
    assert!(r.contains("\"valid\":false"), "{r}");
}

// --- FHIRPath tier three: type tests, %resource, intersect ---

#[test]
fn fhirpath_type_tests_use_the_elements_declared_type() {
    use fhir_pods_core::document::Parsed;
    use fhir_pods_core::path::{check, check_with, parse, Verdict};
    let e = parse("$this is dateTime implies $this.toString().length() >= 10").unwrap();
    let d = Parsed::parse(br#""2026-09""#).unwrap();
    // the element's type comes from the profile, never from the value's shape
    assert_eq!(check_with(&e, &d.doc(), Some("dateTime"), None), Verdict::Fail);
    assert_eq!(check_with(&e, &d.doc(), Some("Period"), None), Verdict::Pass);
    assert_eq!(check_with(&e, &d.doc(), Some("FHIR.dateTime"), None), Verdict::Fail);
    // unknown type: skipped, not guessed
    assert_eq!(check(&e, &d.doc()), Verdict::Unsupported);
    // vs-1: `as` yields $this or empty
    let vs1 = parse("($this as dateTime).toString().length() >= 8").unwrap();
    assert_eq!(check_with(&vs1, &d.doc(), Some("dateTime"), None), Verdict::Fail);
    assert_eq!(check_with(&vs1, &d.doc(), Some("Period"), None), Verdict::Pass); // empty >= 8 is empty → passes
                                                                                 // `is` on a node whose type the table cannot give: unsupported, not guessed
    let other = parse("a is string").unwrap();
    let untyped = Parsed::parse(br#"{"resourceType":"Nope","a":"x"}"#).unwrap();
    assert_eq!(check_with(&other, &untyped.doc(), Some("Nope"), None), Verdict::Unsupported);
}

#[test]
fn fhirpath_resource_variable_and_intersect() {
    use fhir_pods_core::document::Parsed;
    use fhir_pods_core::path::{check, check_with, parse, Verdict};
    // obs-7: a component must not repeat the observation's own code
    let obs7 = parse("value.empty() or component.code.where(coding.intersect(%resource.code.coding).exists()).empty()")
        .unwrap();
    let mk = |comp_code: &str| {
        format!(
            r#"{{"resourceType":"Observation","code":{{"coding":[{{"system":"http://loinc.org","code":"85354-9"}}]}},"valueQuantity":{{"value":1}},"component":[{{"code":{{"coding":[{{"system":"http://loinc.org","code":"{comp_code}"}}]}}}}]}}"#
        )
    };
    let (ok_src, dup_src) = (mk("8480-6"), mk("85354-9"));
    let ok = Parsed::parse(ok_src.as_bytes()).unwrap();
    assert_eq!(check_with(&obs7, &ok.doc(), Some("Observation"), Some(&ok.doc())), Verdict::Pass);
    let dup = Parsed::parse(dup_src.as_bytes()).unwrap();
    assert_eq!(check_with(&obs7, &dup.doc(), Some("Observation"), Some(&dup.doc())), Verdict::Fail);
    // without a root, %resource is unsupported (skipped)
    assert_eq!(check(&obs7, &dup.doc()), Verdict::Unsupported);
    // intersect on scalars
    let d = Parsed::parse(br#"{"a":["x","y"],"b":["y","z"]}"#).unwrap();
    assert_eq!(check(&parse("a.intersect(b) = 'y'").unwrap(), &d.doc()), Verdict::Pass);
    assert_eq!(check(&parse("a.intersect(c).empty()").unwrap(), &d.doc()), Verdict::Pass);
}

#[test]
fn obs7_fires_through_the_engine_on_a_profiled_observation() {
    // A valued observation with a component repeating its own code:
    // obs-7 (an Observation base invariant, reached via %resource).
    let dup = HEARTRATE_OK.replace(
        "\"effectiveDateTime\"",
        "\"component\":[{\"code\":{\"coding\":[{\"system\":\"http://loinc.org\",\"code\":\"8867-4\"}]},\"valueString\":\"x\"}],\"effectiveDateTime\"",
    );
    let r = probe_str(&dup);
    assert!(r.contains("obs-7"), "{r}");
    assert!(!probe_str(HEARTRATE_OK).contains("obs-7"));
}

// --- review findings: what the spec says and we now do ---

#[test]
fn fhirpath_string_escapes_are_decoded() {
    use fhir_pods_core::path::Verdict::Pass;
    let d = r#"{"v":"a.b","n":"12"}"#;
    assert_eq!(fp("v.matches('a\\\\.b')", d), Pass); // FHIRPath source: 'a\\.b' → regex a\.b
    assert_eq!(fp("n.matches('^\\\\d+$')", d), Pass);
    assert_eq!(fp("'it\\'s' = 'it' + '\\'' + 's'", d), Pass); // FHIRPath source: 'it\'s'
    assert_eq!(fp("'\\u0041' = 'A'", d), Pass);
    assert!(fhir_pods_core::path::parse("'\\q' = 'q'").is_err()); // not a FHIRPath escape: the parse is refused
                                                                  // $index / $total: not in the core
    assert!(fhir_pods_core::path::parse("$index = 0").is_err());
    assert!(fhir_pods_core::path::parse("$total").is_err());
}

#[test]
fn implies_is_left_associative_and_booleans_are_singletons() {
    use fhir_pods_core::path::Verdict::{Fail, Pass, Unsupported};
    let d = r#"{"a":["x","y"],"one":["x"]}"#;
    assert_eq!(fp("false implies true implies false", d), Fail); // (false implies true) implies false
                                                                 // more than one item in boolean context: skipped, never guessed
    assert_eq!(fp("a.not()", d), Unsupported);
    assert_eq!(fp("a and true", d), Unsupported);
    assert_eq!(fp("one and true", d), Pass);
    assert_eq!(fp("a.hasValue()", d), Fail);
    assert_eq!(fp("one.hasValue()", d), Pass);
}

#[test]
fn numbers_are_numbers() {
    use fhir_pods_core::path::Verdict::{Fail, Pass, Unsupported};
    let d = r#"{"a":1,"b":2,"c":10,"d":9,"s":"1","x":1.5}"#;
    assert_eq!(fp("a + b = 3", d), Pass);
    assert_eq!(fp("c > d", d), Pass);
    assert_eq!(fp("a = 1", d), Pass);
    assert_eq!(fp("a = s", d), Fail); // different types are not equal
    assert_eq!(fp("1 = '1'", d), Fail);
    assert_eq!(fp("(a).length() = 1", d), Unsupported);
    assert_eq!(fp("x = 1", d), Unsupported); // decimals: outside the core
    assert_eq!(fp("a + s = 2", d), Unsupported);
}

#[test]
fn dates_compare_as_instants() {
    use fhir_pods_core::path::Verdict::{Fail, Pass};
    let d = r#"{"h":"2020-01-01T10:00:00.000Z","p":"2020-01-01T10:00:00Z","q":"2020-01-01T10:00:00+00:00","later":"2020-01-01T19:00:00+10:00","early":"2020-01-01T09:30:00Z","day":"2020-01-01","month":"2020-01"}"#;
    assert_eq!(fp("h >= p", d), Pass);
    assert_eq!(fp("h = p and p = q", d), Pass);
    assert_eq!(fp("later < early", d), Pass); // 19:00+10:00 is 09:00Z — before 09:30Z
    assert_eq!(fp("early > later", d), Pass);
    assert_eq!(fp("(day = month).empty()", d), Pass); // different precision: empty
    assert_eq!(fp("(day < h).empty()", d), Pass); // date vs dateTime with tz: empty
    assert_eq!(fp("day < '2020-01-02'", d), Pass);
    assert_eq!(fp("'b' > 'a'", d), Pass); // plain text still compares
    assert_eq!(fp("h < p", d), Fail);
}

#[test]
fn type_tests_honour_the_hierarchy() {
    use fhir_pods_core::path::Verdict::{Fail, Pass};
    let d = r#"{"resourceType":"Condition","onsetAge":{"value":1,"unit":"a"},"code":{"text":"x"},"subject":{"reference":"Patient/1"},"id":"c1"}"#;
    assert_eq!(fp("onset is Quantity", d), Pass);
    assert_eq!(fp("onset is Age", d), Pass);
    assert_eq!(fp("onset is CodeableConcept", d), Fail);
    assert_eq!(fp("$this is Resource and $this is DomainResource", r#"{"resourceType":"Condition"}"#), Pass);
    assert_eq!(fp("$this is DomainResource", r#"{"resourceType":"Bundle"}"#), Fail); // a Resource, not a Domain one
    assert_eq!(fp("$this is DomainResource", r#"{"resourceType":"Nope"}"#), fhir_pods_core::path::Verdict::Unsupported); // not in the table
    assert_eq!(fp("code is Element", d), Pass);
    assert_eq!(fp("id is id and id is string and id is FHIR.string", d), Pass);
    // type names are matched as written, like the reference engine: the
    // FHIRPath system types answer only when qualified — so `$this is
    // DateTime` on a dateTime is false (and US Core's us-core-1 never fires)
    assert_eq!(fp("id is String", d), Fail);
    // a FHIR element is never a System type (the official suite's testType14:
    // `Patient.active.is(System.Boolean).not()`); a literal is one
    assert_eq!(fp("id is System.String", d), Fail);
    assert_eq!(fp("'x' is System.String and 'x' is String and 1 is Integer and true is Boolean", d), Pass);
    assert_eq!(fp("1 is FHIR.integer or 1 is integer", d), Fail);
    assert_eq!(fp("recordedDate is DateTime", r#"{"resourceType":"Condition","recordedDate":"2020"}"#), Fail);
    assert_eq!(
        fp(
            "recordedDate is dateTime and recordedDate is FHIR.dateTime and (recordedDate is System.DateTime).not()",
            r#"{"resourceType":"Condition","recordedDate":"2020"}"#
        ),
        Pass
    );
    assert_eq!(fp("subject.reference is string", d), Pass);
    let mr =
        r#"{"resourceType":"MedicationRequest","instantiatesCanonical":["http://x"],"instantiatesUri":["http://y"]}"#;
    assert_eq!(fp("descendants().ofType(uri).count() = 1", mr), Pass); // exact: a canonical is not a uri to ofType
    assert_eq!(fp("descendants().where($this is uri).count() = 2", mr), Pass); // but it is one to `is`
}

// --- regex + tier three collections ---

#[test]
fn regex_engine_covers_the_registrys_patterns_and_refuses_the_rest() {
    use fhir_pods_core::regex::Regex;
    let ok = |p: &str, s: &str| Regex::new(p).unwrap().is_match(s);
    assert!(ok("^([0-9]{16})$", "8003608833357361"));
    assert!(!ok("^([0-9]{16})$", "800360883335736"));
    assert!(!ok("^([0-9]{16})$", "8003608833357361x"));
    assert!(ok("^[A-Z]{3}[0-9]{10}$", "ABC0123456789"));
    assert!(!ok("^[A-Z]{3}[0-9]{10}$", "AbC0123456789"));
    assert!(ok("^(800361)([0-9]{10})(@)(800362)([0-9]{10})$", "8003610000000000@8003620000000000"));
    assert!(ok("b+c", "aabbbcc")); // unanchored: a search
    assert!(!ok("^b+c", "aabbbcc"));
    assert!(ok("a|b", "xbx") && !ok("a|b", "xyz"));
    assert!(ok("^\\d{2}-\\w+\\s?$", "12-abc_ "));
    assert!(ok("[^a-c]", "d") && !ok("[^a-c]", "b"));
    assert!(ok("colou?r", "color") && ok("colou?r", "colour"));
    assert!(ok("^a{2,3}$", "aaa") && !ok("^a{2,3}$", "aaaa"));
    assert!(ok("^.$", "é") && !ok("^.$", "ab"));
    assert!(ok("(?:ab)+", "ababab"));
    // linear time: the classic backtracking bomb finishes instantly
    assert!(!Regex::new("^(a+)+$").unwrap().is_match(&format!("{}b", "a".repeat(5000))));
    // outside the supported syntax: refused, never guessed
    for p in ["(a)\\1", "(?=a)", "(?i)a", "\\ba", "\\p{L}", "a{300}", "(a", "a)", "*a", "[z-a]"] {
        assert!(Regex::new(p).is_err(), "{p}");
    }
}

#[test]
fn fhirpath_matches_union_in_descendants_repeat() {
    use fhir_pods_core::path::Verdict::{Fail, Pass, Unsupported};
    let d = r#"{"value":"8003608833357361","a":["x","y"],"b":["y","z"],"item":[{"linkId":"1","item":[{"linkId":"2"}]},{"linkId":"3"}]}"#;
    assert_eq!(fp("value.matches('^([0-9]{16})$')", d), Pass);
    assert_eq!(fp("value.matches('^([0-9]{15})$')", d), Fail);
    assert_eq!(fp("value.matches('(a)\\\\1')", d), Unsupported); // a backreference: refused
    assert_eq!(fp("nope.matches('x').empty()", d), Pass);
    assert_eq!(fp("(a | b).count() = 3", d), Pass);
    assert_eq!(fp("'y' in a and 'y' in b and ('q' in a).not()", d), Pass);
    assert_eq!(fp("a contains 'x'", d), Pass);
    assert_eq!(fp("descendants().linkId.count() = 3", d), Pass);
    assert_eq!(fp("repeat(item).linkId.count() = 3", d), Pass);
    assert_eq!(fp("repeat(item).linkId.isDistinct()", d), Pass);
    assert_eq!(fp("(a | b | a).isDistinct()", d), Pass);
    assert_eq!(fp("a.trace('t').count() = 2", d), Pass);
    assert_eq!(fp("(true | false).anyTrue() and (true | false).allTrue().not() and (false).allFalse()", d), Pass);
    assert_eq!(fp("nope.allTrue()", d), Pass); // vacuous
                                               // sdcqr-2's shape: every group's items have distinct linkIds
    assert_eq!(fp("repeat(item).select(item.linkId.isDistinct()).allTrue()", d), Pass);
}

#[test]
fn ihi_format_invariant_fires_through_the_slice_profile() {
    let short = PATIENT_OK.replace(
        PATIENT_ID,
        r#""identifier":[{"type":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v2-0203","code":"NI"}]},"system":"http://ns.electronichealth.net.au/id/hi/ihi/1.0","value":"800360883335736"}]"#,
    );
    let r = probe_str(&short);
    assert!(r.contains("16 digit"), "{r}");
    assert!(r.contains("\"valid\":false"), "{r}");
}

#[test]
fn questionnaireresponse_repeat_invariant_is_live() {
    use fhir_pods_core::path::parse;
    // the registry's own sdcqr-2 must now parse
    let expr = fhir_pods_core::generated::REGISTRY
        .iter()
        .flat_map(|p| p.invariants.iter())
        .find(|i| i.key == "sdcqr-2")
        .map(|i| i.expression)
        .expect("sdcqr-2 in the registry");
    assert!(parse(expr).is_ok(), "{expr}");
}

// --- typed traversal + dom-3 ---

#[test]
fn nodes_know_their_fhir_type_from_the_table() {
    use fhir_pods_core::document::{Document, Parsed};
    let src = br#"{"resourceType":"Observation","valueQuantity":{"value":1},"component":[{"code":{"coding":[{"system":"x"}]}}],"contained":[{"resourceType":"Patient","name":[{"family":"D"}]}],"referenceRange":[{"low":{"value":1}}]}"#;
    let d = Parsed::parse(src).unwrap();
    let doc = d.doc();
    assert_eq!(doc.fhir_type(), Some("Observation"));
    let value = doc.instances("$this.valueQuantity");
    assert_eq!(value[0].fhir_type(), Some("Quantity"));
    assert_eq!(value[0].instances("$this.value")[0].fhir_type(), Some("decimal"));
    let comp = doc.instances("$this.component");
    assert_eq!(comp[0].fhir_type(), Some("BackboneElement"));
    assert_eq!(comp[0].instances("$this.code.coding.system")[0].fhir_type(), Some("uri"));
    let contained = doc.instances("$this.contained");
    assert_eq!(contained[0].fhir_type(), Some("Patient"));
    assert_eq!(contained[0].instances("$this.name")[0].fhir_type(), Some("HumanName"));
    // content reference
    assert_eq!(doc.instances("$this.referenceRange.low")[0].fhir_type(), Some("Quantity"));
    // untyped: a resource the table doesn't cover, or a [x] path
    assert_eq!(Parsed::parse(br#"{"resourceType":"Nope","a":1}"#).unwrap().doc().fhir_type(), None);
    assert_eq!(doc.instances("$this.value[x]")[0].fhir_type(), None);
    // descendants carry types
    let kinds: Vec<_> = doc.all_children().iter().map(|c| c.fhir_type()).collect();
    assert!(
        kinds.contains(&Some("Quantity"))
            && kinds.contains(&Some("Patient"))
            && kinds.contains(&Some("BackboneElement")),
        "{kinds:?}"
    );
}

#[test]
fn fhirpath_type_functions_on_typed_nodes() {
    use fhir_pods_core::path::Verdict::{Fail, Pass, Unsupported};
    let d = r#"{"resourceType":"MedicationRequest","instantiatesCanonical":["http://x/plan"],"instantiatesUri":["http://x/u"],"subject":{"reference":"Patient/1"},"medicationCodeableConcept":{"text":"m"}}"#;
    assert_eq!(fp("descendants().as(canonical).count() = 1", d), Pass);
    // `ofType` / `as` name the exact type (the official suite: `Patient.gender.ofType(string)`
    // is empty though a code is a string); `is` honours the hierarchy
    assert_eq!(fp("descendants().ofType(FHIR.uri).count() = 1", d), Pass);
    assert_eq!(fp("descendants().where($this is uri).count() = 2", d), Pass);
    assert_eq!(fp("instantiatesCanonical.first().as(string1)", d), Unsupported); // an unknown type is an error
    assert_eq!(fp("medication.ofType(CodeableConcept).exists()", d), Pass);
    assert_eq!(fp("medication is CodeableConcept", d), Pass);
    assert_eq!(fp("medication is Reference", d), Fail);
    assert_eq!(fp("subject is Reference and subject.reference is string", d), Pass);
    // untyped resource: skipped, not guessed
    assert_eq!(fp("descendants().as(canonical).empty()", r#"{"resourceType":"Nope","a":"b"}"#), Unsupported);
}

#[test]
fn document_bundle_functions_is_tail_and_string_contains() {
    use fhir_pods_core::path::Verdict::{Fail, Pass};
    let b = r#"{"resourceType":"Bundle","type":"document","entry":[{"fullUrl":"urn:uuid:a","resource":{"resourceType":"Composition","id":"a"}},{"fullUrl":"urn:uuid:b","resource":{"resourceType":"Patient","id":"b"}}]}"#;
    assert_eq!(fp("entry.first().resource.is(Composition)", b), Pass);
    assert_eq!(fp("entry.tail().where(resource is Composition).empty()", b), Pass);
    assert_eq!(fp("entry.tail().resource.is(Composition)", b), Fail);
    assert_eq!(fp("entry.all(fullUrl.contains('/_history/').not())", b), Pass);
    assert_eq!(fp("'abc'.contains('bc') and 'abc'.contains('x').not()", b), Pass);
    assert_eq!(fp("entry.first().resource.is(Patient)", b), Fail);
}

#[test]
fn dom3_contained_resources_must_be_referenced() {
    use fhir_pods_core::path::Verdict::{Fail, Pass};
    let mk = |reference: &str| {
        format!(
            r#"{{"resourceType":"MedicationRequest","contained":[{{"resourceType":"Medication","id":"med","code":{{"text":"x"}}}}],"status":"active","intent":"order","medicationReference":{{"reference":"{reference}"}},"subject":{{"reference":"Patient/1"}}}}"#
        )
    };
    let dom3 = fhir_pods_core::generated::REGISTRY
        .iter()
        .flat_map(|p| p.invariants.iter())
        .find(|i| i.key == "dom-3")
        .unwrap()
        .expression;
    use fhir_pods_core::document::Parsed;
    use fhir_pods_core::path::{check_with, parse};
    let e = parse(dom3).unwrap();
    let (ok_src, bad_src) = (mk("#med"), mk("Medication/other"));
    let ok = Parsed::parse(ok_src.as_bytes()).unwrap();
    assert_eq!(check_with(&e, &ok.doc(), Some("MedicationRequest"), Some(&ok.doc())), Pass);
    let bad = Parsed::parse(bad_src.as_bytes()).unwrap();
    assert_eq!(check_with(&e, &bad.doc(), Some("MedicationRequest"), Some(&bad.doc())), Fail);
    // through the engine, on a profiled resource
    let profiled = bad_src.replace("\"resourceType\":\"MedicationRequest\",", "\"resourceType\":\"MedicationRequest\",\"meta\":{\"profile\":[\"http://hl7.org/fhir/us/core/StructureDefinition/us-core-medicationrequest\"]},");
    let r = probe_str(&profiled);
    assert!(r.contains("dom-3"), "{r}");
}

/// `isDistinct()` / `distinct()` over plain strings take the sorted path
/// (a bundle of thousands of fullUrls, F057) with the same answers as the
/// pairwise one: order kept, duplicates found wherever they sit.
#[test]
fn fhirpath_distinct_over_strings_keeps_order_and_finds_far_duplicates() {
    use fhir_pods_core::document::Parsed;
    use fhir_pods_core::path::{check, parse, Verdict};
    let bundle = Parsed::parse(
        br#"{"resourceType":"Bundle","type":"collection","entry":[{"fullUrl":"urn:uuid:b"},{"fullUrl":"urn:uuid:a"},{"fullUrl":"urn:uuid:c"},{"fullUrl":"urn:uuid:a"}]}"#,
    )
    .unwrap();
    let dup = parse("entry.fullUrl.isDistinct()").unwrap();
    assert_eq!(check(&dup, &bundle.doc()), Verdict::Fail);
    let three = parse("entry.fullUrl.distinct().count() = 3").unwrap();
    assert_eq!(check(&three, &bundle.doc()), Verdict::Pass);
    let order =
        parse("entry.fullUrl.distinct().first() = 'urn:uuid:b' and entry.fullUrl.distinct().last() = 'urn:uuid:c'")
            .unwrap();
    assert_eq!(check(&order, &bundle.doc()), Verdict::Pass);
    let clean = Parsed::parse(
        br#"{"resourceType":"Bundle","type":"collection","entry":[{"fullUrl":"urn:uuid:b"},{"fullUrl":"urn:uuid:a"}]}"#,
    )
    .unwrap();
    assert_eq!(check(&dup, &clean.doc()), Verdict::Pass);
}
