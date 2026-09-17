//! The HL7 v2 proof. Synthetic messages (no real data) of the structures the
//! pod converts, each written as `v2-<structure>.json` with a manifest
//! carrying our own verdict on the bundle, so the conformance harness can put
//! every bundle in front of the reference validator: where the message
//! carries what FHIR requires we must both say valid; where it does not
//! (an ADT's Provenance with no agent) we must both say invalid — the pod
//! invents nothing to close the gap.
//!
//!     cargo run -p fhir-pods-core --example v2_corpus -- conformance/corpus
use std::{env, fs};

use fhir_pods_core::v2::convert::Options;
use fhir_pods_core::{wire, Validator};

const ORU_R01: &str = "MSH|^~\\&|LAB|ACME^1.2.3^ISO|EHR|CLINIC|20260916100000+1000||ORU^R01^ORU_R01|MSG0001|P|2.5.1\r\
PID|1||MRN12345^^^ACME^MR||Doe^Jane^A||19700615|F|||12 High St^^Springfield^VIC^3000^AU||^PRN^PH^^^^0400000000\r\
PV1|1|O|WARD1^BED2^^ACME||||1234^Who^Doctor^^^Dr^^^ACME&1.2.3&ISO\r\
ORC|RE|PLC001^EHR|FIL001^LAB\r\
OBR|1|PLC001^EHR|FIL001^LAB|24331-1^Lipid panel^LN|||20260916090000+1000||||||||||||||||||F\r\
OBX|1|NM|2093-3^Cholesterol^LN||5.2|mmol/L^millimole per liter^UCUM|<5.5|N|||F|||20260916093000+1000\r\
OBX|2|NM|2571-8^Triglycerides^LN||1.1|mmol/L^millimole per liter^UCUM|<2.0|N|||F|||20260916093000+1000\r\
NTE|1||Fasting sample\r\
ORC|RE|PLC002^EHR|FIL002^LAB\r\
OBR|2|PLC002^EHR|FIL002^LAB|58410-2^CBC panel^LN|||20260916090000+1000||||||||||||||||||F\r\
OBX|1|NM|718-7^Hemoglobin^LN||140|g/L^gram per liter^UCUM|130-170|N|||F|||20260916094000+1000\r\
OBX|2|ST|11475-1^Microorganism^LN||No growth||||||F\r";

const ADT_A01: &str = "MSH|^~\\&|ADT1|GOOD HEALTH HOSPITAL|GHH LAB, INC.|GOOD HEALTH HOSPITAL|20260916103000+1000||ADT^A01^ADT_A01|MSG00001|P|2.8\r\
EVN|A01|20260916102000+1000||||20260916101500+1000\r\
PID|1||PATID1234^^^GOOD HEALTH HOSPITAL^MR||Everyman^Adam^A^III||19610615|M||2106-3^White^HL70005|2222 Home Street^^Greensboro^NC^27401-1020^US|GL|(555) 555-2004|(555)555-2004||S||PATID12345001^2^M10|123456789\r\
NK1|1|Nuclear^Nelda^W|SPO^Spouse^HL70063||(555) 555-2004||NK^Next of kin^HL70131\r\
PV1|1|I|2000^2012^01^GOOD HEALTH HOSPITAL||||004777^Attend^Aaron^A|||SUR||||ADM|A0|||||||||||||||||||||||||||||20260916101500+1000\r\
AL1|1||^Penicillin||Produces hives~Rash\r\
AL1|2||^Cat dander|SV|Anaphylaxis\r\
IN1|1|PLAN001^^^GH^PLAN|INS001|Good Health Insurance|||||||||||Everyman^Adam^A|SEL|19610615\r";

const VXU_V04: &str = "MSH|^~\\&|IMM|CLINIC|IIS|STATE|20260916110000+1000||VXU^V04^VXU_V04|MSG0002|P|2.5.1\r\
PID|1||MRN777^^^CLINIC^MR||Child^Casey||20200101|F|||1 Park Rd^^Springfield^VIC^3000^AU\r\
NK1|1|Child^Pat|MTH^Mother^HL70063\r\
ORC|RE||IMM0001^CLINIC\r\
RXA|0|1|20260916103000+1000||08^Hep B, adolescent or pediatric^CVX|0.5|mL^^UCUM||00^New immunization record^NIP001||||||LOT123|20271231|MSD^Merck^MVX|||CP|A\r";

const OML_O21: &str = "MSH|^~\\&|EHR|CLINIC|LAB|ACME|20260916120000+1000||OML^O21^OML_O21|MSG0003|P|2.5.1\r\
PID|1||MRN12345^^^ACME^MR||Doe^Jane^A||19700615|F\r\
PV1|1|O\r\
ORC|NW|PLC003^EHR|||||||20260916115500+1000|1234^Who^Doctor||1234^Who^Doctor\r\
OBR|1|PLC003^EHR||24331-1^Lipid panel^LN||20260916115500+1000\r\
NTE|1||Patient fasting since midnight\r";

fn main() {
    let dir = env::args().nth(1).unwrap_or_else(|| "conformance/corpus".into());
    let validator = Validator::new();
    let mut manifest_lines = Vec::new();
    let mut not_valid = 0;
    let two_patients = format!(
        "{ORU_R01}PID|2||MRN99999^^^ACME^MR||Roe^Richard||19800101|M\r\
ORC|RE|PLC-B1^EHR|FIL-B1^LAB\r\
OBR|1|PLC-B1^EHR|FIL-B1^LAB|24331-1^Lipid panel^LN|||20260916090000+1000||||||||||||||||||F\r\
OBX|1|NM|2093-3^Cholesterol^LN||6.9|mmol/L^millimole per liter^UCUM|<5.5|H|||F|||20260916093000+1000\r"
    );
    for (name, message) in [
        ("oru-r01", ORU_R01),
        ("oru-r01-two-patients", two_patients.as_str()),
        ("adt-a01", ADT_A01),
        ("vxu-v04", VXU_V04),
        ("oml-o21", OML_O21),
    ] {
        let c = validator.v2(message.as_bytes(), &Options::default()).expect("a synthetic message converts");
        let valid = c.validation.is_valid();
        if !valid {
            not_valid += 1;
            eprintln!("v2-{name}: not valid on our side (recorded as such):\n{}", wire::report(&c.validation));
        }
        for u in &c.unplaced {
            eprintln!("v2-{name}: unplaced {u}");
        }
        fs::write(format!("{dir}/v2-{name}.json"), format!("{}\n", c.bundle)).unwrap();
        manifest_lines
            .push(format!("{{\"file\": \"v2-{name}.json\", \"profile\": \"\", \"expect\": {valid}, \"ig\": \"v2\"}}"));
        println!(
            "{:<44} {} ({} skipped)",
            format!("v2-{name}.json"),
            if valid { "ok" } else { "not valid" },
            c.skipped.len()
        );
    }
    fs::write(format!("{dir}/v2-manifest.json"), format!("[\n{}\n]\n", manifest_lines.join(",\n"))).unwrap();
    println!("v2 bundles: {}; {not_valid} not valid on our side (their own verdict recorded)", manifest_lines.len());
}
