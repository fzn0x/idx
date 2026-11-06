//! `stocks intraday`: one session by the minute (`idx.intraday.v1`).

use chrono::{DateTime, FixedOffset, NaiveDate};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Candle, Interval, Quote, Symbol};

/// One session of one stock or index by the minute (or the bar size asked
/// for): the bars, the quote as its source reports it, and a summary of the
/// bars computed by idx.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Intraday {
    pub symbol: Symbol,
    pub name: Option<String>,
    /// WIB date of the bars; `null` when there are none.
    pub session: Option<NaiveDate>,
    /// Bar size asked for. A source that serves another granularity says so in
    /// `warnings` (Stockbit's same-day window is always one-minute).
    pub interval: Interval,
    /// The quote as its source reports it (previous close, last price and its
    /// change, the day's open, high, low and volume, market state, last trade
    /// time); `null` when the quote could not be fetched (a warning says why).
    pub quote: Option<Quote>,
    /// Oldest first, start of each bar in WIB. Minutes without a trade have no
    /// bar.
    pub bars: Vec<Candle>,
    /// Over every bar of the session; `null` without bars.
    pub summary: Option<IntradaySummary>,
}

/// Figures over a session's bars, computed by idx. They describe the bars, so
/// they can differ from the quote's official figures: the opening auction's
/// volume is in no bar on Yahoo, and the first bar's open is not always the
/// opening price.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IntradaySummary {
    /// Start of the first bar, WIB.
    pub first_bar: DateTime<FixedOffset>,
    /// Start of the last bar, WIB.
    pub last_bar: DateTime<FixedOffset>,
    /// Number of bars.
    pub bars: usize,
    /// First bar's open.
    pub open: f64,
    /// Highest high.
    pub high: f64,
    /// Lowest low.
    pub low: f64,
    /// Last bar's close.
    pub close: f64,
    /// Sum of the bars' volume, shares. Below the day's volume when the source
    /// leaves auction volume out of the bars.
    pub bars_volume: u64,
    /// Volume-weighted average of each bar's typical price
    /// ((high + low + close) / 3), computed by idx and rounded to 2 decimals:
    /// an approximation of IDX's average price (value / volume of the regular
    /// market). `null` when no bar has volume (indices on Yahoo).
    pub vwap: Option<f64>,
}
