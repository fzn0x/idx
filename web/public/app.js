// Shared browser helpers: API access, number formatting, small renderers.
// Plain ES module, no build step. Served with script-src 'self'. Numbers follow
// the page's language (i18n.js): 5.050,43 on the Indonesian pages, 5,050.43 on
// the English ones; the machine formats (fmt.wib, fmt.time, fmt.date) do not.
import { LOCALE, ID, TIME_SEP, MONTHS_SHORT, t } from "./i18n.js";

export const API = "/api";
// The API gives up on a slow source well before this; past it the terminal
// reports the request instead of leaving it pending.
const API_TIMEOUT_MS = 60_000;

export async function api(path) {
  let r;
  try {
    r = await fetch(`${API}/${path}`, { headers: { Accept: "application/json" }, signal: AbortSignal.timeout?.(API_TIMEOUT_MS) });
  } catch (err) {
    const e = new Error(err?.name === "TimeoutError" ? `no answer within ${API_TIMEOUT_MS / 1000}s; try again` : `network error: ${err?.message || err}`);
    e.code = "network";
    e.exit = 3;
    throw e;
  }
  const body = await r.json().catch(() => ({ error: { code: "provider", message: `HTTP ${r.status}` }, exit_code: 3 }));
  if (!r.ok || body.error) {
    const e = new Error(body.error?.message || `HTTP ${r.status}`);
    e.code = body.error?.code || "provider";
    e.exit = body.exit_code ?? 3;
    throw e;
  }
  return body;
}

