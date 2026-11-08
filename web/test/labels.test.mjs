// node --test web/test/*.mjs (no network). Text the web builds from a label
// says it in the CLI's words, while the JSON keeps the label: the brief reads
// "fairly valued" and "insufficient data", its payload "fair" and
// "insufficient_data". The CLI's words are its as_str, read here from the
// Rust source; src/service.rs pins the same tables.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as an from "../src/analysis.js";
import { valuationKeyPoint } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

// What the CLI prints for each label, keyed by the JSON label.
const VERDICT_CLI = { undervalued: "undervalued", fair: "fairly valued", overvalued: "overvalued", insufficient_data: "insufficient data" };

// A Rust as_str's `Type::Variant => "words"` arms, keyed by the variant's
// snake_case name (the serde label).
function asStr(file, type) {
  const src = readFileSync(join(root, "src", "model", file), "utf8");
  const body = src.match(new RegExp(`impl ${type} \\{[\\s\\S]*?fn as_str[\\s\\S]*?\\n {4}\\}`))?.[0];
  assert.ok(body, `${type}::as_str found in src/model/${file}`);
  return Object.fromEntries([...body.matchAll(new RegExp(`${type}::(\\w+) => "([^"]+)"`, "g"))].map(([, v, words]) => [v.replace(/(?<!^)([A-Z])/g, "_$1").toLowerCase(), words]));
}

test("the CLI's words for each verdict label (VerdictLabel::as_str)", () => {
  assert.deepEqual(asStr("fundamentals.rs", "VerdictLabel"), VERDICT_CLI);
  assert.deepEqual(an.VERDICT_WORDS, VERDICT_CLI, "web/src/analysis.js");
});

test("the brief's valuation key point says the verdict in the CLI's words (service::valuation_key_point)", () => {
  for (const [label, words] of Object.entries(VERDICT_CLI)) {
    const f = { currency: "IDR", financial_currency: null, valuation: { pe_trailing: null, price_to_book: null }, verdict: { label, score: 0, reasons: [], inputs_available: 3 } };
    assert.equal(valuationKeyPoint(f), `Valuation verdict: ${words} (score +0, 3 inputs)`);
    assert.equal(f.verdict.label, label, "the JSON keeps the label");
  }
});

// ---- the terminal's views: public/terminal.js is a browser module (it
// imports /app.js and reads the DOM), so these tests lift what they need out
// of its source and run it with stand-ins for app.js.

const src = readFileSync(join(root, "web", "public", "terminal.js"), "utf8");
const lift = (re, what) => { const m = src.match(re)?.[0]; assert.ok(m, `${what} found in terminal.js`); return m; };
// A top-level `const NAME = ...;` of terminal.js, evaluated.
const constant = (name) => new Function(`${lift(new RegExp(`^const ${name} = .*;$`, "m"), name)}\nreturn ${name};`)();
// One of R's views, from `  name(arg) {` to its closing `  },`.
const view = (name, arg) => lift(new RegExp(`^ {2}${name}\\(${arg}\\) \\{[\\s\\S]*?^ {2}\\},$`, "m"), `R.${name}`);
const el = (tag, attrs = {}, children = []) => ({ tag, attrs, children: [...children], append(...c) { this.children.push(...c); } });
const text = (v) => (v == null ? "n/a" : String(v));
const fmt = { wib: (s) => `${s} WIB`, price: text, ratio: text, compact: text, pct: text };
const deps = {
  document: { createDocumentFragment: () => el("fragment") }, el, fmt, NA: "n/a", kv: () => el("dl"), pct: text, x: text,
  table: (cols, rows) => ({ tag: "table", cols, rows }), wrapScroll: (t) => t, newsTable: () => el("table"),
  mixedCurrency: an.mixedCurrency, mixedRatio: (f, v) => text(v), inStatementCurrency: (f, v, s) => s,
};
const views = (...names) => new Function(...Object.keys(deps), "VERDICT_WORDS", `return {\n${names.map(([n, a]) => view(n, a)).join("\n")}\n};`)(...Object.values(deps), constant("VERDICT_WORDS"));
const find = (node, ok) => {
  if (ok(node)) return node;
  for (const c of node.children || []) { const hit = typeof c === "object" && find(c, ok); if (hit) return hit; }
  return null;
};
const fundamentals = (label) => {
  const nulls = { market_cap: null, enterprise_value: null, pe_trailing: null, pe_forward: null, peg: null, price_to_book: null, price_to_sales: null, ev_to_ebitda: null };
  return {
    symbol: "BBCA", name: null, sector: null, industry: null, price: null, week52_low: null, week52_high: null, beta: null, currency: "IDR", financial_currency: null,
    valuation: nulls, profitability: {}, health: {}, growth: {}, dividend: {}, per_share: {}, analyst: { recommendation: null, target_mean: null }, sources: ["yahoo"],
    verdict: { label, score: 0, reasons: [], inputs_available: 3 },
  };
};

test("the terminal keeps the CLI's verdict words", () => {
  assert.deepEqual(constant("VERDICT_WORDS"), VERDICT_CLI);
});

test("the fundamentals view says the verdict in the CLI's words (table_reports::verdict_cell)", () => {
  const R = views(["fundamentals", "f"]);
  for (const [label, words] of Object.entries(VERDICT_CLI)) {
    const pill = find(R.fundamentals(fundamentals(label)), (n) => n.tag === "span" && n.attrs?.class?.startsWith("pill"));
    assert.equal(pill?.attrs.text, words, label);
  }
});
