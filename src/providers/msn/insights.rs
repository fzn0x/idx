//! MSN Money insights: about 30 rule-based statements per company, each rated
//! against its peers, its industry, its own history or the price.
//!
//! Endpoint (undocumented, verified 2026-11-06; fixtures and notes in
//! `tests/fixtures/msn/README.md`):
//! `GET https://api.msn.com/msn/v0/pages/finance/insights?ids=<msn id>&apikey=<public key>&cm=en-id&...`
//! - A second host: `assets.msn.com/service/Finance/Insights` does not exist.
//!   The same public web key applies; without it the service answers 401.
//! - The body is an array with one object per id: `instrumentId`, `symbo`
//!   (sic), `displayName`, `timeLastUpdated`, `peers` (MSN ids) and
//!   `insights[]`. Each item carries `category`, `insightName`,
//!   `insightStatement`, `shortInsightStatement`, `benchmarkObjectType` /
//!   `benchmarkObjectId` and `details` with `propertyValue`,
//!   `benchmarkValue`, `criteria` and `evaluationStatus` (`good`, `bad`,
//!   `neutral`, `NA`). The item-level `timeLastUpdated` is always year 1 and
//!   is ignored; so is `differencePercentage`.
//! - Benchmarks: `AggregateData` with id `<instrumentId>peers` is the peer
//!   group, other `AggregateData` ids (`ididbankingservices`) the industry,
//!   `Earning` and `History` the company's own earlier quarters or years,
//!   `Quote` the current price and `Constant` a fixed threshold.
//! - Unknown id: HTTP 404 with a `NoDataFounded` error object.
//! - MSN refreshes the statements about once a day, so they are cached with
//!   the fundamentals TTL.
//!
//! MSN's crowd sentiment (`Finance/SentimentBrowser`) is not used: it answers
//! `[]` for every IDX ticker in every market tried (checked 2026-11-06).

use chrono::{DateTime, SubsecRound, Utc};
use serde_json::Value;

use super::parse::{f64_of, flatten, obj, snake, text, IDX_MIC};
use super::{Msn, COMMON_QUERY, NAME};
use crate::error::{IdxError, Result};
use crate::model::{Insight, InsightBenchmark, InsightStatus, Insights, Symbol};
use crate::providers::Sourced;

pub const INSIGHTS_URL: &str = "https://api.msn.com/msn/v0/pages/finance/insights";

/// One parsed insights object before the peers are resolved.
#[derive(Debug, Clone)]
pub struct ParsedInsights {
    /// `peers` is still empty here.
    pub report: Insights,
    /// MSN ids of the peer group, in MSN's order.
    pub peer_ids: Vec<String>,
    /// Ratings idx does not know (listed as `not_rated`).
    pub warnings: Vec<String>,
}

fn status(raw: Option<&str>) -> Option<InsightStatus> {
    Some(match raw.map(str::to_ascii_lowercase).as_deref() {
        Some("good") => InsightStatus::Good,
        Some("bad") => InsightStatus::Bad,
        Some("neutral") => InsightStatus::Neutral,
        Some("na") | None => InsightStatus::NotRated,
        Some(_) => return None,
    })
}

fn benchmark(item: &Value) -> Option<InsightBenchmark> {
    let id = text(item, "benchmarkObjectId").unwrap_or_default();
    Some(match text(item, "benchmarkObjectType")?.as_str() {
        "AggregateData" if id.ends_with("peers") => InsightBenchmark::Peers,
        "AggregateData" => InsightBenchmark::Industry,
        "Earning" | "History" => InsightBenchmark::OwnHistory,
        "Quote" => InsightBenchmark::Price,
        "Constant" => InsightBenchmark::Threshold,
        _ => return None,
    })
}

