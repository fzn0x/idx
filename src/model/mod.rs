//! Provider-agnostic domain types. Everything a provider returns is normalised
//! into these structs, and these structs are what the table/JSON renderers and
//! the MCP tools consume. Field names here are the JSON contract.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{IdxError, Result};

pub mod actions;
pub mod bandar;
pub mod brief;
pub mod broker;
pub mod calendar;
pub mod dividends;
pub mod flow;
pub mod fundamentals;
pub mod instruments;
pub mod market;
pub mod news;
pub mod orderbook;
pub mod ownership;
pub mod returns;
pub mod screen;
pub mod technical;
pub mod watchlist;

pub use actions::{
    ActionKind, CorporateAction, CorporateActions, ShareholderMeeting, WarrantRight,
};
pub use bandar::{
    BandarDaily, BandarDay, BandarPeriod, BandarPhase, BandarReport, BandarScreen, BandarScreenRow,
    BandarSignal, BandarTier,
};
pub use brief::{Brief, DividendSummary, NewsSummary, OwnershipSummary};
pub use broker::{BrokerBoard, BrokerFlow, BrokerInvestor, BrokerKind, BrokerSummary};
pub use calendar::{Calendar, CalendarCategory, CalendarEvent, Holiday, Holidays};
pub use dividends::{DividendEvent, DividendHistory, DividendYear, SplitEvent};
pub use flow::{
    BrokerActivity, BrokerDayRow, BrokerRanking, BrokerSort, BrokerStockRow, ForeignDay,
    ForeignFlow, ForeignRow, ForeignSort, ForeignTotals, MarketForeign,
};
pub use fundamentals::{
    Analyst, BalanceSheet, CashFlow, Director, Dividend, Earnings, EarningsEstimate,
    EarningsQuarter, FinancialPeriod, Financials, Fundamentals, Growth, Health, IncomeStatement,
    NextEarnings, PerShare, PeriodKind, Profile, Profitability, RevenueEarnings, StatementPeriod,
    Valuation, Verdict, VerdictLabel,
};
pub use instruments::{IndexBoard, IndexRow, Instrument, InstrumentList, Member, MemberList};
pub use market::{MarketOverview, MarketRow};
pub use news::{NewsItem, NewsReport, SentimentLabel, SentimentReport};
pub use orderbook::{BookLevel, OrderBook};
pub use ownership::{
    Change, ChangeBrief, ChangeKind, ChangesReport, ChangesSummary, DiscoveredFile, EntityReport,
    ForeignFlowReport, ForeignPoint, ForeignReport, HoldersReport, Holding, ImportReport,
    IssuerFlow, IssuerForeign, Release, TypeFlow,
};
pub use returns::{BenchmarkReturns, DayMove, Drawdown, Performance, ReturnWindows};
pub use screen::{ScreenReport, ScreenRow};
pub use technical::{
    Bollinger, Levels, Macd, MovingAverages, Signal, SignalLabel, Technical, Trend, VolumeStats,
};
pub use watchlist::Watchlist;

/// Jakarta is UTC+7 all year (WIB).
pub const WIB_OFFSET_SECS: i32 = 7 * 3600;

pub fn wib() -> FixedOffset {
    FixedOffset::east_opt(WIB_OFFSET_SECS).expect("valid offset")
}

/// Unix seconds to a WIB timestamp.
pub fn wib_from_unix(secs: i64) -> Option<DateTime<FixedOffset>> {
    Utc.timestamp_opt(secs, 0)
        .single()
        .map(|dt| dt.with_timezone(&wib()))
}

/// An IDX ticker such as `BBCA`, always stored upper-case without any exchange suffix.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Symbol(String);

