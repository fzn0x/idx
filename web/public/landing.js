// The home page: the market page of the morning paper, live. IHSG leads with
// its move, breadth and the session; the heavyweights and the most active
// stocks follow; a figure that changed in the minute refresh is marked with
// fresh ink. Every word and number follows the page's language (i18n.js).
import { api, fmt, NA, el, flip, setLive, copyButton, loadUniverse, quoteHref, shortName, signClass, scrollCue, TICKER } from "/app.js";
import { TOOLS } from "/tools.js";
import { session, wib, TIMETABLE } from "/clock.js";
import { niceTicks } from "/chart.js";
import { ID, t, here, PHASES, DAYS, DAYS_SHORT, MONTHS, TIME_SEP } from "/i18n.js";
import { initShell, hotkeys, onPause, isPaused, togglePause } from "/shell.js";
import { mountGate, suggestions } from "/gate.js";

const $ = (id) => document.getElementById(id);
const REFRESH_MS = 60_000;
/** Older than this while the market is open, the snapshot reads as stale (the cron runs every minute). */
const STALE_OPEN_S = 180;

let market = null;
let refreshedAt = null;
let failed = null;
let nextRefresh = 0;
let holidays = new Map();
let holidaysRead = false;

// ---- words for times and spans ------------------------------------------------

const p2 = (n) => String(n).padStart(2, "0");
/** 16.14 (Indonesian) or 16:14 for seconds after midnight. */
const hm = (secs) => `${p2(Math.floor(secs / 3600))}${TIME_SEP}${p2(Math.floor((secs % 3600) / 60))}`;
/** "Rab 15.50" for a moment, in WIB. */
const stampOf = (date) => { const w = wib(date); return `${DAYS_SHORT[w.dow]} ${hm(w.secs)}`; };
/** "Jumat, 6 November 2026" for an ISO date. */
const longDate = (iso) => { const d = new Date(`${iso}T00:00:00Z`); return `${DAYS[d.getUTCDay()]}, ${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()}`; };
/** A span in words: "2 hari 13 jam", "1 jam 12 menit", "12 menit". */
function span(secs) {
  const s = Math.max(0, Math.round(secs)), d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
  if (d) return t(`${d} hari ${h} jam`, `${d} d ${h} h`);
  if (h) return t(`${h} jam ${m} menit`, `${h} h ${m} min`);
  if (m) return t(`${m} menit`, `${m} min`);
  return t("kurang dari 1 menit", "under a minute");
}

// ---- tables that re-ink in place ------------------------------------------------

/** The name plate: the code (a link to its quote) over the company's name. */
const plate = (r) => el("span", { class: "plate" }, [
  TICKER.test(r.symbol) ? el("a", { href: quoteHref(r.symbol), text: r.symbol }) : el("b", { text: r.symbol || NA }),
  el("span", { text: shortName(r.name) || "", title: r.name || "" }),
]);
/** A change with the printed page's triangle beside its sign. */
const moveNode = (v, text) => el("span", { class: "chg" }, [el("span", { class: "tri", "aria-hidden": "true", text: fmt.arrow(v) }), text]);

const value = (r) => r.value ?? (r.price != null && r.volume != null ? r.price * r.volume : null);
const PLATE = { cls: "l", key: (r) => `${r.symbol}|${r.name}`, node: plate };
const PRICE = { f: (r) => fmt.price(r.price) };
const PCT = { f: (r) => fmt.pct(r.change_pct), sign: (r) => r.change_pct, node: (r) => moveNode(r.change_pct, fmt.pct(r.change_pct)) };
const BOARD = [
  PLATE, PRICE,
  { f: (r) => fmt.change(r.change), sign: (r) => r.change, node: (r) => moveNode(r.change, fmt.change(r.change)) },
  { cls: "chgv", f: (r) => fmt.pct(r.change_pct), sign: (r) => r.change_pct, node: (r) => moveNode(r.change_pct, fmt.pct(r.change_pct)) },
  { cls: "vol", f: (r) => fmt.compact(r.volume) },
  { cls: "val", f: (r) => fmt.compact(value(r)) },
  { f: (r) => fmt.compact(r.market_cap) },
];
const HEAD = { stock: t("Saham", "Stock"), last: t("Harga", "Price"), pct: t("Persen", "Percent"), value: t("Nilai", "Value") };
const MOVERS = {
  g: { head: [HEAD.stock, HEAD.last, HEAD.pct], cols: [PLATE, PRICE, PCT], rows: (m) => m.gainers, what: t("saham naik", "gainers") },
  l: { head: [HEAD.stock, HEAD.last, HEAD.pct], cols: [PLATE, PRICE, PCT], rows: (m) => m.losers, what: t("saham turun", "losers") },
  v: { head: [HEAD.stock, HEAD.last, HEAD.value], cols: [PLATE, PRICE, { f: (r) => fmt.compact(value(r)) }], rows: (m) => m.most_active, what: t("saham teramai menurut nilai", "most traded by value") },
};

