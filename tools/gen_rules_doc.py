#!/usr/bin/env python3
"""Write `docs/rules.md` from the `RULES` table in `src/rules.rs` — the table
is the contract; the page is its rendering, and a test holds them together.

    python3 tools/gen_rules_doc.py
"""
import os
import re

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
src = open(os.path.join(ROOT, "crates", "fhir-pods-core", "src", "rules.rs")).read()
rows = re.findall(r'\("([^"]+)", "([^"]+)", "([^"]+)"\),', src.split("pub static RULES")[1].split("];")[0])
families = {}
for rule_id, severity, meaning in rows:
    families.setdefault(rule_id.split(".")[0], []).append((rule_id, severity, meaning))
names = {
    "parse": "The input", "profile": "Declared profiles",
    "structure": "Structure — the shape walk against the base definitions", "value": "Values",
    "binding": "Bindings and codes", "code": "Code systems", "reference": "References", "extension": "Extensions",
    "inv": "Invariants", "bundle": "Bundles and documents", "questionnaire": "Questionnaires",
    "lower": "Lowering inputs", "not-supported": "What was not checked", "other": "Fallback",
}
out = [
    "# Rule ids", "",
    "Every finding the pod reports carries a stable rule id — `issues[].id` in the",
    "pods' JSON report, `issue.details.coding` (system `urn:fhir-pods:rule`) in an",
    "`OperationOutcome` — so a host asserts on *which* rule fired, never on",
    "message text. The contract:", "",
    "- an id never changes meaning;",
    "- a new kind of finding gets a new id;",
    "- an id that falls out of use stays listed here as retired;",
    "- an invariant's id is `inv.<key>` — `inv.cmp-1`, `inv.que-7`, `inv.dom-6` —",
    "  for every key the base definitions and the compiled profiles carry;",
    "- `other.<type>` is the fallback for a finding no family claims — a defect",
    "  of the table, which the core's tests hold to zero over the release",
    "  corpus, the mutation files and the gut-path scenario.", "",
    "The ids are derived from the finding's type and message family in",
    "`crates/fhir-pods-core/src/rules.rs`; the table there is the source of this",
    "page (`tools/gen_rules_doc.py`) and a test holds the two together — both",
    "ways, so neither can carry an id the other does not.", "",
    "## Severities", "",
    "Every finding carries one of four severities — `issues[].severity` in the",
    "pods' report, `issue.severity` in the `OperationOutcome` — and the tables",
    "below say which a given id can arrive at:", "",
    "| Severity | What it means |",
    "| --- | --- |",
    "| `fatal` | Something could not be checked at all, and the report says which. Two findings are fatal: the input is not JSON — there is no document, so that finding stands alone — and the Questionnaire a response is held to has an item with no `type`, so the response cannot be judged against that item (the Questionnaire's own findings stand beside it). |",
    "| `error` | The resource does not conform. |",
    "| `warning` | Something a host may need to act on that is not a conformance failure: a check that could not be performed (no bundled terminology, an expression outside the supported core), a declared profile this build has not compiled, an invariant the definitions grade best-practice. |",
    "| `information` | Said for the record, never a defect: a profile invariant skipped as unsupported, no Questionnaire named at all, no option list to validate an answer against. |", "",
    "**`valid` means no `error` and no `fatal`** — `Report::is_valid`,",
    "`\"valid\": true` in the pods' report, exit 0 from the CLI. A `warning` or an",
    "`information` finding never changes the verdict, however much it matters:",
    "a resource whose declared profile was never compiled here is *valid* by",
    "default, with `profile.unknown` naming it (the reference validator's own",
    "severity), and a host that will not accept that asks for",
    "`unknownProfile: \"error\"` rather than filtering the issue out",
    "(`docs/scope.md`). A `fatal` finding makes `valid` false and says that",
    "something was not checked: for `parse.json` nothing at all was (no bytes",
    "parsed, no document, that one finding in the report — so reading its short",
    "issue list as \"almost conformant\" would be wrong), and for",
    "`questionnaire.definition` the response could not be judged against the",
    "item whose definition is broken. Both were probed against validator_cli",
    "6.10.4 (2026-10-14): it reports `fatal` for the same two inputs.", "",
]
for family, items in families.items():
    out += [f"## {names.get(family, family)}", "", "| Id | Severity | Meaning |", "| --- | --- | --- |"]
    out += [f"| `{i}` | {s} | {m} |" for i, s, m in items]
    out.append("")
out += [
    "## Gate mode", "",
    "`fhir-pods gate [--questionnaire Q.json]… [--terminology T.txt] FILE…` validates",
    "every file, prints one JSON report — per file its verdict and error-level",
    "findings by id, and the counts by id over the run — and exits 1 when any",
    "file has an error. That is what runs on a vendor's deliverables and in CI.", "",
]
open(os.path.join(ROOT, "docs", "rules.md"), "w").write("\n".join(out))
print(f"docs/rules.md: {len(rows)} ids")
