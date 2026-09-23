#!/usr/bin/env python3
"""The correctness lane: agreeing on a verdict is not agreeing on WHY.

`run.py` proves every corpus file gets the same valid/invalid verdict from us
and from validator_cli. This lane looks inside: for every file it puts our
OperationOutcome beside the oracle's (the bundle `run.py` left in
`corpus/_oracle.bundle.json` — no JVM here), normalises both sides' issue
paths to one spelling, and reports

  - errors the oracle raises at an element where we raise none — a gap the
    verdict hides whenever another error already makes the file invalid;
  - errors we raise where the oracle raises none — we would be stricter
    there on a file that had no other fault;
  - on files valid for both, the oracle's warnings we do not have (by
    message family) and ours it does not have.

Everything is written to `corpus-reasons/evidence.json` (gitignored) and a
summary is printed. Exit 1 when either error list is non-empty and not
covered by `reasons-triage.json` — a reviewed reason per file, naming the
exact paths it accepts under `missed` / `extra`, so a fix or a regression
moves the lane — and when a triaged file no longer differs.

    python3 conformance/reasons.py [--filter SUBSTRING] [--all]
"""
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CORPUS = os.path.join(HERE, "corpus")
OUT = os.path.join(HERE, "corpus-reasons")
TRIAGE = os.path.join(HERE, "reasons-triage.json")
sys.path.insert(0, HERE)
import run  # noqa: E402  (JAR_VERSION, FILE_EXT, the probe contract)


def normalise(path):
    """One spelling for both sides. Ours: `Bundle.entry.resource.Patient.gender`,
    `Observation.effectiveDateTime`, root issues at `resourceType` /
    `meta.profile` / the type name. The oracle's: `Bundle.entry[0].resource
    /*Patient/x*/.gender`, `Observation.effective[x]` (or `.ofType(dateTime)`),
    root issues at the type name or `NotAResource`. Indices and slice labels
    go; a nested resource's type segment goes; a choice's concrete key is
    kept as the oracle writes it (`effective[x]`) only where the oracle side
    says `[x]` — see `same`."""
    p = re.sub(r"\.ofType\([^)]*\)", "[x]", path)
    p = re.sub(r"/\*[^*]*\*/", "", p)  # the oracle's nested-resource marker
    p = re.sub(r"\[\d+\]", "", p)
    p = re.sub(r":[^.\[\]]+", "", p)
    segs = p.split(".")
    out = []
    for i, s in enumerate(segs):
        # our type segment for a nested resource or datatype (`resource.Patient`,
        # `contained.Organization`, `extension.Extension`): element names are
        # lowercase-initial, so an uppercase segment past the root is ours
        if i > 0 and s[:1].isupper():
            continue
        out.append(s)
    p = ".".join(out)
    if p in ("resourceType", "meta.profile", "NotAResource", "(document)") or "." not in p:
        return "(root)"
    # the Reference element is a finding's locus on both sides; we sometimes
    # name its `reference` member
    return p.removesuffix(".reference") if p.count(".") > 1 else p


def same(a, b, ta="", tb=""):
    """Are two issues the same finding? Paths must name the same element —
    a concrete choice key on one side (`effectiveDateTime`) matches
    `effective[x]` on the other. One level of parent/child is allowed only
    when the two messages say the same thing (a cardinality gap the oracle
    puts on the container and we put on the member, an unknown member it
    reports at the resource): never a bare path match, which once hid a
    datatype invariant behind a sibling's cardinality error."""
    if a == b:
        return True
    if ta and tb and family(ta) == family(tb) and family(ta) in LOCUS_FREE:
        return True
    if "(root)" in (a, b):
        other = b if a == "(root)" else a
        return other.count(".") == 1 and _kin(ta, tb)
    sa, sb = a.split("."), b.split(".")
    if len(sa) == len(sb):
        return all(x == y or _choice(x, y) or _choice(y, x) for x, y in zip(sa, sb))
    shorter, longer = (sa, sb) if len(sa) < len(sb) else (sb, sa)
    return (
        len(longer) - len(shorter) == 1
        and all(x == y or _choice(x, y) or _choice(y, x) for x, y in zip(shorter, longer))
        and _kin(ta, tb)
    )


