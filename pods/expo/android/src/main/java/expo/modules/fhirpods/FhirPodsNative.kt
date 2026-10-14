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
  external fun lower(profile: ByteArray, inputs: ByteArray, terminology: FhirPodsTerminology?): ByteArray

  /** {"inputs","validation","missing","extras"} — a resource read back into the profile's slots, or {"error"}. */
  external fun lift(profile: ByteArray, resource: ByteArray, terminology: FhirPodsTerminology?): ByteArray

  /** {"bundle","validation","skipped","unplaced"} — an HL7 v2 message as a FHIR message Bundle, or {"error"}. */
  external fun v2(message: ByteArray, options: ByteArray, terminology: FhirPodsTerminology?): ByteArray

  /** {"slots":[...]} — the inputs a profile takes, or {"error"}. */
  external fun slots(profile: ByteArray): ByteArray

  /** {"document","placed","validation"} — an assembled patient summary, or {"error"}. */
  external fun document(kind: ByteArray, inputs: ByteArray, terminology: FhirPodsTerminology?): ByteArray

  /** {"resource","validation"} — an IHE BALP AuditEvent for a kind of event, or {"error"}. */
  external fun audit(kind: ByteArray, inputs: ByteArray, terminology: FhirPodsTerminology?): ByteArray

  /** {"regions","profiles"} — what is compiled in. */
  external fun profiles(): ByteArray

  /** {"region","resources","document","audit"} — a record re-profiled for a region, or {"error"}. */
  external fun export(region: ByteArray, inputs: ByteArray, terminology: FhirPodsTerminology?): ByteArray

  /** {"resource","applied","validation","remaining"} — the de-identified copy, or {"error"}. */
  external fun deidentify(resource: ByteArray, policy: ByteArray): ByteArray

  /** Load a compiled terminology table: 1 = loaded, 0 = unparseable (the old one stays). */
  external fun terminologyLoad(table: ByteArray): Int

  /** Load a Questionnaire for the responses that name it: 1 = loaded, 0 = not a Questionnaire. */
  external fun questionnaireLoad(json: ByteArray): Int

  /** Host options for the calls that follow: 1 = taken, 0 = refused (the previous options stay). */
  external fun options(json: ByteArray): Int

  /** {"needs":[...]} — the bindings neither the bundled sets nor the table could decide. */
  external fun terminologyNeeds(json: ByteArray): ByteArray
}
