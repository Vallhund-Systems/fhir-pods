#!/usr/bin/env python3
"""The mutation lane: every release-corpus original, derived under a fixed set
of mutations, put before the oracle and before us. A publisher-valid example
can only show where we are stricter than the reference validator; a mutated
one shows where we are more lenient — the defects a host finds first.

    JAVA_HOME=... python3 conformance/mutate.py                 # every class
    python3 conformance/mutate.py --class drop,codeless           # some
    python3 conformance/mutate.py --reuse-oracle                  # our side only

Per class, one JVM over every derived file (the oracle result is cached in
`conformance/mutations/<class>/_oracle.json` and reused with --reuse-oracle);
our verdicts always come fresh from the probe binary. The lane prints, per
class, how many derived files agree, where we are lenient (we say valid,
the oracle says invalid — the ones that matter), where we are stricter, and
where both say invalid but we miss an error the oracle reports. Accepted
verdict differences are named in `mutate-triage.json`, one entry per derived
file (`<class>:<file>`), each bound to the file's bytes (sha256), the
oracle's verdict and full error signature, the pod's verdict and a reviewed
reason; `mutate_triage.py` derives it from the lane's evidence and its own
table of reasons, never by hand. The lane exits 1 while an unaccepted
lenient or stricter case remains, and when an entry no longer matches a
difference (the file, the oracle or the pod moved: a stale entry).
"""
import argparse
import copy
import hashlib
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run  # noqa: E402
import reasons  # noqa: E402
import canonical_guard  # noqa: E402

ROOT = run.ROOT
CORPUS = os.path.join(HERE, "corpus")
OUT = os.path.join(HERE, "mutations")
TRIAGE = os.path.join(HERE, "mutate-triage.json")
BASELINE = os.path.join(HERE, "corpus-reasons", "evidence.json")


def baseline_missed():
    """The error paths the reasons lane already records as missed on each
    original — a mutation inherits them, and they are not its finding."""
    try:
        ev = json.load(open(BASELINE))
    except (OSError, ValueError):
        return {}
    return {r["file"]: {reasons.normalise(m["path"]) for m in r.get("missed", [])} for r in ev.get("files", [])}
DERIVED = ("lowered-", "deid-", "probe-", "export-", "_")
STRUCTURAL = {"resourceType", "id", "meta", "text", "implicitRules", "language", "contained"}


def originals():
    for f in sorted(os.listdir(CORPUS)):
        if f.endswith(".json") and not f.startswith(DERIVED):
            yield f


def walk(node, path=(), depth=0):
    """Every (path, container, key) for members of objects, depth-first."""
    if isinstance(node, dict):
        for k, v in list(node.items()):
            yield path + (k,), node, k
            yield from walk(v, path + (k,), depth + 1)
    elif isinstance(node, list):
        for i, v in enumerate(node):
            yield from walk(v, path + (i,), depth + 1)


def is_primitive(v):
    return isinstance(v, (str, int, float, bool)) and not isinstance(v, bool) or isinstance(v, bool)


# --- the mutation classes: each yields (label, mutated document) -------------

def m_drop(doc, cap):
    """Remove one top-level element at a time: the oracle says which were required."""
    n = 0
    for k in list(doc):
        if k in STRUCTURAL or n >= cap:
            continue
        d = copy.deepcopy(doc)
        del d[k]
        yield f"drop-{k}", d
        n += 1


def m_codeless(doc, cap):
    """A coding with its code removed (system/display kept); a concept reduced to text."""
    n = 0
    for path, container, key in walk(doc):
        if key == "coding" and isinstance(container[key], list) and container[key] and n < cap:
            d = copy.deepcopy(doc)
            node = d
            for p in path[:-1]:
                node = node[p]
            first = node["coding"][0]
            if isinstance(first, dict) and "code" in first:
                del first["code"]
                yield f"codeless-{'.'.join(map(str, path))}", d
                n += 1
            d2 = copy.deepcopy(doc)
            node = d2
            for p in path[:-1]:
                node = node[p]
            node.clear()
            node["text"] = "mutated"
            yield f"textonly-{'.'.join(map(str, path))}", d2
            n += 1


