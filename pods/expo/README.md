# fhir-pods-expo

The **fhir-pods compliance runtime** as an Expo native module for iOS and
Android. Lift/lower FHIR, validate, de-identify, assemble patient summaries,
construct audit records, and convert HL7 v2 through the shared native ABI.

The host selects profiles and policies, maps its domain fields to slots,
and handles storage, transport, and decisions about the returned findings.
See the [runtime overview](../../README.md) for coverage and responsibilities.

## Shape

```
JS  ──operation inputs──►  Swift (Expo Modules API)   ──C───►  Rust runtime
JS  ──operation inputs──►  Kotlin (Expo Modules API)  ──JNI──►  Rust runtime
    ◄──output + findings──                              ◄──────
```

A synchronous native call either way — no bridge round-trip. On iOS Swift
calls the C header directly; on Android the JVM can't, so the same crate
carries a small JNI shim (Android-only, `ByteArray` in / `ByteArray` out) that
the Kotlin wrapper binds to. The native artifacts are vendored inside the
package — an XCFramework for iOS, `jniLibs/<abi>/libfhir_pods_ffi.so` for
Android; nothing here is source-linked into the host app.

## Build the native artifacts

```bash
# from the repo root
./scripts/build-ios-xcframework.sh      # Xcode: → ios/Frameworks/FhirPodsFFI.xcframework (arm64 device + sim)
./scripts/build-android-jnilibs.sh      # NDK:   → android/src/main/jniLibs/{arm64-v8a,x86_64}/libfhir_pods_ffi.so
```

The Android build needs only an NDK (`ANDROID_HOME`, newest NDK picked up
automatically; no `cargo-ndk`), links with 16 KB page alignment for
Android 15+. Artifact sizes are recorded with each release build.

## Use it in an Expo app

```bash
npm install fhir-pods-expo        # or a local/vendored path
npx pod-install
```

```ts
import { validate, operationOutcome } from "fhir-pods-expo";

const v = validate(JSON.stringify(observation));
// { resourceType: "Observation", profile: "AU Core Heart Rate", valid: true, issues: [] }

const outcome = operationOutcome(JSON.stringify(observation));
// { resourceType: "OperationOutcome", issue: [...] }   ← the $validate wire form
```

`validate` never throws for bad input: unparseable JSON is a `fatal` issue in
the report.

## Lowering — construct from profile inputs

```ts
import { lower, slots } from "fhir-pods-expo";
slots("http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate"); // the inputs it takes
const { resource, validation } = lower("…/au-core-heartrate", {
  status: "final", subject: "Patient/1", effectiveDateTime: "2026-09-16T08:00:00+10:00",
  "valueQuantity.value": 72, "valueQuantity.unit": "beats/minute",
});
```

The profile supplies the codings, category, units and slices; the host
supplies the variable data; the result comes back validated (through the
native terminology provider when one is installed).

## Must-support (information, not validation)

```ts
import { mustSupportGaps } from "fhir-pods-expo";
mustSupportGaps(JSON.stringify(patient));
// ["Patient.telecom.system", "Patient.birthDate", …]
```

## Lifting

```ts
import { lift } from "fhir-pods-expo";
const { inputs, validation, missing, extras } = lift("…/au-core-heartrate", observation);
// inputs: { status, subject, effectiveDateTime, "valueQuantity.value", "valueQuantity.unit" }
```

The reverse of lowering: the profile's slots read out of a resource — what a
host maps to its own model, and nothing the profile fixes. `missing` names
the required slots the resource lacks; `extras` names element paths no slot
covers, so nothing is left behind unnamed; `validation` is the verdict on the
way in. `lift("", resource)` takes the most specific compiled profile the
resource declares. `lower(url, inputs)` rebuilds the slot-covered content.
Inspect `extras` before a round trip: uncovered values are listed but are not
automatically preserved. Construction returns validation findings and can
produce an invalid resource when inputs do not meet the profile's requirements.

## HL7 v2

