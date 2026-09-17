# Independent FHIR validator comparison

This finite-corpus runner compares the native C ABI and Java validator 6.10.4 with the **open-source Firely legacy validator 5.11.0 / R4 SDK 5.13.2**. The NuGet dependency graph is locked. It does not invoke Firely Terminal's current default engine.

Requires Python 3, Rust, .NET SDK 10 and Java 17. First run `conformance/run.py`, as described in the [repository README](../../README.md), to generate the existing corpus and install the package caches and pinned validator jar. This runner does not generate the corpus. Run from the repository root, choosing a fresh output directory outside the repository:

```sh
JAVA=/path/to/java python3 conformance/second-oracle/run.py \
  --output /tmp/fhir-second-oracle
```

Optional `--corpus`, `--packages-root` and `--jar` point to existing inputs. The corpus must contain all six manifests named in `run.py`; unused manifests may be empty lists. The nine package names/versions and Java jar version are checked before comparison. Keep package resources and publisher-derived examples out of commits.

All Firely profile/terminology resolution uses those local package directories. External resource resolution is disabled, and Java uses `-tx n/a`. Locally decidable bindings remain enabled: terminology off here means no remote terminology service, not removal of every code check.

The output directory receives the copied corpus, complete native/Java/Firely responses and logs, and `summary.json`. This directory may contain publisher-derived content and must remain outside version control. The summary contains only provenance, filenames, hashes, verdicts and exclusion reasons. Exit 0 requires every file to be comparable and all three verdicts to agree; differences or excluded cases produce exit 1. Missing outputs, process failures, malformed outcomes and failed controls abort the run.

The five controls cover success, a required-field failure, a typed-parser rejection, input normalization and an unresolved profile. The Firely POCO parser can normalize wire representations. The adapter compares JSON structure/types before and after parsing and excludes normalized inputs, rather than treating a verdict on a changed object as a verdict on the original bytes. This check does not prove lexical or numeric precision preservation. Unresolved profiles and rule-evaluation failures are also unscored, even if another rule reports an ordinary error on the same resource.

## Recorded comparison

`evidence/corpus.json` records the 2026-09-17 run against **v0.8.4 `1860d48`**, plus the audit/lowering adapters and this runner. Subsequent core releases have not been substituted into that measurement.

| Result | Count |
| --- | ---: |
| Original corpus files | 1,206 |
| Native / Java agreement | 1,206 / 1,206 |
| Comparable Firely results | 1,168 |
| Three-way agreement | 1,152 / 1,168 |
| Comparable differences | 16 |
| Normalized inputs, unscored | 10 |
| Rule evaluation failures, unscored | 25 |
| Unresolved profiles, unscored | 3 |

Twenty-two typed-parser rejections are included among comparable invalid results. The 25 evaluation failures contain 89 diagnostics about an unavailable `resolve` symbol in this configuration. Full result identities and all controls passed. The comparison exits 1 intentionally; this is not an all-green reference suite.

The differences concern two Bundle URL mismatches, two blood-pressure component cases, one body-weight date, one minimal Patient, four narratives, three AU immunization examples, two IPS Bundles and one masked US Core Goal. The planning repository's `reviews/second-oracle/README.md` gives the rule-by-rule assessment, uncertainties and source links. Agreement alone is not proof of identical coverage or data preservation.

`goal-corpus/` contains three entirely synthetic follow-up examples. Repeat them with:

```sh
JAVA=/path/to/java python3 conformance/second-oracle/run.py \
  --corpus conformance/second-oracle/goal-corpus \
  --output /tmp/fhir-second-oracle-goal
```

All validators accept the base Goal's masked extension and the US Core Goal with an `unknown` absence coding. Only Firely rejects the US Core Goal with an extension-only description. `evidence/goal.json` records the three outcomes; its expected run exit is 1. [US Core 6.1 missing-data guidance](https://hl7.org/fhir/us/core/STU6.1/general-requirements.html#missing-data) supports a coded representation for a missing coded element. This is a profile/deidentification finding for adjudication, not permission to invent a clinical goal.

The .NET build has zero warnings/errors. The frozen Rust workspace passes fmt, all-target clippy and 174 tests. `evidence/provenance.json` hashes retained raw outputs and records the tested adapters. Package hashes in the summary identify `package.json`, not every package resource. Source hashes in each run precede these explanatory documents and the added evidence files; the executable runner files are unchanged.
