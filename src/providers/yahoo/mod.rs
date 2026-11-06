//! Yahoo Finance provider.
//!
//! Endpoints (all unofficial, no key):
//! - `GET https://query1.finance.yahoo.com/v8/finance/chart/{SYM}.JK?range=..&interval=..`
//!   No auth. Used for `history` and as the quote fallback.
//! - `GET https://query1.finance.yahoo.com/v7/finance/quote?symbols=A.JK,B.JK&crumb=..`
//!   Needs the `A3` cookie from `https://fc.yahoo.com` plus the crumb from
//!   `https://query2.finance.yahoo.com/v1/test/getcrumb`. Returns the rich quote
//!   row (previous close, market cap, P/E, dividend yield, ...).
//! - `GET https://query2.finance.yahoo.com/v10/finance/quoteSummary/{SYM}.JK?modules=..&crumb=..`
//!   Same session. `profile`, `fundamental` and `earnings` read it; the
//!   statement modules are empty nowadays, so `financials` does not.
//! - `GET https://query2.finance.yahoo.com/ws/fundamentals-timeseries/v1/finance/timeseries/{SYM}.JK?type=..`
//!   No auth. Annual, quarterly and trailing statement items.
//!
//! Recorded responses live in `tests/fixtures/yahoo/`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::join_all;

use crate::error::{IdxError, Result};
use crate::http::Fetched;
use crate::http::Fetcher;
use crate::model::{
    DividendHistory, Earnings, Financials, Fundamentals, History, Interval, Period, Profile, Quote,
    StatementPeriod, Symbol,
};

use super::{Provider, Sourced, Ttls};

mod parse;
mod session;
pub mod summary;
pub mod timeseries;

pub use parse::{parse_chart, parse_chart_quote, parse_v7_quotes};

pub const NAME: &str = "yahoo";
const CHART_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";
const QUOTE_URL: &str = "https://query1.finance.yahoo.com/v7/finance/quote";
const SUMMARY_BASE: &str = "https://query2.finance.yahoo.com/v10/finance/quoteSummary/";
const TIMESERIES_BASE: &str =
    "https://query2.finance.yahoo.com/ws/fundamentals-timeseries/v1/finance/timeseries/";
/// Intraday candles move constantly; cache them briefly regardless of config.
const INTRADAY_TTL: Duration = Duration::from_secs(120);

pub struct Yahoo {
    fetcher: Arc<Fetcher>,
    ttls: Ttls,
}

impl Yahoo {
    pub fn new(fetcher: Arc<Fetcher>, ttls: Ttls) -> Self {
        Yahoo { fetcher, ttls }
    }

    pub fn chart_url(symbol: &Symbol, period: Period, interval: Interval) -> String {
        format!(
            "{CHART_BASE}{}?range={}&interval={}&includePrePost=false",
            symbol.yahoo(),
            period.as_str(),
            interval.as_str()
        )
    }

    /// Full-history chart call carrying dividend and split events.
    pub fn events_url(symbol: &Symbol) -> String {
        format!(
            "{CHART_BASE}{}?range=max&interval=1mo&events=div%2Csplits",
            symbol.yahoo()
        )
    }

    pub fn summary_key(symbol: &Symbol, modules: &str) -> String {
        format!("{SUMMARY_BASE}{}?modules={modules}", symbol.yahoo())
    }

