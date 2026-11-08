// node --test web/test/*.mjs (no network). /api/market/brokers: the web twin
// of `idx -o json market brokers --source ipot`. IPOT's ranking and the page
// that dates it are read as one pair and kept in D1 with the CLI's lifetimes;
// the dating rules are ipot::top_brokers' and the ranking service's, tested
// with the Rust tests' cases on the same recorded pages
// (src/providers/ipot/mod.rs and src/service/exchange.rs).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as ipot from "../src/ipot.js";
import { topBrokers, marketBrokers, route } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", "ipot", p), "utf8");
const TOP = fixture("topbroker_20261106.html");
const STATISTIC = fixture("statistic_bbca_20261106.html");
/** BBCA's statistics page with its Today row on `day`. */
const named = (day) => STATISTIC.replace("2026-11-06", day).replace("2026-11-06", day);

/** A WIB wall-clock time as a Date. */
const at = (s) => new Date(Date.parse(`${s}+07:00`));
const SUNDAY = at("2026-11-08T10:00:00");
const PAIR_KEY = "ipot:ma-topbroker";
const ACCEPTED_KEY = "ipot:ma-topbroker:accepted";

/** In-memory D1 for the cache and kv statements the ranking runs. */
function memoryDb() {
  const kv = new Map(), cache = new Map();
  const run = (sql, a) => {
    if (sql.startsWith("SELECT v, updated_at FROM kv")) return kv.has(a[0]) ? { v: kv.get(a[0]), updated_at: 0 } : null;
    if (sql.startsWith("INSERT OR REPLACE INTO kv")) return kv.set(a[0], a[1]);
    if (sql.startsWith("SELECT v, fetched_at, expires_at FROM cache")) return cache.get(a[0]) || null;
    if (sql.startsWith("INSERT OR REPLACE INTO cache")) return cache.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] });
    throw new Error("memoryDb: unexpected SQL " + sql);
  };
  const stmt = (sql, a = []) => ({ bind: (...b) => stmt(sql, b), first: async () => run(sql, a), run: async () => (run(sql, a), {}) });
  return { DB: { prepare: (sql) => stmt(sql) }, kv, cache };
}
const accepted = (env) => (env.kv.has(ACCEPTED_KEY) ? JSON.parse(env.kv.get(ACCEPTED_KEY)) : null);

/** Copies of both pages, read together at `read` (a WIB time), in the D1 cache. */
function seed(env, read, top = TOP, statistic = STATISTIC, datedRead = read) {
  const pair = ipot.rankingPair(top, statistic, at(read).getTime(), at(datedRead).getTime());
  env.cache.set(PAIR_KEY, { v: JSON.stringify(pair), expires_at: 0, fetched_at: Math.floor(at(read).getTime() / 1000) });
}

/**
 * IPOT's two pages from `pages` (ranking, statistic: a body, or a status
 * number), a page whose URL contains `slow` answering only after `pause`
 * ms; anything else fails like the network. Counts the requests.
 */
function ipotFetch({ top = TOP, statistic = STATISTIC, slow = null, pause = 0 } = {}) {
  const calls = [];
  const fn = async (u) => {
    const url = String(u);
    calls.push(url);
    if (slow && url.includes(slow)) await new Promise((r) => setTimeout(r, pause));
    const body = url.includes("ma-topbroker.php") ? top : url.includes("statistic.php?code=BBCA") ? statistic : null;
    if (body == null) throw new TypeError(`no route for ${url}`);
    return typeof body === "number" ? new Response("busy", { status: body }) : new Response(body, { status: 200 });
  };
  return { fn, calls };
}

async function withFetch(fn, body) {
  const realFetch = globalThis.fetch;
  globalThis.fetch = fn;
  try {
    return await body();
  } finally {
    globalThis.fetch = realFetch;
  }
}

/** The ApiError a call throws. */
async function failure(promise) {
  try { await promise; } catch (e) { return e; }
  assert.fail("expected an error");
}

