// Pure analysis shared by the Worker API and (bundled) the browser terminal.
// Ports of src/analysis/{technical,fundamental,screen,sentiment}.rs; the
// rules and thresholds are identical so web and CLI agree. The lenses also
// port src/analysis/lens.rs and use the price statistics of extras.js.

import * as ex from "./extras.js";

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
// Shareholders' equity below zero, read from the signs the sources report:
// true when book value per share, P/B or debt/equity is negative, false when
// one of them is reported and none is, null when none is. Port of
// Fundamentals::negative_equity.
export function negativeEquity(f) {
  const signs = [f.per_share?.book_value, f.valuation?.price_to_book, f.health?.debt_to_equity].filter((x) => x != null);
  return signs.length ? signs.some((x) => x < 0) : null;
}
// Statements in another currency than the price, both known (USD statements,
// IDR price). Port of Fundamentals::mixed_currency.
export function mixedCurrency(f) {
  return !!(f.currency && f.financial_currency) && f.currency.toUpperCase() !== f.financial_currency.toUpperCase();
}
export const NEGATIVE_EQUITY_REASON = "negative shareholders' equity (book value below zero): P/B, ROE and debt/equity rules skipped";
export function verdict(f) {
  let score = 0, inputs = 0; const reasons = [];
  const v = f.valuation, p = f.profitability, bank = isFinancial(f);
  // Negative equity scores -2 and sets P/B, ROE and debt/equity aside (they
  // still count as inputs); a P/B over statements in another currency is
  // neither scored, counted nor listed as a reason.
  const neg = negativeEquity(f) === true;
  const mixed = mixedCurrency(f);
  if (v.pe_trailing != null) {
    inputs++; const pe = v.pe_trailing;
    if (pe <= 0) { score--; reasons.push("negative trailing earnings"); }
    else if (pe < 10) { score += 2; reasons.push(`trailing P/E ${pe.toFixed(1)} is low (under 10)`); }
    else if (pe < 15) { score++; reasons.push(`trailing P/E ${pe.toFixed(1)} is moderate (10 to 15)`); }
    else if (pe > 35) { score -= 2; reasons.push(`trailing P/E ${pe.toFixed(1)} is high (over 35)`); }
    else if (pe > 25) { score--; reasons.push(`trailing P/E ${pe.toFixed(1)} is elevated (over 25)`); }
    else reasons.push(`trailing P/E ${pe.toFixed(1)} is in a normal range`);
  }
  if (neg) { inputs++; score -= 2; reasons.push(NEGATIVE_EQUITY_REASON); }
  if (v.price_to_book != null) {
    if (neg) inputs++;
    else if (!mixed) { inputs++; if (v.price_to_book < 1) { score++; reasons.push(`trades below book value (P/B ${v.price_to_book.toFixed(2)})`); } else if (v.price_to_book > 4) { score--; reasons.push(`P/B ${v.price_to_book.toFixed(2)} is rich (over 4)`); } }
  }
  if (v.peg != null) { inputs++; if (v.peg > 0 && v.peg < 1) { score++; reasons.push(`PEG ${v.peg.toFixed(2)} under 1: growth is cheap`); } else if (v.peg > 2) { score--; reasons.push(`PEG ${v.peg.toFixed(2)} over 2: paying up for growth`); } }
  if (p.roe_pct != null) {
    inputs++;
    const loss = (p.net_margin_pct != null && p.net_margin_pct < 0) || (f.per_share?.eps_ttm != null && f.per_share.eps_ttm < 0);
    if (!neg) {
      if (p.roe_pct > 15 && loss) reasons.push(`ROE ${p.roe_pct.toFixed(1)}% alongside a net loss: rule skipped`);
      else if (p.roe_pct > 15) { score++; reasons.push(`ROE ${p.roe_pct.toFixed(1)}% is strong (over 15%)`); }
      else if (p.roe_pct < 5) { score--; reasons.push(`ROE ${p.roe_pct.toFixed(1)}% is weak (under 5%)`); }
    }
  }
  if (p.net_margin_pct != null) { inputs++; if (p.net_margin_pct > 20) { score++; reasons.push(`net margin ${p.net_margin_pct.toFixed(1)}% is wide`); } else if (p.net_margin_pct < 5) { score--; reasons.push(`net margin ${p.net_margin_pct.toFixed(1)}% is thin`); } }
  if (!bank) {
    if (f.health.debt_to_equity != null) { inputs++; const de = f.health.debt_to_equity; if (neg) { /* set aside above */ } else if (de > 2) { score--; reasons.push(`debt is ${de.toFixed(1)}x equity: heavily leveraged`); } else if (de < 0.5) { score++; reasons.push(`debt is ${de.toFixed(2)}x equity: conservative balance sheet`); } }
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
// The words the CLI prints for each verdict label (VerdictLabel::as_str);
// the JSON keeps the label itself.
export const VERDICT_WORDS = { undervalued: "undervalued", fair: "fairly valued", overvalued: "overvalued", insufficient_data: "insufficient data" };

// ---- growth, valuation and risk lenses ---------------------------------------
// Ports of src/analysis/lens.rs: same rule ids, points, reasons, labels and
// rounding. Valuation and growth points favour cheapness and growth; risk
// points add risk. Every rule that ran is in `rules`, every rule that could
// not run is in `notes`.

export const LENS_BENCHMARK = "^JKSE";
const LENS_MIN_RULES = 3;
const LENS_MIN_PRICE_RETURNS = 20;
const lr2 = (v) => Math.round(v * 100) / 100 + 0;
/**
 * `v.toFixed(d)` as Rust's `{:.d}` prints it: an exact tie (a double exactly
 * halfway at `d` decimals, such as 12.5, 4.25 or 0.125) rounds to even, where
 * toFixed rounds it away from zero. A double is such a tie exactly when
 * |v| * 2^(d+1) is an odd integer; |v| * 10^d is then exact.
 */
export function fixed(v, d) {
  const m = Math.abs(v) * 2 ** (d + 1);
  if (!Number.isInteger(m) || m % 2 === 0) return v.toFixed(d);
  const n = Math.floor(Math.abs(v) * 10 ** d);
  const even = n % 2 === 0 ? n : n + 1;
  return `${v < 0 ? "-" : ""}${(even / 10 ** d).toFixed(d)}`;
}
const signed = (v, d) => `${v >= 0 ? "+" : ""}${fixed(v, d)}`;
const nz = (v) => (v === undefined ? null : v);

function lensRules() {
  const r = { rules: [], notes: [] };
  r.add = (rule, points, reason) => r.rules.push({ rule, points, reason });
  r.note = (n) => r.notes.push(n);
  return r;
}
function lensReading(r, label) {
  const score = r.rules.reduce((s, x) => s + x.points, 0);
  return { label: label(score, r.rules), score, inputs_available: r.rules.length, rules: r.rules, notes: r.notes };
}
const currencyPair = (f) => (mixedCurrency(f) ? `statements in ${f.financial_currency}, price in ${f.currency}` : null);

/** Mirrors analysis::lens::valuation: an idx.valuation.v1 payload. */
export function valuationLens(f) {
  const r = lensRules();
  const v = f.valuation || {}, d = f.dividend || {}, a = f.analyst || {};
  const financial = isFinancial(f), neg = negativeEquity(f), mixed = currencyPair(f);
  const pe = nz(v.pe_trailing), pf = nz(v.pe_forward);
  if (pe != null) {
    const [p, why] = pe <= 0 ? [0, "is not positive: it says nothing about value"] : pe < 10 ? [2, "is low (under 10)"] : pe < 15 ? [1, "is moderate (10 to 15)"] : pe > 35 ? [-2, "is high (over 35)"] : pe > 25 ? [-1, "is elevated (over 25)"] : [0, "is in a normal range (15 to 25)"];
    r.add("pe_trailing", p, `trailing P/E ${fixed(pe, 1)} ${why}`);
  } else if (pf != null && pf > 0) {
    const [p, why] = pf < 10 ? [1, "is low (under 10)"] : pf > 25 ? [-1, "is high (over 25)"] : [0, "is in a normal range (10 to 25)"];
    r.add("pe_forward", p, `forward P/E ${fixed(pf, 1)} ${why}; no trailing P/E`);
  } else if (pf != null) r.note(`P/E: no trailing P/E, and the forward P/E ${fixed(pf, 1)} is not positive`);
  else r.note("P/E: no trailing or forward P/E reported");

  const peg = nz(v.peg);
  if (peg != null && peg > 0) {
    const [p, why] = peg < 1 ? [1, "is under 1: growth is cheap"] : peg > 2 ? [-1, "is over 2: paying up for growth"] : [0, "is between 1 and 2"];
    r.add("peg", p, `PEG ${fixed(peg, 2)} ${why}`);
  } else if (peg != null) r.note(`PEG ${fixed(peg, 2)} is not meaningful (shrinking or negative earnings)`);
  else r.note("PEG: not reported");

  const pb = nz(v.price_to_book);
  if (neg === true) r.note("P/B: shareholders' equity is negative, so price-to-book says nothing");
  else if (pb == null) r.note("P/B: not reported");
  else if (mixed) r.note(`P/B ${fixed(pb, 2)} not scored: ${mixed}`);
  else {
    const [p, why] = pb < 1 ? [1, "is below book value (under 1)"] : pb > 4 ? [-1, "is rich (over 4)"] : [0, "is between 1 and 4"];
    r.add("price_to_book", p, `P/B ${fixed(pb, 2)} ${why}`);
  }

  if (financial) r.note("P/S and EV/EBITDA: not scored for a financial company");
  else {
    const ps = nz(v.price_to_sales);
    if (ps == null) r.note("P/S: not reported");
    else if (mixed) r.note(`P/S ${fixed(ps, 2)} not scored: ${mixed}`);
    else {
      const [p, why] = ps < 1 ? [1, "is low (under 1)"] : ps > 5 ? [-1, "is high (over 5)"] : [0, "is between 1 and 5"];
      r.add("price_to_sales", p, `P/S ${fixed(ps, 2)} ${why}`);
    }
    const ev = nz(v.ev_to_ebitda);
    if (ev == null) r.note("EV/EBITDA: not reported");
    else if (mixed) r.note(`EV/EBITDA ${fixed(ev, 1)} not scored: ${mixed}`);
    else if (ev <= 0) r.note(`EV/EBITDA ${fixed(ev, 1)} is not meaningful (negative EBITDA or enterprise value)`);
    else {
      const [p, why] = ev < 6 ? [1, "is low (under 6)"] : ev > 15 ? [-1, "is high (over 15)"] : [0, "is between 6 and 15"];
      r.add("ev_to_ebitda", p, `EV/EBITDA ${fixed(ev, 1)} ${why}`);
    }
  }

  const y = nz(d.yield_pct), po = nz(d.payout_ratio_pct);
  if (y != null && y >= 4) {
    if (po != null && po > 100) r.add("dividend_yield", 0, `dividend yield ${fixed(y, 1)}% is high, but the payout (${fixed(po, 0)}%) exceeds earnings`);
    else r.add("dividend_yield", 1, `dividend yield ${fixed(y, 1)}% is generous (4% or more)`);
  } else if (y != null) r.add("dividend_yield", 0, `dividend yield ${fixed(y, 1)}% is under 4%`);
  else r.note("dividend yield: not reported");

  const price = nz(f.price), target = nz(a.target_mean), n = nz(a.analysts);
  const upside = price != null && target != null && price > 0 ? lr2((target / price - 1) * 100) : null;
  if (upside != null && n != null && n >= 3) {
    const [p, why] = upside > 20 ? [1, "over +20%"] : upside < -10 ? [-1, "under -10%"] : [0, "-10% to +20%"];
    r.add("analyst_target", p, `mean target of ${n} analysts is ${signed(upside, 0)}% from the price (${why})`);
  } else if (upside != null) {
    const who = n === 1 ? "1 analyst" : n != null ? `${n} analysts` : "an unknown number of analysts";
    r.note(`analyst target ${signed(upside, 0)}% from the price, from ${who}: fewer than 3, not scored`);
  } else r.note("analyst target: no mean target or price");

  const reading = lensReading(r, (score, rules) => (rules.length < LENS_MIN_RULES ? "insufficient_data" : score >= 3 ? "cheap" : score <= -3 ? "expensive" : "fair"));
  return {
    symbol: f.symbol, name: nz(f.name), currency: nz(f.currency), financial_currency: nz(f.financial_currency),
    financial_company: financial, negative_equity: neg, price,
    pe_trailing: pe, pe_forward: pf, peg, price_to_book: pb, price_to_sales: nz(v.price_to_sales),
    ev_to_ebitda: nz(v.ev_to_ebitda), ev_to_revenue: nz(v.ev_to_revenue),
    dividend_yield_pct: y, payout_ratio_pct: po, target_mean: target, analysts: n, target_upside_pct: upside,
    reading, sources: [...(f.sources || [])],
  };
}

function growthBand(v, hi, lo) {
  const s = (x) => (x > 0 ? `+${x}` : `${x}`);
  return v > hi ? [1, `over ${s(hi)}%`] : v < lo ? [-1, `below ${s(lo)}%`] : [0, `${s(lo)}% to ${s(hi)}%`];
}
const lensCagr = (first, last, years) => (first > 0 && last > 0 && years > 0 ? lr2(((last / first) ** (1 / years) - 1) * 100) : null);
const changePct = (prev, cur) => (prev != null && cur != null && prev > 0 ? lr2((cur / prev - 1) * 100) : null);

/** Mirrors analysis::lens::growth: an idx.growth.v1 payload. `f` or `fin` may be null. */
export function growthLens(symbol, f, fin) {
  const r = lensRules();
  const g = f?.growth || {}, ps = f?.per_share || {};
  for (const [rule, what, v] of [["revenue_growth", "revenue", nz(g.revenue_growth_pct)], ["earnings_growth", "earnings", nz(g.earnings_growth_pct)]]) {
    if (v == null) { r.note(`${what} growth in the latest quarter: not reported`); continue; }
    const [p, band] = growthBand(v, 10, 0);
    r.add(rule, p, `${what} ${signed(v, 1)}% year on year in the latest quarter (${band})${Math.abs(v) > 100 ? ", from a small or negative base" : ""}`);
  }
  const et = nz(ps.eps_ttm), ef = nz(ps.eps_forward);
  const fg = et != null && ef != null && et > 0 && ef > 0 ? lr2((ef / et - 1) * 100) : null;
  if (fg != null) {
    const [p, band] = growthBand(fg, 10, -10);
    r.add("eps_forward_growth", p, `forward EPS ${fixed(ef, 2)} is ${signed(fg, 1)}% against trailing ${fixed(et, 2)} (${band})`);
  } else if (et != null && ef != null) r.note(`forward vs trailing EPS: needs both above zero (trailing ${fixed(et, 2)}, forward ${fixed(ef, 2)})`);
  else r.note("forward vs trailing EPS: not reported");

  const periods = fin?.periods || [];
  const annual = periods.map((p, i) => {
    const prev = i > 0 ? periods[i - 1].income || {} : {};
    const inc = p.income || {};
    return {
      end_date: p.end_date, revenue: nz(inc.total_revenue), net_income: nz(inc.net_income),
      revenue_change_pct: changePct(nz(prev.total_revenue), nz(inc.total_revenue)),
      net_income_change_pct: changePct(nz(prev.net_income), nz(inc.net_income)),
    };
  });
  const yearOf = (d) => +String(d).slice(0, 4);
  const revenue = annual.filter((y) => y.revenue != null).map((y) => [yearOf(y.end_date), y.revenue]);
  const income = annual.filter((y) => y.net_income != null).map((y) => [yearOf(y.end_date), y.net_income]);

  let revenueCagr = null;
  if (revenue.length >= 3) {
    const [[a, first], [b, last]] = [revenue[0], revenue[revenue.length - 1]];
    revenueCagr = lensCagr(first, last, b - a);
    if (revenueCagr != null) { const [p, band] = growthBand(revenueCagr, 10, 0); r.add("revenue_cagr", p, `revenue CAGR ${signed(revenueCagr, 1)}% over FY${a} to FY${b} (${band})`); }
    else r.note(`revenue CAGR: needs revenue above zero in FY${a} and FY${b}`);
    const k = revenue.length - 1;
    let ups = 0;
    for (let i = 1; i < revenue.length; i++) if (revenue[i][1] > revenue[i - 1][1]) ups++;
    if (ups === k) r.add("revenue_consistency", 1, `revenue grew in each of the last ${k} fiscal years`);
    else if (ups * 2 < k) r.add("revenue_consistency", -1, `revenue grew in only ${ups} of the last ${k} fiscal years (under half)`);
    else r.add("revenue_consistency", 0, `revenue grew in ${ups} of the last ${k} fiscal years`);
  } else {
    const c = nz(g.revenue_cagr_5y_pct);
    if (c != null) { const [p, band] = growthBand(c, 10, 0); r.add("revenue_cagr_5y", p, `revenue 5-year growth ${signed(c, 1)}% a year, as reported (${band})`); }
    else r.note("revenue CAGR: fewer than three fiscal years and no 5-year figure from the source");
    r.note("revenue consistency: fewer than three fiscal years");
  }

  let incomeCagr = null;
  if (income.length >= 3) {
    const [[a, first], [b, last]] = [income[0], income[income.length - 1]];
    incomeCagr = lensCagr(first, last, b - a);
    if (incomeCagr != null) { const [p, band] = growthBand(incomeCagr, 10, 0); r.add("net_income_cagr", p, `net income CAGR ${signed(incomeCagr, 1)}% over FY${a} to FY${b} (${band})`); }
    else if (first > 0) r.add("net_income_cagr", -1, `net income turned from a profit in FY${a} to a loss in FY${b}`);
    else if (last > 0) r.add("net_income_cagr", 1, `net income turned from a loss in FY${a} to a profit in FY${b}`);
    else r.note(`net income CAGR: losses in both FY${a} and FY${b}`);
  } else {
    const c = nz(g.eps_growth_3y_pct);
    if (c != null) { const [p, band] = growthBand(c, 10, 0); r.add("eps_growth_3y", p, `EPS 3-year growth ${signed(c, 1)}% a year, as reported (${band})`); }
    else r.note("net income CAGR: fewer than three fiscal years and no 3-year EPS figure from the source");
  }

  const reading = lensReading(r, (score, rules) => (rules.length < LENS_MIN_RULES ? "insufficient_data" : score >= 2 ? "growing" : score <= -2 ? "shrinking" : rules.some((x) => x.points > 0) && rules.some((x) => x.points < 0) ? "mixed" : "stable"));
  const sources = [...(f?.sources || [])];
  if (fin?.source && !sources.includes(fin.source)) sources.push(fin.source);
  return {
    symbol, name: nz(f?.name), statements_currency: nz(fin?.currency), most_recent_quarter: nz(f?.most_recent_quarter),
    revenue_growth_pct: nz(g.revenue_growth_pct), earnings_growth_pct: nz(g.earnings_growth_pct),
    revenue_cagr_5y_pct: nz(g.revenue_cagr_5y_pct), eps_growth_3y_pct: nz(g.eps_growth_3y_pct),
    eps_ttm: et, eps_forward: ef, eps_forward_growth_pct: fg, annual,
    revenue_cagr_pct: revenueCagr, net_income_cagr_pct: incomeCagr, reading, sources,
  };
}

/** Mirrors analysis::lens::risk: an idx.risk.v1 payload. `h` is the stock's history, `b` IHSG's; any may be null. */
export function riskLens(symbol, f, h, b) {
  const r = lensRules();
  const neg = f ? negativeEquity(f) : null;
  const financial = !!f && isFinancial(f);
  if (!f) r.note("balance sheet and earnings: fundamentals unavailable");
  else {
    if (neg === true) r.add("negative_equity", 2, "shareholders' equity is negative (book value below zero): liabilities exceed assets");
    if (financial) r.note("leverage and liquidity: not scored for a financial company");
    else {
      const de = nz(f.health?.debt_to_equity), cr = nz(f.health?.current_ratio);
      if (neg === true) r.note("leverage: debt/equity says nothing with negative equity");
      else if (de != null) { const [p, why] = de > 2 ? [1, "over 2x"] : de < 0.5 ? [-1, "under 0.5x"] : [0, "0.5x to 2x"]; r.add("leverage", p, `debt is ${fixed(de, 2)}x equity (${why})`); }
      else r.note("leverage: debt/equity not reported");
      if (cr != null) { const [p, why] = cr < 1 ? [1, "is under 1"] : cr >= 1.5 ? [-1, "is 1.5 or more"] : [0, "is between 1 and 1.5"]; r.add("liquidity", p, `current ratio ${fixed(cr, 2)} ${why}`); }
      else r.note("liquidity: current ratio not reported");
    }
    const nm = nz(f.profitability?.net_margin_pct), eps = nz(f.per_share?.eps_ttm);
    const detail = [nm != null ? `net margin ${fixed(nm, 1)}%` : null, eps != null ? `EPS ${fixed(eps, 2)}` : null].filter(Boolean);
    if (!detail.length) r.note("losses: no net margin or EPS reported");
    else if ((nm != null && nm < 0) || (eps != null && eps < 0)) r.add("losses", 1, `loss-making over the trailing twelve months (${detail.join(", ")})`);
    else r.add("losses", 0, `profitable over the trailing twelve months (${detail.join(", ")})`);
  }

  const own = h ? ex.series(h.candles || []) : [];
  const benchmark = b?.symbol || LENS_BENCHMARK;
  let price = null;
  if (!own.length) r.note("price risk: no daily candles for the stock");
  else {
    const [cs, cb] = ex.common(own, b ? ex.series(b.candles || []) : []);
    if (cs.length < 2) {
      r.note(b ? `beta, volatility and drawdown against IHSG: no sessions in common with ${benchmark}` : `beta, volatility and drawdown against IHSG: benchmark ${benchmark} unavailable`);
      price = { benchmark, from: own[0][0], to: own[own.length - 1][0], observations: 0, zero_returns: null, beta: null, correlation: null, volatility_pct: ex.volatility(own), benchmark_volatility_pct: null, max_drawdown: ex.maxDrawdown(own), benchmark_max_drawdown_pct: null };
    } else {
      const st = ex.beta(cs, cb), n = st.observations;
      // Over the same pairs as the beta: the stock's close equals the one before.
      let zero = 0;
      for (let i = 1; i < Math.min(cs.length, cb.length); i++) if (cs[i - 1][1] > 0 && cb[i - 1][1] > 0 && cs[i][1] === cs[i - 1][1]) zero++;
      const vs = ex.volatility(cs), vb = ex.volatility(cb);
      const ds = ex.maxDrawdown(cs), db = ex.maxDrawdown(cb)?.pct ?? null;
      if (n >= LENS_MIN_PRICE_RETURNS && zero * 2 > n) {
        // Suspended, stuck at the floor price or barely traded: a flat price would look calm on all three.
        r.add("illiquidity", 1, `${zero} of ${n} daily returns are zero (more than half): the stock was suspended or barely traded`);
        r.note(`beta, volatility and drawdown: not scored, ${zero} of ${n} daily returns are zero`);
      } else {
        if (st.beta != null) { const bt = st.beta; const [p, why] = bt > 1.3 ? [1, "over 1.3"] : bt < 0.7 ? [-1, "under 0.7"] : [0, "0.7 to 1.3"]; r.add("beta", p, `beta ${fixed(bt, 2)} against IHSG over ${n} daily returns (${why})`); }
        else if (n < ex.MIN_BETA_OBSERVATIONS) r.note(`beta: ${n} matched daily returns with IHSG, needs ${ex.MIN_BETA_OBSERVATIONS}`);
        else r.note("beta: IHSG did not move over the window");
        if (vs != null && vb != null && vb > 0) {
          const rel = vs / vb;
          const [p, why] = rel > 1.5 ? [1, "over 1.5x"] : rel < 0.8 ? [-1, "under 0.8x"] : [0, "0.8x to 1.5x"];
          r.add("volatility", p, `volatility ${fixed(vs, 1)}% a year is ${fixed(rel, 2)}x IHSG's ${fixed(vb, 1)}% (${why})`);
        } else r.note(`volatility against IHSG: fewer than ${LENS_MIN_PRICE_RETURNS} matched daily returns`);
        if (cs.length > LENS_MIN_PRICE_RETURNS) {
          const s = ds ? ds.pct : 0, bb = db ?? 0, gap = s - bb;
          const [p, why] = gap <= -15 ? [1, "15 points or more deeper"] : gap >= 15 ? [-1, "15 points or more shallower"] : [0, "within 15 points"];
          r.add("drawdown", p, `largest fall ${fixed(s, 1)}% against IHSG's ${fixed(bb, 1)}% over the same sessions (${why})`);
        } else r.note(`drawdown against IHSG: fewer than ${LENS_MIN_PRICE_RETURNS} matched daily returns`);
      }
      price = { benchmark, from: cs[0][0], to: cs[cs.length - 1][0], observations: n, zero_returns: zero, beta: st.beta, correlation: st.correlation, volatility_pct: vs, benchmark_volatility_pct: vb, max_drawdown: ds, benchmark_max_drawdown_pct: db };
    }
  }

  const reading = lensReading(r, (score, rules) => (rules.length < LENS_MIN_RULES ? "insufficient_data" : score >= 3 ? "high" : score <= -2 ? "low" : "moderate"));
  return {
    symbol, name: nz(f?.name) ?? nz(h?.name), currency: nz(f?.currency) ?? nz(h?.currency),
    financial_company: financial, negative_equity: neg,
    debt_to_equity: nz(f?.health?.debt_to_equity), current_ratio: nz(f?.health?.current_ratio), quick_ratio: nz(f?.health?.quick_ratio),
    net_margin_pct: nz(f?.profitability?.net_margin_pct), eps_ttm: nz(f?.per_share?.eps_ttm),
    price, reading, sources: [...(f?.sources || [])],
  };
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

/** A screen row (idx.screen.v1) from a board quote: analysis::screen::row_from_quote. */
export function rowFromQuote(q, nameHint = null) {
  return {
    symbol: q.symbol, name: q.name || nameHint || null, currency: q.currency ?? null, financial_currency: q.financial_currency ?? null,
    price: q.price, change_pct: q.change_pct, market_cap: q.market_cap,
    pe_trailing: q.pe_trailing, pe_forward: q.pe_forward, price_to_book: q.price_to_book, dividend_yield_pct: q.dividend_yield_pct,
    eps_ttm: q.eps_ttm, avg_volume_3m: q.avg_volume_3m, week52_high: q.week52_high, week52_low: q.week52_low,
    range_52w_pct: rangePosition(q.price, q.week52_low, q.week52_high),
    return_1w_pct: null, return_1m_pct: null, return_3m_pct: null, return_6m_pct: null, return_ytd_pct: null, return_1y_pct: null,
    fundamentals_checked: false,
    sector: null, industry: null, roe_pct: null, roa_pct: null, net_margin_pct: null, operating_margin_pct: null, debt_to_equity: null,
    revenue_growth_pct: null, earnings_growth_pct: null, peg: null, price_to_sales: null,
  };
}

/**
 * Joins a company's fundamentals into its screen row: analysis::screen::apply_fundamentals.
 * P/B and P/S divide the price by statement figures, so they come from one set of
 * statements: a report whose statement currency (or, when it names none, its
 * price's) is not the row's financial_currency fills neither, nor that currency.
 */
export function applyFundamentals(r, f) {
  Object.assign(r, {
    fundamentals_checked: true, sector: f.sector, industry: f.industry, roe_pct: f.profitability.roe_pct, roa_pct: f.profitability.roa_pct,
    net_margin_pct: f.profitability.net_margin_pct, operating_margin_pct: f.profitability.operating_margin_pct, debt_to_equity: f.health.debt_to_equity,
    revenue_growth_pct: f.growth.revenue_growth_pct, earnings_growth_pct: f.growth.earnings_growth_pct, peg: f.valuation.peg,
  });
  const theirs = f.financial_currency || f.currency;
  const foreign = !!(r.financial_currency && theirs) && r.financial_currency.toUpperCase() !== theirs.toUpperCase();
  if (!foreign) {
    r.price_to_sales = f.valuation.price_to_sales;
    r.price_to_book ??= f.valuation.price_to_book;
    r.financial_currency ??= f.financial_currency ?? null;
  }
  r.currency ??= f.currency ?? null;
  r.pe_trailing ??= f.valuation.pe_trailing; r.dividend_yield_pct ??= f.dividend.yield_pct;
  return r;
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
/**
 * The value a filter or the sort reads (analysis::screen::numeric). A P/B or
 * P/S over statements in another currency than the price is not a ratio, so
 * it reads as missing: no pb or ps clause matches it, and a pb or ps sort
 * puts it last. The row keeps it as reported.
 */
export function screenValue(row, field) {
  return (field === "price_to_book" || field === "price_to_sales") && mixedCurrency(row) ? null : row[field];
}
export function matches(row, filters) {
  return filters.every((f) => {
    const v = screenValue(row, f.field);
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
    const x = screenValue(a, field), y = screenValue(b, field);
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
