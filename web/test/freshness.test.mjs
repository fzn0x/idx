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
import { boardFreshness, envelope, history, market, marketIsOpen, quote, returns } from "../src/api.js";
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
