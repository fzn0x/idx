// The bandarmology scan behind the web terminal's `bandar screen` and
// `bandar broker`.
//
// The minute cron (src/cron.js) runs one step a minute. The session to scan
// comes from the bundled calendar (src/holidays.js): today from 16:45 WIB on
// a trading day, else the trading day before. The first step ranks the quote
// board's 100 stocks most traded that session and probes the first one's
// one-day IPOT broker summary. IPOT publishes a closed session's broker
// lists some time after the close and shows the totals without them until
// then, so the scan waits and probes again, every 10 minutes until 21:00 WIB
// and every 30 after; only a page without trades, on a day the quote board
// (refreshed after that session) shows no trade either, closes the session
// unscanned. Then PER_TICK stocks a minute: for each, the IPOT broker summary
// over 1d, 1w, 1m and 3m ending on that session and the daily candles for
// the price move, one D1 row per stock and period in `bandar_scan`, written
// with the state in one batch. /api/bandar/screen and /api/bandar/broker
// answer from D1 alone with the same rules as the CLI (src/bandar.js). The
// foreign net of 1w, 1m and 3m is rebuilt from single sessions (the range
// page counts the latest session twice): the session before the scanned one
// and each period's rest, so a stock costs 9 requests (a chart, four period
// pages, one more session, three rests), inside the invocation's budget
// (src/budget.js). A stock whose fetch failed is tried once more at the end
// of the pass.

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
/**
 * Minutes between probes while IPOT has not published the session: until
 * 21:00 WIB of the session's day, after that, and once a day of probes has
 * passed (each of those logs an error: IPOT may have changed its page).
 */
const PROBE_GAP_MIN = { early: 10, late: 30, stale: 60 };
const LATE_MIN = 21 * 60;
/** Minutes before a probe that failed (an error, not an answer) is tried again. */
const PROBE_RETRY_MIN = 5;
const DAY_MS = 24 * 3600e3;
const NAMES = new Map(UNIVERSE.map(([s, n]) => [s, n]));

const WIB_MS = 7 * 3600e3;
const wibMinutes = (d) => { const w = new Date(d.getTime() + WIB_MS); return w.getUTCHours() * 60 + w.getUTCMinutes(); };
const after = (now, min) => new Date(now.getTime() + min * 60e3).toISOString();
const short = (e) => String(e?.message ?? e).slice(0, 160);

/**
 * The session to scan: today from 16:45 WIB on a trading day, otherwise the
 * trading day before. Weekends and the bundled holidays (src/holidays.js) are
 * never targets; in a year the list does not cover only weekends close. Pure,
 * tested.
 */