def _kin(ta, tb):
    """Do two messages describe the same kind of finding? The same message
    family, or a member name (`Provenance.target`, `category:us-core`)
    mentioned on both sides."""
    if not ta or not tb:
        return True
    if family(ta) == family(tb):
        return True
    names = lambda t: set(re.findall(r"[A-Z][A-Za-z]+(?:\.[a-z][A-Za-z0-9]*)+(?::[\w-]+)?", t))  # noqa: E731
    return bool(names(ta) & names(tb))


def _choice(concrete, stem_x):
    return stem_x.endswith("[x]") and concrete.startswith(stem_x[:-3]) and concrete != stem_x[:-3]


def issues(outcome, severities):
    out = []
    for i in outcome.get("issue", []):
        if i.get("severity") not in severities:
            continue
        text = (i.get("details") or {}).get("text") or i.get("diagnostics") or ""
        for e in i.get("expression") or i.get("location") or ["(root)"]:
            out.append((normalise(e), i.get("code", ""), text))
    return out


def family(text):
    """The message, reduced to what it is about — the oracle's wording and
    ours side by side, so the two validators' spellings of one finding meet."""
    t = text
    for pat, name in [
        (r"could not be found so is not allowed here", "extension definition not resolvable"),
        (r"No definition could be found for URL", "definition not resolvable"),
        (r"Wrong Display Name", "display name"),
        (r"ref-1|Unable to resolve resource with reference '#", "local reference"),
        (r"did not match any of the allowed profiles|Unable to find a profile match|conforms to none of|is not one of the resource types", "profile match"),
        (r"allows for the types .* but found type|allows the types .*; found", "extension value type"),
        (r"not in the value set|is not a valid code in the|Unknown code|is not in the (?:required )?value set|was not found in the value set|is not in the bound value set|No code provided", "code not in set"),
        (r"code system .* is unknown|CodeSystem .* could not be found|Code System .* unknown", "code system unknown"),
        (r"None of the codings provided are in the value set", "no coding in set"),
        (r"ValueSet .* not found|could not be resolved|Unable to (?:find|resolve)", "definition not resolvable"),
        (r"Slicing cannot be evaluated", "slicing not evaluable"),
        (r"matching more than one slice|does not match any slice|matching slice is required|slice .* is required|required slice", "slice assignment"),
        (r"Reference .* (?:could not|cannot) be resolved|Unable to resolve resource", "reference not resolvable"),
        (r"cardinality|minimum required|maximum allowed|Too many|min\s*=|max\s*=|is required \(min|exceeds max", "cardinality"),
        (r"Constraint failed|invariant|^[A-Za-z][\w-]*-\d+[a-z]?: ", "invariant"),
        (r"Unrecognised property|Unrecognized property|Unknown property|unrecognized|not a valid element|is not an element of|allows one choice", "unknown element"),
        (r"is not a valid|Not a valid|does not match the regex|format|cannot have whitespace", "primitive format"),
        (r"profile .* (?:not )?(?:found|resolved)|Profile reference .* has not been checked", "profile not resolvable"),
        (r"Narrative|xhtml", "narrative"),
        (r"fullUrl", "fullUrl"),
    ]:
        if re.search(pat, t, re.I):
            return name
    return t[:70]


# findings the two validators locate differently by nature: matched by
# family within the file, not by path
LOCUS_FREE = {"profile match"}


