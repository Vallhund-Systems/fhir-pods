/* fhir-pods — C ABI. One surface, compiled per target (wasm32 / native staticlib). */
#ifndef FHIR_PODS_H
#define FHIR_PODS_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version; a host refuses a mismatched module. This header is ABI 15.
 * It rises whenever the exported surface changes, a new entrypoint included:
 * a wrapper that calls one must not be handed a module without it. */
#define FHIR_PODS_ABI_VERSION 15u
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

/* Findings, and what `valid` means.
 * Every issue carries a stable rule id (`issues[].id`; `issue.details.coding`
 * under system urn:fhir-pods:rule in an OperationOutcome) and one of four
 * severities (`issues[].severity`) — the ids, their severities and the
 * conditions that produce them are docs/rules.md:
 *   fatal        something could not be checked at all, and the report says
 *                which. Two findings are fatal: the input is not JSON
 *                (`parse.json` — there is no document, so that finding stands
 *                alone) and the Questionnaire a response is held to has an
 *                item with no type (`questionnaire.definition`).
 *   error        the resource does not conform.
 *   warning      a check that could not be performed (no bundled terminology,
 *                an expression outside the supported core), a declared
 *                profile this build has not compiled, a best-practice
 *                invariant.
 *   information  said for the record, never a defect.
 * `"valid": true` means NO error and NO fatal issue. A warning or an
 * information finding never changes it, however much it matters: a resource
 * whose declared profile this build has not compiled is valid by default
 * (the reference validator's own severity) — fhir_pods_options below is how
 * a host makes that an error instead of filtering the issue out. */

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
 * [element paths no slot covers], "conflicts": [{"path","given","fixed"}]} — or {"error": "..."}. A host's map from
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
 * "unplaced": ["ZPI[1]"] — segments the structure has no place for — and
 * "unmapped": ["PID[1]-2"] — valued fields of placed segments that no map
 * row names, so a host can audit what the IG says nothing about}, or
 * {"error": "..."} when it cannot start. */
uint8_t *fhir_pods_v2(const uint8_t *message, size_t message_len, const uint8_t *options, size_t options_len);
uint8_t *fhir_pods_v2_tx(const uint8_t *message, size_t message_len, const uint8_t *options, size_t options_len,
                         fhir_pods_tx_callback cb, void *ctx);

/* De-identification: a copy of the resource under a policy (JSON:
 * {"salt": "...", "today": "YYYY-MM-DD", "base": "safe-harbor" | "none",
 *  "rules": [{"target": "Patient.address.city", "action": "keep"}, ...]}).
 * The policy is a rule set applied to a typed walk of the resource. `base`
 * defaults to the built-in `safe-harbor` rule set — modelled on HIPAA Safe
 * Harbor's identifier list, by FHIR type (names, contact details, addresses
 * below state, dates to the year, ages over 89 on an Age, narrative, notes,
 * attachments, signatures, device serials; identifiers and references
 * pseudonymised with the salt). It is NOT a compliance determination: it
 * covers the fields its rules name, in the resource it is given, and says
 * nothing about re-identification risk from what remains, from the host's
 * other data, or from how the copy is used. `salt` is REQUIRED and
 * non-empty: without one the call returns {"error": ...} rather than
 * unkeyed pseudonyms. Actions: keep, remove, mask, year, pseudonym, clamp,
 * collapse-age — the last being the host's declaration that the element it
 * names IS an age (over 89 aggregates: an integer or decimal to 90, a
 * quantity to {"value": 90, "comparator": ">="} in its own unit), since
 * nothing says which integer is an age and the pod never guesses.
 * Returns {"resource": {...}, "applied": [{"path","action"}],
 * "validation": {...}, "remaining": [{"path","what"}], "residual":
 * [{"path","what"}]} — `remaining` is what the built-in policy's own check
 * still finds (empty under the default policy), `residual` the free text a
 * rule kept and any collapse-age rule that found no age — or
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

/* Export: a record re-profiled for a region ("au", "us", "ips"). Inputs
 * (JSON): {"resources": [{...}, ...], "document"?: bool, "author", "date",
 * "title"?, "custodian"?, "identifier"?, "timestamp"?, "audit"?: {...}} — the
 * resources as the host lowered them (the base R4 definitions, or any);
 * every one goes out through the region's profile for it (its fixed content
 * added, its id kept), the region's patient summary is assembled, and the
 * read is recorded as a BALP patient-read when "audit" is given. Out:
 * {"region": {"name","title","guide"}, "resources": [{"index",
 * "resourceType", "id", "profile", "also", "resource", "validation",
 * "missing", "alsoMissing", "extras", "carried", "note", "notes"}], "document": {"kind",
 * "document", "placed", "omitted", "validation"} | null, "audit":
 * {"resource", "validation"} | null} — or {"error": "..."} when it could
 * not start. The `_tx` twin takes the terminology callback. */
