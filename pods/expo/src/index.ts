import FhirPodsExpoModule from "./FhirPodsExpoModule";

/** ABI version of the embedded verifier pod. */
export function abiVersion(): number {
  return FhirPodsExpoModule.abiVersion();
}

// The report types below mirror pods/wasm/src/index.d.ts verbatim — the pods are standalone packages; keep them in sync.
export type Severity = "fatal" | "error" | "warning" | "information";

/** One finding, in the pods' ergonomic shape. */
export interface Issue {
  severity: Severity;
  /** FHIR IssueType code (`required`, `value`, `invariant`, `structure`, `code-invalid`, …). */
  code: string;
  /** The element the finding is about, as a dotted path (`Observation.value[x]:valueQuantity.code`). */
  path: string;
  message: string;
}

/** The ergonomic report. Never thrown — validation state is data. */
export interface Validation {
  resourceType: string;
  /** Human title of the profile applied, when one was (the first, when several). */
  profile?: string;
  /** Every profile applied, when a resource declares more than one. */
  profiles?: string[];
  /** No `fatal` or `error` issues. */
  valid: boolean;
  issues: Issue[];
}

/** A FHIR `OperationOutcome` — the `$validate` wire form. */
export interface OperationOutcome {
  resourceType: "OperationOutcome";
  issue: OperationOutcomeIssue[];
}

export interface OperationOutcomeIssue {
  severity: Severity;
  code: string;
  details?: { text: string };
  /** FHIRPath expression(s) locating the finding. */
  expression?: string[];
}

/**
 * Validate a FHIR resource (a JSON string): the ergonomic report. Synchronous —
 * the native side is a sub-millisecond C call, no bridge round-trip. Never
 * throws for bad input: unparseable JSON is a `fatal` issue in the report.
 */
export function validate(json: string): Validation {
  return JSON.parse(FhirPodsExpoModule.validate(json)) as Validation;
}

/** Validate a FHIR resource: the spec `OperationOutcome`, for the wire. */
export function operationOutcome(json: string): OperationOutcome {
  return JSON.parse(FhirPodsExpoModule.operationOutcome(json)) as OperationOutcome;
}

/**
 * The selected profile's must-support elements the resource leaves empty
 * (where their parent is present). Information for a client certifying
 * against the IG — never part of the validation report; empty for
 * unparseable input or no applicable profile.
 */
export function mustSupportGaps(json: string): string[] {
  return (JSON.parse(FhirPodsExpoModule.mustSupport(json)) as { mustSupportGaps: string[] }).mustSupportGaps;
}

/** One variable input of a profile. */
export interface Slot {
  /** Labelled, resource-relative path: `subject`, `component:SystolicBP.valueQuantity.value`. */
  path: string;
  /** The FHIR type the value is coerced to; `choice` for a `[x]` element (see `choices`). */
  type: string;
  /** For a choice slot, the concrete keys accepted as input keys (`effectiveDateTime`, …). */
  choices: string[];
  min: number;
  /** `null` = unbounded. */
  max: number | null;
  mustSupport: boolean;
}

export interface Lowered {
  resource: Record<string, unknown>;
  validation: Validation;
}

/**
 * Build a conformant resource for the profile from its variable inputs (a
 * JSON object keyed by slot path — see {@link slots}), validated on the way
 * out; the native terminology provider, if installed, is consulted. Throws
 * only when lowering cannot start (unknown profile, inputs not an object);
 * every other problem is an issue in `validation`.
 */
export function lower(profileUrl: string, inputs: Record<string, unknown>): Lowered {
  const out = JSON.parse(FhirPodsExpoModule.lower(profileUrl, JSON.stringify(inputs))) as Lowered & { error?: string };
  if (out.error) throw new Error(`fhir-pods: ${out.error}`);
  return out;
}

/** Which patient summary a document is. */
export type DocumentKind = "ips" | "au-ps";

/** What a document is assembled from. */
export interface DocumentInputs {
  /** The Patient and the resources to place, bare or as Bundle entries `{fullUrl, resource}`. */
  resources: Record<string, unknown>[];
  /** A reference (or references) to the author, resolved among the resources. */
  author: string | string[];
  /** `Composition.date`; also the Bundle timestamp when it is an instant. */
  date: string;
  title?: string;
  custodian?: string;
  /** The document's identifier (derived from the inputs otherwise). */
  identifier?: string;
  /** An instant, needed when `date` is not one. */
  timestamp?: string;
}

export interface AssembledDocument {
  document: Record<string, unknown>;
  /** Where each resource went: its entry's fullUrl and the section's LOINC code ("" for a supporting entry). */
  placed: { fullUrl: string; section: string }[];
  /** Resources left out because nothing in the document reaches them (`Type/id`). */
  omitted: string[];
  /** The document's conformance to the kind's Bundle profile. */
  validation: Validation;
}

/**
 * An IPS or AU Patient Summary document from a patient and their resources:
 * sections from the kind's Composition profile, each resource placed by what
 * it is, narrative generated, references rewritten to the entries, and the
 * Bundle validated on the way out. Throws only when it cannot start.
 */
export function document(kind: DocumentKind, inputs: DocumentInputs): AssembledDocument {
  const out = JSON.parse(FhirPodsExpoModule.document(kind, JSON.stringify(inputs))) as AssembledDocument & { error?: string };
  if (out.error) throw new Error(`fhir-pods: ${out.error}`);
  return out;
}


/** Which audit event: the RESTful action, with or without a known patient subject. */
export type AuditKind =
  | "read" | "patient-read" | "query" | "patient-query" | "create" | "patient-create"
  | "update" | "patient-update" | "delete" | "patient-delete";