impl Symbol {
    /// Accepts `bbca`, `BBCA`, `BBCA.JK`, ` bbca ` and returns `BBCA`.
    /// Index codes keep their caret: `^JKSE` (IHSG), `^JKLQ45`.
    pub fn parse(raw: &str) -> Result<Symbol> {
        let mut s = raw.trim().to_ascii_uppercase();
        if let Some(stripped) = s.strip_suffix(".JK") {
            s = stripped.to_string();
        }
        if s.is_empty() {
            return Err(IdxError::Usage("empty symbol".into()));
        }
        let body = s.strip_prefix('^').unwrap_or(&s);
        let ok = !body.is_empty()
            && body.len() <= 12
            && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !ok {
            return Err(IdxError::Usage(format!(
                "invalid symbol {raw:?}: expected an IDX ticker like BBCA or BBRI"
            )));
        }
        Ok(Symbol(s))
    }

    /// `true` for index codes (`^JKSE`), which have no `.JK` suffix anywhere.
    pub fn is_index(&self) -> bool {
        self.0.starts_with('^')
    }

    /// Parses `BBCA,BBRI TLKM` into unique symbols, preserving order.
    pub fn parse_list<I, S>(items: I) -> Result<Vec<Symbol>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out: Vec<Symbol> = Vec::new();
        for item in items {
            for piece in item.as_ref().split(|c: char| c == ',' || c.is_whitespace()) {
                if piece.trim().is_empty() {
                    continue;
                }
                let sym = Symbol::parse(piece)?;
                if !out.contains(&sym) {
                    out.push(sym);
                }
            }
        }
        if out.is_empty() {
            return Err(IdxError::Usage("no symbols given".into()));
        }
        Ok(out)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Yahoo Finance form, e.g. `BBCA.JK`; indices stay as `^JKSE`.
    pub fn yahoo(&self) -> String {
        if self.is_index() {
            self.0.clone()
        } else {
            format!("{}.JK", self.0)
        }
    }

    /// Reverse of [`Symbol::yahoo`].
    pub fn from_yahoo(s: &str) -> Symbol {
        Symbol(s.trim().trim_end_matches(".JK").to_ascii_uppercase())
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A delayed market quote. Every optional field is `null` when the source did
/// not return it; nothing is estimated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Quote {
    pub symbol: Symbol,
    pub name: Option<String>,
    pub currency: Option<String>,
    /// Last traded price.
    pub price: Option<f64>,
    /// Absolute change versus the previous close.
    pub change: Option<f64>,
    /// Percent change versus the previous close, e.g. `-14.81` for -14.81 %.
    pub change_pct: Option<f64>,
    pub previous_close: Option<f64>,
    pub open: Option<f64>,
    pub day_high: Option<f64>,
    pub day_low: Option<f64>,
    pub volume: Option<u64>,
    /// 3-month average daily volume.
    pub avg_volume_3m: Option<u64>,
    /// Market capitalisation in `currency`.
    pub market_cap: Option<f64>,
    pub pe_trailing: Option<f64>,
    pub pe_forward: Option<f64>,
    /// Trailing twelve months earnings per share.
    pub eps_ttm: Option<f64>,
    pub price_to_book: Option<f64>,
    /// Trailing dividend yield in percent, e.g. `6.27`.
    pub dividend_yield_pct: Option<f64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    pub sma50: Option<f64>,
    pub sma200: Option<f64>,
    pub shares_outstanding: Option<u64>,
    /// `REGULAR`, `CLOSED`, `PRE`, `POST`, ... as reported by the source.
    pub market_state: Option<String>,
    /// Time of the last trade, RFC 3339 in WIB (UTC+7).
    pub market_time: Option<DateTime<FixedOffset>>,
    /// Which endpoint produced this row, e.g. `yahoo:quote` or `yahoo:chart`.
    pub source: String,
}

impl Quote {
    pub fn empty(symbol: Symbol, source: &str) -> Self {
        Quote {
            symbol,
            name: None,
            currency: None,
            price: None,
            change: None,
            change_pct: None,
            previous_close: None,
            open: None,
            day_high: None,
            day_low: None,
            volume: None,
            avg_volume_3m: None,
            market_cap: None,
            pe_trailing: None,
            pe_forward: None,
            eps_ttm: None,
            price_to_book: None,
            dividend_yield_pct: None,
            week52_high: None,
            week52_low: None,
            sma50: None,
            sma200: None,
            shares_outstanding: None,
            market_state: None,
            market_time: None,
            source: source.to_string(),
        }
    }
}

/// Look-back window for `stocks history`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
pub enum Period {
    #[value(name = "1d")]
    #[serde(rename = "1d")]
    D1,
    #[value(name = "5d")]
    #[serde(rename = "5d")]
    D5,
    #[value(name = "1mo")]
    #[serde(rename = "1mo")]
    M1,
    #[value(name = "3mo")]
    #[serde(rename = "3mo")]
    M3,
    #[value(name = "6mo")]
    #[serde(rename = "6mo")]
    M6,
    #[value(name = "1y")]
    #[serde(rename = "1y")]
    Y1,
    #[value(name = "2y")]
    #[serde(rename = "2y")]
    Y2,
    #[value(name = "5y")]
    #[serde(rename = "5y")]
    Y5,
    #[value(name = "10y")]
    #[serde(rename = "10y")]
    Y10,
    #[value(name = "ytd")]
    #[serde(rename = "ytd")]
    Ytd,
    #[value(name = "max")]
    #[serde(rename = "max")]
    Max,
}

impl Period {
    pub fn as_str(&self) -> &'static str {
        match self {
            Period::D1 => "1d",
            Period::D5 => "5d",
            Period::M1 => "1mo",
            Period::M3 => "3mo",
            Period::M6 => "6mo",
            Period::Y1 => "1y",
            Period::Y2 => "2y",
            Period::Y5 => "5y",
            Period::Y10 => "10y",
            Period::Ytd => "ytd",
            Period::Max => "max",
        }
    }
}

