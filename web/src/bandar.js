// Bandarmology: the web twin of src/analysis/bandar.rs. Same tiers, thresholds,
// phases, reasons, filter language and presets, tested on the same recorded
// IPOT pages (web/test/bandar.test.mjs). Pure; api.js and bandar-scan.js feed it.
//
// For N in 1, 3, 5, 10 the top N buyers' value (the source ranks them by lots)
// minus the top N sellers' value is the bandar value; as a share of all value
// traded it reads +15% big accumulation, +5% accumulation, -5% distribution,
// -15% big distribution. The phase sets that against the price move (within
// 2% counts as flat). Heuristics, not advice.

export const TIERS = [1, 3, 5, 10];
export const DEFAULT_TOP = 5;
export const ACCUMULATION_PCT = 5;
export const BIG_PCT = 15;
export const FLAT_PCT = 2;
export const PERIODS = ["1d", "1w", "1m", "3m"];
// The web prints a missing figure as n/a, like the rest of the site.
const DASH = "n/a";

export class UsageError extends Error { constructor(m) { super(m); this.code = "usage"; } }

export function checkTop(top) {
  const n = Number(top);
  if (!TIERS.includes(n)) throw new UsageError(`top must be 1, 3, 5 or 10 (got ${top})`);
  return n;
}
export function checkPeriod(p) {
  const s = String(p || "1w").toLowerCase();
  if (!PERIODS.includes(s)) throw new UsageError(`invalid period "${p}"; use 1d, 1w, 1m or 3m`);
  return s;
}

const isWeekend = (iso) => { const d = new Date(`${iso}T00:00:00Z`).getUTCDay(); return d === 0 || d === 6; };
const addDays = (iso, n) => new Date(Date.parse(`${iso}T00:00:00Z`) + n * 86400e3).toISOString().slice(0, 10);
function minusMonths(iso, months) {
  const [y, m, d] = iso.split("-").map(Number);
  const total = y * 12 + (m - 1) - months;
  const ny = Math.floor(total / 12), nm = total % 12;
  const last = new Date(Date.UTC(ny, nm + 1, 0)).getUTCDate();
  return new Date(Date.UTC(ny, nm, Math.min(d, last))).toISOString().slice(0, 10);
}

/** First date of the period ending on `end`, moved off a weekend (BandarPeriod::start). */
export function periodStart(period, end) {
  let d = period === "1d" ? end : period === "1w" ? addDays(end, -6) : addDays(minusMonths(end, period === "1m" ? 1 : 3), 1);
  while (d < end && isWeekend(d)) d = addDays(d, 1);
  return d;
}

/** Weekdays in from..=to. */
export function weekdays(from, to) {
  const out = [];
  for (let d = from; d <= to; d = addDays(d, 1)) if (!isWeekend(d)) out.push(d);
  return out;
}

export function signal(netPct) {
  if (netPct == null) return "neutral";
  if (netPct >= BIG_PCT) return "big_accumulation";
  if (netPct >= ACCUMULATION_PCT) return "accumulation";
  if (netPct <= -BIG_PCT) return "big_distribution";
  if (netPct <= -ACCUMULATION_PCT) return "distribution";
  return "neutral";
}
const isAcc = (s) => s === "big_accumulation" || s === "accumulation";
const isDist = (s) => s === "big_distribution" || s === "distribution";

export function phase(sig, chg) {
  if (sig === "neutral") return "neutral";
  if (chg == null) return null;
  if (isAcc(sig)) return chg > FLAT_PCT ? "markup" : "quiet_accumulation";
  return chg < -FLAT_PCT ? "markdown" : "quiet_distribution";
}

const pctOf = (part, whole) => (whole != null && whole > 0 ? (part / whole) * 100 : null);
export const changePct = (base, price) => (base != null && price != null && base > 0 ? (price / base - 1) * 100 : null);

