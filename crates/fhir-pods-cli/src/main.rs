use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use fhir_pods_core::{
    audit::Kind,
    deidentify::Policy,
    export::Region,
    node::Node,
    terminology::{Bundled, Chain, Expansions, Recorder},
    v2::Options,
    wire, Validator,
};

mod output;
mod stdio;

const HELP: &str = "fhir-pods — local FHIR tools

Usage:
  fhir-pods validate [--outcome] [FILE|-]
  fhir-pods lower PROFILE_URL [FILE|-]
  fhir-pods lift PROFILE_URL|\"\" [FILE|-]
  fhir-pods deidentify --policy POLICY_FILE [FILE|-]
  fhir-pods document ips|au-ps [FILE|-]
  fhir-pods audit KIND [FILE|-]
  fhir-pods export REGION [FILE|-]
  fhir-pods v2 [FILE|-] [--timezone Z|+10:00] [--code-system NAME=URI ...]
  fhir-pods v2 --structure NAME
  fhir-pods needs [FILE|-]
  fhir-pods slots PROFILE_URL
  fhir-pods profiles
  fhir-pods must-support [FILE|-]
  fhir-pods --help | --version

Audit kinds: read, query, create, update, delete; or patient- prefixed variants.
Export regions: au, us, uk, ca, nz, ips. Input: a JSON object with resources, author, date, and optionally audit.
Input defaults to stdin. Use -- before positional arguments beginning with -.
POLICY_FILE may be - only when the resource is read from a file.
--terminology TABLE is accepted by validate, lower, lift, document, audit, export,
v2, needs and must-support. TABLE may be - only with a resource file.
slots, profiles and v2 --structure read no stdin. v2 input is ER7 text; other inputs are JSON.
Results are JSON on stdout. Usage/I/O errors are JSON on stderr.
Exit 0: valid result (deidentify: no remaining findings; lift: no missing; v2: no unplaced;
export: every resource, the document and the audit event valid).
Exit 1: invalid result, remaining findings, missing lift slots, unplaced v2 segments, or rejected input.
Exit 2: command usage, policy/terminology configuration, or I/O failure.
Warnings, including unresolved terminology, do not change a valid verdict.
";

struct Args {
    command: String,
    parameter: Option<String>,
    input: PathBuf,
    policy: Option<PathBuf>,
    outcome: bool,
    v2_options: Options,
    structure: Option<String>,
    terminology: Option<PathBuf>,
}

enum Failure {
    Usage(String),
    Rejected(String),
}

