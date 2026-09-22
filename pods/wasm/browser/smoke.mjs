import { checkOperations } from "./operations.mjs";

const output = document.querySelector("#results");
const button = document.querySelector("#run");
async function run() {
  window.fhirPodsSmoke = null;
  button.disabled = true;
  output.dataset.status = "running";
  try {
    const response = await fetch(new URL("../dist/fhir_pods.wasm", import.meta.url));
    if (!response.ok) throw new Error(`WASM fetch failed: ${response.status}`);
    const bytes = await response.arrayBuffer();
    const wasmSha256 = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map(b=>b.toString(16).padStart(2,"0")).join("");
    const result = await checkOperations(bytes);
    window.fhirPodsSmoke = {status:"passed",wasmBytes:bytes.byteLength,wasmSha256,...result};
  } catch(error) {
    window.fhirPodsSmoke = {status:"failed",error:error.stack ?? String(error)};
  } finally {
    output.dataset.status=window.fhirPodsSmoke.status;
    output.textContent=JSON.stringify(window.fhirPodsSmoke,null,2);
    button.disabled=false;
  }
}
button.addEventListener("click",run);
await run();
