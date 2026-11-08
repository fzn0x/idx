// node --test web/test/*.mjs. public/route.js reads a typed line before the
// terminal runs it: the terminal's own lines pass through untouched, a line in
// the CLI's words runs in the page's, and what the page cannot run (an MCP tool
// with web: null, a CLI-only command, a CLI flag without a twin here) becomes a
// notice with the CLI line, the reason and the page's near commands. The CLI's
// command tree in src/cli/*.rs is the reference: every subcommand it has must
// be routed somewhere.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { routeLine, tokenize, cliPath, ALIASES, BARE, INSTALL, AGENT, WEB_SWITCHES } from "../public/route.js";
import { TOOLS, byName, cliOnlyReason } from "../public/tools.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (...p) => readFileSync(join(root, ...p), "utf8");
const terminal = read("web", "public", "terminal.js");
const COMMANDS = [...terminal.matchAll(/\{ name: "([a-z]+)", usage:/g)].map((m) => m[1]);
const SAMPLE = JSON.parse(terminal.match(/^const SAMPLE = (\[.*\]);$/m)[1]);
const CLI_ONLY_TOOLS = TOOLS.filter((t) => !t.web);
/** "idx stockbit orderbook BBCA" without its arguments: "idx stockbit orderbook". */
const bareCli = (t) => t.cli.split(" ").filter((w) => /^[a-z][a-z-]*$/.test(w)).join(" ");
const DASHES = new RegExp(`[${String.fromCharCode(0x2013, 0x2014)}]`);

/** The variants of one clap enum, as the CLI spells them (BuildSnapshot is build-snapshot). */
function variants(file, name) {
  const src = read("src", "cli", file);
  const start = src.indexOf(`pub enum ${name} {`);
  assert.ok(start >= 0, `${name} in src/cli/${file}`);
  const body = src.slice(start, src.indexOf("\n}\n", start));
  return [...body.matchAll(/^ {4}([A-Z][A-Za-z]*)\b/gm)].map((m) => m[1].replace(/([a-z])([A-Z])/g, "$1-$2").toLowerCase());
}

/** A notice's words come in both languages, without dashes. */
function bilingual(x, what) {
  assert.ok(x && x.id && x.en, `${what} has both languages`);
  assert.notEqual(x.id, x.en, `${what} is translated`);
  assert.doesNotMatch(`${x.id} ${x.en}`, DASHES, what);
}

/** A near command is a line the page runs as typed: one of its commands, never another notice. */
function runsHere(line, what) {
  const r = routeLine(line);
  assert.equal(r.kind, "web", `${what}: ${line} runs here`);
  assert.equal(r.line, line, `${what}: ${line} is the page's own line`);
  assert.equal(r.note, null, `${what}: ${line} needs no rewrite`);
  const head = ALIASES[tokenize(line)[0]] || tokenize(line)[0];
  assert.ok(COMMANDS.includes(head === "json" ? tokenize(line)[1] : head), `${what}: ${line} is a terminal command`);
}

test("every CLI-and-MCP-only tool is reached from its CLI command, its JSON forms and its tool name", () => {
  assert.equal(CLI_ONLY_TOOLS.length, 12);
  for (const tool of CLI_ONLY_TOOLS) {
    const cli = bareCli(tool);
    const group = cli.replace(/^idx /, "");
    const forms = [cli, group, `json ${group}`, `json ${cli}`, `idx -o json ${group}`, `${cli} -o json`, tool.tool, `tools ${tool.tool}`, tool.tool.toUpperCase(), tool.cli];
    for (const line of forms) {
      const r = routeLine(line);
      assert.equal(r.kind, "cli-mcp", `${line}: ${r.kind}`);
      assert.equal(r.tool, tool.tool, line);
      assert.equal(r.path, cliPath(tool), line);
      assert.equal(r.cli.length, 2, line);
      assert.ok(r.cli[0].startsWith(cli), `${line}: ${r.cli[0]}`);
      assert.ok(r.cli[1].startsWith(`idx -o json ${group}`), `${line}: ${r.cli[1]}`);
      assert.deepEqual(r.why, { id: cliOnlyReason(tool, "id"), en: cliOnlyReason(tool, "en") }, line);
      bilingual(r.what, `${tool.tool} what`);
      bilingual(r.why, `${tool.tool} why`);
      for (const n of r.near) { bilingual(n, `${tool.tool} near ${n.line}`); runsHere(n.line, tool.tool); }
    }
    // The tool's own example keeps its arguments; the name alone shows it too.
    assert.equal(routeLine(tool.tool).cli[0], tool.cli.replace(/"/g, ""));
  }
});

test("a notice keeps the user's words: arguments, flags, tickers in the near commands", () => {
  const cases = [
    ["market foreign --date 2026-10-16 -n 20", "idx market foreign --date 2026-10-16 -n 20", ["foreign BBCA", "bandar screen --preset foreign-accumulation"]],
    ["idx -o json market foreign", "idx market foreign", ["foreign BBCA", "bandar screen --preset foreign-accumulation"]],
    // market foreign takes no ticker: it only shapes the near command.
    ["market foreign TLKM", "idx market foreign", ["foreign TLKM", "bandar screen --preset foreign-accumulation"]],
    ["market brokers --top 20 --sort frequency", "idx market brokers --top 20 --sort frequency", ["broker BBCA", "bandar broker YP", "members"]],
    ["brokers TLKM", "idx market brokers", ["broker TLKM", "bandar broker YP", "members"]],
    ["brokers CC", "idx market brokers", ["bandar broker CC", "broker BBCA", "members"]],
    ["market indices LQ45 IDX30", "idx market indices LQ45 IDX30", ["market", "quote ^JKSE"]],
    ["calendar BBCA", "idx market calendar --symbol BBCA", ["actions BBCA", "holidays --upcoming"]],
    ["market calendar --type rups --days 30", "idx market calendar --type rups --days 30", ["actions BBCA", "holidays --upcoming"]],
    ["ownership composition TLKM --as-of 2026-09-30", "idx ownership composition TLKM --as-of 2026-09-30", ["ownership foreign TLKM", "ownership ticker TLKM"]],
    ["own composition BBRI", "idx ownership composition BBRI", ["ownership foreign BBRI", "ownership ticker BBRI"]],
    ["institutions ASII", "idx ownership institutions ASII", ["ownership ticker ASII"]],
    ["ownership sync --force", "idx ownership sync --force", ["ownership releases"]],
    ["ownership status", "idx ownership status", ["ownership releases"]],
    ["stockbit orderbook BBRI -d 20", "idx stockbit orderbook BBRI -d 20", ["quote BBRI", "intraday BBRI"]],
    ["orderbook bbca", "idx stockbit orderbook bbca", ["quote BBCA", "intraday BBCA"]],
    ["idx_orderbook TLKM", "idx stockbit orderbook TLKM", ["quote TLKM", "intraday TLKM"]],
    ["tools idx_orderbook TLKM", "idx stockbit orderbook TLKM", ["quote TLKM", "intraday TLKM"]],
    ["stockbit actions BBCA --kind dividend", "idx stockbit actions BBCA --kind dividend", ["actions BBCA"]],
    ["stockbit calendar --kind dividend --days 30", "idx stockbit calendar --kind dividend --days 30", ["actions BBCA", "holidays --upcoming"]],
    ["movements BBCA --action sell", "idx stockbit movements BBCA --action sell", ["ownership ticker BBCA"]],
    ["stockbit login --page", "idx stockbit login --page", []],
    ["idx --offline stockbit status", "idx --offline stockbit status", []],
    ["market foreign --help", "idx market foreign --help", ["foreign BBCA", "bandar screen --preset foreign-accumulation"]],
  ];
  for (const [line, cli, near] of cases) {
    const r = routeLine(line);
    assert.equal(r.kind, "cli-mcp", line);
    assert.equal(r.cli[0], cli, line);
    assert.equal(r.cli[1], cli.replace(/^idx /, "idx -o json "), line);
    assert.deepEqual(r.near.map((n) => n.line), near, line);
  }
  const stockbit = routeLine("stockbit");
  assert.equal(stockbit.kind, "cli-mcp");
  assert.deepEqual(stockbit.tools, ["idx_orderbook", "idx_stockbit_status", "idx_stockbit_login", "idx_stockbit_actions", "idx_stockbit_movements"]);
  assert.equal(stockbit.cli[0], "idx stockbit login");
});

test("words that can only mean a CLI-and-MCP-only command reach its notice, and none is a terminal command", () => {
  const expect = { orderbook: "idx_orderbook", indices: "idx_indices", calendar: "idx_calendar", composition: "idx_ownership_composition", institutions: "idx_ownership_institutions", movements: "idx_stockbit_movements", brokers: "idx_market_brokers" };
  assert.deepEqual(Object.keys(BARE).sort(), Object.keys(expect).sort());
  for (const [word, tool] of Object.entries(expect)) {
    assert.ok(!COMMANDS.includes(word) && !(word in ALIASES), `${word} is not a terminal command`);
    for (const line of [word, `${word} BBCA`, `json ${word}`, `idx ${word}`]) {
      const r = routeLine(line);
      assert.equal(r.kind, "cli-mcp", line);
      assert.equal(r.tool, tool, line);
    }
  }
  // brokers is one letter from broker, so its notice points there first.
  assert.ok(routeLine("brokers").near[0].line.startsWith("broker "));
});

test("every CLI-only command gets the CLI-only notice, with its own reason", () => {
  const cases = [
    ["watch", "watch", "idx watch", ["quote BBCA,BBRI,TLKM", "returns BBCA BBRI TLKM"]],
    ["watch add BBCA BBRI", "watch", "idx watch add BBCA BBRI", ["quote BBCA,BBRI", "returns BBCA BBRI"]],
    ["watch list", "watch", "idx watch list", ["quote BBCA,BBRI,TLKM", "returns BBCA BBRI TLKM"]],
    ["mcp", "mcp", "idx mcp", ["tools"]],
    ["idx mcp setup claude-code", "mcp", "idx mcp setup claude-code", ["tools"]],
    ["mcp --http 127.0.0.1:8765 --token T", "mcp", "idx mcp --http 127.0.0.1:8765 --token T", ["tools"]],
    ["schema", "schema", "idx schema", ["json quote BBCA"]],
    ["schema history", "schema", "idx schema history", ["json history BBCA"]],
    ["schema bandar-screen", "schema", "idx schema bandar-screen", ["json bandar screen"]],
    ["config set exchange SI", "config", "idx config set exchange SI", []],
    ["cache clear", "cache", "idx cache clear", []],
    ["completions zsh", "completions", "idx completions zsh", []],
    ["version", "version", "idx version", []],
    ["idx --version", "version", "idx version", []],
    ["idx -V", "version", "idx version", []],
    ["stockbit logout", "stockbit logout", "idx stockbit logout", []],
    ["ownership import --latest --family composition", "ownership import", "idx ownership import --latest --family composition", ["ownership releases"]],
    ["ownership resolve candidates", "ownership resolve", "idx ownership resolve candidates", ["ownership search salim"]],
    ["ownership resolve merge \"DRS SURONO SUBEKTI\" \"SURONO SUBEKTI\"", "ownership resolve", "idx ownership resolve merge \"DRS SURONO SUBEKTI\" \"SURONO SUBEKTI\"", ["ownership search \"DRS SURONO SUBEKTI\""]],
    ["ownership discover --family all", "ownership discover", "idx ownership discover --family all", ["ownership releases"]],
    ["ownership path", "ownership path", "idx ownership path", []],
    ["ownership build-snapshot --out build/snap", "ownership build-snapshot", "idx ownership build-snapshot --out build/snap", []],
    ["idx -o json watch", "watch", "idx -o json watch", ["quote BBCA,BBRI,TLKM", "returns BBCA BBRI TLKM"]],
  ];
  for (const [line, path, cli, near] of cases) {
    for (const form of [line, `json ${line}`.replace(/^json idx -o json/, "idx -o json")]) {
      const r = routeLine(form);
      assert.equal(r.kind, "cli", form);
      assert.equal(r.path, path, form);
      bilingual(r.what, `${path} what`);
      bilingual(r.why, `${path} why`);
      assert.match(r.why.en, /CLI only/, path);
      assert.match(r.why.id, /hanya berjalan di CLI/, path);
      assert.deepEqual(r.near.map((n) => n.line), near, form);
      for (const n of r.near) { bilingual(n, `${path} near`); runsHere(n.line, path); }
    }
    assert.deepEqual(routeLine(line).cli, [cli], line);
  }
  // Only idx mcp, which an agent starts, adds the agent's line.
  assert.equal(routeLine("mcp").agent, true);
  assert.equal(routeLine("watch").agent, false);
});

test("the terminal's own lines are never intercepted: commands, aliases, subcommands, examples, tickers", () => {
  const lines = [
    ...COMMANDS, ...Object.keys(ALIASES), ...SAMPLE, ...TOOLS.filter((t) => t.web).map((t) => t.web),
    "BBCA", "bbca bbri", "BBCA.JK", "BBCA,BBRI,TLKM", "json", "json quote BBCA", "json tools idx_quote", "tools idx_quote", "tools idx_nope", "help quote", "help ownership",
    "market", "market now", "market BBCA", "foreign", "broker", "bandar", "ownership", "ownership nope",
    "q BBCA", "own ticker BBCA", "own releases", "s pe<10 --sort div",
    "ownership ticker BBCA --as-of 2026-07-31", "ownership entity \"Dwimuria Investama\" --as-of 2026-08-31", "ownership search \"lo kheng hong\" --limit 5",
    "ownership cross-holders --type individual --origin foreign --min-issuers 5", "ownership concentration --asc BBCA", "ownership concentration BBCA BBRI --by top1",
    "ownership graph \"Anthoni Salim\" --holder --depth 3", "ownership graph AADI --min-pct 5", "ownership changes 2026-08-31 2026-09-30 BBCA --limit 5 --offset 5",
    "ownership foreign BBCA", "ownership foreign --from 2026-02-27 --to 2026-08-31 --limit 30",
    "bandar detect BBCA", "bandar BBCA,BBRI --period 1m --top 5", "bandar BBCA --daily --from 2026-10-01 --to 2026-10-16",
    "bandar screen --preset accumulation --filter acc>=5 --sort net --asc", "bandar broker YP --period 1m --limit 5",
    "screen --returns mcap>10T", "screen pe<10 --preset low-pe --asc --limit 0", "history BBCA 5y 1wk", "history BBCA --period 5d --interval 1h",
    "financials TLKM quarterly --statement income", "earnings --annual BBCA", "earnings BBCA --quarterly --reported", "foreign BBCA --days 5",
    "foreign BBCA --from 2026-10-01 --to 2026-10-16", "broker BBCA,BBRI --investor foreign --board regular", "list --search bank", "members --search mandiri",
    "news --limit 5", "news TLKM --company-only", "insights BBCA --all", "returns BBCA BBRI --benchmark none", "fundamental BBCA --lens risk",
    "actions BBCA --since 2025-01-01", "holidays --upcoming 2027", "holidays 2027 --all", "astro 2026-12-24 --backtest --period 2y", "astro --natal base --days 7",
    // Lines the page has always taken as they are, unknown flags included.
    "news TLKM --source kontan", "foreign BBCA --source idx", "quote BBCA --foo",
  ];
  for (const line of lines) {
    const r = routeLine(line);
    assert.equal(r.kind, "web", `${line}: ${r.kind}`);
    assert.equal(r.line, line, line);
    assert.equal(r.note, null, line);
  }
  // A leading idx alone changes nothing else, and needs no note.
  for (const line of ["quote BBCA", "help", "ownership ticker BBCA", "bandar detect BBCA", "json quote BBCA", "broker BBCA --date 2026-10-16"]) {
    const r = routeLine(`idx ${line}`);
    assert.equal(r.kind, "web", line);
    assert.equal(r.line, line, line);
    assert.equal(r.note, null, line);
  }
});

test("lines in the CLI's words run as the page's own, and say what they ran as", () => {
  const cases = [
    ["idx stocks quote BBCA", "quote BBCA"],
    ["stocks quote BBCA,BBRI", "quote BBCA,BBRI"],
    ["idx -o json stocks quote BBCA", "json quote BBCA"],
    ["idx stocks quote BBCA -o json", "json quote BBCA"],
    ["idx --output=json stocks quote BBCA", "json quote BBCA"],
    ["idx --output json stocks quote BBCA", "json quote BBCA"],
    ["idx -o table stocks quote BBCA", "quote BBCA"],
    ["json stocks quote BBCA", "json quote BBCA"],
    ["quote BBCA -o json", "json quote BBCA"],
    ["idx -ojson market overview", "json market"],
    ["idx market overview", "market"],
    ["market overview", "market"],
    ["idx market holidays --year 2027 --upcoming", "holidays --year 2027 --upcoming"],
    ["market holidays", "holidays"],
    ["idx market members -s mandiri", "members --search mandiri"],
    ["market astro --date 2026-12-24 --days 7 --backtest --period 2y", "astro --date 2026-12-24 --days 7 --backtest --period 2y"],
    ["idx stocks history BBCA -p 5y -i 1wk -n 0", "history BBCA --period 5y --interval 1wk --limit 0"],
    ["idx stocks history BBCA --period=5d --interval=1h", "history BBCA --period 5d --interval 1h"],
    ["idx stocks intraday BBCA -i 5m", "intraday BBCA --interval 5m"],
    ["idx stocks financials TLKM -p quarterly -s income -s cash", "financials TLKM --period quarterly --statement income,cash"],
    ["idx stocks earnings --annual BBCA --estimates", "earnings BBCA --annual --estimates"],
    ["idx stocks technical TLKM -p 2y", "technical TLKM --period 2y"],
    ["idx stocks screen -f \"pe<10,div>5,mcap>5T\" --sort div -n 10", "screen pe<10,div>5,mcap>5T --sort div --limit 10"],
    ["idx stocks screen --preset low-pe --filter mcap>10T --asc", "screen mcap>10T --preset low-pe --asc"],
    ["screen --filter pe<10", "screen pe<10"],
    ["idx stocks returns ASII --no-benchmark", "returns ASII --benchmark none"],
    ["idx stocks news BBCA -n 40 --company-only", "news BBCA --limit 40 --company-only"],
    ["news BBCA -n 5", "news BBCA --limit 5"],
    ["idx stocks list -s bank", "list --search bank"],
    ["idx stocks foreign BBCA -n 10", "foreign BBCA --days 10"],
    ["idx stocks foreign BBCA --source ipot", "foreign BBCA"],
    ["idx stocks actions ENRG --since 2025-01-01", "actions ENRG --since 2025-01-01"],
    ["idx broker BBCA -d 2026-10-16", "broker BBCA --date 2026-10-16"],
    ["idx bandar detect BBCA -p 1m --daily", "bandar detect BBCA --period 1m --daily"],
    ["idx bandar screen --preset accumulation -f acc>=5 -l 10", "bandar screen --preset accumulation --filter acc>=5 --limit 10"],
    ["idx bandar broker YP -p 1m -l 5", "bandar broker YP --period 1m --limit 5"],
    ["idx ownership changes --from 2026-07-31 --to 2026-08-31 -s BBCA -n 100", "ownership changes 2026-07-31 2026-08-31 BBCA --limit 100"],
    ["ownership changes --from 2026-08-31 --to 2026-09-30", "ownership changes 2026-08-31 2026-09-30"],
    ["idx ownership search salim -n 5", "ownership search salim --limit 5"],
    ["idx ownership concentration --ascending BBCA", "ownership concentration BBCA --ascending"],
    ["idx ownership graph --holder \"Anthoni Salim\" --depth 3", "ownership graph \"Anthoni Salim\" --holder --depth 3"],
    ["idx --provider auto stocks quote BBCA", "quote BBCA"],
    ["idx stocks quote --help", "help quote"],
    ["idx bandar screen -h", "help bandar"],
    ["idx", "help"],
    ["stocks", "help"],
    ["idx_quote", "quote BBCA"],
    ["IDX_QUOTE", "quote BBCA"],
    ["idx_quote TLKM", "quote TLKM"],
    ["tools idx_quote TLKM", "quote TLKM"],
    ["idx_market", "market"],
    ["idx_instruments --search bank", "list --search bank"],
    ["idx_bandar_screen --preset accumulation", "bandar screen --preset accumulation"],
    ["idx_ownership_changes --from 2026-08-31 --to 2026-09-30", "ownership changes 2026-08-31 2026-09-30"],
    ["json idx_quote", "json quote BBCA"],
  ];
  for (const [line, want] of cases) {
    const r = routeLine(line);
    assert.equal(r.kind, "web", `${line}: ${r.kind}`);
    assert.equal(r.line, want, line);
    assert.deepEqual(r.note, { line: want, ignored: [] }, line);
    // What it ran as is the page's own line: read again, it stays as it is.
    assert.deepEqual(routeLine(want), { kind: "web", line: want, note: null }, want);
  }
  // Flags that only shape the CLI's own run are left out, and named.
  assert.deepEqual(routeLine("idx --no-color --offline stocks quote BBCA").note, { line: "quote BBCA", ignored: ["--no-color", "--offline"] });
  assert.deepEqual(routeLine("idx -vv --timeout 30 market overview").note, { line: "market", ignored: ["-vv", "--timeout 30"] });
  assert.deepEqual(routeLine("idx -q quote BBCA").note, { line: "quote BBCA", ignored: ["-q"] });
});

test("every MCP tool's CLI example runs here as its web command, or reaches its notice", () => {
  for (const tool of TOOLS) {
    const r = routeLine(tool.cli);
    if (!tool.web) { assert.equal(r.kind, "cli-mcp", tool.tool); continue; }
    assert.equal(r.kind, "web", tool.tool);
    const ran = tokenize(r.line), web = tokenize(tool.web);
    assert.equal(ran[0], web[0], `${tool.tool}: ${r.line} runs ${tool.web}`);
    if (web[0] === "ownership") assert.equal(ran[1], web[1], tool.tool);
    assert.deepEqual(routeLine(r.line), { kind: "web", line: r.line, note: null }, `${tool.tool}: ${r.line}`);
  }
});

test("a CLI flag the page has no twin for gets a notice that says what to type here", () => {
  const cases = [
    ["idx -o csv stocks quote BBCA", null, ["-o csv"], ["quote BBCA", "json quote BBCA"], "idx -o csv stocks quote BBCA"],
    ["idx -o jsonl market overview", null, ["-o jsonl"], ["market", "json market"], "idx -o jsonl market overview"],
    ["quote BBCA -o csv", null, ["-o csv"], ["quote BBCA", "json quote BBCA"], null],
    ["idx --provider stockbit stocks quote BBCA", null, ["--provider stockbit"], ["quote BBCA"], "idx --provider stockbit stocks quote BBCA"],
    ["idx --provider yahoo stocks fundamental BBCA", null, ["--provider yahoo"], ["fundamental BBCA"], "idx --provider yahoo stocks fundamental BBCA"],
    ["idx stocks foreign BBCA --source idx", "foreign", ["--source idx"], ["foreign BBCA"], "idx stocks foreign BBCA --source idx"],
    ["idx stocks news TLKM --source kontan --limit 40", "news", ["--source"], ["news TLKM --limit 40"], "idx stocks news TLKM --source kontan --limit 40"],
    ["idx stocks sentiment BBRI -n 5", "sentiment", ["--limit"], ["sentiment BBRI"], "idx stocks sentiment BBRI -n 5"],
    ["idx stocks screen --universe stocks.csv -f pe<10", "screen", ["--universe"], ["screen pe<10"], "idx stocks screen --universe stocks.csv -f \"pe<10\""],
    ["idx stocks screen -f roe>15 --fundamentals", "screen", ["--fundamentals"], ["screen roe>15"], "idx stocks screen -f \"roe>15\" --fundamentals"],
    ["idx stocks list --board utama", "list", ["--board"], ["list"], "idx stocks list --board utama"],
    ["idx stocks dividends TLKM --limit 20", "dividends", ["--limit"], ["dividends TLKM"], "idx stocks dividends TLKM --limit 20"],
    ["dividends TLKM -n 20", "dividends", ["--limit"], ["dividends TLKM"], null],
    ["idx broker BBCA -n 5", "broker", ["--top"], ["broker BBCA"], "idx broker BBCA -n 5"],
    ["idx market overview --top 15 --min-mcap 5T", "market", ["--top", "--min-mcap"], ["market"], "idx market overview --top 15 --min-mcap 5T"],
    ["idx bandar screen --preset accumulation --scan 50", "bandar screen", ["--scan"], ["bandar screen --preset accumulation"], "idx bandar screen --preset accumulation --scan 50"],
    ["idx bandar broker YP --symbols BBCA,BBRI", "bandar broker", ["--symbols"], ["bandar broker YP"], "idx bandar broker YP --symbols BBCA,BBRI"],
    ["idx ownership graph BBRI --dot", "ownership graph", ["--dot"], ["ownership graph BBRI"], "idx ownership graph BBRI --dot"],
    ["idx ownership entity --exact \"T PERMADI RACHMAT\"", "ownership entity", ["--exact"], ["ownership entity \"T PERMADI RACHMAT\""], "idx ownership entity --exact \"T PERMADI RACHMAT\""],
    ["idx -o json stocks news TLKM -s kontan", "news", ["--source"], ["json news TLKM"], "idx stocks news TLKM -s kontan"],
  ];
  for (const [line, command, flags, near, cli] of cases) {
    const r = routeLine(line);
    assert.equal(r.kind, "flag", `${line}: ${r.kind}`);
    assert.equal(r.command, command, line);
    assert.deepEqual(r.flags, flags, line);
    assert.deepEqual(r.near.map((n) => n.line), near, line);
    for (const n of r.near) { bilingual(n, `${line} near`); assert.equal(routeLine(n.line).kind, "web", n.line); }
    if (cli) assert.deepEqual(r.cli, [cli, cli.replace(/^idx (-o \w+ )?/, "idx -o json ")], line);
    else assert.deepEqual(r.cli, [], line);
    if (r.why) bilingual(r.why, `${line} why`);
  }
  for (const line of ["idx -o csv stocks quote BBCA", "idx --provider stockbit stocks quote BBCA", "idx stocks foreign BBCA --source idx", "idx stocks news TLKM --source kontan", "idx ownership graph BBRI --dot"]) assert.ok(routeLine(line).why, line);
});

test("the router knows every subcommand of the CLI (src/cli/*.rs)", () => {
  const top = variants("mod.rs", "Command");
  assert.deepEqual(top, ["stocks", "ownership", "market", "broker", "bandar", "stockbit", "watch", "config", "cache", "mcp", "schema", "completions", "version"]);
  for (const sub of variants("stocks.rs", "StocksCmd")) {
    const r = routeLine(`stocks ${sub} BBCA`);
    assert.equal(r.kind, "web", sub);
    assert.equal(tokenize(r.line)[0], sub, sub);
    assert.ok(COMMANDS.includes(sub), `stocks ${sub} is the terminal's ${sub}`);
  }
  for (const sub of variants("market.rs", "MarketCmd")) {
    const r = routeLine(`market ${sub}`);
    if (r.kind === "web") assert.equal(r.line, sub === "overview" ? "market" : sub, sub);
    else assert.equal(r.kind, "cli-mcp", sub);
  }
  assert.deepEqual(variants("market.rs", "MarketCmd").filter((s) => routeLine(`market ${s}`).kind !== "web"), ["indices", "foreign", "brokers", "calendar"]);
  for (const sub of variants("ownership.rs", "OwnershipCmd")) {
    const r = routeLine(`ownership ${sub} BBCA`);
    if (r.kind === "web") assert.equal(r.line, `ownership ${sub} BBCA`, sub);
    else assert.ok(["cli-mcp", "cli"].includes(r.kind), sub);
  }
  for (const sub of variants("stockbit.rs", "StockbitCmd")) assert.ok(["cli-mcp", "cli"].includes(routeLine(`stockbit ${sub}`).kind), sub);
  for (const sub of variants("bandar.rs", "BandarCmd")) assert.equal(routeLine(`bandar ${sub} BBCA`).kind, "web", sub);
  for (const cmd of ["watch", "config", "cache", "mcp", "schema", "completions", "version"]) assert.equal(routeLine(cmd).kind, "cli", cmd);
});

test("words that name an object's own properties are words, not lookups", () => {
  for (const w of ["constructor", "__proto__", "toString", "hasOwnProperty", "valueOf"]) {
    for (const line of [w, `market ${w}`, `ownership ${w}`, `bandar ${w}`, `own ${w}`, `quote ${w}`]) {
      const r = routeLine(line);
      assert.deepEqual(r, { kind: "web", line, note: null }, line);
    }
    assert.equal(routeLine(`stockbit ${w}`).kind, "cli-mcp", w);
    assert.equal(routeLine(`stocks ${w}`).line, w, w);
    assert.equal(routeLine(`schema ${w}`).near[0].line, "json quote BBCA", w);
    assert.equal(routeLine(`idx market ${w} --top 5`).kind, "flag", w);
  }
});

test("the install lines are the README's", () => {
  const readme = read("README.md");
  assert.ok(readme.includes(`${INSTALL} `), INSTALL);
  assert.ok(readme.includes(AGENT), AGENT);
});
