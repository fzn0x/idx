// node --test web/test/*.mjs. The terminal's flag parser: public/terminal.js is
// a browser module (it imports /app.js and reads the DOM), so the test lifts
// BOOL_FLAGS and flags(), toolButtons(), commandWords(), nearTable(), renderHelp()
// and the screen, brokers and market foreign renderers out of its source and
// runs them as they are.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { TOOLS, byName, whatOf } from "../public/tools.js";
import { ALIASES } from "../public/route.js";

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

test("help's usage lines wrap only between their words, as a notice's offered commands do", () => {
  const lift = (re, what) => { const m = src.match(re)?.[0]; assert.ok(m, `${what} found in terminal.js`); return m; };
  const commands = lift(/^const COMMANDS = \[[\s\S]*?^\];$/m, "COMMANDS");
  const words = lift(/^const commandWords = .*;$/m, "commandWords");
  const help = lift(/^function renderHelp\(which\) \{[\s\S]*?^\}$/m, "renderHelp()");
  // Stand-ins for app.js on the English page: an el(), a fragment and a table() that keep what they are given,
  // filling each cell as app.js's table() does (f(row), or row[k]).
  const el = (tag, attrs = {}, children = []) => ({ tag, attrs, children: [].concat(children), append(...c) { this.children.push(...c); } });
  const document = { createDocumentFragment: () => el("fragment") };
  const table = (cols, rows) => {
    const tb = { tag: "table", cols, cells: rows.map((row) => cols.map((c) => (c.f ? c.f(row) : row[c.k]))), classes: [] };
    tb.classList = { add: (c) => tb.classes.push(c) };
    return tb;
  };
  const make = new Function("document", "el", "table", "wrapScroll", "t", "ALIASES", "helpOf", "TOOLS", "toolButtons",
    `${commands}\n${words}\n${help}\nreturn { COMMANDS, renderHelp };`);
  const { COMMANDS, renderHelp } = make(document, el, table, (x) => x, (_, en) => en, ALIASES, (c) => c.help, TOOLS, () => el("div"));
  const usageCells = (which) => {
    const tb = renderHelp(which).children.find((c) => c.tag === "table");
    // The table and column the stylesheet's .tbl-help and td.use .word rules reach (checked in the offered-command test above).
    assert.ok(tb.classes.includes("tbl-help"), "the help table is a .tbl-help");
    const col = tb.cols.findIndex((c) => c.cls === "use");
    assert.ok(col >= 0, "the usage column is a td.use");
    return tb.cells.map((row) => row[col]);
  };
  // A cell's words: each kept-whole word as <span.word>, plain text as itself.
  const flat = (n) => (typeof n === "string" ? [n] : n.attrs.class === "word" ? [`<span.word>${text(n)}`] : n.children.flatMap(flat));

  const cells = usageCells();
  assert.equal(cells.length, COMMANDS.length);
  COMMANDS.forEach((c, i) => {
    // The cell reads as the usage line, set as its words, each kept whole: the spaces between them are the only places it wraps.
    assert.equal(text(cells[i]), c.usage, c.name);
    assert.deepEqual(flat(cells[i]), c.usage.split(" ").flatMap((w, j) => [...(j ? [" "] : []), `<span.word>${w}`]), c.name);
  });
  // Every word help shows is short enough to be kept whole (commandWords leaves a word over 40 characters as plain text).
  for (const c of COMMANDS) for (const w of c.usage.split(" ")) assert.ok(w.length <= 40, `${c.name}: ${w}`);
  // Rows that broke at a hyphen at 1280 px (2026-11-08): "[--" / "statement", "[--" / "reported|--estimates]".
  const one = (name) => flat(usageCells(name)[0]).filter((w) => w !== " ");
  assert.deepEqual(one("financials"), ["<span.word>financials", "<span.word><SYMBOL>", "<span.word>[annual|quarterly|ttm]", "<span.word>[--statement", "<span.word>income,balance,cash]"]);
  assert.ok(one("earnings").includes("<span.word>[--reported|--estimates]"));
  // help for an alias shows its command's row the same way.
  assert.deepEqual(one("f"), COMMANDS.find((c) => c.name === "fundamental").usage.split(" ").map((w) => `<span.word>${w}`));
});

