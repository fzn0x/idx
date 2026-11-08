// node --test web/test/*.mjs (no network). The Worker side of bandarmology:
// /api/bandar, the scan after each close (src/bandar-scan.js) inside the
// minute cron's budget (src/cron.js), /api/bandar/screen and
// /api/bandar/broker, over an in-memory D1 and the recorded IPOT and Yahoo
// responses.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { bandar, bandarScreen, bandarBroker } from "../src/api.js";
import { targetSession, mostTraded, ranking, bandarScanStep, scanStatus, scanNotes } from "../src/bandar-scan.js";
import { budget } from "../src/budget.js";
import worker from "../src/worker.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const page = (f) => readFileSync(join(root, "tests", "fixtures", "ipot", f), "utf8");
const CHART = readFileSync(join(root, "tests", "fixtures", "yahoo", "chart_bbca_1mo_1d.json"), "utf8");
const quote = (symbol, price, volume, market_time = "2026-10-16T16:14:58+07:00") => ({ symbol, price, volume, previous_close: 6075, market_time });

/**
 * In-memory D1 for the statements the bandar code runs, and those of the
 * board refresh and the register sync that share its cron minute.
 */
function memoryDb() {
  const kv = new Map(), kvAt = new Map(), cache = new Map(), scan = new Map();
  // The Yahoo session's statements name their key inline.
  const inlineKey = (sql) => sql.match(/k = '([^']+)'|VALUES \('([^']+)'/)?.slice(1).find(Boolean);
  const run = (sql, a) => {
    if (sql.startsWith("SELECT v, updated_at FROM kv")) { const k = inlineKey(sql) ?? a[0]; return kv.has(k) ? { v: kv.get(k), updated_at: kvAt.get(k) ?? 0 } : null; }
    if (sql.startsWith("SELECT v FROM kv")) return kv.has(a[0]) ? { v: kv.get(a[0]) } : null;
    if (sql.startsWith("INSERT OR REPLACE INTO kv")) {
      const k = inlineKey(sql);
      const [key, v, at] = k ? [k, a[0], a[1]] : a;
      kvAt.set(key, at);
      return kv.set(key, v);
    }
    if (sql.startsWith("INSERT OR REPLACE INTO ticks") || sql.startsWith("DELETE FROM ticks")) return null;
    if (sql.startsWith("SELECT as_of, sha256, parts_key FROM own_releases")) return [];
    if (sql.startsWith("SELECT v, fetched_at, expires_at FROM cache")) return cache.get(a[0]) || null;
    if (sql.startsWith("INSERT OR REPLACE INTO cache")) return cache.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] });
    if (sql.startsWith("INSERT OR REPLACE INTO bandar_scan")) return scan.set([a[0], a[1], a[2]].join("|"), a[3]);
    if (sql.startsWith("SELECT symbol, v FROM bandar_scan")) {
      return [...scan].filter(([k]) => k.startsWith([a[0], a[1], ""].join("|"))).map(([k, v]) => ({ symbol: k.split("|")[2], v }));
    }
    if (sql.startsWith("DELETE FROM bandar_scan")) {
      // The newest a[0] sessions stay, and the one shown (a[1]).
      const sessions = [...new Set([...scan.keys()].map((k) => k.split("|")[0]))].sort().reverse();
      const keep = new Set([...sessions.slice(0, a[0]), a[1]]);
      for (const k of [...scan.keys()]) if (!keep.has(k.split("|")[0])) scan.delete(k);
      return null;
    }
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

/** Scan steps at `now`, one per cron minute, until the session is done; the last step's result. */
async function scanAll(env, now, opts = {}) {
  for (let i = 0; i < 40; i++) {
    const r = await bandarScanStep(env, { now, ...opts });
    if (r.done || r.idle) return r;
  }
  throw new Error("the scan did not finish in 40 steps");
}

