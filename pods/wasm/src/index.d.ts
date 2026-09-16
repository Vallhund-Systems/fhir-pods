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

/** What de-identification does to an element. */
export type DeidAction = "keep" | "remove" | "mask" | "year" | "pseudonym";

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
