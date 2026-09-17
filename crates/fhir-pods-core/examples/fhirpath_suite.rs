//! Official-suite adapter over the existing public invariant API. No engine edits.
//! Read cases.jsonl from stdin; emit one JSON result per case, preserving IDs.
use fhir_pods_core::document::{Document, Parsed};
use fhir_pods_core::json::Value;
use fhir_pods_core::node::Node;
use fhir_pods_core::path::{self, Verdict};
use std::io::{self, BufRead};

fn member<'v, 'a>(v: &'v Value<'a>, key: &str) -> Option<&'v Value<'a>> {
    let Value::Object(fields) = v else { return None };
    fields.iter().find(|(k, _)| k == key).map(|(_, value)| value)
}
fn text<'v, 'a>(v: &'v Value<'a>, key: &str) -> &'v str {
    match member(v, key) {
        Some(Value::Str(s)) => s.as_ref(),
        _ => "",
    }
}
fn flag(v: &Value<'_>, key: &str) -> bool {
    matches!(member(v, key), Some(Value::Bool(true)))
}
fn quoted(s: &str) -> String {
    format!(
        "'{}'",
        s.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
    )
}

fn comparison(item: &str, output: &Value<'_>) -> Result<String, String> {
    let ty = text(output, "type");
    let value = text(output, "value");
    let (types, literal, projected) = match ty {
        // The suite's primitive outputs may be a FHIR element or a computed
        // System value. Their namespaces remain distinct; do not require a
        // FHIR element to pass an `is System.*` assertion.
        "boolean" => (vec!["FHIR.boolean", "System.Boolean"], value.to_string(), item.to_string()),
        "integer" => (vec!["FHIR.integer", "System.Integer"], value.to_string(), item.to_string()),
        "string" => (vec!["FHIR.string", "System.String"], quoted(value), item.to_string()),
        "code" => (vec!["FHIR.code"], quoted(value), item.to_string()),
        "id" => (vec!["FHIR.id"], quoted(value), item.to_string()),
        // Date outputs in this suite use @YYYY[-MM[-DD]]. Comparing the
        // precision-preserving string avoids needing an unsupported @ literal.
        // DateTime, decimals and quantities need richer comparison semantics;
        // they are not silently compared as arbitrary strings.
        "date" => {
            let value = value.strip_prefix('@').unwrap_or(value);
            (vec!["FHIR.date", "System.Date"], quoted(value), format!("{item}.toString()"))
        }
        _ => return Err(format!("output-comparison-not-exposed: {ty}")),
    };
    let types = types.iter().map(|ty| format!("({item} is {ty})")).collect::<Vec<_>>().join(" or ");
    Ok(format!("({types}) and ({projected} = {literal})"))
}

