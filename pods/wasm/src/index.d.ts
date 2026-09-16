/** The ABI this wrapper speaks; a module reporting anything else is refused. */
export const ABI_VERSION: number;

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
  /** Human title of the profile applied, when one was. */
  profile?: string;
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

export interface Pod {
  abiVersion: number;
  /** Validate a resource (JSON text or UTF-8 bytes); the ergonomic report. */
  validate(json: string | Uint8Array): Validation;
  /** Validate a resource; the spec `OperationOutcome`. */
  operationOutcome(json: string | Uint8Array): OperationOutcome;
  /** Same as {@link validate}, unparsed — for passing straight through. */
  validateText(json: string | Uint8Array): string;
  /** Same as {@link operationOutcome}, unparsed — for the wire. */
  operationOutcomeText(json: string | Uint8Array): string;
}

/**
 * Instantiate the pod from the `.wasm` bytes (or a precompiled module).
 * Loading the bytes is the host's job (`fetch` / `readFile`).
 */
export function instantiate(source: BufferSource | WebAssembly.Module): Promise<Pod>;
