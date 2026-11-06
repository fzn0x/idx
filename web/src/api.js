// JSON API behind the web terminal. Every response is the CLI envelope:
// { schema, provider, fetched_at, cached, stale, warnings?, data } and errors
// are { error: { code, message }, exit_code } with a matching HTTP status.

import * as y from "./yahoo.js";
import * as an from "./analysis.js";
import * as ex from "./extras.js";
import * as ipot from "./ipot.js";
import * as bd from "./bandar.js";
import * as hol from "./holidays.js";
import * as astroLib from "./astro.js";
import { fetchWithin } from "./net.js";
import { scanRows, scanStatus } from "./bandar-scan.js";
import { syncStatus } from "./ownership-sync.js";
import UNIVERSE from "./universe.json" with { type: "json" };
import BROKERS from "./brokers.json" with { type: "json" };

const NAME_BY_SYMBOL = new Map(UNIVERSE.map(([s, n]) => [s, n]));
const SYMBOLS = UNIVERSE.map(([s]) => s);

export class ApiError extends Error {
  constructor(code, message, status = 400, exit = 2) { super(message); this.code = code; this.status = status; this.exit = exit; }
}
const notFound = (m) => new ApiError("not_found", m, 404, 4);
const usage = (m) => new ApiError("usage", m, 400, 2);
const provider = (m) => new ApiError("provider", m, 502, 3);

export function envelope(schema, data, { provider: p = "yahoo", fetched_at = new Date(), cached = false, stale = false, warnings = [] } = {}) {
  const at = fetched_at instanceof Date ? fetched_at : new Date(fetched_at * 1000);
  const env = { schema, provider: p, fetched_at: at.toISOString().replace(/\.\d{3}Z$/, "Z"), cached, stale };
  if (warnings.length) env.warnings = warnings;
  env.data = data;
  return env;
}

export function parseSymbol(raw) {
  const s = String(raw || "").trim().toUpperCase().replace(/\.JK$/, "");
  if (!s) throw usage("empty symbol");
  // Index codes keep their caret (^JKSE is IHSG), like the CLI's Symbol::parse.
  const body = s.startsWith("^") ? s.slice(1) : s;
  if (!body || body.length > 12 || !/^[A-Z0-9-]+$/.test(body)) throw usage(`invalid symbol "${raw}": expected an IDX ticker like BBCA`);
  return s;
}
export function parseSymbols(raw) {
  const out = [];
  for (const p of String(raw || "").split(/[,\s]+/)) if (p) { const s = parseSymbol(p); if (!out.includes(s)) out.push(s); }
  if (!out.length) throw usage("no symbols given");
  return out;
}

// ---- D1 cache ----------------------------------------------------------

// With `retryAfter`, a failure is remembered for that many seconds (row
// `fail:<key>`), so readers get the stale copy or the error at once instead
// of each waiting on a source that is down.
async function cached(env, key, ttlSecs, producer, { retryAfter = 0 } = {}) {
  const now = Math.floor(Date.now() / 1000);
  const row = await env.DB.prepare("SELECT v, fetched_at, expires_at FROM cache WHERE k = ?1").bind(key).first();
  if (row && row.expires_at > now) return { value: JSON.parse(row.v), cached: true, stale: false, fetched_at: row.fetched_at };
  const staleCopy = (why) => ({ value: JSON.parse(row.v), cached: true, stale: true, fetched_at: row.fetched_at, warning: `live fetch failed (${why}); showing cached copy` });
  if (retryAfter) {
    const fail = await env.DB.prepare("SELECT v, fetched_at, expires_at FROM cache WHERE k = ?1").bind(`fail:${key}`).first();
    if (fail && fail.expires_at > now) {
      const why = `${JSON.parse(fail.v)}; next try in ${fail.expires_at - now}s`;
      if (row) return staleCopy(why);
      throw new Error(why);
    }
  }
  try {
    const value = await producer();
    await env.DB.prepare("INSERT OR REPLACE INTO cache (k, v, expires_at, fetched_at) VALUES (?1, ?2, ?3, ?4)")
      .bind(key, JSON.stringify(value), now + ttlSecs, now).run();
    return { value, cached: false, stale: false, fetched_at: now };
  } catch (e) {
    if (retryAfter) {
      await env.DB.prepare("INSERT OR REPLACE INTO cache (k, v, expires_at, fetched_at) VALUES (?1, ?2, ?3, ?4)")
        .bind(`fail:${key}`, JSON.stringify(e.message), now + retryAfter, now).run();
    }
    if (row) return staleCopy(e.message);
    throw e;
  }
}
async function kvGet(env, k) {
  const row = await env.DB.prepare("SELECT v, updated_at FROM kv WHERE k = ?1").bind(k).first();
  return row ? { value: JSON.parse(row.v), updated_at: row.updated_at } : null;
}
export async function kvPut(env, k, value) {
  await env.DB.prepare("INSERT OR REPLACE INTO kv (k, v, updated_at) VALUES (?1, ?2, ?3)")
    .bind(k, JSON.stringify(value), Math.floor(Date.now() / 1000)).run();
}

// ---- snapshot (cron) ---------------------------------------------------

const WIB_OFFSET_MIN = 7 * 60;
export function marketIsOpen(now = new Date()) {
  const wib = new Date(now.getTime() + WIB_OFFSET_MIN * 60000);
  const day = wib.getUTCDay();
  if (day === 0 || day === 6) return false;
  const mins = wib.getUTCHours() * 60 + wib.getUTCMinutes();
  return mins >= 8 * 60 + 45 && mins <= 16 * 60 + 30;
}

/** Refreshes the quote board (one JSON row) and the index tape. */
export async function refreshSnapshot(env, { force = false } = {}) {
  const now = new Date();
  const minute = Math.floor(now.getTime() / 60000);
  if (!force && !marketIsOpen(now) && minute % 15 !== 0) return { skipped: "market closed" };
  const quotes = [];
  const warnings = [];
  for (let i = 0; i < SYMBOLS.length; i += 190) {
    const batch = SYMBOLS.slice(i, i + 190);
    try {
      for (const q of await y.fetchQuotes(env, batch)) quotes.push(y.parseV7Quote(q));
    } catch (e) {
      warnings.push(`batch ${i / 190 + 1}: ${e.message}`);
    }
  }
  let index = null;
  try {
    const r = await y.fetchQuotes(env, ["^JKSE"]);
    if (r[0]) index = y.parseV7Quote(r[0]);
  } catch (e) {
    warnings.push(`index: ${e.message}`);
  }
  if (!quotes.length) throw provider(`snapshot refresh failed: ${warnings.join("; ")}`);
  const ts = Math.floor(now.getTime() / 1000);
  await kvPut(env, "board", { ts, quotes, index, warnings });
  if (index?.price != null) {
    await env.DB.prepare("INSERT OR REPLACE INTO ticks (symbol, ts, price) VALUES ('^JKSE', ?1, ?2)").bind(minute * 60, index.price).run();
    await env.DB.prepare("DELETE FROM ticks WHERE ts < ?1").bind(ts - 10 * 86400).run();
  }
  return { quotes: quotes.length, index: !!index, warnings };
}

async function board(env) {
  const b = await kvGet(env, "board");
  if (!b) throw provider("no market snapshot yet; the minute refresh has not run");
  return b;
}

// ---- endpoints ---------------------------------------------------------

export async function market(env) {
  const b = await board(env);
  const quotes = b.value.quotes.filter((q) => q.price != null && q.change_pct != null);
  const byChg = [...quotes].sort((a, c) => c.change_pct - a.change_pct);
  const liquid = quotes.filter((q) => (q.volume || 0) > 0 && (q.market_cap || 0) > 1e12);
  const liquidByChg = [...liquid].sort((a, c) => c.change_pct - a.change_pct);
  const byValue = [...quotes].sort((a, c) => ((c.volume || 0) * (c.price || 0)) - ((a.volume || 0) * (a.price || 0)));
  const ticks = await env.DB.prepare("SELECT ts, price FROM ticks WHERE symbol = '^JKSE' AND ts >= ?1 ORDER BY ts")
    .bind(Math.floor(Date.now() / 1000) - 2 * 86400).all();
  const trim = (q) => ({ symbol: q.symbol, name: q.name, price: q.price, change: q.change, change_pct: q.change_pct, volume: q.volume, value: q.price != null && q.volume != null ? q.price * q.volume : null, market_cap: q.market_cap });
  const data = {
    as_of: b.value.quotes.map((q) => q.market_time).filter(Boolean).sort().pop() || null,
    refreshed_at: new Date(b.value.ts * 1000).toISOString().replace(/\.\d{3}Z$/, "Z"),
    market_open: marketIsOpen(),
    index: b.value.index ? trim(b.value.index) : null,
    index_tape: (ticks.results || []).map((t) => [t.ts, t.price]),
    quoted: quotes.length,
    advancing: quotes.filter((q) => q.change_pct > 0).length,
    declining: quotes.filter((q) => q.change_pct < 0).length,
    unchanged: quotes.filter((q) => q.change_pct === 0).length,
    turnover: quotes.reduce((s, q) => s + (q.volume || 0) * (q.price || 0), 0),
    market_state: b.value.index?.market_state || null,
    liquid_min_market_cap: 1e12,
    gainers: liquidByChg.slice(0, 10).map(trim),
    losers: liquidByChg.slice(-10).reverse().map(trim),
    most_active: byValue.slice(0, 10).map(trim),
    heavyweights: [...quotes].sort((a, c) => (c.market_cap || 0) - (a.market_cap || 0)).slice(0, 12).map(trim),
    universe: SYMBOLS.length,
  };
  return envelope("idx.market.v1", data, { fetched_at: b.value.ts, cached: true, warnings: b.value.warnings || [] });
}

