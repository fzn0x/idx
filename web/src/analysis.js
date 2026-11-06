// Pure analysis shared by the Worker API and (bundled) the browser terminal.
// Ports of src/analysis/{technical,fundamental,screen,sentiment}.rs; the
// rules and thresholds are identical so web and CLI agree.

// ---- indicators ------------------------------------------------------------

export function sma(values, n) {
  if (!n || values.length < n) return null;
  let s = 0;
  for (let i = values.length - n; i < values.length; i++) s += values[i];
  return s / n;
}
export function emaSeries(values, n) {
  if (!n || values.length < n) return [];
  const k = 2 / (n + 1);
  let prev = values.slice(0, n).reduce((a, b) => a + b, 0) / n;
  const out = [prev];
  for (let i = n; i < values.length; i++) { prev = values[i] * k + prev * (1 - k); out.push(prev); }
  return out;
}
export const ema = (v, n) => { const s = emaSeries(v, n); return s.length ? s[s.length - 1] : null; };

export function rsi(closes, n = 14) {
  if (closes.length < n + 1) return null;
  let gain = 0, loss = 0;
  for (let i = 1; i <= n; i++) { const d = closes[i] - closes[i - 1]; if (d > 0) gain += d; else loss -= d; }
  let ag = gain / n, al = loss / n;
  for (let i = n + 1; i < closes.length; i++) {
    const d = closes[i] - closes[i - 1];
    ag = (ag * (n - 1) + (d > 0 ? d : 0)) / n;
    al = (al * (n - 1) + (d < 0 ? -d : 0)) / n;
  }
  if (al === 0) return 100;
  return 100 - 100 / (1 + ag / al);
}
export function macd(closes, fast = 12, slow = 26, signal = 9) {
  if (closes.length < slow + signal) return null;
  const ef = emaSeries(closes, fast), es = emaSeries(closes, slow);
  const off = slow - fast;
  const line = es.map((s, i) => ef[i + off] - s);
  const sig = emaSeries(line, signal);
  const l = line[line.length - 1], s = sig[sig.length - 1];
  return { line: l, signal: s, histogram: l - s };
}
export function bollinger(closes, n = 20, k = 2) {
  const mid = sma(closes, n);
  if (mid == null) return null;
  const tail = closes.slice(-n);
  const sd = Math.sqrt(tail.reduce((a, c) => a + (c - mid) ** 2, 0) / n);
  const upper = mid + k * sd, lower = mid - k * sd, close = closes[closes.length - 1], width = upper - lower;
  return { upper, middle: mid, lower, percent_b: width > 0 ? (close - lower) / width : 0.5, bandwidth_pct: mid ? (width / mid) * 100 : 0 };
}
export function atr(candles, n = 14) {
  if (candles.length < n + 1) return null;
  const tr = [];
  for (let i = 1; i < candles.length; i++) {
    const p = candles[i - 1], c = candles[i];
    tr.push(Math.max(c.high - c.low, Math.abs(c.high - p.close), Math.abs(c.low - p.close)));
  }
  let a = tr.slice(0, n).reduce((x, y) => x + y, 0) / n;
  for (let i = n; i < tr.length; i++) a = (a * (n - 1) + tr[i]) / n;
  return a;
}
function dedupeLevels(v, price) {
  v.sort((a, b) => a - b);
  const tol = Math.abs(price) * 0.005, out = [];
  for (const x of v) if (!out.length || Math.abs(x - out[out.length - 1]) > tol) out.push(x);
  return out;
}
export function levels(candles, lookback = 120, w = 5, price) {
  const slice = candles.slice(-lookback);
  let support = [], resistance = [];
  if (slice.length < 2 * w + 1) return { support, resistance };
  for (let i = w; i < slice.length - w; i++) {
    const c = slice[i];
    const others = slice.slice(i - w, i + w + 1).filter((_, j) => j !== w);
    if (others.every((o) => c.high > o.high) && c.high > price) resistance.push(c.high);
    if (others.every((o) => c.low < o.low) && c.low < price) support.push(c.low);
  }
  support = dedupeLevels(support, price).sort((a, b) => b - a).slice(0, 3);
  resistance = dedupeLevels(resistance, price).sort((a, b) => a - b).slice(0, 3);
  return { support, resistance };
}
const pctChange = (now, then) => (then ? ((now - then) / then) * 100 : null);
function closeDaysAgo(candles, days) {
  const last = candles[candles.length - 1];
  const target = Date.parse(last.time) - days * 86400000;
  for (let i = candles.length - 1; i >= 0; i--) if (Date.parse(candles[i].time) <= target) return candles[i].close;
  return null;
}
export function trend(price, ma, sma50Prev) {
  if (ma.sma50 != null && ma.sma200 != null) {
    const rising = sma50Prev == null ? true : ma.sma50 >= sma50Prev;
    if (price > ma.sma50 && ma.sma50 > ma.sma200 && rising) return "uptrend";
    if (price < ma.sma50 && ma.sma50 < ma.sma200 && !rising) return "downtrend";
    return "sideways";
  }
  if (ma.sma50 != null) {
    const rising = sma50Prev == null ? true : ma.sma50 >= sma50Prev;
    if (price > ma.sma50 * 1.02 && rising) return "uptrend";
    if (price < ma.sma50 * 0.98 && !rising) return "downtrend";
    return "sideways";
  }
  return "unknown";
}

