// node --test web/test/*.mjs (no network). The IPOT port in src/ipot.js must
// agree with src/providers/ipot on the same recorded pages, and the bundled
// broker names must match data/brokers.csv.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as ipot from "../src/ipot.js";
import { broker } from "../src/api.js";
import BROKERS from "../src/brokers.json" with { type: "json" };

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", "ipot", p), "utf8");
const DAY = fixture("brokersummary_bbca_20261016.html");
const RANGE = fixture("brokersummary_bbca_20261012_20261016.html");
const EMPTY = fixture("brokersummary_bbca_20261017_no_trades.html");
const INVALID = fixture("brokersummary_zzzz_invalid.html");

test("numbers keep the published precision", () => {
  assert.deepEqual(ipot.number("1.6 M"), { v: 1_600_000, rounded: true });
  assert.deepEqual(ipot.number("841.2 B"), { v: 841_200_000_000, rounded: true });
  assert.deepEqual(ipot.number("-2.3 T"), { v: -2_300_000_000_000, rounded: true });
  assert.deepEqual(ipot.number("687,067 "), { v: 687_067, rounded: false });
  assert.equal(ipot.number(" "), null);
  assert.equal(ipot.number("n/a"), null);
});

test("one day parses like the Rust side", () => {
  const p = ipot.parse(DAY);
  assert.equal(p.from, "2026-10-16");
  assert.equal(p.to, "2026-10-16");
  assert.equal(p.buyers.length, 10);
  assert.equal(p.sellers.length, 10);
  assert.deepEqual(p.buyers[0], { rank: 1, code: "MG", name: null, kind: "local", lots: 1_600_000, value: 841_200_000_000, average_price: 5_191 });
  assert.equal(p.sellers[0].code, "CC");
  assert.equal(p.sellers[0].kind, "state_owned");
  assert.equal(p.buyers[3].kind, "foreign");
  assert.equal(p.sellers[3].lots, 687_067);
  assert.equal(p.sellers[9].rank, 10);
  assert.equal(p.total_value, 6_600_000_000_000);
  assert.equal(p.foreign_net_value, -2_300_000_000_000);
  assert.equal(p.total_lots, 12_600_000);
  assert.equal(p.average_price, 5_228);
  assert.equal(p.rounded, true);
});

test("range, empty and invalid pages", () => {
  const r = ipot.parse(RANGE);
  assert.equal(r.from, "2026-10-12");
  assert.equal(r.total_value, 9_900_000_000_000);
  const e = ipot.parse(EMPTY);
  assert.equal(e.buyers.length + e.sellers.length, 0);
  assert.equal(e.total_value, 0);
  assert.equal(e.average_price, null);
  assert.equal(e.rounded, false);
  assert.deepEqual(ipot.parse(INVALID), { invalid: true });
  assert.throws(() => ipot.parse("<html>maintenance</html>"), /no table/);
  assert.equal(
    ipot.url("BBCA", "2026-10-12", "2026-10-16", "foreign", "regular"),
    "https://www.indopremier.com/module/saham/include/data-brokersummary.php?code=BBCA&start=10/12/2026&end=10/16/2026&fd=F&board=RG",
  );
});

