// node --test web/test/*.mjs (no network). The minute cron's jobs and what
// one invocation spends: the board refresh, the KSEI register sync and the
// bandar scan (whose own tests are in bandar-web.test.mjs).
import { test } from "node:test";
import assert from "node:assert/strict";

import { refreshSnapshot } from "../src/api.js";
import { budget, BudgetError, isBudgetError, hops, CRON_SUBREQUESTS } from "../src/budget.js";
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

test("one budget counts fetches and D1 queries and refuses a fetch past it", async () => {
  assert.ok(CRON_SUBREQUESTS < 50, "below the Free plan's 50 per invocation");
  const sent = [];
  const b = budget(6, { fetchFn: async (u) => { sent.push(u); return new Response("ok"); } });
  await b.fetch("https://www.indopremier.com/a");
  // A GitHub release download redirects to its asset host: two subrequests.
  await b.fetch("https://github.com/fzn0x/idx/releases/download/ownership-data/index.json");
  assert.equal(hops("https://github.com/fzn0x/idx/releases/download/ownership-data/x.json"), 2);
  assert.equal(hops("https://api.github.com/repos/fzn0x/idx"), 1);
  // D1 through the counted binding: a query is one, a batch is one, and the batch gets the plain statements.
  const seen = [];
  const plain = (sql) => ({ sql, bind: () => plain(sql), first: async () => null, run: async () => ({ meta: { changes: 1 } }) });
  const DB = b.db({ prepare: plain, batch: async (list) => { seen.push(...list.map((s) => s.sql)); return []; } });
  await DB.prepare("SELECT 1").first();
  await DB.batch([DB.prepare("INSERT 1").bind(1), DB.prepare("INSERT 2")]);
  assert.deepEqual(seen, ["INSERT 1", "INSERT 2"]);
  assert.deepEqual(b.counts(), { fetch: 3, d1: 2 });
  assert.equal(b.left(), 1);
  await assert.rejects(b.fetch("https://github.com/fzn0x/idx/releases/download/ownership-data/p.json"), BudgetError);
  await b.fetch("https://www.indopremier.com/b");
  await assert.rejects(b.fetch("https://www.indopremier.com/c"), (e) => isBudgetError(e) && /6 of 6/.test(e.message));
  assert.equal(sent.length, 3, "a refused fetch never goes out");
  // The runtime's own errors, if a count were ever off.
  assert.ok(isBudgetError(new Error("Too many subrequests.")));
  assert.ok(isBudgetError(new Error("D1_ERROR: Too many API requests by single Worker invocation")));
  assert.ok(!isBudgetError(new Error("HTTP 404")));
});

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
