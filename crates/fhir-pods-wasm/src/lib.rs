//! # fhir-pods-wasm
//!
//! The WASM boundary for `fhir-pods`, deliberately as a **lean C ABI** rather
//! than wasm-bindgen. The host passes a UTF-8 JSON resource as `(ptr, len)`; the
//! pod returns a JSON report as `(ptr, len)`. Memory across the boundary is
//! managed explicitly via [`pod_alloc`] / [`pod_free`].
//!
//! Why not wasm-bindgen: it pulls a JS-glue toolchain and inflates the module.
//! A raw ABI keeps the `.wasm` tiny — the whole point of running the verifier on
//! device / at the edge. A wasm-bindgen (or a hand-written JS) wrapper can sit
//! *on top* of this ABI later for ergonomic TypeScript, without the core paying
//! for it.
//!
//! `unsafe` is confined to this FFI shim; the verifier core forbids it.
#![allow(clippy::missing_safety_doc)]

/// Allocate `len` bytes in the module's linear memory and hand the pointer to
/// the host. The host fills it, calls an entrypoint, then frees via [`pod_free`].
#[no_mangle]
pub extern "C" fn pod_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Free a buffer previously returned by [`pod_alloc`] (capacity == `len`).
#[no_mangle]
pub unsafe extern "C" fn pod_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, len));
    }
}

/// ABI version, so a host can refuse a mismatched module.
#[no_mangle]
pub extern "C" fn fhir_pods_abi_version() -> u32 {
    0
}

// Entrypoints (`fhir_pods_validate(ptr, len) -> packed(ptr,len)`, etc.) land
// once the core validator is implemented; they call straight into
// `fhir_pods_core::validate`. Kept out of commit zero so the ABI surface is
// designed alongside the first real verifier pass, not ahead of it.