const keyOf = (c, r) => (c.key ? c.key(r) : c.f(r));
const cellClass = (c, r) => [c.cls || "", c.sign ? signClass(c.sign(r)) : "", !c.node && c.f(r) === NA ? "na" : ""].filter(Boolean).join(" ");
function fillCell(td, c, r) {
  if (c.node) td.replaceChildren(c.node(r));
  else td.textContent = c.f(r);
  td.dataset.v = keyOf(c, r);
}

/**
 * Writes rows into a tbody keyed by symbol. The same symbols in the same order
 * are patched cell by cell and each changed figure gets fresh ink; a new order
 * is rebuilt, inking the cells of symbols whose values moved.
 */
function liveRows(tbody, cols, rows) {
  const keys = rows.map((r) => r.symbol).join(",");
  if (tbody.dataset.keys === keys) {
    rows.forEach((r, i) => {
      const tr = tbody.rows[i];
      cols.forEach((c, j) => {
        const td = tr.cells[j], cls = cellClass(c, r);
        if (td.className !== cls) td.className = cls;
        if (td.dataset.v !== keyOf(c, r)) { fillCell(td, c, r); if (j > 0) flip(td); }
      });
    });
    return;
  }
  const was = new Map([...tbody.rows].map((tr) => [tr.dataset.key, [...tr.cells].map((td) => td.dataset.v)]));
  const fresh = !tbody.dataset.keys;
  tbody.replaceChildren(...rows.map((r) => {
    const tr = el("tr");
    tr.dataset.key = r.symbol;
    cols.forEach((c, j) => {
      const td = el("td", { class: cellClass(c, r) });
      fillCell(td, c, r);
      tr.append(td);
      const old = was.get(r.symbol);
      if (!fresh && old && old[j] !== td.dataset.v && j > 0) flip(td);
    });
    return tr;
  }));
  tbody.dataset.keys = keys;
}

function moverTable(id, spec) {
  const panel = $(`mv-${id}`);
  let tbody = panel.querySelector("tbody");
  if (!tbody) {
    const tb = el("table", { class: "tbl", "aria-label": spec.what });
    tb.append(el("thead", {}, [el("tr", {}, spec.head.map((h, i) => el("th", { scope: "col", class: i === 0 ? "l" : "", text: h })))]));
    tbody = el("tbody");
    tb.append(tbody);
    panel.replaceChildren(el("div", { class: "tbl-wrap" }, [tb]));
  }
  return tbody;
}

// ---- the IHSG chart: one plot owns the lead ------------------------------------

const NS = "http://www.w3.org/2000/svg";
const s = (tag, attrs = {}, text) => { const n = document.createElementNS(NS, tag); for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v); if (text != null) n.textContent = text; return n; };
let tapePoints = [];
let tapeRef = null;
let tapeKey = "";
let tapeObserver = null;
/** The phases in which prices move: the two continuous sessions and pre-closing. */
const MOVING = new Set(["session-1", "session-2", "pre-close"]);
const phaseAt = (w) => (w.dow === 5 ? TIMETABLE.friday : TIMETABLE.weekday).find((ph) => w.secs >= ph.from && w.secs < ph.to)?.id ?? null;

