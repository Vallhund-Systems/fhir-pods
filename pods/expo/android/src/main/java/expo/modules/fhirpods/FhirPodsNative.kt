package expo.modules.fhirpods

/**
 * The terminology seam: asked, synchronously and on the calling thread, about
 * the bindings the bundled value sets can't decide. Return 1 (member),
 * 0 (not a member) or anything else (unknown — a warning, never an error).
 * `system` is "" for a bare `code` element. Set once from native app code:
 *
 *     FhirPodsExpoModule.terminology = FhirPodsTerminology { vs, system, code -> if (cache.contains(vs, system, code)) 1 else 0 }
 */
fun interface FhirPodsTerminology {
  fun validateCode(valueSet: String, system: String, code: String): Int
}

/**
 * The JNI surface of libfhir_pods_ffi.so — the same entrypoints every pod
 * calls, reached through the shim in crates/fhir-pods-ffi (Android only).
 * Bytes in, bytes out: UTF-8 both ways, no modified-UTF-8 in the path.
 */
internal object FhirPodsNative {
  init {
    System.loadLibrary("fhir_pods_ffi")
  }

  external fun abiVersion(): Int

  /** The ergonomic report: {"resourceType","profile","valid","issues":[...]}. */
  external fun validate(json: ByteArray, terminology: FhirPodsTerminology?): ByteArray

  /** The spec resource: a FHIR OperationOutcome (the $validate wire form). */
  external fun operationOutcome(json: ByteArray, terminology: FhirPodsTerminology?): ByteArray

  /** {"mustSupportGaps":[...]} — information, never part of the validation report. */
  external fun mustSupport(json: ByteArray): ByteArray

  /** {"resource","validation"} from a profile URL and its variable inputs, or {"error"}. */
  external fun lower(profile: ByteArray, inputs: ByteArray): ByteArray

  /** {"slots":[...]} — the inputs a profile takes, or {"error"}. */
  external fun slots(profile: ByteArray): ByteArray
}
