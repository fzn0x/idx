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
  assert.ok(!v.reasons.some((x) => x.includes("is rich")), v.reasons.join(" | "));
  assert.ok(v.reasons.includes("statements in USD and price in IDR: P/B rule skipped"));
  // Same figures as the Rust test itmg_pb_is_set_aside.
  assert.deepEqual(brief(v), ["undervalued", 7, 8]);
  const same = { ...f, financial_currency: "idr" };
  assert.ok(!an.mixedCurrency(same));
  assert.ok(an.verdict(same).reasons.some((x) => x.includes("is rich")));
});

// ---- growth, valuation and risk lenses: same fixtures and figures as the
// Rust tests in src/analysis/lens.rs (mod fixtures), Yahoo only.

const quoteSummary = (s) => JSON.parse(fixture(`yahoo/quote_summary_fundamentals_${s}.json`)).quoteSummary.result[0];
const lensInputs = (s, statements) => {
  const f = y.parseFundamentals(quoteSummary(s), s.toUpperCase());
  const fin = y.parseTimeseries(JSON.parse(fixture(`yahoo/${statements}`)), s.toUpperCase(), "annual");
  const h = y.parseChart(JSON.parse(fixture(`yahoo/chart_${s}_1y_1d.json`)), "1y", "1d");
  const b = y.parseChart(JSON.parse(fixture("yahoo/chart_jkse_1y_1d.json")), "1y", "1d");
  return { f, fin, h, b, v: an.valuationLens(f), g: an.growthLens(f.symbol, f, fin), r: an.riskLens(f.symbol, f, h, b) };
};
const lensSummary = (x) => [x.reading.label, x.reading.score, x.reading.inputs_available];
const ruleIds = (x) => x.reading.rules.map((r) => [r.rule, r.points]);
const noted = (x, needle) => x.reading.notes.some((n) => n.includes(needle));

test("lenses on BBCA (a bank) match the Rust test bbca_bank", () => {
  const { v, g, r } = lensInputs("bbca", "timeseries_bbca.json");
  assert.deepEqual(lensSummary(v), ["cheap", 3, 5]);
  assert.deepEqual(ruleIds(v), [["pe_trailing", 1], ["peg", 0], ["price_to_book", 0], ["dividend_yield", 1], ["analyst_target", 1]]);
  assert.ok(noted(v, "not scored for a financial company"));
  assert.equal(v.target_upside_pct, 58.33);
  assert.deepEqual(lensSummary(g), ["growing", 3, 6]);
  assert.deepEqual(ruleIds(g), [["revenue_growth", 0], ["earnings_growth", 0], ["eps_forward_growth", 1], ["revenue_cagr", 0], ["revenue_consistency", 1], ["net_income_cagr", 1]]);
  assert.equal(g.annual.length, 4);
  assert.deepEqual(g.sources, ["yahoo:quoteSummary", "yahoo:timeseries"]);
  assert.deepEqual(lensSummary(r), ["moderate", 0, 4]);
  assert.deepEqual(ruleIds(r), [["losses", 0], ["beta", 0], ["volatility", 0], ["drawdown", 0]]);
  assert.equal(r.price.observations, 238);
  assert.ok(Math.abs(r.price.beta - 0.946) <= 0.002, r.price.beta);
});

test("lenses on TLKM match the Rust test tlkm_moderate_risk", () => {
  const { v, g, r } = lensInputs("tlkm", "timeseries_tlkm.json");
  assert.deepEqual(lensSummary(v), ["fair", 2, 7]);
  assert.deepEqual(ruleIds(v), [["pe_trailing", 1], ["peg", -1], ["price_to_book", 0], ["price_to_sales", 0], ["ev_to_ebitda", 1], ["dividend_yield", 0], ["analyst_target", 1]]);
  assert.equal(v.reading.rules[5].reason, "dividend yield 10.0% is high, but the payout (125%) exceeds earnings");
  assert.deepEqual(lensSummary(g), ["mixed", 0, 6]);
  assert.deepEqual(ruleIds(g), [["revenue_growth", 0], ["earnings_growth", 1], ["eps_forward_growth", 1], ["revenue_cagr", -1], ["revenue_consistency", 0], ["net_income_cagr", -1]]);
  assert.deepEqual(lensSummary(r), ["moderate", 2, 6]);
  assert.deepEqual(ruleIds(r), [["leverage", 0], ["liquidity", 1], ["losses", 0], ["beta", 0], ["volatility", 1], ["drawdown", 0]]);
  const p = r.price;
  assert.deepEqual([p.benchmark, p.from, p.to, p.observations], ["^JKSE", "2025-11-05", "2026-11-04", 238]);
  assert.ok(Math.abs(p.beta - 1.122) <= 0.002 && Math.abs(p.correlation - 0.665) <= 0.002, `${p.beta} ${p.correlation}`);
  assert.equal(r.reading.rules[4].reason, "volatility 49.2% a year is 1.68x IHSG's 29.3% (over 1.5x)");
});