/// The insights object for `symbol` in an insights body; `None` when MSN has
/// none (an empty array or the 404 error object).
pub fn parse_insights(body: &str, symbol: &Symbol) -> Result<Option<ParsedInsights>> {
    let v: Value = serde_json::from_str(body)?;
    let objects: Vec<Value> = flatten(v)
        .into_iter()
        .filter(|o| o.get("insights").is_some_and(Value::is_array))
        .collect();
    let mine = |o: &&Value| {
        text(o, "symbo").or_else(|| text(o, "symbol")).as_deref() == Some(symbol.as_str())
    };
    let Some(o) = objects.iter().find(mine).or(objects.first()) else {
        return Ok(None);
    };

    let mut warnings: Vec<String> = Vec::new();
    let mut items = Vec::new();
    for it in o["insights"].as_array().into_iter().flatten() {
        let (Some(category), Some(name)) = (text(it, "category"), text(it, "insightName")) else {
            continue;
        };
        let d = obj(it, "/details");
        let raw = text(d, "evaluationStatus");
        let status = status(raw.as_deref()).unwrap_or_else(|| {
            let w = format!(
                "MSN rated {name:?} {:?}, which idx does not know; listed as not_rated",
                raw.as_deref().unwrap_or_default()
            );
            if !warnings.contains(&w) {
                warnings.push(w);
            }
            InsightStatus::NotRated
        });
        let (mut value, mut benchmark_value) =
            (f64_of(d, "propertyValue"), f64_of(d, "benchmarkValue"));
        // MSN fills both figures with 0 when it lacks the data to rate a
        // statement: that is a placeholder, not a value.
        if status == InsightStatus::NotRated && value == Some(0.0) && benchmark_value == Some(0.0) {
            value = None;
            benchmark_value = None;
        }
        items.push(Insight {
            category,
            name,
            statement: text(it, "insightStatement"),
            short_statement: text(it, "shortInsightStatement"),
            status,
            criteria: text(d, "criteria").map(|c| snake(&c)),
            value,
            benchmark_value,
            compared_with: benchmark(it),
        });
    }
    let updated = text(o, "timeLastUpdated")
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|d| d.with_timezone(&Utc).trunc_subsecs(0));
    let peer_ids = o
        .get("peers")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut report = Insights {
        symbol: symbol.clone(),
        name: text(o, "displayName"),
        updated,
        good: 0,
        bad: 0,
        neutral: 0,
        not_rated: 0,
        peers: Vec::new(),
        insights: items,
    };
    report.recount();
    Ok(Some(ParsedInsights {
        report,
        peer_ids,
        warnings,
    }))
}

