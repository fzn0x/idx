// node --test web/test/*.mjs (no network). The Worker side of bandarmology:
// /api/bandar, the scan after each close (src/bandar-scan.js) inside the
// minute cron's budget (src/cron.js), /api/bandar/screen and
// /api/bandar/broker, over an in-memory D1 and the recorded IPOT and Yahoo
// responses. The scan's writes also run on SQLite with web/migrations, so the
// statements production runs are tested as written.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { bandar, bandarScreen, bandarBroker } from "../src/api.js";
import { targetSession, mostTraded, ranking, bandarScanStep, scanStatus, scanNotes } from "../src/bandar-scan.js";
import { budget } from "../src/budget.js";
import worker from "../src/worker.js";

// The scan logs each session's publication for the Worker's logs; these tests read the state instead.
const log = console.log;
console.log = (...a) => { if (!String(a[0]).startsWith("bandar published")) log(...a); };

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const page = (f) => readFileSync(join(root, "tests", "fixtures", "ipot", f), "utf8");
const CHART = readFileSync(join(root, "tests", "fixtures", "yahoo", "chart_bbca_1mo_1d.json"), "utf8");
const quote = (symbol, price, volume, market_time = "2026-10-16T16:14:58+07:00") => ({ symbol, price, volume, previous_close: 6075, market_time });

/**
 * In-memory D1 for the statements the bandar code runs, and those of the
 * board refresh and the register sync that share its cron minute.
 */
function memoryDb() {
  const kv = new Map(), kvAt = new Map(), cache = new Map(), scan = new Map();
  // The Yahoo session's statements name their key inline.
  const inlineKey = (sql) => sql.match(/k = '([^']+)'|VALUES \('([^']+)'/)?.slice(1).find(Boolean);
  const run = (sql, a) => {
    if (sql.startsWith("SELECT v, updated_at FROM kv")) { const k = inlineKey(sql) ?? a[0]; return kv.has(k) ? { v: kv.get(k), updated_at: kvAt.get(k) ?? 0 } : null; }
    if (sql.startsWith("SELECT v FROM kv")) return kv.has(a[0]) ? { v: kv.get(a[0]) } : null;
    if (sql.startsWith("INSERT OR REPLACE INTO kv")) {
      const k = inlineKey(sql);
      const [key, v, at] = k ? [k, a[0], a[1]] : a;
      kvAt.set(key, at);
      return kv.set(key, v);
    }
    if (sql.startsWith("INSERT INTO kv") && sql.includes("ON CONFLICT(k) DO UPDATE")) {
      // The scan's compare-and-set: written over the revision it read (a[3]), or not at all.
      const held = kv.get(a[0]);
      if (held != null && (JSON.parse(held).rev ?? null) !== (a[3] ?? null)) return { meta: { changes: 0 } };
      kv.set(a[0], a[1]);
      kvAt.set(a[0], a[2]);
      return { meta: { changes: 1 } };
    }
    if (sql.startsWith("INSERT OR REPLACE INTO ticks") || sql.startsWith("DELETE FROM ticks")) return null;
    if (sql.startsWith("SELECT as_of, sha256, parts_key FROM own_releases")) return [];
    if (sql.startsWith("SELECT v, fetched_at, expires_at FROM cache")) return cache.get(a[0]) || null;
    if (sql.startsWith("INSERT OR REPLACE INTO cache")) return cache.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] });
    if (sql.startsWith("INSERT OR REPLACE INTO bandar_scan")) return scan.set([a[0], a[1], a[2]].join("|"), a[3]);
    if (sql.startsWith("SELECT symbol, v FROM bandar_scan")) {
      return [...scan].filter(([k]) => k.startsWith([a[0], a[1], ""].join("|"))).map(([k, v]) => ({ symbol: k.split("|")[2], v }));
    }
    if (sql.startsWith("DELETE FROM bandar_scan")) {
      // Only while the state a[2] holds is the one this batch wrote (a[3]):
      // then the newest a[0] sessions stay, and the one shown (a[1]).
      if (kv.get(a[2]) !== a[3]) return null;
      const sessions = [...new Set([...scan.keys()].map((k) => k.split("|")[0]))].sort().reverse();
      const keep = new Set([...sessions.slice(0, a[0]), a[1]]);
      for (const k of [...scan.keys()]) if (!keep.has(k.split("|")[0])) scan.delete(k);
      return null;
    }
    throw new Error("memoryDb: unexpected SQL " + sql);
  };
  const stmt = (sql, a = []) => ({
    bind: (...b) => stmt(sql, b),
    first: async () => run(sql, a),
    run: async () => (run(sql, a), {}),
    all: async () => ({ results: run(sql, a) }),
    exec: () => run(sql, a),
  });
  return { DB: { prepare: (sql) => stmt(sql), batch: async (list) => list.map((s) => s.exec()) }, kv, scan };
}

/**
 * IPOT and Yahoo from the recorded pages: BBCA, BRPT, TLKM; ZZZZ is unknown.
 * A page recorded for the exact range answers it; any other range gets the
 * stock's 10-12..10-16 page (it stands in for the 1m and 3m periods).
 */
function recordedFetch(calls = []) {
  return async (u) => {
    const url = String(u);
    calls.push(url);
    if (url.includes("finance/chart")) return new Response(CHART, { status: 200 });
    const code = url.match(/code=([A-Z]+)/)[1].toLowerCase();
    const d = url.match(/start=(\d\d)\/(\d\d)\/(\d{4})&end=(\d\d)\/(\d\d)\/(\d{4})/);
    const [start, end] = [d[3] + d[1] + d[2], d[6] + d[4] + d[5]];
    if (code === "zzzz") return new Response(page("brokersummary_zzzz_invalid.html"), { status: 200 });
    const side = url.includes("fd=F") ? "_foreign" : url.includes("fd=D") ? "_domestic" : "";
    const exact = start === end ? `brokersummary_${code}_${end}${side}.html` : `brokersummary_${code}_${start}_${end}${side}.html`;
    let body;
    try { body = page(exact); } catch { body = page(`brokersummary_${code}_20261012_20261016.html`); }
    return new Response(body, { status: 200 });
  };
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

const sqlite = await import("node:sqlite").catch(() => null);
const noSqlite = sqlite ? false : "node:sqlite is not available in this Node";

/**
 * D1 over an in-memory SQLite with the Worker's migrations, as in
 * ownership-sync.test.mjs, and the reads these tests check it with.
 */
function sqliteDb() {
  const db = new sqlite.DatabaseSync(":memory:");
  const dir = join(root, "web", "migrations");
  for (const f of readdirSync(dir).filter((f) => f.endsWith(".sql")).sort()) db.exec(readFileSync(join(dir, f), "utf8"));
  // node:sqlite binds ?NNN only by name; rewrite them to $pNNN.
  const named = (sql) => sql.replace(/\?(\d+)/g, "$$p$1");
  const stmt = (sql, args) => {
    const params = Object.fromEntries(args.map((v, i) => [`p${i + 1}`, v === undefined ? null : v]));
    const s = () => db.prepare(named(sql));
    return {
      bind: (...a) => stmt(sql, a),
      first: async () => s().get(params) ?? null,
      all: async () => ({ results: s().all(params) }),
      run: async () => ({ meta: { changes: Number(s().run(params).changes) } }),
    };
  };
  const batch = async (list) => {
    db.exec("BEGIN");
    try {
      const out = [];
      for (const st of list) out.push(await st.run());
      db.exec("COMMIT");
      return out;
    } catch (e) {
      db.exec("ROLLBACK");
      throw e;
    }
  };
  return {
    DB: { prepare: (sql) => stmt(sql, []), batch },
    put: (k, v) => db.prepare("INSERT OR REPLACE INTO kv (k, v, updated_at) VALUES (?, ?, 0)").run(k, typeof v === "string" ? v : JSON.stringify(v)),
    get: (k) => db.prepare("SELECT v FROM kv WHERE k = ?").get(k)?.v ?? null,
    row: (session, symbol) => { for (const p of ["1d", "1w", "1m", "3m"]) db.prepare("INSERT INTO bandar_scan (session, period, symbol, v) VALUES (?, ?, ?, '{}')").run(session, p, symbol); },
    sessions: () => db.prepare("SELECT DISTINCT session FROM bandar_scan ORDER BY session").all().map((r) => r.session),
  };
}

/** Scan steps at `now`, one per cron minute, until the session is done; the last step's result. */
async function scanAll(env, now, opts = {}) {
  for (let i = 0; i < 40; i++) {
    const r = await bandarScanStep(env, { now, ...opts });
    if (r.done || r.idle) return r;
  }
  throw new Error("the scan did not finish in 40 steps");
}

test("bandar broker reads one broker across the scan like the CLI", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  await withFetch(recordedFetch(), async () => {
    await assert.rejects(bandarBroker(env, "YP"), (e) => e.code === "not_found");
    await scanAll(env, new Date("2026-10-17T03:00:00Z"));
    const out = await bandarBroker(env, " yp ", "1w");
    assert.equal(out.schema, "idx.bandar.broker.v1");
    assert.equal(out.provider, "ipot");
    const a = out.data;
    assert.deepEqual([a.code, a.name, a.from, a.to], ["YP", "MIRAE ASSET SEKURITAS INDONESIA", "2026-10-12", "2026-10-16"]);
    assert.deepEqual([a.scanned, a.checked, a.failed], [4, 3, 1]);
    // Same as the Rust test: a top-ten buyer of BBCA and TLKM, a top-ten seller of BRPT and TLKM.
    assert.deepEqual(a.bought.map((r) => r.symbol).sort(), ["BBCA", "TLKM"]);
    assert.deepEqual(a.sold.map((r) => r.symbol).sort(), ["BRPT", "TLKM"]);
    const bbca = a.bought.find((r) => r.symbol === "BBCA");
    assert.deepEqual([bbca.rank, bbca.opposite_value, bbca.net_value], [9, null, null]);
    const tb = a.bought.find((r) => r.symbol === "TLKM"), ts = a.sold.find((r) => r.symbol === "TLKM");
    assert.equal(tb.opposite_value, ts.value);
    assert.equal(tb.net_value, ts.net_value);
    assert.equal(tb.net_value, tb.value - ts.value);
    assert.ok(a.bought[0].value >= a.bought[1].value);
    assert.equal(a.bought_value, a.bought.reduce((t, r) => t + r.value, 0));
    const one = await bandarBroker(env, "YP", "1w", "1");
    assert.deepEqual([one.data.bought.length, one.data.sold.length], [1, 1]);
    assert.equal(one.data.bought_value, a.bought_value, "sums cover every row");
    await assert.rejects(bandarBroker(env, "MIRAE"), (e) => e.code === "usage");
    await assert.rejects(bandarBroker(env, "YP", "2w"), (e) => e.code === "usage");
  });
});

