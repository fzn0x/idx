// Indo Premier (IPOT) broker summary, corporate actions, the market-wide
// broker ranking and the market statistics: the web twin of
// src/providers/ipot. Same endpoints and parsing rules, tested on the same
// tests/fixtures/ipot pages. Figures of a million and more arrive
// abbreviated to one decimal (1.6 M; two in the ranking and the statistics,
// 2.79 T); they keep that precision and set `rounded`. A spurious
// nginx 404 is retried. Over a range that reaches the latest session the
// footer's foreign net counts that session twice; rebuildNet sums single
// sessions instead, like ipot::broker_summary_range.

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

// ---- market-wide broker ranking (ma-topbroker.php) ---------------------------

// The port of src/providers/ipot/topbroker.rs and of the dating rules in
// ipot::top_brokers. ma-topbroker.php ranks every broker of IPOT's latest
// session by value; it carries no date, so BBCA's statistics page dates it:
// on its Today row the days of the high and the low are the session's. Both
// pages carry commented-out copies (a "Lorem Ipsum" template, a 2015 table),
// so comments go before anything is read.

export const TOP_BROKERS_ENDPOINT = "https://www.indopremier.com/module/saham/include/ma-topbroker.php";
/** The page that dates the ranking. */
export const SESSION_ENDPOINT = "https://www.indopremier.com/module/saham/include/statistic.php?code=BBCA";

/** The head of the ranking's table, label and span of each heading: the rows are read by position, so a page headed otherwise is refused. */
const COLUMNS = [["#", 1], ["Company", 2], ["T.Val", 1], ["N.Val", 1], ["B.Val", 1], ["S.Val", 1], ["T.Vol", 1], ["T.Freq", 1]];

/** `html` without its <!-- ... --> comments; an unclosed one runs to the end. */
export function uncommented(html) {
  let out = "", rest = String(html);
  for (;;) {
    const start = rest.indexOf("<!--");
    if (start < 0) return out + rest;
    out += rest.slice(0, start);
    const end = rest.indexOf("-->", start + 4);
    if (end < 0) return out;
    rest = rest.slice(end + 3);
  }
}

/** Text outside tags, as parse::strip_tags: every < starts a tag and every > ends one; &nbsp; is a space; trimmed. */
function stripTags(s) {
  let out = "", inTag = false;
  for (const c of s) {
    if (c === "<") inTag = true;
    else if (c === ">") inTag = false;
    else if (!inTag) out += c;
  }
  return out.replace(/&nbsp;/g, " ").trim();
}

/** A row's cells as parse::cells: the first class="..." inside each, and its text. */
function cellsOf(row) {
  const out = [];
  let rest = row;
  for (;;) {
    const start = rest.indexOf("<td");
    if (start < 0) return out;
    const after = rest.slice(start);
    const openEnd = after.indexOf(">");
    if (openEnd < 0) return out;
    const close = after.indexOf("</td>", openEnd + 1);
    if (close < 0) return out;
    const inner = after.slice(openEnd + 1, close);
    const at = inner.indexOf('class="');
    const value = at < 0 ? "" : inner.slice(at + 7);
    out.push({ cls: at >= 0 && value.includes('"') ? value.slice(0, value.indexOf('"')) : null, text: stripTags(inner) });
    rest = after.slice(close + 5);
  }
}

/** The headings in the page's <thead>, each with its colspan. */
function headings(html) {
  const out = [];
  const at = html.indexOf("<thead");
  if (at < 0) return out;
  const head = html.slice(at + 6);
  const end = head.indexOf("</thead>");
  let rest = end < 0 ? head : head.slice(0, end);
  for (;;) {
    const i = rest.indexOf("<th");
    if (i < 0) return out;
    const after = rest.slice(i + 3);
    const openEnd = after.indexOf(">"), close = after.indexOf("</th>");
    if (openEnd < 0 || close < 0) return out;
    const attrs = after.slice(0, openEnd);
    // <thing ...>, or a <th whose own > comes after its </th>: not a heading.
    if ((attrs && !/^\s/.test(attrs)) || close < openEnd) { rest = after; continue; }
    const j = attrs.indexOf('colspan="');
    const n = j < 0 ? "" : attrs.slice(j + 9).split('"')[0];
    out.push([stripTags(after.slice(openEnd + 1, close)), /^\+?\d+$/.test(n) ? Number(n) : 1]);
    rest = after.slice(close + 5);
  }
}

/** `#, Company (2 wide), T.Val` for messages. */
const showColumns = (cols) => (cols.length ? cols.map(([label, span]) => (span === 1 ? label : `${label} (${span} wide)`)).join(", ") : "none");

