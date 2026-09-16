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
//! Each has a `_tx` twin taking a host **terminology callback** — the seam
//! through which NCTS / VSAC / an on-device cache answer the bindings the
//! bundled value sets can't. The bundled sets are always consulted first.
//! [`fhir_pods_must_support`] is the client-side completeness check —
//! information, never part of the validation report.
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

use core::ffi::c_void;

use fhir_pods_core::terminology::{Bundled, Chain, Membership, TerminologyProvider};
use fhir_pods_core::{must_support_gaps, validate_bytes_with, wire};

/// Bump when the exported surface or the memory contract changes; the
/// wrappers refuse a mismatched module.
pub const ABI_VERSION: u32 = 3;

/// A host terminology callback: is (`system`, `code`) in `value_set`?
/// Strings are UTF-8 `(ptr, len)`, valid only for the duration of the call;
/// `system` is empty for a bare `code` element. Return `1` = member, `0` = not
/// a member, anything else = unknown (degrades to a warning, never an error).
pub type TxCallback = unsafe extern "C" fn(
    ctx: *mut c_void,
    value_set: *const u8,
    value_set_len: usize,
    system: *const u8,
    system_len: usize,
    code: *const u8,
    code_len: usize,
) -> i32;

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
    pack(wire::report(&validate_bytes_with(input(ptr, len), &Bundled)).as_bytes())
}

/// Validate a FHIR resource; returns a FHIR `OperationOutcome` resource — the
/// `$validate` wire form. Same memory contract as [`fhir_pods_validate`].
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_operation_outcome(ptr: *const u8, len: usize) -> *mut u8 {
    pack(validate_bytes_with(input(ptr, len), &Bundled).to_operation_outcome().as_bytes())
}

/// [`fhir_pods_validate`] with a host terminology callback (`cb` may be null).
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_validate_tx(ptr: *const u8, len: usize, cb: Option<TxCallback>, ctx: *mut c_void) -> *mut u8 {
    with_tx(cb, ctx, |tx| pack(wire::report(&validate_bytes_with(input(ptr, len), tx)).as_bytes()))
}

/// [`fhir_pods_operation_outcome`] with a host terminology callback.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_operation_outcome_tx(
    ptr: *const u8,
    len: usize,
    cb: Option<TxCallback>,
    ctx: *mut c_void,
) -> *mut u8 {
    with_tx(cb, ctx, |tx| pack(validate_bytes_with(input(ptr, len), tx).to_operation_outcome().as_bytes()))
}

/// The selected profile's must-support elements the resource leaves empty
/// (where their parent is present): `{"mustSupportGaps":["Patient.telecom",…]}`.
/// Not validation — no validator fails on must-support — but what a client
/// certifying against the IG must populate when it has the data.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_must_support(ptr: *const u8, len: usize) -> *mut u8 {
    pack(must_support_json(input(ptr, len)).as_bytes())
}

fn must_support_json(input: &[u8]) -> String {
    let mut out = String::from("{\"mustSupportGaps\":[");
    for (i, g) in must_support_gaps(input).iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(g);
        out.push('"');
    }
    out.push_str("]}");
    out
}

struct HostTx {
    cb: TxCallback,
    ctx: *mut c_void,
}

impl TerminologyProvider for HostTx {
    fn validate_code(&self, value_set: &str, system: &str, code: &str) -> Membership {
        let r = unsafe {
            (self.cb)(
                self.ctx,
                value_set.as_ptr(),
                value_set.len(),
                system.as_ptr(),
                system.len(),
                code.as_ptr(),
                code.len(),
            )
        };
        membership(r)
    }
}

fn membership(r: i32) -> Membership {
    match r {
        1 => Membership::Member,
        0 => Membership::NotMember,
        _ => Membership::Unknown,
    }
}

