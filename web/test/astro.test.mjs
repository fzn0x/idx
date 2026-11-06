// node --test web/test/*.mjs (no network). web/src/astro.js against the same
// JPL Horizons, USNO and Imlek recordings the Rust tests use
// (tests/fixtures/astro/), with the same tolerances, and the backtest on the
// recorded two years of ^JKSE with the same literals as src/analysis/astro.rs.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as A from "../src/astro.js";
import * as y from "../src/yahoo.js";
import { astro, history, fitCacheRow } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (...p) => readFileSync(join(root, "tests", "fixtures", ...p), "utf8");
const MON = { Jan: 0, Feb: 1, Mar: 2, Apr: 3, May: 4, Jun: 5, Jul: 6, Aug: 7, Sep: 8, Oct: 9, Nov: 10, Dec: 11 };

/** Rows of a Horizons QUANTITIES='31' CSV export; `skip` extra columns before ObsEcLon. */
function horizons(text, skip = 0) {
  const block = text.split("$$SOE")[1].split("$$EOE")[0];
  const rows = [];
  for (const line of block.split("\n")) {
    const cols = line.split(",").map((c) => c.trim());
    if (cols.length < 5 + skip) continue;
    const m = cols[0].match(/^(\d{4})-(\w{3})-(\d{2}) (\d{2}):(\d{2})(?::(\d{2}(?:\.\d+)?))?$/);
    const ms = Date.UTC(+m[1], MON[m[2]], +m[3], +m[4], +m[5]) + Number(m[6] || 0) * 1000;
    rows.push({ ms, lon: +cols[3 + skip], lat: +cols[4 + skip], extra: +cols[3] });
  }
  return rows;
}
const arcsec = (a, b) => Math.abs(((a - b + 540) % 360) - 180) * 3600;

test("longitudes match JPL Horizons 1800-2050 within the Rust tolerances", () => {
  // Back the claim in METHOD: Sun and Moon within 0.01° (36″), planets within 0.04° (144″).
  const tol = { sun: 36, moon: 30, mercury: 60, venus: 120, mars: 120, jupiter: 120, saturn: 144, uranus: 144, neptune: 90, pluto: 90 };
  for (const body of A.BODIES) {
    const rows = horizons(fixture("astro", `horizons_${body}_1800_2050.txt`));
    assert.ok(rows.length >= 33, body);
    const worst = Math.max(...rows.map((r) => arcsec(A.position(body, r.ms).lon, r.lon)));
    assert.ok(worst < tol[body], `${body}: ${worst.toFixed(1)}″`);
  }
  for (const [body, t] of [["mercury", 60], ["saturn", 144]]) {
    const rows = horizons(fixture("astro", `horizons_${body}_2025_2027_daily.txt`));
    assert.equal(rows.length, 1095);
    assert.ok(Math.max(...rows.map((r) => arcsec(A.position(body, r.ms).lon, r.lon))) < t, body);
    for (let i = 1; i < rows.length - 1; i++) {
      let d = rows[i + 1].lon - rows[i - 1].lon;
      if (d > 180) d -= 360;
      if (d < -180) d += 360;
      assert.equal(A.speed(body, rows[i].ms) < 0, d < 0, `${body} retrograde flag on row ${i}`);
    }
  }
});

test("phases within two minutes of USNO and illumination within 0.0005 of Horizons", () => {
  const target = { "New Moon": 0, "First Quarter": 90, "Full Moon": 180, "Last Quarter": 270 };
  let n = 0;
  for (const year of [1982, 2025, 2026, 2027]) {
    for (const p of JSON.parse(fixture("astro", `usno_moon_phases_${year}.json`)).phasedata) {
      const [hh, mm] = p.time.split(":").map(Number);
      const usno = Date.UTC(p.year, p.month - 1, p.day, hh, mm);
      const ours = A.nextPhase(usno - 3 * 86400e3, target[p.phase]);
      assert.ok(Math.abs(ours - usno) <= 120e3, `${p.phase} ${p.year}-${p.month}-${p.day}: ${(ours - usno) / 1000} s`);
      n++;
    }
  }
  assert.equal(n, 198);
  const rows = horizons(fixture("astro", "horizons_moon_2026_illum.txt"), 1);
  assert.equal(rows.length, 365);
  for (const r of rows) assert.ok(Math.abs(A.illumination(r.ms) - r.extra / 100) < 0.0005, String(r.ms));
});

