// node --test web/test/*.mjs (no network). The web twin of the CLI's data
// freshness: quotes behind the latest IDX session (same session table as
// src/holidays.rs), stale_reason on every stale envelope, warnings that keep
// the cause, and a board that turns stale when the minute refresh stops.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as hol from "../src/holidays.js";
import * as y from "../src/yahoo.js";
import { bandar, boardFreshness, brief, envelope, history, market, marketIsOpen, quote, returns, screen } from "../src/api.js";
import { redactUrls } from "../src/net.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", p), "utf8");
const rows = (p) => JSON.parse(fixture(p)).quoteResponse.result.map(y.parseV7Quote);
const wib = (s) => new Date(`${s}+07:00`);

test("the latest session skips weekends and holidays and waits for 16:15 WIB", () => {
  for (const [now, want, why] of [
    ["2026-11-06T06:52:00", "2026-11-05", "Friday before the open"],
    ["2026-11-06T10:00:00", "2026-11-05", "Friday during the session"],
    ["2026-11-06T16:14:59", "2026-11-05", "one second before the end"],
    ["2026-11-06T16:15:00", "2026-11-06", "post-closing over"],
    ["2026-11-07T12:00:00", "2026-11-06", "Saturday"],
    ["2026-11-08T23:59:59", "2026-11-06", "Sunday night"],
    ["2026-11-09T00:30:00", "2026-11-06", "Monday after midnight"],
    ["2026-11-09T16:15:00", "2026-11-09", "Monday after the close"],
    ["2026-08-17T17:00:00", "2026-08-14", "Independence Day"],
    ["2026-08-18T08:00:00", "2026-08-14", "the morning after it"],
    ["2026-03-25T08:00:00", "2026-03-17", "after Nyepi and Idul Fitri"],
    ["2026-12-24T17:00:00", "2026-12-23", "cuti bersama Natal"],
    ["2027-01-04T08:00:00", "2026-12-30", "12-31 and 01-01 closed"],
    ["2027-01-05T17:00:00", "2027-01-04", "Isra Mikraj"],
  ]) assert.equal(hol.latestSession(wib(now)), want, `${now}: ${why}`);
  assert.equal(hol.latestSession(new Date("2026-11-06T09:15:00Z")), "2026-11-06", "09:15Z is 16:15 WIB");
});

test("behind_session: true, false or null like Quote::behind", () => {
  const latest = "2026-11-06";
  const q = (symbol, market_time) => hol.behindSession({ symbol, market_time }, latest);
  assert.equal(q("BBCA", "2026-11-05T16:14:43+07:00"), true);
  assert.equal(q("BBCA", "2026-11-06T16:14:43+07:00"), false);
  assert.equal(q("BBCA", "2026-11-05T17:30:00Z"), false, "00:30 WIB on the 6th");
  assert.equal(q("WIKA", "2025-02-17T16:14:58+07:00"), true);
  assert.equal(q("BBCA", null), null, "no last trade time");
  assert.equal(q("^JKSE", "2026-11-05T16:00:04+07:00"), true);
  assert.equal(q("IDX30", "2026-11-06T16:00:04+07:00"), false);
  assert.equal(q("^GSPC", "2025-01-02T04:00:00+07:00"), null);
  assert.equal(q("^N225", "2025-01-02T13:45:00+07:00"), null);
  assert.equal(q("USDIDR=X", "2025-01-02T13:45:00+07:00"), null);
});

test("market hours close on IDX holidays", () => {
  assert.equal(marketIsOpen(new Date("2026-12-24T03:00:00Z")), false, "Thursday 10:00 WIB, cuti bersama Natal");
  assert.equal(marketIsOpen(new Date("2026-08-17T03:00:00Z")), false, "Independence Day");
  assert.equal(marketIsOpen(new Date("2026-12-23T03:00:00Z")), true, "Wednesday 10:00 WIB");
});

test("a stale envelope says why, a fresh one has no reason", () => {
  const stale = envelope("idx.quotes.v1", [], { cached: true, stale: true, warnings: ["w"] });
  assert.equal(stale.stale_reason, "fetch_failed");
  assert.deepEqual(Object.keys(stale), ["schema", "provider", "fetched_at", "cached", "stale", "stale_reason", "warnings", "data"]);
  const fresh = envelope("idx.quotes.v1", []);
  assert.equal(fresh.stale, false);
  assert.ok(!("stale_reason" in fresh));
});