    /// GET through the cookie/crumb session. `key` is the URL without the
    /// crumb and doubles as the cache key. A 401/403 refreshes the session once.
    async fn get_with_session(
        &self,
        key: &str,
        ttl: Duration,
        what: &str,
        symbol: &Symbol,
    ) -> Result<Fetched> {
        let mut session = session::get(&self.fetcher, false).await?;
        for attempt in 0..2 {
            let (url, headers) = match &session {
                Some(s) => (format!("{key}&crumb={}", urlencode(&s.crumb)), s.headers()),
                None => (key.to_string(), vec![]),
            };
            let fetched = match self.fetcher.get(&url, &headers, ttl, Some(key)).await {
                Ok(f) => f,
                Err(IdxError::OfflineMiss(_)) => {
                    return Err(IdxError::OfflineMiss(format!(
                        "no cached {what} for {symbol}"
                    )))
                }
                Err(e) => return Err(e),
            };
            if (fetched.status == 401 || fetched.status == 403)
                && attempt == 0
                && !self.fetcher.is_offline()
            {
                tracing::info!("yahoo session rejected, refreshing crumb");
                session = session::get(&self.fetcher, true).await?;
                continue;
            }
            return Ok(fetched);
        }
        unreachable!("loop returns on the second attempt")
    }

    async fn quote_summary(&self, symbol: &Symbol, modules: &str, what: &str) -> Result<Fetched> {
        let key = Self::summary_key(symbol, modules);
        let f = self
            .get_with_session(&key, self.ttls.fundamentals, what, symbol)
            .await?;
        match f.status {
            200..=299 => Ok(f),
            404 => Err(IdxError::NotFound(format!(
                "{symbol}: {}",
                parse::chart_error(&f.body)
            ))),
            401 | 403 => Err(IdxError::provider(
                NAME,
                "quoteSummary endpoint rejected the session",
            )),
            s => Err(IdxError::provider(
                NAME,
                format!("quoteSummary HTTP {s}: {}", parse::chart_error(&f.body)),
            )),
        }
    }

    fn sourced<T>(data: T, f: &Fetched) -> Sourced<T> {
        Sourced::from_fetched(data, NAME, f)
    }

    fn quote_cache_key(symbols: &[Symbol]) -> String {
        let list: Vec<String> = symbols.iter().map(Symbol::yahoo).collect();
        format!("{QUOTE_URL}?symbols={}", list.join(","))
    }

    /// Rich quotes via the v7 endpoint (needs a session).
    async fn quotes_v7(&self, symbols: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
        let key = Self::quote_cache_key(symbols);
        let mut session = session::get(&self.fetcher, false).await?;

        for attempt in 0..2 {
            let (url, headers) = match &session {
                Some(s) => (format!("{key}&crumb={}", urlencode(&s.crumb)), s.headers()),
                // Offline with no session: only a cache hit can satisfy this.
                None => (key.clone(), vec![]),
            };
            let fetched = self
                .fetcher
                .get(&url, &headers, self.ttls.quote, Some(&key))
                .await?;

            if fetched.status == 401 || fetched.status == 403 {
                if attempt == 0 && !self.fetcher.is_offline() {
                    tracing::info!("yahoo session rejected, refreshing crumb");
                    session = session::get(&self.fetcher, true).await?;
                    continue;
                }
                return Err(IdxError::provider(
                    NAME,
                    "quote endpoint rejected the session",
                ));
            }
            if !fetched.is_success() {
                return Err(IdxError::provider(
                    NAME,
                    format!("quote endpoint returned HTTP {}", fetched.status),
                ));
            }

            let quotes = parse_v7_quotes(&fetched.body)?;
            let mut warnings = Vec::new();
            for s in symbols {
                if !quotes.iter().any(|q| &q.symbol == s) {
                    warnings.push(format!("no data for {s}"));
                }
            }
            if quotes.is_empty() {
                let list: Vec<&str> = symbols.iter().map(Symbol::as_str).collect();
                return Err(IdxError::NotFound(format!(
                    "no quotes for {}",
                    list.join(", ")
                )));
            }
            let mut out = Self::sourced(quotes, &fetched);
            out.warnings.extend(warnings);
            return Ok(out);
        }
        unreachable!("loop returns on the second attempt")
    }

