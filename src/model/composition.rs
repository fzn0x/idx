//! KSEI's monthly "Kepemilikan Efek" composition of one stock: how the shares
//! held in KSEI accounts (scripless) split between local and foreign holders
//! across nine investor types. The archive names no holders.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One investor type's local and foreign shares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompositionType {
    /// KSEI's code: `IS`, `CP`, `PF`, `IB`, `ID`, `MF`, `SC`, `FD` or `OT`.
    pub code: String,
    /// `INSURANCE`, `CORPORATE`, `PENSION FUND`, `FINANCIAL INSTITUTION`,
    /// `INDIVIDUAL`, `MUTUAL FUND`, `SECURITIES COMPANY`, `FOUNDATION` or
    /// `OTHERS`.
    pub investor_type: String,
    pub local_shares: i64,
    pub foreign_shares: i64,
    /// Percent of `shares_scripless`, 4 decimals; `null` when no share is
    /// held in KSEI accounts.
    pub local_pct: Option<f64>,
    pub foreign_pct: Option<f64>,
}

/// One imported month of the stock, for the trend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompositionPoint {
    pub as_of: NaiveDate,
    /// Percent of that month's scripless shares.
    pub local_pct: Option<f64>,
    pub foreign_pct: Option<f64>,
    /// Scripless shares as a percent of the shares listed that month.
    pub scripless_pct: Option<f64>,
    /// `foreign_pct` minus the previous imported month's, in percentage
    /// points; `null` for the first month or when either side is `null`.
    pub foreign_change_pp: Option<f64>,
}

/// `ownership composition`: one stock in one month of KSEI's archive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompositionReport {
    pub symbol: String,
    /// The archive's date: the month's last trading day.
    pub as_of: NaiveDate,
    /// Closing price KSEI printed for the date; `null` when blank.
    pub price: Option<f64>,
    /// `Sec. Num`: the shares listed.
    pub shares_listed: i64,
    /// Shares held in KSEI accounts (local total + foreign total). Shares
    /// still held as certificates (scrip) are not in the archive.
    pub shares_scripless: i64,
    /// `shares_scripless` as a percent of `shares_listed`, 4 decimals.
    pub scripless_pct: Option<f64>,
    pub local_shares: i64,
    pub foreign_shares: i64,
    /// Percent of `shares_scripless`, 4 decimals.
    pub local_pct: Option<f64>,
    pub foreign_pct: Option<f64>,
    /// The nine investor types in KSEI's order (IS, CP, PF, IB, ID, MF, SC,
    /// FD, OT).
    pub by_type: Vec<CompositionType>,
    /// Every imported month for the stock, oldest first, `as_of` included.
    pub history: Vec<CompositionPoint>,
    /// File path or URL the month was imported from.
    pub source: String,
}