test("the ranking comes with the session it is for, and a second ask comes from D1", async () => {
  const env = memoryDb();
  const f = ipotFetch();
  await withFetch(f.fn, async () => {
    const s = await topBrokers(env, SUNDAY);
    assert.equal(s.date, "2026-11-06");
    assert.equal(s.brokers.length, 90, "JB is counted in CC");
    assert.equal(s.rounded, true);
    assert.deepEqual(s.warnings, ["JB ([CC] BJB Sekuritas Jawa Barat) is counted in CC, as idx.co.id counts it; IPOT lists it apart"]);
    assert.equal(s.cached, false);
    assert.equal(f.calls.length, 2);
    const again = await topBrokers(env, SUNDAY);
    assert.equal(again.cached, true);
    assert.equal(f.calls.length, 2, "both pages from D1");
  });
  assert.equal(accepted(env).date, "2026-11-06");
});

test("read in a trading day's hours, the two pages can be on different sessions: no date", async () => {
  // With a pair read the night before cached, IPOT is asked, and the page read now cannot be dated.
  for (const during of ["2026-11-06T08:58:00", "2026-11-06T10:00:00", "2026-11-06T16:15:00", "2026-11-06T18:59:59"]) {
    const env = memoryDb();
    seed(env, "2026-11-05T19:30:00");
    const f = ipotFetch();
    const err = await withFetch(f.fn, () => failure(topBrokers(env, at(during))));
    assert.equal(f.calls.length, 2, during);
    assert.equal(err.code, "provider", during);
    assert.equal(err.exit, 3);
    assert.match(err.message, /so it cannot be dated$/, during);
  }
  const env = memoryDb();
  seed(env, "2026-11-05T19:30:00");
  const err = await withFetch(ipotFetch().fn, () => failure(topBrokers(env, at("2026-11-06T10:00:00"))));
  assert.equal(err.message, "ipot: the broker ranking was read at 10:00 WIB on 2026-11-06, in a trading day's hours (08:58 to 19:00 WIB), when it and the page that dates it can be on different sessions (when they roll over has not been measured), so it cannot be dated");
  // The pair read then is kept, read in those hours: the next ask does not ask IPOT.
  const f = ipotFetch();
  const again = await withFetch(f.fn, () => failure(topBrokers(env, at("2026-11-06T10:01:00"))));
  assert.equal(f.calls.length, 0);
  assert.match(again.message, /no copy read outside those hours is cached, so IPOT was not asked$/);

  // After the busy hours, and on a weekday holiday (cuti bersama, the day after the session of 2026-12-23): dated.
  for (const [after, statistic] of [["2026-11-06T19:00:00", STATISTIC], ["2026-12-24T10:00:00", named("2026-12-23")]]) {
    const s = await withFetch(ipotFetch({ statistic }).fn, () => topBrokers(memoryDb(), at(after)));
    assert.equal(s.warnings.length, 1, `${after}: only the fold`);
  }
  // Dated a session that has not opened, the trades are another's.
  for (const early of ["2026-11-06T08:57:59", "2026-11-05T20:00:00"]) {
    const err = await withFetch(ipotFetch().fn, () => failure(topBrokers(memoryDb(), at(early))));
    assert.equal(err.code, "provider", early);
    assert.match(err.message, /disagree/, early);
  }
});

test("a page the call fetches is judged by when it came", async () => {
  // Monday, half a second before the session's first trades at 08:58.
  const call = new Date(at("2026-11-09T08:57:59").getTime() + 500);
  for (const [slow, says] of [
    ["ma-topbroker.php", "the broker ranking was read at 08:58 WIB on 2026-11-09, in a trading day's hours"],
    ["statistic.php", "the page that dates the broker ranking was read at 08:58 WIB on 2026-11-09, in a trading day's hours"],
  ]) {
    const err = await withFetch(ipotFetch({ slow, pause: 600 }).fn, () => failure(topBrokers(memoryDb(), call)));
    assert.equal(err.code, "provider");
    assert.ok(err.message.includes(says), err.message);
    assert.match(err.message, /cannot be dated$/);
  }
  // Coming at once, they were read before then: Friday's whole session.
  const s = await withFetch(ipotFetch().fn, () => topBrokers(memoryDb(), call));
  assert.equal(s.date, "2026-11-06");
});

