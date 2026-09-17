//! # fhir-pods-ffi
//!
//! The one C ABI for `fhir-pods`, compiled per target: `wasm32` for the wasm
//! pod, a native staticlib for the expo pod. Deliberately a **lean C ABI** (no
//! wasm-bindgen, no UniFFI runtime) — the boundary is: pass a UTF-8 JSON
//! resource as `(ptr, len)`, get a length-prefixed JSON document back.
//!
//! The entrypoints — those below in detail, and `fhir_pods_lift`,
//! `fhir_pods_deidentify`, `fhir_pods_document`, `fhir_pods_audit`,
//! `fhir_pods_v2`, `fhir_pods_terminology_load` / `_needs` alongside:
//! - [`fhir_pods_validate`] — the `Validation` JSON (`valid`, `profile(s)`, `issues[]`).
//! - [`fhir_pods_operation_outcome`] — the spec resource, for the wire.
//! - [`fhir_pods_must_support`] — must-support gaps, information only.
//! - [`fhir_pods_lower`] — build a conformant resource from its variable inputs (and validate it).
//! - [`fhir_pods_slots`] — the inputs a profile takes.
//!
//! Each has a `_tx` twin taking a host **terminology callback** — the seam
//! through which NCTS / VSAC / an on-device cache answer the bindings the
//! bundled value sets can't. The bundled sets are always consulted first.
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

use std::sync::{OnceLock, RwLock};

use fhir_pods_core::terminology::{Bundled, Chain, Expansions, Membership, Recorder, TerminologyProvider};
use fhir_pods_core::{must_support_gaps, wire, Validator};

/// One validator per process: the registry's invariants are parsed once, on
/// first use (about half a millisecond), and every call after that is free
/// of parsing. `Validator` is immutable after construction, so sharing it is
/// sound; `OnceLock` also serialises the one-time build on native targets.
fn validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(Validator::new)
}

/// Bump when the exported surface or the memory contract changes; the
/// wrappers refuse a mismatched module.
pub const ABI_VERSION: u32 = 10;

/// The compiled terminology table the host loaded (`fhir_pods_terminology_load`),
/// consulted after the bundled sets and before any host callback.
static TABLE: RwLock<Option<std::sync::Arc<Expansions>>> = RwLock::new(None);

/// Run `f` with the providers in order: bundled sets, the loaded table, the
/// host's callback. The table is taken as a snapshot and the lock released
/// before any host code runs, so a callback may itself load a table
/// (`fhir_pods_terminology_load`) — it takes effect for the calls that follow.
fn with_providers<R>(host: Option<&dyn TerminologyProvider>, f: impl FnOnce(&dyn TerminologyProvider) -> R) -> R {
    let table = TABLE.read().unwrap_or_else(|e| e.into_inner()).clone();
    match (table.as_deref(), host) {
        (Some(t), Some(h)) => f(&Chain(&[&Bundled, t, h])),
        (Some(t), None) => f(&Chain(&[&Bundled, t])),
        (None, Some(h)) => f(&Chain(&[&Bundled, h])),
        (None, None) => f(&Bundled),
    }
}

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
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
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
    with_providers(None, |tx| pack(wire::report(&validator().validate_with(input(ptr, len), tx)).as_bytes()))
}

/// Validate a FHIR resource; returns a FHIR `OperationOutcome` resource — the
/// `$validate` wire form. Same memory contract as [`fhir_pods_validate`].
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_operation_outcome(ptr: *const u8, len: usize) -> *mut u8 {
    with_providers(None, |tx| pack(validator().validate_with(input(ptr, len), tx).to_operation_outcome().as_bytes()))
}

/// [`fhir_pods_validate`] with a host terminology callback (`cb` may be null).
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_validate_tx(
    ptr: *const u8,
    len: usize,
    cb: Option<TxCallback>,
    ctx: *mut c_void,
) -> *mut u8 {
    with_tx(cb, ctx, |tx| pack(wire::report(&validator().validate_with(input(ptr, len), tx)).as_bytes()))
}

/// [`fhir_pods_operation_outcome`] with a host terminology callback.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_operation_outcome_tx(
    ptr: *const u8,
    len: usize,
    cb: Option<TxCallback>,
    ctx: *mut c_void,
) -> *mut u8 {
    with_tx(cb, ctx, |tx| pack(validator().validate_with(input(ptr, len), tx).to_operation_outcome().as_bytes()))
}

/// The selected profile's must-support elements the resource leaves empty
/// (where their parent is present): `{"mustSupportGaps":["Patient.telecom",…]}`.
/// Not validation — no validator fails on must-support — but what a client
/// certifying against the IG must populate when it has the data.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_must_support(ptr: *const u8, len: usize) -> *mut u8 {
    pack(must_support_json(input(ptr, len)).as_bytes())
}

/// Lower: `profile` is a canonical URL, `inputs` a JSON object keyed by slot
/// path. Returns `{"resource": {…}, "validation": {…}}`, or `{"error": "…"}`
/// when lowering could not start (unknown profile, inputs not an object).
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_lower(
    profile: *const u8,
    profile_len: usize,
    inputs: *const u8,
    inputs_len: usize,
) -> *mut u8 {
    with_providers(None, |tx| pack(lower_json(input(profile, profile_len), input(inputs, inputs_len), tx).as_bytes()))
}

