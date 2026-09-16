package expo.modules.fhirpods

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class FhirPodsExpoModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("FhirPodsExpo")

    Function("abiVersion") { FhirPodsNative.abiVersion() }

    // Synchronous: a FHIR resource (JSON string) in, a JSON document out. Each
    // is a straight JNI call into the Rust core — no bridge round-trip.
    Function("validate") { json: String ->
      String(FhirPodsNative.validate(json.toByteArray(Charsets.UTF_8)), Charsets.UTF_8)
    }

    Function("operationOutcome") { json: String ->
      String(FhirPodsNative.operationOutcome(json.toByteArray(Charsets.UTF_8)), Charsets.UTF_8)
    }
  }
}