test("bandar broker reads one broker across the scan like the CLI", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  await withFetch(recordedFetch(), async () => {
    await assert.rejects(bandarBroker(env, "YP"), (e) => e.code === "not_found");
    await scanAll(env, new Date("2026-10-17T03:00:00Z"));
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
  // Listed holidays are never targets: 12-24 and 12-25 close, 12-26 and 12-27 are a weekend.
  assert.equal(targetSession(new Date("2026-12-24T10:00:00Z")), "2026-12-23", "Christmas Eve 17:00 WIB: the 23rd");
  assert.equal(targetSession(new Date("2026-12-28T01:00:00Z")), "2026-12-23", "Monday 28th 08:00 WIB: the 23rd");
  // 2028 is not on the bundled list: its weekdays are trading days, 2027-12-31 is still listed.
  assert.equal(targetSession(new Date("2028-01-03T01:00:00Z")), "2027-12-30", "Monday 2028-01-03 08:00 WIB: past the listed 2027-12-31");
  assert.equal(targetSession(new Date("2028-01-04T10:00:00Z")), "2028-01-04", "a weekday of an uncovered year");
  assert.deepEqual(mostTraded([quote("A", 10, 5), quote("B", 100, 5), quote("^JKSE", 7000, 9e9), quote("C", 1, 0)], 5), ["B", "A"]);
});

test("the ranking counts the stocks that traded on the session", () => {
  const idle = quote("TMPO", 1000, 5, "2026-10-14T15:59:00+07:00"); // last traded two sessions earlier
  assert.deepEqual(mostTraded([quote("A", 10, 5), quote("B", 100, 5), idle], 5, "2026-10-16"), ["B", "A"]);
  assert.deepEqual(ranking([quote("A", 10, 5), idle], "2026-10-16"), { symbols: ["A"], ranked_from: "2026-10-16" });
  // The board has moved on to the next session: the whole board, by its latest value.
  const next = quote("B", 100, 5, "2026-10-19T10:01:00+07:00");
  assert.deepEqual(ranking([quote("A", 10, 5), next, idle], "2026-10-16"), { symbols: ["TMPO", "B", "A"], ranked_from: "2026-10-19" });
  // A board without a quote of the session, refreshed before it traded.
  assert.deepEqual(ranking([idle], "2026-10-16"), { symbols: ["TMPO"], ranked_from: "2026-10-14" });
});

test("the scan fills D1 and the screen reads it like the CLI", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  await withFetch(recordedFetch(), async () => {
    await assert.rejects(bandarScreen(env, "big-distribution"), (e) => e.code === "not_found");
    const now = new Date("2026-10-17T03:00:00Z");
    // The probe finds the lists published; three stocks a minute, so ZZZZ takes a second step.
    const first = await bandarScanStep(env, { now });
    assert.deepEqual([first.session, first.probe, first.read, first.scanned, first.of, first.done], ["2026-10-16", "published", 3, 3, 4, false]);
    const second = await bandarScanStep(env, { now });
    assert.deepEqual([second.read, second.scanned, second.done], [1, 4, true]);
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

const unix = (iso) => Math.floor(Date.parse(iso) / 1000);

test("a page without trades waits for IPOT while the board shows trades that day", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: unix("2026-10-16T16:45:00+07:00"), quotes: [quote("BBCA", 5175, 9e8)] }));
  const calls = [];
  await withFetch(async (u) => { calls.push(String(u)); return new Response(page("brokersummary_bbca_20261017_no_trades.html"), { status: 200 }); }, async () => {
    // Saturday 10:00 WIB: Friday's page shows nothing, but the board's BBCA last traded on Friday.
    assert.equal((await bandarScanStep(env, { now: new Date("2026-10-17T03:00:00Z") })).probe, "unpublished");
    const st = await scanStatus(env);
    assert.deepEqual([st.phase, st.holiday, st.probes, st.next_probe_at], ["waiting", false, 1, "2026-10-17T03:30:00.000Z"]);
    assert.equal(env.scan.size, 0);
    // Before the next probe a step reads its state and nothing else.
    assert.equal((await bandarScanStep(env, { now: new Date("2026-10-17T03:20:00Z") })).idle, true);
    assert.equal(calls.length, 1);
    await bandarScanStep(env, { now: new Date("2026-10-17T03:30:00Z") });
    assert.deepEqual([calls.length, (await scanStatus(env)).probes], [2, 2]);
  });
});

