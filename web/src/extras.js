// Ports of src/analysis/{returns,dividends}.rs, the news ordering in
// src/service.rs and the earnings filter in src/model/fundamentals.rs, plus
// the ownership changes summary. Pure; shared by the
// Worker API and tests so the web terminal agrees with the CLI.

const r2 = (v) => Math.round(v * 100) / 100;
const r4 = (v) => { const r = Math.round(v * 1e4) / 1e4; return r === 0 ? 0 : r; };
const dateOnly = (iso) => String(iso).slice(0, 10);
const addMonths = (d, n) => { const x = new Date(d + "T00:00:00Z"); const day = x.getUTCDate(); x.setUTCDate(1); x.setUTCMonth(x.getUTCMonth() + n); const last = new Date(Date.UTC(x.getUTCFullYear(), x.getUTCMonth() + 1, 0)).getUTCDate(); x.setUTCDate(Math.min(day, last)); return x.toISOString().slice(0, 10); };
const addDays = (d, n) => new Date(Date.parse(d + "T00:00:00Z") + n * 86400000).toISOString().slice(0, 10);
const daysBetween = (a, b) => Math.round((Date.parse(b + "T00:00:00Z") - Date.parse(a + "T00:00:00Z")) / 86400000);

// ---- returns ---------------------------------------------------------------

/** [[date, close]] oldest first, one per calendar day. */
export function series(candles) {
  const out = [];
  for (const c of candles) {
    const d = dateOnly(c.time);
    if (out.length && out[out.length - 1][0] === d) out[out.length - 1][1] = c.close;
    else out.push([d, c.close]);
  }
  return out;
}
function closeOnOrBefore(s, target, tolerance) {
  let idx = 0;
  while (idx < s.length && s[idx][0] <= target) idx++;
  if (idx === 0) { if (!s.length) return null; return daysBetween(target, s[0][0]) <= tolerance ? s[0][1] : null; }
  return s[idx - 1][1];
}
const pctFrom = (base, price) => (base != null && base > 0 ? r2((price / base - 1) * 100) : null);

export const WINDOW_LABELS = ["1w", "1m", "3m", "6m", "ytd", "1y", "3y", "5y"];
export function windows(s) {
  if (!s.length) return Object.fromEntries(WINDOW_LABELS.map((k) => [k, null]));
  const [asOf, price] = s[s.length - 1];
  const at = (d, tol) => closeOnOrBefore(s, d, tol);
  return {
    "1w": pctFrom(at(addDays(asOf, -7), 0), price),
    "1m": pctFrom(at(addMonths(asOf, -1), 0), price),
    "3m": pctFrom(at(addMonths(asOf, -3), 0), price),
    "6m": pctFrom(at(addMonths(asOf, -6), 0), price),
    ytd: pctFrom(at(`${asOf.slice(0, 4) - 1}-12-31`, 0), price),
    "1y": pctFrom(at(addMonths(asOf, -12), 7), price),
    "3y": pctFrom(at(addMonths(asOf, -36), 10), price),
    "5y": pctFrom(at(addMonths(asOf, -60), 10), price),
  };
}
export function minusWindows(a, b) {
  return Object.fromEntries(WINDOW_LABELS.map((k) => [k, a[k] != null && b[k] != null ? r2(a[k] - b[k]) : null]));
}
function cagr(s, years) {
  if (!s.length) return null;
  const [asOf, price] = s[s.length - 1];
  const target = addMonths(asOf, -12 * years);
  let idx = 0;
  while (idx < s.length && s[idx][0] <= target) idx++;
  let start;
  if (idx === 0) { if (daysBetween(target, s[0][0]) > 10) return null; start = s[0]; } else start = s[idx - 1];
  const elapsed = daysBetween(start[0], asOf) / 365.25;
  if (start[1] <= 0 || elapsed < 0.5) return null;
  return r2(((price / start[1]) ** (1 / elapsed) - 1) * 100);
}
export function maxDrawdown(s) {
  if (!s.length) return null;
  let peak = s[0], best = null;
  for (const [d, c] of s) {
    if (c > peak[1]) peak = [d, c];
    const dd = (c / peak[1] - 1) * 100;
    if (!best || dd < best[0]) best = [dd, peak[0], d];
  }
  if (!best || best[0] >= 0) return null;
  const peakClose = s.find(([d]) => d === best[1])[1];
  const recovered = s.some(([d, c]) => d > best[2] && c >= peakClose);
  return { pct: r2(best[0]), peak_date: best[1], trough_date: best[2], recovered };
}
export function volatility(s) {
  const r = [];
  for (let i = 1; i < s.length; i++) if (s[i - 1][1] > 0 && s[i][1] > 0) r.push(Math.log(s[i][1] / s[i - 1][1]));
  if (r.length < 20) return null;
  const mean = r.reduce((a, b) => a + b, 0) / r.length;
  const v = r.reduce((a, x) => a + (x - mean) ** 2, 0) / (r.length - 1);
  return r2(Math.sqrt(v) * Math.sqrt(252) * 100);
}
function extremes(s) {
  let best = null, worst = null;
  for (let i = 1; i < s.length; i++) {
    if (s[i - 1][1] <= 0) continue;
    const pct = (s[i][1] / s[i - 1][1] - 1) * 100;
    if (!best || pct > best.pct) best = { date: s[i][0], pct: r2(pct) };
    if (!worst || pct < worst.pct) worst = { date: s[i][0], pct: r2(pct) };
  }
  return [best, worst];
}
/** Mirrors analysis::returns::analyse over an idx.history.v1 payload. */
export function performance(h) {
  const s = series(h.candles);
  if (!s.length) return null;
  const [asOf, price] = s[s.length - 1];
  const yearAgo = addDays(asOf, -365);
  const lastYear = s.filter(([d]) => d > yearAgo);
  const [best, worst] = extremes(lastYear);
  const highs = lastYear.map((x) => x[1]);
  const high52 = highs.length ? Math.max(...highs) : null, low52 = highs.length ? Math.min(...highs) : null;
  const highDate = high52 == null ? null : [...lastYear].reverse().find((x) => x[1] === high52)[0];
  return {
    symbol: h.symbol, name: h.name, currency: h.currency, as_of: asOf, price, start: s[0][0], candles_used: s.length,
    returns: windows(s), cagr_3y_pct: cagr(s, 3), cagr_5y_pct: cagr(s, 5),
    volatility_1y_pct: volatility(lastYear), max_drawdown_1y: maxDrawdown(lastYear), max_drawdown_all: maxDrawdown(s),
    best_day_1y: best, worst_day_1y: worst, high_52w: high52, low_52w: low52,
    pct_from_52w_high: high52 == null ? null : r2((price / high52 - 1) * 100),
    pct_from_52w_low: low52 ? r2((price / low52 - 1) * 100) : null,
    days_since_52w_high: highDate == null ? null : daysBetween(highDate, asOf),
    benchmark: null,
  };
}