export async function quote(env, symbolsRaw) {
  const symbols = parseSymbols(symbolsRaw);
  const b = await board(env);
  const bySym = new Map(b.value.quotes.map((q) => [q.symbol, q]));
  const found = symbols.map((s) => bySym.get(s)).filter(Boolean);
  const warnings = symbols.filter((s) => !bySym.has(s)).map((s) => `no data for ${s}`);
  if (!found.length) {
    // Not in the universe: try live once, cached briefly.
    const live = await cached(env, `quote:${symbols.join(",")}`, 60, async () => (await y.fetchQuotes(env, symbols)).map(y.parseV7Quote));
    if (!live.value.length) throw notFound(`no quotes for ${symbols.join(", ")}`);
    return envelope("idx.quotes.v1", live.value, { cached: live.cached, stale: live.stale, fetched_at: live.fetched_at, warnings: symbols.filter((s) => !live.value.some((q) => q.symbol === s)).map((s) => `no data for ${s}`) });
  }
  return envelope("idx.quotes.v1", found, { cached: true, fetched_at: b.value.ts, warnings });
}

const PERIODS = ["1d","5d","1mo","3mo","6mo","1y","2y","5y","10y","ytd","max"];
const INTERVALS = ["5m","15m","30m","1h","1d","1wk","1mo"];

/** A D1 row holds at most 2,000,000 bytes; the margin covers the key, the other columns and a non-ASCII name. */
const CACHE_ROW_BUDGET = 1_900_000;
/** More than any candle's JSON takes (91 bytes of keys and time, six numbers), so shorter histories skip the measuring. */
const CANDLE_BYTES_MAX = 256;

/** Keeps the most recent candles of a parsed history that fit one D1 cache row (all daily bars of an old
 * index since 1970 do not); its summary still covers every candle Yahoo returned. */
export function fitCacheRow(h, budget = CACHE_ROW_BUDGET) {
  if (h.candles.length * CANDLE_BYTES_MAX < budget) return h;
  let size = JSON.stringify(h).length, keep = h.candles.length, out = h;
  while (size > budget && keep > 0) {
    keep = Math.floor((keep * budget * 0.98) / size);
    out = { ...h, candles: h.candles.slice(-keep) };
    size = JSON.stringify(out).length;
  }
  return out;
}

export async function history(env, symbolRaw, period = "1y", interval = "1d", limit = 0) {
  const symbol = parseSymbol(symbolRaw);
  if (!PERIODS.includes(period)) throw usage(`invalid period "${period}"; use ${PERIODS.join(",")}`);
  if (!INTERVALS.includes(interval)) throw usage(`invalid interval "${interval}"; use ${INTERVALS.join(",")}`);
  const intraday = ["5m","15m","30m","1h"].includes(interval);
  // Keyed on the request, so a row written for an older URL (0.8.1 asked range=max and got monthly bars
  // labelled 1d) is never served, not even as a stale copy.
  const r = await cached(env, `history:${y.chartUrl(symbol, period, interval)}`, intraday ? 120 : 3600, async () => {
    const res = await y.fetchChart(symbol, period, interval);
    if (res.notFound) return { notFound: res.message };
    return fitCacheRow(y.parseChart(res.body, period, interval));
  });
  if (r.value.notFound) throw notFound(`${symbol}: ${r.value.notFound}`);
  const data = { ...r.value };
  const warnings = r.warning ? [r.warning] : [];
  if (data.interval !== interval) warnings.push(`Yahoo returned ${data.interval} bars for ${period}/${interval}; the candles are labelled ${data.interval}`);
  if (data.summary && data.summary.candles > data.candles.length) warnings.push(`the web terminal keeps the last ${data.candles.length} of ${data.summary.candles} candles (one cache row); summary covers all of them; the CLI returns every candle`);
  if (limit > 0 && data.candles.length > limit) {
    data.candles = data.candles.slice(-limit);
    warnings.push(`candles truncated to the last ${limit}; summary covers the full period`);
  }
  return envelope("idx.history.v1", data, { cached: r.cached, stale: r.stale, fetched_at: r.fetched_at, warnings });
}

async function summary(env, symbol, modules, ttl = 6 * 3600) {
  const r = await cached(env, `summary:${symbol}:${modules}`, ttl, async () => {
    const res = await y.fetchSummary(env, symbol, modules);
    if (res.notFound) return { notFound: res.message };
    return res.body;
  });
  if (r.value?.notFound) throw notFound(`${symbol}: ${r.value.notFound}`);
  return r;
}

export async function profile(env, symbolRaw) {
  const symbol = parseSymbol(symbolRaw);
  const r = await summary(env, symbol, y.FUNDAMENTAL_MODULES);
  const data = y.parseProfile(r.value, symbol);
  if (!data.name) data.name = NAME_BY_SYMBOL.get(symbol) || null;
  return envelope("idx.profile.v1", data, { cached: r.cached, stale: r.stale, fetched_at: r.fetched_at, warnings: r.warning ? [r.warning] : [] });
}

export async function fundamentals(env, symbolRaw) {
  const symbol = parseSymbol(symbolRaw);
  const r = await summary(env, symbol, y.FUNDAMENTAL_MODULES);
  const data = y.parseFundamentals(r.value, symbol);
  data.verdict = an.verdict(data);
  return envelope("idx.fundamentals.v1", data, { cached: r.cached, stale: r.stale, fetched_at: r.fetched_at, warnings: r.warning ? [r.warning] : [] });
}

export async function earnings(env, symbolRaw) {
  const symbol = parseSymbol(symbolRaw);
  const r = await summary(env, symbol, y.EARNINGS_MODULES);
  return envelope("idx.earnings.v1", y.parseEarnings(r.value, symbol), { cached: r.cached, stale: r.stale, fetched_at: r.fetched_at, warnings: r.warning ? [r.warning] : [] });
}

export async function financials(env, symbolRaw, period = "annual") {
  const symbol = parseSymbol(symbolRaw);
  if (!["annual","quarterly","ttm"].includes(period)) throw usage(`invalid period "${period}"; use annual, quarterly or ttm`);
  const r = await cached(env, `financials:${symbol}:${period}`, 24 * 3600, async () => y.parseTimeseries(await y.fetchTimeseries(symbol, period), symbol, period));
  if (r.value.notFound) throw notFound(r.value.message);
  return envelope("idx.financials.v1", r.value, { cached: r.cached, stale: r.stale, fetched_at: r.fetched_at, warnings: r.warning ? [r.warning] : [] });
}

export async function technical(env, symbolRaw, period = "1y") {
  const h = await history(env, symbolRaw, period, "1d", 0);
  const t = an.technical(h.data);
  if (!t) throw notFound(`no daily candles for ${parseSymbol(symbolRaw)}`);
  return envelope("idx.technical.v1", t, { cached: h.cached, stale: h.stale, fetched_at: Date.parse(h.fetched_at) / 1000, warnings: h.warnings || [] });
}

export async function dividends(env, symbolRaw) {
  const symbol = parseSymbol(symbolRaw);
  const r = await cached(env, `dividends:${symbol}`, 6 * 3600, async () => {
    const res = await y.fetchEvents(symbol);
    if (res.notFound) return { notFound: res.message };
    return y.parseEvents(res.body);
  });
  if (r.value.notFound) throw notFound(`${symbol}: ${r.value.notFound}`);
  const today = y.wibIso(Math.floor(Date.now() / 1000)).slice(0, 10);
  const data = ex.enrichDividends(structuredClone(r.value), today);
  const warnings = r.warning ? [r.warning] : [];
  if (!data.dividends.length) warnings.push(`${symbol}: the source lists no dividend events`);
  return envelope("idx.dividends.v1", data, { cached: r.cached, stale: r.stale, fetched_at: r.fetched_at, warnings });
}

export async function returns(env, symbolsRaw, benchmark = "^JKSE") {
  const symbols = parseSymbols(symbolsRaw);
  const bench = benchmark && benchmark.toLowerCase() !== "none" ? parseSymbol(benchmark) : null;
  const fetchOne = (s) => history(env, s, "5y", "1d", 0).then((e) => ({ ok: e })).catch((e) => ({ err: e }));
  const [results, b] = await Promise.all([Promise.all(symbols.map(fetchOne)), bench ? fetchOne(bench) : null]);
  const warnings = [];
  let benchPerf = null;
  if (bench) {
    if (b.ok) { benchPerf = ex.performance(b.ok.data); if (!benchPerf) warnings.push(`benchmark ${bench}: no candles`); }
    else warnings.push(`benchmark ${bench} unavailable: ${b.err.message}`);
  }
  const data = [];
  let cachedAll = true, stale = false, fetched = Math.floor(Date.now() / 1000), firstErr = null;
  results.forEach((r, i) => {
    if (r.err) { warnings.push(r.err.code === "not_found" ? `no data for ${symbols[i]}` : `${symbols[i]}: ${r.err.message}`); firstErr ||= r.err; return; }
    const p = ex.performance(r.ok.data);
    if (!p) { warnings.push(`${symbols[i]}: no candles`); return; }
    if (benchPerf) p.benchmark = { symbol: benchPerf.symbol, name: benchPerf.name, returns: benchPerf.returns, excess: ex.minusWindows(p.returns, benchPerf.returns) };
    cachedAll &&= r.ok.cached; stale ||= r.ok.stale; fetched = Math.min(fetched, Date.parse(r.ok.fetched_at) / 1000);
    data.push(p);
  });
  if (!data.length) throw firstErr || notFound("no data");
  return envelope("idx.returns.v1", data, { cached: cachedAll, stale, fetched_at: fetched, warnings });
}

