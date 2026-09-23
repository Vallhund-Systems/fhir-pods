import {readFile} from 'node:fs/promises';
import {test} from 'node:test';
import {checkOperations} from '../browser/operations.mjs';

test('ABI 13 operation and provider result contracts', async()=> {
  await checkOperations(await readFile(new URL('../dist/fhir_pods.wasm',import.meta.url)));
});