// ---- dividends ---------------------------------------------------------------

/** Mirrors analysis::dividends::enrich; `today` is YYYY-MM-DD. */
export function enrichDividends(h, today) {
  h.dividends.sort((a, b) => a.date.localeCompare(b.date));
  h.splits.sort((a, b) => a.date.localeCompare(b.date));
  h.as_of = today;
  h.last = h.dividends.length ? h.dividends[h.dividends.length - 1] : null;
  const byYear = new Map();
  for (const d of h.dividends) { const y = +d.date.slice(0, 4); const e = byYear.get(y) || [0, 0]; e[0]++; e[1] += d.amount; byYear.set(y, e); }
  let prev = null;
  h.years = [...byYear.entries()].sort((a, b) => a[0] - b[0]).map(([year, [payments, total]]) => {
    const change_pct = prev && prev[0] + 1 === year && prev[1] > 0 ? r2(((total - prev[1]) / prev[1]) * 100) : null;
    prev = [year, total];
    return { year, payments, total: r2(total), change_pct };
  });
  const cutoff = addDays(today, -365);
  const ttm = h.dividends.filter((d) => d.date > cutoff && d.date <= today).map((d) => d.amount);
  h.ttm_payments = ttm.length;
  h.ttm_total = ttm.length ? r2(ttm.reduce((a, b) => a + b, 0)) : null;
  h.ttm_yield_pct = h.ttm_total != null && h.price > 0 ? r2((h.ttm_total / h.price) * 100) : null;
  h.consecutive_years = 0;
  if (h.years.length) {
    let year = h.years[h.years.length - 1].year;
    for (let i = h.years.length - 1; i >= 0; i--) { if (h.years[i].year !== year) break; h.consecutive_years++; year--; }
  }
  return h;
}

// ---- earnings filter -----------------------------------------------------------

/**
 * Mirrors EarningsFilter::from_names: period annual | quarterly, kind
 * reported | estimates; empty or "all" keeps both. Throws a RangeError with
 * the CLI's message on anything else.
 */
export function earningsFilter(period, kind) {
  const norm = (v) => (v == null ? "" : String(v).trim().toLowerCase());
  const f = { annual: false, quarterly: false, reported: false, estimates: false };
  const p = norm(period), k = norm(kind);
  if (p === "annual" || p === "quarterly") f[p] = true;
  else if (p && p !== "all") throw new RangeError(`invalid period "${p}"; use annual or quarterly (or leave it out for both)`);
  if (k === "reported" || k === "estimates") f[k] = true;
  else if (k && k !== "all") throw new RangeError(`invalid kind "${k}"; use reported or estimates (or leave it out for both)`);
  return f;
}

