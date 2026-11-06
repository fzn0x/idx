// node --test web/test/*.mjs (no network). The single-quote chart's layout in
// public/chart.js, on candles parsed from the recorded Yahoo charts.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as y from "../src/yahoo.js";
import { RANGES, PAD, niceTicks, axisLabels, layout, nearest } from "../public/chart.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const chart = (f, period, interval) => y.parseChart(JSON.parse(readFileSync(join(root, "tests", "fixtures", "yahoo", f), "utf8")), period, interval).candles;

test("every range is one the history endpoint accepts", () => {
  const periods = ["1d", "5d", "1mo", "3mo", "6mo", "1y", "2y", "5y", "10y", "ytd", "max"];
  const intervals = ["5m", "15m", "30m", "1h", "1d", "1wk", "1mo"];
  for (const r of RANGES) {
    assert.ok(periods.includes(r.period), r.id);
    assert.ok(intervals.includes(r.interval), r.id);
  }
  assert.equal(RANGES[0].id, "1D");
});

test("ticks are round and inside the range", () => {
  assert.deepEqual(niceTicks(5100, 6150), [5250, 5500, 5750, 6000]);
  assert.deepEqual(niceTicks(0.5, 1.5, 4), [0.5, 1, 1.5]);
  assert.deepEqual(niceTicks(48, 61), [50, 55, 60]);
  assert.deepEqual(niceTicks(7012.4, 7188.9), [7050, 7100, 7150]);
  assert.deepEqual(niceTicks(100, 100), [100]);
});

test("time marks fall on new hours, days, months and years", () => {
  const intraday = ["2026-10-16T09:25:00+07:00", "2026-10-16T09:55:00+07:00", "2026-10-16T10:00:00+07:00", "2026-10-16T11:05:00+07:00"];
  assert.deepEqual(axisLabels(intraday, "hour"), [{ i: 2, text: "10:00" }, { i: 3, text: "11:00" }]);
  const days = ["2026-10-12T09:00:00+07:00", "2026-10-12T15:45:00+07:00", "2026-10-13T09:00:00+07:00"];
  assert.deepEqual(axisLabels(days, "day"), [{ i: 0, text: "12 Oct" }, { i: 2, text: "13 Oct" }]);
  const months = ["2025-12-30T00:00:00+07:00", "2026-01-02T00:00:00+07:00", "2026-02-02T00:00:00+07:00"];
  assert.deepEqual(axisLabels(months, "month"), [{ i: 1, text: "2026" }, { i: 2, text: "Feb" }]);
  const many = Array.from({ length: 20 }, (_, d) => `2026-09-${String(d + 1).padStart(2, "0")}T00:00:00+07:00`);
  assert.equal(axisLabels(many, "day", 5).length, 5);
});

test("a daily chart lays out inside the box", () => {
  const candles = chart("chart_bbca_1mo_1d.json", "1mo", "1d");
  const g = layout(candles, { width: 700, height: 240, unit: "day" });
  assert.equal(g.n, candles.length);
  assert.equal(g.plotW, 700 - PAD.right);
  const xs = [...g.line.matchAll(/[ML]([\d.]+),([\d.]+)/g)].map((m) => [Number(m[1]), Number(m[2])]);
  assert.equal(xs.length, candles.length);
  assert.equal(xs[0][0], 0);
  assert.equal(xs.at(-1)[0], g.plotW);
  for (const [, py] of xs) assert.ok(py >= PAD.top && py <= g.priceBottom, py);
  for (const b of g.bars) assert.ok(b.y >= g.volTop - 1e-9 && b.y + b.h <= g.volBottom + 1e-9 && b.x >= 0 && b.x + b.w <= g.plotW + 1e-9);
  for (const t of g.yTicks) assert.ok(t.v >= g.lo && t.v <= g.hi);
  assert.ok(g.xTicks.length >= 2 && g.xTicks.length <= Math.floor(g.plotW / 72));
  assert.equal(g.rise, candles.at(-1).close >= candles[0].close);
  assert.equal(g.refY, null);
});

test("1D keeps the previous close in view and colours by it", () => {
  const candles = chart("chart_bbri_5d_1h.json", "5d", "1h");
  const last = candles.at(-1).close;
  const above = Math.max(...candles.map((c) => c.close)) * 1.1;
  const g = layout(candles, { width: 600, height: 200, ref: above, unit: "hour" });
  assert.ok(g.hi >= above && g.refY >= PAD.top && g.refY < g.lastY);
  assert.equal(g.rise, last >= above);
  assert.equal(layout(candles, { width: 600, height: 200, ref: 1, unit: "hour" }).rise, true);
});

test("the crosshair snaps to the nearest candle", () => {
  assert.equal(nearest(0, 600, 50), 0);
  assert.equal(nearest(600, 600, 50), 49);
  assert.equal(nearest(-30, 600, 50), 0);
  assert.equal(nearest(9999, 600, 50), 49);
  assert.equal(nearest(306, 600, 50), 25);
  assert.equal(nearest(10, 600, 1), 0);
});

test("max daily and weekly history ask Yahoo for period1=0, and bars are labelled with the granularity Yahoo returned", () => {
  assert.equal(y.chartUrl("^JKSE", "max", "1d"), "https://query1.finance.yahoo.com/v8/finance/chart/^JKSE?period1=0&period2=9999999999&interval=1d&includePrePost=false");
  assert.equal(y.chartUrl("BBCA", "max", "1wk"), "https://query1.finance.yahoo.com/v8/finance/chart/BBCA.JK?period1=0&period2=9999999999&interval=1wk&includePrePost=false");
  assert.equal(y.chartUrl("^JKSE", "max", "1mo"), "https://query1.finance.yahoo.com/v8/finance/chart/^JKSE?range=max&interval=1mo&includePrePost=false");
  assert.equal(y.chartUrl("^JKSE", "2y", "1d"), "https://query1.finance.yahoo.com/v8/finance/chart/^JKSE?range=2y&interval=1d&includePrePost=false");
  const read = (f) => JSON.parse(readFileSync(join(root, "tests", "fixtures", "yahoo", f), "utf8"));
  const daily = y.parseChart(read("chart_jkse_period1_max_1d.json"), "max", "1d");
  assert.equal(daily.interval, "1d");
  assert.equal(daily.candles.length, 8905);
  assert.equal(daily.candles[0].time.slice(0, 10), "1990-04-06");
  // range=max&interval=1d comes back monthly (dataGranularity 1mo).
  const monthly = y.parseChart(read("chart_jkse_range_max_1d.json"), "max", "1d");
  assert.equal(monthly.interval, "1mo");
  assert.equal(monthly.candles.length, 440);
  assert.equal(y.granularity("60m"), "1h");
  assert.equal(y.granularity("3mo"), null);
  assert.ok(JSON.stringify(daily).length < 1_900_000, "fits one D1 cache row");
});