const RANKING_KIND = { local: "local", foreign: "foreign", bumn: "state_owned" };
const kindOf = (cls) => (cls && cls.startsWith("text-") && Object.hasOwn(RANKING_KIND, cls.slice(5)) ? RANKING_KIND[cls.slice(5)] : null);

/**
 * The ranking on a ma-topbroker.php page, like topbroker::ranking: every
 * broker in the page's order (`rank` is its #), with its class and value
 * bought plus sold, net, bought, sold, shares and trades, and whether a
 * figure was abbreviated. A page without the table, or headed otherwise
 * than COLUMNS, throws before any row is read.
 */
export function parseRanking(html) {
  const page = uncommented(html);
  const start = page.indexOf("<tbody>"), end = page.indexOf("</tbody>");
  if (start < 0 || end < 0) throw new Error("unexpected broker ranking page (no table)");
  const head = headings(page);
  const same = head.length === COLUMNS.length && head.every(([l, s], i) => l === COLUMNS[i][0] && s === COLUMNS[i][1]);
  if (!same) throw new Error(`unexpected broker ranking layout: the columns read ${showColumns(head)}, not ${showColumns(COLUMNS)}`);
  let rounded = false;
  const brokers = [];
  for (const row of page.slice(start, end).split("</tr>")) {
    const c = cellsOf(row);
    if (c.length < 9) continue;
    const code = c[1].text.toUpperCase();
    if (!code) continue;
    const num = (i) => { const n = number(c[i].text); if (!n) return null; rounded ||= n.rounded; return n.v; };
    const name = decode(c[2].text).trim();
    brokers.push({
      rank: /^\d+$/.test(c[0].text) ? Number(c[0].text) : brokers.length + 1, code, name: name || null, kind: kindOf(c[1].cls),
      value: num(3), net_value: num(4), buy_value: num(5), sell_value: num(6), volume: num(7), frequency: num(8),
    });
  }
  return { brokers, rounded };
}

/** `false` for a broker the page lists without trades (all zeros). */
export const brokerTraded = (b) => [b.value, b.volume, b.frequency].some((v) => v != null && v !== 0);

const isDay = (t) => /^\d{4}-\d{2}-\d{2}$/.test(t) && new Date(`${t}T00:00:00Z`).toISOString().slice(0, 10) === t;

/**
 * The session on a statistic.php page, like topbroker::session_date: the
 * date on its Today row, where the day's high and its low were both made.
 * null without that row (an unknown code answers "Invalid stock code!") or
 * when its two dates differ.
 */
export function sessionDate(html) {
  const page = uncommented(html);
  const at = page.indexOf("<tbody>");
  if (at < 0) return null;
  const row = page.slice(at).split("</tr>").map(cellsOf).find((c) => c[0]?.text.toLowerCase() === "today");
  if (!row) return null;
  const date = (i) => { const t = row[i]?.text.trim(); return t && isDay(t) ? t : null; };
  const high = date(1), low = date(3);
  return high && low && high === low ? high : null;
}

/** A fingerprint of the ranking's rows, codes and figures as published (not names), to tell the same ranking read again from another session's. */
export async function rankingDigest(brokers) {
  const text = brokers.map((b) => [b.code, b.value, b.net_value, b.buy_value, b.sell_value, b.volume, b.frequency].join("|")).join("\n");
  const bytes = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));
  return [...bytes].map((x) => x.toString(16).padStart(2, "0")).join("");
}

/** CC from "[CC] BJB Sekuritas Jawa Barat". */
export function memberInBrackets(name) {
  const t = String(name ?? "").trimStart();
  if (!t.startsWith("[")) return null;
  const close = t.indexOf("]");
  if (close < 0) return null;
  const code = t.slice(1, close).trim();
  return /^[A-Za-z0-9]{1,4}$/.test(code) ? code.toUpperCase() : null;
}

const addKnown = (a, b) => (a == null || b == null ? null : a + b);

/**
 * Moves every row IPOT lists apart for a broker that trades under another
 * member's code into that member, which is how idx.co.id counts it, like
 * topbroker::fold_listed_apart. Such a row names the member in brackets: JB,
 * "[CC] BJB Sekuritas Jawa Barat" (2026-11-06: CC's 210,936 trades and JB's
 * 276 make the 211,212 idx.co.id reports for CC). A row whose member is not
 * on the page stays. Returns the rows moved that had trades.
 */
