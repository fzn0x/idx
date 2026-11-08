// How the web terminal reads a typed line before it runs it (terminal.js).
//
// The terminal's own lines pass through as typed. A line in the CLI's words is
// rewritten into the page's: a leading idx goes, -o json becomes the json
// prefix, the stocks group drops (stocks quote BBCA runs quote BBCA), market
// overview, holidays, members and astro run as market, holidays, members and
// astro, and the CLI's short flags and spellings become the page's (stocks
// screen -f pe<10 runs screen pe<10). What the page cannot run becomes a
// notice instead of an error, a usage line or another command's output:
//
//   cli-mcp  a command behind an MCP tool with web: null in tools.js
//   cli      a CLI command with no MCP tool (watch, mcp, schema, config, ...)
//   flag     a CLI flag the page has no twin for (-o csv, --source idx, ...)
//
// A notice carries the CLI line in the user's own words and, where the page has
// one, a near command it does run. No DOM and no fetch, so test/route.test.mjs
// runs it as it is. The command tree follows src/cli/*.rs.
import { TOOLS, byName, cliOnlyReason, whatOf } from "./tools.js";

/** The terminal's short names for its commands (help lists them). */
export const ALIASES = { q: "quote", h: "history", f: "fundamental", t: "technical", n: "news", s: "screen", c: "compare", d: "dividends", r: "returns", b: "brief", "?": "help", cls: "clear", own: "ownership" };

/** README.md's install lines: the CLI from npm, and the MCP server for an agent, without installing. */
export const INSTALL = "npm install -g ihsg-cli";
export const AGENT = "npx -y ihsg-mcp";

/** Splits a line into words; "double" or 'single' quotes keep spaces inside one word. */
export function tokenize(line) {
  const toks = [];
  const re = /"([^"]*)"|'([^']*)'|(\S+)/g;
  let m;
  while ((m = re.exec(String(line ?? "")))) toks.push(m[1] ?? m[2] ?? m[3]);
  return toks;
}

/** Words back into a line the terminal reads the same way. */
const webJoin = (words) => words.map((w) => (w === "" || /\s/.test(w) ? (w.includes('"') ? `'${w}'` : `"${w}"`) : w)).join(" ");

