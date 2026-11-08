//! Money flow from the exchange's daily summaries: foreign buying and
//! selling per stock (`idx stocks foreign`, `idx market foreign`), member
//! activity (`idx market brokers`) and one broker across the most traded
//! stocks (`idx bandar broker`).
//!
//! IDX publishes foreign trading in shares only. `foreign_net_value_at_close`
//! multiplies the net shares by the closing price to give a rupiah scale; it
//! is not the value foreigners actually traded at, and is named so.
//!
//! `idx stocks foreign --source ipot` reads Indo Premier's broker summary
//! instead: foreign buying, selling and net in rupiah across all boards
//! ([`ForeignValueFlow`], its own schema). The two sources answer different
//! questions (shares on the regular board against rupiah on every board) and
//! share no field names.
//!
//! `idx market brokers` reads idx.co.id's broker summary or IPOT's ranking
//! of its latest session ([`BrokerSource`]), both in [`BrokerRanking`]; only
//! IPOT publishes each member's buying, selling and net.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{BrokerKind, Symbol};

/// One stock on one trading day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignDay {
    pub date: NaiveDate,
    pub close: Option<f64>,
    pub change: Option<f64>,
    /// Shares traded as IDX's daily stock summary reports them; non-regular
    /// (negotiated) trades are reported apart and not included.
    pub volume: Option<f64>,
    /// Value traded on the same basis, IDR.
    pub value: Option<f64>,
    /// Shares bought by foreign investors.
    pub foreign_buy: Option<f64>,
    /// Shares sold by foreign investors.
    pub foreign_sell: Option<f64>,
    /// `foreign_buy - foreign_sell`, shares.
    pub foreign_net: Option<f64>,
    /// `foreign_net * close`, IDR: a rupiah scale, not the traded value.
    pub foreign_net_value_at_close: Option<f64>,
}

/// Sums over the sessions shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignTotals {
    pub sessions: usize,
    pub foreign_buy: f64,
    pub foreign_sell: f64,
    pub foreign_net: f64,
    pub foreign_net_value_at_close: f64,
    /// Sessions with more foreign buying than selling.
    pub net_buy_sessions: usize,
    /// Sessions with more foreign selling than buying.
    pub net_sell_sessions: usize,
    /// Consecutive sessions on the same side, ending with the latest:
    /// `+3` three net-buy sessions in a row, `-2` two net-sell ones, `0` when
    /// the latest session was flat.
    pub streak: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignFlow {
    pub symbol: Symbol,
    pub name: Option<String>,
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Oldest first.
    pub days: Vec<ForeignDay>,
    pub totals: ForeignTotals,
}

/// Where a stock's foreign flow comes from.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum ForeignSource {
    /// idx.co.id's daily stock summary: shares on the regular board.
    Idx,
    /// IPOT's broker summary: rupiah across all boards.
    Ipot,
}

impl ForeignSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ForeignSource::Idx => "idx",
            ForeignSource::Ipot => "ipot",
        }
    }
}

impl std::str::FromStr for ForeignSource {
    type Err = crate::error::IdxError;
    fn from_str(s: &str) -> crate::error::Result<Self> {
        <ForeignSource as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            crate::error::IdxError::Usage(format!("invalid source {s:?}; use idx or ipot"))
        })
    }
}

/// One stock on one session from IPOT's broker summary: foreign investors'
/// trading in rupiah across all boards (negotiated trades included). Figures
/// of a million and more carry the source's one-decimal rounding (see
/// [`ForeignValueFlow::rounded`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignValueDay {
    pub date: NaiveDate,
    /// Value traded by all investors on all boards, IDR.
    pub value: Option<f64>,
    /// Lots traded on all boards; one lot is 100 shares.
    pub lots: Option<f64>,
    /// Average traded price, IDR.
    pub average_price: Option<f64>,
    /// Value foreign investors bought, IDR, as published.
    pub foreign_buy_value: Option<f64>,
    /// Value foreign investors sold, IDR: `foreign_buy_value -
    /// foreign_net_value`, computed by idx.
    pub foreign_sell_value: Option<f64>,
    /// Foreign buy minus sell value, IDR, as published.
    pub foreign_net_value: Option<f64>,
    /// `(foreign_buy_value + foreign_sell_value) / (2 * value) * 100`: foreign
    /// investors' share of the session's trading, computed by idx.
    pub foreign_share_pct: Option<f64>,
}

