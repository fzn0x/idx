//! idx.co.id, the exchange's own site: the JSON endpoints behind its pages
//! (`https://www.idx.co.id/primary/...`), no key or login.
//!
//! Endpoints verified 2026-10-18 (see `tests/fixtures/idx/`):
//! - `TradingSummary/GetStockSummary?date=YYYYMMDD&start=0&length=9999`:
//!   every stock's day, including foreign buy and sell **in shares**.
//! - `TradingSummary/GetBrokerSummary?date=...`: every member's volume,
//!   value and frequency, both sides of each trade counted.
//! - `TradingSummary/GetIndexSummary?date=...`: every index's day.
//! - `ListedCompany/GetTradingInfoSS?code=BBCA&start=0&length=N`: one
//!   stock's latest N days in the stock-summary shape, newest first.
//! - `Home/GetCalendar?range=d|m&date=YYYYMMDD[&code=XXXX]`: the exchange
//!   calendar (dividend and rights dates, RUPS, public expose, bonds).
//! - `StockData/GetSecuritiesStock?start=0&length=9999`: listed stocks.
//! - `ExchangeMember/GetBrokerSearch?start=0&length=9999`: exchange members.
//!
//! A day without trading answers an empty list; an unknown code an empty
//! history. The site sits behind Cloudflare: ordinary connections pass,
//! while data-centre addresses (CI runners, Cloudflare Workers) are often
//! refused with a 403 "Attention Required" block page or a "Just a moment"
//! challenge, which becomes a `blocked` error (exit 3) naming the cause
//! ([`http::cloudflare`](crate::http::cloudflare)). Any other failure keeps
//! its HTTP status as a `provider` error. Fetched responses are cached like
//! any other.

use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, NaiveDate};

use crate::error::{IdxError, Result};
use crate::http::cloudflare::{self, Refusal};
use crate::http::{Fetched, Fetcher};
use crate::model::{CalendarEvent, IndexRow, Instrument, Member, Symbol};

use super::Sourced;

pub mod parse;

pub use parse::{BrokerDay, StockDay};

pub const NAME: &str = "idx";
const BASE: &str = "https://www.idx.co.id/primary";

/// A finished session's figures do not change.
const TTL_CLOSED: Duration = Duration::from_secs(30 * 24 * 3600);
/// Today's summary fills in after the close.
const TTL_TODAY: Duration = Duration::from_secs(15 * 60);
/// Listed stocks and members change a few times a month.
const TTL_DIRECTORY: Duration = Duration::from_secs(24 * 3600);
/// Calendar entries are added through the day.
const TTL_CALENDAR: Duration = Duration::from_secs(3 * 3600);
/// A stock's recent history, which gains a row after each close.
const TTL_HISTORY: Duration = Duration::from_secs(30 * 60);

pub const BLOCKED_HINT: &str = "idx.co.id refused the request (Cloudflare bot protection). It usually passes later or from an ordinary home or office connection; data centres, VPNs and CI runners are often blocked. Cached copies are still served (try --offline)";

fn headers() -> Vec<(String, String)> {
    vec![
        ("Referer".to_string(), "https://www.idx.co.id/".to_string()),
        (
            "Accept-Language".to_string(),
            "id-ID,id;q=0.9,en;q=0.8".to_string(),
        ),
    ]
}

/// Cloudflare's block and challenge pages. Every idx.co.id response passes
/// through Cloudflare and the origin never answers 403 itself, so a bare 403
/// counts as a block too.
fn refused(f: &Fetched) -> Option<Refusal> {
    cloudflare::refusal(f.status, &f.headers, f.body.as_bytes())
        .or((f.status == 403).then_some(Refusal::Block))
}

async fn get(fetcher: &Arc<Fetcher>, path: &str, ttl: Duration) -> Result<Fetched> {
    let url = format!("{BASE}/{path}");
    let f = fetcher.get(&url, &headers(), ttl, None).await?;
    if let Some(r) = refused(&f) {
        return Err(IdxError::blocked(
            NAME,
            r,
            format!(
                "idx.co.id answered with {}. {BLOCKED_HINT}",
                cloudflare::describe(r, f.status, &f.headers)
            ),
        ));
    }
    if !f.is_success() {
        return Err(IdxError::provider(
            NAME,
            format!("{path}: HTTP {}", f.status),
        ));
    }
    Ok(f)
}