test("Mercury stations within an hour of Horizons, 2025-2027", () => {
  const rows = horizons(fixture("astro", "horizons_mercury_2025_2027_daily.txt"));
  const v = rows.slice(1, -1).map((_, i) => { let d = rows[i + 2].lon - rows[i].lon; if (d > 180) d -= 360; if (d < -180) d += 360; return d / 2; });
  const hz = [];
  for (let i = 1; i < v.length; i++) if ((v[i - 1] < 0) !== (v[i] < 0)) hz.push({ at: rows[i].ms + (v[i - 1] / (v[i - 1] - v[i])) * 86400e3, kind: v[i] < 0 ? "station_retrograde" : "station_direct" });
  const ours = A.stations("mercury", rows[1].ms, rows.at(-2).ms, 1);
  assert.equal(hz.length, 18);
  assert.equal(ours.length, 18);
  ours.forEach((o, i) => { assert.equal(o.kind, hz[i].kind); assert.ok(Math.abs(o.at - hz[i].at) <= 3600e3, `${o.kind} ${(o.at - hz[i].at) / 3600e3} h`); });
});

test("weton on published dates, Imlek 1924-2050 and shio", () => {
  const cases = [
    ["1633-07-08", "Jumat", "Legi", 11], ["1945-08-17", "Jumat", "Legi", 11], ["2026-01-01", "Kamis", "Pon", 15], ["2026-01-02", "Jumat", "Wage", 10],
    ["2026-01-03", "Sabtu", "Kliwon", 17], ["2026-01-10", "Sabtu", "Pahing", 18], ["2026-01-17", "Sabtu", "Wage", 13], ["2026-01-31", "Sabtu", "Pon", 16],
    ["2026-05-03", "Minggu", "Kliwon", 13], ["2026-07-06", "Senin", "Wage", 8], ["2026-07-07", "Selasa", "Kliwon", 11], ["2026-07-15", "Rabu", "Pon", 14],
    ["2026-10-20", "Selasa", "Kliwon", 11], ["2026-10-21", "Rabu", "Legi", 12], ["2026-10-23", "Jumat", "Pon", 13], ["2026-10-25", "Minggu", "Kliwon", 13],
  ];
  for (const [date, day, pasaran, neptu] of cases) {
    const w = A.weton(date);
    assert.deepEqual([w.day, w.pasaran, w.neptu], [day, pasaran, neptu], date);
  }
  const imlek = fixture("astro", "imlek_1924_2050_constructed.txt").split("\n").map((l) => l.trim()).filter((l) => /^\d{4}-/.test(l));
  assert.equal(imlek.length, 127);
  for (const d of imlek) assert.equal(A.chineseNewYear(Number(d.slice(0, 4))), d);
  assert.deepEqual(A.shio("2026-02-16"), { animal: "snake", animal_id: "Ular", element: "wood", year_start: "2025-01-29" });
  assert.deepEqual(A.shio("2026-02-17"), { animal: "horse", animal_id: "Kuda", element: "fire", year_start: "2026-02-17" });
  assert.equal(A.shio("1998-05-21").animal, "tiger");
  assert.equal(A.shio("1924-01-01"), null);
});

