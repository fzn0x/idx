// node --test web/test/*.mjs (no network). /api/intraday and the 1m/2m
// intervals: the JS ports in src/extras.js must give the numbers
// src/analysis/intraday.rs gives on the same recorded Yahoo charts.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as ex from "../src/extras.js";
import * as y from "../src/yahoo.js";
import { history, intraday, checkPeriod, route } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", "yahoo", p), "utf8");
const BBCA = fixture("chart_bbca_1d_1m.json");
const JKSE = fixture("chart_jkse_1d_1m.json");

test("session summary and VWAP match the Rust side", () => {
  const h = y.parseChart(JSON.parse(BBCA), "1d", "1m");
  const bars = ex.sessionBars(h.candles);
  assert.equal(bars.length, 333);
  assert.deepEqual(ex.intradaySummary(bars), {
    first_bar: "2026-11-05T09:00:00+07:00", last_bar: "2026-11-05T16:14:00+07:00", bars: 333,
    open: 5025, high: 5125, low: 5025, close: 5125, bars_volume: 155142100, vwap: 5089.98,
  });
  const two = y.parseChart(JSON.parse(fixture("chart_bbca_1d_2m.json")), "1d", "2m");
  assert.equal(ex.intradaySummary(ex.sessionBars(two.candles)).vwap, 5089.76);
  // In session: the 09:02:16 last-trade snapshot row is not a bar.
  const live = ex.intradaySummary(ex.sessionBars(y.parseChart(JSON.parse(fixture("chart_bbca_1d_1m_session.json")), "1d", "1m").candles));
  assert.deepEqual([live.bars, live.last_bar, live.bars_volume, live.vwap], [2, "2026-11-06T09:01:00+07:00", 697300, 5033.33]);
  const ix = ex.intradaySummary(ex.sessionBars(y.parseChart(JSON.parse(JKSE), "1d", "1m").candles));
  assert.equal(ix.bars, 320);
  assert.equal(ix.bars_volume, 0);
  assert.equal(ix.vwap, null);
  assert.equal(ex.intradaySummary([]), null);
  assert.deepEqual(ex.sessionBars([]), []);
  const mk = (time, close) => ({ time, open: close, high: close, low: close, close, adj_close: null, volume: 100 });
  const two_days = [mk("2026-11-04T15:59:00+07:00", 1), mk("2026-11-05T09:00:00+07:00", 2), mk("2026-11-05T09:01:00+07:00", 3)];
  assert.deepEqual(ex.sessionBars(two_days).map((c) => c.close), [2, 3]);
});

test("a last-trade snapshot stamped on a whole minute is not a bar (parse.rs does the same)", () => {
  const hm = (h) => h.candles.map((c) => c.time.slice(11, 16));
  // After the close, the five-day window's snapshot at 16:14:00 instead of 16:14:43.
  const five = y.parseChart(JSON.parse(fixture("chart_bbca_5d_1m_open.json").replace(",1793870083]", ",1793870040]")), "5d", "1m");
  assert.equal(five.candles.length, 1334);
  assert.deepEqual([five.candles.at(-1).time, five.candles.at(-1).volume], ["2026-11-05T16:14:00+07:00", 20200]);
  // In session at 1m: a null 09:02 row, then the snapshot at 09:02:00.
  const session = y.parseChart(JSON.parse(fixture("chart_bbca_1d_1m_session.json").replace(",1793930536]", ",1793930520]")), "1d", "1m");
  assert.deepEqual(hm(session), ["09:00", "09:01"]);
  // In session at 5m: the in-progress 09:25 bar, then a snapshot off the grid or repeating 09:25.
  const body = (rows) => ({ chart: { result: [{ meta: { symbol: "BBCA.JK" }, timestamp: rows.map((r) => r[0]),
    indicators: { quote: [{ open: rows.map((r) => r[1]), high: rows.map((r) => r[1]), low: rows.map((r) => r[1]), close: rows.map((r) => r[1]), volume: rows.map((r) => r[2]) }] } }] } });
  const open = 1793930400;
  for (const snap of [open + 28 * 60, open + 25 * 60, open + 28 * 60 + 41]) {
    const h = y.parseChart(body([[open + 20 * 60, 5025, 1000], [open + 25 * 60, 5050, 500], [snap, 5050, 0]]), "1d", "5m");
    assert.deepEqual(hm(h), ["09:20", "09:25"], `snapshot at ${snap}`);
  }
  // The grid counts from the window's first row: hourly bars on the half hour stay.
  const half = open + 1800;
  assert.deepEqual(hm(y.parseChart(body([[half, 100, 10], [half + 3600, 101, 10], [half + 9000, 101, 0]]), "1d", "1h")), ["09:30", "10:30"]);
  // Daily bars are left alone.
  assert.equal(y.parseChart(body([[open, 1, 1], [open + 1234, 2, 1]]), "1mo", "1d").candles.length, 2);
});

