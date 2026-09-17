# fhir-pods-wasm

The **wasm pod**: the `fhir-pods` verifier as a WebAssembly module plus a
zero-dependency JS wrapper. Browser, edge runtimes, Node, Bun, Deno — anywhere
`WebAssembly` is.

```
JS  ──validate(json)──►  hand-written wrapper  ──C ABI──►  Rust core (wasm32)
        ◄──report──                             ◄────────
```

No wasm-bindgen, no glue runtime: the wrapper is ~70 lines over `pod_alloc` /
`pod_free` and the entrypoints. The `.wasm` carries the 259 compiled
profiles of the six pinned IGs (AU Core, US Core, IPA, IPS, AU Patient
Summary, IHE BALP — and the datatypes they inherit), the 72 bundled value
sets and the FHIRPath engine — ~1.1 MB, 277 KB gzipped at v0.6.2.

## Build

```bash
./scripts/build-wasm.sh        # → pods/wasm/dist/fhir_pods.wasm
cd pods/wasm && npm test       # drives the artifact through the wrapper
```

`wasm-opt` (binaryen) is used when on `PATH` and shrinks the module further.

## Use

```js
import { instantiate } from "fhir-pods-wasm";

// The host loads the bytes — fetch in a browser, readFile in Node:
const pod = await instantiate(await (await fetch("/fhir_pods.wasm")).arrayBuffer());

const v = pod.validate(JSON.stringify(observation));
// { resourceType: "Observation", profile: "AU Core Heart Rate", valid: true, issues: [] }

const outcome = pod.operationOutcome(JSON.stringify(observation));
// { resourceType: "OperationOutcome", issue: [...] }   ← the $validate wire form
```

`validate` never throws for bad input: unparseable JSON is a `fatal` issue in
the report. Both calls are synchronous — tens of microseconds for a profiled
resource.

### Lowering — a conformant resource from the variable data only

```js
pod.slots("http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate");
// [{ path: "status", type: "code", min: 1, … }, { path: "valueQuantity.value", type: "decimal", … }, …]

const { resource, validation } = pod.lower("…/au-core-heartrate", {
  status: "final", subject: "Patient/1", effectiveDateTime: "2026-09-16T08:00:00+10:00",
  "valueQuantity.value": 72, "valueQuantity.unit": "beats/minute",
});
// resource: the full Observation — LOINC + SNOMED codings, category, UCUM
// system and unit code all filled in by the profile; validation: its verdict.
```

Slot keys are labelled paths (`component:SystolicBP.valueQuantity.value` for
a blood pressure); a string becomes a `Reference`, a number a `Quantity`,
`{system, code}` a `CodeableConcept`, and a datatype object is taken as
given. Unknown keys, wrong kinds and collisions with a fixed value are issues
in `validation`, never dropped.

### Must-support (information, not validation)

```js
pod.mustSupportGaps(JSON.stringify(patient));
// ["Patient.telecom.system", "Patient.birthDate", …] — must-support elements
// the selected profile expects a client to populate when it has the data
```

### Documents

```js
const { document, placed, omitted, validation } = pod.document("au-ps", {
  resources: [patient, condition, allergy, medicationStatement, heartRate, practitioner],
  author: "Practitioner/gp1",
  date: "2026-09-16T10:00:00+10:00",
  title: "Patient Summary",
});
```

