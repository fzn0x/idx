//! `history_provider`: where candles come from, apart from `--provider`.
//!
//! Every candle consumer (`stocks history`, `technical`, `returns`, `brief`,
//! bandar price moves, `watch returns`, MCP `idx_history`) calls
//! [`Provider::history`] on the provider `providers::build` returns, so routing
//! happens here once, for the CLI and MCP alike:
//!
//! | `--provider` | `history_provider` | candles from |
//! | --- | --- | --- |
//! | auto, yahoo | auto, yahoo | the selected provider (Yahoo) |
//! | msn | auto, yahoo | Yahoo (MSN has no history) |
//! | stockbit | auto | Stockbit when it serves the request as asked, then Yahoo if it fails; Yahoo otherwise, with a note |
//! | stockbit | yahoo | Yahoo |
//! | any | stockbit | Stockbit, no fallback |
//!
//! "As asked" is [`stockbit::serves_as_asked`]: Stockbit picks the bar size
//! from the window, so a 2y or 5y "daily" request comes back weekly and would
//! feed weekly returns into daily analytics.
//!
//! Every other capability goes to the selected provider untouched. The
//! envelope's `provider` names the source that served the candles.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::{IdxError, Result};
use crate::http::Fetcher;
use crate::model::{
    DividendHistory, Earnings, Financials, Fundamentals, History, Insights, InstitutionalOwnership,
    Interval, MarketReturns, Period, Profile, Quote, StatementPeriod, Symbol,
};

use super::{
    stockbit, yahoo, Chain, HistoryProviderKind, Provider, ProviderKind, ProviderOptions, Sourced,
};

/// One place candles can come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The provider `--provider` selected.
    Selected,
    Yahoo,
    Stockbit,
}

/// The sources that serve one history request, in the order to try them, and
/// a note for the envelope when the candles do not come from the selected
/// provider for a reason the user may not expect.
pub fn plan(
    selected: ProviderKind,
    mode: HistoryProviderKind,
    period: Period,
    interval: Interval,
) -> (Vec<Source>, Option<String>) {
    use HistoryProviderKind as H;
    use ProviderKind as P;
    match (selected, mode) {
        (P::Stockbit, H::Stockbit) => (vec![Source::Selected], None),
        (_, H::Stockbit) => (vec![Source::Stockbit], None),
        // `auto` is Yahoo then MSN, and MSN has no history: the chain is Yahoo.
        (P::Auto | P::Yahoo, _) => (vec![Source::Selected], None),
        (P::Msn, _) => (vec![Source::Yahoo], None),
        (P::Stockbit, H::Yahoo) => (vec![Source::Yahoo], None),
        (P::Stockbit, H::Auto) => {
            if stockbit::serves_as_asked(period, interval) {
                (vec![Source::Selected, Source::Yahoo], None)
            } else {
                let note = format!(
                    "Stockbit serves {period} at {interval} as {} bars over {}; candles from yahoo (history_provider = stockbit keeps Stockbit)",
                    stockbit::bar_word(stockbit::bars_for(period, interval)),
                    stockbit::window_for(period, interval),
                );
                (vec![Source::Yahoo], Some(note))
            }
        }
    }
}

/// `true` when [`plan`] always answers `[Selected]` for this pair, so no
/// wrapper is needed.
fn trivial(selected: ProviderKind, mode: HistoryProviderKind) -> bool {
    use HistoryProviderKind as H;
    use ProviderKind as P;
    matches!(
        (selected, mode),
        (P::Auto | P::Yahoo, H::Auto | H::Yahoo) | (P::Stockbit, H::Stockbit)
    )
}