/**
 * The minutes of the last trade day while prices move: pre-opening, the break
 * and the closed hours leave flat runs, so they are dropped. The board records
 * the delayed price each minute, so the first minutes of the day still carry
 * the previous close (dropped, the dashed line shows it) and the closing price
 * lands after pre-closing ends (kept, up to the minute it first appears), which
 * makes the line end on the price the page shows above it.
 */
function sessionPoints(points, asOf, ref) {
  if (points.length < 2) return points;
  const lastAt = asOf ? new Date(asOf) : null;
  const day = wib(lastAt && Number.isFinite(lastAt.getTime()) ? lastAt : new Date(points[points.length - 1][0] * 1000)).iso;
  const today = points.filter((p) => wib(new Date(p[0] * 1000)).iso === day);
  const start = today.findIndex((p) => MOVING.has(phaseAt(wib(new Date(p[0] * 1000)))));
  if (start < 0) return points;
  const kept = today.slice(start).filter((p) => { const ph = phaseAt(wib(new Date(p[0] * 1000))); return MOVING.has(ph) || ph === "post" || ph === null; });
  while (kept.length > 2 && kept[kept.length - 1][1] === kept[kept.length - 2][1]) kept.pop();
  // The price carried in from before the open: the last pre-opening minute, or the reference.
  const stale = start > 0 ? today[start - 1][1] : ref;
  let lead = 0;
  if (stale != null) while (lead < kept.length - 2 && Math.abs(kept[lead][1] - stale) < 1e-9) lead++;
  const out = kept.slice(lead);
  return out.length >= 2 ? out : kept.length >= 2 ? kept : points;
}

/**
 * Draws the session's minute closes as a line over a light fill, the previous
 * close dashed across as the reference the day's change is measured from, and
 * the hours along the foot.
 */
