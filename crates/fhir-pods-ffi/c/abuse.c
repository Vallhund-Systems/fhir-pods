/* Bounded ABI 13 functional/input checks. All pointers and lengths describe
 * live allocations (or the shim's documented NULL-as-empty input).
 * Returned buffers are parsed independently by scripts/abuse-c.sh.
 * wasm-only _host_tx functions are exercised by the browser suite.
 */
#include "fhir_pods.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(condition) do { if (!(condition)) { \
    fprintf(stderr, "check failed at %s:%d: %s\n", __FILE__, __LINE__, #condition); exit(1); \
} } while (0)
#define BYTES(s) (const uint8_t *)(s), sizeof(s) - 1
#define RESULT(fn, required, ...) consume(#fn, fn(__VA_ARGS__), required)

static const char profile[] = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient";
static const char patient[] = "{\"resourceType\":\"Patient\"}";
static const char policy[] = "{\"salt\":\"synthetic-abi-test\",\"today\":\"2026-09-22\"}";
static const char inputs[] = "{\"id\":\"synthetic\",\"identifier\":[{\"system\":\"https://example.org/mrn\",\"value\":\"synthetic\"}],\"name\":[{\"family\":\"Example\"}],\"gender\":\"unknown\",\"birthDate\":\"1980\"}";
static const char tx_resource[] = "{\"resourceType\":\"Patient\",\"id\":\"synthetic\",\"meta\":{\"profile\":[\"http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient\"]},\"identifier\":[{\"system\":\"https://example.org/mrn\",\"value\":\"synthetic\"}],\"name\":[{\"family\":\"Example\"}],\"gender\":\"unknown\",\"birthDate\":\"1980\",\"_birthDate\":{\"extension\":[{\"url\":\"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator\",\"valueCoding\":{\"system\":\"https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\",\"code\":\"AAA\"}}]}}";
static const char table[] = "fhir-pods-terminology 1\n= https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1\nhttps://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\tAAA\tSynthetic\n";
static const char v2_message[] = "MSH|^~\\&|LAB|SITE|APP|SITE|20260922100000+1000||ORU^R01|SYNTHETIC|P|2.5\rPID|1||synthetic^^^LOCAL^MR||Example^Synthetic||19800101|U\rOBR|1|||8867-4^Synthetic^LN\rOBX|1|NM|8867-4^Synthetic^LN||72|/min^Synthetic^UCUM|||||F\r";
static const char audit_inputs[] = "{\"recorded\":\"2026-09-22T00:00:00Z\",\"client\":{\"who\":\"Device/app\",\"network\":\"10.0.0.7\"},\"server\":{\"who\":\"Device/server\",\"network\":\"https://example.org/r4\"},\"source\":{\"observer\":\"Device/audit\"},\"data\":\"Patient/synthetic\"}";
static const char questionnaire_profile[] = "http://hl7.org/fhir/StructureDefinition/QuestionnaireResponse";
static const char questionnaire[] = "{\"resourceType\":\"Questionnaire\",\"url\":\"http://forms.example.net/Questionnaire/abi-intake\",\"version\":\"1\",\"status\":\"active\",\"item\":[{\"linkId\":\"age\",\"type\":\"integer\",\"required\":true}]}";
static const char questionnaire_response[] = "{\"resourceType\":\"QuestionnaireResponse\",\"questionnaire\":\"http://forms.example.net/Questionnaire/abi-intake|1\",\"status\":\"completed\",\"item\":[{\"linkId\":\"age\",\"answer\":[{\"valueString\":\"forty-two\"}]}]}";
static const char questionnaire_inputs[] = "{\"questionnaire\":\"http://forms.example.net/Questionnaire/abi-intake|1\",\"status\":\"completed\",\"item\":[{\"linkId\":\"age\",\"answer\":[{\"valueString\":\"forty-two\"}]}]}";
static size_t documents, callback_calls, reentrant_calls;
static int32_t callback_answer = -1;

