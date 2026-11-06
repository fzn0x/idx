// node --test web/test  (no network). Checks the JS ports against the same
// fixtures the Rust tests use, so the web terminal and the CLI agree.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as an from "../src/analysis.js";
import * as y from "../src/yahoo.js";
import { parseRss, matchName, parseSymbols, marketIsOpen, valuationKeyPoint } from "../src/api.js";

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
  assert.equal(f.financial_currency, "IDR");
  // Same figures as the Rust test bbca_is_unchanged.
  assert.deepEqual([v.label, v.score, v.inputs_available], ["undervalued", 5, 8]);
});

// Fundamentals shaped like idx.fundamentals.v1 with nothing reported.
const blank = () => ({ symbol: "TEST", currency: null, financial_currency: null, sector: null, industry: null, price: null, valuation: {}, profitability: {}, health: {}, growth: {}, dividend: {}, per_share: {}, analyst: {} });
const brief = (v) => [v.label, v.score, v.inputs_available];
const rewardsBookOrLeverage = (v) => v.reasons.some((r) => r.includes("conservative") || r.includes("below book") || r.includes("strong"));

test("negative equity: GIAA (2026-11-06) is not a conservative balance sheet", () => {
  const r = JSON.parse(fixture("yahoo/quote_summary_fundamentals_giaa.json")).quoteSummary.result[0];
  const f = y.parseFundamentals(r, "GIAA");
  assert.equal(f.per_share.book_value, -0.001);
  assert.equal(f.health.debt_to_equity, null);
  assert.equal(f.valuation.price_to_book, null);
  assert.equal(f.financial_currency, "USD");
  assert.equal(an.negativeEquity(f), true);
  const v = an.verdict(f);
  // Yahoo alone (the web never asks MSN): the Rust test giaa_yahoo_only_matches_the_web.
  assert.deepEqual(brief(v), ["fair", -2, 4]);
  assert.deepEqual(v.reasons, [an.NEGATIVE_EQUITY_REASON, "net margin -9.4% is thin", "analyst mean target implies 233% upside"]);
});

test("negative equity and ROE rules match the Rust tests", () => {
  // negative_equity_is_adverse_and_sets_pb_roe_and_leverage_aside
  let f = blank();
  Object.assign(f, { price: 39 });
  Object.assign(f.per_share, { book_value: -0.001, eps_ttm: -23.54 });
  Object.assign(f.profitability, { net_margin_pct: -9.375 });
  Object.assign(f.growth, { revenue_growth_pct: 8.2, earnings_growth_pct: -92.482 });
  Object.assign(f.dividend, { yield_pct: 0 });
  Object.assign(f.analyst, { target_mean: 130 });
  let v = an.verdict(f);
  assert.deepEqual(brief(v), ["overvalued", -3, 5]);
  assert.equal(v.reasons[0], an.NEGATIVE_EQUITY_REASON);
  assert.ok(!rewardsBookOrLeverage(v), v.reasons.join(" | "));

  // negative_ratios_mean_negative_equity: a negative P/B is not "below book"...
  f = blank();
  f.valuation.price_to_book = -0.55; f.profitability.net_margin_pct = -51.5; f.growth.revenue_growth_pct = -17.6;
  v = an.verdict(f);
  assert.deepEqual(brief(v), ["overvalued", -4, 4]);
  assert.ok(!rewardsBookOrLeverage(v), v.reasons.join(" | "));
  // ...and a negative debt/equity is not conservative, nor its flipped ROE strong.
  f = blank();
  f.health.debt_to_equity = -2.89; f.profitability.roe_pct = 92; f.profitability.net_margin_pct = -9.4;
  v = an.verdict(f);
  assert.deepEqual(brief(v), ["overvalued", -3, 4]);
  assert.ok(!rewardsBookOrLeverage(v), v.reasons.join(" | "));

  // positive_roe_next_to_a_loss_is_not_rewarded
  f = blank();
  f.valuation.pe_trailing = 12; f.profitability.roe_pct = 40; f.profitability.net_margin_pct = -12; f.health.debt_to_equity = 0.3;
  v = an.verdict(f);
  assert.ok(v.reasons.includes("ROE 40.0% alongside a net loss: rule skipped"), v.reasons.join(" | "));
  assert.ok(!v.reasons.some((r) => r.includes("is strong")));
  assert.deepEqual(brief(v), ["fair", 1, 4]);

  // zero_book_value_is_not_negative_equity (Yahoo rounds tiny USD book values)
  for (const bv of [0, -0]) {
    f = blank();
    f.per_share.book_value = bv; f.health.debt_to_equity = 0.3;
    assert.equal(an.negativeEquity(f), false);
    assert.ok(an.verdict(f).reasons.some((r) => r.includes("conservative")));
  }
  assert.equal(an.negativeEquity(blank()), null, "nothing reported");
});