test("the scan targets the last finished session", () => {
  assert.equal(targetSession(new Date("2026-10-16T03:00:00Z")), "2026-10-15", "Friday 10:00 WIB: Thursday");
  assert.equal(targetSession(new Date("2026-10-16T09:50:00Z")), "2026-10-16", "Friday 16:50 WIB: Friday");
  assert.equal(targetSession(new Date("2026-10-17T03:00:00Z")), "2026-10-16", "Saturday: Friday");
  assert.equal(targetSession(new Date("2026-10-19T01:00:00Z")), "2026-10-16", "Monday 08:00 WIB: Friday");
  // Listed holidays are never targets: 12-24 and 12-25 close, 12-26 and 12-27 are a weekend.
  assert.equal(targetSession(new Date("2026-12-24T10:00:00Z")), "2026-12-23", "Christmas Eve 17:00 WIB: the 23rd");
  assert.equal(targetSession(new Date("2026-12-28T01:00:00Z")), "2026-12-23", "Monday 28th 08:00 WIB: the 23rd");
  // 2028 is not on the bundled list: its weekdays are trading days, 2027-12-31 is still listed.
  assert.equal(targetSession(new Date("2028-01-03T01:00:00Z")), "2027-12-30", "Monday 2028-01-03 08:00 WIB: past the listed 2027-12-31");
  assert.equal(targetSession(new Date("2028-01-04T10:00:00Z")), "2028-01-04", "a weekday of an uncovered year");
  assert.deepEqual(mostTraded([quote("A", 10, 5), quote("B", 100, 5), quote("^JKSE", 7000, 9e9), quote("C", 1, 0)], 5), ["B", "A"]);
});

test("the ranking counts the stocks that traded on the session", () => {
  const idle = quote("TMPO", 1000, 5, "2026-10-14T15:59:00+07:00"); // last traded two sessions earlier
  assert.deepEqual(mostTraded([quote("A", 10, 5), quote("B", 100, 5), idle], 5, "2026-10-16"), ["B", "A"]);
  assert.deepEqual(ranking([quote("A", 10, 5), idle], "2026-10-16"), { symbols: ["A"], ranked_from: "2026-10-16" });
  // The board has moved on to the next session: what traded since comes first; the idle stock stays out.
  const next = quote("B", 1, 5, "2026-10-19T10:01:00+07:00");
  assert.deepEqual(ranking([quote("A", 10, 5), next, idle], "2026-10-16"), { symbols: ["B", "A"], ranked_from: "2026-10-19" });
  // A board without a quote of the session, refreshed before it traded.
  assert.deepEqual(ranking([idle], "2026-10-16"), { symbols: ["TMPO"], ranked_from: "2026-10-14" });
});

test("once the next session opens, the stocks that traded since still rank first, and the screen says which board ranked them", async () => {
  const fri = "2026-11-06T16:14:58+07:00", mon = "2026-11-09T09:01:00+07:00";
  // Stock i was the (i+1)-th most traded on Friday; the 40 most traded have traded on Monday too, a few minutes' worth.
  const board = Array.from({ length: 300 }, (_, i) => (i < 40 ? quote(`S${i}`, 1000, (300 - i) * 10, mon) : quote(`S${i}`, 1000, (300 - i) * 1e6, fri)));
  const r = ranking(board, "2026-11-06");
  assert.deepEqual([...r.symbols].sort(), Array.from({ length: 100 }, (_, i) => `S${i}`).sort(), "Friday's 100 most traded");
  assert.equal(r.ranked_from, "2026-11-09");
  // Scanned from that board, the screen says so.
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-09T09:02:00+07:00"), quotes: [quote("BBCA", 5100, 2e6, mon), quote("TLKM", 2690, 1e8, fri)] }));
  await withFetch(recordedFetch(), async () => { await scanAll(env, new Date("2026-11-09T10:00:00+07:00")); });
  const out = await bandarScreen(env, null, null, "1d", null, null, null, null, new Date("2026-11-09T10:05:00+07:00"));
  assert.deepEqual([out.data.to, out.data.scanned], ["2026-11-06", 2]);
  assert.ok(out.warnings.includes("the scan of 2026-11-06 picked its 2 stocks from the quote board of 2026-11-09, not from that session's close"), out.warnings.join("\n"));
});

test("the scan fills D1 and the screen reads it like the CLI", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  await withFetch(recordedFetch(), async () => {
    await assert.rejects(bandarScreen(env, "big-distribution"), (e) => e.code === "not_found");
    const now = new Date("2026-10-17T03:00:00Z");
    // The probe finds the lists published; three stocks a minute, so ZZZZ takes a second step.
    const first = await bandarScanStep(env, { now });
    assert.deepEqual([first.session, first.probe, first.read, first.scanned, first.of, first.done], ["2026-10-16", "published", 3, 3, 4, false]);
    const second = await bandarScanStep(env, { now });
    assert.deepEqual([second.read, second.scanned, second.done], [1, 4, true]);
    assert.deepEqual(await bandarScanStep(env, { now }), { idle: true, session: "2026-10-16" });
    const st = await scanStatus(env);
    assert.equal(st.complete, "2026-10-16");
    assert.deepEqual(st.failed, ["ZZZZ"]);
    assert.equal(env.scan.size, 12, "three stocks x four periods");

    const out = await bandarScreen(env, "big-distribution", "turnover>1B", "1w");
    assert.equal(out.schema, "idx.bandar.screen.v1");
    assert.equal(out.provider, "ipot");
    const s = out.data;
    assert.deepEqual([s.from, s.to, s.preset, s.descending], ["2026-10-12", "2026-10-16", "big-distribution", false]);
    assert.deepEqual(s.filters, ["acc<=-15", "turnover>1B"]);
    assert.deepEqual([s.scanned, s.checked], [4, 3]);
    assert.deepEqual(s.rows.map((r) => r.symbol), ["BBCA", "TLKM"], "same order as the Rust screen test");
    assert.equal(s.rows[0].phase, "markdown");
    // The week pages count the 16th twice (BBCA -4.1 T, TLKM -319.8 B); the
    // scan rebuilds the nets from single sessions like the CLI.
    assert.deepEqual(s.rows.map((r) => r.foreign_net_value), [-1_823_100_000_000, -204_700_000_000]);
    assert.ok(Math.abs(s.rows[0].price_change_pct - (5175 / 6050 - 1) * 100) < 1e-9);
    assert.ok(out.warnings.includes("1 of 4 scanned stocks had no broker summary"), out.warnings);

    const all = await bandarScreen(env, null, "acc>-100", "1w", "10", "fnet", "0", "1");
    assert.equal(all.data.rows.length, 1);
    assert.equal(all.data.total_matches, 3);
    assert.equal(all.data.sort, "fnet");
    for (const bad of [["everything"], [null, "pe<10"], [null, null, "2w"], [null, null, null, "4"], [null, null, null, null, "phase"]]) {
      await assert.rejects(bandarScreen(env, ...bad), (e) => e.code === "usage", JSON.stringify(bad));
    }
  });
});

const unix = (iso) => Math.floor(Date.parse(iso) / 1000);

test("a page without trades waits for IPOT while the board shows trades that day", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: unix("2026-10-16T16:45:00+07:00"), quotes: [quote("BBCA", 5175, 9e8)] }));
  const calls = [];
  await withFetch(async (u) => { calls.push(String(u)); return new Response(page("brokersummary_bbca_20261017_no_trades.html"), { status: 200 }); }, async () => {
    // Saturday 10:00 WIB: Friday's page shows nothing, but the board's BBCA last traded on Friday.
    assert.equal((await bandarScanStep(env, { now: new Date("2026-10-17T03:00:00Z") })).probe, "unpublished");
    const st = await scanStatus(env);
    assert.deepEqual([st.phase, st.holiday, st.probes, st.next_probe_at], ["waiting", false, 1, "2026-10-17T03:30:00.000Z"]);
    assert.equal(env.scan.size, 0);
    // Before the next probe a step reads its state and nothing else.
    assert.equal((await bandarScanStep(env, { now: new Date("2026-10-17T03:20:00Z") })).idle, true);
    assert.equal(calls.length, 1);
    await bandarScanStep(env, { now: new Date("2026-10-17T03:30:00Z") });
    assert.deepEqual([calls.length, (await scanStatus(env)).probes], [2, 2]);
  });
});

test("in a year the calendar does not cover, a day without trades at IPOT or on the board is closed", async () => {
  const now = new Date("2028-01-03T10:00:00Z"); // Monday 17:00 WIB; 2028 is not on the bundled list
  const board = (market_time) => JSON.stringify({ ts: unix("2028-01-03T16:45:00+07:00"), quotes: [quote("BBCA", 5175, 9e8, market_time)] });
  await withFetch(async () => new Response(page("brokersummary_bbca_20261017_no_trades.html"), { status: 200 }), async () => {
    // The board, refreshed after that session, last traded on 2027-12-30.
    const env = memoryDb();
    env.kv.set("board", board("2027-12-30T16:14:58+07:00"));
    const r = await bandarScanStep(env, { now });
    assert.deepEqual([r.session, r.probe, r.phase], ["2028-01-03", "closed", "closed"]);
    const st = await scanStatus(env);
    assert.deepEqual([st.phase, st.holiday, st.ranked_from], ["closed", true, "2027-12-30"]);
    assert.deepEqual(await bandarScanStep(env, { now }), { idle: true, session: "2028-01-03" });
    // The screen says why 2028-01-03 is not shown, that it does not count as a
    // session behind, and that the calendar does not cover the year.
    const notes = scanNotes(JSON.parse(env.kv.get("bandar_scan")), "2027-12-30", ["BBCA"], now);
    assert.deepEqual(notes.warnings, [
      "2028-01-03 had no trades at IPOT or on the quote board, so it was not scanned",
      "the bundled holiday list does not cover 2028; the scan tells that year's closures from IPOT and the quote board",
    ]);
    // The same page while the board shows trades that day: IPOT has not published.
    const traded = memoryDb();
    traded.kv.set("board", board("2028-01-03T16:14:58+07:00"));
    assert.equal((await bandarScanStep(traded, { now })).probe, "unpublished");
    const t = await scanStatus(traded);
    assert.deepEqual([t.phase, t.holiday], ["waiting", false]);
  });
});