test("lenses on GIAA match the Rust test giaa_high_risk_from_negative_equity", () => {
  const { v, g, r } = lensInputs("giaa", "timeseries_annual_giaa.json");
  assert.deepEqual(lensSummary(v), ["insufficient_data", 0, 1]);
  assert.deepEqual(ruleIds(v), [["dividend_yield", 0]]);
  assert.ok(noted(v, "P/B: shareholders' equity is negative"));
  assert.ok(noted(v, "P/S 3769.72 not scored: statements in USD, price in IDR"));
  assert.ok(noted(v, "from 1 analyst: fewer than 3"));
  assert.deepEqual(lensSummary(g), ["mixed", 0, 4]);
  assert.deepEqual(ruleIds(g), [["revenue_growth", 0], ["revenue_cagr", 1], ["revenue_consistency", 0], ["net_income_cagr", -1]]);
  assert.equal(g.statements_currency, "USD");
  assert.equal(g.reading.rules[3].reason, "net income turned from a profit in FY2022 to a loss in FY2025");
  assert.deepEqual(lensSummary(r), ["high", 6, 6]);
  assert.deepEqual(ruleIds(r), [["negative_equity", 2], ["liquidity", 1], ["losses", 1], ["beta", 0], ["volatility", 1], ["drawdown", 1]]);
  assert.equal(r.negative_equity, true);
});

test("lenses on ITMG match the Rust test itmg_cheap_with_usd_multiples_skipped", () => {
  const { v, g, r } = lensInputs("itmg", "timeseries_annual_itmg.json");
  assert.deepEqual(lensSummary(v), ["cheap", 4, 4]);
  assert.deepEqual(ruleIds(v), [["pe_trailing", 2], ["peg", 1], ["dividend_yield", 1], ["analyst_target", 0]]);
  for (const m of ["P/B 14103.31", "P/S 13807.10", "EV/EBITDA 91203.7"]) assert.ok(noted(v, `${m} not scored: statements in USD, price in IDR`), m);
  assert.ok(v.price_to_book > 14000, "shown as reported");
  assert.deepEqual(lensSummary(g), ["mixed", 0, 6]);
  assert.deepEqual(ruleIds(g), [["revenue_growth", 1], ["earnings_growth", 1], ["eps_forward_growth", 1], ["revenue_cagr", -1], ["revenue_consistency", -1], ["net_income_cagr", -1]]);
  assert.ok(g.reading.rules[1].reason.endsWith("from a small or negative base"));
  assert.deepEqual(lensSummary(r), ["low", -4, 6]);
  assert.deepEqual(ruleIds(r), [["leverage", -1], ["liquidity", -1], ["losses", 0], ["beta", -1], ["volatility", 0], ["drawdown", -1]]);
  assert.ok(Math.abs(r.price.beta - 0.336) <= 0.002);
});

test("lens payloads keep the Rust field order and the reasons of the Rust rules", () => {
  const { v, g, r } = lensInputs("tlkm", "timeseries_tlkm.json");
  assert.deepEqual(Object.keys(v), ["symbol", "name", "currency", "financial_currency", "financial_company", "negative_equity", "price", "pe_trailing", "pe_forward", "peg", "price_to_book", "price_to_sales", "ev_to_ebitda", "ev_to_revenue", "dividend_yield_pct", "payout_ratio_pct", "target_mean", "analysts", "target_upside_pct", "reading", "sources"]);
  assert.deepEqual(Object.keys(g), ["symbol", "name", "statements_currency", "most_recent_quarter", "revenue_growth_pct", "earnings_growth_pct", "revenue_cagr_5y_pct", "eps_growth_3y_pct", "eps_ttm", "eps_forward", "eps_forward_growth_pct", "annual", "revenue_cagr_pct", "net_income_cagr_pct", "reading", "sources"]);
  assert.deepEqual(Object.keys(g.annual[0]), ["end_date", "revenue", "net_income", "revenue_change_pct", "net_income_change_pct"]);
  assert.deepEqual(Object.keys(r), ["symbol", "name", "currency", "financial_company", "negative_equity", "debt_to_equity", "current_ratio", "quick_ratio", "net_margin_pct", "eps_ttm", "price", "reading", "sources"]);
  assert.deepEqual(Object.keys(r.price), ["benchmark", "from", "to", "observations", "beta", "correlation", "volatility_pct", "benchmark_volatility_pct", "max_drawdown", "benchmark_max_drawdown_pct"]);
  assert.deepEqual(Object.keys(v.reading), ["label", "score", "inputs_available", "rules", "notes"]);
  assert.deepEqual(g.reading.rules.map((x) => x.reason), [
    "revenue +6.4% year on year in the latest quarter (0% to +10%)",
    "earnings +28.0% year on year in the latest quarter (over +10%)",
    "forward EPS 261.11 is +50.9% against trailing 173.07 (over +10%)",
    "revenue CAGR -0.1% over FY2022 to FY2025 (below 0%)",
    "revenue grew in 2 of the last 3 fiscal years",
    "net income CAGR -5.5% over FY2022 to FY2025 (below 0%)",
  ]);
});

