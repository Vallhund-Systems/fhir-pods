import ExpoModulesCore
import FhirPodsFFI  // the C ABI, from the vendored XCFramework's modulemap

public class FhirPodsExpoModule: Module {
  public func definition() -> ModuleDefinition {
    Name("FhirPodsExpo")

    Function("abiVersion") { () -> Int in
      Int(fhir_pods_abi_version())
    }

    // Synchronous: a FHIR resource (JSON string) in, a JSON report string out.
    // The native call is a straight C call into the Rust core.
    Function("probe") { (json: String) -> String in
      let bytes = Array(json.utf8)
      let ret: UnsafeMutablePointer<UInt8>? = bytes.withUnsafeBufferPointer { buf in
        fhir_pods_probe(buf.baseAddress, buf.count)
      }
      guard let ret = ret else { return "{}" }

      // Layout: [UInt32 little-endian length][UTF-8 report...]
      let length = Int(UInt32(littleEndian: ret.withMemoryRebound(to: UInt32.self, capacity: 1) { $0.pointee }))
      let report = String(decoding: UnsafeBufferPointer(start: ret.advanced(by: 4), count: length), as: UTF8.self)
      pod_free(ret, 4 + length)
      return report
    }
  }
}