function drawTape() {
  const box = $("tape");
  const pts = tapePoints;
  if (pts.length < 2) { box.replaceChildren(el("p", { class: "empty", text: t("Belum ada data menit", "No minute data yet") })); return; }
  const W = Math.max(280, Math.round(box.getBoundingClientRect().width));
  const H = Math.max(220, Math.round(box.getBoundingClientRect().height));
  const pad = { l: 4, r: 88, t: 14, b: 30 };
  const ys = pts.map((p) => p[1]);
  const first = pts[0], last = pts[pts.length - 1];
  const ref = tapeRef ?? first[1];
  let lo = Math.min(...ys, ref), hi = Math.max(...ys, ref);
  const room = (hi - lo || hi * 0.002 || 1) * 0.12;
  lo -= room; hi += room;
  const ticks = niceTicks(lo, hi, 4).filter((v) => v >= lo && v <= hi);
  const x = (i) => pad.l + (i / (pts.length - 1)) * (W - pad.l - pad.r);
  const y = (v) => pad.t + (1 - (v - lo) / (hi - lo)) * (H - pad.t - pad.b);
  const rise = last[1] >= ref;
  const g = s("g", { class: rise ? "rise" : "fall" });
  for (const v of ticks) {
    g.append(s("line", { class: "grid", x1: pad.l, x2: W - pad.r, y1: Math.round(y(v)) + 0.5, y2: Math.round(y(v)) + 0.5 }));
    g.append(s("text", { x: W - pad.r + 10, y: y(v) + 5 }, fmt.price(v)));
  }
  const line = pts.map((p, i) => `${i ? "L" : "M"}${x(i).toFixed(1)} ${y(p[1]).toFixed(1)}`).join("");
  g.append(s("path", { class: "area", d: `${line}L${x(pts.length - 1).toFixed(1)} ${H - pad.b}L${x(0).toFixed(1)} ${H - pad.b}Z` }));
  g.append(s("line", { class: "ref", x1: pad.l, x2: W - pad.r, y1: y(ref), y2: y(ref) }));
  // The reference's name sits above its line, or below it when the top of the plot (Session II's name) is in the way.
  const s2 = pts.findIndex((p) => phaseAt(wib(new Date(p[0] * 1000))) === "session-2");
  const narrow = W - pad.l - pad.r < 420;
  const refText = tapeRef == null ? t(`Awal ${fmt.price(ref)}`, `Start ${fmt.price(ref)}`)
    : narrow ? t(`Penutupan lalu ${fmt.price(ref)}`, `Prev. close ${fmt.price(ref)}`)
    : t(`Penutupan sebelumnya ${fmt.price(ref)}`, `Previous close ${fmt.price(ref)}`);
  const refWide = pad.l + 6 + refText.length * 8.2;
  const crowded = y(ref) - 8 < pad.t + 34 && (s2 > 0 ? refWide > x(s2) : true);
  g.append(s("text", { class: "lbl", x: pad.l + 6, y: y(ref) + (crowded || y(ref) <= pad.t + 24 ? 20 : -8) }, refText));
  g.append(s("path", { class: "ln", d: line }));
  g.append(s("circle", { class: "dot", cx: x(pts.length - 1), cy: y(last[1]), r: 6 }));
  const ly = Math.min(H - pad.b - 4, Math.max(pad.t + 12, y(last[1]) + 5));
  g.append(s("rect", { x: W - pad.r + 4, y: ly - 17, width: pad.r - 4, height: 24, rx: 3, fill: "var(--sheet)" }));
  g.append(s("text", { class: "last", x: W - pad.r + 10, y: ly }, fmt.price(last[1])));
  const t0 = new Date(first[0] * 1000), t1 = new Date(last[0] * 1000);
  // Where Session II begins after the break, a quiet rule and its name.
  if (s2 > 0) {
    g.append(s("line", { class: "grid", x1: Math.round(x(s2)) + 0.5, x2: Math.round(x(s2)) + 0.5, y1: pad.t, y2: H - pad.b }));
    g.append(s("text", { class: "lbl", x: x(s2) + 6, y: pad.t + 14 }, PHASES["session-2"]));
  }
  // The hours along the foot: a label where a new hour starts, kept apart by at least 56px.
  let lastX = -1e9, lastHour = wib(t0).secs / 3600 | 0;
  pts.forEach((p, i) => {
    const w = wib(new Date(p[0] * 1000)), hour = (w.secs / 3600) | 0;
    if (hour !== lastHour && x(i) - lastX >= 56 && x(i) < W - pad.r - 24) {
      g.append(s("line", { class: "grid", x1: Math.round(x(i)) + 0.5, x2: Math.round(x(i)) + 0.5, y1: H - pad.b, y2: H - pad.b + 5 }));
      g.append(s("text", { x: x(i), y: H - 6, "text-anchor": "middle" }, hm(hour * 3600)));
      lastX = x(i);
    }
    lastHour = hour;
  });
  const svg = s("svg", { viewBox: `0 0 ${W} ${H}`, width: W, height: H, role: "img",
    "aria-label": t(
      `IHSG per menit dari ${fmt.price(first[1])} pada ${stampOf(t0)} sampai ${fmt.price(last[1])} pada ${stampOf(t1)} WIB; terendah ${fmt.price(Math.min(...ys))}, tertinggi ${fmt.price(Math.max(...ys))}.`,
      `IHSG by the minute from ${fmt.price(first[1])} at ${stampOf(t0)} to ${fmt.price(last[1])} at ${stampOf(t1)} WIB; low ${fmt.price(Math.min(...ys))}, high ${fmt.price(Math.max(...ys))}.`) });
  svg.append(g);
  box.replaceChildren(svg);
  const sameDay = wib(t0).iso === wib(t1).iso;
  $("tape-range").textContent = sameDay
    ? `${longDate(wib(t0).iso).replace(/ \d{4}$/, "")}, ${hm(wib(t0).secs)} ${t("sampai", "to")} ${hm(wib(t1).secs)} WIB`
    : t(`${stampOf(t0)} sampai ${stampOf(t1)} WIB`, `${stampOf(t0)} to ${stampOf(t1)} WIB`);
}

