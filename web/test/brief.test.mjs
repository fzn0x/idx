// node --test web/test/*.mjs. The brief as the web terminal draws it:
// public/terminal.js is a browser module (it imports /app.js and reads the
// DOM), so the test lifts R.brief out of its source and runs it with
// stand-ins for app.js's el() and fmt.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "public", "terminal.js"), "utf8");
const body = src.match(/^ {2}brief\(b\) \{[\s\S]*?^ {2}\},$/m)?.[0];

test("the brief's headlines heading agrees with the count", () => {
  assert.ok(body, "R.brief found in terminal.js");
  const el = (tag, attrs = {}, children = []) => ({ tag, attrs, children });
  const document = { createDocumentFragment: () => ({ children: [], append(...c) { this.children.push(...c); } }) };
  const fmt = { wib: (s) => `${s} WIB` };
  const brief = new Function("document", "el", "fmt", "newsTable", `return ({ ${body} }).brief;`)(document, el, fmt, () => el("table"));
  const heading = (company_matches) => {
    const b = {
      symbol: "BBCA", name: null, generated_at: "2026-11-08T01:00:00Z", key_points: [], fundamentals: null, technical: null, returns: null, ownership: null,
      news: { company_matches, sentiment: "positive", score: 0.5, headlines: [{ title: "Laba BBCA naik" }] }, sections_failed: [],
    };
    return brief(b).children.find((c) => c.tag === "h3")?.attrs.text;
  };
  assert.equal(heading(1), "Headlines · 1 mentions the company · sentiment positive");
  assert.equal(heading(2), "Headlines · 2 mention the company · sentiment positive");
});