test("every closure after the shown session stays on record, however many there are", async () => {
  const env = memoryDb();
  // 2028 is not on the bundled list; 02-28 to 03-03 are closed (a week like Eid's).
  const closed = ["2028-02-28", "2028-02-29", "2028-03-01", "2028-03-02", "2028-03-03"];
  const board = (dated, at) => env.kv.set("board", JSON.stringify({ ts: unix(`${at}T16:45:00+07:00`), quotes: [quote("BBCA", 5175, 9e8, `${dated}T16:14:58+07:00`), quote("TLKM", 2690, 3e8, `${dated}T16:14:58+07:00`)] }));
  const recorded = recordedFetch();
  const fetchFn = async (u, init) => {
    const d = String(u).match(/start=(\d\d)\/(\d\d)\/(\d{4})&end=(\d\d)\/(\d\d)\/(\d{4})/);
    const day = d && d[1] + d[2] === d[4] + d[5] ? `${d[6]}-${d[4]}-${d[5]}` : null;
    if (closed.includes(day)) return new Response(page("brokersummary_bbca_20261017_no_trades.html"));
    if (day === "2028-03-06") return new Response(page("brokersummary_bbca_20261106_insession.html"));
    return recorded(u, init);
  };
  await withFetch(fetchFn, async () => {
    board("2028-02-25", "2028-02-25");
    await scanAll(env, new Date("2028-02-25T17:00:00+07:00"));
    for (const d of closed) {
      board("2028-02-25", d);
      assert.equal((await bandarScanStep(env, { now: new Date(`${d}T16:45:00+07:00`) })).probe, "closed");
    }
    board("2028-03-06", "2028-03-06");
    assert.equal((await bandarScanStep(env, { now: new Date("2028-03-06T16:45:00+07:00") })).probe, "unpublished");
  });
  const out = await bandarScreen(env, null, null, "1w", null, null, null, null, new Date("2028-03-06T16:50:00+07:00"));
  assert.equal(out.data.to, "2028-02-25");
  for (const d of closed) assert.ok(out.warnings.includes(`${d} had no trades at IPOT or on the quote board, so it was not scanned`), `${d}: ${out.warnings.join("\n")}`);
  assert.ok(!out.warnings.some((w) => w.includes("sessions behind")), out.warnings.join("\n"));
});

test("while the board cannot tell whether the session traded, each probe asks the board again", async () => {
  const env = memoryDb();
  // 2028 is not on the bundled list and 2028-01-26 is closed; Yahoo failed from 16:00 WIB, so at 16:45 the board is from 15:59.
  const board = (hm) => JSON.stringify({ ts: unix(`2028-01-26T${hm}:00+07:00`), quotes: [quote("BBCA", 5175, 9e8, "2028-01-25T16:14:58+07:00")] });
  env.kv.set("board", board("15:59"));
  await withFetch(async () => new Response(page("brokersummary_bbca_20261017_no_trades.html")), async () => {
    assert.equal((await bandarScanStep(env, { now: new Date("2028-01-26T16:45:00+07:00") })).probe, "unpublished");
    assert.equal(JSON.parse(env.kv.get("bandar_scan")).board_traded, null);
    // Refreshed after the close, the board shows no trade that day either.
    env.kv.set("board", board("16:50"));
    const r = await bandarScanStep(env, { now: new Date("2028-01-26T16:55:00+07:00") });
    assert.deepEqual([r.probe, r.phase], ["closed", "closed"]);
  });
});

test("a session IPOT has not published waits, is probed again every 10 minutes, then scanned", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: [quote("BBCA", 5100, 2e8, "2026-11-06T16:14:58+07:00")] }));
  let published = false;
  const calls = [];
  const recorded = recordedFetch(calls);
  // Until `published`, BBCA's 11-06 page is the one recorded during that session: totals, no lists.
  const fetchFn = async (u, init) => {
    if (!published && String(u).includes("code=BBCA&start=11/06/2026&end=11/06/2026")) { calls.push(String(u)); return new Response(page("brokersummary_bbca_20261106_insession.html")); }
    return recorded(u, init);
  };
  const at = (hm) => new Date(`2026-11-06T${hm}:00+07:00`);
  await withFetch(fetchFn, async () => {
    assert.equal((await bandarScanStep(env, { now: at("16:45") })).probe, "unpublished");
    let st = await scanStatus(env);
    assert.deepEqual([st.phase, st.holiday, st.next_probe_at], ["waiting", false, at("16:55").toISOString()]);
    assert.equal((await bandarScanStep(env, { now: at("16:50") })).idle, true);
    assert.equal(calls.length, 1, "no request before the next probe");
    await assert.rejects(bandarScreen(env, null), (e) => e.code === "not_found");
    assert.equal((await bandarScanStep(env, { now: at("16:55") })).probe, "unpublished");
    // From 21:00 WIB every 30 minutes.
    await bandarScanStep(env, { now: at("21:10") });
    assert.equal((await scanStatus(env)).next_probe_at, at("21:40").toISOString());
    published = true;
    const r = await bandarScanStep(env, { now: at("21:40") });
    assert.deepEqual([r.probe, r.read, r.done], ["published", 1, true]);
    st = await scanStatus(env);
    assert.deepEqual([st.complete, st.probes, st.published_at], ["2026-11-06", 4, at("21:40").toISOString()]);
    assert.equal(env.scan.size, 4);
  });
});