function setTape(points, ref) {
  const key = points.length ? `${points.length}:${points[points.length - 1].join(":")}:${ref}` : "none";
  if (key === tapeKey) return;
  tapeKey = key;
  tapePoints = points;
  tapeRef = ref;
  drawTape();
  if (!tapeObserver && typeof ResizeObserver !== "undefined") {
    let w = 0;
    tapeObserver = new ResizeObserver(([e]) => { const nw = Math.round(e.contentRect.width); if (nw && nw !== w) { w = nw; drawTape(); } });
    tapeObserver.observe($("tape"));
  }
}

// ---- IHSG, breadth, the board --------------------------------------------------

function renderLead(m) {
  const ix = m.index;
  setLive($("ix-px"), ix ? fmt.price(ix.price) : NA);
  const chg = $("ix-chg");
  if (ix && ix.change != null) {
    const text = `${fmt.change(ix.change)} (${fmt.pct(ix.change_pct)})`;
    if (chg.dataset.v !== text) {
      chg.replaceChildren(el("span", { class: "tri", "aria-hidden": "true", text: fmt.arrow(ix.change) }), text);
      chg.className = `ix-chg ${signClass(ix.change)}`;
      if (chg.dataset.v) flip(chg);
      chg.dataset.v = text;
    }
    chg.hidden = false;
  }
  $("ix-asof").textContent = m.as_of ? t(`Perdagangan terakhir ${fmt.when(m.as_of)}`, `Last trade ${fmt.when(m.as_of)}`) : t("Sumber tidak memberi waktu perdagangan", "The source gave no trade time");
  const total = (m.advancing + m.declining + m.unchanged) || 1;
  const bar = $("brd-bar").children;
  [m.advancing, m.declining, m.unchanged].forEach((n, i) => { bar[i].style.width = `${(n / total) * 100}%`; });
  setLive($("adv"), fmt.int(m.advancing));
  setLive($("dec"), fmt.int(m.declining));
  setLive($("unch"), fmt.int(m.unchanged));
  $("brd-q").textContent = t(`${fmt.int(m.quoted)} dari ${fmt.int(m.universe)} saham tercatat memiliki harga`, `${fmt.int(m.quoted)} of ${fmt.int(m.universe)} listed stocks quoted`);
  const prevClose = ix && ix.price != null && ix.change != null ? ix.price - ix.change : null;
  setTape(sessionPoints(m.index_tape || [], m.as_of, prevClose), prevClose);
}

function render(envl) {
  const m = envl.data;
  renderLead(m);
  liveRows($("board-rows"), BOARD, m.heavyweights);
  for (const [id, spec] of Object.entries(MOVERS)) liveRows(moverTable(id, spec), spec.cols, spec.rows(m));
  $("board-m").textContent = t(
    `Sumber: ${envl.provider === "yahoo" ? "Yahoo Finance" : envl.provider} lewat snapshot menit idx, ${fmt.when(m.refreshed_at)}${envl.stale ? ", tertunda" : ""}. Pilih kode saham untuk harga dan grafiknya.`,
    `Source: ${envl.provider === "yahoo" ? "Yahoo Finance" : envl.provider} through the idx minute snapshot, ${fmt.when(m.refreshed_at)}${envl.stale ? ", stale" : ""}. Choose a code for its price and chart.`);
}

// ---- the session and the freshness line ------------------------------------------

/** When the session's next event falls, in words: "Senin, 9 November", "besok", "hari ini". */
function dayWords(iso, today) {
  if (iso === today) return t("hari ini", "today");
  const tomorrow = new Date(Date.parse(`${today}T00:00:00Z`) + 86400e3).toISOString().slice(0, 10);
  if (iso === tomorrow) return t("besok", "tomorrow");
  const d = new Date(`${iso}T00:00:00Z`);
  return `${DAYS[d.getUTCDay()]}, ${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]}`;
}

