import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { instantiate } from "../../pods/wasm/src/index.js";
import { sample, toBaseRecord, prepareExport, liftReading, reviewPacket, approvedPayload, acknowledgeLimitations } from "./integration.mjs";

const pod = await instantiate(await readFile(process.env.FHIR_PODS_WASM ?? new URL("../../pods/wasm/dist/fhir_pods.wasm", import.meta.url)));
const options = { now: "2026-09-22T09:01:00Z" };

test("one unchanged host model maps to base definitions for every discovered destination", () => {
  const before = JSON.stringify(sample);
  const base = toBaseRecord(pod, sample);
  assert.ok(base.validations.every(v => v.valid));
  assert.deepEqual(base.resources.map(r => r.meta.profile), [
    ["http://hl7.org/fhir/StructureDefinition/Patient"],
    ["http://hl7.org/fhir/StructureDefinition/heartrate"],
  ]);
  const regions = pod.profiles().regions;
  assert.deepEqual(regions.map(r => r.name), ["au", "us", "uk", "ca", "nz", "ips"]);
  for (const region of regions) {
    const result = prepareExport(pod, sample, region.name, options);
    assert.equal(result.output.region.name, region.name);
    assert.deepEqual(result.base, base, region.name);
    assert.equal(result.output.resources[1].resource.valueQuantity.value, sample.reading.bpm);
    assert.equal(JSON.stringify(sample), before);
    if (result.decision.ready) {
      assert.ok(result.payloadValidation.valid);
      assert.deepEqual(result.decision.blockers, []);
    } else assert.throws(() => approvedPayload(result), /resolve the review findings/);
  }
});

test("US resources download includes validated profiles and a separately inspectable audit", () => {
  const initial = prepareExport(pod, sample, "us", options);
  if (initial.decision.unsupported.length) assert.throws(() => approvedPayload(initial));
  const result = acknowledgeLimitations(initial, true);
  assert.ok(result.decision.ready);
  assert.ok(result.decision.warnings.length > 0, "nonblocking warnings must stay visible");
  assert.equal(approvedPayload(result).type, "collection");
  assert.equal(result.payload.entry.length, 2);
  assert.ok(result.output.audit.validation.valid);
  assert.equal(result.output.audit.resource.recorded, options.now);
  assert.equal(result.output.audit.resource.action, "R");
  const packet = reviewPacket(result);
  assert.equal(packet.synthetic, true);
  assert.equal(packet.hostDecision.ready, true);
  assert.deepEqual(packet.hostModel, sample);
  assert.equal(packet.export.document, null);
});

test("a missing date is not invented; region requirements pause FHIR download", () => {
  const model = structuredClone(sample); model.person.birthDate = "";
  const result = prepareExport(pod, model, "au", options);
  assert.ok(!("birthDate" in result.base.resources[0]));
  assert.ok(!("birthDate" in result.output.resources[0].resource));
  assert.ok(result.decision.required.some(r => r.path === "birthDate"));
  assert.equal(result.decision.ready, false);
  assert.throws(() => approvedPayload(result));
  assert.equal(reviewPacket(result).hostDecision.ready, false);
});

test("qualifiers survive export, while lift exposes what its slots do not map", () => {
  const model = structuredClone(sample); model.reading.comparator = "<";
  const result = prepareExport(pod, model, "us", options);
  assert.equal(result.output.resources[1].resource.valueQuantity.comparator, "<");
  assert.ok(result.decision.carried.some(r => r.path === "valueQuantity.comparator"));
  assert.ok(acknowledgeLimitations(result, true).decision.ready);
  const lifted = liftReading(pod, result);
  assert.equal(lifted.inputs["valueQuantity.value"], 72);
  assert.ok(lifted.extras.includes("valueQuantity.comparator"));
  assert.deepEqual(lifted.conflicts, []);
});