impl std::str::FromStr for Period {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <Period as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            IdxError::Usage(format!(
                "invalid period {s:?}; use 1d,5d,1mo,3mo,6mo,1y,2y,5y,10y,ytd,max"
            ))
        })
    }
}

impl std::fmt::Display for Period {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Candle width for `stocks history`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
pub enum Interval {
    #[value(name = "5m")]
    #[serde(rename = "5m")]
    Min5,
    #[value(name = "15m")]
    #[serde(rename = "15m")]
    Min15,
    #[value(name = "30m")]
    #[serde(rename = "30m")]
    Min30,
    #[value(name = "1h")]
    #[serde(rename = "1h")]
    Hour1,
    #[value(name = "1d")]
    #[serde(rename = "1d")]
    Day1,
    #[value(name = "1wk")]
    #[serde(rename = "1wk")]
    Week1,
    #[value(name = "1mo")]
    #[serde(rename = "1mo")]
    Month1,
}

impl Interval {
    pub fn as_str(&self) -> &'static str {
        match self {
            Interval::Min5 => "5m",
            Interval::Min15 => "15m",
            Interval::Min30 => "30m",
            Interval::Hour1 => "1h",
            Interval::Day1 => "1d",
            Interval::Week1 => "1wk",
            Interval::Month1 => "1mo",
        }
    }

    pub fn is_intraday(&self) -> bool {
        matches!(
            self,
            Interval::Min5 | Interval::Min15 | Interval::Min30 | Interval::Hour1
        )
    }
}

impl std::str::FromStr for Interval {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <Interval as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            IdxError::Usage(format!(
                "invalid interval {s:?}; use 5m,15m,30m,1h,1d,1wk,1mo"
            ))
        })
    }
}

impl std::fmt::Display for Interval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One OHLCV bar. `time` is the bar's start in WIB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Candle {
    pub time: DateTime<FixedOffset>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    /// Split/dividend adjusted close when the source supplies it.
    pub adj_close: Option<f64>,
    pub volume: u64,
}

