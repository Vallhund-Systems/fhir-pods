package expo.modules.fhirpods

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class FhirPodsExpoModule : Module() {
  companion object {
    /** The host's terminology provider, if any. Native-side by design: the
     *  verifier runs synchronously and never calls back into JS. */
    @JvmStatic
    var terminology: FhirPodsTerminology? = null

    /** The C ABI this wrapper is written against. The vendored jniLibs are
     *  built from the same commit, so a mismatch means mismatched artifacts
     *  were packed or vendored — reported at module creation rather than left
     *  to surface at the first call. Mirrored in `FhirPodsExpoModule.swift`
     *  and `src/index.ts`. */
    const val EXPECTED_ABI = 14
  }

  override fun definition() = ModuleDefinition {
    Name("FhirPodsExpo")

    OnCreate {
      val abi = FhirPodsNative.abiVersion()
      if (abi != EXPECTED_ABI) {
        android.util.Log.e(
          "fhir-pods",
          "the vendored native library speaks ABI $abi, this module expects $EXPECTED_ABI - the packed artifacts do not match",
        )
      }
    }

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

    // Documents: {"document","placed","validation"} or {"error"}.
    Function("document") { kind: String, inputsJson: String ->
      String(FhirPodsNative.document(kind.toByteArray(Charsets.UTF_8), inputsJson.toByteArray(Charsets.UTF_8), terminology), Charsets.UTF_8)
    }

    // Lifting: {"inputs","validation","missing","extras"} or {"error"}.
    Function("lift") { profileUrl: String, resourceJson: String ->
      String(FhirPodsNative.lift(profileUrl.toByteArray(Charsets.UTF_8), resourceJson.toByteArray(Charsets.UTF_8), terminology), Charsets.UTF_8)
    }

    // HL7 v2: {"bundle","validation","skipped","unplaced"} or {"error"}.
    Function("v2") { message: String, optionsJson: String ->
      String(FhirPodsNative.v2(message.toByteArray(Charsets.UTF_8), optionsJson.toByteArray(Charsets.UTF_8), terminology), Charsets.UTF_8)
    }

    // Audit: {"resource","validation"} or {"error"}.
    Function("audit") { kind: String, inputsJson: String ->
      String(FhirPodsNative.audit(kind.toByteArray(Charsets.UTF_8), inputsJson.toByteArray(Charsets.UTF_8), terminology), Charsets.UTF_8)
    }

    // Discovery: {"regions","profiles"}.
    Function("profiles") { String(FhirPodsNative.profiles(), Charsets.UTF_8) }

    // Export: {"region","resources","document","audit"} or {"error"}.
    Function("export") { region: String, inputsJson: String ->
      String(FhirPodsNative.export(region.toByteArray(Charsets.UTF_8), inputsJson.toByteArray(Charsets.UTF_8), terminology), Charsets.UTF_8)
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

    // A Questionnaire for every QuestionnaireResponse validated after it
    // (1 = loaded, 0 = not a Questionnaire; "" clears).
    Function("questionnaireLoad") { json: String ->
      FhirPodsNative.questionnaireLoad(json.toByteArray(Charsets.UTF_8))
    }

    // Host options for every call that follows: {"unknownProfile":
    // "warning"|"error", "externalNarrativeLinks": "allow"|"refuse"} — each
    // default the reference validator's own behaviour (1 = taken, 0 =
    // refused and the previous options stay; "" restores the defaults).
    Function("options") { json: String ->
      FhirPodsNative.options(json.toByteArray(Charsets.UTF_8))
    }

    // {"needs":[{"valueSet","system","code"},…]} — what a host must resolve.
    Function("terminologyNeeds") { json: String ->
      String(FhirPodsNative.terminologyNeeds(json.toByteArray(Charsets.UTF_8)), Charsets.UTF_8)
    }
  }
}
