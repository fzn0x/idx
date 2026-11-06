// node --test web/test/*.mjs. public/tools.js is the one list of MCP tools the
// terminal's buttons and the landing page use: it must match docs/mcp.md,
// every web command must exist in the terminal, and the landing table must
// list the same tools with the same commands.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { TOOLS, byName, cliOnlyReason, CLI_ONLY_REASON } from "../public/tools.js";
import { instruments, members } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (...p) => readFileSync(join(root, ...p), "utf8");

test("the tool list matches docs/mcp.md", () => {
  const documented = [...read("docs", "mcp.md").matchAll(/^\| `(idx_[a-z_]+)` \|/gm)].map((m) => m[1]);
  assert.equal(documented.length, 43);
  assert.deepEqual(TOOLS.map((t) => t.tool).sort(), [...documented].sort());
  assert.equal(new Set(TOOLS.map((t) => t.tool)).size, TOOLS.length);
  for (const t of TOOLS) {
    assert.ok(t.cli.startsWith("idx "), t.tool);
    assert.ok(t.what.length > 10, t.tool);
  }
  assert.equal(byName(" IDX_QUOTE ").web, "quote BBCA");
  assert.equal(byName("nope"), undefined);
});

test("only the idx.co.id, KSEI composition, local-database, Stockbit and unchecked-source tools are CLI-only, and every web command exists in the terminal", () => {
  assert.deepEqual(TOOLS.filter((t) => !t.web).map((t) => t.tool).sort(), ["idx_calendar", "idx_foreign", "idx_indices", "idx_market_brokers", "idx_market_foreign", "idx_orderbook", "idx_ownership_composition", "idx_ownership_institutions", "idx_ownership_snapshot", "idx_stockbit_login", "idx_stockbit_status"]);
  // A tool with its own reason gives it; the rest share CLI_ONLY_REASON, the Stockbit one or the unchecked-source one.
  for (const t of TOOLS.filter((x) => x.why)) assert.ok(!t.web && t.why.length > 40, t.tool);
  assert.match(byName("idx_ownership_snapshot").why, /own machine/);
  assert.equal(cliOnlyReason(byName("idx_ownership_snapshot")), byName("idx_ownership_snapshot").why);
  // Each CLI-only tool says why, by its source.
  assert.equal(cliOnlyReason(byName("idx_calendar")), CLI_ONLY_REASON);
  assert.match(cliOnlyReason(byName("idx_orderbook")), /^Stockbit realtime carries your own account session/);
  assert.match(cliOnlyReason(byName("idx_stockbit_login")), /^Stockbit realtime/);
  assert.match(cliOnlyReason(byName("idx_ownership_institutions")), /^MSN Money has not been checked from this site's host/);
  for (const t of TOOLS.filter((x) => x.unchecked)) assert.equal(t.web, null, t.tool);
  const terminal = read("web", "public", "terminal.js");
  const names = new Set([...terminal.matchAll(/\{ name: "([a-z]+)", usage:/g)].map((m) => m[1]));
  for (const t of TOOLS.filter((x) => x.web)) assert.ok(names.has(t.web.split(" ")[0]), `${t.tool}: ${t.web}`);
  assert.ok(names.has("tools") && names.has("list") && names.has("members"));
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
    else assert.match(web, /CLI and MCP only/, tool);
  }
  assert.match(html, /idx mcp · 43 tools/);
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
