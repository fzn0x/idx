// Yahoo Finance access for the Worker: the same endpoints the Rust CLI uses
// (see src/providers/yahoo/mod.rs), with the cookie/crumb session kept in D1.

import { fetchWithin } from "./net.js";

const UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const CHART = "https://query1.finance.yahoo.com/v8/finance/chart/";
const QUOTE = "https://query1.finance.yahoo.com/v7/finance/quote";
const SUMMARY = "https://query2.finance.yahoo.com/v10/finance/quoteSummary/";
const TIMESERIES = "https://query2.finance.yahoo.com/ws/fundamentals-timeseries/v1/finance/timeseries/";

export const FUNDAMENTAL_MODULES = "assetProfile,summaryDetail,defaultKeyStatistics,financialData,price";
export const EARNINGS_MODULES = "earnings,earningsHistory,earningsTrend,calendarEvents,price";

export function yahooSymbol(sym) {
  const s = String(sym).trim().toUpperCase();
  if (s.startsWith("^")) return s;
  return s.endsWith(".JK") ? s : `${s}.JK`;
}
export function plainSymbol(s) {
  return String(s).toUpperCase().replace(/\.JK$/, "");
}

// ---- session -------------------------------------------------------------

async function newSession(fetchFn) {
  const r = await fetchWithin("https://fc.yahoo.com", { headers: { "User-Agent": UA }, redirect: "manual" }, { fetchFn });
  const setCookie = r.headers.get("set-cookie") || "";
  const m = setCookie.match(/A3=[^;]+/);
  if (!m) throw new Error("yahoo: no A3 cookie");
  const cookie = m[0];
  const c = await fetchWithin("https://query2.finance.yahoo.com/v1/test/getcrumb", {
    headers: { "User-Agent": UA, Cookie: cookie },
  }, { fetchFn });
  const crumb = (await c.text()).trim();
  if (!c.ok || !crumb || crumb.length > 64 || crumb.includes("<")) throw new Error("yahoo: bad crumb");
  return { cookie, crumb, created_at: Date.now() };
}

/** Session cached in `kv` for six hours; `force` refreshes it. `fetchFn` stands in for fetch. */
export async function session(env, force = false, { fetchFn } = {}) {
  if (!force) {
    const row = await env.DB.prepare("SELECT v, updated_at FROM kv WHERE k = 'yahoo:session'").first();
    if (row && Date.now() / 1000 - row.updated_at < 6 * 3600) return JSON.parse(row.v);
  }
  const s = await newSession(fetchFn);
  await env.DB.prepare("INSERT OR REPLACE INTO kv (k, v, updated_at) VALUES ('yahoo:session', ?1, ?2)")
    .bind(JSON.stringify(s), Math.floor(Date.now() / 1000))
    .run();
  return s;
}

/**
 * GET with the cookie and crumb, renewed once on a 401 or 403. `held` keeps
 * the session between calls (a board refresh reads it from D1 once for all
 * its batches); `fetchFn` stands in for fetch.
 */
async function getWithSession(env, url, { fetchFn, held = {} } = {}) {
  held.s ??= await session(env, false, { fetchFn });
  for (let attempt = 0; attempt < 2; attempt++) {
    const r = await fetchWithin(`${url}&crumb=${encodeURIComponent(held.s.crumb)}`, {
      headers: { "User-Agent": UA, Cookie: held.s.cookie, Accept: "application/json" },
    }, { fetchFn });
    if ((r.status === 401 || r.status === 403) && attempt === 0) {
      held.s = await session(env, true, { fetchFn });
      continue;
    }
    return r;
  }
  throw new Error("unreachable");
}

// ---- raw fetchers ----------------------------------------------------------

/** Mirrors Yahoo::chart_url: `range=max` comes back monthly whatever the
 * interval, so full daily or weekly history asks for the whole epoch. */