/** A who: `Type/id` (or a URL) is a reference, any other string a display name, an object a Reference. */
export type AuditWho = string | Record<string, unknown>;

/** A network access point: a string is classified (IP, URI, email, machine name), or `{address, type}`. */
export type AuditNetwork = string | { address: string; type: string };

export interface AuditAgent {
  who: AuditWho;
  network?: AuditNetwork;
  /** Whether this agent initiated the event; false for a client or server, true for the user. */
  requestor?: boolean;
}

export interface AuditUser extends AuditAgent {
  name?: string;
  /** A v3-ParticipationType code (create / update / delete only; `AUT` unless given), or a CodeableConcept. */
  type?: string | Record<string, unknown>;
  role?: Record<string, unknown> | Record<string, unknown>[];
  purposeOfUse?: Record<string, unknown> | Record<string, unknown>[];
}

/** What a host knows at the moment a record is read, queried, created, updated or deleted. */
export interface AuditInputs {
  /** An instant. */
  recorded: string;
  client: AuditAgent;
  server: AuditAgent;
  user?: AuditUser;
  source: { observer: AuditWho; site?: string; type?: Record<string, unknown> | Record<string, unknown>[] };
  /** The resource touched (not for a query); `{what, role}` to give the object role (`4` unless given). */
  data?: AuditWho | { what: AuditWho; role?: string | Record<string, unknown> };
  /** The query, as sent (a query kind); base64-encoded as the profile requires. */
  query?: string;
  /** The patient subject (a patient-* kind). */
  patient?: AuditWho;
  /** The request id (X-Request-Id): the correlation with the trace. */
  transaction?: string;
  /** The RESTful interaction where the profile binds rather than fixes it (`vread`, `patch`, `search-system`…). */
  subtype?: string | Record<string, unknown>;
  id?: string;
  text?: Record<string, unknown>;
  extension?: Record<string, unknown>[];
}

/**
 * An AuditEvent for a read, query, create, update or delete of a record
 * (with or without a known patient), conformant to the IHE Basic Audit Log
 * Patterns profile for that kind, from host-shaped inputs, validated on the
 * way out. Throws only when it cannot start.
 */
export function audit(kind: AuditKind, inputs: AuditInputs): Lowered {
  const out = JSON.parse(FhirPodsExpoModule.audit(kind, JSON.stringify(inputs))) as Lowered & { error?: string };
  if (out.error) throw new Error(`fhir-pods: ${out.error}`);
  return out;
}

/** What de-identification does to an element. */
export type DeidAction = "keep" | "remove" | "mask" | "year" | "pseudonym" | "clamp";

/**
 * A de-identification policy. `base` is Safe Harbor unless "none"; `rules`
 * apply after it, most specific target first (`Patient.address.city` beats
 * `Address.city` beats `Address`). `salt` keys the pseudonyms — keep it
 * secret and stable. `today` (a date) is what "over 89" is measured against.
 */
export interface DeidPolicy {
  salt: string;
  today?: string;
  base?: "safe-harbor" | "none";
  rules?: { target: string; action: DeidAction }[];
}

export interface Deidentified {
  resource: Record<string, unknown>;
  /** What was done, element by element. */
  applied: { path: string; action: DeidAction }[];
  /** The copy's conformance to the profiles it declares. */
  validation: Validation;
  /** What a Safe Harbor check still finds in the copy — empty under the default policy. */
  remaining: { path: string; what: string }[];
  /** Free text the policy kept in place (a CodeableConcept.text, a Dosage.text…), named so an empty `remaining` never reads as "all free text gone". */
  residual: { path: string; what: string }[];
}

/**
 * A de-identified copy under `policy` (Safe Harbor by default): names,
 * contact details, addresses below state, exact dates, ages over 89,
 * narrative, notes and attachments removed (or masked where the profile
 * requires the element); identifiers and references pseudonymised with the
 * salt. Throws only when it cannot start (bad policy, not a resource).
 */
export function deidentify(json: string, policy: DeidPolicy): Deidentified {
  const out = JSON.parse(FhirPodsExpoModule.deidentify(json, JSON.stringify(policy))) as Deidentified & { error?: string };
  if (out.error) throw new Error(`fhir-pods: ${out.error}`);
  return out;
}

/** One binding the bundled sets (and any loaded table) could not decide. */
export interface Need {
  valueSet: string;
  /** "" for a bare `code` element. */
  system: string;
  code: string;
}

/**
 * Load a compiled terminology table (the line format
 * `tools/compile_valueset.py` writes from `$expand` responses). It answers
 * after the bundled sets and before the native provider, for every call that
 * follows, for the process's lifetime. A set in the table is complete: a code
 * not listed is not a member. Returns `false` when the text could not be
 * parsed (the previous table stays); "" clears the table.
 */
export function loadTerminology(table: string): boolean {
  return FhirPodsExpoModule.terminologyLoad(table) === 1;
}

/**
 * What to resolve for this resource to validate without terminology warnings:
 * the bindings neither the bundled sets nor the loaded table could decide.
 * Resolve them against a terminology server (`$validate-code`, or `$expand`
 * and compile the sets) and validate again.
 */
export function terminologyNeeds(json: string): Need[] {
  return (JSON.parse(FhirPodsExpoModule.terminologyNeeds(json)) as { needs: Need[] }).needs;
}

/** The inputs a profile takes. */
export function slots(profileUrl: string): Slot[] {
  const out = JSON.parse(FhirPodsExpoModule.slots(profileUrl)) as { slots?: Slot[]; error?: string };
  if (out.error) throw new Error(`fhir-pods: ${out.error}`);
  return out.slots ?? [];
}