export function technical(h) {
  const candles = h.candles;
  if (!candles.length) return null;
  const closes = candles.map((c) => c.close);
  const last = candles[candles.length - 1], price = last.close;
  const ma = { sma20: sma(closes, 20), sma50: sma(closes, 50), sma200: sma(closes, 200), ema20: ema(closes, 20), ema50: ema(closes, 50), ema200: ema(closes, 200) };
  const sma50Prev = closes.length > 55 ? sma(closes.slice(0, -5), 50) : null;
  const rsi14 = rsi(closes, 14);
  const macdNow = macd(closes), macdPrev = closes.length > 1 ? macd(closes.slice(0, -1)) : null;
  const boll = bollinger(closes), atr14 = atr(candles, 14);
  const vols = candles.map((c) => c.volume);
  const avg20 = sma(vols, 20);
  const volume = { last: last.volume, avg20: avg20 == null ? null : Math.round(avg20), ratio: avg20 ? last.volume / avg20 : null };
  const yearAgo = Date.parse(last.time) - 365 * 86400000;
  const lastYear = candles.filter((c) => Date.parse(c.time) >= yearAgo);
  const high_52w = lastYear.length ? Math.max(...lastYear.map((c) => c.high)) : null;
  const low_52w = lastYear.length ? Math.min(...lastYear.map((c) => c.low)) : null;
  const lv = levels(candles, 120, 5, price);
  const tr = trend(price, ma, sma50Prev);
  let cross = null;
  if (closes.length >= 210) {
    const now = [sma(closes, 50), sma(closes, 200)];
    const before = closes.slice(0, -10);
    const then = [sma(before, 50), sma(before, 200)];
    if (now[0] > now[1] && then[0] <= then[1]) cross = "golden";
    else if (now[0] < now[1] && then[0] >= then[1]) cross = "death";
  }
  const signal = signalOf(price, ma, rsi14, macdNow, macdPrev, boll, tr, cross, volume);
  return {
    symbol: h.symbol, name: h.name, currency: h.currency, as_of: last.time, period: h.period,
    candles_used: candles.length, price,
    change_pct_1d: candles.length >= 2 ? pctChange(price, candles[candles.length - 2].close) : null,
    change_pct_1w: pctChange(price, closeDaysAgo(candles, 7)),
    change_pct_1m: pctChange(price, closeDaysAgo(candles, 30)),
    change_pct_3m: pctChange(price, closeDaysAgo(candles, 91)),
    high_52w, low_52w, moving_averages: ma, rsi14, macd: macdNow, bollinger: boll, atr14, volume, levels: lv, trend: tr, signal,
  };
}
function signalOf(price, ma, r, mNow, mPrev, b, tr, cross, volume) {
  let score = 0; const reasons = [];
  if (tr === "uptrend") { score++; reasons.push("price above a rising 50-day average that sits above the 200-day: uptrend"); }
  else if (tr === "downtrend") { score--; reasons.push("price below a falling 50-day average that sits below the 200-day: downtrend"); }
  else if (tr === "sideways") reasons.push("moving averages disagree: no clear trend");
  else reasons.push("not enough history for the 50/200-day trend read");
  if (r != null) {
    if (r < 30) { score++; reasons.push(`RSI ${r.toFixed(0)} is oversold (<30)`); }
    else if (r > 70) { score--; reasons.push(`RSI ${r.toFixed(0)} is overbought (>70)`); }
    else reasons.push(`RSI ${r.toFixed(0)} is neutral`);
  }
  if (mNow && mPrev) {
    if (mNow.histogram > 0 && mPrev.histogram <= 0) { score++; reasons.push("MACD just crossed above its signal line"); }
    else if (mNow.histogram < 0 && mPrev.histogram >= 0) { score--; reasons.push("MACD just crossed below its signal line"); }
    else if (mNow.histogram > 0) { score++; reasons.push("MACD above its signal line (bullish momentum)"); }
    else if (mNow.histogram < 0) { score--; reasons.push("MACD below its signal line (bearish momentum)"); }
  }
  if (ma.sma200 != null) {
    if (price > ma.sma200) { score++; reasons.push("price above the 200-day average"); }
    else { score--; reasons.push("price below the 200-day average"); }
  }
  if (cross === "golden") { score++; reasons.push("golden cross in the last 10 sessions (50-day crossed above 200-day)"); }
  if (cross === "death") { score--; reasons.push("death cross in the last 10 sessions (50-day crossed below 200-day)"); }
  if (b) {
    if (b.percent_b < 0) { score++; reasons.push("close below the lower Bollinger band (stretched to the downside)"); }
    else if (b.percent_b > 1) { score--; reasons.push("close above the upper Bollinger band (stretched to the upside)"); }
  }
  if (volume.ratio != null && volume.ratio >= 2) reasons.push(`volume ${volume.ratio.toFixed(1)}x its 20-day average: the move has participation`);
  const label = score >= 2 ? "buy" : score <= -2 ? "sell" : "neutral";
  return { label, score, reasons };
}