/** What a table or a line shows where the source has no value (the JSON says null). */
export const NA = ID ? "t/a" : "n/a";
const nf0 = new Intl.NumberFormat(LOCALE, { maximumFractionDigits: 0 });
const nf2 = new Intl.NumberFormat(LOCALE, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
/** Compact units: T, B, M, K in English; triliun (T), miliar (M), juta (jt), ribu (rb) in Indonesian. */
const UNITS = ID ? [[1e12, " T"], [1e9, " M"], [1e6, " jt"], [1e3, " rb"]] : [[1e12, "T"], [1e9, "B"], [1e6, "M"], [1e3, "K"]];
const p2 = (n) => String(n).padStart(2, "0");

export const fmt = {
  price(v) { if (v == null) return NA; return Number.isInteger(v) || Math.abs(v - Math.round(v)) < 1e-9 ? nf0.format(v) : nf2.format(v); },
  int(v) { return v == null ? NA : nf0.format(v); },
  num(v, d = 2) { return v == null ? NA : new Intl.NumberFormat(LOCALE, { minimumFractionDigits: d, maximumFractionDigits: d }).format(v); },
  ratio(v) { return v == null ? NA : nf2.format(v); },
  pct(v) { if (v == null) return NA; const s = nf2.format(v); return v > 0 ? `+${s}%` : `${s}%`; },
  pctPlain(v) { return v == null ? NA : `${nf2.format(v)}%`; },
  change(v) { if (v == null) return NA; const s = fmt.price(Math.abs(v)); return v > 0 ? `+${s}` : v < 0 ? `-${s}` : s; },
  compact(v) {
    if (v == null) return NA;
    const unit = UNITS.find(([u]) => Math.abs(v) >= u);
    const [n, s] = unit ? [v / unit[0], unit[1]] : [v, ""];
    return `${new Intl.NumberFormat(LOCALE, { maximumFractionDigits: Math.abs(n) >= 100 ? 1 : 2 }).format(n)}${s}`;
  },
  /** The direction mark the printed market page uses beside a change: up, down or level. */
  arrow(v) { return v == null ? "" : v > 0 ? "▲" : v < 0 ? "▼" : "◆"; },
  /** A readable WIB moment for people: "6 Nov 2026 16.14 WIB" (Indonesian) or "6 Nov 2026, 16:14 WIB". */
  when(t0) {
    const ms = t0 instanceof Date ? t0.getTime() : Date.parse(t0);
    if (!Number.isFinite(ms)) return NA;
    const d = new Date(ms + 7 * 3600e3);
    const date = `${d.getUTCDate()} ${MONTHS_SHORT[d.getUTCMonth()]} ${d.getUTCFullYear()}`;
    const time = `${p2(d.getUTCHours())}${TIME_SEP}${p2(d.getUTCMinutes())} WIB`;
    return ID ? `${date} ${time}` : `${date}, ${time}`;
  },
  /** Elapsed seconds in words: "42 detik", "12 menit", "3 jam 5 menit" (or the English). */
  ago(secs) {
    if (secs == null || !Number.isFinite(secs)) return NA;
    const s = Math.max(0, Math.floor(secs)), m = Math.floor(s / 60), h = Math.floor(m / 60);
    if (s < 60) return t(`${s} detik`, `${s} s`);
    if (h < 1) return t(`${m} menit`, `${m} min`);
    return t(`${h} jam ${m % 60} menit`, `${h} h ${m % 60} min`);
  },
  wib(iso) { if (!iso) return NA; const m = String(iso).match(/(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/); return m ? `${m[1]}-${m[2]}-${m[3]} ${m[4]}:${m[5]} WIB` : iso; },
  time(iso) { if (!iso) return NA; const m = String(iso).match(/T(\d{2}):(\d{2})/); return m ? `${m[1]}:${m[2]} WIB` : iso; },
  date(iso) { return iso ? String(iso).slice(0, 10) : NA; },
  /** An instant (UTC ISO or Date) as Jakarta wall time: 2026-11-06 07:45 WIB. */
  wibAt(t) { const ms = t instanceof Date ? t.getTime() : Date.parse(t); return Number.isFinite(ms) ? `${new Date(ms + 7 * 3600e3).toISOString().slice(0, 16).replace("T", " ")} WIB` : NA; },
  text(v) { return v == null || v === "" ? NA : String(v); },
  /** Elapsed seconds as 00:42, 12:05 or 3h 05m. */
  age(secs) {
    if (secs == null || !Number.isFinite(secs)) return NA;
    const s = Math.max(0, Math.floor(secs)), p = (n) => String(n).padStart(2, "0");
    return s < 3600 ? `${p(Math.floor(s / 60))}:${p(s % 60)}` : `${Math.floor(s / 3600)}h ${p(Math.floor((s % 3600) / 60))}m`;
  },
};

export const signClass = (v) => (v > 0 ? "up" : v < 0 ? "dn" : "");

/** An IDX ticker as a link that runs `quote` in the web terminal. */
export const TICKER = /^[A-Z]{4}$/;
export const quoteHref = (symbol) => `${ID ? "" : "/en"}/terminal?cmd=${encodeURIComponent(`quote ${symbol}`)}`;

/** Builds a table. cols: [{h, k|f, cls, l}] ; rows: objects. Ticker cells in a `sym` column link to their quote. */
export function table(cols, rows, opts = {}) {
  const t = document.createElement("table");
  t.className = "tbl";
  const thead = t.createTHead();
  const hr = thead.insertRow();
  for (const c of cols) {
    const th = document.createElement("th");
    th.textContent = c.h;
    if (c.l) th.classList.add("l");
    th.scope = "col";
    hr.appendChild(th);
  }
  const tb = t.createTBody();
  for (const row of rows) {
    const tr = tb.insertRow();
    if (opts.key) tr.dataset.key = row[opts.key];
    for (const c of cols) {
      const td = tr.insertCell();
      const val = c.f ? c.f(row) : row[c.k];
      if (val instanceof Node) td.appendChild(val);
      else if (val == null || val === NA) { td.textContent = NA; td.classList.add("na"); }
      else if (typeof c.cls === "string" && c.cls.split(" ").includes("sym") && TICKER.test(val)) td.appendChild(el("a", { href: quoteHref(val), text: val }));
      else td.textContent = val;
      if (c.l) td.classList.add("l");
      if (c.cls) { const cl = typeof c.cls === "function" ? c.cls(row) : c.cls; if (cl) td.className += ` ${cl}`; }
      if (c.name) td.dataset.col = c.name;
      if (c.title) { const t = c.title(row); if (t) td.title = t; }
    }
  }
  return t;
}

/** Horizontal scroll container with a visible cue on touch screens. */
export function wrapScroll(node) {
  const d = document.createElement("div");
  d.className = "tbl-wrap";
  d.appendChild(node);
  const frag = document.createDocumentFragment();
  const cue = el("p", { class: "scroll-cue", text: t("geser untuk kolom lainnya", "scroll for more columns") });
  frag.append(d, cue);
  scrollCue(d, cue);
  return frag;
}

/** Fades the right edge of a sideways-scrolling table while more columns remain, and shows its cue. */
export function scrollCue(d, cue) {
  const check = () => {
    d.classList.toggle("scrollable", d.scrollWidth - d.clientWidth - d.scrollLeft > 4);
    cue?.classList.toggle("on", d.scrollWidth > d.clientWidth + 4);
  };
  requestAnimationFrame(check);
  d.addEventListener("scroll", check, { passive: true });
  if (typeof ResizeObserver !== "undefined") new ResizeObserver(check).observe(d);
}

export function el(tag, attrs = {}, children = []) {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") n.className = v;
    else if (k === "text") n.textContent = v;
    else if (k === "html") n.innerHTML = v;
    else if (k.startsWith("on")) n.addEventListener(k.slice(2), v);
    else n.setAttribute(k, v);
  }
  for (const c of [].concat(children)) if (c != null) n.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
  return n;
}

const SVG = "http://www.w3.org/2000/svg";
const svgEl = (tag, attrs = {}) => { const n = document.createElementNS(SVG, tag); for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v); return n; };
let dotGrids = 0;

/**
 * A tape from [[ts, price], ...] drawn as a lit line on a dot-matrix grid:
 * one column of dots per `pitch` pixels, the last close of each column lit,
 * steep moves filled in like a departure board's LEDs, and the first value
 * marked across as the reference row. Redraws at its own width.
 */
