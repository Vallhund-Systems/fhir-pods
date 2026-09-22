# Fixed lowering conformance examples

Run `JAVA=/path/to/java python3 conformance/lowering/run.py --output /tmp/lowering-proof` from the repository root, with the pinned `conformance/validator_cli.jar` (6.10.4), or pass `--jar PATH`. This uses the sibling audit lane's native ABI/oracle runner.

The 72 fixed synthetic cases cover full forms and wrong JSON kinds for the 26 concrete slot types currently present in profiles accepted by `resource_profile`, a concrete/generic choice pair (the 27th slot type), and primitive extension carriers. Reference, Quantity and CodeableConcept shorthand forms have content assertions. Unknown keys, two alternatives of a choice, fixed-leaf collisions, whole labelled slices alongside fixed or unsupported child keys, and optional gates are separate examples. Each wrong-kind case must produce its intended `value` issue at the supplied slot, so a downstream required-field error alone cannot satisfy the test.

Preservation checks require each supplied value to survive unchanged, while allowing the lowering engine to add fixed leaves and known displays. Complete resources and combined operation reports are retained separately. Every emitted resource is revalidated locally and sent to the reference validator. Coverage inventory fails if a new lowerable slot type has no fixed case;
standalone Extension profiles are excluded because the resource lookup rejects
them. Extension-only types are listed separately in `type-coverage.json`.
This does not assert coverage of every choice alternative or every profile.

On 2026-09-22, after v0.12.1: **72/72 cases pass**, with 70 emitted
resources agreeing with pinned validator 6.10.4, terminology services off.
This includes full/wrong-kind Ratio cases and the extension-only effectiveDateTime
control. The latter is now invalid in both engines following the earlier vs-1
alignment; the historical v0.8.4 result was 69/70. Evidence records the exact
source revision, dirty paths, library/JAR hashes and case hash; see the planning
repository's `reviews/v0.12.1/` for this unreleased review run.
