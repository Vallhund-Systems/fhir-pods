use fhir_pods_core::terminology::{Bundled, Chain, Expansions, Membership, Recorder, TerminologyProvider};
use fhir_pods_core::{wire, Validator};
use std::cell::Cell;

const URL: &str = "https://forms.example.invalid/Questionnaire/versioned";
const SET: &str = "https://forms.example.invalid/ValueSet/shared";
const SYSTEM: &str = "https://forms.example.invalid/CodeSystem/shared";

fn form(version: &str) -> String {
    format!(
        r#"{{"resourceType":"Questionnaire","id":"form-{version}","url":"{URL}","version":"{version}","status":"active","item":[{{"linkId":"answer","type":"choice","required":true,"answerValueSet":"{SET}"}}]}}"#
    )
}
fn table(code: &str) -> String {
    format!("fhir-pods-terminology 1\n= {SET}\n{SYSTEM}\t{code}\n")
}
fn context(questionnaire: &str, terminology: &str) -> String {
    let mut quoted = String::from("\"");
    wire::escape(&mut quoted, terminology);
    quoted.push('"');
    format!(
        r#"{{"format":"fhir-pods.questionnaire-context.v1","questionnaire":{questionnaire},"terminology":{quoted}}}"#
    )
}
fn response(version: &str, code: &str) -> String {
    format!(
        r#"{{"resourceType":"QuestionnaireResponse","id":"response-{version}","questionnaire":"{URL}|{version}","status":"completed","item":[{{"linkId":"answer","answer":[{{"valueCoding":{{"system":"{SYSTEM}","code":"{code}"}}}}]}}]}}"#
    )
}
fn loaded() -> Validator {
    let mut validator = Validator::new();
    assert!(validator.load_questionnaire(context(&form("1"), &table("one")).as_bytes()));
    assert!(validator.load_questionnaire(context(&form("2"), &table("two")).as_bytes()));
    validator
}
fn collection(first: &str, second: &str) -> String {
    format!(
        r#"{{"resourceType":"Bundle","id":"inner","type":"collection","entry":[{{"fullUrl":"https://forms.example.invalid/Questionnaire/form-1","resource":{}}},{{"fullUrl":"https://forms.example.invalid/Questionnaire/form-2","resource":{}}},{{"fullUrl":"https://forms.example.invalid/QuestionnaireResponse/response-1","resource":{first}}},{{"fullUrl":"https://forms.example.invalid/QuestionnaireResponse/response-2","resource":{second}}}]}}"#,
        form("1"),
        form("2")
    )
}

#[test]
fn each_exact_questionnaire_version_has_its_own_complete_versionless_table() {
    let validator = loaded();
    for (version, own, other) in [("1", "one", "two"), ("2", "two", "one")] {
        assert!(validator.validate(response(version, own).as_bytes()).is_valid());
        let bad = validator.validate(response(version, other).as_bytes());
        assert!(!bad.is_valid());
        assert!(bad.report.issues.iter().any(|issue| issue.id == "questionnaire.answer-not-in-set"));
    }
}

#[test]
fn collection_and_nested_collection_keep_each_response_context() {
    let validator = loaded();
    for nested in [false, true] {
        let wrap = |value: String| {
            if nested {
                format!(
                    r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"fullUrl":"https://forms.example.invalid/Bundle/inner","resource":{value}}}]}}"#
                )
            } else {
                value
            }
        };
        let good = validator.validate(wrap(collection(&response("1", "one"), &response("2", "two"))).as_bytes());
        assert!(good.is_valid(), "{}", good.to_operation_outcome());
        let bad = validator.validate(wrap(collection(&response("1", "two"), &response("2", "one"))).as_bytes());
        let errors: Vec<_> =
            bad.report.issues.iter().filter(|issue| issue.id == "questionnaire.answer-not-in-set").collect();
        assert_eq!(errors.len(), 2, "{}", bad.to_operation_outcome());
        assert!(errors.iter().all(|issue| issue.path.contains("entry[")));
    }
}

struct ForeignAuthority(Cell<usize>);
impl TerminologyProvider for ForeignAuthority {
    fn validate_code(&self, _: &str, _: &str, _: &str) -> Membership {
        self.0.set(self.0.get() + 1);
        Membership::Member
    }
}

#[test]
fn a_scoped_table_never_borrows_membership_from_another_global_authority() {
    let validator = loaded();
    let foreign = ForeignAuthority(Cell::new(0));
    let bad = validator.validate_with(response("1", "two").as_bytes(), &Chain(&[&Bundled, &foreign]));
    assert!(!bad.is_valid());
    assert_eq!(foreign.0.get(), 0);
}

