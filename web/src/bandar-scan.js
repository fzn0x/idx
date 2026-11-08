// The bandarmology scan behind the web terminal's `bandar screen` and
// `bandar broker`.
//
// The minute cron (src/cron.js) runs one step a minute. The session to scan
// comes from the bundled calendar (src/holidays.js): today from 16:45 WIB on
// a trading day, else the trading day before. The first step ranks the quote
// board's 100 stocks most traded that session and probes the one-day IPOT
// broker summary of the first, then of the second and third in turn. IPOT
// publishes a closed session's broker lists some time after the close and
// shows the totals without them until then, so the scan waits and probes
// again, every 10 minutes until 21:00 WIB and every 30 after; only a page
// without trades, on a day the quote board (refreshed after that session)
// shows no trade either, closes the session unscanned. Then PER_TICK stocks a
// minute: for each, the IPOT broker summary over 1d, 1w, 1m and 3m ending on
// that session and the daily candles for the price move, one D1 row per stock
// and period in `bandar_scan`, written with the state in one batch.
// /api/bandar/screen and /api/bandar/broker answer from D1 alone with the
// same rules as the CLI (src/bandar.js). The foreign net of 1w, 1m and 3m is
// rebuilt from single sessions (the range page counts the latest session
// twice): the session before the scanned one and each period's rest, so a
// stock costs 9 requests (a chart, four period pages, one more session, three
// rests), inside the invocation's budget (src/budget.js) and its first 45
// seconds, so a minute ends before the next one reads the state.
//
// A stock whose session IPOT has not published yet is read again on the
// probe's schedule; one that failed is tried again in rounds 10, 30, 60 and
// 120 minutes apart, then every 4 hours, eight in all. Once the first pass
// is over, the session is filed in a record (`runs`): what it stored, failed
// and did not get, filed again when a later read changes it, and for good at
// the next session's 16:45, as is a session IPOT never published or never
// answered the probe for. A session is shown once it holds half its stocks
// or more than the one shown before; the API's warnings come from these
// records (scanNotes).

import * as ipot from "./ipot.js";
import * as y from "./yahoo.js";
import * as bd from "./bandar.js";
import * as hol from "./holidays.js";
import { budget, isBudgetError, BudgetError } from "./budget.js";
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
/** ...and only this soon into the invocation... */
const TICK_MS = 40_000;
/**
 * ...and only when the slowest stock of the minute so far would end before
 * this; no request of the scan goes out after it, so a minute ends (its last
 * request has 10 s) before the next one reads the state.
 */
const DEADLINE_MS = 45_000;
const KEEP_SESSIONS = 2;
const STATE = "bandar_scan";
/** Minutes after midnight WIB from which the scan looks at the day's session. */
const READY_MIN = 16 * 60 + 45;
/**
 * Minutes between probes while IPOT has not published the session: until
 * 21:00 WIB of the session's day, after that, and once a day of probes has
 * passed (each of those logs an error: IPOT may have changed its page). The
 * stocks IPOT has not published once the scan reads are read again on the
 * same schedule, the day counted from when it started reading.
 */
const PROBE_GAP_MIN = { early: 10, late: 30, stale: 60 };
const LATE_MIN = 21 * 60;
/**
 * Minutes before a probe that failed (an error, not an answer) is tried
 * again, doubled for each failure in a row up to an hour; failures for an
 * hour in a row log an error (IPOT may block or have moved the page).
 */
const PROBE_RETRY_MIN = 5;
const PROBE_ALARM_MS = 3600e3;
/** The most traded stocks the probe turns among. */
const PROBE_STOCKS = 3;
/**
 * Minutes after the first pass, then after each retry round, before the
 * stocks that failed in it are tried again: eight rounds over about 20 hours
 * (IPOT can fail for hours), then a failure counts, and at the next
 * session's 16:45 WIB whatever still waits does. A source that fails a stock
 * tends to fail its neighbours too (2026-10-30: 72 of 100), so an immediate
 * retry fails the same way.
 */
const RETRY_GAPS_MIN = [10, 30, 60, 120, 240, 240, 240, 240];
/** Times the budget may cut a stock short before that counts as a failure of kind "budget". */
const CUTS_ALLOWED = 1;
/** Failures the state keeps word for word, newest first. */
const ERRORS_KEPT = 5;
/**
 * Records of finished sessions the state keeps (the API reads the shown
 * one's and the newer ones'); the shown one and the closures after it are
 * kept besides, since the sessions-behind count leaves those days out.
 */
