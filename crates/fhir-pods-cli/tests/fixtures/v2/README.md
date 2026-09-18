# Synthetic v2 CLI inputs

These four messages come from `crates/fhir-pods-core/examples/v2_corpus.rs`
at v0.9.1 (`3f24de0`). They contain synthetic example data only. The fixture
manifest records the original Rust source hash and the exact file hashes.
CR segment separators were converted to LF for readable text diffs; no fields
were changed. Tests also supply CR and CRLF inputs. The core derives IDs from
the complete input bytes, so those versions need not have identical IDs.

The integration suite checks complete forwarding of the core reports and
independent content expectations: two orders, four observations, separate
report references, supplied timezone/code-system values, and mapping-limit
and exit-code behavior. It does not claim a new Java reference-validator run.

The v0.9.1 refresh corrects ADT NK1-7 to `N^Next-of-Kin^HL70131` and
adds the missing IN1 separator: the insured name, relationship, and birth date
now occupy IN1-16, IN1-17, and IN1-18. The CLI assertions check the converted
contact and coverage relationship codes. The ADT bundle still deliberately
fails validation for missing Provenance agent/target and Coverage status;
those exact errors are checked so unrelated fixture errors cannot hide behind
an expected invalid verdict.
