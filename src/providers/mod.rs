//! The `Provider` abstraction. Every data source implements this trait and is
//! selected with `--provider`, `IDX_PROVIDER`, or the config file. `auto`
//! builds a [`Chain`] that tries providers in order so one broken source does
//! not take the tool down.
//!
//! A provider that cannot serve a capability returns
//! [`IdxError::Unsupported`]; the chain skips it and moves on.
//!
//! Adding a provider: see `AGENTS.md`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{IdxError, Result};
use crate::http::Fetcher;
use crate::model::{
    DividendHistory, Earnings, Financials, Fundamentals, History, Insights, Interval,
    MarketReturns, Period, Profile, Quote, StatementPeriod, Symbol,
};

pub mod idx;
pub mod ipot;
pub mod msn;
pub mod news;
pub mod stockbit;
pub mod yahoo;

/// Data plus provenance. Renderers put this in the JSON envelope.
#[derive(Debug, Clone)]
pub struct Sourced<T> {
    pub data: T,
    pub provider: &'static str,
    pub cached: bool,
    pub stale: bool,
    pub fetched_at: DateTime<Utc>,
    pub warnings: Vec<String>,
}

impl<T> Sourced<T> {
    pub fn fresh(data: T, provider: &'static str) -> Self {
        Sourced {
            data,
            provider,
            cached: false,
            stale: false,
            fetched_at: Utc::now(),
            warnings: vec![],
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Sourced<U> {
        Sourced {
            data: f(self.data),
            provider: self.provider,
            cached: self.cached,
            stale: self.stale,
            fetched_at: self.fetched_at,
            warnings: self.warnings,
        }
    }

    /// Folds another response's provenance into this one (earliest fetch,
    /// cached only if both were, stale if either was).
    pub fn absorb<U>(&mut self, other: &Sourced<U>) {
        self.cached &= other.cached;
        self.stale |= other.stale;
        self.fetched_at = self.fetched_at.min(other.fetched_at);
        for w in &other.warnings {
            if !self.warnings.contains(w) {
                self.warnings.push(w.clone());
            }
        }
    }
}

/// Per-endpoint cache lifetimes.
#[derive(Debug, Clone, Copy)]
pub struct Ttls {
    pub quote: Duration,
    pub history: Duration,
    /// Profile, ratios, earnings, MSN insights.
    pub fundamentals: Duration,
    /// Financial statements.
    pub financials: Duration,
    /// Symbol-to-provider-id lookups.
    pub lookup: Duration,
}

impl Default for Ttls {
    fn default() -> Self {
        Ttls {
            quote: Duration::from_secs(60),
            history: Duration::from_secs(3600),
            fundamentals: Duration::from_secs(6 * 3600),
            financials: Duration::from_secs(24 * 3600),
            lookup: Duration::from_secs(30 * 24 * 3600),
        }
    }
}

/// Everything `build` needs besides the fetcher.
#[derive(Debug, Clone, Default)]
pub struct ProviderOptions {
    pub ttls: Ttls,
    /// Overrides MSN's built-in public web key (`IDX_MSN_API_KEY`).
    pub msn_api_key: Option<String>,
    /// Stockbit session file and env-seed tokens. The path is empty only in
    /// tests that never select the Stockbit provider.
    pub stockbit: StockbitOptions,
}

/// Where the Stockbit session lives and what seeds it. The file is authoritative;
/// the seeds only fill an empty one (for the web deployment, which injects the
/// tokens through the environment).
#[derive(Debug, Clone, Default)]
pub struct StockbitOptions {
    pub session_path: PathBuf,
    pub access_seed: Option<String>,
    pub refresh_seed: Option<String>,
}

impl StockbitOptions {
    /// Builds the shared session store this process will read and refresh.
    pub fn store(&self) -> Arc<stockbit::SessionStore> {
        Arc::new(stockbit::SessionStore::new(
            self.session_path.clone(),
            self.access_seed.clone(),
            self.refresh_seed.clone(),
        ))
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;

    /// Quotes for one or more symbols. Symbols the source does not know are
    /// reported in `warnings`; if none are known the result is `NotFound`.
    async fn quotes(&self, symbols: &[Symbol]) -> Result<Sourced<Vec<Quote>>>;

    /// OHLCV bars, oldest first.
    async fn history(
        &self,
        symbol: &Symbol,
        period: Period,
        interval: Interval,
    ) -> Result<Sourced<History>>;

    async fn profile(&self, _symbol: &Symbol) -> Result<Sourced<Profile>> {
        Err(IdxError::unsupported(self.name(), "company profiles"))
    }

    /// Ratios without the verdict; `analysis::fundamental::verdict` adds it.
    async fn fundamentals(&self, _symbol: &Symbol) -> Result<Sourced<Fundamentals>> {
        Err(IdxError::unsupported(self.name(), "fundamentals"))
    }

    /// Rule-based statements about one company (MSN Money insights).
    async fn insights(&self, _symbol: &Symbol) -> Result<Sourced<Insights>> {
        Err(IdxError::unsupported(self.name(), "insights"))
    }

    /// Price returns (1w to 1y, YTD) for every listed stock in one call, for
    /// the screener's return-window keys.
    async fn market_returns(&self) -> Result<Sourced<Vec<MarketReturns>>> {
        Err(IdxError::unsupported(
            self.name(),
            "market-wide return windows",
        ))
    }

    async fn financials(
        &self,
        _symbol: &Symbol,
        _period: StatementPeriod,
    ) -> Result<Sourced<Financials>> {
        Err(IdxError::unsupported(self.name(), "financial statements"))
    }

    async fn earnings(&self, _symbol: &Symbol) -> Result<Sourced<Earnings>> {
        Err(IdxError::unsupported(self.name(), "earnings"))
    }

    /// Dividend and split events as reported; `analysis::dividends::enrich`
    /// computes the totals and yield afterwards.
    async fn dividends(&self, _symbol: &Symbol) -> Result<Sourced<DividendHistory>> {
        Err(IdxError::unsupported(self.name(), "dividend history"))
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Try each provider in order; fall back when one fails.
    Auto,
    /// Yahoo Finance (chart, quote, quoteSummary, fundamentals timeseries).
    Yahoo,
    /// MSN Money (quotes, profile, fundamentals, insights).
    Msn,
    /// Stockbit realtime (your own session; `idx stockbit login` first).
    Stockbit,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Auto => "auto",
            ProviderKind::Yahoo => "yahoo",
            ProviderKind::Msn => "msn",
            ProviderKind::Stockbit => "stockbit",
        }
    }
}

impl std::str::FromStr for ProviderKind {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <ProviderKind as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            IdxError::Config(format!(
                "unknown provider {s:?}; use auto, yahoo, msn or stockbit"
            ))
        })
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Tries providers in order. Returns the first success. If every provider
/// fails, surfaces a `NotFound` if any provider said so (the symbol is probably
/// wrong), otherwise the first real error, otherwise `Unsupported`.
pub struct Chain {
    providers: Vec<Box<dyn Provider>>,
}

impl Chain {
    pub fn new(providers: Vec<Box<dyn Provider>>) -> Self {
        Chain { providers }
    }