export function sparkline(points, { height = 36, cls = "tape", pitch = 4, label } = {}) {
  const svg = svgEl("svg", { class: `${cls} dotline`, role: "img", preserveAspectRatio: "none" });
  if (points.length < 2) { svg.setAttribute("aria-label", "not enough points to draw"); return svg; }
  const ys = points.map((p) => p[1]);
  const min = Math.min(...ys), max = Math.max(...ys), span = max - min || 1;
  svg.classList.add(ys[ys.length - 1] >= ys[0] ? "rise" : "fall");
  svg.setAttribute("aria-label", label || `from ${fmt.price(ys[0])} to ${fmt.price(ys[ys.length - 1])}, low ${fmt.price(min)}, high ${fmt.price(max)}`);
  const id = `dots-${++dotGrids}`;
  let drawn = 0;
  const draw = () => {
    const w = Math.round(svg.getBoundingClientRect().width);
    if (!w || w === drawn) return;
    drawn = w;
    const cols = Math.max(2, Math.floor(w / pitch)), rows = Math.max(3, Math.floor(height / pitch));
    const W = cols * pitch, Hh = rows * pitch, r = pitch * 0.32;
    svg.setAttribute("viewBox", `0 0 ${W} ${Hh}`);
    const row = (v) => rows - 1 - Math.round(((v - min) / span) * (rows - 1));
    const cx = (c) => c * pitch + pitch / 2;
    const pattern = svgEl("pattern", { id, width: pitch, height: pitch, patternUnits: "userSpaceOnUse" });
    pattern.append(svgEl("circle", { class: "ghost", cx: pitch / 2, cy: pitch / 2, r }));
    const defs = svgEl("defs");
    defs.append(pattern);
    const grid = svgEl("rect", { width: W, height: Hh, fill: `url(#${id})` });
    const ref = svgEl("g", { class: "ref" });
    const base = row(ys[0]);
    for (let c = 0; c < cols; c += 2) ref.append(svgEl("circle", { cx: cx(c), cy: base * pitch + pitch / 2, r }));
    const lit = svgEl("g", { class: "lit" });
    let prev = null;
    for (let c = 0; c < cols; c++) {
      const i = Math.min(points.length - 1, Math.floor(((c + 1) * points.length) / cols) - 1);
      const y = row(ys[Math.max(0, i)]);
      const [lo, hi] = prev == null ? [y, y] : [Math.min(prev, y), Math.max(prev, y)];
      for (let k = lo; k <= hi; k++) if (prev == null || k !== prev || k === y) lit.append(svgEl("circle", { cx: cx(c), cy: k * pitch + pitch / 2, r }));
      prev = y;
    }
    svg.replaceChildren(defs, grid, ref, lit);
  };
  svg.style.height = `${height}px`;
  const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(draw) : null;
  if (ro) ro.observe(svg); else requestAnimationFrame(draw);
  /** Stops redrawing; call before dropping a tape that is replaced. */
  svg.stop = () => ro?.disconnect();
  return svg;
}

const motionOk = () => !matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * Fresh ink: a figure that changed in a refresh is marked like a line drawn
 * over with a highlighter, which fades back to the paper over about two
 * seconds. Under reduced motion the value just changes.
 */
export function flip(node) {
  if (!node || !motionOk()) return;
  node.classList.remove("flip");
  void node.offsetWidth;
  node.classList.add("flip");
  node.addEventListener("animationend", () => node.classList.remove("flip"), { once: true });
}

/** Sets a node's text and class, flipping it when the text changed. */
export function setLive(node, text, cls) {
  if (!node) return;
  const changed = node.textContent !== text;
  node.textContent = text;
  if (cls != null) node.className = cls;
  if (changed && node.dataset.live === "1") flip(node);
  node.dataset.live = "1";
}

/**
 * A panel's machine line: the provenance fields read as a passport's
 * machine-readable zone. Screen readers get the plain sentence.
 */
export function machineLine(node, fields, sentence) {
  const mrz = fields.filter((f) => f != null && f !== "").map((f) => String(f).toUpperCase().replace(/\s+/g, "<")).join("<<");
  node.replaceChildren(el("span", { class: "sr-only", text: sentence }), el("span", { "aria-hidden": "true", text: mrz }));
}

export function copyButton(text, label = t("Salin", "Copy")) {
  const b = el("button", { class: "copy", type: "button", text: label, "aria-label": `${label}: ${text}` });
  b.addEventListener("click", async () => {
    try { await navigator.clipboard.writeText(text); b.textContent = t("Tersalin", "Copied"); setTimeout(() => (b.textContent = label), 1400); }
    catch { b.textContent = t("Pilih lalu salin", "Select and copy"); }
  });
  return b;
}

let universe = null;
/** Listed tickers with names, [{ symbol, name }], loaded once for the typeahead. */
export function loadUniverse() {
  universe ??= api("universe").then((e) => e.data).catch(() => { universe = null; return []; });
  return universe;
}

/** "PT Bank Central Asia Tbk" as the board prints it: without the PT and Tbk. */
export const shortName = (name) => String(name || "").replace(/^PT\.?\s+/i, "").replace(/,?\s+Tbk\.?$/i, "").trim();