/// Sums over the sessions shown, of the values present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignValueTotals {
    pub sessions: usize,
    pub value: f64,
    pub foreign_buy_value: f64,
    pub foreign_sell_value: f64,
    pub foreign_net_value: f64,
    /// Foreign share of all the sessions' trading, as for one session;
    /// `null` when nothing was traded.
    pub foreign_share_pct: Option<f64>,
    /// Sessions with more foreign buying than selling.
    pub net_buy_sessions: usize,
    /// Sessions with more foreign selling than buying.
    pub net_sell_sessions: usize,
    /// Same meaning as [`ForeignTotals::streak`].
    pub streak: i32,
}

/// Foreign flow of one stock from IPOT, in rupiah (`idx.foreign.value.v1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignValueFlow {
    pub symbol: Symbol,
    pub name: Option<String>,
    /// First session shown (WIB).
    pub from: NaiveDate,
    /// Last session shown (WIB).
    pub to: NaiveDate,
    /// Sessions the stock traded, oldest first.
    pub days: Vec<ForeignValueDay>,
    pub totals: ForeignValueTotals,
    /// `true` when the source abbreviated at least one figure (`1.6 M`), so
    /// those numbers carry only its one-decimal precision.
    pub rounded: bool,
}

/// One stock in the market-wide foreign ranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignRow {
    pub symbol: Symbol,
    pub name: Option<String>,
    pub close: Option<f64>,
    pub change: Option<f64>,
    /// `change / previous close * 100`, computed by idx.
    pub change_pct: Option<f64>,
    /// Shares traded, non-regular (negotiated) trades excluded, as published.
    pub volume: Option<f64>,
    /// Value traded on the same basis, IDR.
    pub value: Option<f64>,
    pub foreign_buy: Option<f64>,
    pub foreign_sell: Option<f64>,
    /// `foreign_buy - foreign_sell`, shares.
    pub foreign_net: Option<f64>,
    /// `foreign_net * close`, IDR: a rupiah scale, not the traded value.
    pub foreign_net_value_at_close: Option<f64>,
}

/// Which figure ranks the foreign lists.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum ForeignSort {
    /// Net shares times the closing price (comparable across price levels).
    Value,
    /// Net shares.
    Shares,
}

impl ForeignSort {
    pub fn as_str(&self) -> &'static str {
        match self {
            ForeignSort::Value => "value",
            ForeignSort::Shares => "shares",
        }
    }
}

impl std::str::FromStr for ForeignSort {
    type Err = crate::error::IdxError;
    fn from_str(s: &str) -> crate::error::Result<Self> {
        <ForeignSort as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            crate::error::IdxError::Usage(format!("invalid sort {s:?}; use value or shares"))
        })
    }
}

/// Foreign net buying and selling across every stock on one trading day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MarketForeign {
    pub date: NaiveDate,
    /// Stocks in the day's summary.
    pub stocks: usize,
    /// Sums over every stock, shares.
    pub foreign_buy: f64,
    pub foreign_sell: f64,
    pub foreign_net: f64,
    /// Sum of the per-stock `foreign_net_value_at_close`, IDR.
    pub foreign_net_value_at_close: f64,
    pub sort: ForeignSort,
    /// Largest foreign net buying first.
    pub net_buy: Vec<ForeignRow>,
    /// Largest foreign net selling first.
    pub net_sell: Vec<ForeignRow>,
}

/// One exchange member's trading on one day, both sides together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrokerDayRow {
    /// Position by the sort key, 1 = most.
    pub rank: u32,
    pub code: String,
    /// Member name as the source publishes it.
    pub name: Option<String>,
    /// Shares bought plus shares sold.
    pub volume: Option<f64>,
    /// Value bought plus value sold, IDR.
    pub value: Option<f64>,
    /// Trades on either side.
    pub frequency: Option<f64>,
    /// `value` as a percent of all members' value; across all members this
    /// sums to 100, and all members' value is twice the market's (each trade
    /// has a buying and a selling member).
    pub value_share_pct: Option<f64>,
    /// Value bought, IDR, from IPOT; `null` from idx.co.id, which publishes
    /// both sides together only.
    #[serde(default)]
    pub buy_value: Option<f64>,
    /// Value sold, IDR, from IPOT; `null` from idx.co.id.
    #[serde(default)]
    pub sell_value: Option<f64>,
    /// Value bought minus value sold, IDR: IPOT's figure, summed with that of
    /// any row folded into the member (`JB` into `CC`). IPOT abbreviates it
    /// and `buy_value` and `sell_value` each to two decimals, so it can
    /// differ from `buy_value - sell_value` by the rounding of both, billions
    /// for a member trading trillions. `null` from idx.co.id.
    #[serde(default)]
    pub net_value: Option<f64>,
    /// The source's class of the member (IPOT); `null` from idx.co.id.
    #[serde(default)]
    pub kind: Option<BrokerKind>,
}