/** The quote counts for a range that ends on its session (market_time is a WIB timestamp). */
export const liveQuote = (quote, to) => (quote && quote.price != null && String(quote.market_time || "").slice(0, 10) === to ? quote : null);
/** One-session range on the quote's day: the previous close is the base, no candles needed. */
export const quoteMove = (quote, from, to) => {
  const live = liveQuote(quote, to);
  return live && from === to && live.previous_close != null ? { price: live.price, chg: changePct(live.previous_close, live.price) } : null;
};

/**
 * Last price and move over from..=to (service::price_move): the quote's price
 * when the range ends on its session, else the close on or before `to`; the
 * base is the last close before `from`. Candles are oldest first, WIB times.
 */
export function priceMove(candles, from, to, quote) {
  const q = quoteMove(quote, from, to);
  if (q) return q;
  const day = (c) => String(c.time).slice(0, 10);
  let base = null, end = null;
  for (const c of candles) {
    if (day(c) < from) base = c.close;
    if (day(c) <= to) end = c.close;
  }
  const price = liveQuote(quote, to)?.price ?? end;
  return { price, chg: changePct(base, price) };
}

/** Daily-candle period that reaches back past `from` (service::history_period_for). */
export function historyPeriod(from, today) {
  const days = (Date.parse(`${today}T00:00:00Z`) - Date.parse(`${from}T00:00:00Z`)) / 86400e3 + 10;
  return [[28, "1mo"], [88, "3mo"], [178, "6mo"], [360, "1y"], [720, "2y"], [1800, "5y"]].find(([max]) => days <= max)?.[1] || "max";
}

function vwap(flows) {
  let value = 0, shares = 0;
  for (const f of flows) if (f.value != null && f.average_price != null && f.value > 0 && f.average_price > 0) { value += f.value; shares += f.value / f.average_price; }
  return shares > 0 ? value / shares : null;
}

export function tier(s, top) {
  const buyers = s.buyers.slice(0, top), sellers = s.sellers.slice(0, top);
  const sum = (list, k) => list.reduce((a, f) => a + (f[k] ?? 0), 0);
  const buy_value = sum(buyers, "value"), sell_value = sum(sellers, "value");
  const buy_lots = sum(buyers, "lots"), sell_lots = sum(sellers, "lots");
  const net_value = buy_value - sell_value;
  const net_pct = pctOf(net_value, s.total_value);
  return {
    top, buy_value, sell_value, net_value, buy_lots, sell_lots, net_lots: buy_lots - sell_lots, net_pct,
    buy_share_pct: pctOf(buy_value, s.total_value), sell_share_pct: pctOf(sell_value, s.total_value),
    buy_average: vwap(buyers), sell_average: vwap(sellers), signal: signal(net_pct),
  };
}

// ---- number text, as src/output/format.rs prints it ---------------------------

export function num(v, decimals) {
  if (!Number.isFinite(v)) return DASH;
  const [int, frac] = Math.abs(v).toFixed(decimals).split(".");
  const grouped = int.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return `${v < 0 ? "-" : ""}${grouped}${frac != null ? `.${frac}` : ""}`;
}
export function compact(v) {
  if (v == null) return DASH;
  const a = Math.abs(v);
  const [scaled, suffix] = a >= 1e12 ? [v / 1e12, "T"] : a >= 1e9 ? [v / 1e9, "B"] : a >= 1e6 ? [v / 1e6, "M"] : a >= 1e3 ? [v / 1e3, "K"] : [null, ""];
  if (scaled == null) return num(v, 0);
  return `${num(scaled, Math.abs(scaled) >= 100 ? 1 : 2)}${suffix}`;
}
export const price = (v) => (v == null ? DASH : Math.abs(v - Math.round(v)) < 1e-9 ? num(v, 0) : num(v, 2));
const pct1 = (v) => `${v > 0 ? "+" : ""}${v.toFixed(1)}%`;

const WORDS = { big_accumulation: "big accumulation", accumulation: "accumulation", neutral: "neutral", distribution: "distribution", big_distribution: "big distribution" };
const THRESHOLD = {
  big_accumulation: `at +${BIG_PCT}% or more`, accumulation: `+${ACCUMULATION_PCT}% to +${BIG_PCT}%`, neutral: `within ±${ACCUMULATION_PCT}%`,
  distribution: `-${ACCUMULATION_PCT}% to -${BIG_PCT}%`, big_distribution: `at -${BIG_PCT}% or less`,
};