    /// Reduced quotes assembled from the chart endpoint, one request per symbol.
    async fn quotes_chart(&self, symbols: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
        let futs = symbols.iter().map(|s| async move {
            let url = Self::chart_url(s, Period::D5, Interval::Day1);
            let f = self.fetcher.get(&url, &[], self.ttls.quote, None).await?;
            if f.status == 404 {
                return Err(IdxError::NotFound(format!("{s}")));
            }
            if !f.is_success() {
                return Err(IdxError::provider(
                    NAME,
                    format!("chart endpoint HTTP {}", f.status),
                ));
            }
            Ok::<_, IdxError>(Self::sourced(parse_chart_quote(&f.body)?, &f))
        });
        let results = join_all(futs).await;

        let mut out = Sourced::fresh(Vec::new(), NAME);
        out.cached = true;
        let mut first_err: Option<IdxError> = None;
        for (sym, r) in symbols.iter().zip(results) {
            match r {
                Ok(q) => {
                    out.absorb(&q);
                    out.data.push(q.data);
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
}

#[async_trait]
impl Provider for Yahoo {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn quotes(&self, symbols: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
        match self.quotes_v7(symbols).await {
            Ok(q) => Ok(q),
            Err(e @ IdxError::NotFound(_)) => Err(e),
            Err(e) => {
                tracing::info!(error = %e, "yahoo quote endpoint failed, falling back to chart");
                let mut fallback = match self.quotes_chart(symbols).await {
                    Ok(f) => f,
                    Err(IdxError::OfflineMiss(_)) => {
                        let list: Vec<&str> = symbols.iter().map(Symbol::as_str).collect();
                        return Err(IdxError::OfflineMiss(format!(
                            "no cached quote for {}",
                            list.join(", ")
                        )));
                    }
                    Err(other) => return Err(other),
                };
                fallback.warnings.insert(
                    0,
                    format!("quote endpoint unavailable ({e}); reduced fields from chart"),
                );
                Ok(fallback)
            }
        }
    }

    async fn history(
        &self,
        symbol: &Symbol,
        period: Period,
        interval: Interval,
    ) -> Result<Sourced<History>> {
        let url = Self::chart_url(symbol, period, interval);
        let ttl = if interval.is_intraday() {
            INTRADAY_TTL.min(self.ttls.history)
        } else {
            self.ttls.history
        };
        let f = match self.fetcher.get(&url, &[], ttl, None).await {
            Ok(f) => f,
            Err(IdxError::OfflineMiss(_)) => {
                return Err(IdxError::OfflineMiss(format!(
                    "no cached {period}/{interval} history for {symbol}"
                )))
            }
            Err(e) => return Err(e),
        };
        if f.status == 404 {
            return Err(IdxError::NotFound(format!(
                "{symbol}: {}",
                parse::chart_error(&f.body)
            )));
        }
        if !f.is_success() {
            return Err(IdxError::provider(
                NAME,
                format!(
                    "chart endpoint HTTP {}: {}",
                    f.status,
                    parse::chart_error(&f.body)
                ),
            ));
        }
        Ok(Self::sourced(parse_chart(&f.body, period, interval)?, &f))
    }

    async fn profile(&self, symbol: &Symbol) -> Result<Sourced<Profile>> {
        let f = self
            .quote_summary(symbol, summary::FUNDAMENTAL_MODULES, "company profile")
            .await?;
        Ok(Self::sourced(summary::parse_profile(&f.body, symbol)?, &f))
    }

    async fn fundamentals(&self, symbol: &Symbol) -> Result<Sourced<Fundamentals>> {
        let f = self
            .quote_summary(symbol, summary::FUNDAMENTAL_MODULES, "fundamentals")
            .await?;
        Ok(Self::sourced(
            summary::parse_fundamentals(&f.body, symbol)?,
            &f,
        ))
    }

    async fn earnings(&self, symbol: &Symbol) -> Result<Sourced<Earnings>> {
        let f = self
            .quote_summary(symbol, summary::EARNINGS_MODULES, "earnings")
            .await?;
        Ok(Self::sourced(summary::parse_earnings(&f.body, symbol)?, &f))
    }

    async fn financials(
        &self,
        symbol: &Symbol,
        period: StatementPeriod,
    ) -> Result<Sourced<Financials>> {
        let key = format!(
            "{TIMESERIES_BASE}{}?type={}",
            symbol.yahoo(),
            timeseries::types_param(period)
        );
        let now = chrono::Utc::now().timestamp();
        let url = format!(
            "{key}&period1={}&period2={}&merge=false&padTimeSeries=false",
            now - 6 * 365 * 86_400,
            now + 86_400
        );
        let f = match self
            .fetcher
            .get(&url, &[], self.ttls.financials, Some(&key))
            .await
        {
            Ok(f) => f,
            Err(IdxError::OfflineMiss(_)) => {
                return Err(IdxError::OfflineMiss(format!(
                    "no cached {period} financials for {symbol}"
                )))
            }
            Err(e) => return Err(e),
        };
        if f.status == 404 {
            return Err(IdxError::NotFound(format!("no financials for {symbol}")));
        }
        if !f.is_success() {
            return Err(IdxError::provider(
                NAME,
                format!("timeseries endpoint HTTP {}", f.status),
            ));
        }
        Ok(Self::sourced(
            timeseries::parse_timeseries(&f.body, symbol, period)?,
            &f,
        ))
    }

    async fn dividends(&self, symbol: &Symbol) -> Result<Sourced<DividendHistory>> {
        let url = Self::events_url(symbol);
        let f = match self
            .fetcher
            .get(&url, &[], self.ttls.fundamentals, None)
            .await
        {
            Ok(f) => f,
            Err(IdxError::OfflineMiss(_)) => {
                return Err(IdxError::OfflineMiss(format!(
                    "no cached dividend history for {symbol}"
                )))
            }
            Err(e) => return Err(e),
        };
        if f.status == 404 {
            return Err(IdxError::NotFound(format!(
                "{symbol}: {}",
                parse::chart_error(&f.body)
            )));
        }
        if !f.is_success() {
            return Err(IdxError::provider(
                NAME,
                format!(
                    "chart endpoint HTTP {}: {}",
                    f.status,
                    parse::chart_error(&f.body)
                ),
            ));
        }
        Ok(Self::sourced(parse::parse_events(&f.body)?, &f))
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::http::{CacheMode, FixtureBackend};
    use crate::model::StaleReason;

    const V7: &str = include_str!("../../../tests/fixtures/yahoo/quote_v7_bbca_bbri_tlkm.json");
    const V7_PARTIAL: &str =
        include_str!("../../../tests/fixtures/yahoo/quote_v7_partial_bbca_zzzz.json");
    const V7_401: &str = include_str!("../../../tests/fixtures/yahoo/quote_v7_unauthorized.json");
    const CHART_1MO: &str = include_str!("../../../tests/fixtures/yahoo/chart_bbca_1mo_1d.json");
    const CHART_404: &str = include_str!("../../../tests/fixtures/yahoo/chart_not_found.json");

    fn session_routes(be: FixtureBackend) -> FixtureBackend {
        be.route_with_headers(
            "https://fc.yahoo.com",
            404,
            "",
            &[(
                "set-cookie",
                "A3=d=abc&S=xyz; Expires=Fri; Domain=.yahoo.com; Secure; HttpOnly",
            )],
        )
        .route("v1/test/getcrumb", 200, "crumb123")
    }

    fn yahoo(
        be: FixtureBackend,
        mode: CacheMode,
    ) -> (Yahoo, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(be);
        let fetcher = Arc::new(Fetcher::new(be.clone(), Cache::new(dir.path()), mode));
        (Yahoo::new(fetcher, Ttls::default()), be, dir)
    }

    fn syms(list: &[&str]) -> Vec<Symbol> {
        list.iter().map(|s| Symbol::parse(s).unwrap()).collect()
    }

    #[tokio::test]
    async fn quotes_via_v7_with_session() {
        let be = session_routes(FixtureBackend::new()).route("v7/finance/quote", 200, V7);
        let (y, be, _d) = yahoo(be, CacheMode::Normal);
        let got = y.quotes(&syms(&["BBCA", "BBRI", "TLKM"])).await.unwrap();
        assert_eq!(got.provider, "yahoo");
        assert_eq!(got.data.len(), 3);
        assert_eq!(got.data[0].symbol.as_str(), "BBCA");
        assert_eq!(got.data[0].price, Some(5175.0));
        assert_eq!(got.data[0].previous_close, Some(6075.0));
        assert!(got.warnings.is_empty());
        // fc + crumb + quote
        assert_eq!(be.call_count(), 3);
        {
            let calls = be.calls.lock().unwrap();
            assert!(calls[2].contains("crumb=crumb123"), "{}", calls[2]);
        }

        // Second call: session and quote both come from cache -> no network.
        let again = y.quotes(&syms(&["BBCA", "BBRI", "TLKM"])).await.unwrap();
        assert!(again.cached);
        assert_eq!(be.call_count(), 3);
    }

    #[tokio::test]
    async fn stale_quote_says_why_without_the_crumb() {
        // A session but no quote route: the live request fails like a
        // refused connection, and the URL it names carries the crumb.
        let be = session_routes(FixtureBackend::new());
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let symbols = syms(&["BBCA", "BBRI", "TLKM"]);
        cache
            .put_at(
                &Yahoo::quote_cache_key(&symbols),
                200,
                V7,
                chrono::Utc::now() - chrono::Duration::hours(1),
            )
            .unwrap();
        let fetcher = Arc::new(Fetcher::new(Arc::new(be), cache, CacheMode::Normal));
        let y = Yahoo::new(fetcher, Ttls::default());
        let got = y.quotes(&symbols).await.unwrap();
        assert_eq!(got.data.len(), 3);
        assert!(got.cached && got.stale);
        assert_eq!(got.stale_reason, Some(StaleReason::FetchFailed));
        assert_eq!(got.warnings.len(), 1, "{:?}", got.warnings);
        let w = &got.warnings[0];
        assert!(w.starts_with("live fetch failed ("), "{w}");
        assert!(w.contains("v7/finance/quote"), "{w}");
        assert!(!w.contains("crumb") && !w.contains("symbols="), "{w}");
    }

    #[tokio::test]
    async fn unknown_symbol_is_a_warning_when_others_exist() {
        let be = session_routes(FixtureBackend::new()).route("v7/finance/quote", 200, V7_PARTIAL);
        let (y, _be, _d) = yahoo(be, CacheMode::Normal);
        let got = y.quotes(&syms(&["BBCA", "ZZZZ"])).await.unwrap();
        assert_eq!(got.data.len(), 1);
        assert_eq!(got.warnings, vec!["no data for ZZZZ".to_string()]);
    }

    #[tokio::test]
    async fn v7_rejection_refreshes_then_falls_back_to_chart() {
        let be = session_routes(FixtureBackend::new())
            .route("v7/finance/quote", 401, V7_401)
            .route("v8/finance/chart/BBCA.JK", 200, CHART_1MO)
            .route("v8/finance/chart/ZZZZ.JK", 404, CHART_404);
        let (y, be, _d) = yahoo(be, CacheMode::Normal);
        let got = y.quotes(&syms(&["BBCA", "ZZZZ"])).await.unwrap();
        assert_eq!(got.data.len(), 1);
        assert_eq!(got.data[0].source, "yahoo:chart");
        assert_eq!(got.data[0].price, Some(5175.0));
        assert!(got.warnings.iter().any(|w| w.contains("no data for ZZZZ")));
        assert!(got.warnings[0].contains("reduced fields"));
        let calls = be.calls.lock().unwrap();
        // fc, crumb, quote(401), fc, crumb, quote(401), chart x2
        assert_eq!(calls.iter().filter(|u| u.contains("getcrumb")).count(), 2);
        assert_eq!(
            calls
                .iter()
                .filter(|u| u.contains("v8/finance/chart"))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn all_unknown_is_not_found() {
        let be = session_routes(FixtureBackend::new()).route(
            "v7/finance/quote",
            200,
            r#"{"quoteResponse":{"result":[],"error":null}}"#,
        );
        let (y, _be, _d) = yahoo(be, CacheMode::Normal);
        let err = y.quotes(&syms(&["ZZZZ"])).await.unwrap_err();
        assert_eq!(err.exit_code(), 4);
    }

    #[tokio::test]
    async fn history_parses_and_maps_404() {
        let be = FixtureBackend::new()
            .route("v8/finance/chart/BBCA.JK", 200, CHART_1MO)
            .route("v8/finance/chart/ZZZZ.JK", 404, CHART_404);
        let (y, _be, _d) = yahoo(be, CacheMode::Normal);
        let h = y
            .history(&Symbol::parse("BBCA").unwrap(), Period::M1, Interval::Day1)
            .await
            .unwrap();
        assert_eq!(h.data.candles.len(), 22);
        assert_eq!(h.data.summary.as_ref().unwrap().last_close, 6075.0);
        let err = y
            .history(&Symbol::parse("ZZZZ").unwrap(), Period::M1, Interval::Day1)
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(err.to_string().contains("delisted"), "{err}");
    }

    #[tokio::test]
    async fn offline_without_cache_is_exit_5() {
        let (y, _be, _d) = yahoo(FixtureBackend::new(), CacheMode::Offline);
        let err = y.quotes(&syms(&["BBCA"])).await.unwrap_err();
        assert_eq!(err.exit_code(), 5, "{err}");
    }

    const QS_BBCA: &str =
        include_str!("../../../tests/fixtures/yahoo/quote_summary_fundamentals_bbca.json");
    const QS_404: &str = include_str!("../../../tests/fixtures/yahoo/quote_summary_not_found.json");
    const TS_BBCA: &str = include_str!("../../../tests/fixtures/yahoo/timeseries_bbca.json");

    #[tokio::test]
    async fn fundamentals_and_profile_share_one_request() {
        let be = session_routes(FixtureBackend::new())
            .route("quoteSummary/BBCA.JK", 200, QS_BBCA)
            .route("quoteSummary/ZZZZ.JK", 404, QS_404);
        let (y, be, _d) = yahoo(be, CacheMode::Normal);
        let f = y
            .fundamentals(&Symbol::parse("BBCA").unwrap())
            .await
            .unwrap();
        assert_eq!(f.provider, "yahoo");
        assert!((f.data.valuation.pe_trailing.unwrap() - 12.870893).abs() < 1e-6);
        let n = be.call_count();
        let p = y.profile(&Symbol::parse("BBCA").unwrap()).await.unwrap();
        assert_eq!(p.data.sector.as_deref(), Some("Financial Services"));
        assert_eq!(be.call_count(), n, "profile served from the cached summary");
        let err = y
            .profile(&Symbol::parse("ZZZZ").unwrap())
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4, "{err}");
    }

    #[tokio::test]
    async fn financials_via_timeseries_without_session() {
        let be = FixtureBackend::new().route("fundamentals-timeseries", 200, TS_BBCA);
        let (y, be, _d) = yahoo(be, CacheMode::Normal);
        let f = y
            .financials(&Symbol::parse("BBCA").unwrap(), StatementPeriod::Annual)
            .await
            .unwrap();
        assert_eq!(f.data.periods.len(), 4);
        let calls = be.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("type=annualTotalRevenue"), "{}", calls[0]);
        assert!(!calls[0].contains("crumb"));
    }

    #[test]
    fn url_building() {
        let u = Yahoo::chart_url(&Symbol::parse("bbca").unwrap(), Period::Y1, Interval::Week1);
        assert_eq!(
            u,
            "https://query1.finance.yahoo.com/v8/finance/chart/BBCA.JK?range=1y&interval=1wk&includePrePost=false"
        );
        assert_eq!(urlencode("a/b c"), "a%2Fb%20c");
    }
}