    fn pick(errors: Vec<IdxError>) -> IdxError {
        let mut errors = errors;
        if let Some(pos) = errors
            .iter()
            .position(|e| matches!(e, IdxError::NotFound(_)))
        {
            return errors.swap_remove(pos);
        }
        if let Some(pos) = errors
            .iter()
            .position(|e| !matches!(e, IdxError::Unsupported { .. }))
        {
            return errors.swap_remove(pos);
        }
        errors
            .into_iter()
            .next()
            .unwrap_or_else(|| IdxError::Other("no providers configured".into()))
    }
}

macro_rules! first_ok {
    ($self:ident, $p:ident => $call:expr) => {{
        let mut errors = Vec::new();
        for $p in &$self.providers {
            match $call.await {
                Ok(v) => return Ok(v),
                Err(e @ IdxError::Unsupported { .. }) => {
                    tracing::debug!(provider = $p.name(), %e, "skipping provider");
                    errors.push(e);
                }
                Err(e) => {
                    tracing::info!(provider = $p.name(), error = %e, "provider failed, trying next");
                    errors.push(e);
                }
            }
        }
        Err(Self::pick(errors))
    }};
}

#[async_trait]
impl Provider for Chain {
    fn name(&self) -> &'static str {
        "auto"
    }

    async fn quotes(&self, symbols: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
        first_ok!(self, p => p.quotes(symbols))
    }

    async fn history(
        &self,
        symbol: &Symbol,
        period: Period,
        interval: Interval,
    ) -> Result<Sourced<History>> {
        first_ok!(self, p => p.history(symbol, period, interval))
    }

    /// First provider's profile, with the board and founding year filled from
    /// the others when missing (Yahoo has neither; MSN has both).
    async fn profile(&self, symbol: &Symbol) -> Result<Sourced<Profile>> {
        let mut errors = Vec::new();
        let mut base: Option<Sourced<Profile>> = None;
        for p in &self.providers {
            if base.as_ref().is_some_and(|b| !b.data.has_gaps()) {
                break;
            }
            match p.profile(symbol).await {
                Ok(v) => match &mut base {
                    None => base = Some(v),
                    Some(b) => {
                        b.data.fill_from(&v.data);
                        b.absorb(&v);
                    }
                },
                Err(e @ IdxError::Unsupported { .. }) => errors.push(e),
                Err(e) => {
                    tracing::info!(provider = p.name(), error = %e, "profile provider failed");
                    errors.push(e);
                }
            }
        }
        base.ok_or_else(|| Self::pick(errors))
    }