test("warnings never carry a URL's query string", () => {
  assert.equal(
    redactUrls("error for url (https://query1.finance.yahoo.com/v7/finance/quote?symbols=BBCA.JK,^JKSE&crumb=abc): refused"),
    "error for url (https://query1.finance.yahoo.com/v7/finance/quote): refused",
  );
  assert.equal(redactUrls("timed out waiting for https://a.test/q#frag"), "timed out waiting for https://a.test/q");
  assert.equal(redactUrls("no URL here"), "no URL here");
});

// D1 as the Worker sees it: the board in `kv`, rows in `cache`, ticks empty.
function boardDb(board) {
  const kv = new Map([["board", { v: JSON.stringify(board), updated_at: board.ts }]]);
  return {
    prepare: (sql) => ({
      bind: (...a) => ({
        first: async () => (sql.includes("FROM kv") ? kv.get(a[0]) ?? null : null),
        all: async () => ({ results: [] }),
        run: async () => ({}),
      }),
    }),
  };
}

const BOARD_ROWS = [...rows("yahoo/quote_v7_wika_suspended_kaef_bbca.json"), ...rows("yahoo/quote_v7_bbca_gspc.json").filter((q) => q.symbol === "^GSPC")];

test("board quotes carry behind_session and the board turns stale when its refresh stops", async () => {
  // Friday 2026-11-06 after the close: the latest session is the 6th.
  const now = wib("2026-11-06T17:00:00");
  const ts = Math.floor(now.getTime() / 1000) - 5 * 60;
  const board = { ts, quotes: BOARD_ROWS, index: null, warnings: [] };
  const env = { DB: boardDb(board) };
  const got = await quote(env, "WIKA,BBCA,^GSPC", now);
  assert.deepEqual(got.data.map((q) => [q.symbol, q.behind_session]), [["WIKA", true], ["BBCA", true], ["^GSPC", null]]);
  assert.equal(Object.keys(got.data[0]).pop(), "behind_session", "last field, as in the CLI");
  assert.equal(board.quotes[0].behind_session, undefined, "board rows are not touched");
  assert.equal(got.stale, false);
  assert.ok(!("stale_reason" in got));

  // The same rows read before that close: the 5th is the latest session.
  const morning = await quote(env, "WIKA,BBCA", wib("2026-11-06T16:00:00"));
  assert.deepEqual(morning.data.map((q) => q.behind_session), [true, false]);

  // Two hours without a refresh.
  const old = { ...board, ts: ts - 2 * 3600 };
  const late = await quote({ DB: boardDb(old) }, "WIKA", now);
  assert.equal(late.stale, true);
  assert.equal(late.stale_reason, "fetch_failed");
  assert.match(late.warnings[0], /^the market snapshot is from 2026-11-06 14:55 WIB; its refresh has failed since$/);

  // The board holds the IDX universe only; its newest quote is the 5th's
  // close, so after the 6th's close it lags the session.
  const idxOnly = { ...old, quotes: BOARD_ROWS.filter((q) => q.symbol !== "^GSPC") };
  const m = await market({ DB: boardDb(idxOnly) }, now);
  assert.equal(m.data.as_of, "2026-11-05T16:14:43+07:00");
  assert.equal(m.data.behind_session, true);
  assert.equal(m.stale, true);
  assert.equal(m.stale_reason, "fetch_failed");
  const before = await market({ DB: boardDb({ ...idxOnly, ts: Math.floor(wib("2026-11-06T07:00:00").getTime() / 1000) }) }, wib("2026-11-06T07:05:00"));
  assert.equal(before.data.behind_session, false, "before the 6th's close the 5th is the latest session");
  assert.equal(before.stale, false);
});

test("board freshness follows the cron cadence", () => {
  const at = (s) => Math.floor(wib(s).getTime() / 1000);
  // Open for a while: one refresh a minute, stale past 10 minutes.
  assert.equal(boardFreshness(at("2026-11-06T10:00:00"), wib("2026-11-06T10:09:00")).stale, false);
  assert.equal(boardFreshness(at("2026-11-06T10:00:00"), wib("2026-11-06T10:11:00")).stale, true);
  // Just opened: the last refresh was the 08:30 one, still fine.
  assert.equal(boardFreshness(at("2026-11-06T08:30:00"), wib("2026-11-06T08:46:00")).stale, false);
  // Closed (weekend, holiday, night): every 15 minutes, stale past 35.
  assert.equal(boardFreshness(at("2026-11-07T10:00:00"), wib("2026-11-07T10:30:00")).stale, false);
  assert.equal(boardFreshness(at("2026-12-24T10:00:00"), wib("2026-12-24T10:36:00")).stale, true);
});

