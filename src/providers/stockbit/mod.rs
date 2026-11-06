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
//! - `POST /login/refresh` — rotate the session (see `auth.rs`).
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
use crate::model::{History, Interval, OrderBook, Period, Quote, Symbol};

use super::{Provider, Sourced, Ttls};

pub mod auth;
pub mod login;
mod parse;

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

pub struct Stockbit {
    fetcher: Arc<Fetcher>,
    ttls: Ttls,
    store: Arc<SessionStore>,
}

impl Stockbit {
    pub fn new(fetcher: Arc<Fetcher>, ttls: Ttls, store: Arc<SessionStore>) -> Self {
        Stockbit {
            fetcher,
            ttls,
            store,
        }
    }

    pub fn store(&self) -> &Arc<SessionStore> {
        &self.store
    }

    /// GET an exodus URL with a valid bearer, refreshing the access token once
    /// if the first attempt is rejected. `url` doubles as the cache key — it
    /// carries no token, so no secret enters the cache (rule 6).
    async fn get(&self, url: &str, ttl: Duration, what: &str) -> Result<Fetched> {
        let now = chrono::Utc::now().timestamp();
        for attempt in 0..2 {
            let token = self.store.valid_access(&self.fetcher, now).await?;
            let headers = request_headers(&token);
            let fetched = match self.fetcher.get(url, &headers, ttl, Some(url)).await {
                Ok(f) => f,
                Err(IdxError::OfflineMiss(_)) => {
                    return Err(IdxError::OfflineMiss(format!("no cached {what}")))
                }
                Err(e) => return Err(e),
            };
            if (fetched.status == 401 || fetched.status == 403)
                && attempt == 0
                && !self.fetcher.is_offline()
                && !fetched.cached
            {
                tracing::info!("stockbit rejected the token; refreshing once");
                // Force the next loop to refresh by expiring the cached access.
                let session = self.store.load();
                let _ = self
                    .store
                    .refresh(&self.fetcher, &session)
                    .await
                    .map(|s| self.store.save(&s));
                continue;
            }
            return Ok(fetched);
        }
        unreachable!("loop returns on the second attempt")
    }

    fn sourced<T>(data: T, f: &Fetched) -> Sourced<T> {
        Sourced {
            data,
            provider: NAME,
            cached: f.cached,
            stale: f.stale,
            fetched_at: f.fetched_at,
            warnings: vec![],
        }
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

    async fn quote_one(&self, symbol: &Symbol) -> Result<Sourced<Quote>> {
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
        let (tf, prev) = parse::timeframe_for(period, interval);
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
        // Stockbit picks the granularity from the window; say so when it is not
        // the daily bars the interval nominally asks for.
        let granularity = match tf {
            "1w" => Some("hourly"),
            "1d" => Some("intraday (one-minute)"),
            "3y" | "5y" => Some("weekly"),
            _ => None,
        };
        if let Some(g) = granularity {
            out.warnings.push(format!(
                "Stockbit returned {g} bars for this window, not {interval} bars; each candle's time reflects the real granularity"
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::http::{CacheMode, FixtureBackend};

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
}
