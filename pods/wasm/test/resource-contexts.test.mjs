import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {test} from 'node:test';
import {instantiate} from '../src/index.js';
const wasm=await readFile(new URL('../dist/fhir_pods.wasm',import.meta.url));
const set='https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1';
const system='https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1';
const profile='http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient';
const table=code=>`fhir-pods-terminology 1\n= ${set}\n${system}\t${code}\n`;
const config=()=>({format:'fhir-pods.terminology-contexts.v1',contexts:[['one','AAA'],['two','BBB']].map(([id,code])=>({terminology:table(code),resources:[{resourceType:'Patient',id}]}))});
const patient=(id,code)=>({resourceType:'Patient',id,meta:{profile:[profile]},text:{status:'generated',div:'<div xmlns="http://www.w3.org/1999/xhtml">Synthetic</div>'},identifier:[{system:'https://example.invalid/mrn',value:id}],name:[{family:'Synthetic'}],gender:'unknown',birthDate:'1970-01-01',_birthDate:{extension:[{url:'http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator',valueCoding:{system,code}}]}});
const bundle=resources=>({resourceType:'Bundle',id:'mixed',type:'collection',entry:resources.map((resource,i)=>({fullUrl:`urn:uuid:00000000-0000-4000-8000-${String(i).padStart(12,'0')}`,resource}))});

test('resource contexts isolate the same profile through validation, lower/lift and nested collections',async()=>{
 const pod=await instantiate(wasm);assert.equal(pod.loadTerminology(table('CCC')),true);assert.equal(pod.loadTerminology(JSON.stringify(config())),true);
 for(const [id,own,foreign] of [['one','AAA','BBB'],['two','BBB','AAA']]){
  const good=patient(id,own),bad=patient(id,foreign);
  assert.equal(pod.validate(JSON.stringify(good)).valid,true);assert.equal(pod.validate(JSON.stringify(bad)).valid,false);
  const lifted=pod.lift(profile,JSON.stringify(good));assert.equal(lifted.validation.valid,true);
  assert.equal(pod.lower(profile,lifted.inputs).validation.valid,true);
  const badLift=pod.lift(profile,JSON.stringify(bad));assert.equal(pod.lower(profile,badLift.inputs).validation.valid,false);
 }
 const good=bundle([patient('one','AAA'),patient('two','BBB')]);const bad=bundle([patient('one','BBB'),patient('two','AAA')]);
 assert.equal(pod.validate(JSON.stringify(good)).valid,true);assert.equal(pod.validate(JSON.stringify(bad)).valid,false);
 assert.equal(pod.validate(JSON.stringify(bundle([bad]))).valid,false);
 assert.equal(pod.validate(JSON.stringify(patient('other','CCC'))).valid,true);
});

test('invalid installs preserve all prior contexts and global state; explicit clear/replacement is honest',async()=>{
 const pod=await instantiate(wasm);assert.equal(pod.loadTerminology(table('CCC')),true);assert.equal(pod.loadTerminology(JSON.stringify(config())),true);
 const cases=[];for(const key of ['id','resourceType'])for(const value of [1,true,null,{},[],'',' ']){const bad=config();bad.contexts[0].resources[0][key]=value;cases.push(bad);}
 let bad=config();bad.contexts[1].resources[0].id='one';cases.push(bad);bad=config();bad.contexts[0].terminology='invalid table';cases.push(bad);bad=config();bad.contexts[0].extra=true;cases.push(bad);
 for(const malformed of cases){assert.equal(pod.loadTerminology(JSON.stringify(malformed)),false);assert.equal(pod.validate(JSON.stringify(patient('one','AAA'))).valid,true);assert.equal(pod.validate(JSON.stringify(patient('one','BBB'))).valid,false);assert.equal(pod.validate(JSON.stringify(patient('other','CCC'))).valid,true);}
 assert.equal(pod.loadTerminology(JSON.stringify({format:config().format,contexts:[]})),true);assert.equal(pod.validate(JSON.stringify(patient('one','CCC'))).valid,true);
 assert.equal(pod.loadTerminology(JSON.stringify(config())),true);assert.equal(pod.loadTerminology(table('CCC')),true);assert.equal(pod.validate(JSON.stringify(patient('one','CCC'))).valid,true);
 assert.equal(pod.loadTerminology(''),true);assert.ok(pod.validate(JSON.stringify(patient('one','CCC'))).issues.some(i=>i.id==='binding.unknown'));
});

test('scoped unknowns are recorded without consulting a foreign callback and ambiguous input refuses',async()=>{
 let calls=0;const pod=await instantiate(wasm,{terminology:()=>{calls++;return true;}});const install=config();install.contexts[0].terminology='fhir-pods-terminology 1\n';assert.equal(pod.loadTerminology(JSON.stringify(install)),true);
 const input=JSON.stringify(patient('one','AAA'));const result=pod.validate(input);assert.ok(result.issues.some(i=>i.id==='binding.unknown'));assert.equal(calls,0);assert.equal(pod.terminologyNeeds(input).length,1);
 const duplicate=pod.validate(JSON.stringify(bundle([patient('one','AAA'),patient('one','AAA')])));assert.equal(duplicate.valid,false);assert.ok(duplicate.issues.some(i=>i.id==='terminology.context-ambiguous'));
});