export function targetSession(now = new Date()) {
  const today = hol.wibDate(now);
  return hol.isTradingDay(today) && wibMinutes(now) >= READY_MIN ? today : hol.previousTradingDay(today);
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

/**
 * Whether the quote board shows a trade dated `session`; null when it was
 * refreshed before that session ended, so it cannot tell. Pure, tested.
 */
export function boardTraded(board, session) {
  const end = Date.parse(`${session}T00:00:00+07:00`) + hol.SESSION_END_SECS * 1000;
  if (!(board?.ts * 1000 >= end)) return null;
  return (board.quotes || []).some((q) => !String(q.symbol).startsWith("^") && tradedOn(q) === session);
}

/**
 * What the probe's one-day page says of its session: "published" (broker
 * lists, or a code IPOT does not know, so the stocks speak for themselves),
 * "unpublished" (totals without lists, or a page without trades while the
 * board shows trades that day or cannot tell) or "closed" (no trades at IPOT
 * nor on a board refreshed after that session). Pure, tested.
 */
export function probeVerdict(page, boardTradedOnSession) {
  if (page.invalid || page.buyers.length || page.sellers.length) return "published";
  if (ipot.unpublished(page) || boardTradedOnSession !== false) return "unpublished";
  return "closed";
}

/**
 * A state from before the phases that marked a trading day of the bundled
 * calendar as a holiday: its probe had read IPOT before the session's lists
 * were out (2026-11-06). It counts as no state for that session. Pure, tested.
 */
export const legacyHoliday = (st) => !!st?.holiday && st.phase == null && hol.covers(Number(String(st.session).slice(0, 4))) && hol.isTradingDay(String(st.session));

/** The state as this code keeps it; one written before the phases is read into them. */
function upgrade(st) {
  if (!st || st.phase) return st;
  const base = { v: 2, session: st.session, complete: st.complete ?? null };
  if (legacyHoliday(st)) return { ...base, phase: "new" };
  if (st.holiday) return { ...base, phase: "closed", holiday: true };
  if (st.done) {
    const symbols = [...new Set(st.symbols || [])], failed = st.failed || [];
    return { ...base, phase: "done", symbols, quotes: {}, queue: [], retry: [], retried: true, failed, stored: symbols.length - failed.length, checked: st.checked || 0, started_at: st.started_at ?? null, done_at: st.done_at ?? null };
  }
  // A pass in progress starts over.
  return { ...base, phase: "new" };
}

const fresh = (session, prev) => ({ v: 2, session, phase: "new", complete: prev?.complete ?? null });

/** Stocks of the first pass the scan has got to. */
const attempted = (st) => (st.symbols?.length ?? 0) - (st.retried ? 0 : (st.queue?.length ?? st.symbols?.length ?? 0));

async function kvGet(env, k) {
  const row = await env.DB.prepare("SELECT v FROM kv WHERE k = ?1").bind(k).first();
  return row ? JSON.parse(row.v) : null;
}

/** The step's rows, the pruning of old sessions when it finished one, and the state: one D1 batch. */
async function save(env, st, rows, prune) {
  const ins = env.DB.prepare("INSERT OR REPLACE INTO bandar_scan (session, period, symbol, v) VALUES (?1, ?2, ?3, ?4)");
  const list = rows.map((r) => ins.bind(st.session, r.period, r.symbol, JSON.stringify(r.v)));
  if (prune) list.push(env.DB.prepare("DELETE FROM bandar_scan WHERE session NOT IN (SELECT DISTINCT session FROM bandar_scan ORDER BY session DESC LIMIT ?1)").bind(KEEP_SESSIONS));
  list.push(env.DB.prepare("INSERT OR REPLACE INTO kv (k, v, updated_at) VALUES (?1, ?2, ?3)").bind(STATE, JSON.stringify(st), Math.floor(Date.now() / 1000)));
  await env.DB.batch(list);
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

/** Minutes to the next probe of a session IPOT has not published. */
function probeGap(st, now) {
  if (now.getTime() - Date.parse(st.first_probe_at) >= DAY_MS) return PROBE_GAP_MIN.stale;
  return hol.wibDate(now) === st.session && wibMinutes(now) < LATE_MIN ? PROBE_GAP_MIN.early : PROBE_GAP_MIN.late;
}

/** Reads the probe stock's one-day page and moves the state on; returns the verdict, "error" or "budget". */
async function probe(st, now, fetchPage) {
  const at = now.toISOString();
  let page;
  try {
    page = ipot.parse(await fetchPage(ipot.url(st.symbols[0], st.session, st.session)));
  } catch (e) {
    if (isBudgetError(e)) return "budget"; // the next minute probes
    st.first_probe_at ??= at;
    st.probe_errors = (st.probe_errors || 0) + 1;
    st.probe_error = short(e);
    st.next_probe_at = after(now, PROBE_RETRY_MIN);
    return "error";
  }
  st.first_probe_at ??= at;
  st.probes = (st.probes || 0) + 1;
  st.last_probe_at = at;
  st.probe_error = null;
  const verdict = probeVerdict(page, st.board_traded);
  if (verdict === "published") {
    Object.assign(st, { phase: "running", published_at: at, started_at: at, next_probe_at: null, queue: [...st.symbols], retry: [], retried: false, failed: [], stored: 0, checked: 0 });
    // A week of these shows when IPOT publishes a session.
    console.log("bandar published", JSON.stringify({ session: st.session, published_at: at, last_unpublished_at: st.last_unpublished_at ?? null, probes: st.probes }));
  } else if (verdict === "closed") {
    Object.assign(st, { phase: "closed", holiday: true, next_probe_at: null, done_at: at });
  } else {
    st.last_unpublished_at = at;
    st.next_probe_at = after(now, probeGap(st, now));
    if (now.getTime() - Date.parse(st.first_probe_at) >= DAY_MS) {
      console.error("bandar scan: IPOT has not published", st.session, "since the first probe at", st.first_probe_at, "; its page may have changed");
    }
  }
  return verdict;
}

/** Up to PER_TICK stocks off the queue into `rows`, while the budget and the minute last; returns how many it read. */
async function readStocks(st, rows, { now, b, fetchPage, fetchChart }) {
  const today = hol.wibDate(now);
  let read = 0;
  while (read < PER_TICK) {
    if (!st.queue.length) {
      if (!st.retry.length || st.retried) break;
      // One more pass over the stocks that failed.
      st.queue = st.retry;
      st.retry = [];
      st.retried = true;
    }
    if (b.left() < STOCK_RESERVE || b.elapsedMs() >= TICK_MS) break;
    const symbol = st.queue[0];
    let r;
    try {
      r = await scanSymbol(symbol, st.session, st.quotes?.[symbol] ?? null, { today, fetchPage, fetchChart });
    } catch (e) {
      // The budget ran out under it: it stays first in line.
      if (isBudgetError(e)) break;
      st.queue.shift();
      (st.retried ? st.failed : st.retry).push(symbol);
      read++;
      continue;
    }
    st.queue.shift();
    read++;
    if (r.invalid) {
      st.failed.push(symbol);
      continue;
    }
    for (const row of r.rows) rows.push({ symbol, ...row });
    st.stored++;
    if (r.rows[0]?.v.summary.buyers.length) st.checked++;
  }
  return read;
}

/**
 * One cron step. Returns what it did; `{ idle: true }` when there is nothing
 * to do this minute. `budget` is the invocation's (src/budget.js; a fresh
 * one by default): a stock starts only while STOCK_RESERVE subrequests and
 * some of the minute are left, and with `stocks` false (a minute the other
 * cron jobs keep busy) the step reads no stock. `force` probes now, and
 * starts a finished session over. `pauseMs` sets IPOT's 404 pauses (tests).
 */
export async function bandarScanStep(env, { now = new Date(), force = false, budget: b = budget(), stocks = true, pauseMs } = {}) {
  const fetchPage = (u) => ipot.fetchPage(u, { fetchFn: b.fetch, ...(pauseMs == null ? {} : { pauseMs }) });
  const fetchChart = (s, period, interval) => y.fetchChart(s, period, interval, { fetchFn: b.fetch });
  const session = targetSession(now);
  let st = upgrade(await kvGet(env, STATE));
  if (!st || st.session !== session || (force && (st.phase === "done" || st.phase === "closed"))) st = fresh(session, st);
  if (st.phase === "done" || st.phase === "closed") return { idle: true, session };
  if (st.phase === "waiting" && !force && st.next_probe_at && now.getTime() < Date.parse(st.next_probe_at)) {
    return { idle: true, session, phase: "waiting", next_probe_at: st.next_probe_at };
  }
  const out = { session };
  let changed = false;
  if (st.phase === "new") {
    const board = await kvGet(env, "board");
    const { symbols, ranked_from } = ranking(board?.quotes || [], session);
    if (!symbols.length) return { skipped: "no quote board yet" };
    // The quotes the price moves need, so later steps do not parse the board again.
    const bySym = new Map(board.quotes.map((q) => [q.symbol, q]));
    const quotes = Object.fromEntries(symbols.map((s) => { const q = bySym.get(s); return [s, { price: q.price ?? null, previous_close: q.previous_close ?? null, market_time: q.market_time ?? null }]; }));
    Object.assign(st, { phase: "waiting", symbols, quotes, ranked_from, board_traded: boardTraded(board, session), probes: 0 });
    changed = true;
  }
  if (st.phase === "waiting") {
    out.probe = await probe(st, now, fetchPage);
    changed ||= out.probe !== "budget";
  }
  const rows = [];
  let prune = false;
  if (st.phase === "running" && stocks) {
    out.read = await readStocks(st, rows, { now, b, fetchPage, fetchChart });
    changed ||= out.read > 0;
    if (!st.queue.length && (!st.retry.length || st.retried)) {
      Object.assign(st, { phase: "done", complete: session, done_at: now.toISOString() });
      changed = prune = true;
    }
  }
  if (!changed) return { ...out, idle: true, phase: st.phase };
  await save(env, st, rows, prune);
  return { ...out, phase: st.phase, scanned: attempted(st), of: st.symbols.length, done: st.phase === "done" };
}

/** What the screen reads: the newest finished session, else the running one. */
export async function scanRows(env, period) {
  const st = upgrade(await kvGet(env, STATE));
  const running = st && st.phase === "running" && st.session !== st.complete ? st : null;
  const session = st?.complete || running?.session || null;
  if (!session) return { session: null, state: st, rows: [] };
  const res = await env.DB.prepare("SELECT symbol, v FROM bandar_scan WHERE session = ?1 AND period = ?2").bind(session, period).all();
  return { session, state: st, partial: !st?.complete, rows: (res.results || []).map((r) => ({ symbol: r.symbol, ...JSON.parse(r.v) })) };
}

const hhmm = (iso) => new Date(Date.parse(iso) + WIB_MS).toISOString().slice(11, 16);

/**
 * The screen's and the broker's notes on the scan: which session they show
 * and why, as warnings, and how many of its stocks failed. `shown` is the
 * session the rows come from, `rows` how many stocks they hold. Pure, tested
 * through the API.
 */
export function scanNotes(st, shown, rows) {
  const warnings = [];
  if (!st) return { warnings, failed: 0 };
  const of = st.symbols?.length ?? 0;
  if (st.session === shown && st.phase === "running") warnings.push(`the scan of ${shown} is still running: ${rows} of ${of} stocks so far`);
  else if (st.session > shown) {
    if (st.phase === "new") warnings.push(`the scan of ${st.session} has not started yet; showing ${shown}`);
    else if (st.phase === "waiting") {
      const n = (st.probes || 0) + (st.probe_errors || 0);
      const since = st.first_probe_at ? ` (checked ${n} time${n === 1 ? "" : "s"} since ${hhmm(st.first_probe_at)} WIB${st.probe_error ? `; the last check failed: ${st.probe_error}` : ""})` : "";
      warnings.push(`the scan of ${st.session} is waiting for IPOT to publish that session's broker lists${since}; showing ${shown}`);
    } else if (st.phase === "running") warnings.push(`the scan of ${st.session} is running (${st.stored + st.failed.length} of ${of}); showing ${shown}`);
    else if (st.phase === "closed") warnings.push(`${st.session} had no trades at IPOT nor on the quote board, so it was not scanned; showing ${shown}`);
  }
  return { warnings, failed: st.session === shown ? (st.failed || []).length : 0 };
}

export async function scanStatus(env) {
  const st = upgrade(await kvGet(env, STATE));
  if (!st) return { session: null, complete: null, running: false };
  return {
    session: st.session, complete: st.complete, running: ["new", "waiting", "running"].includes(st.phase), holiday: st.phase === "closed",
    scanned: attempted(st), of: (st.symbols || []).length, with_trades: st.checked || 0,
    failed: st.failed || [], started_at: st.started_at || null, done_at: st.done_at || null,
    phase: st.phase, probes: st.probes || 0, probe_errors: st.probe_errors || 0, probe_error: st.probe_error ?? null,
    first_probe_at: st.first_probe_at ?? null, next_probe_at: st.next_probe_at ?? null, published_at: st.published_at ?? null,
    ranked_from: st.ranked_from ?? null,
  };
}
