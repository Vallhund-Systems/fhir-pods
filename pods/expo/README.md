# fhir-pods-expo

The **expo pod**: the `fhir-pods` Rust verifier as an Expo native module, over
the lean C ABI. iOS and Android.

## Shape

```
JS  ──validate(json)──►  Swift (Expo Modules API)   ──C───►  Rust core (fhir-pods-ffi)
JS  ──validate(json)──►  Kotlin (Expo Modules API)  ──JNI──►  Rust core (fhir-pods-ffi)
        ◄──report──                                  ◄──────
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
Android 15+, and produces a ~1 MB stripped `.so` per ABI.

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

## Lowering — a conformant resource from the variable data only

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

## Documents

```ts
import { document } from "fhir-pods-expo";
const { document: bundle, placed, omitted, validation } = document("au-ps", {
  resources: [patient, ...conditions, ...allergies, ...medications, ...observations, practitioner],
  author: "Practitioner/gp1",
  date: new Date().toISOString(),
});
```

`ips` or `au-ps`: sections from the kind's Composition profile, each resource
placed by what it is, narrative generated, references rewritten to the
entries, resources nothing reaches left out (`omitted`), required sections
with nothing in them marked `emptyReason: unavailable`, and the Bundle
validated on the way out (`validation`).

## De-identification

```ts
import { deidentify } from "fhir-pods-expo";
const { resource, applied, validation, remaining } = deidentify(json, {
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
synchronous provider from native code — the verifier never calls back into
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

Wired end to end and proven on the simulator: the pod, packed with `npm pack`
and installed into a fresh Expo SDK 57 app as a vendored tarball, autolinks
(`FhirPodsExpo` in `Podfile.lock`, deployment target raised to 16.4 by Expo),
builds, and answers from JS on both platforms — ABI 3, correct findings at the
right paths, garbage in → `fatal` out, and **~22,000 validations/s on an
iPhone 17 simulator (45 µs each) / ~19,600 on a Pixel 8 emulator (51 µs each),
including `JSON.parse` of the report**. The real verifier is
behind it (AU Core 2.0.0, US Core 6.1.0, IPA 1.1.0 — 168 compiled profiles,
FHIRPath invariants; 289/289 agreement with the HL7 reference validator). The bridge is the same calls the
wasm pod makes — the ABI never changes per target.

## Boundary

A standalone module, consumed as a published/vendored artifact. The pod never
imports the host app; the host never imports pod source.