function paintSession(now) {
  const s0 = session(now, holidays);
  const w = wib(now);
  $("clock").textContent = hm(w.secs);
  $("dateline").textContent = `${longDate(s0.date)} · ${t("waktu Jakarta (WIB)", "Jakarta time (WIB)")}`;
  if (!holidaysRead) return s0;
  const st = $("state");
  const at = hm(wib(s0.next.at).secs);
  $("ses-now").textContent = s0.phase ? PHASES[s0.phase.id] : t("Tutup", "Closed");
  if (s0.phase) {
    st.dataset.open = "true";
    st.replaceChildren(t("Bursa buka", "Market open"), " ", el("small", { text: `· ${PHASES[s0.phase.id]}` }));
    $("ses-next").textContent = s0.next.id === "closed"
      ? t(`Perdagangan hari ini selesai pukul ${at} WIB, ${span(s0.until)} lagi`, `Today's trading ends at ${at} WIB, in ${span(s0.until)}`)
      : t(`${PHASES[s0.next.id]} pukul ${at} WIB, ${span(s0.until)} lagi`, `${PHASES[s0.next.id]} at ${at} WIB, in ${span(s0.until)}`);
  } else {
    st.dataset.open = "false";
    const why = s0.holiday ? s0.holiday : !s0.tradingDay ? t("akhir pekan", "weekend") : t("di luar jam perdagangan", "outside trading hours");
    st.replaceChildren(t("Bursa tutup", "Market closed"), " ", el("small", { text: `· ${why}` }));
    $("ses-next").textContent = t(
      `Buka lagi ${dayWords(s0.next.day || s0.date, s0.date)} pukul ${at} WIB (pra-pembukaan), ${span(s0.until)} lagi`,
      `Opens ${dayWords(s0.next.day || s0.date, s0.date)} at ${at} WIB (pre-opening), in ${span(s0.until)}`);
  }
  return s0;
}

const STATE_WORDS = {
  live: t("Langsung", "Live"), closed: t("Data penutupan", "Closing data"), stale: t("Tertunda", "Stale"),
  offline: t("Terputus", "Offline"), paused: t("Dijeda", "Paused"), idle: t("Menghubungkan", "Connecting"),
};
function lamp(kind) { const n = $("st-state"); n.dataset.state = kind; n.textContent = STATE_WORDS[kind]; }

function status(now, s0) {
  if (isPaused()) return lamp("paused");
  const age = refreshedAt ? (now - refreshedAt) / 1000 : null;
  const stale = s0.open && age != null && age > STALE_OPEN_S;
  for (const n of document.querySelectorAll("[data-age]")) {
    n.textContent = age == null ? NA : t(`${fmt.ago(age)} lalu`, `${fmt.ago(age)} ago`);
    n.parentElement.classList.toggle("stale", stale);
  }
  if (refreshedAt) $("st-age").textContent = t(`Diperbarui ${fmt.ago(age)} lalu.`, `Updated ${fmt.ago(age)} ago.`);
  if (failed) return lamp("offline");
  if (!market) return lamp("idle");
  if (stale) return lamp("stale");
  return lamp(s0.open ? "live" : "closed");
}

function tick() {
  const now = new Date();
  if (!isPaused()) {
    const s0 = paintSession(now);
    if (Date.now() >= nextRefresh) refresh();
    status(now, s0);
  }
  setTimeout(tick, 1000 - (Date.now() % 1000) + 5);
}

async function refresh() {
  nextRefresh = Date.now() + REFRESH_MS;
  try {
    const envl = await api("market");
    market = envl;
    failed = null;
    refreshedAt = new Date(envl.data.refreshed_at);
    render(envl);
    status(new Date(), session(new Date(), holidays));
  } catch (e) {
    failed = e;
    if (!market) {
      $("board-rows").replaceChildren(el("tr", {}, [el("td", { class: "l dn", colspan: "7", text: t(`Papan harga tidak tersedia: ${e.message}. CLI membaca sumber yang sama: idx market overview`, `The board is unavailable: ${e.message}. The CLI reads the same sources: idx market overview`) })]));
      $("ix-asof").textContent = t("Data tidak tersedia", "Data unavailable");
    }
  }
}

async function loadHolidays() {
  try {
    const envl = await api("holidays?all=1&upcoming=1");
    holidays = new Map(envl.data.holidays.map((h) => [h.date, ID ? h.name || h.name_en : h.name_en || h.name]));
  } catch {
    // Without the list only weekends close; the session still reads.
  } finally {
    holidaysRead = true;
    if (!isPaused()) paintSession(new Date());
  }
}