test("in a year the calendar does not cover, a day without trades at IPOT or on the board is closed", async () => {
  const now = new Date("2028-01-03T10:00:00Z"); // Monday 17:00 WIB; 2028 is not on the bundled list
  const board = (market_time) => JSON.stringify({ ts: unix("2028-01-03T16:45:00+07:00"), quotes: [quote("BBCA", 5175, 9e8, market_time)] });
  await withFetch(async () => new Response(page("brokersummary_bbca_20261017_no_trades.html"), { status: 200 }), async () => {
    // The board, refreshed after that session, last traded on 2027-12-30.
    const env = memoryDb();
    env.kv.set("board", board("2027-12-30T16:14:58+07:00"));
    const r = await bandarScanStep(env, { now });
    assert.deepEqual([r.session, r.probe, r.phase], ["2028-01-03", "closed", "closed"]);
    const st = await scanStatus(env);
    assert.deepEqual([st.phase, st.holiday, st.ranked_from], ["closed", true, "2027-12-30"]);
    assert.deepEqual(await bandarScanStep(env, { now }), { idle: true, session: "2028-01-03" });
    // The screen says why 2028-01-03 is not shown, that it does not count as a
    // session behind, and that the calendar does not cover the year.
    const notes = scanNotes(JSON.parse(env.kv.get("bandar_scan")), "2027-12-30", 10, now);
    assert.deepEqual(notes.warnings, [
      "2028-01-03 had no trades at IPOT nor on the quote board, so it was not scanned",
      "the bundled holiday list does not cover 2028; the scan tells that year's closures from IPOT and the quote board",
    ]);
    // The same page while the board shows trades that day: IPOT has not published.
    const traded = memoryDb();
    traded.kv.set("board", board("2028-01-03T16:14:58+07:00"));
    assert.equal((await bandarScanStep(traded, { now })).probe, "unpublished");
    const t = await scanStatus(traded);
    assert.deepEqual([t.phase, t.holiday], ["waiting", false]);
  });
});

test("a session IPOT has not published waits, is probed again every 10 minutes, then scanned", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: [quote("BBCA", 5100, 2e8, "2026-11-06T16:14:58+07:00")] }));
  let published = false;
  const calls = [];
  const recorded = recordedFetch(calls);
  // Until `published`, BBCA's 11-06 page is the one recorded during that session: totals, no lists.
  const fetchFn = async (u, init) => {
    if (!published && String(u).includes("code=BBCA&start=11/06/2026&end=11/06/2026")) { calls.push(String(u)); return new Response(page("brokersummary_bbca_20261106_insession.html")); }
    return recorded(u, init);
  };
  const at = (hm) => new Date(`2026-11-06T${hm}:00+07:00`);
  await withFetch(fetchFn, async () => {
    assert.equal((await bandarScanStep(env, { now: at("16:45") })).probe, "unpublished");
    let st = await scanStatus(env);
    assert.deepEqual([st.phase, st.holiday, st.next_probe_at], ["waiting", false, at("16:55").toISOString()]);
    assert.equal((await bandarScanStep(env, { now: at("16:50") })).idle, true);
    assert.equal(calls.length, 1, "no request before the next probe");
    await assert.rejects(bandarScreen(env, null), (e) => e.code === "not_found");
    assert.equal((await bandarScanStep(env, { now: at("16:55") })).probe, "unpublished");
    // From 21:00 WIB every 30 minutes.
    await bandarScanStep(env, { now: at("21:10") });
    assert.equal((await scanStatus(env)).next_probe_at, at("21:40").toISOString());
    published = true;
    const r = await bandarScanStep(env, { now: at("21:40") });
    assert.deepEqual([r.probe, r.read, r.done], ["published", 1, true]);
    st = await scanStatus(env);
    assert.deepEqual([st.complete, st.probes, st.published_at], ["2026-11-06", 4, at("21:40").toISOString()]);
    assert.equal(env.scan.size, 4);
  });
});

