#!/usr/bin/env python3
"""Run fixed synthetic cases through the native ABI and the pinned FHIR oracle.

  JAVA=/path/to/java python3 conformance/audit/run.py --output /tmp/audit-evidence

Input rejection, combined operation validation, preservation and validation of
the emitted resource are scored separately. Every emitted resource is passed
to validator_cli 6.10.4 with remote terminology disabled. All inputs and outputs
are synthetic. Results go to the explicit output folder, never a cached pass.
"""
import argparse
import ctypes
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import zipfile

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[1]

def encoded(value):
    return json.dumps(value,ensure_ascii=False,separators=(',',':')).encode()

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class Native:
    def __init__(self, path):
        self.lib = ctypes.CDLL(str(path))
        self.lib.pod_free.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
        self.lib.pod_free.restype = None
        self.lib.fhir_pods_abi_version.restype = ctypes.c_uint32
        for name, count in [("fhir_pods_audit", 2), ("fhir_pods_lower", 2), ("fhir_pods_validate", 1)]:
            fn = getattr(self.lib, name)
            fn.argtypes = [ctypes.c_void_p, ctypes.c_size_t] * count
            fn.restype = ctypes.c_void_p

    def call(self, name, *inputs):
        buffers = [ctypes.create_string_buffer(value) for value in inputs]
        args = [item for buffer, value in zip(buffers, inputs) for item in (ctypes.cast(buffer, ctypes.c_void_p), len(value))]
        pointer = getattr(self.lib, name)(*args)
        if not pointer:
            raise RuntimeError("null output")
        length = int.from_bytes(ctypes.string_at(pointer, 4), "little")
        try:
            return json.loads(ctypes.string_at(pointer + 4, length).decode("utf-8"))
        finally:
            self.lib.pod_free(pointer, 4 + length)



def run_suite(cases,inspect,output,jar):
    output=output.resolve();output.mkdir(parents=True,exist_ok=True)
    ids=[c['id'] for c in cases]
    if not ids or len(ids)!=len(set(ids)):raise ValueError('empty/duplicate case identities')
    with zipfile.ZipFile(jar) as z:
        metadata=z.read('META-INF/maven/ca.uhn.hapi.fhir/org.hl7.fhir.validation.cli/pom.properties').decode()
    if 'version=6.10.4' not in metadata.splitlines():raise ValueError('wrong reference validator')
    subprocess.run(['cargo','build','-q','-p','fhir-pods-ffi'],cwd=ROOT,check=True)
    library=ROOT/('target/debug/libfhir_pods_ffi.dylib' if sys.platform=='darwin' else 'target/debug/libfhir_pods_ffi.so')
    native=Native(library);library_hash=digest(library)
    records=[];files=[]
    for case in cases:
        result=native.call('fhir_pods_'+case['operation'],case['parameter'].encode(),encoded(case['inputs']))
        failures=inspect(case,result)
        record={'case':case,'result':result,'failures':failures}
        if 'resource' in result:
            revalidated=native.call('fhir_pods_validate',encoded(result['resource']))
            if not isinstance(revalidated.get('valid'),bool):raise ValueError('missing native verdict')
            record['revalidated']=revalidated
            path=output/(case['id']+'.json');path.write_bytes(encoded(result['resource']));files.append(path)
        records.append(record)
    (output/'native-results.json').write_text(json.dumps(records,ensure_ascii=False,indent=2)+'\n')
    java=os.environ.get('JAVA') or (str(Path(os.environ['JAVA_HOME'])/'bin/java') if os.environ.get('JAVA_HOME') else 'java')
    oracle_path=output/'oracle.json'
    if oracle_path.exists():oracle_path.unlink()
    igs=['hl7.fhir.au.core#2.0.0','hl7.fhir.us.core#6.1.0','hl7.fhir.uv.ipa#1.1.0','hl7.fhir.uv.ips#2.0.1','hl7.fhir.au.ps#1.0.0','ihe.iti.balp#1.1.4']
    command=[java,'-jar',str(jar),*map(str,files),'-version','4.0',*[a for ig in igs for a in ('-ig',ig)],'-tx','n/a','-allow-example-urls','true','-output',str(oracle_path)]
    with (output/'oracle.log').open('w') as log:proc=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,timeout=600)
    if proc.returncode not in [0,1] or not oracle_path.exists():raise RuntimeError('oracle execution failure: '+str(proc.returncode))
    spec=importlib.util.spec_from_file_location('document_runner',ROOT/'conformance/documents/run.py')
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    outcomes=module.outcomes_from(json.loads(oracle_path.read_text()),[p.name for p in files])
    for record in records:
        if 'revalidated' not in record:continue
        outcome=outcomes[record['case']['id']+'.json']
        record['oracle']=outcome
        record['oracle_valid']=not any(i['severity'] in ['error','fatal'] for i in outcome['issue'])
        if record['oracle_valid']!=record['revalidated']['valid']:record['failures'].append('resource verdict disagrees with oracle')
    if digest(library)!=library_hash:raise RuntimeError('native library changed during run')
    report={'recorded_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
            'dirty_paths':subprocess.check_output(['git','status','--porcelain'],cwd=ROOT,text=True).splitlines(),
            'library_sha256':library_hash,'jar_sha256':digest(jar),'cases_sha256':hashlib.sha256(encoded(cases)).hexdigest(),
            'abi':native.lib.fhir_pods_abi_version(),'oracle_version':'6.10.4','remote_terminology':'off',
            'counts':{'cases':len(records),'emitted':len(files),'pass':sum(not r['failures'] for r in records),'fail':sum(bool(r['failures']) for r in records)},'records':records}
    (output/'evidence.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    for record in records:
        if record['failures']:print(record['case']['id']+': '+', '.join(record['failures']))
    print(json.dumps(report['counts']))
    return 1 if report['counts']['fail'] else 0


def main():
    import cases
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--jar',type=Path,default=ROOT/'conformance/validator_cli.jar')
    args=parser.parse_args()
    return run_suite(cases.cases(),cases.inspect,args.output,args.jar.resolve())

if __name__=='__main__':sys.exit(main())