impl Candle {
    pub fn date(&self) -> NaiveDate {
        self.time.date_naive()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HistorySummary {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub first_close: f64,
    pub last_close: f64,
    pub change: f64,
    pub change_pct: f64,
    pub high: f64,
    pub low: f64,
    pub avg_volume: u64,
    pub candles: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct History {
    pub symbol: Symbol,
    pub name: Option<String>,
    pub currency: Option<String>,
    pub period: Period,
    pub interval: Interval,
    /// Oldest first.
    pub candles: Vec<Candle>,
    /// `null` when there are no candles.
    pub summary: Option<HistorySummary>,
}

impl History {
    pub fn summarise(candles: &[Candle]) -> Option<HistorySummary> {
        let first = candles.first()?;
        let last = candles.last()?;
        let high = candles.iter().map(|c| c.high).fold(f64::MIN, f64::max);
        let low = candles.iter().map(|c| c.low).fold(f64::MAX, f64::min);
        let vol: u128 = candles.iter().map(|c| c.volume as u128).sum();
        let change = last.close - first.close;
        let change_pct = if first.close != 0.0 {
            change / first.close * 100.0
        } else {
            0.0
        };
        Some(HistorySummary {
            start: first.time,
            end: last.time,
            first_close: first.close,
            last_close: last.close,
            change,
            change_pct,
            high,
            low,
            avg_volume: (vol / candles.len() as u128) as u64,
            candles: candles.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_normalises() {
        assert_eq!(Symbol::parse(" bbca ").unwrap().as_str(), "BBCA");
        assert_eq!(Symbol::parse("BBRI.JK").unwrap().as_str(), "BBRI");
        assert_eq!(Symbol::parse("bbri.jk").unwrap().yahoo(), "BBRI.JK");
        assert!(Symbol::parse("").is_err());
        assert!(Symbol::parse("B B").is_err());
        assert!(Symbol::parse("BBCA$").is_err());
        let ix = Symbol::parse("^jkse").unwrap();
        assert!(ix.is_index());
        assert_eq!(ix.yahoo(), "^JKSE");
        assert_eq!(Symbol::from_yahoo("^JKSE").as_str(), "^JKSE");
        assert!(Symbol::parse("^").is_err());
        assert!(!Symbol::parse("BBCA").unwrap().is_index());
    }

    #[test]
    fn symbol_list_dedupes_and_splits() {
        let v = Symbol::parse_list(["BBCA,bbri", "TLKM BBCA"]).unwrap();
        let names: Vec<_> = v.iter().map(|s| s.as_str().to_string()).collect();
        assert_eq!(names, ["BBCA", "BBRI", "TLKM"]);
        assert!(Symbol::parse_list([" , "]).is_err());
    }

    #[test]
    fn period_and_interval_roundtrip() {
        for p in [
            "1d", "5d", "1mo", "3mo", "6mo", "1y", "2y", "5y", "10y", "ytd", "max",
        ] {
            assert_eq!(p.parse::<Period>().unwrap().as_str(), p);
        }
        for i in ["5m", "15m", "30m", "1h", "1d", "1wk", "1mo"] {
            assert_eq!(i.parse::<Interval>().unwrap().as_str(), i);
        }
        assert!("7d".parse::<Period>().is_err());
        assert_eq!(serde_json::to_string(&Period::M1).unwrap(), "\"1mo\"");
        assert_eq!(serde_json::to_string(&Interval::Week1).unwrap(), "\"1wk\"");
    }

    #[test]
    fn summary_math() {
        let t = wib_from_unix(1_789_524_000).unwrap();
        let mk = |close: f64, vol: u64| Candle {
            time: t,
            open: close,
            high: close + 10.0,
            low: close - 10.0,
            close,
            adj_close: None,
            volume: vol,
        };
        let s = History::summarise(&[mk(100.0, 10), mk(110.0, 30)]).unwrap();
        assert_eq!(s.change, 10.0);
        assert!((s.change_pct - 10.0).abs() < 1e-9);
        assert_eq!(s.high, 120.0);
        assert_eq!(s.low, 90.0);
        assert_eq!(s.avg_volume, 20);
        assert!(History::summarise(&[]).is_none());
    }
}
