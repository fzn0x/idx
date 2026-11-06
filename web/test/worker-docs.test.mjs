// node --test web/test/*.mjs (no network). The Worker serves the generated
// docs: wrangler routes them through it and builds them first, and it adds
// the charset, CORS and the security headers.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import worker from "../src/worker.js";
import { SECURITY_HEADERS } from "../src/headers.js";

const web = join(dirname(fileURLToPath(import.meta.url)), "..");
const HOST = "ihsg.midoriresearch.com";

// The assets binding: answers every path with `status`, as the asset layer
// would (static files carry no charset).
function env(body = "# x\n", type = "text/markdown", status = 200) {
  return {
    PUBLIC_HOST: HOST,
    ASSETS: { fetch: async () => new Response(status === 304 ? null : body, { status, headers: { "content-type": type, etag: '"1"' } }) },
  };
}
const get = (path, e) => worker.fetch(new Request(`https://${HOST}${path}`), e, {});

test("Markdown and llms files get a charset, CORS and the security headers", async () => {
  for (const [path, type] of [["/docs/mcp.md", "text/markdown; charset=utf-8"], ["/docs/index.md", "text/markdown; charset=utf-8"], ["/llms.txt", "text/plain; charset=utf-8"], ["/llms-full.txt", "text/plain; charset=utf-8"]]) {
    for (const status of [200, 304]) {
      const r = await get(path, env("x", "text/plain", status));
      assert.equal(r.status, status, path);
      assert.equal(r.headers.get("content-type"), type, `${path} ${status}`);
      assert.equal(r.headers.get("access-control-allow-origin"), "*", path);
      assert.equal(r.headers.get("etag"), '"1"', path);
      for (const [k, v] of Object.entries(SECURITY_HEADERS)) assert.equal(r.headers.get(k), v, `${path} ${k}`);
    }
  }
});

test("HTML docs pages keep their type and get the security headers; misses stay misses", async () => {
  const r = await get("/docs/mcp", env("<!doctype html>", "text/html"));
  assert.equal(r.headers.get("content-type"), "text/html");
  assert.match(r.headers.get("content-security-policy"), /script-src 'self'/);
  assert.equal(r.headers.get("access-control-allow-origin"), null);
  // A missing .md is the 404 page: no Markdown type, no CORS, but the headers.
  const miss = await get("/docs/nope.md", env("<!doctype html>", "text/html", 404));
  assert.equal(miss.status, 404);
  assert.equal(miss.headers.get("content-type"), "text/html");
  assert.equal(miss.headers.get("access-control-allow-origin"), null);
  assert.equal(miss.headers.get("x-frame-options"), "DENY");
  const other = await worker.fetch(new Request("https://idx-web.example.workers.dev/llms.txt"), env(), {});
  assert.equal(other.status, 404);
});

test("wrangler routes the docs through the Worker and builds them first", () => {
  const toml = readFileSync(join(web, "wrangler.toml"), "utf8").replace(/\r\n?/g, "\n");
  const first = toml.match(/^run_worker_first = \[([^\]]*)\]/m)?.[1] ?? "";
  for (const p of ["/api/*", "/healthz", "/docs", "/docs/*", "/llms.txt", "/llms-full.txt"]) assert.ok(first.includes(`"${p}"`), p);
  assert.match(toml, /^\[build\]\ncommand = "node scripts\/docs\.mjs"$/m);
  // The generated files never reach git.
  const ignore = readFileSync(join(web, "..", ".gitignore"), "utf8").split(/\r?\n/);
  for (const p of ["/web/public/docs/", "/web/public/llms.txt", "/web/public/llms-full.txt"]) assert.ok(ignore.includes(p), `.gitignore ${p}`);
});