fn sourced<T>(data: T, f: &Fetched) -> Sourced<T> {
    let mut s = Sourced::fresh(data, NAME);
    s.cached = f.cached;
    s.stale = f.stale;
    s.fetched_at = f.fetched_at;
    s
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y%m%d").to_string()
}

fn day_ttl(date: NaiveDate, today: NaiveDate) -> Duration {
    if date < today {
        TTL_CLOSED
    } else {
        TTL_TODAY
    }
}

/// Every stock's day on `date`; empty when nothing traded.
pub async fn stock_summary(
    fetcher: &Arc<Fetcher>,
    date: NaiveDate,
    today: NaiveDate,
) -> Result<Sourced<Vec<StockDay>>> {
    let path = format!(
        "TradingSummary/GetStockSummary?date={}&start=0&length=9999",
        ymd(date)
    );
    let f = get(fetcher, &path, day_ttl(date, today)).await?;
    Ok(sourced(parse::stock_summary(&f.body)?, &f))
}

/// Every member's day on `date`; empty when nothing traded.
pub async fn broker_summary(
    fetcher: &Arc<Fetcher>,
    date: NaiveDate,
    today: NaiveDate,
) -> Result<Sourced<Vec<BrokerDay>>> {
    let path = format!(
        "TradingSummary/GetBrokerSummary?date={}&start=0&length=9999",
        ymd(date)
    );
    let f = get(fetcher, &path, day_ttl(date, today)).await?;
    Ok(sourced(parse::broker_summary(&f.body)?, &f))
}

/// Every index's day on `date`; empty when nothing traded.
pub async fn index_summary(
    fetcher: &Arc<Fetcher>,
    date: NaiveDate,
    today: NaiveDate,
) -> Result<Sourced<Vec<IndexRow>>> {
    let path = format!(
        "TradingSummary/GetIndexSummary?date={}&start=0&length=9999",
        ymd(date)
    );
    let f = get(fetcher, &path, day_ttl(date, today)).await?;
    Ok(sourced(parse::index_summary(&f.body)?, &f))
}

/// The latest `days` sessions of one stock, newest first. An unknown code
/// is `NotFound`.
pub async fn trading_history(
    fetcher: &Arc<Fetcher>,
    symbol: &Symbol,
    days: usize,
) -> Result<Sourced<Vec<StockDay>>> {
    let path = format!(
        "ListedCompany/GetTradingInfoSS?code={}&start=0&length={days}",
        symbol.as_str()
    );
    let f = get(fetcher, &path, TTL_HISTORY).await?;
    let rows = parse::trading_history(&f.body)?;
    if rows.is_empty() {
        return Err(IdxError::NotFound(format!(
            "idx.co.id has no trading history for {symbol}"
        )));
    }
    Ok(sourced(rows, &f))
}

/// Which slice of the calendar to ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarRange {
    Day(NaiveDate),
    /// The month containing the date.
    Month(NaiveDate),
}

/// Calendar entries for a day or a month, optionally for one issuer.
pub async fn calendar(
    fetcher: &Arc<Fetcher>,
    range: CalendarRange,
    code: Option<&str>,
) -> Result<Sourced<Vec<CalendarEvent>>> {
    let (r, date) = match range {
        CalendarRange::Day(d) => ("d", d),
        CalendarRange::Month(d) => ("m", d.with_day0(0).unwrap_or(d)),
    };
    let mut path = format!(
        "Home/GetCalendar?start=1&length=9999&range={r}&date={}&lang=id",
        ymd(date)
    );
    if let Some(c) = code {
        path.push_str(&format!("&code={}", c.trim().to_ascii_uppercase()));
    }
    let f = get(fetcher, &path, TTL_CALENDAR).await?;
    Ok(sourced(parse::calendar(&f.body)?, &f))
}

/// Every listed stock.
pub async fn securities(fetcher: &Arc<Fetcher>) -> Result<Sourced<Vec<Instrument>>> {
    let f = get(
        fetcher,
        "StockData/GetSecuritiesStock?start=0&length=9999&code=&sector=&board=&language=id-id",
        TTL_DIRECTORY,
    )
    .await?;
    Ok(sourced(parse::securities(&f.body)?, &f))
}

