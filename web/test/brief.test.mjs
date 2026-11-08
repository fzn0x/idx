// node --test web/test/*.mjs. The brief as the web terminal draws it:
// public/terminal.js is a browser module (it imports /app.js and reads the
// DOM), so the tests lift R.brief, and R.fundamentals that it draws, out of
// its source and run them with stand-ins for app.js's el() and fmt.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as an from "../src/analysis.js";
import * as y from "../src/yahoo.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const src = readFileSync(join(root, "web", "public", "terminal.js"), "utf8");
// One of R's views, from `  name(arg) {` to its closing `  },`.
const view = (name, arg) => src.match(new RegExp(`^ {2}${name}\\(${arg}\\) \\{[\\s\\S]*?^ {2}\\},$`, "m"))?.[0];
const body = view("brief", "b");
// A top-level `const NAME = ...;` of terminal.js (its word tables), evaluated.
const constant = (name) => {
  const line = src.match(new RegExp(`^const ${name} = .*;$`, "m"))?.[0];
  assert.ok(line, `${name} found in terminal.js`);
  return new Function(`${line}\nreturn ${name};`)();
};

test("the brief's headlines heading agrees with the count", () => {
  assert.ok(body, "R.brief found in terminal.js");
  const el = (tag, attrs = {}, children = []) => ({ tag, attrs, children });
  const document = { createDocumentFragment: () => ({ children: [], append(...c) { this.children.push(...c); } }) };
  const fmt = { wib: (s) => `${s} WIB` };
  const brief = new Function("document", "el", "fmt", "newsTable", "SENTIMENT_WORDS", `return ({ ${body} }).brief;`)(document, el, fmt, () => el("table"), constant("SENTIMENT_WORDS"));
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

test("the verdict line under the brief's key points agrees with the input count", () => {
  const fundamentals = view("fundamentals", "f");
  assert.ok(body && fundamentals, "R.brief and R.fundamentals found in terminal.js");
  const el = (tag, attrs = {}, children = []) => ({ tag, attrs, children, append(...c) { this.children.push(...c); } });
  const document = { createDocumentFragment: () => el("fragment") };
  const text = (v) => (v == null ? "n/a" : String(v));
  const fmt = { wib: (s) => `${s} WIB`, price: text, ratio: text, compact: text, pct: text };
  const deps = { document, el, fmt, NA: "n/a", kv: () => el("dl"), pct: text, x: text, mixedCurrency: an.mixedCurrency, mixedRatio: (f, v) => text(v), inStatementCurrency: (f, v, s) => s, newsTable: () => el("table"), VERDICT_WORDS: constant("VERDICT_WORDS"), SENTIMENT_WORDS: constant("SENTIMENT_WORDS") };
  const R = new Function(...Object.keys(deps), `const R = {\n${fundamentals}\n${body}\n};\nreturn R;`)(...Object.values(deps));
  // Only the trailing P/E, as in the Rust test too_little_data.
  const f = y.parseFundamentals(JSON.parse(readFileSync(join(root, "tests", "fixtures", "yahoo", "quote_summary_fundamentals_bbca.json"), "utf8")).quoteSummary.result[0], "BBCA");
  for (const group of ["valuation", "profitability", "health", "growth", "dividend", "analyst"]) for (const k of Object.keys(f[group])) if (!(group === "valuation" && k === "pe_trailing")) f[group][k] = null;
  const find = (node, ok) => {
    if (ok(node)) return node;
    for (const c of node.children || []) { const hit = typeof c === "object" && find(c, ok); if (hit) return hit; }
    return null;
  };
  const verdictLine = () => {
    f.verdict = an.verdict(f);
    const b = { symbol: "BBCA", name: null, generated_at: "2026-11-08T01:00:00Z", key_points: [], fundamentals: f, technical: null, returns: null, ownership: null, news: null, sections_failed: [] };
    return find(R.brief(b), (n) => n.tag === "p" && n.children[0]?.attrs?.class?.startsWith("pill"))?.children[1];
  };
  assert.equal(verdictLine(), " score +1, 1 input · heuristic from public ratios, not investment advice");
  // A net margin between 5% and 20% counts without moving the score.
  f.profitability.net_margin_pct = 10;
  assert.equal(verdictLine(), " score +1, 2 inputs · heuristic from public ratios, not investment advice");
});
