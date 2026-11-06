// node --test web/test/*.mjs (no network). The Worker side of bandarmology:
// /api/bandar, the nightly scan (src/bandar-scan.js) and /api/bandar/screen,
// over an in-memory D1 and the recorded IPOT and Yahoo responses.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { bandar, bandarScreen, bandarBroker } from "../src/api.js";
import { targetSession, mostTraded, bandarScanStep, scanStatus } from "../src/bandar-scan.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const page = (f) => readFileSync(join(root, "tests", "fixtures", "ipot", f), "utf8");
const CHART = readFileSync(join(root, "tests", "fixtures", "yahoo", "chart_bbca_1mo_1d.json"), "utf8");
const quote = (symbol, price, volume) => ({ symbol, price, volume, previous_close: 6075, market_time: "2026-10-16T16:14:58+07:00" });

/** In-memory D1 for the statements the bandar code runs. */
function memoryDb() {
  const kv = new Map(), cache = new Map(), scan = new Map();
  const run = (sql, a) => {
    if (sql.startsWith("SELECT v, updated_at FROM kv")) return kv.has(a[0]) ? { v: kv.get(a[0]), updated_at: 0 } : null;
    if (sql.startsWith("SELECT v FROM kv")) return kv.has(a[0]) ? { v: kv.get(a[0]) } : null;
    if (sql.startsWith("INSERT OR REPLACE INTO kv")) return kv.set(a[0], a[1]);
    if (sql.startsWith("SELECT v, fetched_at, expires_at FROM cache")) return cache.get(a[0]) || null;
    if (sql.startsWith("INSERT OR REPLACE INTO cache")) return cache.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] });
    if (sql.startsWith("INSERT OR REPLACE INTO bandar_scan")) return scan.set([a[0], a[1], a[2]].join("|"), a[3]);
    if (sql.startsWith("SELECT symbol, v FROM bandar_scan")) {
      return [...scan].filter(([k]) => k.startsWith([a[0], a[1], ""].join("|"))).map(([k, v]) => ({ symbol: k.split("|")[2], v }));
    }
    if (sql.startsWith("DELETE FROM bandar_scan")) return null;
    throw new Error("memoryDb: unexpected SQL " + sql);
  };
  const stmt = (sql, a = []) => ({
    bind: (...b) => stmt(sql, b),
    first: async () => run(sql, a),
    run: async () => (run(sql, a), {}),
    all: async () => ({ results: run(sql, a) }),
    exec: () => run(sql, a),
  });
  return { DB: { prepare: (sql) => stmt(sql), batch: async (list) => list.map((s) => s.exec()) }, kv, scan };
}

/**
 * IPOT and Yahoo from the recorded pages: BBCA, BRPT, TLKM; ZZZZ is unknown.
 * A page recorded for the exact range answers it; any other range gets the
 * stock's 10-12..10-16 page (it stands in for the 1m and 3m periods).
 */