test("while IPOT fails the probe, the probe backs off up to the probe's own gap and an hour of failures logs an error each hour", async () => {
  const env = memoryDb();
  const traded = "2026-11-06T16:14:58+07:00";
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: [quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded)] }));
  const probed = [];
  const wib = (ms) => new Date(ms + 7 * 3600e3).toISOString().slice(11, 16);
  let clock = 0;
  const fetchFn = async () => { probed.push(wib(clock)); return new Response("bad gateway", { status: 502 }); };
  const { error } = console;
  const errors = [];
  console.error = (...a) => errors.push(`${wib(clock)} ${a.join(" ")}`);
  try {
    await withFetch(fetchFn, async () => {
      for (clock = Date.parse("2026-11-06T16:45:00+07:00"); clock <= Date.parse("2026-11-06T19:30:00+07:00"); clock += 60e3) {
        await bandarScanStep(env, { now: new Date(clock), stocks: false });
      }
    });
  } finally {
    console.error = error;
  }
  // 5 minutes, then 10: never longer than the 10 minutes between probes before 21:00 WIB, so
  // the scan is back on that schedule as soon as IPOT answers.
  const hm = (m) => `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
  assert.deepEqual(probed, ["16:45", "16:50", ...Array.from({ length: 16 }, (_, i) => hm(17 * 60 + 10 * i))], probed.join(" "));
  assert.deepEqual(errors.map((e) => e.slice(0, 5)), ["17:50", "18:50"], errors.join("\n"));
  assert.match(errors[0], /IPOT has failed the probe of 2026-11-06 since 2026-11-06 16:45 WIB \(8 in a row, the last: HTTP 502\)/);
  const st = await scanStatus(env);
  assert.deepEqual([st.phase, st.probes, st.probe_errors, st.probe_error], ["waiting", 0, 18, "HTTP 502"]);
});

test("a probe stock without trades that session does not hold the scan up: the next probe asks the next stock", async () => {
  const env = memoryDb();
  const traded = "2026-11-06T16:14:58+07:00";
  // TMPO tops the board, but IPOT shows no trades for it on 11-06 (suspended); BBCA's lists are out.
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: [quote("TMPO", 9000, 2e8, traded), quote("BBCA", 5100, 2e8, traded)] }));
  const recorded = recordedFetch();
  const fetchFn = async (u, init) => (String(u).includes("code=TMPO") ? new Response(page("brokersummary_bbca_20261017_no_trades.html")) : recorded(u, init));
  const at = (hm) => new Date(`2026-11-06T${hm}:00+07:00`);
  await withFetch(fetchFn, async () => {
    assert.equal((await bandarScanStep(env, { now: at("16:45"), stocks: false })).probe, "unpublished");
    assert.equal((await scanStatus(env)).probe_symbol, "TMPO");
    assert.equal((await bandarScanStep(env, { now: at("16:55"), stocks: false })).probe, "published");
    const st = await scanStatus(env);
    assert.deepEqual([st.probe_symbol, st.phase, st.probes], ["BBCA", "running", 2]);
  });
});

test("a probe stock IPOT does not know says nothing of the session: the next one is asked in the same minute", async () => {
  const traded = "2026-11-06T16:14:58+07:00";
  const board = (quotes) => JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes });
  const calls = [];
  const recorded = recordedFetch(calls);
  // ZZZZ (a new listing IPOT does not know yet) tops the board; BBCA's 11-06 lists are not out.
  const fetchFn = async (u, init) => {
    if (String(u).includes("code=BBCA&start=11/06/2026&end=11/06/2026")) { calls.push(String(u)); return new Response(page("brokersummary_bbca_20261106_insession.html")); }
    return recorded(u, init);
  };
  const at = (hm) => new Date(`2026-11-06T${hm}:00+07:00`);
  await withFetch(fetchFn, async () => {
    const env = memoryDb();
    env.kv.set("board", board([quote("ZZZZ", 9000, 9e8, traded), quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded)]));
    const r = await bandarScanStep(env, { now: at("16:45"), stocks: false });
    assert.deepEqual([r.probe, r.phase], ["unpublished", "waiting"]);
    assert.deepEqual(calls.map((u) => u.match(/code=([A-Z]+)/)[1]), ["ZZZZ", "BBCA"]);
    const st = await scanStatus(env);
    assert.deepEqual([st.phase, st.probes, st.probe_symbol], ["waiting", 1, "BBCA"]);
    // The next probe does not ask ZZZZ again.
    calls.length = 0;
    await bandarScanStep(env, { now: at("16:55"), stocks: false });
    assert.equal(calls.length, 1);
    assert.ok(!calls[0].includes("ZZZZ"), calls[0]);
    // When IPOT knows none of them, the stocks speak for themselves.
    const unknown = memoryDb();
    unknown.kv.set("board", board([quote("ZZZZ", 9000, 9e8, traded)]));
    assert.equal((await bandarScanStep(unknown, { now: at("16:45"), stocks: false })).probe, "published");
  });
});

test("a stock IPOT has not published is read again, not stored without its lists", async () => {
  const env = memoryDb();
  const traded = "2026-11-06T16:14:58+07:00";
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: [quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded)] }));
  let published = false;
  const calls = [];
  const recorded = recordedFetch(calls);
  // BBCA's 11-06 lists are out; TLKM's 11-06 page shows totals without lists until `published`.
  const fetchFn = async (u, init) => {
    if (!published && String(u).includes("code=TLKM&start=11/06/2026&end=11/06/2026")) { calls.push(String(u)); return new Response(page("brokersummary_bbca_20261106_insession.html")); }
    return recorded(u, init);
  };
  const at = (hm) => new Date(`2026-11-06T${hm}:00+07:00`);
  const tlkmRows = () => [...env.scan.keys()].filter((k) => k.endsWith("|TLKM")).length;
  await withFetch(fetchFn, async () => {
    const r = await bandarScanStep(env, { now: at("17:00") });
    assert.deepEqual([r.probe, r.read], ["published", 2]);
    assert.equal(tlkmRows(), 0, "nothing stored for TLKM");
    assert.deepEqual(calls.filter((u) => u.includes("TLKM")), [calls.find((u) => u.includes("code=TLKM"))], "one request: its session's page");
    let st = await scanStatus(env);
    assert.deepEqual([st.phase, st.complete, st.failed, st.pending.map((p) => p.symbol)], ["running", "2026-11-06", [], ["TLKM"]]);
    const shown = await bandarScreen(env, null, null, "1d", null, null, null, null, at("17:05"));
    assert.ok(shown.warnings.includes("IPOT has not published 2026-11-06 for 1 of the 2 stocks yet; the scan reads it again at 17:10 WIB"), shown.warnings);
    // Not before ten minutes; then once more, still without lists.
    calls.length = 0;
    assert.equal((await bandarScanStep(env, { now: at("17:05") })).idle, true);
    assert.equal(calls.length, 0);
    await bandarScanStep(env, { now: at("17:10") });
    assert.deepEqual([calls.length, tlkmRows()], [1, 0]);
    // IPOT publishes it: the next check reads it whole and the session is done.
    published = true;
    const done = await bandarScanStep(env, { now: at("17:20") });
    assert.equal(done.done, true);
    st = await scanStatus(env);
    assert.deepEqual([st.phase, st.pending, st.failed], ["done", [], []]);
    assert.equal(tlkmRows(), 4);
  });
});

/**
 * IPOT for made-up codes: every page is BBCA's (11-06 for a single day, else
 * its 10-12..10-16 week), and a code in `late` shows 11-06 without its lists.
 */
function lateFetch(late) {
  const day = page("brokersummary_bbca_20261106.html"), range = page("brokersummary_bbca_20261012_20261016.html"), unpub = page("brokersummary_bbca_20261106_insession.html");
  return async (u) => {
    const url = String(u);
    if (url.includes("finance/chart")) return new Response(CHART);
    const code = url.match(/code=([A-Z]+)/)[1];
    if (url.includes("start=11/06/2026&end=11/06/2026")) return new Response(late.has(code) ? unpub : day);
    return new Response(range);
  };
}
const lateBoard = (codes) => JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: codes.map((s, i) => quote(s, 1000, 1e8 - i, "2026-11-06T16:14:58+07:00")) });

test("a page without trades for a stock the board shows trading that session waits like one IPOT has not published", async () => {
  const env = memoryDb();
  const traded = "2026-11-06T16:14:58+07:00";
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: [quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded)] }));
  const recorded = recordedFetch();
  let published = false;
  // BBCA's lists are out; TLKM's 11-06 page shows no trades at all until `published`.
  const fetchFn = async (u, init) => (!published && String(u).includes("code=TLKM&start=11/06/2026&end=11/06/2026") ? new Response(page("brokersummary_bbca_20261017_no_trades.html")) : recorded(u, init));
  const at = (hm) => new Date(`2026-11-06T${hm}:00+07:00`);
  await withFetch(fetchFn, async () => {
    await bandarScanStep(env, { now: at("17:00") });
    let st = await scanStatus(env);
    assert.deepEqual([st.phase, st.pending.map((p) => p.symbol), st.with_trades], ["running", ["TLKM"], 1]);
    assert.equal([...env.scan.keys()].filter((k) => k.endsWith("|TLKM")).length, 0, "nothing stored for TLKM");
    published = true;
    await bandarScanStep(env, { now: at("17:10") });
    st = await scanStatus(env);
    assert.deepEqual([st.phase, st.pending, st.with_trades], ["done", [], 2]);
  });
});

test("stocks IPOT has not published are read again apart from the pass, and count as unpublished meanwhile", async () => {
  const env = memoryDb();
  env.kv.set("board", lateBoard(["AAAA", "BBBB", "CCCC", "DDDD", "EEEE"]));
  const at = (hm) => new Date(`2026-11-06T${hm}:00+07:00`);
  const screen = (hm) => bandarScreen(env, null, null, "1d", null, null, null, null, at(hm));
  await withFetch(lateFetch(new Set(["BBBB", "CCCC", "DDDD", "EEEE"])), async () => {
    for (const hm of ["17:00", "17:01"]) await bandarScanStep(env, { now: at(hm) });
    // 17:10: the four are read again, three this minute; the fourth waits for the next.
    assert.equal((await bandarScanStep(env, { now: at("17:10") })).read, 3);
    const out = await screen("17:10");
    assert.equal(out.data.scanned, 5);
    assert.deepEqual(out.warnings, ["IPOT has not published 2026-11-06 for 4 of the 5 stocks yet; the scan is reading them again"]);
    // Cut off now, the session counts all four as unpublished and none as left unread.
    await bandarScanStep(env, { now: new Date("2026-11-09T16:45:00+07:00"), stocks: false });
    const last = (await scanStatus(env)).last;
    assert.deepEqual([last.session, last.stored, last.unpublished.length, last.left], ["2026-11-06", 1, 4, 0]);
  });
});

test("while stocks are read again, the screen counts each of them once", async () => {
  const env = memoryDb();
  env.kv.set("board", lateBoard(["AAAA", "BBBB", "CCCC", "DDDD", "EEEE", "FFFF", "GGGG", "HHHH"]));
  const at = (hm) => new Date(`2026-11-06T${hm}:00+07:00`);
  const late = new Set(["BBBB", "CCCC", "DDDD", "EEEE", "FFFF"]);
  await withFetch(lateFetch(late), async () => {
    for (const hm of ["17:00", "17:01", "17:02"]) await bandarScanStep(env, { now: at(hm) });
    // IPOT publishes them; at 17:10 three are read whole, two wait for the next minute.
    late.clear();
    assert.equal((await bandarScanStep(env, { now: at("17:10") })).read, 3);
    const out = await bandarScreen(env, null, null, "1d", null, null, null, null, at("17:10"));
    assert.deepEqual([out.data.scanned, out.data.checked], [8, 6]);
    assert.deepEqual(out.warnings, ["IPOT has not published 2026-11-06 for 2 of the 8 stocks yet; the scan is reading them again"]);
  });
});

test("a stock IPOT has not published is read again less often as the wait grows, and files nothing new until it comes", async () => {
  const env = memoryDb();
  const traded = "2026-11-06T16:14:58+07:00";
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: [quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded)] }));
  const recorded = recordedFetch();
  const reads = [];
  const wib = (ms) => new Date(ms + 7 * 3600e3).toISOString().slice(5, 16).replace("T", " ");
  let clock = 0;
  const fetchFn = async (u, init) => {
    if (String(u).includes("code=TLKM&start=11/06/2026&end=11/06/2026")) { reads.push(wib(clock)); return new Response(page("brokersummary_bbca_20261106_insession.html")); }
    return recorded(u, init);
  };
  await withFetch(fetchFn, async () => {
    for (clock = Date.parse("2026-11-06T17:00:00+07:00"); clock <= Date.parse("2026-11-07T20:00:00+07:00"); clock += 60e3) await bandarScanStep(env, { now: new Date(clock) });
  });
  // Every 10 minutes until 21:00 WIB of the session's day, every 30 after, every hour once a day has passed.
  assert.deepEqual(reads.slice(0, 3), ["11-06 17:00", "11-06 17:10", "11-06 17:20"]);
  assert.deepEqual(reads.filter((r) => r >= "11-06 20:40" && r <= "11-06 22:00"), ["11-06 20:40", "11-06 20:50", "11-06 21:00", "11-06 21:30", "11-06 22:00"]);
  assert.deepEqual(reads.filter((r) => r >= "11-07 16:00"), ["11-07 16:00", "11-07 16:30", "11-07 17:00", "11-07 18:00", "11-07 19:00", "11-07 20:00"]);
  assert.equal((await scanStatus(env)).last.done_at, "2026-11-06T10:00:00.000Z", "a read that finds the stock still unpublished files nothing");
  const out = await bandarScreen(env, null, null, "1d", null, null, null, null, new Date(clock));
  assert.equal(out.fetched_at, "2026-11-06T10:00:00Z");
  assert.ok(out.warnings.includes("IPOT has not published 2026-11-06 for 1 of the 2 stocks yet; the scan reads it again at 21:00 WIB"), out.warnings.join("\n"));
});

test("a stock IPOT fails for hours is read once IPOT is back", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8)] }));
  const recorded = recordedFetch();
  let clock = 0;
  // IPOT fails TLKM from 10:00 to 13:00 WIB.
  const fetchFn = async (u, init) => (String(u).includes("code=TLKM") && clock < Date.parse("2026-10-17T13:00:00+07:00") ? new Response("busy", { status: 503 }) : recorded(u, init));
  await withFetch(fetchFn, async () => {
    for (clock = Date.parse("2026-10-17T10:00:00+07:00"); clock <= Date.parse("2026-10-17T15:00:00+07:00"); clock += 60e3) await bandarScanStep(env, { now: new Date(clock) });
  });
  const st = await scanStatus(env);
  assert.deepEqual([st.phase, st.failed, st.retry, st.complete, st.last.stored], ["done", [], [], "2026-10-16", 2]);
});

test("a stock that fails is tried again 10, 30, 60 and 120 minutes after its pass, then every 4 hours, then counts as failed", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8)] }));
  const recorded = recordedFetch();
  const tlkm = [];
  const wib = (ms) => new Date(ms + 7 * 3600e3).toISOString().slice(5, 16).replace("T", " ");
  let clock = 0;
  // IPOT is busy for TLKM (503 is not retried within a request).
  const fetchFn = async (u, init) => (String(u).includes("code=TLKM") ? (tlkm.push(wib(clock)), new Response("busy", { status: 503 })) : recorded(u, init));
  const at = (hm) => new Date(`2026-10-17T${hm}:00+07:00`);
  await withFetch(fetchFn, async () => {
    clock = at("10:00").getTime();
    const r = await bandarScanStep(env, { now: at("10:00") });
    assert.deepEqual([r.read, r.errors, r.done], [2, { http_503: 1 }, false]);
    const st = await scanStatus(env);
    // The first pass is over: the session is filed and shown, TLKM counted as failed for now.
    assert.deepEqual([st.retry, st.retry_due, st.failed, st.complete, st.last.failed, st.last.retrying], [["TLKM"], at("10:10").toISOString(), [], "2026-10-16", ["TLKM"], 1]);
    const out = await bandarScreen(env, null, null, "1w", null, null, null, null, at("10:05"));
    assert.deepEqual(out.warnings, ["1 of 2 scanned stocks had no broker summary; the scan tries it again at 10:10 WIB"]);
    for (clock = at("10:01").getTime(); clock <= Date.parse("2026-10-18T12:00:00+07:00"); clock += 60e3) await bandarScanStep(env, { now: new Date(clock) });
  });
  assert.deepEqual(tlkm, ["10-17 10:00", "10-17 10:10", "10-17 10:40", "10-17 11:40", "10-17 13:40", "10-17 17:40", "10-17 21:40", "10-18 01:40", "10-18 05:40"]);
  const st = await scanStatus(env);
  assert.deepEqual([st.phase, st.failed, st.retry, st.errors, st.last.failed, st.last.retrying], ["done", ["TLKM"], [], { http_503: 1 }, ["TLKM"], 0]);
  assert.deepEqual([st.last_errors[0].symbol, st.last_errors[0].kind, st.last_errors[0].error], ["TLKM", "http_503", "HTTP 503"]);
});

test("/api/bandar/status gives done_at only once the scan is done, and when its record was filed meanwhile", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8)] }));
  const recorded = recordedFetch();
  let busy = true;
  const fetchFn = async (u, init) => (busy && String(u).includes("code=TLKM") ? new Response("busy", { status: 503 }) : recorded(u, init));
  const at = (hm) => new Date(`2026-10-17T${hm}:00+07:00`);
  await withFetch(fetchFn, async () => {
    // The first pass is over and filed, TLKM waits for its retry round.
    await bandarScanStep(env, { now: at("10:00") });
    let st = await scanStatus(env);
    assert.deepEqual([st.phase, st.running, st.done_at, st.filed_at], ["running", true, null, at("10:00").toISOString()]);
    busy = false;
    await bandarScanStep(env, { now: at("10:10") });
    st = await scanStatus(env);
    assert.deepEqual([st.phase, st.running, st.done_at, st.filed_at], ["done", false, at("10:10").toISOString(), at("10:10").toISOString()]);
  });
});

test("the state production held on 2026-11-08 heals: 2026-11-06 is probed again and scanned", async () => {
  const env = memoryDb();
  // Written at 16:45:39 WIB on 2026-11-06, when IPOT had not published that session's lists.
  env.kv.set("bandar_scan", '{"session":"2026-11-06","holiday":true,"done":true,"complete":"2026-10-30","symbols":[],"next":0,"retry":[],"failed":[],"checked":0}');
  const traded = "2026-11-06T16:14:58+07:00";
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-08T07:00:37+07:00"), quotes: [quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded), quote("BRPT", 1210, 1e8, traded), quote("ZZZZ", 50, 1e6, traded)] }));
  const calls = [];
  await withFetch(recordedFetch(calls), async () => {
    const now = new Date("2026-11-08T00:05:00Z"); // Sunday 07:05 WIB
    const first = await bandarScanStep(env, { now });
    assert.deepEqual([first.session, first.probe, first.read], ["2026-11-06", "published", 3]);
    assert.ok(calls[0].includes("code=BBCA&start=11/06/2026&end=11/06/2026"), "the probe reads 11-06 again");
    let st = await scanStatus(env);
    assert.deepEqual([st.phase, st.holiday, st.complete], ["running", false, "2026-10-30"]);
    const meanwhile = await bandarScreen(env, null, null, "1d", null, null, null, null, now);
    assert.equal(meanwhile.data.to, "2026-10-30");
    assert.deepEqual(meanwhile.warnings, ["the scan of 2026-11-06 is running (3 of 4); showing 2026-10-30", "2026-10-30 is 5 sessions behind 2026-11-06"]);
    assert.equal(meanwhile.fetched_at, "2026-10-30T09:45:00Z", "10-30 kept no record: 16:45 WIB that day, never the time of the request");
    assert.equal((await bandarScanStep(env, { now })).done, true);
    st = await scanStatus(env);
    assert.deepEqual([st.complete, st.failed, st.last.session, st.last.promoted], ["2026-11-06", ["ZZZZ"], "2026-11-06", true]);
    const out = await bandarScreen(env, null, null, "1d", null, null, null, null, now);
    assert.deepEqual([out.data.to, out.data.scanned, out.data.checked], ["2026-11-06", 4, 3]);
    assert.deepEqual(out.warnings, ["1 of 4 scanned stocks had no broker summary"]);
    assert.equal(out.fetched_at, "2026-11-08T00:05:00Z");
  });
});

test("a finished session keeps its numbers while the next one waits, and the warnings say why it is shown", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  const recorded = recordedFetch();
  // BBCA's pages of 10-19 and 10-20 show totals without lists: IPOT has not published those sessions.
  const fetchFn = async (u, init) => (/code=BBCA&start=10\/(19|20)\/2026&end=10\/(19|20)\/2026/.test(String(u)) ? new Response(page("brokersummary_bbca_20261106_insession.html")) : recorded(u, init));
  const screen = (now) => bandarScreen(env, null, null, "1w", null, null, null, null, now);
  const waiting = (d, since = "16:46 WIB") => `the scan of ${d} is waiting for IPOT to publish that session's broker lists (checked once, at ${since}); showing 2026-10-16`;
  await withFetch(fetchFn, async () => {
    await scanAll(env, new Date("2026-10-17T03:00:00Z")); // Saturday 10:00 WIB
    // Monday 16:46 WIB: the scan of 10-19 starts and waits for IPOT.
    assert.equal((await bandarScanStep(env, { now: new Date("2026-10-19T09:46:00Z") })).probe, "unpublished");
    let out = await screen(new Date("2026-10-19T09:50:00Z"));
    assert.equal(out.data.to, "2026-10-16");
    assert.deepEqual(out.warnings, [waiting("2026-10-19"), "1 of 4 scanned stocks had no broker summary"]);
    assert.equal(out.fetched_at, "2026-10-17T03:00:00Z", "when the scan of 10-16 ended, not the time of the request");
    assert.deepEqual([out.data.scanned, out.data.checked], [4, 3]);
    // Tuesday 16:30 WIB: 10-20 has closed, its scan starts at 16:45; the wait began the day before.
    out = await screen(new Date("2026-10-20T09:30:00Z"));
    assert.deepEqual(out.warnings, ["the scan of 2026-10-20 starts at 16:45 WIB; showing 2026-10-16", waiting("2026-10-19", "2026-10-19 16:46 WIB"), "2026-10-16 is 2 sessions behind 2026-10-20", "1 of 4 scanned stocks had no broker summary"]);
    // Tuesday 16:46: 10-19 is filed as not published and 10-20 waits in turn.
    await bandarScanStep(env, { now: new Date("2026-10-20T09:46:00Z") });
    out = await screen(new Date("2026-10-20T09:50:00Z"));
    assert.deepEqual(out.warnings, [
      waiting("2026-10-20"),
      "IPOT had not published the broker lists of 2026-10-19 by 2026-10-19 16:46 WIB; showing 2026-10-16",
      "2026-10-16 is 2 sessions behind 2026-10-20",
      "1 of 4 scanned stocks had no broker summary",
    ]);
    const st = await scanStatus(env);
    assert.deepEqual([st.complete, st.last.session, st.last.scanned, st.last.reason, st.last.probes], ["2026-10-16", "2026-10-19", false, "unpublished", 1]);
    const broker = await bandarBroker(env, "YP", "1w", null, new Date("2026-10-20T09:50:00Z"));
    assert.deepEqual([broker.data.to, broker.data.scanned, broker.data.failed, broker.fetched_at], ["2026-10-16", 4, 1, "2026-10-17T03:00:00Z"]);
    assert.ok(broker.warnings.includes("2026-10-16 is 2 sessions behind 2026-10-20"), broker.warnings);
  });
});

