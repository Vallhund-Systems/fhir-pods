/* fhir-pods — C ABI. One surface, compiled per target (wasm32 / native staticlib). */
#ifndef FHIR_PODS_H
#define FHIR_PODS_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version; a host refuses a mismatched module. This header is ABI 10. */
#define FHIR_PODS_ABI_VERSION 10u
uint32_t fhir_pods_abi_version(void);

/* Allocate `len` bytes in the module for the host to fill. */
uint8_t *pod_alloc(size_t len);

/* Free a buffer handed to the host; `len` is its full byte length. */
void pod_free(uint8_t *ptr, size_t len);

/* All entrypoints share one contract.
 * Input:  UTF-8 JSON resource at (ptr, len).
 * Output: a freshly allocated buffer laid out as
 *           [uint32 little-endian length][UTF-8 JSON document ...]
 *         Read the length prefix, copy the document, then free the WHOLE buffer
 *         with pod_free(ret, 4 + length). Never NULL: a parse failure is a
 *         `fatal` issue in the document, not an error at the boundary. */

/* The ergonomic report: {"resourceType","profile","valid","issues":[...]}. */
uint8_t *fhir_pods_validate(const uint8_t *ptr, size_t len);

/* The spec resource: a FHIR OperationOutcome (the $validate wire form). */
uint8_t *fhir_pods_operation_outcome(const uint8_t *ptr, size_t len);

/* Must-support: the selected profile's must-support elements the resource
 * leaves empty (where their parent is present), as
 * {"mustSupportGaps":["Patient.telecom", ...]}. Information for a client
 * certifying against the IG — never part of the validation report. */
uint8_t *fhir_pods_must_support(const uint8_t *ptr, size_t len);

/* Lowering: build a conformant resource for the profile (a canonical URL)
 * from its variable inputs (a JSON object keyed by slot path), validated on
 * the way out: {"resource": {...}, "validation": {...}} — or {"error": "..."}
 * when it could not start. `fhir_pods_slots` lists a profile's inputs:
 * {"slots":[{"path","type","choices","min","max","mustSupport"}, ...]}. */
uint8_t *fhir_pods_lower(const uint8_t *profile, size_t profile_len, const uint8_t *inputs, size_t inputs_len);
uint8_t *fhir_pods_slots(const uint8_t *profile, size_t profile_len);

/* The terminology seam. A host callback answering "is (system, code) in this
 * value set?" for the bindings the bundled value sets can't decide (the
 * bundled sets are always consulted first). Strings are UTF-8 (ptr, len),
 * valid only during the call; `system` is empty for a bare `code` element.
 * Return 1 = member, 0 = not a member, anything else = unknown (a warning,
 * never an error). `cb` may be NULL. */
typedef int32_t (*fhir_pods_tx_callback)(void *ctx,
                                         const uint8_t *value_set, size_t value_set_len,
                                         const uint8_t *system, size_t system_len,
                                         const uint8_t *code, size_t code_len);

uint8_t *fhir_pods_validate_tx(const uint8_t *ptr, size_t len, fhir_pods_tx_callback cb, void *ctx);
uint8_t *fhir_pods_operation_outcome_tx(const uint8_t *ptr, size_t len, fhir_pods_tx_callback cb, void *ctx);
uint8_t *fhir_pods_lower_tx(const uint8_t *profile, size_t profile_len, const uint8_t *inputs, size_t inputs_len,
                            fhir_pods_tx_callback cb, void *ctx);

/* Lifting: the reverse — read a resource back into the profile's slots
 * (`profile` a canonical URL, or empty to take the most specific compiled
 * profile the resource declares): {"inputs": {...slot path: value...},
 * "validation": {...}, "missing": [required slots absent], "extras":
 * [element paths no slot covers]} — or {"error": "..."}. A host's map from
 * FHIR to its own model is a map from slot paths to fields. */
uint8_t *fhir_pods_lift(const uint8_t *profile, size_t profile_len, const uint8_t *resource, size_t resource_len);
uint8_t *fhir_pods_lift_tx(const uint8_t *profile, size_t profile_len, const uint8_t *resource, size_t resource_len,
                           fhir_pods_tx_callback cb, void *ctx);