#[test]
fn an_unknown_scoped_set_stays_unknown_and_is_reported_as_a_terminology_need() {
    let mut validator = Validator::new();
    assert!(validator.load_questionnaire(context(&form("1"), "fhir-pods-terminology 1\n").as_bytes()));
    let foreign = ForeignAuthority(Cell::new(0));
    let recorder = Recorder::new();
    let result = validator.validate_with(response("1", "one").as_bytes(), &Chain(&[&Bundled, &foreign, &recorder]));
    assert!(result
        .report
        .issues
        .iter()
        .any(|issue| issue.severity == fhir_pods_core::validate::Severity::Warning && issue.id == "binding.unknown"));
    assert_eq!(foreign.0.get(), 0);
    assert_eq!(recorder.needs().len(), 1);
    assert_eq!(recorder.needs()[0].value_set, SET);
    assert_eq!(validator.terminology_needs(response("1", "one").as_bytes()).len(), 1);
}

#[test]
fn malformed_context_loads_do_not_replace_either_previous_version() {
    let mut validator = loaded();
    let good = context(&form("1"), &table("two"));
    let cases = [
        good.replace("fhir-pods.questionnaire-context.v1", "unknown-format"),
        good.replace("\"terminology\":", "\"unexpected\":true,\"terminology\":"),
        good.replace("\"format\":", "\"format\":\"fhir-pods.questionnaire-context.v1\",\"format\":"),
        context(&form("1"), "not a terminology table"),
        context(&form("1").replace("\"version\":\"1\",", ""), &table("two")),
        context(&form("1").replace(&format!("\"url\":\"{URL}\","), ""), &table("two")),
        context(&form("1").replace("\"version\":\"1\",", "\"version\":\"1\",\"version\":\"2\","), &table("two")),
        context(
            &form("1").replace(
                &format!("\"url\":\"{URL}\","),
                &format!("\"url\":\"{URL}\",\"url\":\"https://other.example.invalid/form\","),
            ),
            &table("two"),
        ),
    ];
    for malformed in cases {
        assert!(!validator.load_questionnaire(malformed.as_bytes()), "{malformed}");
        assert!(validator.validate(response("1", "one").as_bytes()).is_valid());
        assert!(!validator.validate(response("1", "two").as_bytes()).is_valid());
        assert!(validator.validate(response("2", "two").as_bytes()).is_valid());
    }
}

#[test]
fn replacing_or_clearing_forms_drops_only_their_owned_context() {
    let mut validator = loaded();
    let global = Expansions::parse(&table("two")).unwrap();
    assert!(validator.load_questionnaire(form("1").as_bytes()));
    assert!(validator.validate_with(response("1", "two").as_bytes(), &Chain(&[&Bundled, &global])).is_valid());
    assert!(!validator.validate_with(response("2", "one").as_bytes(), &Chain(&[&Bundled, &global])).is_valid());
    validator.clear_questionnaires();
    assert!(validator.load_questionnaire(form("2").as_bytes()));
    assert!(validator
        .validate_with(response("2", "one").as_bytes(), &Chain(&[&Bundled, &Expansions::parse(&table("one")).unwrap()]))
        .is_valid());
}

#[test]
fn scoped_contexts_require_an_explicit_response_version_without_changing_legacy_resolution() {
    let mut validator = loaded();
    let unversioned = response("1", "one").replace(&format!("{URL}|1"), URL);
    let result = validator.validate(unversioned.as_bytes());
    assert!(result.report.issues.iter().any(|issue| issue.id == "questionnaire.context-version-required"));
    validator.clear_questionnaires();
    assert!(validator.load_questionnaire(form("1").as_bytes()));
    let global = Expansions::parse(&table("one")).unwrap();
    assert!(validator.validate_with(unversioned.as_bytes(), &Chain(&[&Bundled, &global])).is_valid());
}

#[test]
fn scoped_tables_preserve_the_existing_bundled_standard_sets() {
    let mut validator = Validator::new();
    let questionnaire = form("1").replace(SET, "http://hl7.org/fhir/ValueSet/administrative-gender");
    assert!(validator.load_questionnaire(context(&questionnaire, "fhir-pods-terminology 1\n").as_bytes()));
    let json = response("1", "male").replace(SYSTEM, "http://hl7.org/fhir/administrative-gender");
    assert!(validator.validate(json.as_bytes()).is_valid());
    assert!(validator.terminology_needs(json.as_bytes()).is_empty());
}
