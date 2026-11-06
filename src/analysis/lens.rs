//! Growth, valuation and risk lenses: three focused rule scores next to the
//! blended verdict in `fundamental.rs`. Every rule that ran is listed with its
//! points and a reason that carries the figure and the threshold; every rule
//! that could not run leaves a note. Nothing is estimated: a missing input
//! skips its rule. Thresholds are heuristics (documented in
//! docs/json-output.md); rule ids are stable so they can be tuned later.
//!
//! - Valuation points favour cheapness; growth points favour growth; risk
//!   points add risk.
//! - Negative equity is read the same way as the verdict reads it
//!   ([`Fundamentals::negative_equity`]); multiples that divide an IDR price
//!   by USD statements ([`Fundamentals::mixed_currency`]) are not scored.
//! - Price risk compares the stock with IHSG over the sessions both traded
//!   (beta, volatility, largest fall), so a market-wide crash year does not
//!   make every stock look risky.
//!
//! `web/src/analysis.js` ports these rules (same ids, points and reasons).

use chrono::Datelike;

use crate::analysis::returns;
use crate::model::{
    AnnualGrowth, Financials, Fundamentals, GrowthLabel, GrowthLens, History, LensRule, PriceRisk,
    Reading, RiskLabel, RiskLens, Symbol, ValuationLabel, ValuationLens,
};

/// The benchmark of the risk lens: IHSG.
pub const BENCHMARK: &str = "^JKSE";

/// Fewer rules than this give `insufficient_data`.
const MIN_RULES: usize = 3;

/// Matched daily returns the volatility and drawdown comparisons need
/// (`returns::volatility` needs 20).
const MIN_PRICE_RETURNS: usize = 20;

/// Two decimals; `+ 0.0` turns a rounded -0.0 into 0.0 (the web port does the same).
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0 + 0.0
}

#[derive(Default)]
struct Rules {
    rules: Vec<LensRule>,
    notes: Vec<String>,
}

impl Rules {
    fn add(&mut self, rule: &str, points: i32, reason: String) {
        self.rules.push(LensRule {
            rule: rule.to_string(),
            points,
            reason,
        });
    }

    fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    fn finish<L>(self, label: impl FnOnce(i32, &[LensRule]) -> L) -> Reading<L> {
        let score = self.rules.iter().map(|r| r.points).sum();
        Reading {
            label: label(score, &self.rules),
            score,
            inputs_available: self.rules.len(),
            rules: self.rules,
            notes: self.notes,
        }
    }
}

/// `"statements in USD, price in IDR"` when the two differ.
fn currencies(f: &Fundamentals) -> Option<String> {
    match (&f.financial_currency, &f.currency) {
        (Some(statements), Some(price)) if f.mixed_currency() => {
            Some(format!("statements in {statements}, price in {price}"))
        }
        _ => None,
    }
}

// ---- valuation ---------------------------------------------------------------

/// Multiples, yield and the analyst target.
pub fn valuation(f: &Fundamentals) -> ValuationLens {
    let mut r = Rules::default();
    let v = &f.valuation;
    let financial = f.is_financial_sector();
    let negative_equity = f.negative_equity();
    let mixed = currencies(f);

    match (v.pe_trailing, v.pe_forward) {
        (Some(pe), _) => {
            let (points, why) = if pe <= 0.0 {
                (0, "is not positive: it says nothing about value")
            } else if pe < 10.0 {
                (2, "is low (under 10)")
            } else if pe < 15.0 {
                (1, "is moderate (10 to 15)")
            } else if pe > 35.0 {
                (-2, "is high (over 35)")
            } else if pe > 25.0 {
                (-1, "is elevated (over 25)")
            } else {
                (0, "is in a normal range (15 to 25)")
            };
            r.add("pe_trailing", points, format!("trailing P/E {pe:.1} {why}"));
        }
        (None, Some(pf)) if pf > 0.0 => {
            let (points, why) = if pf < 10.0 {
                (1, "is low (under 10)")
            } else if pf > 25.0 {
                (-1, "is high (over 25)")
            } else {
                (0, "is in a normal range (10 to 25)")
            };
            r.add(
                "pe_forward",
                points,
                format!("forward P/E {pf:.1} {why}; no trailing P/E"),
            );
        }
        (None, Some(pf)) => r.note(format!(
            "P/E: no trailing P/E, and the forward P/E {pf:.1} is not positive"
        )),
        (None, None) => r.note("P/E: no trailing or forward P/E reported"),
    }

    match v.peg {
        Some(peg) if peg > 0.0 => {
            let (points, why) = if peg < 1.0 {
                (1, "is under 1: growth is cheap")
            } else if peg > 2.0 {
                (-1, "is over 2: paying up for growth")
            } else {
                (0, "is between 1 and 2")
            };
            r.add("peg", points, format!("PEG {peg:.2} {why}"));
        }
        Some(peg) => r.note(format!(
            "PEG {peg:.2} is not meaningful (shrinking or negative earnings)"
        )),
        None => r.note("PEG: not reported"),
    }

    match v.price_to_book {
        _ if negative_equity == Some(true) => {
            r.note("P/B: shareholders' equity is negative, so price-to-book says nothing")
        }
        None => r.note("P/B: not reported"),
        Some(pb) => match &mixed {
            Some(c) => r.note(format!("P/B {pb:.2} not scored: {c}")),
            None => {
                let (points, why) = if pb < 1.0 {
                    (1, "is below book value (under 1)")
                } else if pb > 4.0 {
                    (-1, "is rich (over 4)")
                } else {
                    (0, "is between 1 and 4")
                };
                r.add("price_to_book", points, format!("P/B {pb:.2} {why}"));
            }
        },
    }

    if financial {
        r.note("P/S and EV/EBITDA: not scored for a financial company");
    } else {
        match (v.price_to_sales, &mixed) {
            (None, _) => r.note("P/S: not reported"),
            (Some(ps), Some(c)) => r.note(format!("P/S {ps:.2} not scored: {c}")),
            (Some(ps), None) => {
                let (points, why) = if ps < 1.0 {
                    (1, "is low (under 1)")
                } else if ps > 5.0 {
                    (-1, "is high (over 5)")
                } else {
                    (0, "is between 1 and 5")
                };
                r.add("price_to_sales", points, format!("P/S {ps:.2} {why}"));
            }
        }
        match (v.ev_to_ebitda, &mixed) {
            (None, _) => r.note("EV/EBITDA: not reported"),
            (Some(ev), Some(c)) => r.note(format!("EV/EBITDA {ev:.1} not scored: {c}")),
            (Some(ev), None) if ev <= 0.0 => r.note(format!(
                "EV/EBITDA {ev:.1} is not meaningful (negative EBITDA or enterprise value)"
            )),
            (Some(ev), None) => {
                let (points, why) = if ev < 6.0 {
                    (1, "is low (under 6)")
                } else if ev > 15.0 {
                    (-1, "is high (over 15)")
                } else {
                    (0, "is between 6 and 15")
                };
                r.add("ev_to_ebitda", points, format!("EV/EBITDA {ev:.1} {why}"));
            }
        }
    }

    let d = &f.dividend;
    match d.yield_pct {
        Some(y) if y >= 4.0 => match d.payout_ratio_pct {
            Some(po) if po > 100.0 => r.add(
                "dividend_yield",
                0,
                format!(
                    "dividend yield {y:.1}% is high, but the payout ({po:.0}%) exceeds earnings"
                ),
            ),
            _ => r.add(
                "dividend_yield",
                1,
                format!("dividend yield {y:.1}% is generous (4% or more)"),
            ),
        },
        Some(y) => r.add(
            "dividend_yield",
            0,
            format!("dividend yield {y:.1}% is under 4%"),
        ),
        None => r.note("dividend yield: not reported"),
    }

    let a = &f.analyst;
    let upside = match (f.price, a.target_mean) {
        (Some(p), Some(t)) if p > 0.0 => Some(round2((t / p - 1.0) * 100.0)),
        _ => None,
    };
    match (upside, a.analysts) {
        (Some(up), Some(n)) if n >= 3 => {
            let (points, why) = if up > 20.0 {
                (1, "over +20%")
            } else if up < -10.0 {
                (-1, "under -10%")
            } else {
                (0, "-10% to +20%")
            };
            r.add(
                "analyst_target",
                points,
                format!("mean target of {n} analysts is {up:+.0}% from the price ({why})"),
            );
        }
        (Some(up), n) => {
            let who = match n {
                Some(1) => "1 analyst".to_string(),
                Some(n) => format!("{n} analysts"),
                None => "an unknown number of analysts".to_string(),
            };
            r.note(format!(
                "analyst target {up:+.0}% from the price, from {who}: fewer than 3, not scored"
            ));
        }
        (None, _) => r.note("analyst target: no mean target or price"),
    }

    let reading = r.finish(|score, rules| {
        if rules.len() < MIN_RULES {
            ValuationLabel::InsufficientData
        } else if score >= 3 {
            ValuationLabel::Cheap
        } else if score <= -3 {
            ValuationLabel::Expensive
        } else {
            ValuationLabel::Fair
        }
    });
    ValuationLens {
        symbol: f.symbol.clone(),
        name: f.name.clone(),
        currency: f.currency.clone(),
        financial_currency: f.financial_currency.clone(),
        financial_company: financial,
        negative_equity,
        price: f.price,
        pe_trailing: v.pe_trailing,
        pe_forward: v.pe_forward,
        peg: v.peg,
        price_to_book: v.price_to_book,
        price_to_sales: v.price_to_sales,
        ev_to_ebitda: v.ev_to_ebitda,
        ev_to_revenue: v.ev_to_revenue,
        dividend_yield_pct: d.yield_pct,
        payout_ratio_pct: d.payout_ratio_pct,
        target_mean: a.target_mean,
        analysts: a.analysts,
        target_upside_pct: upside,
        reading,
        sources: f.sources.clone(),
    }
}

