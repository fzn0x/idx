// The home monitor: the minute board as a station display. The LED band
// carries IHSG, the WIB clock and the session timetable; the heavyweights
// board and the movers flip in place when the minute refresh changes a
// value; the gate sends a line to the web terminal.
import { api, fmt, NA, el, sparkline, flip, setLive, machineLine, copyButton, loadUniverse, quoteHref, shortName, signClass, scrollCue, TICKER } from "/app.js";
import { TOOLS } from "/tools.js";
import { session, countdown, clockText, dayText, hhmm, stamp, wib } from "/clock.js";
import { initShell, hotkeys, jump, lamp, onPause, isPaused, togglePause } from "/shell.js";
import { mountGate, suggestions } from "/gate.js";

const $ = (id) => document.getElementById(id);
const PANELS = ["papan", "teraktif", "keterangan", "ikhtisar", "catatan", "perintah"];
const REFRESH_MS = 60_000;
/** Older than this while the market is open, the snapshot reads as stale (the cron runs every minute). */
const STALE_OPEN_S = 180;

let market = null;
let refreshedAt = null;
let failed = null;
let nextRefresh = 0;
let holidays = new Map();

// ---- tables that flip in place --------------------------------------------------

const SYM = { cls: "l sym", f: (r) => r.symbol, link: true };
const NAME = { cls: "l name", f: (r) => shortName(r.name) || NA, title: (r) => r.name };
const LAST = { f: (r) => fmt.price(r.price) };
const PCT = { f: (r) => fmt.pct(r.change_pct), sign: (r) => r.change_pct };
const value = (r) => r.value ?? (r.price != null && r.volume != null ? r.price * r.volume : null);
const BOARD = [
  { cls: "rk", f: (r, i) => String(i + 1).padStart(2, "0") },
  SYM, NAME, LAST,
  { f: (r) => fmt.change(r.change), sign: (r) => r.change },
  PCT,
  { cls: "vol", f: (r) => fmt.compact(r.volume) },
  { cls: "val", f: (r) => fmt.compact(value(r)) },
  { f: (r) => fmt.compact(r.market_cap) },
];
const MOVERS = {
  g: { head: ["Symbol", "Name", "Last", "Chg %"], cols: [SYM, NAME, LAST, PCT], rows: (m) => m.gainers, what: "gainers" },
  l: { head: ["Symbol", "Name", "Last", "Chg %"], cols: [SYM, NAME, LAST, PCT], rows: (m) => m.losers, what: "losers" },
  v: { head: ["Symbol", "Name", "Last", "Value"], cols: [SYM, NAME, LAST, { f: (r) => fmt.compact(value(r)) }], rows: (m) => m.most_active, what: "most traded by value" },
};

const cellClass = (c, r, text) => [c.cls || "", c.sign ? signClass(c.sign(r)) : "", text === NA ? "na" : ""].filter(Boolean).join(" ");

function fillCell(td, c, r, text) {
  if (c.link && TICKER.test(text)) td.replaceChildren(el("a", { href: quoteHref(text), text }));
  else td.textContent = text;
  const t = c.title?.(r);
  if (t) td.title = t; else td.removeAttribute("title");
}

/**
 * Writes rows into a tbody keyed by symbol. The same symbols in the same order
 * are patched cell by cell and each changed cell flips; a new order is rebuilt,
 * flipping the cells of symbols whose values moved.
 */
function liveRows(tbody, cols, rows) {
  const keys = rows.map((r) => r.symbol).join(",");
  if (tbody.dataset.keys === keys) {
    rows.forEach((r, i) => cols.forEach((c, j) => {
      const td = tbody.rows[i].cells[j], text = c.f(r, i), cls = cellClass(c, r, text);
      if (td.className !== cls) td.className = cls;
      if (td.textContent !== text) { fillCell(td, c, r, text); flip(td); }
    }));
    return;
  }
  const was = new Map([...tbody.rows].map((tr) => [tr.dataset.key, [...tr.cells].map((td) => td.textContent)]));
  const fresh = !tbody.dataset.keys;
  tbody.replaceChildren(...rows.map((r, i) => {
    const tr = el("tr");
    tr.dataset.key = r.symbol;
    cols.forEach((c, j) => {
      const text = c.f(r, i);
      const td = el("td", { class: cellClass(c, r, text) });
      fillCell(td, c, r, text);
      tr.append(td);
      const old = was.get(r.symbol);
      if (!fresh && old && old[j] !== text) flip(td);
    });
    return tr;
  }));
  tbody.dataset.keys = keys;
}

function moverTable(id, spec) {
  const panel = $(`mv-${id}`);
  let tbody = panel.querySelector("tbody");
  if (!tbody) {
    const t = el("table", { class: "tbl", "aria-label": `Movers: ${spec.what}` });
    t.append(el("thead", {}, [el("tr", {}, spec.head.map((h, i) => el("th", { scope: "col", class: i < 2 ? "l" : "", text: h })))]));
    tbody = el("tbody");
    t.append(tbody);
    panel.replaceChildren(el("div", { class: "tbl-wrap" }, [t]));
  }
  return tbody;
}

