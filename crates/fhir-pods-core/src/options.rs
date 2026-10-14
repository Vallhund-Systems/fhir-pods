//! Host options: the few judgement calls a host may make for itself, where
//! the default is always what validator_cli 6.10.4 does. Nothing here
//! changes what is *checked* — only how strictly two things the oracle is
//! lenient about are reported:
//!
//! - `unknownProfile` — a declared profile this build has not compiled is a
//!   warning (the oracle's severity, settled as D004: "Profile reference …
//!   has not been checked because it could not be found"). A host that
//!   treats an unverified claim as a failure sets `"error"` and the resource
//!   is invalid: fail closed instead of filtering the issue out.
//! - `externalNarrativeLinks` — a narrative reference that leaves the
//!   resource (`<img src="http://…">`, `<a href="file:///…">`) is accepted,
//!   as the oracle accepts it; an image's source is not even held to the
//!   hyperlink scheme rules, so `javascript:` in an `img src` passes there.
//!   A host that will render the narrative, and does not want it to fetch
//!   or run anything, sets `"refuse"`.
//!
//! Options are data on the [`Validator`](crate::Validator): a call takes the
//! snapshot it was built with. The pods persist them for the calls that
//! follow (`fhir_pods_options`); an empty document restores the defaults.

use alloc::string::String;

use crate::json::Value;

/// What a declared profile this build has not compiled is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnknownProfile {
    /// The oracle's severity, and the default: the resource stays valid and
    /// the finding names the profile (`profile.unknown`).
    #[default]
    Warning,
    /// Fail closed: the same finding, at `error`, so the resource is invalid.
    Error,
}

/// Whether a narrative reference out of the resource is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExternalNarrativeLinks {
    /// The oracle's behaviour, and the default.
    #[default]
    Allow,
    /// An `<img src>` or `<a href>` that is neither a `#fragment` resolving
    /// inside the resource nor a self-contained `data:` URL is an error
    /// (`narrative.external-link`).
    Refuse,
}

/// The host options in force. [`Default`] is the oracle's behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    pub unknown_profile: UnknownProfile,
    pub external_narrative_links: ExternalNarrativeLinks,
}

impl Options {
    /// From JSON: `{"unknownProfile": "warning" | "error",
    /// "externalNarrativeLinks": "allow" | "refuse"}`. Every member is
    /// optional and takes its default when absent; an empty document (or no
    /// bytes at all) is the defaults. An unknown member or value is refused
    /// outright — a typo in a strictness setting must not read as "off".
    pub fn from_json(bytes: &[u8]) -> Result<Options, OptionsError> {
        if bytes.iter().all(|b| b.is_ascii_whitespace()) {
            return Ok(Options::default());
        }
        let v = crate::json::parse(bytes).map_err(|_| OptionsError::NotAnObject)?;
        let Value::Object(members) = &v else { return Err(OptionsError::NotAnObject) };
        let mut options = Options::default();
        for (key, value) in members {
            let Value::Str(text) = value else { return Err(OptionsError::BadValue(key.as_ref().into())) };
            match (key.as_ref(), text.as_ref()) {
                ("unknownProfile", "warning") => options.unknown_profile = UnknownProfile::Warning,
                ("unknownProfile", "error") => options.unknown_profile = UnknownProfile::Error,
                ("externalNarrativeLinks", "allow") => options.external_narrative_links = ExternalNarrativeLinks::Allow,
                ("externalNarrativeLinks", "refuse") => {
                    options.external_narrative_links = ExternalNarrativeLinks::Refuse
                }
                ("unknownProfile" | "externalNarrativeLinks", _) => {
                    return Err(OptionsError::BadValue(key.as_ref().into()))
                }
                _ => return Err(OptionsError::UnknownMember(key.as_ref().into())),
            }
        }
        Ok(options)
    }

    /// Whether a narrative reference out of the resource is refused.
    pub(crate) fn refuse_external_narrative_links(&self) -> bool {
        self.external_narrative_links == ExternalNarrativeLinks::Refuse
    }
}

/// Why an options document was refused. The previous options stay in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionsError {
    /// Not a JSON object.
    NotAnObject,
    /// A member no option defines.
    UnknownMember(String),
    /// A member whose value is not one this option takes.
    BadValue(String),
}

impl core::fmt::Display for OptionsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            OptionsError::NotAnObject => f.write_str("options must be a JSON object"),
            OptionsError::UnknownMember(k) => write!(f, "'{k}' is not an option"),
            OptionsError::BadValue(k) => write!(f, "'{k}' does not take that value"),
        }
    }
}

impl core::error::Error for OptionsError {}