test("a session IPOT never answered for is filed as unreachable, not as unpublished", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8)] }));
  const recorded = recordedFetch();
  let down = false;
  const fetchFn = async (u, init) => (down && String(u).includes("indopremier") ? new Response("bad gateway", { status: 502 }) : recorded(u, init));
  await withFetch(fetchFn, async () => {
    await scanAll(env, new Date("2026-10-17T03:00:00Z"));
    // IPOT answers 502 to every probe of 10-19, from Monday 16:45 WIB to Tuesday's.
    down = true;
    for (let t = Date.parse("2026-10-19T16:45:00+07:00"); t <= Date.parse("2026-10-20T16:45:00+07:00"); t += 60e3) await bandarScanStep(env, { now: new Date(t), stocks: false });
  });
  const st = await scanStatus(env);
  assert.deepEqual([st.session, st.last.session, st.last.scanned, st.last.reason, st.last.probes], ["2026-10-20", "2026-10-19", false, "unreachable", 0]);
  const out = await bandarScreen(env, null, null, "1w", null, null, null, null, new Date("2026-10-20T16:50:00+07:00"));
  assert.ok(out.warnings.includes(`IPOT did not answer the scan's ${st.last.probe_errors} checks of 2026-10-19 (the last failed: HTTP 502); showing 2026-10-16`), out.warnings.join("\n"));
  assert.ok(!out.warnings.some((w) => w.includes("had not published")), out.warnings.join("\n"));
});

test("a scan that read too few of its stocks is filed, and the session before it stays shown", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  const recorded = recordedFetch();
  let busy = false;
  const fetchFn = async (u, init) => (busy && /code=(TLKM|BRPT)/.test(String(u)) ? new Response("busy", { status: 503 }) : recorded(u, init));
  await withFetch(fetchFn, async () => {
    await scanAll(env, new Date("2026-10-17T03:00:00Z"));
    // On Monday IPOT fails TLKM and BRPT through the pass and the retry rounds.
    busy = true;
    const at = (hm) => new Date(`2026-10-19T${hm}:00+07:00`);
    for (const hm of ["16:46", "16:47", "16:57", "17:27"]) await bandarScanStep(env, { now: at(hm) });
    let st = await scanStatus(env);
    assert.deepEqual([st.session, st.phase, st.complete, st.last.stored, st.last.of, st.last.promoted], ["2026-10-19", "running", "2026-10-16", 1, 4, false]);
    let out = await bandarScreen(env, null, null, "1w", null, null, null, null, at("17:30"));
    assert.equal(out.data.to, "2026-10-16");
    assert.deepEqual(out.warnings, ["the scan of 2026-10-19 has read 1 of 4 stocks so far (3 failed, 0 not published by IPOT) and reads 2 of them again later; showing 2026-10-16", "1 of 4 scanned stocks had no broker summary"]);
    // Tuesday 16:45 WIB: 10-19 is filed with what it got and the next session's scan starts.
    await bandarScanStep(env, { now: new Date("2026-10-20T16:45:00+07:00"), stocks: false });
    st = await scanStatus(env);
    const filed = JSON.parse(env.kv.get("bandar_scan")).runs.find((r) => r.session === "2026-10-19");
    assert.deepEqual([st.complete, filed.stored, filed.of, filed.failed.length, filed.retrying, filed.promoted], ["2026-10-16", 1, 4, 3, 0, false]);
    out = await bandarScreen(env, null, null, "1w", null, null, null, null, new Date("2026-10-20T16:50:00+07:00"));
    assert.ok(out.warnings.includes("the scan of 2026-10-19 read only 1 of 4 stocks (3 failed, 0 not published by IPOT); showing 2026-10-16"), out.warnings.join("\n"));
    // Both sessions' rows stay: the shown one is never pruned.
    assert.deepEqual([...new Set([...env.scan.keys()].map((k) => k.split("|")[0]))].sort(), ["2026-10-16", "2026-10-19"]);
  });
});

