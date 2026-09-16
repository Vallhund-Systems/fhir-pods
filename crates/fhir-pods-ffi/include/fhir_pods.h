/* fhir-pods — C ABI. One surface, compiled per target (wasm32 / native staticlib). */
#ifndef FHIR_PODS_H
#define FHIR_PODS_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version; a host refuses a mismatched module. This header is ABI 4. */
#define FHIR_PODS_ABI_VERSION 4u
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

#ifdef __cplusplus
}
#endif

#endif /* FHIR_PODS_H */
