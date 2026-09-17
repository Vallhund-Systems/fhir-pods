use fhir_pods_core::{
    assemble::Assembled,
    deidentify::{Deidentified, Finding},
    lower::Lowered,
    node::Node,
    terminology::Need,
    wire, Validation,
};

pub fn lowered(result: &Lowered) -> String {
    format!("{{\"resource\":{},\"validation\":{}}}", result.resource, wire::report(&result.validation))
}

pub fn deidentified(result: &Deidentified) -> String {
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
    format!(
        "{{\"resource\":{},\"applied\":{},\"validation\":{},\"remaining\":{},\"residual\":{}}}",
        result.resource,
        applied.to_json(),
        wire::report(&result.validation),
        findings(&result.remaining).to_json(),
        findings(&result.residual).to_json()
    )
}

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

pub fn assembled(result: &Assembled) -> String {
    let placed = Node::Array(
        result
            .placed
            .iter()
            .map(|(full, section)| {
                Node::Object(vec![
                    ("fullUrl".into(), Node::Str(full.clone())),
                    ("section".into(), Node::Str(section.clone())),
                ])
            })
            .collect(),
    );
    let omitted = Node::Array(result.omitted.iter().cloned().map(Node::Str).collect());
    format!(
        "{{\"document\":{},\"placed\":{},\"omitted\":{},\"validation\":{}}}",
        result.document,
        placed.to_json(),
        omitted.to_json(),
        wire::report(&result.validation)
    )
}

pub fn needs(needs: &[Need], validation: &Validation) -> String {
    let needs = Node::Array(
        needs
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
    format!("{{\"needs\":{},\"validation\":{}}}", needs.to_json(), wire::report(validation))
}
