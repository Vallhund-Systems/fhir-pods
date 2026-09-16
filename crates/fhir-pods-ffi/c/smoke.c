/* Native C smoke test for the ABI: link against libfhir_pods_ffi.a and call it
 * exactly as the Swift / Kotlin wrappers do. Run via scripts/smoke-c.sh. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "fhir_pods.h"

static char *unpack(uint8_t *ret) {
    uint32_t len = (uint32_t)ret[0] | (uint32_t)ret[1] << 8 | (uint32_t)ret[2] << 16 | (uint32_t)ret[3] << 24;
    char *out = malloc(len + 1);
    memcpy(out, ret + 4, len);
    out[len] = 0;
    pod_free(ret, 4 + len);
    return out;
}

static char *call(uint8_t *(*f)(const uint8_t *, size_t), const char *json) {
    size_t n = strlen(json);
    uint8_t *in = pod_alloc(n);
    memcpy(in, json, n);
    uint8_t *ret = f(in, n);
    pod_free(in, n);
    return unpack(ret);
}

/* A host terminology provider: answers "member" for exactly one code and
 * "not a member" otherwise, counting how often it is asked. */
struct tx { const char *accept; int asked; };

static int32_t tx_cb(void *ctx, const uint8_t *vs, size_t vsl, const uint8_t *sys, size_t sysl,
                     const uint8_t *code, size_t cl) {
    struct tx *t = ctx;
    (void)vs; (void)vsl; (void)sys; (void)sysl;
    t->asked++;
    return strlen(t->accept) == cl && memcmp(t->accept, code, cl) == 0 ? 1 : 0;
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

    /* The terminology seam: telecom.system is bound to a set the core does not
     * bundle, so the host is asked; its answer decides. */
    const char *tel = "{\"resourceType\":\"Patient\",\"identifier\":[{\"system\":\"http://h.example.org/mrn\",\"value\":\"1\"}],"
                      "\"name\":[{\"family\":\"D\"}],\"gender\":\"male\",\"birthDate\":\"1970-01-01\","
                      "\"telecom\":[{\"system\":\"phone\",\"value\":\"1\"}]}";
    struct tx yes = { "phone", 0 }, no = { "fax", 0 };
    char *r_yes = unpack(fhir_pods_validate_tx((const uint8_t *)tel, strlen(tel), tx_cb, &yes));
    char *r_no = unpack(fhir_pods_validate_tx((const uint8_t *)tel, strlen(tel), tx_cb, &no));
    char *r_none = unpack(fhir_pods_validate_tx((const uint8_t *)tel, strlen(tel), NULL, NULL));
    ok = ok && yes.asked == 1 && !strstr(r_yes, "contact-point-system")
            && strstr(r_no, "'phone' is not in the bound value set")
            && strstr(r_none, "no bundled terminology for http://hl7.org/fhir/ValueSet/contact-point-system");
    printf("tx: asked %d; with host=yes: %s\n", yes.asked, r_yes);
    free(r_yes); free(r_no); free(r_none);

    puts(ok ? "C ABI smoke: OK" : "C ABI smoke: FAILED");
    return ok ? 0 : 1;
}
