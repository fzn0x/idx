// node --test web/test/*.mjs (no network). The web screen over the board,
// the twin of tests/cli.rs screen_passes_over_pb_and_ps_that_mix_currencies:
// the same recorded quotes and fundamentals, the same rows in the same order.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as y from "../src/yahoo.js";
import { screen } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", p), "utf8");

// D1 as the Worker sees it: `kv` and `cache` rows, nothing written back.
function db({ kv, cache }) {
  const query = (sql, a) => ({
    first: async () => (sql.includes("FROM kv") ? kv[a[0] ?? "yahoo:session"] : sql.includes("FROM cache") ? cache[a[0]] : null) ?? null,
    all: async () => ({ results: [] }),
    run: async () => ({}),
  });
  return { prepare: (sql) => ({ ...query(sql, []), bind: (...a) => query(sql, a) }) };
}

// The 2026-11-06 close: BSSR, GEMS, ESSA, UNIC and ITMG report in USD and
// trade in IDR; BBCA reports in IDR. Fundamentals are cached for ITMG and
// BBCA only, and every fetch fails, so the other four have none.
async function withBoard(run) {
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (url) => { throw new TypeError(`fetch failed for ${url}`); };
  try {
    const now = new Date();
    const ts = Math.floor(now.getTime() / 1000);
    const quotes = JSON.parse(fixture("yahoo/quote_v7_usd_reporters_bbca.json")).quoteResponse.result.map(y.parseV7Quote);
    const summary = (s) => ({ v: JSON.stringify(JSON.parse(fixture(`yahoo/quote_summary_fundamentals_${s.toLowerCase()}.json`)).quoteSummary.result[0]), expires_at: ts + 3600, fetched_at: ts });
    const env = { DB: db({
      kv: {
        board: { v: JSON.stringify({ ts: ts - 60, quotes, index: null, warnings: [] }), updated_at: ts - 60 },
        "yahoo:session": { v: JSON.stringify({ cookie: "A3=C", crumb: "C" }), updated_at: ts },
      },
      cache: { [`summary:ITMG:${y.FUNDAMENTAL_MODULES}`]: summary("ITMG"), [`summary:BBCA:${y.FUNDAMENTAL_MODULES}`]: summary("BBCA") },
    }) };
    await run((filter, sort = null) => screen(env, filter, sort, 25, 20, null, null, false, now));
  } finally {
    globalThis.fetch = realFetch;
  }
}
const symbols = (s) => s.data.rows.map((r) => r.symbol);

test("the web screen passes over a P/B or P/S that mixes currencies, as the CLI does", () => withBoard(async (run) => {
  // The terminal's example screen keeps the USD reporters, values as reported.
  let s = await run("pe<10,div>5,mcap>5T", "div");
  assert.deepEqual(symbols(s), ["BSSR", "GEMS", "ESSA", "UNIC", "ITMG"]);
  const bssr = s.data.rows[0];
  assert.deepEqual([bssr.currency, bssr.financial_currency, bssr.price_to_book], ["IDR", "USD", 48921.57]);

  // No pb clause matches them, low or high, and a pb sort puts them last.
  assert.deepEqual(symbols(await run("pb>0")), ["BBCA"]);
  assert.equal((await run("pb>1000")).data.total_matches, 0);
  s = await run("", "pb");
  assert.deepEqual(symbols(s), ["BBCA", "BSSR", "ESSA", "GEMS", "ITMG", "UNIC"]);
  assert.equal(s.data.rows[0].financial_currency, "IDR");

  // P/S comes with the fundamentals: ITMG's 13807 over USD statements is passed over too.
  s = await run("ps>0");
  assert.deepEqual(symbols(s), ["BBCA"]);
  assert.equal(s.data.fundamentals_checked, 2);
  s = await run("mcap>20T", "ps");
  assert.deepEqual(symbols(s), ["BBCA", "GEMS", "ITMG"]);
  assert.equal(s.data.rows[2].price_to_sales, 13807.103);
}));