function reasons(r, head) {
  const n = r.top, out = [];
  const total = r.total_value;
  if (!(total > 0)) return [`No trades from ${r.from} to ${r.to}.`];
  out.push(`Top ${n} buyers bought ${compact(head.buy_value)}, top ${n} sellers sold ${compact(head.sell_value)}: net ${compact(head.net_value)}, ${head.net_pct == null ? DASH : pct1(head.net_pct)} of the ${compact(total)} traded (${WORDS[head.signal]}, ${THRESHOLD[head.signal]}).`);
  out.push(`Net by tier: ${r.tiers.map((t) => `top ${t.top} ${t.net_pct == null ? DASH : pct1(t.net_pct)}`).join(", ")}.`);
  if (head.buy_share_pct != null && head.sell_share_pct != null) {
    const b = head.buy_share_pct, s = head.sell_share_pct;
    out.push(`The top ${n} buyers took ${b.toFixed(1)}% of the value, the top ${n} sellers ${s.toFixed(1)}%: ${b > s ? "buying is more concentrated than selling" : b < s ? "selling is more concentrated than buying" : "buying and selling are equally concentrated"}.`);
  }
  if (r.foreign_net_value != null) {
    const f = r.foreign_net_value, share = r.foreign_net_pct != null ? ` (${pct1(r.foreign_net_pct)} of value)` : "";
    out.push(f > 0 ? `Foreign investors net bought ${compact(f)}${share}.` : f < 0 ? `Foreign investors net sold ${compact(-f)}${share}.` : "Foreign investors were flat on balance.");
  }
  if (r.phase == null) out.push("No price move for the range, so no phase.");
  else if (r.phase !== "neutral" && r.price_change_pct != null) {
    const chg = r.price_change_pct;
    const how = chg > FLAT_PCT ? `up more than ${FLAT_PCT}%` : chg < -FLAT_PCT ? `down more than ${FLAT_PCT}%` : `within ±${FLAT_PCT}%`;
    out.push(`Price ${pct1(chg)} over the range: ${WORDS[r.signal]} with the price ${how} reads as ${r.phase.replace(/_/g, " ")}.`);
  }
  if (r.bandar_average != null && r.price != null && r.bandar_gain_pct != null) {
    const g = r.bandar_gain_pct;
    out.push(`The top ${n} buyers paid ${price(Math.round(r.bandar_average))} on average against the last price ${price(r.price)}: ${g >= 0 ? `a ${g.toFixed(1)}% gain` : `${(-g).toFixed(1)}% under water`}.`);
  }
  if (r.rounded) out.push("The source rounds figures of a million and more to one decimal.");
  return out;
}

/** The full reading of one broker summary (idx.bandar.v1 item). */
export function report(s, period, top, lastPrice, chg) {
  const tiers = TIERS.map((n) => tier(s, n));
  const head = tiers.find((t) => t.top === top) || tier(s, top);
  const traded = s.total_value != null && s.total_value > 0;
  const r = {
    symbol: s.symbol, name: s.name ?? null, period, from: s.from, to: s.to, investor: s.investor, board: s.board, top,
    signal: head.signal, phase: traded ? phase(head.signal, chg ?? null) : "neutral",
    net_value: head.net_value, net_lots: head.net_lots, net_pct: head.net_pct,
    price: lastPrice ?? null, price_change_pct: chg ?? null,
    bandar_average: head.buy_average, bandar_gain_pct: changePct(head.buy_average, lastPrice ?? null),
    total_value: s.total_value, total_lots: s.total_lots, foreign_net_value: s.foreign_net_value,
    foreign_net_pct: s.foreign_net_value != null ? pctOf(s.foreign_net_value, s.total_value) : null,
    tiers, buyers: s.buyers.slice(0, top), sellers: s.sellers.slice(0, top), daily: null, reasons: [], rounded: !!s.rounded,
  };
  r.reasons = reasons(r, head);
  return r;
}