const RUNS_KEPT = 4;
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
 * most first. A quote whose last trade is dated before the session is left
 * out: a suspended or idle stock keeps its last volume. Once the next
 * session has opened, the stocks that traded since come first, by their
 * latest value: the most traded trade first, and their value of the session
 * has left the board. When the board holds no quote of the session or
 * later, the whole board is ranked by its latest value. `ranked_from` names
 * the newest date the ranking read. Without a session the whole board is
 * ranked. Pure, tested.
 */
export function ranking(quotes, session = null, n = SCAN_SIZE) {
  const stocks = quotes.filter((q) => valueOf(q) > 0 && !String(q.symbol).startsWith("^"));
  const byValue = (list) => [...list].sort((a, b) => valueOf(b) - valueOf(a));
  const ranked = (list) => { const top = list.slice(0, n); return { symbols: top.map((q) => q.symbol), ranked_from: top.map(tradedOn).filter(Boolean).sort().pop() ?? null }; };
  if (!session) return { symbols: byValue(stocks).slice(0, n).map((q) => q.symbol), ranked_from: null };
  const on = stocks.filter((q) => tradedOn(q) === session);
  const later = stocks.filter((q) => (tradedOn(q) ?? "") > session);
  if (!on.length && !later.length) return ranked(byValue(stocks));
  return ranked([...byValue(later), ...byValue(on)]);
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
 * lists), "invalid" (a code IPOT does not know, which says nothing of the
 * session), "unpublished" (totals without lists, or a page without trades
 * while the board shows trades that day or cannot tell) or "closed" (no
 * trades at IPOT nor on a board refreshed after that session). Pure, tested.
 */
export function probeVerdict(page, boardTradedOnSession) {
  if (page.invalid) return "invalid";
  if (page.buyers.length || page.sellers.length) return "published";
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
  const base = { v: 2, session: st.session, complete: st.complete ?? null, runs: [] };
  if (legacyHoliday(st)) return { ...base, phase: "new" };
  if (st.holiday) return { ...base, phase: "closed", holiday: true };
  if (st.done) {
    // Its retry pass appended the stocks it retried to `symbols`.
    const symbols = [...new Set(st.symbols || [])], failed = st.failed || [];
    const done = { ...base, phase: "done", symbols, quotes: {}, queue: [], retry: [], pending: [], pass: RETRY_GAPS_MIN.length, failed, errors: {}, stored: symbols.length - failed.length, checked: st.checked || 0, started_at: st.started_at ?? null, done_at: st.done_at ?? null };
    done.runs = [{ ...runOf(done, done.done_at), promoted: done.complete === done.session }];
    return done;
  }
  // A pass in progress starts over.
  return { ...base, phase: "new" };
}

/** A new state for `session`: what the last one showed and its records carry over. */
const fresh = (session, prev) => ({ v: 2, session, phase: "new", complete: prev?.complete ?? null, runs: prev?.runs ?? [] });

/**
 * The record of the scan of `st.session` as the API reads it; `at` is when
 * it stopped reading. Each stock counts once: stored, failed (for good, or
 * still to be tried again: `retrying` of them), unpublished (IPOT had not
 * published it when last read) or left (the first pass had not got to it).
 */
function runOf(st, at) {
  const pending = st.pending || [], retry = st.retry || [];
  const failed = [...st.failed, ...retry], errors = { ...st.errors };
  for (const s of retry) { const kind = st.kinds?.[s] || "other"; errors[kind] = (errors[kind] || 0) + 1; }
  return {
    session: st.session, scanned: true, of: st.symbols.length, stored: st.stored, checked: st.checked,
    failed, retrying: retry.length, unpublished: [...pending], left: st.queue.length, errors, ranked_from: st.ranked_from ?? null,
    started_at: st.started_at ?? null, published_at: st.published_at ?? null, stored_at: st.stored_at ?? null, done_at: at,
    partial: failed.length + pending.length + st.queue.length > 0,
  };
}

/** What a scanned record says of its stocks, whatever their order and the time it was filed. */
const says = (r) => JSON.stringify([r.stored, r.checked, [...r.failed].sort(), r.retrying ?? 0, [...r.unpublished].sort(), r.left]);

/**
 * Files `run` as the newest record and makes its session the one shown when
 * it holds enough: some stocks, and half of them or more than the shown
 * session's record (none for a session kept from before these records). The
 * shown session's record is kept however many came after it. A forced
 * rescan files nothing over the session's earlier scan until it has stored
 * as many stocks: that scan's rows are still in D1.
 */
