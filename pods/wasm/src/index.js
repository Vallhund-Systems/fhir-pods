// fhir-pods — the wasm pod's JS wrapper. Hand-written over the lean C ABI: no
// wasm-bindgen glue, no dependencies, no build step. Works anywhere
// WebAssembly does (browser, edge runtimes, Node, Bun, Deno).

/** The ABI this wrapper speaks; a module reporting anything else is refused. */
export const ABI_VERSION = 3;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * Instantiate the pod from the `.wasm` bytes (or a precompiled module).
 * Loading the bytes is the host's job — `fetch` in a browser, `readFile` in
 * Node — so this wrapper has no environment-specific code.
 *
 * `options.terminology(valueSet, system, code)` is the terminology seam: it
 * is asked, synchronously, about the bindings the bundled value sets can't
 * decide. Return `true` (member), `false` (not a member) or `undefined`
 * (unknown — a warning, never an error). `system` is "" for a bare `code`.
 */
export async function instantiate(source, options = {}) {
  const module = source instanceof WebAssembly.Module ? source : await WebAssembly.compile(source);
  const terminology = options.terminology;
  let memory; // set once instantiated; the import reads strings out of it

  const imports = {
    env: {
      fhir_pods_host_tx(vsPtr, vsLen, sysPtr, sysLen, codePtr, codeLen) {
        if (!terminology) return -1;
        const str = (p, n) => decoder.decode(new Uint8Array(memory.buffer, p, n));
        try {
          const r = terminology(str(vsPtr, vsLen), str(sysPtr, sysLen), str(codePtr, codeLen));
          return r === true ? 1 : r === false ? 0 : -1;
        } catch {
          return -1; // a throwing provider is an unknown answer, not a crash
        }
      },
    },
  };
  const { exports: ex } = await WebAssembly.instantiate(module, imports);
  memory = ex.memory;
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

  const validate = terminology ? ex.fhir_pods_validate_host_tx : ex.fhir_pods_validate;
  const outcome = terminology ? ex.fhir_pods_operation_outcome_host_tx : ex.fhir_pods_operation_outcome;

  return {
    abiVersion: abi,
    validate: (json) => JSON.parse(call(validate, json)),
    operationOutcome: (json) => JSON.parse(call(outcome, json)),
    mustSupportGaps: (json) => JSON.parse(call(ex.fhir_pods_must_support, json)).mustSupportGaps,
    validateText: (json) => call(validate, json),
    operationOutcomeText: (json) => call(outcome, json),
  };
}
