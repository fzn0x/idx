// node --test web/test/*.mjs (no network). The Worker side of MSN insights
// and the screen's return windows: the JS ports in src/msn-insights.js and
// src/msn-screen.js against the fixtures the Rust tests use, and
// /api/insights and /api/screen over an in-memory D1 and recorded responses.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as msn from "../src/msn-insights.js";
import * as ms from "../src/msn-screen.js";
import { insights, screen } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const raw = (f) => readFileSync(join(root, "tests", "fixtures", "msn", f), "utf8");
const fixture = (f) => JSON.parse(raw(f));

test("insights parse like providers::msn::insights (BBCA)", () => {
  const p = msn.parseInsights(fixture("insights_bbca.json"), "BBCA");
  const r = p.report;
  assert.equal(r.name, "Bank Central Asia Tbk PT");
  assert.equal(r.insights.length, 30);
  assert.deepEqual([r.good, r.bad, r.neutral, r.not_rated], [17, 4, 0, 9]);
  assert.equal(r.updated, "2026-11-05T17:51:31Z");
  assert.deepEqual(p.peerIds, ["bkfmar", "bkfgqh", "cj53lh", "bkgttc"]);
  assert.deepEqual(p.warnings, []);
  assert.deepEqual(r.peers, []);
  assert.deepEqual(r.insights[2], {
    category: "Valuation", name: "P/B ratio",
    statement: "BBCA is trading at a P/B multiple (2.10x) above weighted average of its peers (0.56x)",
    short_statement: r.insights[2].short_statement, status: "bad", criteria: "lower_is_better",
    value: 2.09778355810005, benchmark_value: 0.564114114036551, compared_with: "peers",
  });
  assert.equal(r.insights[3].compared_with, "industry");
  const by = (n) => r.insights.find((i) => i.name === n);
  assert.equal(by("Analyst price target").compared_with, "price");
  assert.equal(by("Analyst rating").compared_with, "threshold");
  assert.equal(by("Earnings (last Q)").compared_with, "own_history");
  assert.equal(by("Change in Debt to equity over last 5 years (Quarterly data)").compared_with, "own_history");
  assert.equal(r.insights[0].status, "not_rated");
  assert.equal(r.insights[0].value, 10.2114621902489, "real figures stay");
  const cr = by("Current ratio (Last Quarter) vs peers");
  assert.deepEqual([cr.status, cr.value, cr.benchmark_value], ["not_rated", null, null], "0/0 placeholder becomes null");
  assert.equal(new Set(r.insights.map((i) => i.category)).size, 5);
});

test("insights: GOTO's neutral item, the 404 body and an unknown rating", () => {
  const g = msn.parseInsights(fixture("insights_goto_neutral.json"), "GOTO").report;
  assert.deepEqual([g.good, g.bad, g.neutral, g.not_rated], [13, 2, 1, 14]);
  assert.equal(g.insights.find((i) => i.status === "neutral").compared_with, "industry");
  assert.equal(msn.parseInsights(fixture("insights_bogus_404.json"), "ZZZZ"), null);
  assert.equal(msn.parseInsights([], "ZZZZ"), null);
  const odd = JSON.parse(raw("insights_bbca.json").replace('"evaluationStatus":"bad"', '"evaluationStatus":"meh"'));
  const p = msn.parseInsights(odd, "BBCA");
  assert.deepEqual([p.report.bad, p.report.not_rated], [3, 10]);
  assert.equal(p.warnings.length, 1);
  assert.match(p.warnings[0], /"meh"/);
});

test("peers resolve in MSN's order; the lookup picks the XIDX listing", () => {
  const ids = ["bkfmar", "bkfgqh", "cj53lh", "bkgttc"];
  assert.deepEqual(msn.parsePeers(fixture("quotes_bbca_insight_peers.json"), ids), ["BBKP", "BABP", "SUPA", "INPC"]);
  assert.equal(msn.parsePeers(fixture("quotes_bbca_insight_peers.json"), [...ids].reverse())[0], "INPC");
  assert.equal(msn.parseAutosuggest(fixture("autosuggest_bbca.json"), "BBCA"), "bn91jc");
  assert.equal(msn.parseAutosuggest(fixture("autosuggest_goto_ambiguous.json"), "GOTO"), "c5tv8m");
  assert.equal(msn.parseAutosuggest(fixture("autosuggest_zzzz_no_idx_match.json"), "ZZZZ"), null);
});

