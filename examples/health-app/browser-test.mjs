#!/usr/bin/env node
// Test the actual page, controls and downloaded files in an isolated Chrome.
import assert from "node:assert/strict";
import { readFile, writeFile, mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { setTimeout as delay } from "node:timers/promises";
import { startServer } from "./serve.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const option = (name, fallback) => {
  const at = process.argv.indexOf(name);
  if (at < 0) return fallback;
  if (!process.argv[at + 1] || process.argv[at + 1].startsWith("--")) throw new Error(`Missing ${name} value`);
  return process.argv[at + 1];
};
const output = resolve(option("--output", join(tmpdir(), "fhir-pods-reference-app")));
await mkdir(output, { recursive: true });
const wasmPath = resolve(option("--wasm", join(root, "pods/wasm/dist/fhir_pods.wasm")));
const wasm = await readFile(wasmPath); // Freeze the tested artifact before serving.
const sha = data => createHash("sha256").update(data).digest("hex");
const files = ["index.html", "style.css", "app.mjs", "integration.mjs", "serve.mjs", "browser-test.mjs"].map(f => `examples/health-app/${f}`);
files.push("pods/wasm/src/index.js");
const sourceHashes = Object.fromEntries(await Promise.all(files.map(async file => [file, sha(await readFile(join(root, file)))])));
const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
const provenance = { revision: git("rev-parse", "HEAD"), dirtyPaths: git("status", "--porcelain").split("\n").filter(Boolean), sourceHashes, wasmPath, wasmSha256: sha(wasm), wasmBytes: wasm.length, node: process.version };
const temporary = await mkdtemp(join(tmpdir(), "fhir-pods-reference-browser-"));
const downloads = join(temporary, "downloads"); await mkdir(downloads);
const { server, url } = await startServer(0, { wasm });
const chrome = process.env.CHROME ?? (process.platform === "darwin" ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" : "chromium");
const checks = [], requests = [], exceptions = [];
let child, socket, browser, failure;
try {
  child = spawn(chrome, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--remote-debugging-port=0", `--user-data-dir=${join(temporary, "profile")}`, "about:blank"], { stdio: ["ignore", "ignore", "pipe"] });
  let launchError, log = "";
  child.on("error", error => { launchError = error; });
  child.stderr.on("data", chunk => { log = (log + chunk).slice(-3000); });
  const until = async (fn, label, timeout = 15000) => {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) { const value = await fn(); if (value) return value; await delay(60); }
    throw new Error(`Timed out: ${label}`);
  };
  const port = await until(async () => {
    if (launchError) throw launchError;
    if (child.exitCode !== null) throw new Error(`Chrome exited: ${log}`);
    try { return (await readFile(join(temporary, "profile/DevToolsActivePort"), "utf8")).split("\n")[0]; } catch { return null; }
  }, "Chrome startup");
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  socket = new WebSocket(targets.find(t => t.type === "page").webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener("open", resolve, { once: true }); socket.addEventListener("error", reject, { once: true }); });
  let next = 0;
  const pending = new Map();
  socket.addEventListener("message", ({ data }) => {
    const message = JSON.parse(data);
    if (message.method === "Network.requestWillBeSent") requests.push(message.params.request.url);
    if (message.method === "Runtime.exceptionThrown") exceptions.push(message.params.exceptionDetails.text);
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    if (message.error) request.reject(new Error(JSON.stringify(message.error))); else request.resolve(message.result);
  });
  const send = (method, params = {}) => {
    const id = ++next;
    return Promise.race([
      new Promise((resolve, reject) => { pending.set(id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params })); }),
      delay(10000, undefined, { ref: false }).then(() => { pending.delete(id); throw new Error(`CDP timeout: ${method}`); }),
    ]);
  };
  const evaluate = async expression => {
    const out = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true, userGesture: true });
    if (out.exceptionDetails) throw new Error(out.exceptionDetails.exception?.description ?? out.exceptionDetails.text);
    return out.result?.value;
  };
  const click = selector => evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  const selectView = async view => {
    if (!await evaluate("document.getElementById('integration-details').open")) await click("#integration-details > summary");
    await click(`[data-view=${view}]`);
  };
  const set = (selector, value, event = "input") => evaluate(`{const e=document.querySelector(${JSON.stringify(selector)});e.value=${JSON.stringify(value)};e.dispatchEvent(new Event(${JSON.stringify(event)},{bubbles:true}));}`);
  const check = async (name, fn) => { await fn(); checks.push({ name, pass: true }); };
  const title = () => evaluate("document.getElementById('decision-title').textContent");
  const disabled = id => evaluate(`document.getElementById(${JSON.stringify(id)}).disabled`);
  const acknowledge = async () => {
    if (!await evaluate("document.getElementById('limitations-panel').hidden") && !await evaluate("document.getElementById('acknowledge-limits').checked")) await click("#acknowledge-limits");
  };
  const screenshot = async name => {
    await evaluate("new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))");
    const { cssContentSize } = await send("Page.getLayoutMetrics");
    const { data } = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true,
      clip: { x: 0, y: 0, width: cssContentSize.width, height: cssContentSize.height, scale: 1 } });
    await writeFile(join(output, name), Buffer.from(data, "base64"));
  };
  await send("Runtime.enable"); await send("Network.enable");
  await send("Browser.setDownloadBehavior", { behavior: "allow", downloadPath: downloads });
  await send("Emulation.setDeviceMetricsOverride", { width: 1440, height: 1300, deviceScaleFactor: 1, mobile: false });
  browser = (await send("Browser.getVersion")).product;
  await send("Page.navigate", { url });
  await until(() => evaluate("document.documentElement?.dataset.runtime"), "runtime initialization");
  await check("initial AU preview includes unsupported findings; all six destinations are discovered", async () => {
    await selectView("export");
    const result = JSON.parse(await evaluate("document.getElementById('json-output').textContent"));
    assert.equal(result.region.name, "au");
    if (result.resources.some(r => r.validation.issues.some(i => i.code === "not-supported"))) {
      assert.equal(await title(), "Review before use"); assert.equal(await disabled("download-fhir"), true);
      assert.ok((await evaluate("document.getElementById('findings').textContent")).includes("not checked"));
    }
    assert.equal(await evaluate("document.querySelectorAll('[data-region]').length"), 6);
  });
  await click("#integration-details > summary");
  await screenshot("desktop.png");
  await check("the downloaded FHIR is the validated collection, not a review envelope", async () => {
    await click("[data-region=us]"); await acknowledge(); assert.equal(await disabled("download-fhir"), false);
    await click("#download-fhir");
    const data = await until(async () => { try { return JSON.parse(await readFile(join(downloads, "check-in-us.fhir.json"), "utf8")); } catch { return null; } }, "FHIR download");
    assert.equal(data.resourceType, "Bundle"); assert.equal(data.type, "collection");
    assert.equal(data.entry.find(e => e.resource.resourceType === "Observation").resource.valueQuantity.value, 72);
  });
  await check("editing invalidates stale downloads; rebuilding uses the changed reading", async () => {
    await set("#bpm", "81"); assert.equal(await disabled("download-fhir"), true); assert.equal(await disabled("download-review"), true);
    await click("#build");
    assert.equal(await evaluate("document.getElementById('acknowledge-limits').checked"), false);
    await acknowledge(); assert.equal(await disabled("download-fhir"), false);
    await selectView("lift");
    const lifted = JSON.parse(await evaluate("document.getElementById('json-output').textContent"));
    assert.equal(lifted.inputs["valueQuantity.value"], 81);
    assert.deepEqual(lifted.conflicts, []);
  });
  await check("missing regional inputs block FHIR and stay in the review download", async () => {
    await click("[data-region=au]");
    await set("#preset", "missing", "change");
    assert.equal(await title(), "Review before use"); assert.equal(await disabled("download-fhir"), true);
    await click("#download-review");
    const data = await until(async () => { try { return JSON.parse(await readFile(join(downloads, "check-in-au.review.json"), "utf8")); } catch { return null; } }, "review download");
    assert.equal(data.kind, "fhir-pods-reference-review"); assert.equal(data.hostDecision.ready, false);
    assert.ok(data.hostDecision.required.some(r => r.path === "birthDate"));
    assert.equal(data.hostModel.person.birthDate, "");
  });
  await check("qualifier preservation and partial lift are both visible", async () => {
    await click("[data-region=us]");
    await set("#preset", "qualified", "change");
    await acknowledge();
    assert.equal(await disabled("download-fhir"), false);
    assert.ok((await evaluate("document.getElementById('findings').textContent")).includes("valueQuantity.comparator"));
    await selectView("lift");
    const lifted = JSON.parse(await evaluate("document.getElementById('json-output').textContent"));
    assert.ok(lifted.extras.includes("valueQuantity.comparator"));
  });
  await check("summary terminology and IPS validation findings pause download", async () => {
    await click("[data-region=au]");
    await click("#include-document");
    await acknowledge();
    assert.equal(await disabled("download-fhir"), true);
    assert.ok((await evaluate("document.getElementById('findings').textContent")).includes("Unresolved terminology"));
    await click("[data-region=ips]"); await selectView("export");
    const result = JSON.parse(await evaluate("document.getElementById('json-output').textContent"));
    assert.equal(result.document.validation.valid, false); assert.equal(await disabled("download-fhir"), true);
  });
  await check("host limitation acceptance is explicit, recorded and reversible", async () => {
    await click("#reset"); await click("[data-region=us]");
    const limited = !await evaluate("document.getElementById('limitations-panel').hidden");
    if (limited) {
      assert.equal(await disabled("download-fhir"), true);
      await acknowledge(); assert.equal(await disabled("download-fhir"), false);
      assert.equal(await title(), "Local download with limitations");
      await click("#download-review");
      const data = await until(async () => { try { return JSON.parse(await readFile(join(downloads, "check-in-us.review.json"), "utf8")); } catch { return null; } }, "accepted-limits review download");
      assert.ok(data.hostDecision.acceptedLimitations.length > 0);
      await click("#acknowledge-limits"); assert.equal(await disabled("download-fhir"), true);
      await acknowledge(); await click("[data-region=au]");
      assert.equal(await evaluate("document.getElementById('acknowledge-limits').checked"), false);
    }
  });
  await check("every destination renders its current runtime report", async () => {
    await click("#reset");
    for (const name of ["au", "us", "uk", "ca", "nz", "ips"]) {
      await click(`[data-region=${name}]`);
      const result = JSON.parse(await evaluate("document.getElementById('json-output').textContent"));
      assert.equal(result.region.name, name);
      if (result.resources.some(r => r.extras.length || r.notes.length || r.note)) assert.equal(await disabled("download-fhir"), true);
    }
  });
  await check("rejected terminology preserves prior state and can be cleared", async () => {
    const file = join(temporary, "invalid-table.txt"); await writeFile(file, "not a terminology table");
    const dom = await send("DOM.getDocument");
    const { nodeId } = await send("DOM.querySelector", { nodeId: dom.root.nodeId, selector: "#terminology-file" });
    await send("DOM.setFileInputFiles", { nodeId, files: [file] });
    await until(async () => (await evaluate("document.getElementById('terminology-status').textContent")).includes("rejected"), "rejected table report");
    await click("#clear-terminology");
    assert.equal(await evaluate("document.getElementById('terminology-status').textContent"), "Bundled terminology only.");
  });
  await check("adapter errors cannot leave an older approved preview downloadable", async () => {
    await click("#reset"); await set("#given", "   "); await click("#build");
    assert.equal(await title(), "Export could not be built");
    assert.equal(await disabled("download-fhir"), true); assert.equal(await disabled("download-review"), true);
  });
  await click("#reset");
  await check("mobile controls fit without horizontal page overflow", async () => {
    if (await evaluate("document.getElementById('integration-details').open")) await click("#integration-details > summary");
    await send("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
    await evaluate("window.scrollTo(0,0)");
    assert.equal(await evaluate("document.documentElement.scrollWidth <= innerWidth"), true);
    assert.equal(await evaluate("document.getElementById('bpm').getBoundingClientRect().width > 60"), true);
    await screenshot("mobile.png");
  });
  await check("no external network or uncaught page errors", async () => {
    assert.deepEqual(exceptions, []);
    const origin = new URL(url).origin;
    // Chrome renders its date-picker icon as a data URL; it performs no I/O.
    assert.deepEqual(requests.filter(r => !r.startsWith(origin + "/") && !r.startsWith("blob:" + origin + "/") && !r.startsWith("data:")), []);
  });
  for (const file of files) assert.equal(sha(await readFile(join(root, file))), sourceHashes[file], `Source changed during test: ${file}`);
} catch (error) { failure = error.stack; process.exitCode = 1; }
finally {
  const report = { recordedAt: new Date().toISOString(), ...provenance, browser, status: failure ? "failed" : "passed", checks, requests, exceptions, ...(failure ? { failure } : {}) };
  await writeFile(join(output, "browser.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ status: report.status, checks: checks.length, output, ...(failure ? { failure } : {}) }, null, 2));
  socket?.close();
  if (child?.pid && child.exitCode === null) {
    child.kill("SIGTERM");
    await Promise.race([new Promise(resolve => child.once("exit", resolve)), delay(3000)]);
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  }
  server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
  await rm(temporary, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