export function chartUrl(symbol, period = "1y", interval = "1d") {
  if (period === "max" && (interval === "1d" || interval === "1wk")) {
    return `${CHART}${yahooSymbol(symbol)}?period1=0&period2=9999999999&interval=${interval}&includePrePost=false`;
  }
  return `${CHART}${yahooSymbol(symbol)}?range=${period}&interval=${interval}&includePrePost=false`;
}

export async function fetchChart(symbol, period = "1y", interval = "1d", { fetchFn } = {}) {
  const url = chartUrl(symbol, period, interval);
  const r = await fetchWithin(url, { headers: { "User-Agent": UA } }, { fetchFn });
  const body = await r.json().catch(() => null);
  if (r.status === 404 || body?.chart?.error?.code === "Not Found") {
    return { notFound: true, message: body?.chart?.error?.description || "not found" };
  }
  if (!r.ok) throw new Error(`yahoo chart HTTP ${r.status}`);
  return { body };
}

/** Full-history chart call carrying dividend and split events (no session needed). */
export async function fetchEvents(symbol) {
  const url = `${CHART}${yahooSymbol(symbol)}?range=max&interval=1mo&events=div%2Csplits`;
  const r = await fetchWithin(url, { headers: { "User-Agent": UA } });
  const body = await r.json().catch(() => null);
  if (r.status === 404 || body?.chart?.error?.code === "Not Found") {
    return { notFound: true, message: body?.chart?.error?.description || "not found" };
  }
  if (!r.ok) throw new Error(`yahoo chart HTTP ${r.status}`);
  return { body };
}

/** v7 quotes for up to ~200 symbols. Unknown symbols are silently omitted. `opts`: { fetchFn, held } as for getWithSession. */
export async function fetchQuotes(env, symbols, opts = {}) {
  const list = symbols.map(yahooSymbol).join(",");
  const r = await getWithSession(env, `${QUOTE}?symbols=${encodeURIComponent(list)}&formatted=false`, opts);
  if (!r.ok) throw new Error(`yahoo quote HTTP ${r.status}`);
  const body = await r.json();
  return body?.quoteResponse?.result || [];
}

export async function fetchSummary(env, symbol, modules) {
  const r = await getWithSession(env, `${SUMMARY}${yahooSymbol(symbol)}?modules=${modules}`);
  const body = await r.json().catch(() => null);
  if (r.status === 404 || body?.quoteSummary?.error?.code === "Not Found") {
    return { notFound: true, message: body?.quoteSummary?.error?.description || "not found" };
  }
  if (!r.ok) throw new Error(`yahoo quoteSummary HTTP ${r.status}`);
  return { body: body?.quoteSummary?.result?.[0] || null };
}

const ITEMS = [
  "TotalRevenue","GrossProfit","OperatingIncome","EBITDA","EBIT","InterestIncome","InterestExpense",
  "NetInterestIncome","PretaxIncome","TaxProvision","NetIncome","NetIncomeCommonStockholders","BasicEPS",
  "DilutedEPS","TotalAssets","TotalLiabilitiesNetMinorityInterest","StockholdersEquity",
  "CashAndCashEquivalents","TotalDebt","NetDebt","OrdinarySharesNumber","OperatingCashFlow",
  "InvestingCashFlow","FinancingCashFlow","CapitalExpenditure","FreeCashFlow","CashDividendsPaid",
];
const TTM_ITEMS = [
  "TotalRevenue","GrossProfit","OperatingIncome","EBITDA","EBIT","PretaxIncome","TaxProvision","NetIncome",
  "NetIncomeCommonStockholders","BasicEPS","DilutedEPS","OperatingCashFlow","InvestingCashFlow",
  "FinancingCashFlow","CapitalExpenditure","FreeCashFlow","CashDividendsPaid",
];

