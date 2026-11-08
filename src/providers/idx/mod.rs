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
//! history. The site sits behind Cloudflare, whose bot protection now and
//! then refuses a request from any network (1 in 10 from a home connection
//! on 2026-11-08; the same request passed 25 s later) and refuses
//! data-centre addresses (CI runners, Cloudflare Workers) more often, with a
//! "Just a moment" challenge or a 403 "Attention Required" block page
//! ([`http::cloudflare`](crate::http::cloudflare)). A challenged request is
//! asked again three times within 30 s (`CHALLENGE_PAUSES`); one met by a
//! block page is not. A refusal that stays is a failed fetch: the cached
//! copy is served in its place, marked stale with a warning naming it, and
//! with no copy it becomes a `blocked` error (exit 3) naming the cause. An
//! empty copy of a day's summary stands in for nothing, since it may predate
//! the summary: the error stands then too. Any other failure keeps its HTTP
//! status as a `provider` error. Answers are cached like any other;
//! refusals never are.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{Datelike, NaiveDate};

use crate::error::{Advice, IdxError, Result};
use crate::http::cloudflare::{self, Refusal};
use crate::http::{CacheMode, Fetched, Fetcher, RawResponse};
use crate::model::{CalendarEvent, IndexRow, Instrument, Member, StaleReason, Symbol};

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
/// Pauses before asking again after a Cloudflare challenge. One met from a
/// home connection on 2026-11-08 had lifted when the next request came 25 s
/// later, so the tries come at 0, 5, 15 and 30 s: four tries and at most
/// 30 s of pauses per request, plus the requests, which a refusal answers
/// in a fraction of a second. A block page is not asked again. No pauses in
/// tests.
const CHALLENGE_PAUSES: [Duration; 3] = if cfg!(test) {
    [Duration::ZERO; 3]
} else {
    [
        Duration::from_secs(5),
        Duration::from_secs(10),
        Duration::from_secs(15),
    ]
};

/// What to do when a challenge stays and nothing is cached. Most pass
/// within a minute, but some networks are challenged on every request.
/// Like every hint here it ends without a full stop: callers append
/// `; ...`.
pub const BLOCKED_HINT: &str = "idx.co.id's bot protection refused the request, which usually passes within a minute: retry then, and from another network if it keeps refusing (data centres, VPNs and CI runners are refused more often)";
/// The same for a block page, which can last longer.
pub const BLOCK_PAGE_HINT: &str = "idx.co.id's bot protection refused the request; a block page can last longer than a challenge: retry later, or from another network if it persists (data centres, VPNs and CI runners are refused more often)";
/// Why no cached copy stood in under `--no-cache`.
pub const NO_CACHE: Advice = Advice {
    cli: ", and --no-cache skipped the cache (--offline serves a cached copy if there is one)",
    mcp: ", and the server runs with --no-cache, which skips the cache",
};

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
fn refused(r: &RawResponse) -> Option<Refusal> {
    cloudflare::refusal(r.status, &r.headers, r.body.as_bytes())
        .or((r.status == 403).then_some(Refusal::Block))
}

/// What the tries met, for the error and for a cached copy's warning. Only
/// a challenge is asked again, so every try before the last met one:
/// `idx.co.id answered with a Cloudflare challenge (HTTP 403, cf-mitigated:
/// challenge) on 4 tries over 30 s`, or, when the last met a block page,
/// `idx.co.id answered with a Cloudflare block page (HTTP 403) on try 2,
/// after a challenge 5 s earlier`.
fn refusal_text(kind: Refusal, r: &RawResponse, tries: usize, waited: Duration) -> String {
    let what = cloudflare::describe(kind, r.status, &r.headers);
    let secs = waited.as_secs();
    match (kind, tries) {
        (_, 1) => format!("idx.co.id answered with {what}"),
        (Refusal::Challenge, n) => {
            format!("idx.co.id answered with {what} on {n} tries over {secs} s")
        }
        (Refusal::Block, 2) => {
            format!("idx.co.id answered with {what} on try 2, after a challenge {secs} s earlier")
        }
        (Refusal::Block, n) => format!(
            "idx.co.id answered with {what} on try {n}, after challenges on the {} tries before it, the first {secs} s earlier",
            n - 1
        ),
    }
}

fn hint(kind: Refusal) -> &'static str {
    match kind {
        Refusal::Challenge => BLOCKED_HINT,
        Refusal::Block => BLOCK_PAGE_HINT,
    }
}