def m_wrongtype(doc, cap):
    """A primitive of the wrong JSON type; an object where a primitive goes."""
    n = 0
    for k, v in list(doc.items()):
        if k in STRUCTURAL or n >= cap:
            continue
        d = copy.deepcopy(doc)
        if isinstance(v, bool):
            d[k] = "true"
        elif isinstance(v, (int, float)):
            d[k] = "1"
        elif isinstance(v, str):
            d[k] = 1
        elif isinstance(v, dict):
            d[k] = "mutated"
        elif isinstance(v, list):
            d[k] = "mutated"
        else:
            continue
        yield f"wrongtype-{k}", d
        n += 1


def m_emptybb(doc, cap):
    """An array of objects reduced to one empty object."""
    n = 0
    for path, container, key in walk(doc):
        if n >= cap or len(path) > 3:
            continue
        v = container[key]
        if isinstance(v, list) and v and all(isinstance(x, dict) for x in v):
            d = copy.deepcopy(doc)
            node = d
            for p in path[:-1]:
                node = node[p]
            node[key] = [{}]
            yield f"emptybb-{'.'.join(map(str, path))}", d
            n += 1


def m_badenum(doc, cap):
    """A code-typed field set to a value outside its required set."""
    n = 0
    for path, container, key in walk(doc):
        if n >= cap or len(path) > 4:
            continue
        if key in ("status", "gender", "use", "intent", "mode", "kind", "type") and isinstance(container[key], str):
            d = copy.deepcopy(doc)
            node = d
            for p in path[:-1]:
                node = node[p]
            node[key] = "mutated"
            yield f"badenum-{'.'.join(map(str, path))}", d
            n += 1


def m_nullprim(doc, cap):
    """A top-level primitive set to JSON null — never valid FHIR JSON outside an array."""
    n = 0
    for k, v in list(doc.items()):
        if k in STRUCTURAL or n >= cap or not isinstance(v, (str, int, float, bool)):
            continue
        d = copy.deepcopy(doc)
        d[k] = None
        yield f"nullprim-{k}", d
        n += 1


def m_emptystr(doc, cap):
    """A string primitive set to the empty string — FHIR forbids it."""
    n = 0
    for path, container, key in walk(doc):
        if n >= cap or len(path) > 3 or key in STRUCTURAL:
            continue
        if isinstance(container[key], str) and container[key]:
            d = copy.deepcopy(doc)
            node = d
            for p in path[:-1]:
                node = node[p]
            node[key] = ""
            yield f"emptystr-{'.'.join(map(str, path))}", d
            n += 1


def m_badref(doc, cap):
    """A reference to a type that is not a resource type, and one with no target type at all."""
    n = 0
    for path, container, key in walk(doc):
        if n >= cap or key != "reference" or not isinstance(container[key], str):
            continue
        for label, value in (("badref", "NotAType/1"), ("bareref", "1")):
            d = copy.deepcopy(doc)
            node = d
            for p in path[:-1]:
                node = node[p]
            node[key] = value
            yield f"{label}-{'.'.join(map(str, path))}", d
        n += 1


def m_orphanext(doc, cap):
    """An extension carrying a url and nothing else (ext-1), and one with no url."""
    d = copy.deepcopy(doc)
    d.setdefault("extension", []).append({"url": "http://example.org/fhir/StructureDefinition/mutated"})
    yield "orphanext", d
    d2 = copy.deepcopy(doc)
    d2.setdefault("extension", []).append({"valueString": "mutated"})
    yield "nourlext", d2


# the parts of a definition that decide membership: a coding-shaped object
# there (`ValueSet.expansion.contains`) is the definition, never an instance
DEFINITION_PARTS = {"compose", "expansion", "concept"}


def coding_shaped(v):
    """A Coding: a code and a system, and none of a Quantity's value/unit/comparator."""
    return (isinstance(v, dict) and isinstance(v.get("code"), str) and "system" in v
            and not any(k in v for k in ("value", "unit", "comparator")))