/// Wraps `selected` so its history follows `opts.history`. Returns it
/// untouched when the routing is the selected provider's own (the default
/// for `--provider auto|yahoo`). Only the extra sources the plan can use are
/// built; under `--provider stockbit` the Stockbit source is the selected
/// provider, never a second instance.
pub fn route(
    kind: ProviderKind,
    selected: Box<dyn Provider>,
    fetcher: Arc<Fetcher>,
    opts: &ProviderOptions,
) -> Box<dyn Provider> {
    let mode = opts.history;
    if trivial(kind, mode) {
        return selected;
    }
    let yahoo: Option<Box<dyn Provider>> = match (kind, mode) {
        (ProviderKind::Msn | ProviderKind::Stockbit, HistoryProviderKind::Auto)
        | (_, HistoryProviderKind::Yahoo) => {
            Some(Box::new(yahoo::Yahoo::new(fetcher.clone(), opts.ttls)))
        }
        _ => None,
    };
    let stockbit: Option<Box<dyn Provider>> =
        (mode == HistoryProviderKind::Stockbit && kind != ProviderKind::Stockbit).then(|| {
            Box::new(stockbit::Stockbit::new(
                fetcher,
                opts.ttls,
                opts.stockbit.store(),
            )) as Box<dyn Provider>
        });
    Box::new(HistoryRouted {
        inner: selected,
        kind,
        mode,
        yahoo,
        stockbit,
    })
}

/// The selected provider with its candles routed per [`plan`].
pub struct HistoryRouted {
    inner: Box<dyn Provider>,
    kind: ProviderKind,
    mode: HistoryProviderKind,
    yahoo: Option<Box<dyn Provider>>,
    stockbit: Option<Box<dyn Provider>>,
}

impl HistoryRouted {
    pub fn new(
        inner: Box<dyn Provider>,
        kind: ProviderKind,
        mode: HistoryProviderKind,
        yahoo: Option<Box<dyn Provider>>,
        stockbit: Option<Box<dyn Provider>>,
    ) -> Self {
        HistoryRouted {
            inner,
            kind,
            mode,
            yahoo,
            stockbit,
        }
    }

    fn source(&self, s: Source) -> Option<&dyn Provider> {
        match s {
            Source::Selected => Some(self.inner.as_ref()),
            Source::Yahoo => self.yahoo.as_deref(),
            Source::Stockbit => self.stockbit.as_deref(),
        }
    }
}

#[async_trait]
impl Provider for HistoryRouted {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    async fn quotes(&self, symbols: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
        self.inner.quotes(symbols).await
    }

    /// Asks the source that would serve these candles: the first of [`plan`].
    fn serves_interval(&self, period: Period, interval: Interval) -> bool {
        let (sources, _) = plan(self.kind, self.mode, period, interval);
        sources
            .first()
            .and_then(|s| self.source(*s))
            .is_some_and(|p| p.serves_interval(period, interval))
    }