test("the screen marks a P/B over another currency's statements and names the companies as the CLI does", () => {
  const lift = (re, what) => { const m = src.match(re)?.[0]; assert.ok(m, `${what} found in terminal.js`); return m; };
  const mixed = lift(/^const mixedCurrency = .*;$/m, "mixedCurrency");
  const ratio = lift(/^const mixedRatio = .*;$/m, "mixedRatio");
  const note = lift(/^function screenCurrencyNote\(rows\) \{[\s\S]*?^\}$/m, "screenCurrencyNote()");
  const screen = lift(/^  screen\(s\) \{[\s\S]*?^  \},$/m, "R.screen()");
  // Stand-ins for app.js on the English page: fmt with two decimals and n/a for a
  // missing value, and an el(), a fragment and a table() that keep what they are given.
  const fmt = { ratio: (v) => (v == null ? "n/a" : v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })), price: String, pct: String, compact: String, num: String };
  const el = (tag, attrs = {}, children = []) => ({ tag, attrs, children: [...children], append(...c) { this.children.push(...c); } });
  const document = { createDocumentFragment: () => el("fragment") };
  const table = (cols, rows) => ({ tag: "table", cols, rows });
  const make = new Function("fmt", "el", "document", "table", "wrapScroll", "signClass", "pct", "x", `${mixed}\n${ratio}\n${note}\nconst R = { ${screen} };\nreturn (s) => R.screen(s);`);
  const render = make(fmt, el, document, table, (t) => t, () => "", String, String);
  const shown = (rows) => {
    const page = render({ preset: null, filters: ["pe<10"], quoted: rows.length, universe: rows.length, matched_quote_level: rows.length, fundamentals_checked: 0, total_matches: rows.length, sort: "div", descending: true, rows, returns_source: null });
    const t = page.children.find((c) => c.tag === "table");
    const pb = t.cols.find((c) => c.h === "P/B");
    return { pb: t.rows.map((r) => pb.f(r)), notes: page.children.filter((c) => c.tag === "p" && c.attrs.class === "warn").map((c) => c.attrs.text) };
  };

  // 2026-11-06: BSSR and GEMS report in USD, BBCA in IDR; PTRO has no P/B (the cases of src/output/table_news.rs).
  const row = (symbol, statements, pb) => ({ symbol, name: symbol, currency: "IDR", financial_currency: statements, price_to_book: pb });
  assert.deepEqual(shown([row("BSSR", "USD", 48921.57), row("BBCA", "IDR", 2.253823), row("GEMS", "USD", 47202.797), row("PTRO", "USD", null)]), {
    pb: ["48,921.57*", "2.25", "47,202.80*", "n/a"],
    notes: ["* P/B divides the price by statement figures in another currency, so it is not comparable: BSSR, GEMS (statements in USD, price in IDR). Filters and sorts on pb and ps read such P/B and P/S as missing."],
  });
  // Same currency, or one not named: no mark and no note.
  assert.deepEqual(shown([row("BBCA", "IDR", 2.25), row("TLKM", null, 2.1)]), { pb: ["2.25", "2.10"], notes: [] });
});

