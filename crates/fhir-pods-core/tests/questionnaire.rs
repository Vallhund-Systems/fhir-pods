//! A QuestionnaireResponse against its Questionnaire — the rules the
//! reference validator applies, on synthetic forms (fhir-test-cases'
//! questionnaire module is the lane: `conformance/questionnaire.py`).
use fhir_pods_core::validate::Severity;
use fhir_pods_core::Validator;

const Q: &str = r##"{
  "resourceType": "Questionnaire", "url": "http://example.org/Questionnaire/intake", "version": "1", "status": "active",
  "contained": [{"resourceType": "ValueSet", "id": "yn", "compose": {"include": [{"system": "http://example.org/cs", "concept": [{"code": "y"}, {"code": "n"}]}]}}],
  "item": [
    {"linkId": "name", "type": "string", "required": true, "maxLength": 10,
     "extension": [{"url": "http://hl7.org/fhir/StructureDefinition/minLength", "valueInteger": 2}]},
    {"linkId": "smoker", "type": "choice", "required": true, "answerOption": [
       {"valueCoding": {"system": "http://example.org/cs", "code": "y"}},
       {"valueCoding": {"system": "http://example.org/cs", "code": "n"},
        "extension": [{"url": "http://hl7.org/fhir/StructureDefinition/questionnaire-optionExclusive", "valueBoolean": true}]}]},
    {"linkId": "packs", "type": "integer", "enableWhen": [{"question": "smoker", "operator": "=", "answerCoding": {"system": "http://example.org/cs", "code": "y"}}],
     "extension": [{"url": "http://hl7.org/fhir/StructureDefinition/minValue", "valueInteger": 0},
                   {"url": "http://hl7.org/fhir/StructureDefinition/maxValue", "valueInteger": 5}]},
    {"linkId": "vs", "type": "choice", "answerValueSet": "#yn"},
    {"linkId": "weight", "type": "quantity", "extension": [
       {"url": "http://hl7.org/fhir/uv/sdc/StructureDefinition/sdc-questionnaire-maxQuantity", "valueQuantity": {"value": 200, "system": "http://unitsofmeasure.org", "code": "kg"}}]},
    {"linkId": "history", "type": "group", "required": true, "item": [
       {"linkId": "history.when", "type": "date", "extension": [{"url": "http://hl7.org/fhir/StructureDefinition/minValue", "valueDate": "2000"}]}]},
    {"linkId": "note", "type": "display"}
  ]
}"##;

fn response(items: &str, status: &str) -> String {
    format!(
        r#"{{"resourceType": "QuestionnaireResponse", "questionnaire": "http://example.org/Questionnaire/intake|1", "status": "{status}", "item": [{items}]}}"#
    )
}

const GOOD: &str = r#"{"linkId": "name", "answer": [{"valueString": "Alex"}]},
  {"linkId": "smoker", "answer": [{"valueCoding": {"system": "http://example.org/cs", "code": "y"}}]},
  {"linkId": "packs", "answer": [{"valueInteger": 2}]},
  {"linkId": "vs", "answer": [{"valueCoding": {"system": "http://example.org/cs", "code": "n"}}]},
  {"linkId": "weight", "answer": [{"valueQuantity": {"value": 80, "system": "http://unitsofmeasure.org", "code": "kg"}}]},
  {"linkId": "history", "item": [{"linkId": "history.when", "answer": [{"valueDate": "2010-04-01"}]}]}"#;

fn validator() -> Validator {
    Validator::new().with_questionnaire(Q.as_bytes())
}

fn errors(v: &Validator, qr: &str) -> Vec<String> {
    v.validate(qr.as_bytes())
        .report
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| format!("{} | {}", i.path, i.message))
        .collect()
}

#[test]
fn a_complete_response_to_a_loaded_questionnaire_is_valid() {
    let v = validator();
    let r = v.validate(response(GOOD, "completed").as_bytes());
    assert!(r.is_valid(), "{}", fhir_pods_core::wire::report(&r));
    assert!(!r.report.issues.iter().any(|i| i.message.contains("could not be resolved")));
}

