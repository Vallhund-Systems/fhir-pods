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


def source_identities(packages):
    seen = {}
    result = {}
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
                raise ValueError(f"duplicate host profile canonical {url}: {seen[url].name}, {path.name}")
            seen[url] = path
            compiled_kind = sd.get("kind") == "resource" or (
                sd.get("kind") == "complex-type" and sd.get("type") in ("Identifier", "Extension")
            )
            if sd.get("derivation") == "constraint" and compiled_kind and (sd.get("snapshot") or {}).get("element"):
                result[url] = hashlib.sha256(raw).hexdigest()
    return result


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