test("lenses are null-safe: nothing reported gives insufficient_data and notes", () => {
  const f = blank();
  const v = an.valuationLens(f);
  assert.deepEqual(lensSummary(v), ["insufficient_data", 0, 0]);
  assert.ok(v.reading.notes.length >= 6);
  assert.equal(v.target_upside_pct, null);
  assert.deepEqual(lensSummary(an.growthLens("TEST", f, null)), ["insufficient_data", 0, 0]);
  assert.deepEqual(lensSummary(an.growthLens("TEST", null, null)), ["insufficient_data", 0, 0]);
  const r = an.riskLens("TEST", null, null, null);
  assert.deepEqual(lensSummary(r), ["insufficient_data", 0, 0]);
  assert.equal(r.price, null);
  assert.ok(noted(r, "fundamentals unavailable") && noted(r, "no daily candles"));
  // Without IHSG the price rules become notes and the stock keeps its own figures.
  const h = y.parseChart(JSON.parse(fixture("yahoo/chart_tlkm_1y_1d.json")), "1y", "1d");
  const own = an.riskLens("TLKM", null, h, null);
  assert.equal(own.price.observations, 0);
  assert.equal(own.price.beta, null);
  assert.ok(own.price.volatility_pct > 40);
  assert.ok(noted(own, "benchmark ^JKSE unavailable"));
});

test("lens rules on synthetic inputs match the Rust unit tests", () => {
  // negative_equity_skips_pb_and_leverage_and_adds_risk
  let f = blank();
  f.health.debt_to_equity = -2.89; f.valuation.price_to_book = -0.55; f.health.current_ratio = 1.2; f.profitability.net_margin_pct = -9.4;
  assert.ok(noted(an.valuationLens(f), "P/B: shareholders' equity is negative"));
  let r = an.riskLens("TEST", f, null, null);
  assert.deepEqual(ruleIds(r), [["negative_equity", 2], ["liquidity", 0], ["losses", 1]]);
  assert.equal(r.reading.label, "high");
  // analyst_target_needs_three_analysts
  const target = (t, n) => { const g = blank(); g.price = 100; g.analyst.target_mean = t; g.analyst.analysts = n; return an.valuationLens(g); };
  assert.deepEqual(ruleIds(target(120.01, 3)), [["analyst_target", 1]]);
  assert.deepEqual(ruleIds(target(120, 3)), [["analyst_target", 0]]);
  assert.deepEqual(ruleIds(target(89.9, 3)), [["analyst_target", -1]]);
  assert.ok(noted(target(150, 2), "from 2 analysts: fewer than 3, not scored"));
  // fiscal_years_give_cagr_consistency_and_turns
  const fin = (rows) => ({ symbol: "TEST", currency: "IDR", period: "annual", source: "t:annual", periods: rows.map(([yr, rev, ni]) => ({ end_date: `${yr}-12-31`, period: "annual", income: { total_revenue: rev, net_income: ni }, balance: {}, cash_flow: {} })) });
  const g = an.growthLens("TEST", blank(), fin([[2022, 100, 10], [2023, 110, 12], [2024, 121, 9], [2025, 133.1, 20]]));
  assert.equal(g.revenue_cagr_pct, 10);
  assert.equal(g.net_income_cagr_pct, 25.99);
  assert.deepEqual(ruleIds(g), [["revenue_cagr", 0], ["revenue_consistency", 1], ["net_income_cagr", 1]]);
  assert.equal(g.reading.label, "growing");
  assert.deepEqual([g.annual[0].revenue_change_pct, g.annual[1].revenue_change_pct, g.annual[2].net_income_change_pct], [null, 10, -25]);
  const turn = (a, b) => an.growthLens("TEST", blank(), fin([[2022, null, a], [2023, null, 1], [2025, null, b]]));
  assert.deepEqual(ruleIds(turn(5, -1)), [["net_income_cagr", -1]]);
  assert.deepEqual(ruleIds(turn(-5, 1)), [["net_income_cagr", 1]]);
  assert.ok(noted(turn(-5, -1), "losses in both FY2022 and FY2025"));
});