export function foldListedApart(r) {
  const folded = [];
  for (let i = 0; i < r.brokers.length;) {
    const row = r.brokers[i];
    const m = memberInBrackets(row.name);
    const at = m && m !== row.code ? r.brokers.findIndex((b) => b.code === m) : -1;
    if (at < 0) { i++; continue; }
    r.brokers.splice(i, 1);
    const into = r.brokers[at > i ? at - 1 : at];
    for (const k of ["value", "buy_value", "sell_value", "net_value", "volume", "frequency"]) into[k] = addKnown(into[k], row[k]);
    if (brokerTraded(row)) folded.push({ code: row.code, name: row.name ?? "", into: into.code });
  }
  return folded;
}

// ---- the market's summary of the session (ma-marketstatistic.php) -------------

// The port of src/providers/ipot/marketstat.rs. ma-marketstatistic.php sums up
// IPOT's latest session by board and by foreign investors' side; like the
// ranking it carries no date, and the same statistics page dates it.

export const MARKET_STATISTIC_ENDPOINT = "https://www.indopremier.com/module/saham/include/ma-marketstatistic.php";

/** The heads of the page's two tables: rows are read by label, their figures by position, so a page headed otherwise is refused. */
const SUMMARY_HEADS = [["Board", "Value", "Lot", "Freq"], ["Foreign", "Value", "Lot", "Freq"]];
const showHeads = (heads) => (heads.length ? heads.map((h) => h.join(", ")).join("; ") : "none");

/**
 * The market summary on a ma-marketstatistic.php page, like
 * marketstat::summary: the TOTAL row's value (regular, negotiated and cash)
 * and foreign investors' value bought, sold and net, as published, and
 * whether a figure was abbreviated. A page headed otherwise, or without one
 * of those rows, throws.
 */
export function parseMarketSummary(html) {
  const page = uncommented(html);
  const heads = [];
  for (let i = page.indexOf("<thead"); i >= 0; i = page.indexOf("<thead", i + 6)) heads.push(headings(page.slice(i)).map(([label]) => label));
  if (JSON.stringify(heads) !== JSON.stringify(SUMMARY_HEADS)) throw new Error(`unexpected market summary layout: the tables are headed ${showHeads(heads)}, not ${showHeads(SUMMARY_HEADS)}`);
  const rows = page.split("</tr>").map(cellsOf);
  let rounded = false;
  const value = (label) => {
    const row = rows.find((c) => c.length >= 4 && c[0].text === label);
    if (!row) throw new Error(`unexpected market summary page (no ${label} row)`);
    const n = number(row[1].text);
    if (!n) return null;
    rounded ||= n.rounded;
    return n.v;
  };
  const total = value("TOTAL"), buy = value("F. Buy"), sell = value("F. Sell"), net = value("F. NET");
  return { value: total, foreign_buy_value: buy, foreign_sell_value: sell, foreign_net_value: net, rounded };
}

/** `false` for a page without trades (no value traded or bought), like MarketSummary::traded. */
export const summaryTraded = (s) => [s.value, s.foreign_buy_value, s.foreign_sell_value].some((v) => v != null && v > 0);

/** A fingerprint of the summary's figures as published, to tell the same summary read again from another session's. */
export async function summaryDigest(s) {
  const text = [s.value, s.foreign_buy_value, s.foreign_sell_value, s.foreign_net_value].join("|");
  const bytes = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));
  return [...bytes].map((x) => x.toString(16).padStart(2, "0")).join("");
}

// ---- when an undated page can be dated (ipot::latest_session) -----------------

const WIB_MS = 7 * 3600e3;
/** Seconds after midnight WIB, on a trading day, from before the pre-opening (08:45) until well after the close (16:15) when the ranking may change between two reads. */
const BUSY_FROM = 8 * 3600 + 30 * 60, BUSY_TO = 19 * 3600;
const TTL_BUSY_SECS = 5 * 60;
/** The longest a copy is kept outside the busy hours. */
const TTL_QUIET_SECS = 6 * 3600;
/** The ranking and its date count as one read when fetched this close. */
export const READ_TOGETHER_MS = 60e3;
/** A session's first trades: the pre-opening match starts at 08:58:00 WIB (IDX trading hours, Kep-00158/BEI/10-2026). */
const FIRST_TRADES_SECS = 8 * 3600 + 58 * 60;
/** How long the ranking last accepted is remembered: long past the next session. */
export const ACCEPTED_TTL_MS = 30 * 86400e3;

/** An instant (epoch ms) as its WIB day, clock (HH:MM) and seconds after midnight. */
function wibParts(ms) {
  const iso = new Date(ms + WIB_MS).toISOString();
  return { day: iso.slice(0, 10), clock: iso.slice(11, 16), secs: Number(iso.slice(11, 13)) * 3600 + Number(iso.slice(14, 16)) * 60 + Number(iso.slice(17, 19)) };
}