export async function fetchTimeseries(symbol, period = "annual") {
  const prefix = period === "quarterly" ? "quarterly" : period === "ttm" ? "trailing" : "annual";
  const items = prefix === "trailing" ? TTM_ITEMS : ITEMS;
  const types = items.map((i) => prefix + i).join(",");
  const now = Math.floor(Date.now() / 1000);
  const url = `${TIMESERIES}${yahooSymbol(symbol)}?type=${types}&period1=${now - 6 * 365 * 86400}&period2=${now + 86400}&merge=false&padTimeSeries=false`;
  const r = await fetchWithin(url, { headers: { "User-Agent": UA } });
  if (!r.ok) throw new Error(`yahoo timeseries HTTP ${r.status}`);
  return r.json();
}

// ---- parsers (mirror src/providers/yahoo/parse.rs & summary.rs) ----------

const WIB = 7 * 3600;
export function wibIso(unixSecs) {
  const d = new Date((unixSecs + WIB) * 1000);
  return d.toISOString().replace("Z", "+07:00").replace(/\.\d{3}/, "");
}
const num = (v) => (typeof v === "number" && Number.isFinite(v) ? v : null);
const raw = (o, k) => {
  const v = o?.[k];
  if (typeof v === "number") return num(v);
  if (v && typeof v === "object") return num(v.raw);
  return null;
};
const pct = (o, k) => {
  const v = raw(o, k);
  return v == null ? null : v * 100;
};
const text = (o, k) => {
  const v = o?.[k];
  return typeof v === "string" && v.trim() ? v.trim() : null;
};
// Yahoo pads amounts it has not loaded as {raw: 0, fmt: null}; a reported
// figure carries its formatted form. Padded means missing, not zero.
const amount = (o, k) => {
  const v = o?.[k];
  if (v && typeof v === "object" && v.fmt === null && v.raw === 0) return null;
  return raw(o, k);
};
const dateOf = (o, k) => {
  const v = raw(o, k);
  return v == null ? null : new Date(v * 1000).toISOString().slice(0, 10);
};

export function parseV7Quote(q) {
  return {
    symbol: plainSymbol(q.symbol),
    name: q.longName || q.shortName || null,
    currency: q.currency || null,
    price: num(q.regularMarketPrice),
    change: num(q.regularMarketChange),
    change_pct: num(q.regularMarketChangePercent),
    previous_close: num(q.regularMarketPreviousClose),
    open: num(q.regularMarketOpen),
    day_high: num(q.regularMarketDayHigh),
    day_low: num(q.regularMarketDayLow),
    volume: num(q.regularMarketVolume),
    avg_volume_3m: num(q.averageDailyVolume3Month),
    market_cap: num(q.marketCap),
    pe_trailing: num(q.trailingPE),
    pe_forward: num(q.forwardPE),
    eps_ttm: num(q.epsTrailingTwelveMonths),
    price_to_book: num(q.priceToBook),
    dividend_yield_pct: num(q.dividendYield),
    week52_high: num(q.fiftyTwoWeekHigh),
    week52_low: num(q.fiftyTwoWeekLow),
    sma50: num(q.fiftyDayAverage),
    sma200: num(q.twoHundredDayAverage),
    shares_outstanding: num(q.sharesOutstanding),
    market_state: q.marketState || null,
    market_time: q.regularMarketTime ? wibIso(q.regularMarketTime) : null,
    source: "yahoo:quote",
  };
}

/** Interval::bar_seconds: the intraday bar sizes in seconds. */
const BAR_SECONDS = { "1m": 60, "2m": 120, "5m": 300, "15m": 900, "30m": 1800, "1h": 3600 };

const INTERVALS = ["1m", "2m", "5m", "15m", "30m", "1h", "1d", "1wk", "1mo"];
/** Yahoo's dataGranularity as an interval idx knows ("60m" is its other name for "1h"), or null. */
export function granularity(g) {
  if (g === "60m") return "1h";
  return INTERVALS.includes(g) ? g : null;
}

/** Mirrors parse::parse_chart: the interval is the bar size Yahoo reports
 * (meta.dataGranularity) when idx knows it, otherwise the one asked for. */