A loaded terminology table, or the `terminology` callback, reaches the
document's validation too — the IPS pregnancy profiles, told apart only by
LOINC sets the pod does not ship, need one (`tools/compile_valueset.py` on
the IG's own `ValueSet`s writes the table).

`ips` (IPS 2.0.1) or `au-ps` (AU Patient Summary 1.0.0). The sections come
from the kind's Composition profile; each resource is placed by what it is
(a Condition in Problems, or History of Past Illness when inactive; a
vital-signs Observation in Vital Signs; a laboratory one in Results…), the
narrative is a plain table per section built from what the resource
carries, references are rewritten to the entries they name (`Type/id`,
your own `fullUrl` when a resource arrives as a Bundle entry, `urn:uuid:id`),
resources nothing reaches are left out and listed in `omitted`, a required
section with nothing in it says `emptyReason: unavailable` — no clinical
fact is invented — and the Bundle is validated against the kind's profile:
`validation` is the verdict. The same inputs make the same document.

### Audit

```js
const { resource, validation } = pod.audit("patient-read", {
  recorded: new Date().toISOString(),
  client: { who: "Device/app-7", network: "10.0.0.7" },
  server: { who: { display: "fhir.example.org" }, network: "https://fhir.example.org/r4" },
  user: { who: "Practitioner/gp1", name: "Dr Who" },
  source: { observer: "Device/audit-collector", site: "clinic-1" },
  patient: "Patient/p1",
  data: "Observation/hr1",
  transaction: requestId, // X-Request-Id: the correlation with the trace
});
```

`read`, `query`, `create`, `update`, `delete`, each with or without a known
patient (`patient-read`…): an `AuditEvent` conformant to the IHE Basic Audit
Log Patterns profile for that kind. The profile supplies the event type,
subtype, action, outcome and the agents' types; a network address is
classified for its type (or given as `{address, type}`); a `who` is a
reference (`Type/id`) or a display name; a query (`query: "GET …"`) is
base64-encoded. The user's participation on a create / update / delete is
`author` and the data entity's role `Domain Resource` unless given
(`user.type`, `data: {what, role}`). A required input left out, or one out
of place, throws before anything is built; `validation` is the verdict. A
failed action is not a BALP basic event — lower it from the base
`AuditEvent` definition (`pod.lower("http://hl7.org/fhir/StructureDefinition/AuditEvent", …)`),
as `Provenance` is.

### De-identification

```js
const { resource, applied, validation, remaining } = pod.deidentify(JSON.stringify(patient), {
  salt: process.env.DEID_SALT,   // keys the pseudonyms — secret, stable
  today: "2026-09-16",           // what "over 89" is measured against
  rules: [{ target: "Observation.effectiveDateTime", action: "keep" }], // on top of Safe Harbor
});
```

Safe Harbor by default: names, contact details, addresses below state,
exact dates (year only), ages over 89, narrative, notes, attachments,
signatures and device serials go; identifiers and references become
HMAC-SHA256 pseudonyms under the salt, the same everywhere. Where the
declared profile requires an element that goes, a `data-absent-reason:
masked` stub stands in, so the copy stays conformant wherever the profile
permits — `validation` says where it does not (a vital sign's date must be
precise to the day, for one). `applied` is the audit trail; `remaining` is
what a Safe Harbor check still finds, empty under the default policy. Rules
target a type (`HumanName`), a member of a type (`Address.city`) or an
element (`Patient.address.city`); the most specific wins; `base: "none"`
starts from nothing. Free text outside narrative and notes is the host's
call — name it in the rules.

Free text goes under Safe Harbor — a concept's `text`, a note, an
organisation's name, an extension's string payload — except a coding's
display, a quantity's unit, an address's state and country, and an
element's own `id` / `linkId`. A host that keeps a free-text site by rule
(`{ target: "CodeableConcept.text", action: "keep" }`) gets every kept site
back in `residual`, so an empty `remaining` never reads as "all free text
gone". What the base resource or an applied profile requires is masked
(`data-absent-reason: masked`) rather than dropped, required children
included; an extension whose value went goes with it.

### Terminology

The `.wasm` bundles every licence-free value set the registry binds (the
FHIR and HL7 v2/v3 sets, generated from the packages). What it cannot decide
— SNOMED CT, LOINC answer lists, VSAC and NCTS-hosted sets — is a warning,
never an error, until the host answers. Three ways to answer:

**1. Compiled expansions (offline, the on-device route).** Expand the sets
you are licensed for on a terminology server, compile them, load the table
once; it answers for every call after that.

```sh
# NCTS (Australia; free, needs a client credential) — or any FHIR terminology server
curl -s -H "Authorization: Bearer $NCTS_TOKEN" \
  'https://api.healthterminologies.gov.au/integration/R4/fhir/ValueSet/$expand?url=https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1' > dai.json
# Ontoserver's public R4 endpoint works the same way for licence-free sets
curl -s 'https://r4.ontoserver.csiro.au/fhir/ValueSet/$expand?url=http://hl7.org/fhir/ValueSet/mimetypes' > mime.json
python3 tools/compile_valueset.py dai.json mime.json > terminology.txt
```

```js
pod.loadTerminology(await readFile("terminology.txt", "utf8")); // true; false = unparseable, old table kept
```

A set in the table is complete: a code not listed is not a member. Displays
in the table reach `lower()` — a `{system, code}` input comes out as a full
coding. Keep the licence of what you compile: a SNOMED expansion belongs in
your build, never in this package.

**2. Ask, then answer.** `terminologyNeeds(json)` lists exactly what the
bundled sets and the loaded table could not decide; resolve those with
`$validate-code` (or `$expand` the sets and load them) and validate again.

```js
for (const { valueSet, system, code } of pod.terminologyNeeds(json)) {
  // every parameter encoded (a code may carry `+`, `&` or `#`), the answer
  // cached only when the server actually answered
  const q = new URLSearchParams({ url: valueSet, system, code });
  const r = await fetch(`${tx}/ValueSet/$validate-code?${q}`);
  if (!r.ok) continue;
  const result = (await r.json()).parameter?.find((p) => p.name === "result");
  if (typeof result?.valueBoolean === "boolean") cache.set(`${valueSet}|${system}|${code}`, result.valueBoolean);
}
```

**3. A synchronous provider.** For a host that already holds the answers
(a cache like the one above):

```js
const pod = await instantiate(bytes, {
  // asked, synchronously, only about bindings the bundled sets and the loaded table can't decide
  terminology: (valueSet, system, code) => cache.get(`${valueSet}|${system}|${code}`), // true / false / undefined
});
```

`true` = member, `false` = not a member (an error on a required binding),
`undefined` or a throw = unknown (a warning, never an error). SNOMED content
never ships in the `.wasm`; it only ever arrives through this seam.

## Boundary

A standalone package. It never imports the host; the host never imports the
core's source.
