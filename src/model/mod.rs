//! Provider-agnostic domain types. Everything a provider returns is normalised
//! into these structs, and these structs are what the table/JSON renderers and
//! the MCP tools consume. Field names here are the JSON contract.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{IdxError, Result};

pub mod actions;
pub mod astro;
pub mod bandar;
pub mod brief;
pub mod broker;
pub mod calendar;
pub mod composition;
pub mod corporate;
pub mod dividends;
pub mod flow;
pub mod fundamentals;
pub mod insights;
pub mod institutions;
pub mod instruments;
pub mod intraday;
pub mod lenses;
pub mod market;
pub mod movements;
pub mod news;
pub mod orderbook;
pub mod ownership;
pub mod ownership_analytics;
pub mod ownership_sync;
pub mod returns;
pub mod screen;
pub mod technical;
pub mod watchlist;

pub use actions::{
    ActionKind, CorporateAction, CorporateActions, ShareholderMeeting, WarrantRight,
};
pub use astro::{
    Aspect, Astro, AstroBacktest, AstroBody, AstroEvent, AstroEventKind, AstroPeriod, AstroWindow,
    BacktestFactor, BacktestGroup, Body, IhsgChart, MercuryState, MoonPhase, MoonState, NatalBody,
    NatalChart, NatalChoice, RetrogradePeriod, Shio, Transit, Weton, ZodiacSign,
};
pub use bandar::{
    BandarDaily, BandarDay, BandarPeriod, BandarPhase, BandarReport, BandarScreen, BandarScreenRow,
    BandarSignal, BandarTier,
};
pub use brief::{Brief, DividendSummary, NewsSummary, OwnershipSummary};
pub use broker::{
    BrokerBoard, BrokerFlow, BrokerInvestor, BrokerKind, BrokerSummary, BrokerSummaryChoice,
    BrokerSummarySource,
};
pub use calendar::{Calendar, CalendarCategory, CalendarEvent, Holiday, Holidays};
pub use composition::{CompositionPoint, CompositionReport, CompositionType};
pub use corporate::{CorporateEvent, CorporateEventKind, CorporateEvents};
pub use dividends::{DividendEvent, DividendHistory, DividendYear, SplitEvent};
pub use flow::{
    BrokerActivity, BrokerDayRow, BrokerRanking, BrokerSort, BrokerSource, BrokerStockRow,
    ForeignDay, ForeignFlow, ForeignMarketValue, ForeignRow, ForeignSort, ForeignSource,
    ForeignTotals, ForeignValueDay, ForeignValueFlow, ForeignValueRow, ForeignValueTotals,
    MarketForeign, MarketForeignValue,
};
pub use fundamentals::{
    Analyst, BalanceSheet, CashFlow, Director, Dividend, Earnings, EarningsEstimate,
    EarningsFilter, EarningsQuarter, EarningsScope, FinancialPeriod, Financials, Fundamentals,
    Growth, Health, IncomeStatement, NextEarnings, PerShare, PeriodKind, Profile, Profitability,
    RevenueEarnings, StatementPeriod, Valuation, Verdict, VerdictLabel,
};
pub use insights::{Insight, InsightBenchmark, InsightStatus, Insights};
pub use institutions::{
    HolderFlow, HolderKind, HolderSignal, InstitutionalFlow, InstitutionalHolder,
    InstitutionalOwnership,
};
pub use instruments::{IndexBoard, IndexRow, Instrument, InstrumentList, Member, MemberList};
pub use intraday::{Intraday, IntradaySummary};
pub use lenses::{
    AnnualGrowth, GrowthLabel, GrowthLens, Lens, LensRule, PriceRisk, Reading, RiskLabel, RiskLens,
    ValuationLabel, ValuationLens,
};
pub use market::{MarketOverview, MarketRow};
pub use movements::{HolderMovement, HolderMovements, MovementFilter, MovementSource};
pub use news::{NewsItem, NewsReport, SentimentLabel, SentimentReport};
pub use orderbook::{BookLevel, OrderBook};
pub use ownership::{
    Change, ChangeBrief, ChangeKind, ChangesReport, ChangesSummary, DiscoveredFile, EntityReport,
    ForeignFlowReport, ForeignPoint, ForeignReport, HoldersReport, Holding, ImportReport,
    IssuerFlow, IssuerForeign, Release, TypeFlow,
};
pub use ownership_analytics::{
    CandidateHolder, CandidateReason, Concentration, ConcentrationReport, ConcentrationSort,
    CrossHolder, CrossHoldersReport, CrossPosition, GraphHolder, GraphIssuer, GraphLink,
    GraphReport, HolderOrigin, IssuerMonth, RenameEvidence, Resolution, ResolutionAction,
    ResolutionKind, ResolutionReport, ResolveCandidate, ResolveCandidates, SearchMatch,
    SearchMatchKind, SearchReport,
};
pub use ownership_sync::{
    InstalledSnapshot, MirrorState, SnapshotManifest, SnapshotRelease, SnapshotSummary,
    StatusReport, SyncAction, SyncNeeded, SyncReport,
};
pub use returns::{BenchmarkReturns, DayMove, Drawdown, Performance, ReturnWindows};
pub use screen::{MarketReturns, ScreenReport, ScreenRow};
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

