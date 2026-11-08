// node --test web/test/*.mjs (no network). IPOT's market-wide broker ranking
// (ma-topbroker.php) and the page that dates it (statistic.php?code=BBCA):
// the parser in src/ipot.js is the port of src/providers/ipot/topbroker.rs,
// tested on the same recorded pages with the same expectations, so the CLI's
// `market brokers --source ipot` and the web's read them alike.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as ipot from "../src/ipot.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", "ipot", p), "utf8");
const TOP = fixture("topbroker_20261106.html");
const STATISTIC = fixture("statistic_bbca_20261106.html");

const top = () => ipot.parseRanking(TOP);
const row = (r, code) => r.brokers.find((b) => b.code === code);

test("every broker of the session in the page's order", () => {
  const r = top();
  assert.equal(r.brokers.length, 91);
  assert.ok(r.brokers.every((b, i) => b.rank === i + 1));
  assert.equal(r.rounded, true);

  const first = r.brokers[0];
  assert.deepEqual([first.code, first.name, first.kind], ["CP", "KB Valbury Sekuritas", "local"]);
  assert.equal(first.value, 2_790_000_000_000);
  assert.equal(first.net_value, 4_770_000_000);
  assert.equal(first.buy_value, 1_400_000_000_000);
  assert.equal(first.sell_value, 1_390_000_000_000);
  assert.equal(first.volume, 6_480_000_000);
  assert.equal(first.frequency, 43_020, "trades are exact");

  assert.equal(r.brokers[1].kind, "foreign", "AK");
  const cc = row(r, "CC");
  assert.equal(cc.kind, "state_owned");
  assert.equal(cc.net_value, -264_040_000_000, "net selling");
  assert.equal(cc.sell_value, 1_070_000_000_000);
  const gi = row(r, "GI");
  assert.equal(gi.value, 56_370_000);
  assert.equal(gi.volume, 278_800, "small figures are exact");

  // The last two traded nothing; the name keeps its ampersand.
  const last = r.brokers.at(-1);
  assert.deepEqual([last.rank, last.code], [91, "ZZ"]);
  assert.equal(last.name, "Kliring & Penjaminan Efek Ind.");
  assert.deepEqual([last.value, last.net_value, last.frequency], [0, 0, 0]);
  assert.ok(!ipot.brokerTraded(last) && !ipot.brokerTraded(row(r, "PS")));
  assert.equal(r.brokers.filter(ipot.brokerTraded).length, 89);
});

test("the commented template is not read", () => {
  const r = top();
  assert.ok(r.brokers.every((b) => b.code !== "AA"));
  assert.ok(r.brokers.every((b) => !(b.name || "").includes("Lorem")));
  // The template alone, as the page would show it without rows.
  const start = TOP.indexOf("<!--tr>");
  const head = TOP.slice(TOP.indexOf("<thead>"), TOP.indexOf("</thead>") + 8);
  const empty = `<table>${head}<tbody>${TOP.slice(start, TOP.indexOf("</tbody>"))}</tbody></table>`;
  assert.deepEqual(ipot.parseRanking(empty).brokers, []);
  assert.throws(() => ipot.parseRanking("<html>maintenance</html>"), /^Error: unexpected broker ranking page \(no table\)$/);
});

test("a page whose columns moved is refused", () => {
  const swapped = TOP.replace(">B.Val<", ">\u0000<").replace(">S.Val<", ">B.Val<").replace(">\u0000<", ">S.Val<");
  assert.throws(() => ipot.parseRanking(swapped), (e) => {
    assert.ok(e.message.endsWith("the columns read #, Company (2 wide), T.Val, N.Val, S.Val, B.Val, T.Vol, T.Freq, not #, Company (2 wide), T.Val, N.Val, B.Val, S.Val, T.Vol, T.Freq"), e.message);
    return true;
  });
  for (const [page, why] of [
    [swapped, "bought and sold swapped"],
    [TOP.replace(">S.Val</th>", ">Avg</th><th>S.Val</th>"), "a column added"],
    [TOP.replace('<th colspan="2" align="left">', "<th>"), "code and name in one column"],
    [TOP.replace("<thead>", "").replace("</thead>", ""), "no head"],
  ]) {
    assert.throws(() => ipot.parseRanking(page), /unexpected broker ranking layout/, why);
  }
  assert.throws(() => ipot.parseRanking(TOP.replace("<thead>", "").replace("</thead>", "")), /the columns read none, not #/);
});

test("the session is the date on the Today row", () => {
  assert.equal(ipot.sessionDate(STATISTIC), "2026-11-06");
  // The commented-out 2015 table has a Today row of its own.
  const old = STATISTIC.slice(STATISTIC.indexOf("<!--\n"));
  assert.equal(ipot.sessionDate(`<tbody>${old}`), null);
  assert.equal(ipot.sessionDate(`<tbody>${old.replace("<!--", "")}`), "2015-09-15");
  assert.equal(ipot.sessionDate("Invalid stock code!"), null);
  // The first date on the page is the Today row's high.
  assert.equal(ipot.sessionDate(STATISTIC.replace("2026-11-06", "2026-11-05")), null, "high and low on different days");
});

test("a broker trading under another member's code is counted in it", () => {
  let r = top();
  const jb = { ...row(r, "JB") };
  assert.equal(jb.name, "[CC] BJB Sekuritas Jawa Barat");
  const cc = { ...row(r, "CC") };
  assert.deepEqual(ipot.foldListedApart(r), [{ code: "JB", name: "[CC] BJB Sekuritas Jawa Barat", into: "CC" }]);
  assert.equal(r.brokers.length, 90);
  assert.ok(r.brokers.every((b) => b.code !== "JB"));
  const merged = row(r, "CC");
  // idx.co.id's 211,212 trades for CC on 2026-11-06.
  assert.equal(merged.frequency, 211_212);
  assert.equal(merged.value, 1_880_000_000_000 + 1_490_000_000);
  assert.equal(merged.volume, 4_470_000_000 + 38_660_000);
  assert.equal(merged.net_value, -264_040_000_000 + 1_130_000_000);
  assert.deepEqual([merged.rank, merged.name, merged.kind], [cc.rank, cc.name, cc.kind]);

  // A member that is not on the page: the row stays as published.
  r = top();
  r.brokers = r.brokers.filter((b) => b.code !== "CC");
  assert.deepEqual(ipot.foldListedApart(r), []);
  assert.ok(r.brokers.some((b) => b.code === "JB"));
  assert.equal(ipot.memberInBrackets(" [cc] x"), "CC");
  assert.equal(ipot.memberInBrackets("[] x"), null);
  assert.equal(ipot.memberInBrackets("[not a code] x"), null);
  assert.equal(ipot.memberInBrackets("MANDIRI SEKURITAS"), null);
});

test("the same figures give the same fingerprint, other figures another", async () => {
  const a = await ipot.rankingDigest(top().brokers);
  assert.match(a, /^[0-9a-f]{64}$/);
  assert.equal(await ipot.rankingDigest(top().brokers), a);
  // Names are not part of it; a figure is.
  const renamed = top().brokers.map((b) => ({ ...b, name: "x" }));
  assert.equal(await ipot.rankingDigest(renamed), a);
  assert.notEqual(await ipot.rankingDigest(ipot.parseRanking(TOP.replace(">2.79 T<", ">2.80 T<")).brokers), a);
});
