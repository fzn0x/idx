// Price chart under a single-symbol quote in the web terminal: closes as one
// line over a volume strip, a dashed reference (the previous close on 1D, the
// range's first close otherwise), ruled price ticks and a crosshair readout.
// Candles are spaced by index, so nights, weekends and the lunch break leave
// no gaps. Times arrive in WIB (+07:00) and are read off the string as is.
// The layout functions are pure and tested in web/test/chart.test.mjs.
import { api, fmt, el, signClass } from "./app.js";

export const RANGES = [
  { id: "1D", period: "1d", interval: "5m", unit: "hour", note: "5-minute candles" },
  { id: "5D", period: "5d", interval: "15m", unit: "day", note: "15-minute candles" },
  { id: "1M", period: "1mo", interval: "1d", unit: "day", note: "daily closes" },
  { id: "6M", period: "6mo", interval: "1d", unit: "month", note: "daily closes" },
  { id: "YTD", period: "ytd", interval: "1d", unit: "month", note: "daily closes" },
  { id: "1Y", period: "1y", interval: "1d", unit: "month", note: "daily closes" },
  { id: "5Y", period: "5y", interval: "1wk", unit: "year", note: "weekly closes" },
];

/** Plot margins in px: right holds the price labels, bottom the time labels. */
export const PAD = { top: 10, right: 60, bottom: 20, vol: 36, gap: 8 };

const MON = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const day = (t) => `${Number(t.slice(8, 10))} ${MON[Number(t.slice(5, 7)) - 1]}`;
const hhmm = (t) => t.slice(11, 16);
const KEY = { hour: (t) => t.slice(0, 13), day: (t) => t.slice(0, 10), month: (t) => t.slice(0, 7), year: (t) => t.slice(0, 4) };
const LABEL = {
  hour: (t) => `${t.slice(11, 13)}:00`,
  day,
  month: (t) => (t.slice(5, 7) === "01" ? t.slice(0, 4) : MON[Number(t.slice(5, 7)) - 1]),
  year: (t) => t.slice(0, 4),
};

/**
 * Round tick values between lo and hi: the step of 1, 2, 2.5 or 5 x 10^n whose
 * tick count lands nearest `count` (the larger step on a tie).
 */
export function niceTicks(lo, hi, count = 4) {
  if (!(hi > lo)) return [lo];
  const mag = 10 ** Math.floor(Math.log10((hi - lo) / count));
  const run = (step) => {
    const out = [];
    for (let v = Math.ceil(lo / step - 1e-9) * step; v <= hi + step * 1e-9; v += step) out.push(Number(v.toPrecision(12)));
    return out;
  };
  let best = null;
  for (const step of [1, 2, 2.5, 5, 10].map((m) => m * mag)) {
    const t = run(step);
    if (!best || Math.abs(t.length - count) <= Math.abs(best.length - count)) best = t;
  }
  return best;
}

/**
 * Time-axis marks: the first candle of each new hour, day, month or year,
 * thinned to at most `max`. On the day unit the first candle counts too
 * (5D opens a session there; on 1M every candle is a day).
 */
export function axisLabels(times, unit, max = 6) {
  const key = KEY[unit];
  const marks = [];
  times.forEach((t, i) => {
    if ((i === 0 && unit === "day") || (i > 0 && key(t) !== key(times[i - 1]))) marks.push({ i, text: LABEL[unit](t) });
  });
  const step = Math.max(1, Math.ceil(marks.length / Math.max(1, max)));
  return marks.filter((_, k) => k % step === 0);
}

