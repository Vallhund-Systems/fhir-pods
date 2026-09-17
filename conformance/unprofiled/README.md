# Unprofiled package examples

This lane checks the examples omitted by `make_corpus.py`'s profile filter:
root resources that declare none of their package's IG profiles. It scans
all six pinned package versions and compares their base-resource behavior
with reference validator 6.10.4.

Run from the repository root after the pinned package caches and validator
JAR have been installed by the main conformance setup:

```sh
JAVA=/path/to/java python3 conformance/unprofiled/run.py
python3 -m unittest discover -s conformance/unprofiled -p 'test_*.py' -v
```

`--source /path/to/clean/checkout` builds and runs that checkout's existing
`probe` example without using concurrent source edits. `--packages`, `--jar`,
`--corpus` and `--output` override the package-cache root, validator JAR,
derived workspace and evidence destination. No package fetching is hidden
inside this runner; a missing/wrong-version cache fails the run.

Selection matches the complement of the existing root-profile prefix test.
Nested resources can still declare IG profiles, so the runner recursively
removes `meta.profile` from every resource on **both** sides before comparison.
It removes an emptied `meta` object and preserves other metadata. Evidence
records removed declarations and hashes both original and transformed bytes.
This isolates the normal no-default-profile behavior; it is not validation
of the untouched published examples against their original declarations.
R4 implied vital-sign profiles and known extension definitions remain active.
The oracle loads the six pinned IGs for definitions, with external terminology
disabled (`-tx n/a`). Bundled/offline terminology can still run.

`corpus/` is derived and ignored: it contains publisher examples, the input
manifest, full native/oracle results and the JVM log. Never commit that
content. The committed evidence records package versions and hashes, input
hashes, verdicts, issue severity/code/locations and hashes of complete issues;
it does not copy publisher narrative or terminology displays. Regenerate the
ignored full results to inspect diagnostic text.

The runner checks the validator JAR's actual version, uses a fresh temporary
oracle output, and requires exactly one well-formed OperationOutcome per
input. A native crash, a foreign exit, contradictory exit/report/issues,
missing/duplicate/foreign outcome or failed JVM cannot count as agreement.
Exit 0 means all verdicts agree; exit 1 means at least one disagreement.
Execution/evidence failures also return nonzero with a diagnostic.

## Recorded v0.6.0 result

At clean source `26e0486`: **22 selected examples, 19 agreements and three
differences**. AU Core contributes one, US Core nine, BALP twelve; IPA, IPS
and AU PS have zero selected roots. The package metadata is version-checked
and every package's total/selected count is in `evidence.json`.

All 22 native verdicts are valid. The reference validator rejects three US
Core Bundles:

| Example | Oracle errors |
| --- | --- |
| `Bundle-66c8856b-ba11-4876-8aa8-467aad8c11a2.json` | Entry fullUrl/id mismatch; unresolved XHTML hyperlinks |
| `Bundle-c887e62f-6166-419f-8268-b5ecd6c7b901.json` | Unresolved XHTML hyperlink |
| `Bundle-docref-example-2.json` | Entry fullUrl/id mismatch |

XHTML checking is an existing documented exclusion and remains a measured
verdict difference. REST-shaped entry URLs must agree with the resource's
identity under the [R4 Bundle rules](https://hl7.org/fhir/R4/bundle-definitions.html#Bundle.entry.fullUrl).
Neither mismatch is suppressed or relabeled as a passing result. The suite
is intentionally red until behavior or explicitly scored scope changes.
This result does not replace the separate release corpus's historical count.