    async fn history(
        &self,
        symbol: &Symbol,
        period: Period,
        interval: Interval,
    ) -> Result<Sourced<History>> {
        let (sources, note) = plan(self.kind, self.mode, period, interval);
        let mut failed: Vec<(&'static str, IdxError)> = Vec::new();
        for s in sources {
            let Some(p) = self.source(s) else {
                failed.push((
                    "history",
                    IdxError::Other(format!("history source {s:?} is not configured")),
                ));
                continue;
            };
            match p.history(symbol, period, interval).await {
                Ok(mut h) => {
                    let mut lead: Vec<String> = note.into_iter().collect();
                    lead.extend(failed.iter().map(|(name, e)| {
                        format!("{name} history failed ({e}); candles from {}", h.provider)
                    }));
                    h.warnings.splice(0..0, lead);
                    return Ok(h);
                }
                Err(e) => {
                    tracing::info!(provider = p.name(), error = %e, "history source failed");
                    failed.push((p.name(), e));
                }
            }
        }
        Err(Chain::pick(failed.into_iter().map(|(_, e)| e).collect()))
    }

    async fn profile(&self, symbol: &Symbol) -> Result<Sourced<Profile>> {
        self.inner.profile(symbol).await
    }

    async fn fundamentals(&self, symbol: &Symbol) -> Result<Sourced<Fundamentals>> {
        self.inner.fundamentals(symbol).await
    }

    async fn financials(
        &self,
        symbol: &Symbol,
        period: StatementPeriod,
    ) -> Result<Sourced<Financials>> {
        self.inner.financials(symbol, period).await
    }

    async fn earnings(&self, symbol: &Symbol) -> Result<Sourced<Earnings>> {
        self.inner.earnings(symbol).await
    }

    async fn dividends(&self, symbol: &Symbol) -> Result<Sourced<DividendHistory>> {
        self.inner.dividends(symbol).await
    }

    async fn insights(&self, symbol: &Symbol) -> Result<Sourced<Insights>> {
        self.inner.insights(symbol).await
    }

    async fn market_returns(&self) -> Result<Sourced<Vec<MarketReturns>>> {
        self.inner.market_returns().await
    }

    async fn institutions(&self, symbol: &Symbol) -> Result<Sourced<InstitutionalOwnership>> {
        self.inner.institutions(symbol).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use HistoryProviderKind as H;
    use ProviderKind as P;
    use Source::{Selected, Stockbit, Yahoo};

    #[test]
    fn plan_covers_every_routing_row() {
        let daily_1mo = (Period::M1, Interval::Day1);
        let daily_5y = (Period::Y5, Interval::Day1);
        let plan_of = |p, h, (period, interval)| plan(p, h, period, interval);

        // Default for the free providers: untouched.
        for p in [P::Auto, P::Yahoo] {
            for h in [H::Auto, H::Yahoo] {
                assert_eq!(plan_of(p, h, daily_5y), (vec![Selected], None));
                assert!(trivial(p, h));
            }
        }
        // MSN has no history: Yahoo serves it.
        assert_eq!(plan_of(P::Msn, H::Auto, daily_1mo), (vec![Yahoo], None));
        assert_eq!(plan_of(P::Msn, H::Yahoo, daily_1mo), (vec![Yahoo], None));
        // Pinned to Stockbit: Stockbit only, whatever the request.
        for p in [P::Auto, P::Yahoo, P::Msn] {
            assert_eq!(plan_of(p, H::Stockbit, daily_5y), (vec![Stockbit], None));
            assert!(!trivial(p, H::Stockbit));
        }
        assert_eq!(
            plan_of(P::Stockbit, H::Stockbit, daily_5y),
            (vec![Selected], None)
        );
        assert!(trivial(P::Stockbit, H::Stockbit));
        // Stockbit selected, history pinned to Yahoo.
        assert_eq!(
            plan_of(P::Stockbit, H::Yahoo, daily_1mo),
            (vec![Yahoo], None)
        );
        // Stockbit selected, auto: Stockbit where it serves as asked, Yahoo after.
        for (period, interval) in [
            (Period::M1, Interval::Day1),
            (Period::Y1, Interval::Day1),
            (Period::D5, Interval::Hour1),
            (Period::D1, Interval::Min5),
            (Period::Y5, Interval::Week1),
        ] {
            assert_eq!(
                plan(P::Stockbit, H::Auto, period, interval),
                (vec![Selected, Yahoo], None),
                "{period} {interval}"
            );
        }
        // ...and Yahoo with a note otherwise.
        let (sources, note) = plan(P::Stockbit, H::Auto, Period::Y5, Interval::Day1);
        assert_eq!(sources, vec![Yahoo]);
        let note = note.unwrap();
        assert!(note.contains("weekly bars over five years"), "{note}");
        assert!(note.contains("candles from yahoo"), "{note}");
        assert!(note.contains("history_provider = stockbit"), "{note}");
        let (_, note) = plan(P::Stockbit, H::Auto, Period::Y1, Interval::Week1);
        assert!(note.unwrap().contains("daily bars over one year"));
        let (_, note) = plan(P::Stockbit, H::Auto, Period::Y2, Interval::Week1);
        assert!(note.unwrap().contains("weekly bars over three years"));
        for (period, interval) in [
            (Period::D1, Interval::Day1),
            (Period::D5, Interval::Day1),
            (Period::D5, Interval::Min15),
            (Period::M1, Interval::Hour1),
            (Period::Y2, Interval::Day1),
            (Period::Max, Interval::Day1),
            (Period::Y5, Interval::Month1),
        ] {
            let (sources, note) = plan(P::Stockbit, H::Auto, period, interval);
            assert_eq!(sources, vec![Yahoo], "{period} {interval}");
            assert!(note.is_some(), "{period} {interval}");
        }
        assert!(!trivial(P::Stockbit, H::Auto));
        assert!(!trivial(P::Msn, H::Auto));
    }

    /// A history source that answers with candles or a fixed error, and counts calls.
    struct Fake {
        name: &'static str,
        fail: Option<fn() -> IdxError>,
        calls: Arc<AtomicUsize>,
    }

    impl Fake {
        fn ok(name: &'static str) -> (Box<dyn Provider>, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            let f = Fake {
                name,
                fail: None,
                calls: calls.clone(),
            };
            (Box::new(f), calls)
        }
        fn failing(
            name: &'static str,
            e: fn() -> IdxError,
        ) -> (Box<dyn Provider>, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            let f = Fake {
                name,
                fail: Some(e),
                calls: calls.clone(),
            };
            (Box::new(f), calls)
        }
    }

    #[async_trait]
    impl Provider for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        async fn quotes(&self, s: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
            Ok(Sourced::fresh(
                vec![Quote::empty(s[0].clone(), self.name)],
                self.name,
            ))
        }
        async fn history(
            &self,
            symbol: &Symbol,
            period: Period,
            interval: Interval,
        ) -> Result<Sourced<History>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(e) = self.fail {
                return Err(e());
            }
            let mut s = Sourced::fresh(
                History {
                    symbol: symbol.clone(),
                    name: None,
                    currency: Some("IDR".into()),
                    period,
                    interval,
                    candles: vec![],
                    summary: None,
                },
                self.name,
            );
            s.warnings.push(format!("{} own warning", self.name));
            Ok(s)
        }
    }

    /// The MSN provider over recorded responses, inside a `Chain` as
    /// `providers::build` makes it.
    fn msn_chain() -> (Box<dyn Provider>, tempfile::TempDir) {
        use crate::cache::Cache;
        use crate::http::{CacheMode, FixtureBackend};
        const AS_BBCA: &str = include_str!("../../tests/fixtures/msn/autosuggest_bbca.json");
        const INSIGHTS: &str = include_str!("../../tests/fixtures/msn/insights_bbca.json");
        const PEERS: &str = include_str!("../../tests/fixtures/msn/quotes_bbca_insight_peers.json");
        const SCREENER: &str =
            include_str!("../../tests/fixtures/msn/screener_52wkhi_id_complete.json");
        const AS_JTPE: &str = include_str!("../../tests/fixtures/msn/autosuggest_jtpe.json");
        const JTPE: &str = include_str!("../../tests/fixtures/msn/equities_jtpe_all_signals.json");
        let be = FixtureBackend::new()
            .route("csautosuggest/api/v1/Query?query=BBCA", 200, AS_BBCA)
            .route("finance/insights?ids=bn91jc", 200, INSIGHTS)
            .route("Finance/Quotes?ids=bkfmar,bkfgqh,cj53lh,bkgttc", 200, PEERS)
            .route("Finance/Screener", 200, SCREENER)
            .route("csautosuggest/api/v1/Query?query=JTPE", 200, AS_JTPE)
            .route("Finance/Equities?ids=bkgwu2", 200, JTPE);
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(be),
            Cache::new(dir.path().join("cache")),
            CacheMode::Normal,
        ));
        let msn = super::super::msn::Msn::new(fetcher, super::super::Ttls::default(), None);
        (Box::new(Chain::new(vec![Box::new(msn)])), dir)
    }

    /// Every capability but candles goes to the wrapped provider untouched,
    /// whichever routing wrapped it (`--provider msn`, `history_provider
    /// stockbit`).
    #[tokio::test]
    async fn every_other_capability_reaches_the_wrapped_chain() {
        for (kind, mode) in [(P::Msn, H::Auto), (P::Auto, H::Stockbit)] {
            let (chain, _dir) = msn_chain();
            let r = HistoryRouted::new(chain, kind, mode, None, None);
            let i = r.insights(&sym()).await.unwrap();
            assert_eq!((i.provider, i.data.symbol.as_str()), ("msn", "BBCA"));
            let m = r.market_returns().await.unwrap();
            assert_eq!(m.provider, "msn");
            assert!(!m.data.is_empty());
            let h = r
                .institutions(&Symbol::parse("JTPE").unwrap())
                .await
                .unwrap();
            assert_eq!((h.provider, h.data.holders.len()), ("msn", 9));
        }
    }

    /// `serves_interval` answers for the source that would serve the
    /// candles: pinned to Stockbit, its five-day window is hourly.
    #[tokio::test]
    async fn serves_interval_follows_the_planned_source() {
        use crate::cache::Cache;
        use crate::http::{CacheMode, FixtureBackend};
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(FixtureBackend::new()),
            Cache::new(dir.path().join("cache")),
            CacheMode::Normal,
        ));
        let sb = || -> Box<dyn Provider> {
            Box::new(stockbit::Stockbit::new(
                fetcher.clone(),
                super::super::Ttls::default(),
                Arc::new(stockbit::SessionStore::new(
                    dir.path().join("session.json"),
                    None,
                    None,
                )),
            ))
        };
        let (inner, _) = Fake::ok("yahoo");
        let pinned = HistoryRouted::new(inner, P::Auto, H::Stockbit, None, Some(sb()));
        assert!(!pinned.serves_interval(Period::D5, Interval::Min1));
        assert!(pinned.serves_interval(Period::D5, Interval::Hour1));
        assert!(pinned.serves_interval(Period::D1, Interval::Min1));
        // Stockbit selected, auto: a five-day 1m request goes to Yahoo, which
        // serves it.
        let (y, _) = Fake::ok("yahoo");
        let auto = HistoryRouted::new(sb(), P::Stockbit, H::Auto, Some(y), None);
        assert!(auto.serves_interval(Period::D5, Interval::Min1));
    }

    fn no_session() -> IdxError {
        IdxError::Usage(
            "no Stockbit session; run `idx stockbit login` (or set IDX_STOCKBIT_REFRESH_TOKEN)"
                .into(),
        )
    }

    fn sym() -> Symbol {
        Symbol::parse("BBCA").unwrap()
    }

    #[tokio::test]
    async fn stockbit_failure_falls_back_to_yahoo_with_a_warning() {
        let (sb, sb_calls) = Fake::failing("stockbit", no_session);
        let (y, y_calls) = Fake::ok("yahoo");
        let r = HistoryRouted::new(sb, P::Stockbit, H::Auto, Some(y), None);
        assert_eq!(r.name(), "stockbit");
        let h = r.history(&sym(), Period::M1, Interval::Day1).await.unwrap();
        assert_eq!(h.provider, "yahoo");
        assert!(h.warnings[0].starts_with("stockbit history failed (no Stockbit session"));
        assert!(h.warnings[0].contains("idx stockbit login"));
        assert!(h.warnings[0].ends_with("candles from yahoo"));
        assert_eq!(h.warnings[1], "yahoo own warning");
        assert_eq!(sb_calls.load(Ordering::SeqCst), 1);
        assert_eq!(y_calls.load(Ordering::SeqCst), 1);
        // Non-history calls stay on the selected provider.
        let q = r.quotes(&[sym()]).await.unwrap();
        assert_eq!(q.provider, "stockbit");
    }

    #[tokio::test]
    async fn stockbit_serves_what_it_can_and_yahoo_the_rest() {
        let (sb, sb_calls) = Fake::ok("stockbit");
        let (y, y_calls) = Fake::ok("yahoo");
        let r = HistoryRouted::new(sb, P::Stockbit, H::Auto, Some(y), None);
        let h = r.history(&sym(), Period::Y1, Interval::Day1).await.unwrap();
        assert_eq!(h.provider, "stockbit");
        assert_eq!(h.warnings, vec!["stockbit own warning".to_string()]);
        let h = r.history(&sym(), Period::Y5, Interval::Day1).await.unwrap();
        assert_eq!(h.provider, "yahoo");
        assert!(h.warnings[0].contains("weekly bars"), "{:?}", h.warnings);
        assert_eq!(h.warnings[1], "yahoo own warning");
        assert_eq!(sb_calls.load(Ordering::SeqCst), 1);
        assert_eq!(y_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn both_failing_prefers_not_found_then_the_first_real_error() {
        let (sb, _) = Fake::failing("stockbit", no_session);
        let (y, _) = Fake::failing("yahoo", || IdxError::NotFound("ZZZZ".into()));
        let r = HistoryRouted::new(sb, P::Stockbit, H::Auto, Some(y), None);
        let e = r
            .history(&sym(), Period::M1, Interval::Day1)
            .await
            .unwrap_err();
        assert_eq!(e.exit_code(), 4);
        let (sb, _) = Fake::failing("stockbit", no_session);
        let (y, _) = Fake::failing("yahoo", || IdxError::Network("down".into()));
        let r = HistoryRouted::new(sb, P::Stockbit, H::Auto, Some(y), None);
        let e = r
            .history(&sym(), Period::M1, Interval::Day1)
            .await
            .unwrap_err();
        assert_eq!(e.exit_code(), 2, "{e}");
    }

    #[tokio::test]
    async fn pinned_stockbit_never_falls_back() {
        let (inner, inner_calls) = Fake::ok("yahoo");
        let (sb, sb_calls) = Fake::failing("stockbit", no_session);
        let r = HistoryRouted::new(inner, P::Auto, H::Stockbit, None, Some(sb));
        let e = r
            .history(&sym(), Period::Y5, Interval::Day1)
            .await
            .unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert!(e.to_string().contains("stockbit login"));
        assert_eq!(sb_calls.load(Ordering::SeqCst), 1);
        assert_eq!(inner_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn msn_history_comes_from_yahoo() {
        let (inner, inner_calls) =
            Fake::failing("msn", || IdxError::unsupported("msn", "price history"));
        let (y, _) = Fake::ok("yahoo");
        let r = HistoryRouted::new(inner, P::Msn, H::Auto, Some(y), None);
        let h = r.history(&sym(), Period::Y1, Interval::Day1).await.unwrap();
        assert_eq!(h.provider, "yahoo");
        assert_eq!(h.warnings, vec!["yahoo own warning".to_string()]);
        assert_eq!(inner_calls.load(Ordering::SeqCst), 0);
        assert_eq!(r.name(), "msn");
    }

    #[tokio::test]
    async fn route_leaves_the_default_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(crate::http::FixtureBackend::new()),
            crate::cache::Cache::new(dir.path()),
            crate::http::CacheMode::Offline,
        ));
        let opts = ProviderOptions::default();
        let (inner, _) = Fake::ok("inner");
        let p = route(P::Auto, inner, fetcher.clone(), &opts);
        assert_eq!(p.name(), "inner");
        // The wrapper keeps the selected provider's name too, so check the
        // routing through a history call instead: MSN's goes to Yahoo, whose
        // offline cache miss proves the inner provider was not asked.
        let (inner, inner_calls) = Fake::ok("msn");
        let p = route(P::Msn, inner, fetcher, &opts);
        assert_eq!(p.name(), "msn");
        let e = p
            .history(&sym(), Period::M1, Interval::Day1)
            .await
            .unwrap_err();
        assert_eq!(e.exit_code(), 5, "{e}");
        assert_eq!(inner_calls.load(Ordering::SeqCst), 0);
    }
}
