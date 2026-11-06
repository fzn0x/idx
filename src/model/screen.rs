//! `stocks screen` output.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Symbol;

/// One matching stock. Quote-level fields come from the batch quote call;
/// the return windows only when a filter, the sort or `--returns` asked for
/// them (see `ScreenReport.returns_source`); the fundamentals block only when
/// a filter or `--fundamentals` asked for it (see `fundamentals_checked`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScreenRow {
    pub symbol: Symbol,
    pub name: Option<String>,
    pub price: Option<f64>,
    pub change_pct: Option<f64>,
    pub market_cap: Option<f64>,
    pub pe_trailing: Option<f64>,
    pub pe_forward: Option<f64>,
    pub price_to_book: Option<f64>,
    pub dividend_yield_pct: Option<f64>,
    pub eps_ttm: Option<f64>,
    pub avg_volume_3m: Option<u64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    /// Position of the price in the 52-week range, computed by idx from the
    /// quote: 0 at the low, 100 at the high. Null without all three figures
    /// or when the high equals the low.
    pub range_52w_pct: Option<f64>,
    /// Price return over one week, percent, as the returns source reports it.
    pub return_1w_pct: Option<f64>,
    /// One month, percent.
    pub return_1m_pct: Option<f64>,
    /// Three months, percent.
    pub return_3m_pct: Option<f64>,
    /// Six months, percent.
    pub return_6m_pct: Option<f64>,
    /// Year to date, percent.
    pub return_ytd_pct: Option<f64>,
    /// One year, percent.
    pub return_1y_pct: Option<f64>,
    /// `true` when the fundamentals block below was fetched for this row.
    pub fundamentals_checked: bool,
    pub sector: Option<String>,
    pub industry: Option<String>,
    pub roe_pct: Option<f64>,
    pub roa_pct: Option<f64>,
    pub net_margin_pct: Option<f64>,
    pub operating_margin_pct: Option<f64>,
    pub debt_to_equity: Option<f64>,
    pub revenue_growth_pct: Option<f64>,
    pub earnings_growth_pct: Option<f64>,
    pub peg: Option<f64>,
    pub price_to_sales: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScreenReport {
    /// Preset name when one was used.
    pub preset: Option<String>,
    /// Every clause applied, one per clause, the preset's included.
    pub filters: Vec<String>,
    pub sort: String,
    pub descending: bool,
    /// Tickers in the universe that was screened.
    pub universe: usize,
    /// Tickers the quote source returned.
    pub quoted: usize,
    /// Rows left after the quote-level filters (the return windows included).
    pub matched_quote_level: usize,
    /// Rows for which fundamentals were fetched.
    pub fundamentals_checked: usize,
    /// `true` when more rows qualified for a fundamentals check than the cap allowed.
    pub fundamentals_capped: bool,
    /// Where the `return_*` fields came from (`msn:screener`); null when they
    /// were not joined.
    pub returns_source: Option<String>,
    pub total_matches: usize,
    pub rows: Vec<ScreenRow>,
}

/// One stock's price returns as the source reports them (percent). Internal:
/// the screener joins these into its rows. Not `ReturnWindows`, which idx
/// computes from closes.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketReturns {
    pub symbol: Symbol,
    /// `msn:screener`.
    pub source: &'static str,
    pub w1: Option<f64>,
    pub m1: Option<f64>,
    pub m3: Option<f64>,
    pub m6: Option<f64>,
    pub ytd: Option<f64>,
    pub y1: Option<f64>,
}