function keep(st, run) {
  const prior = st.runs?.find((r) => r.session === run.session && r.scanned);
  if (prior && (!run.scanned || run.stored < prior.stored)) return;
  const shown = st.runs?.find((r) => r.session === st.complete);
  if (run.scanned && run.stored > 0 && st.complete !== run.session && (!st.complete || run.stored * 2 >= run.of || run.stored > (shown?.stored ?? 0))) st.complete = run.session;
  run.promoted = st.complete === run.session;
  const needed = (r) => r.session === st.complete || (r.reason === "closed" && st.complete != null && r.session > st.complete);
  st.runs = [run, ...(st.runs || []).filter((r) => r.session !== run.session)].filter((r, i) => i < RUNS_KEPT || needed(r));
  st.done_at = run.done_at;
}

/**
 * The target moved to the next session: files what `st`'s session got. A
 * scan cut short counts the stocks still waiting for a retry as failed and
 * those IPOT had not published as unpublished. Returns whether it filed a
 * scan with rows, so old sessions are pruned.
 */
function cutoff(st, at) {
  if (st.phase === "waiting") {
    // Without one answer the scan cannot say IPOT had not published it.
    const reason = st.probes ? "unpublished" : "unreachable";
    keep(st, { session: st.session, scanned: false, reason, probes: st.probes || 0, probe_errors: st.probe_errors || 0, probe_error: st.probe_error ?? null, first_probe_at: st.first_probe_at ?? null, last_probe_at: st.last_probe_at ?? null, done_at: at });
    return false;
  }
  if (st.phase !== "running") return false;
  for (const s of st.retry) {
    st.failed.push(s);
    const kind = st.kinds?.[s] || "other";
    st.errors[kind] = (st.errors[kind] || 0) + 1;
  }
  st.retry = [];
  keep(st, runOf(st, at));
  return true;
}

/** Stocks of the first pass the scan has got to. */
const attempted = (st) => (st.symbols?.length ?? 0) - (st.queue?.length ?? st.symbols?.length ?? 0);

async function kvGet(env, k) {
  const row = await env.DB.prepare("SELECT v FROM kv WHERE k = ?1").bind(k).first();
  return row ? JSON.parse(row.v) : null;
}

/**
 * The step's rows, the state and the pruning of old sessions when it
 * finished one (the newest KEEP_SESSIONS stay, and the one shown): one D1
 * batch. The state is written only over the revision the step read (`rev`):
 * when a minute that overlapped this one wrote first, this one's state is
 * dropped (its rows are the same stocks', so they may stay), and so is its
 * prune, which runs only while the stored state is the one this step wrote:
 * by the dropped state's `complete` it could delete the shown session's rows.
 * Returns whether the state was written.
 */
async function save(env, st, rows, prune, rev) {
  st.rev = (rev ?? 0) + 1;
  const json = JSON.stringify(st);
  const ins = env.DB.prepare("INSERT OR REPLACE INTO bandar_scan (session, period, symbol, v) VALUES (?1, ?2, ?3, ?4)");
  const list = rows.map((r) => ins.bind(st.session, r.period, r.symbol, JSON.stringify(r.v)));
  const write = list.length;
  list.push(env.DB.prepare(
    "INSERT INTO kv (k, v, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(k) DO UPDATE SET v = excluded.v, updated_at = excluded.updated_at WHERE json_extract(kv.v, '$.rev') IS ?4",
  ).bind(STATE, json, Math.floor(Date.now() / 1000), rev ?? null));
  if (prune) {
    list.push(env.DB.prepare(
      "DELETE FROM bandar_scan WHERE session IS NOT ?2 AND session NOT IN (SELECT DISTINCT session FROM bandar_scan ORDER BY session DESC LIMIT ?1) AND EXISTS (SELECT 1 FROM kv WHERE k = ?3 AND v = ?4)",
    ).bind(KEEP_SESSIONS, st.complete, STATE, json));
  }
  const res = await env.DB.batch(list);
  return res?.[write]?.meta?.changes !== 0;
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
  // Each page once per stock: the 1d page is the scanned session the
  // rebuilds start from.
  const pages = new Map();
  const get = async (from, to) => {
    const k = `${from}:${to}`;
    if (!pages.has(k)) pages.set(k, ipot.parse(await fetchPage(ipot.url(symbol, from, to))));
    return { page: pages.get(k), fetchedOn: today };
  };
  // The session first: until IPOT publishes the stock's lists for it, its
  // ranges do not hold them either, so nothing more is read or stored. Like
  // the probe, a page without trades counts as not published while the
  // board's quote of the stock is dated that session.
  const { page: day } = await get(session, session);
  if (day.invalid) return { invalid: true, rows: [] };
  if (!day.buyers.length && !day.sellers.length && (ipot.unpublished(day) || tradedOn(quote) === session)) return { pending: true, rows: [] };
  let candles = [];
  try {
    const res = await fetchChart(symbol, "6mo", "1d");
    if (!res.notFound) candles = y.parseChart(res.body, "6mo", "1d").candles;
  } catch (e) {
    // No price move then; the reading itself does not need it. A spent
    // budget is not a missing chart: the stock is read again next tick.
    if (isBudgetError(e)) throw e;
  }
  const rows = [];
  for (const period of bd.PERIODS) {
    const from = bd.periodStart(period, session);
    const { page: p } = await get(from, session);
    if (p.invalid) return { invalid: true, rows: [] };
    const summary = named(symbol, p, from, session);
    if (ipot.rangeNetSuspect(from, session, p, today, today)) summary.foreign_net_value = await ipot.rebuildNet(from, session, today, get);
    const move = bd.priceMove(candles, from, session, quote);
    // Without candles, the quote's price only when it is the session's (a stock ranked after the next open has a later one).
    rows.push({ period, v: { summary, price: move.price ?? bd.liveQuote(quote, session)?.price ?? null, chg: move.chg } });
  }
  return { rows };
}