test("the sky on 2026-11-06 is the CLI's payload", () => {
  const { data, warnings } = A.sky("2026-11-06", 30, "launch");
  assert.deepEqual(warnings, []);
  // The same payload `idx -o json market astro --date 2026-11-06` prints (src/astro/mod.rs).
  const golden = JSON.parse(fixture("astro", "golden_sky_2026-11-06.json"));
  assert.deepEqual(data, golden);
  assert.equal(data.moon.phase, "waning_crescent");
  assert.equal(data.weton.neptu, 15);
  assert.equal(data.upcoming.events.find((e) => e.kind === "new_moon").at, "2026-11-09T14:02:00+07:00");
});

test("Student t p-values and Welch", () => {
  assert.equal(A.pTwoSided(2.0, 10).toFixed(5), "0.07339");
  assert.equal(A.pTwoSided(1.96, 1e6).toFixed(5), "0.05000");
  assert.equal(A.pTwoSided(3, 5).toFixed(5), "0.03010");
  assert.equal(A.welch([1], [1, 2]), null);
  const w = A.welch([1, 2, 3, 4], [2, 4, 6, 8, 10]);
  assert.ok(Math.abs(w.t + 2.251436) < 1e-6 && Math.abs(w.p - 0.069134) < 1e-6);
});

test("the backtest on two recorded years of ^JKSE matches the Rust numbers", () => {
  const h = y.parseChart(JSON.parse(fixture("yahoo", "chart_jkse_2y_1d.json")), "2y", "1d");
  const b = A.backtest(h, "2y");
  assert.deepEqual([b.from, b.to, b.returns, b.mean_pct, b.median_pct, b.win_rate_pct], ["2024-11-06", "2026-11-04", 473, -0.0763, -0.0187, 49.89]);
  assert.deepEqual([b.tests, b.bonferroni_alpha, b.significant], [27, 0.001852, 0]);
  const g = (f, n) => b.factors.find((x) => x.factor === f).groups.find((x) => x.group === n);
  assert.deepEqual([g("mercury", "retrograde").n, g("mercury", "retrograde").p_value], [93, 0.709909]);
  assert.deepEqual(["legi", "pahing", "pon", "wage", "kliwon"].map((p) => g("pasaran", p).n), [95, 95, 98, 92, 93]);
  assert.deepEqual([g("moon_phase", "full_moon").n, g("moon_phase", "full_moon").mean_pct], [59, -0.276]);
  assert.deepEqual([g("sun_sign", "capricorn").n, g("sun_sign", "capricorn").p_value], [32, 0.023102]);
  // Every group, field by field, as src/analysis/astro.rs computes it.
  assert.deepEqual(b, JSON.parse(fixture("astro", "golden_backtest_jkse_2y.json")));
  const monthly = y.parseChart(JSON.parse(fixture("yahoo", "chart_jkse_range_max_1d.json")), "max", "1d");
  assert.throws(() => A.backtest(monthly, "max"), /coarser than daily \(1mo bars\)/);
});

test("a session still trading is left out", () => {
  const h = y.parseChart(JSON.parse(fixture("yahoo", "chart_jkse_2y_1d.json")), "2y", "1d");
  const n = h.candles.length;
  assert.equal(A.dropUnfinished(h, Date.parse("2026-11-04T10:00:00Z")), null);
  assert.equal(A.dropUnfinished(h, Date.parse("2026-11-04T02:30:00Z")), "2026-11-04");
  assert.equal(h.candles.length, n - 1);
});