    /// Takes the first provider's report and fills gaps from the others, so a
    /// bank's missing EV/EBITDA on one source can still come from another.
    async fn fundamentals(&self, symbol: &Symbol) -> Result<Sourced<Fundamentals>> {
        let mut errors = Vec::new();
        let mut base: Option<Sourced<Fundamentals>> = None;
        for p in &self.providers {
            if let Some(b) = &base {
                if b.data.missing_key_fields() == 0 {
                    break;
                }
            }
            match p.fundamentals(symbol).await {
                Ok(v) => match &mut base {
                    None => base = Some(v),
                    Some(b) => {
                        b.data.fill_from(&v.data);
                        b.absorb(&v);
                    }
                },
                Err(e @ IdxError::Unsupported { .. }) => errors.push(e),
                Err(e) => {
                    if base.is_none() {
                        tracing::info!(provider = p.name(), error = %e, "provider failed, trying next");
                    } else {
                        tracing::debug!(provider = p.name(), error = %e, "secondary provider failed");
                    }
                    errors.push(e);
                }
            }
        }
        base.ok_or_else(|| Self::pick(errors))
    }

    async fn insights(&self, symbol: &Symbol) -> Result<Sourced<Insights>> {
        first_ok!(self, p => p.insights(symbol))
    }

    async fn market_returns(&self) -> Result<Sourced<Vec<MarketReturns>>> {
        first_ok!(self, p => p.market_returns())
    }

    async fn financials(
        &self,
        symbol: &Symbol,
        period: StatementPeriod,
    ) -> Result<Sourced<Financials>> {
        first_ok!(self, p => p.financials(symbol, period))
    }

    async fn earnings(&self, symbol: &Symbol) -> Result<Sourced<Earnings>> {
        first_ok!(self, p => p.earnings(symbol))
    }

    async fn dividends(&self, symbol: &Symbol) -> Result<Sourced<DividendHistory>> {
        first_ok!(self, p => p.dividends(symbol))
    }
}

/// Names of the providers `auto` tries, in order.
pub const AUTO_ORDER: &[&str] = &[yahoo::NAME, msn::NAME];

