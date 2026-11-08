// node --test web/test/*.mjs (no network). IPOT's market statistics and the
// page that dates them (src/ipot.js, dated by the broker ranking's rules), on
// the same recorded pages as src/providers/ipot/marketstat.rs and
// ipot::market_summary's tests, with the same figures and messages.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as ipot from "../src/ipot.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", "ipot", p), "utf8");
const SUMMARY = fixture("marketstatistic_20261106.html");
const STATISTIC = fixture("statistic_bbca_20261106.html");
/** BBCA's statistics page with its Today row on `day`. */
const named = (day) => STATISTIC.replaceAll("2026-11-06", day);
const at = (wib) => Date.parse(`${wib}+07:00`);

test("the session total and the foreign side, as marketstat::summary reads them", () => {
  const s = ipot.parseMarketSummary(SUMMARY);
  assert.deepEqual(s, { value: 10_320_000_000_000, foreign_buy_value: 3_760_000_000_000, foreign_sell_value: 3_430_000_000_000, foreign_net_value: 329_330_000_000, rounded: true });
  assert.ok(ipot.summaryTraded(s));
  const sold = ipot.parseMarketSummary(SUMMARY.replace(">329.33 B<", ">-1.20 T<"));
  assert.equal(sold.foreign_net_value, -1_200_000_000_000);
  const zero = SUMMARY.replace(">10.32 T<", ">0<").replace(">3.76 T<", ">0<").replace(">3.43 T<", ">0<");
  assert.ok(!ipot.summaryTraded(ipot.parseMarketSummary(zero)));
});

test("a page headed otherwise or without a row is refused with the CLI's words", () => {
  const moved = SUMMARY.replace('<th align="right">Value</th>', '<th align="right">Lot</th>');
  assert.throws(() => ipot.parseMarketSummary(moved), { message: "unexpected market summary layout: the tables are headed Board, Lot, Lot, Freq; Foreign, Value, Lot, Freq, not Board, Value, Lot, Freq; Foreign, Value, Lot, Freq" });
  assert.throws(() => ipot.parseMarketSummary(SUMMARY.replace(">F. NET<", ">F. NETT<")), { message: "unexpected market summary page (no F. NET row)" });
  // Kept with the pair, the error is the provider's.
  const pair = ipot.summaryPair("Invalid stock code!", STATISTIC, at("2026-11-08T10:00:00"));
  assert.match(pair.summary.error, /^unexpected market summary layout/);
});

test("a summary is dated by the broker ranking's rules, in its own words", async () => {
  const kind = ipot.MARKET_SUMMARY;
  const sunday = at("2026-11-08T10:00:00");
  const d = await ipot.dateLatest(ipot.summaryPair(SUMMARY, STATISTIC, sunday), kind, { now: sunday });
  assert.equal(d.date, "2026-11-06");
  assert.equal(d.page.foreign_net_value, 329_330_000_000);
  assert.equal(d.accept.date, "2026-11-06");
  const refused = async (pair, opts, message) => {
    await assert.rejects(ipot.dateLatest(pair, kind, { now: pair.summary_at, ...opts }), (e) => e instanceof ipot.RankingError && (message instanceof RegExp ? message.test(e.message) : e.message === message));
  };
  // Read in a trading day's hours, or apart.
  await refused(ipot.summaryPair(SUMMARY, STATISTIC, at("2026-11-09T10:00:00")), {}, "ipot: the market summary was read at 10:00 WIB on 2026-11-09, in a trading day's hours (08:58 to 19:00 WIB), when it and the page that dates it can be on different sessions (when they roll over has not been measured), so it cannot be dated");
  await refused(ipot.summaryPair(SUMMARY, STATISTIC, sunday, sunday + 61e3), {}, "ipot: the market summary and the page that dates it were read at different times, so the summary has no date");
  // A Saturday is no session; a Friday page read Tuesday night is behind Tuesday's.
  await refused(ipot.summaryPair(SUMMARY, named("2026-11-07"), sunday), {}, "ipot: the page that dates the market summary names 2026-11-07, which is not a trading day");
  await refused(ipot.summaryPair(SUMMARY, STATISTIC, at("2026-11-10T19:30:00")), {}, "ipot: the market summary read at 19:30 WIB on 2026-11-10 is dated 2026-11-06, yet the session of 2026-11-10 had ended then: the summary may be that session's while the page that dates it has not rolled over, so it cannot be dated");
  // Unless it is the very pair accepted for that session.
  const ok = await ipot.dateLatest(ipot.summaryPair(SUMMARY, STATISTIC, at("2026-11-10T19:30:00")), kind, { now: at("2026-11-10T19:30:00"), accepted: d.accept });
  assert.equal(ok.date, "2026-11-06");
  // The figures accepted for Friday dated Monday have not rolled over.
  await refused(ipot.summaryPair(SUMMARY, named("2026-11-09"), at("2026-11-09T19:30:00")), { accepted: d.accept }, "ipot: the market summary is the one IPOT showed for 2026-11-06, yet the page that dates it now names 2026-11-09: the summary has not rolled over to that session");
  // A copy standing in for a failed read is that failure.
  await refused(ipot.summaryPair(SUMMARY, STATISTIC, at("2026-11-06T10:00:00")), { cached: true, failure: "HTTP 503", now: sunday }, /^IPOT could not be read \(HTTP 503\), and its cached copy cannot be used: the market summary was read at 10:00 WIB on 2026-11-06/);
  assert.equal(ipot.notAsked(at("2026-11-09T10:00:00"), kind), "a market summary read at 10:00 WIB on 2026-11-09, in a trading day's hours (08:58 to 19:00 WIB), cannot be dated, as it and the page that dates it can be on different sessions then (when they roll over has not been measured); no copy read outside those hours is cached, so IPOT was not asked");
  // Its pairs answer in the hours as the ranking's do.
  assert.ok(ipot.answersInHours(ipot.summaryPair(SUMMARY, STATISTIC, at("2026-11-06T19:30:00")), kind));
  assert.ok(!ipot.answersInHours(ipot.summaryPair(SUMMARY, STATISTIC, at("2026-11-06T10:00:00")), kind));
  assert.ok(!ipot.readTogether(ipot.summaryPair(SUMMARY, STATISTIC, sunday, sunday + 61e3), kind));
});