fn with_tx<R>(cb: Option<TxCallback>, ctx: *mut c_void, f: impl FnOnce(&dyn TerminologyProvider) -> R) -> R {
    match cb {
        Some(cb) => {
            let host = HostTx { cb, ctx };
            f(&Chain(&[&Bundled, &host]))
        }
        None => f(&Bundled),
    }
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

/// wasm: JS can't hand the module a function pointer, so the host terminology
/// provider is an **import** (`env.fhir_pods_host_tx`) the JS wrapper always
/// supplies (returning "unknown" when the host gave no provider), and these
/// twins route through it.
#[cfg(target_arch = "wasm32")]
mod wasm_host {
    use super::*;

    extern "C" {
        fn fhir_pods_host_tx(
            value_set: *const u8,
            value_set_len: usize,
            system: *const u8,
            system_len: usize,
            code: *const u8,
            code_len: usize,
        ) -> i32;
    }

    struct ImportTx;

    impl TerminologyProvider for ImportTx {
        fn validate_code(&self, value_set: &str, system: &str, code: &str) -> Membership {
            membership(unsafe {
                fhir_pods_host_tx(value_set.as_ptr(), value_set.len(), system.as_ptr(), system.len(), code.as_ptr(), code.len())
            })
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn fhir_pods_validate_host_tx(ptr: *const u8, len: usize) -> *mut u8 {
        pack(wire::report(&validate_bytes_with(input(ptr, len), &Chain(&[&Bundled, &ImportTx]))).as_bytes())
    }

    #[no_mangle]
    pub unsafe extern "C" fn fhir_pods_operation_outcome_host_tx(ptr: *const u8, len: usize) -> *mut u8 {
        pack(validate_bytes_with(input(ptr, len), &Chain(&[&Bundled, &ImportTx])).to_operation_outcome().as_bytes())
    }
}

/// Android: the JVM cannot call a C ABI directly, so the expo pod's Kotlin
/// wrapper reaches the same entrypoints through this JNI shim. Bytes in,
/// bytes out (`ByteArray`), so no modified-UTF-8 surprises; the JVM owns both
/// arrays, so there is no `pod_alloc`/`pod_free` dance across JNI. A host
/// terminology provider is a Java object with
/// `int validateCode(String valueSet, String system, String code)`.
/// Symbol names bind to `expo.modules.fhirpods.FhirPodsNative`.
#[cfg(target_os = "android")]
mod android {
    use core::cell::RefCell;

    use jni::objects::{JByteArray, JObject, JValue};
    use jni::sys::{jbyteArray, jint};
    use jni::JNIEnv;

    use super::*;

    fn bytes_in(env: &mut JNIEnv, arr: &JByteArray) -> Vec<u8> {
        env.convert_byte_array(arr).unwrap_or_default()
    }

    fn bytes_out(env: &mut JNIEnv, s: &str) -> jbyteArray {
        match env.byte_array_from_slice(s.as_bytes()) {
            Ok(arr) => arr.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }

    /// Calls back into the Java provider. The env is borrowed mutably per
    /// call; the engine is single-threaded and never re-enters.
    struct JavaTx<'a, 'l> {
        env: RefCell<&'a mut JNIEnv<'l>>,
        provider: &'a JObject<'l>,
    }

    impl TerminologyProvider for JavaTx<'_, '_> {
        fn validate_code(&self, value_set: &str, system: &str, code: &str) -> Membership {
            let mut env = self.env.borrow_mut();
            let (Ok(vs), Ok(sys), Ok(cd)) = (env.new_string(value_set), env.new_string(system), env.new_string(code)) else {
                return Membership::Unknown;
            };
            let r = env.call_method(
                self.provider,
                "validateCode",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I",
                &[JValue::Object(&vs), JValue::Object(&sys), JValue::Object(&cd)],
            );
            let m = match r.and_then(|v| v.i()) {
                Ok(i) => membership(i),
                Err(_) => {
                    // A throwing provider is an unknown answer, not a crash.
                    let _ = env.exception_clear();
                    Membership::Unknown
                }
            };
            let _ = env.delete_local_ref(vs);
            let _ = env.delete_local_ref(sys);
            let _ = env.delete_local_ref(cd);
            m
        }
    }

    fn run<'l>(env: &mut JNIEnv<'l>, json: &JByteArray<'l>, provider: &JObject<'l>, outcome: bool) -> jbyteArray {
        let input = bytes_in(env, json);
        let doc = if provider.is_null() {
            let v = validate_bytes_with(&input, &Bundled);
            if outcome { v.to_operation_outcome() } else { wire::report(&v) }
        } else {
            let host = JavaTx { env: RefCell::new(env), provider };
            let v = validate_bytes_with(&input, &Chain(&[&Bundled, &host]));
            if outcome { v.to_operation_outcome() } else { wire::report(&v) }
        };
        bytes_out(env, &doc)
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_abiVersion(_env: JNIEnv, _this: JObject) -> jint {
        ABI_VERSION as jint
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_validate<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        json: JByteArray<'l>,
        provider: JObject<'l>,
    ) -> jbyteArray {
        run(&mut env, &json, &provider, false)
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_mustSupport<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        json: JByteArray<'l>,
    ) -> jbyteArray {
        let input = bytes_in(&mut env, &json);
        let doc = must_support_json(&input);
        bytes_out(&mut env, &doc)
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_operationOutcome<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        json: JByteArray<'l>,
        provider: JObject<'l>,
    ) -> jbyteArray {
        run(&mut env, &json, &provider, true)
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
        unpack(ret)
    }

    unsafe fn unpack(ret: *mut u8) -> String {
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
    fn must_support_is_information_not_validation() {
        unsafe {
            let r = call(fhir_pods_must_support, r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org/fhir/us/core/StructureDefinition/us-core-patient"]},"identifier":[{"system":"http://h.example.org/mrn","value":"1"}],"name":[{"family":"D"}],"gender":"male"}"#);
            assert!(r.starts_with("{\"mustSupportGaps\":[") && r.contains("\"Patient.birthDate\""), "{r}");
            assert_eq!(call(fhir_pods_must_support, "not json"), "{\"mustSupportGaps\":[]}");
        }
    }

    #[test]
    fn null_input_is_empty_input() {
        unsafe {
            let ret = fhir_pods_validate(std::ptr::null(), 0);
            unpack(ret);
        }
    }

    /// The host callback is asked only what the bundled sets can't answer,
    /// sees UTF-8 strings, and its answer lands in the report.
    #[test]
    fn host_terminology_callback() {
        struct Log(Vec<String>, i32);
        unsafe extern "C" fn cb(ctx: *mut c_void, vs: *const u8, vsl: usize, sys: *const u8, sysl: usize, code: *const u8, cl: usize) -> i32 {
            let log = &mut *(ctx as *mut Log);
            let s = |p: *const u8, n: usize| String::from_utf8_lossy(std::slice::from_raw_parts(p, n)).into_owned();
            log.0.push(format!("{} {} {}", s(vs, vsl), s(sys, sysl), s(code, cl)));
            log.1
        }
        let json = r#"{"resourceType":"Patient","identifier":[{"system":"http://h.example.org/mrn","value":"1"}],"name":[{"family":"D"}],"gender":"male","birthDate":"1970-01-01","telecom":[{"system":"phone","value":"1"}]}"#;
        unsafe {
            let mut log = Log(Vec::new(), 0);
            let r = unpack(fhir_pods_validate_tx(json.as_ptr(), json.len(), Some(cb), &mut log as *mut Log as *mut c_void));
            assert_eq!(log.0, vec!["http://hl7.org/fhir/ValueSet/contact-point-system|4.0.1  phone"]);
            assert!(r.contains("'phone' is not in the bound value set"), "{r}");

            let mut log = Log(Vec::new(), 1);
            let r = unpack(fhir_pods_validate_tx(json.as_ptr(), json.len(), Some(cb), &mut log as *mut Log as *mut c_void));
            assert!(!r.contains("contact-point-system"), "{r}");

            let r = unpack(fhir_pods_validate_tx(json.as_ptr(), json.len(), None, std::ptr::null_mut()));
            assert!(r.contains("no bundled terminology for http://hl7.org/fhir/ValueSet/contact-point-system"), "{r}");
        }
    }
}
