# Adversarial validation probes

These 37 synthetic fixtures cover valid controls, JSON shapes, primitive limits,
extensions, contained resources, unknown profiles, offline references, required
coded content, recursive content-reference validation, and the shape a
de-identified age over 89 goes out as.
`manifest.json` records an expected verdict and reason for each fixture. The
shared corpus builder automatically includes them as `probe-*` cases.

Run a direct comparison from the repository root:

```sh
JAVA_HOME=/path/to/jdk python3 conformance/probes/run.py
```

Requires Python 3, Cargo, a JDK, the pinned `conformance/validator_cli.jar`
(6.10.4), and the AU Core 2.0.0 package used by the main conformance harness.
`JAVA` can select a Java executable instead of `JAVA_HOME`. The validator may
download packages or resolve profile URLs; `-tx n/a` disables terminology-server
calls, not all network access.

Each run uses a fresh temporary oracle output, verifies the JAR version and
every result identity, and writes `evidence.json` with fixture hashes, oracle
issues, local reports and provenance. A missing result, nonzero oracle exit,
unexpected verdict or local/oracle disagreement fails the command. Use
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

References: [R4 JSON representation](https://hl7.org/fhir/R4/json.html),
[R4 datatypes](https://hl7.org/fhir/R4/datatypes.html),
[DomainResource constraints](https://hl7.org/fhir/R4/domainresource.html), and
the exact validator diagnostics in `evidence.json`.