test("IPOT is not asked when a page read now cannot be dated", async () => {
  const monday = at("2026-11-09T10:00:00");
  // Nothing cached: a page read in a trading day's hours cannot be dated, so none is asked for.
  const f = ipotFetch();
  const err = await withFetch(f.fn, () => failure(topBrokers(memoryDb(), monday)));
  assert.deepEqual([err.code, err.exit, err.status], ["provider", 3, 502]);
  assert.equal(err.message, "ipot: a broker ranking read at 10:00 WIB on 2026-11-09, in a trading day's hours (08:58 to 19:00 WIB), cannot be dated, as it and the page that dates it can be on different sessions then (when they roll over has not been measured); no copy read outside those hours is cached, so IPOT was not asked");
  assert.equal(f.calls.length, 0);

  // A pair read on Friday night can answer: IPOT is asked, and when it cannot be read, the copy stands in.
  let env = memoryDb();
  seed(env, "2026-11-06T19:30:00");
  const down = ipotFetch({ top: 503, statistic: 503 });
  const s = await withFetch(down.fn, () => topBrokers(env, monday));
  assert.equal(s.date, "2026-11-06");
  assert.deepEqual([s.stale, s.cached], [true, true], "the copy, after the failed read");
  assert.equal(s.warnings[0], "live fetch failed (broker ranking for the market: HTTP 503); showing cached copy");
  assert.equal(down.calls.length, 2);
  // When IPOT answers, the page read now cannot be dated.
  env = memoryDb();
  seed(env, "2026-11-06T19:30:00");
  const up = ipotFetch();
  const refused = await withFetch(up.fn, () => failure(topBrokers(env, monday)));
  assert.ok(refused.message.includes("the broker ranking was read at 10:00 WIB on 2026-11-09"), refused.message);
  assert.equal(up.calls.length, 2);
});

test("the ranking and its date are read together", async () => {
  // A pair whose pages were read an hour apart is read again.
  const env = memoryDb();
  seed(env, "2026-11-08T08:55:00", TOP, STATISTIC, "2026-11-08T07:55:00");
  const f = ipotFetch();
  const s = await withFetch(f.fn, () => topBrokers(env, SUNDAY));
  assert.equal(s.cached, false);
  assert.equal(f.calls.length, 2);
  // When it cannot be, it has no date, and the failed read comes first.
  const apart = memoryDb();
  seed(apart, "2026-11-08T08:55:00", TOP, STATISTIC, "2026-11-08T07:55:00");
  const err = await withFetch(ipotFetch({ top: 503, statistic: 503 }).fn, () => failure(topBrokers(apart, SUNDAY)));
  assert.deepEqual([err.code, err.exit], ["network", 3]);
  assert.equal(err.message, "IPOT could not be read (broker ranking for the market: HTTP 503), and its cached copy cannot be used: the broker ranking and the page that dates it were read at different times, so the ranking has no date");
  // The pair is kept whole: when only the date page cannot be read again, the new ranking is not
  // paired with the old date page (the CLI refuses that pair); the old pair, read together, stands in.
  const old = memoryDb();
  seed(old, "2026-11-08T03:00:00");
  const half = await withFetch(ipotFetch({ statistic: 503 }).fn, () => topBrokers(old, SUNDAY));
  assert.deepEqual([half.date, half.stale], ["2026-11-06", true]);
  assert.equal(half.warnings[0], "live fetch failed (session date for BBCA: HTTP 503); showing cached copy");
});

