# fhir-pods-wasm

The **wasm pod**: the `fhir-pods` verifier as a WebAssembly module plus a
zero-dependency JS wrapper. Browser, edge runtimes, Node, Bun, Deno — anywhere
`WebAssembly` is.

```
JS  ──validate(json)──►  hand-written wrapper  ──C ABI──►  Rust core (wasm32)
        ◄──report──                             ◄────────
```

No wasm-bindgen, no glue runtime: the wrapper is ~40 lines over `pod_alloc` /
`pod_free` and the two entrypoints. The `.wasm` carries the 156 compiled
AU Core + US Core profiles (and the AU Base / US Core datatypes they inherit),
the bundled value sets and the FHIRPath engine — ~260 KB, 66 KB gzipped.

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

## Boundary

A standalone package. It never imports the host; the host never imports the
core's source.
