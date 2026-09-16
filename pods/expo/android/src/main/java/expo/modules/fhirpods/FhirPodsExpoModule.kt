package expo.modules.fhirpods

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class FhirPodsExpoModule : Module() {
  companion object {
    /** The host's terminology provider, if any. Native-side by design: the
     *  verifier runs synchronously and never calls back into JS. */
    @JvmStatic
    var terminology: FhirPodsTerminology? = null
  }

  override fun definition() = ModuleDefinition {
    Name("FhirPodsExpo")

    Function("abiVersion") { FhirPodsNative.abiVersion() }

    // Synchronous: a FHIR resource (JSON string) in, a JSON document out. Each
    // is a straight JNI call into the Rust core — no bridge round-trip.
    Function("validate") { json: String ->
      String(FhirPodsNative.validate(json.toByteArray(Charsets.UTF_8), terminology), Charsets.UTF_8)
    }

    Function("operationOutcome") { json: String ->
      String(FhirPodsNative.operationOutcome(json.toByteArray(Charsets.UTF_8), terminology), Charsets.UTF_8)
    }

    // Must-support gaps: information for a client certifying against the IG,
    // never part of the validation report.
    Function("mustSupport") { json: String ->
      String(FhirPodsNative.mustSupport(json.toByteArray(Charsets.UTF_8)), Charsets.UTF_8)
    }

    // Lowering: a conformant resource from a profile URL and its variable inputs.
    Function("lower") { profileUrl: String, inputsJson: String ->
      String(FhirPodsNative.lower(profileUrl.toByteArray(Charsets.UTF_8), inputsJson.toByteArray(Charsets.UTF_8), terminology), Charsets.UTF_8)
    }

    Function("slots") { profileUrl: String ->
      String(FhirPodsNative.slots(profileUrl.toByteArray(Charsets.UTF_8)), Charsets.UTF_8)
    }

    // De-identification: {"resource","applied","validation","remaining"} or {"error"}.
    Function("deidentify") { json: String, policyJson: String ->
      String(FhirPodsNative.deidentify(json.toByteArray(Charsets.UTF_8), policyJson.toByteArray(Charsets.UTF_8)), Charsets.UTF_8)
    }

    // Compiled terminology: load a table for every call that follows
    // (1 = loaded, 0 = unparseable; "" clears).
    Function("terminologyLoad") { table: String ->
      FhirPodsNative.terminologyLoad(table.toByteArray(Charsets.UTF_8))
    }

    // {"needs":[{"valueSet","system","code"},…]} — what a host must resolve.
    Function("terminologyNeeds") { json: String ->
      String(FhirPodsNative.terminologyNeeds(json.toByteArray(Charsets.UTF_8)), Charsets.UTF_8)
    }
  }
}
