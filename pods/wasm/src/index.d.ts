/** The ABI this wrapper speaks; a module reporting anything else is refused. */
export const ABI_VERSION: number;

// The report types below mirror pods/expo/src/index.ts verbatim — the pods are standalone packages; keep them in sync.
/**
 * A finding's severity. `valid` is "no `error` and no `fatal`", so these are
 * the two that decide a verdict:
 *
 * - `fatal` — something could not be checked at all, and the finding says
 *   which. Only two are fatal: the input is not JSON (`parse.json`; there is
 *   no document, so that finding stands alone — a short issue list there does
 *   not mean "almost conformant") and the Questionnaire a response is held to
 *   has an item with no `type` (`questionnaire.definition`).
 * - `error` — the resource does not conform.
 * - `warning` — a check that could not be performed (no bundled terminology,
 *   an expression outside the supported core), a declared profile this build
 *   has not compiled (`profile.unknown` — see {@link HostOptions} to make it
 *   an error), or an invariant the definitions grade best-practice.
 * - `information` — said for the record, never a defect (a skipped profile
 *   invariant; no Questionnaire named at all).
 *
 * Every id, its severity and the condition that produces it are in
 * `docs/rules.md` in the fhir-pods repository.
 */
export type Severity = "fatal" | "error" | "warning" | "information";

/** One finding, in the pods' ergonomic shape. */
export interface Issue {
  severity: Severity;
  /** FHIR IssueType code (`required`, `value`, `invariant`, `structure`, `code-invalid`, …). */
  code: string;
  /** The stable rule id (`binding.no-code`, `inv.cmp-1`, `questionnaire.required-answer`, …) — what to assert on, never the message text; see docs/rules.md for every id, its severity and what produces it. */
  id: string;
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
  /** No `fatal` and no `error` issue. A `warning` or `information` finding never changes it — including `profile.unknown`, which says a declared profile was never checked. */
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
  /** `coding` carries the stable rule id under system `urn:fhir-pods:rule`. */
  details?: { text: string; coding?: { system: string; code: string }[] };
  /** FHIRPath expression(s) locating the finding. */
  expression?: string[];
}

/** One variable input of a profile. */
/** A fixed leaf beside or beneath a slot: what the profile supplies there. */
export interface SlotFixed {
  path: string;
  value: string;
}

/** A member of a composite datatype slot (a `HumanName`'s `family`, `given`…), from the base element table. */
export interface SlotPart {
  name: string;
  type: string;
  repeats: boolean;
  required: boolean;
}

export interface CatalogueProfile {
  url: string;
  name: string;
  resourceType: string;
  kind: "resource" | "datatype" | "extension";
  base: string | null;
}

export interface Catalogue {
  regions: { name: ExportRegion; title: string; guide: string; document: DocumentKind | null }[];
  profiles: CatalogueProfile[];
}

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
  /** What the profile fixes beside this slot (same parent) or beneath it: a Quantity `unit` slot shows the fixed UCUM `code`, which lowering writes as the unit when none is given. */
  fixed: SlotFixed[];
  /** For a composite datatype slot, its members — what to write in the object. Empty for a primitive. */
  parts: SlotPart[];
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
  /** Record values that disagree with fixed leaves; lowering would replace them. */
  conflicts: { path: string; given: string; fixed: string }[];
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

/** A jurisdiction the exporter speaks: `au` (AU Core + AU Patient Summary), `us` (US Core + IPS document), `ips` (the international floor). */
export type ExportRegion = "au" | "us" | "uk" | "ca" | "nz" | "ips";

/** What an export takes: the record's resources as the host lowered them (the base R4 definitions, or any), the document's inputs, and optionally the audit context. */
export interface ExportInputs {
  resources: Record<string, unknown>[];
  /** `false` for the re-profiled resources only, no document. */
  document?: boolean;
  /** As `document` takes them; `author` and `date` are required when a document is assembled. */
  author?: string | string[];
  date?: string;
  title?: string;
  custodian?: string;
  identifier?: string;
  timestamp?: string;
  /** The BALP `patient-read` of this export: everything but `patient` and `data`, which the export fills in. */
  audit?: Omit<AuditInputs, "patient" | "data"> & { patient?: AuditWho; data?: AuditInputs["data"] };
}

export interface ExportedProfile {
  url: string;
  name: string;
}

export interface ExportedResource {
  /** Its position in `inputs.resources`. */
  index: number;
  resourceType: string;
  id: string | null;
  /** The region's profile it went out through; `null` when it went out as given (see `note`). */
  profile: ExportedProfile | null;
  /** The profile the region's document asks of such an entry when it is not the target: declared beside it, verified. */
  also: ExportedProfile | null;
  resource: Record<string, unknown>;
  validation: Validation;
  /** Required slots of the profile the record had no value for — reported, never invented. */
  missing: string[];
  /** Required slots of `also` the record had no value for. */
  alsoMissing: string[];
  /** Element paths of the given resource the profile has no slot for: not carried. */
  extras: string[];
  /** Why no profile applied (none in the region for the type; several and none declared). */
  note: string | null;
}

