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
import { astro } from "../src/api.js";

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
  for (const args of [["2026-11-06", "91"], ["2026-11-06", "0"], ["1799-12-31"], ["2051-01-01"], ["06/11/2026"], ["2026-11-06", null, "birth"], ["2026-11-06", null, null, "1", "max"], ["2026-11-06", null, null, "1", "6mo"]]) {
    const [date, days, natal, backtest, period] = args;
    await assert.rejects(astro(env, date, days, natal, backtest, period), (err) => err.code === "usage", args.join(" "));
  }
  await assert.rejects(astro(env, "2026-11-06", null, null, "1", "max"), /CLI runs max/);
  await assert.rejects(astro(env, "2026-11-06", "91"), /up to 90|1 to 90/);
});
