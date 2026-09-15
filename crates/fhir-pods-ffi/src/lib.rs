//! # fhir-pods-ffi
//!
//! The one C ABI for `fhir-pods`, compiled per target: `wasm32` for the web/edge
//! pod, a native staticlib for the expo pod. Deliberately a **lean C ABI** (no
//! wasm-bindgen, no UniFFI runtime) — the boundary is: pass a UTF-8 JSON
//! resource as `(ptr, len)`, get a length-prefixed JSON report back.
//!
//! ## Memory contract
//! - Buffers returned to the host are `[u32 little-endian length][UTF-8 bytes…]`.
//! - The host frees any buffer (its own input, or a returned report) with
//!   [`pod_free`], passing the buffer's full byte length.
//! - Alloc/free use boxed-slice semantics so the freed length always matches the
//!   allocated length exactly (sound across the FFI boundary).
//!
//! `unsafe` is confined to this shim; `fhir-pods-core` forbids it.
#![allow(clippy::missing_safety_doc)]

use fhir_pods_core::probe;

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
    0
}

/// Probe a FHIR resource. Input: UTF-8 JSON at `(ptr, len)`. Returns a freshly
/// allocated `[u32 le length][UTF-8 report…]` buffer — read the prefix, copy the
/// report, then `pod_free(ret, 4 + length)`. Returns null only if allocation of
/// the empty case is impossible (it is not; null is reserved for future errors).
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_probe(ptr: *const u8, len: usize) -> *mut u8 {
    let input: &[u8] = if ptr.is_null() { &[] } else { std::slice::from_raw_parts(ptr, len) };
    let report = probe(input);
    pack(report.as_bytes())
}

/// Length-prefix `bytes` into a boxed slice and hand ownership to the caller.
fn pack(bytes: &[u8]) -> *mut u8 {
    let mut v = Vec::with_capacity(4 + bytes.len());
    v.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    v.extend_from_slice(bytes);
    Box::into_raw(v.into_boxed_slice()) as *mut u8
}
