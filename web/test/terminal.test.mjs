// node --test web/test/*.mjs. The terminal's flag parser: public/terminal.js is
// a browser module (it imports /app.js and reads the DOM), so the test lifts
// BOOL_FLAGS and flags() out of its source and runs them as they are.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

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
