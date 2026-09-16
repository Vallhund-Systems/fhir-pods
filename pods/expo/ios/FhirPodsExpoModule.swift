import ExpoModulesCore
import FhirPodsFFI  // the C ABI, from the vendored XCFramework's modulemap

public class FhirPodsExpoModule: Module {
  public func definition() -> ModuleDefinition {
    Name("FhirPodsExpo")

    Function("abiVersion") { () -> Int in
      Int(fhir_pods_abi_version())
    }

    // Synchronous: a FHIR resource (JSON string) in, a JSON document out. Each
    // is a straight C call into the Rust core — no bridge round-trip.

    // The ergonomic report: {"resourceType","profile","valid","issues":[...]}.
    Function("validate") { (json: String) -> String in
      call(fhir_pods_validate, json)
    }

    // The spec resource: a FHIR OperationOutcome (the $validate wire form).
    Function("operationOutcome") { (json: String) -> String in
      call(fhir_pods_operation_outcome, json)
    }
  }
}

/// One call across the C ABI: copy the input in, read the length-prefixed
/// document out, free both.
private func call(
  _ entry: (UnsafePointer<UInt8>?, Int) -> UnsafeMutablePointer<UInt8>?,
  _ json: String
) -> String {
  let bytes = Array(json.utf8)
  let ret: UnsafeMutablePointer<UInt8>? = bytes.withUnsafeBufferPointer { buf in
    entry(buf.baseAddress, buf.count)
  }
  // Never null by contract; the empty report is the only defensive fallback.
  guard let ret = ret else { return "{\"valid\":false,\"issues\":[]}" }

  // Layout: [UInt32 little-endian length][UTF-8 document...]
  var lenLE: UInt32 = 0
  withUnsafeMutableBytes(of: &lenLE) { $0.copyBytes(from: UnsafeRawBufferPointer(start: ret, count: 4)) }
  let length = Int(UInt32(littleEndian: lenLE))
  let document = String(decoding: UnsafeBufferPointer(start: ret.advanced(by: 4), count: length), as: UTF8.self)
  pod_free(ret, 4 + length)
  return document
}
