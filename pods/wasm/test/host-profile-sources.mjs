// Explicit host-artifact proof, separate from the base-artifact Node test suite.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFile} from 'node:fs/promises';
import {instantiate} from '../src/index.js';
const input=await readFile(process.argv[2]);
const definition=JSON.parse(input);
const expected=createHash('sha256').update(input).digest('hex');
const pod=await instantiate(await readFile(new URL('../dist/fhir_pods.wasm',import.meta.url)));
const catalogue=pod.profiles();
const profile=catalogue.profiles.find(p=>p.url===definition.url);
assert.ok(profile,'Profile must be compiled');
assert.equal(profile.sourceSha256,expected,'Catalogue must attest exact input bytes');
const changed=Buffer.from(JSON.stringify({...definition,description:'Different raw declaration'}));
assert.notEqual(profile.sourceSha256,createHash('sha256').update(changed).digest('hex'));
const base=catalogue.profiles.find(p=>p.url==='http://hl7.org/fhir/StructureDefinition/Patient');
assert.equal(base?.sourceSha256,undefined,'Shipped profile must not claim an unrelated host source');
console.log(JSON.stringify({profile:profile.url,sourceSha256:profile.sourceSha256,abi:pod.abiVersion,verified:true}));