test("1m and 2m keep to the windows Yahoo serves, before any fetch", async () => {
  assert.doesNotThrow(() => checkPeriod("5d", "1m"));
  assert.doesNotThrow(() => checkPeriod("1mo", "2m"));
  assert.doesNotThrow(() => checkPeriod("max", "1d"));
  for (const [period, interval, hint] of [["1mo", "1m", "1d or 5d"], ["3mo", "2m", "1d, 5d or 1mo"]]) {
    await assert.rejects(history({}, "BBCA", period, interval), (e) => e.code === "usage" && e.message.includes(hint));
  }
  await assert.rejects(intraday({}, "BBCA", "1d"), (e) => e.code === "usage" && /intraday takes 1m/.test(e.message));
  await assert.rejects(intraday({}, "BB$CA"), (e) => e.code === "usage");
});

/** In-memory D1 for the cache and kv statements history and quote run. */
function memoryDb() {
  const kv = new Map(), cache = new Map();
  const run = (sql, a) => {
    if (sql.startsWith("SELECT v, updated_at FROM kv")) return kv.has(a[0]) ? { v: kv.get(a[0]), updated_at: 0 } : null;
    if (sql.startsWith("SELECT v, fetched_at, expires_at FROM cache")) return cache.get(a[0]) || null;
    if (sql.startsWith("INSERT OR REPLACE INTO cache")) return cache.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] });
    throw new Error("memoryDb: unexpected SQL " + sql);
  };
  const stmt = (sql, a = []) => ({ bind: (...b) => stmt(sql, b), first: async () => run(sql, a), run: async () => (run(sql, a), {}) });
  return { DB: { prepare: (sql) => stmt(sql) }, kv };
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

test("/api/intraday: bars, the board quote and the summary in the CLI envelope", async () => {
  const env = memoryDb();
  const q = y.parseV7Quote(JSON.parse(fixture("quote_v7_bbca.json")).quoteResponse.result[0]);
  // A board refreshed just now: its own age (boardFreshness) is not under test here.
  env.kv.set("board", JSON.stringify({ ts: Math.floor(Date.now() / 1000), quotes: [q] }));
  const calls = [];
  await withFetch(async (u) => {
    calls.push(String(u));
    if (String(u).includes("BBCA.JK?range=1d&interval=1m")) return new Response(BBCA, { status: 200 });
    throw new Error("unexpected fetch " + u);
  }, async () => {
    const out = await route(env, new URL("https://x/api/intraday?symbol=bbca&limit=20"));
    assert.equal(out.schema, "idx.intraday.v1");
    assert.equal(out.provider, "yahoo");
    const d = out.data;
    assert.deepEqual([d.symbol, d.name, d.session, d.interval], ["BBCA", "PT Bank Central Asia Tbk", "2026-11-05", "1m"]);
    assert.equal(d.bars.length, 20);
    assert.equal(d.bars[19].time, "2026-11-05T16:14:00+07:00");
    assert.equal(d.summary.bars, 333);
    assert.equal(d.summary.vwap, 5089.98);
    assert.equal(d.quote.previous_close, 4930);
    assert.deepEqual(out.warnings, ["bars truncated to the last 20; the summary covers the whole session"]);
    // The second call is served from the D1 cache.
    const again = await intraday(env, "BBCA");
    assert.equal(again.cached, true);
    assert.equal(again.data.bars.length, 333);
    assert.equal(calls.length, 1);
  });
});

test("/api/intraday just after the open: the 1-day window is empty, the 5-day one gives the last session", async () => {
  const env = memoryDb();
  const q = y.parseV7Quote(JSON.parse(fixture("quote_v7_bbca.json")).quoteResponse.result[0]);
  // A board refreshed just now: its own age (boardFreshness) is not under test here.
  env.kv.set("board", JSON.stringify({ ts: Math.floor(Date.now() / 1000), quotes: [q] }));
  await withFetch(async (u) => {
    if (String(u).includes("BBCA.JK?range=1d&interval=1m")) return new Response(fixture("chart_bbca_1d_1m_open_empty.json"), { status: 200 });
    if (String(u).includes("BBCA.JK?range=5d&interval=1m")) return new Response(fixture("chart_bbca_5d_1m_open.json"), { status: 200 });
    throw new Error("unexpected fetch " + u);
  }, async () => {
    const out = await intraday(env, "BBCA");
    assert.deepEqual(out.warnings, ["no 1m bars in the source's 1-day window (empty for some minutes after the open, or no trade today); showing the last session with bars, 2026-11-05"]);
    assert.equal(out.data.session, "2026-11-05");
    assert.equal(out.data.bars.length, 333, "the 16:14:43 snapshot row is not a bar");
    assert.equal(out.data.summary.vwap, 5089.98);
  });
});

test("/api/intraday: a quote that cannot be had is a warning, not an error", async () => {
  const env = memoryDb();
  await withFetch(async (u) => {
    if (String(u).includes("^JKSE?range=1d&interval=1m")) return new Response(JKSE, { status: 200 });
    throw new Error("unexpected fetch " + u);
  }, async () => {
    const out = await intraday(env, "^jkse");
    assert.equal(out.data.quote, null);
    assert.equal(out.data.summary.vwap, null);
    assert.match(out.warnings[0], /^no quote for \^JKSE: no market snapshot yet/);
  });
});