// The cache table as D1 would hold it: k -> { v, expires_at, fetched_at }.
function memoryDb() {
  const rows = new Map();
  return {
    rows,
    prepare: (sql) => ({ bind: (...a) => ({
      first: async () => (sql.startsWith("SELECT") ? rows.get(a[0]) ?? null : null),
      run: async () => { if (sql.startsWith("INSERT")) rows.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] }); return {}; },
    }) }),
  };
}

test("a failed fetch keeps its cause in history and in the returns built on it", async () => {
  const realFetch = globalThis.fetch;
  const chart = fixture("yahoo/chart_bbca_1mo_1d.json");
  const up = { value: true };
  globalThis.fetch = async (url) => {
    if (!up.value) throw new TypeError(`fetch failed for ${url}`);
    return new Response(chart, { status: 200 });
  };
  try {
    const env = { DB: memoryDb() };
    const live = await history(env, "BBCA", "5y", "1d");
    assert.equal(live.stale, false);
    assert.ok(!("stale_reason" in live));

    for (const row of env.DB.rows.values()) row.expires_at = 0;
    up.value = false;
    const h = await history(env, "BBCA", "5y", "1d");
    assert.equal(h.stale, true);
    assert.equal(h.stale_reason, "fetch_failed");
    assert.equal(h.warnings.length, 1);
    assert.match(h.warnings[0], /^live fetch failed \(fetch failed for https:\/\/query1\.finance\.yahoo\.com\/v8\/finance\/chart\/BBCA\.JK\); showing cached copy$/);

    const r = await returns(env, "BBCA", "none");
    assert.equal(r.stale, true);
    assert.equal(r.stale_reason, "fetch_failed");
    assert.deepEqual(r.warnings, h.warnings, "the cause survives into returns");
  } finally {
    globalThis.fetch = realFetch;
  }
});

test("a stale benchmark makes the returns stale, as in the CLI", async () => {
  const realFetch = globalThis.fetch;
  const chart = fixture("yahoo/chart_bbca_1mo_1d.json");
  const down = new Set();
  globalThis.fetch = async (url) => {
    if ([...down].some((s) => String(url).includes(s))) throw new TypeError(`fetch failed for ${url}`);
    return new Response(chart, { status: 200 });
  };
  try {
    const env = { DB: memoryDb() };
    const warm = await returns(env, "BBCA", "^JKSE");
    assert.equal(warm.stale, false);
    assert.ok(!("stale_reason" in warm));

    // Only the benchmark's copy is expired, and only its fetch fails.
    // History rows are keyed on the chart URL (one per request).
    const benchKey = [...env.DB.rows.keys()].find((k) => k.startsWith("history:") && k.includes("/^JKSE?"));
    env.DB.rows.get(benchKey).expires_at = 0;
    down.add("^JKSE");
    const r = await returns(env, "BBCA", "^JKSE");
    assert.equal(r.data[0].benchmark.returns.length, warm.data[0].benchmark.returns.length);
    assert.equal(r.stale, true);
    assert.equal(r.stale_reason, "fetch_failed");
    assert.equal(r.cached, true);
    assert.equal(r.warnings.length, 1);
    assert.match(r.warnings[0], /^live fetch failed \(fetch failed for https:\/\/query1\.finance\.yahoo\.com\/v8\/finance\/chart\/\^JKSE\); showing cached copy$/);

    // Without the benchmark nothing is stale.
    const plain = await returns(env, "BBCA", "none");
    assert.equal(plain.stale, false);
    assert.ok(!plain.warnings);
  } finally {
    globalThis.fetch = realFetch;
  }
});

// D1 with both tables: `kv` (board, Yahoo session) and `cache` rows.
function fullDb({ kv = {}, cache = {} }) {
  const kvRows = new Map(Object.entries(kv));
  const rows = new Map(Object.entries(cache));
  const query = (sql, a) => ({
    first: async () => {
      if (sql.includes("FROM kv")) return kvRows.get(a[0] ?? "yahoo:session") ?? null;
      if (sql.includes("FROM cache")) return rows.get(a[0]) ?? null;
      return null;
    },
    all: async () => ({ results: [] }),
    run: async () => { if (sql.includes("INTO cache")) rows.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] }); return {}; },
  });
  return { rows, prepare: (sql) => ({ ...query(sql, []), bind: (...a) => query(sql, a) }) };
}

