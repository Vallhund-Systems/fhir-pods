use fhir_pods_core::{
    assemble::Assembled,
    compiled::CompiledProfile,
    deidentify::{Deidentified, Finding},
    lift::Lifted,
    lower::Lowered,
    node::Node,
    terminology::Need,
    v2::Converted,
    wire, Validation,
};

pub fn lowered(result: &Lowered) -> String {
    format!("{{\"resource\":{},\"validation\":{}}}", result.resource, wire::report(&result.validation))
}

pub fn converted(result: &Converted) -> String {
    let skipped = Node::Array(
        result
            .skipped
            .iter()
            .map(|s| {
                Node::Object(vec![
                    ("at".into(), Node::Str(s.at.clone())),
                    ("target".into(), Node::Str(s.target.clone())),
                    ("why".into(), Node::Str(s.why.clone())),
                ])
            })
            .collect(),
    );
    let strings = |items: &[String]| Node::Array(items.iter().cloned().map(Node::Str).collect()).to_json();
    format!(
        "{{\"bundle\":{},\"validation\":{},\"skipped\":{},\"unplaced\":{},\"unmapped\":{}}}",
        result.bundle,
        wire::report(&result.validation),
        skipped.to_json(),
        strings(&result.unplaced),
        strings(&result.unmapped)
    )
}

pub fn lifted(result: &Lifted) -> String {
    result.to_json()
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

pub fn slots(profile: &CompiledProfile) -> String {
    profile.slots_json()
}

pub fn must_support(gaps: &[&str], validation: &Validation) -> String {
    let gaps = Node::Array(gaps.iter().map(|s| Node::Str((*s).into())).collect());
    format!("{{\"mustSupportGaps\":{},\"validation\":{}}}", gaps.to_json(), wire::report(validation))
}
