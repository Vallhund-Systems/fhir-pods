#!/usr/bin/env node
// Dependency-free Chrome DevTools runner. Requires Node 22+ and local Chrome.
import { createServer } from "node:http";
import { readFile, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, extname, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { setTimeout as delay } from "node:timers/promises";

const root = fileURLToPath(new URL("../", import.meta.url));
const repoRoot = fileURLToPath(new URL("../../../", import.meta.url));
const git = (...args) => execFileSync("git",args,{cwd:repoRoot,encoding:"utf8"}).trim();
const files = ["browser/run.mjs","browser/smoke.mjs","browser/operations.mjs","browser/index.html","bench/fixtures.mjs","src/index.js"];
const sha256 = data => createHash("sha256").update(data).digest("hex");
const sourceHashes = Object.fromEntries(await Promise.all(files.map(async file=>[file,sha256(await readFile(join(root,file)))])));
const provenance = {revision:git("rev-parse","HEAD"),dirtyPaths:git("status","--porcelain","--untracked-files=no").split("\n").filter(Boolean),sourceHashes,node:process.version,platform:process.platform,architecture:process.arch};
const chrome = process.env.CHROME ?? (process.platform === "darwin" ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" : "chromium");
const profile = await mkdtemp(join(tmpdir(), "fhir-pods-browser-"));
const server = createServer(async (request, response) => {
  try {
    const path = resolve(root, `.${decodeURIComponent(new URL(request.url, "http://localhost").pathname)}`);
    if (!path.startsWith(root.endsWith(sep) ? root : root + sep)) throw new Error("Path outside pod");
    const bytes = await readFile(path);
    response.setHeader("Content-Type", ({ ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".wasm": "application/wasm" })[extname(path)] ?? "application/octet-stream");
    response.end(bytes);
  } catch { response.writeHead(404).end("Not found"); }
});
let child, socket;
try {
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  child = spawn(chrome, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "about:blank"], { stdio: ["ignore", "ignore", "pipe"] });
  let launchError, log = "";
  child.on("error", (error) => { launchError = error; });
  child.stderr.on("data", (data) => { log = (log + data).slice(-5000); });
  const deadline = Date.now() + 30000;
  let port;
  while (!port && Date.now() < deadline) {
    if (launchError) throw launchError;
    if (child.exitCode !== null) throw new Error(`Chrome exited: ${log}`);
    try { port = (await readFile(join(profile, "DevToolsActivePort"), "utf8")).split("\n")[0]; } catch { await delay(100); }
  }
  if (!port) throw new Error(`Chrome startup timeout: ${log}`);
  const tabs = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  socket = new WebSocket(tabs.find((tab) => tab.type === "page").webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener("open", resolve, { once: true }); socket.addEventListener("error", reject, { once: true }); });
  const pending = new Map();
  let nextId = 0;
  socket.addEventListener("message", ({ data }) => {
    const message = JSON.parse(data);
    const operation = pending.get(message.id);
    if (!operation) return;
    pending.delete(message.id);
    if (message.error) operation.reject(new Error(JSON.stringify(message.error))); else operation.resolve(message.result);
  });
  function send(method, params = {}) {
    const id = ++nextId;
    return Promise.race([
      new Promise((resolve, reject) => { pending.set(id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params })); }),
      delay(10000, undefined, { ref: false }).then(() => { pending.delete(id); throw new Error(`CDP timeout: ${method}`); }),
    ]);
  }
  await send("Page.navigate", { url: `http://127.0.0.1:${server.address().port}/browser/index.html` });
  let report;
  while (!report && Date.now() < deadline) {
    const result = await send("Runtime.evaluate", { expression: "window.fhirPodsSmoke ?? null", returnByValue: true });
    report = result.result?.value;
    if (!report) await delay(100);
  }
  if (!report) throw new Error("Browser smoke did not finish within 30 seconds");
  const browser = await send("Browser.getVersion");
  for(const file of files) if(sha256(await readFile(join(root,file)))!==sourceHashes[file]) throw new Error(`Source changed during browser run: ${file}`);
  console.log(JSON.stringify({ recordedAt: new Date().toISOString(), browser: browser.product, ...provenance, ...report }, null, 2));
  if (report.status !== "passed") process.exitCode = 1;
} finally {
  socket?.close();
  if (child?.pid && child.exitCode === null) {
    child.kill("SIGTERM");
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), delay(3000)]);
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  }
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
  await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