/// Why a cached copy older than its TTL was served (`stale: true`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StaleReason {
    /// `--offline` (or `IDX_OFFLINE`): the network was not tried.
    Offline,
    /// The live request failed or the source refused it (a Cloudflare
    /// challenge), or it failed moments ago and is not retried yet; a
    /// warning names the failure.
    FetchFailed,
}

/// Yahoo Finance's suffix for IDX listings. A bare ticker means IDX.
pub const IDX_EXCHANGE: &str = "JK";

/// A ticker, stored upper-case. An IDX ticker has no suffix (`BBCA`), an index
/// code keeps its caret (`^JKSE`), and a ticker from another Yahoo Finance
/// market keeps its suffix (`D05.SI`, accepted only by the Yahoo-backed
/// commands through [`Symbol::parse_on`]).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Symbol(String);

/// A code starts with a letter or digit and holds letters, digits and `-`
/// (`BUKA-W`), at most 12 characters.
fn valid_code(body: &str) -> bool {
    body.starts_with(|c: char| c.is_ascii_alphanumeric())
        && body.len() <= 12
        && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// `BASE.XX` with XX one to four ASCII letters: `(BASE, XX)`.
fn split_suffix(s: &str) -> Option<(&str, &str)> {
    let (base, suffix) = s.rsplit_once('.')?;
    let ok = (1..=4).contains(&suffix.len()) && suffix.chars().all(|c| c.is_ascii_alphabetic());
    ok.then_some((base, suffix))
}

impl Symbol {
    /// An IDX ticker: accepts `bbca`, `BBCA`, `BBCA.JK`, ` bbca ` and returns
    /// `BBCA`. Index codes keep their caret: `^JKSE` (IHSG), `^JKLQ45`. A code
    /// starts with a letter or digit (`BUKA-W` is fine, `-` and `-BBCA` are
    /// not, so the `-` that reads stdin never passes as a ticker). Another
    /// market's suffix (`D05.SI`) is refused; see [`Symbol::parse_on`].
    pub fn parse(raw: &str) -> Result<Symbol> {
        let mut s = raw.trim().to_ascii_uppercase();
        if let Some(stripped) = s.strip_suffix(".JK") {
            s = stripped.to_string();
        }
        if s.is_empty() {
            return Err(IdxError::Usage("empty symbol".into()));
        }
        let body = s.strip_prefix('^').unwrap_or(&s);
        if !valid_code(body) {
            let other = split_suffix(&s).filter(|(base, _)| valid_code(base));
            if let Some((_, suffix)) = other {
                return Err(IdxError::Usage(format!(
                    "invalid symbol {raw:?}: listed on another exchange ({suffix}); \
                     this command takes IDX tickers only"
                )));
            }
            return Err(IdxError::Usage(format!(
                "invalid symbol {raw:?}: expected an IDX ticker like BBCA or BBRI"
            )));
        }
        Ok(Symbol(s))
    }

    /// A ticker for the Yahoo-backed commands. An explicit suffix wins: `.JK`
    /// gives the bare IDX ticker, any other (`D05.SI`) is kept. A bare code is
    /// IDX when `exchange` is `JK`, else it gets `.{exchange}` (`exchange` is
    /// a normalised suffix such as `SI`). Index codes (`^STI`) stay as they are.
    pub fn parse_on(raw: &str, exchange: &str) -> Result<Symbol> {
        let s = raw.trim().to_ascii_uppercase();
        if s.starts_with('^') {
            return Symbol::parse(raw);
        }
        let invalid = || {
            IdxError::Usage(format!(
                "invalid symbol {raw:?}: expected a ticker like BBCA, or one with a \
                 Yahoo Finance suffix like D05.SI"
            ))
        };
        if let Some((base, suffix)) = split_suffix(&s) {
            if suffix == IDX_EXCHANGE {
                return Symbol::parse(raw);
            }
            return if valid_code(base) {
                Ok(Symbol(format!("{base}.{suffix}")))
            } else {
                Err(invalid())
            };
        }
        if s.is_empty() || exchange.eq_ignore_ascii_case(IDX_EXCHANGE) {
            return Symbol::parse(raw);
        }
        if valid_code(&s) {
            Ok(Symbol(format!("{s}.{}", exchange.to_ascii_uppercase())))
        } else {
            Err(invalid())
        }
    }

    /// `true` for index codes (`^JKSE`), which have no `.JK` suffix anywhere.
    pub fn is_index(&self) -> bool {
        self.0.starts_with('^')
    }

    /// `true` when the symbol trades on IDX and so follows its sessions:
    /// every plain code without another market's suffix (a `.JK` listing such
    /// as `BBCA` or `IDX30`, not `D05.SI`) and the exchange's own indices
    /// (`^JKSE`, `^JKLQ45`). Other indices (`^GSPC`, `^N225`) and Yahoo's
    /// currency or futures codes (`USDIDR=X`) do not.
    pub fn trades_on_idx(&self) -> bool {
        match self.0.strip_prefix('^') {
            Some(index) => index.starts_with("JK"),
            None => !self.0.contains('=') && self.exchange().is_none(),
        }
    }

    /// The Yahoo Finance suffix of a ticker listed outside IDX (`SI` for
    /// `D05.SI`); `None` for IDX tickers and index codes.
    pub fn exchange(&self) -> Option<&str> {
        if self.is_index() {
            return None;
        }
        self.0.rsplit_once('.').map(|(_, x)| x)
    }

    /// Parses `BBCA,BBRI TLKM` into unique IDX symbols, preserving order.
    pub fn parse_list<I, S>(items: I) -> Result<Vec<Symbol>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::parse_list_with(items, Symbol::parse)
    }

    /// [`Symbol::parse_list`] with [`Symbol::parse_on`].
    pub fn parse_list_on<I, S>(items: I, exchange: &str) -> Result<Vec<Symbol>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::parse_list_with(items, |p| Symbol::parse_on(p, exchange))
    }

    fn parse_list_with<I, S>(
        items: I,
        parse: impl Fn(&str) -> Result<Symbol>,
    ) -> Result<Vec<Symbol>>
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
                let sym = parse(piece)?;
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

    /// Yahoo Finance form, e.g. `BBCA.JK`; indices stay as `^JKSE` and other
    /// markets' tickers keep their suffix (`D05.SI`).
    pub fn yahoo(&self) -> String {
        if self.is_index() || self.exchange().is_some() {
            self.0.clone()
        } else {
            format!("{}.{IDX_EXCHANGE}", self.0)
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
    /// Currency of the financial statements when the source names it (Yahoo's
    /// `financialCurrency`; MSN, Stockbit and the chart fallback do not). When
    /// it differs from `currency`, `price_to_book` divides the price by a book
    /// value in this currency; P/E, EPS and the dividend yield are in the
    /// price's terms.
    #[serde(default)]
    pub financial_currency: Option<String>,
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
    /// `true` when the last trade (`market_time`) is dated before the latest
    /// IDX session that has ended (16:15 WIB; weekends and IDX holidays are
    /// skipped): the stock is suspended or did not trade, or the source lags
    /// the close. `null` when `market_time` is unknown or the symbol does not
    /// trade on IDX (`^GSPC`). Computed when the quote is returned.
    #[serde(default)]
    pub behind_session: Option<bool>,
}

/// Whether a last trade at `last_trade` is dated (in WIB) before `latest`,
/// the latest IDX session that has ended; `None` when the time is unknown.
pub fn behind_session(
    last_trade: Option<DateTime<FixedOffset>>,
    latest: NaiveDate,
) -> Option<bool> {
    last_trade.map(|t| t.with_timezone(&wib()).date_naive() < latest)
}

impl Quote {
    pub fn empty(symbol: Symbol, source: &str) -> Self {
        Quote {
            symbol,
            name: None,
            currency: None,
            financial_currency: None,
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
            behind_session: None,
        }
    }

    /// [`behind_session`] for this quote; `None` off IDX.
    pub fn behind(&self, latest: NaiveDate) -> Option<bool> {
        if !self.symbol.trades_on_idx() {
            return None;
        }
        behind_session(self.market_time, latest)
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
    #[value(name = "1m")]
    #[serde(rename = "1m")]
    Min1,
    #[value(name = "2m")]
    #[serde(rename = "2m")]
    Min2,
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
            Interval::Min1 => "1m",
            Interval::Min2 => "2m",
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
            Interval::Min1
                | Interval::Min2
                | Interval::Min5
                | Interval::Min15
                | Interval::Min30
                | Interval::Hour1
        )
    }

    /// The bar size in seconds for the intraday sizes; `None` for daily and
    /// longer bars, whose length varies.
    pub fn bar_seconds(&self) -> Option<i64> {
        match self {
            Interval::Min1 => Some(60),
            Interval::Min2 => Some(120),
            Interval::Min5 => Some(300),
            Interval::Min15 => Some(900),
            Interval::Min30 => Some(1800),
            Interval::Hour1 => Some(3600),
            Interval::Day1 | Interval::Week1 | Interval::Month1 => None,
        }
    }

    /// Refuses a look-back window the bar size cannot cover. Yahoo serves
    /// one-minute bars for at most 8 days per request (HTTP 422 beyond) and
    /// two-minute bars for 60 days; the larger sizes keep their old behaviour.
    pub fn check_period(&self, period: Period) -> Result<()> {
        match (self, period) {
            (Interval::Min1, Period::D1 | Period::D5) => Ok(()),
            (Interval::Min1, _) => Err(IdxError::Usage(
                "1-minute bars cover at most 5 days; use period 1d or 5d".into(),
            )),
            (Interval::Min2, Period::D1 | Period::D5 | Period::M1) => Ok(()),
            (Interval::Min2, _) => Err(IdxError::Usage(
                "2-minute bars cover at most a month; use period 1d, 5d or 1mo".into(),
            )),
            _ => Ok(()),
        }
    }
}