export async function compare(env, symbolsRaw) {
  const symbols = parseSymbols(symbolsRaw);
  if (symbols.length < 2) throw usage("compare needs at least two different symbols");
  const results = await Promise.all(symbols.map((s) => fundamentals(env, s).then((e) => ({ ok: e })).catch((e) => ({ err: e, s }))));
  const data = [], warnings = [];
  let cachedAll = true, stale = false, fetched = Math.floor(Date.now() / 1000);
  for (const r of results) {
    if (r.ok) { data.push(r.ok.data); cachedAll &&= r.ok.cached; stale ||= r.ok.stale; fetched = Math.min(fetched, Date.parse(r.ok.fetched_at) / 1000); }
    else warnings.push(r.err.code === "not_found" ? `no data for ${r.s}` : `${r.s}: ${r.err.message}`);
  }
  if (!data.length) throw results[0].err;
  return envelope("idx.compare.v1", data, { cached: cachedAll, stale, fetched_at: fetched, warnings });
}

export async function screen(env, filterSpec, sort = "mcap", limit = 25, maxFundamentals = 20, descending = true) {
  const filters = an.parseFilters(filterSpec || "");
  const sortField = an.sortField(sort);
  if (!sortField) throw usage(`unknown sort key "${sort}"; use one of ${an.KEY_NAMES}`);
  const b = await board(env);
  let rows = b.value.quotes.map((q) => ({
    symbol: q.symbol, name: q.name || NAME_BY_SYMBOL.get(q.symbol) || null, price: q.price, change_pct: q.change_pct, market_cap: q.market_cap,
    pe_trailing: q.pe_trailing, pe_forward: q.pe_forward, price_to_book: q.price_to_book, dividend_yield_pct: q.dividend_yield_pct,
    eps_ttm: q.eps_ttm, avg_volume_3m: q.avg_volume_3m, week52_high: q.week52_high, week52_low: q.week52_low, fundamentals_checked: false,
    sector: null, industry: null, roe_pct: null, roa_pct: null, net_margin_pct: null, operating_margin_pct: null, debt_to_equity: null,
    revenue_growth_pct: null, earnings_growth_pct: null, peg: null, price_to_sales: null,
  }));
  const quoted = rows.length;
  const quoteFilters = filters.filter((f) => !an.FUNDAMENTAL_FIELDS.has(f.field));
  const fundFilters = filters.filter((f) => an.FUNDAMENTAL_FIELDS.has(f.field));
  rows = rows.filter((r) => an.matches(r, quoteFilters));
  const matchedQuote = rows.length;
  let checked = 0, capped = false;
  const warnings = [];
  if (fundFilters.length || an.FUNDAMENTAL_FIELDS.has(sortField)) {
    an.sortRows(rows, "market_cap", true);
    const cap = Math.max(1, Math.min(maxFundamentals, 40));
    if (rows.length > cap) { capped = true; rows = rows.slice(0, cap); }
    const results = await Promise.all(rows.map((r) => fundamentals(env, r.symbol).then((e) => e.data).catch(() => null)));
    rows.forEach((r, i) => {
      const f = results[i];
      if (!f) return;
      checked++;
      Object.assign(r, {
        fundamentals_checked: true, sector: f.sector, industry: f.industry, roe_pct: f.profitability.roe_pct, roa_pct: f.profitability.roa_pct,
        net_margin_pct: f.profitability.net_margin_pct, operating_margin_pct: f.profitability.operating_margin_pct, debt_to_equity: f.health.debt_to_equity,
        revenue_growth_pct: f.growth.revenue_growth_pct, earnings_growth_pct: f.growth.earnings_growth_pct, peg: f.valuation.peg, price_to_sales: f.valuation.price_to_sales,
      });
      r.pe_trailing ??= f.valuation.pe_trailing; r.price_to_book ??= f.valuation.price_to_book; r.dividend_yield_pct ??= f.dividend.yield_pct;
    });
    rows = rows.filter((r) => an.matches(r, fundFilters));
    if (capped) warnings.push(`fundamentals were fetched for the ${cap} largest companies that passed the quote filters; the web terminal caps this at 40`);
  }
  an.sortRows(rows, sortField, descending);
  const total = rows.length;
  if (limit > 0) rows = rows.slice(0, limit);
  const data = { filters: filters.map((f) => f.raw), sort: sort.toLowerCase(), descending, universe: SYMBOLS.length, quoted, matched_quote_level: matchedQuote, fundamentals_checked: checked, fundamentals_capped: capped, total_matches: total, rows };
  return envelope("idx.screen.v1", data, { cached: true, fetched_at: b.value.ts, warnings });
}

// ---- news ---------------------------------------------------------------

const FEEDS = [
  { id: "kontan/investasi", url: "https://investasi.kontan.co.id/rss" },
  { id: "kontan/keuangan", url: "https://keuangan.kontan.co.id/rss" },
  { id: "kontan/industri", url: "https://industri.kontan.co.id/rss" },
  { id: "cnbcindonesia/market", url: "https://www.cnbcindonesia.com/market/rss" },
  { id: "cnbcindonesia/research", url: "https://www.cnbcindonesia.com/research/rss" },
];
function stripHtml(s) {
  return String(s || "").replace(/<[^>]*>/g, " ").replace(/&nbsp;/g, " ").replace(/&amp;/g, "&").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/\s+/g, " ").trim();
}
function cdata(s) { const m = String(s || "").match(/^\s*<!\[CDATA\[([\s\S]*?)\]\]>\s*$/); return m ? m[1] : s; }
export function parseRss(xml, source) {
  const items = [];
  const re = /<item>([\s\S]*?)<\/item>/g;
  let m;
  while ((m = re.exec(xml))) {
    const it = m[1];
    const tag = (n) => { const t = it.match(new RegExp(`<${n}(?:\\s[^>]*)?>([\\s\\S]*?)</${n}>`)); return t ? cdata(t[1]).trim() : ""; };
    const title = stripHtml(tag("title")), url = tag("link").trim();
    if (!title || !url) continue;
    const pub = tag("pubDate");
    const d = pub ? new Date(pub) : null;
    items.push({ title, url, source, published: d && !isNaN(d) ? y.wibIso(Math.floor(d.getTime() / 1000)) : null, summary: stripHtml(tag("description")), matched_by: "all", scope: "market", sentiment: 0, positive_terms: [], negative_terms: [] });
  }
  return items;
}
export function matchName(raw) {
  return String(raw || "").split(/\s+/).map((t) => t.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "")).filter((t) => t && !["PT","TBK","PERSERO","LTD","INC"].includes(t.toUpperCase())).join(" ");
}
function itemMatches(item, ticker, name) {
  const text = `${item.title} ${item.summary}`.toUpperCase();
  if (text.split(/[^A-Z0-9]+/).includes(ticker)) return "ticker";
  if (name && name.length >= 4 && text.includes(name.toUpperCase())) return "name";
  return null;
}
// A feed can accept the connection and never answer (Kontan, Oct 2026): one
// silent source must not hold the request, nor make every reader wait.
const FEED_TIMEOUT_MS = 8000;
const FEED_RETRY_SECS = 5 * 60;
async function fetchFeed(f) {
  const r = await fetchWithin(f.url, { headers: { "User-Agent": "Mozilla/5.0 (idx web terminal)", Accept: "application/rss+xml, application/xml;q=0.9, */*;q=0.8" } }, { ms: FEED_TIMEOUT_MS });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return parseRss(await r.text(), f.id);
}
export async function news(env, symbolRaw, limit = 20, companyOnly = false) {
  const symbol = symbolRaw ? parseSymbol(symbolRaw) : null;
  const results = await Promise.all(FEEDS.map((f) => cached(env, `feed:${f.id}`, 15 * 60, () => fetchFeed(f), { retryAfter: FEED_RETRY_SECS })
    .then((r) => ({ f, r })).catch((e) => ({ f, e }))));
  let items = [], cachedAll = true, stale = false;
  const warnings = [];
  let okCount = 0, fetched = Math.floor(Date.now() / 1000);
  for (const x of results) {
    if (x.e) { warnings.push(`${x.f.id}: ${x.e.message}`); continue; }
    if (x.r.warning) warnings.push(`${x.f.id}: ${x.r.warning}`);
    okCount++; cachedAll &&= x.r.cached; stale ||= x.r.stale; fetched = Math.min(fetched, x.r.fetched_at);
    items.push(...x.r.value);
  }
  if (!okCount) throw provider(`no news feed could be read: ${warnings.join("; ")}`);
  items.sort((a, b) => String(b.published).localeCompare(String(a.published)));
  const seen = new Set(); items = items.filter((i) => !seen.has(i.url) && seen.add(i.url));
  const scanned = items.length;
  const matchedName = symbol ? matchName(NAME_BY_SYMBOL.get(symbol) || "") || null : null;
  const arranged = ex.arrangeNews(items, symbol, matchedName, limit, companyOnly, itemMatches);
  items = arranged.items;
  for (const it of items) { const s = an.scoreText(`${it.title} ${it.summary}`); it.sentiment = s.score; it.positive_terms = s.pos; it.negative_terms = s.neg; }
  const all = items;
  const data = { symbol, matched_name: matchedName, sources: FEEDS.map((f) => f.id), scanned, company_matches: arranged.companyMatches, company_only: !!symbol && companyOnly, items };
  return { env: envelope("idx.news.v1", data, { provider: "news", cached: cachedAll, stale, fetched_at: fetched, warnings }), all, matchedName };
}
export async function sentiment(env, symbolRaw) {
  const symbol = parseSymbol(symbolRaw);
  const n = await news(env, symbol, 0, true);
  const rep = an.aggregateSentiment(symbol, n.matchedName, n.all, 10);
  return envelope("idx.sentiment.v1", rep, { provider: "news", cached: n.env.cached, stale: n.env.stale, fetched_at: Date.parse(n.env.fetched_at) / 1000, warnings: n.env.warnings || [] });
}