test("a stock IPOT has not published is read again, not stored without its lists", async () => {
  const env = memoryDb();
  const traded = "2026-11-06T16:14:58+07:00";
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: [quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded)] }));
  let published = false;
  const calls = [];
  const recorded = recordedFetch(calls);
  // BBCA's 11-06 lists are out; TLKM's 11-06 page shows totals without lists until `published`.
  const fetchFn = async (u, init) => {
    if (!published && String(u).includes("code=TLKM&start=11/06/2026&end=11/06/2026")) { calls.push(String(u)); return new Response(page("brokersummary_bbca_20261106_insession.html")); }
    return recorded(u, init);
  };
  const at = (hm) => new Date(`2026-11-06T${hm}:00+07:00`);
  const tlkmRows = () => [...env.scan.keys()].filter((k) => k.endsWith("|TLKM")).length;
  await withFetch(fetchFn, async () => {
    const r = await bandarScanStep(env, { now: at("17:00") });
    assert.deepEqual([r.probe, r.read], ["published", 2]);
    assert.equal(tlkmRows(), 0, "nothing stored for TLKM");
    assert.deepEqual(calls.filter((u) => u.includes("TLKM")), [calls.find((u) => u.includes("code=TLKM"))], "one request: its session's page");
    let st = await scanStatus(env);
    assert.deepEqual([st.phase, st.complete, st.failed, st.pending.map((p) => p.symbol)], ["running", "2026-11-06", [], ["TLKM"]]);
    const shown = await bandarScreen(env, null, null, "1d");
    assert.ok(shown.warnings.includes("IPOT has not published 2026-11-06 for 1 of the 2 stocks yet; the scan reads them again every 10 minutes"), shown.warnings);
    // Not before ten minutes; then once more, still without lists.
    calls.length = 0;
    assert.equal((await bandarScanStep(env, { now: at("17:05") })).idle, true);
    assert.equal(calls.length, 0);
    await bandarScanStep(env, { now: at("17:10") });
    assert.deepEqual([calls.length, tlkmRows()], [1, 0]);
    // IPOT publishes it: the next check reads it whole and the session is done.
    published = true;
    const done = await bandarScanStep(env, { now: at("17:20") });
    assert.equal(done.done, true);
    st = await scanStatus(env);
    assert.deepEqual([st.phase, st.pending, st.failed], ["done", [], []]);
    assert.equal(tlkmRows(), 4);
  });
});

test("a stock that fails is tried again 10, then 30 minutes after its pass, then counts as failed", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8)] }));
  const calls = [];
  const recorded = recordedFetch(calls);
  const tlkm = [];
  // IPOT is busy for TLKM (503 is not retried within a request).
  const fetchFn = async (u, init) => (String(u).includes("code=TLKM") ? (tlkm.push(String(u)), new Response("busy", { status: 503 })) : recorded(u, init));
  const at = (hm) => new Date(`2026-10-17T${hm}:00+07:00`);
  await withFetch(fetchFn, async () => {
    const r = await bandarScanStep(env, { now: at("10:00") });
    assert.deepEqual([r.read, r.errors, r.done], [2, { http_503: 1 }, false]);
    let st = await scanStatus(env);
    assert.deepEqual([st.retry, st.retry_due, st.failed, st.complete], [["TLKM"], at("10:10").toISOString(), [], null]);
    assert.equal((await bandarScanStep(env, { now: at("10:05") })).idle, true);
    assert.equal(tlkm.length, 1, "no retry within the pass's minute or before it is due");
    await bandarScanStep(env, { now: at("10:10") });
    st = await scanStatus(env);
    assert.deepEqual([tlkm.length, st.pass, st.retry_due], [2, 1, at("10:40").toISOString()]);
    assert.equal((await bandarScanStep(env, { now: at("10:39") })).idle, true);
    const last = await bandarScanStep(env, { now: at("10:40") });
    assert.equal(last.done, true);
    st = await scanStatus(env);
    assert.deepEqual([tlkm.length, st.failed, st.errors, st.complete], [3, ["TLKM"], { http_503: 1 }, "2026-10-16"]);
    assert.deepEqual([st.last_errors[0].symbol, st.last_errors[0].kind, st.last_errors[0].error], ["TLKM", "http_503", "HTTP 503"]);
  });
});

