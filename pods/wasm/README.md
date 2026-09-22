# fhir-pods-wasm

The **fhir-pods compliance runtime** as a WebAssembly module with a
dependency-free JS wrapper. Lift/lower FHIR, validate, de-identify, assemble
patient summaries, construct audit records, and convert HL7 v2 in a browser,
edge runtime, Node, Bun, or Deno.

```
JS  ──operation inputs──►  JS wrapper  ──C ABI──►  Rust compliance runtime
    ◄──output + findings──            ◄─────────         (wasm32)
```

The wrapper uses `pod_alloc` / `pod_free` and the shared ABI 12 entrypoints.
The module carries 992 compiled profiles from the pinned IGs and R4
extension/base definitions, 126 bundled value sets, base code-system
membership tables, FHIRPath, transformation and assembly engines, and HL7 v2
maps. The v0.10.0 artifact is 2,387,366 bytes (586,075 bytes gzip-9).

The host selects profiles and policies, maps its domain fields to slots,
and handles storage, transport, and decisions about the returned findings.
See the [runtime overview](../../README.md) for coverage and responsibilities.

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
the report. Both calls are synchronous. Historical v0.8.3 measurements were
69–89 µs on an M4 Max for the recorded profiled-resource fixtures, including JSON copy
and parse; see the [benchmark guide](bench/README.md) for scope and provenance.

### Lowering — construct from profile inputs

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

### Lifting

```js
const { inputs, validation, missing, extras } = pod.lift("…/au-core-heartrate", observation);
// inputs: { status, subject, effectiveDateTime, "valueQuantity.value", "valueQuantity.unit" }
```

The reverse of lowering: the profile's slots read out of a resource — what a
host maps to its own model, and nothing the profile fixes. `missing` names
the required slots the resource lacks; `extras` names element paths no slot
covers, so nothing is left behind unnamed; `validation` is the verdict on the
way in. `pod.lift("", resource)` takes the most specific compiled profile the
resource declares. `pod.lower(url, inputs)` rebuilds the slot-covered content.
Inspect `extras` before a round trip: uncovered values are listed but are not
automatically preserved. Construction returns validation findings and can
produce an invalid resource when inputs do not meet the profile's requirements.

### HL7 v2

```js
const { bundle, validation, skipped, unplaced, unmapped } = pod.v2(er7, { timezone: "+10:00" });
// bundle: a message Bundle — MessageHeader, Patient, Encounter, DiagnosticReport, Observations…
```

An ER7 message (`ORU^R01`, `ADT^A01`, `VXU^V04`, `OML^O21`, …; MLLP framing
is the transport's) converted through the published `hl7.fhir.uv.v2mappings`
maps compiled into the pod, and validated on the way out. `skipped` names
every map row the pod could not execute for data the message carried (a
narrative-only condition, a prose formula, a target form the engine does not
take) with where and why; `unplaced` names the segments the structure has no
place for (`ZPI[1]`); `unmapped` names the valued fields of placed segments
that no map row mentions (`PID[1]-2`) — what the IG says nothing about, so
nothing was done. What FHIR requires and the message does not carry is
in `validation`, never invented. `timezone` is the zone of a time the
message states without one (else such a time is truncated to its date and
reported); `codeSystems` maps coding-system names (`L`, `99LAB`) to uris —
the host's names first, then the standard ones the pod knows. The same message gives the same
bundle. Throws only when it cannot start.

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

### Discovery — what is compiled in, and what a slot wants

```js
const { regions, profiles } = pod.profiles();
// regions   → [{ name: "au", title, guide, document: "au-ps" }, …]
// profiles  → [{ url, name, resourceType, kind: "resource" | "datatype" | "extension", base }, …]  (992)
const slots = pod.slots(profiles.find((p) => p.name === "AU Core Blood Pressure").url);
// slots[i]  → { path, type, choices, min, max, mustSupport,
//               fixed: [{ path: "component:SystolicBP.valueQuantity.code", value: "mm[Hg]" }, …],   ← what the profile fixes beside it
//               parts: [{ name: "family", type: "string", repeats: false, required: false }, …] }  ← a composite slot's members
```

A Quantity's `unit` slot is optional where the profile fixes the UCUM
`code`: lowering writes the code as the unit when none is given (`mm[Hg]`),
and keeps a host's own display unit when one is.

### Export — a record spoken for a region

```js
const out = pod.export("au", {
  resources: [patient, bloodPressure, condition, allergy, medication, immunization],  // as lowered against the base definitions, or any
  author: "Patient/u1", date: "2026-09-21T10:00:00+10:00", title: "My health record",
  audit: { recorded, client, server, user, source, transaction },  // optional: the BALP patient-read of this export
});
// out.region        → { name: "au", title: "Australia", guide: "AU Core 2.0.0 resources, AU Patient Summary 1.0.0 document" }
// out.resources[i]  → { index, resourceType, id, profile: {url, name} | null, also, resource, validation, missing, alsoMissing, extras, carried, note, notes }
// out.document      → { kind: "au-ps", document, placed, omitted, validation } | null   (`document: false` to skip)
// out.audit         → { resource, validation } | null
```

Write the transformer once against the base R4 definitions (`lower` into
`…/StructureDefinition/bp`, `…/Patient`, `…/Condition`, …); `export` picks
each resource's profile in the region's guide — a declared `meta.profile` of
the region's wins, a fixed code or category identifies among sibling
profiles, a type with one profile lineage takes it — and re-profiles it
through that profile's slots: the jurisdiction's codings, categories and
slices come from the profile, the data from the record, the `id` stays.
What the region requires and the record lacks is `missing`, never
invented; what no slot takes is carried as given where the profile's shape
allows (`carried`) and verified, or named in `extras` where it cannot be; a
record that contradicts what the profile fixes is not re-profiled and says
so in `note`; a resource nothing in the region profiles goes out as given
with the reason in `note`; `notes` say what else a host should know (a
declared profile not applied). A record is one patient's: a second Patient
or a resource about someone else is an error. Where the region's document asks its entries for a
second profile (the IPS's Condition beside US Core's) it is declared
alongside and verified (`also`, `alsoMissing`). Regions: `au` (AU Core + AU
Patient Summary), `us` (US Core + IPS document), `ips` (the international
floor — what to use for a jurisdiction without a compiled guide; the pod
refuses a region it does not know, and `profiles()` lists the ones it does). Offline, an IPS
document with any Observation needs the IG's two pregnancy LOINC sets
loaded (`loadTerminology`) to tell its pregnancy entries apart — the same
limit the reference validator has without a terminology server.

### De-identification

```js
const { resource, applied, validation, remaining, residual } = pod.deidentify(JSON.stringify(patient), {
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