/// [`fhir_pods_lower`] with a host terminology callback: it answers the
/// bindings and supplies displays for `{system, code}` inputs.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_lower_tx(
    profile: *const u8,
    profile_len: usize,
    inputs: *const u8,
    inputs_len: usize,
    cb: Option<TxCallback>,
    ctx: *mut c_void,
) -> *mut u8 {
    with_tx(cb, ctx, |tx| pack(lower_json(input(profile, profile_len), input(inputs, inputs_len), tx).as_bytes()))
}

/// De-identify: `resource` under `policy` (JSON — `{"salt": "…", "today":
/// "2026-09-16", "base": "safe-harbor" | "none", "rules": [{"target",
/// "action"}]}`). Returns `{"resource": {…}, "applied": [{"path","action"}],
/// "validation": {…}, "remaining": [{"path","what"}]}`, or `{"error": "…"}`
/// when it could not start (bad policy, not a resource, unknown type).
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_deidentify(
    resource: *const u8,
    resource_len: usize,
    policy: *const u8,
    policy_len: usize,
) -> *mut u8 {
    pack(deidentify_json(input(resource, resource_len), input(policy, policy_len)).as_bytes())
}

fn deidentify_json(resource: &[u8], policy: &[u8]) -> String {
    let error = |e: &dyn core::fmt::Display| {
        let mut s = String::from("{\"error\":\"");
        wire::escape(&mut s, &e.to_string());
        s.push_str("\"}");
        s
    };
    let policy = match fhir_pods_core::deidentify::Policy::from_json(policy) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };
    let d = match validator().deidentify(resource, &policy) {
        Ok(d) => d,
        Err(e) => return error(&e),
    };
    let mut s = String::with_capacity(d.resource.len() + 512);
    s.push_str("{\"resource\":");
    s.push_str(&d.resource);
    s.push_str(",\"applied\":[");
    for (i, a) in d.applied.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str("{\"path\":\"");
        wire::escape(&mut s, &a.path);
        s.push_str("\",\"action\":\"");
        s.push_str(a.action.name());
        s.push_str("\"}");
    }
    s.push_str("],\"validation\":");
    s.push_str(&wire::report(&d.validation));
    s.push_str(",\"remaining\":[");
    for (i, f) in d.remaining.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str("{\"path\":\"");
        wire::escape(&mut s, &f.path);
        s.push_str("\",\"what\":\"");
        wire::escape(&mut s, &f.what);
        s.push_str("\"}");
    }
    s.push_str("],\"residual\":[");
    for (i, f) in d.residual.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str("{\"path\":\"");
        wire::escape(&mut s, &f.path);
        s.push_str("\",\"what\":\"");
        wire::escape(&mut s, &f.what);
        s.push_str("\"}");
    }
    s.push_str("]}");
    s
}

/// Assemble a patient summary document: `kind` is `ips` or `au-ps`, `inputs`
/// a JSON object — `{"resources": [Patient, …], "author": "Practitioner/x",
/// "date": "…", "title"?, "custodian"?, "identifier"?, "timestamp"?}`. Returns
/// `{"document": {…Bundle…}, "placed": [{"fullUrl", "section"}], "validation": {…}}`,
/// or `{"error": "…"}` when it could not start.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_document(
    kind: *const u8,
    kind_len: usize,
    inputs: *const u8,
    inputs_len: usize,
) -> *mut u8 {
    with_providers(None, |tx| pack(document_json(input(kind, kind_len), input(inputs, inputs_len), tx).as_bytes()))
}

/// [`fhir_pods_document`] with a host terminology callback: the document's
/// validation, and the profile slices that turn on a binding, ask it.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_document_tx(
    kind: *const u8,
    kind_len: usize,
    inputs: *const u8,
    inputs_len: usize,
    cb: Option<TxCallback>,
    ctx: *mut c_void,
) -> *mut u8 {
    with_tx(cb, ctx, |tx| pack(document_json(input(kind, kind_len), input(inputs, inputs_len), tx).as_bytes()))
}

/// Lift: read `resource` back into the slots of `profile` (a canonical URL,
/// or empty to take the most specific compiled profile the resource
/// declares): `{"inputs": {…slot path: value…}, "validation": {…},
/// "missing": [required slots absent], "extras": [element paths no slot
/// covers]}`, or `{"error": "…"}` when it could not start.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_lift(
    profile: *const u8,
    profile_len: usize,
    resource: *const u8,
    resource_len: usize,
) -> *mut u8 {
    with_providers(None, |tx| {
        pack(lift_json(input(profile, profile_len), input(resource, resource_len), tx).as_bytes())
    })
}

/// [`fhir_pods_lift`] with a host terminology callback for the validation.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_lift_tx(
    profile: *const u8,
    profile_len: usize,
    resource: *const u8,
    resource_len: usize,
    cb: Option<TxCallback>,
    ctx: *mut c_void,
) -> *mut u8 {
    with_tx(cb, ctx, |tx| pack(lift_json(input(profile, profile_len), input(resource, resource_len), tx).as_bytes()))
}

/// HL7 v2: an ER7 message converted to a FHIR message Bundle through the
/// `hl7.fhir.uv.v2mappings` maps, validated on the way out:
/// `{"bundle": {…}, "validation": {…}, "skipped": [{"at", "target", "why"}],
/// "unplaced": ["ZPI[1]"]}`, or `{"error": "…"}` when it cannot start (not
/// a message, no map for its structure, bad options). `options` is JSON
/// (`{}` for none): `timezone` (the zone of a v2 time stated without one)
/// and `codeSystems` (coding-system names to uris beyond the standard ones).
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_v2(
    message: *const u8,
    message_len: usize,
    options: *const u8,
    options_len: usize,
) -> *mut u8 {
    with_providers(None, |tx| pack(v2_json(input(message, message_len), input(options, options_len), tx).as_bytes()))
}