test("a screen on stale fundamentals is stale and names the failure once", async () => {
  const realFetch = globalThis.fetch;
  const asked = [];
  globalThis.fetch = async (url) => { asked.push(String(url)); throw new TypeError(`fetch failed for ${url}`); };
  try {
    const now = new Date();
    const ts = Math.floor(now.getTime() / 1000);
    const body = JSON.parse(fixture("yahoo/quote_summary_fundamentals_bbca.json")).quoteSummary.result[0];
    const env = { DB: fullDb({
      kv: {
        board: { v: JSON.stringify({ ts: ts - 60, quotes: BOARD_ROWS, index: null, warnings: [] }), updated_at: ts - 60 },
        "yahoo:session": { v: JSON.stringify({ cookie: "A3=SECRETCOOKIE", crumb: "SECRETCRUMB" }), updated_at: ts },
      },
      // BBCA's fundamentals are an expired copy; WIKA and KAEF have none.
      cache: { [`summary:BBCA:${y.FUNDAMENTAL_MODULES}`]: { v: JSON.stringify(body), expires_at: 0, fetched_at: ts - 7 * 3600 } },
    }) };
    const s = await screen(env, "", "roe", 25, 20, true, null, false, now);
    assert.ok(asked.some((u) => u.includes("crumb=SECRETCRUMB")), "the session was used");
    assert.equal(s.data.fundamentals_checked, 1);
    assert.equal(s.data.rows.find((r) => r.symbol === "BBCA").fundamentals_checked, true);
    assert.equal(s.stale, true);
    assert.equal(s.stale_reason, "fetch_failed");
    assert.equal(s.cached, true);
    const summary = s.warnings.filter((w) => w.startsWith("fundamentals for"));
    assert.deepEqual(summary, [
      "fundamentals for 1 of 1 companies are stale cached copies: the live fetch failed (fetch failed for https://query2.finance.yahoo.com/v10/finance/quoteSummary/BBCA.JK)",
    ]);
    assert.ok(!s.warnings.some((w) => w.startsWith("live fetch failed")), "no per-company warning");
    assert.ok(!JSON.stringify(s).includes("SECRET"), "no crumb or cookie in the envelope");

    // Fresh fundamentals leave the board's freshness alone.
    env.DB.rows.get(`summary:BBCA:${y.FUNDAMENTAL_MODULES}`).expires_at = ts + 3600;
    const ok = await screen(env, "", "roe", 25, 20, true, null, false, now);
    assert.equal(ok.stale, false);
    assert.ok(!("stale_reason" in ok));
    assert.ok(!(ok.warnings || []).some((w) => w.startsWith("fundamentals for")));
  } finally {
    globalThis.fetch = realFetch;
  }
});