function recordedFetch(calls = []) {
  return async (u) => {
    const url = String(u);
    calls.push(url);
    if (url.includes("finance/chart")) return new Response(CHART, { status: 200 });
    const code = url.match(/code=([A-Z]+)/)[1].toLowerCase();
    const d = url.match(/start=(\d\d)\/(\d\d)\/(\d{4})&end=(\d\d)\/(\d\d)\/(\d{4})/);
    const [start, end] = [d[3] + d[1] + d[2], d[6] + d[4] + d[5]];
    if (code === "zzzz") return new Response(page("brokersummary_zzzz_invalid.html"), { status: 200 });
    const side = url.includes("fd=F") ? "_foreign" : url.includes("fd=D") ? "_domestic" : "";
    const exact = start === end ? `brokersummary_${code}_${end}${side}.html` : `brokersummary_${code}_${start}_${end}${side}.html`;
    let body;
    try { body = page(exact); } catch { body = page(`brokersummary_${code}_20261012_20261016.html`); }
    return new Response(body, { status: 200 });
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

test("bandar broker reads one broker across the scan like the CLI", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  await withFetch(recordedFetch(), async () => {
    await assert.rejects(bandarBroker(env, "YP"), (e) => e.code === "not_found");
    await bandarScanStep(env, { now: new Date("2026-10-17T03:00:00Z") });
    const out = await bandarBroker(env, " yp ", "1w");
    assert.equal(out.schema, "idx.bandar.broker.v1");
    assert.equal(out.provider, "ipot");
    const a = out.data;
    assert.deepEqual([a.code, a.name, a.from, a.to], ["YP", "MIRAE ASSET SEKURITAS INDONESIA", "2026-10-12", "2026-10-16"]);
    assert.deepEqual([a.scanned, a.checked, a.failed], [4, 3, 1]);
    // Same as the Rust test: a top-ten buyer of BBCA and TLKM, a top-ten seller of BRPT and TLKM.
    assert.deepEqual(a.bought.map((r) => r.symbol).sort(), ["BBCA", "TLKM"]);
    assert.deepEqual(a.sold.map((r) => r.symbol).sort(), ["BRPT", "TLKM"]);
    const bbca = a.bought.find((r) => r.symbol === "BBCA");
    assert.deepEqual([bbca.rank, bbca.opposite_value, bbca.net_value], [9, null, null]);
    const tb = a.bought.find((r) => r.symbol === "TLKM"), ts = a.sold.find((r) => r.symbol === "TLKM");
    assert.equal(tb.opposite_value, ts.value);
    assert.equal(tb.net_value, ts.net_value);
    assert.equal(tb.net_value, tb.value - ts.value);
    assert.ok(a.bought[0].value >= a.bought[1].value);
    assert.equal(a.bought_value, a.bought.reduce((t, r) => t + r.value, 0));
    const one = await bandarBroker(env, "YP", "1w", "1");
    assert.deepEqual([one.data.bought.length, one.data.sold.length], [1, 1]);
    assert.equal(one.data.bought_value, a.bought_value, "sums cover every row");
    await assert.rejects(bandarBroker(env, "MIRAE"), (e) => e.code === "usage");
    await assert.rejects(bandarBroker(env, "YP", "2w"), (e) => e.code === "usage");
  });
});

test("the scan targets the last finished session", () => {
  assert.equal(targetSession(new Date("2026-10-16T03:00:00Z")), "2026-10-15", "Friday 10:00 WIB: Thursday");
  assert.equal(targetSession(new Date("2026-10-16T09:50:00Z")), "2026-10-16", "Friday 16:50 WIB: Friday");
  assert.equal(targetSession(new Date("2026-10-17T03:00:00Z")), "2026-10-16", "Saturday: Friday");
  assert.equal(targetSession(new Date("2026-10-19T01:00:00Z")), "2026-10-16", "Monday 08:00 WIB: Friday");
  assert.deepEqual(mostTraded([quote("A", 10, 5), quote("B", 100, 5), quote("^JKSE", 7000, 9e9), quote("C", 1, 0)], 5), ["B", "A"]);
});

test("the nightly scan fills D1 and the screen reads it like the CLI", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  await withFetch(recordedFetch(), async () => {
    await assert.rejects(bandarScreen(env, "big-distribution"), (e) => e.code === "not_found");
    const now = new Date("2026-10-17T03:00:00Z");
    assert.deepEqual(await bandarScanStep(env, { now }), { session: "2026-10-16", scanned: 4, of: 4, done: true });
    assert.deepEqual(await bandarScanStep(env, { now }), { idle: true, session: "2026-10-16" });
    const st = await scanStatus(env);
    assert.equal(st.complete, "2026-10-16");
    assert.deepEqual(st.failed, ["ZZZZ"]);
    assert.equal(env.scan.size, 12, "three stocks x four periods");

    const out = await bandarScreen(env, "big-distribution", "turnover>1B", "1w");
    assert.equal(out.schema, "idx.bandar.screen.v1");
    assert.equal(out.provider, "ipot");
    const s = out.data;
    assert.deepEqual([s.from, s.to, s.preset, s.descending], ["2026-10-12", "2026-10-16", "big-distribution", false]);
    assert.deepEqual(s.filters, ["acc<=-15", "turnover>1B"]);
    assert.deepEqual([s.scanned, s.checked], [4, 3]);
    assert.deepEqual(s.rows.map((r) => r.symbol), ["BBCA", "TLKM"], "same order as the Rust screen test");
    assert.equal(s.rows[0].phase, "markdown");
    // The week pages count the 16th twice (BBCA -4.1 T, TLKM -319.8 B); the
    // scan rebuilds the nets from single sessions like the CLI.
    assert.deepEqual(s.rows.map((r) => r.foreign_net_value), [-1_823_100_000_000, -204_700_000_000]);
    assert.ok(Math.abs(s.rows[0].price_change_pct - (5175 / 6050 - 1) * 100) < 1e-9);
    assert.ok(out.warnings.includes("1 of 4 scanned stocks had no broker summary"), out.warnings);

    const all = await bandarScreen(env, null, "acc>-100", "1w", "10", "fnet", "0", "1");
    assert.equal(all.data.rows.length, 1);
    assert.equal(all.data.total_matches, 3);
    assert.equal(all.data.sort, "fnet");
    for (const bad of [["everything"], [null, "pe<10"], [null, null, "2w"], [null, null, null, "4"], [null, null, null, null, "phase"]]) {
      await assert.rejects(bandarScreen(env, ...bad), (e) => e.code === "usage", JSON.stringify(bad));
    }
  });
});

