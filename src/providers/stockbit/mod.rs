//! Stockbit provider: realtime IDX data from `exodus.stockbit.com`, the
//! backend Stockbit's own web and desktop apps use.
//!
//! This is the one provider that needs a credential, and the one deliberate
//! exception to idx's "free public endpoints, no keys" rule: it carries the
//! user's own Stockbit session, obtained once with `idx stockbit login`. It is
//! never in the `auto` chain and is used only when selected with
//! `--provider stockbit` (or `IDX_PROVIDER=stockbit`).
//!
//! Endpoints (unofficial, documented in `docs/stockbit.md`):
//! - `GET /company-price-feed/v2/orderbook/companies/{SYM}` — quote (OHLC,
//!   volume, value, foreign flow) and the full depth ladder.
//! - `GET /charts/{SYM}/daily?timeframe=..&chart_type=PRICE_CHART_TYPE_CANDLE`
//!   — OHLCV candles; Stockbit picks the granularity from the window.
//! - `GET /corpaction/{type}[?symbol=SYM]` (`corpaction.rs`): corporate
//!   actions, one kind per request; paced a second apart, cached 6 h.
//! - `GET /insider/company/majorholder?page=N&..` (`movement.rs`): insider
//!   and 5 % holder movements, 50 rows a page; paced, cached 1 h.
//! - `POST /login/refresh` — rotate the session (see `auth.rs`).
//!
//! Only GETs under a short allowlist of read-only paths ([`READ_PREFIXES`])
//! ever leave with the bearer: the same API places orders and moves cash. A
//! 401, or a 403 with a JSON body, is a refused token (one refresh, one
//! retry); an HTML 403 is Stockbit's CDN refusing this client, which no
//! refresh changes, so it is reported and never retried.
//!
//! The order book's foreign flow is kept only while Stockbit marks it current
//! (`has_foreign_bs: true`); otherwise it is left empty and the envelope says
//! why (a book recorded during trading with the flag `false` still held the
//! previous session's figures).
//!
//! Symbols are IDX codes (`BBCA`); indices use Stockbit's own code (`IHSG`,
//! `LQ45`), not idx's caret form.
//!
//! Recorded responses live in `tests/fixtures/stockbit/`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::join_all;

use crate::error::{IdxError, Result};
use crate::http::{Fetched, Fetcher};
use crate::model::{
    CorporateEvent, CorporateEventKind, History, HolderMovement, Interval, OrderBook, Period,
    Quote, Symbol,
};

use super::{Provider, Sourced, Ttls};

pub mod auth;
pub mod corpaction;
pub mod login;
pub mod movement;
pub mod page;
mod parse;
pub mod signin;

pub use auth::{Session, SessionStore, Status};

pub const NAME: &str = "stockbit";
const EXODUS: &str = "https://exodus.stockbit.com";
/// Realtime figures move constantly; cache them only long enough to coalesce a
/// burst of requests, regardless of the configured quote TTL.
const REALTIME_TTL: Duration = Duration::from_secs(5);
/// Intraday candle windows (hourly / same-day) also move during the session.
const INTRADAY_TTL: Duration = Duration::from_secs(120);

/// Browser-shaped headers every exodus route expects; some 400/403 without
/// them. The bearer is added per request. The user agent comes from the
/// backend's default.
pub fn request_headers(bearer: &str) -> Vec<(String, String)> {
    vec![
        ("authorization".into(), format!("Bearer {bearer}")),
        ("accept".into(), "application/json".into()),
        ("origin".into(), "https://stockbit.com".into()),
        ("referer".into(), "https://stockbit.com/".into()),
    ]
}

/// The envelope warning when the order book's foreign figures were left out;
/// `flag` is the `has_foreign_bs` value Stockbit sent (`false` in the recorded
/// in-session book).
fn foreign_not_current(flag: &str) -> String {
    format!(
        "foreign flow left empty: Stockbit flags it as not current (has_foreign_bs {flag}); \
         in session it still holds the previous session's figures"
    )
}

/// Stockbit lists IDX only: a ticker from another market (`D05.SI`) is
/// unsupported, before any request (and before the session is touched).
pub(crate) fn idx_only(symbol: &Symbol) -> Result<()> {
    match symbol.exchange() {
        Some(_) => Err(IdxError::unsupported(NAME, "tickers outside IDX")),
        None => Ok(()),
    }
}

