#!/usr/bin/env node
// End-to-end JS validate(): UTF-8 copy, WASM call, output copy, and JSON.parse.
import { readFile, appendFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import os from "node:os";
import { instantiate } from "../src/index.js";
import { fixtures } from "./fixtures.mjs";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const args = process.argv.slice(2);
function option(name, fallback) {
  const i = args.indexOf(name);
  if (i < 0) return fallback;
  if (!args[i + 1] || args[i + 1].startsWith("--")) throw new Error(`Missing ${name} value`);
  return args[i + 1];
}
const iterations = Number(option("--iterations", "2000"));
const warmup = 200;
const rounds = 5;
if (!Number.isSafeInteger(iterations) || iterations < 1 || iterations > 1000000) throw new Error("--iterations must be between 1 and 1000000");
const wasmPath = resolve(option("--wasm", fileURLToPath(new URL("../dist/fhir_pods.wasm", import.meta.url))));
const bytes = await readFile(wasmPath);
const sha256 = (data) => createHash("sha256").update(data).digest("hex");
const start = performance.now();
const pod = await instantiate(bytes);
const instantiateMs = performance.now() - start;
const results = [];
for (const fixture of fixtures) {
  const before = pod.validate(fixture.json);
  if (before.valid !== fixture.valid) throw new Error(`${fixture.name}: unexpected verdict ${JSON.stringify(before)}`);
  for (let i = 0; i < warmup; i++) pod.validate(fixture.json);
  const samples = [];
  let validCount = 0;
  for (let round = 0; round < rounds; round++) {
    const begin = performance.now();
    for (let i = 0; i < iterations; i++) validCount += Number(pod.validate(fixture.json).valid);
    samples.push((performance.now() - begin) * 1000 / iterations);
  }
  if (validCount !== (fixture.valid ? iterations * rounds : 0)) throw new Error(`${fixture.name}: unstable verdict`);
  const sorted = [...samples].sort((a, b) => a - b);
  results.push({ name: fixture.name, inputBytes: Buffer.byteLength(fixture.json), sha256: sha256(fixture.json), expectedValid: fixture.valid, appliedProfile: before.profile ?? null, samplesUs: samples, medianUs: sorted[2], minUs: sorted[0], maxUs: sorted[4] });
}
const git = (...argv) => execFileSync("git", argv, { cwd: root, encoding: "utf8" }).trim();
const report = {
  recordedAt: new Date().toISOString(), revision: git("rev-parse", "HEAD"),
  trackedSourceDirty: Boolean(git("status", "--porcelain", "--untracked-files=no")),
  runtime: process.version, platform: process.platform, architecture: process.arch,
  cpu: os.cpus()[0]?.model, abi: pod.abiVersion,
  wasm: { bytes: bytes.length, gzip9Bytes: gzipSync(bytes, { level: 9 }).length, sha256: sha256(bytes) },
  measurement: "Warm end-to-end JS validate, including input/output copies and JSON parsing; no terminology callback; no throughput pass threshold",
  iterations, warmup, rounds, instantiateMs, fixtures: results,
};
const line = `${JSON.stringify(report)}\n`;
const output = option("--output", null);
if (output) {
  await mkdir(dirname(resolve(output)), { recursive: true });
  await appendFile(output, line);
}
process.stdout.write(line);