test("a holiday is noticed and skipped", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8)] }));
  await withFetch(async () => new Response(page("brokersummary_bbca_20261017_no_trades.html"), { status: 200 }), async () => {
    assert.deepEqual(await bandarScanStep(env, { now: new Date("2026-10-17T03:00:00Z") }), { holiday: "2026-10-16" });
    assert.equal((await scanStatus(env)).holiday, true);
    assert.equal(env.scan.size, 0);
  });
});

test("/api/bandar for one investor side reads against the value traded", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8)] }));
  await withFetch(recordedFetch(), async () => {
    const out = await bandar(env, "BBCA", null, "2026-10-16", null, null, null, "foreign", "all", "1");
    const r = out.data[0];
    assert.equal(r.investor, "foreign");
    // Like the Rust test: the foreign page's 1.7 T is their buying; percentages are of the 6.6 T traded.
    assert.equal(r.total_value, 6_600_000_000_000);
    const head = r.tiers.find((t) => t.top === 5);
    assert.ok(Math.abs(head.net_pct - (head.net_value / 6.6e12) * 100) < 1e-9);
    assert.ok(Math.abs(r.foreign_net_pct - (-2.3 / 6.6) * 100) < 1e-9);
    assert.equal(r.daily.days[0].total_value, 6_600_000_000_000);
    await assert.rejects(bandar(env, "BBCA", "1m", null, null, null, null, "foreign", "all", "1"), (e) => e.code === "usage" && /one investor side/.test(e.message));
  });
});

test("/api/bandar reads a range with its daily series", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8)] }));
  const calls = [];
  await withFetch(recordedFetch(calls), async () => {
    const out = await bandar(env, "BBCA", null, null, "2026-10-12", "2026-10-16", null, "all", "all", "1");
    assert.equal(out.schema, "idx.bandar.v1");
    const r = out.data[0];
    assert.equal(r.period, "custom");
    assert.equal(r.name, "BANK CENTRAL ASIA Tbk");
    assert.equal(r.signal, "big_distribution");
    assert.equal(r.phase, "markdown");
    assert.equal(r.buyers[0].name, "CLSA SEKURITAS INDONESIA");
    assert.equal(r.reasons[0], "Top 5 buyers bought 4.23T, top 5 sellers sold 6.16T: net -1.93T, -19.5% of the 9.90T traded (big distribution, at -15% or less).");
    assert.equal(r.daily.days.length, 5);
    assert.ok(r.daily.streak < 0);
    assert.equal(calls.filter((u) => u.includes("indopremier")).length, 6, "the range and five sessions");
    for (const bad of [
      ["BBCA,BBRI,BMRI,BBNI,TLKM,ASII"],
      ["BBCA", "2w"],
      ["BBCA", null, null, null, null, "4"],
      ["BBCA,BBRI", null, null, null, null, null, "all", "all", "1"],
      ["BBCA", "3m", null, null, null, null, "all", "all", "1"],
      ["BBCA", null, "2026-10-16", "2026-10-01"],
      ["BBCA", null, null, null, null, null, "retail"],
    ]) await assert.rejects(bandar(env, ...bad), (e) => e.code === "usage", JSON.stringify(bad));
  });
});
