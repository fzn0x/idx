// Indo Premier (IPOT) broker summary and corporate actions: the web twin of
// src/providers/ipot. Same endpoints and parsing rules, tested on the same
// tests/fixtures/ipot pages. Figures of a million and more arrive abbreviated
// to one decimal (1.6 M); they keep that precision and set `rounded`. A
// spurious nginx 404 is retried. Over a range that reaches the latest
// session the footer's foreign net counts that session twice; rebuildNet
// sums single sessions instead, like ipot::broker_summary_range.

import { fetchWithin } from "./net.js";
import * as hol from "./holidays.js";

export const NAME = "ipot";
const ENDPOINT = "https://www.indopremier.com/module/saham/include/data-brokersummary.php";
const ACTIONS_ENDPOINT = "https://www.indopremier.com/module/saham/include/corporateaction.php";
const FD = { all: "all", foreign: "F", domestic: "D" };
const BOARD = { all: "all", regular: "RG", cash: "TN", negotiated: "NG" };
const KIND = { local: "local", foreign: "foreign", bumn: "state_owned" };
export const INVESTORS = Object.keys(FD);
export const BOARDS = Object.keys(BOARD);

const mdy = (iso) => { const [y, m, d] = iso.split("-"); return `${m}/${d}/${y}`; };
const iso = (m, d, y) => `${y}-${m}-${d}`;

export function url(symbol, from, to, investor = "all", board = "all") {
  return `${ENDPOINT}?code=${symbol}&start=${mdy(from)}&end=${mdy(to)}&fd=${FD[investor]}&board=${BOARD[board]}`;
}

/** "1.6 M" -> { v: 1600000, rounded: true }; "687,067" -> { v: 687067, rounded: false }; "" -> null. */
export function number(text) {
  const t = String(text ?? "").trim().replace(/,/g, "");
  if (!t) return null;
  const mult = { K: 1e3, M: 1e6, B: 1e9, T: 1e12 }[t.slice(-1).toUpperCase()];
  const v = Number((mult ? t.slice(0, -1) : t).trim());
  if (!Number.isFinite(v)) return null;
  return { v: Math.round(v * (mult || 1)), rounded: !!mult };
}

const strip = (s) => s.replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").trim();