/* HL7 v2: an ER7 message (bytes; MLLP framing is the transport's) converted
 * to a FHIR message Bundle through the hl7.fhir.uv.v2mappings maps and
 * validated on the way out. Options are JSON ({} for none): {"timezone":
 * "+10:00"} — the zone of a v2 time stated without one (else such a time is
 * truncated to its date, and reported) — and {"codeSystems": {"L": "http://
 * example.org/local"}} — coding-system names to uris, the host's names
 * first, then the standard ones the pod knows. Returns {"bundle": {...}, "validation": {...},
 * "skipped": [{"at": "PID[1]-3.4", "target": "...", "why": "..."}] — what the
 * maps state for data the message carried and the pod did not do — and
 * "unplaced": ["ZPI[1]"] — segments the structure has no place for}, or
 * {"error": "..."} when it cannot start. */
uint8_t *fhir_pods_v2(const uint8_t *message, size_t message_len, const uint8_t *options, size_t options_len);
uint8_t *fhir_pods_v2_tx(const uint8_t *message, size_t message_len, const uint8_t *options, size_t options_len,
                         fhir_pods_tx_callback cb, void *ctx);

/* De-identification: a copy of the resource under a policy (JSON:
 * {"salt": "...", "today": "YYYY-MM-DD", "base": "safe-harbor" | "none",
 *  "rules": [{"target": "Patient.address.city", "action": "keep"}, ...]}
 * — Safe Harbor by default; identifiers and references pseudonymised with
 * the salt). Returns {"resource": {...}, "applied": [{"path","action"}],
 * "validation": {...}, "remaining": [{"path","what"}]} — `remaining` is what
 * a Safe Harbor check still finds (empty under the default policy) — or
 * {"error": "..."} when it could not start. */
uint8_t *fhir_pods_deidentify(const uint8_t *resource, size_t resource_len, const uint8_t *policy, size_t policy_len);

/* Documents: an IPS ("ips") or AU Patient Summary ("au-ps") Bundle assembled
 * from a patient and their resources — inputs are a JSON object
 * {"resources": [...], "author": "Practitioner/x", "date": "...", "title"?,
 *  "custodian"?, "identifier"?, "timestamp"?}. Sections come from the kind's
 * Composition profile, each resource is placed by what it is, the narrative is
 * generated, references are rewritten to the entries, and the document is
 * validated against the kind's Bundle profile: {"document": {...}, "placed":
 * [{"fullUrl","section"}], "validation": {...}} — or {"error": "..."}. */
uint8_t *fhir_pods_document(const uint8_t *kind, size_t kind_len, const uint8_t *inputs, size_t inputs_len);
uint8_t *fhir_pods_document_tx(const uint8_t *kind, size_t kind_len, const uint8_t *inputs, size_t inputs_len,
                               fhir_pods_tx_callback cb, void *ctx);

/* Audit: an AuditEvent for a kind of event ("read", "patient-read", "query",
 * "patient-query", "create", "patient-create", "update", "patient-update",
 * "delete", "patient-delete") from host-shaped inputs — a JSON object
 * {"recorded": "<instant>", "client": {"who", "network"}, "server": {...},
 *  "user"?: {"who", "name"?, ...}, "source": {"observer", "site"?},
 *  "data" | "query", "patient"?, "transaction"?} — conformant to the IHE BALP
 * profile for that kind and validated on the way out: {"resource": {...},
 * "validation": {...}} — or {"error": "..."} when it could not start. The
 * `_tx` twin takes the terminology callback. */
uint8_t *fhir_pods_audit(const uint8_t *kind, size_t kind_len, const uint8_t *inputs, size_t inputs_len);
uint8_t *fhir_pods_audit_tx(const uint8_t *kind, size_t kind_len, const uint8_t *inputs, size_t inputs_len,
                            fhir_pods_tx_callback cb, void *ctx);

/* Compiled terminology. Load a table (the line format tools/compile_valueset.py
 * writes from $expand responses): it answers after the bundled sets and before
 * any callback, for every call that follows, for the module's lifetime. A set
 * in the table is complete — a code not listed is not a member. Returns 1 when
 * loaded, 0 when the text could not be parsed (the previous table stays);
 * empty input clears it. */
int32_t fhir_pods_terminology_load(const uint8_t *ptr, size_t len);

/* What a host must resolve for this resource to validate without terminology
 * warnings — the bindings neither the bundled sets nor the loaded table could
 * decide: {"needs":[{"valueSet","system","code"}, ...]}. Same output contract
 * as the validators. */
uint8_t *fhir_pods_terminology_needs(const uint8_t *ptr, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* FHIR_PODS_H */