/// [`fhir_pods_v2`] with a host terminology callback for the validation.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_v2_tx(
    message: *const u8,
    message_len: usize,
    options: *const u8,
    options_len: usize,
    cb: Option<TxCallback>,
    ctx: *mut c_void,
) -> *mut u8 {
    with_tx(cb, ctx, |tx| pack(v2_json(input(message, message_len), input(options, options_len), tx).as_bytes()))
}

fn v2_json(message: &[u8], options: &[u8], tx: &dyn TerminologyProvider) -> String {
    let error = |e: String| {
        let mut s = String::from("{\"error\":\"");
        wire::escape(&mut s, &e);
        s.push_str("\"}");
        s
    };
    let opts = match fhir_pods_core::v2::convert::Options::from_json(options) {
        Ok(o) => o,
        Err(e) => return error(e.to_string()),
    };
    let c = match validator().v2_with(message, &opts, tx) {
        Ok(c) => c,
        Err(e) => return error(e.to_string()),
    };
    let mut s = String::with_capacity(c.bundle.len() + 512);
    s.push_str("{\"bundle\":");
    s.push_str(&c.bundle);
    s.push_str(",\"validation\":");
    s.push_str(&wire::report(&c.validation));
    s.push_str(",\"skipped\":[");
    for (i, k) in c.skipped.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str("{\"at\":\"");
        wire::escape(&mut s, &k.at);
        s.push_str("\",\"target\":\"");
        wire::escape(&mut s, &k.target);
        s.push_str("\",\"why\":\"");
        wire::escape(&mut s, &k.why);
        s.push_str("\"}");
    }
    s.push_str("],\"unplaced\":[");
    for (i, u) in c.unplaced.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('"');
        wire::escape(&mut s, u);
        s.push('"');
    }
    s.push_str("]}");
    s
}

fn lift_json(profile: &[u8], resource: &[u8], tx: &dyn TerminologyProvider) -> String {
    let url = core::str::from_utf8(profile).unwrap_or("");
    let l = match validator().lift_with(url, resource, tx) {
        Ok(l) => l,
        Err(e) => {
            let mut s = String::from("{\"error\":\"");
            wire::escape(&mut s, &e.to_string());
            s.push_str("\"}");
            return s;
        }
    };
    let mut s = String::with_capacity(l.inputs.len() + 512);
    s.push_str("{\"inputs\":");
    s.push_str(&l.inputs);
    s.push_str(",\"validation\":");
    s.push_str(&wire::report(&l.validation));
    s.push_str(",\"missing\":[");
    for (i, m) in l.missing.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('"');
        wire::escape(&mut s, m);
        s.push('"');
    }
    s.push_str("],\"extras\":[");
    for (i, e) in l.extras.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('"');
        wire::escape(&mut s, e);
        s.push('"');
    }
    s.push_str("]}");
    s
}

/// Audit: an `AuditEvent` for a `kind` of event (`read`, `patient-read`,
/// `query`, `patient-query`, `create`, `patient-create`, `update`,
/// `patient-update`, `delete`, `patient-delete`) from host-shaped inputs
/// (JSON: `{"recorded", "client": {"who","network"}, "server": {…},
/// "user"?: {…}, "source": {"observer"}, "data" | "query", "patient"?,
/// "transaction"?}`), conformant to the IHE BALP profile for it and validated
/// on the way out: `{"resource": {…}, "validation": {…}}`, or `{"error": "…"}`
/// when it could not start.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_audit(
    kind: *const u8,
    kind_len: usize,
    inputs: *const u8,
    inputs_len: usize,
) -> *mut u8 {
    with_providers(None, |tx| pack(audit_json(input(kind, kind_len), input(inputs, inputs_len), tx).as_bytes()))
}

/// [`fhir_pods_audit`] with a host terminology callback.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_audit_tx(
    kind: *const u8,
    kind_len: usize,
    inputs: *const u8,
    inputs_len: usize,
    cb: Option<TxCallback>,
    ctx: *mut c_void,
) -> *mut u8 {
    with_tx(cb, ctx, |tx| pack(audit_json(input(kind, kind_len), input(inputs, inputs_len), tx).as_bytes()))
}

fn audit_json(kind: &[u8], inputs: &[u8], tx: &dyn TerminologyProvider) -> String {
    let kind = core::str::from_utf8(kind).unwrap_or("");
    match validator().audit_with(kind, inputs, tx) {
        Ok(l) => lowered_json(&l.resource, &l.validation),
        Err(e) => {
            let mut s = String::from("{\"error\":\"");
            wire::escape(&mut s, &e.to_string());
            s.push_str("\"}");
            s
        }
    }
}