/** Geometry for one chart: paths, bars and ticks in px for a width x height box. */
export function layout(candles, { width, height, ref = null, unit = "day" }) {
  const pts = candles.filter((c) => c.close != null);
  const n = pts.length;
  const plotW = Math.max(1, width - PAD.right);
  const volBottom = height - PAD.bottom;
  const volTop = volBottom - PAD.vol;
  const priceBottom = volTop - PAD.gap;
  const closes = pts.map((c) => c.close);
  let lo = Math.min(...closes), hi = Math.max(...closes);
  if (ref != null) { lo = Math.min(lo, ref); hi = Math.max(hi, ref); }
  const pad = (hi - lo) * 0.06 || Math.max(1, Math.abs(hi) * 0.01);
  lo -= pad; hi += pad;
  const x = (i) => (n < 2 ? plotW / 2 : (i / (n - 1)) * plotW);
  const y = (v) => PAD.top + (1 - (v - lo) / (hi - lo)) * (priceBottom - PAD.top);
  const line = pts.map((c, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(c.close).toFixed(1)}`).join("");
  const vmax = Math.max(0, ...pts.map((c) => c.volume || 0));
  const bw = Math.max(1, plotW / Math.max(1, n) - 1);
  const bars = vmax > 0
    ? pts.flatMap((c, i) => {
        if (!c.volume) return [];
        const h = Math.max(1, (c.volume / vmax) * PAD.vol);
        const left = Math.max(0, x(i) - bw / 2);
        return [{ x: left, y: volBottom - h, w: Math.min(bw, plotW - left), h }];
      })
    : [];
  const last = closes[n - 1];
  return {
    n, plotW, height, priceBottom, volTop, volBottom, lo, hi, x, y, line, bars, vmax,
    yTicks: niceTicks(lo, hi, 4).map((v) => ({ v, y: y(v) })),
    xTicks: axisLabels(pts.map((c) => c.time), unit, Math.max(2, Math.floor(plotW / 72))).map((m) => ({ ...m, x: x(m.i) })),
    refY: ref != null ? y(ref) : null,
    lastY: n ? y(last) : null,
    rise: n ? last >= (ref ?? closes[0]) : true,
  };
}

/** Nearest candle index to a pixel offset inside the plot. */
export function nearest(px, plotW, n) {
  if (n < 2) return 0;
  return Math.min(n - 1, Math.max(0, Math.round((px / plotW) * (n - 1))));
}

// ---- DOM ------------------------------------------------------------------------

const NS = "http://www.w3.org/2000/svg";
function s(tag, attrs = {}, text) {
  const n = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v);
  if (text != null) n.textContent = text;
  return n;
}
const signed = (v, text) => el("span", { class: signClass(v), text });

/** The chart block for one quote row: range buttons, readout, plot. Loads history itself. */
export function priceChart(q) {
  const root = el("div", { class: "chart" });
  const ranges = el("div", { class: "chart-ranges", role: "group", "aria-label": "Chart range" });
  const note = el("span", { class: "chart-note" });
  const readout = el("p", { class: "chart-readout", "aria-live": "polite" });
  const plot = el("div", { class: "chart-plot" });
  root.append(el("div", { class: "chart-head" }, [ranges, note]), readout, plot);

  const loaded = new Map();
  let range = RANGES[0], data = null, geo = null, svg = null, cross = null, hover = null, token = 0;

  const ref = () => (range.id === "1D" && q.previous_close != null ? q.previous_close : null);
  const pts = () => (data ? data.candles.filter((c) => c.close != null) : []);

  function summary() {
    const p = pts();
    if (!p.length) return [];
    const first = p[0], last = p[p.length - 1];
    const base = ref() ?? first.close;
    const chg = last.close - base, pct = base ? (chg / base) * 100 : null;
    const span = range.unit === "hour" ? `${day(first.time)} ${hhmm(first.time)} to ${hhmm(last.time)} WIB` : `${fmt.date(first.time)} to ${fmt.date(last.time)}`;
    return [`${span} · `, el("b", { text: fmt.price(last.close) }), " ", signed(chg, `${fmt.change(chg)} (${fmt.pct(pct)})`), ref() != null ? ` vs previous close ${fmt.price(base)}` : ` from ${fmt.price(base)}`];
  }
  function at(i) {
    const c = pts()[i];
    const when = range.interval.endsWith("m") ? `${day(c.time)} ${hhmm(c.time)} WIB` : range.interval === "1wk" ? `week of ${fmt.date(c.time)}` : fmt.date(c.time);
    return [`${when} · `, el("b", { text: fmt.price(c.close) }), ` · O ${fmt.price(c.open)} H ${fmt.price(c.high)} L ${fmt.price(c.low)} · vol ${fmt.compact(c.volume)}`];
  }
  const show = () => readout.replaceChildren(...(hover == null ? summary() : at(hover)));

  function point(i) {
    hover = i;
    if (cross) {
      if (i == null) cross.setAttribute("visibility", "hidden");
      else {
        const c = pts()[i];
        const cx = geo.x(i).toFixed(1);
        cross.firstChild.setAttribute("x1", cx); cross.firstChild.setAttribute("x2", cx);
        cross.lastChild.setAttribute("cx", cx); cross.lastChild.setAttribute("cy", geo.y(c.close).toFixed(1));
        cross.setAttribute("visibility", "visible");
      }
    }
    show();
  }

  function draw() {
    const width = Math.round(plot.clientWidth);
    if (!width || !data) return;
    const p = pts();
    if (p.length < 2) { plot.replaceChildren(el("p", { class: "warn", text: `Not enough ${range.id} candles to draw yet.` })); svg = cross = null; return; }
    const height = width < 560 ? 200 : 240;
    geo = layout(p, { width, height, ref: ref(), unit: range.unit });
    svg = s("svg", { viewBox: `0 0 ${width} ${height}`, width, height, class: geo.rise ? "rise" : "fall", role: "img", tabindex: "0",
      "aria-label": `${q.symbol} ${range.id} price chart: ${fmt.price(p[0].close)} to ${fmt.price(p[p.length - 1].close)}, ${p.length} ${range.note}. Arrow keys step through candles.` });
    // Price labels in the right gutter; the last price and the reference win over nearby ticks.
    const pinned = [geo.lastY, geo.refY].filter((v) => v != null);
    for (const t of geo.yTicks) {
      svg.append(s("line", { class: "grid", x1: 0, x2: geo.plotW, y1: t.y, y2: t.y }));
      if (pinned.every((py) => Math.abs(py - t.y) > 14)) svg.append(s("text", { x: geo.plotW + 6, y: t.y, "dominant-baseline": "middle" }, fmt.price(t.v)));
    }
    for (const b of geo.bars) svg.append(s("rect", { class: "vol", x: b.x.toFixed(1), y: b.y.toFixed(1), width: b.w.toFixed(1), height: b.h.toFixed(1) }));
    svg.append(s("text", { x: geo.plotW + 6, y: geo.volTop, "dominant-baseline": "hanging" }, `vol ${fmt.compact(geo.vmax)}`));
    svg.append(s("line", { class: "axis", x1: 0, x2: geo.plotW, y1: geo.volBottom + 0.5, y2: geo.volBottom + 0.5 }));
    for (const t of geo.xTicks) {
      if (t.x > geo.plotW - 28) continue;
      svg.append(s("line", { class: "axis", x1: t.x, x2: t.x, y1: geo.volBottom, y2: geo.volBottom + 4 }));
      svg.append(s("text", { x: t.x, y: height - 4 }, t.text));
    }
    if (geo.refY != null) {
      svg.append(s("line", { class: "ref", x1: 0, x2: geo.plotW, y1: geo.refY, y2: geo.refY }));
      const ly = Math.abs(geo.refY - geo.lastY) < 13 ? geo.lastY + (geo.refY >= geo.lastY ? 13 : -13) : geo.refY;
      svg.append(s("text", { class: "reflabel", x: geo.plotW + 6, y: ly, "dominant-baseline": "middle" }, fmt.price(ref())));
    }
    svg.append(s("path", { class: "ln", d: geo.line }));
    svg.append(s("text", { class: "last", x: geo.plotW + 6, y: geo.lastY, "dominant-baseline": "middle" }, fmt.price(p[p.length - 1].close)));
    cross = s("g", { class: "cross", visibility: "hidden" });
    cross.append(s("line", { x1: 0, x2: 0, y1: PAD.top, y2: geo.volBottom }), s("circle", { r: 3.5, cx: 0, cy: 0 }));
    svg.append(cross);

    const move = (ev) => {
      const box = svg.getBoundingClientRect();
      const px = ((ev.clientX - box.left) / box.width) * width;
      point(px > geo.plotW + 4 ? null : nearest(px, geo.plotW, geo.n));
    };
    svg.addEventListener("pointermove", move);
    svg.addEventListener("pointerdown", move);
    svg.addEventListener("pointerleave", (ev) => { if (ev.pointerType === "mouse") point(null); });
    svg.addEventListener("blur", () => point(null));
    svg.addEventListener("keydown", (ev) => {
      const k = ev.key, last = geo.n - 1;
      const i = k === "ArrowLeft" ? Math.max(0, (hover ?? last + 1) - 1) : k === "ArrowRight" ? Math.min(last, (hover ?? -1) + 1)
        : k === "Home" ? 0 : k === "End" ? last : k === "Escape" ? null : undefined;
      if (i === undefined) return;
      ev.preventDefault();
      point(i);
    });
    plot.replaceChildren(svg);
    point(hover != null && hover < geo.n ? hover : null);
  }

  async function select(r) {
    range = r; hover = null;
    for (const b of ranges.children) b.setAttribute("aria-pressed", String(b.dataset.range === r.id));
    note.textContent = `${r.note}, WIB · Yahoo`;
    const mine = ++token;
    if (!loaded.has(r.id)) {
      readout.replaceChildren(`loading ${r.id}…`);
      try {
        const env = await api(`history?symbol=${encodeURIComponent(q.symbol)}&period=${r.period}&interval=${r.interval}`);
        loaded.set(r.id, env.data);
      } catch (err) {
        if (mine !== token) return;
        data = null; svg = cross = null;
        readout.replaceChildren();
        plot.replaceChildren(el("p", { class: "warn", text: `${r.id} chart unavailable: ${err.message}` }));
        return;
      }
      if (mine !== token) return;
    }
    data = loaded.get(r.id);
    draw();
    if (!svg) show();
  }

  for (const r of RANGES) ranges.append(el("button", { type: "button", text: r.id, "data-range": r.id, "aria-pressed": "false", onclick: () => select(r) }));
  if (typeof ResizeObserver !== "undefined") {
    let w = 0;
    new ResizeObserver(() => { const nw = Math.round(plot.clientWidth); if (nw !== w) { w = nw; draw(); } }).observe(plot);
  }
  select(RANGES[0]);
  return root;
}