test("the market list parses like providers::msn::screener", () => {
  const l = ms.parseMarketList(fixture("screener_region_id_limit12.json"));
  assert.deepEqual([Object.keys(l.returns).length, l.listed, l.count], [12, 12, 887]);
  assert.deepEqual(l.returns.BBCA, [3.7449, -17.004, -20.5426, -13.8655, -36.5325, -41.092]);
  const h = ms.parseMarketList(fixture("screener_52wkhi_id_complete.json"));
  assert.deepEqual([Object.keys(h.returns).length, h.count], [38, 38]);
  assert.deepEqual(h.returns.GLOB, [null, null, null, null, null, null]);
  assert.deepEqual(h.returns.IBST.slice(0, 4), [null, null, null, null]);
  assert.equal(h.returns.ALKA[5], 1300.2732);
  assert.deepEqual(ms.parseMarketList(fixture("screener_lowpe_id_empty.json")).returns, {});
  const row = ms.applyReturns({ symbol: "BBCA" }, l.returns.BBCA);
  assert.equal(row.return_1y_pct, -41.092);
  assert.equal(ms.applyReturns({}, undefined).return_1w_pct, null);
  assert.ok(ms.MARKET_LIST_BODY.includes('"st_reg_id"') && ms.MARKET_LIST_BODY.includes('"limit":2000'));
});

/** In-memory D1 for the cache and kv statements. */
function memoryDb() {
  const kv = new Map(), cache = new Map();
  const run = (sql, a) => {
    if (sql.startsWith("SELECT v, updated_at FROM kv")) return kv.has(a[0]) ? { v: kv.get(a[0]), updated_at: 0 } : null;
    if (sql.startsWith("INSERT OR REPLACE INTO kv")) return kv.set(a[0], a[1]);
    if (sql.startsWith("SELECT v, fetched_at, expires_at FROM cache")) return cache.get(a[0]) || null;
    if (sql.startsWith("INSERT OR REPLACE INTO cache")) return cache.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] });
    throw new Error("memoryDb: unexpected SQL " + sql);
  };
  const stmt = (sql, a = []) => ({ bind: (...b) => stmt(sql, b), first: async () => run(sql, a), run: async () => (run(sql, a), {}) });
  return { DB: { prepare: (sql) => stmt(sql) }, kv, cache };
}

/** MSN from the recorded responses; `fail` names routes that answer 500. */
function recordedMsn(calls, fail = []) {
  return async (u, init = {}) => {
    const url = String(u);
    calls.push(`${init.method || "GET"} ${url}`);
    const answer = (f, status = 200) => new Response(raw(f), { status });
    if (fail.some((f) => url.includes(f))) return new Response("{}", { status: 500 });
    if (url.includes("csautosuggest") && url.includes("query=BBCA")) return answer("autosuggest_bbca.json");
    if (url.includes("csautosuggest")) return answer("autosuggest_zzzz_no_idx_match.json");
    if (url.includes("finance/insights?ids=bn91jc")) return answer("insights_bbca.json");
    if (url.includes("Finance/Quotes?ids=bkfmar,bkfgqh,cj53lh,bkgttc")) return answer("quotes_bbca_insight_peers.json");
    if (url.includes("Finance/Screener") && init.method === "POST" && init.body === ms.MARKET_LIST_BODY) return answer("screener_region_id_limit12.json");
    return new Response("[]", { status: 404 });
  };
}

async function withFetch(fn, body) {
  const realFetch = globalThis.fetch;
  globalThis.fetch = fn;
  try { await body(); } finally { globalThis.fetch = realFetch; }
}