export interface Exported {
  region: { name: ExportRegion; title: string; guide: string };
  resources: ExportedResource[];
  document: (AssembledDocument & { kind: DocumentKind }) | null;
  audit: Lowered | null;
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
 * What de-identification does to an element. `collapse-age` is the host's
 * declaration that this element *is* an age: over 89 aggregates (an integer
 * or decimal to `90`; a quantity to `comparator: ">="` with 90 in its own
 * unit). Nothing says which integer is an age, so the pod never guesses —
 * and an element no age can be read from is left alone and named in
 * `residual`. `clamp` is what the walk does to an `Age` on its own.
 */
export type DeidAction = "keep" | "remove" | "mask" | "year" | "pseudonym" | "clamp" | "collapse-age";

/**
 * A de-identification policy: the rule set a typed walk of the resource
 * applies. `base` is the built-in `safe-harbor` policy unless `"none"` — a
 * rule set *modelled on* HIPAA Safe Harbor's identifier list, by FHIR type.
 * It is **not a compliance determination**: it covers the fields its rules
 * name, in the resource it is given, and says nothing about
 * re-identification risk from what remains, from the host's other data, or
 * from how the copy is used. `rules` apply after the base, most specific
 * target first (`Patient.address.city` beats `Address.city` beats
 * `Address`). `salt` is **required and non-empty** — it keys the pseudonyms,
 * and the call throws without one rather than emitting unkeyed ones; keep it
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
  /** What the built-in policy's own check still finds in the copy — empty under the default policy. Not a compliance verdict. */
  remaining: { path: string; what: string }[];
  /** Free text the policy kept in place (a CodeableConcept.text, a Dosage.text…), named so an empty `remaining` never reads as "all free text gone" — and a `collapse-age` rule that found no age to collapse. */
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
  /** What is compiled in: the regions `export` speaks and every profile in the registry. */
  profiles(): Catalogue;
  slots(profileUrl: string): Slot[];
  /**
   * The reverse of `lower`: a resource read back into the profile's slots
   * (`profileUrl` "" takes the most specific compiled profile the resource
   * declares). `inputs` is what `lower` takes; `missing` names required
   * slots the resource lacks; `extras` names element paths no slot covers;
   * `conflicts` names values that disagree with fixed profile content.
   * Validated on the way in. Throws only
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
   * A record spoken for a region: every resource re-profiled through the
   * region's profile for it (the profile's fixed content added, its id
   * kept, `missing`/`extras` reported), the region's patient summary
   * assembled, and the read recorded as a BALP event when `audit` is given.
   * Write the transformer once against the base definitions; export picks
   * the jurisdiction. Throws only when it cannot start (unknown region, not
   * resources, a document or audit that cannot be built).
   */
  export(region: ExportRegion, inputs: ExportInputs | string): Exported;
  /**
   * A copy of the resource under `policy` — the built-in `safe-harbor` rule
   * set unless `base` says otherwise: names, contact details, addresses
   * below state, exact dates (year only), ages over 89 on an `Age`,
   * narrative, notes, attachments, signatures and device serials removed (or
   * masked where the profile requires the element); identifiers and
   * references pseudonymised with the salt. A rule set modelled on HIPAA
   * Safe Harbor's identifier list — **not a compliance determination**; see
   * {@link DeidPolicy}. Throws when it cannot start: a policy with no
   * non-empty `salt`, an unknown `base` or action, or input that is not a
   * FHIR resource.
   */
  deidentify(json: string | Uint8Array, policy: DeidPolicy): Deidentified;
  /**
   * Set the host options for every call that follows — the two strictness
   * calls whose default is the reference validator's own behaviour, so a
   * default pod is unchanged. `false` when the document is refused (an
   * unknown member, an unknown value, or a member given twice, since a typo
   * in a strictness setting must not read as "off") and the previous options
   * stay in force; "" or `{}` restores the defaults.
   */
  options(json: HostOptions | string): boolean;
  /**
   * Load a compiled terminology table (the line format `tools/compile_valueset.py`
   * writes from `$expand` responses). It answers after the bundled sets and
   * before `options.terminology`, for every call that follows. A set in the
   * table is complete: a code not listed is not a member. `false` when the
   * text could not be parsed (the previous table stays); "" clears the table.
   */
  loadTerminology(table: string): boolean;
  /**
   * Load a Questionnaire (JSON) for every QuestionnaireResponse validated
   * after it that names it by canonical (`url`, or `url|version`); a
   * contained Questionnaire or a bundle entry is found without loading.
   * Also accepts a strict `{ format: "fhir-pods.questionnaire-context.v1",
   * questionnaire: <Questionnaire>, terminology: <table text> }` envelope.
   * The form must name its url and version; responses name that exact version.
   * Its complete table is isolated from other forms/global providers. Unknown
   * membership remains a warning and a terminology need. Plain loads retain
   * global/provider behavior. `false` on refusal without replacing the prior
   * entry; "" clears all forms and their owned contexts.
   */
  loadQuestionnaire(json: string): boolean;
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
 * The host options (see {@link Pod.options}). Every member is optional and
 * takes its default — what the reference validator does — when absent.
 */
export interface HostOptions {
  /**
   * A declared profile this build has not compiled: `"warning"` (the
   * default, and the reference validator's own severity) leaves the resource
   * valid with a `profile.unknown` finding naming it; `"error"` fails closed
   * — the same finding, at error.
   */
  unknownProfile?: "warning" | "error";
  /**
   * A narrative reference out of the resource — `<img src="http://…">`,
   * `<a href="file:///…">`: `"allow"` (the default) is the reference
   * validator's behaviour, which does not hold an image's `src` to the
   * hyperlink scheme rules at all. `"refuse"` makes any reference that is
   * neither a `#fragment` resolved inside the resource nor a `data:` URL an
   * error (`narrative.external-link`) — for a host that renders the
   * narrative and wants it to fetch nothing.
   */
  externalNarrativeLinks?: "allow" | "refuse";
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
