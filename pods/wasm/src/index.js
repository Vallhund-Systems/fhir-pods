// fhir-pods — the wasm pod's JS wrapper. Hand-written over the lean C ABI: no
// wasm-bindgen glue, no dependencies, no build step. Works anywhere
// WebAssembly does (browser, edge runtimes, Node, Bun, Deno).

/** The ABI this wrapper speaks; a module reporting anything else is refused. */
export const ABI_VERSION = 8;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * Instantiate the pod from the `.wasm` bytes (or a precompiled module).
 * Loading the bytes is the host's job — `fetch` in a browser, `readFile` in
 * Node — so this wrapper has no environment-specific code.
 *
 * `options.terminology(valueSet, system, code)` is the terminology seam: it
 * is asked, synchronously, about the bindings the bundled value sets (and a
 * table loaded with `loadTerminology`) can't decide. Return `true` (member),
 * `false` (not a member) or `undefined` (unknown — a warning, never an
 * error). `system` is "" for a bare `code`.
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

  // One call across the boundary: copy the inputs in, read the
  // length-prefixed document out, free everything. `memory.buffer` is re-read
  // after the call because the call may have grown (and so detached) it.
  function call(entry, ...jsons) {
    const buffers = jsons.map((j) => (typeof j === "string" ? encoder.encode(j) : j));
    const ptrs = buffers.map((b) => {
      const p = ex.pod_alloc(b.byteLength);
      new Uint8Array(ex.memory.buffer, p, b.byteLength).set(b);
      return p;
    });
    const ret = entry(...buffers.flatMap((b, i) => [ptrs[i], b.byteLength]));
    buffers.forEach((b, i) => ex.pod_free(ptrs[i], b.byteLength));
    const len = new DataView(ex.memory.buffer).getUint32(ret, true);
    const text = decoder.decode(new Uint8Array(ex.memory.buffer, ret + 4, len));
    ex.pod_free(ret, 4 + len);
    return text;
  }

  // Copy bytes in for an entrypoint that returns a plain integer.
  function callInt(entry, text) {
    const b = encoder.encode(text);
    const p = ex.pod_alloc(b.byteLength);
    new Uint8Array(ex.memory.buffer, p, b.byteLength).set(b);
    const r = entry(p, b.byteLength);
    ex.pod_free(p, b.byteLength);
    return r;
  }

  const validate = terminology ? ex.fhir_pods_validate_host_tx : ex.fhir_pods_validate;
  const outcome = terminology ? ex.fhir_pods_operation_outcome_host_tx : ex.fhir_pods_operation_outcome;
  const lowerEntry = terminology ? ex.fhir_pods_lower_host_tx : ex.fhir_pods_lower;
  const auditEntry = terminology ? ex.fhir_pods_audit_host_tx : ex.fhir_pods_audit;

  return {
    abiVersion: abi,
    validate: (json) => JSON.parse(call(validate, json)),
    operationOutcome: (json) => JSON.parse(call(outcome, json)),
    mustSupportGaps: (json) => JSON.parse(call(ex.fhir_pods_must_support, json)).mustSupportGaps,
    lower: (profileUrl, inputs) => {
      const out = JSON.parse(call(lowerEntry, profileUrl, typeof inputs === "string" ? inputs : JSON.stringify(inputs)));
      if (out.error) throw new Error(`fhir-pods: ${out.error}`);
      return out;
    },
    slots: (profileUrl) => {
      const out = JSON.parse(call(ex.fhir_pods_slots, profileUrl));
      if (out.error) throw new Error(`fhir-pods: ${out.error}`);
      return out.slots;
    },
    // Documents: an IPS / AU Patient Summary bundle from a patient and their resources.
    document: (kind, inputs) => {
      const out = JSON.parse(call(ex.fhir_pods_document, kind, typeof inputs === "string" ? inputs : JSON.stringify(inputs)));
      if (out.error) throw new Error(`fhir-pods: ${out.error}`);
      return out;
    },
    // Audit: an IHE BALP AuditEvent for a read / query / create / update /
    // delete (with or without a patient) from host-shaped inputs.
    audit: (kind, inputs) => {
      const out = JSON.parse(call(auditEntry, kind, typeof inputs === "string" ? inputs : JSON.stringify(inputs)));
      if (out.error) throw new Error(`fhir-pods: ${out.error}`);
      return out;
    },
    // De-identification: a Safe Harbor copy (by default) under a policy object.
    deidentify: (json, policy = {}) => {
      const out = JSON.parse(call(ex.fhir_pods_deidentify, json, JSON.stringify(policy)));
      if (out.error) throw new Error(`fhir-pods: ${out.error}`);
      return out;
    },
    // Compiled terminology: load a table (the line format tools/compile_valueset.py
    // writes from $expand responses). It answers after the bundled sets and
    // before `options.terminology`, for every call that follows. Returns false
    // when the text could not be parsed (the previous table stays); "" clears.
    loadTerminology: (text) => callInt(ex.fhir_pods_terminology_load, text) === 1,
    // What to resolve for a verdict free of terminology warnings.
    terminologyNeeds: (json) => JSON.parse(call(ex.fhir_pods_terminology_needs, json)).needs,
    validateText: (json) => call(validate, json),
    operationOutcomeText: (json) => call(outcome, json),
  };
}
