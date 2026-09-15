# fhir-pods-expo

The **expo pod**: the `fhir-pods` Rust verifier as an Expo native module, over
the lean C ABI. iOS-first skeleton — Android (a JNI shim + `cargo-ndk`) is the
next pod.

## Shape

```
JS  ──probe(json)──►  Swift (Expo Modules API)  ──C──►  Rust core (fhir-pods-ffi)
        ◄──report──                              ◄────
```

A synchronous C call — no bridge round-trip. The native artifact is an
XCFramework vendored by the podspec; nothing here is source-linked into the host
app.

## Build the native artifact (on your Xcode rig)

```bash
# from the repo root
./scripts/build-ios-xcframework.sh
# → pods/expo/ios/Frameworks/FhirPodsFFI.xcframework
```

## Use it in an Expo app

```bash
npm install fhir-pods-expo        # or a local/vendored path
npx pod-install
```

```ts
import { probe, abiVersion } from "fhir-pods-expo";

const report = probe(JSON.stringify({ resourceType: "Observation" }));
// { resourceType: "Observation", pod: "fhir-pods", stage: "skeleton", issues: [] }
```

## Status

End-to-end skeleton: proves the Rust → C ABI → Swift → JS pipeline with a
placeholder `probe`. The real verifier grows behind the same ABI — the bridge
never changes again.

## Boundary

A standalone module, consumed as a published/vendored artifact. The pod never
imports the host app; the host never imports pod source.