// ---- broker summary (IPOT) ---------------------------------------------

const BROKER_MAX_SYMBOLS = 10;
const BROKER_LOOKBACK_DAYS = 10;
const wibToday = () => new Date(Date.now() + 7 * 3600e3).toISOString().slice(0, 10);
const addDays = (iso, n) => new Date(Date.parse(`${iso}T00:00:00Z`) + n * 86400e3).toISOString().slice(0, 10);
function isoDate(raw, what) {
  if (raw == null || raw === "") return null;
  const s = String(raw).trim();
  const ms = Date.parse(`${s}T00:00:00Z`);
  // Date.parse rolls 2026-02-30 over to 2 March; only a real date survives the round trip.
  if (!/^\d{4}-\d{2}-\d{2}$/.test(s) || Number.isNaN(ms) || new Date(ms).toISOString().slice(0, 10) !== s) throw usage(`invalid ${what} "${raw}"; use YYYY-MM-DD`);
  return s;
}

async function brokerRange(env, symbol, from, to, investor, board, today) {
  // A range that ended before today no longer changes; today's moves with every trade.
  const ttl = to < today ? 7 * 86400 : 5 * 60;
  const r = await cached(env, `ipot:${symbol}:${from}:${to}:${investor}:${board}`, ttl, async () => {
    let html;
    try { html = await ipot.fetchPage(ipot.url(symbol, from, to, investor, board)); } catch (e) { throw provider(`ipot: broker summary for ${symbol}: ${e.message}`); }
    const p = ipot.parse(html);
    if (p.invalid) throw notFound(`${symbol} is not a stock code IPOT knows`);
    return p;
  });
  const p = r.value;
  const flow = (f) => ({ ...f, name: BROKERS[f.code] ?? null });
  const data = {
    symbol, name: NAME_BY_SYMBOL.get(symbol) || null, from: p.from || from, to: p.to || to, investor, board,
    buyers: p.buyers.map(flow), sellers: p.sellers.map(flow),
    total_value: p.total_value, total_lots: p.total_lots, foreign_net_value: p.foreign_net_value, average_price: p.average_price, rounded: p.rounded,
  };
  return { data, cached: r.cached, stale: r.stale, fetched_at: r.fetched_at, warnings: r.warning ? [r.warning] : [] };
}

async function brokerLatest(env, symbol, investor, board, today) {
  let last = null;
  for (let back = 0; back < BROKER_LOOKBACK_DAYS; back++) {
    const day = addDays(today, -back);
    const dow = new Date(`${day}T00:00:00Z`).getUTCDay();
    if (dow === 0 || dow === 6) continue;
    const s = await brokerRange(env, symbol, day, day, investor, board, today);
    if (s.data.buyers.length || s.data.sellers.length) return s;
    last = s;
  }
  last.warnings.push(`${symbol}: no trades in the ${BROKER_LOOKBACK_DAYS} days to ${today}`);
  return last;
}

export async function broker(env, symbolsRaw, dateRaw, fromRaw, toRaw, investor = "all", board = "all") {
  const symbols = parseSymbols(symbolsRaw);
  if (symbols.length > BROKER_MAX_SYMBOLS) throw usage(`broker takes at most ${BROKER_MAX_SYMBOLS} tickers per call`);
  investor = String(investor || "all").toLowerCase(); board = String(board || "all").toLowerCase();
  if (!ipot.INVESTORS.includes(investor)) throw usage(`invalid investor "${investor}"; use all, foreign or domestic`);
  if (!ipot.BOARDS.includes(board)) throw usage(`invalid board "${board}"; use all, regular, cash or negotiated`);
  const date = isoDate(dateRaw, "date");
  let from = isoDate(fromRaw, "from"), to = isoDate(toRaw, "to");
  if (date && (from || to)) throw usage("use date or from/to, not both");
  const today = wibToday();
  if (date) from = to = date;
  else if (from && !to) to = today;
  else if (to && !from) from = to;
  if (from && from > to) throw usage(`the range starts (${from}) after it ends (${to})`);
  const results = await Promise.all(symbols.map((s) => (from ? brokerRange(env, s, from, to, investor, board, today).then((r) => {
    if (!r.data.buyers.length && !r.data.sellers.length) r.warnings.push(`${s}: no trades from ${from} to ${to}`);
    return r;
  }) : brokerLatest(env, s, investor, board, today)).then((ok) => ({ ok })).catch((err) => ({ err, s }))));
  const data = [], warnings = [];
  let cachedAll = true, stale = false, fetched = Math.floor(Date.now() / 1000);
  for (const r of results) {
    if (r.ok) { data.push(r.ok.data); cachedAll &&= r.ok.cached; stale ||= r.ok.stale; fetched = Math.min(fetched, r.ok.fetched_at); warnings.push(...r.ok.warnings); }
    else warnings.push(r.err.code === "not_found" ? `no broker summary for ${r.s}` : `${r.s}: ${r.err.message}`);
  }
  if (!data.length) throw results[0].err;
  return envelope("idx.broker.v1", data, { provider: ipot.NAME, cached: cachedAll, stale, fetched_at: fetched, warnings });
}

// ---- corporate actions and holidays ------------------------------------

/** Days `actions` reaches back without `since`, like the CLI. */
const ACTIONS_DEFAULT_DAYS = 365;

export async function actions(env, symbolRaw, sinceRaw) {
  const symbol = parseSymbol(symbolRaw);
  if (symbol.startsWith("^")) throw usage(`${symbol} is an index; corporate actions belong to stocks`);
  const since = isoDate(sinceRaw, "since") || addDays(wibToday(), -ACTIONS_DEFAULT_DAYS);
  // Announcements are added and confirmed through the day.
  const r = await cached(env, `ipotca:${symbol}:${since}`, 6 * 3600, async () => {
    let html;
    try { html = await ipot.fetchPage(ipot.actionsUrl(symbol, since)); } catch (e) { throw provider(`ipot: corporate actions for ${symbol}: ${e.message}`); }
    const p = ipot.parseActions(html);
    if (p.invalid) throw notFound(`${symbol} is not a stock code IPOT knows`);
    return p;
  });
  const p = r.value;
  const data = { symbol, name: NAME_BY_SYMBOL.get(symbol) || null, since, actions: p.actions, meetings: p.meetings, warrants_rights: p.warrants_rights };
  return envelope("idx.actions.v1", data, { provider: ipot.NAME, cached: r.cached, stale: r.stale, fetched_at: r.fetched_at, warnings: r.warning ? [r.warning] : [] });
}

export function holidays(yearRaw, allRaw, upcomingRaw) {
  const truthy = (v) => v === "1" || v === "true" || v === true;
  const today = wibToday();
  let year = null;
  if (!truthy(allRaw)) {
    year = yearRaw == null || yearRaw === "" ? Number(today.slice(0, 4)) : Number(yearRaw);
    if (!Number.isInteger(year) || year < 1990 || year > 2100) throw usage(`year ${yearRaw} is out of range`);
  }
  const { data, warnings } = hol.holidays(year, truthy(upcomingRaw), today);
  return envelope("idx.holidays.v1", data, { provider: "bundled", cached: true, warnings });
}

// ---- astro --------------------------------------------------------------

/** The web lists up to 90 days of events; the CLI takes 366. */
const ASTRO_WEB_MAX_DAYS = 90;
/** The web backtests up to 10y (default 5y); max is CLI-only (9,000+ sessions). */
const ASTRO_WEB_PERIODS = ["1y", "2y", "5y", "10y"];