/** Minutes to the next look at `session`, which IPOT had not published, waited for since `since`. */
function waitGap(session, since, now) {
  if (now.getTime() - Date.parse(since) >= DAY_MS) return PROBE_GAP_MIN.stale;
  return hol.wibDate(now) === session && wibMinutes(now) < LATE_MIN ? PROBE_GAP_MIN.early : PROBE_GAP_MIN.late;
}
const probeGap = (st, now) => waitGap(st.session, st.first_probe_at, now);

/**
 * Reads a probe stock's one-day page and moves the state on; returns the
 * verdict, "error" or "budget". The probe turns among the PROBE_STOCKS most
 * traded: one that did not trade that session shows no trades however long
 * ago IPOT published it. A code IPOT does not know (a new listing) is not
 * asked again and the next stock is asked in the same minute; when IPOT knows
 * none of them, the session counts as published and the stocks speak for
 * themselves.
 */
async function probe(st, now, fetchPage) {
  const at = now.toISOString();
  st.probe_invalid ??= [];
  let verdict = "published";
  for (;;) {
    const known = st.symbols.slice(0, PROBE_STOCKS).filter((s) => !st.probe_invalid.includes(s));
    if (!known.length) break;
    const symbol = known[((st.probes || 0) + (st.probe_errors || 0)) % known.length];
    st.probe_symbol = symbol;
    let page;
    try {
      page = ipot.parse(await fetchPage(ipot.url(symbol, st.session, st.session)));
    } catch (e) {
      if (isBudgetError(e)) return "budget"; // the next minute probes
      st.first_probe_at ??= at;
      st.probe_errors = (st.probe_errors || 0) + 1;
      st.probe_error = short(e);
      st.failing_since ??= at;
      st.failures = (st.failures || 0) + 1;
      st.next_probe_at = after(now, Math.min(PROBE_RETRY_MIN * 2 ** (st.failures - 1), PROBE_GAP_MIN.stale));
      if (now.getTime() - Date.parse(st.failing_since) >= PROBE_ALARM_MS) {
        console.error(`bandar scan: IPOT has failed the probe of ${st.session} since ${stamp(st.failing_since)} (${st.failures} in a row, the last: ${st.probe_error}); it may block the Worker or have moved its page`);
      }
      return "error";
    }
    verdict = probeVerdict(page, st.board_traded);
    if (verdict !== "invalid") break;
    st.probe_invalid.push(symbol);
    verdict = "published";
  }
  // A session the scan has read traded (a forced rescan): no trades now means IPOT is not showing them.
  if (verdict === "closed" && st.runs?.some((r) => r.session === st.session && r.scanned)) verdict = "unpublished";
  st.first_probe_at ??= at;
  st.probes = (st.probes || 0) + 1;
  st.last_probe_at = at;
  st.probe_error = null;
  st.failing_since = null;
  st.failures = 0;
  if (verdict === "published") {
    Object.assign(st, { phase: "running", published_at: at, started_at: at, next_probe_at: null, queue: [...st.symbols], retry: [], retry_due: null, retry_left: 0, pass: 0, failed: [], errors: {}, last_errors: [], pending: [], pending_since: {}, pending_due: null, pending_left: 0, stored: 0, checked: 0 });
    // A week of these shows when IPOT publishes a session.
    console.log("bandar published", JSON.stringify({ session: st.session, published_at: at, last_unpublished_at: st.last_unpublished_at ?? null, probes: st.probes }));
  } else if (verdict === "closed") {
    Object.assign(st, { phase: "closed", holiday: true, next_probe_at: null });
    keep(st, { session: st.session, scanned: false, reason: "closed", probes: st.probes, first_probe_at: st.first_probe_at, last_probe_at: at, done_at: at });
  } else {
    st.last_unpublished_at = at;
    st.next_probe_at = after(now, probeGap(st, now));
    if (now.getTime() - Date.parse(st.first_probe_at) >= DAY_MS) {
      console.error("bandar scan: IPOT has not published", st.session, "since the first probe at", st.first_probe_at, "; its page may have changed");
    }
  }
  return verdict;
}