test("the state production held on 2026-11-08 heals: 2026-11-06 is probed again and scanned", async () => {
  const env = memoryDb();
  // Written at 16:45:39 WIB on 2026-11-06, when IPOT had not published that session's lists.
  env.kv.set("bandar_scan", '{"session":"2026-11-06","holiday":true,"done":true,"complete":"2026-10-30","symbols":[],"next":0,"retry":[],"failed":[],"checked":0}');
  const traded = "2026-11-06T16:14:58+07:00";
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-08T07:00:37+07:00"), quotes: [quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded), quote("BRPT", 1210, 1e8, traded), quote("ZZZZ", 50, 1e6, traded)] }));
  const calls = [];
  await withFetch(recordedFetch(calls), async () => {
    const now = new Date("2026-11-08T00:05:00Z"); // Sunday 07:05 WIB
    const first = await bandarScanStep(env, { now });
    assert.deepEqual([first.session, first.probe, first.read], ["2026-11-06", "published", 3]);
    assert.ok(calls[0].includes("code=BBCA&start=11/06/2026&end=11/06/2026"), "the probe reads 11-06 again");
    let st = await scanStatus(env);
    assert.deepEqual([st.phase, st.holiday, st.complete], ["running", false, "2026-10-30"]);
    const meanwhile = await bandarScreen(env, null, null, "1d", null, null, null, null, now);
    assert.equal(meanwhile.data.to, "2026-10-30");
    assert.deepEqual(meanwhile.warnings, ["the scan of 2026-11-06 is running (3 of 4); showing 2026-10-30", "2026-10-30 is 5 sessions behind 2026-11-06"]);
    assert.equal(meanwhile.fetched_at, "2026-10-30T09:45:00Z", "10-30 kept no record: 16:45 WIB that day, never the time of the request");
    assert.equal((await bandarScanStep(env, { now })).done, true);
    st = await scanStatus(env);
    assert.deepEqual([st.complete, st.failed, st.last.session, st.last.promoted], ["2026-11-06", ["ZZZZ"], "2026-11-06", true]);
    const out = await bandarScreen(env, null, null, "1d", null, null, null, null, now);
    assert.deepEqual([out.data.to, out.data.scanned, out.data.checked], ["2026-11-06", 4, 3]);
    assert.deepEqual(out.warnings, ["1 of 4 scanned stocks had no broker summary"]);
    assert.equal(out.fetched_at, "2026-11-08T00:05:00Z");
  });
});

test("a finished session keeps its numbers while the next one waits, and the warnings say why it is shown", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  const recorded = recordedFetch();
  // BBCA's pages of 10-19 and 10-20 show totals without lists: IPOT has not published those sessions.
  const fetchFn = async (u, init) => (/code=BBCA&start=10\/(19|20)\/2026&end=10\/(19|20)\/2026/.test(String(u)) ? new Response(page("brokersummary_bbca_20261106_insession.html")) : recorded(u, init));
  const screen = (now) => bandarScreen(env, null, null, "1w", null, null, null, null, now);
  const waiting = (d) => `the scan of ${d} is waiting for IPOT to publish that session's broker lists (checked 1 time since 16:46 WIB); showing 2026-10-16`;
  await withFetch(fetchFn, async () => {
    await scanAll(env, new Date("2026-10-17T03:00:00Z")); // Saturday 10:00 WIB
    // Monday 16:46 WIB: the scan of 10-19 starts and waits for IPOT.
    assert.equal((await bandarScanStep(env, { now: new Date("2026-10-19T09:46:00Z") })).probe, "unpublished");
    let out = await screen(new Date("2026-10-19T09:50:00Z"));
    assert.equal(out.data.to, "2026-10-16");
    assert.deepEqual(out.warnings, [waiting("2026-10-19"), "1 of 4 scanned stocks had no broker summary"]);
    assert.equal(out.fetched_at, "2026-10-17T03:00:00Z", "when the scan of 10-16 ended, not the time of the request");
    assert.deepEqual([out.data.scanned, out.data.checked], [4, 3]);
    // Tuesday 16:30 WIB: 10-20 has closed, its scan starts at 16:45.
    out = await screen(new Date("2026-10-20T09:30:00Z"));
    assert.deepEqual(out.warnings, ["the scan of 2026-10-20 starts at 16:45 WIB; showing 2026-10-16", waiting("2026-10-19"), "2026-10-16 is 2 sessions behind 2026-10-20", "1 of 4 scanned stocks had no broker summary"]);
    // Tuesday 16:46: 10-19 is filed as not published and 10-20 waits in turn.
    await bandarScanStep(env, { now: new Date("2026-10-20T09:46:00Z") });
    out = await screen(new Date("2026-10-20T09:50:00Z"));
    assert.deepEqual(out.warnings, [
      waiting("2026-10-20"),
      "IPOT had not published the broker lists of 2026-10-19 by 2026-10-19 16:46 WIB; showing 2026-10-16",
      "2026-10-16 is 2 sessions behind 2026-10-20",
      "1 of 4 scanned stocks had no broker summary",
    ]);
    const st = await scanStatus(env);
    assert.deepEqual([st.complete, st.last.session, st.last.scanned, st.last.reason, st.last.probes], ["2026-10-16", "2026-10-19", false, "unpublished", 1]);
    const broker = await bandarBroker(env, "YP", "1w", null, new Date("2026-10-20T09:50:00Z"));
    assert.deepEqual([broker.data.to, broker.data.scanned, broker.data.failed, broker.fetched_at], ["2026-10-16", 4, 1, "2026-10-17T03:00:00Z"]);
    assert.ok(broker.warnings.includes("2026-10-16 is 2 sessions behind 2026-10-20"), broker.warnings);
  });
});