/** idx.astro.v1: the sky for a date (computed here) and, with backtest, IHSG's daily returns by factor. */
export async function astro(env, dateRaw, daysRaw, natalRaw, backtestRaw, periodRaw) {
  const truthy = (v) => v === "1" || v === "true" || v === true;
  const date = isoDate(dateRaw, "date") || wibToday();
  if (date < astroLib.FIRST_DATE || date > astroLib.LAST_DATE) throw usage(`astro covers ${astroLib.FIRST_DATE} to ${astroLib.LAST_DATE}, the range of the planetary elements; got ${date}`);
  const days = daysRaw == null || daysRaw === "" ? 30 : Number(daysRaw);
  if (!Number.isInteger(days) || days < 1 || days > ASTRO_WEB_MAX_DAYS) throw usage(`days must be 1 to ${ASTRO_WEB_MAX_DAYS} in the web terminal (the CLI takes 366); got ${daysRaw}`);
  const natal = String(natalRaw || "launch").trim().toLowerCase();
  if (!["launch", "base", "none"].includes(natal)) throw usage(`invalid natal chart "${natalRaw}"; use launch, base or none`);
  const period = String(periodRaw || "5y").trim().toLowerCase();
  if (period === "max") throw usage("the web terminal backtests up to 10y; the CLI runs max (idx market astro --backtest)");
  if (!ASTRO_WEB_PERIODS.includes(period)) throw usage(`invalid backtest period "${periodRaw}"; use ${ASTRO_WEB_PERIODS.join(", ")}`);
  const { data, warnings } = astroLib.sky(date, days, natal);
  if (!truthy(backtestRaw)) return envelope("idx.astro.v1", data, { provider: "computed", warnings });
  // The result, not the candles, is cached: one D1 row per period, refreshed every 6 hours.
  const r = await cached(env, `astro_backtest:${period}`, 6 * 3600, async () => {
    const h = await history(env, "^JKSE", period, "1d", 0);
    const hist = { ...h.data };
    const leftOut = astroLib.dropUnfinished(hist, Date.parse(h.fetched_at));
    try {
      return { backtest: astroLib.backtest(hist, period), left_out: leftOut };
    } catch (e) {
      if (e instanceof astroLib.BacktestError) throw e.code === "not_found" ? notFound(e.message) : provider(`backtest: ${e.message}`);
      throw e;
    }
  });
  data.backtest = r.value.backtest;
  if (r.value.left_out) warnings.push(`the backtest leaves out ${r.value.left_out}, a session that had not closed when the history was fetched`);
  if (r.warning) warnings.push(r.warning);
  return envelope("idx.astro.v1", data, { provider: "yahoo", cached: r.cached, stale: r.stale, fetched_at: r.fetched_at, warnings });
}

// ---- bandarmology ------------------------------------------------------

const BANDAR_WEB_MAX_SYMBOLS = 5;
/** Sessions the web reads for one daily series (a month); the CLI goes further. */
const BANDAR_DAILY_MAX_SESSIONS = 23;

async function boardQuotes(env) {
  const b = await kvGet(env, "board");
  return new Map((b?.value?.quotes || []).map((q) => [q.symbol, q]));
}

/** Every weekday's summary of the range as one cached row (one D1 read, one write). */
async function bandarSessions(env, symbol, from, to, investor, mboard, today) {
  const ttl = to < today ? 7 * 86400 : 5 * 60;
  return cached(env, `bandar_daily:${symbol}:${from}:${to}:${investor}:${mboard}`, ttl, async () => {
    const days = bd.weekdays(from, to), out = [];
    for (let i = 0; i < days.length; i += 3) {
      const pages = await Promise.all(days.slice(i, i + 3).map(async (d) => [d, ipot.parse(await ipot.fetchPage(ipot.url(symbol, d, d, investor, mboard)))]));
      for (const [d, p] of pages) if (!p.invalid) out.push({ ...p, from: p.from || d, to: p.to || d });
    }
    return out;
  });
}

async function bandarOne(env, s, q) {
  const { period, from, to, top, investor, mboard, daily, today, quotes } = q;
  let sum;
  if (from) {
    sum = await brokerRange(env, s, from, to, investor, mboard, today);
  } else {
    const latest = await brokerLatest(env, s, investor, mboard, today);
    const empty = !latest.data.buyers.length && !latest.data.sellers.length;
    if (period === "1d" || empty) sum = latest;
    else {
      sum = await brokerRange(env, s, bd.periodStart(period, latest.data.to), latest.data.to, investor, mboard, today);
      sum.warnings.push(...latest.warnings);
      sum.cached &&= latest.cached;
    }
  }
  const d = sum.data, warnings = [...sum.warnings];
  const empty = !d.buyers.length && !d.sellers.length;
  if (from && empty) warnings.push(`${s}: no trades from ${from} to ${to}`);
  const quote = quotes.get(s);
  let move = { price: quote?.price ?? null, chg: null };
  if (!empty) {
    move = bd.quoteMove(quote, d.from, d.to);
    if (!move) {
      try {
        const h = await history(env, s, bd.historyPeriod(d.from, today), "1d", 0);
        move = bd.priceMove(h.data.candles, d.from, d.to, quote);
      } catch (e) {
        warnings.push(`${s}: no price move for the range (${e.message})`);
        move = { price: quote?.price ?? null, chg: null };
      }
    }
  }
  const rep = bd.report(d, from ? "custom" : period, top, move.price ?? quote?.price ?? null, move.chg);
  if (daily && !empty) {
    try {
      const r = await bandarSessions(env, s, d.from, d.to, investor, mboard, today);
      rep.daily = bd.daily(r.value, top);
    } catch (e) {
      warnings.push(`${s}: the daily series failed (${e.message})`);
    }
  }
  return { rep, warnings, cached: sum.cached, stale: sum.stale, fetched_at: sum.fetched_at };
}

/** /api/bandar: the CLI's `idx bandar detect` (idx.bandar.v1), live through the D1 cache. */
export async function bandar(env, symbolsRaw, periodRaw, dateRaw, fromRaw, toRaw, topRaw, investor = "all", mboard = "all", dailyRaw = null) {
  const symbols = parseSymbols(symbolsRaw);
  if (symbols.length > BANDAR_WEB_MAX_SYMBOLS) throw usage(`the web terminal reads at most ${BANDAR_WEB_MAX_SYMBOLS} tickers per bandar call; the CLI takes 10`);
  investor = String(investor || "all").toLowerCase(); mboard = String(mboard || "all").toLowerCase();
  if (!ipot.INVESTORS.includes(investor)) throw usage(`invalid investor "${investor}"; use all, foreign or domestic`);
  if (!ipot.BOARDS.includes(mboard)) throw usage(`invalid board "${mboard}"; use all, regular, cash or negotiated`);
  const period = bd.checkPeriod(periodRaw || "1w");
  const top = topRaw == null || topRaw === "" ? bd.DEFAULT_TOP : bd.checkTop(topRaw);
  const daily = dailyRaw === true || dailyRaw === "1" || dailyRaw === "true";
  const date = isoDate(dateRaw, "date");
  let from = isoDate(fromRaw, "from"), to = isoDate(toRaw, "to");
  if (date && (from || to)) throw usage("use date or from/to, not both");
  const today = wibToday();
  if (date) from = to = date;
  else if (from && !to) to = today;
  else if (to && !from) from = to;
  if (from && from > to) throw usage(`the range starts (${from}) after it ends (${to})`);
  if (daily && symbols.length > 1) throw usage("the daily series on the web takes one ticker; the CLI takes three");
  if (daily && ((!from && period === "3m") || (from && bd.weekdays(from, to).length > BANDAR_DAILY_MAX_SESSIONS))) {
    throw usage(`the daily series on the web covers at most ${BANDAR_DAILY_MAX_SESSIONS} sessions (1m); the CLI goes further`);
  }
  const quotes = await boardQuotes(env);
  const q = { period, from, to, top, investor, mboard, daily, today, quotes };
  const results = await Promise.all(symbols.map((s) => bandarOne(env, s, q).then((ok) => ({ ok })).catch((err) => ({ err, s }))));
  const data = [], warnings = [];
  let cachedAll = true, stale = false, fetched = Math.floor(Date.now() / 1000);
  for (const r of results) {
    if (r.ok) { data.push(r.ok.rep); cachedAll &&= r.ok.cached; stale ||= r.ok.stale; fetched = Math.min(fetched, r.ok.fetched_at); warnings.push(...r.ok.warnings); }
    else warnings.push(r.err.code === "not_found" ? `no broker summary for ${r.s}` : `${r.s}: ${r.err.message}`);
  }
  if (!data.length) throw results[0].err;
  return envelope("idx.bandar.v1", data, { provider: ipot.NAME, cached: cachedAll, stale, fetched_at: fetched, warnings });
}