const isEmpty = (s) => !s.buyers.length && !s.sellers.length;

/** Session-by-session series; empty sessions are left out. */
export function daily(sessions, top) {
  const days = [];
  let cum = 0;
  for (const s of sessions.filter((x) => !isEmpty(x))) {
    const t = tier(s, top);
    cum += t.net_value;
    days.push({ date: s.to, total_value: s.total_value, net_value: t.net_value, net_pct: t.net_pct, signal: t.signal, cumulative_net_value: cum, foreign_net_value: s.foreign_net_value });
  }
  let streak = 0;
  for (const d of [...days].reverse()) {
    const dir = isAcc(d.signal) ? 1 : isDist(d.signal) ? -1 : 0;
    if (dir === 0 || (streak !== 0 && dir !== Math.sign(streak))) break;
    streak += dir;
  }
  return {
    days, accumulation_days: days.filter((d) => isAcc(d.signal)).length, distribution_days: days.filter((d) => isDist(d.signal)).length,
    neutral_days: days.filter((d) => d.signal === "neutral").length, streak, cumulative_net_value: cum,
  };
}

export function screenRow(r) {
  const head = r.tiers.find((t) => t.top === r.top);
  return {
    symbol: r.symbol, name: r.name, signal: r.signal, phase: r.phase, net_pct: r.net_pct, net_value: r.net_value, net_lots: r.net_lots,
    price: r.price, price_change_pct: r.price_change_pct, bandar_average: r.bandar_average, bandar_gain_pct: r.bandar_gain_pct,
    buy_share_pct: head ? head.buy_share_pct : null, total_value: r.total_value, foreign_net_value: r.foreign_net_value, foreign_net_pct: r.foreign_net_pct,
    top_buyers: r.buyers.map((f) => f.code), top_sellers: r.sellers.map((f) => f.code), rounded: r.rounded,
  };
}

// ---- screen filters -----------------------------------------------------------------

const KEYS = {
  acc: "acc", "net%": "acc", netpct: "acc", bandar: "acc", net: "net", value: "net", bval: "net", lots: "lots", bvol: "lots",
  fnet: "fnet", foreign: "fnet", fpct: "fpct", "fnet%": "fpct", "foreign%": "fpct", gain: "gain", chg: "chg", change: "chg",
  turnover: "turnover", tval: "turnover", conc: "conc", share: "conc", price: "price", signal: "signal", phase: "phase",
};
const FIELD = { acc: "net_pct", net: "net_value", lots: "net_lots", fnet: "foreign_net_value", fpct: "foreign_net_pct", gain: "bandar_gain_pct", chg: "price_change_pct", turnover: "total_value", conc: "buy_share_pct", price: "price" };
export const KEY_NAMES = "acc, net, lots, fnet, fpct, gain, chg, turnover, conc, price, signal, phase";
const SIGNALS = { big_accumulation: "big_accumulation", bigacc: "big_accumulation", big_acc: "big_accumulation", accumulation: "accumulation", acc: "accumulation", neutral: "neutral", distribution: "distribution", dist: "distribution", big_distribution: "big_distribution", bigdist: "big_distribution", big_dist: "big_distribution" };
const PHASES = { quiet_accumulation: "quiet_accumulation", quietacc: "quiet_accumulation", markup: "markup", quiet_distribution: "quiet_distribution", quietdist: "quiet_distribution", markdown: "markdown", neutral: "neutral" };
export const needsMove = (key) => key === "chg" || key === "phase";

function parseNumber(s) {
  let t = String(s).trim().replace(/%$/, "").trim().replace(/[_,]/g, "");
  if (!t) return null;
  const mult = { K: 1e3, M: 1e6, B: 1e9, T: 1e12 }[t.slice(-1).toUpperCase()];
  if (mult) t = t.slice(0, -1).trim();
  if (!/^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i.test(t)) return null;
  return Number(t) * (mult || 1);
}

