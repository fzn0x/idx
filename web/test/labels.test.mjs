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
