/** The ABI this wrapper speaks; a module reporting anything else is refused. */
export const ABI_VERSION: number;

// The report types below mirror pods/expo/src/index.ts verbatim — the pods are standalone packages; keep them in sync.
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

export interface Lifted {
  /** Slot path → value, as `lower` takes them. */
  inputs: Record<string, unknown>;
  validation: Validation;
  /** Required slots the resource has no value for. */
  missing: string[];
  /** Element paths present that no slot, fixed leaf or pass-through member covers. */
  extras: string[];
}

/** Options for converting an HL7 v2 message. */
export interface V2Options {
  /**
   * The zone a v2 time stated without one is in (`+10:00`); without it such a
   * time is truncated to its date, and reported in `skipped`.
   */
  timezone?: string;
  /** Coding-system names (`CWE.3`: `L`, `99LAB`) to uris, beyond the standard names the pod knows. */
  codeSystems?: Record<string, string>;
}

/** What the maps state for data the message carried, and the pod did not do. */
export interface V2Skip {
  /** Where in the message (`PID[1]-3.4`, `OBX[2]-5`). */
  at: string;
  /** The FHIR target the map names. */
  target: string;
  why: string;
}

export interface V2Converted {
  /** The message Bundle: MessageHeader first, `urn:uuid` entries, deterministic for the same message. */
  bundle: Record<string, unknown>;
  validation: Validation;
  skipped: V2Skip[];
  /** Segments the message structure has no place for (`ZPI[1]`), left out. */
  unplaced: string[];
  /** Valued fields of placed segments that no map row names (`PID[1]-2`): the IG says nothing about them, so nothing was done. */
  unmapped: string[];
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

/** One binding the bundled sets (and any loaded table) could not decide. */
export interface Need {
  valueSet: string;
  /** "" for a bare `code` element. */
  system: string;
  code: string;
}

export interface Pod {
  abiVersion: number;
  /** Validate a resource (JSON text or UTF-8 bytes); the ergonomic report. */
  validate(json: string | Uint8Array): Validation;
  /** Validate a resource; the spec `OperationOutcome`. */
  operationOutcome(json: string | Uint8Array): OperationOutcome;
  /**
   * The selected profile's must-support elements the resource leaves empty
   * (where their parent is present). Information for a client certifying
   * against the IG — never part of the validation report; empty for
   * unparseable input or no applicable profile.
   */
  mustSupportGaps(json: string | Uint8Array): string[];
  /**
   * Build a conformant resource for the profile from its variable inputs (a
   * JSON object keyed by slot path — see {@link Pod.slots}), validated on the
   * way out. Throws only when lowering cannot start (unknown profile, inputs
   * not an object); every other problem is an issue in `validation`.
   */
  lower(profileUrl: string, inputs: Record<string, unknown> | string): Lowered;
  /** The inputs a profile takes. */
  slots(profileUrl: string): Slot[];
  /**
   * The reverse of `lower`: a resource read back into the profile's slots
   * (`profileUrl` "" takes the most specific compiled profile the resource
   * declares). `inputs` is what `lower` takes; `missing` names required
   * slots the resource lacks; `extras` names element paths no slot covers,
   * so nothing is left behind unnamed. Validated on the way in. Throws only
   * when it cannot start.
   */
  lift(profileUrl: string, resource: Record<string, unknown> | string): Lifted;
  /**
   * An HL7 v2 message (ER7 text or bytes; MLLP framing is the transport's)
   * converted to a FHIR message Bundle through the hl7.fhir.uv.v2mappings
   * maps and validated on the way out. What the maps state and the pod
   * could not do for data the message carried is in `skipped`, never
   * invented; what FHIR requires and the message does not carry is in
   * `validation`. Throws only when it cannot start (not a message, no map
   * for its structure, bad options).
   */
  v2(message: string | Uint8Array, options?: V2Options | string): V2Converted;
  /**
   * An IPS or AU Patient Summary document from a patient and their resources:
   * sections from the kind's Composition profile, each resource placed by what
   * it is, narrative generated, references rewritten to the entries, and the
   * Bundle validated on the way out. Throws only when it cannot start.
   */
  document(kind: DocumentKind, inputs: DocumentInputs | string): AssembledDocument;
  /**
   * An AuditEvent for a read, query, create, update or delete of a record
   * (with or without a known patient), conformant to the IHE Basic Audit Log
   * Patterns profile for that kind, from host-shaped inputs, validated on the
   * way out. Throws only when it cannot start (unknown kind, a required input
   * missing or out of place).
   */
  audit(kind: AuditKind, inputs: AuditInputs | string): Lowered;
  /**
   * A de-identified copy under `policy` (Safe Harbor by default): names,
   * contact details, addresses below state, exact dates, ages over 89,
   * narrative, notes and attachments removed (or masked where the profile
   * requires the element); identifiers and references pseudonymised with
   * the salt. Throws only when it cannot start (bad policy, not a resource).
   */
  deidentify(json: string | Uint8Array, policy: DeidPolicy): Deidentified;
  /**
   * Load a compiled terminology table (the line format `tools/compile_valueset.py`
   * writes from `$expand` responses). It answers after the bundled sets and
   * before `options.terminology`, for every call that follows. A set in the
   * table is complete: a code not listed is not a member. `false` when the
   * text could not be parsed (the previous table stays); "" clears the table.
   */
  loadTerminology(table: string): boolean;
  /**
   * What to resolve for this resource to validate without terminology
   * warnings: the bindings neither the bundled sets nor the loaded table
   * could decide. Resolve them (`$validate-code`, or `$expand` and compile
   * the sets) and validate again.
   */
  terminologyNeeds(json: string | Uint8Array): Need[];
  /** Same as {@link validate}, unparsed — for passing straight through. */
  validateText(json: string | Uint8Array): string;
  /** Same as {@link operationOutcome}, unparsed — for the wire. */
  operationOutcomeText(json: string | Uint8Array): string;
}

/**
 * The terminology seam: asked, synchronously, about the bindings the bundled
 * value sets can't decide. `true` = member, `false` = not a member,
 * `undefined` = unknown (a warning, never an error). `system` is "" for a
 * bare `code` element.
 */
export type TerminologyProvider = (valueSet: string, system: string, code: string) => boolean | undefined;

export interface Options {
  terminology?: TerminologyProvider;
}

/**
 * Instantiate the pod from the `.wasm` bytes (or a precompiled module).
 * Loading the bytes is the host's job (`fetch` / `readFile`).
 */
export function instantiate(source: BufferSource | WebAssembly.Module, options?: Options): Promise<Pod>;
