//! MSN Money's screener, used for one thing: the whole Indonesian market's
//! price returns (1 week to 1 year and YTD) in one request, for the return
//! keys of `stocks screen`.
//!
//! Endpoint (undocumented, verified 2026-11-06; fixtures and notes in
//! `tests/fixtures/msn/README.md`):
//! `POST https://assets.msn.com/service/Finance/Screener?apikey=<public key>&cm=en-id&...`
//! with the JSON body [`MARKET_LIST_BODY`].
//! - POST only: a GET answers 500. Without the key: 401. A malformed body: 500.
//! - The `st_reg_id` filter with `returnValueType: ["quote"]` returns every
//!   Indonesian stock (887 on 2026-11-05) as `{count, matchIds, quote[]}`,
//!   about 1.3 MB, ordered by market cap. `limit` is honoured (`count` stays
//!   the full total), `pageIndex` is ignored, so `limit` is the only lever.
//!   Asking for `equity` as well makes it 21 MB: not done.
//! - Each quote has `symbol` (plain `BBCA`), `exchangeCode` (`XIDX`) and
//!   `return1Week`, `return1Month`, `return3Month`, `return6Month`,
//!   `returnYTD`, `return1Year` in percent; any of them can be missing
//!   (suspended or newly listed stocks). They match idx's own `stocks
//!   returns` computed from Yahoo closes.
//! - MSN's own Indonesian preset lists (`st_list_topperfs`, `st_list_lowpe`,
//!   `st_list_52wkhi`, ...) are not used: they are slices of a stale
//!   ordering or empty. idx defines its presets over its own screener.

use std::time::Duration;

use serde_json::Value;

use super::parse::{f64_of, text, IDX_MIC};
use super::{Msn, COMMON_QUERY, NAME};
use crate::error::{IdxError, Result};
use crate::model::{MarketReturns, Symbol};
use crate::providers::Sourced;

pub const SCREENER_URL: &str = "https://assets.msn.com/service/Finance/Screener";
/// Every Indonesian stock, quote block only (one POST).
pub const MARKET_LIST_BODY: &str = r#"{"filter":[{"key":"st_reg_id","keyGroup":"st_reg_","isRange":false}],"returnValueType":["quote"],"screenerType":"stock","limit":2000,"pageIndex":0}"#;
/// Return windows move with the price, so the list is kept briefly.
pub const MARKET_LIST_TTL: Duration = Duration::from_secs(15 * 60);
/// Names the request without the api key; the body is fixed.
const MARKET_LIST_KEY: &str =
    "https://assets.msn.com/service/Finance/Screener?region=id&values=quote";
pub const SOURCE_SCREENER: &str = "msn:screener";

/// A parsed whole-market list.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketList {
    /// IDX rows with a usable symbol, in MSN's order.
    pub rows: Vec<MarketReturns>,
    /// Quote rows in the response.
    pub listed: usize,
    /// Stocks matching the filter according to MSN (`count`).
    pub count: Option<usize>,
}

/// Rows of a screener response: IDX listings only, returns as reported.
pub fn parse_market_list(body: &str) -> Result<MarketList> {
    let v: Value = serde_json::from_str(body)?;
    let quotes = v
        .get("quote")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let rows = quotes
        .iter()
        .filter(|q| text(q, "exchangeCode").as_deref() == Some(IDX_MIC))
        .filter_map(|q| {
            Some(MarketReturns {
                symbol: Symbol::parse(&text(q, "symbol")?).ok()?,
                source: SOURCE_SCREENER,
                w1: f64_of(q, "return1Week"),
                m1: f64_of(q, "return1Month"),
                m3: f64_of(q, "return3Month"),
                m6: f64_of(q, "return6Month"),
                ytd: f64_of(q, "returnYTD"),
                y1: f64_of(q, "return1Year"),
            })
        })
        .collect();
    Ok(MarketList {
        rows,
        listed: quotes.len(),
        count: v.get("count").and_then(Value::as_u64).map(|c| c as usize),
    })
}

