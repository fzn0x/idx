// The nightly bandarmology scan behind the web terminal's `bandar screen`.
//
// After the close the minute cron reads the session's 100 most traded stocks
// (by value, from the quote board), PER_TICK per minute: for each, the IPOT
// broker summary over 1d, 1w, 1m and 3m ending on that session and the daily
// candles for the price move, one D1 row per stock and period in
// `bandar_scan`. /api/bandar/screen then answers from D1 alone with the same
// rules as the CLI (src/bandar.js). The foreign net of 1w, 1m and 3m is
// rebuilt from single sessions (the range page counts the latest session
// twice): the session before the scanned one and each period's rest, so a
// stock costs 9 requests (a chart, four period pages, one more session,
// three rests) and a tick PER_TICK x 9 and one D1 batch, inside a Worker
// invocation's 50 subrequests; a stock whose fetch failed is tried once more
// at the end of the pass. Holidays are noticed by a probe and skipped.

import * as ipot from "./ipot.js";
import * as y from "./yahoo.js";
import * as bd from "./bandar.js";
import * as hol from "./holidays.js";
import { budget, isBudgetError } from "./budget.js";
import BROKERS from "./brokers.json" with { type: "json" };
import UNIVERSE from "./universe.json" with { type: "json" };

export const SCAN_SIZE = 100;
/** Stocks a minute reads at most: 9 requests each, 27 of the invocation's 44. */
const PER_TICK = 3;
/**
 * A stock starts only with this many subrequests left: 9 typical, 11 when a
 * rebuild walks past sessions without trades.
 */
const STOCK_RESERVE = 12;
/** ...and only this soon into the invocation, so a minute ends before the next one starts. */
const TICK_MS = 40_000;
const KEEP_SESSIONS = 2;
const STATE = "bandar_scan";
/** Minutes after midnight WIB from which the scan looks at the day's session. */
const READY_MIN = 16 * 60 + 45;
const NAMES = new Map(UNIVERSE.map(([s, n]) => [s, n]));

/**
 * The session to scan: today from 16:45 WIB on a trading day, otherwise the
 * trading day before. Weekends and the bundled holidays (src/holidays.js) are
 * never targets; in a year the list does not cover only weekends close. Pure,
 * tested.
 */
export function targetSession(now = new Date()) {
  const w = new Date(now.getTime() + 7 * 3600e3);
  const today = w.toISOString().slice(0, 10);
  const mins = w.getUTCHours() * 60 + w.getUTCMinutes();
  return hol.isTradingDay(today) && mins >= READY_MIN ? today : hol.previousTradingDay(today);
}

const valueOf = (q) => (q.price || 0) * (q.volume || 0);
/** The WIB date of a quote's last trade, or null. */
const tradedOn = (q) => (q?.market_time ? hol.wibDate(q.market_time) : null);

/**
 * The board's stocks most traded on `session`, by value (price x volume),
 * most first. A quote whose last trade is dated another day is left out: a
 * suspended or idle stock keeps its last volume. When the board has moved
 * past the session (quotes dated later number more than a tenth of those
 * dated on it) or holds no quote of it, the whole board is ranked by its
 * latest value instead, and `ranked_from` names the newest date it holds.
 * Without a session the whole board is ranked. Pure, tested.
 */
export function ranking(quotes, session = null, n = SCAN_SIZE) {
  const stocks = quotes.filter((q) => valueOf(q) > 0 && !String(q.symbol).startsWith("^"));
  const top = (list) => [...list].sort((a, b) => valueOf(b) - valueOf(a)).slice(0, n).map((q) => q.symbol);
  if (!session) return { symbols: top(stocks), ranked_from: null };
  const on = stocks.filter((q) => tradedOn(q) === session);
  const later = stocks.filter((q) => (tradedOn(q) ?? "") > session).length;
  if (on.length && later * 10 <= on.length) return { symbols: top(on), ranked_from: session };
  return { symbols: top(stocks), ranked_from: stocks.map(tradedOn).filter(Boolean).sort().pop() ?? null };
}

