import FhirPodsExpoModule from "./FhirPodsExpoModule";

/** ABI version of the embedded verifier pod. */
export function abiVersion(): number {
  return FhirPodsExpoModule.abiVersion();
}

/**
 * The report returned by {@link probe}. Skeleton shape — the `issues` array is
 * where conformance findings will land once the validator is implemented.
 */
export interface ProbeReport {
  resourceType: string;
  pod: "fhir-pods";
  stage: string;
  issues: unknown[];
}

/**
 * Probe a FHIR resource (a JSON string). Skeleton pass: identifies the resource
 * and returns a {@link ProbeReport}. Synchronous — the native side is a
 * sub-millisecond C call, no bridge round-trip.
 */
export function probe(json: string): ProbeReport {
  return JSON.parse(FhirPodsExpoModule.probe(json)) as ProbeReport;
}
