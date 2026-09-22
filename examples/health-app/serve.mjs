#!/usr/bin/env node
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

const root = new URL("../../", import.meta.url);
// Serve only files used by the example, never the repository's working files.
const routes = new Map([
  ...["index.html", "style.css", "app.mjs", "integration.mjs", "README.md"].map(file => [`/examples/health-app/${file}`, [`examples/health-app/${file}`, file.endsWith(".html") ? "text/html" : file.endsWith(".css") ? "text/css" : file.endsWith(".md") ? "text/plain" : "text/javascript"]]),
  ["/pods/wasm/src/index.js", ["pods/wasm/src/index.js", "text/javascript"]],
  ["/pods/wasm/dist/fhir_pods.wasm", ["pods/wasm/dist/fhir_pods.wasm", "application/wasm"]],
]);

export async function startServer(port = 0, { wasm } = {}) {
  const server = createServer(async (request, response) => {
    const pathname = new URL(request.url, "http://localhost").pathname;
    if (["/", "/examples/health-app", "/examples/health-app/"].includes(pathname)) {
      response.writeHead(302, { Location: "/examples/health-app/index.html" }).end(); return;
    }
    const route = routes.get(pathname);
    if (!route || !["GET", "HEAD"].includes(request.method)) { response.writeHead(404).end("Not found"); return; }
    try {
      const body = pathname.endsWith(".wasm") && wasm ? wasm : await readFile(new URL(route[0], root));
      response.writeHead(200, { "Content-Type": route[1], "Cache-Control": "no-store" });
      response.end(request.method === "HEAD" ? undefined : body);
    } catch { response.writeHead(404).end("File missing. Build the WASM artifact before opening this example."); }
  });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(port, "127.0.0.1", resolve); });
  return { server, url: `http://127.0.0.1:${server.address().port}/examples/health-app/index.html` };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const port = Number(process.env.PORT ?? 4173);
  if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error("PORT must be between 0 and 65535.");
  const { url } = await startServer(port);
  console.log(`Reference app: ${url}\nSynthetic data; local downloads. Ctrl-C to stop.`);
}