test("brokers prints IPOT's member ranking with the CLI's columns, class and net included", () => {
  const body = src.match(/^ {2}marketBrokers\(r\) \{[\s\S]*?^ {2}\},$/m)?.[0];
  assert.ok(body, "R.marketBrokers() found in terminal.js");
  // Stand-ins for app.js on the English page, as above; compact as the CLI's (2.79T, 4.77B, 43.02K).
  const compact = (v) => { if (v == null) return "n/a"; const [u, s] = [[1e12, "T"], [1e9, "B"], [1e6, "M"], [1e3, "K"]].find(([x]) => Math.abs(v) >= x) || [1, ""]; return `${Number((v / u).toFixed(Math.abs(v / u) >= 100 ? 1 : 2))}${s}`; };
  const fmt = { compact, num: (v, d) => v.toFixed(d) };
  const el = (tag, attrs = {}, children = []) => ({ tag, attrs, children: [...children], append(...c) { this.children.push(...c); } });
  const document = { createDocumentFragment: () => el("fragment") };
  const table = (cols, rows) => ({ tag: "table", cols, rows });
  const signClass = (v) => (v > 0 ? "up" : v < 0 ? "dn" : "");
  const render = new Function("fmt", "el", "document", "table", "wrapScroll", "signClass", "NA", `const R = { ${body} };\nreturn (r) => R.marketBrokers(r);`)(fmt, el, document, table, (t) => t, signClass, "n/a");
  const row = { rank: 3, code: "CC", name: "MANDIRI SEKURITAS", volume: 4_508_660_000, value: 1_881_490_000_000, frequency: 211_212, value_share_pct: 9.111, buy_value: 808.3e9, sell_value: 1.07e12, net_value: -262_910_000_000, kind: "state_owned" };
  const ranking = { date: "2026-11-06", members: 88, total_volume: 9.47e10, total_value: 20_648_897_010_000, total_frequency: 2_205_570, sort: "value", rounded: true, brokers: [row] };
  const page = render(ranking);
  const t = page.children.find((c) => c.tag === "table");
  // src/output/table_exchange.rs, IPOT's rows.
  assert.deepEqual(t.cols.map((c) => c.h), ["#", "Code", "Member", "Class", "Value", "Share", "Net", "Volume", "Frequency"]);
  assert.deepEqual(t.cols.map((c) => (c.f ? c.f(row) : row[c.k])), [3, "CC", "MANDIRI SEKURITAS", "BUMN", "1.88T", "9.11%", "-262.9B", "4.51B", "211.2K"]);
  assert.equal(t.cols.find((c) => c.h === "Net").cls(row), "num dn");
  assert.equal(page.children[0].attrs.html, "<b>Exchange members</b> · 2026-11-06 · 88 active · by value · all members' value 20.65T (both sides)");
  const notes = page.children.filter((c) => c.attrs?.class === "warn").map((c) => c.attrs.text);
  assert.deepEqual(notes.slice(0, 1), ["Showing 1 of 88 members; --top 0 for all."]);
  assert.match(notes[1], /^Each member's buying and selling together, as IPOT \(Indo Premier\) publishes them, and its net \(bought minus sold; json brokers adds the value bought and sold\)/);
  assert.match(notes[1], /abbreviates figures of a million and more to two decimals/);
  // Rows without a side or a class (idx.co.id's shape) print without those columns, as the CLI does.
  const plain = render({ ...ranking, rounded: false, members: 1, brokers: [{ ...row, buy_value: null, sell_value: null, net_value: null, kind: null }] });
  assert.deepEqual(plain.children.find((c) => c.tag === "table").cols.map((c) => c.h), ["#", "Code", "Member", "Value", "Share", "Volume", "Frequency"]);
  assert.doesNotMatch(plain.children.filter((c) => c.attrs?.class === "warn").map((c) => c.attrs.text).join(" "), /abbreviates|its net/);
});

test("market foreign prints the rupiah ranking with the CLI's columns and its market line", () => {
  const body = src.match(/^ {2}marketForeign\(m\) \{[\s\S]*?^ {2}\},$/m)?.[0];
  assert.ok(body, "R.marketForeign() found in terminal.js");
  // Stand-ins for app.js on the English page, as above; compact as the CLI's (10.32T, 329.3B).
  const compact = (v) => { if (v == null) return "n/a"; const [u, s] = [[1e12, "T"], [1e9, "B"], [1e6, "M"], [1e3, "K"]].find(([x]) => Math.abs(v) >= x) || [1, ""]; return `${Number((v / u).toFixed(Math.abs(v / u) >= 100 ? 1 : 2))}${s}`; };
  const fmt = { compact, num: (v, d) => v.toFixed(d), price: (v) => (v == null ? "n/a" : v.toLocaleString("en")), pct: (v) => (v == null ? "n/a" : `${v > 0 ? "+" : ""}${v.toFixed(2)}%`) };
  const el = (tag, attrs = {}, children = []) => ({ tag, attrs, children: [...children], append(...c) { this.children.push(...c); } });
  const document = { createDocumentFragment: () => el("fragment") };
  const table = (cols, rows) => ({ tag: "table", cols, rows });
  const signClass = (v) => (v > 0 ? "up" : v < 0 ? "dn" : "");
  const render = new Function("fmt", "el", "document", "table", "wrapScroll", "signClass", "NA", `const R = { ${body} };\nreturn (m) => R.marketForeign(m);`)(fmt, el, document, table, (t) => t, signClass, "n/a");
  const row = { symbol: "BBCA", name: "BANK CENTRAL ASIA Tbk", price: 5200, change_pct: 1.46, value: 1.0e12, foreign_net_value: 219.0e9, foreign_net_pct: 21.9 };
  const m = { date: "2026-11-06", scanned: 100, checked: 98, market: { value: 10.32e12, foreign_buy_value: 3.76e12, foreign_sell_value: 3.43e12, foreign_net_value: 329.33e9 }, net_buy: [row], net_sell: [], rounded: true };
  const page = render(m);
  // src/output/table_exchange.rs, market_foreign_value.
  assert.equal(page.children[0].attrs.html, "<b>Foreign flow</b> · 2026-11-06 · the 100 most traded stocks (98 with trades) · rupiah (IPOT)");
  assert.equal(page.children[1].attrs.html, 'Market · value 10.32T · foreign buy 3.76T · sell 3.43T · net <span class="up">+329.3B</span>');
  const t = page.children.find((c) => c.tag === "table");
  assert.deepEqual(t.cols.map((c) => c.h), ["#", "Symbol", "Name", "Price", "%", "Value", "Foreign net", "Net %"]);
  assert.deepEqual(t.cols.map((c) => (c.f ? c.f(row) : row[c.k])), [1, "BBCA", "BANK CENTRAL ASIA Tbk", "5,200", "+1.46%", "1T", "+219B", "21.9%"]);
  assert.equal(t.cols.find((c) => c.h === "Foreign net").cls(row), "num up");
  const words = page.children.filter((c) => c.attrs?.class === "warn").map((c) => c.attrs.text);
  assert.deepEqual(words.slice(0, 1), ["None."], "no net selling");
  assert.match(words[1], /^Rupiah from IPOT \(Indo Premier\): each stock's foreign net is its broker summary's, across all boards, for the 100 stocks most traded by value, not every stock/);
  assert.match(words[1], /two for the market/);
  // Without totals of the session, the line says so.
  const plain = render({ ...m, market: null, rounded: false });
  assert.equal(plain.children[1].attrs.html, "Market · no totals for this session (see the warning)");
  assert.doesNotMatch(plain.children.filter((c) => c.attrs?.class === "warn").map((c) => c.attrs.text).join(" "), /abbreviates/);
});
