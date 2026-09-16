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
Android 15+, and produces a ~560 KB stripped `.so` per ABI.

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

Wired end to end and proven on the simulator: the pod, packed with `npm pack`
and installed into a fresh Expo SDK 57 app as a vendored tarball, autolinks
(`FhirPodsExpo` in `Podfile.lock`, deployment target raised to 16.4 by Expo),
builds, and answers from JS on both platforms — ABI 1, correct findings at the
right paths, garbage in → `fatal` out, and **~22,000 validations/s on an
iPhone 17 simulator (45 µs each) / ~19,600 on a Pixel 8 emulator (51 µs each),
including `JSON.parse` of the report**. The real verifier is
behind it (AU Core 2.0.0, 61 compiled profiles, FHIRPath invariants; 84/84
agreement with the HL7 reference validator). The bridge is the same two calls
the wasm pod makes — the ABI never changes per target.

## Boundary

A standalone module, consumed as a published/vendored artifact. The pod never
imports the host app; the host never imports pod source.