export function parseChart(body, period, requested) {
  const r = body?.chart?.result?.[0];
  if (!r) throw new Error("chart response has no result");
  const meta = r.meta || {};
  const interval = granularity(meta.dataGranularity) || requested;
  const ts = r.timestamp || [];
  const q = r.indicators?.quote?.[0] || {};
  const adj = r.indicators?.adjclose?.[0]?.adjclose || [];
  const candles = [];
  // In session, and in multi-day windows after the close, an intraday window
  // ends with a row at the last trade time (09:02:16, 16:14:43): a snapshot, not
  // a bar. Bars start on the minute and on the interval's grid from the window's
  // first row, so a row off that grid, or one repeating the time of the row
  // before it, is the snapshot (parse.rs does the same).
  const step = BAR_SECONDS[String(interval)];
  const anchor = ts.find((t) => t != null);
  const mod = (a, n) => ((a % n) + n) % n;
  for (let i = 0; i < ts.length; i++) {
    const o = q.open?.[i], h = q.high?.[i], l = q.low?.[i], c = q.close?.[i];
    if (ts[i] == null || [o, h, l, c].some((v) => v == null)) continue;
    if (step && anchor != null && (ts[i] % 60 !== 0 || mod(ts[i] - anchor, step) !== 0 || (i > 0 && ts[i - 1] === ts[i]))) continue;
    candles.push({
      time: wibIso(ts[i]),
      t: ts[i],
      open: o, high: h, low: l, close: c,
      adj_close: adj[i] ?? null,
      volume: q.volume?.[i] ?? 0,
    });
  }
  candles.sort((a, b) => a.t - b.t);
  let summary = null;
  if (candles.length) {
    const first = candles[0], last = candles[candles.length - 1];
    const high = Math.max(...candles.map((c) => c.high));
    const low = Math.min(...candles.map((c) => c.low));
    const vol = candles.reduce((s, c) => s + c.volume, 0);
    summary = {
      start: first.time, end: last.time,
      first_close: first.close, last_close: last.close,
      change: last.close - first.close,
      change_pct: first.close ? ((last.close - first.close) / first.close) * 100 : 0,
      high, low, avg_volume: Math.round(vol / candles.length), candles: candles.length,
    };
  }
  return {
    symbol: plainSymbol(meta.symbol || ""),
    name: meta.longName || meta.shortName || null,
    currency: meta.currency || null,
    period, interval,
    candles: candles.map(({ t, ...c }) => c),
    summary,
  };
}

/** Mirrors parse::parse_events: raw dividend/split events; computed fields empty. */
export function parseEvents(body) {
  const r = body?.chart?.result?.[0];
  if (!r) throw new Error("chart response has no result");
  const meta = r.meta || {};
  const day = (secs) => (secs == null ? null : wibIso(secs).slice(0, 10));
  const dividends = Object.values(r.events?.dividends || {}).filter((e) => e.date != null && typeof e.amount === "number").map((e) => ({ date: day(e.date), amount: e.amount })).sort((a, b) => a.date.localeCompare(b.date));
  const splits = Object.values(r.events?.splits || {}).filter((e) => e.date != null).map((e) => ({ date: day(e.date), numerator: e.numerator, denominator: e.denominator, ratio: e.splitRatio || `${e.numerator}:${e.denominator}` })).sort((a, b) => a.date.localeCompare(b.date));
  return {
    symbol: plainSymbol(meta.symbol || ""), name: meta.longName || meta.shortName || null, currency: meta.currency || null,
    price: num(meta.regularMarketPrice), price_date: day(meta.regularMarketTime),
    dividends, splits, years: [], last: null, ttm_total: null, ttm_payments: 0, ttm_yield_pct: null, consecutive_years: 0, as_of: null,
  };
}

