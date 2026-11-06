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
        Ok(Self::sourced(ob, &f))
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
        // Stockbit picks the bar size from the window; the warning below says
        // so when it is not the interval asked for.
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

/// The warning a Stockbit history carries when its bars are not the interval
/// asked for (Stockbit picks the bar size from the window, not the interval).
pub fn granularity_warning(period: Period, interval: Interval) -> Option<String> {
    let bars = bars_for(period, interval);
    (bars != interval.as_str()).then(|| {
        format!(
            "Stockbit returned {} bars for this window, not {interval} bars; each candle's time reflects the real granularity",
            bar_word(bars)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
