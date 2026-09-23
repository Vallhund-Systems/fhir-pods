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
    const char *bad = "{\"resourceType\":\"Patient\",\"meta\":{\"profile\":[\"http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient\"]},\"active\":true}";
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

    /* The terminology seam: the date-accuracy coding is bound to an NCTS-hosted
     * set the core does not bundle, so the host is asked; its answer decides.
     * telecom.system is bundled and never asked about. */
    const char *tel = "{\"resourceType\":\"Patient\",\"meta\":{\"profile\":[\"http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient\"]},\"identifier\":[{\"system\":\"http://h.example.org/mrn\",\"value\":\"1\"}],"
                      "\"name\":[{\"family\":\"D\"}],\"gender\":\"male\",\"birthDate\":\"1970-01-01\","
                      "\"_birthDate\":{\"extension\":[{\"url\":\"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator\","
                      "\"valueCoding\":{\"system\":\"https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\",\"code\":\"AAA\"}}]},"
                      "\"telecom\":[{\"system\":\"phone\",\"value\":\"1\"}]}";
    struct tx yes = { "AAA", 0 }, no = { "ZZZ", 0 };
    char *r_yes = unpack(fhir_pods_validate_tx((const uint8_t *)tel, strlen(tel), tx_cb, &yes));
    char *r_no = unpack(fhir_pods_validate_tx((const uint8_t *)tel, strlen(tel), tx_cb, &no));
    char *r_none = unpack(fhir_pods_validate_tx((const uint8_t *)tel, strlen(tel), NULL, NULL));
    ok = ok && yes.asked == 1 && !strstr(r_yes, "date-accuracy-indicator")
            && strstr(r_no, "'AAA' is not in the bound value set")
            && strstr(r_none, "no bundled terminology for https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1");
    printf("tx: asked %d; with host=yes: %s\n", yes.asked, r_yes);
    free(r_yes); free(r_no); free(r_none);

    /* A compiled table answers after the bundled sets; needs lists the rest. */
    char *needs = unpack(fhir_pods_terminology_needs((const uint8_t *)tel, strlen(tel)));
    ok = ok && strstr(needs, "\"code\":\"AAA\"");
    const char *table = "fhir-pods-terminology 1\n= https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1\n"
                        "https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\tAAA\tAccurate\n";
    ok = ok && fhir_pods_terminology_load((const uint8_t *)table, strlen(table)) == 1;
    char *r_table = unpack(fhir_pods_validate((const uint8_t *)tel, strlen(tel)));
    char *needs_after = unpack(fhir_pods_terminology_needs((const uint8_t *)tel, strlen(tel)));
    ok = ok && !strstr(r_table, "date-accuracy-indicator") && strcmp(needs_after, "{\"needs\":[]}") == 0
            && fhir_pods_terminology_load((const uint8_t *)"nope", 4) == 0
            && fhir_pods_terminology_load(NULL, 0) == 1;
    printf("terminology: needs %s; after load: %s\n", needs, r_table);
    free(needs); free(r_table); free(needs_after);

    /* A QuestionnaireResponse against a loaded Questionnaire (ABI 13). */
    const char *q = "{\"resourceType\":\"Questionnaire\",\"url\":\"http://forms.example.net/Questionnaire/q1\",\"status\":\"active\","
                    "\"item\":[{\"linkId\":\"name\",\"type\":\"string\",\"required\":true}]}";
    const char *qr = "{\"resourceType\":\"QuestionnaireResponse\",\"questionnaire\":\"http://forms.example.net/Questionnaire/q1\","
                     "\"status\":\"completed\",\"item\":[{\"linkId\":\"nope\",\"answer\":[{\"valueString\":\"x\"}]}]}";
    char *r_unloaded = unpack(fhir_pods_validate((const uint8_t *)qr, strlen(qr)));
    ok = ok && strstr(r_unloaded, "\"valid\":true") && strstr(r_unloaded, "could not be resolved");
    ok = ok && fhir_pods_questionnaire_load((const uint8_t *)q, strlen(q)) == 1;
    char *r_loaded = unpack(fhir_pods_validate((const uint8_t *)qr, strlen(qr)));
    ok = ok && strstr(r_loaded, "\"valid\":false") && strstr(r_loaded, "LinkId 'nope' not found in questionnaire")
            && strstr(r_loaded, "No response answer found for required item 'name'")
            && fhir_pods_questionnaire_load((const uint8_t *)"{\"resourceType\":\"Patient\"}", 26) == 0
            && fhir_pods_questionnaire_load(NULL, 0) == 1;
    printf("questionnaire: unloaded %.60s...; loaded %.60s...\n", r_unloaded, r_loaded);
    free(r_unloaded); free(r_loaded);

    /* Must-support: information, separate from validation. */
    const char *us = "{\"resourceType\":\"Patient\",\"meta\":{\"profile\":[\"http://hl7.org/fhir/us/core/StructureDefinition/us-core-patient\"]},"
                     "\"identifier\":[{\"system\":\"http://h.example.org/mrn\",\"value\":\"1\"}],\"name\":[{\"family\":\"D\"}],\"gender\":\"male\"}";
    char *ms = call(fhir_pods_must_support, us);
    ok = ok && strstr(ms, "\"mustSupportGaps\":[") && strstr(ms, "\"Patient.birthDate\"");
    printf("must-support: %s\n", ms);
    free(ms);

    /* Lifting: the lowered heart rate read back into its slots. */
    const char *hr_url = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";
    const char *hr_in = "{\"status\":\"final\",\"subject\":\"Patient/1\",\"effectiveDateTime\":\"2026-09-16T08:00:00+10:00\",\"valueQuantity.value\":72,\"valueQuantity.unit\":\"beats/minute\"}";
    char *hr_low = unpack(fhir_pods_lower((const uint8_t *)hr_url, strlen(hr_url), (const uint8_t *)hr_in, strlen(hr_in)));
    char *hr_res = strstr(hr_low, "{\"resource\":") + strlen("{\"resource\":");
    char *hr_end = strstr(hr_res, ",\"validation\":");
    char *lifted = unpack(fhir_pods_lift((const uint8_t *)hr_url, strlen(hr_url), (const uint8_t *)hr_res, (size_t)(hr_end - hr_res)));
    ok = ok && strstr(lifted, "\"valueQuantity.value\":72") && strstr(lifted, "\"missing\":[],\"extras\":[]");
    printf("lift: %.100s...\n", lifted);
    free(hr_low); free(lifted);

    /* HL7 v2: an ORU^R01 as a message Bundle, the Z segment named. */
    const char *v2msg = "MSH|^~\\&|LAB|ACME|EHR|CLINIC|20260916100000+1000||ORU^R01^ORU_R01|MSG0001|P|2.5.1\r"
                        "PID|1||MRN12345^^^ACME^MR||Doe^Jane||19700615|F\r"
                        "OBR|1|PLC001^EHR|FIL001^LAB|24331-1^Lipid panel^LN|||20260916090000+1000||||||||||||||||||F\r"
                        "OBX|1|NM|2093-3^Cholesterol^LN||5.2|mmol/L^^UCUM|||||F|||20260916093000+1000\rZZZ|1\r";
    char *v2 = unpack(fhir_pods_v2((const uint8_t *)v2msg, strlen(v2msg), (const uint8_t *)"{}", 2));
    ok = ok && strstr(v2, "{\"bundle\":{\"resourceType\":\"Bundle\"") && strstr(v2, "\"valueQuantity\":{\"value\":5.2")
         && strstr(v2, "\"unplaced\":[\"ZZZ[1]\"],\"unmapped\":[");
    printf("v2: %.100s...\n", v2);
    free(v2);

    /* Audit: a BALP AuditEvent from host-shaped inputs, validated. */
    const char *ev = "{\"recorded\":\"2026-09-16T10:00:00Z\",\"client\":{\"who\":\"Device/app-7\",\"network\":\"10.0.0.7\"},"
                     "\"server\":{\"who\":{\"display\":\"fhir.example.org\"},\"network\":\"https://fhir.example.org/r4\"},"
                     "\"source\":{\"observer\":\"Device/audit\"},\"patient\":\"Patient/p1\",\"data\":\"Observation/hr1\"}";
    char *audit = unpack(fhir_pods_audit((const uint8_t *)"patient-read", 12, (const uint8_t *)ev, strlen(ev)));
    ok = ok && strstr(audit, "IHE.BasicAudit.PatientRead") && strstr(audit, "\"valid\":true");
    char *audit_err = unpack(fhir_pods_audit((const uint8_t *)"browse", 6, (const uint8_t *)ev, strlen(ev)));
    ok = ok && strstr(audit_err, "{\"error\":\"kind must be one of");
    printf("audit: %.120s...\n", audit);
    free(audit); free(audit_err);

    puts(ok ? "C ABI smoke: OK" : "C ABI smoke: FAILED");
    return ok ? 0 : 1;
}