/// Which figure ranks the members.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum BrokerSort {
    Value,
    Volume,
    Frequency,
}

impl BrokerSort {
    pub fn as_str(&self) -> &'static str {
        match self {
            BrokerSort::Value => "value",
            BrokerSort::Volume => "volume",
            BrokerSort::Frequency => "frequency",
        }
    }
}

impl std::str::FromStr for BrokerSort {
    type Err = crate::error::IdxError;
    fn from_str(s: &str) -> crate::error::Result<Self> {
        <BrokerSort as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            crate::error::IdxError::Usage(format!(
                "invalid sort {s:?}; use value, volume or frequency"
            ))
        })
    }
}

/// Where the member ranking comes from.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum BrokerSource {
    /// idx.co.id, and IPOT when idx.co.id refuses, cannot be reached, fails or has not published
    /// the session, if IPOT holds it.
    Auto,
    /// idx.co.id's daily broker summary: any session.
    Idx,
    /// IPOT's ranking: its latest session only, with each member's class and net (and in json,
    /// csv and jsonl its value bought and sold).
    Ipot,
}

impl std::str::FromStr for BrokerSource {
    type Err = crate::error::IdxError;
    fn from_str(s: &str) -> crate::error::Result<Self> {
        <BrokerSource as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            crate::error::IdxError::Usage(format!("invalid source {s:?}; use auto, idx or ipot"))
        })
    }
}

/// Every exchange member's trading on one day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrokerRanking {
    pub date: NaiveDate,
    /// Members with trades that day.
    pub members: usize,
    /// Sums over all members (each trade counted on both sides).
    pub total_volume: f64,
    pub total_value: f64,
    pub total_frequency: f64,
    pub sort: BrokerSort,
    /// `true` when the source abbreviated figures (IPOT, to two decimals:
    /// `2.79 T`), so they and the totals carry only that precision; `false`
    /// from idx.co.id, which publishes them exact.
    #[serde(default)]
    pub rounded: bool,
    pub brokers: Vec<BrokerDayRow>,
}

/// One stock where the broker is among the top ten on a side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrokerStockRow {
    pub symbol: Symbol,
    pub name: Option<String>,
    /// The broker's place on this side, 1 = most lots, as the source ranks it.
    pub rank: u32,
    /// Lots bought (in `bought`) or sold (in `sold`); one lot is 100 shares.
    pub lots: Option<f64>,
    /// Value in IDR.
    pub value: Option<f64>,
    pub average_price: Option<f64>,
    /// `value` as a percent of all value traded in the stock over the range.
    pub share_pct: Option<f64>,
    /// The broker's value on the other side when it is also among the top
    /// ten there; `null` otherwise (it may still have traded, below tenth place).
    pub opposite_value: Option<f64>,
    /// `value - opposite_value` (in `sold`: `opposite_value - value`, so
    /// positive is net buying) when both sides are visible; otherwise `null`.
    pub net_value: Option<f64>,
}

/// Where one broker shows up among the top buyers and sellers of the most
/// traded stocks. Only the top ten per side are published, so this is what
/// is visible, not the broker's whole book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrokerActivity {
    pub code: String,
    /// Member name from the IDX exchange-member list bundled with idx.
    pub name: Option<String>,
    /// `1d`, `1w`, `1m` or `3m`.
    pub period: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Stocks scanned.
    pub scanned: usize,
    /// Scanned stocks with trades in the range.
    pub checked: usize,
    /// Scanned stocks whose summary could not be fetched.
    pub failed: usize,
    /// Sum of `value` over `bought`, IDR.
    pub bought_value: f64,
    /// Sum of `value` over `sold`, IDR.
    pub sold_value: f64,
    /// Stocks where the broker is a top-ten buyer, largest value first.
    pub bought: Vec<BrokerStockRow>,
    /// Stocks where the broker is a top-ten seller, largest value first.
    pub sold: Vec<BrokerStockRow>,
}
