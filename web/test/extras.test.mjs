// node --test web/test/*.mjs (no network). The JS ports in src/extras.js
// must agree with src/analysis/{returns,dividends}.rs and service::arrange.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as ex from "../src/extras.js";
import * as y from "../src/yahoo.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", p), "utf8");

test("dividend events parse and enrich like the Rust side", () => {
  const h = y.parseEvents(JSON.parse(fixture("yahoo/chart_events_bbca_max.json")));
  assert.equal(h.symbol, "BBCA");
  assert.equal(h.dividends.length, 47);
  assert.equal(h.splits.length, 2);
  assert.equal(h.splits[1].ratio, "5:1");
  assert.equal(h.dividends[0].date, "2004-07-01");
  ex.enrichDividends(h, "2026-10-17");
  assert.equal(h.ttm_payments, 4);
  assert.equal(h.ttm_total, 381);
  assert.equal(h.ttm_yield_pct, 7.36);
  assert.equal(h.consecutive_years, 23);
  assert.equal(h.last.date, "2026-08-31");
  const y2026 = h.years[h.years.length - 1];
  assert.deepEqual([y2026.year, y2026.payments, y2026.total, y2026.change_pct], [2026, 3, 326, 6.89]);
});

const hist = (rows) => ({
  symbol: "T", name: null, currency: null, period: "5y", interval: "1d",
  candles: rows.map(([d, c]) => ({ time: `${d}T09:00:00+07:00`, open: c, high: c, low: c, close: c, adj_close: null, volume: 1 })),
});

test("return windows resolve on calendar dates (same cases as analysis::returns)", () => {
  const p = ex.performance(hist([
    ["2021-10-18", 50], ["2023-10-16", 80], ["2025-10-15", 100], ["2025-12-31", 120],
    ["2026-04-17", 110], ["2026-07-17", 125], ["2026-09-17", 130], ["2026-10-10", 128], ["2026-10-17", 140],
  ]));
  assert.deepEqual(p.returns, { "1w": 9.38, "1m": 7.69, "3m": 12, "6m": 27.27, ytd: 16.67, "1y": 40, "3y": 75, "5y": 180 });
  assert.equal(p.cagr_3y_pct, 20.48);
  assert.ok(p.cagr_5y_pct > 22 && p.cagr_5y_pct < 23);
  assert.equal(p.pct_from_52w_high, 0);
  assert.equal(p.days_since_52w_high, 0);
  assert.deepEqual(ex.minusWindows(p.returns, { "1y": 10 })["1y"], 30);
  assert.equal(ex.minusWindows(p.returns, { "1y": 10 })["1m"], null);
});

test("drawdown and extremes", () => {
  const p = ex.performance(hist([["2026-01-05", 100], ["2026-01-06", 120], ["2026-01-07", 90], ["2026-01-08", 60], ["2026-01-09", 121], ["2026-01-12", 110]]));
  assert.deepEqual(p.max_drawdown_all, { pct: -50, peak_date: "2026-01-06", trough_date: "2026-01-08", recovered: true });
  assert.equal(p.best_day_1y.pct, 101.67);
  assert.equal(p.worst_day_1y.pct, -33.33);
  assert.equal(p.volatility_1y_pct, null);
  assert.equal(p.returns["1y"], null);
});

// Same cases as the Rust tests beta_of_a_doubled_index_is_two,
// beta_needs_enough_returns_and_a_moving_index and common_keeps_only_shared_dates.
const days = (start, closes) => closes.map((c, i) => [new Date(Date.parse(`${start}T00:00:00Z`) + i * 86400e3).toISOString().slice(0, 10), c]);
const indexPath = (n) => { const out = [1000]; for (let i = 1; i < n; i++) out.push(out[i - 1] * (1 + [0.01, -0.005, 0.002, -0.012, 0.007][i % 5])); return out; };

test("beta against a benchmark on shared dates", () => {
  const idx = indexPath(200);
  const stock = [500];
  for (let i = 1; i < idx.length; i++) stock.push(stock[i - 1] * (1 + 2 * (idx[i] / idx[i - 1] - 1)));
  assert.deepEqual(ex.beta(days("2026-01-01", stock), days("2026-01-01", idx)), { beta: 2, correlation: 1, observations: 199 });
  const s = days("2026-01-01", indexPath(120));
  assert.deepEqual(ex.beta(s, s), { beta: null, correlation: null, observations: 119 });
  const flat = days("2026-01-01", Array(200).fill(100)), moving = days("2026-01-01", indexPath(200));
  assert.equal(ex.beta(moving, flat).beta, null, "index never moved");
  assert.deepEqual([ex.beta(flat, moving).beta, ex.beta(flat, moving).correlation], [0, null]);
});