test("unsupported checks are not suppressed by their informational severity", () => {
  const result = prepareExport(pod, sample, "au", options);
  for (const report of [...result.output.resources.map(r => r.validation), result.payloadValidation]) {
    for (const issue of report.issues.filter(i => i.code === "not-supported")) {
      assert.ok(result.decision.unsupported.some(i => i.path === issue.path && i.message === issue.message));
      assert.equal(result.decision.ready, false);
    }
  }
  // An engine upgrade may resolve that slice; retain the host-policy contract
  // with one explicit unsupported diagnostic on otherwise valid output.
  const reportingPod = { ...pod, validate: bytes => {
    const report = pod.validate(bytes);
    report.issues.push({ severity: "information", code: "not-supported", path: "Bundle", message: "Synthetic unevaluated check" });
    return report;
  }};
  const reported = prepareExport(reportingPod, sample, "us", options);
  assert.equal(reported.payloadValidation.valid, true);
  assert.ok(reported.decision.blockers.some(b => b.kind === "unsupported" && b.message === "Synthetic unevaluated check"));
  assert.throws(() => approvedPayload(reported));
  const accepted = acknowledgeLimitations(reported, true);
  assert.ok(accepted.decision.ready);
  assert.ok(reviewPacket(accepted).hostDecision.acceptedLimitations.some(i => i.message === "Synthetic unevaluated check"));
  assert.throws(() => approvedPayload(acknowledgeLimitations(accepted, false)));
});

test("acknowledging unsupported checks cannot waive missing data or unresolved terminology", () => {
  const model = structuredClone(sample); model.person.birthDate = "";
  const incomplete = acknowledgeLimitations(prepareExport(pod, model, "au", options), true);
  assert.equal(incomplete.decision.ready, false);
  assert.ok(incomplete.decision.blockers.some(b => b.kind === "missing"));
  assert.throws(() => approvedPayload(incomplete));
  const summary = acknowledgeLimitations(prepareExport(pod, sample, "au", { ...options, document: true }), true);
  assert.ok(summary.decision.blockers.some(b => b.kind === "terminology"));
  assert.throws(() => approvedPayload(summary));
  const rejectingPod = { ...pod, validate: bytes => {
    const report = pod.validate(bytes);
    report.valid = false;
    report.issues.push({ severity: "error", code: "not-supported", path: "Bundle", message: "Synthetic unsupported operation error" });
    return report;
  }};
  assert.throws(() => approvedPayload(acknowledgeLimitations(prepareExport(rejectingPod, sample, "us", options), true)));
});

test("offline summary terminology prevents a valid AU document being treated as fully checked", () => {
  const result = prepareExport(pod, sample, "au", { ...options, document: true });
  assert.equal(result.payload.type, "document");
  assert.ok(result.output.document.validation.valid);
  assert.ok(result.decision.needs.length > 0);
  assert.ok(result.decision.blockers.some(b => b.kind === "terminology"));
  assert.equal(result.decision.ready, false);
  assert.throws(() => approvedPayload(result));
});

test("IPS document findings remain visible and the review packet remains downloadable", () => {
  const result = prepareExport(pod, sample, "ips", { ...options, document: true });
  assert.equal(result.output.document.validation.valid, false);
  assert.ok(result.decision.blockers.some(b => b.kind === "validation"));
  assert.ok(result.decision.needs.length > 0);
  assert.equal(reviewPacket(result).downloadBundle.type, "document");
  assert.throws(() => approvedPayload(result));
});

test("coverage and selection notes gate output even when regional validation passes", () => {
  for (const region of ["uk", "ca", "nz"]) {
    const result = prepareExport(pod, sample, region, options);
    assert.ok(result.output.resources.every(r => r.validation.valid));
    for (const r of result.output.resources) {
      for (const path of r.extras) assert.ok(result.decision.blockers.some(b => b.kind === "extras" && b.path === path));
      for (const message of [r.note, ...r.notes].filter(Boolean)) assert.ok(result.decision.blockers.some(b => b.kind === "note" && b.message === message));
    }
    // A future runtime may carry more values or resolve selection; the host
    // decision must follow the current report, not require a historic gap.
    if (result.decision.uncarried.length || result.decision.notes.length) {
      assert.equal(result.decision.ready, false);
      assert.throws(() => approvedPayload(result));
    }
  }
});

test("invalid base input and operation errors cannot produce approved output", () => {
  const model = structuredClone(sample); model.reading.bpm = "seventy-two";
  const result = prepareExport(pod, model, "au", options);
  assert.ok(result.decision.blockers.some(b => b.kind === "validation" && b.label.startsWith("Base")));
  assert.throws(() => approvedPayload(result));
  assert.throws(() => approvedPayload(null));
  assert.throws(() => prepareExport(pod, sample, "uncompiled", options), /region must be/);
});
