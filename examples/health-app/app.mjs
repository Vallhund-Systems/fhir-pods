import { instantiate } from "../../pods/wasm/src/index.js";
import { sample, prepareExport, liftReading, reviewPacket, approvedPayload, acknowledgeLimitations } from "./integration.mjs";

const $ = id => document.getElementById(id);
let pod, review = null, region = "au", regions = [], currentView = "host";
let tableRevision = 0;
let tableLoading = false;
const form = $("check-in");
const node = (tag, text, className) => {
  const element = document.createElement(tag);
  if (text !== undefined) element.textContent = text;
  if (className) element.className = className;
  return element;
};

function fillSample(preset = "complete") {
  $("given").value = sample.person.given;
  $("family").value = sample.person.family;
  $("birth-date").value = preset === "missing" ? "" : sample.person.birthDate;
  $("gender").value = sample.person.gender;
  $("bpm").value = sample.reading.bpm;
  $("comparator").value = preset === "qualified" ? "<" : "";
  $("measured-at").value = sample.reading.measuredAt.replace(/Z$/, "");
  $("preset").value = preset;
}

function readModel() {
  const given = $("given").value.trim(), family = $("family").value.trim();
  if (!given || !family) throw new Error("Enter a given name and family name for the synthetic person.");
  return {
    person: { ...sample.person, given, family, birthDate: $("birth-date").value, gender: $("gender").value },
    reading: {
      ...sample.reading, bpm: $("bpm").valueAsNumber, comparator: $("comparator").value,
      measuredAt: new Date(`${$("measured-at").value}Z`).toISOString(),
    },
  };
}

function status(kind, title, message) {
  $("decision").className = `decision ${kind}`;
  $("decision").querySelector(".decision-icon").textContent = kind === "ready" ? "✓" : kind === "review" ? "!" : "↻";
  $("decision-title").textContent = title;
  $("decision-detail").textContent = message;
}

function markStale() {
  review = null;
  $("download-fhir").disabled = true;
  $("download-review").disabled = true;
  $("counts").hidden = true;
  $("resources").replaceChildren();
  $("findings").replaceChildren();
  $("audit-status").textContent = "";
  $("limitations-panel").hidden = true;
  $("acknowledge-limits").checked = false;
  $("json-output").textContent = "Rebuild the preview to inspect the current inputs.";
  status("waiting", "Inputs changed", "Build the export again to review this record.");
}

function findingGroup(title, items, render, { open = false, blocking = false } = {}) {
  if (!items.length) return;
  const detail = node("details", undefined, blocking ? "blockers" : "");
  detail.open = open;
  detail.append(node("summary", `${title} (${items.length})`));
  const list = node("ul");
  for (const item of items) list.append(node("li", render(item)));
  detail.append(list);
  $("findings").append(detail);
}

function inspectView() {
  for (const button of $("inspector-tabs").querySelectorAll("button")) button.setAttribute("aria-pressed", String(button.dataset.view === currentView));
  if (!review) return;
  let data;
  const descriptions = {
    host: "The app model has person and reading fields. It contains no regional profile URLs.",
    base: "The adapter lowers the app model into base R4 once. It has no destination-specific branches.",
    export: "The runtime chooses regional profiles and returns validation, missing data, carried values, extras and notes.",
    audit: "The read used to prepare this local preview. This event has not been sent or saved to an audit store.",
    lift: "Read the exported observation through the base heart-rate slots. Extras and conflicts remain visible; these inputs alone are not a lossless import.",
  };
  if (currentView === "host") data = review.model;
  if (currentView === "base") data = review.base;
  if (currentView === "export") data = review.output;
  if (currentView === "audit") data = review.output.audit;
  if (currentView === "lift") {
    try { data = liftReading(pod, review); }
    catch (error) { data = { error: error.message }; }
  }
  $("inspector-help").textContent = descriptions[currentView];
  $("json-output").textContent = JSON.stringify(data, null, 2);
}