def systemless_sites(doc):
    """Every coded site carrying a code and a system: a concept's `coding`
    array (the codings in it that have both), a standalone Coding, an array
    of Codings (`meta.tag`) — never inside a definition's own parts."""
    for path, container, key in walk(doc):
        if any(p in DEFINITION_PARTS for p in path if isinstance(p, str)):
            continue
        v = container[key]
        if isinstance(v, list):
            idx = [i for i, c in enumerate(v) if coding_shaped(c)]
            if idx:
                yield path, idx
        elif coding_shaped(v):
            yield path, None


def strip_at(d, path, idx):
    node = d
    for p in path[:-1]:
        node = node[p]
    v = node[path[-1]]
    for c in ([v[i] for i in idx] if idx is not None else [v]):
        del c["system"]


def m_systemless(doc, cap):
    """Every coding that has a code loses its system; then one coded site at a
    time (a concept's codings, or a standalone Coding)."""
    sites = list(systemless_sites(doc))
    if not sites:
        return
    d = copy.deepcopy(doc)
    for path, idx in sites:
        strip_at(d, path, idx)
    yield "systemless-all", d
    for path, idx in sites[:cap]:
        if len(sites) == 1:
            break  # the one site is the whole document's
        d = copy.deepcopy(doc)
        strip_at(d, path, idx)
        yield f"systemless-{'.'.join(map(str, path))}", d


CLASSES = {
    "drop": m_drop,
    "codeless": m_codeless,
    "wrongtype": m_wrongtype,
    "emptybb": m_emptybb,
    "badenum": m_badenum,
    "nullprim": m_nullprim,
    "emptystr": m_emptystr,
    "badref": m_badref,
    "orphanext": m_orphanext,
    "systemless": m_systemless,
}

# classes whose derived files must name each canonical once (the oracle caches
# a url-bearing ValueSet by url+version, not content: `canonical_guard.py`)
GUARDED = {"systemless"}


def derive(cls, cap, limit):
    """Write the class's derived files; return their paths."""
    out = os.path.join(OUT, cls)
    os.makedirs(out, exist_ok=True)
    for f in os.listdir(out):
        if f.endswith(".json") and not f.startswith("_"):
            os.remove(os.path.join(out, f))
    paths = []
    for i, f in enumerate(originals()):
        if limit and i >= limit:
            break
        try:
            doc = json.load(open(os.path.join(CORPUS, f)))
        except ValueError:
            continue
        if not isinstance(doc, dict):
            continue
        for label, mutated in CLASSES[cls](doc, cap):
            name = f"{f[:-5]}__{label}.json"
            p = os.path.join(out, name)
            json.dump(mutated, open(p, "w"), separators=(",", ":"))
            paths.append(p)
    return paths