static uint32_t length_of(const uint8_t *p) {
    CHECK(p != NULL);
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}
static void inspect(const char *entry, uint8_t *p, const char *required) {
    uint32_t n = length_of(p);
    CHECK(n > 1 && n < 32 * 1024 * 1024);
    CHECK(p[4] == '{' && p[4 + n - 1] == '}');
    char *copy = malloc((size_t)n + 1);
    CHECK(copy != NULL);
    memcpy(copy, p + 4, n); copy[n] = 0;
    CHECK(strlen(copy) == n);
    if (required && strstr(copy, required) == NULL) {
        fprintf(stderr, "%s: expected %s in %.500s\n", entry, required, copy);
        CHECK(0);
    }
    CHECK(printf("{\"entry\":\"%s\",\"result\":", entry) > 0);
    CHECK(fwrite(copy, 1, n, stdout) == n);
    CHECK(fputs("}\n", stdout) >= 0);
    free(copy); documents++;
}
static void consume(const char *entry, uint8_t *p, const char *required) {
    inspect(entry, p, required);
    pod_free(p, (size_t)length_of(p) + 4);
}
static void scalar(const char *entry, int value) {
    CHECK(printf("{\"entry\":\"%s\",\"result\":{\"value\":%d}}\n", entry, value) > 0);
}
static int32_t callback(void *ctx, const uint8_t *vs, size_t vn,
                        const uint8_t *sys, size_t sn, const uint8_t *code, size_t cn) {
    CHECK(ctx == NULL || ctx == &callback_calls);
    CHECK(vs != NULL && vn > 0 && code != NULL && cn > 0);
    CHECK(sys != NULL || sn == 0);
    callback_calls++;
    return callback_answer;
}
static int32_t reentrant(void *ctx, const uint8_t *vs, size_t vn,
                         const uint8_t *sys, size_t sn, const uint8_t *code, size_t cn) {
    CHECK(ctx == NULL);
    callback(ctx, vs, vn, sys, sn, code, cn);
    reentrant_calls++;
    CHECK(fhir_pods_terminology_load(BYTES(table)) == 1);
    RESULT(fhir_pods_validate, "\"valid\":true", BYTES(patient));
    return 1;
}

static void exercise(const char *label, const uint8_t *p, size_t n, int invalid_json) {
    fprintf(stderr, "ABI input: %s (%zu bytes)\n", label, n);
    RESULT(fhir_pods_validate, invalid_json ? "\"valid\":false" : "\"valid\":", p, n);
    RESULT(fhir_pods_operation_outcome, "\"resourceType\":\"OperationOutcome\"", p, n);
    RESULT(fhir_pods_must_support, "\"mustSupportGaps\":", p, n);
    RESULT(fhir_pods_terminology_needs, "\"needs\":", p, n);
    RESULT(fhir_pods_slots, "\"error\":", p, n);
    RESULT(fhir_pods_validate_tx, invalid_json ? "\"valid\":false" : NULL, p, n, NULL, NULL);
    RESULT(fhir_pods_validate_tx, NULL, p, n, callback, NULL);
    RESULT(fhir_pods_operation_outcome_tx, "\"resourceType\":\"OperationOutcome\"", p, n, NULL, NULL);
    RESULT(fhir_pods_operation_outcome_tx, NULL, p, n, callback, &callback_calls);
    RESULT(fhir_pods_lower, "\"error\":", p, n, BYTES(inputs));
    RESULT(fhir_pods_lower, NULL, BYTES(profile), p, n);
    RESULT(fhir_pods_lower_tx, "\"error\":", p, n, BYTES(inputs), NULL, NULL);
    RESULT(fhir_pods_lower_tx, NULL, BYTES(profile), p, n, callback, &callback_calls);
    RESULT(fhir_pods_lift, "\"error\":", p, n, BYTES(patient));
    RESULT(fhir_pods_lift, NULL, BYTES(profile), p, n);
    RESULT(fhir_pods_lift_tx, NULL, BYTES(profile), p, n, callback, NULL);
    RESULT(fhir_pods_deidentify, NULL, p, n, BYTES(policy));
    RESULT(fhir_pods_deidentify, "\"error\":", BYTES(patient), p, n);
    RESULT(fhir_pods_document, "\"error\":", p, n, BYTES("{}"));
    RESULT(fhir_pods_document, "\"error\":", BYTES("ips"), p, n);
    RESULT(fhir_pods_document_tx, "\"error\":", BYTES("ips"), p, n, callback, NULL);
    RESULT(fhir_pods_audit, "\"error\":", p, n, BYTES(audit_inputs));
    RESULT(fhir_pods_audit, "\"error\":", BYTES("read"), p, n);
    RESULT(fhir_pods_audit_tx, "\"error\":", BYTES("read"), p, n, callback, NULL);
    RESULT(fhir_pods_v2, "\"error\":", p, n, BYTES("{}"));
    RESULT(fhir_pods_v2, NULL, BYTES(v2_message), p, n);
    RESULT(fhir_pods_v2_tx, "\"error\":", p, n, BYTES("{}"), callback, NULL);
    RESULT(fhir_pods_v2_tx, NULL, BYTES(v2_message), p, n, NULL, NULL);
    RESULT(fhir_pods_export, "\"error\":", p, n, BYTES("{}"));
    RESULT(fhir_pods_export, "\"error\":", BYTES("au"), p, n);
    RESULT(fhir_pods_export_tx, "\"error\":", BYTES("au"), p, n, callback, NULL);
    int32_t questionnaire_loaded = fhir_pods_questionnaire_load(p, n);
    CHECK(questionnaire_loaded == 0 || questionnaire_loaded == 1);
    scalar("fhir_pods_questionnaire_load", questionnaire_loaded);
    int32_t loaded = fhir_pods_terminology_load(p, n);
    CHECK(loaded == 0 || loaded == 1);
    scalar("fhir_pods_terminology_load", loaded);
    CHECK(fhir_pods_terminology_load(NULL, 0) == 1);
}

