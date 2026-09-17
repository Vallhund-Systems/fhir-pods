//! JSONL driver for conformance/documents. Each line has id, kind and inputs.
use fhir_pods_core::{
    assemble::Kind,
    json,
    node::Node,
    terminology::{Bundled, Chain, Expansions},
    wire, Validator,
};
use std::io::{self, BufRead};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let validator = Validator::new();
    for line in io::stdin().lock().lines() {
        let line = line?;
        let request = Node::from_value(&json::parse(line.as_bytes())?);
        let id = request.member("id").ok_or("missing id")?;
        let kind = request.member("kind").and_then(Node::as_str).and_then(Kind::parse).ok_or("bad kind")?;
        let inputs = request.member("inputs").ok_or("missing inputs")?.to_json();
        let terminology = request
            .member("terminology")
            .and_then(Node::as_str)
            .map(Expansions::parse)
            .transpose()
            .map_err(|error| format!("bad terminology table at line {}: {}", error.line, error.msg))?;
        match validator.document(kind.name(), inputs.as_bytes()) {
            Ok(result) => {
                let placed = Node::Array(
                    result
                        .placed
                        .iter()
                        .map(|(url, section)| {
                            Node::Object(vec![
                                ("fullUrl".into(), Node::Str(url.clone())),
                                ("section".into(), Node::Str(section.clone())),
                            ])
                        })
                        .collect(),
                );
                let omitted = Node::Array(result.omitted.iter().cloned().map(Node::Str).collect());
                let rechecked = terminology
                    .as_ref()
                    .map(|table| {
                        wire::report(&validator.validate_with(result.document.as_bytes(), &Chain(&[&Bundled, table])))
                    })
                    .unwrap_or_else(|| "null".into());
                let needs = Node::Array(
                    validator
                        .terminology_needs(result.document.as_bytes())
                        .iter()
                        .map(|need| {
                            Node::Object(vec![
                                ("valueSet".into(), Node::Str(need.value_set.clone())),
                                ("system".into(), Node::Str(need.system.clone())),
                                ("code".into(), Node::Str(need.code.clone())),
                            ])
                        })
                        .collect(),
                );
                println!(
                    "{{\"id\":{},\"document\":{},\"placed\":{},\"omitted\":{},\"validation\":{},\"validationWithTerminology\":{},\"needs\":{}}}",
                    id.to_json(),
                    result.document,
                    placed.to_json(),
                    omitted.to_json(),
                    wire::report(&result.validation), rechecked, needs.to_json()
                );
            }
            Err(error) => println!("{{\"id\":{},\"error\":{}}}", id.to_json(), Node::Str(error.to_string()).to_json()),
        }
    }
    Ok(())
}
