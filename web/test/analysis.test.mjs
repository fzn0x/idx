// node --test web/test  (no network). Checks the JS ports against the same
// fixtures the Rust tests use, so the web terminal and the CLI agree.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as an from "../src/analysis.js";
import * as y from "../src/yahoo.js";
import { parseRss, matchName, parseSymbols, marketIsOpen } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fixture = (p) => readFileSync(join(root, "tests", "fixtures", p), "utf8");

test("rsi matches Wilder reference", () => {
  const closes = [44.34, 44.09, 44.15, 43.61, 44.33, 44.83, 45.10, 45.42, 45.84, 46.08, 45.89, 46.03, 45.61, 46.28, 46.28];
  assert.ok(Math.abs(an.rsi(closes, 14) - 70.53) < 0.1);
  assert.equal(an.rsi(closes.slice(0, 10), 14), null);
});

test("chart parse and technical report on the BBCA fixture", () => {
  const body = JSON.parse(fixture("yahoo/chart_bbca_1mo_1d.json"));
  const h = y.parseChart(body, "1mo", "1d");
  assert.equal(h.symbol, "BBCA");
  assert.equal(h.candles.length, 22, "current-day bar with null close is dropped");
  assert.equal(h.summary.last_close, 6075);
  const t = an.technical(h);
  assert.equal(t.candles_used, 22);
  assert.equal(t.moving_averages.sma50, null);
  assert.equal(t.trend, "unknown");
  assert.ok(["buy", "sell", "neutral"].includes(t.signal.label));
});

test("fundamentals parse and verdict agree with the Rust expectations", () => {
  const r = JSON.parse(fixture("yahoo/quote_summary_fundamentals_bbca.json")).quoteSummary.result[0];
  const f = y.parseFundamentals(r, "BBCA");
  assert.ok(Math.abs(f.valuation.pe_trailing - 12.870893) < 1e-6);
  assert.equal(f.profitability.gross_margin_pct, null, "0 means n/a");
  assert.ok(Math.abs(f.dividend.yield_pct - 6.27) < 1e-9);
  assert.equal(f.analyst.recommendation, "strong_buy");
  const v = an.verdict(f);
  assert.ok(v.inputs_available >= 6);
  assert.ok(v.reasons.some((x) => x.includes("financial company")));
});

test("earnings parse: padded zero quarters become null", () => {
  const r = JSON.parse(fixture("yahoo/quote_summary_statements_bbca.json")).quoteSummary.result[0];
  const e = y.parseEarnings(r, "BBCA");
  assert.equal(e.quarters.length, 4);
  assert.equal(e.quarters[0].period, "3Q2025");
  assert.ok(e.quarterly.some((q) => q.earnings === null));
  assert.equal(e.next.analysts, 3);
});

test("earnings parse: padded zero revenue is null and has no margin (recorded 2026-11-06)", () => {
  for (const f of ["yahoo/quote_summary_earnings_tlkm_20261106.json", "yahoo/quote_summary_earnings_tlkm.json"]) {
    const r = JSON.parse(fixture(f)).quoteSummary.result[0];
    const e = y.parseEarnings(r, "TLKM");
    const q = e.quarterly.find((x) => x.period === "1Q2026");
    assert.equal(q.revenue, null, f);
    assert.equal(q.profit_margin_pct, null, f);
    assert.equal(q.earnings, 4344000000000, f);
    const ok = e.quarterly.find((x) => x.period === "2Q2026");
    assert.ok(Math.abs(ok.profit_margin_pct - 17.503685) < 1e-6, f);
  }
  const b = y.parseEarnings(JSON.parse(fixture("yahoo/quote_summary_statements_bbca.json")).quoteSummary.result[0], "BBCA");
  const q = b.quarterly.find((x) => x.period === "2Q2025");
  assert.equal(q.earnings, null);
  assert.equal(q.revenue, 28589918000000);
});

test("earnings estimates: this and next quarter and year, same numbers as the Rust parser", () => {
  const e = y.parseEarnings(JSON.parse(fixture("yahoo/quote_summary_earnings_tlkm.json")).quoteSummary.result[0], "TLKM");
  assert.deepEqual(e.estimates.map((x) => x.period), ["0q", "+1q", "0y", "+1y"]);
  assert.deepEqual(e.estimates.map((x) => x.period_type), ["quarterly", "quarterly", "annual", "annual"]);
  const q = e.estimates[0];
  assert.equal(q.end_date, "2026-09-30");
  assert.equal(q.eps_avg, 58.23986);
  assert.equal(q.eps_low, 57.81064);
  assert.equal(q.eps_year_ago, 48.54);
  assert.ok(Math.abs(q.eps_growth_pct - 19.98) < 1e-9);
  assert.equal(q.eps_analysts, 3);
  assert.ok(Math.abs(q.revenue_growth_pct - 4.25) < 1e-9);
  assert.equal(e.estimates[2].eps_analysts, 19);
  assert.equal(e.estimates[2].revenue_analysts, 22);
  const late = y.parseEarnings(JSON.parse(fixture("yahoo/quote_summary_earnings_tlkm_20261106.json")).quoteSummary.result[0], "TLKM");
  const nq = late.estimates[1];
  assert.equal(nq.eps_avg, 47.35);
  assert.equal(nq.revenue_avg, null, "padded revenue consensus");
  assert.equal(nq.revenue_growth_pct, null);
  assert.equal(nq.revenue_analysts, 0);
});

