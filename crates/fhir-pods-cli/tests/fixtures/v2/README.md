# Synthetic v2 CLI inputs

These four messages come from `crates/fhir-pods-core/examples/v2_corpus.rs`
at v0.9.0 (`40f56be`). They contain synthetic example data only. The fixture
manifest records the original Rust source hash and the exact file hashes.
CR segment separators were converted to LF for readable text diffs; no fields
were changed. Tests also supply CR and CRLF inputs. The core derives IDs from
the complete input bytes, so those versions need not have identical IDs.

The integration suite checks complete forwarding of the core reports and
independent content expectations: two orders, four observations, separate
report references, supplied timezone/code-system values, and mapping-limit
and exit-code behavior. It does not claim a new Java reference-validator run.
