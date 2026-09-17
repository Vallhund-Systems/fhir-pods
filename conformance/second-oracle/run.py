#!/usr/bin/env python3
"""Compare the finite corpus with an independent, locally resolved Firely SDK.

Requires .NET 10 SDK, Rust, Python and the pinned Java validator 6.10.4.
  JAVA=/path/to/java python3 conformance/second-oracle/run.py --output /tmp/second-oracle

The Firely legacy validator is 5.11.0 with the R4 model SDK 5.13.2. All conformance
resources resolve from local pinned packages. External resource resolution is
disabled; LocalTerminologyService uses no external terminology service. Java
uses -tx n/a. Neither setting disables locally decidable code bindings.
Publisher fixtures, full diagnostics and derived outputs remain in --output.
The source contains no corpus, expansion or licensed terminology content.
"""
import argparse
import collections
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[1]
PACKAGES=[('package-r4','hl7.fhir.r4.core','4.0.1'),('package','hl7.fhir.au.core','2.0.0'),
          ('package-us','hl7.fhir.us.core','6.1.0'),('package-ipa','hl7.fhir.uv.ipa','1.1.0'),
          ('package-ips','hl7.fhir.uv.ips','2.0.1'),('package-au-ps','hl7.fhir.au.ps','1.0.0'),
          ('package-balp','ihe.iti.balp','1.1.4'),('package-au-base','hl7.fhir.au.base','6.0.0'),
          ('package-ext','hl7.fhir.uv.extensions.r4','5.3.0')]
MANIFESTS=['manifest.json','lowered-manifest.json','deid-manifest.json','doc-manifest.json','audit-manifest.json','v2-manifest.json']