test("a copy is judged by when its pages were read", async () => {
  // Read on Friday at 10:00, served on Sunday because IPOT is down: still read in trading hours.
  let env = memoryDb();
  seed(env, "2026-11-06T10:00:00");
  let err = await withFetch(ipotFetch({ top: 503, statistic: 503 }).fn, () => failure(topBrokers(env, SUNDAY)));
  assert.deepEqual([err.code, err.exit], ["network", 3]);
  assert.ok(err.message.startsWith("IPOT could not be read (broker ranking for the market: HTTP 503), and its cached copy cannot be used: the broker ranking was read at 10:00 WIB on 2026-11-06, in a trading day's hours"), err.message);
  // Read that evening, the same copies are the whole session.
  env = memoryDb();
  seed(env, "2026-11-06T19:30:00");
  const s = await withFetch(ipotFetch({ top: 503, statistic: 503 }).fn, () => topBrokers(env, SUNDAY));
  assert.deepEqual([s.stale, s.cached, s.date], [true, true, "2026-11-06"]);
  // Read at 08:57, dated that day and showing trades: refused, and still refused when served after the open.
  for (const served of ["2026-11-06T09:00:00", "2026-11-08T10:00:00"]) {
    env = memoryDb();
    seed(env, "2026-11-06T08:57:00");
    err = await withFetch(ipotFetch({ top: 503, statistic: 503 }).fn, () => failure(topBrokers(env, at(served))));
    assert.ok(err.message.includes("read at 08:57 WIB on 2026-11-06 is dated 2026-11-06, a session that had not opened then"), `${served}: ${err.message}`);
  }
  // Read at 08:57 and dated the session before: the whole of it, and three minutes later still fresh.
  env = memoryDb();
  seed(env, "2026-11-06T08:57:00", TOP, named("2026-11-05"));
  const f = ipotFetch();
  const thursday = await withFetch(f.fn, () => topBrokers(env, at("2026-11-06T09:00:00")));
  assert.deepEqual([thursday.date, thursday.cached, thursday.stale, f.calls.length], ["2026-11-05", true, false, 0]);
  assert.deepEqual(thursday.warnings, ["JB ([CC] BJB Sekuritas Jawa Barat) is counted in CC, as idx.co.id counts it; IPOT lists it apart"], "only the fold");
});

test("a date that cannot be the ranking's session is refused", async () => {
  const empty = TOP.slice(0, TOP.indexOf("<tbody>") + 7) + TOP.slice(TOP.indexOf("</tbody>"));
  // Reset for a new session while the date page names the last one, which had trades.
  let err = await withFetch(ipotFetch({ top: empty }).fn, () => failure(topBrokers(memoryDb(), SUNDAY)));
  assert.equal(err.code, "provider");
  assert.ok(err.message.includes("shows no trades, yet the page that dates it names 2026-11-06, a session that had ended then"), err.message);
  // Before the open, a ranking without trades dated that day is right.
  const s = await withFetch(ipotFetch({ top: empty }).fn, () => topBrokers(memoryDb(), at("2026-11-06T08:00:00")));
  assert.deepEqual(s.brokers, []);
  // A Saturday is no session.
  err = await withFetch(ipotFetch({ statistic: named("2026-11-07") }).fn, () => failure(topBrokers(memoryDb(), SUNDAY)));
  assert.ok(err.message.includes("names 2026-11-07, which is not a trading day"), err.message);
});

test("a ranking accepted for one session is not dated another", async () => {
  const env = memoryDb();
  // Friday's ranking, read on Sunday, is remembered with its date.
  await withFetch(ipotFetch().fn, () => topBrokers(env, SUNDAY));
  assert.equal(accepted(env).date, "2026-11-06");
  // Monday night the date page names Monday, the ranking is Friday's: it has not rolled over.
  const mondayNight = at("2026-11-09T19:30:00");
  const err = await withFetch(ipotFetch({ statistic: named("2026-11-09") }).fn, () => failure(topBrokers(env, mondayNight)));
  assert.equal(err.code, "provider");
  assert.ok(err.message.includes("the broker ranking is the one IPOT showed for 2026-11-06, yet the page that dates it now names 2026-11-09"), err.message);
  // A ranking that moved on is Monday's, and remembered so (read again: the refused pair is in D1).
  env.cache.delete(PAIR_KEY);
  const s = await withFetch(ipotFetch({ top: TOP.replace(">2.79 T<", ">2.80 T<"), statistic: named("2026-11-09") }).fn, () => topBrokers(env, mondayNight));
  assert.equal(s.date, "2026-11-09");
  assert.equal(accepted(env).date, "2026-11-09");
});

