// node --test web/test/*.mjs. public/tools.js is the one list of MCP tools the
// terminal's buttons and the landing page use: it must match docs/mcp.md,
// every web command must exist in the terminal, and the landing table must
// list the same tools with the same commands.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { TOOLS, byName, cliOnlyReason, whatOf, CLI_ONLY_REASON } from "../public/tools.js";
import { instruments, members } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (...p) => readFileSync(join(root, ...p), "utf8");

test("the tool list matches docs/mcp.md", () => {
  const documented = [...read("docs", "mcp.md").matchAll(/^\| `(idx_[a-z_]+)` \|/gm)].map((m) => m[1]);
  assert.equal(documented.length, 48);
  assert.deepEqual(TOOLS.map((t) => t.tool).sort(), [...documented].sort());
  assert.equal(new Set(TOOLS.map((t) => t.tool)).size, TOOLS.length);
  for (const t of TOOLS) {
    assert.ok(t.cli.startsWith("idx "), t.tool);
    assert.ok(t.what.length > 10, t.tool);
  }
  assert.equal(byName(" IDX_QUOTE ").web, "quote BBCA");
  assert.equal(byName("nope"), undefined);
});

test("only the idx.co.id, KSEI composition, local-database, Stockbit, unchecked-source and not-yet-drawn tools are CLI-only, and every web command exists in the terminal", () => {
  assert.deepEqual(TOOLS.filter((t) => !t.web).map((t) => t.tool).sort(), ["idx_calendar", "idx_indices", "idx_orderbook", "idx_ownership_composition", "idx_ownership_institutions", "idx_ownership_snapshot", "idx_rotation", "idx_stockbit_actions", "idx_stockbit_login", "idx_stockbit_movements", "idx_stockbit_status"]);
  // The rotation graph waits for a scatter chart on the page.
  assert.match(cliOnlyReason(byName("idx_rotation"), "en"), /scatter chart/);
  assert.match(cliOnlyReason(byName("idx_rotation"), "id"), /diagram sebar/);
  // A tool with its own reason gives it; the rest share CLI_ONLY_REASON, the Stockbit one or the unchecked-source one.
  for (const t of TOOLS.filter((x) => x.why)) assert.ok(!t.web && t.why.length > 40, t.tool);
  assert.match(byName("idx_ownership_snapshot").why, /own machine/);
  assert.equal(cliOnlyReason(byName("idx_ownership_snapshot")), byName("idx_ownership_snapshot").why);
  // Each CLI-only tool says why, by its source.
  assert.equal(cliOnlyReason(byName("idx_calendar")), CLI_ONLY_REASON);
  // Top brokers run here from IPOT's ranking, which this host can read.
  assert.equal(byName("idx_market_brokers").web, "brokers");
  // So does the market's foreign flow, from IPOT in rupiah for the most traded stocks.
  assert.equal(byName("idx_market_foreign").web, "market foreign");
  assert.match(cliOnlyReason(byName("idx_orderbook")), /^The Stockbit tools carry your own account session/);
  assert.match(cliOnlyReason(byName("idx_stockbit_login")), /^The Stockbit tools/);
  assert.match(cliOnlyReason(byName("idx_stockbit_movements")), /^The Stockbit tools/);
  assert.match(cliOnlyReason(byName("idx_ownership_institutions")), /^MSN Money has not been checked from this site's host/);
  for (const t of TOOLS.filter((x) => x.unchecked)) assert.equal(t.web, null, t.tool);
  const terminal = read("web", "public", "terminal.js");
  const names = new Set([...terminal.matchAll(/\{ name: "([a-z]+)", usage:/g)].map((m) => m[1]));
  for (const t of TOOLS.filter((x) => x.web)) assert.ok(names.has(t.web.split(" ")[0]), `${t.tool}: ${t.web}`);
  assert.ok(names.has("tools") && names.has("list") && names.has("members"));
});

test("a CLI-and-MCP-only tool says what it gives and why in the page's language", () => {
  // The words are each landing page's, row for row: the Indonesian page's and the English one's.
  const unescape = (s) => s.replace(/&quot;/g, '"').replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
  const cellsOf = (...page) => new Map([...read("web", "public", ...page).matchAll(/<tr><td>(idx_[a-z_]+)<\/td><td><code>[^<]+<\/code><\/td><td>.*?<\/td><td class="muted">(.*?)<\/td><\/tr>/g)].map((m) => [m[1], unescape(m[2])]));
  const cells = cellsOf("index.html"), cellsEn = cellsOf("en", "index.html");
  for (const t of TOOLS.filter((x) => !x.web)) {
    assert.equal(whatOf(t, "en"), t.what, t.tool);
    assert.equal(whatOf(t, "id"), cells.get(t.tool), t.tool);
    assert.equal(whatOf(t, "en"), cellsEn.get(t.tool), `${t.tool} on the English page`);
    const en = cliOnlyReason(t, "en"), id = cliOnlyReason(t, "id");
    // Outside a browser the page's language is English.
    assert.equal(cliOnlyReason(t), en, t.tool);
    assert.notEqual(id, en, t.tool);
    if (t.tool === "idx_ownership_snapshot") assert.match(id, /komputer Anda sendiri/);
    else assert.match(id, /hanya berjalan di CLI dan server MCP/, t.tool);
    assert.doesNotMatch(`${id} ${en}`, new RegExp(`[${String.fromCharCode(0x2013, 0x2014)}]`), t.tool);
  }
  // A tool that runs here keeps its English words on both pages.
  assert.equal(whatOf(byName("idx_quote"), "id"), byName("idx_quote").what);
});

test("the landing page lists every tool with its commands", () => {
  const html = read("web", "public", "index.html");
  const start = html.indexOf('aria-labelledby="cmd-h"');
  assert.ok(start > 0, "the Daftar Perintah section exists");
  const section = html.slice(start, html.indexOf("</section>", start));
  const rows = [...section.matchAll(/<tr><td>(idx_[a-z_]+)<\/td><td><code>([^<]+)<\/code><\/td><td>(.*?)<\/td>/g)];
  assert.deepEqual(rows.map((r) => r[1]), TOOLS.map((t) => t.tool));
  const unescape = (s) => s.replace(/&quot;/g, '"').replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
  for (const [, tool, cli, web] of rows) {
    const t = byName(tool);
    assert.equal(unescape(cli), t.cli, tool);
    if (t.web) assert.ok(web.includes(`href="/terminal?cmd=${encodeURIComponent(t.web)}"`), tool);
    else assert.match(web, /hanya CLI dan MCP/, tool);
  }
  assert.match(html, /idx mcp · 48 alat/);
  // The English page carries the same rows, linking to the English terminal.
  const en = read("web", "public", "en", "index.html");
  const enStart = en.indexOf('aria-labelledby="cmd-h"');
  const enRows = [...en.slice(enStart, en.indexOf("</section>", enStart)).matchAll(/<tr><td>(idx_[a-z_]+)<\/td><td><code>([^<]+)<\/code><\/td><td>(.*?)<\/td>/g)];
  assert.deepEqual(enRows.map((r) => r[1]), TOOLS.map((t) => t.tool));
  for (const [, tool, , web] of enRows) {
    const t = byName(tool);
    if (t.web) assert.ok(web.includes(`href="/en/terminal?cmd=${encodeURIComponent(t.web)}"`), tool);
    else assert.match(web, /CLI and MCP only/, tool);
  }
  assert.match(en, /idx mcp · 48 tools/);
});

test("/api/instruments and /api/members serve the bundled lists", () => {
  const all = instruments();
  assert.equal(all.schema, "idx.instruments.v1");
  assert.equal(all.provider, "bundled");
  assert.ok(all.data.total > 900);
  assert.equal(all.data.instruments.length, all.data.total);
  assert.match(all.warnings[0], /bundled universe/);
  const bank = instruments("central asia");
  assert.deepEqual(bank.data.instruments.map((i) => i.symbol), ["BBCA"]);
  assert.deepEqual([bank.data.instruments[0].board, bank.data.instruments[0].listing_date], [null, null]);
  assert.equal(instruments(null, "utama").data.instruments.length, 0);
  const m = members("mandiri");
  assert.equal(m.schema, "idx.members.v1");
  assert.deepEqual(m.data.members, [{ code: "CC", name: "MANDIRI SEKURITAS", licenses: null }]);
  assert.equal(m.data.total, 89);
  assert.ok(members().data.members.every((x, i, l) => i === 0 || l[i - 1].code < x.code));
});
