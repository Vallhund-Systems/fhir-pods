/* Native C smoke test for the ABI: link against libfhir_pods_ffi.a and call it
 * exactly as the Swift / Kotlin wrappers do. Run via scripts/smoke-c.sh. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "fhir_pods.h"

static char *call(uint8_t *(*f)(const uint8_t *, size_t), const char *json) {
    size_t n = strlen(json);
    uint8_t *in = pod_alloc(n);
    memcpy(in, json, n);
    uint8_t *ret = f(in, n);
    pod_free(in, n);
    uint32_t len = (uint32_t)ret[0] | (uint32_t)ret[1] << 8 | (uint32_t)ret[2] << 16 | (uint32_t)ret[3] << 24;
    char *out = malloc(len + 1);
    memcpy(out, ret + 4, len);
    out[len] = 0;
    pod_free(ret, 4 + len);
    return out;
}

int main(void) {
    if (fhir_pods_abi_version() != FHIR_PODS_ABI_VERSION) {
        fprintf(stderr, "ABI mismatch: %u\n", fhir_pods_abi_version());
        return 2;
    }
    const char *bad = "{\"resourceType\":\"Patient\",\"active\":true}";
    char *report = call(fhir_pods_validate, bad);
    char *outcome = call(fhir_pods_operation_outcome, bad);
    int ok = strstr(report, "\"valid\":false") && strstr(report, "Patient.identifier is required")
          && strstr(outcome, "\"resourceType\":\"OperationOutcome\"");
    printf("report:  %s\noutcome: %s\n", report, outcome);
    free(report);
    free(outcome);
    char *garbage = call(fhir_pods_validate, "not json");
    ok = ok && strstr(garbage, "\"severity\":\"fatal\"");
    free(garbage);
    puts(ok ? "C ABI smoke: OK" : "C ABI smoke: FAILED");
    return ok ? 0 : 1;
}
