# fhir-pods-expo

The **expo pod**: the `fhir-pods` Rust verifier as an Expo native module, over
the lean C ABI. iOS first — Android (a JNI shim + `cargo-ndk`) is the next pod.

## Shape

```
JS  ──validate(json)──►  Swift (Expo Modules API)  ──C──►  Rust core (fhir-pods-ffi)
        ◄──report──                                 ◄────
```

A synchronous C call — no bridge round-trip. The native artifact is an
XCFramework vendored by the podspec; nothing here is source-linked into the host
app.

## Build the native artifact (on your Xcode rig)

```bash
# from the repo root
./scripts/build-ios-xcframework.sh
# → pods/expo/ios/Frameworks/FhirPodsFFI.xcframework  (arm64 device + arm64 simulator)
```

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

## Status

Wired end to end: Rust → C ABI → Swift → JS, with the real verifier behind it
(AU Core 2.0.0, 61 compiled profiles, FHIRPath invariants; 84/84 agreement with
the HL7 reference validator). The bridge is the same two calls the wasm pod
makes — the ABI never changes per target.

## Boundary

A standalone module, consumed as a published/vendored artifact. The pod never
imports the host app; the host never imports pod source.