test("a scan the next session's 16:45 cut off in its first pass counts the stocks it never reached", async () => {
  const env = memoryDb();
  const board = (d) => env.kv.set("board", JSON.stringify({ ts: 0, quotes: [["BBCA", 5175, 9e8], ["TLKM", 2690, 3e8], ["BRPT", 1210, 2e8]].map(([s, p, v]) => quote(s, p, v, `${d}T16:14:58+07:00`)) }));
  await withFetch(recordedFetch(), async () => {
    board("2026-10-16");
    await scanAll(env, new Date("2026-10-17T10:00:00+07:00"));
    // Monday: the probe finds 10-19 published, then a minute with little budget left reads one stock.
    board("2026-10-19");
    await bandarScanStep(env, { now: new Date("2026-10-19T16:46:00+07:00"), stocks: false });
    assert.equal((await bandarScanStep(env, { now: new Date("2026-10-19T16:47:00+07:00"), budget: budget(12) })).read, 1);
    // Tuesday 16:45 WIB files 10-19 with the two stocks its first pass had not got to.
    board("2026-10-20");
    await bandarScanStep(env, { now: new Date("2026-10-20T16:45:00+07:00"), stocks: false });
  });
  const filed = JSON.parse(env.kv.get("bandar_scan")).runs.find((r) => r.session === "2026-10-19");
  assert.deepEqual([filed.stored, filed.of, filed.failed.length, filed.unpublished.length, filed.left, filed.promoted], [1, 3, 0, 0, 2, false]);
  const out = await bandarScreen(env, null, null, "1w", null, null, null, null, new Date("2026-10-20T16:50:00+07:00"));
  assert.equal(out.data.to, "2026-10-16");
  assert.ok(out.warnings.includes("the scan of 2026-10-19 read only 1 of 3 stocks (0 failed, 0 not published by IPOT, 2 not reached before the next session's 16:45 WIB); showing 2026-10-16"), out.warnings.join("\n"));
});

test("a stock IPOT still had not published at the next session's 16:45 counts as unpublished, not failed", async () => {
  const env = memoryDb();
  const traded = "2026-11-06T16:14:58+07:00";
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: [quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded)] }));
  const recorded = recordedFetch();
  const fetchFn = async (u, init) => (String(u).includes("code=TLKM&start=11/06/2026&end=11/06/2026") ? new Response(page("brokersummary_bbca_20261106_insession.html")) : recorded(u, init));
  await withFetch(fetchFn, async () => {
    await bandarScanStep(env, { now: new Date("2026-11-06T10:00:00Z") });
    // Monday 16:45 WIB, the probe alone: 11-06 is filed with TLKM unpublished.
    const monday = new Date("2026-11-09T09:45:00Z");
    await bandarScanStep(env, { now: monday, stocks: false });
    const st = await scanStatus(env);
    assert.deepEqual([st.session, st.complete, st.last.session, st.last.unpublished, st.last.failed, st.last.partial], ["2026-11-09", "2026-11-06", "2026-11-06", ["TLKM"], [], true]);
    const out = await bandarScreen(env, null, null, "1d", null, null, null, null, new Date("2026-11-09T09:50:00Z"));
    assert.deepEqual(out.warnings, ["the scan of 2026-11-09 is running (0 of 2); showing 2026-11-06", "IPOT had not published 2026-11-06 for 1 of the 2 scanned stocks when its scan ended"]);
    assert.equal(out.data.scanned, 2);
    assert.equal(out.fetched_at, "2026-11-06T10:00:00Z", "when the scan stored its last stock, not when it was cut off");
  });
});

test("a forced rescan of the shown session never takes its record or its numbers away", async () => {
  const board = (d, ts = 0) => JSON.stringify({ ts, quotes: [quote("BBCA", 5175, 9e8, `${d}T16:14:58+07:00`), quote("TLKM", 2690, 3e8, `${d}T16:14:58+07:00`)] });
  const sat = new Date("2026-10-17T10:00:00+07:00"), sun = new Date("2026-10-18T10:00:00+07:00");
  const recorded = recordedFetch();
  let mode = "recorded";
  const fetchFn = async (u, init) => {
    if (mode === "down" && String(u).includes("indopremier")) return new Response("bad gateway", { status: 502 });
    if (mode === "empty" && String(u).includes("indopremier")) return new Response(page("brokersummary_bbca_20261017_no_trades.html"));
    if (mode === "tlkm" && String(u).includes("code=TLKM")) return new Response("busy", { status: 503 });
    return recorded(u, init);
  };
  const scanned = async () => { const env = memoryDb(); env.kv.set("board", board("2026-10-16")); mode = "recorded"; await scanAll(env, sat); return env; };
  const screen = (env, now) => bandarScreen(env, null, null, "1w", null, null, null, null, now);
  await withFetch(fetchFn, async () => {
    // IPOT down for the forced probe and until Monday's 16:45 WIB: 10-16 keeps its record.
    let env = await scanned();
    mode = "down";
    assert.equal((await bandarScanStep(env, { now: sun, force: true })).probe, "error");
    const monday = new Date("2026-10-19T16:45:00+07:00");
    await bandarScanStep(env, { now: monday, stocks: false });
    let st = await scanStatus(env);
    const kept = JSON.parse(env.kv.get("bandar_scan")).runs.find((r) => r.session === "2026-10-16");
    assert.deepEqual([st.complete, kept.scanned, kept.stored], ["2026-10-16", true, 2]);
    let out = await screen(env, monday);
    assert.deepEqual([out.data.to, out.data.scanned], ["2026-10-16", 2]);
    assert.equal((await bandarBroker(env, "YP", "1w", null, monday)).data.to, "2026-10-16");
    // A forced probe that finds no trades at IPOT or on a later board does not close a session the scan read.
    env = await scanned();
    env.kv.set("board", board("2026-10-15", unix("2026-10-18T07:00:00+07:00")));
    mode = "empty";
    assert.equal((await bandarScanStep(env, { now: sun, force: true })).probe, "unpublished");
    st = await scanStatus(env);
    assert.deepEqual([st.phase, st.holiday, st.complete], ["waiting", false, "2026-10-16"]);
    assert.deepEqual((await screen(env, sun)).data.scanned, 2);
    // A rescan where TLKM fails this time: its row from the first scan stays, and it counts once.
    env = await scanned();
    mode = "tlkm";
    await bandarScanStep(env, { now: sun, force: true });
    out = await screen(env, sun);
    assert.deepEqual([out.data.scanned, out.data.checked], [2, 2]);
    assert.ok(!(out.warnings ?? []).some((w) => w.includes("no broker summary")), out.warnings);
  });
});

test("a forced rescan that has stored fewer stocks than the first scan writes nothing while its stocks wait", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8)] }));
  const recorded = recordedFetch();
  let busy = false;
  const fetchFn = async (u, init) => (busy && String(u).includes("code=TLKM") ? new Response("busy", { status: 503 }) : recorded(u, init));
  const sun = Date.parse("2026-10-18T10:00:00+07:00");
  await withFetch(fetchFn, async () => {
    await scanAll(env, new Date("2026-10-17T10:00:00+07:00"));
    // A maintainer forces a rescan on Sunday and IPOT fails TLKM this time: its first pass stores one stock of two.
    busy = true;
    await bandarScanStep(env, { now: new Date(sun), force: true });
    const written = env.kv.get("bandar_scan");
    // Until TLKM's retry round at 10:10 WIB a minute has nothing to read or file.
    for (let m = 1; m < 10; m++) assert.equal((await bandarScanStep(env, { now: new Date(sun + m * 60e3) })).idle, true, `10:0${m} WIB`);
    assert.equal(env.kv.get("bandar_scan"), written, "the state stays as the forced step wrote it");
    // The round reads TLKM, which fails again: that minute saves.
    assert.equal((await bandarScanStep(env, { now: new Date(sun + 10 * 60e3) })).read, 1);
    assert.notEqual(env.kv.get("bandar_scan"), written);
  });
  const st = await scanStatus(env);
  assert.deepEqual([st.complete, st.last.session, st.last.stored], ["2026-10-16", "2026-10-16", 2], "the first scan's record stands");
});

test("a forced rescan reads the session's own stocks again, however the board has moved since", async () => {
  const env = memoryDb();
  const fri = "2026-11-06T16:14:58+07:00", mon = "2026-11-09T09:59:00+07:00";
  const stocks = ["AAAA", "BBBB", "CCCC", "DDDD"];
  const friday = stocks.map((s, i) => quote(s, 1000, 1e8 - i, fri));
  env.kv.set("board", JSON.stringify({ ts: unix("2026-11-06T16:45:00+07:00"), quotes: friday }));
  const screen = (now) => bandarScreen(env, null, null, "1d", null, null, null, null, now);
  const now = new Date("2026-11-09T10:00:00+07:00");
  await withFetch(lateFetch(new Set()), async () => {
    await scanAll(env, new Date("2026-11-06T17:00:00+07:00"));
    // Monday 10:00 WIB: two other stocks trade heavily this morning, and a maintainer forces a rescan of 11-06.
    env.kv.set("board", JSON.stringify({ ts: unix("2026-11-09T10:00:00+07:00"), quotes: [...friday, quote("EEEE", 1000, 5e8, mon), quote("FFFF", 1000, 4e8, mon)] }));
    await bandarScanStep(env, { now, force: true });
    const st = JSON.parse(env.kv.get("bandar_scan"));
    assert.deepEqual([st.symbols, st.ranked_from, Object.keys(st.quotes)], [stocks, "2026-11-06", stocks]);
    // Three read again so far; the screen counts each stock once and says where the others come from.
    const out = await screen(now);
    assert.equal(out.data.scanned, 4);
    assert.deepEqual(out.warnings, ["the scan of 2026-11-06 is reading its stocks again: 3 of 4 so far; the others are from its earlier scan"]);
    await scanAll(env, now);
  });
  let out = await screen(now);
  assert.deepEqual([out.data.scanned, out.data.checked, out.warnings], [4, 4, undefined]);
  assert.deepEqual([...new Set([...env.scan.keys()].map((k) => k.split("|")[2]))].sort(), stocks);
  // A row of a stock outside the scan's own (a rescan before this one ranked the board again) is not one of its stocks.
  env.scan.set("2026-11-06|1d|EEEE", env.scan.get("2026-11-06|1d|AAAA"));
  out = await screen(now);
  assert.equal(out.data.scanned, 4);
});