test("a brief is as old as its oldest section, as Sections::take makes it in the CLI", async () => {
  const realFetch = globalThis.fetch;
  const chart = fixture("yahoo/chart_bbca_1mo_1d.json");
  // Yahoo's chart answers (technical, returns, dividends); the feeds and the rest are down.
  globalThis.fetch = async (url) => {
    if (String(url).includes("/v8/finance/chart/")) return new Response(chart, { status: 200 });
    throw new TypeError(`fetch failed for ${url}`);
  };
  try {
    const ts = Math.floor(Date.now() / 1000);
    const week = ts - 7 * 86400;
    const body = JSON.parse(fixture("yahoo/quote_summary_fundamentals_bbca.json")).quoteSummary.result[0];
    const env = { DB: fullDb({
      kv: { board: { v: JSON.stringify({ ts: ts - 60, quotes: BOARD_ROWS, index: null, warnings: [] }), updated_at: ts - 60 } },
      // The fundamentals are a copy fetched a week ago that has not expired.
      cache: { [`summary:BBCA:${y.FUNDAMENTAL_MODULES}`]: { v: JSON.stringify(body), expires_at: ts + 3600, fetched_at: week } },
    }) };
    const iso = (secs) => new Date(secs * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");
    const first = await brief(env, "BBCA");
    assert.ok(first.data.fundamentals && first.data.technical, "both sections read");
    assert.equal(first.cached, false, "the candles were fetched for this brief");
    assert.equal(first.fetched_at, iso(week), "the week-old fundamentals, not the time of the answer");
    // Every section from the cache now: still the oldest one's time.
    const again = await brief(env, "BBCA");
    assert.equal(again.cached, true);
    assert.equal(again.fetched_at, iso(week));
  } finally {
    globalThis.fetch = realFetch;
  }
});

/** IPOT's recorded BBCA pages: the page recorded for a range, else the week to 10-16; `down.value` fails single sessions. */
function ipotFetch(down = { value: false }) {
  return async (u) => {
    const url = String(u);
    if (url.includes("/v8/finance/chart/")) return new Response(fixture("yahoo/chart_bbca_1mo_1d.json"), { status: 200 });
    const d = url.match(/start=(\d\d)\/(\d\d)\/(\d{4})&end=(\d\d)\/(\d\d)\/(\d{4})/);
    if (!d) throw new TypeError(`fetch failed for ${url}`);
    const [start, end] = [d[3] + d[1] + d[2], d[6] + d[4] + d[5]];
    if (start === end && down.value) throw new TypeError("connection reset");
    let body;
    try { body = fixture(`ipot/brokersummary_bbca_${start === end ? end : `${start}_${end}`}.html`); } catch { body = fixture("ipot/brokersummary_bbca_20261012_20261016.html"); }
    return new Response(body, { status: 200 });
  };
}

const boardKv = { board: { v: JSON.stringify({ ts: 0, quotes: [] }), updated_at: 0 } };

test("bandar's daily series counts toward its provenance, as each session does in service::bandar_one", async () => {
  const realFetch = globalThis.fetch;
  const down = { value: false };
  globalThis.fetch = ipotFetch(down);
  try {
    const env = { DB: fullDb({ kv: boardKv }) };
    const ask = () => bandar(env, "BBCA", null, null, "2026-10-12", "2026-10-16", null, "all", "all", "1");
    const iso = (secs) => new Date(secs * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");
    assert.equal((await ask()).data[0].daily.days.length, 5);
    const days = "bandar_days:BBCA:2026-10-12:2026-10-16:all:all";
    const old = Math.floor(Date.now() / 1000) - 3 * 86400;
    env.DB.rows.get(days).fetched_at = old;
    env.DB.rows.get("ipot:BBCA:2026-10-12:2026-10-16:all:all").expires_at = 0;
    const out = await ask();
    assert.equal(out.cached, false, "the range's page was fetched again");
    assert.equal(out.fetched_at, iso(old), "the series read three days before");
    // A series that cannot be read again is served stale, and says why.
    env.DB.rows.get(days).expires_at = 0;
    down.value = true;
    const late = await ask();
    assert.equal(late.stale, true);
    assert.equal(late.stale_reason, "fetch_failed");
    assert.ok(late.warnings.some((w) => /^live fetch failed \(.*\); showing cached copy$/.test(w)), JSON.stringify(late.warnings));
  } finally {
    globalThis.fetch = realFetch;
  }
});

test("bandar over a period counts its latest session's page, as service::bandar_one absorbs it", async (t) => {
  // Saturday 2026-10-17, 10:00 WIB: the latest session is Friday's.
  t.mock.timers.enable({ apis: ["Date"], now: new Date("2026-10-17T03:00:00Z") });
  const realFetch = globalThis.fetch;
  globalThis.fetch = ipotFetch();
  try {
    const env = { DB: fullDb({ kv: boardKv }) };
    // One board: the source gives no foreign net for it, so no session is read again to rebuild one.
    const ask = () => bandar(env, "BBCA", "1w", null, null, null, null, "all", "regular");
    const iso = (secs) => new Date(secs * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");
    const first = await ask();
    assert.deepEqual([first.data[0].from, first.data[0].to], ["2026-10-12", "2026-10-16"]);
    const early = Math.floor(Date.now() / 1000) - 2 * 3600;
    env.DB.rows.get("ipot:BBCA:2026-10-16:2026-10-16:all:regular").fetched_at = early;
    env.DB.rows.get("ipot:BBCA:2026-10-12:2026-10-16:all:regular").expires_at = 0;
    const out = await ask();
    assert.equal(out.cached, false, "the week's page was fetched again");
    assert.equal(out.fetched_at, iso(early), "the latest session's page, read two hours before");
  } finally {
    globalThis.fetch = realFetch;
  }
});