function render() {
  const { decision: d, output } = review;
  const documentOn = Boolean(output.document);
  status(d.ready ? "ready" : "review", d.ready ? (d.acceptedLimitations.length ? "Local download with limitations" : "Ready for local download") : "Review before use",
    d.ready ? (d.acceptedLimitations.length ? `${d.acceptedLimitations.length} unsupported findings accepted for this local preview. The runtime’s findings remain in the review.` : `The ${documentOn ? "patient summary" : "resource bundle"} passes this example’s host policy. ${d.warnings.length} validation warnings remain visible below.`)
      : `${d.blockers.length} finding${d.blockers.length === 1 ? " needs" : "s need"} attention. The full review is available; FHIR download is paused.`);
  $("counts").hidden = false;
  $("counts").replaceChildren();
  for (const [label, count] of [["Required inputs", d.required.length], ["Uncarried values", d.uncarried.length], ["Terminology needs", d.needs.length]]) {
    const metric = node("div"); metric.append(node("strong", count), node("span", label)); $("counts").append(metric);
  }
  $("resources").replaceChildren();
  for (const r of output.resources) {
    const card = node("div", undefined, "resource"), body = node("div");
    body.append(node("strong", r.resourceType), node("small", r.profile?.name ?? "Exported as given"));
    if (r.also) body.append(node("small", `Also: ${r.also.name}`));
    const limited = r.validation.issues.some(i => i.code === "not-supported");
    card.append(body, node("span", !r.validation.valid ? "Findings" : limited ? "Limited checks" : "Valid", `resource-status${r.validation.valid && !limited ? "" : " invalid"}`));
    $("resources").append(card);
  }
  $("findings").replaceChildren();
  findingGroup("Host review", d.blockers.filter(i => !["terminology", "unsupported"].includes(i.kind)), i => `${i.label}${i.path ? ` · ${i.path}` : ""}: ${i.message}`, { open: true, blocking: true });
  findingGroup("Unsupported checks", d.unsupported, i => `${i.label}${i.path ? ` · ${i.path}` : ""}: ${i.message}`, { open: true, blocking: !d.acceptedLimitations.length });
  findingGroup("Unresolved terminology", d.needs, i => `${i.valueSet} · ${i.system || "(bare code)"} | ${i.code}`, { blocking: true });
  findingGroup("Values carried through", d.carried, i => `${i.label} · ${i.path}`);
  findingGroup("Validation warnings", d.warnings, i => `${i.label}${i.path ? ` · ${i.path}` : ""}: ${i.message}`);
  findingGroup("Additional findings", d.information, i => `${i.label}${i.path ? ` · ${i.path}` : ""}: ${i.message}`);
  $("limitations-panel").hidden = !d.unsupported.length;
  $("acknowledge-limits").checked = d.acceptedLimitations.length > 0;
  const auditLimited = output.audit?.validation.issues.some(i => i.code === "not-supported");
  $("audit-status").textContent = `${output.audit?.validation.valid && !auditLimited ? "✓" : "!"} Local preview audit ${auditLimited ? "built with unsupported checks" : output.audit?.validation.valid ? "built and validated" : "needs review"}${documentOn ? ` · ${output.document.kind.toUpperCase()} document ${output.document.validation.valid ? "valid" : "has findings"}` : " · Resource bundle"}.`;
  $("download-fhir").disabled = !d.ready;
  $("download-review").disabled = false;
  inspectView();
}

function build() {
  markStale();
  if (tableLoading) { status("waiting", "Reading terminology table", "The preview will rebuild when the selected table has been read."); return; }
  if (!pod || !form.reportValidity()) return;
  try {
    review = prepareExport(pod, readModel(), region, { document: $("include-document").checked });
    render();
  } catch (error) {
    review = null;
    status("review", "Export could not be built", error.message);
  }
}

function chooseRegion(name) {
  region = name;
  for (const button of $("regions").querySelectorAll("button")) button.setAttribute("aria-pressed", String(button.dataset.region === region));
  $("guide").textContent = regions.find(r => r.name === region).guide;
  build();
}