test("/api/astro computes offline and keeps the web limits", async () => {
  const env = { DB: { prepare: () => { throw new Error("the sky needs no database"); } } };
  const e = await astro(env, "2026-11-06", "7", "base", null, null);
  assert.equal(e.schema, "idx.astro.v1");
  assert.equal(e.provider, "computed");
  assert.equal(e.data.natal.date, "1982-08-10");
  assert.equal(e.data.upcoming.to, "2026-11-13T00:00:00+07:00");
  assert.equal((await astro(env, "2026-11-06", null, "none", null, null)).data.natal, null);
  for (const args of [["2026-11-06", "91"], ["2026-11-06", "0"], ["1799-12-31"], ["2051-01-01"], ["06/11/2026"], ["2026-02-30"], ["2026-04-31"], ["2026-11-06", null, "birth"], ["2026-11-06", null, null, "1", "max"], ["2026-11-06", null, null, "1", "6mo"], ["2026-11-06", null, null, null, "2y"], ["2026-11-06", null, null, "0", "max"]]) {
    const [date, days, natal, backtest, period] = args;
    await assert.rejects(astro(env, date, days, natal, backtest, period), (err) => err.code === "usage", args.join(" "));
  }
  await assert.rejects(astro(env, "2026-11-06", null, null, "1", "max"), /CLI runs max/);
  await assert.rejects(astro(env, "2026-11-06", "91"), /up to 90|1 to 90/);
  // Like the CLI's --period, which requires --backtest.
  await assert.rejects(astro(env, "2026-11-06", null, null, null, "2y"), /pass it with backtest=1/);
  // 2026-02-30 is not 2 March under another name.
  await assert.rejects(astro(env, "2026-02-30"), /invalid date "2026-02-30"/);
  assert.equal((await astro(env, "2024-02-29", "1", "none")).data.date, "2024-02-29");
});

/** In-memory D1 for the cache table; `cap` mimics D1's row limit (SQLITE_TOOBIG). */
function memoryD1(cap = 2_000_000) {
  const cache = new Map();
  const run = (sql, a) => {
    if (sql.startsWith("SELECT v, fetched_at, expires_at FROM cache")) return cache.get(a[0]) || null;
    if (sql.startsWith("INSERT OR REPLACE INTO cache")) {
      if (a[1].length > cap) throw new Error("D1_ERROR: string or blob too big: SQLITE_TOOBIG");
      return cache.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] });
    }
    throw new Error("memoryD1: unexpected SQL " + sql);
  };
  const stmt = (sql, a = []) => ({ bind: (...b) => stmt(sql, b), first: async () => run(sql, a), run: async () => (run(sql, a), {}) });
  return { DB: { prepare: (sql) => stmt(sql) }, cache };
}

async function withFetch(fn, body) {
  const realFetch = globalThis.fetch;
  globalThis.fetch = fn;
  try {
    await body();
  } finally {
    globalThis.fetch = realFetch;
  }
}

