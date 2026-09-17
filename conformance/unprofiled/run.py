#!/usr/bin/env python3
"""Compare unprofiled examples from the six pinned IGs with the R4 oracle.

Examples and full diagnostics stay in the ignored corpus directory. Committed
results contain hashes, verdicts and issue locations, not publisher text.
"""
import argparse
import copy
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / 'conformance'))
from make_corpus import IGS

VERSION = '6.10.4'
SEVERITIES = {'fatal', 'error', 'warning', 'information'}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def strip_profiles(value, path='$', removed=None):
    """Remove declarations only on resource objects, including nested ones."""
    if removed is None:
        removed = []
    if isinstance(value, dict):
        if isinstance(value.get('resourceType'), str) and isinstance(value.get('meta'), dict):
            profiles = value['meta'].pop('profile', None)
            if profiles:
                removed.append({'path': path + '.meta.profile', 'profiles': profiles})
            if not value['meta']:
                value.pop('meta')
        for key, child in value.items():
            strip_profiles(child, path + '.' + key, removed)
    elif isinstance(value, list):
        for i, child in enumerate(value):
            strip_profiles(child, f'{path}[{i}]', removed)
    return removed


def local_result(process):
    if process.returncode not in (0, 1):
        raise ValueError(f'Local process failed: exit {process.returncode}')
    obj = json.loads(process.stdout)
    if not isinstance(obj, dict) or type(obj.get('valid')) is not bool or not isinstance(obj.get('issues'), list):
        raise ValueError('Malformed local report')
    if any(not isinstance(i, dict) or i.get('severity') not in SEVERITIES for i in obj['issues']):
        raise ValueError('Malformed local issue')
    issues_valid = not any(i['severity'] in ('error', 'fatal') for i in obj['issues'])
    if obj['valid'] != issues_valid or process.returncode != (0 if obj['valid'] else 1):
        raise ValueError('Local exit/verdict/issues disagree')
    return obj


def oracle_results(bundle, expected):
    if not isinstance(bundle, dict) or bundle.get('resourceType') != 'Bundle':
        raise ValueError('Expected a Bundle of OperationOutcomes')
    outcomes = {}
    for entry in bundle.get('entry', []):
        outcome = entry.get('resource', {})
        issues = outcome.get('issue')
        if outcome.get('resourceType') != 'OperationOutcome' or not isinstance(issues, list) or not issues:
            raise ValueError('Malformed oracle outcome')
        if any(not isinstance(i, dict) or i.get('severity') not in SEVERITIES or not i.get('code') for i in issues):
            raise ValueError('Malformed oracle issue')
        sources = [e.get('valueString') for e in outcome.get('extension', [])
                   if e.get('url', '').endswith('operationoutcome-file')]
        if len(sources) != 1 or not isinstance(sources[0], str) or not sources[0]:
            raise ValueError('Missing or ambiguous oracle source')
        name = Path(sources[0]).name
        if name in outcomes:
            raise ValueError('Duplicate oracle source')
        outcomes[name] = outcome
    if set(outcomes) != set(expected) or len(outcomes) != len(expected):
        raise ValueError('Missing or unexpected oracle results')
    return outcomes