// ---- verdict ---------------------------------------------------------------

export function isFinancial(f) {
  const s = (f.sector || "").toLowerCase(), i = (f.industry || "").toLowerCase();
  return s.includes("financ") || i.includes("bank") || i.includes("insurance");
}
export function verdict(f) {
  let score = 0, inputs = 0; const reasons = [];
  const v = f.valuation, p = f.profitability, bank = isFinancial(f);
  if (v.pe_trailing != null) {
    inputs++; const pe = v.pe_trailing;
    if (pe <= 0) { score--; reasons.push("negative trailing earnings"); }
    else if (pe < 10) { score += 2; reasons.push(`trailing P/E ${pe.toFixed(1)} is low (under 10)`); }
    else if (pe < 15) { score++; reasons.push(`trailing P/E ${pe.toFixed(1)} is moderate (10 to 15)`); }
    else if (pe > 35) { score -= 2; reasons.push(`trailing P/E ${pe.toFixed(1)} is high (over 35)`); }
    else if (pe > 25) { score--; reasons.push(`trailing P/E ${pe.toFixed(1)} is elevated (over 25)`); }
    else reasons.push(`trailing P/E ${pe.toFixed(1)} is in a normal range`);
  }
  if (v.price_to_book != null) { inputs++; if (v.price_to_book < 1) { score++; reasons.push(`trades below book value (P/B ${v.price_to_book.toFixed(2)})`); } else if (v.price_to_book > 4) { score--; reasons.push(`P/B ${v.price_to_book.toFixed(2)} is rich (over 4)`); } }
  if (v.peg != null) { inputs++; if (v.peg > 0 && v.peg < 1) { score++; reasons.push(`PEG ${v.peg.toFixed(2)} under 1: growth is cheap`); } else if (v.peg > 2) { score--; reasons.push(`PEG ${v.peg.toFixed(2)} over 2: paying up for growth`); } }
  if (p.roe_pct != null) { inputs++; if (p.roe_pct > 15) { score++; reasons.push(`ROE ${p.roe_pct.toFixed(1)}% is strong (over 15%)`); } else if (p.roe_pct < 5) { score--; reasons.push(`ROE ${p.roe_pct.toFixed(1)}% is weak (under 5%)`); } }
  if (p.net_margin_pct != null) { inputs++; if (p.net_margin_pct > 20) { score++; reasons.push(`net margin ${p.net_margin_pct.toFixed(1)}% is wide`); } else if (p.net_margin_pct < 5) { score--; reasons.push(`net margin ${p.net_margin_pct.toFixed(1)}% is thin`); } }
  if (!bank) {
    if (f.health.debt_to_equity != null) { inputs++; const de = f.health.debt_to_equity; if (de > 2) { score--; reasons.push(`debt is ${de.toFixed(1)}x equity: heavily leveraged`); } else if (de < 0.5) { score++; reasons.push(`debt is ${de.toFixed(2)}x equity: conservative balance sheet`); } }
  } else reasons.push("leverage and margin rules relaxed for a financial company");
  const g = f.growth;
  if (g.revenue_growth_pct != null) { inputs++; if (g.revenue_growth_pct > 10) { score++; reasons.push(`revenue growing ${g.revenue_growth_pct.toFixed(1)}% year on year`); } else if (g.revenue_growth_pct < 0) { score--; reasons.push(`revenue shrinking ${(-g.revenue_growth_pct).toFixed(1)}% year on year`); } }
  if (g.earnings_growth_pct != null) { inputs++; if (g.earnings_growth_pct > 10) { score++; reasons.push(`earnings growing ${g.earnings_growth_pct.toFixed(1)}% year on year`); } else if (g.earnings_growth_pct < 0) { score--; reasons.push(`earnings shrinking ${(-g.earnings_growth_pct).toFixed(1)}% year on year`); } }
  if (f.dividend.yield_pct != null) { inputs++; if (f.dividend.yield_pct >= 4) { score++; reasons.push(`dividend yield ${f.dividend.yield_pct.toFixed(1)}% is generous`); } }
  if (f.price != null && f.analyst.target_mean != null && f.price > 0) {
    const up = ((f.analyst.target_mean - f.price) / f.price) * 100;
    if (up > 20) { score++; reasons.push(`analyst mean target implies ${up.toFixed(0)}% upside`); }
    else if (up < -10) { score--; reasons.push(`analyst mean target sits ${(-up).toFixed(0)}% below the price`); }
  }
  const label = inputs < 3 ? "insufficient_data" : score >= 3 ? "undervalued" : score <= -3 ? "overvalued" : "fair";
  return { label, score, reasons, inputs_available: inputs };
}