/// Stockbit's code for a symbol: indices drop idx's caret and use Stockbit's
/// own name (`^JKSE` -> `IHSG`, `^JKLQ45` -> `LQ45`).
pub fn stockbit_code(symbol: &Symbol) -> String {
    if let Some(body) = symbol.as_str().strip_prefix('^') {
        match body {
            "JKSE" | "JKSE.JK" => "IHSG".into(),
            "JKLQ45" => "LQ45".into(),
            other => other.to_string(),
        }
    } else {
        symbol.as_str().to_string()
    }
}

/// Paths idx may GET on the session API. The same API places orders and
/// moves cash; anything else fails before a token is read. A feature that
/// reads a new endpoint adds its prefix here.
const READ_PREFIXES: &[&str] = &[
    "/company-price-feed/",
    "/charts/",
    "/corpaction/",
    "/insider/company/majorholder?",
    "/marketdetectors/",
];

/// `true` for an exodus URL under one of [`READ_PREFIXES`].
fn is_read_url(url: &str) -> bool {
    url.strip_prefix(EXODUS).is_some_and(|p| {
        READ_PREFIXES.iter().any(|x| p.starts_with(x))
            && !p.contains("..")
            && !p.contains('%')
            && !p.contains('#')
    })
}

/// Stockbit refusing the bearer: 401, or a 403 with a JSON body. An HTML 403
/// is CloudFront refusing this client, which a refresh cannot change.
pub(crate) fn rejected(status: u16, body: &str) -> bool {
    status == 401 || (status == 403 && serde_json::from_str::<serde_json::Value>(body).is_ok())
}

/// The error for an HTML 403 from Stockbit's CDN.
const CDN_BLOCKED: &str = "exodus.stockbit.com refused this client (HTTP 403 from its CDN); idx does not retry a block, try later or from another network";

/// Errors that no other Stockbit request in the same command can get past
/// (no session, a refused session, a CDN block), so a command that reads
/// several lists stops at the first one instead of repeating it.
pub fn is_fatal(e: &IdxError) -> bool {
    match e {
        IdxError::Usage(_) => true,
        IdxError::Provider { provider, message } if *provider == NAME => {
            message == auth::REFRESH_REJECTED
                || message == CDN_BLOCKED
                || message.starts_with("Stockbit rejected the session")
        }
        _ => false,
    }
}

/// Pause between live requests of the commands that read several lists.
pub const PACE: Duration = Duration::from_secs(1);
/// Corporate actions change a few times a day at most.
const CORPACTION_TTL: Duration = Duration::from_secs(6 * 3600);
/// Filings arrive through the day; an hour keeps paging consistent.
const MOVEMENTS_TTL: Duration = Duration::from_secs(3600);

pub struct Stockbit {
    fetcher: Arc<Fetcher>,
    ttls: Ttls,
    store: Arc<SessionStore>,
    pace: Duration,
}

impl Stockbit {
    pub fn new(fetcher: Arc<Fetcher>, ttls: Ttls, store: Arc<SessionStore>) -> Self {
        Stockbit {
            fetcher,
            ttls,
            store,
            pace: PACE,
        }
    }

    /// The same provider with another pause between paced requests (tests
    /// use zero).
    pub fn with_pace(mut self, pace: Duration) -> Self {
        self.pace = pace;
        self
    }

    pub fn store(&self) -> &Arc<SessionStore> {
        &self.store
    }

    pub fn fetcher(&self) -> &Arc<Fetcher> {
        &self.fetcher
    }

    /// GET an exodus URL with a valid bearer, refreshing the access token once
    /// if the first attempt is rejected. `url` doubles as the cache key; it
    /// carries no token, so no secret enters the cache (rule 6). Only
    /// [`READ_PREFIXES`] paths are ever requested.
    async fn get(&self, url: &str, ttl: Duration, what: &str) -> Result<Fetched> {
        if !is_read_url(url) {
            return Err(IdxError::Other(format!(
                "idx does not call {url} on Stockbit's session API"
            )));
        }
        let now = chrono::Utc::now().timestamp();
        let mut session = self.store.valid_session(&self.fetcher, now).await?;
        for attempt in 0..2 {
            let headers = request_headers(&session.access);
            let fetched = match self.fetcher.get(url, &headers, ttl, Some(url)).await {
                Ok(f) => f,
                Err(IdxError::OfflineMiss(_)) => {
                    return Err(IdxError::OfflineMiss(format!("no cached {what}")))
                }
                Err(e) => return Err(e),
            };
            if fetched.status == 403 && !rejected(fetched.status, &fetched.body) {
                return Err(IdxError::provider(NAME, CDN_BLOCKED));
            }
            if rejected(fetched.status, &fetched.body) {
                if attempt == 1 || self.fetcher.is_offline() || fetched.cached {
                    return Err(IdxError::provider(
                        NAME,
                        format!(
                            "Stockbit rejected the session (HTTP {}) after a refresh; run `idx stockbit login` again",
                            fetched.status
                        ),
                    ));
                }
                tracing::info!("stockbit rejected the token; refreshing once");
                session = self.store.rotate(&self.fetcher, &session, now).await?;
                continue;
            }
            return Ok(fetched);
        }
        unreachable!("loop returns on the second attempt")
    }

