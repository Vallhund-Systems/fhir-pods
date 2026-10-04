// Bounded synthetic ABI 13 checks shared by Chrome and Node.
import { ABI_VERSION, instantiate } from "../src/index.js";
import { fixtures, patient, patientProfile, practitioner, heartRate } from "../bench/fixtures.mjs";

const HR = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";
const assert = (condition, message) => { if (!condition) throw new Error(message); };
const same = (a, b, label) => assert(JSON.stringify(a) === JSON.stringify(b), label);
const rejects = (fn, text) => { let error; try { fn(); } catch (e) { error = e; } assert(error?.message.includes(text), `Expected error containing ${text}`); };
export const txPatient = { ...patient, _birthDate: { extension: [{ url: "http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator", valueCoding: { system: "https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1", code: "AAA" } }] } };
export const txTable = "fhir-pods-terminology 1\n= https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1\nhttps://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\tAAA\tSynthetic\n";
export const auditInputs = { recorded: "2026-09-22T00:00:00Z", client: { who: "Device/app", network: "10.0.0.7" }, server: { who: "Device/server", network: "https://example.org/r4" }, source: { observer: "Device/audit" }, patient: "Patient/sample-patient", data: "Observation/sample-heart-rate" };
export const v2Message = "MSH|^~\\&|LAB|SITE|APP|SITE|20260922100000+1000||ORU^R01|SYNTHETIC|P|2.5\rPID|1||synthetic^^^LOCAL^MR||Example^Synthetic||19800101|U\rOBR|1|||8867-4^Synthetic^LN\rOBX|1|NM|8867-4^Synthetic^LN||72|/min^Synthetic^UCUM|||||F\r";
const exportInput = { resources: [patient, heartRate], author: "Patient/sample-patient", date: "2026-09-22T00:00:00Z", audit: auditInputs };
const patientInputs = { id: patient.id, identifier: patient.identifier, name: patient.name, gender: patient.gender, birthDate: patient.birthDate };