test("earnings estimates: a ticker no analyst covers has no consensus rows (recorded 2026-11-06)", () => {
  const e = y.parseEarnings(JSON.parse(fixture("yahoo/quote_summary_earnings_beef_20261106.json")).quoteSummary.result[0], "BEEF");
  assert.deepEqual(e.estimates, []);
  assert.equal(e.name, "PT Estika Tata Tiara Tbk");
  assert.equal(e.next.eps_avg, null);
  assert.equal(e.next.revenue_avg, null);
  const row = (n) => ({ trend: [{ period: "0q", earningsEstimate: {}, revenueEstimate: { avg: { raw: 0, fmt: null }, numberOfAnalysts: { raw: n, fmt: null } } }] });
  assert.deepEqual(y.parseEstimates(row(0)), []);
  assert.equal(y.parseEstimates(row(2))[0].revenue_analysts, 2);
  // Padded consensus amounts in calendarEvents are null too, as in the Rust parser.
  const padded = y.parseEarnings({ calendarEvents: { earnings: { revenueAverage: { raw: 0, fmt: null }, earningsAverage: { raw: 12.5, fmt: "12.50" } } } }, "X");
  assert.equal(padded.next.revenue_avg, null);
  assert.equal(padded.next.eps_avg, 12.5);
});

test("timeseries pivot", () => {
  const body = JSON.parse(fixture("yahoo/timeseries_bbca.json"));
  const f = y.parseTimeseries(body, "BBCA", "annual");
  assert.equal(f.periods.length, 4);
  assert.equal(f.periods.at(-1).income.net_income, 57537287000000);
  assert.equal(f.periods.at(-1).income.ebitda, undefined);
  const nf = y.parseTimeseries(JSON.parse(fixture("yahoo/timeseries_not_found.json")), "ZZZZ", "annual");
  assert.ok(nf.notFound);
});

test("v7 quote parse", () => {
  const body = JSON.parse(fixture("yahoo/quote_v7_bbca_bbri_tlkm.json"));
  const q = y.parseV7Quote(body.quoteResponse.result[0]);
  assert.equal(q.symbol, "BBCA");
  assert.equal(q.price, 5175);
  assert.equal(q.previous_close, 6075);
  assert.equal(q.source, "yahoo:quote");
  assert.ok(q.market_time.endsWith("+07:00"));
});

test("screener DSL", () => {
  const f = an.parseFilters("pe<15, roe>=15,sector=Bank, mcap>10T, div>3%");
  assert.equal(f.length, 5);
  assert.equal(f[3].value, 10e12);
  assert.throws(() => an.parseFilters("pe 15"), /operator/);
  assert.throws(() => an.parseFilters("nope<1"), /unknown filter key/);
  const row = { symbol: "A", pe_trailing: 10, market_cap: 2e12, sector: "Financial Services" };
  assert.ok(an.matches(row, an.parseFilters("pe<15,mcap>1T,sector=financial")));
  assert.ok(!an.matches(row, an.parseFilters("roe>1")), "missing never matches");
  const rows = [{ symbol: "X", pe_trailing: 12 }, { symbol: "Y", pe_trailing: 8 }, { symbol: "Z", pe_trailing: null }];
  an.sortRows(rows, "pe_trailing", false);
  assert.deepEqual(rows.map((r) => r.symbol), ["Y", "X", "Z"]);
});

test("sentiment lexicon", () => {
  assert.ok(an.scoreText("Saham BBCA melonjak, laba naik 12%").score > 0);
  assert.ok(an.scoreText("IHSG anjlok, investor asing jual bersih").score < 0);
  assert.equal(an.scoreText("Russell index rebalancing").neg.length, 0);
  const items = ["Laba BBCA naik", "BBCA menguat ke rekor", "Saham BBCA turun tipis"].map((title) => {
    const s = an.scoreText(title);
    return { title, published: null, sentiment: s.score, positive_terms: s.pos, negative_terms: s.neg };
  });
  const r = an.aggregateSentiment("BBCA", null, items, 2);
  assert.equal(r.label, "positive");
  assert.equal(r.headlines.length, 2);
});

test("rss parse and name matching", () => {
  const items = parseRss(fixture("news/kontan_investasi.xml"), "kontan/investasi");
  assert.equal(items.length, 25);
  assert.ok(items[0].title.startsWith("Yield SBN"));
  assert.equal(items[0].published, "2026-10-17T09:35:05+07:00");
  const cnbc = parseRss(fixture("news/cnbc_market_excerpt.xml"), "cnbcindonesia/market");
  assert.equal(cnbc.length, 20);
  assert.ok(!cnbc[0].summary.includes("<"));
  assert.equal(matchName("PT Bank Central Asia Tbk"), "Bank Central Asia");
  assert.deepEqual(parseSymbols("bbca, BBRI tlkm bbca"), ["BBCA", "BBRI", "TLKM"]);
  assert.throws(() => parseSymbols("BB$CA"));
});

test("market hours in WIB", () => {
  assert.equal(marketIsOpen(new Date("2026-10-14T03:00:00Z")), true, "Wednesday 10:00 WIB");
  assert.equal(marketIsOpen(new Date("2026-10-14T12:00:00Z")), false, "Wednesday 19:00 WIB");
  assert.equal(marketIsOpen(new Date("2026-10-17T03:00:00Z")), false, "Saturday");
});