def file_sha256(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def oracle_signature(detail):
    """The oracle's whole error list, order-free: what a triage entry binds."""
    return hashlib.sha256(json.dumps(sorted(detail)).encode()).hexdigest()


def accepts(entry, row):
    """Does a triage entry accept this row exactly as it now stands?"""
    return bool(entry) and all(entry.get(k) == row.get(k) for k in ("sha256", "oracle_signature", "oracle", "ours"))


def family(detail):
    """The oracle's first error message, with the variable parts removed."""
    for line in detail:
        if line.startswith("error"):
            text = line.split(": ", 1)[-1]
            text = re.sub(r"'[^']*'", "'…'", text)
            text = re.sub(r"\[[^\]]*\]", "", text)
            text = re.sub(r"\b\d+\b", "n", text)
            return text[:90]
    return "(no error text)"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--class", dest="classes", default=",".join(CLASSES))
    ap.add_argument("--per-file", type=int, default=4)
    ap.add_argument("--limit", type=int, default=0, help="only the first N originals")
    ap.add_argument("--reuse-oracle", action="store_true")
    ap.add_argument("--show", type=int, default=12, help="lenient cases to list per family")
    a = ap.parse_args()

    subprocess.check_call(["cargo", "build", "-q", "-p", "fhir-pods-core", "--example", "probe"], cwd=ROOT)
    probe = os.path.join(ROOT, "target", "debug", "examples", "probe")
    triage = json.load(open(TRIAGE)) if os.path.exists(TRIAGE) else {}
    inherited = baseline_missed()
    evidence = {}
    ungated = 0
    used = set()
    for cls in a.classes.split(","):
        paths = derive(cls, a.per_file, a.limit)
        if cls in GUARDED:
            canonical_guard.refuse_shared_canonicals(paths)
        cache = os.path.join(OUT, cls, "_oracle.json")
        if a.reuse_oracle and os.path.exists(cache):
            oracle = json.load(open(cache))
        else:
            run.CORPUS = os.path.join(OUT, cls)  # the bundle lands beside the class
            oracle = {}
            # the oracle in slices: one JVM at a time, bounded memory
            for i in range(0, len(paths), 800):
                oracle.update({k: [v[0], v[1], sorted(v[2])] for k, v in run.oracle_batch(paths[i:i + 800]).items()})
            json.dump(oracle, open(cache, "w"))
        rows = []
        for p in paths:
            key = os.path.basename(p)
            o = oracle.get(key)
            if not o:
                # a derived file the oracle never judged (a stale cache, a JVM
                # that did not finish): counted, never skipped
                rows.append({"file": key, "kind": "error", "ours": None, "oracle": None, "family": "(no oracle verdict)",
                             "missed": [], "oracle_detail": []})
                continue
            ov, odetail = o[0], o[1]
            opaths = {reasons.normalise(x) for x in o[2]}
            nv, npaths = run.ours(probe, p)
            npaths = {reasons.normalise(x) for x in npaths}
            original = key.split("__", 1)[0] + ".json"
            new_missed = (opaths - npaths) - inherited.get(original, set())
            if nv == "error" or ov == "error":
                kind = "error"
            elif nv == ov == "valid":
                kind = "agree"
            elif nv == ov == "invalid":
                kind = "missed" if new_missed else "agree"
            elif nv == "valid":
                kind = "lenient"
            else:
                kind = "stricter"
            rows.append({"file": key, "kind": kind, "ours": nv, "oracle": ov,
                         "family": family(odetail), "missed": sorted(new_missed),
                         "sha256": file_sha256(p), "oracle_signature": oracle_signature(odetail),
                         "oracle_detail": [x[:160] for x in odetail[:4]]})
        counts = {}
        for r in rows:
            counts[r["kind"]] = counts.get(r["kind"], 0) + 1
        print(f"\n== {cls}: {len(rows)} derived — " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
        for kind in ("lenient", "stricter", "missed", "error"):
            fams = {}
            for r in rows:
                if r["kind"] == kind:
                    fams.setdefault(r["family"] if kind != "stricter" else "(we error, oracle valid)", []).append(r)
            for fam, rs in sorted(fams.items(), key=lambda kv: -len(kv[1])):
                gated = kind in ("lenient", "stricter", "error")
                taken = [r for r in rs if gated and accepts(triage.get(f"{cls}:{r['file']}"), r)]
                used.update(f"{cls}:{r['file']}" for r in taken)
                tag = ("triaged" if len(taken) == len(rs) else "UNTRIAGED") if gated else "reported"
                # a verdict disagreement, or no verdict at all, gates the lane
                ungated += len(rs) - len(taken) if gated else 0
                print(f"  {kind:8} {len(rs):4}  {tag:9} {fam}")
                for r in rs[: a.show]:
                    extra = f" missed {r['missed']}" if kind == "missed" else ""
                    print(f"             {r['file']}{extra}")
        evidence[cls] = rows
    json.dump({"oracle": run.JAR_VERSION, "classes": evidence}, open(os.path.join(OUT, "evidence.json"), "w"), indent=1)
    # an entry for a class just run that accepted nothing is stale: the file,
    # the oracle or the pod moved, and the review is owed again
    stale = [] if a.limit else sorted(
        k for k in triage if k.split(":", 1)[0] in a.classes.split(",") and k not in used)
    for k in stale:
        print(f"  STALE    {k}")
    print(f"\n{ungated} untriaged lenient/stricter case(s), {len(stale)} stale triage entr{'y' if len(stale) == 1 else 'ies'}")
    sys.exit(1 if ungated or stale else 0)


if __name__ == "__main__":
    main()
