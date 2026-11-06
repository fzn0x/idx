//! Growth, valuation and risk lenses (`stocks growth | valuation | risk`,
//! MCP `idx_fundamental` with `lens`): the provider calls behind
//! `analysis::lens`. A lens that needs two or three inputs fetches them
//! concurrently and fails only when the essential ones all fail; a missing
//! side becomes a warning and the reading runs with fewer rules.

use crate::analysis::lens as rules;
use crate::error::{IdxError, Result};
use crate::model::{
    GrowthLens, Interval, Lens, Period, RiskLens, StatementPeriod, Symbol, ValuationLens,
};
use crate::providers::{Provider, Sourced};

/// The error a lens reports when every essential input failed: a "not
/// found" from any side wins (the ticker is unknown), else the first error.
fn pick(errors: Vec<IdxError>) -> IdxError {
    let mut errors = errors;
    match errors
        .iter()
        .position(|e| matches!(e, IdxError::NotFound(_)))
    {
        Some(i) => errors.swap_remove(i),
        None => errors.swap_remove(0),
    }
}

/// The provenance of one input, without its data.
fn meta<T>(s: &Sourced<T>) -> Sourced<()> {
    Sourced {
        data: (),
        provider: s.provider,
        cached: s.cached,
        stale: s.stale,
        fetched_at: s.fetched_at,
        warnings: s.warnings.clone(),
    }
}

/// Folds the inputs that arrived into one envelope: the first one's
/// provider, cached only if all were, stale if any was, warnings merged.
fn provenance(parts: impl IntoIterator<Item = Option<Sourced<()>>>) -> Option<Sourced<()>> {
    let mut out: Option<Sourced<()>> = None;
    for p in parts.into_iter().flatten() {
        match &mut out {
            None => out = Some(p),
            Some(acc) => acc.absorb(&p),
        }
    }
    out
}

/// Multiples, yield and the analyst target.
pub async fn valuation(p: &dyn Provider, symbol: &Symbol) -> Result<Sourced<ValuationLens>> {
    let f = p.fundamentals(symbol).await?;
    Ok(f.map(|d| rules::valuation(&d)))
}

/// Latest quarter and forward EPS from the fundamentals, fiscal years from
/// the annual statements. Fails only when both fail.
pub async fn growth(p: &dyn Provider, symbol: &Symbol) -> Result<Sourced<GrowthLens>> {
    let (f, a) = futures::join!(
        p.fundamentals(symbol),
        p.financials(symbol, StatementPeriod::Annual)
    );
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let f = f
        .map_err(|e| {
            warnings.push(format!("fundamentals: {e}"));
            errors.push(e);
        })
        .ok();
    let a = a
        .map_err(|e| {
            warnings.push(format!("annual statements: {e}"));
            errors.push(e);
        })
        .ok();
    let Some(mut out) = provenance([f.as_ref().map(meta), a.as_ref().map(meta)]) else {
        return Err(pick(errors));
    };
    out.warnings.extend(warnings);
    let data = rules::growth(
        symbol,
        f.as_ref().map(|s| &s.data),
        a.as_ref().map(|s| &s.data),
    );
    Ok(out.map(|_| data))
}