/** Words as a shell line: a word the shell would split, expand or redirect is quoted. */
const shellJoin = (words) => words.map((w) => (/^[\w.,:/=+^%@-]+$/.test(w) ? w : /["$`\\!]/.test(w) ? `'${w.replace(/'/g, "'\\''")}'` : `"${w}"`)).join(" ");

const say = (id, en) => ({ id, en });
const near = (line, id, en) => ({ line, id, en });
const isFlag = (w) => /^--?[A-Za-z]/.test(w ?? "");
/** A table's own entry for a typed word: "constructor" is a word, not Object.prototype's. */
const own = (table, key) => (table && Object.hasOwn(table, key) ? table[key] : undefined);

// ---- global flags (GlobalOpts in src/cli/mod.rs) ---------------------------------

const GLOBAL = {
  output: "value", provider: "value", "history-provider": "value", timeout: "value", "cache-dir": "value", config: "value",
  offline: "switch", "no-cache": "switch", "no-color": "switch", verbose: "switch", quiet: "switch", help: "switch", version: "switch",
};
const GLOBAL_SHORT = { o: "output", v: "verbose", q: "quiet", h: "help", V: "version" };

/** Takes the global flags out of a line, wherever they stand, as clap does. */
function takeGlobals(words) {
  const rest = [], found = [];
  for (let i = 0; i < words.length; i++) {
    const w = words[i];
    const long = w.match(/^--([A-Za-z][\w-]*)(?:=(.*))?$/);
    const short = !long && w.match(/^-([ovqhV])(.*)$/);
    const name = long ? long[1].toLowerCase() : short ? GLOBAL_SHORT[short[1]] : null;
    const kind = name && GLOBAL[name];
    // -vv counts; -q, -h and -V stand alone; -o takes its value attached (-ojson) or next.
    const shortOk = !short || name === "output" || (name === "verbose" ? /^v*$/.test(short[2]) : short[2] === "");
    if (!kind || !shortOk) { rest.push(w); continue; }
    const attached = long ? long[2] : short[2].replace(/^=/, "");
    if (kind === "switch") found.push({ name, value: true, raw: [w] });
    else if (attached) found.push({ name, value: attached, raw: [w] });
    else if (i + 1 < words.length && !isFlag(words[i + 1])) found.push({ name, value: words[i + 1], raw: [w, words[++i]] });
    else found.push({ name, value: true, raw: [w] });
  }
  return { rest, found };
}

/** Splits a command's words into positionals and flags with their values, in order. */
function scan(words, switches = new Set()) {
  const items = [];
  for (let i = 0; i < words.length; i++) {
    const w = words[i];
    const long = w.match(/^--([A-Za-z][\w-]*)(?:=(.*))?$/);
    const short = !long && w.match(/^-([A-Za-z])(.*)$/);
    if (!long && !short) { items.push({ pos: w, raw: [w] }); continue; }
    const name = long ? long[1].toLowerCase() : short[1];
    const attached = long ? long[2] : short[2].replace(/^=/, "") || undefined;
    if (attached != null) items.push({ name, short: !!short, value: attached, raw: [w] });
    else if ((short || !switches.has(name)) && i + 1 < words.length && !isFlag(words[i + 1])) items.push({ name, short: !!short, value: words[i + 1], raw: [w, words[++i]] });
    else items.push({ name, short: !!short, value: true, raw: [w] });
  }
  return items;
}

// ---- the commands the page runs, in the CLI's spellings (src/cli/*.rs) ----------

/**
 * Each command the page runs, in the CLI's words: its CLI path, then where each
 * positional typed here goes on the CLI's line, in order. "@" stays a positional,
 * "@*" takes every one left; "--name" becomes that flag's value; a trailing ","
 * or " " joins every one left into one word (screen's filter clauses, a holder's
 * name). The CLI path also reads a CLI line: stocks quote runs here as quote.
 */
export const CLI_FORM = {
  quote: ["stocks quote", "@*"], history: ["stocks history", "@", "--period", "--interval"], intraday: ["stocks intraday", "@", "--interval"],
  profile: ["stocks profile", "@"], financials: ["stocks financials", "@", "--period"], earnings: ["stocks earnings", "@"],
  dividends: ["stocks dividends", "@"], returns: ["stocks returns", "@*"], brief: ["stocks brief", "@"], fundamental: ["stocks fundamental", "@"],
  growth: ["stocks growth", "@"], valuation: ["stocks valuation", "@"], risk: ["stocks risk", "@"], technical: ["stocks technical", "@", "--period"],
  screen: ["stocks screen", "--filter,"], news: ["stocks news", "@"], sentiment: ["stocks sentiment", "@"], insights: ["stocks insights", "@"],
  list: ["stocks list", "--search "], actions: ["stocks actions", "@"], foreign: ["stocks foreign", "@"], compare: ["stocks compare", "@*"],
  market: ["market overview"], holidays: ["market holidays", "--year"], members: ["market members", "--search "], astro: ["market astro", "--date"],
  broker: ["broker", "@*"], bandar: ["bandar detect", "@*"], "bandar screen": ["bandar screen"], "bandar broker": ["bandar broker", "@"],
  "ownership ticker": ["ownership ticker", "@"], "ownership entity": ["ownership entity", "@ "], "ownership search": ["ownership search", "@*"],
  "ownership cross-holders": ["ownership cross-holders"], "ownership concentration": ["ownership concentration", "@*"], "ownership graph": ["ownership graph", "@*"],
  "ownership changes": ["ownership changes", "--from", "--to", "--symbol"], "ownership foreign": ["ownership foreign", "@"], "ownership releases": ["ownership releases"],
};
/** The page's command for each CLI path in CLI_FORM: stocks quote is quote, market overview is market. */
const PAGE_CMD = Object.fromEntries(Object.entries(CLI_FORM).map(([cmd, [path]]) => [path, cmd]));
/** The page's switches (BOOL_FLAGS in terminal.js): they never take the next word. */
export const WEB_SWITCHES = ["all", "asc", "company-only", "daily", "desc", "returns", "upcoming"];

/**
 * The CLI flags of each command the page runs that need the page's word, keyed by
 * the page's command: `short` maps a short flag to its long name; `switch` lists
 * the flags that take no value; `cli` the flags the page has no twin for (`keep`:
 * values that are the page's own anyway, as foreign --source ipot); `rename` a
 * flag the page writes otherwise; `pos` the flags whose value is a positional
 * word here, in that order; `join` the flags given twice that the page takes
 * comma-separated; `cliName` a flag of the page's that the CLI spells otherwise.
 * Every other flag is the page's too, under the same name.
 */
export const SPEC = {
  history: { short: { p: "period", i: "interval", n: "limit" } },
  intraday: { short: { i: "interval", n: "limit" } },
  financials: { short: { p: "period", s: "statement" }, join: ["statement"] },
  earnings: { switch: ["annual", "quarterly", "reported", "history", "estimates", "forecast"] },
  dividends: { short: { n: "limit" }, cli: ["limit"] },
  returns: { switch: ["no-benchmark"], rename: { "no-benchmark": ["benchmark", "none"] } },
  technical: { short: { p: "period" } },
  screen: { short: { f: "filter", n: "limit" }, switch: ["fundamentals"], pos: ["filter"], cli: ["fundamentals", "universe"] },
  news: { short: { s: "source", n: "limit" }, cli: ["source"] },
  sentiment: { short: { s: "source", n: "limit" }, cli: ["source", "limit"] },
  list: { short: { s: "search" }, cli: ["board"] },
  foreign: { short: { n: "days" }, cli: ["source"], keep: { source: ["ipot"] } },
  market: { short: { n: "top" }, cli: ["top", "min-mcap", "index", "universe"] },
  members: { short: { s: "search" } },
  astro: { short: { d: "date" }, switch: ["backtest"] },
  broker: { short: { d: "date", n: "top" }, cli: ["top"] },
  bandar: { short: { p: "period", d: "date", n: "top" } },
  "bandar screen": { short: { f: "filter", p: "period", n: "top", l: "limit" }, cli: ["scan", "symbols"] },
  "bandar broker": { short: { p: "period", l: "limit" }, cli: ["scan", "symbols"] },
  "ownership entity": { switch: ["exact"], cli: ["exact"] },
  "ownership search": { short: { n: "limit" } },
  "ownership cross-holders": { short: { n: "limit" } },
  "ownership concentration": { short: { n: "limit" }, switch: ["ascending"], cliName: { asc: "ascending" } },
  "ownership graph": { switch: ["holder", "dot"], cli: ["dot"] },
  "ownership changes": { short: { s: "symbol", n: "limit" }, pos: ["from", "to", "symbol"] },
  "ownership foreign": { short: { n: "limit" } },
};

/** Why the page has no twin for a CLI flag, by "command flag", then by flag. */
const FLAG_WHY = {
  output: say("Terminal web mencetak tabel, dan amplop JSON bila json ditulis di depan perintah; CSV dan JSONL adalah format CLI untuk berkas dan pipa.", "The web terminal prints tables, and the JSON envelope when json comes before the command; CSV and JSONL are the CLI's formats for files and pipes."),
  provider: say("Terminal web memilih sumber tiap perintah sendiri dan menyebutnya di kepala hasil.", "The web terminal picks each command's source itself and names it in the result's head."),
  stockbit: say("Stockbit memakai sesi akun Anda sendiri, jadi hanya berjalan di CLI dan server MCP.", "Stockbit runs on your own account session, so it works in the CLI and the MCP server only."),
  "foreign source": say("Terminal web membaca IPOT, dalam rupiah di semua papan; idx.co.id menolak permintaan dari server situs ini, jadi --source idx hanya ada di CLI dan MCP.", "The web terminal reads IPOT, in rupiah across all boards; idx.co.id refuses requests from this site's host, so --source idx runs in the CLI and MCP only."),
  universe: say("--universe membaca berkas CSV di komputer Anda.", "--universe reads a CSV file on your machine."),
  "news source": say("Terminal web membaca umpan Kontan dan CNBC Indonesia sekaligus.", "The web terminal reads the Kontan and CNBC Indonesia feeds together."),
  "sentiment source": say("Terminal web membaca umpan Kontan dan CNBC Indonesia sekaligus.", "The web terminal reads the Kontan and CNBC Indonesia feeds together."),
  scan: say("Terminal web membaca pindaian tiap malam atas 100 saham teramai sesi itu; CLI memindai langsung.", "The web terminal reads the nightly scan of the session's 100 most traded stocks; the CLI scans live."),
  symbols: say("Terminal web membaca pindaian tiap malam atas 100 saham teramai sesi itu; CLI memindai langsung.", "The web terminal reads the nightly scan of the session's 100 most traded stocks; the CLI scans live."),
  "ownership entity exact": say("Terminal web mencocokkan sebagian nama: tanpa --exact, kata yang sama juga menemukan pemegang yang namanya memuatnya.", "The web terminal matches part of a name: without --exact the same words also find holders whose names contain them."),
  "ownership graph dot": say("Terminal web menampilkan jaringannya sebagai tabel; --dot mencetak Graphviz untuk dot di komputer Anda.", "The web terminal shows the network as a table; --dot prints Graphviz for dot on your machine."),
  "list board": say("Terminal web mendaftar kode dan nama bawaan, yang tidak memuat papan pencatatan.", "The web terminal lists the bundled codes and names, which carry no listing boards."),
  "broker top": say("Terminal web selalu menampilkan 10 pembeli dan penjual teratas yang diterbitkan sumbernya.", "The web terminal always shows the top 10 buyers and sellers the source publishes."),
  "dividends limit": say("Terminal web menampilkan 12 pembayaran terakhir; json dividends memuat semuanya.", "The web terminal lists the last 12 payments; json dividends carries all of them."),
  market: say("Panel market di terminal web memakai daftar tetap.", "The web terminal's market panel uses fixed lists."),
};
const flagWhy = (cmd, name) => own(FLAG_WHY, `${cmd} ${name}`) || own(FLAG_WHY, name) || own(FLAG_WHY, cmd) || null;

// ---- near commands the page does run ------------------------------------------------

const N = {
  foreign: (s) => near(`foreign ${s}`, "beli dan jual asing satu saham per sesi, dari IPOT dalam rupiah", "one stock's foreign buying and selling per session, from IPOT in rupiah"),
  foreignScreen: () => near("bandar screen --preset foreign-accumulation", "saham teramai yang diakumulasi broker teratas dengan beli bersih asing (pindaian tiap malam)", "the most traded stocks the top brokers accumulate with foreign net buying (nightly scan)"),
  broker: (s) => near(`broker ${s}`, "broker yang membeli dan menjual satu saham, 10 teratas tiap sisi (IPOT)", "the brokers that bought and sold one stock, top 10 per side (IPOT)"),
  bandarBroker: (c) => near(`bandar broker ${c}`, "di saham mana satu broker masuk 10 pembeli atau penjual teratas", "where one broker is a top-10 buyer or seller"),
  members: () => near("members", "kode dan nama anggota bursa", "the exchange members' codes and names"),
  market: () => near("market", "IHSG, saham naik dan turun, penggerak pasar", "IHSG, breadth and the day's movers"),
  ihsg: () => near("quote ^JKSE", "harga IHSG dan grafiknya", "IHSG's quote and its chart"),
  actions: (s) => near(`actions ${s}`, "aksi korporasi dan RUPS satu saham, termasuk yang akan datang (IPOT)", "one stock's corporate actions and RUPS, upcoming included (IPOT)"),
  holidays: () => near("holidays --upcoming", "hari libur bursa mulai hari ini", "the exchange's trading holidays from today"),
  ownForeign: (s) => near(`ownership foreign ${s}`, "porsi asing dan lokal pemegang di atas 1 %, per bulan (daftar KSEI)", "the foreign and local stakes above 1 %, month by month (KSEI register)"),
  holders: (s) => near(`ownership ticker ${s}`, "pemegang saham di atas 1 % (daftar bulanan KSEI)", "the holders above 1 % (KSEI's monthly register)"),
  releases: () => near("ownership releases", "bulan-bulan daftar KSEI yang dimuat halaman ini", "the monthly KSEI registers this page holds"),
  quote: (s) => near(`quote ${s}`, "harga tertunda dengan rentang hari itu dan grafik", "the delayed quote with the day's range and a chart"),
  intraday: (s) => near(`intraday ${s}`, "sesi terakhir per menit, dengan VWAP", "the latest session by the minute, with VWAP"),
  quotes: (l) => near(`quote ${l.join(",")}`, "harga beberapa saham sekaligus", "quotes for several stocks at once"),
  returns: (l) => near(`returns ${l.join(" ")}`, "imbal hasil 1w..5y dibanding IHSG", "1w..5y returns against IHSG"),
  tools: () => near("tools", `${TOOLS.length} alat MCP dengan perintah web-nya`, `the ${TOOLS.length} MCP tools with their web commands`),
  json: (cmd) => near(`json ${cmd}`, "amplop JSON mentah sebuah perintah; field schema menyebut bentuknya", "a command's raw JSON envelope; its schema field names the shape"),
  search: (q) => near(webJoin(["ownership", "search", q]), "cari pemegang dan setiap ejaan yang dicetak KSEI", "find a holder and every spelling KSEI printed"),
};

/** idx schema names, by the page's command that prints that shape. */
const SCHEMA_CMD = {
  quotes: "quote BBCA", quote: "quote BBCA", history: "history BBCA", candle: "history BBCA", intraday: "intraday BBCA", profile: "profile BBCA",
  fundamentals: "fundamental BBCA", growth: "growth BBCA", valuation: "valuation BBCA", risk: "risk BBCA", compare: "compare BBCA BBRI",
  financials: "financials BBCA", earnings: "earnings BBCA", dividends: "dividends BBCA", returns: "returns BBCA", market: "market", brief: "brief BBCA",
  broker: "broker BBCA", bandar: "bandar BBCA", "bandar-screen": "bandar screen", "bandar-broker": "bandar broker YP", holidays: "holidays",
  astro: "astro", instruments: "list --search bank", members: "members", actions: "actions BBCA", "foreign-value": "foreign BBCA", foreign: "foreign BBCA",
  technical: "technical BBCA", screen: "screen --preset low-pe", news: "news BBCA", sentiment: "sentiment BBCA", insights: "insights BBCA",
  "ownership-holders": "ownership ticker BBCA", "ownership-releases": "ownership releases", "ownership-foreign": "ownership foreign BBCA",
  "ownership-foreign-flow": "ownership foreign", "ownership-search": "ownership search salim", "ownership-cross-holders": "ownership cross-holders",
  "ownership-concentration": "ownership concentration", "ownership-graph": "ownership graph BBCA",
};

/** An IDX ticker as typed (BBCA, bbca, BBCA.JK), and a day (2026-10-19). */
const TICKER = /^[A-Za-z]{4}(\.JK)?$/i;
const DAY = /^\d{4}-\d{2}-\d{2}$/;

/** Subcommand words of the CLI-only commands (watch list, config init, resolve undo), never tickers. */
const SUBWORDS = new Set(["add", "remove", "rm", "list", "ls", "clear", "returns", "init", "set", "get", "path", "stats", "setup", "candidates", "unresolved", "merge", "map", "undo"]);

/** The words a notice's near commands take from the line: the tickers, a typed broker code, a holder's name. */
function picks(items) {
  const pos = items.filter((x) => x.pos != null && !SUBWORDS.has(x.pos.toLowerCase())).map((x) => x.pos);
  const flagged = items.filter((x) => (x.name === "symbol" || x.name === "s") && typeof x.value === "string").map((x) => x.value);
  const syms = [...flagged, ...pos].flatMap((w) => w.split(",")).filter((w) => TICKER.test(w)).map((w) => w.toUpperCase().replace(/\.JK$/, ""));
  const code = pos.find((w) => /^[A-Za-z]{2}$/.test(w));
  const name = pos.find((w) => !/^[A-Za-z]{4}$/.test(w) && /[A-Za-z]{3}/.test(w));
  return { sym: syms[0] || "BBCA", syms, code: code ? code.toUpperCase() : null, name: name || "salim" };
}

// ---- what runs only in the CLI and the MCP server, or only in the CLI -----------------

/** The exchange calendar's --type names (CalendarCategory in src/model/calendar.rs), aliases, all and default. */
export const CALENDAR_TYPES = new Set(["dividend", "rights", "bonus", "split", "warrant", "rups", "public-expose", "public_expose", "expose", "buyback", "listing", "bond", "other", "all", "default"]);
/** Stockbit's --kind names (CorporateEventKind in src/model/corporate.rs), aliases and all. */
export const STOCKBIT_KINDS = new Set(["dividend", "stock_dividend", "stock-dividend", "bonus", "rights", "rights_issue", "rights-issue", "warrant", "warrants", "split", "stock_split", "stock-split", "reverse_split", "reverse-split", "rups", "meeting", "public_expose", "public-expose", "expose", "ipo", "tender_offer", "tender-offer", "tender", "all"]);

/**
 * CLI commands behind an MCP tool with web: null, by their CLI path. `takes`, how
 * the command takes the words typed after it: "none" (a ticker typed only shapes
 * the near commands), "one" positional, "many", or "symbol" (a ticker as --symbol,
 * a date as --date). `needs`: the CLI requires that positional, so a line without
 * one takes the near commands' ticker. `kinds`: the words that name a type of
 * event, which go to that flag and never to the tickers. `switch`: its flags that
 * take no value.
 */
export const MCP_ONLY = {
  "market foreign": { tool: "idx_market_foreign", takes: "none", near: (a) => [N.foreign(a.sym), N.foreignScreen()] },
  "market brokers": { tool: "idx_market_brokers", takes: "none", near: (a) => (a.code && !a.syms.length ? [N.bandarBroker(a.code), N.broker(a.sym)] : [N.broker(a.sym), N.bandarBroker(a.code || "YP")]).concat(N.members()) },
  "market indices": { tool: "idx_indices", takes: "many", near: () => [N.market(), N.ihsg()] },
  "market calendar": { tool: "idx_calendar", takes: "symbol", kinds: { flag: "type", short: "t", words: CALENDAR_TYPES }, near: (a) => [N.actions(a.sym), N.holidays()] },
  "ownership composition": { tool: "idx_ownership_composition", takes: "one", needs: true, near: (a) => [N.ownForeign(a.sym), N.holders(a.sym)] },
  "ownership institutions": { tool: "idx_ownership_institutions", takes: "one", needs: true, near: (a) => [N.holders(a.sym)] },
  "ownership sync": { tool: "idx_ownership_snapshot", takes: "none", switch: ["force"], near: () => [N.releases()] },
  "ownership status": { tool: "idx_ownership_snapshot", takes: "none", near: () => [N.releases()] },
  "stockbit orderbook": { tool: "idx_orderbook", takes: "one", needs: true, near: (a) => [N.quote(a.sym), N.intraday(a.sym)] },
  "stockbit status": { tool: "idx_stockbit_status", takes: "none" },
  "stockbit login": { tool: "idx_stockbit_login", takes: "none", switch: ["manual", "page", "no-browser", "force"] },
  "stockbit actions": { tool: "idx_stockbit_actions", takes: "one", needs: true, kinds: { flag: "kind", short: "k", words: STOCKBIT_KINDS }, near: (a) => [N.actions(a.sym)] },
  "stockbit calendar": { tool: "idx_stockbit_actions", takes: "none", kinds: { flag: "kind", short: "k", words: STOCKBIT_KINDS }, near: (a) => [N.actions(a.sym), N.holidays()] },
  "stockbit movements": { tool: "idx_stockbit_movements", takes: "one", near: (a) => [N.holders(a.sym)] },
};

/** Words typed alone that can only mean one of those (brokers, one letter from broker, points at broker too). */
export const BARE = { orderbook: "stockbit orderbook", indices: "market indices", calendar: "market calendar", composition: "ownership composition", institutions: "ownership institutions", movements: "stockbit movements", brokers: "market brokers" };

/**
 * The words of a CLI-only line the CLI needs: with no subcommand typed, `alone`
 * (a read-only one) where the CLI requires one; for an unknown one, --help; and a
 * subcommand short of the words `subs` says it needs gets `short` (--help, so the
 * CLI says what is missing). `items`: the scanned `args`.
 */
function whole(items, args, subs, alone = null, short = "--help") {
  const pos = items.filter((x) => x.pos != null);
  if (!pos.length) return alone ? [...alone, ...args] : args;
  const need = own(subs, pos[0].pos.toLowerCase());
  if (need == null) return ["--help"];
  return pos.length - 1 >= need || args.includes(short) ? args : [...args, short];
}
const WATCH_SUBS = { add: 1, remove: 1, rm: 1, list: 0, ls: 0, clear: 0, returns: 0 };

/**
 * CLI commands with no MCP tool: what each does and why it stays in the CLI.
 * `cli(items, args)`: the words the CLI needs after the command (whole); `switch`:
 * its flags that take no value.
 */
export const CLI_ONLY = {
  watch: {
    what: say("harga daftar pantauan Anda, atau mengelolanya", "quote your watchlist, or manage it"),
    why: say("Daftar pantauan adalah berkas teks di samping konfigurasi CLI di komputer Anda, jadi watch hanya berjalan di CLI.", "Your watchlist is a plain-text file next to the CLI's config on your machine, so watch runs in the CLI only."),
    // idx watch takes only its subcommands: tickers typed after it are for watch add.
    cli: (items, args) => { const first = items.find((x) => x.pos != null); return first && own(WATCH_SUBS, first.pos.toLowerCase()) == null ? ["add", ...args] : whole(items, args, WATCH_SUBS); },
    near: (a) => { const l = a.syms.length ? a.syms : ["BBCA", "BBRI", "TLKM"]; return [l.length === 1 ? N.quote(l[0]) : N.quotes(l), N.returns(l)]; },
  },
  mcp: {
    what: say("server MCP lewat stdio atau HTTP; mcp setup mencetak cara mendaftarkannya ke agen", "the MCP server over stdio or HTTP; mcp setup prints how to register it with an agent"),
    why: say("idx mcp menjalankan server di komputer Anda, dan agen tersambung ke sana, jadi perintah ini hanya berjalan di CLI; halaman ini menjalankan alat yang sama sebagai perintah.", "idx mcp starts the server on your machine for an agent to connect to, so it runs in the CLI only; this page runs the same tools as commands."),
    switch: ["list", "npx"], cli: (items, args) => whole(items, args, { setup: 1 }, null, "--list"),
    agent: true, near: () => [N.tools()],
  },
  schema: {
    what: say("JSON Schema untuk satu bentuk keluaran", "the JSON Schema of an output shape"),
    why: say("Skemanya dibawa oleh CLI, jadi schema hanya berjalan di CLI; di sini, json di depan perintah mencetak amplopnya, dan field schema-nya menyebut bentuknya.", "The schemas ship with the CLI, so schema runs in the CLI only; here, json before a command prints its envelope, whose schema field names the shape."),
    near: (a, words) => [N.json(own(SCHEMA_CMD, String(words.find((w) => !isFlag(w)) || "").toLowerCase()) || "quote BBCA")],
  },
  config: {
    what: say("membaca atau menulis berkas konfigurasi", "read or write the config file"),
    why: say("Berkas konfigurasi ada di komputer Anda, jadi config hanya berjalan di CLI.", "The config file lives on your machine, so config runs in the CLI only."),
    switch: ["force"], cli: (items, args) => whole(items, args, { init: 0, set: 2, get: 0, path: 0 }, ["get"]),
  },
  cache: {
    what: say("memeriksa atau mengosongkan cache jawaban lokal", "inspect or clear the local response cache"),
    why: say("Cache jawaban CLI ada di komputer Anda, jadi cache hanya berjalan di CLI; halaman ini punya cache sendiri, dan kepala tiap hasil menyebut apakah jawabannya dari cache.", "The CLI's response cache lives on your machine, so cache runs in the CLI only; this page keeps its own, and each result's head says whether it came from cache."),
    cli: (items, args) => whole(items, args, { stats: 0, clear: 0, path: 0 }, ["stats"]),
  },
  completions: {
    what: say("pelengkapan perintah untuk shell: bash, zsh, fish, powershell atau elvish", "shell completions: bash, zsh, fish, powershell or elvish"),
    why: say("Pelengkapan perintah dipasang di shell komputer Anda, jadi completions hanya berjalan di CLI.", "Completions go into the shell on your machine, so completions runs in the CLI only."),
    cli: (items, args) => (items.some((x) => x.pos != null) ? args : ["bash", ...args]),
  },
  version: {
    what: say("versi dan info build", "version and build information"),
    why: say("Versinya milik idx yang terpasang di komputer Anda, jadi version hanya berjalan di CLI.", "It reports the idx installed on your machine, so version runs in the CLI only."),
  },
  "stockbit logout": {
    what: say("menghapus sesi Stockbit yang tersimpan", "delete the stored Stockbit session"),
    why: say("Sesi Stockbit tersimpan di komputer Anda dan tidak pernah di halaman ini, jadi logout hanya berjalan di CLI.", "The Stockbit session is stored on your machine and never on this page, so logout runs in the CLI only."),
  },
  "ownership import": {
    what: say("memuat daftar bulanan atau arsip komposisi KSEI ke basis data lokal", "import a monthly register or a KSEI composition archive into the local database"),
    why: say("Perintah ini menulis basis data kepemilikan lokal milik CLI, jadi hanya berjalan di CLI; daftar di halaman ini disinkronkan dari mirror GitHub yang sama.", "It writes the CLI's local ownership database, so it runs in the CLI only; this page's register is synced from the same GitHub mirror."),
    // The CLI needs one of --file, --url, --latest or --all.
    switch: ["latest", "all", "force"], cli: (items, args) => (items.some((x) => ["file", "url", "latest", "all"].includes(x.name)) ? args : [...args, "--latest"]),
    near: () => [N.releases()],
  },
  "ownership resolve": {
    what: say("menyarankan, menggabungkan dan memisahkan ejaan pemegang", "suggest, merge and split holder spellings"),
    why: say("Aturannya disimpan di basis data kepemilikan lokal Anda, jadi resolve hanya berjalan di CLI; halaman ini memakai pengelompokan otomatis.", "Its rules live in your local ownership database, so resolve runs in the CLI only; this page keeps the automatic grouping."),
    switch: ["fold", "dry-run", "new"], cli: (items, args) => whole(items, args, { candidates: 0, unresolved: 0, merge: 2, map: 2, list: 0, undo: 1 }, ["candidates"]),
    near: (a) => [N.search(a.name)],
  },
  "ownership discover": {
    what: say("daftar berkas yang diterbitkan idx.co.id dan ksei.co.id", "list the files idx.co.id and ksei.co.id publish"),
    why: say("idx.co.id menolak permintaan dari server situs ini, jadi discover hanya berjalan di CLI.", "idx.co.id refuses requests from this site's host, so discover runs in the CLI only."),
    near: () => [N.releases()],
  },
  "ownership path": {
    what: say("lokasi basis data kepemilikan", "the ownership database's path"),
    why: say("Basis datanya ada di komputer Anda, jadi path hanya berjalan di CLI.", "The database lives on your machine, so path runs in the CLI only."),
  },
  "ownership build-snapshot": {
    what: say("alat pengelola: menulis snapshot basis data lokal", "maintainer tool: write the local database's snapshot"),
    why: say("Ini alat pengelola untuk snapshot yang diterbitkan CI, jadi hanya berjalan di CLI.", "It is a maintainer tool for the snapshot CI publishes, so it runs in the CLI only."),
    cli: (items, args) => (items.some((x) => x.name === "out") ? args : [...args, "--help"]),
  },
};

/** The CLI command a tool runs, without its example arguments: idx stockbit orderbook BBCA is stockbit orderbook. */
export function cliPath(tool) {
  const w = tokenize(tool.cli).slice(1);
  const two = `${w[0]} ${w[1]}`;
  return ["stocks", "market", "ownership", "bandar", "stockbit"].includes(w[0]) && w[1] && !isFlag(w[1]) ? two : w[0];
}

// ---- the router ---------------------------------------------------------------------

/**
 * What a typed line does on this page:
 *   { kind: "web", line, note }   run `line` (note: { line, ignored } when it was rewritten)
 *   { kind: "cli-mcp", path, tool, tools?, what, why, cli, near }
 *   { kind: "cli", path, what, why, cli, near, agent }
 *   { kind: "flag", command, flags, why, cli, near }
 * `what`, `why` and each near command's words come in both languages ({ id, en });
 * `cli` holds the CLI line, then its -o json form.
 */
export function routeLine(line) {
  const typed = String(line ?? "").trim();
  const all = tokenize(typed);
  let k = 0, json = false, fromCli = false;
  for (; k < all.length; k++) {
    const w = all[k].toLowerCase();
    if (w === "json") json = true;
    else if (w === "idx") fromCli = true;
    else break;
  }
  const { rest, found } = takeGlobals(all.slice(k));
  const ctx = { typed, all, k, json, fromCli, found, ignored: [], help: false, version: false, badOutput: null, badProvider: null };
  for (const g of found) {
    const v = String(g.value).toLowerCase();
    if (g.name === "output") { if (v === "json") ctx.json = true; else if (v !== "table" && v !== "auto") ctx.badOutput = g; }
    else if (g.name === "provider" || g.name === "history-provider") { if (v !== "auto") ctx.badProvider = g; }
    else if (g.name === "help") ctx.help = true;
    else if (g.name === "version") ctx.version = true;
    else ctx.ignored.push(g);
  }
  if (ctx.version) return cliNotice(ctx, "version", []);
  return dispatch(ctx, rest);
}

function dispatch(ctx, words) {
  if (!words.length) return ctx.fromCli || ctx.help ? web(ctx, ["help"]) : { kind: "web", line: ctx.typed, note: null };
  const head = words[0].toLowerCase();
  const name = own(ALIASES, head) || head;
  const sub = (words[1] || "").toLowerCase();
  if (name === "stocks") {
    if (!sub) return web(ctx, ["help"]);
    const runs = own(PAGE_CMD, `stocks ${sub}`);
    // Any other word after stocks starts a line of its own: stocks market foreign is market foreign.
    if (!runs) return dispatch({ ...ctx, rewrite: true }, words.slice(1));
    return webCommand(ctx, runs, [runs], words.slice(2), { group: true });
  }
  if (name === "market") {
    const runs = own(PAGE_CMD, `market ${sub}`);
    if (runs) return webCommand(ctx, runs, [runs], words.slice(2), { group: true });
    if (own(MCP_ONLY, `market ${sub}`)) return mcpNotice(ctx, `market ${sub}`, words.slice(2));
    return webCommand(ctx, "market", [words[0]], words.slice(1));
  }
  if (name === "ownership") {
    const runs = own(PAGE_CMD, `ownership ${sub}`);
    if (runs) return webCommand(ctx, runs, words.slice(0, 2), words.slice(2));
    if (own(MCP_ONLY, `ownership ${sub}`)) return mcpNotice(ctx, `ownership ${sub}`, words.slice(2));
    if (own(CLI_ONLY, `ownership ${sub}`)) return cliNotice(ctx, `ownership ${sub}`, words.slice(2));
    return webCommand(ctx, "ownership", words.slice(0, 1), words.slice(1));
  }
  if (name === "stockbit") {
    if (own(MCP_ONLY, `stockbit ${sub}`)) return mcpNotice(ctx, `stockbit ${sub}`, words.slice(2));
    if (own(CLI_ONLY, `stockbit ${sub}`)) return cliNotice(ctx, `stockbit ${sub}`, words.slice(2));
    return stockbitNotice(ctx);
  }
  if (name === "bandar") {
    // bandar BBCA is the page's bandar detect BBCA.
    const runs = own(PAGE_CMD, `bandar ${sub}`);
    return runs ? webCommand(ctx, runs, words.slice(0, 2), words.slice(2)) : webCommand(ctx, "bandar", words.slice(0, 1), words.slice(1));
  }
  if (own(CLI_ONLY, name)) return cliNotice(ctx, name, words.slice(1));
  if (own(BARE, name)) return mcpNotice(ctx, BARE[name], words.slice(1));
  if (/^idx_[a-z_]+$/.test(name)) return toolLine(ctx, name, words.slice(1), false);
  if (name === "tools" && /^idx_[a-z_]+$/.test(sub)) return toolLine(ctx, sub, words.slice(2), true);
  return webCommand(ctx, name, [words[0]], words.slice(1));
}

/** An MCP tool's name, typed alone or after tools: its web example, its notice, or its CLI command with the words after it. */
function toolLine(ctx, name, args, typedTools) {
  const tool = byName(name);
  if (!tool) return web(ctx, ["tools", name, ...args]);
  const path = cliPath(tool);
  // The tool's CLI command with the words after the name: never the typed line itself.
  if (args.length) return dispatch({ ...ctx, rewrite: true }, [...path.split(" "), ...args]);
  if (!tool.web) return mcpNotice(ctx, path, tokenize(tool.cli).slice(1 + path.split(" ").length), tool);
  if (typedTools && !ctx.fromCli && !ctx.found.length) return { kind: "web", line: ctx.typed, note: null };
  // The tool's web example is read as a line of its own, so -o csv or --provider still gets its notice.
  return dispatch({ ...ctx, rewrite: true }, tokenize(tool.web));
}

/** A command the page runs: the CLI's flags in the page's words, or a notice for the ones it has no twin for. */
function webCommand(ctx, cmd, prefix, args, { group = false } = {}) {
  const spec = own(SPEC, cmd) || {};
  const items = scan(args, new Set([...(spec.switch || []), ...WEB_SWITCHES]));
  const shortUsed = items.some((x) => x.short && own(spec.short, x.name));
  const renamed = items.some((x) => !x.short && (own(spec.rename, x.name) || spec.pos?.includes(x.name)));
  const cliShaped = ctx.fromCli || group || ctx.found.length > 0 || shortUsed || renamed;
  // `cliOnly`: the typed words of the flags the page lacks, for the CLI line.
  const pos = [], flags = [], unsupported = [], cliOnly = [], asPos = {};
  let changed = false;
  for (const x of items) {
    if (x.pos != null) { pos.push(x.pos); continue; }
    const name = x.short ? own(spec.short, x.name) : x.name;
    if (!name) { pos.push(...x.raw); continue; }
    if (x.short) changed = true;
    const rename = own(spec.rename, name);
    if (rename) { flags.push({ name: rename[0], value: rename[1] }); changed = true; continue; }
    if (spec.pos?.includes(name)) { asPos[name] = x.value; changed = true; continue; }
    if (cliShaped && spec.cli?.includes(name)) {
      const keep = own(spec.keep, name);
      if (keep?.includes(String(x.value).toLowerCase())) { changed = true; continue; }
      unsupported.push(keep ? `--${name} ${x.value}` : `--${name}`);
      cliOnly.push(...x.raw);
      continue;
    }
    const twice = spec.join?.includes(name) && flags.find((f) => f.name === name);
    if (twice) { twice.value = `${twice.value},${x.value}`; changed = true; continue; }
    flags.push({ name, value: x.value });
  }
  for (const k of spec.pos || []) if (typeof asPos[k] === "string") pos.push(asPos[k]);
  const words = [...prefix, ...pos, ...flags.flatMap((f) => (f.value === true ? [`--${f.name}`] : [`--${f.name}`, String(f.value)]))];
  if (ctx.help) return web(ctx, ["help", cmd.split(" ")[0]]);
  const cli = cliForm(cmd, spec, pos, flags, cliOnly);
  if (ctx.badOutput) {
    const plain = webJoin(words), jsonLine = webJoin(["json", ...words]);
    return flagNotice(ctx, null, [ctx.badOutput.raw.join(" ")], FLAG_WHY.output, cli,
      [near(plain, "sebagai tabel", "as a table"), near(jsonLine, "sebagai amplop JSON", "as the JSON envelope")]);
  }
  if (ctx.badProvider) {
    const stockbit = String(ctx.badProvider.value).toLowerCase() === "stockbit";
    return flagNotice(ctx, null, [ctx.badProvider.raw.join(" ")], stockbit ? FLAG_WHY.stockbit : FLAG_WHY.provider, cli, [without(ctx, words, ctx.badProvider.raw[0])]);
  }
  if (unsupported.length) {
    const why = unsupported.map((f) => flagWhy(cmd, f.split(" ")[0].slice(2))).find(Boolean) || null;
    return flagNotice(ctx, cmd, unsupported, why, cli, [without(ctx, words, unsupported.map((f) => f.split(" ")[0]).join(", "))]);
  }
  if (!cliShaped && !changed && !ctx.rewrite) return { kind: "web", line: ctx.typed, note: null };
  return web(ctx, words);
}

/**
 * What the page runs, in the CLI's words (CLI_FORM): its CLI path, the page's
 * positionals where the CLI takes them, its flags, then `extra` (the typed words
 * of the flags the page lacks). Null when a word has no place on the CLI's line,
 * or a positional and a flag give the same setting twice.
 */
function cliForm(cmd, spec, pos, flags, extra) {
  const form = own(CLI_FORM, cmd);
  if (!form || pos.some(isFlag)) return null;
  const [path, ...slots] = form;
  const out = path.split(" "), set = [];
  let k = 0;
  for (const slot of slots) {
    if (k >= pos.length) break;
    const flag = slot.match(/^--([a-z-]+)/)?.[1];
    const join = slot.slice(flag ? flag.length + 2 : 1);
    const taken = join ? pos.slice(k) : [pos[k]];
    k += taken.length;
    if (!flag) out.push(...(join === "*" ? taken : [taken.join(join)]));
    else { set.push(flag); out.push(`--${flag}`, taken.join(join)); }
  }
  if (k < pos.length) return null;
  for (const f of flags) {
    const name = own(spec.cliName, f.name) || f.name;
    if (set.includes(name)) return null;
    out.push(`--${name}`, ...(f.value === true ? [] : [String(f.value)]));
  }
  return [...out, ...extra];
}

/** The near command for a flag notice: the same line, without what the page lacks. */
function without(ctx, words, what) {
  return near(webJoin([...(ctx.json ? ["json"] : []), ...words]), `sama, tanpa ${what}`, `the same, without ${what}`);
}

/** A line the page runs, with a note when it was rewritten or flags were left to the CLI. */
function web(ctx, words) {
  const line = webJoin([...(ctx.json ? ["json"] : []), ...words]);
  const bare = webJoin(ctx.all.filter((w, i) => !(i < ctx.k && w.toLowerCase() === "idx")));
  const ignored = ctx.ignored.map((g) => g.raw.join(" "));
  return { kind: "web", line, note: line !== bare || ignored.length ? { line, ignored } : null };
}

/** The CLI line, then its -o json form: the user's global flags kept, --help last. */
function cliLines(ctx, words) {
  const lead = ctx.found.filter((g) => !["output", "help", "version"].includes(g.name)).flatMap((g) => g.raw);
  const own = ctx.found.find((g) => g.name === "output" && String(g.value).toLowerCase() !== "json");
  const help = ctx.found.filter((g) => g.name === "help").flatMap((g) => g.raw);
  return [
    shellJoin(["idx", ...(own ? own.raw : []), ...lead, ...words, ...help]),
    shellJoin(["idx", "-o", "json", ...lead, ...words, ...help]),
  ];
}

/**
 * A command's words for the CLI line (MCP_ONLY's `takes` and `needs`): flags as
 * typed, positionals as the command takes them; `sym` stands in for a required
 * positional the line lacks.
 */
function cliArgs(items, { takes, needs }, sym) {
  const typed = (...names) => items.some((x) => names.includes(x.name));
  // --date stands for one day, so it goes only where no other range was typed.
  let given = 0, dated = typed("date", "d", "from", "to", "days");
  const out = items.flatMap((x) => {
    if (x.pos == null) return x.raw;
    if (takes === "symbol") {
      if (DAY.test(x.pos) && !dated) { dated = true; return ["--date", x.pos]; }
      if (!TICKER.test(x.pos) || given++ || typed("symbol", "s")) return [];
      return ["--symbol", x.pos];
    }
    if (takes === "none" || (takes !== "many" && given)) return [];
    given++;
    return x.raw;
  });
  return needs && !given ? [sym, ...out] : out;
}

/** The words that name a type of event (calendar dividend rups) as that one flag, with its typed value first. */
function kindWords(items, kinds) {
  const isKind = (x) => kinds && x.pos != null && kinds.words.has(x.pos.toLowerCase());
  const words = items.filter(isKind).map((x) => x.pos.toLowerCase());
  if (!words.length) return items;
  const typed = items.find((x) => x.name === kinds.flag || x.name === kinds.short);
  const value = [...(typeof typed?.value === "string" ? [typed.value] : []), ...words].join(",");
  return [...items.filter((x) => x !== typed && !isKind(x)), { name: kinds.flag, value, raw: [`--${kinds.flag}`, value] }];
}

function mcpNotice(ctx, path, args, tool = byName(MCP_ONLY[path].tool)) {
  const entry = MCP_ONLY[path] || {};
  const items = kindWords(scan(args, new Set(entry.switch || [])), entry.kinds);
  const a = picks(items);
  return {
    kind: "cli-mcp", path, tool: tool.tool,
    what: say(whatOf(tool, "id"), whatOf(tool, "en")),
    why: say(cliOnlyReason(tool, "id"), cliOnlyReason(tool, "en")),
    cli: cliLines(ctx, [...path.split(" "), ...cliArgs(items, entry, a.sym)]),
    near: entry.near ? entry.near(a) : [],
  };
}

/** stockbit alone: every Stockbit tool, and the sign-in that comes first. */
function stockbitNotice(ctx) {
  const login = byName("idx_stockbit_login");
  return {
    kind: "cli-mcp", path: "stockbit", tool: null,
    tools: TOOLS.filter((t) => t.tool === "idx_orderbook" || t.tool.startsWith("idx_stockbit")).map((t) => t.tool),
    what: say("perintah Stockbit dengan sesi Anda sendiri: login, status, logout, orderbook, actions, calendar, movements", "the Stockbit commands on your own session: login, status, logout, orderbook, actions, calendar, movements"),
    why: say(cliOnlyReason(login, "id"), cliOnlyReason(login, "en")),
    cli: cliLines(ctx, ["stockbit", "login"]),
    near: [],
  };
}

function cliNotice(ctx, path, args) {
  const entry = CLI_ONLY[path];
  const items = scan(args, new Set(entry.switch || []));
  // One line: the command as typed and made whole, in its -o json form when that was asked for.
  const words = entry.cli && !ctx.help ? entry.cli(items, args) : args;
  const [plain, asJson] = cliLines(ctx, [...path.split(" "), ...words]);
  return {
    kind: "cli", path, what: entry.what, why: entry.why,
    cli: [ctx.json ? asJson : plain],
    near: entry.near ? entry.near(picks(items), args) : [],
    agent: !!entry.agent,
  };
}

/** `cliWords`: the command in the CLI's words (cliForm), or null when it has none. */
function flagNotice(ctx, command, flags, why, cliWords, nearBy) {
  return { kind: "flag", command, flags, why, cli: cliWords ? cliLines(ctx, cliWords) : [], near: nearBy };
}
