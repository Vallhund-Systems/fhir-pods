use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use fhir_pods_core::{audit::Kind, deidentify::Policy, node::Node, wire, Validator};

mod output;

const HELP: &str = "fhir-pods — local FHIR tools

Usage:
  fhir-pods validate [--outcome] [FILE|-]
  fhir-pods lower PROFILE_URL [FILE|-]
  fhir-pods lift PROFILE_URL|\"\" [FILE|-]
  fhir-pods deidentify --policy POLICY_FILE [FILE|-]
  fhir-pods document ips|au-ps [FILE|-]
  fhir-pods audit KIND [FILE|-]
  fhir-pods needs [FILE|-]
  fhir-pods --help | --version

Audit kinds: read, query, create, update, delete; or patient- prefixed variants.
Input defaults to stdin. Use -- before positional arguments beginning with -.
POLICY_FILE may be - only when the resource is read from a file.
Results are JSON on stdout. Usage/I/O errors are JSON on stderr.
Exit 0: valid result (deidentify requires no remaining findings; lift no missing slots).
Exit 1: invalid result, remaining findings, missing lift slots, or rejected input.
Exit 2: command usage, policy configuration, or I/O failure.
Warnings, including unresolved terminology, do not change a valid verdict.
";

struct Args {
    command: String,
    parameter: Option<String>,
    input: PathBuf,
    policy: Option<PathBuf>,
    outcome: bool,
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
    if !matches!(command, "validate" | "lower" | "lift" | "deidentify" | "document" | "audit" | "needs") {
        return Err(bad("unknown command; use --help"));
    }
    let mut positional = Vec::new();
    let mut policy = None;
    let mut outcome = false;
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
            Some(s) if !literal && s.starts_with('-') && s != "-" => {
                return Err(bad("unknown or repeated option; use --help"));
            }
            _ => positional.push(arg.clone()),
        }
    }
    let parameter = if matches!(command, "lower" | "lift" | "document" | "audit") {
        if positional.is_empty() {
            return Err(bad(match command {
                "lower" => "lower needs a profile URL",
                "lift" => "lift needs a profile URL or an empty string for a declared profile",
                "document" => "document needs ips or au-ps",
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
        Some(value)
    } else {
        None
    };
    if positional.len() > 1 {
        return Err(bad("only one input file is accepted"));
    }
    let input = positional.pop().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("-"));
    if command == "deidentify" && policy.is_none() {
        return Err(bad("deidentify requires --policy POLICY_FILE"));
    }
    if input.as_os_str() == "-" && policy.as_ref().is_some_and(|p| p.as_os_str() == "-") {
        return Err(bad("resource and policy cannot both use stdin"));
    }
    Ok(Args { command: command.into(), parameter, input, policy, outcome })
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
    // Read/validate policy before consuming the resource stream.
    let policy = args
        .policy
        .as_ref()
        .map(|p| Policy::from_json(&read(p)?).map_err(|error| Failure::Usage(format!("invalid policy: {error}"))))
        .transpose()?;
    let input = read(&args.input)?;
    let validator = Validator::new();
    let rejected = |error: &dyn std::fmt::Display| Failure::Rejected(error.to_string());
    Ok(match args.command.as_str() {
        "validate" => {
            let report = validator.validate(&input);
            (if args.outcome { report.to_operation_outcome() } else { wire::report(&report) }, report.is_valid())
        }
        "lower" => {
            let lowered = validator.lower(args.parameter.as_deref().unwrap(), &input).map_err(|e| rejected(&e))?;
            (output::lowered(&lowered), lowered.validation.is_valid())
        }
        "lift" => {
            let result = validator.lift(args.parameter.as_deref().unwrap(), &input).map_err(|e| rejected(&e))?;
            let valid = result.validation.is_valid() && result.missing.is_empty();
            (output::lifted(&result), valid)
        }
        "deidentify" => {
            let result = validator.deidentify(&input, policy.as_ref().unwrap()).map_err(|e| rejected(&e))?;
            let valid = result.validation.is_valid() && result.remaining.is_empty();
            (output::deidentified(&result), valid)
        }
        "document" => {
            let result = validator.document(args.parameter.as_deref().unwrap(), &input).map_err(|e| rejected(&e))?;
            (output::assembled(&result), result.validation.is_valid())
        }
        "audit" => {
            let result = validator.audit(args.parameter.as_deref().unwrap(), &input).map_err(|e| rejected(&e))?;
            (output::lowered(&result), result.validation.is_valid())
        }
        "needs" => {
            let validation = validator.validate(&input);
            let needs = validator.terminology_needs(&input);
            (output::needs(&needs, &validation), validation.is_valid())
        }
        _ => unreachable!("parse checked the command"),
    })
}

fn write_line(text: &str, stderr: bool) -> io::Result<()> {
    if stderr {
        writeln!(io::stderr().lock(), "{text}")
    } else {
        writeln!(io::stdout().lock(), "{text}")
    }
}

fn main() -> ExitCode {
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