fn parse(args: Vec<OsString>) -> Result<Args, Failure> {
    let bad = |message: &str| Failure::Usage(message.into());
    let Some(command) = args.first().and_then(|a| a.to_str()) else {
        return Err(bad("a command is required; use --help"));
    };
    if !matches!(
        command,
        "validate"
            | "lower"
            | "lift"
            | "deidentify"
            | "document"
            | "audit"
            | "export"
            | "v2"
            | "needs"
            | "slots"
            | "profiles"
            | "must-support"
    ) {
        return Err(bad("unknown command; use --help"));
    }
    let mut positional = Vec::new();
    let mut policy = None;
    let mut outcome = false;
    let mut timezone = None;
    let mut code_systems = Vec::new();
    let mut structure = None;
    let mut terminology = None;
    let mut literal = false;
    let mut tail = args[1..].iter();
    while let Some(arg) = tail.next() {
        match arg.to_str() {
            Some("--") if !literal => literal = true,
            Some("--policy") if !literal && command == "deidentify" => {
                if policy.is_some() {
                    return Err(bad("--policy may only be supplied once"));
                }
                policy = Some(PathBuf::from(tail.next().ok_or_else(|| bad("--policy needs a file path"))?));
            }
            Some("--outcome") if !literal && command == "validate" && !outcome => outcome = true,
            Some("--terminology") if !literal && !matches!(command, "deidentify" | "slots" | "profiles") => {
                if terminology.is_some() {
                    return Err(bad("--terminology may only be supplied once"));
                }
                terminology = Some(PathBuf::from(tail.next().ok_or_else(|| bad("--terminology needs a file path"))?));
            }
            Some("--timezone" | "--code-system" | "--structure") if !literal && command == "v2" => {
                let flag = arg.to_str().unwrap();
                let value = tail.next().and_then(|v| v.to_str()).ok_or_else(|| bad("v2 option needs a UTF-8 value"))?;
                match flag {
                    "--timezone" if timezone.is_none() => timezone = Some(value.to_owned()),
                    "--structure" if structure.is_none() => structure = Some(value.to_owned()),
                    "--code-system" => {
                        let (name, uri) = value.split_once('=').ok_or_else(|| bad("--code-system needs NAME=URI"))?;
                        if code_systems.iter().any(|(n, _)| n == name) {
                            return Err(bad("--code-system names must be unique"));
                        }
                        code_systems.push((name.to_owned(), Node::Str(uri.to_owned())));
                    }
                    _ => return Err(bad("v2 option may only be supplied once")),
                }
            }
            Some(s) if !literal && s.starts_with('-') && s != "-" => {
                return Err(bad("unknown or repeated option; use --help"));
            }
            _ => positional.push(arg.clone()),
        }
    }
    let parameter = if matches!(command, "lower" | "lift" | "document" | "audit" | "export" | "slots") {
        if positional.is_empty() {
            return Err(bad(match command {
                "lower" => "lower needs a profile URL",
                "slots" => "slots needs a profile URL",
                "lift" => "lift needs a profile URL or an empty string for a declared profile",
                "document" => "document needs ips or au-ps",
                "export" => "export needs a region: au, us, uk, ca, nz or ips",
                _ => "audit needs an event kind; use --help",
            }));
        }
        let value = positional.remove(0).into_string().map_err(|_| bad("profile URL/kind must be UTF-8"))?;
        if command == "document" && !matches!(value.as_str(), "ips" | "au-ps") {
            return Err(bad("document kind must be ips or au-ps"));
        }
        if command == "audit" && Kind::parse(&value).is_none() {
            return Err(bad("unknown audit kind; use --help"));
        }
        if command == "export" && Region::parse(&value).is_none() {
            return Err(bad("unknown export region; use --help"));
        }
        Some(value)
    } else {
        None
    };
    if positional.len() > 1 {
        return Err(bad("only one input file is accepted"));
    }
    if structure.is_some()
        && (!positional.is_empty() || timezone.is_some() || !code_systems.is_empty() || terminology.is_some())
    {
        return Err(bad("--structure does not accept an input file, conversion options or terminology"));
    }
    if command == "slots" && !positional.is_empty() {
        return Err(bad("slots accepts a profile URL, not an input file"));
    }
    if command == "profiles" && !positional.is_empty() {
        return Err(bad("profiles takes no arguments"));
    }
    let options = Node::Object(vec![
        ("timezone".into(), timezone.map(Node::Str).unwrap_or(Node::Null)),
        ("codeSystems".into(), Node::Object(code_systems)),
    ]);
    // Use the core's option grammar, before consuming the input stream.
    let v2_options = Options::from_json(options.to_json().as_bytes()).map_err(|e| Failure::Usage(e.to_string()))?;
    let input = positional.pop().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("-"));
    if command == "deidentify" && policy.is_none() {
        return Err(bad("deidentify requires --policy POLICY_FILE"));
    }
    if input.as_os_str() == "-" && policy.as_ref().is_some_and(|p| p.as_os_str() == "-") {
        return Err(bad("resource and policy cannot both use stdin"));
    }
    if input.as_os_str() == "-" && terminology.as_ref().is_some_and(|p| p.as_os_str() == "-") {
        return Err(bad("resource and terminology cannot both use stdin"));
    }
    Ok(Args { command: command.into(), parameter, input, policy, outcome, v2_options, structure, terminology })
}

fn read(path: &std::path::Path) -> Result<Vec<u8>, Failure> {
    let result = if path.as_os_str() == "-" {
        let mut bytes = Vec::new();
        io::stdin().lock().read_to_end(&mut bytes).map(|_| bytes)
    } else {
        std::fs::read(path)
    };
    result.map_err(|error| Failure::Usage(format!("cannot read {}: {error}", path.display())))
}