/** Parses the fragment: { invalid: true } or the summary fields (names left null). */
export function parse(html) {
  if (/invalid stock code/i.test(String(html).trim())) return { invalid: true };
  const a = html.indexOf("<tbody>"), b = html.indexOf("</tbody>");
  if (a < 0 || b < 0) throw new Error("unexpected broker summary page (no table)");
  let rounded = false;
  const num = (text) => { const n = number(text); if (!n) return null; rounded ||= n.rounded; return n.v; };
  const date = (id) => { const m = html.match(new RegExp(`id="${id}"[^>]*?value="(\\d{2})/(\\d{2})/(\\d{4})"`)); return m ? iso(m[1], m[2], m[3]) : null; };
  const buyers = [], sellers = [];
  html.slice(a, b).split("</tr>").forEach((row, i) => {
    const cells = [...row.matchAll(/<td[^>]*>([\s\S]*?)<\/td>/g)].map((m) => ({ cls: (m[1].match(/class="([^"]*)"/) || [])[1] || null, text: strip(m[1]) }));
    if (cells.length < 9) return;
    const rank = Number.parseInt(cells[4].text, 10) || i + 1;
    for (const [side, at] of [[buyers, 0], [sellers, 5]]) {
      const code = cells[at].text.toUpperCase();
      if (!code) continue;
      side.push({ rank, code, name: null, kind: KIND[(cells[at].cls || "").replace(/^text-/, "")] || null, lots: num(cells[at + 1].text), value: num(cells[at + 2].text), average_price: num(cells[at + 3].text) });
    }
  });
  const footer = (label) => { const i = html.indexOf(label); if (i < 0) return null; const rest = html.slice(i + label.length); return num(rest.slice(0, rest.indexOf("<"))); };
  const total_value = footer("T. Val :"), foreign_net_value = footer("F. NVal :"), total_lots = footer("T.Lot :"), avg = footer("Avg :");
  return {
    from: date("startDateBrokerSummary"), to: date("endDateBrokerSummary"), buyers, sellers,
    total_value, total_lots, foreign_net_value,
    // The page prints "Avg : 0" when nothing traded; there is no average then.
    average_price: total_lots > 0 ? avg : null,
    rounded,
  };
}

// ---- what the footer means ----------------------------------------------

/** What the selected investors sold, like ipot::parse::sell_value: all investors what they bought, foreign their buying less the net, domestic plus it. */
export function sellValue(investor, buy, net) {
  if (investor === "all") return buy;
  if (buy == null || net == null) return null;
  return investor === "foreign" ? buy - net : buy + net;
}

/**
 * The footer read like ipot::parse::Parsed::into_summary: for one investor
 * side T. Val, T.Lot and Avg are that side's buying, so the totals are null
 * and T. Val is buy_value; the foreign net covers every board, so a board
 * filter leaves it out.
 */
export function totals(p, investor = "all", board = "all") {
  const all = investor === "all";
  const net = board === "all" ? p.foreign_net_value : null;
  return {
    total_value: all ? p.total_value : null, total_lots: all ? p.total_lots : null, foreign_net_value: net,
    average_price: all ? p.average_price : null, buy_value: p.total_value, sell_value: sellValue(investor, p.total_value, net),
  };
}

// ---- per-session foreign flow (idx.foreign.value.v1) ---------------------

/**
 * Like ipot::unpublished: the footer shows trades but there are no broker
 * lists, so the source has not published the range yet (seen for a session
 * still trading; a past date read that way is treated the same).
 */
export const unpublished = (p) => !traded(p) && p.total_value > 0;

/**
 * One session from its all-investor and foreign pages, like
 * ipot::foreign_day: { session: "none" } when the stock did not trade,
 * "pending" while the source has not published the session (trades without
 * broker lists on any date; today, an empty foreign page too), else
 * "traded" with the day and whether a figure was rounded. `foreign` is only
 * read for a traded day.
 */
export function foreignDay(date, all, foreign, today) {
  if (!traded(all)) return { session: unpublished(all) ? "pending" : "none" };
  if (date >= today && !traded(foreign)) return { session: "pending" };
  const f = totals(foreign, "foreign"), value = all.total_value;
  const share = f.buy_value != null && f.sell_value != null && value > 0 ? ((f.buy_value + f.sell_value) / (2 * value)) * 100 : null;
  return {
    session: "traded", rounded: all.rounded || foreign.rounded,
    day: {
      date, value, lots: all.total_lots, average_price: all.average_price,
      foreign_buy_value: f.buy_value, foreign_sell_value: f.sell_value, foreign_net_value: f.foreign_net_value, foreign_share_pct: share,
    },
  };
}

/** Sums over the sessions (oldest first), the foreign share and the streak, like service::foreign_value_totals. */
export function foreignTotals(days) {
  const sum = (k) => days.reduce((t, d) => t + (d[k] ?? 0), 0);
  const sign = (d) => (d.foreign_net_value > 0 ? 1 : d.foreign_net_value < 0 ? -1 : 0);
  let sides = 0, value = 0;
  for (const d of days) if (d.foreign_buy_value != null && d.foreign_sell_value != null && d.value != null) { sides += d.foreign_buy_value + d.foreign_sell_value; value += d.value; }
  const last = days.length ? sign(days[days.length - 1]) : 0;
  let streak = 0;
  if (last) for (let i = days.length - 1; i >= 0 && sign(days[i]) === last; i--) streak += last;
  return {
    sessions: days.length, value: sum("value"), foreign_buy_value: sum("foreign_buy_value"), foreign_sell_value: sum("foreign_sell_value"),
    foreign_net_value: sum("foreign_net_value"), foreign_share_pct: value > 0 ? (sides / (2 * value)) * 100 : null,
    net_buy_sessions: days.filter((d) => sign(d) > 0).length, net_sell_sessions: days.filter((d) => sign(d) < 0).length, streak,
  };
}

// ---- range foreign net -------------------------------------------------

const addDays = (d, n) => new Date(Date.parse(`${d}T00:00:00Z`) + n * 86400e3).toISOString().slice(0, 10);
const traded = (p) => p.buyers.length > 0 || p.sellers.length > 0;
/** Trading days the rebuild reads one by one from the end of a range. */
const REBUILD_WALK = 5;

/** The trading day before `d` (bundled holiday list). */
export function prevTradingDay(d) {
  let x = addDays(d, -1);
  while (!hol.isTradingDay(x)) x = addDays(x, -1);
  return x;
}
const lastTradingDay = (d) => (hol.isTradingDay(d) ? d : prevTradingDay(d));

/** Unix seconds to the date in Jakarta. */
export const wibDate = (secs) => new Date(secs * 1000 + 7 * 3600e3).toISOString().slice(0, 10);

/**
 * Like ipot::range_net_suspect: a page of several days with trades whose
 * range, on the day it was fetched (`fetchedOn`), reached one of the last
 * two trading days. Its foreign net may count the latest session twice.
 */
export function rangeNetSuspect(from, to, p, fetchedOn, today) {
  if (from >= to || !traded(p) || p.foreign_net_value == null) return false;
  const on = fetchedOn < today ? fetchedOn : today;
  return to >= prevTradingDay(lastTradingDay(on));
}

/**
 * The foreign net of from..to from pages that hold, like the Rust rebuild:
 * single sessions walked back from the end until two had trades, plus the
 * rest of the range (split again if that page is itself suspect). `get(from,
 * to)` resolves to { page, fetchedOn }; null when a page carries no net.
 */
export async function rebuildNet(from, to, today, get) {
  let net = 0, end = to < today ? to : today;
  for (;;) {
    let walked = 0, trades = 0, earliest = null;
    for (let d = end; d >= from && trades < 2 && walked < REBUILD_WALK; d = addDays(d, -1)) {
      if (!hol.isTradingDay(d)) continue;
      const { page } = await get(d, d);
      if (page.foreign_net_value == null) return null;
      net += page.foreign_net_value;
      if (traded(page)) trades++;
      walked++;
      earliest = d;
    }
    if (!earliest) return null;
    const restEnd = prevTradingDay(earliest);
    if (restEnd < from) return net;
    const { page: rest, fetchedOn } = await get(from, restEnd);
    if (!rangeNetSuspect(from, restEnd, rest, fetchedOn, today)) return rest.foreign_net_value == null ? null : net + rest.foreign_net_value;
    end = restEnd;
  }
}

/** GET with the source's spurious 404 retried (up to 5 tries, doubling pauses); each try has a deadline. */
export async function fetchPage(u, { attempts = 5, pauseMs = 250, fetchFn = fetch, ms } = {}) {
  let r;
  for (let i = 0; i < attempts; i++) {
    if (i) await new Promise((res) => setTimeout(res, pauseMs * 2 ** (i - 1)));
    r = await fetchWithin(u, { headers: { "User-Agent": "Mozilla/5.0 (idx web terminal)", Accept: "text/html" } }, { fetchFn, ms });
    if (r.status !== 404) break;
  }
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.text();
}

// ---- corporate actions (corporateaction.php) ----------------------------

export function actionsUrl(symbol, since) {
  return `${ACTIONS_ENDPOINT}?code=${symbol}&start=${mdy(since)}`;
}

const decode = (s) => s.replace(/&nbsp;/g, " ").replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'");

/** Tags removed; paragraphs, list items and breaks become line breaks; spaces collapsed; empty lines dropped. */
function plain(html) {
  const marked = String(html)
    .replace(/<(\/p|br|li|\/tr|\/li)\b[^>]*>/gi, "\n")
    .replace(/<\/(td|th)\b[^>]*>/gi, " ")
    .replace(/<[^>]*>/g, "");
  return decode(marked).split("\n").map((l) => l.split(/\s+/).filter(Boolean).join(" ")).filter(Boolean).join("\n");
}

/** Inner HTML of each top-level <tag> in `html`; nested ones stay inside their parent. */
function elements(html, tag) {
  const lower = html.toLowerCase();
  const open = new RegExp(`<${tag}[\\s>]`, "g");
  const close = `</${tag}>`;
  const nextOpen = (from) => { open.lastIndex = from; const m = open.exec(lower); return m ? m.index : -1; };
  const out = [];
  let pos = 0;
  for (;;) {
    const start = nextOpen(pos);
    if (start < 0) break;
    const gt = lower.indexOf(">", start);
    if (gt < 0) break;
    const inner = gt + 1;
    let depth = 1, cur = inner;
    for (;;) {
      const o = nextOpen(cur), c = lower.indexOf(close, cur);
      if (c < 0) return out;
      if (o >= 0 && o < c) { depth++; cur = o + tag.length + 1; continue; }
      depth--; cur = c + close.length;
      if (depth === 0) { out.push(html.slice(inner, c)); break; }
    }
    pos = cur;
  }
  return out;
}

function sectionRows(html, heading) {
  const h = html.indexOf(heading);
  if (h < 0) return [];
  const section = html.slice(h);
  const b = section.indexOf("<tbody>");
  if (b < 0) return [];
  let body = section.slice(b + "<tbody>".length);
  const e = body.indexOf("</tbody>");
  if (e >= 0) body = body.slice(0, e);
  return elements(body, "tr");
}

const cellText = (c) => { if (c == null) return null; const t = plain(c).trim(); return t === "" || t === "-" ? null : t; };
const cellDate = (c) => { const t = cellText(c); return t && /^\d{4}-\d{2}-\d{2}$/.test(t) ? t : null; };
const cellNum = (c) => { const t = cellText(c); if (!t) return null; const v = Number(t.replace(/,/g, "")); return Number.isFinite(v) ? v : null; };
const nonzero = (v) => (v == null || v === 0 ? null : v);

/** The source's label to a kind; it spells "Deviden" as often as "Dividend". */
export function actionKind(label) {
  const l = String(label).toLowerCase().replace(/deviden/g, "dividen");
  if (l.includes("reverse")) return "reverse_split";
  if (l.includes("split")) return "stock_split";
  if (l.includes("bonus")) return "bonus";
  if (l.includes("right") || l.includes("hmetd")) return "rights_issue";
  if (l.includes("warrant") || l.includes("waran")) return "warrant";
  const dividend = l.includes("dividen") || l.includes("dividend");
  if (dividend && l.includes("stock")) return "stock_dividend";
  if (dividend) return "cash_dividend";
  return "other";
}

const dedup = (rows) => { const seen = new Set(); return rows.filter((r) => { const k = JSON.stringify(r); if (seen.has(k)) return false; seen.add(k); return true; }); };
const newestFirst = (key) => (a, b) => (a[key] === b[key] ? 0 : a[key] == null ? 1 : b[key] == null ? -1 : a[key] < b[key] ? 1 : -1);

/** Parses the "Corporate Action & RUPS" fragment: { invalid: true } or { actions, meetings, warrants_rights }. */
export function parseActions(html) {
  html = String(html);
  if (/^\s*invalid stock code/i.test(html)) return { invalid: true };
  const cut = html.indexOf('id="detail_ca"');
  if (cut >= 0) html = html.slice(0, cut);
  const actions = dedup(sectionRows(html, ">Corporate Actions</h5>").map((row) => {
    // hidden code, #, type, amount, R1, R2, cum, status, ex, rec, dist, end, text
    const c = elements(row, "td");
    if (c.length < 8) return null;
    const label = cellText(c[2]);
    if (!label) return null;
    const kind = actionKind(label), cash = kind === "cash_dividend";
    return {
      kind, label, amount: nonzero(cellNum(c[3])),
      ratio_old: cash ? null : nonzero(cellNum(c[4])), ratio_new: cash ? null : nonzero(cellNum(c[5])),
      cum_date: cellDate(c[6]), ex_date: cellDate(c[8]), recording_date: cellDate(c[9]), distribution_date: cellDate(c[10]),
      end_date: cellDate(c[11]), status: cellText(c[7]), details: cellText(c[12]),
    };
  }).filter(Boolean)).sort(newestFirst("cum_date"));
  const meetings = dedup(sectionRows(html, ">RUPS</h5>").map((row) => {
    // hidden code, #, date, time, status, venue, notice
    const c = elements(row, "td");
    if (c.length < 5) return null;
    return { date: cellDate(c[2]), time: cellText(c[3]), status: cellText(c[4]), venue: cellText(c[5]), agenda: cellText(c[6]) };
  }).filter(Boolean)).sort(newestFirst("date"));
  const wr = [...sectionRows(html, ">Warrant &amp; Right</h5>"), ...sectionRows(html, ">Warrant & Right</h5>")];
  const warrants_rights = dedup(wr.map((row) => {
    // #, code, W/R, exercise, shares, listing, maturity, status, text
    const c = elements(row, "td");
    if (c.length < 8) return null;
    const code = cellText(c[1]), kind = cellText(c[2]);
    if (!code || !kind) return null;
    return {
      code, kind, exercise_price: nonzero(cellNum(c[3])), shares: nonzero(cellNum(c[4])),
      listing_date: cellDate(c[5]), maturity_date: cellDate(c[6]), status: cellText(c[7]), details: cellText(c[8]),
    };
  }).filter(Boolean)).sort(newestFirst("listing_date"));
  return { actions, meetings, warrants_rights };
}
