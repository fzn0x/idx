// node --test web/test/*.mjs (no network). /api/foreign: the web twin of
// `idx stocks foreign --source ipot`, on the same recorded IPOT pages as
// service::foreign_flow_ipot's tests, with the same totals.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as ipot from "../src/ipot.js";
import { foreignFlow } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", "ipot", p), "utf8");
const page = (p) => ipot.parse(fixture(p));

// Minimal D1 stand-in: every read misses, writes succeed.
const env = { DB: { prepare: () => ({ bind: () => ({ first: async () => null, run: async () => ({}) }) }) } };

/** IPOT from the recorded single sessions; anything else fails like the network. */
function recordedFetch(calls) {
  return async (u) => {
    const url = String(u);
    calls.push(url);
    const d = url.match(/start=(\d\d)\/(\d\d)\/(\d{4})&end=(\d\d)\/(\d\d)\/(\d{4})/);
    const side = url.includes("fd=F") ? "_foreign" : "";
    try {
      return new Response(fixture(`brokersummary_bbca_${d[3]}${d[1]}${d[2]}${side}.html`), { status: 200 });
    } catch {
      return new Response("busy", { status: 503 });
    }
  };
}

async function withFetch(fn, body) {
  const realFetch = globalThis.fetch;
  globalThis.fetch = fn;
  try {
    await body();
  } finally {
    globalThis.fetch = realFetch;
  }
}

test("one session from its two pages, like ipot::foreign_day", () => {
  const s = ipot.foreignDay("2026-10-16", page("brokersummary_bbca_20261016.html"), page("brokersummary_bbca_20261016_foreign.html"), "2026-10-17");
  assert.equal(s.session, "traded");
  assert.equal(s.rounded, true);
  assert.deepEqual(
    [s.day.value, s.day.lots, s.day.average_price, s.day.foreign_buy_value, s.day.foreign_sell_value, s.day.foreign_net_value],
    [6_600_000_000_000, 12_600_000, 5_228, 1_700_000_000_000, 4_000_000_000_000, -2_300_000_000_000],
  );
  assert.ok(Math.abs(s.day.foreign_share_pct - (5.7 / 13.2) * 100) < 1e-9);
  // TRUK traded and no foreigner did: zeros.
  const t = ipot.foreignDay("2026-11-05", page("brokersummary_truk_20261105.html"), page("brokersummary_truk_20261105_foreign.html"), "2026-11-06");
  assert.deepEqual([t.day.foreign_buy_value, t.day.foreign_sell_value, t.day.foreign_net_value, t.day.foreign_share_pct], [0, 0, 0, 0]);
  // No trades; and today while the session trades (totals, no lists).
  assert.equal(ipot.foreignDay("2026-10-17", page("brokersummary_bbca_20261017_no_trades.html"), null, "2026-10-17").session, "none");
  const live = page("brokersummary_bbca_20261106_insession.html");
  assert.equal(live.total_value, 89_500_000_000);
  assert.equal(ipot.foreignDay("2026-11-06", live, null, "2026-11-06").session, "pending");
});

test("totals over the week of 10-12, like service::foreign_value_totals", () => {
  const days = ["12", "13", "14", "15", "16"].map((d) => ipot.foreignDay(`2026-10-${d}`, page(`brokersummary_bbca_202610${d}.html`), page(`brokersummary_bbca_202610${d}_foreign.html`), "2026-10-17").day);
  const t = ipot.foreignTotals(days);
  assert.deepEqual(
    [t.sessions, t.foreign_buy_value, t.foreign_sell_value, t.foreign_net_value, t.value, t.net_buy_sessions, t.net_sell_sessions, t.streak],
    [5, 4_364_700_000_000, 6_187_800_000_000, -1_823_100_000_000, 9_937_300_000_000, 3, 2, -1],
  );
  assert.ok(Math.abs(t.foreign_share_pct - 53.095) < 0.001, t.foreign_share_pct);
  assert.equal(ipot.foreignTotals([]).foreign_share_pct, null);
});

test("/api/foreign reads a range from IPOT and refuses idx.co.id", async () => {
  const calls = [];
  await withFetch(recordedFetch(calls), async () => {
    const out = await foreignFlow(env, "bbca", null, "2026-10-12", "2026-10-16");
    assert.equal(out.schema, "idx.foreign.value.v1");
    assert.equal(out.provider, "ipot");
    const fl = out.data;
    assert.deepEqual([fl.symbol, fl.name, fl.from, fl.to, fl.days.length, fl.rounded], ["BBCA", "BANK CENTRAL ASIA Tbk", "2026-10-12", "2026-10-16", 5, true]);
    assert.equal(fl.totals.foreign_net_value, -1_823_100_000_000);
    assert.equal(calls.length, 10, "two pages per session");
    assert.equal(out.warnings, undefined);

    // 10-09 is not recorded: a warning, the rest stands.
    const part = await foreignFlow(env, "BBCA", null, "2026-10-09", "2026-10-12");
    assert.deepEqual(part.data.days.map((d) => d.date), ["2026-10-12"]);
    assert.match(part.warnings[0], /^BBCA: the session of 2026-10-09 failed/);

    for (const bad of [
      ["BBCA", null, null, null, "idx"],
      ["BBCA", null, null, null, "rti"],
      ["BBCA", "3", "2026-10-12"],
      ["BBCA", null, null, "2026-10-16"],
      ["BBCA", null, "2026-10-16", "2026-10-12"],
      ["BBCA", null, "2026-01-01", "2099-01-01"],
      ["BBCA", "21"],
      ["BBCA", "0"],
      ["BBCA", null, "2026-09-01", "2026-10-16"],
      ["^JKSE"],
    ]) await assert.rejects(foreignFlow(env, ...bad), (e) => e.code === "usage", JSON.stringify(bad));
  });
});