export async function checkOperations(bytes) {
  const pod = await instantiate(bytes);
  const checks = [];
  const check = (name, fn) => { try { fn(); checks.push({ name, pass: true }); } catch (e) { throw new Error(`${name}: ${e.message}`, { cause: e }); } };
  check("ABI matches wrapper", () => assert(pod.abiVersion === ABI_VERSION && pod.abiVersion === 13, "ABI mismatch"));
  for (const fixture of fixtures) check(`validate ${fixture.name}`, () => assert(pod.validate(fixture.json).valid === fixture.valid, "Unexpected verdict"));
  check("OperationOutcome", () => assert(pod.operationOutcome("{").issue.some(i => i.severity === "fatal"), "Fatal parse finding missing"));
  check("discovery regions and profile kinds", () => {
    const d = pod.profiles(); same(d.regions.map(r => r.name), ["au", "us", "uk", "ca", "nz", "ips"], "Region inventory");
    assert(d.profiles.some(p => p.url === HR && p.kind === "resource"), "Heart-rate profile missing");
    assert(d.profiles.some(p => p.kind === "extension"), "Extension inventory missing");
  });
  check("slot fixed values and composite parts", () => {
    const unit = pod.slots(HR).find(s => s.path === "valueQuantity.unit");
    assert(unit.min === 0 && unit.fixed.some(f => f.path.endsWith("code") && f.value === "/min"), "Fixed unit metadata");
    assert(pod.slots(patientProfile).find(s => s.path === "name").parts.some(p => p.name === "given"), "Name parts missing");
  });
  check("must-support gaps", () => assert(pod.mustSupportGaps(JSON.stringify({resourceType:"Patient",meta:{profile:[patientProfile]}})).length > 0, "No gaps"));
  check("lower/lift preserve variable content", () => {
    const lower = pod.lower(patientProfile, patientInputs);
    const lift = pod.lift(patientProfile, lower.resource);
    assert(lower.validation.valid && lift.validation.valid && !lift.missing.length && !lift.conflicts.length, "Invalid round trip");
    same(pod.lower(patientProfile, lift.inputs).resource, lower.resource, "Resource changed");
  });
  const questionnaireProfile = "http://hl7.org/fhir/StructureDefinition/Questionnaire";
  const responseProfile = "http://hl7.org/fhir/StructureDefinition/QuestionnaireResponse";
  const questionnaireUrl = "http://forms.example.net/Questionnaire/browser-intake";
  const questionnaire = pod.lower(questionnaireProfile, {
    url: questionnaireUrl, version: "1", status: "active",
    item: [{ linkId: "age", type: "integer", required: true }],
  }).resource;
  const responseInputs = answer => ({
    questionnaire: `${questionnaireUrl}|1`, status: "completed", subject: "Patient/1",
    item: [{ linkId: "age", ...(answer ? { answer } : {}) }],
  });
  const responseVerdicts = answer => {
    const lowered = pod.lower(responseProfile, responseInputs(answer));
    return {
      lower: lowered.validation,
      lift: pod.lift(responseProfile, lowered.resource).validation,
      validate: pod.validate(JSON.stringify(lowered.resource)),
    };
  };
  const messages = verdict => verdict.issues.map(issue => issue.message).join("\n");
  check("questionnaire store reaches lower, lift and validate", () => {
    for (const verdict of Object.values(responseVerdicts([{ valueString: "forty-two" }]))) {
      assert(verdict.valid && messages(verdict).includes("could not be resolved"), "Unloaded form was assumed");
    }
    assert(pod.loadQuestionnaire(JSON.stringify(questionnaire)), "Questionnaire load failed");
    for (const [answer, finding] of [
      [[{ valueString: "forty-two" }], "Answer value must be of the type integer not string"],
      [undefined, "No response answer found for required item 'age'"],
      [[{ valueInteger: 42 }], null],
    ]) {
      for (const verdict of Object.values(responseVerdicts(answer))) {
        assert(verdict.valid === !finding, `Unexpected questionnaire verdict: ${messages(verdict)}`);
        if (finding) assert(messages(verdict).includes(finding), `Missing questionnaire finding: ${finding}`);
        assert(!messages(verdict).includes("could not be resolved"), "Loaded form was unresolved");
      }
    }
  });
  const isolatedPod = await instantiate(bytes);
  check("questionnaire store is isolated and clearable", () => {
    const isolated = isolatedPod.lower(responseProfile, responseInputs(undefined)).validation;
    assert(isolated.valid && messages(isolated).includes("could not be resolved"), "Questionnaire leaked between instances");
    assert(pod.loadQuestionnaire(""), "Questionnaire clear failed");
    for (const verdict of Object.values(responseVerdicts(undefined))) {
      assert(verdict.valid && messages(verdict).includes("could not be resolved"), "Cleared form still resolved");
    }
  });
  check("lift conflicts cross the ABI", () => {
    const conflict = structuredClone(heartRate); conflict.valueQuantity.code = "Hz";
    assert(pod.lift(HR, conflict).conflicts.some(c => c.path === "valueQuantity.code" && c.given === "Hz" && c.fixed === "/min"), "Conflict missing");
  });
  check("deidentify result and residual", () => {
    const copy = pod.deidentify(JSON.stringify(patient), {salt:"synthetic-browser",today:"2026-09-22"});
    assert(copy.validation.valid && !copy.remaining.length && !copy.residual.length, "Unexpected findings");
    assert(!JSON.stringify(copy.resource).includes("sample-001") && !JSON.stringify(copy.resource).includes("Synthetic"), "Original identifying values survived");
  });
  check("IPS document assembly", () => {
    const out = pod.document("ips", {resources:[patient,practitioner],author:"Practitioner/sample-practitioner",date:"2026-09-22T00:00:00Z"});
    assert(out.validation.valid && pod.validate(JSON.stringify(out.document)).valid, "Invalid summary");
    assert(out.document.entry[0].resource.resourceType === "Composition", "Composition ordering");
  });
  check("audit", () => { const a = pod.audit("patient-read", auditInputs); assert(a.resource.resourceType === "AuditEvent" && a.validation.valid, "Invalid audit"); });
  check("v2 conversion and coverage fields", () => {
    const out = pod.v2(v2Message); assert(out.bundle.type === "message", "Missing bundle");
    assert([out.skipped,out.unplaced,out.unmapped].every(Array.isArray), "Missing coverage");
  });
  check("AU export document and audit", () => {
    const out = pod.export("au", exportInput);
    assert(out.document.validation.valid && out.audit.validation.valid, "Invalid export artifacts");
    assert(out.resources[1].profile.url === HR && out.resources.every(r=>r.validation.valid), "Wrong resource selection");
  });
  check("export preserves qualifiers and carriers", () => {
    const resource = structuredClone(heartRate);
    resource.valueQuantity.comparator = "<";
    resource.valueQuantity._code = {extension:[{url:"https://example.org/precision",valueString:"synthetic"}]};
    const out = pod.export("au", {resources:[patient,resource],document:false}).resources[1];
    assert(out.resource.valueQuantity.comparator === "<", "Comparator lost");
    same(out.resource.valueQuantity._code, resource.valueQuantity._code, "Carrier lost");
    assert(out.carried.includes("valueQuantity.comparator") && out.carried.includes("valueQuantity._code"), "Missing carried paths");
  });
  check("export refuses fixed-value conversion", () => {
    const resource = structuredClone(heartRate); resource.valueQuantity.code="Hz";
    const out = pod.export("au", {resources:[patient,resource],document:false}).resources[1];
    assert(out.profile === null && out.note.includes("not re-profiled") && out.resource.valueQuantity.code === "Hz", "Conflict changed data");
  });
  check("export patient reference forms", () => {
    for (const reference of ["Patient/OTHER", "https://example.org/r4/Patient/OTHER"]) {
      const resource = structuredClone(heartRate); resource.subject.reference=reference;
      rejects(()=>pod.export("au",{resources:[patient,resource],document:false}), "not this record's Patient");
    }
    const version = structuredClone(heartRate); version.subject.reference="Patient/sample-patient/_history/7";
    const out = pod.export("au",{resources:[patient,version],document:false});
    assert(!out.resources[1].notes.some(n=>n.includes("patient identity")), "Version rejected as foreign");
    version.subject.reference="https://example.org/r4/Patient/sample-patient";
    assert(pod.export("au",{resources:[patient,version],document:false}).resources[1].notes.some(n=>n.includes("patient identity not established")), "Absolute identity assumed");
    rejects(()=>pod.export("au",{resources:[patient,{...patient,id:"other"}],document:false}), "more than one Patient");
  });
  check("unknown region and rejected operation", () => { rejects(()=>pod.export("unknown",exportInput), "region must be"); rejects(()=>pod.slots("https://example.org/unknown"), "profile"); });
  check("terminology load, rejected replacement and clear", () => {
    assert(pod.terminologyNeeds(JSON.stringify(txPatient)).length > 0, "Expected need");
    assert(pod.loadTerminology(txTable), "Load failed");
    assert(!pod.loadTerminology("malformed table"), "Accepted malformed table");
    assert(!pod.terminologyNeeds(JSON.stringify(txPatient)).length, "Table not preserved");
    assert(pod.loadTerminology("") && pod.terminologyNeeds(JSON.stringify(txPatient)).length > 0, "Clear failed");
  });
  let callbackCount=0;
  const provider = await instantiate(bytes, {terminology(set,system,code) {
    callbackCount++;
    set = set.split("|")[0];
    if (set.endsWith("date-accuracy-indicator-1")) return code === "AAA";
    // Synthetic negative membership controls for the heart-rate fixture;
    // no clinical expansion table or licensed terminology is bundled here.
    if (set.endsWith("edd-method-uv-ips") || set.endsWith("pregnancies-summary-uv-ips")) return false;
    return undefined;
  }});
  const withCallback = (name, fn) => check(name, ()=> {const before=callbackCount;fn();assert(callbackCount>before,"Provider was not called");});
  withCallback("provider validate", ()=>assert(provider.validate(JSON.stringify(txPatient)).valid,"Invalid patient"));
  withCallback("provider OperationOutcome", ()=>assert(!provider.operationOutcome(JSON.stringify(txPatient)).issue.some(i=>["error","fatal"].includes(i.severity)),"Error outcome"));
  withCallback("provider lower", ()=>assert(provider.lower(patientProfile,{...patientInputs,_birthDate:txPatient._birthDate}).validation.valid,"Invalid lower"));
  withCallback("provider lift", ()=>assert(provider.lift(patientProfile,txPatient).validation.valid,"Invalid lift"));
  withCallback("provider document", ()=>assert(provider.document("ips",{resources:[txPatient],author:`Patient/${patient.id}`,date:"2026-09-22T00:00:00Z"}).validation.valid,"Invalid document"));
  withCallback("provider US export", ()=>assert(provider.export("us",{...exportInput,resources:[txPatient,heartRate]}).document.validation.valid,"Invalid US summary"));
  check("provider audit and v2 routes", ()=> {
    assert(provider.audit("patient-read",auditInputs).validation.valid,"Invalid audit");
    assert(provider.v2(v2Message).bundle.type === "message","Missing v2 bundle");
  });
  let reentrantCount=0, nested;
  const reentrant = await instantiate(bytes,{terminology() {
    reentrantCount++;
    assert(nested.loadTerminology(txTable),"Reentrant load failed");
    assert(nested.validate(JSON.stringify(patient)).valid,"Nested validation failed");
    return true;
  }});
  nested=reentrant;
  check("provider reentrant load and validation", ()=> {
    assert(reentrant.validate(JSON.stringify(txPatient)).valid && reentrantCount>0,"Callback failed");
    assert(!reentrant.terminologyNeeds(JSON.stringify(txPatient)).length,"Loaded table not visible to next call");
  });
  return {abi:pod.abiVersion,checks,callbackCount,reentrantCount};
}