/// IDX tickers for MSN peer ids from a `Quotes` body, in the order of
/// `peer_ids`. Listings on other exchanges are left out.
pub fn parse_peers(body: &str, peer_ids: &[String]) -> Result<Vec<Symbol>> {
    let v: Value = serde_json::from_str(body)?;
    let mut found: Vec<(usize, Symbol)> = flatten(v)
        .iter()
        .filter(|q| text(q, "exchangeCode").as_deref() == Some(IDX_MIC))
        .filter_map(|q| {
            let symbol = Symbol::parse(&text(q, "symbol")?).ok()?;
            let pos = text(q, "instrumentId")
                .and_then(|id| peer_ids.iter().position(|p| *p == id))
                .unwrap_or(usize::MAX);
            Some((pos, symbol))
        })
        .collect();
    found.sort_by_key(|(pos, _)| *pos);
    let mut out: Vec<Symbol> = Vec::new();
    for (_, s) in found {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    Ok(out)
}

impl Msn {
    /// Insights for one ticker, peers resolved to IDX tickers. A failed peer
    /// lookup leaves `peers` empty with a warning instead of failing.
    pub(super) async fn insights_report(&self, symbol: &Symbol) -> Result<Sourced<Insights>> {
        let id = self.lookup(symbol).await?;
        let key = format!("{INSIGHTS_URL}?ids={id}");
        let url = format!("{key}&apikey={}&{COMMON_QUERY}", self.api_key);
        let f = match self
            .fetcher
            .get(&url, &[], self.ttls.fundamentals, Some(&key))
            .await
        {
            Ok(f) => f,
            Err(IdxError::OfflineMiss(_)) => return Err(Self::offline_miss(symbol, "insights")),
            Err(e) => return Err(e),
        };
        let not_found = || IdxError::NotFound(format!("MSN has no insights for {symbol}"));
        match f.status {
            200..=299 => {}
            404 => return Err(not_found()),
            401 | 403 => return Err(super::key_rejected()),
            s => {
                return Err(IdxError::provider(
                    NAME,
                    format!("insights returned HTTP {s}"),
                ))
            }
        }
        let parsed = parse_insights(&f.body, symbol)?.ok_or_else(not_found)?;
        let mut out = Sourced {
            data: parsed.report,
            provider: NAME,
            cached: f.cached,
            stale: f.stale,
            fetched_at: f.fetched_at,
            warnings: parsed.warnings,
        };
        if parsed.peer_ids.is_empty() {
            return Ok(out);
        }
        let peers = self
            .fetch_service("Quotes", &parsed.peer_ids, self.ttls.fundamentals)
            .await
            .and_then(|p| {
                if !p.is_success() {
                    return Err(IdxError::provider(
                        NAME,
                        format!("Quotes returned HTTP {}", p.status),
                    ));
                }
                let symbols = parse_peers(&p.body, &parsed.peer_ids)?;
                Ok(Sourced {
                    data: symbols,
                    provider: NAME,
                    cached: p.cached,
                    stale: p.stale,
                    fetched_at: p.fetched_at,
                    warnings: vec![],
                })
            });
        match peers {
            Ok(p) => {
                out.absorb(&p);
                out.data.peers = p.data;
            }
            Err(e) => out
                .warnings
                .push(format!("could not resolve MSN's peer list: {e}")),
        }
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

    const BBCA: &str = include_str!("../../../tests/fixtures/msn/insights_bbca.json");
    const GOTO: &str = include_str!("../../../tests/fixtures/msn/insights_goto_neutral.json");
    const BOGUS: &str = include_str!("../../../tests/fixtures/msn/insights_bogus_404.json");
    const PEERS: &str = include_str!("../../../tests/fixtures/msn/quotes_bbca_insight_peers.json");
    const AS_BBCA: &str = include_str!("../../../tests/fixtures/msn/autosuggest_bbca.json");

    fn sym(s: &str) -> Symbol {
        Symbol::parse(s).unwrap()
    }

    fn msn(be: FixtureBackend, mode: CacheMode) -> (Msn, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(be);
        let fetcher = Arc::new(Fetcher::new(be.clone(), Cache::new(dir.path()), mode));
        (Msn::new(fetcher, Ttls::default(), None), be, dir)
    }

    #[test]
    fn bbca_items_counts_and_benchmarks() {
        let p = parse_insights(BBCA, &sym("BBCA")).unwrap().unwrap();
        let r = &p.report;
        assert_eq!(r.name.as_deref(), Some("Bank Central Asia Tbk PT"));
        assert_eq!(r.insights.len(), 30);
        assert_eq!((r.good, r.bad, r.neutral, r.not_rated), (17, 4, 0, 9));
        assert_eq!(r.updated.unwrap().to_rfc3339(), "2026-11-05T17:51:31+00:00");
        assert_eq!(p.peer_ids, ["bkfmar", "bkfgqh", "cj53lh", "bkgttc"]);
        assert!(p.warnings.is_empty());
        assert!(r.peers.is_empty(), "peers are resolved by the provider");

        let pb = &r.insights[2];
        assert_eq!(pb.category, "Valuation");
        assert_eq!(pb.name, "P/B ratio");
        assert_eq!(pb.status, InsightStatus::Bad);
        assert_eq!(pb.criteria.as_deref(), Some("lower_is_better"));
        assert_eq!(pb.value, Some(2.09778355810005));
        assert_eq!(pb.benchmark_value, Some(0.564114114036551));
        assert_eq!(pb.compared_with, Some(InsightBenchmark::Peers));
        assert!(pb.statement.as_deref().unwrap().contains("P/B multiple"));
        assert_eq!(
            r.insights[3].compared_with,
            Some(InsightBenchmark::Industry)
        );

        let by_name = |n: &str| r.insights.iter().find(|i| i.name == n).unwrap();
        assert_eq!(
            by_name("Analyst price target").compared_with,
            Some(InsightBenchmark::Price)
        );
        assert_eq!(
            by_name("Analyst rating").compared_with,
            Some(InsightBenchmark::Threshold)
        );
        assert_eq!(
            by_name("Earnings (last Q)").compared_with,
            Some(InsightBenchmark::OwnHistory)
        );
        assert_eq!(
            by_name("Change in Debt to equity over last 5 years (Quarterly data)").compared_with,
            Some(InsightBenchmark::OwnHistory)
        );
        // Not rated: real figures stay, MSN's 0/0 placeholder becomes null.
        let pe = &r.insights[0];
        assert_eq!(pe.status, InsightStatus::NotRated);
        assert_eq!(pe.value, Some(10.2114621902489));
        let cr = by_name("Current ratio (Last Quarter) vs peers");
        assert_eq!(cr.status, InsightStatus::NotRated);
        assert_eq!((cr.value, cr.benchmark_value), (None, None));
        assert_eq!(
            r.insights
                .iter()
                .map(|i| i.category.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            5
        );
    }

    #[test]
    fn goto_has_a_neutral_statement() {
        let r = parse_insights(GOTO, &sym("GOTO")).unwrap().unwrap().report;
        assert_eq!((r.good, r.bad, r.neutral, r.not_rated), (13, 2, 1, 14));
        let n = r
            .insights
            .iter()
            .find(|i| i.status == InsightStatus::Neutral)
            .unwrap();
        assert_eq!(n.compared_with, Some(InsightBenchmark::Industry));
        assert_eq!(n.value, n.benchmark_value);
    }

    #[test]
    fn not_found_body_and_unknown_status() {
        assert!(parse_insights(BOGUS, &sym("ZZZZ")).unwrap().is_none());
        assert!(parse_insights("[]", &sym("ZZZZ")).unwrap().is_none());
        let body = BBCA.replacen(
            "\"evaluationStatus\":\"bad\"",
            "\"evaluationStatus\":\"meh\"",
            1,
        );
        let p = parse_insights(&body, &sym("BBCA")).unwrap().unwrap();
        assert_eq!((p.report.bad, p.report.not_rated), (3, 10));
        assert_eq!(p.warnings.len(), 1);
        assert!(p.warnings[0].contains("\"meh\""), "{:?}", p.warnings);
    }

    #[test]
    fn peers_keep_msn_order() {
        let ids: Vec<String> = ["bkfmar", "bkfgqh", "cj53lh", "bkgttc"]
            .map(String::from)
            .to_vec();
        let got = parse_peers(PEERS, &ids).unwrap();
        let names: Vec<&str> = got.iter().map(Symbol::as_str).collect();
        assert_eq!(names, ["BBKP", "BABP", "SUPA", "INPC"]);
        let reversed: Vec<String> = ids.iter().rev().cloned().collect();
        let got = parse_peers(PEERS, &reversed).unwrap();
        assert_eq!(got[0].as_str(), "INPC");
    }

    fn routes() -> FixtureBackend {
        FixtureBackend::new()
            .route("csautosuggest/api/v1/Query?query=BBCA", 200, AS_BBCA)
            .route("finance/insights?ids=bn91jc", 200, BBCA)
    }

    #[tokio::test]
    async fn provider_resolves_peers_and_caches() {
        let be = routes().route("Finance/Quotes?ids=bkfmar,bkfgqh,cj53lh,bkgttc", 200, PEERS);
        let (m, be, _d) = msn(be, CacheMode::Normal);
        let got = m.insights(&sym("BBCA")).await.unwrap();
        assert_eq!(got.provider, "msn");
        let peers: Vec<&str> = got.data.peers.iter().map(Symbol::as_str).collect();
        assert_eq!(peers, ["BBKP", "BABP", "SUPA", "INPC"]);
        assert!(got.warnings.is_empty(), "{:?}", got.warnings);
        {
            let calls = be.calls.lock().unwrap();
            let url = calls
                .iter()
                .find(|u| u.contains("finance/insights"))
                .unwrap();
            assert!(url.starts_with(INSIGHTS_URL), "{url}");
            assert!(url.contains("apikey="), "{url}");
        }
        let n = be.call_count();
        let again = m.insights(&sym("BBCA")).await.unwrap();
        assert!(again.cached);
        assert_eq!(be.call_count(), n, "second call is served from the cache");
    }

    #[tokio::test]
    async fn failed_peer_lookup_is_a_warning() {
        let be = routes().route("Finance/Quotes", 500, "oops");
        let (m, _be, _d) = msn(be, CacheMode::Normal);
        let got = m.insights(&sym("BBCA")).await.unwrap();
        assert!(got.data.peers.is_empty());
        assert_eq!(got.data.insights.len(), 30);
        assert_eq!(got.warnings.len(), 1);
        assert!(
            got.warnings[0].starts_with("could not resolve MSN's peer list"),
            "{:?}",
            got.warnings
        );
    }

    #[tokio::test]
    async fn not_found_rejected_key_and_offline() {
        let be = FixtureBackend::new()
            .route("csautosuggest", 200, AS_BBCA)
            .route("finance/insights", 404, BOGUS);
        let (m, _be, _d) = msn(be, CacheMode::Normal);
        let err = m.insights(&sym("BBCA")).await.unwrap_err();
        assert_eq!(err.exit_code(), 4, "{err}");

        let be = FixtureBackend::new()
            .route("csautosuggest", 200, AS_BBCA)
            .route("finance/insights", 401, "");
        let (m, _be, _d) = msn(be, CacheMode::Normal);
        let err = m.insights(&sym("BBCA")).await.unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert!(err.to_string().contains("IDX_MSN_API_KEY"), "{err}");

        let (m, _be, _d) = msn(FixtureBackend::new(), CacheMode::Offline);
        let err = m.insights(&sym("BBCA")).await.unwrap_err();
        assert_eq!(err.exit_code(), 5, "{err}");
    }
}