export function parseProfile(r, symbol) {
  const ap = r?.assetProfile || {}, price = r?.price || {}, dks = r?.defaultKeyStatistics || {}, sd = r?.summaryDetail || {};
  return {
    symbol,
    name: text(price, "longName") || text(price, "shortName"),
    sector: text(ap, "sector"), industry: text(ap, "industry"),
    summary: text(ap, "longBusinessSummary"),
    employees: raw(ap, "fullTimeEmployees"), founded_year: null,
    website: text(ap, "website"), city: text(ap, "city"), country: text(ap, "country"),
    exchange: text(price, "exchangeName"), currency: text(price, "currency"),
    market_cap: raw(price, "marketCap") ?? raw(sd, "marketCap"),
    shares_outstanding: raw(dks, "sharesOutstanding"),
    directors: (ap.companyOfficers || []).filter((o) => o?.name).map((o) => ({ name: o.name, title: o.title || null })),
    source: "yahoo:quoteSummary",
  };
}

export function parseFundamentals(r, symbol) {
  const ap = r?.assetProfile || {}, price = r?.price || {}, dks = r?.defaultKeyStatistics || {};
  const sd = r?.summaryDetail || {}, fd = r?.financialData || {};
  const marginOrNull = (k) => { const v = pct(fd, k); return v === 0 ? null : v; };
  const de = raw(fd, "debtToEquity");
  return {
    symbol,
    name: text(price, "longName") || text(price, "shortName"),
    // The price's currency only (summaryDetail names it too): financialCurrency
    // is the statements' and has its own field.
    currency: text(price, "currency") || text(sd, "currency"),
    // USD for coal, oil and gas, and airline issuers: Yahoo's P/B, P/S and EV
    // multiples then divide the IDR price by USD figures.
    financial_currency: text(fd, "financialCurrency"),
    sector: text(ap, "sector"), industry: text(ap, "industry"),
    price: raw(fd, "currentPrice") ?? raw(price, "regularMarketPrice"),
    beta: raw(sd, "beta"), week52_high: raw(sd, "fiftyTwoWeekHigh"), week52_low: raw(sd, "fiftyTwoWeekLow"),
    valuation: {
      market_cap: raw(price, "marketCap") ?? raw(sd, "marketCap"),
      enterprise_value: raw(dks, "enterpriseValue"),
      pe_trailing: raw(sd, "trailingPE"), pe_forward: raw(sd, "forwardPE"), peg: raw(dks, "pegRatio"),
      price_to_book: raw(dks, "priceToBook"), price_to_sales: raw(sd, "priceToSalesTrailing12Months"),
      ev_to_ebitda: raw(dks, "enterpriseToEbitda"), ev_to_revenue: raw(dks, "enterpriseToRevenue"),
    },
    profitability: {
      roe_pct: pct(fd, "returnOnEquity"), roa_pct: pct(fd, "returnOnAssets"),
      gross_margin_pct: marginOrNull("grossMargins"), operating_margin_pct: pct(fd, "operatingMargins"),
      net_margin_pct: pct(fd, "profitMargins"),
    },
    health: {
      debt_to_equity: de == null ? null : de / 100, current_ratio: raw(fd, "currentRatio"),
      quick_ratio: raw(fd, "quickRatio"), total_debt: raw(fd, "totalDebt"), total_cash: raw(fd, "totalCash"),
    },
    growth: {
      revenue_growth_pct: pct(fd, "revenueGrowth"), earnings_growth_pct: pct(fd, "earningsGrowth"),
      revenue_cagr_5y_pct: null, eps_growth_3y_pct: null,
    },
    dividend: { yield_pct: pct(sd, "dividendYield"), payout_ratio_pct: pct(sd, "payoutRatio"), ex_dividend_date: dateOf(sd, "exDividendDate") },
    per_share: { eps_ttm: raw(dks, "trailingEps"), eps_forward: raw(dks, "forwardEps"), book_value: raw(dks, "bookValue"), revenue: raw(fd, "revenuePerShare") },
    analyst: {
      recommendation: text(fd, "recommendationKey"), target_mean: raw(fd, "targetMeanPrice"),
      target_high: raw(fd, "targetHighPrice"), target_low: raw(fd, "targetLowPrice"),
      analysts: raw(fd, "numberOfAnalystOpinions"),
    },
    fiscal_year_end: dateOf(dks, "lastFiscalYearEnd"), most_recent_quarter: dateOf(dks, "mostRecentQuarter"),
    verdict: null,
    sources: ["yahoo:quoteSummary"],
  };
}