test("/api/insights: the CLI envelope, peers resolved, cached in D1", async () => {
  const env = memoryDb(), calls = [];
  await withFetch(recordedMsn(calls), async () => {
    const out = await insights(env, "bbca");
    assert.equal(out.schema, "idx.insights.v1");
    assert.equal(out.provider, "msn");
    assert.equal(out.warnings, undefined);
    assert.deepEqual(out.data.peers, ["BBKP", "BABP", "SUPA", "INPC"]);
    assert.equal(out.data.good, 17);
    assert.ok(calls.some((c) => c.startsWith("GET https://api.msn.com/msn/v0/pages/finance/insights?ids=bn91jc&apikey=")));
    const n = calls.length;
    const again = await insights(env, "BBCA");
    assert.equal(again.cached, true);
    assert.equal(calls.length, n, "served from D1");
    await assert.rejects(insights(env, "ZZZZ"), (e) => e.code === "not_found" && e.exit === 4);
    await assert.rejects(insights(env, "BB$CA"), (e) => e.code === "usage");
  });
  const env2 = memoryDb();
  await withFetch(recordedMsn([], ["Finance/Quotes"]), async () => {
    const out = await insights(env2, "BBCA");
    assert.deepEqual(out.data.peers, []);
    assert.match(out.warnings[0], /^could not resolve MSN's peer list/);
  });
  await withFetch(recordedMsn([], ["finance/insights"]), async () => {
    await assert.rejects(insights(memoryDb(), "BBCA"), (e) => e.code === "provider" && e.exit === 3);
  });
});

const boardQuote = (symbol, price, low, high, market_cap, pe_trailing) => ({ symbol, name: null, price, change_pct: 0, market_cap, pe_trailing, week52_low: low, week52_high: high });

test("/api/screen: presets expand, return windows join from one POST", async () => {
  const env = memoryDb(), calls = [];
  env.kv.set("board", JSON.stringify({ ts: 1000, quotes: [
    boardQuote("BBCA", 5175, 4820, 8750, 6.36e14, 10.96), boardQuote("BBRI", 2640, 2540, 4050, 3.98e14, 6.49), boardQuote("TLKM", 1900, 1900, 3990, 1.87e14, 10.98),
  ] }));
  await withFetch(recordedMsn(calls), async () => {
    const low = await screen(env, "", null, 25, 20, null, "low-pe");
    assert.deepEqual([low.data.preset, low.data.sort, low.data.descending, low.data.returns_source], ["low-pe", "pe", false, null]);
    assert.deepEqual(low.data.filters, ["pe>0"]);
    assert.deepEqual(low.data.rows.map((r) => r.symbol), ["BBRI", "BBCA", "TLKM"]);
    assert.equal(calls.length, 0, "no return key, no MSN request");
    const lo52 = await screen(env, "", null, 25, 20, null, "52w-low");
    assert.deepEqual(lo52.data.rows.map((r) => [r.symbol, r.range_52w_pct]), [["TLKM", 0]]);

    const top = await screen(env, "", null, 25, 20, null, "top-performers");
    assert.deepEqual([top.data.sort, top.data.descending, top.data.returns_source], ["ret1y", true, "msn:screener"]);
    assert.deepEqual(top.data.rows.map((r) => [r.symbol, r.return_1y_pct]), [["BBRI", -31.9095], ["BBCA", -41.092], ["TLKM", -43.7853]]);
    assert.ok(top.warnings.some((w) => w.startsWith("MSN listed 12 of 887 stocks")));
    assert.equal(calls.filter((c) => c.startsWith("POST https://assets.msn.com/service/Finance/Screener?apikey=")).length, 1);
    const byYtd = await screen(env, "ret1y<-40", "ytd", 25, 20, false);
    assert.deepEqual(byYtd.data.rows.map((r) => r.symbol), ["TLKM", "BBCA"]);
    assert.equal(byYtd.data.sort, "retytd");
    assert.equal(calls.length, 1, "the market list is cached in D1");
    await assert.rejects(screen(env, "", null, 25, 20, null, "best-ever"), (e) => e.code === "usage");
  });
  const down = memoryDb();
  down.kv.set("board", env.kv.get("board"));
  await withFetch(recordedMsn([], ["Finance/Screener"]), async () => {
    await assert.rejects(screen(down, "ret1y>0"), (e) => e.code === "provider", "a needed list that fails is an error");
    const opt = await screen(down, "pe<20", null, 25, 20, null, null, true);
    assert.equal(opt.data.rows.length, 3);
    assert.ok(opt.warnings.some((w) => w.startsWith("return windows unavailable")));
  });
});