def main():
    args = sys.argv[1:]
    filt = None
    if "--filter" in args:
        filt = args[args.index("--filter") + 1]
    bundle_path = os.path.join(CORPUS, "_oracle.bundle.json")
    if not os.path.exists(bundle_path):
        sys.exit("no cached oracle bundle: run conformance/run.py (with Java) first")
    subprocess.check_call(["cargo", "build", "-q", "-p", "fhir-pods-core", "--example", "probe"], cwd=ROOT)
    probe = os.path.join(ROOT, "target", "debug", "examples", "probe")
    manifest = json.load(open(os.path.join(CORPUS, "manifest.json")))
    for extra in ("lowered", "deid", "doc", "audit", "v2", "export"):
        p = os.path.join(CORPUS, f"{extra}-manifest.json")
        if os.path.exists(p):
            manifest += json.load(open(p))
    oracle = {}
    b = json.load(open(bundle_path))
    for e in b.get("entry", []):
        o = e.get("resource", {})
        src = next((x.get("valueString") for x in o.get("extension", []) if x.get("url", "").endswith(run.FILE_EXT)), None)
        if src:
            oracle[os.path.basename(src)] = o
    triage = json.load(open(TRIAGE)) if os.path.exists(TRIAGE) else {}

    os.makedirs(OUT, exist_ok=True)
    evidence = []
    missed, extra, oracle_warn, our_warn = {}, {}, {}, {}
    ungated = 0
    for c in manifest:
        f = c["file"]
        if filt and filt not in f:
            continue
        if f not in oracle:
            continue
        r = subprocess.run([probe, "--outcome", os.path.join(CORPUS, f)], capture_output=True, text=True)
        try:
            ours = json.loads(r.stdout)
        except ValueError:
            continue
        theirs = oracle[f]
        oe, te = issues(ours, ("error", "fatal")), issues(theirs, ("error", "fatal"))
        rec = {"file": f, "ours_invalid": bool(oe), "oracle_invalid": bool(te), "missed": [], "extra": []}
        # errors the oracle raises where we raise none
        for path, code, text in te:
            if not any(same(path, p, text, t2) for p, _, t2 in oe):
                rec["missed"].append({"path": path, "message": text[:200]})
                missed.setdefault(family(text), []).append((f, path))
        for path, code, text in oe:
            if not any(same(path, p, text, t2) for p, _, t2 in te):
                rec["extra"].append({"path": path, "message": text[:200]})
                extra.setdefault(family(text), []).append((f, path))
        if not oe and not te:
            ow, tw = issues(ours, ("warning",)), issues(theirs, ("warning",))
            for path, code, text in tw:
                if not any(same(path, p, text, t2) for p, _, t2 in ow):
                    oracle_warn.setdefault(family(text), []).append((f, path))
            for path, code, text in ow:
                if not any(same(path, p, text, t2) for p, _, t2 in tw):
                    our_warn.setdefault(text[:60], []).append((f, path))
        # a triage entry names the exact residue it accepts (paths per list):
        # anything beyond it — or an entry whose residue has gone — is ungated
        t = triage.get(f)
        if rec["missed"] or rec["extra"]:
            accepted = t or {}
            if any(m["path"] not in accepted.get("missed", []) for m in rec["missed"]) or any(
                x["path"] not in accepted.get("extra", []) for x in rec["extra"]
            ):
                ungated += 1
        elif t:
            print(f"stale triage: {f} no longer differs")
            ungated += 1
        evidence.append(rec)
    json.dump({"oracle": run.JAR_VERSION, "files": evidence}, open(os.path.join(OUT, "evidence.json"), "w"), indent=1)

    def show(title, groups, limit=6):
        total = sum(len(v) for v in groups.values())
        print(f"\n== {title}: {total} across {len(groups)} families")
        for fam, items in sorted(groups.items(), key=lambda kv: -len(kv[1])):
            files = sorted({f for f, _ in items})
            print(f"  {len(items):4}  {fam}")
            for f, p in items[:limit]:
                print(f"          {f}  @ {p}")
            if len(items) > limit:
                print(f"          … {len(items) - limit} more in {len(files)} files")

    n = len(evidence)
    print(f"{n} files compared with the cached oracle ({run.JAR_VERSION}); {len(triage)} triaged")
    show("errors the oracle raises where we raise none", missed)
    show("errors we raise where the oracle raises none", extra)
    show("warnings the oracle raises on files valid for both, that we do not", oracle_warn, 3)
    show("warnings we raise on files valid for both, that the oracle does not", our_warn, 3)
    print(f"\n{ungated} file(s) with an untriaged error difference")
    sys.exit(1 if ungated else 0)


if __name__ == "__main__":
    main()
