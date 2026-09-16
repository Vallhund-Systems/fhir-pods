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
