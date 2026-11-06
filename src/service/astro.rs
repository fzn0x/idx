//! `market astro`: the sky for a date (computed here, no network) and, when
//! asked, the backtest over IHSG's daily history from the provider. Shared by
//! the CLI and the MCP server like the rest of `service`.

use chrono::NaiveDate;

use crate::astro;
use crate::error::Result;
use crate::model::{Astro, AstroPeriod, Interval, NatalChoice, Symbol};
use crate::providers::{Provider, Sourced};

/// Provider name for data computed on this machine.
pub const COMPUTED: &str = "computed";
/// Default number of days of upcoming events.
pub const ASTRO_DEFAULT_DAYS: u32 = 30;

#[derive(Debug, Clone)]
pub struct AstroQuery {
    pub date: NaiveDate,
    pub days: u32,
    pub natal: NatalChoice,
    /// Look-back of the returns backtest; `None` skips it (and the network).
    pub backtest: Option<AstroPeriod>,
}

/// The sky, weton, shio and natal transits for `q.date`; with a backtest
/// period, also IHSG's daily returns by factor. Without the backtest the
/// envelope's provider is `computed`; with it, the history's provenance.
pub async fn astro(p: &dyn Provider, q: &AstroQuery) -> Result<Sourced<Astro>> {
    let (mut sky, warnings) = astro::sky(q.date, q.days, q.natal)?;
    let mut s = match q.backtest {
        None => Sourced::fresh(sky, COMPUTED),
        Some(period) => {
            let symbol = Symbol::parse("^JKSE")?;
            let mut history = p
                .history(&symbol, period.history_period(), Interval::Day1)
                .await?;
            // Decided by when the candles were fetched, not by the clock now:
            // a cached copy can hold a mid-session price long after the close.
            if let Some(day) =
                crate::analysis::astro::drop_unfinished(&mut history.data, history.fetched_at)
            {
                history.warnings.push(format!(
                    "the backtest leaves out {day}, a session that had not closed when the history was fetched"
                ));
            }
            sky.backtest = Some(crate::analysis::astro::backtest(&history.data, period)?);
            history.map(|_| sky)
        }
    };
    s.warnings.extend(warnings);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::IdxError;
    use crate::model::{History, Period, Quote};
    use async_trait::async_trait;

    /// Two recorded years of ^JKSE (last bar 2026-11-04), served from cache
    /// as fetched at the given time (`None`: now).
    struct Recorded(Option<&'static str>);

    #[async_trait]
    impl Provider for Recorded {
        fn name(&self) -> &'static str {
            "yahoo"
        }
        async fn quotes(&self, _: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
            unreachable!("astro asks for history only")
        }
        async fn history(&self, s: &Symbol, p: Period, i: Interval) -> Result<Sourced<History>> {
            assert_eq!((s.as_str(), p, i), ("^JKSE", Period::Y2, Interval::Day1));
            let body = include_str!("../../tests/fixtures/yahoo/chart_jkse_2y_1d.json");
            let mut out =
                Sourced::fresh(crate::providers::yahoo::parse_chart(body, p, i)?, "yahoo");
            out.cached = true;
            if let Some(at) = self.0 {
                out.fetched_at = at.parse().unwrap();
            }
            Ok(out)
        }
    }

    struct Offline;

    #[async_trait]
    impl Provider for Offline {
        fn name(&self) -> &'static str {
            "offline"
        }
        async fn quotes(&self, _: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
            unreachable!("astro asks for history only")
        }
        async fn history(&self, _: &Symbol, _: Period, _: Interval) -> Result<Sourced<History>> {
            Err(IdxError::OfflineMiss("nothing cached".into()))
        }
    }

    fn q(backtest: Option<AstroPeriod>) -> AstroQuery {
        AstroQuery {
            date: "2026-11-06".parse().unwrap(),
            days: 7,
            natal: NatalChoice::Launch,
            backtest,
        }
    }

    #[tokio::test]
    async fn sky_alone_is_computed_and_never_calls_the_provider() {
        let s = astro(&Offline, &q(None)).await.unwrap();
        assert_eq!((s.provider, s.cached), (COMPUTED, false));
        assert!(s.data.backtest.is_none());
        assert_eq!(s.data.bodies.len(), 10);
    }

    #[tokio::test]
    async fn backtest_carries_the_history_provenance() {
        let s = astro(&Recorded(None), &q(Some(AstroPeriod::Y2)))
            .await
            .unwrap();
        assert_eq!((s.provider, s.cached), ("yahoo", true));
        assert_eq!(s.data.backtest.as_ref().unwrap().returns, 473);
        assert!(s.warnings.iter().all(|w| !w.contains("leaves out")));
        let e = astro(&Offline, &q(Some(AstroPeriod::Y2)))
            .await
            .unwrap_err();
        assert_eq!(e.exit_code(), 5);
    }

    #[tokio::test]
    async fn a_cached_mid_session_price_is_left_out_whenever_it_is_read() {
        // Fetched at 10:00 WIB on 2026-11-04, the last bar's date, and read
        // now (days later, after the close): its price is still not a close.
        let s = astro(
            &Recorded(Some("2026-11-04T03:00:00Z")),
            &q(Some(AstroPeriod::Y2)),
        )
        .await
        .unwrap();
        let b = s.data.backtest.as_ref().unwrap();
        assert_eq!((b.returns, b.to.to_string()), (472, "2026-11-03".into()));
        assert!(
            s.warnings
                .iter()
                .any(|w| w.contains("leaves out 2026-11-04")),
            "{:?}",
            s.warnings
        );
        // Fetched after 17:00 WIB that day: the close is final and kept.
        let s = astro(
            &Recorded(Some("2026-11-04T11:00:00Z")),
            &q(Some(AstroPeriod::Y2)),
        )
        .await
        .unwrap();
        assert_eq!(s.data.backtest.as_ref().unwrap().returns, 473);
    }
}
