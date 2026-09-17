//! JSONL driver for conformance/deid. Each line has id, resource and policy.
use fhir_pods_core::{
    deidentify::{self, Finding, Policy},
    json,
    node::Node,
    wire, Validator,
};
use std::io::{self, BufRead};

fn findings(items: &[Finding]) -> Node {
    Node::Array(
        items
            .iter()
            .map(|f| {
                Node::Object(vec![
                    ("path".into(), Node::Str(f.path.clone())),
                    ("what".into(), Node::Str(f.what.clone())),
                ])
            })
            .collect(),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let validator = Validator::new();
    for line in io::stdin().lock().lines() {
        let line = line?;
        let request = Node::from_value(&json::parse(line.as_bytes())?);
        let id = request.member("id").ok_or("missing id")?;
        let input = request.member("resource").ok_or("missing resource")?.to_json();
        let policy = Policy::from_json(request.member("policy").ok_or("missing policy")?.to_json().as_bytes())?;
        match validator.deidentify(input.as_bytes(), &policy) {
            Ok(result) => {
                let applied = Node::Array(
                    result
                        .applied
                        .iter()
                        .map(|a| {
                            Node::Object(vec![
                                ("path".into(), Node::Str(a.path.clone())),
                                ("action".into(), Node::Str(a.action.name().into())),
                            ])
                        })
                        .collect(),
                );
                println!("{{\"id\":{},\"inputValidation\":{},\"checkBefore\":{},\"resource\":{},\"applied\":{},\"validation\":{},\"remaining\":{},\"checkAfter\":{}}}",
                    id.to_json(), wire::report(&validator.validate(input.as_bytes())),
                    findings(&deidentify::check(input.as_bytes())).to_json(), result.resource,
                    applied.to_json(), wire::report(&result.validation), findings(&result.remaining).to_json(),
                    findings(&deidentify::check(result.resource.as_bytes())).to_json());
            }
            Err(error) => println!("{{\"id\":{},\"error\":{}}}", id.to_json(), Node::Str(error.to_string()).to_json()),
        }
    }
    Ok(())
}