/** No stock waits for a read: none of the first pass, a retry round or IPOT's publication. */
const settled = (st) => !st.queue.length && !st.retry.length && !st.pending.length;

/** A failure's kind, for the counts the logs and the status keep. Pure, tested. */
export function errorKind(e) {
  const m = String(e?.message ?? e);
  const http = m.match(/\bHTTP (\d{3})\b/);
  if (http) return `http_${http[1]}`;
  if (/no answer within/.test(m)) return "timeout";
  if (/no table/.test(m)) return "page";
  return "other";
}

/**
 * When the stocks that wait are read again: a retry round RETRY_GAPS_MIN
 * after the first pass or the round that failed them, and the stocks IPOT
 * had not published on the probe's schedule (waitGap) after the first of
 * them went back to waiting. Nothing is due while a round of the same list
 * is under way.
 */
function schedule(st, now) {
  if (!st.queue.length && st.retry.length && !st.retry_left) st.retry_due ??= after(now, RETRY_GAPS_MIN[st.pass]);
  if (st.pending.length && !st.pending_left) st.pending_due ??= after(now, waitGap(st.session, st.published_at ?? now.toISOString(), now));
}

/**
 * The list the next stock comes from: the first pass (`queue`), else a
 * retry round or a round of re-reads of the stocks IPOT had not published,
 * under way or due; null when none is. A round reads, from the front, the
 * stocks its list held when it started (`retry_left`, `pending_left`); one
 * that has to wait again goes to the back, for the next round. The stocks
 * stay in their lists, so they count as failed or unpublished meanwhile.
 */
function nextList(st, now) {
  if (st.queue.length) return "queue";
  if (!st.retry_left && st.retry.length && st.retry_due && now.getTime() >= Date.parse(st.retry_due)) {
    Object.assign(st, { retry_left: st.retry.length, retry_due: null, pass: st.pass + 1 });
  }
  if (st.retry_left) return "retry";
  if (!st.pending_left && st.pending.length && st.pending_due && now.getTime() >= Date.parse(st.pending_due)) {
    Object.assign(st, { pending_left: st.pending.length, pending_due: null });
  }
  return st.pending_left ? "pending" : null;
}

/** Takes the stock just read off the front of its list. */
function take(st, list) {
  st[list].shift();
  if (list !== "queue") st[`${list}_left`]--;
}

/** A stock's failure: kept for the next retry round while one is left, else final. */
function failure(st, symbol, kind, e, at, seen) {
  seen[kind] = (seen[kind] || 0) + 1;
  st.last_errors = [{ symbol, kind, error: short(e), at }, ...(st.last_errors || [])].slice(0, ERRORS_KEPT);
  (st.kinds ??= {})[symbol] = kind;
  if (kind !== "invalid" && st.pass < RETRY_GAPS_MIN.length) return void st.retry.push(symbol);
  st.failed.push(symbol);
  st.errors[kind] = (st.errors[kind] || 0) + 1;
}

/**
 * Up to PER_TICK stocks into `rows`, from the lists nextList picks, while
 * the budget and the minute last. Returns how many it read, the kinds of the
 * failures among them, and whether the budget cut a stock short.
 */