test("common keeps only the dates both series have", () => {
  const a = days("2026-01-01", [1, 2, 3, 4]), b = days("2026-01-02", [20, 30, 40, 50]);
  const [ca, cb] = ex.common(a, b);
  assert.deepEqual(ca.map((x) => x[0]), ["2026-01-02", "2026-01-03", "2026-01-04"]);
  assert.deepEqual(cb.map((x) => x[1]), [20, 30, 40]);
  const [ga, gb] = ex.common(a.filter((x) => x[1] !== 3), b);
  assert.equal(ga.length, 2);
  assert.deepEqual(gb.map((x) => x[1]), [20, 40]);
  assert.equal(ex.common(a, [])[0].length, 0);
});

test("news: company headlines first, market context fills the limit", () => {
  const mk = (title) => ({ title, summary: "", matched_by: "all", scope: "market" });
  const matches = (it, t) => (it.title.toUpperCase().split(/[^A-Z0-9]+/).includes(t) ? "ticker" : null);
  const feed = [mk("IHSG menguat"), mk("BBCA bagi dividen"), mk("Rupiah melemah"), mk("Laba BBCA naik"), mk("Harga emas")];
  const a = ex.arrangeNews(feed.map((x) => ({ ...x })), "BBCA", null, 3, false, matches);
  assert.equal(a.companyMatches, 2);
  assert.deepEqual(a.items.map((i) => [i.title, i.scope]), [["BBCA bagi dividen", "company"], ["Laba BBCA naik", "company"], ["IHSG menguat", "market"]]);
  assert.equal(ex.arrangeNews(feed.map((x) => ({ ...x })), "BBCA", null, 3, true, matches).items.length, 2);
  assert.equal(ex.arrangeNews(feed.map((x) => ({ ...x })), null, null, 0, false, matches).items.length, 5);
});

test("ownership changes summary", () => {
  const s = ex.summariseChanges([
    { symbol: "AADI", holder: "A", entity_key: "A", investor_type: "CORPORATE", local_foreign: "L", kind: "changed", pct_from: 39.82, pct_to: 41.1, pct_change: 1.28 },
    { symbol: "AADI", holder: "B", entity_key: "B", investor_type: "CORPORATE", local_foreign: "A", kind: "new", pct_from: null, pct_to: 1.15, pct_change: 1.15 },
    { symbol: "BBCA", holder: "C", entity_key: "C", investor_type: "INDIVIDUAL", local_foreign: "L", kind: "exited", pct_from: 1.02, pct_to: null, pct_change: -1.02 },
  ]);
  assert.equal(s.issuers, 2);
  assert.equal(s.holders, 3);
  assert.deepEqual([s.new_holders, s.exits, s.increased, s.decreased], [1, 1, 1, 0]);
  assert.equal(s.net_pp, 1.41);
  assert.equal(s.foreign_net_pp, 1.15);
  assert.equal(s.local_net_pp, 0.26);
  assert.equal(s.issuer_flow[0].symbol, "AADI");
  assert.equal(s.top_increases[0].holder, "A");
  assert.equal(s.top_decreases[0].holder, "C");
});

import { plan, sha256Hex } from "../src/ownership-sync.js";

test("ownership sync plan: newest first, prune outside the window, reload on new sha", () => {
  const index = { schema: "idx.ownership.mirror.v1", releases: ["2026-09-30", "2026-08-31", "2026-07-31", "2026-06-30"].map((d, i) => ({ as_of: d, sha256: `s${i}`, parts: [] })) };
  assert.equal(plan(index, new Map(), 3).load.as_of, "2026-09-30");
  assert.equal(plan(index, new Map([["2026-09-30", "s0"]]), 3).load.as_of, "2026-08-31");
  assert.equal(plan(index, new Map([["2026-09-30", "old"], ["2026-08-31", "s1"]]), 3).load.as_of, "2026-09-30", "re-published file is reloaded");
  assert.deepEqual(plan(index, new Map([["2026-01-30", "x"], ["2026-09-30", "s0"]]), 3), { prune: "2026-01-30" });
  assert.deepEqual(plan(index, new Map([["2026-09-30", "s0"], ["2026-08-31", "s1"], ["2026-07-31", "s2"]]), 3), { upToDate: 3 });
  assert.throws(() => plan({ schema: "nope", releases: [] }, new Map(), 3));
});

test("sha256Hex matches a known digest", async () => {
  assert.equal(await sha256Hex("hello"), "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
});
