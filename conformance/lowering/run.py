#!/usr/bin/env python3
"""Run the fixed lowering examples; shared native/oracle runner is in ../audit.

JAVA=/path/to/java python3 conformance/lowering/run.py --output /tmp/lowering-evidence
"""
import argparse
import importlib.util
from pathlib import Path
import sys
import re
import json
import cases
HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[1]

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--jar',type=Path,default=ROOT/'conformance/validator_cli.jar')
    args=parser.parse_args()
    spec=importlib.util.spec_from_file_location('audit_runner',HERE.parent/'audit/run.py')
    runner=importlib.util.module_from_spec(spec);spec.loader.exec_module(runner)
    fixtures=cases.cases()
    declared=set(); extension_types=set()
    for path in (ROOT/'crates/fhir-pods-core/src/generated').glob('*.rs'):
        source=path.read_text()
        resource_type=re.search(r'resource_type: "([^"\n]+)"',source)
        if not resource_type:continue
        target=extension_types if resource_type[1]=="Extension" else declared
        target.update(re.findall(r'Slot\s*\{\s*path:\s*"[^"\n]+",\s*ty:\s*"([^"\n]+)"',source))
    covered={c['slot_type'] for c in fixtures if 'slot_type' in c}
    missing=declared-covered
    args.output.mkdir(parents=True,exist_ok=True)
    (args.output/'type-coverage.json').write_text(json.dumps({'scope':'resource_profile lookup (Extension excluded)', 'extension_only':sorted(extension_types-declared),'declared':sorted(declared),'covered':sorted(covered),'missing':sorted(missing)},indent=2)+'\n')
    if missing:raise ValueError('add fixed examples for new slot types: '+repr(sorted(missing)))
    return runner.run_suite(fixtures,cases.inspect,args.output,args.jar.resolve())

if __name__=='__main__':sys.exit(main())
