# Adversarial validation probes

These 82 synthetic fixtures cover valid controls, JSON shapes, primitive limits,
extensions, contained resources, unknown profiles, offline references, required
coded content, recursive content-reference validation, the shape a
de-identified age over 89 goes out as, the datatype profile R4 puts on
an element (`sq-*`: a `SimpleQuantity` with and without a comparator), and an
IG's own datatype profiles (`nz-*`, `ca-*`, `au-*`: `NzAddress`,
`NzContactpoint`, CA's `profile-identifier`, `au-address` and the AU
identifier profiles), and `resolve()` (`resolve-*`: `ctm-1` and US Core
`provenance-1` on a relative, an absolute, a `urn:uuid:` and a `#missing`
reference nothing resolves — the empty collection, evaluated — and on a
contained target that does resolve, among them AU Core heart rate's subject
as a contained Patient that is no AU Core Patient).
`manifest.json` records an expected verdict and reason for each fixture. The
shared corpus builder automatically includes them as `probe-*` cases.

Run a direct comparison from the repository root:

```sh
JAVA_HOME=/path/to/jdk python3 conformance/probes/run.py
```

Requires Python 3, Cargo, a JDK, the pinned `conformance/validator_cli.jar`
(6.10.4), and the packages the probes name. A probe's `ig` names the guide the
oracle loads for it (default `hl7.fhir.au.core#2.0.0`; the NZ probes
`fhir.org.nz.ig.base#3.1.0`, the CA ones `hl7.fhir.ca.baseline#1.2.0`); the
runner starts one JVM per guide, one after another.
`JAVA` can select a Java executable instead of `JAVA_HOME`. The validator may
download packages or resolve profile URLs; `-tx n/a` disables terminology-server
calls, not all network access.

Each run uses a fresh temporary oracle output, verifies the JAR version and
every result identity, and writes `evidence.json` with fixture hashes, oracle
issues, local reports and provenance. A missing result, nonzero oracle exit,
unexpected verdict or local/oracle disagreement fails the command, unless the
disagreement is a reviewed difference.

A manifest entry may carry `reviewed`: `{"sha256": <the fixture's hash>,
"oracle": <the oracle signature>, "local_valid": <the pod's verdict>,
"reason": <why>}`. The oracle signature is the SHA-256 of the oracle's verdict
and every error-level issue (severity, code, text, location), canonically
ordered; each record in `evidence.json` carries the one it was given
(`oracle_signature`). Such a difference counts as `reviewed`, not as a
failure, only while all three still match: an edited fixture, a different
oracle answer or a different pod verdict makes the review stale and fails the
run, and so does a review of a probe whose verdicts now agree. One is
recorded today, `resolve-heartrate-contained-bare-patient`: the oracle holds
a contained target to its `targetProfile` by conformance, the pod by type
(docs/scope.md, Reference target types; `conformance/test_probes.py` holds
the mechanism). Use
`--output /tmp/probe-evidence.json` to preserve the checked-in snapshot, or
`--oracle-only` to establish reference verdicts without running the local engine.

The snapshot describes its recorded working tree; it is not a release-wide
conformance claim. Disagreements are retained as regression evidence.

Two expectations are deliberately narrower than a reader might assume:

- An unresolved declared profile produces a warning in this oracle configuration.
- `Patient.managingOrganization` referencing a Patient is accepted when that
  reference cannot be resolved. This case does not prove target-type checking.
- A standalone required-bound `Coding` with `system` but no `code` is accepted
  by validator_cli 6.10.4. The same Coding inside a required-bound
  `CodeableConcept` does not satisfy the parent binding.
- An `Age` or `Quantity` carrying `comparator: ">="` with `value: 90` is
  valid — the form the `collapse-age` de-identification rule writes, so a
  collapsed age is conformant where the host names the element.
- A comparator is refused only on an element the base types `SimpleQuantity`
  (`type.profile`) — 55 of them, among them `Observation.referenceRange.low`,
  `Range.low` wherever a Range sits, `Dosage.doseAndRate.doseQuantity` — with
  two errors at that element: `Quantity.comparator: max allowed = 0, but
  found 1 (from …/SimpleQuantity|4.0.1)` and `sqty-1`. A comparator present
  only through its `_comparator` carrier counts; a contained resource, a
  Bundle entry and a declared profile change nothing.
- The same two errors where a compiled extension profile, not the base, types
  its `value[x]` `Quantity` with `SimpleQuantity`
  (`medicationdispense-quantityRemaining`): `sq-extension-value-*`.

References: [R4 JSON representation](https://hl7.org/fhir/R4/json.html),
[R4 datatypes](https://hl7.org/fhir/R4/datatypes.html),
[DomainResource constraints](https://hl7.org/fhir/R4/domainresource.html), and
the exact validator diagnostics in `evidence.json`.