/** /api/bandar/screen: `idx bandar screen` over the nightly scan in D1 (idx.bandar.screen.v1). */
export async function bandarScreen(env, presetRaw, filterRaw, periodRaw, topRaw, sortRaw, ascRaw, limitRaw) {
  const pr = bd.preset(presetRaw);
  const filters = [...(pr ? bd.parseFilters(pr.filter) : []), ...bd.parseFilters(filterRaw || "")];
  const period = bd.checkPeriod(periodRaw || "1w");
  const top = topRaw == null || topRaw === "" ? bd.DEFAULT_TOP : bd.checkTop(topRaw);
  const sort = bd.parseSort(sortRaw) || "acc";
  const descending = ascRaw == null || ascRaw === "" ? !pr?.ascending : !(ascRaw === "1" || ascRaw === "true");
  const limit = num(limitRaw, 25);
  const scan = await scanRows(env, period);
  if (!scan.session) throw notFound("the bandar scan has not run yet: after the close (from 16:45 WIB) the minute cron reads the 100 most traded stocks; the CLI scans live meanwhile");
  const st = scan.state || {};
  const warnings = [];
  if (scan.partial) warnings.push(`the scan of ${scan.session} is still running: ${scan.rows.length} of ${new Set(st.symbols || []).size} stocks so far`);
  else if (!st.done && st.session !== scan.session) warnings.push(`the scan of ${st.session} is running (${Math.min(st.next || 0, (st.symbols || []).length)} of ${(st.symbols || []).length}); showing ${scan.session}`);
  const failed = st.session === scan.session ? (st.failed || []).length : 0;
  if (failed) warnings.push(`${failed} of ${scan.rows.length + failed} scanned stocks had no broker summary`);
  const traded = scan.rows.filter((r) => r.summary.buyers.length || r.summary.sellers.length);
  let rows = traded.map((r) => bd.screenRow(bd.report(r.summary, period, top, r.price, r.chg))).filter((r) => bd.matches(r, filters));
  bd.sortRows(rows, sort, descending);
  const total = rows.length;
  if (limit > 0) rows = rows.slice(0, limit);
  const data = {
    period, from: traded[0]?.summary.from || bd.periodStart(period, scan.session), to: scan.session, top, preset: pr?.name ?? null,
    filters: filters.map((f) => f.raw), sort, descending, scanned: scan.rows.length + failed, checked: traded.length, total_matches: total, rows,
  };
  const at = Date.parse(st.done_at || st.started_at || "") / 1000;
  return envelope("idx.bandar.screen.v1", data, { provider: ipot.NAME, cached: true, fetched_at: Number.isFinite(at) ? at : new Date(), warnings });
}

/** One stock where the broker is top ten on a side, like service::stock_row. */
function brokerStockRow(s, flow, other, buying) {
  const opposite = other?.value ?? null;
  const net = flow.value == null || opposite == null ? null : buying ? flow.value - opposite : opposite - flow.value;
  return {
    symbol: s.symbol, name: s.name ?? null, rank: flow.rank, lots: flow.lots, value: flow.value, average_price: flow.average_price,
    share_pct: flow.value != null && s.total_value > 0 ? (flow.value / s.total_value) * 100 : null,
    opposite_value: opposite, net_value: net,
  };
}

/** Broker activity from the nightly scan: where one broker is a top-ten buyer or seller of the scanned stocks. */
export async function bandarBroker(env, codeRaw, periodRaw, limitRaw) {
  const code = String(codeRaw || "").trim().toUpperCase();
  if (!/^[A-Z0-9]{2}$/.test(code)) throw usage(`"${codeRaw ?? ""}" is not a broker code; codes are two characters such as YP or CC (see members)`);
  const period = bd.checkPeriod(periodRaw || "1w");
  const limit = num(limitRaw, 20);
  const scan = await scanRows(env, period);
  if (!scan.session) throw notFound("the bandar scan has not run yet: after the close (from 16:45 WIB) the minute cron reads the 100 most traded stocks; the CLI scans live meanwhile");
  const st = scan.state || {};
  const warnings = [];
  if (!BROKERS[code]) warnings.push(`${code} is not on the bundled exchange-member list`);
  if (scan.partial) warnings.push(`the scan of ${scan.session} is still running: ${scan.rows.length} of ${new Set(st.symbols || []).size} stocks so far`);
  const failed = st.session === scan.session ? (st.failed || []).length : 0;
  if (failed) warnings.push(`${failed} of ${scan.rows.length + failed} scanned stocks had no broker summary`);
  const traded = scan.rows.filter((r) => r.summary.buyers.length || r.summary.sellers.length);
  const bought = [], sold = [];
  for (const { summary: s } of traded) {
    const buy = s.buyers.find((f) => f.code === code), sell = s.sellers.find((f) => f.code === code);
    if (buy) bought.push(brokerStockRow(s, buy, sell, true));
    if (sell) sold.push(brokerStockRow(s, sell, buy, false));
  }
  const byValue = (a, b) => (b.value ?? -Infinity) - (a.value ?? -Infinity);
  bought.sort(byValue); sold.sort(byValue);
  const sum = (rows) => rows.reduce((t, r) => t + (r.value ?? 0), 0);
  const data = {
    code, name: BROKERS[code] ?? null, period, from: traded[0]?.summary.from || bd.periodStart(period, scan.session), to: scan.session,
    scanned: scan.rows.length + failed, checked: traded.length, failed, bought_value: sum(bought), sold_value: sum(sold),
    bought: limit > 0 ? bought.slice(0, limit) : bought, sold: limit > 0 ? sold.slice(0, limit) : sold,
  };
  const at = Date.parse(st.done_at || st.started_at || "") / 1000;
  return envelope("idx.bandar.broker.v1", data, { provider: ipot.NAME, cached: true, fetched_at: Number.isFinite(at) ? at : new Date(), warnings });
}

// ---- directories (bundled) ---------------------------------------------

const matchesSearch = (q, code, name) => {
  const s = String(q ?? "").trim().toLowerCase();
  return !s || code.toLowerCase().includes(s) || String(name ?? "").toLowerCase().includes(s);
};

/** Listed stocks: the bundled universe (codes and names); the CLI reads boards, listing dates and shares from idx.co.id. */
export function instruments(searchRaw, boardRaw) {
  const search = String(searchRaw ?? "").trim() || null, board = String(boardRaw ?? "").trim() || null;
  const warnings = ["the web terminal lists the bundled universe (codes and names); `idx stocks list` reads listing boards, dates and shares from idx.co.id"];
  let list = UNIVERSE.map(([symbol, name]) => ({ symbol, name: name || null, listing_date: null, shares: null, board: null }));
  const total = list.length;
  if (board) { warnings.push("the bundled list has no boards, so a board filter matches nothing here"); list = []; }
  list = list.filter((i) => matchesSearch(search, i.symbol, i.name)).sort((a, b) => (a.symbol < b.symbol ? -1 : 1));
  return envelope("idx.instruments.v1", { total, board, search, instruments: list }, { provider: "bundled", cached: true, warnings });
}

/** Exchange members from the bundled list (codes and names); the CLI adds licences from idx.co.id. */
export function members(searchRaw) {
  const search = String(searchRaw ?? "").trim() || null;
  const all = Object.entries(BROKERS).map(([code, name]) => ({ code, name, licenses: null })).sort((a, b) => (a.code < b.code ? -1 : 1));
  const list = all.filter((m) => matchesSearch(search, m.code, m.name));
  return envelope("idx.members.v1", { total: all.length, search, members: list }, { provider: "bundled", cached: true, warnings: ["the web terminal lists the bundled member list (codes and names); `idx market members` adds licences from idx.co.id"] });
}

// ---- ownership (synced register) --------------------------------------

async function emptyRegisterMessage(env) {
  const s = await syncStatus(env);
  if (s.loading) return `the KSEI register for ${s.loading.as_of} is loading from the monthly mirror (part ${s.loading.part} of ${s.loading.of}); try again in a few minutes`;
  return `no ownership release loaded yet; the web terminal pulls the monthly KSEI register from the GitHub mirror by itself${s.checked_at ? ` (last checked ${s.checked_at})` : ""}`;
}
async function ownershipRelease(env, asOf) {
  const row = asOf
    ? await env.DB.prepare("SELECT as_of FROM own_releases WHERE as_of = ?1").bind(asOf).first()
    : await env.DB.prepare("SELECT as_of FROM own_releases ORDER BY as_of DESC LIMIT 1").first();
  if (!row) {
    const have = (await env.DB.prepare("SELECT as_of FROM own_releases ORDER BY as_of").all()).results.map((r) => r.as_of);
    throw notFound(have.length ? `no ownership release for ${asOf}; available: ${have.join(", ")}` : await emptyRegisterMessage(env));
  }
  return row.as_of;
}
const holdingRow = (r) => ({ as_of: r.as_of, symbol: r.symbol, issuer_name: r.issuer_name, holder: r.holder, holder_raw: r.holder_raw, entity_key: r.entity_key, investor_type: r.investor_type, local_foreign: r.local_foreign, nationality: r.nationality ?? null, domicile: r.domicile ?? null, shares_scripless: r.shares_scripless ?? null, shares_scrip: r.shares_scrip ?? null, shares_total: r.shares_total, pct: r.pct });