test("a cron minute that runs late does not take the scan back to the session before", async () => {
  const env = memoryDb();
  const board = (d) => env.kv.set("board", JSON.stringify({ ts: unix(`${d}T16:45:00+07:00`), quotes: [quote("BBCA", 5100, 2e8, `${d}T16:14:58+07:00`), quote("TLKM", 2690, 1e8, `${d}T16:14:58+07:00`)] }));
  const recorded = recordedFetch();
  // At 16:45 WIB on 11-09 IPOT has not published that session.
  const fetchFn = async (u, init) => (String(u).includes("start=11/09/2026&end=11/09/2026") ? new Response(page("brokersummary_bbca_20261106_insession.html")) : recorded(u, init));
  const at = (iso) => new Date(`${iso}+07:00`);
  await withFetch(fetchFn, async () => {
    board("2026-11-06");
    await scanAll(env, at("2026-11-07T10:00:00"));
    board("2026-11-09");
    assert.equal((await bandarScanStep(env, { now: at("2026-11-09T16:45:00"), stocks: false })).probe, "unpublished");
    const written = env.kv.get("bandar_scan");
    // The 16:44 minute, dispatched late, runs after it with its own scheduled time.
    assert.deepEqual(await bandarScanStep(env, { now: at("2026-11-09T16:44:00") }), { idle: true, session: "2026-11-09" });
    assert.equal(env.kv.get("bandar_scan"), written, "the state stays as the 16:45 minute wrote it");
    const st = await scanStatus(env);
    assert.deepEqual([st.session, st.phase, st.complete, st.last.session, st.last.stored], ["2026-11-09", "waiting", "2026-11-06", "2026-11-06", 2]);
  });
});

test("a minute that overlapped another does not write its state over the other's", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8), quote("ZZZZ", 50, 1e6)] }));
  const now = new Date("2026-10-17T03:00:00Z");
  const recorded = recordedFetch();
  await withFetch(recorded, async () => { await bandarScanStep(env, { now }); });
  const read = JSON.parse(env.kv.get("bandar_scan"));
  assert.equal(read.rev, 1);
  // While this minute reads ZZZZ, a slower one that had read the same state writes first.
  let theirs = null;
  const overlap = async (u, init) => {
    if (!theirs) env.kv.set("bandar_scan", (theirs = JSON.stringify({ ...read, rev: 2, written_by: "the other minute" })));
    return recorded(u, init);
  };
  const { warn } = console;
  const warned = [];
  console.warn = (...a) => warned.push(a.join(" "));
  try {
    await withFetch(overlap, async () => {
      const r = await bandarScanStep(env, { now });
      assert.equal(r.conflict, true);
    });
  } finally {
    console.warn = warn;
  }
  assert.ok(warned.some((w) => w.includes("another minute wrote the state first")), warned);
  assert.equal(env.kv.get("bandar_scan"), theirs, "the other minute's state stands");
  // The next minute goes on from it.
  await withFetch(recorded, async () => { assert.equal((await bandarScanStep(env, { now })).done, true); });
  assert.equal(JSON.parse(env.kv.get("bandar_scan")).rev, 3);
});

test("on SQLite the heal writes over the production row, a stale revision writes nothing and old sessions are pruned", { skip: noSqlite }, async () => {
  const s = sqliteDb();
  const env = { DB: s.DB };
  s.put("bandar_scan", '{"session":"2026-11-06","holiday":true,"done":true,"complete":"2026-10-30","symbols":[],"next":0,"retry":[],"failed":[],"checked":0}');
  const traded = "2026-11-06T16:14:58+07:00";
  s.put("board", { ts: unix("2026-11-08T07:00:37+07:00"), quotes: [quote("BBCA", 5100, 2e8, traded), quote("TLKM", 2690, 1e8, traded), quote("BRPT", 1210, 1e8, traded), quote("ZZZZ", 50, 1e6, traded)] });
  // The sessions production holds rows of.
  for (const d of ["2026-10-23", "2026-10-30"]) s.row(d, "BBRI");
  const now = new Date("2026-11-08T00:05:00Z");
  const recorded = recordedFetch();
  await withFetch(recorded, async () => { assert.equal((await bandarScanStep(env, { now })).read, 3); });
  const read = JSON.parse(s.get("bandar_scan"));
  assert.deepEqual([read.rev, read.phase, read.complete], [1, "running", "2026-10-30"], "the row without a revision is written over");
  // While this minute reads ZZZZ, a slower one that read the same revision writes first.
  const theirs = JSON.stringify({ ...read, rev: 2, written_by: "the other minute" });
  let wrote = false;
  const overlap = async (u, init) => { if (!wrote) { s.put("bandar_scan", theirs); wrote = true; } return recorded(u, init); };
  const { warn } = console;
  console.warn = () => {};
  try {
    await withFetch(overlap, async () => { assert.equal((await bandarScanStep(env, { now })).conflict, true); });
  } finally {
    console.warn = warn;
  }
  assert.equal(s.get("bandar_scan"), theirs, "the stale revision changed nothing");
  await withFetch(recorded, async () => { assert.equal((await bandarScanStep(env, { now })).done, true); });
  const done = JSON.parse(s.get("bandar_scan"));
  assert.deepEqual([done.rev, done.phase, done.complete], [3, "done", "2026-11-06"]);
  assert.deepEqual(s.sessions(), ["2026-10-30", "2026-11-06"], "the newest two sessions stay");
});

test("on SQLite the prune keeps the session shown however many came after it", { skip: noSqlite }, async () => {
  const s = sqliteDb();
  const env = { DB: s.DB };
  const board = (d) => s.put("board", { ts: 0, quotes: [["BBCA", 5175, 9e8], ["TLKM", 2690, 3e8], ["BRPT", 1210, 2e8], ["ZZZZ", 50, 1e6]].map(([sym, p, v]) => quote(sym, p, v, `${d}T16:14:58+07:00`)) });
  const recorded = recordedFetch();
  let busy = false;
  const fetchFn = async (u, init) => (busy && /code=(TLKM|BRPT)/.test(String(u)) ? new Response("busy", { status: 503 }) : recorded(u, init));
  await withFetch(fetchFn, async () => {
    board("2026-10-16");
    await scanAll(env, new Date("2026-10-17T03:00:00Z"));
    // From Monday IPOT fails TLKM and BRPT: each session stores BBCA alone and is not shown.
    busy = true;
    for (const d of ["2026-10-19", "2026-10-20", "2026-10-21"]) {
      board(d);
      for (const hm of ["16:46", "16:47", "16:57", "17:27"]) await bandarScanStep(env, { now: new Date(`${d}T${hm}:00+07:00`) });
    }
  });
  assert.equal(JSON.parse(s.get("bandar_scan")).complete, "2026-10-16");
  assert.deepEqual(s.sessions(), ["2026-10-16", "2026-10-20", "2026-10-21"]);
});

test("on SQLite a minute that loses the compare-and-set prunes nothing", { skip: noSqlite }, async () => {
  const s = sqliteDb();
  const env = { DB: s.DB };
  const board = (d) => s.put("board", { ts: unix(`${d}T16:45:00+07:00`), quotes: ["BBCA", "TLKM", "BRPT", "BMRI"].map((sym, i) => quote(sym, 1000, 1e8 - i, `${d}T16:14:58+07:00`)) });
  const PUBLISHED = page("brokersummary_bbca_20261105.html");
  let failAllBut = null, overlap = null;
  const fetchFn = async (u) => {
    const url = String(u);
    if (overlap) { const run = overlap; overlap = null; await run(); }
    if (url.includes("finance/chart")) return new Response(CHART);
    if (failAllBut && !url.includes(`code=${failAllBut}`)) return new Response("busy", { status: 503 });
    return new Response(PUBLISHED);
  };
  const wib = (iso) => new Date(`${iso}+07:00`);
  const { warn } = console;
  console.warn = () => {};
  try {
    await withFetch(fetchFn, async () => {
      // 10-30 is scanned whole and shown.
      board("2026-10-30");
      await scanAll(env, wib("2026-10-31T10:00:00"));
      // 11-06 stores one stock of four: filed, not shown.
      board("2026-11-06");
      failAllBut = "BBCA";
      for (const hm of ["17:00", "17:01", "17:11", "17:41", "17:42"]) await bandarScanStep(env, { now: wib(`2026-11-06T${hm}:00`) });
      failAllBut = null;
      // 11-09: published, two stocks read and two left.
      board("2026-11-09");
      await bandarScanStep(env, { now: wib("2026-11-09T16:45:00"), stocks: false });
      await bandarScanStep(env, { now: wib("2026-11-09T16:46:00"), budget: budget(22) });
      // The 16:47 minute reads the last two stocks, which shows 11-09 and prunes 10-30; meanwhile the
      // 16:48 minute, from the same revision, reads one stock and writes first.
      let w = null;
      overlap = async () => { w = await bandarScanStep(env, { now: wib("2026-11-09T16:48:00"), budget: budget(12) }); };
      const l = await bandarScanStep(env, { now: wib("2026-11-09T16:47:00") });
      assert.deepEqual([w.read, w.conflict, l.read, l.conflict], [1, undefined, 2, true]);
    });
  } finally {
    console.warn = warn;
  }
  assert.equal(JSON.parse(s.get("bandar_scan")).complete, "2026-10-30", "the state written is the 16:48 minute's");
  assert.ok(s.sessions().includes("2026-10-30"), `the session it shows keeps its rows: ${s.sessions()}`);
});

/** IPOT answering four 404s before every page (five requests a page); the rest as recorded. */
function stormFetch(calls) {
  const recorded = recordedFetch();
  const tries = new Map();
  return async (u, init) => {
    const url = String(u);
    calls.push(url);
    if (url.includes("indopremier")) {
      tries.set(url, (tries.get(url) || 0) + 1);
      if (tries.get(url) % 5) return new Response("<html>404 Not Found</html>", { status: 404 });
    }
    return recorded(u, init);
  };
}

test("a 404 storm stays inside the budget and the stock it cut short stays next", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8)] }));
  const now = new Date("2026-10-17T03:00:00Z");
  const calls = [];
  await withFetch(stormFetch(calls), async () => {
    // The probe takes 5 requests and BBCA would take 41 (a chart and eight pages): the budget stops it at 44.
    await bandarScanStep(env, { now, budget: budget(44), pauseMs: 0 });
    assert.equal(calls.length, 44);
    assert.equal(env.scan.size, 0, "nothing stored from a stock cut short");
    const st = await scanStatus(env);
    assert.deepEqual([st.scanned, st.failed], [0, []], "BBCA is neither scanned nor failed");
  });
  // The next minute, with a budget of its own, reads it whole; then too little is left to start TLKM.
  const next = [];
  await withFetch(stormFetch(next), async () => {
    await bandarScanStep(env, { now, budget: budget(44), pauseMs: 0 });
    assert.equal(next.length, 41);
    const st = await scanStatus(env);
    assert.deepEqual([st.scanned, st.failed], [1, []]);
    assert.equal(env.scan.size, 4, "BBCA's four periods");
  });
});

