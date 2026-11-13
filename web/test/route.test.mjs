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

import { routeLine, tokenize, cliPath, shownLine, ALIASES, BARE, INSTALL, AGENT, WEB_SWITCHES, SPEC, CLI_FORM, MCP_ONLY, CLI_ONLY } from "../public/route.js";
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
  assert.equal(CLI_ONLY_TOOLS.length, 11);
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
    ["market indices LQ45 IDX30", "idx market indices LQ45 IDX30", ["market", "quote ^JKSE"]],
    ["calendar BBCA", "idx market calendar --symbol BBCA", ["actions BBCA", "holidays --upcoming"]],
    ["market calendar --type rups --days 30", "idx market calendar --type rups --days 30", ["actions BBCA", "holidays --upcoming"]],
    // A calendar's type words go to --type (Stockbit's to --kind), a day to --date; only a ticker is a --symbol.
    ["calendar dividend", "idx market calendar --type dividend", ["actions BBCA", "holidays --upcoming"]],
    ["market calendar rups", "idx market calendar --type rups", ["actions BBCA", "holidays --upcoming"]],
    ["calendar rups dividend BBRI", "idx market calendar --symbol BBRI --type rups,dividend", ["actions BBRI", "holidays --upcoming"]],
    ["calendar RUPS --type dividend", "idx market calendar --type dividend,rups", ["actions BBCA", "holidays --upcoming"]],
    ["calendar 2026-10-19", "idx market calendar --date 2026-10-19", ["actions BBCA", "holidays --upcoming"]],
    ["calendar 2026-10-19 --days 30", "idx market calendar --days 30", ["actions BBCA", "holidays --upcoming"]],
    ["stockbit calendar dividend", "idx stockbit calendar --kind dividend", ["actions BBCA", "holidays --upcoming"]],
    ["stockbit actions BBCA dividend", "idx stockbit actions BBCA --kind dividend", ["actions BBCA"]],
    ["stockbit actions rups -k ipo", "idx stockbit actions BBCA --kind ipo,rups", ["actions BBCA"]],
    ["ownership composition TLKM --as-of 2026-09-30", "idx ownership composition TLKM --as-of 2026-09-30", ["ownership foreign TLKM", "ownership ticker TLKM"]],
    ["own composition BBRI", "idx ownership composition BBRI", ["ownership foreign BBRI", "ownership ticker BBRI"]],
    ["institutions ASII", "idx ownership institutions ASII", ["ownership ticker ASII"]],
    ["ownership sync --force", "idx ownership sync --force", ["ownership releases"]],
    ["ownership status", "idx ownership status", ["ownership releases"]],
    ["stockbit orderbook BBRI -d 20", "idx stockbit orderbook BBRI -d 20", ["quote BBRI", "intraday BBRI"]],
    ["orderbook bbca", "idx stockbit orderbook bbca", ["quote BBCA", "intraday BBCA"]],
    // A command the CLI gives a ticker to takes the near commands' one when none was typed, and only one.
    ["orderbook", "idx stockbit orderbook BBCA", ["quote BBCA", "intraday BBCA"]],
    ["stockbit orderbook -d 20", "idx stockbit orderbook BBCA -d 20", ["quote BBCA", "intraday BBCA"]],
    ["orderbook TLKM BBRI", "idx stockbit orderbook TLKM", ["quote TLKM", "intraday TLKM"]],
    ["stockbit actions", "idx stockbit actions BBCA", ["actions BBCA"]],
    ["composition", "idx ownership composition BBCA", ["ownership foreign BBCA", "ownership ticker BBCA"]],
    ["ownership composition --as-of 2026-09-30", "idx ownership composition BBCA --as-of 2026-09-30", ["ownership foreign BBCA", "ownership ticker BBCA"]],
    ["institutions", "idx ownership institutions BBCA", ["ownership ticker BBCA"]],
    ["movements", "idx stockbit movements", ["ownership ticker BBCA"]],
    ["idx_orderbook TLKM", "idx stockbit orderbook TLKM", ["quote TLKM", "intraday TLKM"]],
    ["tools idx_orderbook TLKM", "idx stockbit orderbook TLKM", ["quote TLKM", "intraday TLKM"]],
    ["stockbit actions BBCA --kind dividend", "idx stockbit actions BBCA --kind dividend", ["actions BBCA"]],
    ["stockbit calendar --kind dividend --days 30", "idx stockbit calendar --kind dividend --days 30", ["actions BBCA", "holidays --upcoming"]],
    ["movements BBCA --action sell", "idx stockbit movements BBCA --action sell", ["ownership ticker BBCA"]],
    ["stockbit login --page", "idx stockbit login --page", []],
    ["idx --offline stockbit status", "idx --offline stockbit status", []],
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
  const expect = { rotation: "idx_rotation", rrg: "idx_rotation", orderbook: "idx_orderbook", indices: "idx_indices", calendar: "idx_calendar", composition: "idx_ownership_composition", institutions: "idx_ownership_institutions", movements: "idx_stockbit_movements" };
  assert.deepEqual(Object.keys(BARE).sort(), Object.keys(expect).sort());
  for (const [word, tool] of Object.entries(expect)) {
    assert.ok(!COMMANDS.includes(word) && !(word in ALIASES), `${word} is not a terminal command`);
    for (const line of [word, `${word} BBCA`, `json ${word}`, `idx ${word}`]) {
      const r = routeLine(line);
      assert.equal(r.kind, "cli-mcp", line);
      assert.equal(r.tool, tool, line);
    }
  }
});

