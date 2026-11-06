//! MSN Money provider.
//!
//! Endpoints (undocumented, verified 2026-10-17; see `tests/fixtures/msn/README.md`):
//! - Symbol lookup: `GET https://services.bingapis.com/contentservices-finance.csautosuggest/api/v1/Query?query=BBCA&market=en-id`
//!   returns candidates across exchanges; we keep the one whose `ExMicCode` is
//!   `XIDX` and remember its `SecId` for 30 days.
//! - `GET https://assets.msn.com/service/Finance/Quotes?ids=<secid>,..&apikey=..`
//! - `GET https://assets.msn.com/service/Finance/Equities?ids=<secid>&apikey=..`
//!   (profile, key metrics, analyst estimates).
//!
//! The `apikey` is MSN's own public web-client key, not a user credential; the
//! service answers 401 without one. Override it with `IDX_MSN_API_KEY` if MSN
//! rotates it. MSN's `annualStatements` block has broken units for IDR and is
//! deliberately not parsed; financial statements come from Yahoo.
//! History is not served by MSN here (`Unsupported`), so `auto` uses Yahoo.

use std::sync::Arc;

use async_trait::async_trait;
use futures::future::join_all;

use crate::error::{IdxError, Result};
use crate::http::Fetcher;
use crate::model::{Fundamentals, History, Interval, Period, Profile, Quote, Symbol};

use super::{Provider, Sourced, Ttls};

pub mod parse;

pub use parse::{parse_autosuggest, parse_quotes};

pub const NAME: &str = "msn";
pub const DEFAULT_API_KEY: &str = "0QfOX3Vn51YCzitbLaRkTTBadtWpgTN8NZLW0C1SEM";
const AUTOSUGGEST_URL: &str =
    "https://services.bingapis.com/contentservices-finance.csautosuggest/api/v1/Query";
const SERVICE_BASE: &str = "https://assets.msn.com/service/Finance/";
const COMMON_QUERY: &str = "cm=en-id&ocid=finance-utils-peregrine&it=web&scn=ANON&wrapodata=false";

pub struct Msn {
    fetcher: Arc<Fetcher>,
    ttls: Ttls,
    api_key: String,
}

impl Msn {
    pub fn new(fetcher: Arc<Fetcher>, ttls: Ttls, api_key: Option<String>) -> Self {
        Msn {
            fetcher,
            ttls,
            api_key: api_key
                .filter(|k| !k.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_API_KEY.to_string()),
        }
    }

    pub fn autosuggest_url(symbol: &Symbol) -> String {
        format!("{AUTOSUGGEST_URL}?query={}&market=en-id", symbol.as_str())
    }

    /// `(url, cache_key)`; the key leaves the api key out.
    fn service_url(&self, endpoint: &str, ids: &[String]) -> (String, String) {
        let key = format!("{SERVICE_BASE}{endpoint}?ids={}", ids.join(","));
        let url = format!("{key}&apikey={}&{COMMON_QUERY}", self.api_key);
        (url, key)
    }

    fn offline_miss(symbol: &Symbol, what: &str) -> IdxError {
        IdxError::OfflineMiss(format!("no cached MSN {what} for {symbol}"))
    }

    /// Resolves the MSN security id for an IDX ticker (cached 30 days).
    /// MSN keeps only the IDX row, so a ticker from another market
    /// (`D05.SI`) is unsupported here and `auto` leaves it to Yahoo.
    async fn lookup(&self, symbol: &Symbol) -> Result<String> {
        if symbol.exchange().is_some() {
            return Err(IdxError::unsupported(NAME, "tickers outside IDX"));
        }
        let url = Self::autosuggest_url(symbol);
        let f = match self.fetcher.get(&url, &[], self.ttls.lookup, None).await {
            Ok(f) => f,
            Err(IdxError::OfflineMiss(_)) => return Err(Self::offline_miss(symbol, "id")),
            Err(e) => return Err(e),
        };
        if !f.is_success() {
            return Err(IdxError::provider(
                NAME,
                format!("symbol lookup returned HTTP {}", f.status),
            ));
        }
        parse_autosuggest(&f.body, symbol)?
            .ok_or_else(|| IdxError::NotFound(format!("{symbol} is not listed on IDX per MSN")))
    }

    async fn fetch_service(
        &self,
        endpoint: &str,
        ids: &[String],
        ttl: std::time::Duration,
    ) -> Result<crate::http::Fetched> {
        let (url, key) = self.service_url(endpoint, ids);
        let f = self.fetcher.get(&url, &[], ttl, Some(&key)).await?;
        match f.status {
            200..=299 | 404 => Ok(f),
            401 | 403 => Err(IdxError::provider(
                NAME,
                "MSN rejected the API key; set IDX_MSN_API_KEY to the key msn.com/money currently uses",
            )),
            s => Err(IdxError::provider(NAME, format!("{endpoint} returned HTTP {s}"))),
        }
    }