// ---- the search ------------------------------------------------------------------

function readHistory() {
  try { return JSON.parse(localStorage.getItem("idx.history") || "[]").filter((x) => typeof x === "string"); } catch { return []; }
}

function search() {
  const named = new Map();
  for (const tool of TOOLS) {
    if (!tool.web) continue;
    const name = tool.web.split(" ")[0];
    if (!named.has(name)) named.set(name, { name, label: tool.web, help: tool.what, value: tool.web, run: true, symbols: /\b[A-Z]{4}\b/.test(tool.web) });
  }
  named.set("tools", { name: "tools", help: t(`${TOOLS.length} alat MCP dengan perintah CLI dan web-nya`, `the ${TOOLS.length} MCP tools with their CLI and web commands`), value: "tools", run: true });
  named.set("help", { name: "help", help: t("semua perintah dan cara pakainya", "every command with its usage"), value: "help", run: true });
  const commands = [...named.values()];
  const hist = readHistory();
  let tickers = [];
  /** A four-letter code opens its quote, a command runs as typed, anything else searches the listed names. */
  const resolve = (line) => {
    const s0 = line.trim();
    if (/^[a-z]{4}$/i.test(s0)) return `quote ${s0.toUpperCase()}`;
    if (named.has(s0.split(/\s+/)[0].toLowerCase())) return s0;
    return `list --search "${s0.replace(/"/g, "")}"`;
  };
  mountGate({
    form: $("gate"), input: $("cmd"), list: $("gate-list"), status: $("gate-status"),
    history: () => hist,
    suggest: (line) => suggestions(line, { commands, tickers, history: [...hist].reverse() }),
    onRun: (line) => { location.href = `${here("/terminal")}?cmd=${encodeURIComponent(resolve(line))}`; },
    onFocus: () => loadUniverse().then((u) => { tickers = u; }),
  });
}

// ---- tabs, keys, start ------------------------------------------------------------

function tabs() {
  const list = [...document.querySelectorAll('#teraktif [role="tab"]')];
  const select = (tab, focus) => {
    for (const x of list) {
      const on = x === tab;
      x.setAttribute("aria-selected", String(on));
      x.tabIndex = on ? 0 : -1;
      $(x.getAttribute("aria-controls")).hidden = !on;
    }
    if (focus) tab.focus();
  };
  list.forEach((tab, i) => {
    tab.addEventListener("click", () => select(tab));
    tab.addEventListener("keydown", (ev) => {
      const n = list.length;
      const j = { ArrowRight: (i + 1) % n, ArrowLeft: (i - 1 + n) % n, Home: 0, End: n - 1 }[ev.key];
      if (j == null) return;
      ev.preventDefault();
      select(list[j], true);
    });
  });
}

initShell();
hotkeys({ "/": () => $("cmd").focus(), p: () => togglePause() });
onPause((p) => { if (!p) refresh(); status(new Date(), session(new Date(), holidays)); });
document.addEventListener("visibilitychange", () => { if (!document.hidden && !isPaused()) refresh(); });

$("copy-install").replaceWith(copyButton($("install-line").textContent));
$("copy-mcp").replaceWith(copyButton($("mcp-line").textContent));
$("today").textContent = longDate(wib(new Date()).iso).replace(/^[^,]+, /, "");

tabs();
search();
for (const w of document.querySelectorAll("#perintah .tbl-wrap")) scrollCue(w);
// On a phone the command table restacks its rows as blocks; explicit roles keep it a table to assistive technology.
const toolsTable = document.querySelector("#perintah table");
if (toolsTable) {
  toolsTable.setAttribute("role", "table");
  for (const gr of toolsTable.querySelectorAll("thead, tbody")) gr.setAttribute("role", "rowgroup");
  for (const tr of toolsTable.rows) tr.setAttribute("role", "row");
  for (const th of toolsTable.querySelectorAll("th")) th.setAttribute("role", "columnheader");
  for (const td of toolsTable.querySelectorAll("td")) td.setAttribute("role", "cell");
}
loadHolidays();
tick();