/// Every exchange member.
pub async fn members(fetcher: &Arc<Fetcher>) -> Result<Sourced<Vec<Member>>> {
    let f = get(
        fetcher,
        "ExchangeMember/GetBrokerSearch?option=0&license=&start=0&length=9999",
        TTL_DIRECTORY,
    )
    .await?;
    Ok(sourced(parse::members(&f.body)?, &f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::http::{CacheMode, FixtureBackend};

    const BROKERS: &str = include_str!("../../../tests/fixtures/idx/brokersummary_20261016.json");
    const BLOCKED: &str = include_str!("../../../tests/fixtures/idx/cloudflare_blocked.html");
    const CHALLENGE: &str =
        include_str!("../../../tests/fixtures/idx/cloudflare_challenge_synthetic.html");
    const HISTORY_EMPTY: &str =
        include_str!("../../../tests/fixtures/idx/tradinginfoss_zzzz_empty.json");

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn fetcher(be: FixtureBackend) -> (Arc<Fetcher>, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(be);
        let f = Arc::new(Fetcher::new(
            be.clone(),
            Cache::new(dir.path()),
            CacheMode::Normal,
        ));
        (f, be, dir)
    }

    #[tokio::test]
    async fn fetches_with_dates_in_the_url_and_caches() {
        let (f, be, _d) = fetcher(FixtureBackend::new().route(
            "https://www.idx.co.id/primary/TradingSummary/GetBrokerSummary?date=20261016&start=0&length=9999",
            200,
            BROKERS,
        ));
        let s = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap();
        assert_eq!(s.provider, "idx");
        assert_eq!(s.data.len(), 88);
        assert!(!s.cached);
        let again = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap();
        assert!(again.cached);
        assert_eq!(be.call_count(), 1);
    }

    #[tokio::test]
    async fn cloudflare_block_names_the_cause_and_unknown_codes_are_not_found() {
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route("GetBrokerSummary", 403, BLOCKED)
                .route("GetTradingInfoSS?code=ZZZZ", 200, HISTORY_EMPTY)
                .route("GetIndexSummary", 500, "oops"),
        );
        let err = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert_eq!(err.code(), "blocked");
        assert!(
            err.to_string().contains("Cloudflare block page (HTTP 403)"),
            "{err}"
        );
        let err = trading_history(&f, &Symbol::parse("ZZZZ").unwrap(), 20)
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        let err = index_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("HTTP 500"), "{err}");
        assert_eq!(err.code(), "provider");
    }

    #[tokio::test]
    async fn a_challenge_is_blocked_and_an_outage_is_not_cloudflare() {
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route_with_headers(
                    "GetBrokerSummary",
                    403,
                    CHALLENGE,
                    &[("cf-mitigated", "challenge"), ("server", "cloudflare")],
                )
                .route("GetIndexSummary", 503, "oops")
                .route("GetStockSummary", 403, ""),
        );
        let err = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("blocked", 3));
        assert!(
            err.to_string()
                .contains("a Cloudflare challenge (HTTP 403, cf-mitigated: challenge)"),
            "{err}"
        );
        assert!(matches!(
            err,
            IdxError::Blocked {
                refusal: Refusal::Challenge,
                ..
            }
        ));
        // A 503 without Cloudflare's pages is an outage: its status, no Cloudflare.
        let err = index_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("provider", 3));
        assert!(err.to_string().contains("HTTP 503"), "{err}");
        assert!(!err.to_string().contains("Cloudflare"), "{err}");
        // idx.co.id never answers 403 itself: a bare one is a block.
        let err = stock_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "blocked");
    }

    #[tokio::test]
    async fn calendar_asks_for_the_first_of_the_month_and_the_code() {
        let (f, be, _d) =
            fetcher(FixtureBackend::new().route("GetCalendar", 200, "{\"Results\":[]}"));
        calendar(&f, CalendarRange::Month(d(2026, 10, 18)), Some("bbca"))
            .await
            .unwrap();
        calendar(&f, CalendarRange::Day(d(2026, 10, 19)), None)
            .await
            .unwrap();
        let calls = be.calls.lock().unwrap().clone();
        assert!(calls[0].contains("range=m&date=20261001") && calls[0].ends_with("&code=BBCA"));
        assert!(calls[1].contains("range=d&date=20261019"));
    }
}
