"""Refuse a fixture set in which one canonical url names two definitions.

The reference validator caches a url-bearing ValueSet's validation results
by url and version, not by content (`TerminologyCache`); only a set with no
url is keyed by its compose. So when two fixtures in one run carry different
ValueSets under one url, whichever is judged first decides the answer for
every later one — the lane then records the cache, not the validator. The
same holds for a CodeSystem url given two different contents. A probe lane
calls `refuse_shared_canonicals(paths)` before it starts the oracle.
"""
import json
from pathlib import Path

# what a definition is, for this purpose: the parts that decide membership
DEFINING = {
    "ValueSet": ("version", "compose", "expansion"),
    "CodeSystem": ("version", "content", "caseSensitive", "concept", "property", "filter", "supplements"),
}


def _resources(resource):
    """The resource, its contained resources and its bundle entries, nested."""
    if not isinstance(resource, dict):
        return
    yield resource
    for c in resource.get("contained") or []:
        yield from _resources(c)
    for e in resource.get("entry") or []:
        if isinstance(e, dict):
            yield from _resources(e.get("resource"))


def shared_canonicals(paths):
    """{(resourceType, url): [file, …]} for every url given two definitions."""
    seen, conflicts = {}, {}
    for path in paths:
        try:
            root = json.loads(Path(path).read_text())
        except ValueError:
            continue  # a fixture that is not JSON is the lane's own business
        for r in _resources(root):
            kind, url = r.get("resourceType"), r.get("url")
            if kind not in DEFINING or not isinstance(url, str):
                continue
            body = json.dumps({k: r.get(k) for k in DEFINING[kind]}, sort_keys=True)
            first = seen.setdefault((kind, url), (body, Path(path).name))
            if first[0] != body:
                conflicts.setdefault((kind, url), [first[1]]).append(Path(path).name)
    return conflicts


def refuse_shared_canonicals(paths):
    conflicts = shared_canonicals(paths)
    if conflicts:
        lines = [f"  {kind} {url}: {', '.join(sorted(set(files)))}" for (kind, url), files in sorted(conflicts.items())]
        raise SystemExit("One canonical url names two definitions — the oracle would judge every one of them "
                         "by whichever it read first (it caches by url, not content):\n" + "\n".join(lines))
