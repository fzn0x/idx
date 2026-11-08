// node --test web/test/*.mjs. A notice from public/route.js hands the user a CLI
// line to copy, so every such line must be one the CLI accepts. This test reads
// the CLI's clap definitions (src/cli/*.rs) into a model of its command tree:
// subcommands and their aliases, flags with their short names and aliases,
// whether each takes a value, positionals, required arguments and required
// groups. It then reads each notice's CLI line the way clap parses it, so a line
// the CLI would refuse with a usage error fails here.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { routeLine, CALENDAR_TYPES, STOCKBIT_KINDS } from "../public/route.js";
import { TOOLS } from "../public/tools.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (...p) => readFileSync(join(root, ...p), "utf8");
const FILES = Object.fromEntries(readdirSync(join(root, "src", "cli")).filter((f) => f.endsWith(".rs")).map((f) => [f.slice(0, -3), read("src", "cli", f)]));

// ---- a Rust lexer, enough for clap's derive input -----------------------------

/** Tokens of Rust source: attributes whole (`#[...]`), literals, words (paths such as a::B) and punctuation; comments dropped. */
function lex(src) {
  const toks = [];
  const n = src.length;
  // The end of a string or char literal starting at j, or -1.
  const literal = (j) => {
    if (src[j] === '"') { j++; while (j < n && src[j] !== '"') j += src[j] === "\\" ? 2 : 1; return j + 1; }
    const raw = src.slice(j, j + 12).match(/^r(#*)"/);
    if (raw) return src.indexOf(`"${raw[1]}`, j + raw[0].length) + 1 + raw[1].length;
    if (src[j] === "'" && src[j + 1] !== "\\" && src[j + 2] === "'") return j + 3;
    if (src[j] === "'" && src[j + 1] === "\\" && src[j + 3] === "'") return j + 4;
    return -1;
  };
  for (let i = 0; i < n;) {
    if (/\s/.test(src[i])) { i++; continue; }
    if (src.startsWith("//", i)) { const e = src.indexOf("\n", i); i = e < 0 ? n : e; continue; }
    if (src.startsWith("/*", i)) { i = src.indexOf("*/", i) + 2; continue; }
    if (src.startsWith("#[", i) || src.startsWith("#![", i)) {
      let j = src.indexOf("[", i) + 1, depth = 1;
      while (j < n && depth) {
        const e = literal(j);
        if (e > 0) { j = e; continue; }
        if (src[j] === "[") depth++;
        else if (src[j] === "]") depth--;
        j++;
      }
      toks.push({ t: "attr", v: src.slice(i, j) });
      i = j;
      continue;
    }
    const e = literal(i);
    if (e > 0) { toks.push({ t: "lit", v: src.slice(i, e) }); i = e; continue; }
    const w = src.slice(i, i + 200).match(/^[A-Za-z_]\w*(?:::[A-Za-z_]\w*)*/);
    if (w) { toks.push({ t: "word", v: w[0] }); i += w[0].length; continue; }
    toks.push({ t: "punct", v: src[i] });
    i++;
  }
  return toks;
}

/** The index of the token that closes the one at `k` ({ or (). */
function closing(toks, k) {
  const open = toks[k].v, close = open === "{" ? "}" : ")";
  for (let depth = 0, e = k; e < toks.length; e++) {
    if (toks[e].v === open) depth++;
    else if (toks[e].v === close && !--depth) return e;
  }
  throw new Error(`no ${close}`);
}

/** Every struct and enum with a braced body, by name: [{ file, kind, attrs, body }]. */
const DEFS = new Map();
for (const [file, src] of Object.entries(FILES)) {
  const toks = lex(src);
  for (let k = 0; k + 2 < toks.length; k++) {
    if (toks[k].t !== "word" || !["enum", "struct"].includes(toks[k].v) || toks[k + 1].t !== "word" || toks[k + 2].v !== "{") continue;
    const end = closing(toks, k + 2);
    const attrs = [];
    for (let b = k - 1; b >= 0 && (toks[b].t === "attr" || ["pub", "(", ")", "crate"].includes(toks[b].v)); b--) if (toks[b].t === "attr") attrs.unshift(toks[b].v);
    const list = DEFS.get(toks[k + 1].v) || [];
    list.push({ file, kind: toks[k].v, attrs, body: toks.slice(k + 3, end) });
    DEFS.set(toks[k + 1].v, list);
  }
}

/** A type named in `file` (stocks::StocksCmd, ImportArgs): the module named first, else the same file, else any. */
function resolve(type, file) {
  const parts = type.split("::"), list = DEFS.get(parts.at(-1)) || [];
  const def = list.find((d) => parts.length > 1 && d.file === parts.at(-2)) || list.find((d) => d.file === file) || list[0];
  assert.ok(def, `${type} (from src/cli/${file}.rs) is defined in src/cli/`);
  return def;
}

/** A struct body's fields: [{ name, type, attrs }]. */
function fieldsOf(body) {
  const out = [];
  let attrs = [];
  for (let k = 0; k < body.length; k++) {
    const tk = body[k];
    if (tk.t === "attr") { attrs.push(tk.v); continue; }
    if (tk.v === "pub" && body[k + 1]?.v === "(") { k = closing(body, k + 1); continue; }
    if (tk.t !== "word" || tk.v === "pub" || body[k + 1]?.v !== ":") continue;
    const type = [];
    let e = k + 2;
    for (let depth = 0; e < body.length; e++) {
      const v = body[e].v;
      if (v === "<") depth++;
      else if (v === ">") depth--;
      else if (v === "," && !depth) break;
      type.push(v);
    }
    out.push({ name: tk.v, type: type.join(""), attrs });
    attrs = [];
    k = e;
  }
  return out;
}

/** An enum body's variants: [{ name, attrs, fields?, type? }]. */
function variantsOf(body) {
  const out = [];
  let attrs = [];
  for (let k = 0; k < body.length; k++) {
    const tk = body[k];
    if (tk.t === "attr") { attrs.push(tk.v); continue; }
    if (tk.t !== "word") continue;
    const v = { name: tk.v, attrs };
    attrs = [];
    if (body[k + 1]?.v === "{") { const e = closing(body, k + 1); v.fields = fieldsOf(body.slice(k + 2, e)); k = e; }
    else if (body[k + 1]?.v === "(") { const e = closing(body, k + 1); v.type = body.slice(k + 2, e).map((x) => x.v).join(""); k = e; }
    out.push(v);
  }
  return out;
}

// ---- the command tree --------------------------------------------------------------

const kebab = (s) => s.replace(/([a-z0-9])([A-Z])/g, "$1-$2").replace(/_/g, "-").toLowerCase();
/** An attribute's text without its string contents, so a word inside help text is never read as a setting. */
const bare = (a) => a.replace(/"(?:[^"\\]|\\.)*"/g, '""');
const strings = (a, key) => [...a.matchAll(new RegExp(`\\b${key}\\s*=\\s*"([^"]+)"`, "g"))].map((m) => m[1]);
const blank = () => ({ subs: new Map(), subRequired: false, longs: new Map(), shorts: new Map(), args: [], pos: [], groups: [] });

function argOf(f, attr) {
  const plain = bare(attr), ty = f.type;
  const count = /ArgAction::Count/.test(plain), sw = ty === "bool" || count;
  const long = strings(attr, "long")[0] || (/\blong\b(?!\s*=)/.test(plain) ? kebab(f.name) : null);
  const short = attr.match(/\bshort\s*=\s*'(.)'/)?.[1] || (/\bshort\b(?!\s*=)/.test(plain) ? f.name[0] : null);
  const optional = sw || /^(Option|Vec)</.test(ty) || /\bdefault_value(_t)?\b/.test(plain);
  const unless = strings(attr, "required_unless_present")[0] || null;
  const list = (key) => [...strings(attr, key), ...(attr.match(new RegExp(`\\b${key}_all\\s*=\\s*\\[([^\\]]*)\\]`))?.[1].match(/"[^"]+"/g) || []).map((s) => s.slice(1, -1))];
  return {
    id: f.name, long, short, aliases: [...strings(attr, "alias"), ...strings(attr, "visible_alias")],
    takes: !sw, multiple: count || /^Vec</.test(ty) || /\bnum_args\b/.test(plain),
    required: /\brequired\s*=\s*true\b/.test(plain) || !optional || !!unless, unless,
    requires: strings(attr, "requires"), conflicts: list("conflicts_with"),
    global: /\bglobal\s*=\s*true\b/.test(plain),
  };
}

function addArg(node, a) {
  node.args.push(a);
  if (!a.long && !a.short) { node.pos.push(a); return; }
  for (const name of [a.long, ...a.aliases].filter(Boolean)) node.longs.set(name, a);
  if (a.short) node.shorts.set(a.short, a);
}

function addFields(node, fields, file) {
  for (const f of fields) {
    const cmd = f.attrs.filter((a) => a.startsWith("#[command")).map(bare).join(" ");
    if (/\bsubcommand\b/.test(cmd)) {
      const inner = f.type.match(/^Option<(.+)>$/);
      node.subs = enumNode(resolve(inner ? inner[1] : f.type, file)).subs;
      node.subRequired = !inner;
    } else if (/\bflatten\b/.test(cmd)) {
      for (const a of structNode(resolve(f.type, file)).args) addArg(node, a);
    } else {
      addArg(node, argOf(f, f.attrs.filter((a) => a.startsWith("#[arg")).join(" ")));
    }
  }
}

function structNode(def) {
  const node = blank();
  addFields(node, fieldsOf(def.body), def.file);
  for (const a of def.attrs) {
    const g = a.match(/ArgGroup::new\("[^"]+"\)([\s\S]*)/);
    // An ArgGroup takes one of its arguments unless it says .multiple(true).
    if (g) node.groups.push({ required: /\.required\(true\)/.test(g[1]), multiple: /\.multiple\(true\)/.test(g[1]), ids: (g[1].match(/\.args\(\[([^\]]*)\]\)/)?.[1].match(/"[^"]+"/g) || []).map((s) => s.slice(1, -1)) });
  }
  return node;
}

function enumNode(def) {
  const node = blank();
  node.subRequired = true;
  for (const v of variantsOf(def.body)) {
    const cmd = v.attrs.filter((a) => a.startsWith("#[command")).join(" ").replace(/\b(after_help|long_about|about)\s*=\s*"(?:[^"\\]|\\.)*"/g, "");
    let sub = blank();
    if (v.fields) addFields(sub, v.fields, def.file);
    else if (v.type) { const d = resolve(v.type, def.file); sub = d.kind === "enum" ? enumNode(d) : structNode(d); }
    for (const name of [strings(cmd, "name")[0] || kebab(v.name), ...strings(cmd, "alias"), ...strings(cmd, "visible_alias")]) node.subs.set(name, sub);
  }
  return node;
}

const ROOT = structNode(resolve("Cli", "mod"));
const GLOBALS = ROOT.args.filter((a) => a.global);

/** A shell line's words, as sh splits them: quotes kept together, '\'' and backslash escapes undone. */
function shellWords(line) {
  const out = [];
  let cur = null;
  for (let i = 0; i < line.length;) {
    const c = line[i];
    if (/\s/.test(c)) { if (cur != null) out.push(cur); cur = null; i++; continue; }
    cur ??= "";
    if (c === "'") { const e = line.indexOf("'", i + 1); assert.ok(e > i, `closed quote in ${line}`); cur += line.slice(i + 1, e); i = e + 1; }
    else if (c === '"') {
      for (i++; i < line.length && line[i] !== '"'; i++) cur += line[i] === "\\" && /["\\$`]/.test(line[i + 1]) ? line[++i] : line[i];
      assert.ok(i < line.length, `closed quote in ${line}`);
      i++;
    } else if (c === "\\") { cur += line[i + 1]; i += 2; }
    else { cur += c; i++; }
  }
  if (cur != null) out.push(cur);
  return out;
}

/** What clap says about a CLI line: null when it parses (or asks for help), else the usage error. */
export function clapError(line) {
  const words = shellWords(line);
  if (words[0] !== "idx") return `not an idx line: ${line}`;
  let node = ROOT, seen = new Set(), pos = [];
  const seenGlobal = new Set();
  const present = (id) => [...seen, ...seenGlobal].some((a) => a.id === id) || node.pos.some((p, i) => p.id === id && i < pos.length);
  const done = () => {
    let k = 0;
    for (const p of node.pos) {
      if (k >= pos.length) { if (p.required && !(p.unless && present(p.unless))) return `the following required arguments were not provided: <${p.id.toUpperCase()}>`; continue; }
      k += p.multiple ? pos.length - k : 1;
    }
    if (k < pos.length) return `unexpected argument '${pos[k]}' found`;
    for (const a of node.args) if (a.required && (a.long || a.short) && !seen.has(a) && !(a.unless && present(a.unless))) return `the following required arguments were not provided: --${a.long}`;
    for (const a of [...GLOBALS, ...node.args].filter((x) => present(x.id))) {
      const other = a.conflicts.find(present);
      if (other) return `the argument '${a.id}' cannot be used with '${other}'`;
      // clap does not require an argument that a present one conflicts with.
      const missing = a.requires.find((id) => !present(id) && !node.args.some((x) => present(x.id) && x.conflicts.includes(id)));
      if (missing) return `the following required arguments were not provided: ${missing}`;
    }
    for (const g of node.groups) {
      if (g.required && !g.ids.some(present)) return `the following required arguments were not provided: <${g.ids.join("|")}>`;
      if (!g.multiple && g.ids.filter(present).length > 1) return `the arguments ${g.ids.filter(present).join(", ")} cannot be used together`;
    }
    if (node.subs.size && node.subRequired) return "requires a subcommand but one was not provided";
    return null;
  };
  const use = (a, w) => {
    const set = GLOBALS.includes(a) ? seenGlobal : seen;
    if (set.has(a) && !a.multiple) return `the argument '${w}' cannot be used multiple times`;
    set.add(a);
    return null;
  };
  for (let i = 1; i < words.length; i++) {
    const w = words[i];
    if (["--help", "-h", "--version", "-V"].includes(w)) return null;
    if (/^--./.test(w)) {
      const [name, ...v] = w.slice(2).split("=");
      const a = node.longs.get(name) || GLOBALS.find((g) => g.long === name);
      if (!a) return `unexpected argument '${w}' found`;
      if (a.takes && !v.length && (i + 1 >= words.length || /^-./.test(words[i + 1]))) return `a value is required for '${w}'`;
      if (!a.takes && v.length) return `unexpected value for '${w}'`;
      if (a.takes && !v.length) i++;
      const err = use(a, w);
      if (err) return err;
      continue;
    }
    if (/^-./.test(w)) {
      for (let j = 1; j < w.length; j++) {
        const a = node.shorts.get(w[j]) || GLOBALS.find((g) => g.short === w[j]);
        if (!a) return `unexpected argument '-${w[j]}' found`;
        const err = use(a, `-${w[j]}`);
        if (err) return err;
        if (!a.takes) continue;
        if (j + 1 === w.length) { if (i + 1 >= words.length || /^-./.test(words[i + 1])) return `a value is required for '-${w[j]}'`; i++; }
        break;
      }
      continue;
    }
    if (node.subs.size) {
      const sub = node.subs.get(w);
      if (!sub) return `unrecognized subcommand '${w}'`;
      node = sub;
      seen = new Set();
      pos = [];
      continue;
    }
    pos.push(w);
  }
  return done();
}

// ---- the lines ----------------------------------------------------------------------

test("the model reads the CLI's command tree as clap does", () => {
  for (const line of [
    "idx stocks quote BBCA", "idx -o json stocks quote BBCA BBRI", "idx stocks history BBCA -p 5y -i 1wk", "idx --offline market overview -n 5",
    "idx -o csv bandar detect BBCA", "idx watch", "idx watch add BBCA", "idx watch rm BBCA", "idx stockbit login --timeout 600",
    "idx --timeout 600 stockbit login", "idx stockbit orderbook BBCA -d 20", "idx ownership changes --from 2026-08-31 --to 2026-09-30 -s BBCA",
    "idx ownership entity \"Dwimuria Investama\"", "idx ownership graph Anthoni Salim --holder", "idx ownership cross-holders --min-tickers 5",
    "idx ownership import --latest --family composition", "idx mcp setup --list", "idx mcp setup claude-code", "idx completions zsh",
    "idx config get", "idx config set provider yahoo", "idx cache stats", "idx schema", "idx version", "idx stocks earnings BBCA --history",
    "idx stocks news BBCA -s kontan -s cnbc", "idx -vv stocks quote BBCA", "idx -ojson stocks quote BBCA", "idx stocks screen -f 'pe<10'",
    "idx market calendar --type rups,dividend --symbol BBCA", "idx stocks returns ASII --benchmark none", "idx config --help", "idx stocks quote --version",
  ]) assert.equal(clapError(line), null, line);
  for (const [line, why] of [
    ["idx bandar BBCA", /unrecognized subcommand 'BBCA'/], ["idx own ticker BBCA", /unrecognized subcommand 'own'/], ["idx watch BBCA", /unrecognized subcommand 'BBCA'/],
    ["idx stockbit orderbook", /required arguments/], ["idx completions", /required arguments/], ["idx stocks quote", /required arguments/],
    ["idx ownership concentration --asc BBCA", /unexpected argument '--asc'/], ["idx stocks screen pe<10", /unexpected argument 'pe<10'/],
    ["idx stocks history BBCA 5y 1wk", /unexpected argument '5y'/], ["idx ownership changes 2026-08-31 2026-09-30", /unexpected argument '2026-08-31'/],
    ["idx ownership entity Dwimuria Investama", /unexpected argument 'Investama'/], ["idx market holidays 2027", /unexpected argument '2027'/],
    ["idx ownership import", /required arguments/], ["idx ownership import --family composition", /required arguments/], ["idx mcp setup", /required arguments/],
    ["idx config", /requires a subcommand/], ["idx cache", /requires a subcommand/], ["idx ownership resolve", /requires a subcommand/], ["idx stocks", /requires a subcommand/],
    ["idx ownership build-snapshot", /required arguments/], ["idx watch add", /required arguments/], ["idx stocks screen -f a -f b", /multiple times/],
    ["idx stocks quote BBCA --foo", /unexpected argument '--foo'/], ["idx stocks history BBCA --period", /value is required/],
  ]) assert.match(clapError(line) || "parsed", why, line);
});

/** Page commands typed with a CLI flag the page has no twin for, in the page's words and the CLI's. */
const PAGE_LINES = [
  // The page's own forms with a CLI-only global flag.
  ...["quote BBCA", "q BBCA,BBRI", "history BBCA 5y 1wk", "h BBCA --period 5d --interval 1h", "intraday BBCA 5m", "profile BBCA", "financials TLKM quarterly",
    "financials TLKM --statement income,cash", "earnings BBCA --annual --reported", "fundamental BBCA --lens risk", "growth BBCA", "valuation BBCA", "risk TLKM",
    "technical TLKM 2y", "compare BBCA BBRI", "screen pe<10,div>5 --sort div --limit 10", "screen pe<10 div>5", "screen --preset low-pe --returns", "news",
    "news TLKM --limit 5 --company-only", "dividends TLKM", "returns BBCA BBRI --benchmark none", "brief BBCA", "sentiment BBRI", "insights BBCA --all",
    "broker BBCA,BBRI --investor foreign --board regular", "broker BBCA --date 2026-10-16", "foreign BBCA --days 5", "foreign BBCA --from 2026-10-01 --to 2026-10-16",
    "bandar BBCA", "bandar BBCA,BBRI --period 1m --top 5", "bandar detect BBCA --daily", "bandar screen --preset accumulation --filter acc>=5 --sort net --asc",
    "bandar broker YP --period 1m --limit 5", "actions BBCA --since 2025-01-01", "market", "holidays", "holidays 2027", "holidays --upcoming 2027", "astro",
    "astro 2026-12-24 --backtest --period 2y", "astro --natal base --days 7", "list", "list --search bank", "list bank", "members", "members mandiri",
    "own ticker BBCA", "ownership ticker BBCA --as-of 2026-07-31", "ownership entity \"Dwimuria Investama\"", "ownership entity Dwimuria Investama",
    "ownership search \"lo kheng hong\" --limit 5", "ownership cross-holders --type individual --origin foreign --min-issuers 5", "ownership cross-holders --min-tickers 3",
    "ownership concentration --asc BBCA", "ownership concentration BBCA BBRI --by top1", "ownership graph \"Anthoni Salim\" --holder --depth 3",
    "ownership graph AADI --min-pct 5", "ownership changes 2026-08-31 2026-09-30", "ownership changes 2026-08-31 2026-09-30 BBCA --limit 5 --offset 5",
    "ownership foreign BBCA", "ownership foreign --from 2026-02-27 --to 2026-08-31 --limit 30", "ownership releases",
  ].flatMap((l) => [`${l} -o csv`, `idx -o csv ${l}`, `${l} -o jsonl`, `json ${l} --provider stockbit`, `${l} --provider yahoo`]),
  // The CLI's own forms with a flag the page has no twin for.
  "idx stocks foreign BBCA --source idx", "idx stocks news TLKM --source kontan --limit 40", "idx stocks sentiment BBRI -n 5", "idx stocks screen --universe stocks.csv -f pe<10",
  "idx stocks screen -f roe>15 --fundamentals", "idx stocks list --board utama", "idx stocks dividends TLKM --limit 20", "idx broker BBCA -n 5",
  "idx market overview --top 15 --min-mcap 5T", "idx bandar screen --preset accumulation --scan 50", "idx bandar broker YP --symbols BBCA,BBRI",
  "idx ownership graph BBRI --dot", "idx ownership entity --exact \"T PERMADI RACHMAT\"", "idx -o json stocks news TLKM -s kontan", "stocks history BBCA 5y 1wk -o csv",
  "idx stocks foreign BBCA", "stocks foreign BBCA -n 10", "own graph BBRI --dot", "idx own ticker BBCA -o csv", "bandar BBCA --provider stockbit",
];

/** Checks every CLI line the notices for `typed` print; returns how many there were. */
function parseAll(typed) {
  let n = 0;
  for (const line of typed) {
    const r = routeLine(line);
    if (r.kind === "web") continue;
    for (const cli of r.cli) { n++; assert.equal(clapError(cli), null, `${line} gives ${cli}`); }
  }
  return n;
}

test("a flag notice gives the command the page runs in the CLI's words", () => {
  assert.ok(parseAll(PAGE_LINES) > 300);
});

/** What runs only in the CLI and MCP, as typed, alone and with words, and by tool name. */
const MCP_LINES = [
  ...TOOLS.filter((t) => !t.web).flatMap((t) => [t.cli, t.tool, `tools ${t.tool}`, `json ${t.cli.replace(/^idx /, "")}`, `${t.cli.split(" ").filter((w) => /^[a-z]+$/.test(w)).join(" ")} -o json`]),
  "orderbook", "orderbook BBCA", "orderbook TLKM BBRI", "stockbit orderbook", "stockbit orderbook -d 20", "stockbit actions", "stockbit actions BBCA --kind dividend",
  "composition", "ownership composition", "ownership composition --as-of 2026-09-30", "institutions", "ownership institutions", "indices", "indices LQ45 IDX30",
  "calendar", "calendar BBCA", "calendar dividend", "calendar rups", "market calendar rups BBCA", "calendar 2026-10-19", "calendar 2026-10-19 --from 2026-10-01",
  "market calendar --type rups --days 30", "stockbit calendar dividend", "stockbit calendar --kind dividend --days 30", "stockbit actions BBCA dividend",
  "stockbit actions rups -k ipo", "movements", "movements BBCA --action sell",
  "brokers", "brokers CC", "market foreign TLKM", "market foreign --date 2026-10-16 -n 20", "ownership sync --force", "ownership status", "stockbit",
  "stockbit status", "stockbit login --page", "stockbit login --timeout 600", "idx --offline stockbit status", "market brokers --top 20 --sort frequency",
  "market brokers --source ipot", "brokers --source ipot -n 5", "idx -o json market brokers --source=idx --date 2026-11-06",
  // Read again after stocks, or as a tool's web example.
  "idx stocks market foreign", "stocks ownership composition BBCA", "stocks orderbook BBCA", "stocks watch", "stocks q BBCA -o csv", "idx_quote -o csv",
  "tools idx_quote -o csv", "idx_quote --provider stockbit", "idx_ownership_changes -o csv", "json idx_bandar -o jsonl",
];

test("a notice for what runs only in the CLI and MCP gives a CLI line the CLI takes", () => {
  assert.ok(parseAll(MCP_LINES) > 100);
});

test("the calendar words a notice reads are the CLI's types and kinds", () => {
  // A ValueEnum's names: each variant in kebab case or its #[value(name)], with its aliases.
  const names = (file, name) => {
    const toks = lex(read("src", "model", file));
    const at = toks.findIndex((t, i) => t.v === "enum" && toks[i + 1]?.v === name);
    return variantsOf(toks.slice(at + 3, closing(toks, at + 2))).flatMap((v) => {
      const value = v.attrs.filter((a) => a.startsWith("#[value")).join(" ");
      return [strings(value, "name")[0] || kebab(v.name), ...strings(value, "alias")];
    });
  };
  // parse_list in each file also takes all, and the calendar default.
  assert.deepEqual([...CALENDAR_TYPES].sort(), [...names("calendar.rs", "CalendarCategory"), "all", "default"].sort());
  assert.deepEqual([...STOCKBIT_KINDS].sort(), [...names("corporate.rs", "CorporateEventKind"), "all"].sort());
});

/** What runs only in the CLI, alone, with its subcommands and with the words they take. */
const CLI_LINES = [
  "watch", "watch BBCA", "watch BBCA BBRI", "json watch BBCA", "watch add", "watch add BBCA BBRI", "watch rm TLKM", "watch remove", "watch list", "watch ls",
  "watch returns", "watch clear", "mcp", "mcp setup", "mcp setup claude-code", "mcp setup --list", "mcp setup --npx cursor", "mcp --http 127.0.0.1:8765 --token T",
  "mcp nope", "schema", "schema history", "schema market-foreign", "config", "config get", "config get provider", "config set", "config set exchange SI",
  "config init --force", "config path", "config nope", "cache", "cache clear", "cache stats", "cache path", "completions", "completions zsh", "version",
  "idx --version", "idx -V", "stockbit logout", "ownership import", "ownership import --latest --family composition", "ownership import --family composition",
  "ownership import --file register.xlsx", "ownership resolve", "ownership resolve candidates", "ownership resolve unresolved",
  "ownership resolve merge \"DRS SURONO SUBEKTI\" \"SURONO SUBEKTI\"", "ownership resolve merge X", "ownership resolve map A B --dry-run", "ownership resolve undo",
  "ownership resolve undo 3", "ownership resolve list", "ownership discover --family all", "ownership path", "ownership build-snapshot",
  "ownership build-snapshot --out build/snap", "config --help", "watch --help", "idx -o json watch",
];

test("a notice for what runs only in the CLI gives a CLI line the CLI takes", () => {
  assert.ok(parseAll(CLI_LINES) > 50);
});
