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