// ---- growth --------------------------------------------------------------------

/// `(points, band)` for a growth rate scored against `hi` and `lo`.
fn growth_band(v: f64, hi: f64, lo: f64) -> (i32, String) {
    let sign = |x: f64| {
        if x > 0.0 {
            format!("+{x}")
        } else {
            format!("{x}")
        }
    };
    if v > hi {
        (1, format!("over {}%", sign(hi)))
    } else if v < lo {
        (-1, format!("below {}%", sign(lo)))
    } else {
        (0, format!("{}% to {}%", sign(lo), sign(hi)))
    }
}

/// Compound annual growth, percent, when both ends are above zero.
fn cagr(first: f64, last: f64, years: i32) -> Option<f64> {
    (first > 0.0 && last > 0.0 && years > 0)
        .then(|| round2(((last / first).powf(1.0 / years as f64) - 1.0) * 100.0))
}

fn change_pct(prev: Option<f64>, cur: Option<f64>) -> Option<f64> {
    match (prev, cur) {
        (Some(p), Some(c)) if p > 0.0 => Some(round2((c / p - 1.0) * 100.0)),
        _ => None,
    }
}

/// Latest quarter, forward EPS against trailing, and revenue and net income
/// across the annual statements. Either input may be missing (the service
/// warns why); the reading then has fewer rules.
pub fn growth(
    symbol: &Symbol,
    f: Option<&Fundamentals>,
    annual: Option<&Financials>,
) -> GrowthLens {
    let mut r = Rules::default();
    let g = f.map(|f| &f.growth);
    let ps = f.map(|f| &f.per_share);

    for (rule, what, v) in [
        (
            "revenue_growth",
            "revenue",
            g.and_then(|g| g.revenue_growth_pct),
        ),
        (
            "earnings_growth",
            "earnings",
            g.and_then(|g| g.earnings_growth_pct),
        ),
    ] {
        match v {
            Some(v) => {
                let (points, band) = growth_band(v, 10.0, 0.0);
                let base = if v.abs() > 100.0 {
                    ", from a small or negative base"
                } else {
                    ""
                };
                r.add(
                    rule,
                    points,
                    format!("{what} {v:+.1}% year on year in the latest quarter ({band}){base}"),
                );
            }
            None => r.note(format!("{what} growth in the latest quarter: not reported")),
        }
    }

    let eps_ttm = ps.and_then(|p| p.eps_ttm);
    let eps_forward = ps.and_then(|p| p.eps_forward);
    let eps_forward_growth_pct = match (eps_ttm, eps_forward) {
        (Some(t), Some(fw)) if t > 0.0 && fw > 0.0 => Some(round2((fw / t - 1.0) * 100.0)),
        _ => None,
    };
    match (eps_ttm, eps_forward, eps_forward_growth_pct) {
        (Some(t), Some(fw), Some(fg)) => {
            let (points, band) = growth_band(fg, 10.0, -10.0);
            r.add(
                "eps_forward_growth",
                points,
                format!("forward EPS {fw:.2} is {fg:+.1}% against trailing {t:.2} ({band})"),
            );
        }
        (Some(t), Some(fw), None) => r.note(format!(
            "forward vs trailing EPS: needs both above zero (trailing {t:.2}, forward {fw:.2})"
        )),
        _ => r.note("forward vs trailing EPS: not reported"),
    }

    let periods = annual.map(|a| a.periods.as_slice()).unwrap_or_default();
    let mut years: Vec<AnnualGrowth> = Vec::with_capacity(periods.len());
    for (i, p) in periods.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| &periods[j].income);
        years.push(AnnualGrowth {
            end_date: p.end_date,
            revenue: p.income.total_revenue,
            net_income: p.income.net_income,
            revenue_change_pct: change_pct(
                prev.and_then(|x| x.total_revenue),
                p.income.total_revenue,
            ),
            net_income_change_pct: change_pct(prev.and_then(|x| x.net_income), p.income.net_income),
        });
    }
    let revenue: Vec<(i32, f64)> = years
        .iter()
        .filter_map(|y| y.revenue.map(|v| (y.end_date.year(), v)))
        .collect();
    let net_income: Vec<(i32, f64)> = years
        .iter()
        .filter_map(|y| y.net_income.map(|v| (y.end_date.year(), v)))
        .collect();
    let span = |s: &[(i32, f64)]| (s[0], s[s.len() - 1]);

    let mut revenue_cagr_pct = None;
    if revenue.len() >= 3 {
        let ((a, first), (b, last)) = span(&revenue);
        revenue_cagr_pct = cagr(first, last, b - a);
        match revenue_cagr_pct {
            Some(c) => {
                let (points, band) = growth_band(c, 10.0, 0.0);
                r.add(
                    "revenue_cagr",
                    points,
                    format!("revenue CAGR {c:+.1}% over FY{a} to FY{b} ({band})"),
                );
            }
            None => r.note(format!(
                "revenue CAGR: needs revenue above zero in FY{a} and FY{b}"
            )),
        }
        let k = revenue.len() - 1;
        let ups = revenue.windows(2).filter(|w| w[1].1 > w[0].1).count();
        let (points, reason) = if ups == k {
            (
                1,
                format!("revenue grew in each of the last {k} fiscal years"),
            )
        } else if ups * 2 < k {
            (
                -1,
                format!("revenue grew in only {ups} of the last {k} fiscal years (under half)"),
            )
        } else {
            (
                0,
                format!("revenue grew in {ups} of the last {k} fiscal years"),
            )
        };
        r.add("revenue_consistency", points, reason);
    } else {
        // The source's own rate, under its own rule id (not `revenue_cagr`).
        match g.and_then(|g| g.revenue_cagr_5y_pct) {
            Some(c) => {
                let (points, band) = growth_band(c, 10.0, 0.0);
                r.add(
                    "revenue_cagr_5y",
                    points,
                    format!("revenue 5-year growth {c:+.1}% a year, as reported ({band})"),
                );
            }
            None => r.note(
                "revenue CAGR: fewer than three fiscal years and no 5-year figure from the source",
            ),
        }
        r.note("revenue consistency: fewer than three fiscal years");
    }

    let mut net_income_cagr_pct = None;
    if net_income.len() >= 3 {
        let ((a, first), (b, last)) = span(&net_income);
        net_income_cagr_pct = cagr(first, last, b - a);
        match net_income_cagr_pct {
            Some(c) => {
                let (points, band) = growth_band(c, 10.0, 0.0);
                r.add(
                    "net_income_cagr",
                    points,
                    format!("net income CAGR {c:+.1}% over FY{a} to FY{b} ({band})"),
                );
            }
            None if first > 0.0 => r.add(
                "net_income_cagr",
                -1,
                format!("net income turned from a profit in FY{a} to a loss in FY{b}"),
            ),
            None if last > 0.0 => r.add(
                "net_income_cagr",
                1,
                format!("net income turned from a loss in FY{a} to a profit in FY{b}"),
            ),
            None => r.note(format!("net income CAGR: losses in both FY{a} and FY{b}")),
        }
    } else {
        // EPS is another metric than net income: its own rule id.
        match g.and_then(|g| g.eps_growth_3y_pct) {
            Some(c) => {
                let (points, band) = growth_band(c, 10.0, 0.0);
                r.add(
                    "eps_growth_3y",
                    points,
                    format!("EPS 3-year growth {c:+.1}% a year, as reported ({band})"),
                );
            }
            None => r.note(
                "net income CAGR: fewer than three fiscal years and no 3-year EPS figure from the source",
            ),
        }
    }

    let reading = r.finish(|score, rules| {
        if rules.len() < MIN_RULES {
            GrowthLabel::InsufficientData
        } else if score >= 2 {
            GrowthLabel::Growing
        } else if score <= -2 {
            GrowthLabel::Shrinking
        } else if rules.iter().any(|x| x.points > 0) && rules.iter().any(|x| x.points < 0) {
            GrowthLabel::Mixed
        } else {
            GrowthLabel::Stable
        }
    });
    let mut sources: Vec<String> = f.map(|f| f.sources.clone()).unwrap_or_default();
    if let Some(a) = annual {
        if !sources.contains(&a.source) {
            sources.push(a.source.clone());
        }
    }
    GrowthLens {
        symbol: symbol.clone(),
        name: f.and_then(|f| f.name.clone()),
        statements_currency: annual.and_then(|a| a.currency.clone()),
        most_recent_quarter: f.and_then(|f| f.most_recent_quarter),
        revenue_growth_pct: g.and_then(|g| g.revenue_growth_pct),
        earnings_growth_pct: g.and_then(|g| g.earnings_growth_pct),
        revenue_cagr_5y_pct: g.and_then(|g| g.revenue_cagr_5y_pct),
        eps_growth_3y_pct: g.and_then(|g| g.eps_growth_3y_pct),
        eps_ttm,
        eps_forward,
        eps_forward_growth_pct,
        annual: years,
        revenue_cagr_pct,
        net_income_cagr_pct,
        reading,
        sources,
    }
}