// ---- screener DSL ----------------------------------------------------------

const KEYS = {
  pe: "pe_trailing", "p/e": "pe_trailing", per: "pe_trailing", fpe: "pe_forward", forwardpe: "pe_forward",
  pb: "price_to_book", pbv: "price_to_book", "p/b": "price_to_book", mcap: "market_cap", marketcap: "market_cap", cap: "market_cap",
  price: "price", close: "price", div: "dividend_yield_pct", yield: "dividend_yield_pct", dividend: "dividend_yield_pct",
  eps: "eps_ttm", volume: "avg_volume_3m", vol: "avg_volume_3m", avgvolume: "avg_volume_3m", chg: "change_pct", change: "change_pct",
  range: "range_52w_pct", pos52: "range_52w_pct", "52w": "range_52w_pct",
  ret1w: "return_1w_pct", ret1m: "return_1m_pct", ret3m: "return_3m_pct", ret6m: "return_6m_pct",
  retytd: "return_ytd_pct", ytd: "return_ytd_pct", ret1y: "return_1y_pct",
  roe: "roe_pct", roa: "roa_pct", margin: "net_margin_pct", npm: "net_margin_pct", opmargin: "operating_margin_pct", opm: "operating_margin_pct",
  de: "debt_to_equity", der: "debt_to_equity", debt: "debt_to_equity", growth: "revenue_growth_pct", revgrowth: "revenue_growth_pct",
  epsgrowth: "earnings_growth_pct", peg: "peg", ps: "price_to_sales", "p/s": "price_to_sales", sector: "sector", industry: "industry",
};
export const FUNDAMENTAL_FIELDS = new Set(["roe_pct","roa_pct","net_margin_pct","operating_margin_pct","debt_to_equity","revenue_growth_pct","earnings_growth_pct","peg","price_to_sales","sector","industry"]);
// The return windows: joined from MSN's whole-market list (one request).
export const RETURN_FIELDS = new Set(["return_1w_pct","return_1m_pct","return_3m_pct","return_6m_pct","return_ytd_pct","return_1y_pct"]);
export const KEY_NAMES = "pe, fpe, pb, ps, peg, mcap, price, div, eps, volume, chg, range, ret1w, ret1m, ret3m, ret6m, retytd, ret1y, roe, roa, margin, opmargin, de, growth, epsgrowth, sector, industry";

