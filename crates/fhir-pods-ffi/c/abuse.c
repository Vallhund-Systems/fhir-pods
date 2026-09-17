/* Memory-valid adversarial inputs for every native entrypoint in fhir_pods.h.
 * No dangling pointers, double-free, or allocation overreads: those violate
 * the caller's ownership contract and cannot have a portable no-crash promise.
 * stdout is NDJSON: scripts/abuse-c.sh validates EVERY returned JSON document.
 */
#include "fhir_pods.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char profile[] = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient";
static const char patient[] = "{\"resourceType\":\"Patient\"}";
static const char policy[] = "{\"salt\":\"synthetic-abi-test\",\"today\":\"2026-09-16\"}";
static const char inputs[] = "{\"identifier\":[{\"value\":\"synthetic\"}],\"name\":[{\"family\":\"Example\"}],\"gender\":\"unknown\",\"birthDate\":\"1980\"}";
static size_t documents = 0;
static size_t callback_calls = 0;
static int32_t callback_answer = -1;

static int32_t callback(void *ctx, const uint8_t *vs, size_t vn,
                        const uint8_t *sys, size_t sn, const uint8_t *code, size_t cn) {
    assert(ctx == &callback_calls);
    assert(vs != NULL && vn > 0 && code != NULL && cn > 0);
    assert(sys != NULL || sn == 0);
    callback_calls++;
    return callback_answer;
}

static uint32_t length_of(const uint8_t *p) {
    assert(p != NULL);
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}

static void inspect(uint8_t *p, const char *required) {
    uint32_t n = length_of(p);
    assert(n > 1 && n < 32 * 1024 * 1024);
    assert(p[4] == '{' && p[4 + n - 1] == '}');
    char *copy = malloc((size_t)n + 1);
    assert(copy != NULL);
    memcpy(copy, p + 4, n);
    copy[n] = 0;
    assert(strlen(copy) == n);
    if (required) assert(strstr(copy, required) != NULL);
    assert(fwrite(copy, 1, n, stdout) == n);
    putchar('\n');
    free(copy);
    documents++;
}

static void consume(uint8_t *p, const char *required) {
    inspect(p, required);
    pod_free(p, (size_t)length_of(p) + 4);
}

static void exercise(const char *label, const uint8_t *p, size_t n, int invalid_json) {
    fprintf(stderr, "ABI input: %s (%zu bytes)\n", label, n);
    consume(fhir_pods_validate(p, n), invalid_json ? "\"valid\":false" : "\"valid\":");
    consume(fhir_pods_operation_outcome(p, n), "\"resourceType\":\"OperationOutcome\"");
    consume(fhir_pods_must_support(p, n), "\"mustSupportGaps\":");
    consume(fhir_pods_terminology_needs(p, n), "\"needs\":");
    consume(fhir_pods_slots(p, n), "\"error\":");
    consume(fhir_pods_validate_tx(p, n, NULL, NULL), invalid_json ? "\"valid\":false" : NULL);
    consume(fhir_pods_validate_tx(p, n, callback, &callback_calls), NULL);
    consume(fhir_pods_operation_outcome_tx(p, n, NULL, NULL), "\"resourceType\":\"OperationOutcome\"");
    consume(fhir_pods_operation_outcome_tx(p, n, callback, &callback_calls), NULL);
    consume(fhir_pods_lower(p, n, (const uint8_t *)inputs, sizeof(inputs) - 1), "\"error\":");
    consume(fhir_pods_lower((const uint8_t *)profile, sizeof(profile) - 1, p, n), NULL);
    consume(fhir_pods_lower_tx(p, n, (const uint8_t *)inputs, sizeof(inputs) - 1, NULL, NULL), "\"error\":");
    consume(fhir_pods_lower_tx((const uint8_t *)profile, sizeof(profile) - 1, p, n, callback, &callback_calls), NULL);
    consume(fhir_pods_deidentify(p, n, (const uint8_t *)policy, sizeof(policy) - 1), NULL);
    consume(fhir_pods_deidentify((const uint8_t *)patient, sizeof(patient) - 1, p, n), "\"error\":");
    consume(fhir_pods_document(p, n, (const uint8_t *)"{}", 2), "\"error\":");
    consume(fhir_pods_document((const uint8_t *)"ips", 3, p, n), "\"error\":");
    int32_t loaded = fhir_pods_terminology_load(p, n);
    assert(loaded == 0 || loaded == 1);
    assert(fhir_pods_terminology_load(NULL, 0) == 1);
}

