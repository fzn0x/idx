// node --test web/test/*.mjs (no network). Static pages and Worker responses
// must carry the same security headers, and the pages must work under the CSP
// they are served with.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { SECURITY_HEADERS } from "../src/headers.js";
import worker from "../src/worker.js";

const pub = join(dirname(fileURLToPath(import.meta.url)), "..", "public");

/** public/_headers -> [{ pattern, headers: {lowercased name: value} }] */
function parseHeadersFile(text) {
  const rules = [];
  for (const line of text.replace(/\r\n?/g, "\n").split("\n")) {
    if (!line.trim() || line.trimStart().startsWith("#")) continue;
    if (!/^\s/.test(line)) {
      rules.push({ pattern: line.trim(), headers: {} });
      continue;
    }
    assert.ok(rules.length, `header line before any URL pattern: ${line}`);
    const k = line.indexOf(":");
    assert.ok(k > 0, `not a header line: ${line}`);
    const name = line.slice(0, k).trim().toLowerCase();
    assert.equal(rules.at(-1).headers[name], undefined, `${name} set twice (Cloudflare would join the values)`);
    rules.at(-1).headers[name] = line.slice(k + 1).trim();
  }
  return rules;
}

test("public/_headers gives every static asset the Worker's security headers", () => {
  const text = readFileSync(join(pub, "_headers"), "utf8");
  for (const line of text.split("\n")) assert.ok(line.length <= 2000, "Cloudflare caps a _headers line at 2,000 characters");
  const rules = parseHeadersFile(text);
  const all = rules.find((r) => r.pattern === "/*");
  assert.ok(all, "a /* rule");
  assert.deepEqual(all.headers, SECURITY_HEADERS);
  // Another rule repeating one of these names would join the values with a comma.
  for (const r of rules.filter((r) => r !== all)) {
    for (const name of Object.keys(r.headers)) assert.ok(!(name in SECURITY_HEADERS), `${r.pattern} repeats ${name}`);
  }
});

test("Worker responses carry the same security headers", async () => {
  const env = {
    PUBLIC_HOST: "ihsg.midoriresearch.com",
    ASSETS: { fetch: async () => new Response("<!doctype html>", { status: 200, headers: { "content-type": "text/html" } }) },
  };
  for (const path of ["/healthz", "/healthz-missing-page", "/api/nope"]) {
    const r = await worker.fetch(new Request(`https://ihsg.midoriresearch.com${path}`), env, {});
    for (const [k, v] of Object.entries(SECURITY_HEADERS)) assert.equal(r.headers.get(k), v, `${path} ${k}`);
  }
});

// What the CSP allows: scripts and styles from this site, Google Fonts' CSS,
// inline style attributes. An inline script or event handler would be blocked
// (a form is fine: the terminal handles submit in JS, form-action only stops
// a real submission).
const STYLE_HOSTS = ["https://fonts.googleapis.com/"];

function cspProblems(html) {
  const problems = [];
  for (const m of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
    const attrs = m[1];
    if (/type="application\/ld\+json"/.test(attrs)) continue;
    const src = attrs.match(/\bsrc="([^"]*)"/)?.[1];
    if (!src) problems.push("inline script");
    else if (!src.startsWith("/") || src.startsWith("//")) problems.push(`script from ${src}`);
  }
  for (const m of html.matchAll(/<[a-z][^>]*\s(on[a-z]+)=/gi)) problems.push(`inline handler ${m[1]}`);
  if (/href="javascript:/i.test(html)) problems.push("javascript: URL");
  for (const m of html.matchAll(/<link\b[^>]*rel="stylesheet"[^>]*>/gi)) {
    const href = m[0].match(/\bhref="([^"]*)"/)?.[1] ?? "";
    if (!(href.startsWith("/") && !href.startsWith("//")) && !STYLE_HOSTS.some((h) => href.startsWith(h))) problems.push(`stylesheet from ${href}`);
  }
  for (const m of html.matchAll(/<(?:img|source|video|audio|embed|object)\b[^>]*\b(?:src|srcset|data)="([^"]*)"/gi)) {
    if (!(m[1].startsWith("/") && !m[1].startsWith("//")) && !m[1].startsWith("data:")) problems.push(`media from ${m[1]}`);
  }
  return problems;
}

test("every page in public/ works under the CSP", () => {
  const pages = readdirSync(pub).filter((f) => f.endsWith(".html"));
  assert.ok(pages.length >= 3);
  for (const page of pages) assert.deepEqual(cspProblems(readFileSync(join(pub, page), "utf8")), [], page);
});
