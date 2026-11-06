// node --test web/test/*.mjs (no network). /api/fundamental?lens=growth|valuation|risk
// over an in-memory D1 and the recorded Yahoo responses: the same readings as
// `idx stocks growth|valuation|risk` and the Rust tests in src/analysis/lens.rs.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { route } from "../src/api.js";
import * as y from "../src/yahoo.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", "yahoo", p), "utf8");

/** In-memory D1 for the cache statements; quoteSummary is seeded (it needs a crumb session). */
function memoryDb(summaries) {
  const cache = new Map();
  const now = Math.floor(Date.now() / 1000);
  for (const s of summaries) {
    const result = JSON.parse(fixture(`quote_summary_fundamentals_${s.toLowerCase()}.json`)).quoteSummary.result[0];
    cache.set(`summary:${s}:${y.FUNDAMENTAL_MODULES}`, { v: JSON.stringify(result), expires_at: now + 3600, fetched_at: now - 60 });
  }
  const run = (sql, a) => {
    if (sql.startsWith("SELECT v, fetched_at, expires_at FROM cache")) return cache.get(a[0]) || null;
    if (sql.startsWith("INSERT OR REPLACE INTO cache")) return cache.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] });
    throw new Error("memoryDb: unexpected SQL " + sql);
  };
  const stmt = (sql, a = []) => ({ bind: (...b) => stmt(sql, b), first: async () => run(sql, a), run: async () => (run(sql, a), {}) });
  return { DB: { prepare: (sql) => stmt(sql) } };
}

/** Yahoo chart and timeseries from the recordings of 2026-11-06; `down` lists URL parts that fail. */
function recordedFetch(down = []) {
  return async (u) => {
    const url = decodeURIComponent(String(u));
    if (down.some((d) => url.includes(d))) return new Response("{}", { status: 500 });
    const chart = url.match(/finance\/chart\/\^?([A-Z]+)(?:\.JK)?\?range=1y&interval=1d/);
    if (chart) {
      const name = `chart_${chart[1].toLowerCase()}_1y_1d.json`;
      try { return new Response(fixture(name), { status: 200 }); } catch { return new Response(fixture("chart_not_found.json"), { status: 404 }); }
    }
    if (url.includes("fundamentals-timeseries") && url.includes("TLKM.JK")) return new Response(fixture("timeseries_tlkm.json"), { status: 200 });
    throw new Error(`offline: ${url}`);
  };
}

async function withFetch(fn, body) {
  const realFetch = globalThis.fetch;
  globalThis.fetch = fn;
  try { await body(); } finally { globalThis.fetch = realFetch; }
}

const get = (env, qs) => route(env, new URL(`https://ihsg.test/api/fundamental?${qs}`));
const ruleIds = (d) => d.reading.rules.map((r) => [r.rule, r.points]);

test("lens=risk reads a year against IHSG, as stocks risk does", async () => {
  await withFetch(recordedFetch(), async () => {
    const out = await get(memoryDb(["TLKM"]), "symbol=tlkm&lens=risk");
    assert.equal(out.schema, "idx.risk.v1");
    assert.equal(out.warnings, undefined);
    assert.deepEqual([out.data.reading.label, out.data.reading.score], ["moderate", 2]);
    assert.deepEqual(ruleIds(out.data), [["leverage", 0], ["liquidity", 1], ["losses", 0], ["beta", 0], ["volatility", 1], ["drawdown", 0]]);
    assert.equal(out.data.price.observations, 238);
    assert.equal(out.data.price.to, "2026-11-04");
  });
});

test("lens=growth and lens=valuation, and no lens is the full report", async () => {
  await withFetch(recordedFetch(), async () => {
    const env = memoryDb(["TLKM"]);
    const g = await get(env, "symbol=TLKM&lens=growth");
    assert.equal(g.schema, "idx.growth.v1");
    assert.equal(g.data.reading.label, "mixed");
    assert.equal(g.data.annual.length, 4);
    const v = await get(env, "symbol=TLKM&lens=Valuation");
    assert.equal(v.schema, "idx.valuation.v1");
    assert.deepEqual([v.data.reading.label, v.data.reading.score, v.data.reading.inputs_available], ["fair", 2, 7]);
    assert.equal(v.cached, true);
    for (const qs of ["symbol=TLKM", "symbol=TLKM&lens="]) assert.equal((await get(env, qs)).schema, "idx.fundamentals.v1");
    await assert.rejects(get(env, "symbol=TLKM&lens=bogus"), (e) => e.code === "usage" && /growth, valuation or risk/.test(e.message));
  });
});

test("a missing input is a warning; the call fails only without the essential ones", async () => {
  await withFetch(recordedFetch(["chart/^JKSE"]), async () => {
    const out = await get(memoryDb(["TLKM"]), "symbol=TLKM&lens=risk");
    assert.match(out.warnings.join(" | "), /benchmark \^JKSE unavailable: yahoo chart HTTP 500/);
    assert.equal(out.data.price.beta, null);
    assert.ok(out.data.price.volatility_pct > 40);
    assert.deepEqual(ruleIds(out.data), [["leverage", 0], ["liquidity", 1], ["losses", 0]]);
  });
  await withFetch(recordedFetch(["fundamentals-timeseries"]), async () => {
    const out = await get(memoryDb(["TLKM"]), "symbol=TLKM&lens=growth");
    assert.match(out.warnings.join(" | "), /annual statements: yahoo timeseries HTTP 500/);
    assert.equal(out.data.annual.length, 0);
    assert.equal(out.data.reading.inputs_available, 3);
  });
  // Unknown ticker: the chart says not found, which wins over the summary's failure.
  await withFetch(recordedFetch(), async () => {
    await assert.rejects(get(memoryDb([]), "symbol=ZZZZ&lens=risk"), (e) => e.code === "not_found");
  });
});

test("a statements not_found does not hide why the fundamentals failed", async () => {
  // A listed ticker with no annual statements (Yahoo answers with an empty
  // result) while quoteSummary is unreachable: the summary's error is reported.
  const fetchFn = async (u) => {
    const url = decodeURIComponent(String(u));
    if (url.includes("fundamentals-timeseries")) return new Response(fixture("timeseries_not_found.json"), { status: 200 });
    return new Response("{}", { status: 502 });
  };
  await withFetch(fetchFn, async () => {
    await assert.rejects(get(memoryDb([]), "symbol=TLKM&lens=growth"), (e) => {
      assert.notEqual(e.code, "not_found", e.message);
      return true;
    });
  });
  // With the fundamentals cached, the same missing statements are only a warning.
  await withFetch(fetchFn, async () => {
    const out = await get(memoryDb(["TLKM"]), "symbol=TLKM&lens=growth");
    assert.match(out.warnings.join(" | "), /annual statements: /);
    assert.equal(out.data.annual.length, 0);
  });
});