/// A refusal that stayed with nothing cached to stand in for it, and what
/// to do now. A cached copy would have been served, unless `--no-cache`
/// kept the fetcher from looking.
fn advise(e: IdxError, mode: CacheMode) -> IdxError {
    let IdxError::Blocked {
        provider,
        refusal,
        message,
    } = e
    else {
        return e;
    };
    let cache = match mode {
        CacheMode::Bypass => NO_CACHE.cli,
        _ => ", and nothing is cached for it",
    };
    IdxError::Blocked {
        provider,
        refusal,
        message: format!("{message}{cache}. {}", hint(refusal)),
    }
}

/// The failed fetch a cached copy was served in place of. A copy can be
/// too old to answer: an empty day's summary may predate the summary, a
/// history may stop before the sessions asked for. The caller then fails
/// with [`error`](Self::error), as with nothing cached.
#[derive(Debug, Clone)]
pub struct Failure {
    /// The refusal the last try met; `None` for another failure, such as a
    /// network error.
    refusal: Option<Refusal>,
    /// What the live fetch met, as the copy's warning names it.
    cause: String,
    /// When the copy was fetched.
    cached_at: chrono::DateTime<chrono::Utc>,
}

impl Failure {
    /// The error the failure is when its copy cannot answer, `lacks` saying
    /// why: `is empty` reads `..., and the copy cached at 2026-11-09 10:02
    /// WIB is empty`, then what to do after a refusal.
    pub fn error(&self, lacks: &str) -> IdxError {
        let at = self
            .cached_at
            .with_timezone(&crate::model::wib())
            .format("%Y-%m-%d %H:%M WIB");
        let copy = format!("the copy cached at {at} {lacks}");
        match self.refusal {
            Some(kind) => IdxError::blocked(
                NAME,
                kind,
                format!("{}, and {copy}. {}", self.cause, hint(kind)),
            ),
            None => IdxError::Network(format!("{}, and {copy}", self.cause)),
        }
    }
}

/// [`ask`], for callers that any cached copy answers.
async fn get(fetcher: &Arc<Fetcher>, path: &str, ttl: Duration) -> Result<Fetched> {
    Ok(ask(fetcher, path, ttl).await?.0)
}

/// One idx.co.id endpoint. A request met by a Cloudflare challenge is asked
/// again after each of [`CHALLENGE_PAUSES`]; a challenge that stays, or a
/// block page, is a failed fetch: the cached copy stands in for it, marked
/// stale with a warning naming the refusal, or the call fails with a
/// `blocked` error. A copy that stands in for a failed fetch comes with
/// that [`Failure`].
async fn ask(
    fetcher: &Arc<Fetcher>,
    path: &str,
    ttl: Duration,
) -> Result<(Fetched, Option<Failure>)> {
    let url = format!("{BASE}/{path}");
    let headers = headers();
    let (mut tries, mut waited) = (1, Duration::ZERO);
    let (got, refusal) = loop {
        // The refusal this try met. The fetcher has already turned it into
        // the cached copy or the error; asking again replaces either.
        let seen = OnceLock::new();
        let refuse = |r: &RawResponse| {
            let kind = refused(r)?;
            let _ = seen.set(kind);
            Some(IdxError::blocked(
                NAME,
                kind,
                refusal_text(kind, r, tries, waited),
            ))
        };
        let got = fetcher.get_unless(&url, &headers, ttl, None, &refuse).await;
        let seen = seen.get().copied();
        match (seen, CHALLENGE_PAUSES.get(tries - 1)) {
            (Some(Refusal::Challenge), Some(&pause)) => {
                tracing::info!(
                    path,
                    tries,
                    "idx.co.id answered a Cloudflare challenge; asking again in {} s",
                    pause.as_secs()
                );
                {
                    let _pause = crate::progress::retrying(&url, tries);
                    tokio::time::sleep(pause).await;
                }
                tries += 1;
                waited += pause;
            }
            _ => break (got, seen),
        }
    };
    let f = got.map_err(|e| advise(e, fetcher.mode()))?;
    if !f.is_success() {
        return Err(IdxError::provider(
            NAME,
            format!("{path}: HTTP {}", f.status),
        ));
    }
    let failure = (f.stale_reason == Some(StaleReason::FetchFailed)).then(|| Failure {
        refusal,
        cause: f
            .failure
            .clone()
            .unwrap_or_else(|| "the live fetch failed".into()),
        cached_at: f.fetched_at,
    });
    Ok((f, failure))
}