#[test]
fn an_unknown_questionnaire_is_a_warning_not_a_guess() {
    let r = Validator::new().validate(response(GOOD, "completed").as_bytes());
    assert!(r.is_valid());
    // an example.org canonical is only informational, as the reference validator has it
    let w = r
        .report
        .issues
        .iter()
        .find(|i| i.severity == Severity::Information && i.message.contains("could not be resolved"));
    assert!(w.is_some(), "{}", fhir_pods_core::wire::report(&r));
    let elsewhere = response(GOOD, "completed")
        .replace("http://example.org/Questionnaire/intake|1", "http://forms.example.net/Questionnaire/intake");
    let r = Validator::new().validate(elsewhere.as_bytes());
    assert!(r
        .report
        .issues
        .iter()
        .any(|i| i.severity == Severity::Warning && i.message.contains("could not be resolved")));
    // nothing else was judged
    assert!(!r.report.issues.iter().any(|i| i.path.contains(".item[")));
}

#[test]
fn a_contained_questionnaire_and_a_bundled_one_are_found_without_loading() {
    let contained = format!(
        r##"{{"resourceType": "QuestionnaireResponse", "questionnaire": "#q", "status": "completed",
             "contained": [{}], "item": [{{"linkId": "nope", "answer": [{{"valueString": "x"}}]}}]}}"##,
        Q.replacen("\"resourceType\": \"Questionnaire\",", "\"resourceType\": \"Questionnaire\", \"id\": \"q\",", 1)
    );
    let e = errors(&Validator::new(), &contained);
    assert!(e.iter().any(|m| m.contains("LinkId 'nope' not found in questionnaire")), "{e:?}");
    let bundle = format!(
        r#"{{"resourceType": "Bundle", "type": "collection", "entry": [
             {{"fullUrl": "http://example.org/Questionnaire/intake", "resource": {Q}}},
             {{"fullUrl": "urn:uuid:8b3c8a1c-2f3e-4f0a-9f1b-0d3e5c6a7b8c", "resource": {}}}]}}"#,
        response(r#"{"linkId": "nope", "answer": [{"valueString": "x"}]}"#, "completed")
    );
    let e = errors(&Validator::new(), &bundle);
    assert!(e.iter().any(|m| m.contains("LinkId 'nope' not found in questionnaire")), "{e:?}");
}