fn document_json(kind: &[u8], inputs: &[u8], tx: &dyn TerminologyProvider) -> String {
    let kind = core::str::from_utf8(kind).unwrap_or("");
    let a = match validator().document_with(kind, inputs, tx) {
        Ok(a) => a,
        Err(e) => {
            let mut s = String::from("{\"error\":\"");
            wire::escape(&mut s, &e.to_string());
            s.push_str("\"}");
            return s;
        }
    };
    let mut s = String::with_capacity(a.document.len() + 512);
    s.push_str("{\"document\":");
    s.push_str(&a.document);
    s.push_str(",\"placed\":[");
    for (i, (full, section)) in a.placed.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str("{\"fullUrl\":\"");
        wire::escape(&mut s, full);
        s.push_str("\",\"section\":\"");
        wire::escape(&mut s, section);
        s.push_str("\"}");
    }
    s.push_str("],\"omitted\":[");
    for (i, key) in a.omitted.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('"');
        wire::escape(&mut s, key);
        s.push('"');
    }
    s.push_str("],\"validation\":");
    s.push_str(&wire::report(&a.validation));
    s.push('}');
    s
}

/// Load a compiled terminology table (the line format `tools/compile_valueset.py`
/// writes; see `Expansions` in the core). It answers, after the bundled sets
/// and before any host callback, for every call that follows — on this
/// module, for its lifetime. Returns 1 when loaded, 0 when the text could not
/// be parsed (the previous table stays). Empty input clears the table.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_terminology_load(ptr: *const u8, len: usize) -> i32 {
    let text = input(ptr, len);
    let mut slot = TABLE.write().unwrap_or_else(|e| e.into_inner());
    if text.is_empty() {
        *slot = None;
        return 1;
    }
    match core::str::from_utf8(text).ok().and_then(|t| Expansions::parse(t).ok()) {
        Some(t) => {
            *slot = Some(std::sync::Arc::new(t));
            1
        }
        None => 0,
    }
}

/// What a host must resolve for this resource to validate without
/// terminology warnings — every binding the bundled sets and the loaded
/// table could not decide: `{"needs":[{"valueSet","system","code"},…]}`.
/// Resolve them (`$validate-code`, or `$expand` + `compile_valueset.py` +
/// `fhir_pods_terminology_load`) and validate again.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_terminology_needs(ptr: *const u8, len: usize) -> *mut u8 {
    let needs = with_providers(None, |tx| {
        let rec = Recorder::new();
        validator().validate_with(input(ptr, len), &Chain(&[tx, &rec]));
        rec.needs()
    });
    let mut out = String::from("{\"needs\":[");
    for (i, n) in needs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"valueSet\":\"");
        wire::escape(&mut out, &n.value_set);
        out.push_str("\",\"system\":\"");
        wire::escape(&mut out, &n.system);
        out.push_str("\",\"code\":\"");
        wire::escape(&mut out, &n.code);
        out.push_str("\"}");
    }
    out.push_str("]}");
    pack(out.as_bytes())
}

/// The inputs a profile takes: `{"slots":[{"path","type","choices","min","max","mustSupport"}]}`,
/// or `{"error": "…"}` for an unknown profile.
#[no_mangle]
pub unsafe extern "C" fn fhir_pods_slots(profile: *const u8, profile_len: usize) -> *mut u8 {
    let url = core::str::from_utf8(input(profile, profile_len)).unwrap_or("");
    let url = url.split('|').next().unwrap_or(url);
    let Some(p) = fhir_pods_core::generated::REGISTRY.iter().find(|p| p.url == url) else {
        return pack(b"{\"error\":\"no compiled profile with that URL\"}");
    };
    let mut out = String::from("{\"slots\":[");
    for (i, s) in p.lowering.slots.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"path\":\"{}\",\"type\":\"{}\",\"choices\":[{}],\"min\":{},\"max\":{},\"mustSupport\":{}}}",
            s.path,
            s.ty,
            s.choices.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(","),
            s.min,
            match s.max {
                fhir_pods_core::compiled::Max::Bounded(n) => n.to_string(),
                fhir_pods_core::compiled::Max::Star => "null".into(),
            },
            s.must_support
        ));
    }
    out.push_str("]}");
    pack(out.as_bytes())
}

fn lower_json(profile: &[u8], inputs: &[u8], tx: &dyn TerminologyProvider) -> String {
    let url = core::str::from_utf8(profile).unwrap_or("");
    match validator().lower_with(url, inputs, tx) {
        Ok(l) => lowered_json(&l.resource, &l.validation),
        Err(e) => {
            let mut s = String::from("{\"error\":\"");
            s.push_str(&e.to_string());
            s.push_str("\"}");
            s
        }
    }
}