// ---- risk ------------------------------------------------------------------------

/// Balance sheet, losses and a year of daily closes against IHSG. `bench` is
/// the IHSG history; without it the price rules become notes and
/// [`PriceRisk`] carries the stock's own statistics.
pub fn risk(
    symbol: &Symbol,
    f: Option<&Fundamentals>,
    stock: Option<&History>,
    bench: Option<&History>,
) -> RiskLens {
    let mut r = Rules::default();
    let negative_equity = f.and_then(Fundamentals::negative_equity);
    let financial = f.is_some_and(Fundamentals::is_financial_sector);

    match f {
        None => r.note("balance sheet and earnings: fundamentals unavailable"),
        Some(f) => {
            if negative_equity == Some(true) {
                r.add(
                    "negative_equity",
                    2,
                    "shareholders' equity is negative (book value below zero): liabilities exceed assets"
                        .into(),
                );
            }
            if financial {
                r.note("leverage and liquidity: not scored for a financial company");
            } else {
                match f.health.debt_to_equity {
                    _ if negative_equity == Some(true) => {
                        r.note("leverage: debt/equity says nothing with negative equity")
                    }
                    Some(de) => {
                        let (points, why) = if de > 2.0 {
                            (1, "over 2x")
                        } else if de < 0.5 {
                            (-1, "under 0.5x")
                        } else {
                            (0, "0.5x to 2x")
                        };
                        r.add(
                            "leverage",
                            points,
                            format!("debt is {de:.2}x equity ({why})"),
                        );
                    }
                    None => r.note("leverage: debt/equity not reported"),
                }
                match f.health.current_ratio {
                    Some(cr) => {
                        let (points, why) = if cr < 1.0 {
                            (1, "is under 1")
                        } else if cr >= 1.5 {
                            (-1, "is 1.5 or more")
                        } else {
                            (0, "is between 1 and 1.5")
                        };
                        r.add("liquidity", points, format!("current ratio {cr:.2} {why}"));
                    }
                    None => r.note("liquidity: current ratio not reported"),
                }
            }
            let nm = f.profitability.net_margin_pct;
            let eps = f.per_share.eps_ttm;
            let detail: Vec<String> = [
                nm.map(|m| format!("net margin {m:.1}%")),
                eps.map(|e| format!("EPS {e:.2}")),
            ]
            .into_iter()
            .flatten()
            .collect();
            if detail.is_empty() {
                r.note("losses: no net margin or EPS reported");
            } else if nm.is_some_and(|m| m < 0.0) || eps.is_some_and(|e| e < 0.0) {
                r.add(
                    "losses",
                    1,
                    format!(
                        "loss-making over the trailing twelve months ({})",
                        detail.join(", ")
                    ),
                );
            } else {
                r.add(
                    "losses",
                    0,
                    format!(
                        "profitable over the trailing twelve months ({})",
                        detail.join(", ")
                    ),
                );
            }
        }
    }

    let own = stock
        .map(|h| returns::series(&h.candles))
        .unwrap_or_default();
    let bench_symbol = bench
        .map(|b| b.symbol.clone())
        .unwrap_or_else(|| Symbol::parse(BENCHMARK).expect("valid benchmark"));
    let price = if own.is_empty() {
        r.note("price risk: no daily candles for the stock");
        None
    } else {
        let b = bench
            .map(|b| returns::series(&b.candles))
            .unwrap_or_default();
        let (cs, cb) = returns::common(&own, &b);
        if cs.len() < 2 {
            r.note(if bench.is_none() {
                format!("beta, volatility and drawdown against IHSG: benchmark {bench_symbol} unavailable")
            } else {
                format!("beta, volatility and drawdown against IHSG: no sessions in common with {bench_symbol}")
            });
            Some(PriceRisk {
                benchmark: bench_symbol,
                from: own[0].0,
                to: own[own.len() - 1].0,
                observations: 0,
                zero_returns: None,
                beta: None,
                correlation: None,
                volatility_pct: returns::volatility(&own),
                benchmark_volatility_pct: None,
                max_drawdown: returns::max_drawdown(&own),
                benchmark_max_drawdown_pct: None,
            })
        } else {
            Some(price_rules(&mut r, bench_symbol, &cs, &cb))
        }
    };

    let reading = r.finish(|score, rules| {
        if rules.len() < MIN_RULES {
            RiskLabel::InsufficientData
        } else if score >= 3 {
            RiskLabel::High
        } else if score <= -2 {
            RiskLabel::Low
        } else {
            RiskLabel::Moderate
        }
    });
    RiskLens {
        symbol: symbol.clone(),
        name: f
            .and_then(|f| f.name.clone())
            .or_else(|| stock.and_then(|h| h.name.clone())),
        currency: f
            .and_then(|f| f.currency.clone())
            .or_else(|| stock.and_then(|h| h.currency.clone())),
        financial_company: financial,
        negative_equity,
        debt_to_equity: f.and_then(|f| f.health.debt_to_equity),
        current_ratio: f.and_then(|f| f.health.current_ratio),
        quick_ratio: f.and_then(|f| f.health.quick_ratio),
        net_margin_pct: f.and_then(|f| f.profitability.net_margin_pct),
        eps_ttm: f.and_then(|f| f.per_share.eps_ttm),
        price,
        reading,
        sources: f.map(|f| f.sources.clone()).unwrap_or_default(),
    }
}

