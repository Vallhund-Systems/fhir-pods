import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { instantiate } from "../src/index.js";

const wasm = await readFile(new URL("../dist/fhir_pods.wasm", import.meta.url));
const url = "https://forms.example.invalid/Questionnaire/versioned";
const set = "https://forms.example.invalid/ValueSet/shared";
const system = "https://forms.example.invalid/CodeSystem/shared";
const base = "http://hl7.org/fhir/StructureDefinition/QuestionnaireResponse";
const text = { status: "generated", div: '<div xmlns="http://www.w3.org/1999/xhtml">Synthetic context example</div>' };
const patient = { resourceType: "Patient", id: "synthetic", text, identifier: [{system: "https://forms.example.invalid/subject", value: "synthetic"}], name: [{family: "Example", given: ["Synthetic"]}], gender: "unknown", birthDate: "2000-01-01" };
const form = (version) => ({ resourceType: "Questionnaire", id: `form-${version}`, url, version, status: "active", text,
  item: [{ linkId: "answer", type: "choice", required: true, answerValueSet: set }] });
const table = (code) => `fhir-pods-terminology 1\n= ${set}\n${system}\t${code}\n`;
const context = (version, terminology) => JSON.stringify({ format: "fhir-pods.questionnaire-context.v1", questionnaire: form(version), terminology });
const response = (version, code) => ({ resourceType: "QuestionnaireResponse", id: `response-${version}`, questionnaire: `${url}|${version}`, status: "completed", text, subject: { reference: "Patient/synthetic" }, authored: "2026-10-26T00:00:00Z",
  item: [{ linkId: "answer", answer: [{ valueCoding: { system, code } }] }] });
const collection = (one, two) => ({ resourceType: "Bundle", id: "mixed", type: "collection",
  entry: [patient, form("1"), form("2"), one, two].map((resource) => ({ fullUrl: `https://forms.example.invalid/${resource.resourceType}/${resource.id}`, resource })) });

test("exact questionnaire contexts isolate mixed versions through validation, lower, lift and export", async () => {
  const pod = await instantiate(wasm);
  assert.equal(pod.abiVersion, 14);
  assert.equal(pod.loadQuestionnaire(context("1", table("one"))), true);
  assert.equal(pod.loadQuestionnaire(context("2", table("two"))), true);
  const good = [response("1", "one"), response("2", "two")];
  const wrong = [response("1", "two"), response("2", "one")];
  for (const resource of good) {
    assert.deepEqual(pod.validate(JSON.stringify(resource)).issues, []);
    assert.deepEqual(pod.terminologyNeeds(JSON.stringify(resource)), []);
  }
  for (const resource of wrong) {
    const json = JSON.stringify(resource);
    assert.equal(pod.validate(json).valid, false);
    const { resourceType, ...inputs } = resource;
    assert.equal(pod.lower(base, inputs).validation.valid, false);
    assert.equal(pod.lift(base, resource).validation.valid, false);
  }
  assert.deepEqual(pod.validate(JSON.stringify(collection(...good))).issues, []);
  const bad = collection(...wrong);
  const verdict = pod.validate(JSON.stringify(bad));
  assert.equal(verdict.valid, false);
  assert.equal(verdict.issues.filter((issue) => issue.id === "questionnaire.answer-not-in-set").length, 2);
  assert.equal(pod.operationOutcome(JSON.stringify(bad)).issue.filter((issue) => issue.details?.coding?.some((code) => code.code === "questionnaire.answer-not-in-set")).length, 2);
  const exportResources = (responses) => pod.export("us", { resources: [patient, form("1"), form("2"), ...responses], document: false, author: "Patient/synthetic", date: "2026-10-26T00:00:00Z" }).resources.filter((resource) => resource.resourceType === "QuestionnaireResponse");
  const exportedGood = exportResources(good);
  assert.equal(exportedGood.length, 2);
  assert.ok(exportedGood.every((resource) => resource.validation.valid), JSON.stringify(exportedGood));
  const exportedWrong = exportResources(wrong);
  assert.equal(exportedWrong.length, 2);
  assert.ok(exportedWrong.every((resource) => !resource.validation.valid));
});

test("unknown scoped membership remains observable without global-table or callback borrowing", async () => {
  let callbackCalls = 0;
  const pod = await instantiate(wasm, { terminology: () => { callbackCalls++; return true; } });
  assert.equal(pod.loadTerminology(table("two")), true);
  assert.equal(pod.loadQuestionnaire(context("1", "fhir-pods-terminology 1\n")), true);
  const json = JSON.stringify(response("1", "two"));
  const unknown = pod.validate(json);
  assert.ok(unknown.issues.some((issue) => issue.severity === "warning" && issue.id === "binding.unknown"), JSON.stringify(unknown));
  const needs = pod.terminologyNeeds(json);
  assert.equal(needs.length, 1);
  assert.equal(needs[0].valueSet, set);
  assert.equal(callbackCalls, 0);
  // A plain replacement explicitly returns that form to legacy provider behavior.
  assert.equal(pod.loadQuestionnaire(JSON.stringify(form("1"))), true);
  assert.equal(pod.loadTerminology(""), true);
  assert.equal(pod.validate(json).valid, true);
  assert.ok(callbackCalls > 0);
});

test("invalid envelopes preserve prior contexts, and clear removes their tables", async () => {
  const pod = await instantiate(wasm);
  assert.equal(pod.loadQuestionnaire(context("1", table("one"))), true);
  assert.equal(pod.loadQuestionnaire(context("2", table("two"))), true);
  assert.equal(pod.loadQuestionnaire(context("1", "invalid table")), false);
  assert.equal(pod.loadQuestionnaire(context("1", table("two")).replace('"version":"1"', '"version":"1","version":"2"')), false);
  assert.equal(pod.loadQuestionnaire(context("1", table("two")).replace("fhir-pods.questionnaire-context.v1", "unknown-format")), false);
  assert.equal(pod.validate(JSON.stringify(response("1", "one"))).valid, true);
  assert.equal(pod.validate(JSON.stringify(response("1", "two"))).valid, false);
  assert.equal(pod.validate(JSON.stringify(response("2", "two"))).valid, true);
  const unversioned = { ...response("1", "one"), questionnaire: url };
  assert.ok(pod.validate(JSON.stringify(unversioned)).issues.some((issue) => issue.id === "questionnaire.context-version-required"));
  assert.equal(pod.loadQuestionnaire(""), true);
  assert.equal(pod.loadTerminology(table("two")), true);
  assert.equal(pod.loadQuestionnaire(JSON.stringify(form("1"))), true);
  assert.equal(pod.validate(JSON.stringify(response("1", "two"))).valid, true);
});