test("a date page behind the latest session does not date new figures", async () => {
  const env = memoryDb();
  const monday = TOP.replace(">2.79 T<", ">2.80 T<"), tuesday = TOP.replace(">2.79 T<", ">2.81 T<");
  const tuesdayNight = at("2026-11-10T19:30:00");
  // Each call reads both pages live, on a D1 that keeps the ranking accepted last between calls.
  const live = (top, statistic, now, db = env) => { db.cache.delete(PAIR_KEY); return withFetch(ipotFetch({ top, statistic }).fn, () => topBrokers(db, now)); };
  // Monday night both pages are on Monday: accepted and remembered.
  assert.equal((await live(monday, named("2026-11-09"), at("2026-11-09T19:30:00"))).date, "2026-11-09");
  // Tuesday night the ranking has new figures while the date page still names Monday: no date, nothing remembered.
  const err = await failure(live(tuesday, named("2026-11-09"), tuesdayNight));
  assert.deepEqual([err.code, err.exit], ["provider", 3]);
  assert.equal(err.message, "ipot: the broker ranking read at 19:30 WIB on 2026-11-10 is dated 2026-11-09, yet the session of 2026-11-10 had ended then: the ranking may be that session's while the page that dates it has not rolled over, so it cannot be dated");
  assert.equal(accepted(env).date, "2026-11-09", "not written for a refused pair");
  // Both pages still on Monday: the very pair accepted for Monday.
  assert.equal((await live(monday, named("2026-11-09"), tuesdayNight)).date, "2026-11-09");
  // Once the date page names Tuesday, the right pair is accepted, that night and the next morning.
  for (const when of ["2026-11-10T20:30:00", "2026-11-11T07:00:00"]) assert.equal((await live(tuesday, named("2026-11-10"), at(when))).date, "2026-11-10", when);
  assert.equal(accepted(env).date, "2026-11-10");
  // On a D1 that remembers nothing, an older session's date is not believed either.
  const fresh = memoryDb();
  const refused = await failure(live(monday, named("2026-11-09"), tuesdayNight, fresh));
  assert.match(refused.message, /so it cannot be dated/);
  assert.equal(accepted(fresh), null);
  // The record is kept 30 days.
  const digest = await ipot.rankingDigest(ipot.parseRanking(monday).brokers);
  const recorded = (days) => { const db = memoryDb(); db.kv.set(ACCEPTED_KEY, JSON.stringify({ date: "2026-11-09", digest, at: tuesdayNight.getTime() - days * 86400e3 })); return db; };
  assert.equal((await live(monday, named("2026-11-09"), tuesdayNight, recorded(29))).date, "2026-11-09");
  assert.match((await failure(live(monday, named("2026-11-09"), tuesdayNight, recorded(31)))).message, /so it cannot be dated/);
});

test("a ranking without its date, or an HTTP error, is a provider error", async () => {
  let err = await withFetch(ipotFetch({ statistic: "Invalid stock code!" }).fn, () => failure(topBrokers(memoryDb(), SUNDAY)));
  assert.deepEqual([err.code, err.exit], ["provider", 3]);
  assert.match(err.message, /no Today row/);
  err = await withFetch(ipotFetch({ top: 503 }).fn, () => failure(topBrokers(memoryDb(), SUNDAY)));
  assert.equal(err.message, "ipot: broker ranking for the market: HTTP 503");
  // A page whose columns moved: the ranking is refused, and so is the copy kept of it.
  const env = memoryDb();
  const moved = TOP.replace(">S.Val</th>", ">Avg</th><th>S.Val</th>");
  err = await withFetch(ipotFetch({ top: moved }).fn, () => failure(topBrokers(env, SUNDAY)));
  assert.match(err.message, /^ipot: unexpected broker ranking layout: the columns read /);
  const f = ipotFetch();
  err = await withFetch(f.fn, () => failure(topBrokers(env, SUNDAY)));
  assert.match(err.message, /unexpected broker ranking layout/);
  assert.equal(f.calls.length, 0, "from D1, as the CLI's cache keeps the page");
});

// ---- the endpoint (service::market_brokers_ipot) ----------------------------

