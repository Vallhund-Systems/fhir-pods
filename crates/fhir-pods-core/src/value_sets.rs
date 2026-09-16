//! Offline membership for the small base-FHIR value sets we can legally bundle.
//! `None` for a set we don't ship — the engine degrades that to a warning.
//! SNOMED-bound sets are intentionally absent: they resolve at runtime against
//! a licensed seam.

pub fn in_value_set(url: &str, code: &str) -> Option<bool> {
    let base = url.split('|').next().unwrap_or(url);
    match base {
        "http://hl7.org/fhir/ValueSet/administrative-gender" => {
            Some(matches!(code, "male" | "female" | "other" | "unknown"))
        }
        "http://hl7.org/fhir/ValueSet/observation-status" => Some(matches!(
            code,
            "registered"
                | "preliminary"
                | "final"
                | "amended"
                | "corrected"
                | "cancelled"
                | "entered-in-error"
                | "unknown"
        )),
        "http://hl7.org/fhir/ValueSet/name-use" => {
            Some(matches!(code, "usual" | "official" | "temp" | "nickname" | "anonymous" | "old" | "maiden"))
        }
        "http://hl7.org/fhir/ValueSet/link-type" => {
            Some(matches!(code, "replaced-by" | "replaces" | "refer" | "seealso"))
        }
        "http://hl7.org/fhir/ValueSet/quantity-comparator" => Some(matches!(code, "<" | "<=" | ">=" | ">")),
        // The vital-signs UCUM unit sets, concept lists taken verbatim from
        // hl7.fhir.r4.core#4.0.1 (CC0). UCUM is freely embeddable.
        "http://hl7.org/fhir/ValueSet/ucum-vitals-common" => Some(matches!(
            code,
            "%" | "cm" | "[in_i]" | "kg" | "g" | "[lb_av]" | "Cel" | "[degF]" | "mm[Hg]" | "/min" | "kg/m2" | "m2"
        )),
        "http://hl7.org/fhir/ValueSet/ucum-bodyweight" => Some(matches!(code, "kg" | "[lb_av]" | "g")),
        "http://hl7.org/fhir/ValueSet/ucum-bodylength" => Some(matches!(code, "cm" | "[in_i]")),
        "http://hl7.org/fhir/ValueSet/ucum-bodytemp" => Some(matches!(code, "Cel" | "[degF]")),
        _ => None,
    }
}