async function readStocks(st, rows, { now, b, fetchPage, fetchChart }) {
  const today = hol.wibDate(now), at = now.toISOString();
  st.pending ??= [];
  st.pending_since ??= {};
  st.pass ??= 0;
  st.errors ??= {};
  st.retry_left ??= 0;
  st.pending_left ??= 0;
  const seen = {};
  let read = 0, cut = false, slowest = 0;
  while (read < PER_TICK) {
    schedule(st, now);
    const list = nextList(st, now);
    if (!list) break;
    if (b.left() < STOCK_RESERVE || b.elapsedMs() >= TICK_MS || b.elapsedMs() + slowest > DEADLINE_MS) break;
    const symbol = st[list][0];
    const started = b.elapsedMs();
    let r;
    try {
      r = await scanSymbol(symbol, st.session, st.quotes?.[symbol] ?? null, { today, fetchPage, fetchChart });
      slowest = Math.max(slowest, b.elapsedMs() - started);
    } catch (e) {
      slowest = Math.max(slowest, b.elapsedMs() - started);
      if (isBudgetError(e)) {
        // The budget ran out under it: it stays first in line, once. A stock
        // that needs more than a minute's budget (IPOT answering 404 to most
        // tries) then counts as failed, or it would start again every minute.
        cut = true;
        (st.cuts ??= {})[symbol] = (st.cuts[symbol] || 0) + 1;
        if (st.cuts[symbol] <= CUTS_ALLOWED) break;
        delete st.cuts[symbol];
        take(st, list);
        read++;
        failure(st, symbol, "budget", e, at, seen);
        continue;
      }
      take(st, list);
      read++;
      failure(st, symbol, errorKind(e), e, at, seen);
      continue;
    }
    take(st, list);
    read++;
    if (st.cuts) delete st.cuts[symbol];
    if (r.invalid) {
      failure(st, symbol, "invalid", "a code IPOT does not know", at, seen);
      continue;
    }
    if (r.pending) {
      // IPOT has not published this stock's session: read it again later, store nothing now.
      st.pending.push(symbol);
      st.pending_since[symbol] ??= at;
      continue;
    }
    delete st.pending_since[symbol];
    for (const row of r.rows) rows.push({ symbol, ...row });
    st.stored++;
    st.stored_at = at;
    if (r.rows[0]?.v.summary.buyers.length) st.checked++;
  }
  schedule(st, now);
  return { read, errors: seen, cut };
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
  // Past the minute's deadline a request throws like a spent budget: the
  // stock it cuts short stays first in line.
  const fetchFn = (u, init) => (b.elapsedMs() >= DEADLINE_MS ? Promise.reject(new BudgetError(`the minute's ${DEADLINE_MS / 1000} s for the scan are spent`)) : b.fetch(u, init));
  const fetchPage = (u) => ipot.fetchPage(u, { fetchFn, ...(pauseMs == null ? {} : { pauseMs }) });
  const fetchChart = (s, period, interval) => y.fetchChart(s, period, interval, { fetchFn });
  const session = targetSession(now), at = now.toISOString();
  const stored = await kvGet(env, STATE);
  let st = upgrade(stored);
  let changed = false, prune = false;
  // A cron minute that runs late (its `now` is when it was scheduled) after
  // a later one moved the scan on: the scan never goes back a session.
  if (st && session < st.session) return { idle: true, session: st.session };
  if (st && st.session !== session) {
    // The next session's 16:45 WIB: what this one got is filed first.
    prune = cutoff(st, at);
    st = fresh(session, st);
    changed = true;
  } else if (!st || (force && (st.phase === "done" || st.phase === "closed"))) {
    st = fresh(session, st);
  }
  if (st.phase === "done" || st.phase === "closed") return { idle: true, session };
  if (st.phase === "waiting" && !force && st.next_probe_at && now.getTime() < Date.parse(st.next_probe_at)) {
    return { idle: true, session, phase: "waiting", next_probe_at: st.next_probe_at };
  }
  const out = { session };
  if (st.phase === "new") {
    const board = await kvGet(env, "board");
    const { symbols, ranked_from } = ranking(board?.quotes || [], session);
    if (!symbols.length) return { skipped: "no quote board yet" };
    // The quotes the price moves need, so later steps do not parse the board again.
    const bySym = new Map(board.quotes.map((q) => [q.symbol, q]));
    const quotes = Object.fromEntries(symbols.map((s) => { const q = bySym.get(s); return [s, { price: q.price ?? null, previous_close: q.previous_close ?? null, market_time: q.market_time ?? null }]; }));
    Object.assign(st, { phase: "waiting", symbols, quotes, ranked_from, board_traded: boardTraded(board, session), probes: 0 });
    changed = true;
  } else if (st.phase === "waiting" && st.board_traded == null) {
    // The board was refreshed before the session ended; the one there now may tell.
    st.board_traded = boardTraded(await kvGet(env, "board"), session);
  }
  if (st.phase === "waiting") {
    out.probe = await probe(st, now, fetchPage);
    changed ||= out.probe !== "budget";
  }
  const rows = [];
  if (st.phase === "running" && stocks) {
    const { read, errors, cut } = await readStocks(st, rows, { now, b, fetchPage, fetchChart });
    out.read = read;
    if (Object.keys(errors).length) out.errors = errors;
    changed ||= read > 0 || cut;
  }
  // The first pass over: the session's record, shown from now on when it
  // holds enough, though stocks may still come from the retry rounds and
  // IPOT's late lists. It is filed again only when it says something new, so
  // a read that finds the stocks still failing or unpublished neither moves
  // its time nor prunes.
  if (st.phase === "running" && !st.queue.length) {
    if (settled(st)) st.phase = "done";
    const run = runOf(st, at), filed = (st.runs || []).find((r) => r.session === session && r.scanned);
    if (!filed || says(filed) !== says(run)) {
      keep(st, run);
      changed = prune = true;
    }
    changed ||= st.phase === "done";
  }
  if (!changed) return { ...out, idle: true, phase: st.phase };
  if (!(await save(env, st, rows, prune, stored?.rev))) {
    console.warn("bandar scan: another minute wrote the state first; this one's is dropped", JSON.stringify({ session, rev: stored?.rev ?? null }));
    return { ...out, conflict: true };
  }
  return { ...out, phase: st.phase, scanned: attempted(st), of: st.symbols.length, done: st.phase === "done" };
}