    /// [`get`](Self::get) a second after the previous paced live request on
    /// the same session in this process, from any provider or MCP tool call
    /// (cached answers cost no pause), for the commands that read several
    /// Stockbit lists in a row: a human-ish request rate.
    async fn get_paced(&self, url: &str, ttl: Duration, what: &str) -> Result<Fetched> {
        let cached = self.fetcher.mode() != crate::http::CacheMode::Bypass
            && self.fetcher.cache().get(url, ttl).is_some_and(|h| h.fresh);
        let mut last = self.store.pacer().lock().await;
        if !cached && !self.fetcher.is_offline() {
            if let Some(t) = *last {
                let wait = self.pace.saturating_sub(t.elapsed());
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
            }
        }
        let f = self.get(url, ttl, what).await;
        if matches!(&f, Ok(f) if !f.cached) {
            *last = Some(tokio::time::Instant::now());
        }
        f
    }

    fn sourced<T>(data: T, f: &Fetched) -> Sourced<T> {
        Sourced::from_fetched(data, NAME, f)
    }

    fn orderbook_url(symbol: &Symbol) -> String {
        format!(
            "{EXODUS}/company-price-feed/v2/orderbook/companies/{}",
            stockbit_code(symbol)
        )
    }

    /// The full depth ladder for one symbol (realtime; not in the `Provider`
    /// trait, called directly by the orderbook command and tool).
    pub async fn orderbook(&self, symbol: &Symbol) -> Result<Sourced<OrderBook>> {
        idx_only(symbol)?;
        let url = Self::orderbook_url(symbol);
        let f = self.get(&url, REALTIME_TTL, "order book").await?;
        if f.status == 404 || parse::is_not_found(&f.body) {
            return Err(IdxError::NotFound(symbol.to_string()));
        }
        if !f.is_success() {
            return Err(IdxError::provider(
                NAME,
                format!(
                    "order book HTTP {}: {}",
                    f.status,
                    parse::error_message(&f.body)
                ),
            ));
        }
        let ob = parse::parse_orderbook(symbol, &f.body)?;
        let mut out = Self::sourced(ob, &f);
        if let Some(flag) = parse::foreign_withheld(&f.body) {
            out.warnings.push(foreign_not_current(&flag));
        }
        Ok(out)
    }

    /// One kind of corporate action: one stock's whole history with
    /// `symbol`, the market's recent and announced actions without it.
    /// Paced and cached 6 h. Stockbit answering "not found" is an empty list
    /// with a warning.
    pub async fn corporate_actions(
        &self,
        kind: CorporateEventKind,
        symbol: Option<&Symbol>,
    ) -> Result<Sourced<Vec<CorporateEvent>>> {
        if let Some(s) = symbol {
            idx_only(s)?;
        }
        let code = symbol.map(stockbit_code);
        let url = corpaction::url(kind, code.as_deref());
        let f = self
            .get_paced(&url, CORPACTION_TTL, "corporate actions")
            .await?;
        if f.status == 404 || parse::is_not_found(&f.body) {
            let mut out = Self::sourced(Vec::new(), &f);
            out.warnings.push(match symbol {
                Some(s) => format!("Stockbit lists no {kind} for {s}"),
                None => format!("Stockbit lists no {kind}"),
            });
            return Ok(out);
        }
        if !f.is_success() {
            return Err(IdxError::provider(
                NAME,
                format!(
                    "corporate actions ({kind}) HTTP {}: {}",
                    f.status,
                    parse::error_message(&f.body)
                ),
            ));
        }
        Ok(Self::sourced(corpaction::parse(kind, &f.body)?, &f))
    }

    /// One page (from 1) of insider and 5 % holder movements, newest first:
    /// its rows and whether more follow. Paced and cached 1 h. Stockbit
    /// answering "not found" is an empty last page with a warning.
    pub async fn movements_page(
        &self,
        q: &movement::MovementQuery,
        page: u32,
    ) -> Result<Sourced<(Vec<HolderMovement>, bool)>> {
        self.movements_page_within(q, page, MOVEMENTS_TTL).await
    }