// idx's ready-made screens, the same table as analysis::screen::ScreenPreset.
export const PRESETS = {
  "top-performers": { filter: "", sort: "ret1y", descending: true },
  "worst-performers": { filter: "", sort: "ret1y", descending: false },
  "high-dividend": { filter: "div>0", sort: "div", descending: true },
  "low-pe": { filter: "pe>0", sort: "pe", descending: false },
  "52w-high": { filter: "range>=95", sort: "range", descending: true },
  "52w-low": { filter: "range<=5", sort: "range", descending: false },
  "high-volume": { filter: "", sort: "volume", descending: true },
  "large-cap": { filter: "", sort: "mcap", descending: true },
};
export const PRESET_NAMES = Object.keys(PRESETS).join(", ");
/** Preset name (case and _ forgiven) to its canonical name, or a usage error. */
export function parsePreset(raw) {
  const name = String(raw).trim().toLowerCase().replace(/_/g, "-");
  if (!PRESETS[name]) throw new UsageError(`unknown preset "${raw}"; use ${PRESET_NAMES}`);
  return name;
}
/** Where the price sits in its 52-week range, 0 at the low, 100 at the high (not clamped). */
export function rangePosition(price, low, high) {
  return price != null && low != null && high != null && high > low ? ((price - low) / (high - low)) * 100 : null;
}

export function parseNumber(s) {
  let t = String(s).trim().replace(/%$/, "").trim().replace(/[_,]/g, "");
  if (!t) return null;
  const last = t[t.length - 1].toUpperCase();
  const mult = { K: 1e3, M: 1e6, B: 1e9, T: 1e12 }[last];
  if (mult) t = t.slice(0, -1).trim();
  const v = Number(t);
  return Number.isFinite(v) ? v * (mult || 1) : null;
}
export function parseFilters(spec) {
  const out = [];
  for (const clause of String(spec).split(/[,;]/).map((s) => s.trim()).filter(Boolean)) {
    const m = clause.match(/^(.+?)\s*(<=|>=|!=|<|>|=)\s*(.+)$/);
    if (!m) throw new UsageError(`filter "${clause}" needs an operator (<, <=, >, >=, =, !=)`);
    const key = m[1].trim().toLowerCase(), op = m[2], valS = m[3].trim();
    const field = KEYS[key];
    if (!field) throw new UsageError(`unknown filter key "${key}"; use one of ${KEY_NAMES}`);
    const textKey = field === "sector" || field === "industry";
    if (textKey) {
      if (op !== "=" && op !== "!=") throw new UsageError(`${key} only supports = and !=`);
      out.push({ key, field, op, text: valS.replace(/^"|"$/g, ""), raw: clause });
    } else {
      const n = parseNumber(valS);
      if (n == null) throw new UsageError(`filter "${clause}": "${valS}" is not a number (try 15, 2.5B, 10T, 3%)`);
      out.push({ key, field, op, value: n, raw: clause });
    }
  }
  return out;
}
export function sortField(name) { return KEYS[String(name || "mcap").toLowerCase()] || null; }
const NAME_BY_FIELD = Object.fromEntries(KEY_NAMES.split(", ").map((k) => [KEYS[k], k]));
/** Canonical key name, as the CLI reports it (`ytd` -> `retytd`, `pbv` -> `pb`). */
export function keyName(name) { const k = String(name).toLowerCase(); return NAME_BY_FIELD[KEYS[k]] || k; }
export function matches(row, filters) {
  return filters.every((f) => {
    const v = row[f.field];
    if (v == null) return false;
    if (f.text != null) {
      const hit = String(v).toLowerCase().includes(f.text.toLowerCase());
      return f.op === "!=" ? !hit : hit;
    }
    switch (f.op) {
      case "<": return v < f.value; case "<=": return v <= f.value; case ">": return v > f.value; case ">=": return v >= f.value;
      case "=": return Math.abs(v - f.value) < 1e-9; case "!=": return Math.abs(v - f.value) >= 1e-9;
    }
    return false;
  });
}
export function sortRows(rows, field, descending = true) {
  rows.sort((a, b) => {
    const x = a[field], y = b[field];
    if (x == null && y == null) return a.symbol.localeCompare(b.symbol);
    if (x == null) return 1; if (y == null) return -1;
    return descending ? y - x : x - y;
  });
  return rows;
}
export class UsageError extends Error { constructor(m) { super(m); this.code = "usage"; this.exit = 2; } }

