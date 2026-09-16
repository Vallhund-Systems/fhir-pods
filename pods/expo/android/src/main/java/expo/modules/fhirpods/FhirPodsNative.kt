package expo.modules.fhirpods

/**
 * The JNI surface of libfhir_pods_ffi.so — the same two entrypoints every pod
 * calls, reached through the shim in crates/fhir-pods-ffi (Android only).
 * Bytes in, bytes out: UTF-8 both ways, no modified-UTF-8 in the path.
 */
internal object FhirPodsNative {
  init {
    System.loadLibrary("fhir_pods_ffi")
  }

  external fun abiVersion(): Int

  /** The ergonomic report: {"resourceType","profile","valid","issues":[...]}. */
  external fun validate(json: ByteArray): ByteArray

  /** The spec resource: a FHIR OperationOutcome (the $validate wire form). */
  external fun operationOutcome(json: ByteArray): ByteArray
}