def issue_index(issues, native=False):
    result = []
    for issue in issues:
        paths = [issue.get('path', '')] if native else issue.get('expression', issue.get('location', []))
        result.append({'severity': issue['severity'], 'code': issue.get('code'), 'paths': paths,
                       'issue_sha256': digest(json.dumps(issue, sort_keys=True).encode())})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=ROOT)
    parser.add_argument('--packages', type=Path, default=ROOT / 'conformance')
    parser.add_argument('--jar', type=Path, default=ROOT / 'conformance/validator_cli.jar')
    parser.add_argument('--corpus', type=Path, default=HERE / 'corpus')
    parser.add_argument('--output', type=Path, default=HERE / 'evidence.json')
    args = parser.parse_args()
    args.source, args.packages, args.jar, args.corpus = [p.resolve() for p in (args.source, args.packages, args.jar, args.corpus)]
    with zipfile.ZipFile(args.jar) as archive:
        props = archive.read('META-INF/maven/ca.uhn.hapi.fhir/org.hl7.fhir.validation.cli/pom.properties').decode()
    if f'version={VERSION}' not in props.splitlines():
        raise ValueError(f'Expected reference validator {VERSION}')
    args.corpus.mkdir(parents=True, exist_ok=True)
    manifest, packages = [], []
    for pkg, version, prefix, cache, _ in IGS:
        package = args.packages / cache / 'package'
        metadata = json.loads((package / 'package.json').read_text())
        if (metadata['name'], metadata['version']) != (pkg, version):
            raise ValueError(f'Wrong cached package: {pkg}#{version}')
        examples = sorted((package / 'example').glob('*.json'))
        if not examples:
            raise ValueError(f'Missing examples: {pkg}')
        selected = 0
        for path in examples:
            original = path.read_bytes()
            value = json.loads(original)
            if any(p.startswith(prefix) for p in (value.get('meta') or {}).get('profile', [])):
                continue
            transformed = copy.deepcopy(value)
            removed = strip_profiles(transformed)
            payload = (json.dumps(transformed, ensure_ascii=False, indent=2) + '\n').encode()
            filename = f'{pkg}--{path.name}'
            (args.corpus / filename).write_bytes(payload)
            manifest.append({'file': filename, 'package': pkg, 'version': version,
                             'example': 'example/' + path.name, 'resource_type': value['resourceType'],
                             'original_sha256': digest(original), 'input_sha256': digest(payload),
                             'removed_profiles': removed})
            selected += 1
        packages.append({'id': pkg, 'version': version, 'example_count': len(examples), 'selected': selected,
                         'package_metadata_sha256': digest((package / 'package.json').read_bytes())})
    if not manifest or len({m['file'] for m in manifest}) != len(manifest):
        raise ValueError('Empty or duplicate input manifest')
    (args.corpus / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    subprocess.run(['cargo', 'build', '-q', '-p', 'fhir-pods-core', '--example', 'probe'], cwd=args.source, check=True)
    native = {}
    for item in manifest:
        process = subprocess.run([str(args.source / 'target/debug/examples/probe'), str(args.corpus / item['file'])],
                                 capture_output=True, text=True, timeout=30)
        native[item['file']] = local_result(process)
    java = os.environ.get('JAVA') or (str(Path(os.environ['JAVA_HOME']) / 'bin/java') if os.environ.get('JAVA_HOME') else 'java')
    # Load definitions for recognized extensions, but apply no jurisdictional
    # profile. meta.profile is removed recursively on BOTH sides. R4 implied
    # vital-sign profiles remain the validators' normal base-spec behavior.
    options = ['-version', '4.0', '-tx', 'n/a', '-allow-example-urls', 'true']
    for pkg, version, *_ in IGS:
        options.extend(['-ig', f'{pkg}#{version}'])
    with tempfile.TemporaryDirectory(prefix='fhir-pods-unprofiled-oracle-') as temp:
        destination = Path(temp) / 'outcomes.json'
        command = [java, '-jar', str(args.jar), *[str(args.corpus / m['file']) for m in manifest],
                   *options, '-output', str(destination)]
        oracle = subprocess.run(command, capture_output=True, text=True, timeout=900)
        (args.corpus / 'oracle.log').write_text(oracle.stdout + oracle.stderr)
        if oracle.returncode != 0:
            raise ValueError(f'Oracle execution failed: exit {oracle.returncode}; see corpus/oracle.log')
        outcomes = oracle_results(json.loads(destination.read_text()), [m['file'] for m in manifest])
    records, full = [], []
    for item in manifest:
        local, outcome = native[item['file']], outcomes[item['file']]
        oracle_valid = not any(i['severity'] in ('error', 'fatal') for i in outcome['issue'])
        agrees = local['valid'] == oracle_valid
        record = {**item, 'local_valid': local['valid'], 'oracle_valid': oracle_valid, 'agrees': agrees,
                  'applied_profile': local.get('profile'),
                  'local_issues': issue_index(local['issues'], native=True), 'oracle_issues': issue_index(outcome['issue'])}
        records.append(record)
        full.append({**item, 'local': local, 'oracle': outcome})
        print(f"{'ok' if agrees else 'DIFF':4} {item['file']}: local={local['valid']} oracle={oracle_valid}", flush=True)
    (args.corpus / 'results.json').write_text(json.dumps(full, indent=2) + '\n')
    failures = sum(not r['agrees'] for r in records)
    report = {'recorded_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=args.source, text=True).strip(),
              'source_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=args.source, text=True)),
              'probe_sha256': digest((args.source / 'target/debug/examples/probe').read_bytes()),
              'runner_sha256': digest(Path(__file__).read_bytes()), 'oracle_version': VERSION,
              'oracle_sha256': digest(args.jar.read_bytes()), 'oracle_options': options, 'oracle_exit': oracle.returncode,
              'mode': 'No jurisdiction default; recursively remove meta.profile. Normal R4 implied profiles and known extension definitions remain.',
              'packages': packages, 'counts': {'total': len(records), 'agree': len(records)-failures, 'differ': failures},
              'cases': records}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report['counts']))
    return 1 if failures else 0


if __name__ == '__main__':
    raise SystemExit(main())