// earningsTrend rows for this and next quarter and fiscal year (port of
// estimates() in src/providers/yahoo/summary.rs). Counts keep a zero; amounts
// padded as {raw: 0, fmt: null} are null. A ticker no analyst covers still gets
// the four rows with nothing in them; those are left out (has_consensus).
const ESTIMATE_KINDS = { "0q": "quarterly", "+1q": "quarterly", "0y": "annual", "+1y": "annual" };
const ESTIMATE_FIGURES = ["eps_avg", "eps_low", "eps_high", "eps_year_ago", "eps_growth_pct", "revenue_avg", "revenue_low", "revenue_high", "revenue_year_ago", "revenue_growth_pct"];
const hasConsensus = (r) => ESTIMATE_FIGURES.some((k) => r[k] != null) || r.eps_analysts > 0 || r.revenue_analysts > 0;
export function parseEstimates(trend) {
  return (trend?.trend || []).filter((t) => ESTIMATE_KINDS[t?.period]).map((t) => {
    const eps = t.earningsEstimate || {}, rev = t.revenueEstimate || {};
    const growth = pct(eps, "growth");
    const end = typeof t.endDate === "string" && /^\d{4}-\d{2}-\d{2}$/.test(t.endDate.trim()) ? t.endDate.trim() : null;
    return {
      period: t.period, period_type: ESTIMATE_KINDS[t.period], end_date: end,
      eps_avg: amount(eps, "avg"), eps_low: amount(eps, "low"), eps_high: amount(eps, "high"), eps_year_ago: amount(eps, "yearAgoEps"),
      eps_growth_pct: growth ?? pct(t, "growth"), eps_analysts: raw(eps, "numberOfAnalysts"),
      revenue_avg: amount(rev, "avg"), revenue_low: amount(rev, "low"), revenue_high: amount(rev, "high"), revenue_year_ago: amount(rev, "yearAgoRevenue"),
      revenue_growth_pct: pct(rev, "growth"), revenue_analysts: raw(rev, "numberOfAnalysts"),
    };
  }).filter(hasConsensus);
}

export function parseEarnings(r, symbol) {
  const price = r?.price || {}, e = r?.earnings || {}, cal = r?.calendarEvents || {}, trend = r?.earningsTrend || {};
  const label = (x) => (typeof x?.date === "string" ? x.date : x?.date != null ? String(x.date) : "");
  const quarters = (e.earningsChart?.quarterly || []).map((q) => ({
    period: label(q), end_date: dateOf(q, "periodEndDate"), reported_date: dateOf(q, "reportedDate"),
    eps_actual: raw(q, "actual"), eps_estimate: raw(q, "estimate"),
    surprise_pct: typeof q.surprisePct === "string" ? (parseFloat(q.surprisePct) || null) : num(q.surprisePct),
  }));
  const re = (arr) => (arr || []).map((x) => {
    const revenue = amount(x, "revenue");
    let earnings = amount(x, "earnings");
    if (earnings === 0 && revenue) earnings = null;
    return { period: label(x), revenue, earnings, profit_margin_pct: earnings != null && revenue ? pct(x, "profitMargin") : null };
  });
  const ce = cal.earnings || {};
  const cur = (trend.trend || []).find((t) => t.period === "0q");
  const nextDate = ce.earningsDate?.[0]?.raw;
  return {
    symbol, name: text(price, "longName") || text(price, "shortName"),
    currency: text(e, "financialCurrency") || text(price, "currency"),
    estimates: parseEstimates(trend),
    quarters, yearly: re(e.financialsChart?.yearly), quarterly: re(e.financialsChart?.quarterly),
    next: (!cal.earnings && !cur) ? null : {
      date: nextDate ? new Date(nextDate * 1000).toISOString().slice(0, 10) : null,
      date_is_estimate: !!ce.isEarningsDateEstimate,
      eps_avg: amount(ce, "earningsAverage"), eps_low: amount(ce, "earningsLow"), eps_high: amount(ce, "earningsHigh"),
      eps_growth_pct: cur ? pct(cur, "growth") : null,
      revenue_avg: amount(ce, "revenueAverage"), revenue_low: amount(ce, "revenueLow"), revenue_high: amount(ce, "revenueHigh"),
      analysts: cur ? raw(cur.earningsEstimate || {}, "numberOfAnalysts") : null,
    },
    ex_dividend_date: dateOf(cal, "exDividendDate"), dividend_date: dateOf(cal, "dividendDate"),
    source: "yahoo:quoteSummary",
    scope: null,
  };
}