fn run(args: Args) -> Result<(String, bool), Failure> {
    if args.command == "slots" {
        let url = args.parameter.as_deref().unwrap().split('|').next().unwrap();
        let profile = fhir_pods_core::resource_profile(url)
            .ok_or_else(|| Failure::Rejected("no compiled profile with that URL".into()))?;
        return Ok((output::slots(profile), true));
    }
    if args.command == "profiles" {
        return Ok((fhir_pods_core::catalogue_json(), true));
    }
    if let Some(name) = args.structure {
        let tree = fhir_pods_core::v2::convert::structure_tree(&name)
            .ok_or_else(|| Failure::Rejected(format!("no map for message structure '{name}'")))?;
        return Ok((
            Node::Object(vec![("structure".into(), Node::Str(name)), ("tree".into(), Node::Str(tree))]).to_json(),
            true,
        ));
    }
    // Read/validate policy before consuming the resource stream.
    let policy = args
        .policy
        .as_ref()
        .map(|p| Policy::from_json(&read(p)?).map_err(|error| Failure::Usage(format!("invalid policy: {error}"))))
        .transpose()?;
    let table = args
        .terminology
        .as_ref()
        .map(|p| {
            let bytes = read(p)?;
            let text =
                std::str::from_utf8(&bytes).map_err(|_| Failure::Usage("terminology table must be UTF-8".into()))?;
            Expansions::parse(text)
                .map_err(|e| Failure::Usage(format!("invalid terminology table at line {}: {}", e.line, e.msg)))
        })
        .transpose()?
        .unwrap_or_default();
    let tx = Chain(&[&Bundled, &table]);
    let input = read(&args.input)?;
    let validator = Validator::new();
    let rejected = |error: &dyn std::fmt::Display| Failure::Rejected(error.to_string());
    Ok(match args.command.as_str() {
        "validate" => {
            let report = validator.validate_with(&input, &tx);
            (if args.outcome { report.to_operation_outcome() } else { wire::report(&report) }, report.is_valid())
        }
        "lower" => {
            let lowered =
                validator.lower_with(args.parameter.as_deref().unwrap(), &input, &tx).map_err(|e| rejected(&e))?;
            (output::lowered(&lowered), lowered.validation.is_valid())
        }
        "lift" => {
            let result =
                validator.lift_with(args.parameter.as_deref().unwrap(), &input, &tx).map_err(|e| rejected(&e))?;
            let valid = result.validation.is_valid() && result.missing.is_empty();
            (output::lifted(&result), valid)
        }
        "deidentify" => {
            let result = validator.deidentify(&input, policy.as_ref().unwrap()).map_err(|e| rejected(&e))?;
            let valid = result.validation.is_valid() && result.remaining.is_empty();
            (output::deidentified(&result), valid)
        }
        "document" => {
            let result =
                validator.document_with(args.parameter.as_deref().unwrap(), &input, &tx).map_err(|e| rejected(&e))?;
            (output::assembled(&result), result.validation.is_valid())
        }
        "audit" => {
            let result =
                validator.audit_with(args.parameter.as_deref().unwrap(), &input, &tx).map_err(|e| rejected(&e))?;
            (output::lowered(&result), result.validation.is_valid())
        }
        "export" => {
            let result =
                validator.export_with(args.parameter.as_deref().unwrap(), &input, &tx).map_err(|e| rejected(&e))?;
            (result.to_json(), result.is_valid())
        }
        "v2" => {
            let result = validator.v2_with(&input, &args.v2_options, &tx).map_err(|e| rejected(&e))?;
            let valid = result.validation.is_valid() && result.unplaced.is_empty();
            (output::converted(&result), valid)
        }
        "needs" => {
            let recorder = Recorder::new();
            let validation = validator.validate_with(&input, &Chain(&[&tx, &recorder]));
            (output::needs(&recorder.needs(), &validation), validation.is_valid())
        }
        "must-support" => {
            let validation = validator.validate_with(&input, &tx);
            let gaps = validator.must_support_gaps(&input);
            (output::must_support(&gaps, &validation), validation.is_valid())
        }
        _ => unreachable!("parse checked the command"),
    })
}

fn main() -> ExitCode {
    use stdio::write_line;
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    // Help is only special before --, so a file literally named --help works.
    if args.iter().take_while(|a| *a != OsStr::new("--")).any(|a| a == "--help" || a == "-h") {
        return ExitCode::from(if write_line(HELP, false).is_ok() { 0 } else { 2 });
    }
    if args.len() == 1 && args[0] == "--version" {
        return ExitCode::from(if write_line(concat!("fhir-pods ", env!("CARGO_PKG_VERSION")), false).is_ok() {
            0
        } else {
            2
        });
    }
    let (text, stderr, code) = match parse(args).and_then(run) {
        Ok((text, true)) => (text, false, 0),
        Ok((text, false)) => (text, false, 1),
        Err(failure) => {
            let (error, stderr, code) = match failure {
                Failure::Usage(error) => (error, true, 2),
                Failure::Rejected(error) => (error, false, 1),
            };
            (Node::Object(vec![("error".into(), Node::Str(error))]).to_json(), stderr, code)
        }
    };
    ExitCode::from(if write_line(&text, stderr).is_ok() { code } else { 2 })
}