// ---- sentiment -------------------------------------------------------------

const POS = ["naik","menguat","melonjak","melesat","meroket","menanjak","rebound","untung","laba","cuan","tumbuh","pertumbuhan","ekspansi","rekor","positif","optimis","optimistis","dividen","akuisisi","buyback","surplus","kenaikan","membaik","pulih","pemulihan","tertinggi","prospektif","kinerja solid","solid","moncer","cemerlang","menebar","beli","akumulasi","upgrade","apresiasi","bullish","gain","gains","rally","rallies","rise","rises","soar","soars","jump","jumps","profit","profits","growth","record","beat","beats","outperform","strong","raised","upgrades","buy","surge","surges","recover","recovery","high"];
const NEG = ["turun","melemah","anjlok","ambles","merosot","tergelincir","rugi","kerugian","koreksi","terkoreksi","jatuh","longsor","tertekan","negatif","pesimis","pesimistis","gagal bayar","default","suspensi","suspend","disuspensi","delisting","denda","sanksi","korupsi","phk","penurunan","terendah","lesu","melambat","perlambatan","jual","tekanan jual","downgrade","rontok","bearish","defisit","penipuan","loss","losses","fall","falls","drop","drops","plunge","plunges","slump","slumps","decline","declines","weak","cut","cuts","downgrades","fraud","lawsuit","miss","misses","sell","selloff","low","probe","investigation"];
export const SENTIMENT_METHOD = "lexicon: (positive - negative) / (positive + negative) per headline over title + summary, Indonesian and English finance terms; mean over matched items";
function normalise(t) { return String(t).toLowerCase().replace(/[^\p{L}\p{N} ]+/gu, " ").split(/\s+/).filter(Boolean).join(" "); }
function hits(norm, lex) { const padded = ` ${norm} `; return lex.filter((w) => padded.includes(` ${w} `)); }
export function scoreText(text) {
  const n = normalise(text), pos = hits(n, POS), neg = hits(n, NEG), total = pos.length + neg.length;
  return { score: total ? (pos.length - neg.length) / total : 0, pos, neg };
}
export function aggregateSentiment(symbol, matchedName, items, headlineLimit = 10) {
  const n = items.length;
  const positive = items.filter((i) => i.sentiment > 0).length, negative = items.filter((i) => i.sentiment < 0).length;
  const score = n ? items.reduce((s, i) => s + i.sentiment, 0) / n : 0;
  const label = !n ? "no_coverage" : score >= 0.15 ? "positive" : score <= -0.15 ? "negative" : "neutral";
  const count = (key) => { const m = new Map(); for (const it of items) for (const t of it[key]) m.set(t, (m.get(t) || 0) + 1); return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 5); };
  const headlines = [...items].sort((a, b) => Math.abs(b.sentiment) - Math.abs(a.sentiment) || String(b.published).localeCompare(String(a.published))).slice(0, headlineLimit);
  const dates = items.map((i) => i.published).filter(Boolean).sort();
  return {
    symbol, matched_name: matchedName, label, score: Math.round(score * 1000) / 1000, items: n, positive, negative, neutral: n - positive - negative,
    oldest: dates[0] || null, newest: dates[dates.length - 1] || null,
    top_positive_terms: count("positive_terms"), top_negative_terms: count("negative_terms"), headlines, method: SENTIMENT_METHOD,
  };
}
