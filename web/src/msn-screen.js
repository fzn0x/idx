// MSN's whole-market screener list for the web terminal: the port of
// src/providers/msn/screener.rs. One POST returns every Indonesian stock's
// price returns (1 week to 1 year, YTD) for the screen's ret* keys.
// MSN's own preset lists are stale for IDX and not used.
import { API_KEY, COMMON_QUERY, IDX_MIC } from "./msn-insights.js";

export const SCREENER_URL = "https://assets.msn.com/service/Finance/Screener";
export const MARKET_LIST_BODY = '{"filter":[{"key":"st_reg_id","keyGroup":"st_reg_","isRange":false}],"returnValueType":["quote"],"screenerType":"stock","limit":2000,"pageIndex":0}';
export const SOURCE = "msn:screener";
export const screenerUrl = () => `${SCREENER_URL}?apikey=${API_KEY}&${COMMON_QUERY}`;

const num = (v, k) => { const x = v?.[k]; return typeof x === "number" && Number.isFinite(x) ? x : null; };
const WINDOWS = ["return1Week", "return1Month", "return3Month", "return6Month", "returnYTD", "return1Year"];

/**
 * Compact list for the D1 cache: { count, listed, returns: { BBCA: [w1, m1, m3, m6, ytd, y1], ... } }.
 * IDX listings only; windows MSN does not report are null.
 */
export function parseMarketList(body) {
  const quotes = Array.isArray(body?.quote) ? body.quote : [];
  const returns = {};
  for (const q of quotes) {
    if (q?.exchangeCode !== IDX_MIC || typeof q.symbol !== "string" || !q.symbol.trim()) continue;
    const s = q.symbol.trim().toUpperCase();
    if (!(s in returns)) returns[s] = WINDOWS.map((k) => num(q, k));
  }
  return { count: typeof body?.count === "number" ? body.count : null, listed: quotes.length, returns };
}

export const RETURN_FIELDS_ORDER = ["return_1w_pct", "return_1m_pct", "return_3m_pct", "return_6m_pct", "return_ytd_pct", "return_1y_pct"];

/** Copies one stock's windows from the compact list into a screen row. */
export function applyReturns(row, windows) {
  RETURN_FIELDS_ORDER.forEach((f, i) => { row[f] = windows ? windows[i] : null; });
  return row;
}
