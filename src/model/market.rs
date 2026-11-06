//! `idx market overview`: one pass over the universe's quotes.

use chrono::{DateTime, FixedOffset};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Quote, Symbol};

/// A quote reduced to what a board shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MarketRow {
    pub symbol: Symbol,
    pub name: Option<String>,
    pub price: Option<f64>,
    pub change: Option<f64>,
    pub change_pct: Option<f64>,
    pub volume: Option<u64>,
    /// `price * volume` in the quote currency; `null` when either is missing.
    pub value: Option<f64>,
    pub market_cap: Option<f64>,
}

impl MarketRow {
    pub fn from_quote(q: &Quote, name_hint: Option<&str>) -> Self {
        MarketRow {
            symbol: q.symbol.clone(),
            name: q.name.clone().or_else(|| name_hint.map(str::to_string)),
            price: q.price,
            change: q.change,
            change_pct: q.change_pct,
            volume: q.volume,
            value: match (q.price, q.volume) {
                (Some(p), Some(v)) => Some(p * v as f64),
                _ => None,
            },
            market_cap: q.market_cap,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MarketOverview {
    /// Latest trade time among the quotes (WIB).
    pub as_of: Option<DateTime<FixedOffset>>,
    /// Market state reported for the index quote (`REGULAR`, `CLOSED`, ...).
    pub market_state: Option<String>,
    /// `true` when even the newest quote (`as_of`) is dated before the latest
    /// IDX session that has ended: the source has not caught up with the
    /// close. `null` when `as_of` is unknown.
    #[serde(default)]
    pub behind_session: Option<bool>,
    /// The benchmark index quote (`^JKSE`), `null` when unavailable.
    pub index: Option<MarketRow>,
    /// Tickers in the universe walked.
    pub universe: usize,
    /// Tickers that returned a quote with a price and a change.
    pub quoted: usize,
    pub advancing: usize,
    pub declining: usize,
    pub unchanged: usize,
    /// Sum of `value` over quoted tickers.
    pub turnover: f64,
    /// Market cap above `liquid_min_market_cap` and volume above zero qualify for gainers/losers.
    pub liquid_min_market_cap: f64,
    pub gainers: Vec<MarketRow>,
    pub losers: Vec<MarketRow>,
    /// By traded value.
    pub most_active: Vec<MarketRow>,
    /// By market cap.
    pub heavyweights: Vec<MarketRow>,
}
