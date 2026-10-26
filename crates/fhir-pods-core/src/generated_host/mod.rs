// The host's own compiled profiles — a stub until scripts/gen-host-profiles.sh
// writes the registry for a host package (see README, "Host profiles"). Built
// in with `--features host-profiles`; the shipped registry stays as it is.
use crate::compiled::CompiledProfile;

pub static REGISTRY: &[&CompiledProfile] = &[];

pub static DATATYPE_PROFILE_ELEMENTS: &[(&str, &str, &str, &str, &str)] = &[];

pub static SOURCE_SHA256: &[(&str, &str)] = &[];