    async fn quote_for(&self, symbol: &Symbol, id: &str) -> Result<Sourced<Option<Quote>>> {
        let f = self
            .fetch_service("Quotes", &[id.to_string()], self.ttls.quote)
            .await
            .map_err(|e| match e {
                IdxError::OfflineMiss(_) => Self::offline_miss(symbol, "quote"),
                other => other,
            })?;
        let q = parse_quotes(&f.body)?.into_iter().next();
        Ok(Sourced {
            data: q,
            provider: NAME,
            cached: f.cached,
            stale: f.stale,
            fetched_at: f.fetched_at,
            warnings: vec![],
        })
    }

    async fn equity(&self, symbol: &Symbol, id: &str) -> Result<Sourced<String>> {
        let f = self
            .fetch_service("Equities", &[id.to_string()], self.ttls.fundamentals)
            .await
            .map_err(|e| match e {
                IdxError::OfflineMiss(_) => Self::offline_miss(symbol, "company data"),
                other => other,
            })?;
        if f.status == 404 {
            return Err(IdxError::NotFound(format!(
                "MSN has no company data for {symbol}"
            )));
        }
        Ok(Sourced {
            data: f.body,
            provider: NAME,
            cached: f.cached,
            stale: f.stale,
            fetched_at: f.fetched_at,
            warnings: vec![],
        })
    }
}

#[async_trait]
impl Provider for Msn {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn quotes(&self, symbols: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
        let lookups = join_all(symbols.iter().map(|s| self.lookup(s))).await;
        let mut ids = Vec::new();
        let mut warnings = Vec::new();
        let mut first_err: Option<IdxError> = None;
        for (s, r) in symbols.iter().zip(lookups) {
            match r {
                Ok(id) => ids.push((s.clone(), id)),
                Err(IdxError::NotFound(_)) => warnings.push(format!("no data for {s}")),
                Err(e) => {
                    warnings.push(format!("{s}: {e}"));
                    first_err.get_or_insert(e);
                }
            }
        }
        if ids.is_empty() {
            if let Some(e) = first_err {
                return Err(e);
            }
            let list: Vec<&str> = symbols.iter().map(Symbol::as_str).collect();
            return Err(IdxError::NotFound(format!(
                "no quotes for {}",
                list.join(", ")
            )));
        }
        let id_list: Vec<String> = ids.iter().map(|(_, id)| id.clone()).collect();
        let f = self
            .fetch_service("Quotes", &id_list, self.ttls.quote)
            .await
            .map_err(|e| match e {
                IdxError::OfflineMiss(_) => {
                    let list: Vec<&str> = symbols.iter().map(Symbol::as_str).collect();
                    IdxError::OfflineMiss(format!("no cached quote for {}", list.join(", ")))
                }
                other => other,
            })?;
        let quotes = parse_quotes(&f.body)?;
        for (s, _) in &ids {
            if !quotes.iter().any(|q| &q.symbol == s) {
                warnings.push(format!("no data for {s}"));
            }
        }
        if quotes.is_empty() {
            return Err(IdxError::NotFound("MSN returned no quotes".into()));
        }
        Ok(Sourced {
            data: quotes,
            provider: NAME,
            cached: f.cached,
            stale: f.stale,
            fetched_at: f.fetched_at,
            warnings,
        })
    }

    async fn history(
        &self,
        _symbol: &Symbol,
        _period: Period,
        _interval: Interval,
    ) -> Result<Sourced<History>> {
        Err(IdxError::unsupported(NAME, "price history"))
    }

    async fn profile(&self, symbol: &Symbol) -> Result<Sourced<Profile>> {
        let id = self.lookup(symbol).await?;
        let eq = self.equity(symbol, &id).await?;
        let quote = self.quote_for(symbol, &id).await.ok();
        let profile = parse::parse_profile(
            &eq.data,
            symbol,
            quote.as_ref().and_then(|q| q.data.as_ref()),
        )?
        .ok_or_else(|| IdxError::NotFound(format!("MSN has no company data for {symbol}")))?;
        let mut out = Sourced {
            data: profile,
            provider: NAME,
            cached: eq.cached,
            stale: eq.stale,
            fetched_at: eq.fetched_at,
            warnings: vec![],
        };
        if let Some(q) = &quote {
            out.absorb(q);
        }
        Ok(out)
    }

