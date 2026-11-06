//! Live order book and the session figures that come with it. Only a realtime
//! source (Stockbit) fills this; the free delayed sources do not have depth.
//!
//! Prices are IDR. Sizes are **shares** (one IDX lot is 100 shares). Foreign
//! buy/sell/net are values in IDR, and `null` while the source marks them as
//! not current. A field the source did not return is `null`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Symbol;

/// One price level of the book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BookLevel {
    /// Limit price, IDR.
    pub price: f64,
    /// Resting size at this level, in shares.
    pub shares: u64,
    /// Number of orders queued at this level.
    pub orders: u64,
}

/// The order book for one instrument at the moment it was fetched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OrderBook {
    pub symbol: Symbol,
    pub name: Option<String>,
    /// Last traded price.
    pub last: Option<f64>,
    pub change: Option<f64>,
    pub change_pct: Option<f64>,
    pub previous_close: Option<f64>,
    pub open: Option<f64>,
    pub day_high: Option<f64>,
    pub day_low: Option<f64>,
    /// Session volume, shares.
    pub volume: Option<u64>,
    /// Session value traded, IDR.
    pub value: Option<f64>,
    /// Bids (buy side), best price first as the source orders them.
    pub bids: Vec<BookLevel>,
    /// Offers (sell side), best price first.
    pub asks: Vec<BookLevel>,
    /// Foreign buy value over the session, IDR. `null` while Stockbit marks
    /// the foreign figures as not current (`has_foreign_bs` not true, seen
    /// during trading, when they are the previous session's); a warning then
    /// says so.
    pub foreign_buy: Option<f64>,
    /// Foreign sell value over the session, IDR. `null` while Stockbit marks
    /// the foreign figures as not current, as for `foreign_buy`.
    pub foreign_sell: Option<f64>,
    /// Foreign net (buy minus sell), IDR. `null` while Stockbit marks the
    /// foreign figures as not current, as for `foreign_buy`.
    pub foreign_net: Option<f64>,
}

impl OrderBook {
    /// Best (highest) bid price.
    pub fn best_bid(&self) -> Option<&BookLevel> {
        self.bids.first()
    }
    /// Best (lowest) offer price.
    pub fn best_ask(&self) -> Option<&BookLevel> {
        self.asks.first()
    }
    /// Spread between best offer and best bid, when both sides are present.
    pub fn spread(&self) -> Option<f64> {
        Some(self.best_ask()?.price - self.best_bid()?.price)
    }
}