fn lowered_json(resource: &str, validation: &fhir_pods_core::Validation) -> String {
    let mut s = String::with_capacity(resource.len() + 256);
    s.push_str("{\"resource\":");
    s.push_str(resource);
    s.push_str(",\"validation\":");
    s.push_str(&wire::report(validation));
    s.push('}');
    s
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
            with_providers(Some(&host), f)
        }
        None => with_providers(None, f),
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
                fhir_pods_host_tx(
                    value_set.as_ptr(),
                    value_set.len(),
                    system.as_ptr(),
                    system.len(),
                    code.as_ptr(),
                    code.len(),
                )
            })
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn fhir_pods_validate_host_tx(ptr: *const u8, len: usize) -> *mut u8 {
        with_providers(Some(&ImportTx), |tx| {
            pack(wire::report(&validator().validate_with(input(ptr, len), tx)).as_bytes())
        })
    }

    #[no_mangle]
    pub unsafe extern "C" fn fhir_pods_operation_outcome_host_tx(ptr: *const u8, len: usize) -> *mut u8 {
        with_providers(Some(&ImportTx), |tx| {
            pack(validator().validate_with(input(ptr, len), tx).to_operation_outcome().as_bytes())
        })
    }

    #[no_mangle]
    pub unsafe extern "C" fn fhir_pods_lower_host_tx(
        profile: *const u8,
        profile_len: usize,
        inputs: *const u8,
        inputs_len: usize,
    ) -> *mut u8 {
        with_providers(Some(&ImportTx), |tx| {
            pack(lower_json(input(profile, profile_len), input(inputs, inputs_len), tx).as_bytes())
        })
    }

    #[no_mangle]
    pub unsafe extern "C" fn fhir_pods_v2_host_tx(
        message: *const u8,
        message_len: usize,
        options: *const u8,
        options_len: usize,
    ) -> *mut u8 {
        with_providers(Some(&ImportTx), |tx| {
            pack(v2_json(input(message, message_len), input(options, options_len), tx).as_bytes())
        })
    }

    #[no_mangle]
    pub unsafe extern "C" fn fhir_pods_lift_host_tx(
        profile: *const u8,
        profile_len: usize,
        resource: *const u8,
        resource_len: usize,
    ) -> *mut u8 {
        with_providers(Some(&ImportTx), |tx| {
            pack(lift_json(input(profile, profile_len), input(resource, resource_len), tx).as_bytes())
        })
    }

    #[no_mangle]
    pub unsafe extern "C" fn fhir_pods_document_host_tx(
        kind: *const u8,
        kind_len: usize,
        inputs: *const u8,
        inputs_len: usize,
    ) -> *mut u8 {
        with_providers(Some(&ImportTx), |tx| {
            pack(document_json(input(kind, kind_len), input(inputs, inputs_len), tx).as_bytes())
        })
    }

    #[no_mangle]
    pub unsafe extern "C" fn fhir_pods_audit_host_tx(
        kind: *const u8,
        kind_len: usize,
        inputs: *const u8,
        inputs_len: usize,
    ) -> *mut u8 {
        with_providers(Some(&ImportTx), |tx| {
            pack(audit_json(input(kind, kind_len), input(inputs, inputs_len), tx).as_bytes())
        })
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
            let (Ok(vs), Ok(sys), Ok(cd)) = (env.new_string(value_set), env.new_string(system), env.new_string(code))
            else {
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

    /// The providers for a call: bundled, the loaded table, then the Java
    /// provider when one was passed.
    fn with_java<'l, R>(
        env: &mut JNIEnv<'l>,
        provider: &JObject<'l>,
        f: impl FnOnce(&dyn TerminologyProvider) -> R,
    ) -> R {
        if provider.is_null() {
            with_providers(None, f)
        } else {
            let host = JavaTx { env: RefCell::new(env), provider };
            with_providers(Some(&host), f)
        }
    }

    fn run<'l>(env: &mut JNIEnv<'l>, json: &JByteArray<'l>, provider: &JObject<'l>, outcome: bool) -> jbyteArray {
        let input = bytes_in(env, json);
        let doc = with_java(env, provider, |tx| {
            let v = validator().validate_with(&input, tx);
            if outcome {
                v.to_operation_outcome()
            } else {
                wire::report(&v)
            }
        });
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
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_lower<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        profile: JByteArray<'l>,
        inputs: JByteArray<'l>,
        provider: JObject<'l>,
    ) -> jbyteArray {
        let (p, i) = (bytes_in(&mut env, &profile), bytes_in(&mut env, &inputs));
        let doc = with_java(&mut env, &provider, |tx| lower_json(&p, &i, tx));
        bytes_out(&mut env, &doc)
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_deidentify<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        resource: JByteArray<'l>,
        policy: JByteArray<'l>,
    ) -> jbyteArray {
        let (r, p) = (bytes_in(&mut env, &resource), bytes_in(&mut env, &policy));
        let doc = deidentify_json(&r, &p);
        bytes_out(&mut env, &doc)
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_document<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        kind: JByteArray<'l>,
        inputs: JByteArray<'l>,
        provider: JObject<'l>,
    ) -> jbyteArray {
        let (k, i) = (bytes_in(&mut env, &kind), bytes_in(&mut env, &inputs));
        let doc = with_java(&mut env, &provider, |tx| document_json(&k, &i, tx));
        bytes_out(&mut env, &doc)
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_v2<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        message: JByteArray<'l>,
        options: JByteArray<'l>,
        provider: JObject<'l>,
    ) -> jbyteArray {
        let (m, o) = (bytes_in(&mut env, &message), bytes_in(&mut env, &options));
        let doc = with_java(&mut env, &provider, |tx| v2_json(&m, &o, tx));
        bytes_out(&mut env, &doc)
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_lift<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        profile: JByteArray<'l>,
        resource: JByteArray<'l>,
        provider: JObject<'l>,
    ) -> jbyteArray {
        let (p, r) = (bytes_in(&mut env, &profile), bytes_in(&mut env, &resource));
        let doc = with_java(&mut env, &provider, |tx| lift_json(&p, &r, tx));
        bytes_out(&mut env, &doc)
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_audit<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        kind: JByteArray<'l>,
        inputs: JByteArray<'l>,
        provider: JObject<'l>,
    ) -> jbyteArray {
        let (k, i) = (bytes_in(&mut env, &kind), bytes_in(&mut env, &inputs));
        let doc = with_java(&mut env, &provider, |tx| audit_json(&k, &i, tx));
        bytes_out(&mut env, &doc)
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_terminologyLoad<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        table: JByteArray<'l>,
    ) -> jint {
        let t = bytes_in(&mut env, &table);
        unsafe { fhir_pods_terminology_load(t.as_ptr(), t.len()) as jint }
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_terminologyNeeds<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        json: JByteArray<'l>,
    ) -> jbyteArray {
        let j = bytes_in(&mut env, &json);
        let doc = unsafe { unpack_owned(fhir_pods_terminology_needs(j.as_ptr(), j.len())) };
        bytes_out(&mut env, &doc)
    }

    #[no_mangle]
    pub extern "system" fn Java_expo_modules_fhirpods_FhirPodsNative_slots<'l>(
        mut env: JNIEnv<'l>,
        _this: JObject<'l>,
        profile: JByteArray<'l>,
    ) -> jbyteArray {
        let p = bytes_in(&mut env, &profile);
        let doc = unsafe { unpack_owned(fhir_pods_slots(p.as_ptr(), p.len())) };
        bytes_out(&mut env, &doc)
    }

    /// Take a length-prefixed buffer back from the C ABI as a String.
    unsafe fn unpack_owned(ret: *mut u8) -> String {
        let len = u32::from_le_bytes(*(ret as *const [u8; 4])) as usize;
        let s = String::from_utf8_lossy(std::slice::from_raw_parts(ret.add(4), len)).into_owned();
        pod_free(ret, 4 + len);
        s
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

    /// The loaded terminology table is process-wide state; tests that load
    /// one and tests that expect none run one at a time.
    static TABLE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
            let r = call(
                fhir_pods_validate,
                r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"active":true}"#,
            );
            assert!(r.contains("\"valid\":false"), "{r}");
            assert!(r.contains("Patient.identifier is required"), "{r}");
            let o = call(
                fhir_pods_operation_outcome,
                r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"active":true}"#,
            );
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
            let r = call(
                fhir_pods_must_support,
                r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org/fhir/us/core/StructureDefinition/us-core-patient"]},"identifier":[{"system":"http://h.example.org/mrn","value":"1"}],"name":[{"family":"D"}],"gender":"male"}"#,
            );
            assert!(r.starts_with("{\"mustSupportGaps\":[") && r.contains("\"Patient.birthDate\""), "{r}");
            assert_eq!(call(fhir_pods_must_support, "not json"), "{\"mustSupportGaps\":[]}");
        }
    }

    #[test]
    fn lower_and_slots_over_the_abi() {
        unsafe {
            let hr = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";
            let inputs = r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-16","valueQuantity.value":72,"valueQuantity.unit":"/min"}"#;
            let out = unpack(fhir_pods_lower(hr.as_ptr(), hr.len(), inputs.as_ptr(), inputs.len()));
            assert!(
                out.starts_with("{\"resource\":{\"resourceType\":\"Observation\"") && out.contains("\"valid\":true"),
                "{out}"
            );
            let slots = unpack(fhir_pods_slots(hr.as_ptr(), hr.len()));
            assert!(slots.contains("\"path\":\"valueQuantity.value\",\"type\":\"decimal\""), "{slots}");
            let nope = "http://example.org/nope";
            assert!(
                unpack(fhir_pods_lower(nope.as_ptr(), nope.len(), inputs.as_ptr(), inputs.len())).contains("\"error\"")
            );
        }
    }

    /// A loaded table answers after the bundled sets; `needs` lists what
    /// neither could decide; an unparseable table is refused and the old
    /// one kept.
    #[test]
    fn compiled_table_and_needs_over_the_abi() {
        let _serial = TABLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // date-accuracy-indicator is bound to an NCTS-hosted set the core does not bundle
        let json = r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"identifier":[{"system":"http://h.example.org/mrn","value":"1"}],"name":[{"family":"D"}],"gender":"male","birthDate":"1970-01-01","_birthDate":{"extension":[{"url":"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator","valueCoding":{"system":"https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1","code":"AAA"}}]}}"#;
        unsafe {
            let needs = call(fhir_pods_terminology_needs, json);
            assert!(
                needs.contains(
                    "\"valueSet\":\"https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1\""
                ) && needs.contains("\"code\":\"AAA\""),
                "{needs}"
            );
            let r = call(fhir_pods_validate, json);
            assert!(r.contains("no bundled terminology for https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1"), "{r}");

            let table = "fhir-pods-terminology 1\n= https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1\nhttps://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\tAAA\tAccurate\n";
            assert_eq!(fhir_pods_terminology_load(table.as_ptr(), table.len()), 1);
            let r = call(fhir_pods_validate, json);
            assert!(!r.contains("date-accuracy-indicator-1"), "{r}");
            assert_eq!(call(fhir_pods_terminology_needs, json), "{\"needs\":[]}");
            // a code outside a loaded (complete) set is an error
            let bad = json.replace("\"AAA\"", "\"ZZZ\"");
            let r = call(fhir_pods_validate, &bad);
            assert!(r.contains("'ZZZ' is not in the bound value set"), "{r}");
            // garbage is refused, the table stays
            assert_eq!(fhir_pods_terminology_load(b"nope".as_ptr(), 4), 0);
            assert!(!call(fhir_pods_validate, json).contains("date-accuracy-indicator-1"));
            // cleared
            assert_eq!(fhir_pods_terminology_load(std::ptr::null(), 0), 1);
            assert!(call(fhir_pods_validate, json).contains("no bundled terminology"));
        }
    }

    #[test]
    fn deidentify_over_the_abi() {
        unsafe {
            let json = r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"identifier":[{"system":"http://h.example.org/mrn","value":"1"}],"name":[{"family":"Doe"}],"gender":"male","birthDate":"1970-01-01"}"#;
            let policy = r#"{"salt":"s","today":"2026-09-16"}"#;
            let out = unpack(fhir_pods_deidentify(json.as_ptr(), json.len(), policy.as_ptr(), policy.len()));
            assert!(out.starts_with("{\"resource\":{\"resourceType\":\"Patient\"") && !out.contains("Doe"), "{out}");
            assert!(
                out.contains("\"applied\":[") && out.contains("\"path\":\"Patient.name\",\"action\":\"mask\""),
                "{out}"
            );
            assert!(out.contains("\"remaining\":[]") && out.contains("\"valid\":true"), "{out}");
            let bad = unpack(fhir_pods_deidentify(json.as_ptr(), json.len(), b"{}".as_ptr(), 2));
            assert_eq!(bad, "{\"error\":\"policy needs a non-empty salt\"}");
            let nope = unpack(fhir_pods_deidentify(b"[]".as_ptr(), 2, policy.as_ptr(), policy.len()));
            assert!(nope.contains("\"error\""), "{nope}");
        }
    }

    #[test]
    fn document_over_the_abi() {
        unsafe {
            let inputs = r#"{"resources":[{"resourceType":"Patient","id":"p1","identifier":[{"system":"http://h.example.org/mrn","value":"1"}],"name":[{"family":"Doe"}],"gender":"female","birthDate":"1970"}],"author":"Practitioner/x","date":"2026-09-16T10:00:00+10:00"}"#;
            let kind = "ips";
            let out = unpack(fhir_pods_document(kind.as_ptr(), kind.len(), inputs.as_ptr(), inputs.len()));
            assert!(
                out.starts_with("{\"document\":{\"resourceType\":\"Bundle\"")
                    && out.contains("\"placed\":[")
                    && out.contains("\"validation\":"),
                "{out}"
            );
            let bad = "cda";
            let err = unpack(fhir_pods_document(bad.as_ptr(), bad.len(), inputs.as_ptr(), inputs.len()));
            assert_eq!(err, "{\"error\":\"kind must be ips or au-ps\"}");
        }
    }

    /// F021: a host callback that installs a table while being asked must
    /// not deadlock (the table lock is never held across host code); the
    /// table answers from the next call on.
    #[test]
    fn a_callback_may_load_a_table_while_being_asked() {
        let _serial = TABLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe extern "C" fn loading_cb(
            ctx: *mut c_void,
            _vs: *const u8,
            _vs_len: usize,
            _sys: *const u8,
            _sys_len: usize,
            _code: *const u8,
            _code_len: usize,
        ) -> i32 {
            let calls = &mut *(ctx as *mut u32);
            *calls += 1;
            let table = "fhir-pods-terminology 1\n= https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1\nhttps://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\tAAA\tAccurate\n";
            assert_eq!(fhir_pods_terminology_load(table.as_ptr(), table.len()), 1);
            2 // unknown: the table, from now on, knows
        }
        let json = r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"identifier":[{"system":"http://h.example.org/mrn","value":"1"}],"name":[{"family":"D"}],"gender":"male","birthDate":"1970-01-01","_birthDate":{"extension":[{"url":"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator","valueCoding":{"system":"https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1","code":"AAA"}}]}}"#;
        let mut calls = 0u32;
        unsafe {
            fhir_pods_terminology_load(std::ptr::null(), 0);
            let first = unpack(fhir_pods_validate_tx(
                json.as_ptr(),
                json.len(),
                Some(loading_cb),
                &mut calls as *mut u32 as *mut c_void,
            ));
            assert_eq!(calls, 1, "asked once, and the call returned");
            assert!(first.contains("date-accuracy-indicator"), "the answer during the load was unknown: {first}");
            let second = unpack(fhir_pods_validate_tx(
                json.as_ptr(),
                json.len(),
                Some(loading_cb),
                &mut calls as *mut u32 as *mut c_void,
            ));
            assert_eq!(calls, 1, "the loaded table answers now; the host is not asked");
            assert!(!second.contains("date-accuracy-indicator") && second.contains("\"valid\":true"), "{second}");
            fhir_pods_terminology_load(std::ptr::null(), 0);
        }
    }

    #[test]
    fn lift_round_trips() {
        unsafe {
            let url = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";
            let inputs = r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-16T08:00:00+10:00","valueQuantity.value":72,"valueQuantity.unit":"beats/minute"}"#;
            let lowered = unpack(fhir_pods_lower(url.as_ptr(), url.len(), inputs.as_ptr(), inputs.len()));
            let resource = lowered.split(",\"validation\":").next().unwrap().trim_start_matches("{\"resource\":");
            let out = unpack(fhir_pods_lift(url.as_ptr(), url.len(), resource.as_ptr(), resource.len()));
            assert!(
                out.starts_with("{\"inputs\":{")
                    && out.contains("\"valueQuantity.value\":72")
                    && out.contains("\"missing\":[],\"extras\":[]}"),
                "{out}"
            );
            let err = unpack(fhir_pods_lift(url.as_ptr(), url.len(), b"[]".as_ptr(), 2));
            assert_eq!(err, "{\"error\":\"input is not a FHIR resource\"}");
        }
    }

    #[test]
    fn v2_round_trips() {
        unsafe {
            let msg = "MSH|^~\\&|LAB|ACME|EHR|CLINIC|20260916100000+1000||ORU^R01^ORU_R01|MSG0001|P|2.5.1\rPID|1||MRN12345^^^ACME^MR||Doe^Jane||19700615|F\rOBR|1|PLC001^EHR|FIL001^LAB|24331-1^Lipid panel^LN|||20260916090000+1000||||||||||||||||||F\rOBX|1|NM|2093-3^Cholesterol^LN||5.2|mmol/L^^UCUM|||||F|||20260916093000+1000\rZZZ|1\r";
            let out = unpack(fhir_pods_v2(msg.as_ptr(), msg.len(), b"{}".as_ptr(), 2));
            assert!(
                out.starts_with("{\"bundle\":{\"resourceType\":\"Bundle\",")
                    && out.contains("\"valueQuantity\":{\"value\":5.2,\"code\":\"mmol/L\",\"unit\":\"mmol/L\",\"system\":\"http://unitsofmeasure.org\"}")
                    && out.contains(",\"validation\":{")
                    && out.contains("\"skipped\":[{\"at\":\"")
                    && out.ends_with("\"unplaced\":[\"ZZZ[1]\"]}"),
                "{out}"
            );
            let no_zone = msg.replace("20260916093000+1000", "20260916093000");
            let opts = "{\"timezone\":\"+10:00\"}";
            let out = unpack(fhir_pods_v2(no_zone.as_ptr(), no_zone.len(), opts.as_ptr(), opts.len()));
            assert!(out.contains("\"effectiveDateTime\":\"2026-09-16T09:30:00+10:00\""), "{out}");
            let err = unpack(fhir_pods_v2(b"nope".as_ptr(), 4, b"{}".as_ptr(), 2));
            assert_eq!(err, "{\"error\":\"message must start with MSH and its separators\"}");
            let err = unpack(fhir_pods_v2(msg.as_ptr(), msg.len(), b"[]".as_ptr(), 2));
            assert!(err.starts_with("{\"error\":\"options must be a JSON object"), "{err}");
        }
    }

    #[test]
    fn audit_round_trips() {
        unsafe {
            let inputs = r#"{"recorded":"2026-09-16T10:00:00Z","client":{"who":"Device/app-7","network":"10.0.0.7"},"server":{"who":{"display":"fhir.example.org"},"network":"https://fhir.example.org/r4"},"source":{"observer":"Device/audit"},"patient":"Patient/p1","data":"Observation/hr1"}"#;
            let kind = "patient-read";
            let out = unpack(fhir_pods_audit(kind.as_ptr(), kind.len(), inputs.as_ptr(), inputs.len()));
            assert!(
                out.starts_with("{\"resource\":{\"resourceType\":\"AuditEvent\"")
                    && out.contains("IHE.BasicAudit.PatientRead")
                    && out.contains("\"valid\":true"),
                "{out}"
            );
            let bad = "browse";
            let err = unpack(fhir_pods_audit(bad.as_ptr(), bad.len(), inputs.as_ptr(), inputs.len()));
            assert!(err.starts_with("{\"error\":\"kind must be one of read"), "{err}");
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
        let _serial = TABLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        struct Log(Vec<String>, i32);
        unsafe extern "C" fn cb(
            ctx: *mut c_void,
            vs: *const u8,
            vsl: usize,
            sys: *const u8,
            sysl: usize,
            code: *const u8,
            cl: usize,
        ) -> i32 {
            let log = &mut *(ctx as *mut Log);
            let s = |p: *const u8, n: usize| String::from_utf8_lossy(std::slice::from_raw_parts(p, n)).into_owned();
            log.0.push(format!("{} {} {}", s(vs, vsl), s(sys, sysl), s(code, cl)));
            log.1
        }
        // telecom.system is bundled and never asked about; the date-accuracy
        // coding is bound to an NCTS-hosted set the core leaves to the seam
        let json = r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"identifier":[{"system":"http://h.example.org/mrn","value":"1"}],"name":[{"family":"D"}],"gender":"male","birthDate":"1970-01-01","_birthDate":{"extension":[{"url":"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator","valueCoding":{"system":"https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1","code":"AAA"}}]},"telecom":[{"system":"phone","value":"1"}]}"#;
        unsafe {
            let mut log = Log(Vec::new(), 0);
            let r =
                unpack(fhir_pods_validate_tx(json.as_ptr(), json.len(), Some(cb), &mut log as *mut Log as *mut c_void));
            assert_eq!(log.0, vec!["https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1 https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1 AAA"]);
            assert!(r.contains("'AAA' is not in the bound value set"), "{r}");

            let mut log = Log(Vec::new(), 1);
            let r =
                unpack(fhir_pods_validate_tx(json.as_ptr(), json.len(), Some(cb), &mut log as *mut Log as *mut c_void));
            assert!(!r.contains("date-accuracy-indicator"), "{r}");

            let r = unpack(fhir_pods_validate_tx(json.as_ptr(), json.len(), None, std::ptr::null_mut()));
            assert!(r.contains("no bundled terminology for https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1"), "{r}");
        }
    }
}