test("a scan that read too few of its stocks is filed, and the session before it stays shown", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  const recorded = recordedFetch();
  let busy = false;
  const fetchFn = async (u, init) => (busy && /code=(TLKM|BRPT)/.test(String(u)) ? new Response("busy", { status: 503 }) : recorded(u, init));
  await withFetch(fetchFn, async () => {
    await scanAll(env, new Date("2026-10-17T03:00:00Z"));
    // On Monday IPOT fails TLKM and BRPT through the pass and both retries.
    busy = true;
    const at = (hm) => new Date(`2026-10-19T${hm}:00+07:00`);
    for (const hm of ["16:46", "16:47", "16:57", "17:27"]) await bandarScanStep(env, { now: at(hm) });
    const st = await scanStatus(env);
    assert.deepEqual([st.session, st.phase, st.complete, st.last.stored, st.last.of, st.last.promoted], ["2026-10-19", "done", "2026-10-16", 1, 4, false]);
    const out = await bandarScreen(env, null, null, "1w", null, null, null, null, at("17:30"));
    assert.equal(out.data.to, "2026-10-16");
    assert.deepEqual(out.warnings, ["the scan of 2026-10-19 read only 1 of 4 stocks (3 failed, 0 not published by IPOT); showing 2026-10-16", "1 of 4 scanned stocks had no broker summary"]);
    // Both sessions' rows stay: the shown one is never pruned.
    assert.deepEqual([...new Set([...env.scan.keys()].map((k) => k.split("|")[0]))].sort(), ["2026-10-16", "2026-10-19"]);
  });
});

test("a stock IPOT still had not published at the next session's 16:45 counts as unpublished, not failed", async () => {
  const env = memoryDb();
  const traded = "2026-11-06T16:14:58+07:00";
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: [quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded)] }));
  const recorded = recordedFetch();
  const fetchFn = async (u, init) => (String(u).includes("code=TLKM&start=11/06/2026&end=11/06/2026") ? new Response(page("brokersummary_bbca_20261106_insession.html")) : recorded(u, init));
  await withFetch(fetchFn, async () => {
    await bandarScanStep(env, { now: new Date("2026-11-06T10:00:00Z") });
    // Monday 16:45 WIB, the probe alone: 11-06 is filed with TLKM unpublished.
    const monday = new Date("2026-11-09T09:45:00Z");
    await bandarScanStep(env, { now: monday, stocks: false });
    const st = await scanStatus(env);
    assert.deepEqual([st.session, st.complete, st.last.session, st.last.unpublished, st.last.failed, st.last.partial], ["2026-11-09", "2026-11-06", "2026-11-06", ["TLKM"], [], true]);
    const out = await bandarScreen(env, null, null, "1d", null, null, null, null, new Date("2026-11-09T09:50:00Z"));
    assert.deepEqual(out.warnings, ["the scan of 2026-11-09 is running (0 of 2); showing 2026-11-06", "IPOT had not published 2026-11-06 for 1 of the 2 scanned stocks when its scan ended"]);
    assert.equal(out.data.scanned, 2);
  });
});

/** IPOT answering four 404s before every page (five requests a page); the rest as recorded. */
function stormFetch(calls) {
  const recorded = recordedFetch();
  const tries = new Map();
  return async (u, init) => {
    const url = String(u);
    calls.push(url);
    if (url.includes("indopremier")) {
      tries.set(url, (tries.get(url) || 0) + 1);
      if (tries.get(url) % 5) return new Response("<html>404 Not Found</html>", { status: 404 });
    }
    return recorded(u, init);
  };
}

test("a 404 storm stays inside the budget and the stock it cut short stays next", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8)] }));
  const now = new Date("2026-10-17T03:00:00Z");
  const calls = [];
  await withFetch(stormFetch(calls), async () => {
    // The probe takes 5 requests and BBCA would take 41 (a chart and eight pages): the budget stops it at 44.
    await bandarScanStep(env, { now, budget: budget(44), pauseMs: 0 });
    assert.equal(calls.length, 44);
    assert.equal(env.scan.size, 0, "nothing stored from a stock cut short");
    const st = await scanStatus(env);
    assert.deepEqual([st.scanned, st.failed], [0, []], "BBCA is neither scanned nor failed");
  });
  // The next minute, with a budget of its own, reads it whole; then too little is left to start TLKM.
  const next = [];
  await withFetch(stormFetch(next), async () => {
    await bandarScanStep(env, { now, budget: budget(44), pauseMs: 0 });
    assert.equal(next.length, 41);
    const st = await scanStatus(env);
    assert.deepEqual([st.scanned, st.failed], [1, []]);
    assert.equal(env.scan.size, 4, "BBCA's four periods");
  });
});

