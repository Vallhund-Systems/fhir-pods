/* fhir-pods — C ABI. One surface, compiled per target (wasm32 / native staticlib). */
#ifndef FHIR_PODS_H
#define FHIR_PODS_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version; a host refuses a mismatched module. This header is ABI 1. */
#define FHIR_PODS_ABI_VERSION 1u
uint32_t fhir_pods_abi_version(void);

/* Allocate `len` bytes in the module for the host to fill. */
uint8_t *pod_alloc(size_t len);

/* Free a buffer handed to the host; `len` is its full byte length. */
void pod_free(uint8_t *ptr, size_t len);

/* Both entrypoints share one contract.
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

#ifdef __cplusplus
}
#endif

#endif /* FHIR_PODS_H */
