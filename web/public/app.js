// Shared browser helpers: API access, number formatting, small renderers.
// Plain ES module, no build step. Served with script-src 'self' (plus Cloudflare's analytics beacon). Numbers follow
// the page's language (i18n.js): 5.050,43 on the Indonesian pages, 5,050.43 on
// the English ones; the machine formats (fmt.wib, fmt.time, fmt.date) do not.
import { LOCALE, ID, TIME_SEP, MONTHS_SHORT, t } from "./i18n.js";

export const API = "/api";
// The API gives up on a slow source well before this; past it the terminal
// reports the request instead of leaving it pending.
const API_TIMEOUT_MS = 60_000;

// When each answer was asked, on the clock its fetched_at is on: the server's
// Date less the time the request took (this machine's clock when an answer
// has no Date). Kept beside the envelope, which stays the CLI's, and only for
// a fresh request: within the max-age the Worker sends (20 to 60 s) the
// browser may answer any other from its own copy, at once and under the Date
// that copy was sent with, which says nothing of when it was asked.
const ASKED = new WeakMap();

/**
 * GET /api/<path>: the envelope, or an error carrying the CLI's code and exit
 * code. `fresh` asks the Worker past the browser's HTTP cache (the Worker
 * sends no validators, so it answers in full), for a result whose head says
 * how it was served.
 */
export async function api(path, { fresh = false } = {}) {
  let r;
  const t0 = performance.now();
  try {
    r = await fetch(`${API}/${path}`, { headers: { Accept: "application/json" }, signal: AbortSignal.timeout?.(API_TIMEOUT_MS), ...(fresh ? { cache: "no-cache" } : {}) });
  } catch (err) {
    const e = new Error(err?.name === "TimeoutError" ? `no answer within ${API_TIMEOUT_MS / 1000}s; try again` : `network error: ${err?.message || err}`);
    e.code = "network";
    e.exit = 3;
    throw e;
  }
  const answered = Date.parse(r.headers.get("date") ?? "");
  const asked = (Number.isFinite(answered) ? answered : Date.now()) - (performance.now() - t0);
  const body = await r.json().catch(() => ({ error: { code: "provider", message: `HTTP ${r.status}` }, exit_code: 3 }));
  if (!r.ok || body.error) {
    const e = new Error(body.error?.message || `HTTP ${r.status}`);
    e.code = body.error?.code || "provider";
    e.exit = body.exit_code ?? 3;
    throw e;
  }
  if (fresh && body && typeof body === "object") ASKED.set(body, asked);
  return body;
}

/** When the request behind an envelope api() returned fresh was asked (ms, server clock); undefined for anything else. */
export const askedAt = (env) => ASKED.get(env);

/** Leeway for the whole seconds fetched_at and the Date header are given in. */
const ASKED_SLACK_MS = 2000;

/**
 * How an answer was served, for the terminal's head and machine line:
 * "cached" when every response it was built from came from the cache, "live"
 * when they were fetched for this request, "partly" when some were and the
 * others came from the cache. An envelope built from several responses is
 * cached only when all were and carries the oldest one's fetched_at
 * (Sourced::absorb in the CLI), so an uncached one whose fetched_at is older
 * than its request holds both. Without the time it was asked, it reads live.
 */
export function cacheState(env, asked = askedAt(env)) {
  if (env.cached) return "cached";
  const at = Date.parse(env.fetched_at);
  return Number.isFinite(at) && Number.isFinite(asked) && at < asked - ASKED_SLACK_MS ? "partly" : "live";
}

/** The head's words for each cacheState() and for when it was fetched; the machine line keeps the English. */
export const CACHE_STATES = {
  cached: { id: "dari cache", en: "cached", fetched: { id: "diambil", en: "fetched" } },
  partly: { id: "sebagian dari cache", en: "partly cached", fetched: { id: "bagian tertua diambil", en: "oldest part fetched" } },
  live: { id: "langsung", en: "live", fetched: { id: "diambil", en: "fetched" } },
};

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

/**
 * A small chart of [[ts, value], ...]: the line over a light fill, rising
 * in the up colour and falling in the down colour, with the first value
 * dashed across as the reference. Redraws at its own width.
 */
export function sparkline(points, { height = 48, cls = "tape", label } = {}) {
  const svg = svgEl("svg", { class: `${cls} tapeline`, role: "img", preserveAspectRatio: "none" });
  if (points.length < 2) { svg.setAttribute("aria-label", "not enough points to draw"); return svg; }
  const ys = points.map((p) => p[1]);
  const min = Math.min(...ys), max = Math.max(...ys), span = max - min || 1;
  svg.classList.add(ys[ys.length - 1] >= ys[0] ? "rise" : "fall");
  svg.setAttribute("aria-label", label || `from ${fmt.price(ys[0])} to ${fmt.price(ys[ys.length - 1])}, low ${fmt.price(min)}, high ${fmt.price(max)}`);
  let drawn = 0;
  const draw = () => {
    const w = Math.round(svg.getBoundingClientRect().width);
    if (!w || w === drawn) return;
    drawn = w;
    const pad = 3;
    const x = (i) => (i / (points.length - 1)) * w;
    const y = (v) => pad + (1 - (v - min) / span) * (height - 2 * pad);
    svg.setAttribute("viewBox", `0 0 ${w} ${height}`);
    const line = points.map((p, i) => `${i ? "L" : "M"}${x(i).toFixed(1)} ${y(p[1]).toFixed(1)}`).join("");
    svg.replaceChildren(
      svgEl("path", { class: "area", d: `${line}L${w} ${height}L0 ${height}Z` }),
      svgEl("line", { class: "ref", x1: 0, x2: w, y1: y(ys[0]), y2: y(ys[0]) }),
      svgEl("path", { class: "ln", d: line }),
    );
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
