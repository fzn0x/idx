// node --test web/test/*.mjs (no network). The minute cron's jobs and what
// one invocation spends: the board refresh, the KSEI register sync and the
// bandar scan (whose own tests are in bandar-web.test.mjs).
import { test } from "node:test";
import assert from "node:assert/strict";

import { refreshSnapshot } from "../src/api.js";
import UNIVERSE from "../src/universe.json" with { type: "json" };

/** A D1 that records every statement it runs and answers the Yahoo session as fresh. */
function recordingDb() {
  const sql = [];
  const session = JSON.stringify({ cookie: "A3=d=test", crumb: "crumb" });
  const stmt = (q) => ({
    bind: () => stmt(q),
    first: async () => { sql.push(q); return q.includes("yahoo:session") ? { v: session, updated_at: Math.floor(Date.now() / 1000) } : null; },
    run: async () => { sql.push(q); return { meta: { changes: 1 } }; },
  });
  return { sql, DB: { prepare: (q) => stmt(q) } };
}

const v7 = (symbol) => ({ symbol, regularMarketPrice: 5100, regularMarketVolume: 1e8, regularMarketTime: 1793956498 });

/** Runs `fn` with the global fetch refusing: every request must go through the fetch a test hands in. */
async function offline(fn) {
  const real = globalThis.fetch;
  globalThis.fetch = async (u) => { throw new Error(`the global fetch was called for ${u}`); };
  try {
    return await fn();
  } finally {
    globalThis.fetch = real;
  }
}

test("a board refresh reads the Yahoo session from D1 once for all its batches", () => offline(async () => {
  const env = recordingDb();
  const urls = [];
  const fetchFn = async (u) => { urls.push(String(u)); return new Response(JSON.stringify({ quoteResponse: { result: [v7("BBCA.JK")] } })); };
  const r = await refreshSnapshot(env, { force: true, fetchFn });
  assert.deepEqual(r.warnings, []);
  assert.equal(urls.length, Math.ceil(UNIVERSE.length / 190) + 1, "the universe in batches of 190, then ^JKSE");
  assert.equal(env.sql.filter((q) => q.includes("yahoo:session")).length, 1);
  assert.equal(env.sql.length, 4, "the session, the board, the index tick and its trim");
}));