#[test]
fn required_answers_and_groups_are_owed_and_in_progress_only_warns() {
    let v = validator();
    let e = errors(
        &v,
        &response(
            r#"{"linkId": "smoker", "answer": [{"valueCoding": {"system": "http://example.org/cs", "code": "n"}}]}"#,
            "completed",
        ),
    );
    assert!(
        e.iter().any(|m| m == "QuestionnaireResponse | No response answer found for required item 'name'"),
        "{e:?}"
    );
    assert!(
        e.iter().any(|m| m == "QuestionnaireResponse | No response answer found for required item 'history'"),
        "{e:?}"
    );
    let r = v.validate(response(r#"{"linkId": "history"}"#, "completed").as_bytes());
    assert!(r.report.issues.iter().any(|i| i.severity == Severity::Error
        && i.path == "QuestionnaireResponse.item[0]"
        && i.message == "No sub-items found for required group"));
    let r = v
        .validate(response(r#"{"linkId": "history", "item": [{"linkId": "history.when"}]}"#, "in-progress").as_bytes());
    let missing: Vec<_> =
        r.report.issues.iter().filter(|i| i.message.starts_with("No response answer found")).collect();
    assert!(missing.iter().all(|i| i.severity == Severity::Warning), "{}", fhir_pods_core::wire::report(&r));
}

#[test]
fn enable_when_gates_an_answer_and_the_answer_type_is_the_definitions() {
    let v = validator();
    // not a smoker: packs is not enabled, so its answer is an error
    let e = errors(
        &v,
        &response(
            r#"{"linkId": "name", "answer": [{"valueString": "Alex"}]},
        {"linkId": "smoker", "answer": [{"valueCoding": {"system": "http://example.org/cs", "code": "n"}}]},
        {"linkId": "packs", "answer": [{"valueInteger": 2}]},
        {"linkId": "history", "item": [{"linkId": "history.when", "answer": [{"valueDate": "2010"}]}]}"#,
            "completed",
        ),
    );
    assert_eq!(
        e,
        ["QuestionnaireResponse.item[2] | Item has answer, even though it is not enabled (item id = 'packs')"]
    );
    let e = errors(&v, &response(&GOOD.replace(r#"{"valueInteger": 2}"#, r#"{"valueString": "two"}"#), "completed"));
    assert_eq!(
        e,
        ["QuestionnaireResponse.item[2].answer[0].valueString | Answer value must be of the type integer not string"]
    );
}

#[test]
fn bounds_options_and_exclusivity_are_the_definitions() {
    let v = validator();
    let e = errors(&v, &response(&GOOD.replace(r#"{"valueInteger": 2}"#, r#"{"valueInteger": 9}"#), "completed"));
    assert_eq!(
        e,
        ["QuestionnaireResponse.item[2].answer[0].valueInteger | The value 9 is greater than the allowed maximum of 5"]
    );
    let e = errors(&v, &response(&GOOD.replace(r#""valueString": "Alex""#, r#""valueString": "A""#), "completed"));
    assert_eq!(e, ["QuestionnaireResponse.item[0].answer[0].valueString | The answer 'A' is shorter then the required minimum length of 2"]);
    let e = errors(&v, &response(&GOOD.replace(r#""code": "y"}}]}"#, r#""code": "maybe"}}]}"#), "completed"));
    assert!(e.iter().any(|m| m == "QuestionnaireResponse.item[1].answer[0] | The code http://example.org/cs::maybe is not in the set of permitted values"), "{e:?}");
    // the exclusive option beside another
    let both = GOOD.replace(
        r#"{"linkId": "smoker", "answer": [{"valueCoding": {"system": "http://example.org/cs", "code": "y"}}]}"#,
        r#"{"linkId": "smoker", "answer": [{"valueCoding": {"system": "http://example.org/cs", "code": "y"}}, {"valueCoding": {"system": "http://example.org/cs", "code": "n"}}]}"#,
    );
    let e = errors(&v, &response(&both, "completed"));
    assert!(e.iter().any(|m| m.contains("Selected answer http://example.org/cs#n is an exclusive option")), "{e:?}");
    assert!(e.iter().any(|m| m.contains("Only one response answer item with this linkId allowed")), "{e:?}");
    // a contained value set decides membership
    let e = errors(
        &v,
        &response(
            &GOOD.replace(
                r#""code": "n"}}]},
  {"linkId": "weight""#,
                r#""code": "x"}}]},
  {"linkId": "weight""#,
            ),
            "completed",
        ),
    );
    assert!(
        e.iter().any(|m| m.contains(
            "The code 'x' in the system 'http://example.org/cs' is not in the options value set (ValueSet[null])"
        )),
        "{e:?}"
    );
    // a quantity over its SDC maximum, and a date before its minimum
    let e = errors(&v, &response(&GOOD.replace(r#""value": 80"#, r#""value": 250"#), "completed"));
    assert_eq!(e, ["QuestionnaireResponse.item[4].answer[0].valueQuantity | The quantity 250 kg is greater than the allowed maximum of 200 kg"]);
    let e = errors(&v, &response(&GOOD.replace("2010-04-01", "1999-12-31"), "completed"));
    assert_eq!(e, ["QuestionnaireResponse.item[5].item[0].answer[0].valueDate | The value 1999-12-31 is less than the allowed minimum of 2000"]);
}

#[test]
fn a_questionnaires_own_enable_when_wiring_is_checked() {
    let q = Q.replace(r#""question": "smoker""#, r#""question": "no-such-item""#);
    let r = Validator::new().validate(q.as_bytes());
    assert!(r.report.issues.iter().any(|i| i.severity == Severity::Error
        && i.path == "Questionnaire.item[2].enableWhen[0]"
        && i.message == "Unable to find an item with the linkId 'no-such-item' which is referenced in the enableWhen for 'packs'"), "{}", fhir_pods_core::wire::report(&r));
}

/// A coded answer with no system (a display alone) names nothing a value
/// set could hold: the reference validator's terminology error, not a pass.
/// Found by the gut-path lane.
#[test]
fn a_coded_answer_without_a_system_is_a_terminology_error() {
    let v = validator();
    let items = GOOD.replace(
        r#"{"linkId": "vs", "answer": [{"valueCoding": {"system": "http://example.org/cs", "code": "n"}}]}"#,
        r#"{"linkId": "vs", "answer": [{"valueCoding": {"display": "No"}}]}"#,
    );
    let e = errors(&v, &response(&items, "completed"));
    assert!(e.iter().any(|x| x.contains("Coding has no system")), "{e:?}");
    // a display alone, a system alone: no code — the error, under every shape
    for answer in [
        r#"{"display": "No"}"#,
        r#"{"system": "http://example.org/cs"}"#,
        r#"{"system": "http://example.org/cs", "display": "No"}"#,
    ] {
        let items = GOOD.replace(r#"{"system": "http://example.org/cs", "code": "n"}}]}"#, &format!("{answer}}}]}}"));
        let r = v.validate(response(&items, "completed").as_bytes());
        let i: Vec<_> =
            r.report.issues.iter().filter(|i| i.path == "QuestionnaireResponse.item[3].answer[0]").collect();
        assert_eq!(i.len(), 1, "{answer}: {:?}", r.report.issues);
        assert_eq!((i[0].severity, i[0].id), (Severity::Error, "questionnaire.answer-not-in-set"), "{answer}");
    }
}

/// A code with no system is not the same finding as no code at all. Where
/// the set is contained (in the questionnaire, or beside it in the response)
/// and holds the code, validator_cli 6.10.4 only warns — the code has no
/// defined meaning and cannot be validated — and the response is valid; a
/// code the set does not hold is still the error. Measured with
/// `conformance/coded-answers/` (the fhir-test-cases case
/// `nested-questionnaire-nested-valueset` is this shape).
#[test]
fn a_code_with_no_system_in_a_contained_set_is_a_warning() {
    let v = validator();
    let member = GOOD.replace(r#""system": "http://example.org/cs", "code": "n""#, r#""code": "n""#);
    let r = v.validate(response(&member, "completed").as_bytes());
    assert!(r.is_valid(), "{}", fhir_pods_core::wire::report(&r));
    let w: Vec<_> = r.report.issues.iter().filter(|i| i.id == "questionnaire.answer-no-system").collect();
    assert_eq!(w.len(), 1, "{:?}", r.report.issues);
    assert_eq!(w[0].severity, Severity::Warning);
    assert_eq!(w[0].path, "QuestionnaireResponse.item[3].answer[0]");
    assert_eq!(
        w[0].message,
        "Coding has no system. A code with no system has no defined meaning, and it cannot be validated. A system should be provided"
    );
    let stranger = GOOD.replace(r#""system": "http://example.org/cs", "code": "n""#, r#""code": "maybe""#);
    let e = errors(&v, &response(&stranger, "completed"));
    assert_eq!(e, ["QuestionnaireResponse.item[3].answer[0] | The code 'maybe' in the system 'null' is not in the options value set (ValueSet[null]) specified by the questionnaire. Terminology Error: Coding has no system. A code with no system has no defined meaning, and it cannot be validated. A system should be provided"]);
    // under inline answerOptions the option's system must match: an error, as the oracle has it
    let options = GOOD.replace(
        r#"{"linkId": "smoker", "answer": [{"valueCoding": {"system": "http://example.org/cs", "code": "y"}}]}"#,
        r#"{"linkId": "smoker", "answer": [{"valueCoding": {"code": "y"}}]}"#,
    );
    let e = errors(&v, &response(&options, "completed"));
    assert!(e.iter().any(|x| x.ends_with("The code null::y is not in the set of permitted values")), "{e:?}");
}

/// Under a set reached by canonical the reference validator's verdict on a
/// code with no system turns on the set's definition (an error where it
/// includes a whole code system, a warning where it enumerates codes); the
/// expansion a host compiles does not say which, so the pod keeps the error
/// — never the more lenient reading.
#[test]
fn a_code_with_no_system_under_a_canonical_set_stays_an_error() {
    let q = Q.replace(r##""answerValueSet": "#yn""##, r#""answerValueSet": "http://example.org/ValueSet/yn""#);
    let table = fhir_pods_core::terminology::Expansions::parse(
        "fhir-pods-terminology 1\n= http://example.org/ValueSet/yn\nhttp://example.org/cs\ty\tYes\nhttp://example.org/cs\tn\tNo\n",
    )
    .expect("table");
    let v = Validator::new().with_questionnaire(q.as_bytes());
    assert!(v.validate_with(response(GOOD, "completed").as_bytes(), &table).is_valid());
    let member = GOOD.replace(r#""system": "http://example.org/cs", "code": "n""#, r#""code": "n""#);
    let e: Vec<String> = v
        .validate_with(response(&member, "completed").as_bytes(), &table)
        .report
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| format!("{} | {}", i.path, i.message))
        .collect();
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(e[0].starts_with("QuestionnaireResponse.item[3].answer[0] | The code 'n' in the system 'null' is not in the options value set (yn)"), "{e:?}");
}

/// A questionnaire contained in the response finds an `answerValueSet` of
/// `#id` beside it, among the response's contained resources (a contained
/// questionnaire may not itself contain one, dom-2), and the set is named
/// as the reference validator names it: title, else name, else its url.
#[test]
fn a_set_beside_a_contained_questionnaire_decides_membership_and_is_named() {
    let qr = |vs: &str, answer: &str| {
        format!(
            r##"{{"resourceType": "QuestionnaireResponse", "status": "completed", "questionnaire": "#q",
  "contained": [{{"resourceType": "Questionnaire", "id": "q", "status": "active",
                 "item": [{{"linkId": "a", "type": "choice", "answerValueSet": "#vs"}}]}}, {vs}],
  "item": [{{"linkId": "a", "answer": [{{"valueCoding": {answer}}}]}}]}}"##
        )
    };
    let set = |label: &str| {
        format!(
            r#"{{"resourceType": "ValueSet", "id": "vs", "status": "active"{label},
  "compose": {{"include": [{{"system": "http://example.org/cs", "concept": [{{"code": "y"}}, {{"code": "n"}}]}}]}}}}"#
        )
    };
    let v = Validator::new();
    let named = set(r#", "url": "http://example.org/ValueSet/beside", "name": "Beside""#);
    assert!(v.validate(qr(&named, r#"{"system": "http://example.org/cs", "code": "y"}"#).as_bytes()).is_valid());
    let r = v.validate(qr(&named, r#"{"code": "y"}"#).as_bytes());
    assert!(r.is_valid(), "{}", fhir_pods_core::wire::report(&r));
    assert!(r.report.issues.iter().any(|i| i.id == "questionnaire.answer-no-system"), "{:?}", r.report.issues);
    for (label, printed) in [
        (r#", "url": "http://example.org/ValueSet/beside", "name": "Beside", "title": "Beside us""#, "Beside us"),
        (r#", "url": "http://example.org/ValueSet/beside", "name": "Beside""#, "Beside"),
        (r#", "url": "http://example.org/ValueSet/beside""#, "ValueSet[http://example.org/ValueSet/beside]"),
        ("", "ValueSet[null]"),
    ] {
        let e = errors(&v, &qr(&set(label), r#"{"system": "http://example.org/cs", "code": "maybe"}"#));
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains(&format!("is not in the options value set ({printed}) specified")), "{label}: {e:?}");
    }
}

/// A response naming a version of a loaded questionnaire other than the one
/// loaded is said — a warning a host pinning instrument versions can gate
/// on — and not judged against it (the reference validator resolves by url
/// alone; a host that wants that loads the version it means).
#[test]
fn a_response_naming_another_version_of_a_loaded_questionnaire_is_warned() {
    let v = validator();
    let qr = response(GOOD, "completed").replace("intake|1", "intake|2");
    let report = v.validate(qr.as_bytes());
    assert!(report.is_valid(), "{:?}", report.report.issues);
    let w: Vec<_> = report.report.issues.iter().filter(|i| i.id == "questionnaire.version-mismatch").collect();
    assert_eq!(w.len(), 1, "{:?}", report.report.issues);
    assert!(w[0].message.contains("names version 2 but version 1 is loaded"), "{}", w[0].message);
    assert_eq!(w[0].severity, Severity::Warning);
}

/// The warning for a code with no system is narrower than "the set holds
/// it": validator_cli 6.10.4 warns where the contained set's compose has
/// exactly one include or the set has a url, and keeps the error for a set
/// with no url and two or more includes — even of one system, wherever the
/// code sits. An enumerated exclude is applied. A system of whitespace is
/// no system. Each shape probed (`conformance/coded-answers/set-*`).
#[test]
fn a_code_with_no_system_warns_only_where_the_oracle_does() {
    let qr = |label: &str, compose: &str, answer: &str| {
        format!(
            r##"{{"resourceType": "QuestionnaireResponse", "status": "completed", "questionnaire": "#q",
  "contained": [{{"resourceType": "Questionnaire", "id": "q", "status": "active",
                 "item": [{{"linkId": "a", "type": "choice", "answerValueSet": "#vs"}}]}},
                {{"resourceType": "ValueSet", "id": "vs", "status": "active"{label}, "compose": {compose}}}],
  "item": [{{"linkId": "a", "answer": [{{"valueCoding": {answer}}}]}}]}}"##
        )
    };
    let cs = "http://example.org/cs";
    let other = "http://example.org/other";
    let one = format!(r#"{{"include": [{{"system": "{cs}", "concept": [{{"code": "y"}}, {{"code": "n"}}]}}]}}"#);
    let two_one_system = format!(
        r#"{{"include": [{{"system": "{cs}", "concept": [{{"code": "y"}}]}}, {{"system": "{cs}", "concept": [{{"code": "n"}}]}}]}}"#
    );
    let two_systems = format!(
        r#"{{"include": [{{"system": "{other}", "concept": [{{"code": "o"}}]}}, {{"system": "{cs}", "concept": [{{"code": "y"}}]}}]}}"#
    );
    let named = r#", "name": "Beside""#;
    let url = r#", "url": "http://example.org/ValueSet/beside""#;
    let v = Validator::new();
    let verdict = |label: &str, compose: &str, answer: &str| {
        let r = v.validate(qr(label, compose, answer).as_bytes());
        let ids: Vec<_> = r.report.issues.iter().filter(|i| i.path.ends_with("answer[0]")).map(|i| i.id).collect();
        (r.is_valid(), ids)
    };
    let code_only = r#"{"code": "y"}"#;
    // one include: a warning, url or none
    assert_eq!(verdict(named, &one, code_only), (true, vec!["questionnaire.answer-no-system"]));
    // two includes and no url: the error, one system or two
    for compose in [&two_one_system, &two_systems] {
        assert_eq!(verdict(named, compose, code_only), (false, vec!["questionnaire.answer-not-in-set"]), "{compose}");
        // the same set with a url: the warning
        assert_eq!(verdict(url, compose, code_only), (true, vec!["questionnaire.answer-no-system"]), "{compose}");
        // and a code with its system is a member either way
        assert!(verdict(named, compose, &format!(r#"{{"system": "{cs}", "code": "y"}}"#)).0, "{compose}");
    }
    // an enumerated exclude is applied: the excluded code is not a member, with a system or without
    let excluded = format!(
        r#"{{"include": [{{"system": "{cs}", "concept": [{{"code": "y"}}, {{"code": "n"}}]}}], "exclude": [{{"system": "{cs}", "concept": [{{"code": "n"}}]}}]}}"#
    );
    assert_eq!(
        verdict(named, &excluded, &format!(r#"{{"system": "{cs}", "code": "n"}}"#)),
        (false, vec!["questionnaire.answer-not-in-set"])
    );
    assert_eq!(verdict(named, &excluded, r#"{"code": "n"}"#), (false, vec!["questionnaire.answer-not-in-set"]));
    assert_eq!(verdict(named, &excluded, code_only), (true, vec!["questionnaire.answer-no-system"]));
    // a system of whitespace is no system at the answer (the uri finding stands beside it)
    let (_, ids) = verdict(named, &one, r#"{"system": " ", "code": "y"}"#);
    assert_eq!(ids, ["questionnaire.answer-no-system"]);
}
