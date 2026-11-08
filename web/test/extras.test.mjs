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

test("earnings filter keeps the rows the Rust filter keeps", () => {
  const e = y.parseEarnings(JSON.parse(fixture("yahoo/quote_summary_earnings_tlkm.json")).quoteSummary.result[0], "TLKM");
  const periods = (x) => x.estimates.map((r) => r.period);
  assert.equal(e.scope, null);
  // Neither flag of a pair, or both: untouched, scope null.
  assert.equal(ex.filterEarnings(e, ex.earningsFilter(null, null)), e);
  assert.equal(ex.filterEarnings(e, ex.earningsFilter("all", "all")), e);
  const a = ex.filterEarnings(e, ex.earningsFilter("annual", null));
  assert.deepEqual(a.quarters, []);
  assert.deepEqual(a.quarterly, []);
  assert.equal(a.yearly.length, 4);
  assert.deepEqual(periods(a), ["0y", "+1y"]);
  assert.equal(a.next, null);
  assert.deepEqual(a.scope, { annual: true, quarterly: false, reported: true, estimates: true });
  assert.equal(a.ex_dividend_date, e.ex_dividend_date);
  const qe = ex.filterEarnings(e, ex.earningsFilter("Quarterly", " estimates "));
  assert.deepEqual(periods(qe), ["0q", "+1q"]);
  assert.ok(qe.next);
  assert.deepEqual(qe.yearly, []);
  const r = ex.filterEarnings(e, ex.earningsFilter(null, "reported"));
  assert.deepEqual(r.estimates, []);
  assert.equal(r.next, null);
  assert.equal(r.quarters.length, 4);
  assert.equal(e.estimates.length, 4, "the input is not mutated");
  assert.throws(() => ex.earningsFilter("weekly", null), /invalid period "weekly"/);
  assert.throws(() => ex.earningsFilter(null, "forecasts"), /invalid kind "forecasts"/);
});

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

import { route } from "../src/api.js";

test("ownership entity search key mirrors normalize::entity_key", () => {
  assert.equal(ex.entitySearchKey("PT Alamtri Resources Indonesia, Tbk."), "ALAMTRI RESOURCES INDONESIA");
  assert.equal(ex.entitySearchKey("dwimuria"), "DWIMURIA");
  assert.equal(ex.entitySearchKey("PT"), "PT", "legal forms alone are kept, like the Rust fallback");
  assert.equal(ex.entitySearchKey("%%%"), "");
});

test("ownership entity: a name without 3 letters or digits is a usage error (400) before any query", async () => {
  const env = { DB: { prepare: () => { throw new Error("no query expected"); } } };
  for (const name of ["___", "%%%", "!!!", "PT", "a.b"]) {
    await assert.rejects(
      route(env, new URL(`https://x/api/ownership/entity?name=${encodeURIComponent(name)}`)),
      (e) => e.code === "usage" && e.status === 400 && /3 letters or digits|3 characters/.test(e.message),
      name,
    );
  }
});

import { plan, partsKey, sha256Hex } from "../src/ownership-sync.js";

test("ownership sync plan: newest first, prune outside the window, reload on new sha", () => {
  const index = { schema: "idx.ownership.mirror.v1", releases: ["2026-09-30", "2026-08-31", "2026-07-31", "2026-06-30"].map((d, i) => ({ as_of: d, sha256: `s${i}`, parts: [] })) };
  const h = (sha256, parts_key = null) => ({ sha256, parts_key });
  assert.equal(plan(index, new Map(), 3).load.as_of, "2026-09-30");
  assert.equal(plan(index, new Map([["2026-09-30", h("s0")]]), 3).load.as_of, "2026-08-31");
  assert.equal(plan(index, new Map([["2026-09-30", h("old")], ["2026-08-31", h("s1")]]), 3).load.as_of, "2026-09-30", "re-published file is reloaded");
  assert.deepEqual(plan(index, new Map([["2026-01-30", h("x")], ["2026-09-30", h("s0")]]), 3), { prune: "2026-01-30" });
  assert.deepEqual(plan(index, new Map([["2026-09-30", h("s0")], ["2026-08-31", h("s1")], ["2026-07-31", h("s2")]]), 3), { upToDate: 3 });
  assert.throws(() => plan({ schema: "nope", releases: [] }, new Map(), 3));
});