function download(value, filename) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(value, null, 2) + "\n"], { type: "application/json" }));
  const link = node("a"); link.href = url; link.download = filename;
  document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

form.addEventListener("input", markStale);
form.addEventListener("submit", event => { event.preventDefault(); build(); });
$("preset").addEventListener("change", () => { fillSample($("preset").value); build(); });
$("reset").addEventListener("click", () => {
  tableRevision++; tableLoading = false; pod.loadTerminology(""); $("terminology-file").value = "";
  $("terminology-status").textContent = "Bundled terminology only.";
  fillSample(); $("include-document").checked = false;
  chooseRegion(regions.some(r => r.name === "au") ? "au" : regions[0].name);
});
$("include-document").addEventListener("change", build);
$("acknowledge-limits").addEventListener("change", () => {
  if (review) { review = acknowledgeLimitations(review, $("acknowledge-limits").checked); render(); }
});
$("inspector-tabs").addEventListener("click", event => {
  const button = event.target.closest("button[data-view]");
  if (button) { currentView = button.dataset.view; inspectView(); }
});
$("download-fhir").addEventListener("click", () => {
  try { download(approvedPayload(review), `check-in-${region}.fhir.json`); }
  catch (error) { status("review", "Download paused", error.message); }
});
$("download-review").addEventListener("click", () => { if (review) download(reviewPacket(review), `check-in-${region}.review.json`); });
$("terminology-file").addEventListener("change", async () => {
  const file = $("terminology-file").files[0];
  if (!file) return;
  const revision = ++tableRevision;
  tableLoading = true;
  markStale();
  $("terminology-status").textContent = "Reading terminology table…";
  try {
    const text = await file.text();
    if (revision !== tableRevision) return;
    if (!text.trim()) throw new Error("Empty file; use Clear table to remove the loaded table.");
    if (!pod.loadTerminology(text)) throw new Error("The table format was rejected. The previous table is still active.");
    $("terminology-status").textContent = `Loaded ${file.name}. Memberships have been rechecked.`;
  } catch (error) { if (revision === tableRevision) $("terminology-status").textContent = error.message; }
  if (revision === tableRevision) { tableLoading = false; build(); }
});
$("clear-terminology").addEventListener("click", () => {
  tableRevision++; tableLoading = false; pod.loadTerminology(""); $("terminology-file").value = "";
  $("terminology-status").textContent = "Bundled terminology only."; build();
});

fillSample();
try {
  const response = await fetch(new URL("../../pods/wasm/dist/fhir_pods.wasm", import.meta.url));
  if (!response.ok) throw new Error("WASM artifact is missing. Run bash scripts/build-wasm.sh from the repository root, then reload.");
  pod = await instantiate(await response.arrayBuffer());
  regions = pod.profiles().regions;
  if (!regions.length) throw new Error("This runtime has no compiled export destinations.");
  for (const item of regions) {
    const button = node("button"); button.type = "button"; button.dataset.region = item.name;
    button.append(node("strong", item.name.toUpperCase()), node("span", item.title));
    button.addEventListener("click", () => chooseRegion(item.name)); $("regions").append(button);
  }
  for (const id of ["app-fields", "include-document", "terminology-file", "clear-terminology"]) $(id).disabled = false;
  $("region-count").textContent = `${regions.length} destinations`;
  $("runtime-status").textContent = "Runtime ready. Change the record or destination to see the boundary decisions.";
  $("abi-label").textContent = `WASM · ABI ${pod.abiVersion}`;
  chooseRegion(regions.some(r => r.name === "au") ? "au" : regions[0].name);
  document.documentElement.dataset.runtime = "ready";
} catch (error) {
  $("runtime-status").textContent = error.message;
  status("review", "Runtime unavailable", "Build the WASM artifact and serve this example over localhost. See the integration notes below.");
  document.documentElement.dataset.runtime = "error";
}
