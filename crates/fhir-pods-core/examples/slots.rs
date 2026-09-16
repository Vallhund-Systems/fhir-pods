fn main() {
    for url in std::env::args().skip(1) {
        let p = fhir_pods_core::generated::REGISTRY.iter().find(|p| p.url.ends_with(&url)).expect("profile");
        println!("== {} ==", p.name);
        for s in p.lowering.slots {
            println!(
                "  slot {:<50} {:<16} min {} {}",
                s.path,
                s.ty,
                s.min,
                if s.choices.is_empty() { String::new() } else { format!("{:?}", s.choices) }
            );
        }
        for (path, min) in p.lowering.slices {
            println!("  slice {:<49} min {min}", path);
        }
    }
}
