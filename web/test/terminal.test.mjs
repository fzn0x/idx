// node --test web/test/*.mjs. The terminal's flag parser: public/terminal.js is
// a browser module (it imports /app.js and reads the DOM), so the test lifts
// BOOL_FLAGS and flags(), toolButtons(), commandWords() and nearTable() out of its source and runs them as they are.
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

/** commandWords() and nearTable() as they are, with stand-ins for app.js's el(), table() and wrapScroll(), and the Indonesian page's here() and t(). */
function liftNear() {
  const words = src.match(/^const commandWords = .*;$/m)?.[0];
  const near = src.match(/^function nearTable\(list\) \{[\s\S]*?^\}$/m)?.[0];
  assert.ok(words && near, "commandWords() and nearTable() found in terminal.js");
  const el = (tag, attrs = {}, children = []) => ({ tag, attrs, children: [].concat(children) });
  const table = (cols, rows) => ({ cells: rows.map((row) => cols.map((c) => c.f(row))), classList: { add() {} } });
  return new Function("el", "table", "wrapScroll", "here", "t", `${words}\n${near}\nreturn { commandWords, nearTable };`)(el, table, (x) => x, (p) => p, (id) => id);
}
const text = (n) => (typeof n === "string" ? n : n.attrs.text ?? n.children.map(text).join(""));

test("an offered command wraps only between its words, and its link runs the whole line", () => {
  const { nearTable } = liftNear();
  const lines = ["bandar screen --preset foreign-accumulation", "json ownership changes 2026-08-31 2026-09-30", 'json ownership entity "Dwimuria Investama"', "members"];
  const { cells } = nearTable(lines.map((line) => ({ line, id: "", en: "" })));
  lines.forEach((line, i) => {
    const a = cells[i][0];
    // The link runs, and reads as, the whole line...
    assert.equal(a.attrs.href, `/terminal?cmd=${encodeURIComponent(line)}`, line);
    assert.equal(text(a), line);
    // ...set as its words, each kept whole, so the spaces between them are the only places it wraps.
    assert.deepEqual(a.children.filter((c) => typeof c !== "string").map((c) => [c.tag, c.attrs.class, text(c)]), line.split(" ").map((w) => ["span", "word", w]), line);
    assert.ok(a.children.filter((c) => typeof c === "string").every((c) => c === " "), line);
  });
  // The stylesheet keeps each word whole; on a phone a word wider than the screen wraps inside its own box.
  const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "public", "styles.css"), "utf8");
  // A one-line rule's declarations, by its exact selector.
  const rule = (block, selector) => block.split("\n").map((l) => l.trim()).find((l) => l.startsWith(`${selector} { `) && l.endsWith(" }"))?.slice(selector.length + 3, -2) ?? "";
  assert.equal(rule(css, ".result .tbl td.use .word"), "white-space: nowrap;");
  const phone = [...css.matchAll(/^@media \(max-width: 48rem\) \{[\s\S]*?^\}$/gm)].map((m) => m[0]).join("\n");
  const fallback = rule(phone, ".result .tbl-help td.use .word");
  for (const d of ["display: inline-block;", "max-width: 100%;", "white-space: normal;", "text-decoration: inherit;"]) assert.ok(fallback.includes(d), `phone .word rule has ${d}`);
});

test("a word over 40 characters stays plain text, so it breaks at its hyphens instead of widening the table", () => {
  const { commandWords, nearTable } = liftNear();
  // Only a typed word gets this long (route.js's own are at most 20 characters, foreign-accumulation). Kept whole, a word
  // makes the command column at least as wide as itself: at 769 px the 62-character one below made the table 798 px wide
  // in its 719 px sheet, scrolling sideways behind the cue. Left as plain text it may break at its hyphens, as before.
  const forty = "dividend-aristocrats-with-low-debt-ratio";
  const long = "dividend-aristocrats-with-low-debt-to-equity-and-rising-payout";
  assert.equal(forty.length, 40);
  const shape = (line) => commandWords(line).map((c) => (typeof c === "string" ? c : `<${c.tag}.${c.attrs.class}>${text(c)}`));
  assert.deepEqual(shape(`screen --preset ${forty}`), ["<span.word>screen", " ", "<span.word>--preset", " ", `<span.word>${forty}`]);
  assert.deepEqual(shape(`screen --preset ${forty}s`), ["<span.word>screen", " ", "<span.word>--preset", " ", `${forty}s`]);
  assert.deepEqual(shape(`json screen --preset ${long} --sort div`), ["<span.word>json", " ", "<span.word>screen", " ", "<span.word>--preset", " ", long, " ", "<span.word>--sort", " ", "<span.word>div"]);
  // The link still reads as, and runs, the whole line.
  const line = `screen --preset ${long}`;
  const a = nearTable([{ line, id: "", en: "" }]).cells[0][0];
  assert.equal(a.attrs.href, `/terminal?cmd=${encodeURIComponent(line)}`);
  assert.equal(text(a), line);
});