/** Yahoo's session and quotes, the KSEI mirror's empty index, then IPOT and the charts as recorded. */
function cronFetch(urls) {
  const recorded = recordedFetch();
  const time = Math.floor(Date.parse("2026-10-16T16:14:58+07:00") / 1000);
  const board = { "BBCA.JK": [5175, 9e8], "TLKM.JK": [2690, 3e8], "BRPT.JK": [1210, 2e8] };
  return async (u, init) => {
    const url = String(u);
    urls.push(url);
    if (url.startsWith("https://fc.yahoo.com")) return new Response("", { status: 404, headers: { "set-cookie": "A3=d=test; Path=/" } });
    if (url.includes("/getcrumb")) return new Response("crumb");
    if (url.includes("/v7/finance/quote")) {
      const asked = new URL(url).searchParams.get("symbols").split(",");
      const result = asked.filter((s) => board[s] || s === "^JKSE").map((s) => (s === "^JKSE"
        ? { symbol: s, regularMarketPrice: 7000, regularMarketTime: time }
        : { symbol: s, regularMarketPrice: board[s][0], regularMarketVolume: board[s][1], regularMarketPreviousClose: 6075, regularMarketTime: time }));
      return new Response(JSON.stringify({ quoteResponse: { result } }));
    }
    if (url.endsWith("/index.json")) return new Response(JSON.stringify({ schema: "idx.ownership.mirror.v1", releases: [] }));
    return recorded(u, init);
  };
}

test("the minute cron refreshes the board and the register first, then scans, inside one budget", async () => {
  const env = memoryDb();
  const urls = [];
  const ctx = { waitUntil() { throw new Error("the cron leaves nothing to waitUntil"); } };
  const minute = (iso) => ({ scheduledTime: Date.parse(iso), cron: "* * * * *" });
  const { log, error } = console;
  const logs = [];
  console.log = console.error = (...a) => logs.push(a.join(" "));
  try {
    await withFetch(cronFetch(urls), async () => {
      // 16:45 WIB: the 15-minute board refresh after the close and the register's check, then the scan's probe alone.
      const first = await worker.scheduled(minute("2026-10-16T09:45:00Z"), env, ctx);
      const scanFrom = urls.findIndex((u) => u.includes("indopremier"));
      assert.equal(urls.filter((u) => u.includes("/v7/finance/quote")).length, 7, "the board in six batches and the index");
      assert.ok(urls.slice(0, scanFrom).some((u) => u.endsWith("/ownership-data/index.json")), "the register's check first");
      assert.equal(urls.length - scanFrom, 1, "the probe, and no stock in the refresh's minute");
      assert.ok(first.subrequests.fetch + first.subrequests.d1 <= 44, JSON.stringify(first.subrequests));
      assert.equal(first.subrequests.fetch, 12, "2 for the session, 7 quote calls, 2 for the mirror's redirect, the probe");
      // The step ran before scheduled() resolved: its state is there.
      assert.equal((await scanStatus(env)).session, "2026-10-16");
      // 16:46: no refresh, the register idle: three stocks.
      urls.length = 0;
      const second = await worker.scheduled(minute("2026-10-16T09:46:00Z"), env, ctx);
      assert.equal(urls.filter((u) => u.includes("indopremier")).length, 24, "eight pages a stock");
      assert.equal(urls.length, 27, "and a chart each");
      assert.ok(second.subrequests.fetch + second.subrequests.d1 <= 44, JSON.stringify(second.subrequests));
      assert.equal((await scanStatus(env)).complete, "2026-10-16");
    });
  } finally {
    console.log = log;
    console.error = error;
  }
  assert.ok(logs.some((l) => l.startsWith("bandar ") && l.includes('"subrequests"')), logs.join("\n"));
  assert.ok(!logs.some((l) => l.includes("failed")), logs.join("\n"));
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