/// Builds the provider for `kind`.
pub fn build(
    kind: ProviderKind,
    fetcher: Arc<Fetcher>,
    opts: ProviderOptions,
) -> Result<Box<dyn Provider>> {
    match kind {
        ProviderKind::Yahoo => Ok(Box::new(yahoo::Yahoo::new(fetcher, opts.ttls))),
        ProviderKind::Msn => Ok(Box::new(msn::Msn::new(
            fetcher,
            opts.ttls,
            opts.msn_api_key,
        ))),
        // Stockbit stands alone: it needs a credential and is never folded into
        // the keyless `auto` chain.
        ProviderKind::Stockbit => Ok(Box::new(stockbit::Stockbit::new(
            fetcher,
            opts.ttls,
            opts.stockbit.store(),
        ))),
        ProviderKind::Auto => Ok(Box::new(Chain::new(vec![
            Box::new(yahoo::Yahoo::new(fetcher.clone(), opts.ttls)),
            Box::new(msn::Msn::new(fetcher, opts.ttls, opts.msn_api_key)),
        ]))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Failing(&'static str, fn() -> IdxError);
    struct Ok1;

    #[async_trait]
    impl Provider for Failing {
        fn name(&self) -> &'static str {
            self.0
        }
        async fn quotes(&self, _: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
            Err((self.1)())
        }
        async fn history(&self, _: &Symbol, _: Period, _: Interval) -> Result<Sourced<History>> {
            Err((self.1)())
        }
        async fn fundamentals(&self, _: &Symbol) -> Result<Sourced<Fundamentals>> {
            Err((self.1)())
        }
    }

    #[async_trait]
    impl Provider for Ok1 {
        fn name(&self) -> &'static str {
            "ok"
        }
        async fn quotes(&self, s: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
            Ok(Sourced::fresh(vec![Quote::empty(s[0].clone(), "ok")], "ok"))
        }
        async fn history(&self, _: &Symbol, _: Period, _: Interval) -> Result<Sourced<History>> {
            unimplemented!()
        }
        async fn fundamentals(&self, s: &Symbol) -> Result<Sourced<Fundamentals>> {
            let mut f = Fundamentals::empty(s.clone(), "ok");
            f.valuation.pe_trailing = Some(9.0);
            Ok(Sourced::fresh(f, "ok"))
        }
        async fn insights(&self, s: &Symbol) -> Result<Sourced<Insights>> {
            Ok(Sourced::fresh(
                Insights {
                    symbol: s.clone(),
                    name: None,
                    updated: None,
                    good: 0,
                    bad: 0,
                    neutral: 0,
                    not_rated: 0,
                    peers: vec![],
                    insights: vec![],
                },
                "ok",
            ))
        }
        async fn market_returns(&self) -> Result<Sourced<Vec<MarketReturns>>> {
            Ok(Sourced::fresh(vec![], "ok"))
        }
    }

    struct Ok2;
    #[async_trait]
    impl Provider for Ok2 {
        fn name(&self) -> &'static str {
            "ok2"
        }
        async fn quotes(&self, _: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
            unimplemented!()
        }
        async fn history(&self, _: &Symbol, _: Period, _: Interval) -> Result<Sourced<History>> {
            unimplemented!()
        }
        async fn fundamentals(&self, s: &Symbol) -> Result<Sourced<Fundamentals>> {
            let mut f = Fundamentals::empty(s.clone(), "ok2");
            f.valuation.pe_trailing = Some(99.0);
            f.valuation.price_to_book = Some(2.0);
            Ok(Sourced::fresh(f, "ok2"))
        }
    }

    fn sym() -> Symbol {
        Symbol::parse("BBCA").unwrap()
    }

    #[tokio::test]
    async fn chain_falls_through_to_working_provider() {
        let chain = Chain::new(vec![
            Box::new(Failing("a", || IdxError::Network("down".into()))),
            Box::new(Ok1),
        ]);
        let got = chain.quotes(&[sym()]).await.unwrap();
        assert_eq!(got.provider, "ok");
    }

    #[tokio::test]
    async fn chain_prefers_not_found_over_network_errors() {
        let chain = Chain::new(vec![
            Box::new(Failing("a", || IdxError::Network("down".into()))),
            Box::new(Failing("b", || IdxError::NotFound("ZZZZ".into()))),
        ]);
        let err = chain.quotes(&[sym()]).await.unwrap_err();
        assert_eq!(err.exit_code(), 4);
    }

    #[tokio::test]
    async fn chain_skips_unsupported_and_reports_real_error() {
        let chain = Chain::new(vec![
            Box::new(Failing("a", || IdxError::unsupported("a", "x"))),
            Box::new(Failing("b", || IdxError::Network("down".into()))),
        ]);
        let err = chain.quotes(&[sym()]).await.unwrap_err();
        assert_eq!(err.exit_code(), 3);
        // Only unsupported providers -> exit 6.
        let chain = Chain::new(vec![Box::new(Failing("a", || {
            IdxError::unsupported("a", "profiles")
        }))]);
        let err = chain.profile(&sym()).await.unwrap_err();
        assert_eq!(err.exit_code(), 6);
    }

    #[tokio::test]
    async fn chain_forwards_msn_only_capabilities() {
        let chain = Chain::new(vec![
            Box::new(Failing("yahoo", || IdxError::unsupported("yahoo", "x"))),
            Box::new(Ok1),
        ]);
        assert_eq!(chain.insights(&sym()).await.unwrap().provider, "ok");
        assert_eq!(chain.market_returns().await.unwrap().provider, "ok");
        // A provider without the capability alone: exit 6.
        let chain = Chain::new(vec![Box::new(Ok2)]);
        assert_eq!(chain.insights(&sym()).await.unwrap_err().exit_code(), 6);
        assert_eq!(chain.market_returns().await.unwrap_err().exit_code(), 6);
    }

    #[tokio::test]
    async fn fundamentals_merge_fills_gaps_only() {
        let chain = Chain::new(vec![Box::new(Ok1), Box::new(Ok2)]);
        let got = chain.fundamentals(&sym()).await.unwrap();
        assert_eq!(got.provider, "ok");
        assert_eq!(got.data.valuation.pe_trailing, Some(9.0));
        assert_eq!(got.data.valuation.price_to_book, Some(2.0));
        assert_eq!(got.data.sources, vec!["ok".to_string(), "ok2".to_string()]);
    }

    #[test]
    fn provider_kind_parses() {
        assert_eq!(
            "Yahoo".parse::<ProviderKind>().unwrap(),
            ProviderKind::Yahoo
        );
        assert_eq!("auto".parse::<ProviderKind>().unwrap(), ProviderKind::Auto);
        assert!("x".parse::<ProviderKind>().is_err());
    }
}
