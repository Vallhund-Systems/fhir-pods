# Coded-answer probes

A hundred and twenty-two synthetic cases: a coded answer with one part missing, under every
way a Questionnaire can name its options, and the same Codings outside a
questionnaire. A rule written from one shape (a display with no code, which
is an error) once reached another (a code with no system, which
validator_cli 6.10.4 only warns about in most shapes), so the lane records
the oracle's verdict for each shape instead of reasoning from one case.

The answers: system and code in the set (the control), a code not in the
set, a code from another system, a code with no system (in the set, not in
it, with and without a display), a display alone, a system alone, a system
and a display, and a `valueString`. The options: a ValueSet contained in the
Questionnaire (loaded by canonical, because a contained Questionnaire may not
itself contain one: `dom-2`), a ValueSet contained beside the Questionnaire
in the response, a ValueSet by canonical (one including a whole code system,
one enumerating its codes, one with no version), and inline `answerOption`s.
Required and optional items. Then the contained set's compose varied (the
`set-*` probes): one include (a code listed twice), two includes of one
system (the code in the first, in the second), two systems (a code unique
to one, a code both share, the code in the second), an enumerated
`exclude`, an include of another value set — each with a url and a name,
with neither, with a url alone and with a name alone — and a system of a
single space. Outside a questionnaire: a required
`CodeableConcept`, a `Coding`, an example binding, no binding.

What validator_cli 6.10.4 does (`-tx n/a`; `evidence.json` has every issue):

- **A code with no system, the code in the set**: a *warning* —
  `Coding has no system. A code with no system has no defined meaning, and it
  cannot be validated. A system should be provided` at the answer, and a
  second one at the Coding — where the set is contained (in the
  Questionnaire or beside it, enumerated or including a whole code system)
  **and** its compose has exactly one include or the set has a url; or where
  the set is reached by canonical and enumerates its codes. A contained set
  with no url and two or more includes keeps the *error*, even when both
  include one system, wherever the code sits (a name does not change it:
  measured on the 20 `set-*` code-only probes, not read from the
  validator's source). Where a canonical set includes a whole code system it
  is an *error*. Under `answerOption`s the
  option's system must match: an error (`The code null::yes is not in the
  set of permitted values`).
- **A code with no system, the code not in the set**: an error, every shape.
- **No code** (a display alone, a system alone, a system and a display): an
  error, every shape and every item.
- **An enumerated `exclude`** is applied: an excluded code is an error,
  with its system or without.
- **A system of whitespace** is no system at the answer (the warning above),
  besides the uri finding on the Coding.
- **The set's name in the message** is its title, else its name, else
  `ValueSet[<url>]`; `ValueSet[null]` only when the set has no url either.
- **Outside a questionnaire**, a required `CodeableConcept` whose only coding
  has no system is refused (`None of the codings provided are in the value
  set`); a `Coding` (Encounter.class, extensible in R4) and unbound or
  example-bound concepts only warn.

The pod is given `defs/` as a host gives it: the Questionnaires loaded
(`--questionnaire`), the ValueSets compiled into a terminology table
(`tools/compile_valueset.py`). The oracle gets the same folder as `-ig`.

```sh
JAVA_HOME=/opt/homebrew/opt/openjdk@17 CARGO_BUILD_JOBS=2 python3 conformance/coded-answers/run.py
python3 conformance/coded-answers/run.py --oracle-only
python3 conformance/coded-answers/make.py      # regenerate the fixtures (expectations kept)
```

One JVM, one temporary oracle output, no shared cache. The run verifies the
JAR's version and every result identity, compares the verdict three ways
(manifest expectation, oracle, pod) and the error-level paths both ways, and
fails on any difference not named in the probe's `reviewed` entry in
`manifest.json` — which must match exactly, so a fix that removes a reviewed
difference fails until the entry goes. The reviewed differences, each with
its reason there:

- the oracle also refuses an unknown code against the CodeSystem it holds;
  the pod is given value-set expansions, never a host's CodeSystem (the same
  verdict, one finding fewer);
- a code with no system under a canonical set that enumerates its codes:
  the pod keeps the error it reports for a canonical set that includes a
  whole system, because a compiled expansion does not say which it was —
  stricter there, never more lenient;
- a contained set that includes a whole code system, or another value set
  (`include.valueSet`): the pod cannot enumerate it, so a coded answer with
  a system is not checked (a warning, where the oracle refuses a code the
  system lacks) and one with no system stays an error (where the oracle
  warns);
- a `valueString` on a choice item with an answerValueSet, refused by the
  pod after fhir-test-cases' stored outcome for `answerconstraint-qr`: kept
  on purpose, stricter than the pinned 6.10.4, which has no message for it
  and reports information — the entry goes when the pin moves;
- a system of a single space: the uri finding's path spelling;
- the same string under `answerOption`s: a path spelling only;
- outside a questionnaire, a required `CodeableConcept` whose only coding has
  a code and no system: the pod's general binding check accepts it.

The pod's exact messages and rule ids for these shapes are pinned by the
core's questionnaire tests (`crates/fhir-pods-core/tests/questionnaire.rs`).