test("lens reasons round exact ties as Rust does (half to even)", () => {
  // rustc 1.9x: format!("{:.0}", 12.5) = "12", format!("{:.1}", 4.25) = "4.2",
  // format!("{:.1}", 4.75) = "4.8", format!("{:.2}", 0.125) = "0.12", format!("{:.1}", -0.25) = "-0.2".
  for (const [v, d, want] of [[12.5, 0, "12"], [13.5, 0, "14"], [4.25, 1, "4.2"], [4.75, 1, "4.8"], [0.125, 2, "0.12"], [0.375, 2, "0.38"], [-0.25, 1, "-0.2"], [-2.5, 0, "-2"], [1.005, 2, "1.00"], [49.16, 1, "49.2"], [-0.04, 1, "-0.0"], [0, 1, "0.0"]]) {
    assert.equal(an.fixed(v, d), want, `${v} to ${d}`);
  }
  // Price 8000, mean target 9000: an upside of exactly +12.5 %.
  const t = blank();
  t.price = 8000; t.analyst.target_mean = 9000; t.analyst.analysts = 3; t.dividend.yield_pct = 4.25;
  const v = an.valuationLens(t);
  assert.equal(v.target_upside_pct, 12.5);
  assert.deepEqual(v.reading.rules.map((x) => x.reason), [
    "dividend yield 4.2% is generous (4% or more)",
    "mean target of 3 analysts is +12% from the price (-10% to +20%)",
  ]);
});

test("lens rules on synthetic inputs match the Rust unit tests", () => {
  // negative_equity_skips_pb_and_leverage_and_adds_risk
  let f = blank();
  f.health.debt_to_equity = -2.89; f.valuation.price_to_book = -0.55; f.health.current_ratio = 1.2; f.profitability.net_margin_pct = -9.4;
  assert.ok(noted(an.valuationLens(f), "P/B: shareholders' equity is negative"));
  let r = an.riskLens("TEST", f, null, null);
  assert.deepEqual(ruleIds(r), [["negative_equity", 2], ["liquidity", 0], ["losses", 1]]);
  assert.equal(r.reading.label, "high");
  // analyst_target_needs_three_analysts
  const target = (t, n) => { const g = blank(); g.price = 100; g.analyst.target_mean = t; g.analyst.analysts = n; return an.valuationLens(g); };
  assert.deepEqual(ruleIds(target(120.01, 3)), [["analyst_target", 1]]);
  assert.deepEqual(ruleIds(target(120, 3)), [["analyst_target", 0]]);
  assert.deepEqual(ruleIds(target(89.9, 3)), [["analyst_target", -1]]);
  assert.ok(noted(target(150, 2), "from 2 analysts: fewer than 3, not scored"));
  // fiscal_years_give_cagr_consistency_and_turns
  const fin = (rows) => ({ symbol: "TEST", currency: "IDR", period: "annual", source: "t:annual", periods: rows.map(([yr, rev, ni]) => ({ end_date: `${yr}-12-31`, period: "annual", income: { total_revenue: rev, net_income: ni }, balance: {}, cash_flow: {} })) });
  const g = an.growthLens("TEST", blank(), fin([[2022, 100, 10], [2023, 110, 12], [2024, 121, 9], [2025, 133.1, 20]]));
  assert.equal(g.revenue_cagr_pct, 10);
  assert.equal(g.net_income_cagr_pct, 25.99);
  assert.deepEqual(ruleIds(g), [["revenue_cagr", 0], ["revenue_consistency", 1], ["net_income_cagr", 1]]);
  assert.equal(g.reading.label, "growing");
  assert.deepEqual([g.annual[0].revenue_change_pct, g.annual[1].revenue_change_pct, g.annual[2].net_income_change_pct], [null, 10, -25]);
  const turn = (a, b) => an.growthLens("TEST", blank(), fin([[2022, null, a], [2023, null, 1], [2025, null, b]]));
  assert.deepEqual(ruleIds(turn(5, -1)), [["net_income_cagr", -1]]);
  assert.deepEqual(ruleIds(turn(-5, 1)), [["net_income_cagr", 1]]);
  assert.ok(noted(turn(-5, -1), "losses in both FY2022 and FY2025"));
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
});

test("market hours in WIB", () => {
  assert.equal(marketIsOpen(new Date("2026-10-14T03:00:00Z")), true, "Wednesday 10:00 WIB");
  assert.equal(marketIsOpen(new Date("2026-10-14T12:00:00Z")), false, "Wednesday 19:00 WIB");
  assert.equal(marketIsOpen(new Date("2026-10-17T03:00:00Z")), false, "Saturday");
});
