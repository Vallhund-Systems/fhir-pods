//! # fhir-pods-ffi
//!
//! The one C ABI for `fhir-pods`, compiled per target: `wasm32` for the wasm
//! pod, a native staticlib for the expo pod. Deliberately a **lean C ABI** (no
//! wasm-bindgen, no UniFFI runtime) — the boundary is: pass a UTF-8 JSON
//! resource as `(ptr, len)`, get a length-prefixed JSON document back.
//!
//! Two entrypoints, the two layers of the API:
//! - [`fhir_pods_validate`] — the ergonomic report (`valid`, `profile`, `issues[]`).
//! - [`fhir_pods_operation_outcome`] — the spec resource, for the wire.
//!
//! ## Memory contract
//! - Buffers returned to the host are `[u32 little-endian length][UTF-8 bytes…]`.
//! - The host frees any buffer (its own input, or a returned document) with
//!   [`pod_free`], passing the buffer's full byte length.
//! - Alloc/free use boxed-slice semantics so the freed length always matches the
//!   allocated length exactly (sound across the FFI boundary).
//!
//! `unsafe` is confined to this shim; `fhir-pods-core` forbids it.
#![allow(clippy::missing_safety_doc)]

use fhir_pods_core::{validate_bytes, wire};

/// Bump when the exported surface or the memory contract changes; the
/// wrappers refuse a mismatched module.
pub const ABI_VERSION: u32 = 1;

/// Allocate `len` bytes for the host to fill, then pass back via an entrypoint.
#[no_mangle]
pub extern "C" fn pod_alloc(len: usize) -> *mut u8 {
    let boxed = vec![0u8; len].into_boxed_slice();
    Box::into_raw(boxed) as *mut u8
}

/// Free a buffer previously handed to the host (`len` == its full byte length).
#[no_mangle]
pub unsafe extern "C" fn pod_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Box::from_raw(std::slice::from_raw_parts_mut(ptr, len)));
    }
}

/// ABI version, so a host can refuse a mismatched module.
#[no_mangle]
pub extern "C" fn fhir_pods_abi_version() -> u32 {
    ABI_VERSION
}

/// Validate a FHIR resource; returns the pods' report
/// (`{"resourceType","profile","valid","issues":[…]}`).
///
/// Input: UTF-8 JSON at `(ptr, len)`. Returns a freshly allocated
/// `[u32 le length][UTF-8 JSON…]` buffer — read the prefix, copy the document,
/// then `pod_free(ret, 4 + length)`. Never null: a parse failure is a `fatal`
/// issue in the report, not an error at the boundary.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_validate(ptr: *const u8, len: usize) -> *mut u8 {
    let v = validate_bytes(input(ptr, len));
    pack(wire::report(&v).as_bytes())
}

/// Validate a FHIR resource; returns a FHIR `OperationOutcome` resource — the
/// `$validate` wire form. Same memory contract as [`fhir_pods_validate`].
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_operation_outcome(ptr: *const u8, len: usize) -> *mut u8 {
    let v = validate_bytes(input(ptr, len));
    pack(v.to_operation_outcome().as_bytes())
}

unsafe fn input<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

/// Length-prefix `bytes` into a boxed slice and hand ownership to the caller.
fn pack(bytes: &[u8]) -> *mut u8 {
    let mut v = Vec::with_capacity(4 + bytes.len());
    v.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    v.extend_from_slice(bytes);
    Box::into_raw(v.into_boxed_slice()) as *mut u8
}

/// Android: the JVM cannot call a C ABI directly, so the expo pod's Kotlin
/// wrapper reaches the same two entrypoints through this JNI shim. Bytes in,
/// bytes out (`ByteArray`), so no modified-UTF-8 surprises; the JVM owns both
/// arrays, so there is no `pod_alloc`/`pod_free` dance across JNI.
/// Symbol names bind to `expo.modules.fhirpods.FhirPodsNative`.
#[cfg(target_os = "android")]
mod android {
    use jni::objects::{JByteArray, JObject};
    use jni::sys::{jbyteArray, jint};
    use jni::JNIEnv;

    use fhir_pods_core::{validate_bytes, wire};

    fn bytes_in(env: &mut JNIEnv, arr: &JByteArray) -> Vec<u8> {
        env.convert_byte_array(arr).unwrap_or_default()
    }

    fn bytes_out(env: &mut JNIEnv, s: &str) -> jbyteArray {
        match env.byte_array_from_slice(s.as_bytes()) {
            Ok(arr) => arr.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_abiVersion(_env: JNIEnv, _this: JObject) -> jint {
        super::ABI_VERSION as jint
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_validate<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        json: JByteArray<'l>,
    ) -> jbyteArray {
        let v = validate_bytes(&bytes_in(&mut env, &json));
        bytes_out(&mut env, &wire::report(&v))
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_operationOutcome<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        json: JByteArray<'l>,
    ) -> jbyteArray {
        let v = validate_bytes(&bytes_in(&mut env, &json));
        bytes_out(&mut env, &v.to_operation_outcome())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip through the C ABI exactly as a host would.
    unsafe fn call(f: unsafe extern "C" fn(*const u8, usize) -> *mut u8, json: &str) -> String {
        let inp = pod_alloc(json.len());
        std::ptr::copy_nonoverlapping(json.as_ptr(), inp, json.len());
        let ret = f(inp, json.len());
        pod_free(inp, json.len());
        let len = u32::from_le_bytes(*(ret as *const [u8; 4])) as usize;
        let out = String::from_utf8(std::slice::from_raw_parts(ret.add(4), len).to_vec()).unwrap();
        pod_free(ret, 4 + len);
        out
    }

    #[test]
    fn both_entrypoints_round_trip() {
        unsafe {
            let r = call(fhir_pods_validate, r#"{"resourceType":"Patient","active":true}"#);
            assert!(r.contains("\"valid\":false"), "{r}");
            assert!(r.contains("Patient.identifier is required"), "{r}");
            let o = call(fhir_pods_operation_outcome, r#"{"resourceType":"Patient","active":true}"#);
            assert!(o.starts_with("{\"resourceType\":\"OperationOutcome\""), "{o}");
            let bad = call(fhir_pods_validate, "not json");
            assert!(bad.contains("\"severity\":\"fatal\""), "{bad}");
            assert!(call(fhir_pods_validate, "").contains("fatal"));
            assert_eq!(fhir_pods_abi_version(), ABI_VERSION);
        }
    }

    #[test]
    fn null_input_is_empty_input() {
        unsafe {
            let ret = fhir_pods_validate(std::ptr::null(), 0);
            let len = u32::from_le_bytes(*(ret as *const [u8; 4])) as usize;
            pod_free(ret, 4 + len);
        }
    }
}
