// fhir-pods — the wasm pod's JS wrapper. Hand-written over the lean C ABI: no
// wasm-bindgen glue, no dependencies, no build step. Works anywhere
// WebAssembly does (browser, edge runtimes, Node, Bun, Deno).

/** The ABI this wrapper speaks; a module reporting anything else is refused. */
export const ABI_VERSION = 1;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * Instantiate the pod from the `.wasm` bytes (or a precompiled module).
 * Loading the bytes is the host's job — `fetch` in a browser, `readFile` in
 * Node — so this wrapper has no environment-specific code.
 */
export async function instantiate(source) {
  const module = source instanceof WebAssembly.Module ? source : await WebAssembly.compile(source);
  const { exports: ex } = await WebAssembly.instantiate(module, {});
  const abi = ex.fhir_pods_abi_version();
  if (abi !== ABI_VERSION) {
    throw new Error(`fhir-pods: module speaks ABI ${abi}, this wrapper expects ${ABI_VERSION}`);
  }

  // One call across the boundary: copy the input in, read the
  // length-prefixed document out, free both. `memory.buffer` is re-read after
  // the call because the call may have grown (and so detached) it.
  function call(entry, json) {
    const input = typeof json === "string" ? encoder.encode(json) : json;
    const inPtr = ex.pod_alloc(input.byteLength);
    new Uint8Array(ex.memory.buffer, inPtr, input.byteLength).set(input);
    const ret = entry(inPtr, input.byteLength);
    ex.pod_free(inPtr, input.byteLength);
    const len = new DataView(ex.memory.buffer).getUint32(ret, true);
    const text = decoder.decode(new Uint8Array(ex.memory.buffer, ret + 4, len));
    ex.pod_free(ret, 4 + len);
    return text;
  }

  return {
    abiVersion: abi,
    validate: (json) => JSON.parse(call(ex.fhir_pods_validate, json)),
    operationOutcome: (json) => JSON.parse(call(ex.fhir_pods_operation_outcome, json)),
    validateText: (json) => call(ex.fhir_pods_validate, json),
    operationOutcomeText: (json) => call(ex.fhir_pods_operation_outcome, json),
  };
}