function splitClause(clause) {
  const ops = ["<=", ">=", "!=", "<", ">", "="];
  let best = null;
  for (const op of ops) {
    const i = clause.indexOf(op);
    if (i >= 0 && (!best || i < best.i || (i === best.i && op.length > best.op.length))) best = { i, op };
  }
  if (!best) throw new UsageError(`filter "${clause}" needs an operator (<, <=, >, >=, =, !=)`);
  return [clause.slice(0, best.i).trim(), best.op, clause.slice(best.i + best.op.length).trim()];
}

export function parseFilters(spec) {
  const out = [];
  for (const clause of String(spec || "").split(/[,;]/).map((c) => c.trim()).filter(Boolean)) {
    const [k, op, v] = splitClause(clause);
    const key = KEYS[k.toLowerCase()];
    if (!key) throw new UsageError(`unknown bandar filter key "${k}"; use one of ${KEY_NAMES}`);
    let value;
    if (key === "signal" || key === "phase") {
      if (op !== "=" && op !== "!=") throw new UsageError(`${key} only supports = and !=`);
      const name = (key === "signal" ? SIGNALS : PHASES)[v.toLowerCase().replace(/[- ]/g, "_")];
      if (!name) throw new UsageError(key === "signal" ? `unknown signal "${v}"; use big_accumulation, accumulation, neutral, distribution or big_distribution` : `unknown phase "${v}"; use quiet_accumulation, markup, quiet_distribution, markdown or neutral`);
      value = name;
    } else {
      value = parseNumber(v);
      if (value == null) throw new UsageError(`filter "${clause}": "${v}" is not a number (try 5, -15, 10B, 2%)`);
    }
    out.push({ key, op, value, raw: clause });
  }
  return out;
}

function cmp(v, op, t) {
  switch (op) {
    case "<": return v < t; case "<=": return v <= t; case ">": return v > t; case ">=": return v >= t;
    case "=": return Math.abs(v - t) < 1e-9; default: return Math.abs(v - t) >= 1e-9;
  }
}
export function matches(row, filters) {
  return filters.every((f) => {
    if (f.key === "signal" || f.key === "phase") {
      const have = f.key === "signal" ? row.signal : row.phase;
      return have != null && (have === f.value) === (f.op === "=");
    }
    const v = row[FIELD[f.key]];
    return v != null && cmp(v, f.op, f.value);
  });
}

export function parseSort(s) {
  const t = String(s ?? "").trim();
  if (!t) return null;
  const key = KEYS[t.toLowerCase()];
  if (!key || key === "signal" || key === "phase") throw new UsageError(`unknown sort key "${t}"; use one of acc, net, lots, fnet, fpct, gain, chg, turnover, conc, price`);
  return key;
}
export function sortRows(rows, key, descending) {
  const f = FIELD[key];
  return rows.sort((a, b) => {
    const x = a[f], y = b[f];
    if (x == null && y == null) return a.symbol < b.symbol ? -1 : a.symbol > b.symbol ? 1 : 0;
    if (x == null) return 1;
    if (y == null) return -1;
    return descending ? y - x : x - y;
  });
}

export const PRESETS = {
  "big-accumulation": { filter: "acc>=15", ascending: false },
  accumulation: { filter: "acc>=5", ascending: false },
  distribution: { filter: "acc<=-5", ascending: true },
  "big-distribution": { filter: "acc<=-15", ascending: true },
  "quiet-accumulation": { filter: "phase=quiet_accumulation", ascending: false },
  "quiet-distribution": { filter: "phase=quiet_distribution", ascending: true },
  "foreign-accumulation": { filter: "acc>=5,fpct>=5", ascending: false },
  "below-bandar": { filter: "acc>=5,gain<0", ascending: false },
};
export function preset(name) {
  if (name == null || name === "") return null;
  const key = String(name).toLowerCase().replace(/_/g, "-");
  if (!PRESETS[key]) throw new UsageError(`unknown preset "${name}"; use ${Object.keys(PRESETS).join(", ")}`);
  return { name: key, ...PRESETS[key] };
}