def load_module(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module


def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def firely_verdict(record):
    if 'execution_error' in record:return 'execution-error'
    if 'parse_error' in record:return 'invalid'
    outcome=record.get('outcome')
    if not isinstance(outcome,dict) or outcome.get('resourceType')!='OperationOutcome':raise ValueError('malformed Firely output')
    issues=outcome.get('issue',[])
    if not isinstance(issues,list) or any(i.get('severity') not in ['information','warning','error','fatal'] for i in issues):raise ValueError('invalid Firely issue severity')
    bad=[i for i in issues if i['severity'] in ['error','fatal']]
    if any(i.get('code') in ['exception','not-supported'] for i in bad):return 'evaluation-error'
    if any(i.get('code')=='incomplete' for i in bad):return 'incomplete'
    if record.get('input_shape_changes'):return 'normalized-input'
    return 'invalid' if bad else 'valid'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--corpus',type=Path,default=ROOT/'conformance/corpus')
    parser.add_argument('--packages-root',type=Path,default=ROOT/'conformance')
    parser.add_argument('--jar',type=Path,default=ROOT/'conformance/validator_cli.jar')
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();out=args.output.resolve();out.mkdir(parents=True,exist_ok=True)
    manifest=[]
    for name in MANIFESTS:manifest+=json.loads((args.corpus/name).read_text())
    ids=[row['file'] for row in manifest]
    if not ids or len(ids)!=len(set(ids)):raise ValueError('empty or duplicate corpus identities')
    corpus=out/'corpus';corpus.mkdir(exist_ok=True)
    for name in ids:
        if Path(name).name!=name:raise ValueError('expected a corpus basename')
        shutil.copy2(args.corpus/name,corpus/name)
    packages=[];package_hashes={}
    for folder,name,version in PACKAGES:
        path=(args.packages_root/folder/'package').resolve()
        metadata=json.loads((path/'package.json').read_text())
        if (metadata.get('name'),metadata.get('version'))!=(name,version):raise ValueError('wrong package: '+folder)
        packages.append(str(path));package_hashes[name+'#'+version]=digest(path/'package.json')
    package_file=out/'packages.json';package_file.write_text(json.dumps(packages,indent=2)+'\n')
    with zipfile.ZipFile(args.jar) as z:
        metadata=z.read('META-INF/maven/ca.uhn.hapi.fhir/org.hl7.fhir.validation.cli/pom.properties').decode()
    if 'version=6.10.4' not in metadata.splitlines():raise ValueError('wrong Java reference version')
    subprocess.run(['dotnet','restore','--locked-mode'],cwd=HERE,check=True,stdout=sys.stderr)
    subprocess.run(['dotnet','build','-c','Release','--no-restore'],cwd=HERE,check=True,stdout=sys.stderr)
    executable=HERE/'bin/Release/net10.0/sdk-oracle.dll'
    controls={'valid':{'resourceType':'Patient','active':True},
              'normalized':{'resourceType':'Patient','active':'true'},
              'required':{'resourceType':'Observation','status':'final'},
              'primitive':{'resourceType':'Patient','birthDate':17},
              'unresolved':{'resourceType':'Patient','meta':{'profile':['https://example.org/StructureDefinition/missing']}}}
    requests=[]
    for name,resource in controls.items():
        p=out/('__control-'+name+'.json');p.write_text(json.dumps(resource))
        requests.append({'id':'__control-'+name,'path':str(p)})
    requests += [{'id':name,'path':str(corpus/name)} for name in ids]
    request_file=out/'requests.jsonl';request_file.write_text(''.join(json.dumps(q)+'\n' for q in requests))
    with request_file.open() as inp,(out/'firely.jsonl').open('w') as stream,(out/'firely.log').open('w') as log:
        process=subprocess.run(['dotnet',str(executable),str(package_file)],stdin=inp,stdout=stream,stderr=log,timeout=1800)
    if process.returncode:raise RuntimeError('Firely process failed: '+str(process.returncode))
    records=[json.loads(line) for line in (out/'firely.jsonl').read_text().splitlines()]
    if [r.get('id') for r in records]!=[q['id'] for q in requests]:raise ValueError('missing, extra or reordered Firely outputs')
    control_verdicts={r['id']:firely_verdict(r) for r in records[:len(controls)]}
    if control_verdicts!={'__control-valid':'valid','__control-normalized':'normalized-input','__control-required':'invalid','__control-primitive':'invalid','__control-unresolved':'incomplete'}:raise ValueError('oracle control failed: '+repr(control_verdicts))
    firely={r['id']:r for r in records[len(controls):]}
    runner=load_module('native_runner',ROOT/'conformance/audit/run.py')
    subprocess.run(['cargo','build','-q','-p','fhir-pods-ffi'],cwd=ROOT,check=True)
    library=ROOT/('target/debug/libfhir_pods_ffi.dylib' if sys.platform=='darwin' else 'target/debug/libfhir_pods_ffi.so')
    native=runner.Native(library)
    local={name:native.call('fhir_pods_validate',(corpus/name).read_bytes()) for name in ids}
    if any(not isinstance(r.get('valid'),bool) for r in local.values()):raise ValueError('missing native verdict')
    (out/'native.json').write_text(json.dumps(local,indent=2)+'\n')
    java=os.environ.get('JAVA') or (str(Path(os.environ['JAVA_HOME'])/'bin/java') if os.environ.get('JAVA_HOME') else 'java')
    oracle_file=out/'java-outcomes.json'
    if oracle_file.exists():oracle_file.unlink()
    igs=[name+'#'+version for _,name,version in PACKAGES[1:7]]
    command=[java,'-jar',str(args.jar.resolve()),*[str(corpus/name) for name in ids],'-version','4.0',*[a for ig in igs for a in ('-ig',ig)],'-tx','n/a','-allow-example-urls','true','-output',str(oracle_file)]
    with (out/'java.log').open('w') as log:p=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,timeout=900)
    if p.returncode not in [0,1] or not oracle_file.exists():raise RuntimeError('Java oracle failed')
    docs=load_module('document_runner',ROOT/'conformance/documents/run.py')
    java_outcomes=docs.outcomes_from(json.loads(oracle_file.read_text()),ids)
    rows=[]
    for name in ids:
        j='invalid' if any(i['severity'] in ['error','fatal'] for i in java_outcomes[name]['issue']) else 'valid'
        rows.append({'file':name,'sha256':digest(corpus/name),'native':'valid' if local[name]['valid'] else 'invalid','java':j,'firely':firely_verdict(firely[name]),'firely_parse_rejection':'parse_error' in firely[name],'firely_input_shape_changes':firely[name].get('input_shape_changes',[])})
    comparable=[r for r in rows if r['firely'] in ['valid','invalid']]
    counts={'total':len(rows),'comparable':len(comparable),'native_java_agree':sum(r['native']==r['java'] for r in rows),
            'three_agree':sum(r['native']==r['java']==r['firely'] for r in comparable),'firely_states':dict(collections.Counter(r['firely'] for r in rows)),
            'firely_parse_rejections':sum(r['firely_parse_rejection'] for r in rows)}
    report={'recorded_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
            'dirty_paths':subprocess.check_output(['git','status','--porcelain'],cwd=ROOT,text=True).splitlines(),
            'dotnet':subprocess.check_output(['dotnet','--version'],text=True).strip(),'java':subprocess.run([java,'-version'],capture_output=True,text=True,check=True).stderr.strip(),
            'validator':'Hl7.Fhir.Validation.Legacy.R4 5.11.0 / Hl7.Fhir.R4 5.13.2','remote_terminology':'off; local code checks remain enabled','external_resource_resolution':False,
            'control_verdicts':control_verdicts,'counts':counts,'packages':package_hashes,'library_sha256':digest(library),'java_jar_sha256':digest(args.jar),'firely_dll_sha256':digest(executable),
            'runner_sha256':{p.name:digest(p) for p in HERE.iterdir() if p.is_file()},'rows':rows}
    (out/'summary.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(counts,indent=2))
    return 0 if len(comparable)==len(rows) and counts['three_agree']==len(rows) else 1

if __name__=='__main__':sys.exit(main())