/** What the screen reads: the session shown (the newest that holds enough), else the running one. */
export async function scanRows(env, period) {
  const st = upgrade(await kvGet(env, STATE));
  const running = st && st.phase === "running" && st.session !== st.complete ? st : null;
  const session = st?.complete || running?.session || null;
  if (!session) return { session: null, state: st, rows: [] };
  const res = await env.DB.prepare("SELECT symbol, v FROM bandar_scan WHERE session = ?1 AND period = ?2").bind(session, period).all();
  return { session, state: st, partial: !st?.complete, rows: (res.results || []).map((r) => ({ symbol: r.symbol, ...JSON.parse(r.v) })) };
}

const hhmm = (iso) => new Date(Date.parse(iso) + WIB_MS).toISOString().slice(11, 16);
const stamp = (iso) => `${new Date(Date.parse(iso) + WIB_MS).toISOString().slice(0, 16).replace("T", " ")} WIB`;
/** A time in WIB, with its date when it is not `now`'s day. */
const clock = (iso, now) => (hol.wibDate(iso) === hol.wibDate(now) ? `${hhmm(iso)} WIB` : stamp(iso));
const underWay = (st) => ["new", "waiting", "running"].includes(st?.phase);

/** Trading days after `from` up to `to`. */
function sessionsAfter(from, to) {
  let n = 0;
  for (let d = hol.nextTradingDay(from); d <= to; d = hol.nextTradingDay(d)) n++;
  return n;
}

/**
 * Where the scan of `st.session` stands while it is under way, or null once
 * its record says; with `shown`, which session is shown meanwhile. Times
 * carry their date when it is not `now`'s.
 */
export function scanProgress(st, shown = null, now = new Date()) {
  const tail = shown ? `; showing ${shown}` : "";
  if (st?.phase === "new") return `the scan of ${st.session} has not started yet${tail}`;
  if (st?.phase === "waiting") {
    const n = (st.probes || 0) + (st.probe_errors || 0);
    const since = st.first_probe_at ? ` (checked ${n} time${n === 1 ? "" : "s"} since ${clock(st.first_probe_at, now)}${st.probe_error ? `; the last check failed: ${st.probe_error}` : ""})` : "";
    return `the scan of ${st.session} is waiting for IPOT to publish that session's broker lists${since}${tail}`;
  }
  if (st?.phase === "running" && st.queue.length) return `the scan of ${st.session} is running (${attempted(st)} of ${st.symbols.length})${tail}`;
  if (st?.phase === "running") {
    // The first pass is over; failed and unpublished stocks wait to be read again.
    return `the scan of ${st.session} has read ${st.stored} of ${st.symbols.length} stocks so far (${st.failed.length + st.retry.length} failed, ${st.pending.length} not published by IPOT) and reads ${st.retry.length + st.pending.length} of them again later${tail}`;
  }
  return null;
}

/** Why a finished session newer than the one shown is not the one shown. */
function runNote(r, shown) {
  if (r.reason === "closed") return `${r.session} had no trades at IPOT nor on the quote board, so it was not scanned`;
  if (r.reason === "unreachable") {
    const n = r.probe_errors || 0;
    return n ? `IPOT did not answer the scan's ${n} check${n === 1 ? "" : "s"} of ${r.session} (the last failed: ${r.probe_error}); showing ${shown}` : `the scan did not get to check ${r.session} with IPOT; showing ${shown}`;
  }
  if (!r.scanned) return `IPOT had not published the broker lists of ${r.session} by ${stamp(r.last_probe_at ?? r.done_at)}; showing ${shown}`;
  return `the scan of ${r.session} read only ${r.stored} of ${r.of} stocks (${r.failed.length} failed, ${r.unpublished.length} not published by IPOT); showing ${shown}`;
}

/**
 * The screen's and the broker's notes on the scan, as of `now`: which
 * session they show when the newest trading session is not it, and why
 * (waiting for IPOT, running, not published by the cutoff, too few stocks,
 * not started), how far behind it is, and what its own scan missed. `shown`
 * is the session the rows come from, `rows` the symbols they hold. Each of
 * the scan's stocks counts once: by its row when it has one (a stock a
 * forced rescan failed keeps its row from the first scan), else by why it
 * has none. Returns the warnings, the failed and unpublished counts, the
 * stocks scanned and when the shown session was read. Pure, tested through
 * the API.
 */
