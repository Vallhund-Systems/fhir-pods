/* fhir-pods — C ABI. One surface, compiled per target (wasm32 / native staticlib). */
#ifndef FHIR_PODS_H
#define FHIR_PODS_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version; a host may refuse a mismatched module. */
uint32_t fhir_pods_abi_version(void);

/* Allocate `len` bytes in the module for the host to fill. */
uint8_t *pod_alloc(size_t len);

/* Free a buffer handed to the host; `len` is its full byte length. */
void pod_free(uint8_t *ptr, size_t len);

/* Probe a FHIR resource.
 * Input:  UTF-8 JSON at (ptr, len).
 * Output: a freshly allocated buffer laid out as
 *           [uint32 little-endian length][UTF-8 JSON report ...]
 *         Read the length prefix, copy the report, then free the WHOLE buffer
 *         with pod_free(ret, 4 + length). */
uint8_t *fhir_pods_probe(const uint8_t *ptr, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* FHIR_PODS_H */