export async function ownershipTicker(env, symbolRaw, asOf) {
  const symbol = parseSymbol(symbolRaw);
  const date = await ownershipRelease(env, asOf);
  const rows = (await env.DB.prepare("SELECT * FROM own_holdings WHERE as_of = ?1 AND symbol = ?2 ORDER BY pct DESC").bind(date, symbol).all()).results;
  if (!rows.length) throw notFound(`no holders above 1% recorded for ${symbol} as of ${date}`);
  const holders = rows.map(holdingRow);
  return envelope("idx.ownership.holders.v1", { symbol, issuer_name: rows[0].issuer_name, as_of: date, listed_pct: holders.reduce((s, h) => s + h.pct, 0), holders }, { provider: "github-mirror", cached: true });
}
export async function ownershipEntity(env, query, asOf) {
  if (!query || query.trim().length < 3) throw usage("entity name needs at least 3 characters");
  const date = await ownershipRelease(env, asOf);
  const key = query.toUpperCase().replace(/[^A-Z0-9]+/g, " ").split(" ").filter((t) => t && !["PT","TBK","PERSERO"].includes(t)).join(" ");
  const rows = (await env.DB.prepare("SELECT * FROM own_holdings WHERE as_of = ?1 AND entity_key LIKE ?2 ORDER BY pct DESC").bind(date, `%${key}%`).all()).results;
  if (!rows.length) throw notFound(`no holder matching "${query}" as of ${date}`);
  const entities = [...new Set(rows.map((r) => r.holder))].sort();
  return envelope("idx.ownership.entity.v1", { query, as_of: date, entities, holdings: rows.map(holdingRow) }, { provider: "github-mirror", cached: true });
}
export async function ownershipReleases(env) {
  const rows = (await env.DB.prepare("SELECT * FROM own_releases ORDER BY as_of").all()).results;
  const s = await syncStatus(env);
  const warnings = [];
  if (s.loading) warnings.push(`loading ${s.loading.as_of}: part ${s.loading.part} of ${s.loading.of}`);
  if (!rows.length && !s.loading) warnings.push(await emptyRegisterMessage(env));
  return envelope("idx.ownership.releases.v1", rows.map((r) => ({ as_of: r.as_of, format: r.format, source: r.source, sha256: r.sha256 || "", rows: r.rows, issuers: r.issuers ?? 0, entities: r.entities ?? 0, imported_at: new Date(r.synced_at * 1000).toISOString() })), { provider: "github-mirror", cached: true, warnings });
}
export async function ownershipChanges(env, from, to, symbolRaw, minChange = 0, limit = 50, offset = 0) {
  if (!from || !to || from >= to) throw usage("--from must be earlier than --to (YYYY-MM-DD)");
  await ownershipRelease(env, from); await ownershipRelease(env, to);
  const symbol = symbolRaw ? parseSymbol(symbolRaw) : null;
  const load = async (d) => {
    const q = symbol ? env.DB.prepare("SELECT symbol, holder, entity_key, investor_type, local_foreign, shares_total, pct FROM own_holdings WHERE as_of = ?1 AND symbol = ?2").bind(d, symbol)
                     : env.DB.prepare("SELECT symbol, holder, entity_key, investor_type, local_foreign, shares_total, pct FROM own_holdings WHERE as_of = ?1").bind(d);
    const m = new Map();
    for (const r of (await q.all()).results) m.set(`${r.symbol}|${r.entity_key}`, r);
    return m;
  };
  const [before, after] = await Promise.all([load(from), load(to)]);
  const changes = [];
  const r4 = (v) => Math.round(v * 1e4) / 1e4;
  for (const [k, a] of after) {
    const b = before.get(k);
    if (b) {
      if (Math.abs(a.pct - b.pct) < 1e-9 && a.shares_total === b.shares_total) continue;
      changes.push({ symbol: a.symbol, holder: a.holder, entity_key: a.entity_key, investor_type: a.investor_type, local_foreign: a.local_foreign, kind: "changed", pct_from: b.pct, pct_to: a.pct, pct_change: r4(a.pct - b.pct), shares_from: b.shares_total, shares_to: a.shares_total, shares_change: a.shares_total - b.shares_total });
    } else changes.push({ symbol: a.symbol, holder: a.holder, entity_key: a.entity_key, investor_type: a.investor_type, local_foreign: a.local_foreign, kind: "new", pct_from: null, pct_to: a.pct, pct_change: r4(a.pct), shares_from: null, shares_to: a.shares_total, shares_change: a.shares_total });
  }
  for (const [k, b] of before) if (!after.has(k)) changes.push({ symbol: b.symbol, holder: b.holder, entity_key: b.entity_key, investor_type: b.investor_type, local_foreign: b.local_foreign, kind: "exited", pct_from: b.pct, pct_to: null, pct_change: r4(-b.pct), shares_from: b.shares_total, shares_to: null, shares_change: -b.shares_total });
  const kept = changes.filter((c) => Math.abs(c.pct_change) + 1e-12 >= minChange).sort((a, b) => Math.abs(b.pct_change) - Math.abs(a.pct_change) || a.symbol.localeCompare(b.symbol));
  const summary = ex.summariseChanges(kept);
  const off = Math.min(Math.max(0, offset), kept.length);
  const page = kept.slice(off, limit > 0 ? off + limit : undefined);
  return envelope("idx.ownership.changes.v1", { from, to, symbol, min_pct_change: minChange, total_matches: kept.length, offset: off, summary, changes: page }, { provider: "github-mirror", cached: true });
}

async function ownershipPct(env, asOf) {
  const rows = (await env.DB.prepare("SELECT symbol, MAX(issuer_name) AS issuer_name, COALESCE(SUM(CASE WHEN local_foreign = 'A' THEN pct END), 0) AS foreign_pct, SUM(CASE WHEN local_foreign = 'A' THEN 1 ELSE 0 END) AS foreign_holders FROM own_holdings WHERE as_of = ?1 GROUP BY symbol").bind(asOf).all()).results;
  return new Map(rows.map((r) => [r.symbol, r]));
}
export async function ownershipForeign(env, symbolRaw, from, to, limit = 25) {
  const r4 = (v) => { const r = Math.round(v * 1e4) / 1e4; return r === 0 ? 0 : r; };
  if (symbolRaw) {
    const symbol = parseSymbol(symbolRaw);
    const rows = (await env.DB.prepare("SELECT as_of, MAX(issuer_name) AS issuer_name, COALESCE(SUM(CASE WHEN local_foreign = 'A' THEN pct END), 0) AS fp, COALESCE(SUM(CASE WHEN local_foreign = 'L' THEN pct END), 0) AS lp, COALESCE(SUM(CASE WHEN local_foreign IS NULL OR local_foreign NOT IN ('A','L') THEN pct END), 0) AS up, SUM(CASE WHEN local_foreign = 'A' THEN 1 ELSE 0 END) AS fh, SUM(CASE WHEN local_foreign = 'L' THEN 1 ELSE 0 END) AS lh, SUM(pct) AS listed FROM own_holdings WHERE symbol = ?1 AND as_of IN (SELECT as_of FROM own_releases) GROUP BY as_of ORDER BY as_of").bind(symbol).all()).results;
    if (!rows.length) { await ownershipRelease(env, null); throw notFound(`no holders above 1% recorded for ${symbol} in any loaded release`); }
    const points = rows.map((r, i) => ({ as_of: r.as_of, foreign_pct: r4(r.fp), local_pct: r4(r.lp), unflagged_pct: r4(r.up), foreign_holders: r.fh, local_holders: r.lh, listed_pct: r4(r.listed), foreign_change_pp: i ? r4(r.fp - rows[i - 1].fp) : null }));
    return envelope("idx.ownership.foreign.v1", { symbol, issuer_name: rows.map((r) => r.issuer_name).filter(Boolean).pop() || null, points }, { provider: "github-mirror", cached: true });
  }
  const releases = (await env.DB.prepare("SELECT as_of FROM own_releases ORDER BY as_of").all()).results.map((r) => r.as_of);
  if (!releases.length) throw notFound("no ownership data loaded on the web terminal yet");
  const toDate = to || releases[releases.length - 1];
  const fromDate = from || [...releases].reverse().find((d) => d < toDate);
  if (!fromDate) throw notFound(`only one release (${toDate}) is loaded; nothing earlier to compare`);
  if (fromDate >= toDate) throw usage("from must be earlier than to");
  await ownershipRelease(env, fromDate); await ownershipRelease(env, toDate);
  const [before, after] = await Promise.all([ownershipPct(env, fromDate), ownershipPct(env, toDate)]);
  const rows = [...after.entries()].map(([symbol, a]) => { const b = before.get(symbol); const pf = b ? r4(b.foreign_pct) : null; return { symbol, issuer_name: a.issuer_name, foreign_pct_from: pf, foreign_pct_to: r4(a.foreign_pct), change_pp: r4(a.foreign_pct - (pf || 0)), foreign_holders_to: a.foreign_holders }; })
    .sort((a, b) => Math.abs(b.change_pp) - Math.abs(a.change_pp) || a.symbol.localeCompare(b.symbol));
  const data = { from: fromDate, to: toDate, issuers: rows.length, inflows: rows.filter((r) => r.change_pp > 0).length, outflows: rows.filter((r) => r.change_pp < 0).length, net_pp: r4(rows.reduce((s, r) => s + r.change_pp, 0)), rows: limit > 0 ? rows.slice(0, limit) : rows };
  return envelope("idx.ownership.foreign-flow.v1", data, { provider: "github-mirror", cached: true });
}

// ---- brief ---------------------------------------------------------------------