test("/api/market/brokers counts members as idx.co.id does", async () => {
  const env = memoryDb();
  const f = ipotFetch();
  await withFetch(f.fn, async () => {
    const out = await marketBrokers(env, null, "0", null, null, SUNDAY);
    assert.equal(out.schema, "idx.market.brokers.v1");
    assert.equal(out.provider, "ipot");
    const r = out.data;
    assert.deepEqual(Object.keys(r), ["date", "members", "total_volume", "total_value", "total_frequency", "sort", "rounded", "brokers"]);
    assert.equal(r.date, "2026-11-06");
    // idx.co.id's GetBrokerSummary of the session, read the same morning: 88 members, 2,205,570 trades, IDR 20,648,010,571,340.
    assert.deepEqual([r.members, r.brokers.length], [88, 88]);
    assert.equal(r.total_frequency, 2_205_570);
    assert.ok(Math.abs(r.total_value / 20_648_010_571_340 - 1) < 1e-4, r.total_value);
    assert.equal(r.rounded, true);
    assert.equal(r.sort, "value");
    const cp = r.brokers[0];
    assert.deepEqual(Object.keys(cp), ["rank", "code", "name", "volume", "value", "frequency", "value_share_pct", "buy_value", "sell_value", "net_value", "kind"]);
    assert.deepEqual([cp.rank, cp.code, cp.kind], [1, "CP", "local"]);
    assert.deepEqual([cp.buy_value, cp.sell_value, cp.net_value], [1.4e12, 1.39e12, 4.77e9]);
    assert.equal(r.brokers.find((b) => b.code === "CC").frequency, 211_212, "JB is counted in CC");
    assert.ok(r.brokers.every((b) => !["JB", "PS", "ZZ"].includes(b.code)));
    const share = r.brokers.reduce((t, b) => t + b.value_share_pct, 0);
    assert.ok(Math.abs(share - 100) < 1e-6);
    assert.deepEqual(out.warnings, ["JB ([CC] BJB Sekuritas Jawa Barat) is counted in CC, as idx.co.id counts it; IPOT lists it apart"]);
    assert.equal(f.calls.length, 2);

    // By trades, which the source gives exact; the CLI's default of 20 rows.
    const by = await marketBrokers(env, "2026-11-06", "5", "frequency", "ipot", SUNDAY);
    assert.deepEqual([by.data.members, by.data.brokers.length], [88, 5]);
    assert.ok(by.data.brokers.every((b, i, l) => i === 0 || l[i - 1].frequency >= b.frequency));
    assert.equal(by.data.brokers[0].code, "XL");
    assert.deepEqual(by.data.brokers.map((b) => b.rank), [1, 2, 3, 4, 5]);
    assert.equal(by.cached, true);
    assert.equal((await marketBrokers(env, null, null, null, null, SUNDAY)).data.brokers.length, 20);
    assert.equal(f.calls.length, 2, "from D1");

    // IPOT holds one session; a date in the future costs no request.
    for (const [date, says] of [["2026-10-16", "IPOT holds only its latest session, 2026-11-06, not 2026-10-16; source idx (idx.co.id) has the others"], ["2026-11-09", "2026-11-09 is in the future"]]) {
      const err = await failure(marketBrokers(env, date, null, null, null, SUNDAY));
      assert.deepEqual([err.code, err.exit], ["usage", 2], date);
      assert.equal(err.message, says);
    }
    assert.equal(f.calls.length, 2);
  });
});

test("a session IPOT cannot hold yet is not found", async () => {
  const env = memoryDb();
  const f = ipotFetch();
  await withFetch(f.fn, async () => {
    // A weekend, a holiday, and a session that has not ended: no request.
    for (const [date, now, says] of [
      ["2026-11-07", SUNDAY, "no trading data: 2026-11-07 is a Saturday"],
      ["2026-11-08", SUNDAY, "no trading data: 2026-11-08 is a Sunday"],
      ["2026-08-17", SUNDAY, "no trading data: 2026-08-17 is a trading holiday (Proklamasi Kemerdekaan)"],
      ["2026-11-09", at("2026-11-09T08:00:00"), "the session of 2026-11-09 has not ended (it ends at 16:15 WIB), and IPOT's ranking of it cannot be dated before 19:00 WIB"],
    ]) {
      const err = await failure(marketBrokers(env, date, null, null, null, now));
      assert.deepEqual([err.code, err.exit, err.status], ["not_found", 4, 404], date);
      assert.equal(err.message, says);
    }
    assert.equal(f.calls.length, 0);
  });
  // Monday night both of IPOT's pages still show Friday: a date page naming an older session dates nothing on its own.
  const mondayNight = at("2026-11-09T19:30:00");
  let err = await withFetch(ipotFetch().fn, () => failure(marketBrokers(env, "2026-11-09", null, null, null, mondayNight)));
  assert.deepEqual([err.code, err.exit], ["provider", 3]);
  assert.match(err.message, /so it cannot be dated/);
  // Once Friday's ranking has been accepted, on Sunday, the same pair is Friday's: Monday is not there yet.
  const fresh = memoryDb();
  await withFetch(ipotFetch().fn, () => marketBrokers(fresh, null, null, null, null, SUNDAY));
  fresh.cache.delete(PAIR_KEY);
  err = await withFetch(ipotFetch().fn, () => failure(marketBrokers(fresh, "2026-11-09", null, null, null, mondayNight)));
  assert.deepEqual([err.code, err.exit], ["not_found", 4]);
  assert.equal(err.message, "IPOT has not published 2026-11-09 yet: its latest session is 2026-11-06; source idx (idx.co.id) publishes each session after its close");
  // A session before IPOT's stays a usage error: it never holds one.
  err = await withFetch(ipotFetch().fn, () => failure(marketBrokers(fresh, "2026-11-05", null, null, null, SUNDAY)));
  assert.equal(err.exit, 2);
});