/** The symbols of {@link ranking}. Pure, tested. */
export const mostTraded = (quotes, n = SCAN_SIZE, session = null) => ranking(quotes, session, n).symbols;

async function kvGet(env, k) {
  const row = await env.DB.prepare("SELECT v FROM kv WHERE k = ?1").bind(k).first();
  return row ? JSON.parse(row.v) : null;
}
async function kvPut(env, k, v) {
  await env.DB.prepare("INSERT OR REPLACE INTO kv (k, v, updated_at) VALUES (?1, ?2, ?3)").bind(k, JSON.stringify(v), Math.floor(Date.now() / 1000)).run();
}

const named = (symbol, p, from, to) => ({
  symbol, name: NAMES.get(symbol) || null, from: p.from || from, to: p.to || to, investor: "all", board: "all",
  buyers: p.buyers.map((f) => ({ ...f, name: BROKERS[f.code] ?? null })), sellers: p.sellers.map((f) => ({ ...f, name: BROKERS[f.code] ?? null })),
  total_value: p.total_value, total_lots: p.total_lots, foreign_net_value: p.foreign_net_value, average_price: p.average_price,
  buy_value: p.total_value, sell_value: p.total_value, rounded: p.rounded,
});

/**
 * One stock: a row per period, or why there is none. `today` is the date in
 * Jakarta when the scan runs; pages are read fresh, so it is also the day
 * they were fetched.
 */
export async function scanSymbol(symbol, session, quote, { fetchPage = ipot.fetchPage, fetchChart = y.fetchChart, today = session } = {}) {
  let candles = [];
  try {
    const res = await fetchChart(symbol, "6mo", "1d");
    if (!res.notFound) candles = y.parseChart(res.body, "6mo", "1d").candles;
  } catch (e) {
    // No price move then; the reading itself does not need it. A spent
    // budget is not a missing chart: the stock is read again next tick.
    if (isBudgetError(e)) throw e;
  }
  // Each page once per stock: the 1d page is the scanned session the
  // rebuilds start from.
  const pages = new Map();
  const get = async (from, to) => {
    const k = `${from}:${to}`;
    if (!pages.has(k)) pages.set(k, ipot.parse(await fetchPage(ipot.url(symbol, from, to))));
    return { page: pages.get(k), fetchedOn: today };
  };
  const rows = [];
  for (const period of bd.PERIODS) {
    const from = bd.periodStart(period, session);
    const { page: p } = await get(from, session);
    if (p.invalid) return { invalid: true, rows: [] };
    const summary = named(symbol, p, from, session);
    if (ipot.rangeNetSuspect(from, session, p, today, today)) summary.foreign_net_value = await ipot.rebuildNet(from, session, today, get);
    const move = bd.priceMove(candles, from, session, quote);
    rows.push({ period, v: { summary, price: move.price ?? quote?.price ?? null, chg: move.chg } });
  }
  return { rows };
}

async function store(env, session, symbol, rows) {
  if (!rows.length) return;
  const stmt = env.DB.prepare("INSERT OR REPLACE INTO bandar_scan (session, period, symbol, v) VALUES (?1, ?2, ?3, ?4)");
  await env.DB.batch(rows.map((r) => stmt.bind(session, r.period, symbol, JSON.stringify(r.v))));
}

/**
 * One cron step. Returns what it did; `{ idle: true }` when the target
 * session is done. `budget` is the invocation's (src/budget.js; a fresh one
 * by default): a stock starts only while STOCK_RESERVE subrequests and some
 * of the minute are left, and with `stocks` false (a minute the other cron
 * jobs keep busy) the step reads no stock. `pauseMs` sets IPOT's 404 pauses
 * (tests).
 */