const ASSIGN = {
  TotalRevenue: ["income", "total_revenue"], GrossProfit: ["income", "gross_profit"], OperatingIncome: ["income", "operating_income"],
  EBITDA: ["income", "ebitda"], EBIT: ["income", "ebit"], InterestIncome: ["income", "interest_income"],
  InterestExpense: ["income", "interest_expense"], NetInterestIncome: ["income", "net_interest_income"],
  PretaxIncome: ["income", "pretax_income"], TaxProvision: ["income", "tax_provision"], NetIncome: ["income", "net_income"],
  NetIncomeCommonStockholders: ["income", "net_income_common"], BasicEPS: ["income", "eps_basic"], DilutedEPS: ["income", "eps_diluted"],
  TotalAssets: ["balance", "total_assets"], TotalLiabilitiesNetMinorityInterest: ["balance", "total_liabilities"],
  StockholdersEquity: ["balance", "stockholders_equity"], CashAndCashEquivalents: ["balance", "cash_and_equivalents"],
  TotalDebt: ["balance", "total_debt"], NetDebt: ["balance", "net_debt"], OrdinarySharesNumber: ["balance", "shares_outstanding"],
  OperatingCashFlow: ["cash_flow", "operating"], InvestingCashFlow: ["cash_flow", "investing"], FinancingCashFlow: ["cash_flow", "financing"],
  CapitalExpenditure: ["cash_flow", "capital_expenditure"], FreeCashFlow: ["cash_flow", "free_cash_flow"], CashDividendsPaid: ["cash_flow", "dividends_paid"],
};

export function parseTimeseries(body, symbol, period) {
  const want = period === "quarterly" ? "quarterly" : period === "ttm" ? "trailing" : "annual";
  const periods = new Map();
  let currency = null;
  for (const s of body?.timeseries?.result || []) {
    const type = s?.meta?.type?.[0];
    if (!type || !type.startsWith(want)) continue;
    const item = type.slice(want.length);
    const target = ASSIGN[item];
    if (!target) continue;
    for (const row of s[type] || []) {
      if (!row?.asOfDate || row.reportedValue?.raw == null) continue;
      currency = currency || row.currencyCode || null;
      let p = periods.get(row.asOfDate);
      if (!p) {
        p = { end_date: row.asOfDate, period, income: {}, balance: {}, cash_flow: {} };
        periods.set(row.asOfDate, p);
      }
      p[target[0]][target[1]] = row.reportedValue.raw;
    }
  }
  const list = [...periods.values()].sort((a, b) => a.end_date.localeCompare(b.end_date));
  if (!list.length) return { notFound: true, message: `no ${period} financial statements for ${symbol}` };
  return { symbol, currency, period, periods: list, source: "yahoo:timeseries" };
}