test("a ranking without trades is not found, and a code off the member list is named", async () => {
  const empty = TOP.slice(0, TOP.indexOf("<tbody>") + 7) + TOP.slice(TOP.indexOf("</tbody>"));
  const err = await withFetch(ipotFetch({ top: empty }).fn, () => failure(marketBrokers(memoryDb(), null, null, null, null, at("2026-11-06T08:00:00"))));
  assert.deepEqual([err.code, err.exit], ["not_found", 4]);
  assert.equal(err.message, "IPOT's broker ranking for 2026-11-06 has no trades yet");
  // IPOT lists a code with trades that is not an IDX member: it stays, named.
  const odd = TOP.replace(/<span class="text-local">CP<\/span>/, '<span class="text-local">QQ</span>');
  const out = await withFetch(ipotFetch({ top: odd }).fn, () => marketBrokers(memoryDb(), null, null, null, null, SUNDAY));
  assert.equal(out.data.brokers[0].code, "QQ");
  assert.equal(out.warnings[1], "IPOT lists trades for codes that are not on the IDX member list: QQ (KB Valbury Sekuritas)");
});

test("the source, sort and top are the CLI's, and idx.co.id is named as out of reach", async () => {
  const env = memoryDb();
  const f = ipotFetch();
  await withFetch(f.fn, async () => {
    let err = await failure(marketBrokers(env, null, null, null, "idx", SUNDAY));
    assert.deepEqual([err.code, err.exit], ["usage", 2]);
    assert.match(err.message, /^idx\.co\.id refuses requests from this site's host/);
    assert.match(err.message, /source ipot/);
    err = await failure(marketBrokers(env, null, null, null, "stockbit", SUNDAY));
    assert.equal(err.message, 'invalid source "stockbit"; the web terminal reads ipot');
    err = await failure(marketBrokers(env, null, null, "lots", null, SUNDAY));
    assert.equal(err.message, 'invalid sort "lots"; use value, volume or frequency');
    for (const top of ["-1", "2.5", "x"]) assert.equal((await failure(marketBrokers(env, null, top, null, null, SUNDAY))).code, "usage", top);
    assert.equal((await failure(marketBrokers(env, "2026-02-30", null, null, null, SUNDAY))).message, 'invalid date "2026-02-30"; use YYYY-MM-DD');
    assert.equal(f.calls.length, 0);
    // auto reads IPOT here too, and says why.
    const auto = await marketBrokers(env, null, "3", "VOLUME", "AUTO", SUNDAY);
    assert.equal(auto.data.sort, "volume");
    assert.equal(auto.warnings[0], "idx.co.id refuses requests from this site's host, so the ranking is IPOT's (Indo Premier) for its latest session, 2026-11-06");
    // The route hands each parameter over (checked before the clock is read).
    const ask = (qs) => failure(route(env, new URL(`https://ihsg.midoriresearch.com/api/market/brokers?${qs}`)));
    assert.equal((await ask("sort=lots")).message, 'invalid sort "lots"; use value, volume or frequency');
    assert.equal((await ask("source=stockbit")).message, 'invalid source "stockbit"; the web terminal reads ipot');
    assert.equal((await ask("top=-1")).code, "usage");
    assert.equal((await ask("date=2026-02-30")).message, 'invalid date "2026-02-30"; use YYYY-MM-DD');
  });
});