export function scanNotes(st, shown, rows, now = new Date()) {
  const warnings = [];
  const latest = hol.latestSession(now);
  const runs = st?.runs || [];
  const run = runs.find((r) => r.session === shown && r.scanned) ?? null;
  const held = new Set(rows), missing = (list) => (list || []).filter((s) => !held.has(s));
  const live = st?.session === shown && underWay(st);
  if (st && latest > st.session) {
    warnings.push(hol.wibDate(now) === latest && wibMinutes(now) < READY_MIN ? `the scan of ${latest} starts at 16:45 WIB; showing ${shown}` : `the scan has not reached ${latest} yet; showing ${shown}`);
  }
  if (st && st.session > shown && underWay(st)) warnings.push(scanProgress(st, shown, now));
  for (const r of runs) if (r.session > shown && !(underWay(st) && r.session === st.session)) warnings.push(runNote(r, shown));
  const behind = sessionsAfter(shown, latest) - runs.filter((r) => r.reason === "closed" && r.session > shown && r.session <= latest).length;
  if (behind > 1) warnings.push(`${shown} is ${behind} sessions behind ${latest}`);
  // While the shown session's scan reads, its numbers come from the state,
  // not from a record filed before stocks went back to be read again.
  const reading = live && st.phase === "running";
  const rec = reading ? runOf(st, null) : run;
  const failed = missing(rec?.failed).length;
  const unpublished = missing(rec?.unpublished).length;
  const scanned = held.size + failed + unpublished;
  if (reading && st.queue.length) warnings.push(`the scan of ${shown} is still running: ${held.size} of ${st.symbols.length} stocks so far`);
  if (reading && unpublished) warnings.push(`IPOT has not published ${shown} for ${unpublished} of the ${st.symbols.length} stocks yet; the scan ${st.pending_due ? `reads them again at ${clock(st.pending_due, now)}` : "is reading them again"}`);
  const retrying = reading ? missing(st.retry).length : 0;
  if (failed) warnings.push(`${failed} of ${scanned} scanned stocks had no broker summary${retrying ? `; the scan tries ${retrying} of them again ${st.retry_due ? `at ${clock(st.retry_due, now)}` : "now"}` : ""}`);
  if (!reading && unpublished) warnings.push(`IPOT had not published ${shown} for ${unpublished} of the ${scanned} scanned stocks when its scan ended`);
  if (!reading && rec?.left) warnings.push(`the scan of ${shown} ended before it read ${rec.left} of its ${rec.of} stocks`);
  if (rec?.ranked_from && rec.ranked_from !== shown) warnings.push(`the scan of ${shown} picked its ${rec.of} stocks from the quote board of ${rec.ranked_from}, not from that session's close`);
  const year = Number(latest.slice(0, 4));
  if (!hol.covers(year)) warnings.push(`the bundled holiday list does not cover ${year}; the scan tells that year's closures from IPOT and the quote board`);
  // When the shown session's scan last stored a stock; before it stored one,
  // when it started; for a session kept from before these records, the
  // earliest its scan could have read it.
  const fetched_at = rec?.stored_at || run?.done_at || (st?.session === shown ? st.started_at : null) || new Date(Date.parse(`${shown}T00:00:00+07:00`) + READY_MIN * 60e3).toISOString();
  return { warnings, failed, unpublished, scanned, fetched_at };
}

export async function scanStatus(env) {
  const st = upgrade(await kvGet(env, STATE));
  if (!st) return { session: null, complete: null, running: false };
  return {
    session: st.session, complete: st.complete, running: ["new", "waiting", "running"].includes(st.phase), holiday: st.phase === "closed",
    scanned: attempted(st), of: (st.symbols || []).length, with_trades: st.checked || 0,
    failed: st.failed || [], started_at: st.started_at || null, done_at: st.done_at || null,
    phase: st.phase, probes: st.probes || 0, probe_errors: st.probe_errors || 0, probe_error: st.probe_error ?? null, probe_failing_since: st.failing_since ?? null,
    first_probe_at: st.first_probe_at ?? null, next_probe_at: st.next_probe_at ?? null, published_at: st.published_at ?? null,
    pending: (st.pending || []).map((symbol) => ({ symbol, since: st.pending_since?.[symbol] ?? null })), pending_due: st.pending_due ?? null,
    retry: st.retry || [], retry_due: st.retry_due ?? null, pass: st.pass ?? 0, errors: st.errors || {}, last_errors: st.last_errors || [],
    probe_symbol: st.probe_symbol ?? null, ranked_from: st.ranked_from ?? null, last: st.runs?.[0] ?? null,
  };
}