test("every CLI-only command gets the CLI-only notice, with its own reason", () => {
  const cases = [
    ["watch", "watch", "idx watch", ["quote BBCA,BBRI,TLKM", "returns BBCA BBRI TLKM"]],
    ["watch add BBCA BBRI", "watch", "idx watch add BBCA BBRI", ["quote BBCA,BBRI", "returns BBCA BBRI"]],
    ["watch list", "watch", "idx watch list", ["quote BBCA,BBRI,TLKM", "returns BBCA BBRI TLKM"]],
    // idx watch takes only subcommands: tickers typed after it go to watch add.
    ["watch BBCA", "watch", "idx watch add BBCA", ["quote BBCA", "returns BBCA"]],
    ["watch BBCA BBRI", "watch", "idx watch add BBCA BBRI", ["quote BBCA,BBRI", "returns BBCA BBRI"]],
    // A line short of what the CLI needs takes a read-only default, or asks the CLI for --help.
    ["watch add", "watch", "idx watch add --help", ["quote BBCA,BBRI,TLKM", "returns BBCA BBRI TLKM"]],
    ["mcp setup", "mcp", "idx mcp setup --list", ["tools"]],
    ["config", "config", "idx config get", []],
    ["config set", "config", "idx config set --help", []],
    ["config nope", "config", "idx config --help", []],
    ["cache", "cache", "idx cache stats", []],
    ["completions", "completions", "idx completions bash", []],
    ["ownership import", "ownership import", "idx ownership import --latest", ["ownership releases"]],
    ["ownership import --family composition", "ownership import", "idx ownership import --family composition --latest", ["ownership releases"]],
    ["ownership resolve", "ownership resolve", "idx ownership resolve candidates", ["ownership search salim"]],
    ["ownership resolve undo", "ownership resolve", "idx ownership resolve undo --help", ["ownership search salim"]],
    ["ownership build-snapshot", "ownership build-snapshot", "idx ownership build-snapshot --help", []],
    ["config --help", "config", "idx config --help", []],
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
  // One ticker on the watchlist gets the one-stock quote's words, several the list's.
  assert.deepEqual(routeLine("watch BBCA").near[0], { line: "quote BBCA", id: "harga tertunda dengan rentang hari itu dan grafik", en: "the delayed quote with the day's range and a chart" });
  assert.deepEqual(routeLine("watch add BBCA").near[0], routeLine("watch BBCA").near[0]);
  assert.equal(routeLine("watch BBCA BBRI").near[0].en, "quotes for several stocks at once");
  // Only idx mcp, which an agent starts, adds the agent's line.
  assert.equal(routeLine("mcp").agent, true);
  assert.equal(routeLine("watch").agent, false);
});

test("schema NAME offers the page's command that prints that shape, and none for a CLI-and-MCP-only one", () => {
  const json = read("src", "output", "json.rs");
  const body = json.slice(json.indexOf("pub fn schema_for"), json.indexOf("_ =>", json.indexOf("pub fn schema_for")));
  const names = [...body.matchAll(/^\s+"([a-z-]+)" =>/gm)].map((m) => m[1]);
  assert.ok(names.length > 50, `${names.length} schema names`);
  const cliOnly = ["foreign", "market-foreign", "rotation", "indices", "calendar", "orderbook", "stockbit-actions", "stockbit-movements", "stockbit-status",
    "watchlist", "error", "ownership-import", "ownership-discover", "ownership-resolve-candidates", "ownership-resolution", "ownership-resolutions",
    "ownership-composition", "ownership-snapshot-sync", "ownership-status", "ownership-snapshot", "ownership-institutions"];
  for (const name of names) {
    const near = routeLine(`schema ${name}`).near;
    if (cliOnly.includes(name)) { assert.deepEqual(near, [], name); continue; }
    assert.equal(near.length, 1, name);
    runsHere(near[0].line, `schema ${name}`);
    // Every name has its own command: only quote's shapes fall to the example.
    if (!["quotes", "quote"].includes(name)) assert.notEqual(near[0].line, "json quote BBCA", name);
  }
  assert.equal(routeLine("schema ownership-entity").near[0].line, 'json ownership entity "Dwimuria Investama"');
  assert.equal(routeLine("schema ownership-changes").near[0].line, "json ownership changes 2026-08-31 2026-09-30");
  assert.equal(routeLine("schema foreign-value").near[0].line, "json foreign BBCA");
  assert.equal(routeLine("schema market-foreign-value").near[0].line, "json market foreign");
  // No name, or one the CLI does not know, gets the example.
  assert.equal(routeLine("schema").near[0].line, "json quote BBCA");
  assert.equal(routeLine("schema nope").near[0].line, "json quote BBCA");
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
    "brokers --top 0 --sort volume", "brokers --date 2026-11-06", "brokers CC", "brokers BBCA",
    "market foreign", "market foreign --top 0", "market foreign --date 2026-11-06 --top 5", "json market foreign",
    // Lines the page has always taken as they are, unknown flags included.
    "news TLKM --source kontan", "foreign BBCA --source idx", "brokers --source idx", "quote BBCA --foo",
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
    ["idx market brokers", "brokers"],
    ["market brokers --top 5 --sort frequency", "brokers --top 5 --sort frequency"],
    ["idx -o json market brokers -n 0 -d 2026-11-06", "json brokers --top 0 --date 2026-11-06"],
    // IPOT is the page's own source, and auto reads it here too: idx.co.id refuses this host.
    ["idx market brokers --source ipot", "brokers"],
    ["market brokers --source auto --sort volume", "brokers --sort volume"],
    ["idx_market_brokers", "brokers"],
    ["idx_market_brokers --top 5", "brokers --top 5"],
    // IPOT is the page's own source; the page spells market foreign as the CLI does.
    ["idx market foreign --source ipot", "market foreign"],
    ["market foreign --source ipot -n 20 -d 2026-11-06", "market foreign --top 20 --date 2026-11-06"],
    ["idx -o json market foreign --source=ipot --sort value", "json market foreign"],
    ["idx_market_foreign", "market foreign"],
    ["idx_market_foreign --top 5", "market foreign --top 5"],
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
    ["idx stocks foreign BBCA --source ipot", "foreign BBCA"],
    ["idx stocks foreign BBCA --source ipot -n 10", "foreign BBCA --days 10"],
    // The tool's own name runs the page's foreign, whatever the CLI's default.
    ["idx_foreign TLKM", "foreign TLKM"],
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

test("a line read again, after stocks or as a tool's web example, still reaches its notice", () => {
  const cases = [
    ["idx stocks market foreign", "flag", "idx market foreign"],
    ["stocks ownership composition BBCA", "cli-mcp", "idx ownership composition BBCA"],
    ["stocks orderbook BBCA", "cli-mcp", "idx stockbit orderbook BBCA"],
    ["stocks watch", "cli", "idx watch"],
    ["stocks q BBCA -o csv", "flag", "idx -o csv stocks quote BBCA"],
    ["idx_quote -o csv", "flag", "idx -o csv stocks quote BBCA"],
    ["tools idx_quote -o csv", "flag", "idx -o csv stocks quote BBCA"],
    ["idx_quote --provider stockbit", "flag", "idx --provider stockbit stocks quote BBCA"],
  ];
  for (const [line, kind, cli] of cases) {
    const r = routeLine(line);
    assert.equal(r.kind, kind, line);
    assert.equal(r.cli[0], cli, line);
  }
  assert.deepEqual(routeLine("idx_quote -o csv").flags, ["-o csv"]);
  // What follows stocks runs as typed there.
  assert.deepEqual(routeLine("stocks q BBCA"), { kind: "web", line: "q BBCA", note: { line: "q BBCA", ignored: [] } });
});

test("every MCP tool's CLI example runs here as its web command, or reaches its notice", () => {
  for (const tool of TOOLS) {
    const r = routeLine(tool.cli);
    if (!tool.web) { assert.equal(r.kind, "cli-mcp", tool.tool); continue; }
    // idx stocks foreign reads idx.co.id by default: its notice offers the tool's web command.
    if (["idx_foreign", "idx_market_foreign"].includes(tool.tool)) { assert.equal(r.kind, "flag"); assert.deepEqual(r.near.map((n) => n.line), [tool.web]); continue; }
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
    // The page's short forms get the CLI's: the group in front, positionals as the CLI's flags.
    ["quote BBCA -o csv", null, ["-o csv"], ["quote BBCA", "json quote BBCA"], "idx -o csv stocks quote BBCA"],
    ["idx -o csv quote BBCA", null, ["-o csv"], ["quote BBCA", "json quote BBCA"], "idx -o csv stocks quote BBCA"],
    ["news TLKM -o jsonl", null, ["-o jsonl"], ["news TLKM", "json news TLKM"], "idx -o jsonl stocks news TLKM"],
    ["idx -o csv market", null, ["-o csv"], ["market", "json market"], "idx -o csv market overview"],
    ["idx -o csv holidays", null, ["-o csv"], ["holidays", "json holidays"], "idx -o csv market holidays"],
    ["idx --provider msn quote BBCA", null, ["--provider msn"], ["quote BBCA"], "idx --provider msn stocks quote BBCA"],
    ["bandar BBCA -o csv", null, ["-o csv"], ["bandar BBCA", "json bandar BBCA"], "idx -o csv bandar detect BBCA"],
    ["bandar BBCA --provider stockbit", null, ["--provider stockbit"], ["bandar BBCA"], "idx --provider stockbit bandar detect BBCA"],
    ["own ticker BBCA -o csv", null, ["-o csv"], ["own ticker BBCA", "json own ticker BBCA"], "idx -o csv ownership ticker BBCA"],
    ["idx own graph BBRI --dot", "ownership graph", ["--dot"], ["own graph BBRI"], "idx ownership graph BBRI --dot"],
    ["ownership entity \"Dwimuria Investama\" -o csv", null, ["-o csv"], ["ownership entity \"Dwimuria Investama\"", "json ownership entity \"Dwimuria Investama\""], "idx -o csv ownership entity \"Dwimuria Investama\""],
    ["ownership changes 2026-08-31 2026-09-30 -o csv", null, ["-o csv"], ["ownership changes 2026-08-31 2026-09-30", "json ownership changes 2026-08-31 2026-09-30"], "idx -o csv ownership changes --from 2026-08-31 --to 2026-09-30"],
    ["ownership changes 2026-08-31 2026-09-30 BBCA --limit 5 -o csv", null, ["-o csv"], ["ownership changes 2026-08-31 2026-09-30 BBCA --limit 5", "json ownership changes 2026-08-31 2026-09-30 BBCA --limit 5"], "idx -o csv ownership changes --from 2026-08-31 --to 2026-09-30 --symbol BBCA --limit 5"],
    ["ownership concentration --asc BBCA -o csv", null, ["-o csv"], ["ownership concentration BBCA --asc", "json ownership concentration BBCA --asc"], "idx -o csv ownership concentration BBCA --ascending"],
    ["stocks screen pe<10 -o csv", null, ["-o csv"], ["screen pe<10", "json screen pe<10"], "idx -o csv stocks screen --filter \"pe<10\""],
    ["screen pe<10 div>5 -o csv", null, ["-o csv"], ["screen pe<10 div>5", "json screen pe<10 div>5"], "idx -o csv stocks screen --filter \"pe<10,div>5\""],
    ["stocks history BBCA 5y 1wk -o csv", null, ["-o csv"], ["history BBCA 5y 1wk", "json history BBCA 5y 1wk"], "idx -o csv stocks history BBCA --period 5y --interval 1wk"],
    ["stocks financials TLKM quarterly -o csv", null, ["-o csv"], ["financials TLKM quarterly", "json financials TLKM quarterly"], "idx -o csv stocks financials TLKM --period quarterly"],
    ["market holidays 2027 -o csv", null, ["-o csv"], ["holidays 2027", "json holidays 2027"], "idx -o csv market holidays --year 2027"],
    ["market astro 2026-12-24 -o csv", null, ["-o csv"], ["astro 2026-12-24", "json astro 2026-12-24"], "idx -o csv market astro --date 2026-12-24"],
    ["list bank -o csv", null, ["-o csv"], ["list bank", "json list bank"], "idx -o csv stocks list --search bank"],
    // A word the CLI's line has no place for leaves the CLI line out.
    ["market now -o csv", null, ["-o csv"], ["market now", "json market now"], null],
    ["profile BBCA BBRI -o csv", null, ["-o csv"], ["profile BBCA BBRI", "json profile BBCA BBRI"], null],
    ["idx --provider stockbit stocks quote BBCA", null, ["--provider stockbit"], ["quote BBCA"], "idx --provider stockbit stocks quote BBCA"],
    ["idx --provider yahoo stocks fundamental BBCA", null, ["--provider yahoo"], ["fundamental BBCA"], "idx --provider yahoo stocks fundamental BBCA"],
    ["idx stocks foreign BBCA --source idx", "foreign", ["--source idx"], ["foreign BBCA"], "idx stocks foreign BBCA --source idx"],
    // Every stock in shares from idx.co.id, which refuses this host: the 100 most traded in rupiah from IPOT run here.
    ["market foreign --source idx", "market foreign", ["--source idx"], ["market foreign"], "idx market foreign --source idx"],
    ["market foreign --sort shares -n 5", "market foreign", ["--sort shares"], ["market foreign --top 5"], "idx market foreign --top 5 --sort shares"],
    ["idx -o json market foreign --date 2026-11-06 --source=idx", "market foreign", ["--source idx"], ["json market foreign --date 2026-11-06"], "idx market foreign --date 2026-11-06 --source=idx"],
    // In the CLI's words, the default is idx.co.id's; typed on the page, the same words are the page's.
    ["idx market foreign", "market foreign", ["--source idx"], ["market foreign"], "idx market foreign"],
    ["idx market foreign -n 20", "market foreign", ["--source idx"], ["market foreign --top 20"], "idx market foreign --top 20"],
    // The ranking from idx.co.id, which refuses this host: the same ranking from IPOT runs here.
    ["market brokers --source idx", "brokers", ["--source idx"], ["brokers"], "idx market brokers --source idx"],
    ["idx -o json market brokers --source=idx --date 2026-11-06", "brokers", ["--source idx"], ["json brokers --date 2026-11-06"], "idx market brokers --date 2026-11-06 --source=idx"],
    ["idx market brokers -n 5 --sort volume --source idx", "brokers", ["--source idx"], ["brokers --top 5 --sort volume"], "idx market brokers --top 5 --sort volume --source idx"],
    // --source idx is the CLI's default, so leaving it out is the same CLI command.
    ["idx stocks foreign BBCA", "foreign", ["--source idx"], ["foreign BBCA"], "idx stocks foreign BBCA"],
    ["stocks foreign BBCA -n 10", "foreign", ["--source idx"], ["foreign BBCA --days 10"], "idx stocks foreign BBCA --days 10"],
    ["idx -o json stocks foreign BBCA", "foreign", ["--source idx"], ["json foreign BBCA"], "idx stocks foreign BBCA"],
    // The CLI line puts the flags the page lacks after the ones it runs.
    ["idx stocks news TLKM --source kontan --limit 40", "news", ["--source"], ["news TLKM --limit 40"], "idx stocks news TLKM --limit 40 --source kontan"],
    ["idx stocks sentiment BBRI -n 5", "sentiment", ["--limit"], ["sentiment BBRI"], "idx stocks sentiment BBRI -n 5"],
    ["idx stocks screen --universe stocks.csv -f pe<10", "screen", ["--universe"], ["screen pe<10"], "idx stocks screen --filter \"pe<10\" --universe stocks.csv"],
    ["idx stocks screen -f roe>15 --fundamentals", "screen", ["--fundamentals"], ["screen roe>15"], "idx stocks screen --filter \"roe>15\" --fundamentals"],
    ["idx stocks list --board utama", "list", ["--board"], ["list"], "idx stocks list --board utama"],
    ["idx stocks dividends TLKM --limit 20", "dividends", ["--limit"], ["dividends TLKM"], "idx stocks dividends TLKM --limit 20"],
    ["dividends TLKM -n 20", "dividends", ["--limit"], ["dividends TLKM"], "idx stocks dividends TLKM -n 20"],
    ["idx broker BBCA -n 5", "broker", ["--top"], ["broker BBCA"], "idx broker BBCA -n 5"],
    // Stockbit runs on the user's own session: the CLI and MCP only.
    ["idx broker BBCA --source stockbit", "broker", ["--source stockbit"], ["broker BBCA"], "idx broker BBCA --source stockbit"],
    ["idx broker BBCA -d 2026-11-06 --source stockbit", "broker", ["--source stockbit"], ["broker BBCA --date 2026-11-06"], "idx broker BBCA --date 2026-11-06 --source stockbit"],
    ["idx market overview --top 15 --min-mcap 5T", "market", ["--top", "--min-mcap"], ["market"], "idx market overview --top 15 --min-mcap 5T"],
    ["idx bandar screen --preset accumulation --scan 50", "bandar screen", ["--scan"], ["bandar screen --preset accumulation"], "idx bandar screen --preset accumulation --scan 50"],
    ["idx bandar broker YP --symbols BBCA,BBRI", "bandar broker", ["--symbols"], ["bandar broker YP"], "idx bandar broker YP --symbols BBCA,BBRI"],
    ["idx ownership graph BBRI --dot", "ownership graph", ["--dot"], ["ownership graph BBRI"], "idx ownership graph BBRI --dot"],
    ["idx ownership entity --exact \"T PERMADI RACHMAT\"", "ownership entity", ["--exact"], ["ownership entity \"T PERMADI RACHMAT\""], "idx ownership entity \"T PERMADI RACHMAT\" --exact"],
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
  assert.match(routeLine("market brokers --source idx").why.en, /^The web terminal reads IPOT's ranking of its latest session; idx\.co\.id refuses requests from this site's host/);
  for (const line of ["idx -o csv stocks quote BBCA", "idx --provider stockbit stocks quote BBCA", "idx stocks foreign BBCA --source idx", "idx stocks news TLKM --source kontan", "idx ownership graph BBRI --dot", "idx broker BBCA --source stockbit"]) assert.ok(routeLine(line).why, line);
  // IPOT is what the page reads anyway.
  assert.equal(routeLine("idx broker BBCA --source ipot").kind, "web");
  // Left out, the CLI's default says so: what the CLI reads, and what the page reads instead.
  assert.match(routeLine("idx stocks foreign BBCA").why.en, /^In the CLI, stocks foreign reads idx\.co\.id unless --source ipot/);
  assert.match(routeLine("idx market foreign").why.en, /^In the CLI, market foreign reads idx\.co\.id unless --source ipot is given: every stock in shares/);
  assert.match(routeLine("market foreign --source idx").why.en, /^The web terminal reads IPOT: the foreign net in rupiah of the session's 100 most traded stocks/);
  assert.match(routeLine("market foreign --sort shares").why.en, /^The web terminal ranks IPOT's foreign net in rupiah/);
  assert.match(routeLine("idx stocks foreign BBCA").why.id, /^Di CLI, stocks foreign membaca idx\.co\.id kecuali diberi --source ipot/);
  assert.equal(routeLine("idx stocks foreign BBCA --source idx").why.en, routeLine("stocks foreign BBCA --source idx").why.en);
});

test("every CLI flag the page has no twin for says why, in both languages", () => {
  let flags = 0;
  for (const [cmd, spec] of Object.entries(SPEC)) {
    const [path, slot] = CLI_FORM[cmd];
    for (const flag of spec.cli || []) {
      const line = `idx ${path}${slot?.startsWith("@") ? " BBCA" : ""} --${flag}${spec.switch?.includes(flag) ? "" : " x"}`;
      const r = routeLine(line);
      assert.equal(r.kind, "flag", line);
      assert.ok(r.why, `${line} says why`);
      bilingual(r.why, `${line} why`);
      flags++;
    }
  }
  assert.ok(flags >= 17, `${flags} flags`);
});

test("the router knows every subcommand of the CLI (src/cli/*.rs)", () => {
  const top = variants("mod.rs", "Command");
  assert.deepEqual(top, ["stocks", "ownership", "market", "broker", "bandar", "stockbit", "watch", "config", "cache", "mcp", "schema", "completions", "version"]);
  for (const sub of variants("stocks.rs", "StocksCmd")) {
    // stocks foreign reads idx.co.id unless told --source ipot, the page's source.
    const r = routeLine(`stocks ${sub} BBCA${sub === "foreign" ? " --source ipot" : ""}`);
    // A stocks command the page cannot run yet has its CLI-and-MCP notice.
    if (Object.hasOwn(MCP_ONLY, `stocks ${sub}`)) { assert.equal(r.kind, "cli-mcp", sub); assert.equal(r.tool, MCP_ONLY[`stocks ${sub}`].tool, sub); continue; }
    assert.equal(r.kind, "web", sub);
    assert.equal(tokenize(r.line)[0], sub, sub);
    assert.ok(COMMANDS.includes(sub), `stocks ${sub} is the terminal's ${sub}`);
  }
  for (const sub of variants("market.rs", "MarketCmd")) {
    const r = routeLine(`market ${sub}`);
    if (r.kind === "web") assert.equal(r.line, sub === "overview" ? "market" : sub === "foreign" ? "market foreign" : sub, sub);
    else assert.equal(r.kind, "cli-mcp", sub);
  }
  assert.deepEqual(variants("market.rs", "MarketCmd").filter((s) => routeLine(`market ${s}`).kind !== "web"), ["indices", "calendar"]);
  assert.deepEqual(variants("stocks.rs", "StocksCmd").filter((s) => routeLine(`stocks ${s} BBCA`).kind !== "web" && s !== "foreign"), ["rotation"]);
  assert.equal(routeLine("stocks rrg BBCA TLKM").tool, "idx_rotation");
  // A subcommand the page runs is a CLI_FORM path the terminal handles; any other has a notice of its own.
  const pageRuns = (path) => Object.values(CLI_FORM).some(([p]) => p === path);
  const homes = (path) => [pageRuns(path), Object.hasOwn(MCP_ONLY, path), Object.hasOwn(CLI_ONLY, path)].filter(Boolean).length;
  const ownershipCase = terminal.slice(terminal.indexOf('case "ownership": {'), terminal.indexOf("throw usageLine(cmd.usage);", terminal.indexOf('case "ownership": {')));
  for (const sub of variants("ownership.rs", "OwnershipCmd")) {
    const path = `ownership ${sub}`;
    assert.equal(homes(path), 1, `${path} is routed once`);
    const r = routeLine(`${path} BBCA`);
    if (pageRuns(path)) { assert.equal(r.kind, "web", path); assert.ok(ownershipCase.includes(`sub === "${sub}"`), `the terminal runs ${path}`); }
    else assert.equal(r.path, path, path);
  }
  for (const sub of variants("stockbit.rs", "StockbitCmd")) {
    const path = `stockbit ${sub}`;
    assert.equal(homes(path), 1, `${path} is routed once`);
    // Its own notice, not the general Stockbit panel.
    assert.equal(routeLine(path).path, path, path);
  }
  for (const sub of variants("bandar.rs", "BandarCmd")) {
    assert.ok(pageRuns(`bandar ${sub}`), `bandar ${sub} runs here`);
    assert.match(terminal, new RegExp(`\\(pos\\[0\\] \\|\\| ""\\)\\.toLowerCase\\(\\) === "${sub}"`), `the terminal runs bandar ${sub}`);
    assert.equal(routeLine(`bandar ${sub} BBCA`).kind, "web", sub);
  }
  for (const cmd of ["watch", "config", "cache", "mcp", "schema", "completions", "version"]) assert.equal(routeLine(cmd).kind, "cli", cmd);
});

test("the terminal runs every line through the router, and heads every kind of notice", async () => {
  // run() and NOTICE_META lifted out of terminal.js, with the router itself and stand-ins for the rest.
  const meta = terminal.match(/^const NOTICE_META = .*;$/m)?.[0];
  const body = terminal.match(/^async function run\(line\) \{[\s\S]*?^\}$/m)?.[0];
  assert.ok(meta && body, "NOTICE_META and run() found in terminal.js");
  const ran = [];
  const runWeb = async (line) => { ran.push(line); return { env: {} }; };
  const R = { notice: (plan) => ({ notice: plan }) };
  const { run, NOTICE_META } = new Function("routeLine", "runWeb", "R", "t", "CLI_ONLY", `${meta}\n${body}\nreturn { run, NOTICE_META };`)(routeLine, runWeb, R, (id, en) => en, "CLI and MCP only");
  // A head for every kind of notice the router gives.
  const lines = [...COMMANDS, ...SAMPLE, ...TOOLS.flatMap((t) => [t.cli, t.tool]), ...Object.keys(BARE), ...Object.keys(CLI_ONLY), ...Object.keys(MCP_ONLY), "stockbit", "quote BBCA -o csv", "idx stocks news TLKM -s kontan"];
  const kinds = new Set(lines.map((l) => routeLine(l).kind).filter((k) => k !== "web"));
  assert.deepEqual(Object.keys(NOTICE_META).sort(), [...kinds].sort());
  // A notice answers without running anything.
  for (const line of ["market indices", "idx -o json market indices", "watch BBCA", "idx -o csv stocks quote BBCA", "tools idx_orderbook"]) {
    const plan = routeLine(line);
    assert.deepEqual(await run(line), { local: NOTICE_META[plan.kind], node: { notice: plan } }, line);
  }
  assert.deepEqual(ran, []);
  // A line the page runs reaches runWeb as the router wrote it, with the note that says so.
  assert.deepEqual(await run("idx stocks quote BBCA"), { env: {}, note: { line: "quote BBCA", ignored: [] } });
  assert.deepEqual(await run("quote BBCA"), { env: {} });
  assert.deepEqual(ran, ["quote BBCA", "quote BBCA"]);
});

test("the router and the terminal share one tokenizer, one alias list and the same switches", () => {
  assert.match(terminal, /^import \{ routeLine, tokenize, shownLine, ALIASES, INSTALL, AGENT \} from "\/route\.js";$/m);
  assert.doesNotMatch(terminal, /^function tokenize|^const ALIASES/m);
  const bools = terminal.match(/^const BOOL_FLAGS = new Set\((\[.*\])\);$/m)?.[1];
  assert.deepEqual(JSON.parse(bools), WEB_SWITCHES);
  // The old English-only card is gone; tools idx_<cli-only tool> shows the notice too.
  assert.doesNotMatch(terminal, /cliOnly\(/);
  assert.match(terminal, /node: R\.notice\(routeLine\(tool\.tool\)\)/);
  assert.deepEqual(tokenize(`ownership entity "Dwimuria Investama" 'a b'`), ["ownership", "entity", "Dwimuria Investama", "a b"]);
  assert.equal(byName("idx_quote").web, "quote BBCA");
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

test("a result's heading reads like the CLI line, never idx idx_<tool>", () => {
  assert.equal(shownLine("market foreign"), "idx market foreign");
  assert.equal(shownLine("idx market foreign"), "idx market foreign");
  assert.equal(shownLine("IDX -o json market brokers"), "IDX -o json market brokers");
  assert.equal(shownLine("idx"), "idx");
  assert.equal(shownLine("idx_market_foreign"), "idx_market_foreign");
  assert.equal(shownLine("json idx_market_foreign"), "idx json idx_market_foreign");
  // A word that only starts with idx is still a command of the page.
  assert.equal(shownLine("idxfoo"), "idx idxfoo");
  // The terminal heads every result this way.
  assert.match(terminal, /\[el\("span", \{ class: "no", text: String\(n\)\.padStart\(2, "0"\) \}\), el\("span", \{ words: shownLine\(cmdLine\) \}\)\]/);
});