/**
 * How long, in seconds, a copy of the ranking (and of the page that dates
 * it) is good for at `now`, like ipot::top_brokers_ttl: five minutes on a
 * trading day from 08:30 to 19:00 WIB; outside them six hours at most, and
 * never a copy read before those hours last ended, so the settled figures
 * are read once after them.
 */
export function topBrokersTtl(now) {
  const ms = Number(now);
  const { day, secs } = wibParts(ms);
  const trading = hol.isTradingDay(day);
  if (trading && secs >= BUSY_FROM && secs < BUSY_TO) return TTL_BUSY_SECS;
  const last = trading && secs >= BUSY_TO ? day : prevTradingDay(day);
  const ended = Date.parse(`${last}T00:00:00Z`) - WIB_MS + BUSY_TO * 1000;
  return Math.min(Math.max(0, (ms - ended) / 1000), TTL_QUIET_SECS);
}

/** `t` (epoch ms) falls on a trading day from the session's first trades (08:58) until the busy hours end (19:00 WIB), when a ranking cannot be dated. */
export function undatedAt(t) {
  const { day, secs } = wibParts(Number(t));
  return hol.isTradingDay(day) && secs >= FIRST_TRADES_SECS && secs < BUSY_TO;
}

/**
 * An undated page of IPOT's latest session as dateLatest reads it, like
 * ipot::Undated: its words in messages, when its pair's page was read and
 * its parsed copy (or why it could not be read), whether that shows trades,
 * and its fingerprint.
 */
export const RANKING = {
  noun: "broker ranking", short: "ranking", at: (p) => p.ranked_at, page: (p) => p.ranking,
  traded: (r) => r.brokers.some(brokerTraded), digest: (r) => rankingDigest(r.brokers),
};
export const MARKET_SUMMARY = {
  noun: "market summary", short: "summary", at: (p) => p.summary_at, page: (p) => p.summary,
  traded: summaryTraded, digest: summaryDigest,
};

/**
 * The two pages read as one pair, as the Worker keeps them: when each was
 * read (epoch ms), the date page's session, and the ranking (or why its page
 * could not be read as one: the error is kept with the copy, as the CLI's
 * cache keeps the page that gives it).
 */
export function rankingPair(rankedHtml, datedHtml, rankedAt, datedAt = rankedAt) {
  let ranking;
  try { ranking = parseRanking(rankedHtml); } catch (e) { ranking = { error: e.message }; }
  return { ranked_at: rankedAt, dated_at: datedAt, session: sessionDate(datedHtml), ranking };
}

/** The market statistics page and its date page read as one pair, as rankingPair keeps the ranking's. */
export function summaryPair(summaryHtml, datedHtml, summaryAt, datedAt = summaryAt) {
  let summary;
  try { summary = parseMarketSummary(summaryHtml); } catch (e) { summary = { error: e.message }; }
  return { summary_at: summaryAt, dated_at: datedAt, session: sessionDate(datedHtml), summary };
}

/** Both pages of a pair of `kind` read within READ_TOGETHER_MS of each other. */
export const readTogether = (p, kind = RANKING) => Math.abs(kind.at(p) - p.dated_at) <= READ_TOGETHER_MS;

/** A copy that can answer in the hours a page read now cannot be dated, like ipot::cached_outside_hours: both pages read together outside them. */
export const answersInHours = (p, kind = RANKING) => !!p && readTogether(p, kind) && !undatedAt(kind.at(p)) && !undatedAt(p.dated_at);

/** Why IPOT is not asked at `now`, in those hours, without such a copy of a page of `kind`. */
export function notAsked(now, { noun } = RANKING) {
  const { day, clock } = wibParts(Number(now));
  return `a ${noun} read at ${clock} WIB on ${day}, in a trading day's hours (08:58 to 19:00 WIB), cannot be dated, as it and the page that dates it can be on different sessions then (when they roll over has not been measured); no copy read outside those hours is cached, so IPOT was not asked`;
}

/** Why a pair has no date: `code` provider (IPOT's pages themselves), or network (a copy that stood in for a failed read). */
export class RankingError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}