// ---- the band ---------------------------------------------------------------------

let tape = null;
function drawTape(points) {
  const key = points.length ? `${points.length}:${points[points.length - 1].join(":")}` : "none";
  if (tape?.dataset.key === key) return;
  tape?.stop?.();
  const first = points[0], last = points[points.length - 1];
  tape = sparkline(points, {
    height: 28, cls: "tape",
    label: points.length > 1 ? `IHSG minute closes from ${stamp(new Date(first[0] * 1000))} to ${stamp(new Date(last[0] * 1000))} WIB: ${fmt.price(first[1])} to ${fmt.price(last[1])}` : undefined,
  });
  tape.dataset.key = key;
  $("tape").replaceChildren(tape);
  $("tape-ends").textContent = points.length > 1 ? `${stamp(new Date(first[0] * 1000))} to ${stamp(new Date(last[0] * 1000))} WIB` : "no ticks yet";
}

function renderBand(m) {
  const ix = m.index;
  setLive($("ix-px"), ix ? fmt.price(ix.price) : NA);
  setLive($("ix-chg"), ix ? `${fmt.change(ix.change)} ${fmt.pct(ix.change_pct)}` : "", `led ix-chg ${ix ? signClass(ix.change) : ""}`);
  $("ix-asof").textContent = m.as_of ? `Last trade ${fmt.wib(m.as_of)}` : "The source gave no trade time";
  const total = (m.advancing + m.declining + m.unchanged) || 1;
  const bar = $("brd-bar").children;
  [m.advancing, m.declining, m.unchanged].forEach((n, i) => { bar[i].style.width = `${(n / total) * 100}%`; });
  setLive($("adv"), fmt.int(m.advancing));
  setLive($("dec"), fmt.int(m.declining));
  setLive($("unch"), fmt.int(m.unchanged));
  $("brd-q").textContent = `${fmt.int(m.quoted)} of ${fmt.int(m.universe)} quoted`;
  drawTape(m.index_tape || []);
}

function render(envl) {
  const m = envl.data;
  renderBand(m);
  liveRows($("board-rows"), BOARD, m.heavyweights);
  for (const [id, spec] of Object.entries(MOVERS)) liveRows(moverTable(id, spec), spec.cols, spec.rows(m));
  const when = envl.fetched_at;
  machineLine($("board-m"), [envl.schema, envl.provider, `${m.heavyweights.length} rows`, "by mcap", when, envl.cached ? "cached" : "live", envl.stale ? "stale" : null],
    `Schema ${envl.schema}, provider ${envl.provider}, ${m.heavyweights.length} heavyweights by market cap, snapshot ${fmt.wibAt(m.refreshed_at)}.`);
  machineLine($("movers-m"), [envl.schema, `gainers ${m.gainers.length}`, `losers ${m.losers.length}`, `traded ${m.most_active.length}`, `mcap>${fmt.compact(m.liquid_min_market_cap)}`],
    `Same snapshot: ${m.gainers.length} gainers and ${m.losers.length} losers above ${fmt.compact(m.liquid_min_market_cap)} market cap, ${m.most_active.length} most traded by value.`);
}

// ---- clock, session and the status bar ----------------------------------------

function sessionText(s) {
  const nextWib = wib(s.next.at);
  const time = hhmm(nextWib.secs);
  if (s.phase) return { name: s.phase.name, next: s.next.id === "closed" ? `Until the close at ${time}` : `Until ${s.next.name} at ${time}` };
  const why = s.holiday ? `Closed, ${s.holiday}` : s.tradingDay ? "Closed" : "Closed, weekend";
  return { name: why, next: `Pre-opening ${nextWib.iso === s.date ? "today" : dayText(nextWib.iso).slice(0, -5)} at ${time}` };
}

/** The clock and the session cell; the session waits for the holiday calendar so a holiday never reads as open. */
function paintClock(now) {
  $("clock").textContent = clockText(now);
  const s = session(now, holidays);
  $("clock-d").textContent = dayText(s.date);
  if (holidaysRead) {
    const t = sessionText(s);
    const name = $("ses-name");
    name.textContent = t.name;
    name.classList.toggle("on", s.open);
    $("ses-t").textContent = countdown(s.until);
    $("ses-next").textContent = t.next;
  }
  return s;
}

function tick() {
  const now = new Date();
  if (!isPaused()) {
    const s = paintClock(now);
    if (Date.now() >= nextRefresh) refresh();
    status(now, s);
  }
  setTimeout(tick, 1000 - (Date.now() % 1000) + 5);
}