impl std::str::FromStr for Interval {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <Interval as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            IdxError::Usage(format!(
                "invalid interval {s:?}; use 1m,2m,5m,15m,30m,1h,1d,1wk,1mo"
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
    fn behind_session_rule() {
        let latest = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        let t = |s: &str| Some(DateTime::parse_from_rfc3339(s).unwrap());
        let quote = |sym: &str, at: Option<DateTime<FixedOffset>>| {
            let mut q = Quote::empty(Symbol::from_yahoo(sym), "t");
            q.market_time = at;
            q.behind(latest)
        };
        // BBCA's last close, read the next morning after the next close.
        assert_eq!(quote("BBCA", t("2026-11-05T16:14:43+07:00")), Some(true));
        assert_eq!(quote("BBCA", t("2026-11-06T16:14:43+07:00")), Some(false));
        // UTC stamps compare by their WIB date.
        assert_eq!(quote("BBCA", t("2026-11-06T09:15:00Z")), Some(false));
        assert_eq!(quote("BBCA", t("2026-11-05T17:30:00Z")), Some(false));
        assert_eq!(quote("BBCA", t("2026-11-05T16:59:59Z")), Some(true));
        // WIKA, suspended since February 2025.
        assert_eq!(quote("WIKA", t("2025-02-17T16:14:58+07:00")), Some(true));
        assert_eq!(quote("BBCA", None), None, "no last trade time");
        assert_eq!(quote("^JKSE", t("2026-11-05T16:00:04+07:00")), Some(true));
        assert_eq!(
            quote("IDX30.JK", t("2026-11-06T16:00:04+07:00")),
            Some(false)
        );
        assert_eq!(quote("^GSPC", t("2025-01-02T04:00:00+07:00")), None);
        assert_eq!(quote("^N225", t("2025-01-02T13:45:00+07:00")), None);
        assert_eq!(quote("USDIDR=X", t("2025-01-02T13:45:00+07:00")), None);
        assert!(Symbol::parse("^jklq45").unwrap().trades_on_idx());
        assert!(!Symbol::from_yahoo("^STI").trades_on_idx());
        assert!(!Symbol::from_yahoo("D05.SI").trades_on_idx());
        assert_eq!(behind_session(None, latest), None);
    }

    #[test]
    fn symbol_starts_with_a_letter_or_digit() {
        for bad in ["-", "--", "-BBCA", "^-", "^-JKSE"] {
            let e = Symbol::parse(bad).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{bad}");
        }
        assert_eq!(Symbol::parse("buka-w").unwrap().as_str(), "BUKA-W");
        assert_eq!(Symbol::parse("1A").unwrap().as_str(), "1A");
    }

    #[test]
    fn idx_commands_refuse_other_markets_by_name() {
        let e = Symbol::parse("D05.SI").unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert!(e.to_string().contains("another exchange (SI)"), "{e}");
        let e = Symbol::parse("D05.TOOLONG").unwrap_err();
        assert!(e.to_string().contains("expected an IDX ticker"), "{e}");
        assert_eq!(Symbol::parse("bbca.jk").unwrap().as_str(), "BBCA");
    }

    #[test]
    fn parse_on_honours_explicit_and_default_suffixes() {
        let on = |raw: &str, ex: &str| Symbol::parse_on(raw, ex).map(|s| s.as_str().to_string());
        assert_eq!(on("d05.si", "JK").unwrap(), "D05.SI");
        assert_eq!(on("BBCA", "JK").unwrap(), "BBCA");
        assert_eq!(on("BBCA", "SI").unwrap(), "BBCA.SI");
        assert_eq!(on("D05", "si").unwrap(), "D05.SI");
        assert_eq!(
            on("BBCA.JK", "SI").unwrap(),
            "BBCA",
            ".JK is IDX whatever the default"
        );
        assert_eq!(on("^STI", "SI").unwrap(), "^STI");
        assert_eq!(on("^jkse", "JK").unwrap(), "^JKSE");
        assert_eq!(on("0700.HK", "JK").unwrap(), "0700.HK");
        for bad in ["D05.TOOLONG", "-", "-X.SI", ".SI", "B$CA", "", "A.B.C"] {
            assert!(Symbol::parse_on(bad, "SI").is_err(), "{bad}");
            assert!(Symbol::parse_on(bad, "JK").is_err(), "{bad}");
        }
        // Bare codes take the default, explicit suffixes win, duplicates go.
        let list = Symbol::parse_list_on(["BBCA,D05.SI", "bbca.jk D05"], "SI").unwrap();
        let names: Vec<&str> = list.iter().map(Symbol::as_str).collect();
        assert_eq!(names, ["BBCA.SI", "D05.SI", "BBCA"]);
    }

    #[test]
    fn exchange_and_yahoo_forms() {
        let d05 = Symbol::parse_on("D05.SI", "JK").unwrap();
        assert_eq!(d05.exchange(), Some("SI"));
        assert_eq!(d05.yahoo(), "D05.SI");
        assert_eq!(Symbol::from_yahoo("D05.SI"), d05);
        let bbca = Symbol::parse("BBCA").unwrap();
        assert_eq!(bbca.exchange(), None);
        assert_eq!(bbca.yahoo(), "BBCA.JK");
        assert_eq!(Symbol::from_yahoo("BBCA.JK"), bbca);
        assert_eq!(Symbol::parse("^JKSE").unwrap().exchange(), None);
        assert_eq!(serde_json::to_string(&d05).unwrap(), "\"D05.SI\"");
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
        for i in ["1m", "2m", "5m", "15m", "30m", "1h", "1d", "1wk", "1mo"] {
            assert_eq!(i.parse::<Interval>().unwrap().as_str(), i);
        }
        assert!(
            "1M".parse::<Interval>().is_ok(),
            "clap value names are case-insensitive"
        );
        assert!("3m"
            .parse::<Interval>()
            .unwrap_err()
            .to_string()
            .contains("1m,2m,5m"));
        assert_eq!(serde_json::to_string(&Interval::Min1).unwrap(), "\"1m\"");
        assert!("7d".parse::<Period>().is_err());
        assert_eq!(serde_json::to_string(&Period::M1).unwrap(), "\"1mo\"");
        assert_eq!(serde_json::to_string(&Interval::Week1).unwrap(), "\"1wk\"");
    }

    #[test]
    fn minute_bars_and_their_windows() {
        assert!(Interval::Min1.is_intraday() && Interval::Min2.is_intraday());
        assert!(!Interval::Day1.is_intraday());
        for i in [
            Interval::Min1,
            Interval::Min2,
            Interval::Min5,
            Interval::Min15,
            Interval::Min30,
            Interval::Hour1,
        ] {
            assert!(i.is_intraday() && i.bar_seconds().is_some(), "{i}");
        }
        assert_eq!(Interval::Min2.bar_seconds(), Some(120));
        assert_eq!(Interval::Hour1.bar_seconds(), Some(3600));
        assert_eq!(Interval::Day1.bar_seconds(), None);
        for p in [Period::D1, Period::D5] {
            assert!(Interval::Min1.check_period(p).is_ok());
            assert!(Interval::Min2.check_period(p).is_ok());
        }
        assert!(Interval::Min2.check_period(Period::M1).is_ok());
        let e = Interval::Min1.check_period(Period::M1).unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert!(e.to_string().contains("use period 1d or 5d"), "{e}");
        for p in [Period::M3, Period::Y1, Period::Max, Period::Ytd] {
            assert!(Interval::Min1.check_period(p).is_err());
            assert!(Interval::Min2.check_period(p).is_err());
        }
        // Other sizes keep today's behaviour: the source decides.
        assert!(Interval::Min5.check_period(Period::Y1).is_ok());
        assert!(Interval::Day1.check_period(Period::Max).is_ok());
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
