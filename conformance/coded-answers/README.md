# Coded-answer probes

A hundred and thirty-two synthetic cases: a coded answer with one part missing, under every
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
with neither, with a url alone and with a name alone — and a system of
whitespace (a space; a tab and a newline). Every contained ValueSet carries a
url unique to its file, or none: the oracle caches a url-bearing set's
results by url and version, not content, so a url shared between two
definitions has the first one read decide for both (`run.py` refuses such a
fixture set, `../canonical_guard.py`). Outside a questionnaire: a required
`CodeableConcept` (one coding, and several: a code with no system beside the
same code with its system, in either order, beside a second code with no
system, beside a coding of another system), an extensible and a preferred
`CodeableConcept`, a `Coding`, an example binding, no binding.

What validator_cli 6.10.4 does (`-tx n/a`; `evidence.json` has every issue):

- **A code with no system, the code in the set**: a *warning* —
  `Coding has no system. A code with no system has no defined meaning, and it
  cannot be validated. A system should be provided` at the answer, and a
  second one at the Coding — where the set is contained (in the
  Questionnaire or beside it) and its compose has exactly **one** include
  that lists the code (an enumerated exclude beside it, or another value set
  as that include, does not change it), or where the set is reached by
  canonical and enumerates its codes in one include. Two or more includes
  keep the *error*, even when both include one system, wherever the code
  sits, url or none. A contained or canonical set including a whole code
  system is an *error*. (Measured with a url unique to every set; the
  reviewer's 96-cell cache-clean matrix and solo runs found no
  counterexample. The condition in the validator's source is not traced.) Under `answerOption`s the
  option's system must match: an error (`The code null::yes is not in the
  set of permitted values`).
- **A code with no system, the code not in the set**: an error, every shape.
- **No code** (a display alone, a system alone, a system and a display): an
  error, every shape and every item.
- **An enumerated `exclude`** is applied: an excluded code is an error,
  with its system or without.
- **A system of whitespace** is the error at the answer (`The code 'yes' in
  the system ' ' is not in the options value set … Coding has no system`),
  besides the uri finding on the Coding.
- **The set's name in the message** is its title, else its name, else
  `ValueSet[<url>]`; `ValueSet[null]` only when the set has no url either.
- **Outside a questionnaire**, a required `CodeableConcept` none of whose
  codings is a member is refused (`None of the codings provided are in the
  value set`), and a coding with a code and no system is never a member —
  alone, with a display, beside a second such coding or beside a coding of
  another system; beside the same code with its system the concept is valid,
  in either order (any coding decides). Extensible and preferred concepts, a
  `Coding` (Encounter.class, extensible in R4), and unbound or example-bound
  concepts only warn. (A required-bound standalone `Coding` with no system
  only warns too, member or not — probed outside this lane, since it needs
  the guides loaded: `crates/fhir-pods-core/tests/validate.rs`.)

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
- a contained set the pod cannot enumerate: one that includes a whole code
  system leaves a coded answer with a system unchecked (a warning, where the
  oracle refuses a code the system lacks; a code with no system is an error
  on both sides); one whose include is another value set
  (`include.valueSet`) keeps the error for a code with no system, where the
  oracle warns (stricter);
- a `valueString` on a choice item with an answerValueSet, refused by the
  pod after fhir-test-cases' stored outcome for `answerconstraint-qr`: kept
  on purpose, stricter than the pinned 6.10.4, which has no message for it
  and reports information — the entry goes when the pin moves;
- a system of whitespace: the uri finding's path spelling;
- the same string under `answerOption`s: a path spelling only.

The pod's exact messages and rule ids for these shapes are pinned by the
core's questionnaire tests (`crates/fhir-pods-core/tests/questionnaire.rs`).