function status(now, s) {
  if (isPaused()) return lamp("paused", "Paused");
  const age = refreshedAt ? (now - refreshedAt) / 1000 : null;
  const stale = s.open && age != null && age > STALE_OPEN_S;
  for (const n of document.querySelectorAll("[data-age]")) {
    n.textContent = fmt.age(age);
    n.parentElement.classList.toggle("stale", stale);
  }
  if (refreshedAt) {
    $("st-age").replaceChildren("Snapshot ", el("b", { text: `${hhmm(wib(refreshedAt).secs)} WIB` }), ` · ${fmt.age(age)} ago · next read in ${countdown(Math.max(0, (nextRefresh - Date.now()) / 1000)).slice(3)}`);
  }
  if (failed) return lamp("offline", market ? "Offline, last board shown" : "Offline");
  if (!market) return lamp("idle", "Connecting");
  if (stale) return lamp("stale", "Stale");
  return s.open ? lamp("live", "Live") : lamp("closed", "Market closed");
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
      $("board-rows").replaceChildren(el("tr", {}, [el("td", { class: "l dn", colspan: "9", text: `The minute board is unavailable: ${e.message}. The CLI reads the same sources: idx market overview` })]));
      $("ix-asof").textContent = "Board unavailable";
    }
  }
}

let holidaysRead = false;
async function loadHolidays() {
  try {
    const envl = await api("holidays?all=1&upcoming=1");
    holidays = new Map(envl.data.holidays.map((h) => [h.date, h.name_en || h.name]));
  } catch {
    // Without the list only weekends close; the band still runs.
  } finally {
    holidaysRead = true;
    if (!isPaused()) paintClock(new Date());
  }
}

// ---- the gate ------------------------------------------------------------------

function readHistory() {
  try { return JSON.parse(localStorage.getItem("idx.history") || "[]").filter((x) => typeof x === "string"); } catch { return []; }
}

function gate() {
  const named = new Map();
  for (const t of TOOLS) {
    if (!t.web) continue;
    const name = t.web.split(" ")[0];
    if (!named.has(name)) named.set(name, { name, label: t.web, help: t.what, value: t.web, run: true, symbols: /\b[A-Z]{4}\b/.test(t.web) });
  }
  named.set("tools", { name: "tools", help: `the ${TOOLS.length} MCP tools with their CLI and web commands`, value: "tools", run: true });
  named.set("help", { name: "help", help: "every command with its usage", value: "help", run: true });
  const commands = [...named.values()];
  const hist = readHistory();
  let tickers = [];
  mountGate({
    form: $("gate"), input: $("cmd"), list: $("gate-list"), status: $("gate-status"),
    history: () => hist,
    suggest: (line) => suggestions(line, { commands, tickers, history: [...hist].reverse(), examples: ["market", "quote BBCA", "fundamental TLKM", "screen pe<10,div>5,mcap>5T --sort div"] }),
    onRun: (line) => { location.href = `/terminal?cmd=${encodeURIComponent(line)}`; },
    onFocus: () => loadUniverse().then((u) => { tickers = u; }),
  });
}

// ---- tabs, keys, start ----------------------------------------------------------

function tabs() {
  const list = [...document.querySelectorAll('#teraktif [role="tab"]')];
  const select = (t, focus) => {
    for (const x of list) {
      const on = x === t;
      x.setAttribute("aria-selected", String(on));
      x.tabIndex = on ? 0 : -1;
      $(x.getAttribute("aria-controls")).hidden = !on;
    }
    if (focus) t.focus();
  };
  list.forEach((t, i) => {
    t.addEventListener("click", () => select(t));
    t.addEventListener("keydown", (ev) => {
      const n = list.length;
      const j = { ArrowRight: (i + 1) % n, ArrowLeft: (i - 1 + n) % n, Home: 0, End: n - 1 }[ev.key];
      if (j == null) return;
      ev.preventDefault();
      select(list[j], true);
    });
  });
}

const openKeys = initShell();
hotkeys({
  "/": () => $("cmd").focus(),
  "?": openKeys,
  p: () => togglePause(),
  digit: (n) => (PANELS[n - 1] ? jump(PANELS[n - 1]) : false),
});
addEventListener("popstate", () => { const id = location.hash.slice(1); if (PANELS.includes(id)) $(id).focus({ preventScroll: true }); });
onPause((p) => { if (!p) refresh(); status(new Date(), session(new Date(), holidays)); });
document.addEventListener("visibilitychange", () => { if (!document.hidden && !isPaused()) refresh(); });

$("copy-install").replaceWith(copyButton($("install-line").textContent));
$("copy-mcp").replaceWith(copyButton($("mcp-line").textContent));
$("today").textContent = new Date().toLocaleDateString("en-GB", { day: "numeric", month: "long", year: "numeric", timeZone: "Asia/Jakarta" });
const webTools = TOOLS.filter((t) => t.web).length;
machineLine($("tools-m"), ["idx mcp", `${TOOLS.length} tools`, `${webTools} web`, `${TOOLS.length - webTools} cli and mcp only`],
  `${TOOLS.length} MCP tools; ${webTools} run in the web terminal, ${TOOLS.length - webTools} in the CLI and the MCP server only.`);

tabs();
gate();
for (const w of document.querySelectorAll("#perintah .tbl-wrap")) scrollCue(w);
loadHolidays();
tick();