/// One day's summary at `path`. An empty list is a day without trading,
/// except in a copy served in place of a failed fetch: cached before the
/// summary was out, it answers nothing, and the failure stands as with
/// nothing cached.
async fn day<T>(
    fetcher: &Arc<Fetcher>,
    path: &str,
    date: NaiveDate,
    today: NaiveDate,
    parse: fn(&str) -> Result<Vec<T>>,
) -> Result<Sourced<Vec<T>>> {
    let (f, failure) = ask(fetcher, path, day_ttl(date, today)).await?;
    let rows = parse(&f.body)?;
    if let Some(failure) = failure.filter(|_| rows.is_empty()) {
        return Err(failure.error("is empty"));
    }
    Ok(sourced(rows, &f))
}

fn sourced<T>(data: T, f: &Fetched) -> Sourced<T> {
    Sourced::from_fetched(data, NAME, f)
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
    day(fetcher, &path, date, today, parse::stock_summary).await
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
    day(fetcher, &path, date, today, parse::broker_summary).await
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
    day(fetcher, &path, date, today, parse::index_summary).await
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
        let (f, be, _d) = fetcher(
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
        assert_eq!(be.call_count(), 1, "a block page is not asked again");
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
        let (f, be, _d) = fetcher(
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
        // Asked again after each pause: four tries in all.
        assert!(err.to_string().contains(" on 4 tries over "), "{err}");
        assert_eq!(be.calls_matching("GetBrokerSummary"), 4);
        // A 503 without Cloudflare's pages is an outage: its status, no Cloudflare.
        let err = index_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("provider", 3));
        assert!(err.to_string().contains("HTTP 503"), "{err}");
        assert!(!err.to_string().contains("Cloudflare"), "{err}");
        assert_eq!(be.calls_matching("GetIndexSummary"), 1);
        // idx.co.id never answers 403 itself: a bare one is a block.
        let err = stock_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "blocked");
        assert_eq!(be.calls_matching("GetStockSummary"), 1);
    }

    #[tokio::test]
    async fn a_challenge_is_asked_again_until_it_lifts() {
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route_times("GetBrokerSummary", 2, 403, CHALLENGE, FLAGGED)
                .route("GetBrokerSummary", 200, BROKERS),
        );
        let s = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap();
        assert_eq!(s.data.len(), 88);
        assert!(!s.cached && !s.stale && s.warnings.is_empty());
        assert_eq!(be.call_count(), 3);
        // The answer is cached, the challenges are not.
        let again = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap();
        assert!(again.cached && !again.stale);
        assert_eq!(be.call_count(), 3);
    }

    const FLAGGED: &[(&str, &str)] = &[("cf-mitigated", "challenge"), ("server", "cloudflare")];

    fn broker_url(date: &str) -> String {
        format!("{BASE}/TradingSummary/GetBrokerSummary?date={date}&start=0&length=9999")
    }

    fn hours_ago(h: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() - chrono::Duration::seconds(h * 3600)
    }

    #[tokio::test]
    async fn a_refusal_serves_the_cached_copy_marked_stale() {
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route_with_headers("date=20261016", 403, CHALLENGE, FLAGGED)
                .route("date=20261015", 403, BLOCKED),
        );
        // Today's summaries, cached an hour ago: past their 15 minutes.
        for date in ["20261016", "20261015"] {
            f.cache()
                .put_at(&broker_url(date), 200, BROKERS, hours_ago(1))
                .unwrap();
        }
        let s = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 16))
            .await
            .unwrap();
        assert_eq!(s.data.len(), 88);
        assert!(s.cached && s.stale);
        assert_eq!(s.stale_reason, Some(StaleReason::FetchFailed));
        // Only after the last try (tests do not pause: 0 s).
        assert_eq!(
            s.warnings,
            ["live fetch failed (idx.co.id answered with a Cloudflare challenge (HTTP 403, cf-mitigated: challenge) on 4 tries over 0 s); showing cached copy"]
        );
        assert_eq!(be.calls_matching("date=20261016"), 4);
        let s = broker_summary(&f, d(2026, 10, 15), d(2026, 10, 15))
            .await
            .unwrap();
        assert_eq!(
            s.warnings,
            ["live fetch failed (idx.co.id answered with a Cloudflare block page (HTTP 403)); showing cached copy"]
        );
        assert!(s.stale);
        assert_eq!(be.calls_matching("date=20261015"), 1);
    }

    #[tokio::test]
    async fn a_refusal_that_stays_says_what_to_do() {
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route_with_headers("GetBrokerSummary", 403, CHALLENGE, FLAGGED)
                .route("GetIndexSummary", 403, BLOCKED),
        );
        let err = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("blocked", 3));
        assert_eq!(
            err.to_string(),
            "idx: idx.co.id answered with a Cloudflare challenge (HTTP 403, cf-mitigated: challenge) on 4 tries over 0 s, and nothing is cached for it. idx.co.id's bot protection refused the request, which usually passes within a minute: retry then, and from another network if it keeps refusing (data centres, VPNs and CI runners are refused more often)"
        );
        let msg = index_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            msg.starts_with("idx: idx.co.id answered with a Cloudflare block page (HTTP 403), and nothing is cached for it. "),
            "{msg}"
        );
        assert!(
            msg.ends_with("from another network if it persists (data centres, VPNs and CI runners are refused more often)"),
            "{msg}"
        );

        // `--no-cache` never looked at the copy that `--offline` would serve.
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache.put(&broker_url("20261016"), 200, BROKERS).unwrap();
        let be =
            FixtureBackend::new().route_with_headers("GetBrokerSummary", 403, CHALLENGE, FLAGGED);
        let f = Arc::new(Fetcher::new(Arc::new(be), cache, CacheMode::Bypass));
        let msg = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            msg.contains(" over 0 s, and --no-cache skipped the cache (--offline serves a cached copy if there is one). idx.co.id's bot protection"),
            "{msg}"
        );
    }

    const NO_TRADING: &str =
        include_str!("../../../tests/fixtures/idx/stocksummary_20261018_no_trading.json");

    #[tokio::test]
    async fn an_empty_copy_does_not_answer_for_a_refused_day() {
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route("GetBrokerSummary", 403, BLOCKED)
                .route_with_headers("GetIndexSummary", 403, CHALLENGE, FLAGGED),
        );
        // Today's summaries as cached two hours ago, before they were out.
        for endpoint in ["GetBrokerSummary", "GetIndexSummary", "GetStockSummary"] {
            let url = format!("{BASE}/TradingSummary/{endpoint}?date=20261016&start=0&length=9999");
            f.cache()
                .put_at(&url, 200, NO_TRADING, hours_ago(2))
                .unwrap();
        }
        let today = d(2026, 10, 16);
        // The copy cannot tell a day without trading from one not out yet:
        // the refusal stands, as with nothing cached.
        let err = broker_summary(&f, today, today).await.unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("blocked", 3));
        let msg = err.to_string();
        assert!(
            msg.starts_with("idx: idx.co.id answered with a Cloudflare block page (HTTP 403), and the copy cached at "),
            "{msg}"
        );
        assert!(
            msg.contains(
                " WIB is empty. idx.co.id's bot protection refused the request; a block page "
            ),
            "{msg}"
        );
        let err = index_summary(&f, today, today).await.unwrap_err();
        assert_eq!(err.code(), "blocked");
        assert!(
            err.to_string()
                .contains(" on 4 tries over 0 s, and the copy cached at "),
            "{err}"
        );
        // A network failure the same way, as the network error it is.
        let err = stock_summary(&f, today, today).await.unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("network", 3));
        assert!(err.to_string().ends_with(" WIB is empty"), "{err}");
    }

    #[tokio::test]
    async fn a_block_page_after_challenges_says_what_each_try_met() {
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route_times("date=20261016", 1, 403, CHALLENGE, FLAGGED)
                .route("date=20261016", 403, BLOCKED)
                .route_times("date=20261015", 3, 403, CHALLENGE, FLAGGED)
                .route("date=20261015", 403, BLOCKED),
        );
        let err = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert!(
            err.to_string().starts_with("idx: idx.co.id answered with a Cloudflare block page (HTTP 403) on try 2, after a challenge 0 s earlier, and nothing is cached for it. "),
            "{err}"
        );
        let err = broker_summary(&f, d(2026, 10, 15), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert!(
            err.to_string().starts_with("idx: idx.co.id answered with a Cloudflare block page (HTTP 403) on try 4, after challenges on the 3 tries before it, the first 0 s earlier, and nothing is cached for it. "),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_challenge_page_with_status_200_is_never_cached() {
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route_times(
                    "GetBrokerSummary",
                    1,
                    200,
                    "<html><title>Just a moment...</title>",
                    &[],
                )
                .route("GetBrokerSummary", 200, BROKERS),
        );
        // Had the challenge been cached, the second try would have read it
        // back from the cache instead of asking.
        let s = broker_summary(&f, d(2026, 10, 16), d(2026, 10, 18))
            .await
            .unwrap();
        assert!(!s.cached && s.data.len() == 88);
        assert_eq!(be.call_count(), 2);
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