/// Balance sheet and earnings from the fundamentals; a year of daily closes
/// for the stock and for IHSG. Fails only when the fundamentals and the
/// stock's own history both fail; without IHSG the price rules become notes.
pub async fn risk(p: &dyn Provider, symbol: &Symbol) -> Result<Sourced<RiskLens>> {
    let bench = Symbol::parse(rules::BENCHMARK)?;
    let (f, h, b) = futures::join!(
        p.fundamentals(symbol),
        p.history(symbol, Period::Y1, Interval::Day1),
        p.history(&bench, Period::Y1, Interval::Day1),
    );
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let f = f
        .map_err(|e| {
            warnings.push(format!("fundamentals: {e}"));
            errors.push(e);
        })
        .ok();
    let h = h
        .map_err(|e| {
            warnings.push(format!("price history: {e}"));
            errors.push(e);
        })
        .ok();
    let b = b
        .map_err(|e| warnings.push(format!("benchmark {bench} unavailable: {e}")))
        .ok();
    if f.is_none() && h.is_none() {
        return Err(pick(errors));
    }
    let mut out = provenance([
        f.as_ref().map(meta),
        h.as_ref().map(meta),
        b.as_ref().map(meta),
    ])
    .expect("an input arrived");
    out.warnings.extend(warnings);
    let data = rules::risk(
        symbol,
        f.as_ref().map(|s| &s.data),
        h.as_ref().map(|s| &s.data),
        b.as_ref().map(|s| &s.data),
    );
    Ok(out.map(|_| data))
}

/// One lens, serialised with its schema name: what `stocks fundamental
/// --lens` and MCP `idx_fundamental` with `lens` return.
pub enum LensResult {
    Growth(Sourced<GrowthLens>),
    Valuation(Sourced<ValuationLens>),
    Risk(Sourced<RiskLens>),
}