test("bundled broker names match data/brokers.csv", () => {
  const csv = Object.fromEntries(readFileSync(join(root, "data", "brokers.csv"), "utf8").split("\n").map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#") && l !== "code,name").map((l) => [l.slice(0, l.indexOf(",")), l.slice(l.indexOf(",") + 1)]));
  assert.deepEqual(BROKERS, csv);
  assert.equal(BROKERS.CC, "MANDIRI SEKURITAS");
});

test("a spurious 404 is retried, other failures are not", async () => {
  let calls = 0;
  const flaky = async () => (++calls < 3 ? new Response("nginx", { status: 404 }) : new Response(DAY, { status: 200 }));
  assert.equal(await ipot.fetchPage("u", { pauseMs: 1, fetchFn: flaky }), DAY);
  assert.equal(calls, 3);
  calls = 0;
  const down = async () => { calls++; return new Response("busy", { status: 503 }); };
  await assert.rejects(ipot.fetchPage("u", { pauseMs: 1, fetchFn: down }), /HTTP 503/);
  assert.equal(calls, 1);
});

/** A `get` for rebuildNet over recorded BBCA pages, counting the reads. */
function recorded(files, today) {
  const calls = [];
  const get = async (from, to) => {
    calls.push(`${from}..${to}`);
    const f = files[`${from}..${to}`];
    if (!f) throw new Error(`no page for ${from}..${to}`);
    return { page: ipot.parse(fixture(f)), fetchedOn: today };
  };
  return { get, calls };
}

test("a range through the latest session gets its foreign net rebuilt like the Rust side", async () => {
  // Recorded the weekend after: the week page counts 10-16 twice (-4.1 T).
  const week = ipot.parse(RANGE);
  assert.equal(week.foreign_net_value, -4_100_000_000_000);
  assert.ok(ipot.rangeNetSuspect("2026-10-12", "2026-10-16", week, "2026-10-17", "2026-10-17"));
  let r = recorded({
    "2026-10-12..2026-10-14": "brokersummary_bbca_20261012_20261014.html",
    "2026-10-15..2026-10-15": "brokersummary_bbca_20261015.html",
    "2026-10-16..2026-10-16": "brokersummary_bbca_20261016.html",
  }, "2026-10-17");
  assert.equal(await ipot.rebuildNet("2026-10-12", "2026-10-16", "2026-10-17", r.get), -1_823_100_000_000);
  assert.deepEqual(r.calls, ["2026-10-16..2026-10-16", "2026-10-15..2026-10-15", "2026-10-12..2026-10-14"]);

  // Probed 2026-11-06 before the open: 11-02..11-05 says 595.4 B; Friday's page is empty yet.
  r = recorded({
    "2026-11-06..2026-11-06": "brokersummary_bbca_20261017_no_trades.html",
    "2026-11-05..2026-11-05": "brokersummary_bbca_20261105.html",
    "2026-11-04..2026-11-04": "brokersummary_bbca_20261104.html",
    "2026-11-02..2026-11-03": "brokersummary_bbca_20261102_20261103.html",
  }, "2026-11-06");
  assert.equal(await ipot.rebuildNet("2026-11-02", "2026-11-05", "2026-11-06", r.get), 292_300_000_000);
  assert.equal(await ipot.rebuildNet("2026-11-02", "2026-11-06", "2026-11-06", r.get), 292_300_000_000);
  // Two sessions, both split off: no rest is read.
  r.calls.length = 0;
  assert.equal(await ipot.rebuildNet("2026-11-04", "2026-11-05", "2026-11-06", r.get), 437_200_000_000);
  assert.deepEqual(r.calls, ["2026-11-05..2026-11-05", "2026-11-04..2026-11-04"]);

  // Older ranges, a single session and a page cached while its range was current.
  const older = ipot.parse(fixture("brokersummary_bbca_20261102_20261104.html"));
  assert.equal(ipot.rangeNetSuspect("2026-11-02", "2026-11-04", older, "2026-11-09", "2026-11-09"), false);
  const doubled = ipot.parse(fixture("brokersummary_bbca_20261102_20261105.html"));
  assert.ok(ipot.rangeNetSuspect("2026-11-02", "2026-11-05", doubled, "2026-11-06", "2026-11-09"), "cached on Friday, read on Monday");
  assert.equal(ipot.rangeNetSuspect("2026-11-02", "2026-11-05", doubled, "2026-11-09", "2026-11-09"), false);
  assert.equal(ipot.rangeNetSuspect("2026-11-05", "2026-11-05", doubled, "2026-11-06", "2026-11-06"), false);
  assert.equal(ipot.prevTradingDay("2026-11-09"), "2026-11-06");
  assert.equal(ipot.wibDate(Date.parse("2026-11-05T17:30:00Z") / 1000), "2026-11-06");
});

// Minimal D1 stand-in: every read misses, writes succeed.
const env = { DB: { prepare: () => ({ bind: () => ({ first: async () => null, run: async () => ({}) }) }) } };

test("the /api/broker envelope names brokers and reports failed tickers", async () => {
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (u) => new Response(String(u).includes("code=ZZZZ") ? INVALID : DAY, { status: 200 });
  try {
    const out = await broker(env, "BBCA,ZZZZ", "2026-10-16");
    assert.equal(out.schema, "idx.broker.v1");
    assert.equal(out.provider, "ipot");
    assert.equal(out.data.length, 1);
    const s = out.data[0];
    assert.equal(s.name, "BANK CENTRAL ASIA Tbk");
    assert.equal(s.buyers[0].name, "SEMESTA INDOVEST SEKURITAS");
    assert.equal(s.investor, "all");
    assert.ok(out.warnings.includes("no broker summary for ZZZZ"), out.warnings);
    await assert.rejects(broker(env, "ZZZZ", "2026-10-16"), (e) => e.code === "not_found");
    await assert.rejects(broker(env, "BBCA", "2026-10-16", "2026-10-01"), (e) => e.code === "usage");
    await assert.rejects(broker(env, "BBCA", null, "2026-10-16", "2026-10-01"), (e) => e.code === "usage");
    await assert.rejects(broker(env, "BBCA", "16/10/2026"), (e) => e.code === "usage");
    await assert.rejects(broker(env, "BBCA", null, null, null, "retail"), (e) => e.code === "usage");
  } finally {
    globalThis.fetch = realFetch;
  }
});
