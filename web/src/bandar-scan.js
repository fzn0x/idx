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
import BROKERS from "./brokers.json" with { type: "json" };
import UNIVERSE from "./universe.json" with { type: "json" };

export const SCAN_SIZE = 100;
const PER_TICK = 4;
const KEEP_SESSIONS = 2;
const STATE = "bandar_scan";
/** Minutes after midnight WIB from which the session's summaries are final. */
const READY_MIN = 16 * 60 + 45;
const NAMES = new Map(UNIVERSE.map(([s, n]) => [s, n]));

const addDays = (iso, n) => new Date(Date.parse(`${iso}T00:00:00Z`) + n * 86400e3).toISOString().slice(0, 10);
const weekend = (iso) => [0, 6].includes(new Date(`${iso}T00:00:00Z`).getUTCDay());
const prevWeekday = (iso) => { let d = addDays(iso, -1); while (weekend(d)) d = addDays(d, -1); return d; };

/** The session to scan: today from 16:45 WIB on a weekday, otherwise the weekday before. Pure, tested. */
export function targetSession(now = new Date()) {
  const w = new Date(now.getTime() + 7 * 3600e3);
  const today = w.toISOString().slice(0, 10);
  const mins = w.getUTCHours() * 60 + w.getUTCMinutes();
  return weekend(today) || mins < READY_MIN ? prevWeekday(today) : today;
}

/** The board's stocks ranked by value traded (price x volume), most first. Pure, tested. */
export function mostTraded(quotes, n = SCAN_SIZE) {
  const value = (q) => (q.price || 0) * (q.volume || 0);
  return quotes.filter((q) => value(q) > 0 && !q.symbol.startsWith("^")).sort((a, b) => value(b) - value(a)).slice(0, n).map((q) => q.symbol);
}

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
  total_value: p.total_value, total_lots: p.total_lots, foreign_net_value: p.foreign_net_value, average_price: p.average_price, rounded: p.rounded,
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
  } catch { /* no price move then; the reading itself does not need it */ }
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

/** One cron step. Returns what it did; `{ idle: true }` when the target session is done. */
export async function bandarScanStep(env, { now = new Date(), force = false } = {}) {
  const session = targetSession(now);
  let st = await kvGet(env, STATE);
  if (st?.session === session && st.done && !force) return { idle: true, session };
  if (!st || st.session !== session || (force && st.done)) {
    const board = await kvGet(env, "board");
    const quotes = board?.quotes || [];
    const symbols = mostTraded(quotes);
    if (!symbols.length) return { skipped: "no quote board yet" };
    // Probe the most traded stock: no trades that day means a holiday.
    const probe = ipot.parse(await ipot.fetchPage(ipot.url(symbols[0], session, session)));
    if (!probe.invalid && !probe.buyers.length && !probe.sellers.length) {
      st = { session, holiday: true, done: true, complete: st?.complete || null, symbols: [], next: 0, retry: [], failed: [], checked: 0 };
      await kvPut(env, STATE, st);
      return { holiday: session };
    }
    st = { session, symbols, next: 0, retry: [], retried: false, failed: [], checked: 0, done: false, complete: st?.complete || null, started_at: now.toISOString() };
  }
  const board = await kvGet(env, "board");
  const bySym = new Map((board?.quotes || []).map((q) => [q.symbol, q]));
  const batch = st.symbols.slice(st.next, st.next + PER_TICK);
  for (const symbol of batch) {
    try {
      const r = await scanSymbol(symbol, session, bySym.get(symbol), { today: new Date(now.getTime() + 7 * 3600e3).toISOString().slice(0, 10) });
      if (r.invalid) st.failed.push(symbol);
      else {
        await store(env, session, symbol, r.rows);
        if (r.rows[0]?.v.summary.buyers.length) st.checked++;
      }
    } catch (e) {
      (st.retried ? st.failed : st.retry).push(symbol);
    }
  }
  st.next += batch.length;
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
