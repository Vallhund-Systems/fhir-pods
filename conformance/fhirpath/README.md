# Official R4 FHIRPath coverage catalogue

This runner accounts for every test in the pinned official
[`FHIR/fhir-test-cases`](https://github.com/FHIR/fhir-test-cases/tree/87fe5070c0d7c13d84acecfab9c5cc1b4ec2ba47/r4/fhirpath)
`tests-fhir-r4.xml`. It measures the existing public `fhir_pods_core::path` API;
it does not modify the evaluator or silently drop unsupported tests.

From the repository root:

```sh
# Install the pinned R4 package cache if not already present:
python3 conformance/make_corpus.py
# Explicit network preparation; never part of a Cargo build:
python3 conformance/fhirpath/fetch.py
# Offline coverage report:
python3 conformance/fhirpath/run.py
```

Requires Python 3 and Cargo. The first fetch downloads the suite and its eleven
exact input assets at revision `87fe5070c0d7c13d84acecfab9c5cc1b4ec2ba47`.
`sources.lock.json` pins their SHA-256 hashes. Later fetches reuse the cache and
verify those hashes. Delete `.cache/` to fetch again.

The preparation step translates XML fixtures to JSON using the cached R4 4.0.1
StructureDefinitions for primitive kinds and array cardinalities, retaining
primitive extensions and contained resources. It rejects unknown shapes. This
is a fixture adapter, not a general-purpose FHIR XML parser. Downloaded assets
and prepared `cases.jsonl` are ignored; their hashes are retained in the report.

`run.py` checks the adapter against positive and deliberately wrong expected
results, builds the Rust example, and requires exactly one result for every
case. `catalogue.json` includes each expression, expected outputs, status,
reason, input filename, group, upstream version marker and comparison assertion.
It also records source and engine hashes and per-group totals.

The recorded development run contains **936 cases: 264 pass, 11 fail and 661
unsupported**. This is a coverage inventory, not full FHIRPath conformance.
Default execution succeeds when it produces a complete report; use `--strict`
to fail if any case is failed or unsupported. `--output /tmp/catalogue.json`
preserves the checked-in snapshot.

## What the statuses mean

- **pass:** ordered result cardinality, system primitive type and value match
  for supported boolean/integer/string outputs. Predicate tests explicitly map
  empty to false. A pass does not certify the exact FHIR primitive subtype.
- **fail:** the evaluated output disagrees, or an expression marked invalid by
  the official suite is accepted. The assertion is included for inspection.
- **unsupported:** the engine rejects syntax/evaluation, or the public API and
  adapter cannot make the required comparison. This includes exact error
  categories, static semantics, unordered outputs, and decimal/date/Quantity or
  other output types. Reasons distinguish these limits where possible.

The public API exposes invariant verdicts, not raw result collections. The
adapter checks explicit cardinality before comparing values, preventing an
empty result from passing through invariant semantics. Parser rejection or an
evaluation error cannot be counted as a successful expected-error test because
the API also uses that result for unsupported features. Some assertions use
the evaluator's own `count`, `tail`, `first`, equality and type operators; the
catalogue therefore complements, rather than replaces, independent engine tests.

All upstream cases are retained, including cases tagged `2.1.0` in this R4 test
file. Consult each case's metadata before making a version-specific claim.

The expressions and expected results in the catalogue are derived from the
official test-case repository under [Apache 2.0](UPSTREAM-LICENSE.txt). The
catalogue adds this project's comparison assertions, classifications and
measurement metadata; upstream resource fixtures remain in the ignored cache.