uint8_t *fhir_pods_export(const uint8_t *region, size_t region_len, const uint8_t *inputs, size_t inputs_len);
uint8_t *fhir_pods_export_tx(const uint8_t *region, size_t region_len, const uint8_t *inputs, size_t inputs_len,
                             fhir_pods_tx_callback cb, void *ctx);

/* Discovery: what is compiled in — {"regions": [{"name","title","guide",
 * "document"}], "profiles": [{"url","name","resourceType","kind","base"}]}
 * ("kind": resource | datatype | extension). Compiled host rows may include
 * sourceSha256, the exact input-file fingerprint; missing is not an identity
 * claim. Verify the artifact and compare against the verified package. No input. */
uint8_t *fhir_pods_profiles(void);

/* Compiled terminology. Load a table (the line format tools/compile_valueset.py
 * writes from $expand responses): it answers after the bundled sets and before
 * any callback, for every call that follows, for the module's lifetime. A set
 * in the table is complete — a code not listed is not a member. Returns 1 when
 * loaded, 0 when the text could not be parsed (the previous table stays);
 * empty input clears all tables. Also accepts strict operational JSON
 * fhir-pods.terminology-contexts.v1 with contexts of {terminology, resources:
 * [{resourceType,id}]}. Ownership replaces atomically, preserving the global
 * table for unbound resources. Empty contexts clears ownership only; plain
 * table loads clear ownership. Older loaders return 0; always check it. */
int32_t fhir_pods_terminology_load(const uint8_t *ptr, size_t len);

/* A Questionnaire (JSON) for every QuestionnaireResponse that names it by
 * canonical (url, or url|version) in any call after it — validated, lowered,
 * lifted, exported, composed into a document — on this module, for its
 * lifetime; a contained Questionnaire or a bundle entry is found without
 * loading. Returns 1 when loaded (the same canonical and version replaces
 * that entry), 0 on refusal without changing the store. A strict
 * fhir-pods.questionnaire-context.v1 envelope binds the form version to its
 * complete offline terminology table. Plain loads retain global/provider
 * behavior; empty input clears all forms and their owned contexts. */
int32_t fhir_pods_questionnaire_load(const uint8_t *ptr, size_t len);

/* A resource (JSON), or every entry resource of a Bundle (each under its
 * fullUrl), held in this module's resource store for its lifetime (ABI 15).
 * A reference that names no contained resource and no entry of the document
 * Bundle is looked up there by the rules a Bundle entry is: its fullUrl;
 * Type/id against a fullUrl ending so or the resource's type and id;
 * /_history/version selecting meta.versionId; the first match in load order
 * (so an unversioned reference to a resource stored in two versions finds
 * the one loaded first). resolve() finds a stored resource, and a stored
 * target is held to the element's targetProfiles by conformance and to its
 * base target types, as a Bundle entry is; a reference is looked up when it
 * is written in the validated resource or in a stored resource it reaches.
 * Validation reads the store (validate, operation_outcome, terminology_needs
 * and their _tx variants); lower, lift, export, document, audit, v2 and
 * deidentify do not. Returns 1 when loaded, 0 when the bytes are not a
 * resource or a Bundle of resources (an entry that is not an array of
 * objects included; the store is unchanged). An entry loaded under a fullUrl
 * is identified by it alone (a second load of that fullUrl replaces it); one
 * without is identified by its Type/id and meta.versionId (or none) and
 * replaces only another without a fullUrl. Empty input clears the store. The
 * runtime never fetches: the host retrieves what it stores, with its own
 * authorisation and consent. */
int32_t fhir_pods_resource_load(const uint8_t *ptr, size_t len);

/* Host options, for every call that follows (this module, its lifetime):
 *   {"unknownProfile": "warning" | "error",
 *    "externalNarrativeLinks": "allow" | "refuse"}
 * Each default is what the reference validator itself does, so a default
 * build is unchanged.
 *  - unknownProfile: a declared profile this build has not compiled is a
 *    warning and the resource stays valid (the reference validator's own
 *    severity). "error" fails closed: the same finding (profile.unknown) at
 *    error, so a host cannot accept an unverified claim by accident.
 *  - externalNarrativeLinks: a narrative reference out of the resource is
 *    accepted, as the reference validator accepts it — an image's `src` is
 *    not even held to the hyperlink scheme rules, so `javascript:` in an
 *    `img src` passes there. "refuse" makes any reference that is neither a
 *    `#fragment` resolved inside the resource nor a `data:` URL an error
 *    (narrative.external-link).
 * Returns 1 when the options were taken, 0 when the document was refused —
 * an unknown member, an unknown value and a member given twice all are,
 * since a typo in a strictness setting must not read as "off" — leaving the
 * previous options in force. Empty input restores the defaults. */
int32_t fhir_pods_options(const uint8_t *ptr, size_t len);

/* What a host must resolve for this resource to validate without terminology
 * warnings — the bindings neither the bundled sets nor the loaded table could
 * decide: {"needs":[{"valueSet","system","code"}, ...]}. Same output contract
 * as the validators. */
uint8_t *fhir_pods_terminology_needs(const uint8_t *ptr, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* FHIR_PODS_H */