test("ownership sync plan: rebuilt parts reload a month once, old indexes never force it", () => {
  const parts = [{ file: "a.part01.json", sha256: "p1" }, { file: "a.part02.json", sha256: "p2" }];
  assert.equal(partsKey({ parts }), "p1,p2");
  assert.equal(partsKey({}), "");
  // Index from idx 0.8.1 (no positions): only the file hash counts, so a
  // month loaded before the 0004 migration (parts_key null) stays.
  const old = { schema: "idx.ownership.mirror.v1", releases: [{ as_of: "2026-09-30", sha256: "s0", rows: 7151, parts }] };
  assert.deepEqual(plan(old, new Map([["2026-09-30", { sha256: "s0", parts_key: null }]]), 3), { upToDate: 1 });
  // Index from idx 0.8.2 (positions): the same file with rebuilt parts reloads...
  const rebuilt = { schema: "idx.ownership.mirror.v1", releases: [{ as_of: "2026-09-30", sha256: "s0", rows: 7151, lines: 7154, positions: 7151, parts: [{ sha256: "q1" }, { sha256: "q2" }] }] };
  assert.equal(plan(rebuilt, new Map([["2026-09-30", { sha256: "s0", parts_key: null }]]), 3).load.as_of, "2026-09-30");
  assert.equal(plan(rebuilt, new Map([["2026-09-30", { sha256: "s0", parts_key: "p1,p2" }]]), 3).load.as_of, "2026-09-30");
  // ...once: after loading it D1 holds the new parts key.
  assert.deepEqual(plan(rebuilt, new Map([["2026-09-30", { sha256: "s0", parts_key: "q1,q2" }]]), 3), { upToDate: 1 });
});

// Parts exported by scripts/ownership_mirror.py from a database idx built out of
// the two accounts excerpts (tests/fixtures/ownership/README.md).
const part = (d) => JSON.parse(fixture(`ownership/mirror_part_${d}_accounts_constructed.json`));

test("ownership changes pair summed positions like ownership::db::changes", () => {
  const aug = part("20260831"), sep = part("20260930");
  const kresna = sep.find((r) => r.symbol === "KREN" && r.entity_key === "KRESNA PRIMA INVEST");
  assert.equal(kresna.pct, 13.45);
  assert.equal(kresna.accounts, 2);
  assert.equal(kresna.controller_or_affiliate, null);
  const changes = ex.diffPositions(aug, sep).sort((a, b) => Math.abs(b.pct_change) - Math.abs(a.pct_change) || a.symbol.localeCompare(b.symbol));
  const rows = changes.map((c) => [c.symbol, c.entity_key, c.kind, c.pct_change]);
  assert.equal(changes.length, 7, JSON.stringify(rows));
  const find = (sym, key) => changes.find((c) => c.symbol === sym && c.entity_key === key);
  const k = find("KREN", "KRESNA PRIMA INVEST");
  assert.deepEqual([k.kind, k.pct_change, k.shares_change, k.accounts_from, k.accounts_to], ["changed", 4.01, 729400000, 1, 2]);
  const bp = find("PANS", "BANK PAN INDONESIA");
  assert.deepEqual([bp.kind, bp.pct_change, bp.accounts_from, bp.accounts_to], ["exited", -29, 2, null]);
  assert.deepEqual([find("PANS", "PANIN BANK").kind, find("PANS", "PANIN BANK").pct_change], ["new", 29]);
  assert.equal(find("KREN", "KARUNIA PERDANA ISTENIA").pct_change, -5.13);
  assert.equal(find("ALKA", "WENANTO GUNAWAN").pct_change, 0.03);
  assert.equal(find("TGUK", "BUDIANTO M KURNIAWAN").pct_change, 0.01);
  assert.deepEqual([find("PANS", "WIDAYANTI DARMAWAN").pct_change, find("PANS", "WIDAYANTI DARMAWAN").shares_change], [0, 20500]);
  assert.equal(find("TGUK", "DINASTI KREATIF INDONESIA"), undefined, "same shares, scrip moved to scripless");
  assert.equal(find("ALKA", "GESIT PERKASA"), undefined, "83.79 -> 83.80 is the rounding of two lines");
  const s = ex.summariseChanges(changes);
  assert.deepEqual([s.new_holders, s.exits, s.increased, s.decreased, s.net_pp], [1, 2, 3, 0, -1.08]);
  // Field order matches the Rust Change struct (CSV and JSON parity).
  assert.deepEqual(Object.keys(k), ["symbol", "holder", "entity_key", "investor_type", "local_foreign", "kind", "pct_from", "pct_to", "pct_change", "shares_from", "shares_to", "shares_change", "accounts_from", "accounts_to", "controller_or_affiliate"]);
  // Single lines on both sides keep any move: the issued share count changed.
  const one = { symbol: "X", entity_key: "A", holder: "A", shares_total: 100, pct: 1.0, accounts: 1 };
  assert.equal(ex.diffPositions([one], [{ ...one, pct: 1.01 }]).length, 1);
  assert.equal(ex.diffPositions([one], [{ ...one, pct: 1.01, accounts: 2 }]).length, 0);
  assert.equal(ex.diffPositions([{ ...one, accounts: null }], [{ ...one, pct: 1.01, accounts: null }]).length, 1, "legacy rows count as one line");
});

