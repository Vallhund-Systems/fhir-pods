import { instantiate } from "../src/index.js";
import { fixtures, patient, patientProfile, practitioner } from "../bench/fixtures.mjs";

const output = document.querySelector("#results");
const button = document.querySelector("#run");
const assert = (value, message) => { if (!value) throw new Error(message); };
async function run() {
  window.fhirPodsSmoke = null;
  button.disabled = true;
  output.dataset.status = "running";
  const checks = [];
  try {
    const response = await fetch(new URL("../dist/fhir_pods.wasm", import.meta.url));
    assert(response.ok, `WASM fetch failed: ${response.status}`);
    const bytes = await response.arrayBuffer();
    const wasmSha256 = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map((byte) => byte.toString(16).padStart(2, "0")).join("");
    const pod = await instantiate(bytes);
    for (const fixture of fixtures) {
      const result = pod.validate(fixture.json);
      assert(result.valid === fixture.valid, `${fixture.name}: ${JSON.stringify(result)}`);
      checks.push(`PASS validate ${fixture.name}`);
    }
    const lowered = pod.lower(patientProfile, {
      identifier: patient.identifier, name: patient.name, gender: patient.gender, birthDate: patient.birthDate,
    });
    assert(lowered.resource.resourceType === "Patient" && lowered.validation.valid, `lower: ${JSON.stringify(lowered)}`);
    checks.push("PASS lower AU Core Patient");
    const scrubbed = pod.deidentify(JSON.stringify(patient), { salt: "synthetic-browser-test-only", today: "2026-09-16" });
    assert(scrubbed.remaining.length === 0, `deidentify findings: ${JSON.stringify(scrubbed.remaining)}`);
    assert(!JSON.stringify(scrubbed.resource).includes("sample-001"), "Original identifier survived de-identification");
    assert(!JSON.stringify(scrubbed.resource).includes("Synthetic"), "Original given name survived de-identification");
    assert(scrubbed.validation.valid, `deidentify validation: ${JSON.stringify(scrubbed.validation)}`);
    checks.push("PASS deidentify synthetic identifiers and name");
    const assembled = pod.document("ips", {
      resources: [patient, practitioner], author: "Practitioner/sample-practitioner",
      date: "2026-09-16T12:00:00Z", title: "Synthetic browser smoke",
    });
    assert(assembled.document.resourceType === "Bundle" && assembled.document.type === "document", "Expected a document Bundle");
    assert(assembled.document.entry[0].resource.resourceType === "Composition", "Composition must be first");
    assert(assembled.validation.valid, `document validation: ${JSON.stringify(assembled.validation)}`);
    const again = pod.validate(JSON.stringify(assembled.document));
    assert(again.valid, `document roundtrip: ${JSON.stringify(again)}`);
    checks.push("PASS IPS document assembly and validation roundtrip");
    window.fhirPodsSmoke = { status: "passed", abi: pod.abiVersion, wasmBytes: bytes.byteLength, wasmSha256, checks };
    output.dataset.status = "passed";
  } catch (error) {
    window.fhirPodsSmoke = { status: "failed", checks, error: error.stack ?? String(error) };
    output.dataset.status = "failed";
  } finally {
    output.textContent = JSON.stringify(window.fhirPodsSmoke, null, 2);
    button.disabled = false;
  }
}
button.addEventListener("click", run);
await run();