int main(void) {
    CHECK(fhir_pods_abi_version() == FHIR_PODS_ABI_VERSION);
    scalar("fhir_pods_abi_version", (int)fhir_pods_abi_version());
    RESULT(fhir_pods_profiles, "\"regions\":");
    RESULT(fhir_pods_slots, "\"parts\":", BYTES(profile));
    exercise("null-empty", NULL, 0, 1);
    exercise("null-nonzero", NULL, 37, 1);
    pod_free(NULL, 0); pod_free(NULL, 37);
    uint8_t *zero = pod_alloc(0);
    exercise("allocated-empty", zero, 0, 1); pod_free(zero, 0);
    const uint8_t bad_utf8[] = {0xff, 0xc0, 0x80};
    const uint8_t nul_string[] = {'{','"','x','"',':','"','a',0,'b','"','}'};
    exercise("invalid-utf8", bad_utf8, sizeof(bad_utf8), 1);
    exercise("embedded-NUL-string", nul_string, sizeof(nul_string), 1);
    exercise("escaped-NUL-string", BYTES("{\"resourceType\":\"Patient\",\"name\":[{\"text\":\"a\\u0000b\"}]}"), 0);
    exercise("truncated-json", (const uint8_t *)patient, sizeof(patient) - 2, 1);
    exercise("included-NUL-terminator", (const uint8_t *)patient, sizeof(patient), 1);
    exercise("valid-prefix", BYTES(patient), 0);
    const size_t large_n = 3 * 1024 * 1024;
    uint8_t *large = pod_alloc(large_n); CHECK(large != NULL);
    memset(large, ' ', large_n); memcpy(large, patient, sizeof(patient) - 1);
    exercise("three-MiB-padded-resource", large, large_n, 0);
    memset(large, 0xff, large_n);
    exercise("three-MiB-invalid-UTF8", large, large_n, 1); pod_free(large, large_n);
    const size_t depth = 256, deep_n = depth * 6 + 1;
    uint8_t *deep = pod_alloc(deep_n); CHECK(deep != NULL);
    for (size_t i = 0; i < depth; i++) memcpy(deep + i * 5, "{\"x\":", 5);
    deep[depth * 5] = '0'; memset(deep + depth * 5 + 1, '}', depth);
    exercise("deep-object", deep, deep_n, 1); pod_free(deep, deep_n);

    uint8_t *source = pod_alloc(sizeof(patient) - 1); CHECK(source != NULL);
    memcpy(source, patient, sizeof(patient) - 1);
    uint8_t *live[16];
    for (size_t i = 0; i < 16; i++) live[i] = fhir_pods_validate(source, sizeof(patient) - 1);
    pod_free(source, sizeof(patient) - 1);
    for (size_t i = 0; i < 16; i += 2) consume("fhir_pods_validate", live[i], "\"valid\":");
    for (int i = 15; i >= 1; i -= 2) consume("fhir_pods_validate", live[i], "\"valid\":");
    scalar("pod_alloc", 1); scalar("pod_free", 1);

    const int32_t answers[] = {-1, 0, 1, 2, INT32_MIN, INT32_MAX};
    for (size_t i = 0; i < sizeof(answers) / sizeof(answers[0]); i++) {
        callback_answer = answers[i];
        size_t before = callback_calls;
        RESULT(fhir_pods_validate_tx, NULL, BYTES(tx_resource), callback, NULL);
        CHECK(callback_calls > before);
        RESULT(fhir_pods_operation_outcome_tx, NULL, BYTES(tx_resource), callback, &callback_calls);
        RESULT(fhir_pods_lift_tx, "\"conflicts\":", BYTES(profile), BYTES(tx_resource), callback, NULL);
    }
    callback_answer = 1;
    char record[4096], document[4096];
    CHECK(snprintf(record, sizeof(record), "{\"resources\":[%s],\"document\":false}", tx_resource) < (int)sizeof(record));
    CHECK(snprintf(document, sizeof(document), "{\"resources\":[%s],\"author\":\"Patient/synthetic\",\"date\":\"2026-09-22T00:00:00Z\"}", tx_resource) < (int)sizeof(document));
    RESULT(fhir_pods_export, "\"carried\":", BYTES("au"), (const uint8_t *)record, strlen(record));
    RESULT(fhir_pods_export_tx, "\"notes\":", BYTES("au"), (const uint8_t *)record, strlen(record), callback, NULL);
    RESULT(fhir_pods_document_tx, "\"document\":", BYTES("ips"), (const uint8_t *)document, strlen(document), callback, NULL);
    RESULT(fhir_pods_lower_tx, "\"resource\":", BYTES(profile), BYTES(inputs), callback, NULL);
    RESULT(fhir_pods_audit_tx, "\"resource\":", BYTES("read"), BYTES(audit_inputs), callback, NULL);
    RESULT(fhir_pods_v2_tx, "\"bundle\":", BYTES(v2_message), BYTES("{}"), callback, NULL);

    /* A loaded form reaches all ABI operations that validate a response. A
     * rejected replacement leaves the loaded form intact; empty input clears
     * it. The FFI smoke has the same semantics, while this harness adds the
     * allocator/result-buffer and hostile-input boundary. */
    CHECK(fhir_pods_questionnaire_load(NULL, 0) == 1);
    scalar("fhir_pods_questionnaire_load", 1);
    RESULT(fhir_pods_validate, "could not be resolved", BYTES(questionnaire_response));
    CHECK(fhir_pods_questionnaire_load(BYTES(questionnaire)) == 1);
    scalar("fhir_pods_questionnaire_load", 1);
    RESULT(fhir_pods_validate, "Answer value must be of the type integer not string", BYTES(questionnaire_response));
    RESULT(fhir_pods_lower, "Answer value must be of the type integer not string", BYTES(questionnaire_profile), BYTES(questionnaire_inputs));
    RESULT(fhir_pods_lift, "Answer value must be of the type integer not string", BYTES(questionnaire_profile), BYTES(questionnaire_response));
    CHECK(fhir_pods_questionnaire_load(BYTES(patient)) == 0);
    scalar("fhir_pods_questionnaire_load", 0);
    RESULT(fhir_pods_validate, "Answer value must be of the type integer not string", BYTES(questionnaire_response));
    CHECK(fhir_pods_questionnaire_load(NULL, 0) == 1);
    scalar("fhir_pods_questionnaire_load", 1);
    RESULT(fhir_pods_validate, "could not be resolved", BYTES(questionnaire_response));

    /* The callback loads terminology and validates recursively. A stale read
     * lock would deadlock; the runner has a bounded process timeout. */
    CHECK(fhir_pods_terminology_load(NULL, 0) == 1);
    RESULT(fhir_pods_validate_tx, "\"valid\":true", BYTES(tx_resource), reentrant, NULL);
    CHECK(reentrant_calls > 0);
    CHECK(fhir_pods_terminology_load(bad_utf8, sizeof(bad_utf8)) == 0);
    RESULT(fhir_pods_terminology_needs, "\"needs\":[]", BYTES(tx_resource));
    CHECK(fhir_pods_terminology_load(NULL, 0) == 1);
    CHECK(fflush(stdout) == 0);
    fprintf(stderr, "C ABI checks: OK (%zu buffers, %zu callbacks, %zu reentrant callbacks)\n", documents, callback_calls, reentrant_calls);
    return 0;
}
