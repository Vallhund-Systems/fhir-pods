# Fixed audit conformance examples

Run `JAVA=/path/to/java python3 conformance/audit/run.py --output /tmp/audit-proof` from the repository root. Obtain the pinned `conformance/validator_cli.jar` (6.10.4) first, or pass `--jar PATH`. The runner builds the native ABI, exercises the deterministic synthetic cases, checks preservation and operation outcomes, then passes every emitted resource to the pinned oracle with remote terminology disabled.

The 123 cases cover all ten BALP kinds with/without a user, every reachable public Missing/Unexpected/Malformed field branch, unknown fields/kinds, accepted actor/network representations, standard address classification, Unicode query bytes, explicit default overrides and profile/fixed-leaf assertions. The uncompiled-profile error is unreachable with this compiled registry. The suite is finite; it does not claim broad input-space exploration or complete FHIR conformance.

On v0.8.4 (`1860d48`): 121/123 expectations pass; 62 resources are emitted and compared. Two incorrect supplied coding-display witnesses remain red (F058 in the planning repo), while their correct-display controls pass. The suite intentionally exits 1 for these differences. Input rejection and combined lowering errors are scored separately from the independently revalidated emitted resource; only that resource verdict is compared with the oracle.

Evidence is written to the explicit output folder. Commit synthetic fixtures/scripts only; keep generated run directories outside this source tree. The sibling lowering lane reuses `run_suite` and the native ABI adapter. The reference outcome parser is shared read-only with the documents lane and rejects missing/duplicate/unidentified outputs. Package versions and JDK identity are recorded with the planning report.
