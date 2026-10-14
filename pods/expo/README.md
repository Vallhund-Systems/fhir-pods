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
Every issue carries a stable rule `id` (`binding.no-code`, `inv.cmp-1`,
`questionnaire.required-answer`, …) — assert on that, never on message text;
every id, its severity and the condition that produces it are in
[docs/rules.md](../../docs/rules.md).

`valid` is **no `error` and no `fatal`**. `fatal` means something could not be
checked at all and says which — only two findings are fatal: the input is not
JSON (`parse.json`, and then that finding is the whole report) and the
Questionnaire a response is held to has an item with no `type`
(`questionnaire.definition`). A `warning` or `information` finding never
changes the verdict, however much it matters: a resource whose declared
profile this build has not compiled is valid by default, with
`profile.unknown` naming it — see [Host options](#host-options) to make that
an error instead of filtering the issue out.

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
Patient Summary), `us` (US Core + IPS document), `uk` (UK Core STU2 + IPS
document), `ca` (CA Baseline + IPS document), `nz` (NZ Base + IPS
document), `ips` (the international floor — what to use for a jurisdiction
without a compiled guide; the pod refuses a region it does not know, and
`profiles()` lists the ones it does). The function is
`exportRecord` because `export` is a reserved word. Offline, an IPS
document with any Observation needs the IG's two pregnancy LOINC sets
loaded (`loadTerminology`) to tell its pregnancy entries apart — the same
limit the reference validator has without a terminology server.

## De-identification

```ts
import { deidentify } from "fhir-pods-expo";
const { resource, applied, validation, remaining, residual } = deidentify(json, {
  salt: await SecureStore.getItemAsync("deid-salt"), // REQUIRED, non-empty — secret, stable
  today: "2026-09-16",
  rules: [
    { target: "Observation.effectiveDateTime", action: "keep" },        // on top of the built-in policy
    { target: "Patient.extension.valueInteger", action: "collapse-age" }, // this element is an age
  ],
});
```

**What this does, exactly.** It applies a **policy object** to a typed walk
of the resource and returns the copy with a trail. The built-in policy
(`base: "safe-harbor"`, the default; `"none"` starts from nothing) is a rule
set *modelled on* HIPAA Safe Harbor's identifier list, by FHIR type: names,
contact details, addresses below state, exact dates (year only), ages over
89 on an `Age`, narrative, notes, attachments, signatures and device serials
go; identifiers and references become HMAC-SHA256 pseudonyms under the salt.
**It is not a compliance determination** and the pod claims none: the policy
covers the fields its rules name, in the resource it is given, and nothing
about re-identification risk from what remains, the host's other data, or
how the copy is used. Whoever releases the data owns that determination.

`salt` is required and must be non-empty — the call throws without one
rather than emitting unkeyed pseudonyms. Where the declared profile requires
an element that goes, a `data-absent-reason: masked` stub stands in, so the
copy stays conformant wherever the profile permits (`validation` says where
it does not). `applied` is the audit trail; `remaining` is what the built-in
policy's own check still finds, empty under the default policy. Rules target
a type, a member of a type or an element; the most specific wins.

`collapse-age` is the one rule the host must name an element for: an age over
89 aggregates, but nothing says which *integer* is an age, so the pod never
guesses. Where a rule names one, over 89 becomes `90`; a quantity-typed
element says it exactly — `{"value": 90, "comparator": ">="}` in its own
unit. An element no age can be read from is left alone and named in
`residual`.

Free text goes under the built-in policy — a concept's `text`, a note, an
organisation's name, an extension's string payload — except a coding's
display, a quantity's unit, an address's state and country, and an
element's own `id` / `linkId`. A host that keeps a free-text site by rule
(`{ target: "CodeableConcept.text", action: "keep" }`) gets every kept site
back in `residual`, so an empty `remaining` never reads as "all free text
gone". What the base resource or an applied profile requires is masked
(`data-absent-reason: masked`) rather than dropped, required children
included; an extension whose value went goes with it.

## Questionnaires

A `QuestionnaireResponse` can resolve a contained Questionnaire or one in the
same Bundle without host state. For a separately stored form, load its JSON
before validating, lowering or lifting the response:

```ts
import { loadQuestionnaire, validate } from "fhir-pods-expo";

loadQuestionnaire(JSON.stringify(questionnaire)); // true
const report = validate(JSON.stringify(response));
loadQuestionnaire(""); // clear the loaded-form store
```

The response names the form by `Questionnaire.url`, optionally followed by
`|version`. A non-Questionnaire returns `false` and leaves the existing store
intact. Clearing returns `true`. The store belongs to the native runtime in
the app process, so the host should load the forms needed for that process and
clear them when its data boundary changes. An unresolved form is reported as
such; its item rules are not guessed.

## Host options

Two strictness calls are the host's, and both default to what the HL7
reference validator itself does:

```ts
import { options } from "fhir-pods-expo";

options({ unknownProfile: "error", externalNarrativeLinks: "refuse" }); // true
options({}); // back to the defaults ("" does the same)
```

- **`unknownProfile`** — a resource declaring a profile this build has not
  compiled is *valid* with a `profile.unknown` warning naming it (the
  reference validator's own severity). `"error"` makes it invalid: the same
  finding, at error — a host that will not accept an unverified claim fails
  closed rather than remembering to check for the warning.
- **`externalNarrativeLinks`** — a narrative reference out of the resource is
  accepted, as the reference validator accepts it (an image's `src` is not
  held to the hyperlink scheme rules at all there). `"refuse"` makes any
  `<img src>` or `<a href>` that is neither a `#fragment` resolved inside the
  resource nor a self-contained `data:` URL an error
  (`narrative.external-link`).

The options hold for every call that follows in this process — validate,
lower, lift, export, document, v2 — the way a loaded Questionnaire does. A
document that does not say what it means (an unknown member, a value no
option takes, or a member given twice) returns `false` and leaves the options
already in force: a typo in a strictness setting must never read as "off". Full semantics:
[docs/scope.md](../../docs/scope.md#host-options).

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

The last packed package is **0.21.0, ABI 13**; the source now speaks **ABI
14** (`fhir_pods_options`), so the next build's artifacts carry that. The
package refuses a mismatch on import — unlike the wasm pod, which is handed
its module, this one *contains* its native library, so a mismatch means
mismatched artifacts were packed or vendored; the native modules log it on
creation and `src/index.ts` throws. The
release build produces iOS XCFramework slices, Android libraries, and a
packed Expo artifact. This package exposes the full operation family
documented above.

The latest simulator exercise rebuilt and packed the **0.19.2 / ABI 13**
module from source `98757de` on 2026-10-06, then installed that tarball in a
fresh Expo 57.0.27 / React Native 0.86.3 app. **24 checks passed on both an
iPhone 17 / iOS 26.5 simulator and the `pixel8-dust` Android 15 emulator with
identical functional results.** The suite covers the operation family above,
all six discovered regions, lift conflicts, and the host-loaded Questionnaire
lifecycle across validate, lower and lift: unresolved, load, wrong type,
missing required answer, valid answer, rejected replacement with preservation,
and clear. The packed artifact SHA-256 is
`4d8a040e065fff0f131b93a8ecfd3a7d7b8d471208c8bbbbcc8c6bc043242085`.
The scratch app, full outputs, toolchain and observational debug timings are
kept with the project's planning records.

## Boundary

A standalone module, consumed as a published/vendored artifact. The pod never
imports the host app; the host never imports pod source.
