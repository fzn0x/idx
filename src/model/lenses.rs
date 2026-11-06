//! `stocks growth | valuation | risk`: focused readings next to the blended
//! verdict of `stocks fundamental`. Each lens is a transparent rule score:
//! every rule that ran is listed with its points, every rule that could not
//! run is noted with the reason. Input fields are the source's values as
//! reported (`null` when absent); fields documented as "computed by idx" are
//! derived from reported values only (`analysis::lens`).

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Drawdown, Symbol};
use crate::error::{IdxError, Result};

/// Which focused reading `stocks fundamental --lens` (and MCP `lens`) returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Lens {
    /// Latest quarter, forward EPS and the fiscal years (idx.growth.v1).
    Growth,
    /// Multiples, yield and the analyst target (idx.valuation.v1).
    Valuation,
    /// Balance sheet, losses and price risk against IHSG (idx.risk.v1).
    Risk,
}

impl Lens {
    pub fn as_str(&self) -> &'static str {
        match self {
            Lens::Growth => "growth",
            Lens::Valuation => "valuation",
            Lens::Risk => "risk",
        }
    }
}

impl std::str::FromStr for Lens {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <Lens as clap::ValueEnum>::from_str(s.trim(), true).map_err(|_| {
            IdxError::Usage(format!("unknown lens {s:?}; use growth, valuation or risk"))
        })
    }
}

impl std::fmt::Display for Lens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One rule that ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LensRule {
    /// Stable id, e.g. `pe_trailing` (see docs/json-output.md for each lens).
    pub rule: String,
    /// Points this rule added. Growth and valuation points favour growth and
    /// cheapness; risk points add risk. A rule with 0 points still ran.
    pub points: i32,
    /// Plain language, with the figure and the threshold it was compared with.
    pub reason: String,
}

/// Rule-based reading. Heuristic, not advice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Reading<L> {
    pub label: L,
    /// Sum of `rules[].points`.
    pub score: i32,
    /// Rules that had data and ran (the length of `rules`); fewer than three
    /// gives `insufficient_data`.
    pub inputs_available: usize,
    pub rules: Vec<LensRule>,
    /// Rules that could not run, and why (not reported, not applicable to a
    /// financial company, statements in another currency, negative equity,
    /// too few analysts, benchmark unavailable).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GrowthLabel {
    Growing,
    Stable,
    Mixed,
    Shrinking,
    InsufficientData,
}

impl GrowthLabel {
    pub fn as_str(&self) -> &'static str {
        match self {
            GrowthLabel::Growing => "growing",
            GrowthLabel::Stable => "stable",
            GrowthLabel::Mixed => "mixed",
            GrowthLabel::Shrinking => "shrinking",
            GrowthLabel::InsufficientData => "insufficient data",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValuationLabel {
    Cheap,
    Fair,
    Expensive,
    InsufficientData,
}

impl ValuationLabel {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValuationLabel::Cheap => "cheap",
            ValuationLabel::Fair => "fair",
            ValuationLabel::Expensive => "expensive",
            ValuationLabel::InsufficientData => "insufficient data",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RiskLabel {
    Low,
    Moderate,
    High,
    InsufficientData,
}

impl RiskLabel {
    pub fn as_str(&self) -> &'static str {
        match self {
            RiskLabel::Low => "low",
            RiskLabel::Moderate => "moderate",
            RiskLabel::High => "high",
            RiskLabel::InsufficientData => "insufficient data",
        }
    }
}

/// One fiscal year from the annual statements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AnnualGrowth {
    pub end_date: NaiveDate,
    pub revenue: Option<f64>,
    pub net_income: Option<f64>,
    /// Against the previous fiscal year, percent; computed by idx. `null` for
    /// the first year, a missing figure or a non-positive base.
    pub revenue_change_pct: Option<f64>,
    pub net_income_change_pct: Option<f64>,
}

/// `idx.growth.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GrowthLens {
    pub symbol: Symbol,
    pub name: Option<String>,
    /// Currency of the annual statements (may differ from the trading
    /// currency; growth rates do not depend on it).
    pub statements_currency: Option<String>,
    pub most_recent_quarter: Option<NaiveDate>,
    /// Latest quarter against the same quarter a year earlier, percent, as reported.
    pub revenue_growth_pct: Option<f64>,
    pub earnings_growth_pct: Option<f64>,
    /// As reported by MSN when it contributed (`auto` asks it only to fill gaps).
    pub revenue_cagr_5y_pct: Option<f64>,
    pub eps_growth_3y_pct: Option<f64>,
    pub eps_ttm: Option<f64>,
    pub eps_forward: Option<f64>,
    /// `(eps_forward / eps_ttm - 1) * 100` when both are above zero; computed by idx.
    pub eps_forward_growth_pct: Option<f64>,
    /// Fiscal years from the annual statements, oldest first (empty when unavailable).
    pub annual: Vec<AnnualGrowth>,
    /// Compound annual growth from the oldest to the newest fiscal year that
    /// carries a value (at least three such years, both ends above zero),
    /// percent; computed by idx.
    pub revenue_cagr_pct: Option<f64>,
    pub net_income_cagr_pct: Option<f64>,
    pub reading: Reading<GrowthLabel>,
    /// Endpoints that contributed (fundamentals sources, then the statements source).
    pub sources: Vec<String>,
}