test("a slow IPOT does not run a minute's scan into the next minute", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8), quote("BRPT", 1210, 2e8)] }));
  const now = new Date("2026-10-17T03:00:00Z");
  const recorded = recordedFetch();
  await withFetch(recorded, async () => { assert.equal((await bandarScanStep(env, { now, stocks: false })).probe, "published"); });
  // IPOT answers each page in 4 s, Yahoo in 0.3 s; `ms` is the minute's clock.
  let ms = 0;
  const sent = [];
  const slow = async (u, init) => { sent.push(ms); ms += String(u).includes("indopremier") ? 4000 : 300; return recorded(u, init); };
  await withFetch(slow, async () => {
    // A stock takes 32.3 s: a second one would end past the minute.
    const r = await bandarScanStep(env, { now, budget: budget(44, { clock: () => ms }) });
    assert.equal(r.read, 1);
    // A minute whose other jobs took 30 s: its stock is cut short at 45 s and stays first in line.
    sent.length = 0;
    ms = 0;
    const b = budget(44, { clock: () => ms });
    ms = 30_000;
    const cut = await bandarScanStep(env, { now, budget: b });
    assert.equal(cut.read, 0);
    assert.ok(Math.max(...sent) < 45_000, `no request from 45 s on: ${sent}`);
    assert.equal((await scanStatus(env)).scanned, 1);
  });
});

test("a stock the budget cuts short twice counts as failed instead of starting every minute", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8)] }));
  const now = new Date("2026-10-17T03:00:00Z");
  // Minutes with 20 subrequests left, against a stock that needs 41 in this storm.
  await withFetch(stormFetch([]), async () => { await bandarScanStep(env, { now, budget: budget(20), pauseMs: 0 }); });
  let st = await scanStatus(env);
  assert.deepEqual([st.retry, st.failed, st.scanned], [[], [], 0], "cut once: still first in line");
  await withFetch(stormFetch([]), async () => {
    const r = await bandarScanStep(env, { now, budget: budget(20), pauseMs: 0 });
    assert.deepEqual(r.errors, { budget: 1 });
  });
  st = await scanStatus(env);
  assert.deepEqual([st.retry, st.last_errors[0].kind, st.scanned], [["BBCA"], "budget", 1], "cut twice: off to the retry pass like any failure");
});

test("a stock that fails some other way after a budget cut gets its one cut back", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8), quote("TLKM", 2690, 3e8)] }));
  const now = new Date("2026-10-17T03:00:00Z");
  await withFetch(recordedFetch(), async () => { await bandarScanStep(env, { now, stocks: false }); });
  // A minute with 20 subrequests left cuts BBCA short in a 404 storm: it stays first in line.
  await withFetch(stormFetch([]), async () => { await bandarScanStep(env, { now, budget: budget(20), pauseMs: 0 }); });
  // The next minute IPOT answers 503 for it: off to the retry round.
  const recorded = recordedFetch();
  await withFetch(async (u, init) => (String(u).includes("code=BBCA") ? new Response("busy", { status: 503 }) : recorded(u, init)), async () => { await bandarScanStep(env, { now }); });
  let st = await scanStatus(env);
  assert.deepEqual([st.retry, st.last_errors[0].kind], [["BBCA"], "http_503"]);
  // In that round the storm cuts it short again: a first cut, so it stays first in line instead of failing.
  await withFetch(stormFetch([]), async () => {
    const r = await bandarScanStep(env, { now: new Date(st.retry_due), budget: budget(20), pauseMs: 0 });
    assert.deepEqual([r.read, r.errors], [0, undefined], JSON.stringify(r));
  });
  st = await scanStatus(env);
  assert.deepEqual([st.retry, st.pass, st.last_errors[0].kind], [["BBCA"], 1, "http_503"]);
});

/** Yahoo's session and quotes, the KSEI mirror's empty index, then IPOT and the charts as recorded. */
function cronFetch(urls) {
  const recorded = recordedFetch();
  const time = Math.floor(Date.parse("2026-10-16T16:14:58+07:00") / 1000);
  const board = { "BBCA.JK": [5175, 9e8], "TLKM.JK": [2690, 3e8], "BRPT.JK": [1210, 2e8] };
  return async (u, init) => {
    const url = String(u);
    urls.push(url);
    if (url.startsWith("https://fc.yahoo.com")) return new Response("", { status: 404, headers: { "set-cookie": "A3=d=test; Path=/" } });
    if (url.includes("/getcrumb")) return new Response("crumb");
    if (url.includes("/v7/finance/quote")) {
      const asked = new URL(url).searchParams.get("symbols").split(",");
      const result = asked.filter((s) => board[s] || s === "^JKSE").map((s) => (s === "^JKSE"
        ? { symbol: s, regularMarketPrice: 7000, regularMarketTime: time }
        : { symbol: s, regularMarketPrice: board[s][0], regularMarketVolume: board[s][1], regularMarketPreviousClose: 6075, regularMarketTime: time }));
      return new Response(JSON.stringify({ quoteResponse: { result } }));
    }
    if (url.endsWith("/index.json")) return new Response(JSON.stringify({ schema: "idx.ownership.mirror.v1", releases: [] }));
    return recorded(u, init);
  };
}

test("the minute cron refreshes the board and the register first, then scans, inside one budget", async () => {
  const env = memoryDb();
  const urls = [];
  const ctx = { waitUntil() { throw new Error("the cron leaves nothing to waitUntil"); } };
  const minute = (iso) => ({ scheduledTime: Date.parse(iso), cron: "* * * * *" });
  const { log, error } = console;
  const logs = [];
  console.log = console.error = (...a) => logs.push(a.join(" "));
  try {
    await withFetch(cronFetch(urls), async () => {
      // 16:45 WIB: the 15-minute board refresh after the close and the register's check, then the scan's probe alone.
      const first = await worker.scheduled(minute("2026-10-16T09:45:00Z"), env, ctx);
      const scanFrom = urls.findIndex((u) => u.includes("indopremier"));
      assert.equal(urls.filter((u) => u.includes("/v7/finance/quote")).length, 7, "the board in six batches and the index");
      assert.ok(urls.slice(0, scanFrom).some((u) => u.endsWith("/ownership-data/index.json")), "the register's check first");
      assert.equal(urls.length - scanFrom, 1, "the probe, and no stock in the refresh's minute");
      assert.ok(first.subrequests.fetch + first.subrequests.d1 <= 44, JSON.stringify(first.subrequests));
      assert.equal(first.subrequests.fetch, 12, "2 for the session, 7 quote calls, 2 for the mirror's redirect, the probe");
      // The step ran before scheduled() resolved: its state is there.
      assert.equal((await scanStatus(env)).session, "2026-10-16");
      // 16:46: no refresh, the register idle: three stocks.
      urls.length = 0;
      const second = await worker.scheduled(minute("2026-10-16T09:46:00Z"), env, ctx);
      assert.equal(urls.filter((u) => u.includes("indopremier")).length, 24, "eight pages a stock");
      assert.equal(urls.length, 27, "and a chart each");
      assert.ok(second.subrequests.fetch + second.subrequests.d1 <= 44, JSON.stringify(second.subrequests));
      assert.equal((await scanStatus(env)).complete, "2026-10-16");
    });
  } finally {
    console.log = log;
    console.error = error;
  }
  assert.ok(logs.some((l) => l.startsWith("bandar ") && l.includes('"subrequests"')), logs.join("\n"));
  assert.ok(!logs.some((l) => l.includes("failed")), logs.join("\n"));
});

test("/api/bandar for one investor side reads against the value traded", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8)] }));
  await withFetch(recordedFetch(), async () => {
    const out = await bandar(env, "BBCA", null, "2026-10-16", null, null, null, "foreign", "all", "1");
    const r = out.data[0];
    assert.equal(r.investor, "foreign");
    // Like the Rust test: the foreign page's 1.7 T is their buying; percentages are of the 6.6 T traded.
    assert.equal(r.total_value, 6_600_000_000_000);
    const head = r.tiers.find((t) => t.top === 5);
    assert.ok(Math.abs(head.net_pct - (head.net_value / 6.6e12) * 100) < 1e-9);
    assert.ok(Math.abs(r.foreign_net_pct - (-2.3 / 6.6) * 100) < 1e-9);
    assert.equal(r.daily.days[0].total_value, 6_600_000_000_000);
    await assert.rejects(bandar(env, "BBCA", "1m", null, null, null, null, "foreign", "all", "1"), (e) => e.code === "usage" && /one investor side/.test(e.message));
  });
});

test("/api/bandar reads a range with its daily series", async () => {
  const env = memoryDb();
  env.kv.set("board", JSON.stringify({ ts: 0, quotes: [quote("BBCA", 5175, 9e8)] }));
  const calls = [];
  await withFetch(recordedFetch(calls), async () => {
    const out = await bandar(env, "BBCA", null, null, "2026-10-12", "2026-10-16", null, "all", "all", "1");
    assert.equal(out.schema, "idx.bandar.v1");
    const r = out.data[0];
    assert.equal(r.period, "custom");
    assert.equal(r.name, "BANK CENTRAL ASIA Tbk");
    assert.equal(r.signal, "big_distribution");
    assert.equal(r.phase, "markdown");
    assert.equal(r.buyers[0].name, "CLSA SEKURITAS INDONESIA");
    assert.equal(r.reasons[0], "Top 5 buyers bought 4.23T, top 5 sellers sold 6.16T: net -1.93T, -19.5% of the 9.90T traded (big distribution, at -15% or less).");
    assert.equal(r.daily.days.length, 5);
    assert.ok(r.daily.streak < 0);
    assert.equal(calls.filter((u) => u.includes("indopremier")).length, 6, "the range and five sessions");
    for (const bad of [
      ["BBCA,BBRI,BMRI,BBNI,TLKM,ASII"],
      ["BBCA", "2w"],
      ["BBCA", null, null, null, null, "4"],
      ["BBCA,BBRI", null, null, null, null, null, "all", "all", "1"],
      ["BBCA", "3m", null, null, null, null, "all", "all", "1"],
      ["BBCA", null, "2026-10-16", "2026-10-01"],
      ["BBCA", null, null, null, null, null, "retail"],
    ]) await assert.rejects(bandar(env, ...bad), (e) => e.code === "usage", JSON.stringify(bad));
  });
});