/// Beta, relative volatility and relative drawdown over the shared sessions.
/// A stock whose close did not move on more than half of them (suspended,
/// stuck at the floor price, barely traded) would look calm on all three, so
/// it scores one `illiquidity` point instead and the three become notes.
fn price_rules(
    r: &mut Rules,
    benchmark: Symbol,
    cs: &[(chrono::NaiveDate, f64)],
    cb: &[(chrono::NaiveDate, f64)],
) -> PriceRisk {
    let stats = returns::beta(cs, cb);
    let n = stats.observations;
    // Over the same pairs as the beta: the stock's close equals the one before.
    let zero = cs
        .windows(2)
        .zip(cb.windows(2))
        .filter(|(s, b)| s[0].1 > 0.0 && b[0].1 > 0.0 && s[1].1 == s[0].1)
        .count();
    let vs = returns::volatility(cs);
    let vb = returns::volatility(cb);
    let ds = returns::max_drawdown(cs);
    let db = returns::max_drawdown(cb).map(|d| d.pct);

    if n >= MIN_PRICE_RETURNS && zero * 2 > n {
        r.add(
            "illiquidity",
            1,
            format!(
                "{zero} of {n} daily returns are zero (more than half): the stock was suspended or barely traded"
            ),
        );
        r.note(format!(
            "beta, volatility and drawdown: not scored, {zero} of {n} daily returns are zero"
        ));
    } else {
        rate_beta(r, &stats);
        match (vs, vb) {
            (Some(s), Some(b)) if b > 0.0 => {
                let rel = s / b;
                let (points, why) = if rel > 1.5 {
                    (1, "over 1.5x")
                } else if rel < 0.8 {
                    (-1, "under 0.8x")
                } else {
                    (0, "0.8x to 1.5x")
                };
                r.add(
                    "volatility",
                    points,
                    format!("volatility {s:.1}% a year is {rel:.2}x IHSG's {b:.1}% ({why})"),
                );
            }
            _ => r.note(format!(
                "volatility against IHSG: fewer than {MIN_PRICE_RETURNS} matched daily returns"
            )),
        }
        if cs.len() > MIN_PRICE_RETURNS {
            // No fall over the window is a 0 % drawdown.
            let s = ds.as_ref().map_or(0.0, |d| d.pct);
            let b = db.unwrap_or(0.0);
            let gap = s - b;
            let (points, why) = if gap <= -15.0 {
                (1, "15 points or more deeper")
            } else if gap >= 15.0 {
                (-1, "15 points or more shallower")
            } else {
                (0, "within 15 points")
            };
            r.add(
                "drawdown",
                points,
                format!(
                    "largest fall {s:.1}% against IHSG's {b:.1}% over the same sessions ({why})"
                ),
            );
        } else {
            r.note(format!(
                "drawdown against IHSG: fewer than {MIN_PRICE_RETURNS} matched daily returns"
            ));
        }
    }

    PriceRisk {
        benchmark,
        from: cs[0].0,
        to: cs[cs.len() - 1].0,
        observations: n,
        zero_returns: Some(zero),
        beta: stats.beta,
        correlation: stats.correlation,
        volatility_pct: vs,
        benchmark_volatility_pct: vb,
        max_drawdown: ds,
        benchmark_max_drawdown_pct: db,
    }
}