/// `idx.valuation.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ValuationLens {
    pub symbol: Symbol,
    pub name: Option<String>,
    /// Trading currency of the price.
    pub currency: Option<String>,
    /// Currency of the financial statements when the source names it. When it
    /// differs from `currency`, P/B, P/S and the EV multiples divide the price
    /// by figures in another currency: they are shown as reported and not scored.
    pub financial_currency: Option<String>,
    /// Bank or insurer: P/S and EV/EBITDA are not scored.
    pub financial_company: bool,
    /// From the signs of book value, P/B and debt/equity
    /// (`Fundamentals::negative_equity`); `null` when none is reported.
    pub negative_equity: Option<bool>,
    pub price: Option<f64>,
    pub pe_trailing: Option<f64>,
    pub pe_forward: Option<f64>,
    pub peg: Option<f64>,
    pub price_to_book: Option<f64>,
    pub price_to_sales: Option<f64>,
    pub ev_to_ebitda: Option<f64>,
    /// Shown, not scored (it overlaps P/S).
    pub ev_to_revenue: Option<f64>,
    pub dividend_yield_pct: Option<f64>,
    pub payout_ratio_pct: Option<f64>,
    pub target_mean: Option<f64>,
    pub analysts: Option<u32>,
    /// `(target_mean / price - 1) * 100`; computed by idx.
    pub target_upside_pct: Option<f64>,
    pub reading: Reading<ValuationLabel>,
    pub sources: Vec<String>,
}

/// Price statistics against the benchmark, computed by idx from daily closes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PriceRisk {
    /// `^JKSE` (IHSG).
    pub benchmark: Symbol,
    /// Window the statistics cover: the dates the stock and the benchmark
    /// share (the stock's own dates when the benchmark is unavailable).
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Matched daily returns behind `beta` and `correlation` (0 without the benchmark).
    pub observations: usize,
    /// Against the benchmark, from simple daily returns on the shared dates;
    /// `null` under 120 observations.
    pub beta: Option<f64>,
    pub correlation: Option<f64>,
    /// Annualised standard deviation of daily log returns over the window, percent.
    pub volatility_pct: Option<f64>,
    pub benchmark_volatility_pct: Option<f64>,
    /// Largest peak-to-trough fall over the window; `null` when the price never fell.
    pub max_drawdown: Option<Drawdown>,
    pub benchmark_max_drawdown_pct: Option<f64>,
}

/// `idx.risk.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RiskLens {
    pub symbol: Symbol,
    pub name: Option<String>,
    pub currency: Option<String>,
    /// Bank or insurer: the leverage and liquidity rules do not apply.
    pub financial_company: bool,
    pub negative_equity: Option<bool>,
    pub debt_to_equity: Option<f64>,
    pub current_ratio: Option<f64>,
    pub quick_ratio: Option<f64>,
    pub net_margin_pct: Option<f64>,
    pub eps_ttm: Option<f64>,
    /// `null` when no daily candles could be fetched for the stock.
    pub price: Option<PriceRisk>,
    pub reading: Reading<RiskLabel>,
    pub sources: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lens_parses_case_insensitively() {
        assert_eq!(" Risk ".parse::<Lens>().unwrap(), Lens::Risk);
        assert_eq!("growth".parse::<Lens>().unwrap(), Lens::Growth);
        let e = "bogus".parse::<Lens>().unwrap_err();
        assert_eq!(e.code(), "usage");
        assert!(e.to_string().contains("growth, valuation or risk"));
    }

    #[test]
    fn labels_serialise_snake_case() {
        assert_eq!(
            serde_json::to_string(&GrowthLabel::InsufficientData).unwrap(),
            "\"insufficient_data\""
        );
        assert_eq!(
            serde_json::to_string(&RiskLabel::Moderate).unwrap(),
            "\"moderate\""
        );
        assert_eq!(ValuationLabel::Cheap.as_str(), "cheap");
    }
}