/**
 * Dates a pair of `kind` like ipot::latest_session, judged by when its pages
 * were read (a copy keeps its read times, at most `now`), not when it is
 * served: refused when the pages were read apart, or either in a trading
 * day's hours (08:58 to 19:00 WIB); when the date page has no Today row or
 * names a day without a session; when a page with trades is dated a session
 * that had not opened, or one without trades a session that had ended; when
 * the page `accepted` before for one session (its figures' digest) is dated
 * another; and when a page with trades is dated an older session than the
 * latest that had ended, unless it is the very pair accepted for it. A
 * refusal of a copy that stood in for a failed read (`failure`, its cause) is
 * that failure (network). Returns the session, the parsed page, and the
 * record to remember for a page with trades (`accept`).
 */
export async function dateLatest(pair, kind, { cached = false, failure = null, now, accepted = null }) {
  const { noun, short } = kind;
  const unusable = (why) => (failure
    ? new RankingError("network", `IPOT could not be read (${failure}), and its cached copy cannot be used: ${why}`)
    : new RankingError("provider", `ipot: ${why}`));
  if (!readTogether(pair, kind)) throw unusable(`the ${noun} and the page that dates it were read at different times, so the ${short} has no date`);
  const readAt = (t) => (cached ? Math.min(t, Number(now)) : t);
  const pageAt = readAt(kind.at(pair)), datedAt = readAt(pair.dated_at);
  if (undatedAt(pageAt)) {
    const { day, clock } = wibParts(pageAt);
    throw unusable(`the ${noun} was read at ${clock} WIB on ${day}, in a trading day's hours (08:58 to 19:00 WIB), when it and the page that dates it can be on different sessions (when they roll over has not been measured), so it cannot be dated`);
  }
  if (undatedAt(datedAt)) {
    const { day, clock } = wibParts(datedAt);
    throw unusable(`the page that dates the ${noun} was read at ${clock} WIB on ${day}, in a trading day's hours (08:58 to 19:00 WIB), when the two pages can be on different sessions (when they roll over has not been measured), so the ${short} cannot be dated`);
  }
  // Both reads are outside those hours and a minute apart at most, so either gives the same session below.
  const read = Math.min(pageAt, datedAt);
  const { day, clock, secs } = wibParts(read);
  const date = pair.session;
  if (!date) throw unusable(`BBCA's statistics page has no Today row to date the ${noun}`);
  if (!hol.isTradingDay(date)) throw unusable(`the page that dates the ${noun} names ${date}, which is not a trading day`);
  const page = kind.page(pair);
  if (page.error) throw new RankingError("provider", `ipot: ${page.error}`);
  const traded = kind.traded(page);
  // A date whose session had not opened when the pages were read cannot have trades, and one whose session had ended cannot be without them.
  const opened = date < day || (date === day && secs >= FIRST_TRADES_SECS);
  if (!opened && traded) throw unusable(`the ${noun} read at ${clock} WIB on ${day} is dated ${date}, a session that had not opened then (trading starts at 08:58 WIB), yet it shows trades: the ${short} and its date disagree`);
  const latest = hol.latestSession(new Date(read));
  if (!traded && date <= latest) throw unusable(`the ${noun} read at ${clock} WIB on ${day} shows no trades, yet the page that dates it names ${date}, a session that had ended then: the two pages are on different sessions`);
  let accept = null;
  if (traded) {
    const digest = await kind.digest(page);
    if (accepted && accepted.digest === digest && accepted.date !== date) throw unusable(`the ${noun} is the one IPOT showed for ${accepted.date}, yet the page that dates it now names ${date}: the ${short} has not rolled over to that session`);
    // Outside the refused hours both pages are taken to have rolled over to the latest session that had ended.
    // A date page naming an older one may lag a page that has: only the very pair accepted before is that older session's.
    const before = !!accepted && accepted.digest === digest && accepted.date === date;
    if (date < latest && !before) throw unusable(`the ${noun} read at ${clock} WIB on ${day} is dated ${date}, yet the session of ${latest} had ended then: the ${short} may be that session's while the page that dates it has not rolled over, so it cannot be dated`);
    accept = { date, digest };
  }
  return { date, page, accept };
}

/**
 * A ranking's pair dated by dateLatest, like ipot::top_brokers: the session,
 * the brokers with those IPOT lists apart folded in (a warning names each),
 * and the record to remember for a ranking with trades (`accept`).
 */
export async function dateRanking(pair, opts) {
  const d = await dateLatest(pair, RANKING, opts);
  const r = { brokers: d.page.brokers.map((b) => ({ ...b })), rounded: d.page.rounded };
  const warnings = foldListedApart(r).map((f) => `${f.code} (${f.name}) is counted in ${f.into}, as idx.co.id counts it; IPOT lists it apart`);
  return { date: d.date, brokers: r.brokers, rounded: r.rounded, warnings, accept: d.accept };
}
