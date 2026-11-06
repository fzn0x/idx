// MSN Money for the web terminal: the ticker lookup and the insights page,
// ports of src/providers/msn/{parse,insights}.rs with the same public
// web-client key, endpoints and rules, tested on the same tests/fixtures/msn
// files. MSN's crowd sentiment has no IDX data and is not used.
import { fetchWithin } from "./net.js";

export const NAME = "msn";
export const API_KEY = "0QfOX3Vn51YCzitbLaRkTTBadtWpgTN8NZLW0C1SEM";
export const COMMON_QUERY = "cm=en-id&ocid=finance-utils-peregrine&it=web&scn=ANON&wrapodata=false";
export const IDX_MIC = "XIDX";
export const INSIGHTS_URL = "https://api.msn.com/msn/v0/pages/finance/insights";
const UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

export const autosuggestUrl = (symbol) =>
  `https://services.bingapis.com/contentservices-finance.csautosuggest/api/v1/Query?query=${encodeURIComponent(symbol)}&market=en-id`;
export const quotesUrl = (ids) => `https://assets.msn.com/service/Finance/Quotes?ids=${ids.join(",")}&apikey=${API_KEY}&${COMMON_QUERY}`;
export const insightsUrl = (id) => `${INSIGHTS_URL}?ids=${encodeURIComponent(id)}&apikey=${API_KEY}&${COMMON_QUERY}`;

export class MsnError extends Error {
  constructor(message, status) { super(message); this.status = status; }
}

/** GET (or POST with `init.body`) returning parsed JSON; 404 gives null. */
export async function getJson(url, init = {}, { fetchFn } = {}) {
  const r = await fetchWithin(url, { ...init, headers: { "User-Agent": UA, Accept: "application/json", ...(init.headers || {}) } }, fetchFn ? { fetchFn } : {});
  if (r.status === 404) return null;
  if (r.status === 401 || r.status === 403) throw new MsnError("MSN rejected the API key; the key msn.com/money uses has changed", r.status);
  if (!r.ok) throw new MsnError(`MSN answered HTTP ${r.status}`, r.status);
  return r.json();
}

const text = (v, k) => { const s = v?.[k]; return typeof s === "string" && s.trim() ? s.trim() : null; };
const num = (v, k) => { const x = v?.[k]; return typeof x === "number" && Number.isFinite(x) ? x : null; };
const flatten = (v) => (Array.isArray(v) ? v.flatMap(flatten) : v && typeof v === "object" ? [v] : []);
const snake = (s) => s.replace(/[A-Z]/g, (c, i) => (i ? "_" : "") + c.toLowerCase());

/** MSN security id of an IDX ticker, or null. `data.stocks[]` are JSON strings. */
export function parseAutosuggest(body, symbol) {
  for (const raw of body?.data?.stocks || []) {
    let e = raw;
    if (typeof raw === "string") { try { e = JSON.parse(raw); } catch { continue; } }
    const ticker = text(e, "RT00S") || text(e, "OS001");
    const mic = text(e, "ExMicCode") || text(e, "LS01Z");
    if (ticker === symbol && mic === IDX_MIC && text(e, "SecId")) return text(e, "SecId");
  }
  return null;
}

const STATUS = { good: "good", bad: "bad", neutral: "neutral", na: "not_rated" };

function benchmark(item) {
  const id = text(item, "benchmarkObjectId") || "";
  switch (text(item, "benchmarkObjectType")) {
    case "AggregateData": return id.endsWith("peers") ? "peers" : "industry";
    case "Earning": case "History": return "own_history";
    case "Quote": return "price";
    case "Constant": return "threshold";
    default: return null;
  }
}

/**
 * The idx.insights.v1 payload (peers still empty) plus MSN's peer ids and
 * warnings, or null when MSN has nothing (an empty array or the 404 object).
 */
export function parseInsights(body, symbol) {
  const objects = flatten(body).filter((o) => Array.isArray(o.insights));
  const o = objects.find((x) => (text(x, "symbo") || text(x, "symbol")) === symbol) || objects[0];
  if (!o) return null;
  const warnings = [];
  const insights = [];
  for (const it of o.insights) {
    const category = text(it, "category"), name = text(it, "insightName");
    if (!category || !name) continue;
    const d = it.details || {};
    const raw = text(d, "evaluationStatus");
    let status = raw == null ? "not_rated" : STATUS[raw.toLowerCase()];
    if (!status) {
      const w = `MSN rated "${name}" "${raw}", which idx does not know; listed as not_rated`;
      if (!warnings.includes(w)) warnings.push(w);
      status = "not_rated";
    }
    let value = num(d, "propertyValue"), benchmark_value = num(d, "benchmarkValue");
    // MSN fills both figures with 0 when it lacks the data to rate a statement.
    if (status === "not_rated" && value === 0 && benchmark_value === 0) { value = null; benchmark_value = null; }
    const criteria = text(d, "criteria");
    insights.push({
      category, name, statement: text(it, "insightStatement"), short_statement: text(it, "shortInsightStatement"), status,
      criteria: criteria ? snake(criteria) : null, value, benchmark_value, compared_with: benchmark(it),
    });
  }
  const t = text(o, "timeLastUpdated");
  // Whole seconds, like the CLI: MSN sends seven fractional digits.
  const at = t ? new Date(t.replace(/\.\d+/, "")) : null;
  const updated = at && !isNaN(at) ? at.toISOString().replace(/\.\d{3}Z$/, "Z") : null;
  const count = (s) => insights.filter((i) => i.status === s).length;
  const report = {
    symbol, name: text(o, "displayName"), updated,
    good: count("good"), bad: count("bad"), neutral: count("neutral"), not_rated: count("not_rated"),
    peers: [], insights,
  };
  const peerIds = (Array.isArray(o.peers) ? o.peers : []).filter((p) => typeof p === "string" && p.trim()).map((p) => p.trim());
  return { report, peerIds, warnings };
}

/** IDX tickers for MSN peer ids from a Quotes body, in the order of `peerIds`. */
export function parsePeers(body, peerIds) {
  const found = flatten(body)
    .filter((q) => text(q, "exchangeCode") === IDX_MIC && text(q, "symbol"))
    .map((q) => { const i = peerIds.indexOf(text(q, "instrumentId")); return [i < 0 ? Infinity : i, text(q, "symbol").toUpperCase()]; });
  found.sort((a, b) => a[0] - b[0]);
  return [...new Set(found.map(([, s]) => s))];
}
