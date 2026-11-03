#!/usr/bin/env python3
"""Stage exact host input bytes and record their identities in the compiled pod.

Compilation and hashing both consume the private staged copies. The original
package may change after staging without changing this build's source identity.
"""
import hashlib
import json
from pathlib import Path
import re
import shutil
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gen_profile  # noqa: E402

BASE = "http://hl7.org/fhir/StructureDefinition/"


def stage_packages(destination, sources):
    destination = Path(destination)
    destination.mkdir()
    staged = []
    for index, source in enumerate(sources):
        target = destination / f"package-{index}"
        target.mkdir()
        for path in sorted(Path(source).glob("*.json")):
            if path.is_file():
                shutil.copyfile(path, target / path.name)
        staged.append(target)
    # Refuse ambiguous canonical ownership before any generation/installation.
    source_identities(staged)
    return staged


def _definitions(packages):
    """Every StructureDefinition in the packages: url -> (path, raw bytes, sd).
    A canonical defined twice is refused."""
    seen = {}
    for package in packages:
        for path in sorted(Path(package).glob("*.json")):
            raw = path.read_bytes()
            try:
                sd = json.loads(raw)
            except (ValueError, UnicodeDecodeError):
                continue  # the compiler owns whether an unrelated file is usable
            if not isinstance(sd, dict) or sd.get("resourceType") != "StructureDefinition" or not sd.get("url"):
                continue
            url = sd["url"]
            if not isinstance(url, str):
                raise ValueError(f"non-string profile canonical in {path.name}")
            if url in seen:
                raise ValueError(f"duplicate host profile canonical {url}: {seen[url][0].name}, {path.name}")
            seen[url] = (path, raw, sd)
    return seen


def _held_alone(sd):
    """The datatype profiles an element of `sd` is held to alone — one IG
    profile, no base datatype beside it — which the generator compiles too
    (gen_profile.ig_datatype_profile)."""
    for e in (sd.get("snapshot") or {}).get("element", []):
        for t in e.get("type", []):
            code = t.get("code")
            if code in ("Extension", "Resource", "DomainResource") or code not in gen_profile.DATATYPE_CODES:
                continue
            urls = [u.split("|")[0] for u in t.get("profile", [])]
            own = [u for u in urls if not u.startswith(BASE)]
            if BASE + code not in urls and len(own) == 1:
                yield own[0]


def compiled_definitions(packages):
    """What the generator compiles from the host packages, the generator's
    own rule: every constraint on a resource, an Identifier or a complex
    Extension, and every other datatype profile of the host's that a compiled
    one holds an element to alone (transitively). Returns `(compiled, missing)`:
    url -> (path, raw bytes) for those with a snapshot, and the paths of those
    without one, which would compile to nothing."""
    defs = _definitions(packages)

    def kind(sd):
        if sd.get("derivation") != "constraint":
            return False
        return sd.get("kind") == "resource" or (sd.get("kind") == "complex-type" and sd.get("type") in ("Identifier", "Extension"))

    chosen = {url for url, (_, _, sd) in defs.items() if kind(sd)}
    frontier = list(chosen)
    while frontier:
        for url in _held_alone(defs[frontier.pop()][2]):
            sd = defs.get(url, (None, None, {}))[2]
            if url not in chosen and sd.get("derivation") == "constraint" and sd.get("kind") == "complex-type":
                chosen.add(url)
                frontier.append(url)
    compiled, missing = {}, []
    for url in sorted(chosen):
        path, raw, sd = defs[url]
        if (sd.get("snapshot") or {}).get("element"):
            compiled[url] = (path, raw)
        else:
            missing.append(str(path))
    return compiled, missing


def source_path(path, packages, sources):
    """The host's own path for a staged file: `package-<i>/<name>` of the
    staged copies is `<name>` in the i-th source package dir. A path outside
    the staged packages, or with no sources given, is itself."""
    path = Path(path)
    for staged, source in zip(packages, sources or []):
        if path.parent == Path(staged):
            return str(Path(source) / path.name)
    return str(path)


def precheck(shipped, packages, sources=None):
    """Refuse, before anything is generated: a definition the generator would
    compile but has no snapshot (by name, at the path the host gave —
    `sources`, the package dirs `packages` were staged from, in order), and
    a host canonical the shipped registry already holds (pruning would serve
    the shipped copy instead)."""
    compiled, missing = compiled_definitions(packages)
    if missing:
        named = [source_path(m, packages, sources) for m in missing]
        raise SystemExit("definitions without a snapshot (refused — publish with snapshots):\n  " + "\n  ".join(named))
    both = sorted(set(compiled) & set(shipped))
    if both:
        raise SystemExit("host package redefines shipped canonicals (refused):\n  " + "\n  ".join(both))
    if not compiled:
        raise SystemExit("nothing to compile: no constraint profile with a snapshot in " + ", ".join(map(str, packages)))


def source_identities(packages):
    compiled, _ = compiled_definitions(packages)
    return {url: hashlib.sha256(raw).hexdigest() for url, (_, raw) in compiled.items()}


def rust_literal(value):
    return '"' + ''.join(
        '\\\\' if c == '\\' else '\\"' if c == '"' else
        f'\\u{{{ord(c):x}}}' if ord(c) < 32 else c for c in value
    ) + '"'


def record_sources(generated, packages):
    generated = Path(generated)
    identities = source_identities(packages)
    compiled = set()
    for path in generated.glob("*.rs"):
        match = re.search(r'^\s+url: ("(?:\\.|[^"\\])*"),', path.read_text(), re.M)
        if match:
            compiled.add(json.loads(match[1]))
    missing = set(identities) - compiled
    if missing:
        raise ValueError("host profile source was not compiled: " + ", ".join(sorted(missing)))
    rows = ''.join(f"    ({rust_literal(url)}, {rust_literal(digest)}),\n" for url, digest in sorted(identities.items()))
    with (generated / "mod.rs").open("a") as output:
        output.write("\n// SHA256 of the exact staged StructureDefinition inputs consumed above.\n")
        output.write("pub static SOURCE_SHA256: &[(&str, &str)] = &[\n" + rows + "];\n")


if __name__ == "__main__":
    command, destination, *packages = sys.argv[1:]
    if command == "stage":
        stage_packages(destination, packages)
    elif command == "record":
        record_sources(destination, packages)
    else:
        raise SystemExit("usage: host_profile_sources.py stage|record DEST PACKAGE...")