export async function bandarScanStep(env, { now = new Date(), force = false, budget: b = budget(), stocks = true, pauseMs } = {}) {
  const fetchPage = (u) => ipot.fetchPage(u, { fetchFn: b.fetch, ...(pauseMs == null ? {} : { pauseMs }) });
  const fetchChart = (s, period, interval) => y.fetchChart(s, period, interval, { fetchFn: b.fetch });
  const session = targetSession(now);
  let st = await kvGet(env, STATE);
  if (st?.session === session && st.done && !force) return { idle: true, session };
  if (!st || st.session !== session || (force && st.done)) {
    const board = await kvGet(env, "board");
    const { symbols, ranked_from } = ranking(board?.quotes || [], session);
    if (!symbols.length) return { skipped: "no quote board yet" };
    // Probe the most traded stock: no trades that day means a holiday.
    const probe = ipot.parse(await fetchPage(ipot.url(symbols[0], session, session)));
    if (!probe.invalid && !probe.buyers.length && !probe.sellers.length) {
      st = { session, holiday: true, done: true, complete: st?.complete || null, symbols: [], next: 0, retry: [], failed: [], checked: 0 };
      await kvPut(env, STATE, st);
      return { holiday: session };
    }
    st = { session, symbols, ranked_from, next: 0, retry: [], retried: false, failed: [], checked: 0, done: false, complete: st?.complete || null, started_at: now.toISOString() };
  }
  const board = await kvGet(env, "board");
  const bySym = new Map((board?.quotes || []).map((q) => [q.symbol, q]));
  let read = 0;
  for (const symbol of st.symbols.slice(st.next, st.next + PER_TICK)) {
    if (!stocks || b.left() < STOCK_RESERVE || b.elapsedMs() >= TICK_MS) break;
    try {
      const r = await scanSymbol(symbol, session, bySym.get(symbol), { today: new Date(now.getTime() + 7 * 3600e3).toISOString().slice(0, 10), fetchPage, fetchChart });
      if (r.invalid) st.failed.push(symbol);
      else {
        await store(env, session, symbol, r.rows);
        if (r.rows[0]?.v.summary.buyers.length) st.checked++;
      }
    } catch (e) {
      // The budget ran out under it: the stock stays next in line.
      if (isBudgetError(e)) break;
      (st.retried ? st.failed : st.retry).push(symbol);
    }
    read++;
  }
  st.next += read;
  if (st.next >= st.symbols.length) {
    if (st.retry.length && !st.retried) {
      // One more pass over the stocks that failed, at the end of the list.
      st.symbols = st.symbols.concat(st.retry);
      st.retry = [];
      st.retried = true;
    } else {
      st.done = true;
      st.complete = session;
      st.done_at = now.toISOString();
      await env.DB.prepare("DELETE FROM bandar_scan WHERE session NOT IN (SELECT DISTINCT session FROM bandar_scan ORDER BY session DESC LIMIT ?1)").bind(KEEP_SESSIONS).run();
    }
  }
  await kvPut(env, STATE, st);
  return { session, scanned: Math.min(st.next, st.symbols.length), of: st.symbols.length, done: !!st.done };
}

/** What the screen reads: the newest finished session, else the running one. */
export async function scanRows(env, period) {
  const st = await kvGet(env, STATE);
  const running = st && !st.done && st.session !== st.complete ? st : null;
  const session = st?.complete || running?.session || null;
  if (!session) return { session: null, state: st, rows: [] };
  const res = await env.DB.prepare("SELECT symbol, v FROM bandar_scan WHERE session = ?1 AND period = ?2").bind(session, period).all();
  return { session, state: st, partial: !st?.complete, rows: (res.results || []).map((r) => ({ symbol: r.symbol, ...JSON.parse(r.v) })) };
}

export async function scanStatus(env) {
  const st = await kvGet(env, STATE);
  if (!st) return { session: null, complete: null, running: false };
  return {
    session: st.session, complete: st.complete, running: !st.done, holiday: !!st.holiday,
    scanned: Math.min(st.next || 0, (st.symbols || []).length), of: (st.symbols || []).length, with_trades: st.checked || 0,
    failed: st.failed || [], started_at: st.started_at || null, done_at: st.done_at || null,
  };
}