int main(void) {
    assert(fhir_pods_abi_version() == FHIR_PODS_ABI_VERSION);
    /* The Rust shim intentionally treats null as empty, even with nonzero len. */
    exercise("null-empty", NULL, 0, 1);
    exercise("null-nonzero", NULL, 37, 1);
    pod_free(NULL, 0);
    pod_free(NULL, 37);
    uint8_t *zero = pod_alloc(0);
    exercise("allocated-empty", zero, 0, 1);
    pod_free(zero, 0);

    const uint8_t bad_utf8[] = {0xff, 0xc0, 0x80};
    const uint8_t embedded_bad[] = {'{', '"', 'x', '"', ':', '"', 0xff, '"', '}'};
    exercise("invalid-utf8", bad_utf8, sizeof(bad_utf8), 1);
    exercise("invalid-utf8-in-json-string", embedded_bad, sizeof(embedded_bad), 1);
    exercise("truncated-json", (const uint8_t *)patient, sizeof(patient) - 2, 1);
    exercise("included-NUL-terminator", (const uint8_t *)patient, sizeof(patient), 1);
    exercise("valid-prefix-in-larger-allocation", (const uint8_t *)patient, sizeof(patient) - 1, 0);

    const size_t large_n = 2 * 1024 * 1024;
    uint8_t *large = pod_alloc(large_n);
    memset(large, ' ', large_n);
    memcpy(large, patient, sizeof(patient) - 1);
    exercise("two-MiB-valid-padded-resource", large, large_n, 0);
    /* Large invalid input also exercises error paths without unbounded output. */
    memset(large, 0xff, large_n);
    exercise("two-MiB-invalid-UTF8", large, large_n, 1);
    pod_free(large, large_n);

    uint8_t *deep = pod_alloc(2049);
    memset(deep, '[', 1024); deep[1024] = '0'; memset(deep + 1025, ']', 1024);
    exercise("hostile-nesting", deep, 2049, 1);
    pod_free(deep, 2049);

    /* Output ownership is independent of input lifetime and allocation order. */
    uint8_t *source = pod_alloc(sizeof(patient) - 1);
    memcpy(source, patient, sizeof(patient) - 1);
    uint8_t *live[16];
    for (size_t i = 0; i < 16; i++) live[i] = fhir_pods_validate(source, sizeof(patient) - 1);
    pod_free(source, sizeof(patient) - 1);
    for (size_t i = 0; i < 16; i += 2) consume(live[i], "\"valid\":");
    for (int i = 15; i >= 1; i -= 2) consume(live[i], "\"valid\":");

    /* Exercise a real callback, including values outside {-1,0,1}. */
    const char tx_resource[] = "{\"resourceType\":\"Patient\",\"meta\":{\"profile\":[\"http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient\"]},\"identifier\":[{\"value\":\"synthetic\"}],\"name\":[{\"family\":\"Example\"}],\"gender\":\"unknown\",\"birthDate\":\"1980\",\"_birthDate\":{\"extension\":[{\"url\":\"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator\",\"valueCoding\":{\"system\":\"https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\",\"code\":\"AAA\"}}]}}";
    const int32_t answers[] = {-1, 0, 1, 2, INT32_MIN, INT32_MAX};
    for (size_t i = 0; i < sizeof(answers) / sizeof(answers[0]); i++) {
        callback_answer = answers[i];
        size_t before = callback_calls;
        consume(fhir_pods_validate_tx((const uint8_t *)tx_resource, sizeof(tx_resource) - 1, callback, &callback_calls), NULL);
        assert(callback_calls > before);
        consume(fhir_pods_operation_outcome_tx((const uint8_t *)tx_resource, sizeof(tx_resource) - 1, callback, &callback_calls), NULL);
    }
    /* A rejected terminology load preserves the last usable table. */
    const char table[] = "fhir-pods-terminology 1\n= https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1\nhttps://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\tAAA\tSynthetic\n";
    assert(fhir_pods_terminology_load((const uint8_t *)table, sizeof(table) - 1) == 1);
    assert(fhir_pods_terminology_load(bad_utf8, sizeof(bad_utf8)) == 0);
    consume(fhir_pods_terminology_needs((const uint8_t *)tx_resource, sizeof(tx_resource) - 1), "\"needs\":[]");
    assert(fhir_pods_terminology_load(NULL, 0) == 1);
    fprintf(stderr, "C ABI abuse: OK (%zu JSON documents, %zu callbacks)\n", documents, callback_calls);
    return 0;
}