fn evaluate(case: &Value<'_>) -> (&'static str, String, Option<String>) {
    let expression = text(case, "expression");
    if path::parse(expression).is_err() {
        return (
            "unsupported",
            "parser-rejected: public API combines invalid syntax and unsupported syntax".into(),
            None,
        );
    }
    if text(case, "invalid") == "syntax" {
        return ("fail", "accepted-invalid-syntax".into(), None);
    }
    if text(case, "mode") == "strict" || text(case, "invalid") == "semantic" {
        return ("unsupported", "static-semantic-checking-not-exposed".into(), None);
    }
    let input = member(case, "input").unwrap_or(&Value::Null);
    if matches!(input, Value::Null) && (expression.contains('$') || expression.contains('%')) {
        return ("unsupported", "resource-free-context-variables-not-representable".into(), None);
    }
    let input_json = Node::from_value(input).to_json();
    let parsed = match Parsed::parse(input_json.as_bytes()) {
        Ok(value) => value,
        Err(error) => return ("fail", format!("fixture-adapter-error: {error}"), None),
    };
    let doc = parsed.doc();
    let root = (!matches!(input, Value::Null)).then_some(&doc);
    let bundle = (doc.scalars("$this.resourceType") == ["Bundle"]).then_some(&doc);
    let check = |source: &str| match path::parse(source) {
        Ok(ast) => path::check_in(&ast, &doc, doc.fhir_type(), root, bundle),
        Err(_) => Verdict::Unsupported,
    };
    if text(case, "invalid") == "execution" {
        let actual = check(&format!("({expression}).count() >= 0"));
        return match actual {
            Verdict::Unsupported => {
                ("unsupported", "execution-rejected: cannot distinguish unsupported from expected error".into(), None)
            }
            _ => ("fail", "accepted-invalid-execution".into(), None),
        };
    }
    // First exercise the expression without output comparison. This separates
    // an engine evaluation limit from an adapter that cannot compare a result.
    if check(&format!("({expression}).count() >= 0")) == Verdict::Unsupported {
        return ("unsupported", "engine-evaluation-unsupported".into(), None);
    }
    let Some(Value::Array(expected)) = member(case, "expected") else {
        return ("fail", "fixture-missing-expected-array".into(), None);
    };
    if flag(case, "predicate") {
        if expected.len() != 1 || text(&expected[0], "type") != "boolean" {
            return ("unsupported", "predicate-output-shape".into(), None);
        }
        // Predicate conversion maps empty to false; check_in alone maps it to
        // Pass because it implements invariant semantics. Make that explicit.
        let assertion = format!("(({expression}).exists() and ({expression})) = {}", text(&expected[0], "value"));
        return match check(&assertion) {
            Verdict::Unsupported => ("unsupported", "predicate-evaluation".into(), Some(assertion)),
            Verdict::Pass => ("pass", "predicate".into(), Some(assertion)),
            Verdict::Fail => ("fail", "predicate-mismatch".into(), Some(assertion)),
        };
    }
    // A cardinality comparison produces a definite boolean, so an empty result
    // cannot accidentally pass via invariant truthiness. tail().first() checks
    // each ordered output without extending the evaluator to support indexing.
    // Mixed type predicates may overlap (code also satisfies string). Counting
    // them independently cannot prove a one-to-one unordered match.
    if !flag(case, "ordered") && expected.iter().any(|v| text(v, "type") != text(&expected[0], "type")) {
        return ("unsupported", "unordered-mixed-type-comparison-not-exposed".into(), None);
    }
    let mut clauses = vec![format!("({expression}).count() = {}", expected.len())];
    for (i, output) in expected.iter().enumerate() {
        if flag(case, "ordered") {
            let item = format!("({expression}){}.first()", ".tail()".repeat(i));
            match comparison(&item, output) {
                Ok(clause) => clauses.push(format!("({clause})")),
                Err(reason) => return ("unsupported", reason, None),
            }
        } else {
            // Count each expected value's occurrences, including duplicates.
            // Total cardinality also guards against unexpected extra values.
            let multiplicity = expected
                .iter()
                .filter(|other| {
                    text(other, "type") == text(output, "type") && text(other, "value") == text(output, "value")
                })
                .count();
            match comparison("$this", output) {
                Ok(clause) => clauses.push(format!("({expression}).where({clause}).count() = {multiplicity}")),
                Err(reason) => return ("unsupported", reason, None),
            }
        }
    }
    let assertion = clauses.join(" and ");
    // A present primitive carrier can have no value. Equality then yields
    // empty even though count() is one: do not accept that as invariant truth.
    let assertion = format!("({assertion}).exists() and ({assertion})");
    match check(&assertion) {
        Verdict::Pass => ("pass", "cardinality-type-value-and-order-or-multiplicity".into(), Some(assertion)),
        Verdict::Fail => ("fail", "output-mismatch".into(), Some(assertion)),
        Verdict::Unsupported => ("unsupported", "comparison-unsupported-after-evaluation".into(), Some(assertion)),
    }
}

fn main() {
    for line in io::stdin().lock().lines() {
        let line = line.expect("read case line");
        let case = fhir_pods_core::json::parse(line.as_bytes()).expect("valid case JSON");
        let (status, reason, assertion) = evaluate(&case);
        let result = Node::Object(vec![
            ("id".into(), Node::Str(text(&case, "id").into())),
            ("status".into(), Node::Str(status.into())),
            ("reason".into(), Node::Str(reason)),
            ("assertion".into(), assertion.map(Node::Str).unwrap_or(Node::Null)),
        ]);
        println!("{}", result.to_json());
    }
}