test("USD statements: ITMG's P/B is not scored, BBCA's is", () => {
  const r = JSON.parse(fixture("yahoo/quote_summary_fundamentals_itmg.json")).quoteSummary.result[0];
  const f = y.parseFundamentals(r, "ITMG");
  assert.equal(f.currency, "IDR");
  assert.equal(f.financial_currency, "USD");
  assert.ok(an.mixedCurrency(f));
  assert.ok(f.valuation.price_to_book > 14000, "the reported value is kept");
  const v = an.verdict(f);
  // #25: no P/B reason at all.
  assert.ok(!v.reasons.some((x) => x.includes("P/B")), v.reasons.join(" | "));
  // Same figures as the Rust test itmg_pb_is_set_aside.
  assert.deepEqual(brief(v), ["undervalued", 7, 8]);
  const same = { ...f, financial_currency: "idr" };
  assert.ok(!an.mixedCurrency(same));
  assert.ok(an.verdict(same).reasons.some((x) => x.includes("is rich")));
});

test("currency is the price's, never the statements'", () => {
  // The ITMG body without price.currency: summaryDetail names it too.
  const r = JSON.parse(fixture("yahoo/quote_summary_fundamentals_itmg.json")).quoteSummary.result[0];
  delete r.price.currency;
  let f = y.parseFundamentals(r, "ITMG");
  assert.equal(f.currency, "IDR");
  assert.ok(an.mixedCurrency(f));
  delete r.summaryDetail.currency;
  f = y.parseFundamentals(r, "ITMG");
  assert.equal(f.currency, null, "unknown, not USD");
  assert.equal(f.financial_currency, "USD");
});

test("brief key point leaves out a mixed-currency P/B (service::valuation_key_point)", () => {
  const withVerdict = (name, s) => {
    const f = y.parseFundamentals(JSON.parse(fixture(`yahoo/${name}`)).quoteSummary.result[0], s);
    f.verdict = an.verdict(f);
    return f;
  };
  const itmg = withVerdict("quote_summary_fundamentals_itmg.json", "ITMG");
  assert.equal(valuationKeyPoint(itmg), "Valuation verdict: undervalued (score +7, 8 inputs); P/E 7.5");
  const bbca = withVerdict("quote_summary_fundamentals_bbca.json", "BBCA");
  assert.ok(valuationKeyPoint(bbca).includes(", P/B "), valuationKeyPoint(bbca));
  assert.equal(valuationKeyPoint({ ...itmg, verdict: null }), null);
});

test("earnings parse: padded zero quarters become null", () => {
  const r = JSON.parse(fixture("yahoo/quote_summary_statements_bbca.json")).quoteSummary.result[0];
  const e = y.parseEarnings(r, "BBCA");
  assert.equal(e.quarters.length, 4);
  assert.equal(e.quarters[0].period, "3Q2025");
  assert.ok(e.quarterly.some((q) => q.earnings === null));
  assert.equal(e.next.analysts, 3);
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
  // A ticker starts with a letter or digit, like the CLI's Symbol::parse.
  assert.throws(() => parseSymbols("-"));
  assert.throws(() => parseSymbols("-BBCA"));
  assert.deepEqual(parseSymbols("buka-w"), ["BUKA-W"]);
});

test("market hours in WIB", () => {
  assert.equal(marketIsOpen(new Date("2026-10-14T03:00:00Z")), true, "Wednesday 10:00 WIB");
  assert.equal(marketIsOpen(new Date("2026-10-14T12:00:00Z")), false, "Wednesday 19:00 WIB");
  assert.equal(marketIsOpen(new Date("2026-10-17T03:00:00Z")), false, "Saturday");
});
