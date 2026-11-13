// node --test web/test/*.mjs. The news as the web terminal draws it:
// public/terminal.js is a browser module (it imports /app.js and reads the
// DOM), so the test lifts R.news out of its source and runs it with
// stand-ins for app.js's el() and the terminal's newsTable().
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "public", "terminal.js"), "utf8");
const body = src.match(/^ {2}news\(n\) \{[\s\S]*?^ {2}\},$/m)?.[0];

test("the news headings agree with their counts (table_news::news)", () => {
  assert.ok(body, "R.news found in terminal.js");
  const el = (tag, attrs = {}, children = []) => ({ tag, attrs, children });
  const document = { createDocumentFragment: () => ({ children: [], append(...c) { this.children.push(...c); } }) };
  const news = new Function("document", "el", "newsTable", `return ({ ${body} }).news;`)(document, el, () => el("table"));
  const item = (scope, i) => ({ title: `${scope} headline ${i}`, url: `https://x.test/${scope}/${i}`, source: "kontan/investasi", published: null, sentiment: 0, scope });
  const headings = (company, market) => {
    const items = [...Array.from({ length: company }, (_, i) => item("company", i)), ...Array.from({ length: market }, (_, i) => item("market", i))];
    const n = { symbol: "BBCA", matched_name: "Bank Central Asia", sources: ["kontan/investasi"], scanned: 60, company_matches: company, company_only: false, items };
    const out = news(n).children;
    return [out.find((c) => c.tag === "p")?.attrs.words, out.find((c) => c.tag === "h3")?.attrs.text];
  };
  assert.deepEqual(headings(1, 1), ["1 of 60 scanned headlines mentions BBCA or Bank Central Asia · kontan/investasi", "Market context · 1 headline"]);
  assert.deepEqual(headings(2, 2), ["2 of 60 scanned headlines mention BBCA or Bank Central Asia · kontan/investasi", "Market context · 2 headlines"]);
});
