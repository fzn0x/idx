// node --test web/test/*.mjs. The terminal's flag parser: public/terminal.js is
// a browser module (it imports /app.js and reads the DOM), so the test lifts
// BOOL_FLAGS and flags(), and toolButtons(), out of its source and runs them as they are.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { TOOLS, byName, whatOf } from "../public/tools.js";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "public", "terminal.js"), "utf8");
const bools = src.match(/^const BOOL_FLAGS = .*;$/m)?.[0];
const fn = src.match(/^function flags\(toks\) \{[\s\S]*?^\}$/m)?.[0];
const flags = new Function(`${bools}\n${fn}\nreturn flags;`)();
const parse = (line) => flags(line.split(" "));

test("switches never swallow the next word", () => {
  assert.ok(bools && fn, "BOOL_FLAGS and flags() found in terminal.js");
  assert.deepEqual(parse("--preset low-pe --returns mcap>10T"), { pos: ["mcap>10T"], opt: { preset: "low-pe", returns: true } });
  assert.deepEqual(parse("--preset 52w-high --desc mcap>10T pe<15"), { pos: ["mcap>10T", "pe<15"], opt: { preset: "52w-high", desc: true } });
  assert.deepEqual(parse("--asc pe<10 --sort pe"), { pos: ["pe<10"], opt: { asc: true, sort: "pe" } });
  assert.deepEqual(parse("--all BBCA"), { pos: ["BBCA"], opt: { all: true } });
  assert.deepEqual(parse("--daily BBCA --period 1m"), { pos: ["BBCA"], opt: { daily: true, period: "1m" } });
  assert.deepEqual(parse("--company-only TLKM"), { pos: ["TLKM"], opt: { "company-only": true } });
  assert.deepEqual(parse("--upcoming 2026"), { pos: ["2026"], opt: { upcoming: true } });
});

test("value flags still take the next word, and --key=value always works", () => {
  assert.deepEqual(parse("pe<10 --sort div --limit 5"), { pos: ["pe<10"], opt: { sort: "div", limit: "5" } });
  assert.deepEqual(parse("--returns=1 --limit=0"), { pos: [], opt: { returns: "1", limit: "0" } });
  assert.deepEqual(parse("--sort"), { pos: [], opt: { sort: true } });
});

test("the tool buttons say what each tool gives in the page's language", () => {
  const fn = src.match(/^function toolButtons\(list = TOOLS\) \{[\s\S]*?^\}$/m)?.[0];
  assert.ok(fn, "toolButtons() found in terminal.js");
  // Stand-ins for app.js's el() and the page's language: the Indonesian page here.
  const el = (tag, attrs = {}) => ({ tag, attrs, children: [], append(...c) { this.children.push(...c); } });
  const make = new Function("TOOLS", "whatOf", "t", "CLI_ONLY", "el", "runLine", `${fn}\nreturn toolButtons;`);
  const toolButtons = make(TOOLS, (tool) => whatOf(tool, "id"), (id) => id, "hanya CLI dan MCP", el, () => {});
  const title = (name) => toolButtons().children.find((b) => b.attrs.text === name).attrs.title;
  assert.equal(title("idx_orderbook"), "antrean beli dan jual waktu nyata dan aliran asing (sesi Stockbit Anda) · hanya CLI dan MCP");
  for (const tool of TOOLS.filter((x) => !x.web)) assert.ok(title(tool.tool).startsWith(`${whatOf(tool, "id")} · `), tool.tool);
  assert.equal(title("idx_quote"), `${byName("idx_quote").what} · menjalankan: quote BBCA`);
});