    /// [`movements_page`](Self::movements_page) read live, past a cached
    /// copy (offline, or when the request fails, that copy is served stale).
    pub async fn movements_page_live(
        &self,
        q: &movement::MovementQuery,
        page: u32,
    ) -> Result<Sourced<(Vec<HolderMovement>, bool)>> {
        self.movements_page_within(q, page, Duration::ZERO).await
    }

    async fn movements_page_within(
        &self,
        q: &movement::MovementQuery,
        page: u32,
        ttl: Duration,
    ) -> Result<Sourced<(Vec<HolderMovement>, bool)>> {
        q.validate()?;
        if let Some(s) = &q.symbol {
            idx_only(s)?;
        }
        let url = movement::url(q, page);
        let f = self.get_paced(&url, ttl, "shareholder movements").await?;
        if f.status == 404 || parse::is_not_found(&f.body) {
            let mut out = Self::sourced((Vec::new(), false), &f);
            out.warnings.push(match &q.symbol {
                Some(s) => format!("Stockbit lists no holder movements for {s}"),
                None => "Stockbit lists no holder movements for these filters".into(),
            });
            return Ok(out);
        }
        if !f.is_success() {
            return Err(IdxError::provider(
                NAME,
                format!(
                    "shareholder movements HTTP {}: {}",
                    f.status,
                    parse::error_message(&f.body)
                ),
            ));
        }
        Ok(Self::sourced(movement::parse_page(&f.body)?, &f))
    }

    async fn quote_one(&self, symbol: &Symbol) -> Result<Sourced<Quote>> {
        idx_only(symbol)?;
        let url = Self::orderbook_url(symbol);
        let f = self.get(&url, REALTIME_TTL, "quote").await?;
        if f.status == 404 || parse::is_not_found(&f.body) {
            return Err(IdxError::NotFound(symbol.to_string()));
        }
        if !f.is_success() {
            return Err(IdxError::provider(
                NAME,
                format!("quote HTTP {}: {}", f.status, parse::error_message(&f.body)),
            ));
        }
        Ok(Self::sourced(parse::parse_quote(symbol, &f.body)?, &f))
    }
}

#[async_trait]
impl Provider for Stockbit {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn quotes(&self, symbols: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
        let results = join_all(symbols.iter().map(|s| self.quote_one(s))).await;
        let mut out = Sourced::fresh(Vec::new(), NAME);
        out.cached = true;
        let mut first_err: Option<IdxError> = None;
        for (sym, r) in symbols.iter().zip(results) {
            match r {
                Ok(s) => {
                    out.absorb(&s);
                    out.data.push(s.data);
                }
                Err(IdxError::NotFound(_)) => out.warnings.push(format!("no data for {sym}")),
                Err(e) => {
                    out.warnings.push(format!("{sym}: {e}"));
                    first_err.get_or_insert(e);
                }
            }
        }
        if out.data.is_empty() {
            if let Some(e) = first_err {
                return Err(e);
            }
            let list: Vec<&str> = symbols.iter().map(Symbol::as_str).collect();
            return Err(IdxError::NotFound(format!(
                "no quotes for {}",
                list.join(", ")
            )));
        }
        Ok(out)
    }

    async fn history(
        &self,
        symbol: &Symbol,
        period: Period,
        interval: Interval,
    ) -> Result<Sourced<History>> {
        idx_only(symbol)?;
        let (tf, prev) = parse::timeframe_for(period, interval);
        // Stockbit picks the bar size and the span from its window; the warning
        // below says so when either is not what was asked for.
        let mut url = format!(
            "{EXODUS}/charts/{}/daily?timeframe={tf}&chart_type=PRICE_CHART_TYPE_CANDLE",
            stockbit_code(symbol)
        );
        if prev {
            url.push_str("&is_include_previous_historical=true");
        }
        // Daily and longer windows change once a session; the hourly/same-day
        // windows move intraday.
        let ttl = if matches!(tf, "1w" | "1d") {
            INTRADAY_TTL
        } else {
            self.ttls.history
        };
        let f = self.get(&url, ttl, "chart").await?;
        if f.status == 404 || parse::is_not_found(&f.body) {
            return Err(IdxError::NotFound(symbol.to_string()));
        }
        if !f.is_success() {
            return Err(IdxError::provider(
                NAME,
                format!("chart HTTP {}: {}", f.status, parse::error_message(&f.body)),
            ));
        }
        let mut out = Self::sourced(parse::parse_history(symbol, period, interval, &f.body)?, &f);
        if let Some(w) = granularity_warning(period, interval) {
            out.warnings.push(w);
        }
        Ok(out)
    }