```ts
import { v2 } from "fhir-pods-expo";
const { bundle, validation, skipped, unplaced, unmapped } = v2(er7, { timezone: "+10:00" });
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

## Documents

```ts
import { document } from "fhir-pods-expo";
const { document: bundle, placed, omitted, validation } = document("au-ps", {
  resources: [patient, ...conditions, ...allergies, ...medications, ...observations, practitioner],
  author: "Practitioner/gp1",
  date: new Date().toISOString(),
});
```

A loaded terminology table, or the `terminology` callback, reaches the
document's validation too — the IPS pregnancy profiles, told apart only by
LOINC sets the pod does not ship, need one (`tools/compile_valueset.py` on
the IG's own `ValueSet`s writes the table).

`ips` or `au-ps`: sections from the kind's Composition profile, each resource
placed by what it is, narrative generated, references rewritten to the
entries, resources nothing reaches left out (`omitted`), required sections
with nothing in them marked `emptyReason: unavailable`, and the Bundle
validated on the way out (`validation`).

## Audit

```ts
import { audit } from "fhir-pods-expo";
const { resource, validation } = audit("patient-read", {
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
`AuditEvent` definition (`lower("http://hl7.org/fhir/StructureDefinition/AuditEvent", …)`),
as `Provenance` is.

## Discovery — what is compiled in, and what a slot wants

```ts
import { profiles, slots } from "fhir-pods-expo";

const { regions, profiles: compiled } = profiles();
// regions  → [{ name: "au", title, guide, document: "au-ps" }, …]
// compiled → [{ url, name, resourceType, kind: "resource" | "datatype" | "extension", base }, …]
const bp = slots(compiled.find((p) => p.name === "AU Core Blood Pressure")!.url);
// bp[i]    → { path, type, choices, min, max, mustSupport, fixed: [{ path, value }], parts: [{ name, type, repeats, required }] }
```

A Quantity's `unit` slot is optional where the profile fixes the UCUM
`code`: lowering writes the code as the unit when none is given.

## Export — a record spoken for a region

```ts
import { exportRecord } from "fhir-pods-expo";

const out = exportRecord("au", {
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
refuses a region it does not know, and `profiles()` lists the ones it does). The function is
`exportRecord` because `export` is a reserved word. Offline, an IPS
document with any Observation needs the IG's two pregnancy LOINC sets
loaded (`loadTerminology`) to tell its pregnancy entries apart — the same
limit the reference validator has without a terminology server.

## De-identification

```ts
import { deidentify } from "fhir-pods-expo";
const { resource, applied, validation, remaining, residual } = deidentify(json, {
  salt: await SecureStore.getItemAsync("deid-salt"), // keys the pseudonyms — secret, stable
  today: "2026-09-16",
  rules: [{ target: "Observation.effectiveDateTime", action: "keep" }], // on top of Safe Harbor
});
```

Safe Harbor by default — names, contact details, addresses below state,
exact dates (year only), ages over 89, narrative, notes, attachments,
signatures and device serials go; identifiers and references become
HMAC-SHA256 pseudonyms under the salt. Where the declared profile requires
an element that goes, a `data-absent-reason: masked` stub stands in, so the
copy stays conformant wherever the profile permits (`validation` says where
it does not). `applied` is the audit trail; `remaining` is what a Safe
Harbor check still finds, empty under the default policy. Rules target a
type, a member of a type or an element; the most specific wins.

Free text goes under Safe Harbor — a concept's `text`, a note, an
organisation's name, an extension's string payload — except a coding's
display, a quantity's unit, an address's state and country, and an
element's own `id` / `linkId`. A host that keeps a free-text site by rule
(`{ target: "CodeableConcept.text", action: "keep" }`) gets every kept site
back in `residual`, so an empty `remaining` never reads as "all free text
gone". What the base resource or an applied profile requires is masked
(`data-absent-reason: masked`) rather than dropped, required children
included; an extension whose value went goes with it.

## Terminology

The pod bundles every licence-free value set the registry binds (the FHIR and
HL7 v2/v3 sets). What it cannot decide — SNOMED CT, LOINC answer lists, VSAC
and NCTS-hosted sets — is a warning, never an error, until the host answers.

**Compiled expansions (the on-device route).** Expand the sets you are
licensed for on a terminology server (NCTS in Australia, Ontoserver, VSAC…),
compile them with `tools/compile_valueset.py`, ship the text with the app and
load it once at startup — from JS, no native code:

```ts
import { loadTerminology, terminologyNeeds } from "fhir-pods-expo";
loadTerminology(await FileSystem.readAsStringAsync(tableUri)); // true; false = unparseable, old table kept
terminologyNeeds(json); // [{ valueSet, system, code }] — what is still unanswered for this resource
```

A set in the table is complete: a code not listed is not a member. Displays
in the table reach `lower()`. `terminologyNeeds` is the ask-then-answer loop:
resolve what it lists with `$validate-code` (or `$expand` the sets and load
them) and validate again.

**A native provider.** For a host that already holds the answers, install a
synchronous provider from native code — the runtime never calls back into
JS, so the provider is native by design (an on-device cache, NCTS…):

```swift
// iOS — e.g. in AppDelegate
FhirPodsExpoModule.terminology = { valueSet, system, code in
  myCache.contains(valueSet, system, code)   // true / false / nil (unknown)
}
```

```kotlin
// Android — e.g. in MainApplication.onCreate
FhirPodsExpoModule.terminology = FhirPodsTerminology { valueSet, system, code ->
  if (myCache.contains(valueSet, system, code)) 1 else 0   // anything else = unknown
}
```

The bundled sets are consulted first, then the loaded table; the host is only
asked what they can't answer. `system` is "" for a bare `code` element.

## Status

The current package is **0.12.1, ABI 12**. The release build produces iOS
XCFramework slices, Android libraries, and a packed Expo artifact. This
package exposes the full operation family documented above.

The latest simulator exercise used the **0.12.0 / ABI 12** artifact on
2026-09-21 in a fresh Expo 57 / React Native 0.86 app: **17 checks passed on
both an iPhone 17 simulator and a Pixel 8 Android 15 emulator with identical
outputs** — abi, validate, operationOutcome, mustSupportGaps, slots (fixed
neighbours and parts), lower (unit from the fixed code), lift, profiles,
export for `au` and `us` (document and audit event valid; a contradiction
and another patient's resource refused), document, audit, v2, deidentify,
terminologyNeeds. Mean per-call times in a debug build were ~0.1 ms for
validate and ~10 ms for a whole-record `au` export. The evidence (outputs,
the scratch app's source, toolchain) is kept with the project's planning
records, not here. 0.12.1 only corrects the podspec/gradle version string
that exercise found; the binaries are unchanged.

## Boundary

A standalone module, consumed as a published/vendored artifact. The pod never
imports the host app; the host never imports pod source.
