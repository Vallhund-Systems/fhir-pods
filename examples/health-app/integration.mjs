// The host adapter knows its own model and base R4. Regions belong to export().
export const BASE = "http://hl7.org/fhir/StructureDefinition/";
export const sample = {
  person: { id: "demo-person", given: "Avery", family: "Reed", birthDate: "1988-04-12", gender: "unknown" },
  reading: { id: "demo-pulse", bpm: 72, comparator: "", measuredAt: "2026-09-22T09:00:00Z" },
};

export function toBaseRecord(pod, model) {
  const { person, reading } = model;
  const patient = pod.lower(`${BASE}Patient`, {
    id: person.id,
    identifier: [{ system: "https://example.org/reference-app/people", value: person.id }],
    name: [{ given: [person.given], family: person.family }],
    gender: person.gender,
    ...(person.birthDate ? { birthDate: person.birthDate } : {}),
  });
  const pulse = pod.lower(`${BASE}heartrate`, {
    id: reading.id, status: "final", subject: `Patient/${person.id}`,
    effectiveDateTime: reading.measuredAt,
    valueQuantity: { value: reading.bpm, ...(reading.comparator ? { comparator: reading.comparator } : {}) },
  });
  return { resources: [patient.resource, pulse.resource], validations: [patient.validation, pulse.validation] };
}

function assessment(pod, base, output, payload, payloadValidation) {
  const blockers = [];
  const warnings = [];
  const needs = [];
  const required = [];
  const uncarried = [];
  const carried = [];
  const notes = [];
  const information = [];
  const unsupported = [];
  const inspect = (label, validation) => {
    for (const issue of validation.issues) {
      const finding = { label, ...issue };
      if (["error", "fatal"].includes(issue.severity)) blockers.push({ kind: "validation", ...finding });
      else if (issue.code === "not-supported") {
        unsupported.push(finding);
        blockers.push({ kind: "unsupported", ...finding });
      } else if (issue.severity === "warning") warnings.push(finding);
      else information.push(finding);
    }
    if (!validation.valid && !validation.issues.some(i => ["error", "fatal"].includes(i.severity))) {
      blockers.push({ kind: "validation", label, message: "Validation did not pass." });
    }
  };
  base.validations.forEach((v, i) => inspect(`Base ${base.resources[i].resourceType}`, v));
  for (const r of output.resources) {
    const label = `${r.resourceType}/${r.id}`;
    inspect(label, r.validation);
    for (const path of [...r.missing, ...r.alsoMissing]) required.push({ label, path });
    for (const path of r.extras) uncarried.push({ label, path });
    for (const path of r.carried) carried.push({ label, path });
    for (const message of [r.note, ...r.notes].filter(Boolean)) notes.push({ label, message });
  }
  inspect("Download bundle", payloadValidation);
  if (output.document) {
    inspect("Patient summary", output.document.validation);
    for (const id of output.document.omitted) blockers.push({ kind: "omitted", label: id, message: "Left out of the patient summary." });
  }
  if (output.audit) inspect("Audit record", output.audit.validation);
  else blockers.push({ kind: "audit", label: "Audit record", message: "No audit record was built." });
  for (const [label, resource] of [["Download bundle", payload], ["Audit record", output.audit?.resource]]) {
    if (!resource) continue;
    for (const need of pod.terminologyNeeds(JSON.stringify(resource))) needs.push({ label, ...need });
  }
  for (const item of required) blockers.push({ kind: "missing", ...item, message: "Required input is missing." });
  for (const item of uncarried) blockers.push({ kind: "extras", ...item, message: "This value was not carried into the output." });
  // The example deliberately requires host review of every selection/identity
  // note, including informational ones. It never classifies prose as approval.
  for (const item of notes) blockers.push({ kind: "note", ...item });
  for (const item of needs) blockers.push({ kind: "terminology", ...item, message: "Terminology membership is unresolved." });
  return { ready: blockers.length === 0, blockers, acceptedLimitations: [], warnings, information, unsupported, required, uncarried, carried, notes, needs };
}

export function prepareExport(pod, model, region, { document = false, now = new Date().toISOString() } = {}) {
  const base = toBaseRecord(pod, model);
  const output = pod.export(region, {
    resources: base.resources, document, author: `Patient/${model.person.id}`, date: now,
    // This represents the local read used to prepare the preview. The example
    // neither sends nor stores the AuditEvent, and does not claim delivery.
    audit: {
      recorded: now,
      client: { who: "Device/reference-app", network: "127.0.0.1" },
      server: { who: "Device/local-export", network: "https://example.org/reference-app" },
      source: { observer: "Device/reference-app" },
    },
  });
  const payload = output.document?.document ?? {
    resourceType: "Bundle", type: "collection",
    entry: output.resources.map(r => ({
      fullUrl: `https://example.org/reference-app/fhir/${r.resourceType}/${r.id}`,
      resource: r.resource,
    })),
  };
  const payloadValidation = pod.validate(JSON.stringify(payload));
  return {
    model: structuredClone(model), base, output, payload, payloadValidation,
    decision: assessment(pod, base, output, payload, payloadValidation), preparedAt: now,
  };
}

export function liftReading(pod, review) {
  const resource = review.output.resources.find(r => r.resourceType === "Observation").resource;
  // Read the same base profile regardless of the export destination. Extras
  // and conflicts stay in the returned report; this is not a lossless import.
  return pod.lift(`${BASE}heartrate`, resource);
}

export function reviewPacket(review) {
  return {
    kind: "fhir-pods-reference-review", synthetic: true, preparedAt: review.preparedAt,
    hostDecision: review.decision, hostModel: review.model, base: review.base,
    export: review.output, downloadBundle: review.payload, downloadValidation: review.payloadValidation,
  };
}

// An explicit host decision for this synthetic, local preview only. Errors,
// missing/lost values, terminology needs and selection notes are not waivable.
// The original diagnostics remain in both the report and acceptedLimitations.
export function acknowledgeLimitations(review, accepted) {
  const d = review.decision;
  const blockers = d.blockers.filter(b => b.kind !== "unsupported");
  if (!accepted) blockers.push(...d.unsupported.map(finding => ({ kind: "unsupported", ...finding })));
  return {
    ...review,
    decision: { ...d, ready: blockers.length === 0, blockers, acceptedLimitations: accepted ? [...d.unsupported] : [] },
  };
}

export function approvedPayload(review) {
  if (!review?.decision.ready) throw new Error("Rebuild and resolve the review findings before downloading FHIR.");
  return review.payload;
}
