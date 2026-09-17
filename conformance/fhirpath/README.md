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

The recorded v0.6.4 core run with the widened adapter contains **936 cases:
279 pass, 1 fail and 656 unsupported**. The unchanged old adapter on that same
core gave 262 pass / 11 fail / 663 unsupported. Ten false comparison failures
are corrected and seven previously uncomparable cases can now pass. No core
evaluator code changed in this adapter update. This is a coverage inventory,
not full FHIRPath conformance.
Default execution succeeds when it produces a complete report; use `--strict`
to fail if any case is failed or unsupported. `--output /tmp/catalogue.json`
preserves the checked-in snapshot.

## What the statuses mean

- **pass:** result cardinality, primitive type and value match. Boolean,
  integer and string results may be FHIR elements or System values; explicit
  code/id outputs require the named FHIR type. Date outputs compare type and
  precision-preserving date text. Ordered collections compare each position;
  unordered collections with one expected type compare multiplicities,
  including duplicates. Mixed expected types remain unsupported because their
  type predicates can overlap.
  Predicate tests explicitly map empty to false. General string/integer checks
  do not certify an exact FHIR subtype.
- **fail:** the evaluated output disagrees, or an expression marked invalid by
  the official suite is accepted. The assertion is included for inspection.
- **unsupported:** the engine rejects syntax/evaluation, or the public API and
  adapter cannot make the required comparison. Original-expression evaluation
  is checked before output comparison, so an unsupported evaluator is not
  mislabeled as a missing comparator. Exact error categories, static semantics,
  decimal/dateTime/Quantity and complex output comparisons remain outside the
  adapter's guarantee. No parser/evaluation rejection is scored as a successful
  expected-error test.

The 656 unsupported cases are classified by the stage that blocks comparison:

| Category | Cases | Meaning |
| --- | ---: | --- |
| `engine-parse` | 405 | An expression not marked invalid is rejected by the parser |
| `engine-evaluation` | 215 | The original expression cannot be evaluated, before comparison |
| `api-error-ambiguity` | 23 | An expected error cannot be distinguished from an unsupported feature |
| `api-static-semantics` | 11 | Static checking is not exposed by the public API |
| `api-context` | 2 | The resource-free variable context is not representable |

There are no remaining cases blocked *only* by output comparison on this
baseline. That does not imply support for every possible output type. The 620
parse/evaluation cases describe unavailable execution, not 620 newly confirmed
semantic bugs. All original cases and upstream version tags remain included.

The remaining failed case is `testFHIRPathAsFunction21`:
`Patient.name.as(HumanName).use` is marked as an execution error by the suite,
while this engine intentionally treats collection `as()` as a filter for
compatibility with the reference validator's `dom-3`. It stays a measured
failure (planning F007), not an expected success or an omitted case.

The public API exposes invariant verdicts, not raw result collections. The
adapter checks explicit cardinality and requires the comparison itself to be
nonempty, including when a primitive exists only as an extension carrier.
Twenty-six positive/negative self-checks cover wrong types, order, duplicate
multiplicity, missing values and comparison-vs-evaluation classification.
FHIR elements and System values are distinguished in the
[R4 FHIRPath type rules](https://hl7.org/fhir/R4/fhirpath.html#types).
Some assertions use the evaluator's own `count`, `tail`, `first`, `where`,
equality, type and conversion operators; the
catalogue therefore complements, rather than replaces, independent engine tests.

The report records the engine, adapter, runner and executable hashes along with
the source revision and dirty-tree flag. The snapshot was measured in an
isolated v0.6.4 checkout with only owned review changes overlaid; it excludes
concurrent implementation work in the shared checkout.

All upstream cases are retained, including cases tagged `2.1.0` in this R4 test
file. Consult each case's metadata before making a version-specific claim.

The expressions and expected results in the catalogue are derived from the
official test-case repository under [Apache 2.0](UPSTREAM-LICENSE.txt). The
catalogue adds this project's comparison assertions, classifications and
measurement metadata; upstream resource fixtures remain in the ignored cache.
