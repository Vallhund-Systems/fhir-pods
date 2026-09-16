import ExpoModulesCore
import FhirPodsFFI  // the C ABI, from the vendored XCFramework's modulemap

/// The terminology seam: asked, synchronously and on the calling thread, about
/// the bindings the bundled value sets can't decide. Return `true` (member),
/// `false` (not a member) or `nil` (unknown — a warning, never an error).
/// `system` is "" for a bare `code` element. Set once from native app code:
///
///     FhirPodsExpoModule.terminology = { valueSet, system, code in myCache.contains(valueSet, system, code) }
public typealias FhirPodsTerminology = (_ valueSet: String, _ system: String, _ code: String) -> Bool?

public class FhirPodsExpoModule: Module {
  /// The host's terminology provider, if any. Native-side by design: the
  /// verifier runs synchronously and never calls back into JS.
  public static var terminology: FhirPodsTerminology?

  public func definition() -> ModuleDefinition {
    Name("FhirPodsExpo")

    Function("abiVersion") { () -> Int in
      Int(fhir_pods_abi_version())
    }

    // Synchronous: a FHIR resource (JSON string) in, a JSON document out. Each
    // is a straight C call into the Rust core — no bridge round-trip.

    // The ergonomic report: {"resourceType","profile","valid","issues":[...]}.
    Function("validate") { (json: String) -> String in
      call(fhir_pods_validate_tx, json)
    }

    // The spec resource: a FHIR OperationOutcome (the $validate wire form).
    Function("operationOutcome") { (json: String) -> String in
      call(fhir_pods_operation_outcome_tx, json)
    }

    // Must-support gaps: information for a client certifying against the IG,
    // never part of the validation report.
    Function("mustSupport") { (json: String) -> String in
      call({ ptr, len, _, _ in fhir_pods_must_support(ptr, len) }, json)
    }

    // Lowering: {"resource","validation"} from a profile URL and its variable
    // inputs (a JSON object keyed by slot path), or {"error"}.
    Function("lower") { (profileUrl: String, inputsJson: String) -> String in
      call2(fhir_pods_lower_tx, profileUrl, inputsJson)
    }

    // The inputs a profile takes: {"slots":[…]} or {"error"}.
    Function("slots") { (profileUrl: String) -> String in
      call({ ptr, len, _, _ in fhir_pods_slots(ptr, len) }, profileUrl)
    }
  }
}

/// A two-string call across the C ABI (profile + inputs), same contract.
private func call2(
  _ entry: (UnsafePointer<UInt8>?, Int, UnsafePointer<UInt8>?, Int, fhir_pods_tx_callback?, UnsafeMutableRawPointer?) -> UnsafeMutablePointer<UInt8>?,
  _ a: String, _ b: String
) -> String {
  let ab = Array(a.utf8), bb = Array(b.utf8)
  let cb: fhir_pods_tx_callback? = FhirPodsExpoModule.terminology == nil ? nil : terminologyTrampoline
  let ret: UnsafeMutablePointer<UInt8>? = ab.withUnsafeBufferPointer { pa in
    bb.withUnsafeBufferPointer { pb in entry(pa.baseAddress, pa.count, pb.baseAddress, pb.count, cb, nil) }
  }
  guard let ret = ret else { return "{\"error\":\"no result\"}" }
  var lenLE: UInt32 = 0
  withUnsafeMutableBytes(of: &lenLE) { $0.copyBytes(from: UnsafeRawBufferPointer(start: ret, count: 4)) }
  let length = Int(UInt32(littleEndian: lenLE))
  let document = String(decoding: UnsafeBufferPointer(start: ret.advanced(by: 4), count: length), as: UTF8.self)
  pod_free(ret, 4 + length)
  return document
}

/// The C trampoline for the terminology callback. `@convention(c)` closures
/// capture nothing, so it reaches the provider through the static above.
private let terminologyTrampoline: fhir_pods_tx_callback = { _, vs, vsLen, sys, sysLen, code, codeLen in
  guard let provider = FhirPodsExpoModule.terminology else { return -1 }
  func str(_ p: UnsafePointer<UInt8>?, _ n: Int) -> String {
    String(decoding: UnsafeBufferPointer(start: p, count: n), as: UTF8.self)
  }
  switch provider(str(vs, vsLen), str(sys, sysLen), str(code, codeLen)) {
  case .some(true): return 1
  case .some(false): return 0
  case .none: return -1
  }
}

/// One call across the C ABI: copy the input in, read the length-prefixed
/// document out, free both. The host terminology callback rides along; the
/// core consults it only when its bundled sets can't answer.
private func call(
  _ entry: (UnsafePointer<UInt8>?, Int, fhir_pods_tx_callback?, UnsafeMutableRawPointer?) -> UnsafeMutablePointer<UInt8>?,
  _ json: String
) -> String {
  let bytes = Array(json.utf8)
  let cb: fhir_pods_tx_callback? = FhirPodsExpoModule.terminology == nil ? nil : terminologyTrampoline
  let ret: UnsafeMutablePointer<UInt8>? = bytes.withUnsafeBufferPointer { buf in
    entry(buf.baseAddress, buf.count, cb, nil)
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