    fn serves_interval(&self, period: Period, interval: Interval) -> bool {
        parse::serves_interval(period, interval)
    }
}

/// The bar size Stockbit returns for a request, in idx's interval vocabulary:
/// `1m` (one minute), `1h`, `1d` or `1wk`. Stockbit picks it from the window:
/// the same-day window is one-minute, `1w` hourly, `1m` to `1y` daily, `3y`
/// and `5y` weekly.
pub fn bars_for(period: Period, interval: Interval) -> &'static str {
    match parse::timeframe_for(period, interval).0 {
        "1d" => "1m",
        "1w" => "1h",
        "3y" | "5y" => "1wk",
        _ => "1d",
    }
}

/// Plain name of a bar size from [`bars_for`].
pub fn bar_word(bars: &str) -> &'static str {
    match bars {
        "1m" => "one-minute",
        "1h" => "hourly",
        "1wk" => "weekly",
        _ => "daily",
    }
}

/// The span Stockbit charts for a request, in words (`--period 2y` is charted
/// over three years, `10y` and `max` over five).
pub fn window_for(period: Period, interval: Interval) -> &'static str {
    match parse::timeframe_for(period, interval).0 {
        "1d" => "the current session",
        "1w" => "one week",
        "1m" => "one month",
        "3m" => "three months",
        "ytd" => "its year-to-date window",
        "1y" => "one year",
        "3y" => "three years",
        _ => "five years",
    }
}

/// `true` when Stockbit answers a request as asked: the window it charts is
/// the period and its bars are the interval (daily over `1mo` to `1y`, hourly
/// over `5d`, weekly over `5y`), or a same-day intraday request, where its
/// one-minute bars are finer than asked and realtime. `history_provider =
/// auto` keeps those on Stockbit and sends the rest to Yahoo.
pub fn serves_as_asked(period: Period, interval: Interval) -> bool {
    match (period, interval) {
        (Period::D1, i) => i.is_intraday(),
        (Period::D5, Interval::Hour1) | (Period::Y5, Interval::Week1) => true,
        (Period::M1 | Period::M3 | Period::M6 | Period::Ytd | Period::Y1, Interval::Day1) => true,
        _ => false,
    }
}

/// `true` when the span Stockbit charts for a request is the period asked
/// for. It is not for `2y` (charted over three years), `10y` and `max` (five
/// years), or an intraday interval over `1mo` or longer (one week).
fn window_is_period(period: Period, interval: Interval) -> bool {
    matches!(
        (period, parse::timeframe_for(period, interval).0),
        (Period::D1, "1d")
            | (Period::D5, "1w")
            | (Period::M1, "1m")
            | (Period::M3, "3m")
            | (Period::M6 | Period::Ytd, "ytd")
            | (Period::Y1, "1y")
            | (Period::Y5, "5y")
    )
}