fn rate_beta(r: &mut Rules, stats: &returns::BetaStats) {
    let n = stats.observations;
    match stats.beta {
        Some(b) => {
            let (points, why) = if b > 1.3 {
                (1, "over 1.3")
            } else if b < 0.7 {
                (-1, "under 0.7")
            } else {
                (0, "0.7 to 1.3")
            };
            r.add(
                "beta",
                points,
                format!("beta {b:.2} against IHSG over {n} daily returns ({why})"),
            );
        }
        None if n < returns::MIN_BETA_OBSERVATIONS => r.note(format!(
            "beta: {n} matched daily returns with IHSG, needs {}",
            returns::MIN_BETA_OBSERVATIONS
        )),
        None => r.note("beta: IHSG did not move over the window"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, Interval, Period};
    use chrono::{Duration, NaiveDate, TimeZone};

    fn sym(s: &str) -> Symbol {
        Symbol::parse(s).unwrap()
    }

    fn base() -> Fundamentals {
        Fundamentals::empty(sym("TEST"), "t")
    }

    fn ids<L>(r: &Reading<L>) -> Vec<(&str, i32)> {
        r.rules
            .iter()
            .map(|x| (x.rule.as_str(), x.points))
            .collect()
    }

    fn points<L>(r: &Reading<L>, rule: &str) -> Option<i32> {
        r.rules.iter().find(|x| x.rule == rule).map(|x| x.points)
    }

    fn has_note<L>(r: &Reading<L>, needle: &str) -> bool {
        r.notes.iter().any(|n| n.contains(needle))
    }

    #[test]
    fn empty_fundamentals_are_insufficient_everywhere() {
        let f = base();
        let v = valuation(&f);
        assert_eq!(v.reading.label, ValuationLabel::InsufficientData);
        assert_eq!(v.reading.inputs_available, 0);
        assert!(v.reading.rules.is_empty());
        assert!(v.reading.notes.len() >= 6, "{:?}", v.reading.notes);
        assert_eq!(v.target_upside_pct, None);
        assert_eq!(v.negative_equity, None);

        let g = growth(&f.symbol, Some(&f), None);
        assert_eq!(g.reading.label, GrowthLabel::InsufficientData);
        assert!(g.annual.is_empty());
        assert_eq!(g.revenue_cagr_pct, None);
        let g = growth(&f.symbol, None, None);
        assert_eq!(g.reading.label, GrowthLabel::InsufficientData);
        assert!(g.sources.is_empty());

        let r = risk(&f.symbol, Some(&f), None, None);
        assert_eq!(r.reading.label, RiskLabel::InsufficientData);
        assert_eq!(r.price, None);
        assert!(has_note(&r.reading, "no daily candles"));
        let r = risk(&f.symbol, None, None, None);
        assert_eq!(r.reading.label, RiskLabel::InsufficientData);
        assert!(has_note(&r.reading, "fundamentals unavailable"));
    }

    #[test]
    fn trailing_pe_bands() {
        for (pe, pts) in [
            (-3.0, 0),
            (0.0, 0),
            (9.99, 2),
            (10.0, 1),
            (14.99, 1),
            (15.0, 0),
            (25.0, 0),
            (25.01, -1),
            (35.0, -1),
            (35.01, -2),
        ] {
            let mut f = base();
            f.valuation.pe_trailing = Some(pe);
            f.valuation.pe_forward = Some(5.0);
            let v = valuation(&f);
            assert_eq!(points(&v.reading, "pe_trailing"), Some(pts), "{pe}");
            assert_eq!(points(&v.reading, "pe_forward"), None, "trailing wins");
        }
    }

    #[test]
    fn forward_pe_only_without_a_trailing_one() {
        for (pf, pts) in [(9.9, 1), (10.0, 0), (25.0, 0), (25.1, -1)] {
            let mut f = base();
            f.valuation.pe_forward = Some(pf);
            assert_eq!(points(&valuation(&f).reading, "pe_forward"), Some(pts));
        }
        let mut f = base();
        f.valuation.pe_forward = Some(-4.0);
        let v = valuation(&f);
        assert_eq!(points(&v.reading, "pe_forward"), None);
        assert!(has_note(&v.reading, "forward P/E -4.0 is not positive"));
    }

    #[test]
    fn multiple_bands() {
        let run = |set: &dyn Fn(&mut Fundamentals)| {
            let mut f = base();
            set(&mut f);
            valuation(&f).reading
        };
        for (peg, pts) in [
            (0.99, Some(1)),
            (1.0, Some(0)),
            (2.0, Some(0)),
            (2.01, Some(-1)),
            (0.0, None),
            (-1.0, None),
        ] {
            assert_eq!(
                points(&run(&|f| f.valuation.peg = Some(peg)), "peg"),
                pts,
                "peg {peg}"
            );
        }
        for (pb, pts) in [(0.99, 1), (1.0, 0), (4.0, 0), (4.01, -1)] {
            assert_eq!(
                points(
                    &run(&|f| f.valuation.price_to_book = Some(pb)),
                    "price_to_book"
                ),
                Some(pts),
                "pb {pb}"
            );
        }
        for (ps, pts) in [(0.99, 1), (1.0, 0), (5.0, 0), (5.01, -1)] {
            assert_eq!(
                points(
                    &run(&|f| f.valuation.price_to_sales = Some(ps)),
                    "price_to_sales"
                ),
                Some(pts),
                "ps {ps}"
            );
        }
        for (ev, pts) in [
            (5.99, Some(1)),
            (6.0, Some(0)),
            (15.0, Some(0)),
            (15.01, Some(-1)),
            (0.0, None),
            (-3.0, None),
        ] {
            assert_eq!(
                points(
                    &run(&|f| f.valuation.ev_to_ebitda = Some(ev)),
                    "ev_to_ebitda"
                ),
                pts,
                "ev {ev}"
            );
        }
        let r = run(&|f| f.valuation.ev_to_ebitda = Some(-3.0));
        assert!(has_note(&r, "not meaningful (negative EBITDA"));
    }

    #[test]
    fn dividend_yield_and_payout() {
        let run = |y: f64, po: Option<f64>| {
            let mut f = base();
            f.dividend.yield_pct = Some(y);
            f.dividend.payout_ratio_pct = po;
            valuation(&f).reading
        };
        assert_eq!(points(&run(3.99, None), "dividend_yield"), Some(0));
        assert_eq!(points(&run(4.0, None), "dividend_yield"), Some(1));
        assert_eq!(points(&run(6.0, Some(100.0)), "dividend_yield"), Some(1));
        let r = run(10.0, Some(125.0));
        assert_eq!(points(&r, "dividend_yield"), Some(0));
        assert_eq!(
            r.rules[0].reason,
            "dividend yield 10.0% is high, but the payout (125%) exceeds earnings"
        );
    }

    #[test]
    fn analyst_target_needs_three_analysts() {
        let run = |target: f64, n: Option<u32>| {
            let mut f = base();
            f.price = Some(100.0);
            f.analyst.target_mean = Some(target);
            f.analyst.analysts = n;
            valuation(&f)
        };
        assert_eq!(
            points(&run(120.01, Some(3)).reading, "analyst_target"),
            Some(1)
        );
        assert_eq!(
            points(&run(120.0, Some(3)).reading, "analyst_target"),
            Some(0)
        );
        assert_eq!(
            points(&run(90.0, Some(3)).reading, "analyst_target"),
            Some(0)
        );
        assert_eq!(
            points(&run(89.9, Some(3)).reading, "analyst_target"),
            Some(-1)
        );
        let v = run(150.0, Some(2));
        assert_eq!(points(&v.reading, "analyst_target"), None);
        assert_eq!(v.target_upside_pct, Some(50.0), "still shown");
        assert!(has_note(
            &v.reading,
            "from 2 analysts: fewer than 3, not scored"
        ));
        assert!(has_note(&run(150.0, Some(1)).reading, "from 1 analyst:"));
        assert!(has_note(
            &run(150.0, None).reading,
            "unknown number of analysts"
        ));
    }

    /// Exact ties round half to even; `web/test/analysis.test.mjs` pins the
    /// JS port to the same strings.
    #[test]
    fn reasons_round_exact_ties_to_even() {
        let mut f = base();
        f.price = Some(8000.0);
        f.analyst.target_mean = Some(9000.0);
        f.analyst.analysts = Some(3);
        f.dividend.yield_pct = Some(4.25);
        let v = valuation(&f);
        assert_eq!(v.target_upside_pct, Some(12.5));
        let reasons: Vec<&str> = v.reading.rules.iter().map(|x| x.reason.as_str()).collect();
        assert_eq!(
            reasons,
            [
                "dividend yield 4.2% is generous (4% or more)",
                "mean target of 3 analysts is +12% from the price (-10% to +20%)"
            ]
        );
    }

    #[test]
    fn negative_equity_skips_pb_and_leverage_and_adds_risk() {
        // MSN-style negative ratios (the placeholder is dropped at parse time,
        // but a genuinely negative figure can still arrive).
        let mut f = base();
        f.health.debt_to_equity = Some(-2.89);
        f.valuation.price_to_book = Some(-0.55);
        f.health.current_ratio = Some(1.2);
        f.profitability.net_margin_pct = Some(-9.4);
        let v = valuation(&f);
        assert_eq!(v.negative_equity, Some(true));
        assert_eq!(points(&v.reading, "price_to_book"), None);
        assert!(has_note(
            &v.reading,
            "P/B: shareholders' equity is negative"
        ));
        let r = risk(&f.symbol, Some(&f), None, None);
        assert_eq!(
            ids(&r.reading),
            [("negative_equity", 2), ("liquidity", 0), ("losses", 1)]
        );
        assert!(has_note(&r.reading, "leverage: debt/equity says nothing"));
        assert_eq!(r.reading.label, RiskLabel::High);
        // Banks too: negative equity is scored in any sector.
        f.sector = Some("Financial Services".into());
        let r = risk(&f.symbol, Some(&f), None, None);
        assert_eq!(points(&r.reading, "negative_equity"), Some(2));
        assert_eq!(points(&r.reading, "liquidity"), None);
    }

    #[test]
    fn mixed_currency_skips_book_sales_and_ev_multiples() {
        let mut f = base();
        f.currency = Some("IDR".into());
        f.financial_currency = Some("USD".into());
        f.valuation.pe_trailing = Some(7.46);
        f.valuation.price_to_book = Some(14103.31);
        f.valuation.price_to_sales = Some(13807.1);
        f.valuation.ev_to_ebitda = Some(91203.72);
        let v = valuation(&f);
        assert_eq!(ids(&v.reading), [("pe_trailing", 2)]);
        assert!(has_note(
            &v.reading,
            "P/B 14103.31 not scored: statements in USD, price in IDR"
        ));
        assert!(has_note(&v.reading, "P/S 13807.10 not scored"));
        assert!(has_note(&v.reading, "EV/EBITDA 91203.7 not scored"));
        assert_eq!(
            v.price_to_book,
            Some(14103.31),
            "the reported value is kept"
        );
    }

    #[test]
    fn banks_skip_sales_ev_leverage_and_liquidity() {
        let mut f = base();
        f.sector = Some("Financial Services".into());
        f.valuation.price_to_sales = Some(0.5);
        f.valuation.ev_to_ebitda = Some(3.0);
        f.health.debt_to_equity = Some(0.1);
        f.health.current_ratio = Some(3.0);
        let v = valuation(&f);
        assert!(v.financial_company);
        assert_eq!(points(&v.reading, "price_to_sales"), None);
        assert_eq!(points(&v.reading, "ev_to_ebitda"), None);
        assert!(has_note(&v.reading, "not scored for a financial company"));
        let r = risk(&f.symbol, Some(&f), None, None);
        assert!(r.reading.rules.is_empty());
        assert!(has_note(
            &r.reading,
            "leverage and liquidity: not scored for a financial company"
        ));
    }

    #[test]
    fn leverage_liquidity_and_losses_bands() {
        let run = |de: f64, cr: f64, eps: f64| {
            let mut f = base();
            f.health.debt_to_equity = Some(de);
            f.health.current_ratio = Some(cr);
            f.per_share.eps_ttm = Some(eps);
            risk(&f.symbol, Some(&f), None, None).reading
        };
        assert_eq!(
            ids(&run(2.01, 0.99, -1.0)),
            [("leverage", 1), ("liquidity", 1), ("losses", 1)]
        );
        assert_eq!(
            ids(&run(2.0, 1.0, 0.0)),
            [("leverage", 0), ("liquidity", 0), ("losses", 0)]
        );
        assert_eq!(
            ids(&run(0.5, 1.49, 5.0)),
            [("leverage", 0), ("liquidity", 0), ("losses", 0)]
        );
        assert_eq!(
            ids(&run(0.49, 1.5, 5.0)),
            [("leverage", -1), ("liquidity", -1), ("losses", 0)]
        );
        assert_eq!(run(0.49, 1.5, 5.0).label, RiskLabel::Low);
        assert_eq!(run(2.01, 0.99, -1.0).label, RiskLabel::High);
        assert_eq!(run(2.0, 1.0, 0.0).label, RiskLabel::Moderate);
    }

    fn fin(rows: &[(i32, Option<f64>, Option<f64>)]) -> Financials {
        use crate::model::{FinancialPeriod, StatementPeriod};
        Financials {
            symbol: sym("TEST"),
            currency: Some("IDR".into()),
            period: StatementPeriod::Annual,
            periods: rows
                .iter()
                .map(|&(y, rev, ni)| {
                    let mut p = FinancialPeriod::new(
                        NaiveDate::from_ymd_opt(y, 12, 31).unwrap(),
                        StatementPeriod::Annual,
                    );
                    p.income.total_revenue = rev;
                    p.income.net_income = ni;
                    p
                })
                .collect(),
            source: "t:annual".into(),
        }
    }

    #[test]
    fn latest_quarter_and_forward_eps_bands() {
        let run = |rev: f64, et: f64, ef: f64| {
            let mut f = base();
            f.growth.revenue_growth_pct = Some(rev);
            f.per_share.eps_ttm = Some(et);
            f.per_share.eps_forward = Some(ef);
            growth(&f.symbol, Some(&f), None)
        };
        assert_eq!(
            points(&run(10.01, 100.0, 110.01).reading, "revenue_growth"),
            Some(1)
        );
        assert_eq!(
            points(&run(10.0, 100.0, 110.0).reading, "revenue_growth"),
            Some(0)
        );
        assert_eq!(
            points(&run(0.0, 100.0, 90.0).reading, "revenue_growth"),
            Some(0)
        );
        assert_eq!(
            points(&run(-0.1, 100.0, 89.0).reading, "revenue_growth"),
            Some(-1)
        );
        assert_eq!(
            points(&run(5.0, 100.0, 111.0).reading, "eps_forward_growth"),
            Some(1)
        );
        assert_eq!(
            points(&run(5.0, 100.0, 110.0).reading, "eps_forward_growth"),
            Some(0)
        );
        assert_eq!(
            points(&run(5.0, 100.0, 90.0).reading, "eps_forward_growth"),
            Some(0)
        );
        assert_eq!(
            points(&run(5.0, 100.0, 89.0).reading, "eps_forward_growth"),
            Some(-1)
        );
        let g = run(150.0, -5.0, 3.0);
        assert!(g.reading.rules[0]
            .reason
            .ends_with("from a small or negative base"));
        assert_eq!(g.eps_forward_growth_pct, None);
        assert!(has_note(
            &g.reading,
            "needs both above zero (trailing -5.00, forward 3.00)"
        ));
    }

    #[test]
    fn fiscal_years_give_cagr_consistency_and_turns() {
        let f = base();
        // 100 -> 133.1 over three years is 10 % a year: not over 10.
        let a = fin(&[
            (2022, Some(100.0), Some(10.0)),
            (2023, Some(110.0), Some(12.0)),
            (2024, Some(121.0), Some(9.0)),
            (2025, Some(133.1), Some(20.0)),
        ]);
        let g = growth(&f.symbol, Some(&f), Some(&a));
        assert_eq!(g.revenue_cagr_pct, Some(10.0));
        assert_eq!(g.net_income_cagr_pct, Some(25.99));
        assert_eq!(
            ids(&g.reading),
            [
                ("revenue_cagr", 0),
                ("revenue_consistency", 1),
                ("net_income_cagr", 1)
            ]
        );
        assert_eq!(g.reading.label, GrowthLabel::Growing);
        assert_eq!(g.annual[1].revenue_change_pct, Some(10.0));
        assert_eq!(g.annual[0].revenue_change_pct, None, "first year");
        assert_eq!(g.annual[2].net_income_change_pct, Some(-25.0));
        assert_eq!(g.sources, ["t", "t:annual"]);
        assert_eq!(g.statements_currency.as_deref(), Some("IDR"));

        // Profit to loss, loss to profit, losses at both ends.
        let turn = |first: f64, last: f64| {
            let a = fin(&[
                (2022, None, Some(first)),
                (2023, None, Some(1.0)),
                (2025, None, Some(last)),
            ]);
            growth(&f.symbol, Some(&f), Some(&a)).reading
        };
        assert_eq!(points(&turn(5.0, -1.0), "net_income_cagr"), Some(-1));
        assert_eq!(points(&turn(5.0, 0.0), "net_income_cagr"), Some(-1));
        assert_eq!(points(&turn(-5.0, 1.0), "net_income_cagr"), Some(1));
        let r = turn(-5.0, -1.0);
        assert_eq!(points(&r, "net_income_cagr"), None);
        assert!(has_note(&r, "losses in both FY2022 and FY2025"));

        // Consistency: 1 rise in 3 pairs is under half.
        let a = fin(&[
            (2022, Some(100.0), None),
            (2023, Some(90.0), None),
            (2024, Some(95.0), None),
            (2025, Some(80.0), None),
        ]);
        assert_eq!(
            points(
                &growth(&f.symbol, Some(&f), Some(&a)).reading,
                "revenue_consistency"
            ),
            Some(-1)
        );
        // A null year is skipped, not read as zero.
        let a = fin(&[
            (2022, Some(100.0), None),
            (2023, None, None),
            (2024, Some(120.0), None),
            (2025, Some(130.0), None),
        ]);
        let g = growth(&f.symbol, Some(&f), Some(&a));
        assert_eq!(points(&g.reading, "revenue_consistency"), Some(1));
        assert!(g.reading.rules[0].reason.contains("FY2022 to FY2025"));
    }

    #[test]
    fn source_rates_stand_in_for_missing_statements() {
        let mut f = base();
        f.growth.revenue_cagr_5y_pct = Some(12.0);
        f.growth.eps_growth_3y_pct = Some(-2.0);
        let g = growth(&f.symbol, Some(&f), None);
        // Under their own ids: `revenue_cagr` and `net_income_cagr` always
        // mean idx's CAGR over the fiscal years.
        assert_eq!(
            ids(&g.reading),
            [("revenue_cagr_5y", 1), ("eps_growth_3y", -1)]
        );
        assert_eq!(
            g.reading.rules[0].reason,
            "revenue 5-year growth +12.0% a year, as reported (over +10%)"
        );
        assert_eq!(
            g.reading.rules[1].reason,
            "EPS 3-year growth -2.0% a year, as reported (below 0%)"
        );
        assert!(has_note(
            &g.reading,
            "revenue consistency: fewer than three fiscal years"
        ));
        // Two fiscal years are not enough for a CAGR either.
        let a = fin(&[(2024, Some(1.0), Some(1.0)), (2025, Some(2.0), Some(2.0))]);
        let g = growth(&f.symbol, Some(&f), Some(&a));
        assert_eq!(
            points(&g.reading, "revenue_cagr_5y"),
            Some(1),
            "MSN's figure"
        );
        assert_eq!(points(&g.reading, "revenue_cagr"), None);
        assert_eq!(points(&g.reading, "net_income_cagr"), None);
        assert_eq!(g.revenue_cagr_pct, None);
    }

    #[test]
    fn growth_labels() {
        let mut f = base();
        f.growth.revenue_growth_pct = Some(20.0);
        f.growth.earnings_growth_pct = Some(-5.0);
        f.per_share.eps_ttm = Some(100.0);
        f.per_share.eps_forward = Some(105.0);
        assert_eq!(
            growth(&f.symbol, Some(&f), None).reading.label,
            GrowthLabel::Mixed
        );
        f.growth.earnings_growth_pct = Some(5.0);
        assert_eq!(
            growth(&f.symbol, Some(&f), None).reading.label,
            GrowthLabel::Stable
        );
        f.growth.earnings_growth_pct = Some(50.0);
        assert_eq!(
            growth(&f.symbol, Some(&f), None).reading.label,
            GrowthLabel::Growing
        );
        f.growth.revenue_growth_pct = Some(-1.0);
        f.growth.earnings_growth_pct = Some(-1.0);
        assert_eq!(
            growth(&f.symbol, Some(&f), None).reading.label,
            GrowthLabel::Shrinking
        );
    }

    fn history(symbol: &str, start: NaiveDate, closes: &[f64]) -> History {
        let candles: Vec<Candle> = closes
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let d = start + Duration::days(i as i64);
                Candle {
                    time: crate::model::wib()
                        .from_local_datetime(&d.and_hms_opt(9, 0, 0).unwrap())
                        .unwrap(),
                    open: c,
                    high: c,
                    low: c,
                    close: c,
                    adj_close: None,
                    volume: 1,
                }
            })
            .collect();
        History {
            symbol: sym(symbol),
            name: None,
            currency: None,
            period: Period::Y1,
            interval: Interval::Day1,
            summary: History::summarise(&candles),
            candles,
        }
    }

    fn path(n: usize, scale: f64, start: f64) -> Vec<f64> {
        let mut out = vec![start];
        for i in 1..n {
            let r = [0.01, -0.006, 0.003, -0.011, 0.006][i % 5] * scale;
            out.push(out[i - 1] * (1.0 + r));
        }
        out
    }

    #[test]
    fn price_rules_compare_with_the_benchmark_on_shared_dates() {
        let d0 = NaiveDate::from_ymd_opt(2025, 11, 5).unwrap();
        let bench = history("^JKSE", d0, &path(200, 1.0, 7000.0));
        // Twice as jumpy as the index, and one session behind it at the end.
        let mut s = path(201, 2.0, 1000.0);
        s.remove(0);
        let stock = history("TEST", d0 + Duration::days(1), &s);
        let r = risk(&sym("TEST"), None, Some(&stock), Some(&bench));
        let p = r.price.unwrap();
        assert_eq!(p.observations, 198, "199 shared dates");
        assert_eq!(p.from, d0 + Duration::days(1));
        assert_eq!(p.to, d0 + Duration::days(199));
        assert_eq!(p.beta, Some(2.0));
        assert_eq!(p.correlation, Some(1.0));
        assert_eq!(points(&r.reading, "beta"), Some(1));
        assert_eq!(points(&r.reading, "volatility"), Some(1));
        assert!(p.volatility_pct.unwrap() > 1.9 * p.benchmark_volatility_pct.unwrap());
        assert_eq!(
            points(&r.reading, "drawdown"),
            Some(0),
            "a shallow fall either way"
        );
        assert_eq!(r.reading.label, RiskLabel::Moderate, "{:?}", r.reading);
        // A calmer stock: half the index's moves.
        let calm = history("TEST", d0, &path(200, 0.5, 1000.0));
        let r = risk(&sym("TEST"), None, Some(&calm), Some(&bench));
        assert_eq!(
            ids(&r.reading),
            [("beta", -1), ("volatility", -1), ("drawdown", 0)]
        );
        assert_eq!(r.reading.label, RiskLabel::Low);
        assert_eq!(r.price.unwrap().zero_returns, Some(0));
    }

    #[test]
    fn a_price_that_does_not_move_is_illiquid_not_calm() {
        let d0 = NaiveDate::from_ymd_opt(2025, 11, 5).unwrap();
        let bench = history("^JKSE", d0, &path(200, 1.0, 7000.0));
        // A loss-maker suspended at Rp 50 for the whole year.
        let mut f = base();
        f.health.debt_to_equity = Some(1.0);
        f.health.current_ratio = Some(1.2);
        f.profitability.net_margin_pct = Some(-3.0);
        let flat = history("TEST", d0, &[50.0; 200]);
        let r = risk(&f.symbol, Some(&f), Some(&flat), Some(&bench));
        assert_eq!(
            ids(&r.reading),
            [
                ("leverage", 0),
                ("liquidity", 0),
                ("losses", 1),
                ("illiquidity", 1)
            ]
        );
        assert_eq!(
            r.reading.rules[3].reason,
            "199 of 199 daily returns are zero (more than half): the stock was suspended or barely traded"
        );
        assert!(has_note(
            &r.reading,
            "beta, volatility and drawdown: not scored, 199 of 199 daily returns are zero"
        ));
        assert_eq!(r.reading.label, RiskLabel::Moderate, "not low");
        let p = r.price.unwrap();
        assert_eq!((p.observations, p.zero_returns), (199, Some(199)));
        // The figures are still shown, only not scored.
        assert_eq!(p.beta, Some(0.0));
        assert_eq!(p.volatility_pct, Some(0.0));
        assert_eq!(p.max_drawdown, None);

        // The line is more than half: 100 zero returns of 199 is illiquid,
        // 99 is not.
        let partly = |k: usize| {
            let moving = path(200, 2.0, 1000.0);
            let mut s = vec![moving[0]; k + 1];
            s.extend_from_slice(&moving[1..200 - k]);
            history("TEST", d0, &s)
        };
        let r = risk(&sym("TEST"), None, Some(&partly(100)), Some(&bench));
        assert_eq!(r.price.as_ref().unwrap().zero_returns, Some(100));
        assert_eq!(ids(&r.reading), [("illiquidity", 1)]);
        let r = risk(&sym("TEST"), None, Some(&partly(99)), Some(&bench));
        assert_eq!(r.price.as_ref().unwrap().zero_returns, Some(99));
        assert_eq!(
            r.reading
                .rules
                .iter()
                .map(|x| x.rule.as_str())
                .collect::<Vec<_>>(),
            ["beta", "volatility", "drawdown"]
        );

        // Too short an overlap to call: the usual notes.
        let short = history("^JKSE", d0, &path(15, 1.0, 7000.0));
        let r = risk(&sym("TEST"), None, Some(&flat), Some(&short));
        assert!(r.reading.rules.is_empty(), "{:?}", r.reading);
        assert_eq!(r.price.unwrap().zero_returns, Some(14));
    }

    #[test]
    fn without_the_benchmark_price_rules_are_notes() {
        let d0 = NaiveDate::from_ymd_opt(2025, 11, 5).unwrap();
        let stock = history("TEST", d0, &path(200, 1.0, 1000.0));
        let r = risk(&sym("TEST"), None, Some(&stock), None);
        let p = r.price.as_ref().unwrap();
        assert_eq!(p.benchmark.as_str(), "^JKSE");
        assert_eq!(
            (p.observations, p.beta, p.benchmark_volatility_pct),
            (0, None, None)
        );
        assert!(p.volatility_pct.is_some(), "the stock's own figure");
        assert!(r.reading.rules.is_empty());
        assert!(has_note(&r.reading, "benchmark ^JKSE unavailable"));
        // Too short an overlap for beta, volatility and drawdown.
        let bench = history("^JKSE", d0, &path(15, 1.0, 7000.0));
        let r = risk(&sym("TEST"), None, Some(&stock), Some(&bench));
        assert!(r.reading.rules.is_empty());
        assert!(has_note(
            &r.reading,
            "beta: 14 matched daily returns with IHSG, needs 120"
        ));
        assert!(has_note(&r.reading, "drawdown against IHSG: fewer than 20"));
        // No overlap at all.
        let later = history("^JKSE", d0 + Duration::days(400), &path(50, 1.0, 7000.0));
        let r = risk(&sym("TEST"), None, Some(&stock), Some(&later));
        assert!(has_note(&r.reading, "no sessions in common with ^JKSE"));
        assert_eq!(r.price.unwrap().observations, 0);
    }

    /// The recorded responses of 2026-11-06 (Yahoo only, like the web
    /// terminal; `web/test/analysis.test.mjs` checks the JS port against the
    /// same files and the same numbers).
    mod fixtures {
        use super::*;
        use crate::model::StatementPeriod;
        use crate::providers::yahoo::{self, summary, timeseries};

        fn file(name: &str) -> String {
            std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/yahoo")
                    .join(name),
            )
            .unwrap()
        }

        fn fundamentals(s: &str) -> Fundamentals {
            let body = file(&format!("quote_summary_fundamentals_{s}.json"));
            summary::parse_fundamentals(&body, &sym(s)).unwrap()
        }

        fn annual(s: &str, name: &str) -> Financials {
            timeseries::parse_timeseries(&file(name), &sym(s), StatementPeriod::Annual).unwrap()
        }

        fn chart(name: &str) -> History {
            yahoo::parse_chart(&file(name), Period::Y1, Interval::Day1).unwrap()
        }

        fn lenses(s: &str, statements: &str) -> (ValuationLens, GrowthLens, RiskLens) {
            let f = fundamentals(s);
            let a = annual(s, statements);
            let h = chart(&format!("chart_{s}_1y_1d.json"));
            let b = chart("chart_jkse_1y_1d.json");
            (
                valuation(&f),
                growth(&f.symbol, Some(&f), Some(&a)),
                risk(&f.symbol, Some(&f), Some(&h), Some(&b)),
            )
        }

        fn summary<L: Copy>(r: &Reading<L>) -> (L, i32, usize) {
            (r.label, r.score, r.inputs_available)
        }

        #[test]
        fn bbca_bank() {
            let (v, g, r) = lenses("bbca", "timeseries_bbca.json");
            assert_eq!(summary(&v.reading), (ValuationLabel::Cheap, 3, 5));
            assert_eq!(
                ids(&v.reading),
                [
                    ("pe_trailing", 1),
                    ("peg", 0),
                    ("price_to_book", 0),
                    ("dividend_yield", 1),
                    ("analyst_target", 1)
                ]
            );
            assert!(has_note(&v.reading, "not scored for a financial company"));
            assert_eq!(v.target_upside_pct, Some(58.33));
            assert_eq!(summary(&g.reading), (GrowthLabel::Growing, 3, 6));
            assert_eq!(
                ids(&g.reading),
                [
                    ("revenue_growth", 0),
                    ("earnings_growth", 0),
                    ("eps_forward_growth", 1),
                    ("revenue_cagr", 0),
                    ("revenue_consistency", 1),
                    ("net_income_cagr", 1)
                ]
            );
            assert_eq!(g.annual.len(), 4);
            assert_eq!(g.sources, ["yahoo:quoteSummary", "yahoo:timeseries"]);
            assert_eq!(summary(&r.reading), (RiskLabel::Moderate, 0, 4));
            assert_eq!(
                ids(&r.reading),
                [
                    ("losses", 0),
                    ("beta", 0),
                    ("volatility", 0),
                    ("drawdown", 0)
                ]
            );
            let p = r.price.unwrap();
            assert_eq!(p.observations, 238);
            assert!((p.beta.unwrap() - 0.946).abs() <= 0.002, "{:?}", p.beta);
        }

        #[test]
        fn tlkm_moderate_risk() {
            let (v, g, r) = lenses("tlkm", "timeseries_tlkm.json");
            assert_eq!(summary(&v.reading), (ValuationLabel::Fair, 2, 7));
            assert_eq!(
                ids(&v.reading),
                [
                    ("pe_trailing", 1),
                    ("peg", -1),
                    ("price_to_book", 0),
                    ("price_to_sales", 0),
                    ("ev_to_ebitda", 1),
                    ("dividend_yield", 0),
                    ("analyst_target", 1)
                ]
            );
            assert_eq!(summary(&g.reading), (GrowthLabel::Mixed, 0, 6));
            assert_eq!(
                ids(&g.reading),
                [
                    ("revenue_growth", 0),
                    ("earnings_growth", 1),
                    ("eps_forward_growth", 1),
                    ("revenue_cagr", -1),
                    ("revenue_consistency", 0),
                    ("net_income_cagr", -1)
                ]
            );
            assert_eq!(summary(&r.reading), (RiskLabel::Moderate, 2, 6));
            assert_eq!(
                ids(&r.reading),
                [
                    ("leverage", 0),
                    ("liquidity", 1),
                    ("losses", 0),
                    ("beta", 0),
                    ("volatility", 1),
                    ("drawdown", 0)
                ]
            );
            let p = r.price.unwrap();
            assert_eq!(p.observations, 238);
            assert_eq!(p.from.to_string(), "2025-11-05");
            assert_eq!(p.to.to_string(), "2026-11-04", "the index lags a session");
            assert!((p.beta.unwrap() - 1.122).abs() <= 0.002, "{:?}", p.beta);
            assert!((p.correlation.unwrap() - 0.665).abs() <= 0.002);
            assert!((p.volatility_pct.unwrap() - 49.16).abs() <= 0.02);
            assert!((p.benchmark_volatility_pct.unwrap() - 29.26).abs() <= 0.02);
            assert_eq!(
                r.reading.rules[4].reason,
                "volatility 49.2% a year is 1.68x IHSG's 29.3% (over 1.5x)"
            );
        }

        #[test]
        fn giaa_high_risk_from_negative_equity() {
            let (v, g, r) = lenses("giaa", "timeseries_annual_giaa.json");
            assert_eq!(
                summary(&v.reading),
                (ValuationLabel::InsufficientData, 0, 1)
            );
            assert_eq!(ids(&v.reading), [("dividend_yield", 0)]);
            assert!(has_note(
                &v.reading,
                "P/B: shareholders' equity is negative"
            ));
            assert!(has_note(
                &v.reading,
                "P/S 3769.72 not scored: statements in USD, price in IDR"
            ));
            assert!(has_note(&v.reading, "from 1 analyst: fewer than 3"));
            assert_eq!(summary(&g.reading), (GrowthLabel::Mixed, 0, 4));
            assert_eq!(
                ids(&g.reading),
                [
                    ("revenue_growth", 0),
                    ("revenue_cagr", 1),
                    ("revenue_consistency", 0),
                    ("net_income_cagr", -1)
                ]
            );
            assert_eq!(g.statements_currency.as_deref(), Some("USD"));
            assert_eq!(
                g.reading.rules[3].reason,
                "net income turned from a profit in FY2022 to a loss in FY2025"
            );
            assert_eq!(summary(&r.reading), (RiskLabel::High, 6, 6));
            assert_eq!(
                ids(&r.reading),
                [
                    ("negative_equity", 2),
                    ("liquidity", 1),
                    ("losses", 1),
                    ("beta", 0),
                    ("volatility", 1),
                    ("drawdown", 1)
                ]
            );
            assert_eq!(r.negative_equity, Some(true));
        }

        #[test]
        fn itmg_cheap_with_usd_multiples_skipped() {
            let (v, g, r) = lenses("itmg", "timeseries_annual_itmg.json");
            assert_eq!(summary(&v.reading), (ValuationLabel::Cheap, 4, 4));
            assert_eq!(
                ids(&v.reading),
                [
                    ("pe_trailing", 2),
                    ("peg", 1),
                    ("dividend_yield", 1),
                    ("analyst_target", 0)
                ]
            );
            for m in ["P/B 14103.31", "P/S 13807.10", "EV/EBITDA 91203.7"] {
                assert!(
                    has_note(
                        &v.reading,
                        &format!("{m} not scored: statements in USD, price in IDR")
                    ),
                    "{m}"
                );
            }
            assert_eq!(summary(&g.reading), (GrowthLabel::Mixed, 0, 6));
            assert_eq!(
                ids(&g.reading),
                [
                    ("revenue_growth", 1),
                    ("earnings_growth", 1),
                    ("eps_forward_growth", 1),
                    ("revenue_cagr", -1),
                    ("revenue_consistency", -1),
                    ("net_income_cagr", -1)
                ]
            );
            assert!(g.reading.rules[1]
                .reason
                .ends_with("from a small or negative base"));
            assert_eq!(summary(&r.reading), (RiskLabel::Low, -4, 6));
            assert_eq!(
                ids(&r.reading),
                [
                    ("leverage", -1),
                    ("liquidity", -1),
                    ("losses", 0),
                    ("beta", -1),
                    ("volatility", 0),
                    ("drawdown", -1)
                ]
            );
            assert!((r.price.unwrap().beta.unwrap() - 0.336).abs() <= 0.002);
        }
    }
}
