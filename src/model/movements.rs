//! `idx stockbit movements`: reported changes in insiders' and major holders'
//! stakes, from the user's own Stockbit session.
//!
//! Two kinds of filing share the feed: IDX insider disclosures (directors,
//! commissioners and other insiders, any size, with the trade price) and
//! KSEI's daily changes of holders at 5 % and above (no price filed). Field
//! names follow `idx.ownership.changes.v1` (`shares_from`, `pct_change`, ...).
//! A price the source only estimates is never `price`: it is
//! `estimated_price`, and the value it implies `estimated_value`.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Symbol;

/// Which kind of change to ask the source for.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum MovementFilter {
    Buy,
    Sell,
    Transfer,
    /// Crossings between two accounts (repo, pledges, block deals).
    Cross,
    /// Rights issues, private placements, tender offers, bonus shares and
    /// other corporate actions.
    #[value(alias = "corporate_action")]
    CorporateAction,
}

impl MovementFilter {
    /// The JSON name (`corporate_action`).
    pub fn as_str(&self) -> &'static str {
        match self {
            MovementFilter::Buy => "buy",
            MovementFilter::Sell => "sell",
            MovementFilter::Transfer => "transfer",
            MovementFilter::Cross => "cross",
            MovementFilter::CorporateAction => "corporate_action",
        }
    }
}

/// Which filings to ask the source for.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum MovementSource {
    /// IDX insider disclosures (any size, with the trade price).
    Idx,
    /// KSEI's daily changes of holders at 5 % and above (no price filed).
    Ksei,
}

/// One reported change in a holder's stake.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HolderMovement {
    /// The day of the change as filed.
    pub date: NaiveDate,
    pub symbol: Symbol,
    /// Holder name as filed.
    pub holder: String,
    /// The source's id for the holder (the same on every row of that holder);
    /// `--holder` / `holder` take it.
    pub holder_id: Option<String>,
    /// `director`, `commissioner`; another role the source names, lowercased.
    pub roles: Vec<String>,
    /// `true` foreign, `false` local; `null` when not given.
    pub foreign: Option<bool>,
    /// The source's label, lowercased: `buy`, `sell`, `transfer`, `cross`,
    /// `right_issue`, `private_placement`, `tender_offer`, ...
    pub action: Option<String>,
    /// `idx` (insider disclosure) or `ksei` (holders at 5 % and above).
    pub source: Option<String>,
    /// Shares held before the change.
    pub shares_from: Option<i64>,
    /// Shares held after the change.
    pub shares_to: Option<i64>,
    /// Signed; negative is a decrease. `0` on a rights-issue row that only
    /// dilutes the stake.
    pub shares_change: Option<i64>,
    /// Percent of the company before, as the source rounds it (0.0003 stays
    /// 0.0003).
    pub pct_from: Option<f64>,
    pub pct_to: Option<f64>,
    /// Signed percentage points.
    pub pct_change: Option<f64>,
    /// IDR per share as filed; `null` when the source only estimates it.
    pub price: Option<f64>,
    /// The source's estimate when no price was filed (KSEI rows, some
    /// crossings); `null` otherwise.
    pub estimated_price: Option<f64>,
    /// IDR, the shares changed at the filed price.
    pub value: Option<f64>,
    /// IDR, the same at the estimated price.
    pub estimated_value: Option<f64>,
    /// Purpose as filed (`Pembelian`, `Reverse REPO`, ...).
    pub purpose: Option<String>,
    /// Broker code the source attaches (seen on KSEI rows).
    pub broker: Option<String>,
    /// `foreign`, `government`, `local`, ... as the source labels the broker.
    pub broker_group: Option<String>,
}

/// One page of movements or several, with the filters asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HolderMovements {
    /// One stock; `null` for the whole market.
    pub symbol: Option<Symbol>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub holder_id: Option<String>,
    pub action: Option<MovementFilter>,
    pub source: Option<MovementSource>,
    /// Pages of up to 50 rows read.
    pub pages_read: u32,
    /// `true` when rows remain past the pages read (or a later page failed).
    pub more: bool,
    /// Newest first, as the source orders them.
    pub movements: Vec<HolderMovement>,
}