/** IPOT from `routes` (a substring of the URL to a fixture file, first match); anything else fails like the network. */
function routedFetch(routes, calls = []) {
  return async (u) => {
    const url = String(u);
    calls.push(url);
    const hit = routes.find(([part]) => url.includes(part));
    return hit ? new Response(fixture(hit[1]), { status: 200 }) : new Response("busy", { status: 503 });
  };
}

test("a past session with trades but no broker lists is pending, like ipot::foreign_day", () => {
  const early = page("brokersummary_bbca_20261106_insession.html");
  assert.equal(ipot.unpublished(early), true);
  assert.equal(ipot.unpublished(page("brokersummary_bbca_20261017_no_trades.html")), false);
  assert.equal(ipot.unpublished(page("brokersummary_bbca_20261105.html")), false);
  assert.equal(ipot.foreignDay("2026-11-05", early, null, "2026-11-06").session, "pending");
});

test("/api/foreign leaves out a session not published yet, and says so when nothing else traded", async () => {
  // 11-05 as if read before the source published it (the shape of 11-06 at 09:15 WIB).
  const routes = [
    ["start=11/05/2026&end=11/05/2026&fd=all", "brokersummary_bbca_20261106_insession.html"],
    ["start=11/04/2026&end=11/04/2026&fd=all", "brokersummary_bbca_20261104.html"],
    ["start=11/04/2026&end=11/04/2026&fd=F", "brokersummary_bbca_20261104_foreign.html"],
  ];
  const calls = [];
  await withFetch(routedFetch(routes, calls), async () => {
    const out = await foreignFlow(env, "BBCA", null, "2026-11-04", "2026-11-05");
    assert.deepEqual(out.data.days.map((d) => d.date), ["2026-11-04"]);
    assert.ok(out.warnings.includes("BBCA: the session of 2026-11-05 is left out: the source shows its totals but has not published its broker lists and foreign figures yet"), JSON.stringify(out.warnings));
    assert.equal(calls.filter((u) => u.includes("start=11/05/2026") && u.includes("fd=F")).length, 0);
    await assert.rejects(foreignFlow(env, "BBCA", null, "2026-11-05", "2026-11-05"), (e) => e.code === "not_found" && e.message.includes("the source has not published the session of 2026-11-05 yet"));
  });
});

test("/api/foreign reports a failed day rather than no trades, like the CLI", async () => {
  // 10-08 read without trades, 10-09 failed: that day may have traded.
  await withFetch(routedFetch([["start=10/08/2026&end=10/08/2026", "brokersummary_bbca_20261017_no_trades.html"]]), async () => {
    await assert.rejects(foreignFlow(env, "BBCA", null, "2026-10-08", "2026-10-09"), (e) => e.code === "provider" && /HTTP 503/.test(e.message));
    await assert.rejects(foreignFlow(env, "BBCA", null, "2026-10-08", "2026-10-08"), (e) => e.code === "not_found" && /no sessions with trades/.test(e.message));
  });
});

test("a closed page read before publication is cached for minutes, not the week", async () => {
  const writes = [];
  const recording = { DB: { prepare: () => ({ bind: (...args) => ({ first: async () => null, run: async () => { writes.push(args); return {}; } }) }) } };
  const routes = [
    ["start=11/05/2026&end=11/05/2026&fd=all", "brokersummary_bbca_20261106_insession.html"],
    ["start=11/04/2026&end=11/04/2026&fd=all", "brokersummary_bbca_20261104.html"],
    ["start=11/04/2026&end=11/04/2026&fd=F", "brokersummary_bbca_20261104_foreign.html"],
  ];
  await withFetch(routedFetch(routes), async () => {
    await foreignFlow(recording, "BBCA", null, "2026-11-04", "2026-11-05");
  });
  const ttl = (k) => { const w = writes.find((a) => a[0] === k); return w[2] - w[3]; };
  assert.equal(ttl("ipot:BBCA:2026-11-05:2026-11-05:all:all"), 5 * 60);
  assert.equal(ttl("ipot:BBCA:2026-11-04:2026-11-04:all:all"), 7 * 86400);
});
