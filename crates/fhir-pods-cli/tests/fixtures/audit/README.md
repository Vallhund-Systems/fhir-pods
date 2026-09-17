# Audit CLI expectations

These ten synthetic records are the `audit-<kind>.json` outputs of
`crates/fhir-pods-core/examples/audit_corpus.rs` at v0.6.0 (`26e0486`), with
formatting added for review. They match the corresponding files in the
release conformance corpus. `manifest.json` retains the source revision,
profile, expected verdict and SHA-256 of each original compact JSON file.

The CLI integration tests supply the example's host inputs and compare the
complete returned resource against these fixed expectations. They do not
recompute expectations through the current core, depend on an ignored
`conformance/corpus/`, or need network access / Java to run.

The v0.6.0 release conformance evidence includes these audit records. Copying
these expectations is not a new reference-validator run; the CLI tests verify
command dispatch, serialization and exits.
Changes to expected records should be reviewed against the pinned example
and relevant profile before updating a fixture.