/** What a filter keeps, each unset pair widened to both (EarningsFilter::scope). */
export function earningsScope(f) {
  const periods = !f.annual && !f.quarterly, kinds = !f.reported && !f.estimates;
  return { annual: f.annual || periods, quarterly: f.quarterly || periods, reported: f.reported || kinds, estimates: f.estimates || kinds };
}

/** Mirrors EarningsFilter::apply: the rows the filter keeps and data.scope, or e untouched. */
export function filterEarnings(e, f) {
  const s = earningsScope(f);
  if (s.annual && s.quarterly && s.reported && s.estimates) return e;
  const out = { ...e };
  if (!(s.reported && s.quarterly)) { out.quarters = []; out.quarterly = []; }
  if (!(s.reported && s.annual)) out.yearly = [];
  out.estimates = s.estimates ? (e.estimates || []).filter((x) => (x.period_type === "annual" ? s.annual : s.quarterly)) : [];
  if (!(s.estimates && s.quarterly)) out.next = null;
  out.scope = s;
  return out;
}

// ---- news ordering -------------------------------------------------------------

/** Mirrors service::arrange: company items first, then market context up to limit. */
export function arrangeNews(items, symbol, matchedName, limit, companyOnly, itemMatches) {
  if (!symbol) {
    for (const it of items) it.scope = "market";
    return { items: limit > 0 ? items.slice(0, limit) : items, companyMatches: 0 };
  }
  const company = [], market = [];
  for (const it of items) {
    const how = itemMatches(it, symbol, matchedName);
    if (how) { it.matched_by = how; it.scope = "company"; company.push(it); }
    else { it.matched_by = "all"; it.scope = "market"; market.push(it); }
  }
  const companyMatches = company.length;
  let out = limit > 0 ? company.slice(0, limit) : company;
  if (!companyOnly) {
    const room = limit > 0 ? Math.max(0, limit - out.length) : market.length;
    out = out.concat(market.slice(0, room));
  }
  return { items: out, companyMatches };
}

// ---- ownership changes summary ---------------------------------------------------

/** Mirrors ownership::db::summarise over sorted changes (largest |pct_change| first). */
export function summariseChanges(changes) {
  const brief = (c) => ({ symbol: c.symbol, holder: c.holder, kind: c.kind, pct_from: c.pct_from, pct_to: c.pct_to, pct_change: c.pct_change });
  const isForeign = (c) => c.local_foreign === "A";
  const byType = new Map(), byIssuer = new Map();
  for (const c of changes) {
    const t = c.investor_type && c.investor_type.trim() ? c.investor_type : "UNKNOWN";
    const e = byType.get(t) || { rows: 0, net: 0 }; e.rows++; e.net += c.pct_change; byType.set(t, e);
    const i = byIssuer.get(c.symbol) || { rows: 0, net: 0, foreign: 0 }; i.rows++; i.net += c.pct_change; if (isForeign(c)) i.foreign += c.pct_change; byIssuer.set(c.symbol, i);
  }
  const sum = (pred) => r4(changes.filter(pred).reduce((s, c) => s + c.pct_change, 0));
  return {
    issuers: new Set(changes.map((c) => c.symbol)).size,
    holders: new Set(changes.map((c) => c.entity_key)).size,
    new_holders: changes.filter((c) => c.kind === "new").length,
    exits: changes.filter((c) => c.kind === "exited").length,
    increased: changes.filter((c) => c.kind === "changed" && c.pct_change > 0).length,
    decreased: changes.filter((c) => c.kind === "changed" && c.pct_change < 0).length,
    net_pp: sum(() => true),
    foreign_net_pp: sum(isForeign),
    local_net_pp: sum((c) => c.local_foreign === "L"),
    by_investor_type: [...byType.entries()].map(([investor_type, e]) => ({ investor_type, rows: e.rows, net_pp: r4(e.net) })).sort((a, b) => Math.abs(b.net_pp) - Math.abs(a.net_pp)),
    issuer_flow: [...byIssuer.entries()].map(([symbol, e]) => ({ symbol, rows: e.rows, net_pp: r4(e.net), foreign_net_pp: r4(e.foreign) })).sort((a, b) => Math.abs(b.net_pp) - Math.abs(a.net_pp) || a.symbol.localeCompare(b.symbol)).slice(0, 10),
    top_increases: changes.filter((c) => c.pct_change > 0).slice(0, 5).map(brief),
    top_decreases: changes.filter((c) => c.pct_change < 0).slice(0, 5).map(brief),
  };
}