/// The warning a Stockbit history carries when it is not the request as
/// asked: Stockbit picks the bar size and the span from its own windows, so
/// the bars can differ from the interval and the span from the period
/// (`--period 1mo --interval 1h` comes back as one week of hourly bars). Its
/// candles are not trimmed to the period, so the warning names both.
pub fn granularity_warning(period: Period, interval: Interval) -> Option<String> {
    let bars = bars_for(period, interval);
    if !window_is_period(period, interval) {
        return Some(format!(
            "Stockbit returned {} bars over {}, not {interval} bars over {period}; the candles cover that span and each candle's time reflects the real granularity",
            bar_word(bars),
            window_for(period, interval),
        ));
    }
    (bars != interval.as_str()).then(|| {
        format!(
            "Stockbit returned {} bars for this window, not {interval} bars; each candle's time reflects the real granularity",
            bar_word(bars)
        )
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::http::{CacheMode, FixtureBackend};
    use clap::ValueEnum;

    const IN_SESSION: &str =
        include_str!("../../../tests/fixtures/stockbit/orderbook_bbca_in_session.json");
    const CLOSED: &str = include_str!("../../../tests/fixtures/stockbit/orderbook_bbca.json");
    /// `{"alg":"none"}.{"exp":4102444800}.sig`: fresh until 2100, so no refresh
    /// is attempted. Signed by nobody and only ever sent to the fixture backend.
    const DUMMY_ACCESS: &str = "eyJhbGciOiJub25lIn0.eyJleHAiOjQxMDI0NDQ4MDB9.sig";

    fn stockbit(body: &str) -> (Stockbit, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(FixtureBackend::new().route("orderbook/companies/BBCA", 200, body));
        let fetcher = Arc::new(Fetcher::new(
            be.clone(),
            Cache::new(dir.path().join("cache")),
            CacheMode::Normal,
        ));
        let store = Arc::new(SessionStore::new(
            dir.path().join("session.json"),
            Some(DUMMY_ACCESS.into()),
            None,
        ));
        (Stockbit::new(fetcher, Ttls::default(), store), be, dir)
    }

    #[tokio::test]
    async fn orderbook_warns_when_foreign_flow_is_not_current() {
        let bbca = Symbol::parse("BBCA").unwrap();

        let (sb, be, _d) = stockbit(IN_SESSION);
        let got = sb.orderbook(&bbca).await.unwrap();
        assert_eq!(got.provider, "stockbit");
        assert_eq!(got.data.foreign_net, None);
        assert_eq!(got.data.volume, Some(134_744_100));
        assert_eq!(got.warnings.len(), 1, "{:?}", got.warnings);
        assert!(
            got.warnings[0].contains("(has_foreign_bs false)"),
            "{:?}",
            got.warnings
        );
        assert_eq!(be.calls_matching("orderbook/companies/BBCA"), 1);
        assert_eq!(be.calls_matching("login/refresh"), 0);

        let (sb, _be, _d) = stockbit(CLOSED);
        let got = sb.orderbook(&bbca).await.unwrap();
        assert_eq!(got.data.foreign_net, Some(-2_329_493_087_500.0));
        assert!(got.warnings.is_empty(), "{:?}", got.warnings);
    }

    #[tokio::test]
    async fn other_markets_are_unsupported_before_any_request() {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(FixtureBackend::new());
        let fetcher = Arc::new(Fetcher::new(
            be.clone(),
            Cache::new(dir.path()),
            CacheMode::Normal,
        ));
        let store = Arc::new(SessionStore::new(dir.path().join("s.json"), None, None));
        let sb = Stockbit::new(fetcher, Ttls::default(), store);
        let d05 = Symbol::parse_on("D05.SI", "JK").unwrap();
        let unsupported = |e: IdxError| matches!(e, IdxError::Unsupported { .. });
        assert!(unsupported(
            sb.quotes(std::slice::from_ref(&d05)).await.unwrap_err()
        ));
        assert!(unsupported(
            sb.history(&d05, Period::M1, Interval::Day1)
                .await
                .unwrap_err()
        ));
        assert!(unsupported(sb.orderbook(&d05).await.unwrap_err()));
        assert_eq!(be.call_count(), 0);
    }

    #[test]
    fn bar_sizes_follow_the_window() {
        assert_eq!(bars_for(Period::D1, Interval::Min5), "1m");
        assert_eq!(bars_for(Period::D1, Interval::Day1), "1m");
        assert_eq!(bars_for(Period::D5, Interval::Hour1), "1h");
        assert_eq!(bars_for(Period::D5, Interval::Day1), "1h");
        assert_eq!(bars_for(Period::M1, Interval::Min30), "1h");
        assert_eq!(bars_for(Period::M1, Interval::Day1), "1d");
        assert_eq!(bars_for(Period::Y1, Interval::Week1), "1d");
        assert_eq!(bars_for(Period::Ytd, Interval::Day1), "1d");
        assert_eq!(bars_for(Period::Y2, Interval::Day1), "1wk");
        assert_eq!(bars_for(Period::Max, Interval::Month1), "1wk");
    }

    #[test]
    fn served_as_asked_means_window_and_bars_match() {
        for (period, interval) in [
            (Period::D1, Interval::Min5),
            (Period::D1, Interval::Hour1),
            (Period::D5, Interval::Hour1),
            (Period::M1, Interval::Day1),
            (Period::M3, Interval::Day1),
            (Period::M6, Interval::Day1),
            (Period::Ytd, Interval::Day1),
            (Period::Y1, Interval::Day1),
            (Period::Y5, Interval::Week1),
        ] {
            assert!(serves_as_asked(period, interval), "{period} {interval}");
        }
        for (period, interval) in [
            (Period::D1, Interval::Day1),
            (Period::D5, Interval::Day1),
            (Period::D5, Interval::Min30),
            (Period::M1, Interval::Hour1),
            (Period::Y1, Interval::Week1),
            (Period::Y2, Interval::Day1),
            (Period::Y2, Interval::Week1),
            (Period::Y5, Interval::Day1),
            (Period::Y10, Interval::Week1),
            (Period::Max, Interval::Month1),
        ] {
            assert!(!serves_as_asked(period, interval), "{period} {interval}");
        }
        assert_eq!(window_for(Period::Y2, Interval::Week1), "three years");
        assert_eq!(window_for(Period::M1, Interval::Min15), "one week");
    }

    #[test]
    fn warning_compares_bar_sizes() {
        // Daily bars labelled weekly used to pass silently.
        let w = granularity_warning(Period::Y1, Interval::Week1).unwrap();
        assert!(w.contains("daily bars"), "{w}");
        assert!(w.contains("not 1wk"), "{w}");
        // Hourly bars for an hourly request are what was asked.
        assert_eq!(granularity_warning(Period::D5, Interval::Hour1), None);
        assert_eq!(granularity_warning(Period::Y1, Interval::Day1), None);
        assert_eq!(granularity_warning(Period::Y5, Interval::Week1), None);
        let w = granularity_warning(Period::Y5, Interval::Day1).unwrap();
        assert!(w.contains("weekly bars"), "{w}");
        let w = granularity_warning(Period::D1, Interval::Min5).unwrap();
        assert!(w.contains("one-minute bars"), "{w}");
        let w = granularity_warning(Period::D5, Interval::Day1).unwrap();
        assert!(w.contains("hourly bars"), "{w}");
    }

    #[test]
    fn warning_names_a_span_that_is_not_the_period() {
        // Matching bars over another span used to pass silently.
        let w = granularity_warning(Period::M1, Interval::Hour1).unwrap();
        assert!(w.contains("hourly bars over one week"), "{w}");
        assert!(w.contains("not 1h bars over 1mo"), "{w}");
        let w = granularity_warning(Period::Y2, Interval::Week1).unwrap();
        assert!(w.contains("weekly bars over three years"), "{w}");
        assert!(w.contains("not 1wk bars over 2y"), "{w}");
        for period in [Period::Y10, Period::Max] {
            let w = granularity_warning(period, Interval::Week1).unwrap();
            assert!(w.contains("weekly bars over five years"), "{w}");
            assert!(w.contains(&format!("over {period};")), "{w}");
        }
        let w = granularity_warning(Period::Y2, Interval::Day1).unwrap();
        assert!(w.contains("weekly bars over three years"), "{w}");
        let w = granularity_warning(Period::Y1, Interval::Min15).unwrap();
        assert!(w.contains("hourly bars over one week"), "{w}");
    }

    #[test]
    fn warning_is_silent_exactly_when_served_as_asked() {
        // Same-day intraday is served (finer one-minute bars) and still says so.
        for &period in Period::value_variants() {
            for &interval in Interval::value_variants() {
                let w = granularity_warning(period, interval);
                if period == Period::D1 && interval.is_intraday() && interval != Interval::Min1 {
                    assert!(
                        w.unwrap().contains("one-minute bars"),
                        "{period} {interval}"
                    );
                } else {
                    assert_eq!(
                        w.is_none(),
                        serves_as_asked(period, interval),
                        "{period} {interval}: {w:?}"
                    );
                }
            }
        }
    }

    const BLOCKED: &str = include_str!("../../../tests/fixtures/stockbit/blocked_403.html");
    const UNAUTHORIZED: &str = include_str!("../../../tests/fixtures/stockbit/unauthorized.json");
    const REFRESHED: &str =
        include_str!("../../../tests/fixtures/stockbit/login_refresh_constructed.json");

    /// A provider over `be` whose session holds the dummy access token and a
    /// dummy refresh token.
    pub(crate) fn with_backend(
        be: FixtureBackend,
    ) -> (Stockbit, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(be);
        let fetcher = Arc::new(Fetcher::new(
            be.clone(),
            Cache::new(dir.path().join("cache")),
            CacheMode::Normal,
        ));
        let store = Arc::new(SessionStore::new(
            dir.path().join("session.json"),
            Some(DUMMY_ACCESS.into()),
            Some("eyJ0.eyJleHAiOjF9.dummy".into()),
        ));
        (Stockbit::new(fetcher, Ttls::default(), store), be, dir)
    }

    #[test]
    fn only_read_paths_are_allowed() {
        for ok in [
            "https://exodus.stockbit.com/company-price-feed/v2/orderbook/companies/BBCA",
            "https://exodus.stockbit.com/company-price-feed/market-time/session",
            "https://exodus.stockbit.com/charts/BBCA/daily?timeframe=1m",
            "https://exodus.stockbit.com/corpaction/dividend?symbol=BBCA",
            "https://exodus.stockbit.com/insider/company/majorholder?page=1",
            "https://exodus.stockbit.com/marketdetectors/BBCA?transaction_type=TRANSACTION_TYPE_GROSS",
        ] {
            assert!(is_read_url(ok), "{ok}");
        }
        for no in [
            "https://exodus.stockbit.com/order/buy",
            "https://exodus.stockbit.com/login/refresh",
            "https://exodus.stockbit.com/corpaction/../order/buy",
            "https://exodus.stockbit.com/corpaction/%2e%2e/order/buy",
            "https://exodus.stockbit.com/insider/company/majorholder/../../order",
            "https://exodus.stockbit.com/marketdetectors/../order/buy",
            "https://exodus.stockbit.com/marketdetectors",
            "https://evil.test/corpaction/dividend",
            "https://exodus.stockbit.com.evil.test/corpaction/dividend",
            "http://exodus.stockbit.com/corpaction/dividend",
        ] {
            assert!(!is_read_url(no), "{no}");
        }
    }

    #[test]
    fn a_json_403_is_a_refused_token_an_html_one_a_block() {
        assert!(rejected(401, UNAUTHORIZED));
        assert!(rejected(401, "<html>"));
        assert!(rejected(403, UNAUTHORIZED));
        assert!(!rejected(403, BLOCKED));
        assert!(!rejected(200, "{}"));
        assert!(!rejected(404, UNAUTHORIZED));
    }

    #[tokio::test]
    async fn pacing_is_shared_by_every_provider_on_a_session() {
        let pace = Duration::from_millis(300);
        let (a, be, _d) =
            with_backend(FixtureBackend::new().route("corpaction/", 404, r#"{"message":"x"}"#));
        let a = a.with_pace(pace);
        // A second provider over the same store, as MCP builds per tool call.
        let b =
            Stockbit::new(a.fetcher().clone(), Ttls::default(), a.store().clone()).with_pace(pace);
        let start = tokio::time::Instant::now();
        let (ra, rb) = tokio::join!(
            a.corporate_actions(CorporateEventKind::Dividend, None),
            b.corporate_actions(CorporateEventKind::Rups, None),
        );
        ra.unwrap();
        rb.unwrap();
        assert_eq!(be.calls_matching("corpaction/"), 2);
        assert!(start.elapsed() >= pace, "{:?}", start.elapsed());
    }

    #[tokio::test]
    async fn a_path_off_the_allowlist_fails_before_any_request() {
        let (sb, be, _d) = with_backend(FixtureBackend::new());
        let e = sb
            .get("https://exodus.stockbit.com/order/buy", REALTIME_TTL, "x")
            .await
            .unwrap_err();
        assert!(e.to_string().contains("does not call"), "{e}");
        assert_eq!(be.call_count(), 0);
    }

    #[tokio::test]
    async fn a_cdn_block_is_not_retried_and_spends_no_refresh() {
        let (sb, be, _d) =
            with_backend(FixtureBackend::new().route("orderbook/companies/BBCA", 403, BLOCKED));
        let e = sb
            .orderbook(&Symbol::parse("BBCA").unwrap())
            .await
            .unwrap_err();
        assert_eq!(e.exit_code(), 3);
        assert!(e.to_string().contains("HTTP 403 from its CDN"), "{e}");
        assert!(is_fatal(&e));
        assert_eq!(be.calls_matching("orderbook/companies/BBCA"), 1);
        assert_eq!(be.calls_matching("login/refresh"), 0);
    }

    #[tokio::test]
    async fn a_refused_token_is_refreshed_once_then_reported() {
        let (sb, be, _d) = with_backend(
            FixtureBackend::new()
                .route("orderbook/companies/BBCA", 401, UNAUTHORIZED)
                .route("login/refresh", 200, REFRESHED),
        );
        let e = sb
            .orderbook(&Symbol::parse("BBCA").unwrap())
            .await
            .unwrap_err();
        assert!(
            e.to_string().contains("run `idx stockbit login` again"),
            "{e}"
        );
        assert!(is_fatal(&e));
        assert_eq!(be.calls_matching("orderbook/companies/BBCA"), 2);
        assert_eq!(be.calls_matching("login/refresh"), 1);
        // The retry carried the refreshed access token, not the rejected one.
        let headers = be.headers.lock().unwrap();
        let bearer = |i: usize| {
            headers[i]
                .iter()
                .find(|(k, _)| k == "authorization")
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(bearer(0), format!("Bearer {DUMMY_ACCESS}"));
        assert_ne!(bearer(2), bearer(0));
    }
}