impl Msn {
    /// The whole Indonesian market's return windows, one cached POST.
    pub(super) async fn market_list(&self) -> Result<Sourced<Vec<MarketReturns>>> {
        let url = format!("{SCREENER_URL}?apikey={}&{COMMON_QUERY}", self.api_key);
        let f = match self
            .fetcher
            .post(
                &url,
                &[],
                MARKET_LIST_BODY,
                MARKET_LIST_TTL,
                MARKET_LIST_KEY,
            )
            .await
        {
            Ok(f) => f,
            Err(IdxError::OfflineMiss(_)) => {
                return Err(IdxError::OfflineMiss(
                    "no cached MSN market list (return windows)".into(),
                ))
            }
            Err(e) => return Err(e),
        };
        match f.status {
            200..=299 => {}
            401 | 403 => return Err(super::key_rejected()),
            s => {
                return Err(IdxError::provider(
                    NAME,
                    format!("screener returned HTTP {s}"),
                ))
            }
        }
        let list = parse_market_list(&f.body)?;
        if list.rows.is_empty() {
            return Err(IdxError::NotFound(
                "MSN returned no Indonesian stocks".into(),
            ));
        }
        let mut warnings = Vec::new();
        if let Some(count) = list.count.filter(|c| *c > list.listed) {
            warnings.push(format!(
                "MSN listed {} of {count} stocks; the rest have no return windows",
                list.listed
            ));
        }
        let mut out = Sourced::from_fetched(list.rows, NAME, &f);
        out.warnings.extend(warnings);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::cache::Cache;
    use crate::http::{CacheMode, Fetcher, FixtureBackend};
    use crate::providers::{Provider, Ttls};

    const LIMIT12: &str =
        include_str!("../../../tests/fixtures/msn/screener_region_id_limit12.json");
    const HI52: &str = include_str!("../../../tests/fixtures/msn/screener_52wkhi_id_complete.json");
    const EMPTY: &str = include_str!("../../../tests/fixtures/msn/screener_lowpe_id_empty.json");

    fn msn(be: FixtureBackend, mode: CacheMode) -> (Msn, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(be);
        let fetcher = Arc::new(Fetcher::new(be.clone(), Cache::new(dir.path()), mode));
        (Msn::new(fetcher, Ttls::default(), None), be, dir)
    }

    #[test]
    fn truncated_list_keeps_mns_values() {
        let l = parse_market_list(LIMIT12).unwrap();
        assert_eq!((l.rows.len(), l.listed, l.count), (12, 12, Some(887)));
        let bbca = &l.rows[0];
        assert_eq!(bbca.symbol.as_str(), "BBCA");
        assert_eq!(bbca.source, "msn:screener");
        assert_eq!(bbca.w1, Some(3.7449));
        assert_eq!(bbca.m3, Some(-20.5426));
        assert_eq!(bbca.ytd, Some(-36.5325));
        assert_eq!(bbca.y1, Some(-41.092));
        let syms: Vec<&str> = l.rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(
            syms,
            [
                "BBCA", "BYAN", "DCII", "BBRI", "BREN", "BMRI", "AMMN", "TLKM", "MORA", "DSSA",
                "ASII", "BRPT"
            ]
        );
    }

    #[test]
    fn complete_list_with_missing_windows() {
        let l = parse_market_list(HI52).unwrap();
        assert_eq!((l.rows.len(), l.listed, l.count), (38, 38, Some(38)));
        let row = |s: &str| l.rows.iter().find(|r| r.symbol.as_str() == s).unwrap();
        let glob = row("GLOB");
        assert_eq!(
            (glob.w1, glob.m1, glob.m3, glob.m6, glob.ytd, glob.y1),
            (None, None, None, None, None, None)
        );
        let ibst = row("IBST");
        assert_eq!((ibst.w1, ibst.ytd), (None, Some(62.9808)));
        assert_eq!(row("MTPS").y1, Some(0.0));
        assert_eq!(row("ALKA").y1, Some(1300.2732));

        let e = parse_market_list(EMPTY).unwrap();
        assert_eq!((e.rows.len(), e.count), (0, Some(0)));
    }

    #[tokio::test]
    async fn one_cached_post_for_the_market() {
        let be = FixtureBackend::new().route(
            "POST https://assets.msn.com/service/Finance/Screener",
            200,
            LIMIT12,
        );
        let (m, be, _d) = msn(be, CacheMode::Normal);
        let got = m.market_returns().await.unwrap();
        assert_eq!(got.provider, "msn");
        assert_eq!(got.data.len(), 12);
        assert_eq!(
            got.warnings,
            ["MSN listed 12 of 887 stocks; the rest have no return windows"]
        );
        {
            let calls = be.calls.lock().unwrap();
            assert!(calls[0].contains("apikey="), "{calls:?}");
        }
        let again = m.market_returns().await.unwrap();
        assert!(again.cached);
        assert_eq!(be.calls_matching("POST "), 1, "the second call is cached");

        // Complete list: no warning.
        let be = FixtureBackend::new().route("Finance/Screener", 200, HI52);
        let (m, _be, _d) = msn(be, CacheMode::Normal);
        assert!(m.market_returns().await.unwrap().warnings.is_empty());
    }

    #[tokio::test]
    async fn errors_map_to_exit_codes() {
        let be = FixtureBackend::new().route("Finance/Screener", 401, "");
        let (m, _be, _d) = msn(be, CacheMode::Normal);
        let err = m.market_returns().await.unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert!(err.to_string().contains("IDX_MSN_API_KEY"), "{err}");

        let be = FixtureBackend::new().route("Finance/Screener", 500, "{}");
        let (m, _be, _d) = msn(be, CacheMode::Normal);
        assert_eq!(m.market_returns().await.unwrap_err().exit_code(), 3);

        let be = FixtureBackend::new().route("Finance/Screener", 200, EMPTY);
        let (m, _be, _d) = msn(be, CacheMode::Normal);
        assert_eq!(m.market_returns().await.unwrap_err().exit_code(), 4);

        let (m, _be, _d) = msn(FixtureBackend::new(), CacheMode::Offline);
        assert_eq!(m.market_returns().await.unwrap_err().exit_code(), 5);
    }
}