pub async fn lens(p: &dyn Provider, symbol: &Symbol, which: Lens) -> Result<LensResult> {
    Ok(match which {
        Lens::Growth => LensResult::Growth(growth(p, symbol).await?),
        Lens::Valuation => LensResult::Valuation(valuation(p, symbol).await?),
        Lens::Risk => LensResult::Risk(risk(p, symbol).await?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fundamentals, History, Quote};
    use async_trait::async_trait;

    /// Fundamentals and statements work for TLKM; the history of the stock
    /// and of the index fail in configurable ways.
    struct Fake {
        fundamentals: bool,
        statements: bool,
        stock_history: bool,
        index_history: bool,
    }

    const TLKM: &str =
        include_str!("../../tests/fixtures/yahoo/quote_summary_fundamentals_tlkm.json");
    const TS_TLKM: &str = include_str!("../../tests/fixtures/yahoo/timeseries_tlkm.json");
    const CHART_TLKM: &str = include_str!("../../tests/fixtures/yahoo/chart_tlkm_1y_1d.json");
    const CHART_JKSE: &str = include_str!("../../tests/fixtures/yahoo/chart_jkse_1y_1d.json");

    #[async_trait]
    impl Provider for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn quotes(&self, _: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
            Err(IdxError::unsupported("fake", "quotes"))
        }
        async fn history(&self, s: &Symbol, _: Period, _: Interval) -> Result<Sourced<History>> {
            let (ok, body) = if s.is_index() {
                (self.index_history, CHART_JKSE)
            } else {
                (self.stock_history, CHART_TLKM)
            };
            if !ok {
                return Err(if s.is_index() {
                    IdxError::provider("fake", "chart endpoint HTTP 500")
                } else {
                    IdxError::NotFound(format!("{s}: no data"))
                });
            }
            let mut out = Sourced::fresh(
                crate::providers::yahoo::parse_chart(body, Period::Y1, Interval::Day1)?,
                "fake",
            );
            out.cached = true;
            Ok(out)
        }
        async fn fundamentals(&self, s: &Symbol) -> Result<Sourced<Fundamentals>> {
            if !self.fundamentals {
                return Err(IdxError::OfflineMiss(format!(
                    "no cached fundamentals for {s}"
                )));
            }
            let mut out = Sourced::fresh(
                crate::providers::yahoo::summary::parse_fundamentals(TLKM, s)?,
                "fake",
            );
            out.cached = true;
            Ok(out)
        }
        async fn financials(
            &self,
            s: &Symbol,
            period: StatementPeriod,
        ) -> Result<Sourced<crate::model::Financials>> {
            if !self.statements {
                return Err(IdxError::unsupported("fake", "financial statements"));
            }
            Ok(Sourced::fresh(
                crate::providers::yahoo::timeseries::parse_timeseries(TS_TLKM, s, period)?,
                "fake",
            ))
        }
    }

    fn fake(fundamentals: bool, statements: bool, stock: bool, index: bool) -> Fake {
        Fake {
            fundamentals,
            statements,
            stock_history: stock,
            index_history: index,
        }
    }

    fn tlkm() -> Symbol {
        Symbol::parse("TLKM").unwrap()
    }

    #[tokio::test]
    async fn growth_runs_without_statements() {
        let g = growth(&fake(true, false, true, true), &tlkm())
            .await
            .unwrap();
        assert!(g.data.annual.is_empty());
        assert_eq!(g.warnings.len(), 1);
        assert!(g.warnings[0].starts_with("annual statements: fake does not provide"));
        assert!(g.data.reading.inputs_available >= 3, "{:?}", g.data.reading);
        assert!(
            g.cached,
            "only the fundamentals were fetched, from the cache"
        );

        let g = growth(&fake(true, true, true, true), &tlkm())
            .await
            .unwrap();
        assert!(g.warnings.is_empty());
        assert_eq!(g.data.annual.len(), 4);
        assert!(!g.cached, "the statements were fetched fresh");

        let g = growth(&fake(false, true, true, true), &tlkm())
            .await
            .unwrap();
        assert!(g.warnings[0].starts_with("fundamentals: "));
        assert_eq!(g.data.annual.len(), 4);
    }

    #[tokio::test]
    async fn growth_fails_when_both_sides_fail() {
        let e = growth(&fake(false, false, true, true), &tlkm())
            .await
            .unwrap_err();
        assert_eq!(e.code(), "offline_miss", "{e}");
    }

    #[tokio::test]
    async fn risk_without_the_index_keeps_the_stock() {
        let r = risk(&fake(true, true, true, false), &tlkm()).await.unwrap();
        assert!(r.warnings[0].starts_with("benchmark ^JKSE unavailable: "));
        let p = r.data.price.as_ref().unwrap();
        assert_eq!(p.beta, None);
        assert!(p.volatility_pct.is_some());
        assert_eq!(
            r.data
                .reading
                .rules
                .iter()
                .map(|x| x.rule.as_str())
                .collect::<Vec<_>>(),
            ["leverage", "liquidity", "losses"]
        );

        let r = risk(&fake(true, true, true, true), &tlkm()).await.unwrap();
        assert!(r.warnings.is_empty());
        assert_eq!(r.data.price.unwrap().observations, 238);
        assert!(r.cached);
    }

    #[tokio::test]
    async fn risk_fails_only_without_fundamentals_and_stock_history() {
        // History alone still gives the price rules.
        let r = risk(&fake(false, true, true, true), &tlkm()).await.unwrap();
        assert!(r.warnings[0].starts_with("fundamentals: "));
        assert_eq!(r.data.reading.inputs_available, 3);
        // Nothing for the stock: "not found" from the history side wins over
        // the cache miss on the fundamentals, even though IHSG answered.
        let e = risk(&fake(false, true, false, true), &tlkm())
            .await
            .unwrap_err();
        assert_eq!(e.code(), "not_found", "{e}");
        let e = risk(&fake(false, true, false, false), &tlkm())
            .await
            .unwrap_err();
        assert_eq!(e.code(), "not_found", "{e}");
    }

    #[tokio::test]
    async fn lens_dispatches() {
        let p = fake(true, true, true, true);
        assert!(matches!(
            lens(&p, &tlkm(), Lens::Valuation).await.unwrap(),
            LensResult::Valuation(_)
        ));
        assert!(matches!(
            lens(&p, &tlkm(), Lens::Growth).await.unwrap(),
            LensResult::Growth(_)
        ));
        assert!(matches!(
            lens(&p, &tlkm(), Lens::Risk).await.unwrap(),
            LensResult::Risk(_)
        ));
    }
}