    async fn fundamentals(&self, symbol: &Symbol) -> Result<Sourced<Fundamentals>> {
        let id = self.lookup(symbol).await?;
        let eq = self.equity(symbol, &id).await?;
        let quote = self.quote_for(symbol, &id).await.ok();
        let f = parse::parse_fundamentals(
            &eq.data,
            symbol,
            quote.as_ref().and_then(|q| q.data.as_ref()),
        )?
        .ok_or_else(|| IdxError::NotFound(format!("MSN has no company data for {symbol}")))?;
        let mut out = Sourced {
            data: f,
            provider: NAME,
            cached: eq.cached,
            stale: eq.stale,
            fetched_at: eq.fetched_at,
            warnings: vec![],
        };
        if let Some(q) = &quote {
            out.absorb(q);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::http::{CacheMode, FixtureBackend};

    const AS_BBCA: &str = include_str!("../../../tests/fixtures/msn/autosuggest_bbca.json");
    const AS_ZZZZ: &str =
        include_str!("../../../tests/fixtures/msn/autosuggest_zzzz_no_idx_match.json");
    const Q1: &str = include_str!("../../../tests/fixtures/msn/quotes_bbca.json");
    const EQ: &str = include_str!("../../../tests/fixtures/msn/equities_bbca.json");

    fn msn(be: FixtureBackend, mode: CacheMode) -> (Msn, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(be);
        let fetcher = Arc::new(Fetcher::new(be.clone(), Cache::new(dir.path()), mode));
        (Msn::new(fetcher, Ttls::default(), None), be, dir)
    }

    fn sym(s: &str) -> Symbol {
        Symbol::parse(s).unwrap()
    }

    #[tokio::test]
    async fn other_markets_are_unsupported_before_any_request() {
        let (m, be, _d) = msn(FixtureBackend::new(), CacheMode::Normal);
        let d05 = Symbol::parse_on("D05.SI", "JK").unwrap();
        let unsupported = |e: IdxError| matches!(e, IdxError::Unsupported { .. });
        assert!(unsupported(
            m.quotes(std::slice::from_ref(&d05)).await.unwrap_err()
        ));
        assert!(unsupported(m.profile(&d05).await.unwrap_err()));
        assert!(unsupported(m.fundamentals(&d05).await.unwrap_err()));
        assert_eq!(be.call_count(), 0);
    }

    #[tokio::test]
    async fn quotes_resolve_id_then_fetch() {
        let be = FixtureBackend::new()
            .route("csautosuggest/api/v1/Query?query=BBCA", 200, AS_BBCA)
            .route("csautosuggest/api/v1/Query?query=ZZZZ", 200, AS_ZZZZ)
            .route("Finance/Quotes?ids=bn91jc", 200, Q1);
        let (m, be, _d) = msn(be, CacheMode::Normal);
        let got = m.quotes(&[sym("BBCA"), sym("ZZZZ")]).await.unwrap();
        assert_eq!(got.provider, "msn");
        assert_eq!(got.data.len(), 1);
        assert_eq!(got.data[0].price, Some(5175.0));
        assert_eq!(got.warnings, vec!["no data for ZZZZ".to_string()]);
        {
            let calls = be.calls.lock().unwrap();
            let q = calls.iter().find(|u| u.contains("Finance/Quotes")).unwrap();
            assert!(q.contains(&format!("apikey={DEFAULT_API_KEY}")), "{q}");
        }
        // Lookup and quote are cached: no new calls.
        let n = be.call_count();
        m.quotes(&[sym("BBCA")]).await.unwrap();
        assert_eq!(be.call_count(), n);
    }

    #[tokio::test]
    async fn unknown_everywhere_is_not_found() {
        let be = FixtureBackend::new().route("csautosuggest", 200, AS_ZZZZ);
        let (m, _be, _d) = msn(be, CacheMode::Normal);
        let err = m.quotes(&[sym("ZZZZ")]).await.unwrap_err();
        assert_eq!(err.exit_code(), 4);
    }

    #[tokio::test]
    async fn rejected_key_is_a_provider_error() {
        let be = FixtureBackend::new()
            .route("csautosuggest", 200, AS_BBCA)
            .route("Finance/Quotes", 401, "");
        let (m, _be, _d) = msn(be, CacheMode::Normal);
        let err = m.quotes(&[sym("BBCA")]).await.unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert!(err.to_string().contains("IDX_MSN_API_KEY"), "{err}");
    }

    #[tokio::test]
    async fn profile_and_fundamentals_use_equities_plus_quote() {
        let be = FixtureBackend::new()
            .route("csautosuggest", 200, AS_BBCA)
            .route("Finance/Quotes", 200, Q1)
            .route("Finance/Equities", 200, EQ);
        let (m, _be, _d) = msn(be, CacheMode::Normal);
        let p = m.profile(&sym("BBCA")).await.unwrap();
        assert_eq!(p.data.founded_year, Some(1955));
        assert_eq!(p.data.market_cap, Some(741_407_000_000_000.0));
        let f = m.fundamentals(&sym("BBCA")).await.unwrap();
        assert_eq!(f.data.price, Some(5175.0));
        assert_eq!(f.data.valuation.price_to_book, Some(2.66));
        assert!(m
            .history(&sym("BBCA"), Period::Y1, Interval::Day1)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn offline_miss_is_exit_5() {
        let (m, _be, _d) = msn(FixtureBackend::new(), CacheMode::Offline);
        let err = m.quotes(&[sym("BBCA")]).await.unwrap_err();
        assert_eq!(err.exit_code(), 5, "{err}");
    }

    #[test]
    fn urls() {
        assert_eq!(
            Msn::autosuggest_url(&sym("bbca")),
            "https://services.bingapis.com/contentservices-finance.csautosuggest/api/v1/Query?query=BBCA&market=en-id"
        );
    }
}
