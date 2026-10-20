# Conformance lanes

The release gate (`run.py`), published base examples (`base.py`), and reference
validator test cases (`testcases.py`) are separate comparisons. Their inputs,
terminology settings and denominators differ; agreement is not a blanket
conformance claim. Run JVM steps one at a time and set `CARGO_BUILD_JOBS=2`.

## Published base examples

`base.py` is a regression gate over the JSON examples in
`hl7.fhir.r4.examples#4.0.1`. The 5,292 selected inputs have a pinned inventory
hash. `base-triage.json` names each reviewed verdict difference, its reasons,
input hash and oracle diagnostic signature. A new difference, changed verdict
or changed diagnostic signature fails, including an improvement that makes a
reviewed exception stale. Changes require review, not automatic rebaselining.

```sh
JAVA_HOME=/opt/homebrew/opt/openjdk@17 CARGO_BUILD_JOBS=2 python3 conformance/base.py
CARGO_BUILD_JOBS=2 python3 conformance/base.py --reuse-oracle
python3 conformance/base.py --reuse-oracle --filter Patient-
python3 -m unittest discover -s conformance -p test_base.py
```

The first command runs pinned Java batches sequentially and records complete
outcomes in ignored `corpus-base/`. Reuse needs a cache made by this runner:
package/IG pins, JAR bytes, input bytes and outcome hashes must match. Old
filename-only evidence cannot seed the cache. Missing, duplicate, malformed
or foreign results and process failures fail the gate; native exit status must
agree with its OperationOutcome. A separate Cargo target builds the default
release probe without the optional test-profile registry.

Two hash-pinned inputs (`ValueSet-media-modality.json` and
`ValueSet-use-context.json`) historically produced no usable Java outcome.
They are explicitly excluded from the oracle run before comparison; the
native result is still checked. They are never agreements or oracle-invalid
verdicts. Remove an exclusion only after a separately recorded successful
oracle recheck. All other selected inputs require usable oracle evidence.

Fresh Java runs can also change definition-validation diagnostics when published
StructureDefinitions reuse an invariant key across resources. The initial
gate adoption caught four such changes around `inv-1` (including a swap of
which definition was rejected). Those were reviewed explicitly; future oracle
drift still fails and must be investigated before changing the baseline.

Full results are `corpus-base/evidence.json`; filtered/limited runs write
`evidence-subset.json` and cannot replace the full oracle cache. The summary
separates selected, compared, agreed, lenient, stricter, excluded and failed
cases. A passing gate means the reviewed baseline is unchanged, not that
unimplemented rules were checked or every example agrees.

## Published validator test cases

`testcases.py` fetches only `validator/` from the Apache-2.0
[FHIR test suite](https://github.com/FHIR/fhir-test-cases/tree/591528170b6b31609b7682dc67c37f45edfe33ff/validator).
The suite commit is pinned in the runner. Its 1,042 entries contain 676 R4
cases: 492 JSON and 184 XML. This lane reads JSON instance bytes unchanged;
XML instances are not run. The default command needs no JVM:

```sh
CARGO_BUILD_JOBS=2 python3 conformance/testcases.py
python3 conformance/testcases.py --filter bundle
python3 conformance/testcases.py --module references
python3 -m unittest discover -s conformance -p test_testcases.py
```

Java expectations can be checked-in OperationOutcome files or inline outcomes;
Firely expectations are explicit diagnostic lists. A missing Firely result
is not agreement. Results compare validity; normalized error paths are recorded
separately. These are stored upstream expectations, not fresh runs of both
engines, and some reflect online terminology or Java CLI policies. Reviewed
current-Java checks record their version, verdicts and outcome hashes.

Manifest modules/options identify cases outside the pod's capabilities. A
remaining disagreement stays scored and receives a reason in
`testcases-triage.json`, bound to the exact input, Java and Firely expectations,
and all three verdicts. The command fails on new/changed differences, unresolved
strictness, native failures and unusable evidence. The suite currently names
one absent input (`simple-quantity.json`); this audited omission is reported
separately from compared cases and guarded by its manifest-entry hash.

Full outputs go to ignored `corpus-testcases/base-evidence.json`. They include
source/suite identities, input/output hashes, selected filters, complete native
and Java outcomes, scope exclusions, and evidence failures. Filtered runs
replace the evidence for that lane, so rerun the unfiltered command before
recording release numbers. Working review reports live in the planning repo.

## Stored outcomes against the pinned validator

The questionnaire lane (`questionnaire.py`) scores against fhir-test-cases'
**stored** Java outcomes, and some of those come from a validator later than
the pinned 6.10.4: `answerconstraint-qr`'s error for a string answer where
`answerConstraint` is `optionsOnly` has no message in the pinned JAR's
`Messages.properties`, and 6.10.4 run fresh reports it as information. The
probe lanes (`probes/`, `narrative/`, `coded-answers/`) run the pinned 6.10.4
fresh. Where the two disagree the pod keeps the stored outcome's stricter
finding (fail-closed), and the probe lane names it as a reviewed difference;
those entries go when the pin moves.

## Ad-hoc profiles

The 53 R4 JSON cases with a `profile` entry exercise the same Rust profile
compiler using an optional `testcase-profiles` feature. They never enter the
shipped registry. First prepare their differential snapshots using the pinned
validator_cli 6.10.4 JAR and the pinned R4 package cache:

```sh
python3 conformance/make_corpus.py
JAVA_HOME=/opt/homebrew/opt/openjdk@17 python3 conformance/testcases.py --prepare-profiles
CARGO_BUILD_JOBS=2 python3 conformance/testcases.py --profiles
```

Preparation uses one JVM at a time and saves snapshots only under the ignored
cache. XML StructureDefinitions may be converted during preparation; this does
not add XML instance support. The comparison itself needs no JVM. Snapshot
caches are tied to source hashes and the JAR hash. Two definitions sharing a
canonical URL in different cases have isolated contexts, so neither can
replace the other. Each context generates `src/generated_testcases/`, builds a
separate probe target, adds the requested canonical to the instance's
`meta.profile`, and compares against `profile.java` and its Firely expectation.
The original bytes and derived JSON have separate hashes. Each derived input is
also run through the default probe to record whether adding the test profile
changes the verdict; the probes use separate build targets.

A definition rejected by snapshot preparation is reported. Compiler constructs
that cannot be represented are named by element and excluded before comparing;
they are never converted to a claimed successful compilation. These include
unresolvable/deep slicing, exact complex fixed-value equality, Quantity/additional
bindings, min/max values, and non-sliced datatype profile constraints. Such
exclusions are limitations, not agreement. Output:
`corpus-testcases/profiles-evidence.json` and `profiles-prepared.json`.

Default builds do not read the generated test directory. Direct builds with
`--features testcase-profiles` require a prepared/generated context; normally
use the runner. Its separate Cargo target prevents a feature build from being
reused as the normal probe. Release scripts build without the feature.

Do not commit upstream fixtures, generated definitions, licensed terminology
content, or complete upstream diagnostic text. Keep fetched/derived data in
ignored caches and commit only the runner, reviewed classifications, synthetic
regressions and metadata. A suite-pin update requires rerunning and reviewing
both lanes rather than carrying forward old verdict exceptions automatically.