export async function brief(env, symbolRaw) {
  const symbol = parseSymbol(symbolRaw);
  const sections = {
    quote: () => quote(env, symbol), fundamentals: () => fundamentals(env, symbol), technical: () => technical(env, symbol, "1y"),
    returns: () => returns(env, symbol, "^JKSE"), dividends: () => dividends(env, symbol), ownership: () => ownershipTicker(env, symbol, null),
    news: () => news(env, symbol, 0, true).then((n) => n.env),
  };
  const names = Object.keys(sections);
  const settled = await Promise.allSettled(names.map((n) => sections[n]()));
  const warnings = [], failed = [];
  let cachedAll = true, stale = false, notFoundCount = 0;
  const got = {};
  settled.forEach((r, i) => {
    const n = names[i];
    if (r.status === "fulfilled") { got[n] = r.value; cachedAll &&= !!r.value.cached; stale ||= !!r.value.stale; for (const w of r.value.warnings || []) warnings.push(`${n}: ${w}`); }
    else { if (r.reason?.code === "not_found") notFoundCount++; warnings.push(`${n}: ${r.reason?.message || r.reason}`); failed.push(n); }
  });
  if (!got.quote && !got.fundamentals && !got.technical && notFoundCount >= 3) throw notFound(`${symbol}: unknown to the quote, fundamentals and history sources`);
  const q = got.quote?.data?.[0] || null, f = got.fundamentals?.data || null, t = got.technical?.data || null, r = got.returns?.data?.[0] || null;
  const d = got.dividends?.data || null;
  const o = got.ownership?.data || null;
  const n = got.news?.data || null;
  const rep = n ? an.aggregateSentiment(symbol, n.matched_name, n.items, 0) : null;
  const pct = (v, dp = 1) => `${v >= 0 ? "+" : ""}${v.toFixed(dp)}%`;
  const key_points = [];
  if (q && q.price != null && q.change_pct != null) key_points.push(`Price ${q.price.toLocaleString("en-US")} (${pct(q.change_pct, 2)} today)${q.market_cap ? `, market cap ${(q.market_cap / 1e12).toFixed(1)}T` : ""}`);
  if (f?.verdict) key_points.push(`Valuation verdict: ${f.verdict.label} (score ${f.verdict.score >= 0 ? "+" : ""}${f.verdict.score}, ${f.verdict.inputs_available} inputs)${f.valuation.pe_trailing != null ? `; P/E ${f.valuation.pe_trailing.toFixed(1)}` : ""}${f.valuation.price_to_book != null ? `, P/B ${f.valuation.price_to_book.toFixed(2)}` : ""}`);
  if (t) key_points.push(`Technical signal: ${t.signal.label} (score ${t.signal.score >= 0 ? "+" : ""}${t.signal.score}), trend ${t.trend}${t.rsi14 != null ? `, RSI ${t.rsi14.toFixed(0)}` : ""}`);
  if (r?.returns?.["1y"] != null) key_points.push(`1y return ${pct(r.returns["1y"])}${r.benchmark?.excess?.["1y"] != null ? ` (${r.benchmark.excess["1y"] >= 0 ? "+" : ""}${r.benchmark.excess["1y"].toFixed(1)} pp vs ${r.benchmark.symbol})` : ""}; YTD ${r.returns.ytd != null ? pct(r.returns.ytd) : "n/a"}`);
  if (d) key_points.push(d.ttm_yield_pct != null ? `Dividends: TTM yield ${d.ttm_yield_pct.toFixed(2)}% over ${d.ttm_payments} payment${d.ttm_payments === 1 ? "" : "s"}, ${d.consecutive_years} consecutive years` : d.dividends.length ? "Dividends: none in the last twelve months" : "Dividends: none on record");
  const own = o ? { as_of: o.as_of, listed_pct: o.listed_pct, foreign_pct: o.holders.filter((h) => h.local_foreign === "A").reduce((s, h) => s + h.pct, 0), holders_listed: o.holders.length, top_holders: o.holders.slice(0, 5) } : null;
  if (own) key_points.push(`Ownership (${own.as_of}): ${own.holders_listed} holders above 1% hold ${own.listed_pct.toFixed(1)}%, foreign ${own.foreign_pct.toFixed(1)}%${own.top_holders[0] ? `; largest ${own.top_holders[0].holder} ${own.top_holders[0].pct.toFixed(1)}%` : ""}`);
  if (n) key_points.push(n.company_matches ? `News: ${n.company_matches} current headline${n.company_matches === 1 ? "" : "s"} mention the company, lexicon sentiment ${rep.label} (${rep.score >= 0 ? "+" : ""}${rep.score.toFixed(2)})` : "News: no current headline mentions the company");
  const data = {
    symbol, name: q?.name || f?.name || t?.name || NAME_BY_SYMBOL.get(symbol) || null, generated_at: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"), key_points,
    quote: q, fundamentals: f, technical: t, returns: r,
    dividends: d ? { ttm_total: d.ttm_total, ttm_payments: d.ttm_payments, ttm_yield_pct: d.ttm_yield_pct, consecutive_years: d.consecutive_years, last: d.last, payments_on_record: d.dividends.length } : null,
    ownership: own,
    news: n ? { company_matches: n.company_matches, sentiment: rep.label, score: rep.score, headlines: n.items.slice(0, 5) } : null,
    sections_failed: failed,
  };
  return envelope("idx.brief.v1", data, { cached: cachedAll, stale, warnings });
}

// ---- router ----------------------------------------------------------

const num = (v, d) => { if (v == null || v === "") return d; const n = Number(v); if (!Number.isFinite(n)) throw usage(`"${v}" is not a number`); return n; };

export async function route(env, url) {
  const p = url.pathname.replace(/^\/api/, "");
  const q = url.searchParams;
  switch (p) {
    case "/market": return market(env);
    case "/quote": return quote(env, q.get("symbols") || q.get("symbol"));
    case "/history": return history(env, q.get("symbol"), q.get("period") || "1y", q.get("interval") || "1d", num(q.get("limit"), 0));
    case "/profile": return profile(env, q.get("symbol"));
    case "/fundamental": return fundamentals(env, q.get("symbol"));
    case "/earnings": return earnings(env, q.get("symbol"));
    case "/financials": return financials(env, q.get("symbol"), q.get("period") || "annual");
    case "/technical": return technical(env, q.get("symbol"), q.get("period") || "1y");
    case "/compare": return compare(env, q.get("symbols"));
    case "/screen": return screen(env, q.get("filter") || "", q.get("sort") || "mcap", num(q.get("limit"), 25), num(q.get("max_fundamentals"), 20), q.get("asc") == null);
    case "/news": return (await news(env, q.get("symbol"), num(q.get("limit"), 20), q.get("company_only") === "1" || q.get("company_only") === "true")).env;
    case "/dividends": return dividends(env, q.get("symbol"));
    case "/returns": return returns(env, q.get("symbols") || q.get("symbol"), q.get("benchmark") || "^JKSE");
    case "/brief": return brief(env, q.get("symbol"));
    case "/ownership/foreign": return ownershipForeign(env, q.get("symbol"), q.get("from"), q.get("to"), num(q.get("limit"), 25));
    case "/sentiment": return sentiment(env, q.get("symbol"));
    case "/broker": return broker(env, q.get("symbols") || q.get("symbol"), q.get("date"), q.get("from"), q.get("to"), q.get("investor") || "all", q.get("board") || "all");
    case "/bandar": return bandar(env, q.get("symbols") || q.get("symbol"), q.get("period"), q.get("date"), q.get("from"), q.get("to"), q.get("top"), q.get("investor") || "all", q.get("board") || "all", q.get("daily"));
    case "/bandar/screen": return bandarScreen(env, q.get("preset"), q.get("filter"), q.get("period"), q.get("top"), q.get("sort"), q.get("asc"), q.get("limit"));
    case "/bandar/status": return envelope("idx.bandar.scan.v1", await scanStatus(env), { provider: ipot.NAME, cached: true });
    case "/bandar/broker": return bandarBroker(env, q.get("code"), q.get("period"), q.get("limit"));
    case "/actions": return actions(env, q.get("symbol"), q.get("since"));
    case "/holidays": return holidays(q.get("year"), q.get("all"), q.get("upcoming"));
    case "/astro": return astro(env, q.get("date"), q.get("days"), q.get("natal"), q.get("backtest"), q.get("period"));
    case "/instruments": return instruments(q.get("search"), q.get("board"));
    case "/members": return members(q.get("search"));
    case "/ownership/ticker": return ownershipTicker(env, q.get("symbol"), q.get("as_of"));
    case "/ownership/entity": return ownershipEntity(env, q.get("name"), q.get("as_of"));
    case "/ownership/changes": return ownershipChanges(env, q.get("from"), q.get("to"), q.get("symbol"), num(q.get("min_change"), 0), num(q.get("limit"), 50), num(q.get("offset"), 0));
    case "/ownership/releases": return ownershipReleases(env);
    case "/ownership/status": return envelope("idx.ownership.sync.v1", await syncStatus(env), { provider: "github-mirror", cached: true });
    case "/universe": return envelope("idx.universe.v1", UNIVERSE.map(([symbol, name]) => ({ symbol, name })), { provider: "bundled", cached: true });
    default: throw notFound(`no such endpoint ${p}`);
  }
}