/** A D1 stand-in that answers by the first table named in the SQL. */
function fakeD1(tables) {
  const stmt = (sql) => {
    const t = Object.keys(tables).find((name) => sql.includes(name));
    const rows = () => (typeof tables[t] === "function" ? tables[t](sql) : tables[t]) || [];
    const api = { bind: () => api, all: async () => ({ results: rows() }), first: async () => rows()[0] ?? null, run: async () => ({}) };
    return api;
  };
  return { prepare: stmt };
}

test("ownership releases on the web: lines, positions and legacy copies", async () => {
  const releases = [
    { as_of: "2026-08-31", format: "xlsx", source: "a", rows: 7156, synced_at: 1, sha256: "s1", issuers: 961, entities: 5100, lines: null, other_date_rows: null, controller_column: null },
    { as_of: "2026-09-30", format: "xlsx", source: "b", rows: 7151, synced_at: 2, sha256: "s2", issuers: 961, entities: 5145, lines: 7154, other_date_rows: 0, controller_column: 0 },
  ];
  const env = { DB: fakeD1({ own_releases: releases, kv: [] }) };
  const r = await route(env, new URL("https://x/api/ownership/releases"));
  assert.equal(r.schema, "idx.ownership.releases.v1");
  const [legacy, current] = r.data;
  assert.deepEqual([legacy.rows, legacy.positions, legacy.other_date_rows, legacy.controller_column], [7156, null, null, null]);
  assert.deepEqual([current.rows, current.positions, current.other_date_rows, current.controller_column], [7154, 7151, 0, false]);
  assert.ok(r.warnings.some((w) => w.includes("2026-08-31") && w.includes("predates idx 0.8.2")), JSON.stringify(r.warnings));
  assert.ok(!r.warnings.some((w) => w.includes("2026-09-30")), JSON.stringify(r.warnings));
});

test("ownership notes on the web: rows dated otherwise agree with their count (ingest::other_date_note)", async () => {
  const release = (as_of, other_date_rows) => ({ as_of, format: "xlsx", source: "a", rows: 7151, synced_at: 2, sha256: "s", issuers: 961, entities: 5145, lines: 7154, other_date_rows, controller_column: 0 });
  const env = { DB: fakeD1({ own_releases: [release("2026-08-31", 1), release("2026-09-30", 2)], kv: [] }) };
  const r = await route(env, new URL("https://x/api/ownership/releases"));
  assert.deepEqual(r.warnings, [
    "the 2026-08-31 file also lists 1 row dated otherwise; it is not part of the 2026-08-31 holdings",
    "the 2026-09-30 file also lists 2 rows dated otherwise; they are not part of the 2026-09-30 holdings",
  ]);
});

test("ownership ticker on the web carries accounts and the controller flag", async () => {
  const holdings = [
    { as_of: "2026-10-30", symbol: "BBCA", issuer_name: "BANK CENTRAL ASIA Tbk", entity_key: "DWIMURIA INVESTAMA ANDALAN", holder: "PT DWIMURIA INVESTAMA ANDALAN", holder_raw: "PT DWIMURIA INVESTAMA ANDALAN", investor_type: "CORPORATE", local_foreign: "L", nationality: null, domicile: "INDONESIA", shares_scripless: 0, shares_scrip: 67729950000, shares_total: 67729950000, pct: 54.94, accounts: 1, controller_or_affiliate: "CONTROLLER" },
  ];
  const release = { as_of: "2026-10-30", lines: 11, other_date_rows: 0, controller_column: 1 };
  const env = { DB: fakeD1({ own_holdings: holdings, own_releases: [release] }) };
  const r = await route(env, new URL("https://x/api/ownership/ticker?symbol=BBCA"));
  assert.equal(r.data.controller_reported, true);
  assert.equal(r.data.holders[0].controller_or_affiliate, "CONTROLLER");
  assert.equal(r.data.holders[0].accounts, 1);
  assert.deepEqual(Object.keys(r.data), ["symbol", "issuer_name", "as_of", "listed_pct", "holders", "controller_reported"]);
  assert.equal(r.warnings, undefined);
});

test("sha256Hex matches a known digest", async () => {
  assert.equal(await sha256Hex("hello"), "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
});

test("the earnings endpoint rejects an unknown period or kind before fetching", async () => {
  const { earnings } = await import("../src/api.js");
  for (const [period, kind] of [["weekly", null], [null, "forecasts"]]) {
    await assert.rejects(earnings({}, "TLKM", period, kind), (e) => e.code === "usage" && e.exit === 2 && e.status === 400);
  }
});
