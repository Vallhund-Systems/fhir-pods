#!/usr/bin/env python3
"""Derive `mutate-triage.json` from the mutation lane's evidence — never by hand.

    python3 conformance/mutate.py --reuse-oracle        # writes mutations/evidence.json
    python3 conformance/mutate_triage.py                # rewrites mutate-triage.json

Every verdict difference the lane recorded (lenient, stricter, no verdict)
must match exactly one reviewed reason in `REASONS` below; the entry written
for it is bound to that derived file's bytes (sha256), the oracle's verdict
and its whole error signature (from the class's oracle cache), and the pod's
verdict, so the lane accepts that file as it stands and nothing else, and
fails an entry that no longer matches (`mutate.py`). A difference no reason
claims is listed and the script exits 1 without writing: a new difference is
reviewed — a reason added here, or the pod fixed — before it is accepted.
A reason's predicate reads the oracle's full error list, never a truncated
family.
"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "mutations")
TRIAGE = os.path.join(HERE, "mutate-triage.json")


def all_errors_match(pattern):
    rx = re.compile(pattern)
    return lambda errors: bool(errors) and all(rx.search(e) for e in errors)


def every_error_names(*needles):
    return lambda errors: bool(errors) and all(any(n in e for n in needles) for e in errors)


# (class, kind, file prefix, predicate over the oracle's error lines, reason)
REASONS = [
    ("systemless", "lenient", "us-QuestionnaireResponse-",
     all_errors_match(r"^error: The code null::\S+ is not in the set of permitted values @ "),
     "A QuestionnaireResponse naming a Questionnaire the pod is not given (US Core's own, which the oracle reads "
     "from the package): the pod reports the questionnaire unresolved (a warning) and judges no answer against "
     "it; the oracle refuses each answer whose system was stripped against the item's answerOption codings. Not "
     "the general binding check: the questionnaire rule, given the Questionnaire, refuses it (coded-answers "
     "`options-*`)."),
    ("systemless", "lenient", "us-DocumentReference-",
     every_error_names("None of the codings provided are in the value set 'US Core DocumentReference Type' "
                       "(http://hl7.org/fhir/us/core/ValueSet/us-core-documentreference-type|6.1.0)"),
     "US Core DocumentReference Type is not bundled (its LOINC include is a filter, which the pod does not "
     "expand): the pod reports it not checked (a warning). The oracle refuses a code with no system under it "
     "because the compose has two includes (a listed v3-NullFlavor UNK beside the LOINC filter) and it infers a "
     "system only under one include listing its concepts. A host that supplies the set as a table gets the "
     "refusal for a code the table does not hold."),
]


def main():
    ev = json.load(open(os.path.join(OUT, "evidence.json")))
    triage, unclaimed = {}, []
    for cls, rows in sorted(ev["classes"].items()):
        cache_path = os.path.join(OUT, cls, "_oracle.json")
        cache = json.load(open(cache_path)) if os.path.exists(cache_path) else {}
        for r in rows:
            if r["kind"] not in ("lenient", "stricter", "error"):
                continue
            errors = (cache.get(r["file"]) or [None, []])[1]
            claims = [reason for c, kind, prefix, pred, reason in REASONS
                      if c == cls and kind == r["kind"] and r["file"].startswith(prefix) and pred(errors)]
            if len(claims) != 1 or "sha256" not in r:
                unclaimed.append(f"{cls}:{r['file']} ({r['kind']}; {len(claims)} reasons)")
                continue
            triage[f"{cls}:{r['file']}"] = {
                "kind": r["kind"], "ours": r["ours"], "oracle": r["oracle"], "sha256": r["sha256"],
                "oracle_signature": r["oracle_signature"], "oracle_errors": errors, "reason": claims[0]}
    if unclaimed:
        print("no reviewed reason for:\n  " + "\n  ".join(unclaimed))
        sys.exit(1)
    with open(TRIAGE, "w") as fh:
        json.dump(triage, fh, indent=1, sort_keys=True)
        fh.write("\n")
    print(f"{len(triage)} triage entr{'y' if len(triage) == 1 else 'ies'} written")


if __name__ == "__main__":
    main()