test("/api/astro?backtest=1 runs on ^JKSE's history, caches the result and maps errors", async () => {
  const golden = JSON.parse(fixture("astro", "golden_backtest_jkse_2y.json"));
  const env = memoryD1();
  const asked = [];
  await withFetch(async (u) => { asked.push(String(u)); return new Response(fixture("yahoo", "chart_jkse_2y_1d.json"), { status: 200 }); }, async () => {
    const e = await astro(env, "2026-11-06", null, "none", "1", "2y");
    assert.deepEqual([e.schema, e.provider, e.cached, e.stale], ["idx.astro.v1", "yahoo", false, false]);
    assert.equal(e.warnings, undefined);
    assert.deepEqual(e.data.backtest, golden);
    assert.equal(e.data.backtest.returns, 473);
    assert.match(e.fetched_at, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
    // The result is one D1 row per period; the second call reads it.
    assert.ok(env.cache.has("astro_backtest:2y"));
    const again = await astro(env, "2026-11-06", null, "none", "1", "2y");
    assert.deepEqual([again.cached, again.data.backtest], [true, golden]);
    assert.deepEqual(asked, [y.chartUrl("^JKSE", "2y", "1d")]);
  });

  // A history cached at 10:00 WIB on its last bar's date holds a mid-session price: that bar is left
  // out even when the copy is read after the close (the clock now is days later).
  const mid = memoryD1();
  const h = y.parseChart(JSON.parse(fixture("yahoo", "chart_jkse_2y_1d.json")), "2y", "1d");
  mid.cache.set(`history:${y.chartUrl("^JKSE", "2y", "1d")}`, { v: JSON.stringify(h), expires_at: 4e9, fetched_at: Date.parse("2026-11-04T03:00:00Z") / 1000 });
  await withFetch(async () => { throw new Error("served from cache"); }, async () => {
    const e = await astro(mid, "2026-11-06", null, "none", "1", "2y");
    assert.deepEqual([e.data.backtest.returns, e.data.backtest.to], [472, "2026-11-03"]);
    assert.deepEqual(e.warnings, ["the backtest leaves out 2026-11-04, a session that had not closed when the history was fetched"]);
  });

  // Monthly bars are a provider error, a month of closes is not_found; neither is cached.
  for (const [file, code] of [["chart_jkse_range_max_1d.json", "provider"], ["chart_bbca_1mo_1d.json", "not_found"]]) {
    const bad = memoryD1();
    await withFetch(async () => new Response(fixture("yahoo", file), { status: 200 }), async () => {
      await assert.rejects(astro(bad, "2026-11-06", null, "none", "1", "10y"), (err) => err.code === code, file);
    });
    assert.equal(bad.cache.has("astro_backtest:10y"), false);
  }
});

test("/api/history keeps a history that outgrows one D1 row to its most recent candles", async () => {
  // 12,000 daily bars, more than all of ^JKSE since 1990: over 2 MB parsed.
  const n = 12_000, t0 = Date.parse("1977-01-03T02:00:00Z") / 1000;
  const ts = Array.from({ length: n }, (_, i) => t0 + i * 86400);
  const px = ts.map((_, i) => 1000 + i / 7 + 0.123456789);
  const body = JSON.stringify({ chart: { result: [{ meta: { symbol: "^OLD", currency: "USD", dataGranularity: "1d" }, timestamp: ts, indicators: { quote: [{ open: px, high: px, low: px, close: px, volume: ts.map(() => 123456789) }], adjclose: [{ adjclose: px }] } }], error: null } });
  assert.ok(JSON.stringify(y.parseChart(JSON.parse(body), "max", "1d")).length > 2_000_000, "the synthetic history does not fit one row");
  const env = memoryD1();
  await withFetch(async () => new Response(body, { status: 200 }), async () => {
    const e = await history(env, "^OLD", "max", "1d", 0);
    const c = e.data.candles;
    assert.ok(c.length > 9000 && c.length < n, String(c.length));
    assert.equal(c.at(-1).close, px.at(-1));
    assert.equal(e.data.summary.candles, n);
    assert.equal(e.data.summary.first_close, px[0]);
    assert.deepEqual(e.warnings, [`the web terminal keeps the last ${c.length} of ${n} candles (one cache row); summary covers all of them; the CLI returns every candle`]);
    const row = env.cache.get(`history:${y.chartUrl("^OLD", "max", "1d")}`);
    assert.ok(row.v.length <= 1_900_000, String(row.v.length));
  });
  // A history that fits is returned as it is.
  const short = y.parseChart(JSON.parse(fixture("yahoo", "chart_jkse_2y_1d.json")), "2y", "1d");
  assert.equal(fitCacheRow(short), short);
  const all = y.parseChart(JSON.parse(fixture("yahoo", "chart_jkse_period1_max_1d.json")), "max", "1d");
  assert.equal(fitCacheRow(all), all);
});

test("/api/history is keyed on the request, so a row written for 0.8.1's range=max is never served", async () => {
  const env = memoryD1();
  // 0.8.1 cached monthly bars labelled 1d under history:^JKSE:max:1d; they must not come back, even stale.
  const monthly = y.parseChart(JSON.parse(fixture("yahoo", "chart_jkse_range_max_1d.json")), "max", "1d");
  env.cache.set("history:^JKSE:max:1d", { v: JSON.stringify({ ...monthly, interval: "1d" }), expires_at: 0, fetched_at: 0 });
  await withFetch(async () => { throw new Error("Yahoo is down"); }, async () => {
    await assert.rejects(history(env, "^JKSE", "max", "1d", 0), /Yahoo is down/);
  });
});
