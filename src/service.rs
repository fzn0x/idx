//! Operations shared by the CLI and the MCP server: anything that combines a
//! provider call with local analysis lives here so both front ends behave
//! identically.

use std::sync::Arc;

use chrono::{Datelike, NaiveDate, Weekday};
use futures::future::join_all;
use futures::stream::{self, StreamExt};

use crate::analysis;
use crate::analysis::bandar::{self as bd, Preset};
use crate::analysis::screen::{self as scr, Filter, Key};
use crate::brokers;
use crate::error::{IdxError, Result};
use crate::http::Fetcher;
use crate::model::{
    BandarPeriod, BandarReport, BandarScreen, BenchmarkReturns, Brief, BrokerActivity, BrokerBoard,
    BrokerInvestor, BrokerStockRow, BrokerSummary, DividendHistory, DividendSummary, Fundamentals,
    History, Interval, MarketOverview, MarketRow, NewsItem, NewsReport, NewsSummary,
    OwnershipSummary, Performance, Period, Quote, ScreenReport, ScreenRow, SentimentReport, Symbol,
    Technical,
};
use crate::providers::ipot;
use crate::providers::news as newsfeed;
use crate::providers::{Provider, Sourced};
use crate::universe::Issuer;

mod exchange;
pub use exchange::*;

/// Ratios plus the rule-based verdict.
pub async fn fundamentals(p: &dyn Provider, symbol: &Symbol) -> Result<Sourced<Fundamentals>> {
    let mut result = p.fundamentals(symbol).await?;
    result.data.verdict = Some(analysis::fundamental::verdict(&result.data));
    Ok(result)
}

/// Daily candles over `period`, run through the indicator set.
pub async fn technical(
    p: &dyn Provider,
    symbol: &Symbol,
    period: Period,
) -> Result<Sourced<Technical>> {
    let hist = p.history(symbol, period, Interval::Day1).await?;
    let Some(report) = analysis::technical::analyse(&hist.data) else {
        return Err(IdxError::NotFound(format!("no daily candles for {symbol}")));
    };
    Ok(Sourced {
        data: report,
        provider: hist.provider,
        cached: hist.cached,
        stale: hist.stale,
        fetched_at: hist.fetched_at,
        warnings: hist.warnings,
    })
}

/// Fundamentals for several symbols, fetched concurrently. Symbols that fail
/// become warnings; the call fails only when none succeed.
pub async fn compare(p: &dyn Provider, symbols: &[Symbol]) -> Result<Sourced<Vec<Fundamentals>>> {
    if symbols.len() < 2 {
        return Err(IdxError::Usage(
            "compare needs at least two different symbols".into(),
        ));
    }
    let results = join_all(symbols.iter().map(|s| fundamentals(p, s))).await;

    let mut out: Option<Sourced<Vec<Fundamentals>>> = None;
    let mut pending_warnings: Vec<String> = Vec::new();
    let mut first_err: Option<IdxError> = None;
    for (sym, r) in symbols.iter().zip(results) {
        match r {
            Ok(s) => match &mut out {
                None => out = Some(s.map(|d| vec![d])),
                Some(acc) => {
                    acc.absorb(&s);
                    acc.data.push(s.data);
                }
            },
            Err(e) => {
                let msg = match &e {
                    IdxError::NotFound(_) => format!("no data for {sym}"),
                    other => format!("{sym}: {other}"),
                };
                pending_warnings.push(msg);
                first_err.get_or_insert(e);
            }
        }
    }
    let Some(mut result) = out else {
        return Err(first_err.unwrap_or_else(|| IdxError::NotFound("no data".into())));
    };
    result.warnings.extend(pending_warnings);
    Ok(result)
}

/// Screener settings.
#[derive(Debug, Clone)]
pub struct ScreenOptions {
    pub filters: Vec<Filter>,
    pub sort: Key,
    pub descending: bool,
    /// 0 = all rows.
    pub limit: usize,
    /// Upper bound on per-symbol fundamentals calls (largest market caps first).
    pub max_fundamentals: usize,
    /// Fetch fundamentals even when no filter needs them.
    pub with_fundamentals: bool,
}

/// Quote-level screen over the universe, then a capped fundamentals pass.
pub async fn screen(
    p: &dyn Provider,
    universe: &[Issuer],
    opts: &ScreenOptions,
) -> Result<Sourced<ScreenReport>> {
    if universe.is_empty() {
        return Err(IdxError::Usage("the ticker universe is empty".into()));
    }
    let names: std::collections::HashMap<&Symbol, Option<&str>> = universe
        .iter()
        .map(|i| (&i.symbol, i.name.as_deref()))
        .collect();
    let symbols: Vec<Symbol> = universe.iter().map(|i| i.symbol.clone()).collect();

    // Quotes in batches; one failing batch fails the screen (it is the base data).
    let batches: Vec<&[Symbol]> = symbols.chunks(150).collect();
    let results = join_all(batches.iter().map(|b| p.quotes(b))).await;
    let mut out = Sourced::fresh(Vec::<ScreenRow>::new(), p.name());
    out.cached = true;
    let mut quoted = 0usize;
    for r in results {
        match r {
            Ok(s) => {
                out.absorb(&s);
                out.provider = s.provider;
                quoted += s.data.len();
                for q in &s.data {
                    let hint = names.get(&q.symbol).copied().flatten();
                    out.data.push(scr::row_from_quote(q, hint));
                }
            }
            Err(IdxError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    out.warnings.retain(|w| !w.starts_with("no data for"));
    if quoted < symbols.len() {
        out.warnings.push(format!(
            "{} of {} tickers returned no quote and were skipped",
            symbols.len() - quoted,
            symbols.len()
        ));
    }

    let (quote_filters, fund_filters): (Vec<&Filter>, Vec<&Filter>) = opts
        .filters
        .iter()
        .partition(|f| !f.key.needs_fundamentals());
    let quote_filters: Vec<Filter> = quote_filters.into_iter().cloned().collect();
    let fund_filters: Vec<Filter> = fund_filters.into_iter().cloned().collect();
    let mut rows: Vec<ScreenRow> = out
        .data
        .iter()
        .filter(|r| scr::matches(r, &quote_filters))
        .cloned()
        .collect();
    let matched_quote_level = rows.len();

    let need_fund =
        opts.with_fundamentals || !fund_filters.is_empty() || opts.sort.needs_fundamentals();
    let mut checked = 0usize;
    let mut capped = false;
    if need_fund && !rows.is_empty() {
        scr::sort_rows(&mut rows, Key::Mcap, true);
        let cap = opts.max_fundamentals.max(1);
        if rows.len() > cap {
            capped = true;
            rows.truncate(cap);
        }
        let wanted: Vec<Symbol> = rows.iter().map(|r| r.symbol.clone()).collect();
        let fetched: Vec<(Symbol, Result<Sourced<Fundamentals>>)> = stream::iter(wanted)
            .map(|s| async move {
                let r = fundamentals(p, &s).await;
                (s, r)
            })
            .buffer_unordered(8)
            .collect()
            .await;
        for (sym, r) in fetched {
            match r {
                Ok(f) => {
                    checked += 1;
                    out.cached &= f.cached;
                    out.stale |= f.stale;
                    if let Some(row) = rows.iter_mut().find(|r| r.symbol == sym) {
                        scr::apply_fundamentals(row, &f.data);
                    }
                }
                Err(e) => tracing::debug!(symbol = %sym, error = %e, "fundamentals unavailable"),
            }
        }
        rows.retain(|r| scr::matches(r, &fund_filters));
        if capped {
            out.warnings.push(format!(
                "fundamentals were fetched for the {cap} largest companies that passed the quote filters; raise --max-fundamentals to widen"
            ));
        }
    }

    scr::sort_rows(&mut rows, opts.sort, opts.descending);
    let total = rows.len();
    if opts.limit > 0 {
        rows.truncate(opts.limit);
    }
    Ok(out.map(|_| ScreenReport {
        filters: opts.filters.iter().map(|f| f.raw.clone()).collect(),
        sort: opts.sort.name().to_string(),
        descending: opts.descending,
        universe: symbols.len(),
        quoted,
        matched_quote_level,
        fundamentals_checked: checked,
        fundamentals_capped: capped,
        total_matches: total,
        rows,
    }))
}

/// Quotes for a whole universe in batches. Batches that fail with NotFound
/// are skipped; any other failure is fatal because the quotes are the base
/// data. Returns the quotes and the number of symbols that answered.
pub async fn quotes_all(p: &dyn Provider, symbols: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
    let batches: Vec<&[Symbol]> = symbols.chunks(150).collect();
    let results = join_all(batches.iter().map(|b| p.quotes(b))).await;
    let mut out = Sourced::fresh(Vec::<Quote>::new(), p.name());
    out.cached = true;
    for r in results {
        match r {
            Ok(s) => {
                out.absorb(&s);
                out.provider = s.provider;
                out.data.extend(s.data);
            }
            Err(IdxError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    out.warnings.retain(|w| !w.starts_with("no data for"));
    if out.data.len() < symbols.len() {
        out.warnings.push(format!(
            "{} of {} tickers returned no quote and were skipped",
            symbols.len() - out.data.len(),
            symbols.len()
        ));
    }
    Ok(out)
}

/// The board: index quote, breadth and ranked lists from one pass over the
/// universe. `liquid_floor` is the market-cap floor for gainers/losers.
pub async fn market_overview(
    p: &dyn Provider,
    universe: &[Issuer],
    index: &Symbol,
    top: usize,
    liquid_floor: f64,
) -> Result<Sourced<MarketOverview>> {
    if universe.is_empty() {
        return Err(IdxError::Usage("the ticker universe is empty".into()));
    }
    let names: std::collections::HashMap<&Symbol, Option<&str>> = universe
        .iter()
        .map(|i| (&i.symbol, i.name.as_deref()))
        .collect();
    let symbols: Vec<Symbol> = universe.iter().map(|i| i.symbol.clone()).collect();
    let (quotes, index_q) = futures::join!(
        quotes_all(p, &symbols),
        p.quotes(std::slice::from_ref(index))
    );
    let mut out = quotes?;
    let mut index_row = None;
    let mut market_state = None;
    match index_q {
        Ok(s) => {
            out.absorb(&s);
            if let Some(q) = s.data.first() {
                market_state = q.market_state.clone();
                index_row = Some(MarketRow::from_quote(q, None));
            }
        }
        Err(e) => out.warnings.push(format!("index {index} unavailable: {e}")),
    }

    let rows: Vec<MarketRow> = out
        .data
        .iter()
        .filter(|q| q.price.is_some() && q.change_pct.is_some())
        .map(|q| MarketRow::from_quote(q, names.get(&q.symbol).copied().flatten()))
        .collect();
    let as_of = out.data.iter().filter_map(|q| q.market_time).max();
    let chg = |r: &MarketRow| r.change_pct.unwrap_or(0.0);
    let liquid: Vec<&MarketRow> = rows
        .iter()
        .filter(|r| r.market_cap.unwrap_or(0.0) >= liquid_floor && r.volume.unwrap_or(0) > 0)
        .collect();
    let mut gainers: Vec<&MarketRow> = liquid.iter().copied().filter(|r| chg(r) > 0.0).collect();
    gainers.sort_by(|a, b| {
        chg(b)
            .partial_cmp(&chg(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut losers: Vec<&MarketRow> = liquid.iter().copied().filter(|r| chg(r) < 0.0).collect();
    losers.sort_by(|a, b| {
        chg(a)
            .partial_cmp(&chg(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut by_value: Vec<&MarketRow> = rows.iter().filter(|r| r.value.is_some()).collect();
    by_value.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut by_cap: Vec<&MarketRow> = rows.iter().filter(|r| r.market_cap.is_some()).collect();
    by_cap.sort_by(|a, b| {
        b.market_cap
            .partial_cmp(&a.market_cap)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let take =
        |v: Vec<&MarketRow>| -> Vec<MarketRow> { v.into_iter().take(top).cloned().collect() };
    let data = MarketOverview {
        as_of,
        market_state,
        index: index_row,
        universe: symbols.len(),
        quoted: rows.len(),
        advancing: rows.iter().filter(|r| chg(r) > 0.0).count(),
        declining: rows.iter().filter(|r| chg(r) < 0.0).count(),
        unchanged: rows.iter().filter(|r| chg(r) == 0.0).count(),
        turnover: rows.iter().filter_map(|r| r.value).sum(),
        liquid_min_market_cap: liquid_floor,
        gainers: take(gainers),
        losers: take(losers),
        most_active: take(by_value),
        heavyweights: take(by_cap),
    };
    Ok(out.map(|_| data))
}

/// Price performance from five years of daily candles for each symbol, with
/// excess returns over `benchmark` (an index such as `^JKSE`) when given.
/// Symbols that fail become warnings; the call fails only when none succeed.
pub async fn returns(
    p: &dyn Provider,
    symbols: &[Symbol],
    benchmark: Option<&Symbol>,
) -> Result<Sourced<Vec<Performance>>> {
    let (results, bench) = futures::join!(
        join_all(
            symbols
                .iter()
                .map(|s| p.history(s, Period::Y5, Interval::Day1))
        ),
        async {
            match benchmark {
                Some(b) => Some(p.history(b, Period::Y5, Interval::Day1).await),
                None => None,
            }
        }
    );
    let mut warnings: Vec<String> = Vec::new();
    let mut bench_perf: Option<(Sourced<History>, Performance)> = None;
    if let (Some(b), Some(r)) = (benchmark, bench) {
        match r {
            Ok(h) => match analysis::returns::analyse(&h.data) {
                Some(perf) => bench_perf = Some((h, perf)),
                None => warnings.push(format!("benchmark {b}: no candles")),
            },
            Err(e) => warnings.push(format!("benchmark {b} unavailable: {e}")),
        }
    }

    let mut out: Option<Sourced<Vec<Performance>>> = None;
    let mut first_err: Option<IdxError> = None;
    for (sym, r) in symbols.iter().zip(results) {
        match r {
            Ok(h) => {
                let Some(mut perf) = analysis::returns::analyse(&h.data) else {
                    warnings.push(format!("{sym}: no candles"));
                    continue;
                };
                if let Some((_, bp)) = &bench_perf {
                    perf.benchmark = Some(BenchmarkReturns {
                        symbol: bp.symbol.clone(),
                        name: bp.name.clone(),
                        returns: bp.returns.clone(),
                        excess: perf.returns.minus(&bp.returns),
                    });
                }
                match &mut out {
                    None => out = Some(h.map(|_| vec![perf])),
                    Some(acc) => {
                        acc.absorb(&h);
                        acc.data.push(perf);
                    }
                }
            }
            Err(e) => {
                warnings.push(match &e {
                    IdxError::NotFound(_) => format!("no data for {sym}"),
                    other => format!("{sym}: {other}"),
                });
                first_err.get_or_insert(e);
            }
        }
    }
    let Some(mut result) = out else {
        return Err(first_err.unwrap_or_else(|| IdxError::NotFound("no data".into())));
    };
    if let Some((bh, _)) = &bench_perf {
        result.absorb(bh);
    }
    result.warnings.extend(warnings);
    Ok(result)
}

/// Collects brief sections: merges provenance, records failures.
struct Sections {
    out: Sourced<()>,
    failed: Vec<String>,
    not_found: usize,
}

impl Sections {
    fn take<T>(&mut self, name: &str, r: Result<Sourced<T>>) -> Option<T> {
        match r {
            Ok(s) => {
                self.out.absorb(&s);
                Some(s.data)
            }
            Err(e) => {
                if matches!(e, IdxError::NotFound(_)) {
                    self.not_found += 1;
                }
                self.out.warnings.push(format!("{name}: {e}"));
                self.failed.push(name.to_string());
                None
            }
        }
    }
}

/// One call, every section: quote, fundamentals with verdict, technicals,
/// returns against IHSG, dividend facts, the local ownership register and
/// company headlines with sentiment. Sections fetch concurrently; a failed
/// section becomes `null` plus a warning. Fails only when the ticker is
/// unknown to the quote, fundamentals and history sources alike.
pub async fn brief(
    p: &dyn Provider,
    fetcher: &Arc<Fetcher>,
    universe: &[Issuer],
    ownership_db: &std::path::Path,
    symbol: &Symbol,
) -> Result<Sourced<Brief>> {
    let bench = Symbol::parse("^JKSE")?;
    let (quote, fund, tech, rets, divs, headlines) = futures::join!(
        p.quotes(std::slice::from_ref(symbol)),
        fundamentals(p, symbol),
        technical(p, symbol, Period::Y1),
        returns(p, std::slice::from_ref(symbol), Some(&bench)),
        dividends(p, symbol),
        news(fetcher, universe, Some(symbol), &[], 0, true),
    );

    let mut c = Sections {
        out: Sourced::fresh((), p.name()),
        failed: Vec::new(),
        not_found: 0,
    };
    c.out.cached = true;
    let quote = c.take("quote", quote).and_then(|v| v.into_iter().next());
    let fundamentals = c.take("fundamentals", fund);
    let technical = c.take("technical", tech);
    let returns = c.take("returns", rets).and_then(|v| v.into_iter().next());
    let dividends = c.take("dividends", divs).map(|h| DividendSummary {
        ttm_total: h.ttm_total,
        ttm_payments: h.ttm_payments,
        ttm_yield_pct: h.ttm_yield_pct,
        consecutive_years: h.consecutive_years,
        last: h.last,
        payments_on_record: h.dividends.len(),
    });
    let news = c.take("news", headlines).map(|n| {
        let rep = analysis::sentiment::aggregate(symbol, n.matched_name.clone(), &n.items, 0);
        NewsSummary {
            company_matches: n.company_matches,
            sentiment: rep.label,
            score: rep.score,
            headlines: n.items.into_iter().take(5).collect(),
        }
    });
    let Sections {
        mut out,
        mut failed,
        not_found,
    } = c;
    if not_found >= 3 && quote.is_none() && fundamentals.is_none() && technical.is_none() {
        return Err(IdxError::NotFound(format!(
            "{symbol}: unknown to the quote, fundamentals and history sources"
        )));
    }

    let ownership = match crate::ownership::OwnershipDb::open(ownership_db) {
        Ok(db) if ownership_db.exists() => match db.holders(symbol.as_str(), None) {
            Ok(h) => Some(OwnershipSummary {
                as_of: h.as_of,
                listed_pct: h.listed_pct,
                // `+ 0.0` turns the -0.0 an empty f64 sum yields into 0.0.
                foreign_pct: h
                    .holders
                    .iter()
                    .filter(|x| x.local_foreign.as_deref() == Some("A"))
                    .map(|x| x.pct)
                    .sum::<f64>()
                    + 0.0,
                holders_listed: h.holders.len(),
                top_holders: h.holders.into_iter().take(5).collect(),
            }),
            Err(e) => {
                out.warnings.push(format!("ownership: {e}"));
                failed.push("ownership".into());
                None
            }
        },
        _ => {
            out.warnings
                .push("ownership: no register imported (idx ownership import --latest)".into());
            failed.push("ownership".into());
            None
        }
    };

    let name = quote
        .as_ref()
        .and_then(|q| q.name.clone())
        .or_else(|| fundamentals.as_ref().and_then(|f| f.name.clone()))
        .or_else(|| technical.as_ref().and_then(|t| t.name.clone()))
        .or_else(|| {
            universe
                .iter()
                .find(|i| &i.symbol == symbol)
                .and_then(|i| i.name.clone())
        });

    let mut key_points: Vec<String> = Vec::new();
    if let Some(q) = &quote {
        if let (Some(px), Some(chg)) = (q.price, q.change_pct) {
            key_points.push(format!(
                "Price {} ({}{:.2}% today){}",
                crate::output::format::price(Some(px)),
                if chg >= 0.0 { "+" } else { "" },
                chg,
                q.market_cap
                    .map(|m| format!(", market cap {}", crate::output::format::compact(Some(m))))
                    .unwrap_or_default()
            ));
        }
    }
    if let Some(f) = &fundamentals {
        if let Some(v) = &f.verdict {
            key_points.push(format!(
                "Valuation verdict: {} (score {:+}, {} inputs){}",
                v.label.as_str(),
                v.score,
                v.inputs_available,
                match (f.valuation.pe_trailing, f.valuation.price_to_book) {
                    (Some(pe), Some(pb)) => format!("; P/E {pe:.1}, P/B {pb:.2}"),
                    (Some(pe), None) => format!("; P/E {pe:.1}"),
                    _ => String::new(),
                }
            ));
        }
    }
    if let Some(t) = &technical {
        key_points.push(format!(
            "Technical signal: {} (score {:+}), trend {}{}",
            t.signal.label.as_str(),
            t.signal.score,
            t.trend.as_str(),
            t.rsi14.map(|r| format!(", RSI {r:.0}")).unwrap_or_default()
        ));
    }
    if let Some(r) = &returns {
        if let Some(y1) = r.returns.y1 {
            key_points.push(format!(
                "1y return {y1:+.1}%{}; YTD {}",
                r.benchmark
                    .as_ref()
                    .and_then(|b| b
                        .excess
                        .y1
                        .map(|x| format!(" ({x:+.1} pp vs {})", b.symbol)))
                    .unwrap_or_default(),
                r.returns
                    .ytd
                    .map(|v| format!("{v:+.1}%"))
                    .unwrap_or_else(|| "n/a".into())
            ));
        }
    }
    if let Some(d) = &dividends {
        match d.ttm_yield_pct {
            Some(y) => key_points.push(format!(
                "Dividends: TTM yield {y:.2}% over {} payment{}, {} consecutive years",
                d.ttm_payments,
                if d.ttm_payments == 1 { "" } else { "s" },
                d.consecutive_years
            )),
            None if d.payments_on_record == 0 => {
                key_points.push("Dividends: none on record".into())
            }
            None => key_points.push("Dividends: none in the last twelve months".into()),
        }
    }
    if let Some(o) = &ownership {
        key_points.push(format!(
            "Ownership ({}): {} holders above 1% hold {:.1}%, foreign {:.1}%{}",
            o.as_of,
            o.holders_listed,
            o.listed_pct,
            o.foreign_pct,
            o.top_holders
                .first()
                .map(|h| format!("; largest {} {:.1}%", h.holder, h.pct))
                .unwrap_or_default()
        ));
    }
    if let Some(n) = &news {
        key_points.push(match n.company_matches {
            0 => "News: no current headline mentions the company".into(),
            k => format!(
                "News: {k} current headline{} mention the company, lexicon sentiment {} ({:+.2})",
                if k == 1 { "" } else { "s" },
                n.sentiment.as_str(),
                n.score
            ),
        });
    }

    let brief = Brief {
        symbol: symbol.clone(),
        name,
        generated_at: chrono::Utc::now(),
        key_points,
        quote,
        fundamentals,
        technical,
        returns,
        dividends,
        ownership,
        news,
        sections_failed: failed,
    };
    Ok(out.map(|_| brief))
}

/// Dividend and split history with totals, trailing yield and streak.
pub async fn dividends(p: &dyn Provider, symbol: &Symbol) -> Result<Sourced<DividendHistory>> {
    let mut h = p.dividends(symbol).await?;
    let today = chrono::Utc::now()
        .with_timezone(&crate::model::wib())
        .date_naive();
    analysis::dividends::enrich(&mut h.data, today);
    if h.data.dividends.is_empty() {
        h.warnings
            .push(format!("{symbol}: the source lists no dividend events"));
    }
    Ok(h)
}

/// Company name without legal-form tokens, for headline matching.
pub fn match_name(raw: &str) -> String {
    raw.split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|t| {
            !t.is_empty()
                && !matches!(
                    t.to_ascii_uppercase().as_str(),
                    "PT" | "TBK" | "PERSERO" | "LTD" | "INC"
                )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn item_matches(item: &NewsItem, ticker: &str, name: Option<&str>) -> Option<&'static str> {
    let text = format!("{} {}", item.title, item.summary);
    let upper = text.to_uppercase();
    let has_ticker = upper
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| tok == ticker);
    if has_ticker {
        return Some("ticker");
    }
    if let Some(n) = name {
        if n.len() >= 4 && upper.contains(&n.to_uppercase()) {
            return Some("name");
        }
    }
    None
}

/// Headlines from the selected feeds. With a symbol, the company's own
/// headlines come first (newest first) and the rest of `limit` is filled
/// with general market headlines so a reader always gets context; pass
/// `company_only` to leave those out.
pub async fn news(
    fetcher: &Arc<Fetcher>,
    universe: &[Issuer],
    symbol: Option<&Symbol>,
    sources: &[String],
    limit: usize,
    company_only: bool,
) -> Result<Sourced<NewsReport>> {
    let feeds = newsfeed::select(sources)?;
    let (items, warnings, cached, stale) = newsfeed::fetch(fetcher, &feeds).await?;
    let scanned = items.len();
    let matched_name = symbol.and_then(|s| {
        universe
            .iter()
            .find(|i| &i.symbol == s)
            .and_then(|i| i.name.as_deref())
            .map(match_name)
            .filter(|n| !n.is_empty())
    });
    let (mut items, company_matches) =
        arrange(items, symbol, matched_name.as_deref(), limit, company_only);
    for it in &mut items {
        analysis::sentiment::score_item(it);
    }
    let mut s = Sourced::fresh(
        NewsReport {
            symbol: symbol.cloned(),
            matched_name,
            sources: feeds.iter().map(|f| f.id.to_string()).collect(),
            scanned,
            company_matches,
            company_only: symbol.is_some() && company_only,
            items,
        },
        newsfeed::NAME,
    );
    s.cached = cached;
    s.stale = stale;
    s.warnings = warnings;
    Ok(s)
}

/// Splits scanned items into company and market scope and orders them:
/// company headlines first, then market context up to `limit` (0 = all).
/// Returns the arranged items and the number of company matches.
pub fn arrange(
    items: Vec<NewsItem>,
    symbol: Option<&Symbol>,
    matched_name: Option<&str>,
    limit: usize,
    company_only: bool,
) -> (Vec<NewsItem>, usize) {
    let Some(sym) = symbol else {
        let mut items = items;
        for it in &mut items {
            it.scope = "market".into();
        }
        if limit > 0 {
            items.truncate(limit);
        }
        return (items, 0);
    };
    let mut company = Vec::new();
    let mut market = Vec::new();
    for mut it in items {
        match item_matches(&it, sym.as_str(), matched_name) {
            Some(how) => {
                it.matched_by = how.into();
                it.scope = "company".into();
                company.push(it);
            }
            None => {
                it.matched_by = "all".into();
                it.scope = "market".into();
                market.push(it);
            }
        }
    }
    let company_matches = company.len();
    let mut out = company;
    if limit > 0 {
        out.truncate(limit);
    }
    if !company_only {
        let room = if limit > 0 {
            limit.saturating_sub(out.len())
        } else {
            market.len()
        };
        out.extend(market.into_iter().take(room));
    }
    (out, company_matches)
}

/// Lexicon sentiment over the headlines that mention the company only.
pub async fn sentiment(
    fetcher: &Arc<Fetcher>,
    universe: &[Issuer],
    symbol: &Symbol,
    sources: &[String],
    headline_limit: usize,
) -> Result<Sourced<SentimentReport>> {
    let n = news(fetcher, universe, Some(symbol), sources, 0, true).await?;
    let report = analysis::sentiment::aggregate(
        symbol,
        n.data.matched_name.clone(),
        &n.data.items,
        headline_limit,
    );
    Ok(n.map(|_| report))
}

/// Date range and filters for [`broker`]. Without dates each ticker gets its
/// latest session with trades.
#[derive(Debug, Clone, Copy)]
pub struct BrokerQuery {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub investor: BrokerInvestor,
    pub board: BrokerBoard,
}

/// Most tickers one `broker` call fetches.
pub const BROKER_MAX_SYMBOLS: usize = 10;
/// Calendar days `broker` steps back to find the latest session with trades.
const BROKER_LOOKBACK_DAYS: u32 = 10;

/// Today's date in Jakarta.
pub fn today_wib() -> NaiveDate {
    chrono::Utc::now()
        .with_timezone(&crate::model::wib())
        .date_naive()
}

/// The most recent weekday at or before `today` whose summary has trades,
/// within the look-back; otherwise the last empty one with a warning.
async fn latest_broker_day(
    fetcher: &Arc<Fetcher>,
    symbol: &Symbol,
    q: BrokerQuery,
    today: NaiveDate,
) -> Result<Sourced<BrokerSummary>> {
    let mut last: Option<Sourced<BrokerSummary>> = None;
    for back in 0..BROKER_LOOKBACK_DAYS {
        let day = today - chrono::Days::new(back.into());
        if matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
            continue;
        }
        let s = ipot::broker_summary(fetcher, symbol, day, day, q.investor, q.board, today).await?;
        if !s.data.is_empty() {
            return Ok(s);
        }
        last = Some(s);
    }
    let mut s = last.ok_or_else(|| IdxError::NotFound(format!("no sessions for {symbol}")))?;
    s.warnings.push(format!(
        "{symbol}: no trades in the {BROKER_LOOKBACK_DAYS} days to {today}"
    ));
    Ok(s)
}

/// One ticker: the given range, or its latest session with trades.
async fn broker_one(
    fetcher: &Arc<Fetcher>,
    symbol: Symbol,
    range: Option<(NaiveDate, NaiveDate)>,
    q: BrokerQuery,
    today: NaiveDate,
) -> Result<Sourced<BrokerSummary>> {
    let Some((from, to)) = range else {
        return latest_broker_day(fetcher, &symbol, q, today).await;
    };
    let mut s =
        ipot::broker_summary_range(fetcher, &symbol, from, to, q.investor, q.board, today).await?;
    if s.data.is_empty() {
        s.warnings
            .push(format!("{symbol}: no trades from {from} to {to}"));
    }
    Ok(s)
}

/// Issuer name from the universe and broker names from the member list.
fn fill_names(s: &mut BrokerSummary, universe: &[Issuer]) {
    s.name = universe
        .iter()
        .find(|i| i.symbol == s.symbol)
        .and_then(|i| i.name.clone());
    for f in s.buyers.iter_mut().chain(s.sellers.iter_mut()) {
        f.name = brokers::name(&f.code);
    }
}

/// `from`/`to` as given to [`broker`] or [`bandar`] into a checked range;
/// `None` when neither is set.
fn explicit_range(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<Option<(NaiveDate, NaiveDate)>> {
    let range = match (from, to) {
        (None, None) => None,
        (Some(f), None) => Some((f, today)),
        (None, Some(t)) => Some((t, t)),
        (Some(f), Some(t)) => Some((f, t)),
    };
    if let Some((f, t)) = range {
        if f > t {
            return Err(IdxError::Usage(format!(
                "the range starts ({f}) after it ends ({t})"
            )));
        }
    }
    Ok(range)
}

/// Which brokers bought and sold each ticker (IPOT broker summary), with
/// broker names from the bundled IDX member list. Tickers that fail become
/// warnings; the call fails only when all of them do.
pub async fn broker(
    fetcher: &Arc<Fetcher>,
    universe: &[Issuer],
    symbols: &[Symbol],
    q: BrokerQuery,
    today: NaiveDate,
) -> Result<Sourced<Vec<BrokerSummary>>> {
    if symbols.is_empty() {
        return Err(IdxError::Usage("broker needs at least one ticker".into()));
    }
    if symbols.len() > BROKER_MAX_SYMBOLS {
        return Err(IdxError::Usage(format!(
            "broker takes at most {BROKER_MAX_SYMBOLS} tickers per call"
        )));
    }
    let range = explicit_range(q.from, q.to, today)?;
    let results: Vec<Result<Sourced<BrokerSummary>>> = stream::iter(symbols.to_vec())
        .map(|s| broker_one(fetcher, s, range, q, today))
        .buffered(3)
        .collect()
        .await;

    let mut out: Option<Sourced<Vec<BrokerSummary>>> = None;
    let mut pending_warnings = Vec::new();
    let mut first_err: Option<IdxError> = None;
    for (sym, r) in symbols.iter().zip(results) {
        match r {
            Ok(mut s) => {
                fill_names(&mut s.data, universe);
                match &mut out {
                    None => out = Some(s.map(|d| vec![d])),
                    Some(acc) => {
                        acc.absorb(&s);
                        acc.data.push(s.data);
                    }
                }
            }
            Err(e) => {
                pending_warnings.push(match &e {
                    IdxError::NotFound(_) => format!("no broker summary for {sym}"),
                    other => format!("{sym}: {other}"),
                });
                first_err.get_or_insert(e);
            }
        }
    }
    let Some(mut result) = out else {
        return Err(first_err.unwrap_or_else(|| IdxError::NotFound("no data".into())));
    };
    result.warnings.extend(pending_warnings);
    Ok(result)
}

// ---- bandarmology ---------------------------------------------------------------

/// Range, tier and filters for [`bandar`]. Explicit dates win over `period`.
#[derive(Debug, Clone, Copy)]
pub struct BandarQuery {
    pub period: BandarPeriod,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub top: u32,
    pub investor: BrokerInvestor,
    pub board: BrokerBoard,
    /// Add the session-by-session series (one request per session).
    pub daily: bool,
}

/// Most tickers one `bandar --daily` call takes: the series costs one
/// request per session per ticker.
pub const BANDAR_DAILY_MAX_SYMBOLS: usize = 3;
/// Tickers a bandar screen scans by default: the most traded by value.
pub const BANDAR_SCAN_DEFAULT: usize = 100;
/// Most tickers one bandar screen scans.
pub const BANDAR_SCAN_MAX: usize = 300;

/// Daily candles that reach back past `from` (the base close sits before it).
fn history_period_for(from: NaiveDate, today: NaiveDate) -> Period {
    let days = (today - from).num_days() + 10;
    [
        (28, Period::M1),
        (88, Period::M3),
        (178, Period::M6),
        (360, Period::Y1),
        (720, Period::Y2),
        (1800, Period::Y5),
    ]
    .into_iter()
    .find(|(max, _)| days <= *max)
    .map_or(Period::Max, |(_, p)| p)
}

/// The range's last price and its move in percent. The price is the
/// quote's when the range ends on the quote's session (that session's daily
/// bar can still be missing), otherwise the close on or before `to`. The
/// base is the previous close for a one-session range on the quote's day,
/// otherwise the last close before `from`.
pub async fn price_move(
    p: &dyn Provider,
    symbol: &Symbol,
    from: NaiveDate,
    to: NaiveDate,
    quote: Option<&Quote>,
    today: NaiveDate,
) -> Result<(Option<f64>, Option<f64>)> {
    let live =
        quote.filter(|q| q.price.is_some() && q.market_time.is_some_and(|t| t.date_naive() == to));
    if let Some(q) = live {
        if from == to && q.previous_close.is_some() {
            return Ok((q.price, bd::change_pct(q.previous_close, q.price)));
        }
    }
    let h = p
        .history(symbol, history_period_for(from, today), Interval::Day1)
        .await?;
    let candles = &h.data.candles;
    let base = candles
        .iter()
        .rev()
        .find(|c| c.date() < from)
        .map(|c| c.close);
    let price = live.and_then(|q| q.price).or_else(|| {
        candles
            .iter()
            .rev()
            .find(|c| c.date() <= to)
            .map(|c| c.close)
    });
    Ok((price, bd::change_pct(base, price)))
}

/// One ticker's summary for the query: the explicit range, or the period
/// ending on the ticker's latest session with trades. With `daily`, every
/// weekday of the range as well.
async fn bandar_one(
    fetcher: &Arc<Fetcher>,
    symbol: Symbol,
    range: Option<(NaiveDate, NaiveDate)>,
    q: BandarQuery,
    today: NaiveDate,
) -> Result<(Sourced<BrokerSummary>, Vec<BrokerSummary>)> {
    let bq = BrokerQuery {
        from: None,
        to: None,
        investor: q.investor,
        board: q.board,
    };
    let mut s = match range {
        Some(r) => broker_one(fetcher, symbol.clone(), Some(r), bq, today).await?,
        None => {
            let latest = latest_broker_day(fetcher, &symbol, bq, today).await?;
            if q.period == BandarPeriod::Day1 || latest.data.is_empty() {
                latest
            } else {
                let end = latest.data.to;
                let mut r = ipot::broker_summary_range(
                    fetcher,
                    &symbol,
                    q.period.start(end),
                    end,
                    q.investor,
                    q.board,
                    today,
                )
                .await?;
                r.absorb(&latest);
                r
            }
        }
    };
    let mut sessions = Vec::new();
    if q.daily {
        let days = bd::weekdays(s.data.from, s.data.to);
        let got: Vec<Result<Sourced<BrokerSummary>>> = stream::iter(days)
            .map(|d| ipot::broker_summary(fetcher, &symbol, d, d, q.investor, q.board, today))
            .buffered(3)
            .collect()
            .await;
        for r in got {
            match r {
                Ok(day) => {
                    s.absorb(&day);
                    sessions.push(day.data);
                }
                Err(e) => s.warnings.push(format!(
                    "{symbol}: a session of the daily series failed: {e}"
                )),
            }
        }
    }
    Ok((s, sessions))
}

/// Bandarmology for each ticker: the top 1/3/5/10 buyers against the top
/// sellers over the range, the headline signal and phase, the top buyers'
/// average against the price, foreign flow, and with `daily` the session
/// series. Tickers that fail become warnings; the call fails only when all
/// of them do.
pub async fn bandar(
    p: &dyn Provider,
    fetcher: &Arc<Fetcher>,
    universe: &[Issuer],
    symbols: &[Symbol],
    q: BandarQuery,
    today: NaiveDate,
) -> Result<Sourced<Vec<BandarReport>>> {
    if symbols.is_empty() {
        return Err(IdxError::Usage("bandar needs at least one ticker".into()));
    }
    if symbols.len() > BROKER_MAX_SYMBOLS {
        return Err(IdxError::Usage(format!(
            "bandar takes at most {BROKER_MAX_SYMBOLS} tickers per call"
        )));
    }
    if q.daily && symbols.len() > BANDAR_DAILY_MAX_SYMBOLS {
        return Err(IdxError::Usage(format!(
            "the daily series takes at most {BANDAR_DAILY_MAX_SYMBOLS} tickers per call (one request per session each)"
        )));
    }
    bd::check_top(q.top)?;
    let range = explicit_range(q.from, q.to, today)?;
    let period = if range.is_some() {
        "custom"
    } else {
        q.period.as_str()
    };

    let results: Vec<Result<(Sourced<BrokerSummary>, Vec<BrokerSummary>)>> =
        stream::iter(symbols.to_vec())
            .map(|s| bandar_one(fetcher, s, range, q, today))
            .buffered(3)
            .collect()
            .await;
    let ok: Vec<Symbol> = symbols
        .iter()
        .zip(&results)
        .filter(|(_, r)| r.is_ok())
        .map(|(s, _)| s.clone())
        .collect();
    let quotes = if ok.is_empty() {
        Vec::new()
    } else {
        p.quotes(&ok).await.map(|q| q.data).unwrap_or_default()
    };

    let mut out: Option<Sourced<Vec<BandarReport>>> = None;
    let mut pending = Vec::new();
    let mut first_err: Option<IdxError> = None;
    for (sym, r) in symbols.iter().zip(results) {
        let (mut s, mut sessions) = match r {
            Ok(v) => v,
            Err(e) => {
                pending.push(match &e {
                    IdxError::NotFound(_) => format!("no broker summary for {sym}"),
                    other => format!("{sym}: {other}"),
                });
                first_err.get_or_insert(e);
                continue;
            }
        };
        fill_names(&mut s.data, universe);
        let quote = quotes.iter().find(|x| &x.symbol == sym);
        let (price, chg) = if s.data.is_empty() {
            (quote.and_then(|x| x.price), None)
        } else {
            match price_move(p, sym, s.data.from, s.data.to, quote, today).await {
                Ok(v) => v,
                Err(e) => {
                    s.warnings
                        .push(format!("{sym}: no price move for the range ({e})"));
                    (quote.and_then(|x| x.price), None)
                }
            }
        };
        let mut report = bd::report(&s.data, period, q.top, price, chg);
        if q.daily {
            sessions.sort_by_key(|d| d.to);
            report.daily = Some(bd::daily(&sessions, q.top));
        }
        let s = s.map(|_| report);
        match &mut out {
            None => out = Some(s.map(|d| vec![d])),
            Some(acc) => {
                acc.absorb(&s);
                acc.data.push(s.data);
            }
        }
    }
    let Some(mut result) = out else {
        return Err(first_err.unwrap_or_else(|| IdxError::NotFound("no data".into())));
    };
    result.warnings.extend(pending);
    Ok(result)
}

/// What a bandar screen applies.
#[derive(Debug, Clone)]
pub struct BandarScreenOptions {
    pub period: BandarPeriod,
    pub top: u32,
    pub preset: Option<Preset>,
    /// Clauses on top of the preset's.
    pub filters: Vec<bd::Filter>,
    /// Default `acc`.
    pub sort: Option<bd::Key>,
    /// Default: descending, ascending for the distribution presets.
    pub descending: Option<bool>,
    /// Rows returned; 0 = all.
    pub limit: usize,
    /// How many of the most traded tickers to scan when `symbols` is empty.
    pub scan: usize,
    /// Scan these instead of the most traded.
    pub symbols: Vec<Symbol>,
}

/// One scanned ticker's range summary; owned arguments keep the stream's
/// future general over lifetimes.
async fn screen_summary(
    fetcher: &Arc<Fetcher>,
    symbol: Symbol,
    from: NaiveDate,
    to: NaiveDate,
    today: NaiveDate,
) -> (Symbol, Result<Sourced<BrokerSummary>>) {
    let r = ipot::broker_summary_range(
        fetcher,
        &symbol,
        from,
        to,
        BrokerInvestor::All,
        BrokerBoard::All,
        today,
    )
    .await;
    (symbol, r)
}

/// A row index with its price and move over the range.
type RowMove = (usize, Result<(Option<f64>, Option<f64>)>);

/// One row's price move for the screen.
async fn screen_move(
    p: &dyn Provider,
    i: usize,
    symbol: Symbol,
    from: NaiveDate,
    to: NaiveDate,
    quote: Option<Quote>,
    today: NaiveDate,
) -> RowMove {
    (
        i,
        price_move(p, &symbol, from, to, quote.as_ref(), today).await,
    )
}

/// Tickers to scan and their quotes: the `scan` most traded by value from
/// one batch quote of the universe, or the given `symbols` (quoted when
/// possible).
async fn scan_candidates(
    p: &dyn Provider,
    universe: &[Issuer],
    symbols: &[Symbol],
    scan: usize,
) -> Result<(Vec<Symbol>, Sourced<Vec<Quote>>)> {
    let (candidates, q) = if symbols.is_empty() {
        if universe.is_empty() {
            return Err(IdxError::Usage("the ticker universe is empty".into()));
        }
        let all: Vec<Symbol> = universe.iter().map(|i| i.symbol.clone()).collect();
        let q = quotes_all(p, &all).await?;
        let traded = |x: &Quote| x.price.unwrap_or(0.0) * x.volume.unwrap_or(0) as f64;
        let mut by_value: Vec<&Quote> = q.data.iter().filter(|x| traded(x) > 0.0).collect();
        by_value.sort_by(|a, b| {
            traded(b)
                .partial_cmp(&traded(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let n = scan.clamp(1, BANDAR_SCAN_MAX);
        let picked: Vec<Symbol> = by_value.iter().take(n).map(|x| x.symbol.clone()).collect();
        (picked, q)
    } else {
        if symbols.len() > BANDAR_SCAN_MAX {
            return Err(IdxError::Usage(format!(
                "a bandar screen scans at most {BANDAR_SCAN_MAX} tickers"
            )));
        }
        let q = quotes_all(p, symbols)
            .await
            .unwrap_or_else(|_| Sourced::fresh(Vec::new(), p.name()));
        (symbols.to_vec(), q)
    };
    if candidates.is_empty() {
        return Err(IdxError::NotFound("no traded tickers to scan".into()));
    }
    Ok((candidates, q))
}

/// The latest session with trades, read from the first candidates. When
/// every probe was an offline cache miss, that is the error.
async fn scan_end(
    fetcher: &Arc<Fetcher>,
    candidates: &[Symbol],
    today: NaiveDate,
) -> Result<NaiveDate> {
    let bq = BrokerQuery {
        from: None,
        to: None,
        investor: BrokerInvestor::All,
        board: BrokerBoard::All,
    };
    let mut offline = None;
    let mut all_offline = true;
    for sym in candidates.iter().take(3) {
        match latest_broker_day(fetcher, sym, bq, today).await {
            Ok(s) if !s.data.is_empty() => return Ok(s.data.to),
            Err(e @ IdxError::OfflineMiss(_)) => {
                offline.get_or_insert(e);
            }
            _ => all_offline = false,
        }
    }
    if let (Some(e), true) = (offline, all_offline) {
        return Err(e);
    }
    Err(IdxError::NotFound(format!(
        "no session with trades in the {BROKER_LOOKBACK_DAYS} days to {today}"
    )))
}

/// Every candidate's summary over `from..=end`, three at a time, then one
/// sequential pass over provider failures (not unknown codes).
async fn scan_summaries(
    fetcher: &Arc<Fetcher>,
    candidates: &[Symbol],
    from: NaiveDate,
    end: NaiveDate,
    today: NaiveDate,
) -> Vec<(Symbol, Result<Sourced<BrokerSummary>>)> {
    let mut fetched: Vec<(Symbol, Result<Sourced<BrokerSummary>>)> =
        stream::iter(candidates.to_vec())
            .map(|s| screen_summary(fetcher, s, from, end, today))
            .buffer_unordered(3)
            .collect()
            .await;
    let retry: Vec<Symbol> = fetched
        .iter()
        .filter(|(_, r)| matches!(r, Err(e) if !matches!(e, IdxError::NotFound(_))))
        .map(|(s, _)| s.clone())
        .collect();
    if !retry.is_empty() {
        tokio::time::sleep(ipot::RECOVER_PAUSE).await;
        for sym in retry {
            let again = screen_summary(fetcher, sym.clone(), from, end, today).await;
            if let Some(slot) = fetched.iter_mut().find(|(s, _)| *s == sym) {
                *slot = again;
            }
        }
    }
    fetched
}

/// Bandarmology screener: reads the most traded stocks (or the given ones)
/// over the period ending on the latest session and keeps those matching
/// the preset and filters. One broker-summary request per scanned ticker;
/// on multi-day periods up to three more for the rebuilt foreign net (see
/// `ipot::broker_summary_range`) and one history request per row the other
/// filters left.
pub async fn bandar_screen(
    p: &dyn Provider,
    fetcher: &Arc<Fetcher>,
    universe: &[Issuer],
    opts: &BandarScreenOptions,
    today: NaiveDate,
) -> Result<Sourced<BandarScreen>> {
    bd::check_top(opts.top)?;
    let mut filters = match opts.preset {
        Some(pr) => bd::parse_filters(pr.filter())?,
        None => Vec::new(),
    };
    filters.extend(opts.filters.iter().cloned());

    // Candidates and their quotes.
    let (candidates, mut out) = scan_candidates(p, universe, &opts.symbols, opts.scan).await?;
    let quotes = std::mem::take(&mut out.data);
    out.warnings.retain(|w| !w.contains("returned no quote"));
    let end = scan_end(fetcher, &candidates, today).await?;
    let from = opts.period.start(end);
    let period = opts.period.as_str();
    let fetched = scan_summaries(fetcher, &candidates, from, end, today).await;
    let quote_of = |sym: &Symbol| quotes.iter().find(|x| &x.symbol == sym);
    let mut failed = 0usize;
    let mut reports: Vec<BandarReport> = Vec::new();
    let mut summaries: Vec<BrokerSummary> = Vec::new();
    for (sym, r) in fetched {
        match r {
            Ok(mut s) if !s.data.is_empty() => {
                out.absorb(&s);
                fill_names(&mut s.data, universe);
                if s.data.name.is_none() {
                    s.data.name = quote_of(&sym).and_then(|x| x.name.clone());
                }
                let live =
                    quote_of(&sym).filter(|x| x.market_time.is_some_and(|t| t.date_naive() == end));
                let price = live.and_then(|x| x.price);
                let chg = match (opts.period, live) {
                    (BandarPeriod::Day1, Some(x)) => bd::change_pct(x.previous_close, x.price),
                    _ => None,
                };
                reports.push(bd::report(&s.data, period, opts.top, price, chg));
                summaries.push(s.data);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(symbol = %sym, error = %e, "no broker summary");
                failed += 1;
            }
        }
    }
    let checked = reports.len();
    let (move_filters, plain): (Vec<bd::Filter>, Vec<bd::Filter>) =
        filters.iter().cloned().partition(|f| f.key.needs_move());
    let keep: Vec<usize> = (0..reports.len())
        .filter(|i| bd::matches(&bd::screen_row(&reports[*i]), &plain))
        .collect();

    // The price move for what is left, where the quote could not give it.
    let need: Vec<(usize, Symbol, Option<Quote>)> = keep
        .iter()
        .filter(|i| reports[**i].price_change_pct.is_none())
        .map(|i| {
            let sym = reports[*i].symbol.clone();
            let q = quote_of(&sym).cloned();
            (*i, sym, q)
        })
        .collect();
    let moves: Vec<RowMove> = stream::iter(need)
        .map(|(i, sym, q)| screen_move(p, i, sym, from, end, q, today))
        .buffer_unordered(8)
        .collect()
        .await;
    for (i, m) in moves {
        if let Ok((price, chg)) = m {
            let price = price.or(reports[i].price);
            reports[i] = bd::report(&summaries[i], period, opts.top, price, chg);
        }
    }
    let mut rows: Vec<_> = keep
        .iter()
        .map(|i| bd::screen_row(&reports[*i]))
        .filter(|r| bd::matches(r, &move_filters))
        .collect();
    let sort = opts.sort.unwrap_or(bd::Key::Acc);
    let descending = opts
        .descending
        .unwrap_or_else(|| !opts.preset.is_some_and(|p| p.ascending()));
    bd::sort_rows(&mut rows, sort, descending);
    let total = rows.len();
    if opts.limit > 0 {
        rows.truncate(opts.limit);
    }
    if failed > 0 {
        out.warnings.push(format!(
            "{failed} of {} scanned tickers had no broker summary",
            candidates.len()
        ));
    }
    let mut out = out.map(|_| BandarScreen {
        period: opts.period,
        from,
        to: end,
        top: opts.top,
        preset: opts.preset.map(|p| p.name().to_string()),
        filters: filters.iter().map(|f| f.raw.clone()).collect(),
        sort: sort.name().to_string(),
        descending,
        scanned: candidates.len(),
        checked,
        total_matches: total,
        rows,
    });
    out.provider = ipot::NAME;
    Ok(out)
}

/// What [`bandar_broker`] scans.
#[derive(Debug, Clone)]
pub struct BrokerActivityOptions {
    /// Exchange-member code, e.g. `YP`.
    pub code: String,
    pub period: BandarPeriod,
    /// How many of the most traded tickers to scan when `symbols` is empty.
    pub scan: usize,
    /// Scan these instead of the most traded.
    pub symbols: Vec<Symbol>,
    /// Rows per side; 0 = all.
    pub limit: usize,
}

fn stock_row(
    s: &BrokerSummary,
    this: &crate::model::BrokerFlow,
    other: Option<&crate::model::BrokerFlow>,
    buying: bool,
) -> BrokerStockRow {
    let opposite = other.and_then(|o| o.value);
    let net = match (this.value, opposite) {
        (Some(v), Some(o)) if buying => Some(v - o),
        (Some(v), Some(o)) => Some(o - v),
        _ => None,
    };
    BrokerStockRow {
        symbol: s.symbol.clone(),
        name: s.name.clone(),
        rank: this.rank,
        lots: this.lots,
        value: this.value,
        average_price: this.average_price,
        share_pct: match (this.value, s.total_value) {
            (Some(v), Some(t)) if t > 0.0 => Some(v / t * 100.0),
            _ => None,
        },
        opposite_value: opposite,
        net_value: net,
    }
}

/// Broker activity: where one exchange member shows up among the top ten
/// buyers and sellers of the most traded stocks (or the given ones) over
/// the period ending on the latest session. One broker-summary request per
/// scanned ticker, shared with the bandar screen's cache.
pub async fn bandar_broker(
    p: &dyn Provider,
    fetcher: &Arc<Fetcher>,
    universe: &[Issuer],
    opts: &BrokerActivityOptions,
    today: NaiveDate,
) -> Result<Sourced<BrokerActivity>> {
    let code = opts.code.trim().to_ascii_uppercase();
    if code.len() != 2 || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(IdxError::Usage(format!(
            "{:?} is not a broker code; codes are two characters such as YP or CC (see `idx market members`)",
            opts.code
        )));
    }
    let (candidates, mut out) = scan_candidates(p, universe, &opts.symbols, opts.scan).await?;
    let quotes = std::mem::take(&mut out.data);
    out.warnings.retain(|w| !w.contains("returned no quote"));
    let name = brokers::name(&code);
    if name.is_none() {
        out.warnings
            .push(format!("{code} is not on the bundled exchange-member list"));
    }
    let end = scan_end(fetcher, &candidates, today).await?;
    let from = opts.period.start(end);
    let fetched = scan_summaries(fetcher, &candidates, from, end, today).await;

    let mut failed = 0usize;
    let mut checked = 0usize;
    let mut bought = Vec::new();
    let mut sold = Vec::new();
    for (sym, r) in fetched {
        match r {
            Ok(mut s) if !s.data.is_empty() => {
                out.absorb(&s);
                checked += 1;
                fill_names(&mut s.data, universe);
                if s.data.name.is_none() {
                    s.data.name = quotes
                        .iter()
                        .find(|x| x.symbol == sym)
                        .and_then(|x| x.name.clone());
                }
                let buy = s.data.buyers.iter().find(|f| f.code == code);
                let sell = s.data.sellers.iter().find(|f| f.code == code);
                if let Some(b) = buy {
                    bought.push(stock_row(&s.data, b, sell, true));
                }
                if let Some(x) = sell {
                    sold.push(stock_row(&s.data, x, buy, false));
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(symbol = %sym, error = %e, "no broker summary");
                failed += 1;
            }
        }
    }
    let by_value = |a: &BrokerStockRow, b: &BrokerStockRow| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    bought.sort_by(by_value);
    sold.sort_by(by_value);
    let bought_value = bought.iter().filter_map(|r| r.value).sum();
    let sold_value = sold.iter().filter_map(|r| r.value).sum();
    if opts.limit > 0 {
        bought.truncate(opts.limit);
        sold.truncate(opts.limit);
    }
    if failed > 0 {
        out.warnings.push(format!(
            "{failed} of {} scanned tickers had no broker summary",
            candidates.len()
        ));
    }
    let mut out = out.map(|_| BrokerActivity {
        code,
        name,
        period: opts.period.as_str().to_string(),
        from,
        to: end,
        scanned: candidates.len(),
        checked,
        failed,
        bought_value,
        sold_value,
        bought,
        sold,
    });
    out.provider = ipot::NAME;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{History, Quote};

    #[test]
    fn match_names_and_items() {
        assert_eq!(match_name("PT Bank Central Asia Tbk"), "Bank Central Asia");
        assert_eq!(
            match_name("ADARO ANDALAN INDONESIA Tbk"),
            "ADARO ANDALAN INDONESIA"
        );
        let mk = |t: &str| NewsItem {
            title: t.into(),
            url: "u".into(),
            source: "s".into(),
            published: None,
            summary: String::new(),
            matched_by: String::new(),
            scope: String::new(),
            sentiment: 0.0,
            positive_terms: vec![],
            negative_terms: vec![],
        };
        assert_eq!(
            item_matches(&mk("Saham BBCA naik"), "BBCA", None),
            Some("ticker")
        );
        // Company headlines first, then market context fills the limit.
        let bbca = Symbol::parse("BBCA").unwrap();
        let feed = vec![
            mk("IHSG menguat"),
            mk("BBCA bagi dividen"),
            mk("Rupiah melemah"),
            mk("Laba BBCA naik"),
            mk("Harga emas"),
        ];
        let (items, matches) = arrange(feed.clone(), Some(&bbca), None, 3, false);
        assert_eq!(matches, 2);
        let got: Vec<(&str, &str)> = items
            .iter()
            .map(|i| (i.title.as_str(), i.scope.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("BBCA bagi dividen", "company"),
                ("Laba BBCA naik", "company"),
                ("IHSG menguat", "market"),
            ]
        );
        let (items, _) = arrange(feed.clone(), Some(&bbca), None, 3, true);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.scope == "company"));
        let (items, matches) = arrange(feed, None, None, 0, false);
        assert_eq!((items.len(), matches), (5, 0));
        assert!(items.iter().all(|i| i.scope == "market"));
        assert_eq!(
            item_matches(&mk("Laba (BBCA) naik"), "BBCA", None),
            Some("ticker")
        );
        assert_eq!(
            item_matches(
                &mk("Bank Central Asia bagi dividen"),
                "BBCA",
                Some("Bank Central Asia")
            ),
            Some("name")
        );
        assert_eq!(item_matches(&mk("BBCAX bukan BBCA?"), "BBRI", None), None);
    }
    use async_trait::async_trait;

    struct Fake;

    #[async_trait]
    impl Provider for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn quotes(&self, _: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
            unimplemented!()
        }
        async fn history(&self, _: &Symbol, _: Period, _: Interval) -> Result<Sourced<History>> {
            unimplemented!()
        }
        async fn fundamentals(&self, s: &Symbol) -> Result<Sourced<Fundamentals>> {
            if s.as_str().starts_with("ZZZZ") {
                return Err(IdxError::NotFound(s.to_string()));
            }
            let mut f = Fundamentals::empty(s.clone(), "fake");
            f.valuation.pe_trailing = Some(8.0);
            f.valuation.price_to_book = Some(0.9);
            f.profitability.roe_pct = Some(20.0);
            Ok(Sourced::fresh(f, "fake"))
        }
    }

    fn syms(list: &[&str]) -> Vec<Symbol> {
        list.iter().map(|s| Symbol::parse(s).unwrap()).collect()
    }

    #[tokio::test]
    async fn fundamentals_carry_a_verdict() {
        let f = fundamentals(&Fake, &Symbol::parse("BBCA").unwrap())
            .await
            .unwrap();
        assert!(f.data.verdict.is_some());
    }

    #[tokio::test]
    async fn compare_collects_warnings_and_keeps_order() {
        let c = compare(&Fake, &syms(&["ZZZZ", "BBCA", "BBRI"]))
            .await
            .unwrap();
        let got: Vec<&str> = c.data.iter().map(|f| f.symbol.as_str()).collect();
        assert_eq!(got, ["BBCA", "BBRI"]);
        assert_eq!(c.warnings, vec!["no data for ZZZZ".to_string()]);
        assert!(c.data.iter().all(|f| f.verdict.is_some()));

        let err = compare(&Fake, &syms(&["ZZZZ", "ZZZZ2"])).await.unwrap_err();
        assert_eq!(err.exit_code(), 4);
        let err = compare(&Fake, &syms(&["BBCA"])).await.unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }
    mod broker {
        use super::super::*;
        use crate::cache::Cache;
        use crate::http::{CacheMode, FixtureBackend};

        const DAY: &str = include_str!("../tests/fixtures/ipot/brokersummary_bbca_20261016.html");
        const EMPTY: &str =
            include_str!("../tests/fixtures/ipot/brokersummary_bbca_20261017_no_trades.html");
        const INVALID: &str =
            include_str!("../tests/fixtures/ipot/brokersummary_zzzz_invalid.html");

        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn fetcher(be: FixtureBackend) -> (Arc<Fetcher>, Arc<FixtureBackend>, tempfile::TempDir) {
            let dir = tempfile::tempdir().unwrap();
            let be = Arc::new(be);
            let f = Arc::new(Fetcher::new(
                be.clone(),
                Cache::new(dir.path()),
                CacheMode::Normal,
            ));
            (f, be, dir)
        }

        fn q(from: Option<NaiveDate>, to: Option<NaiveDate>) -> BrokerQuery {
            BrokerQuery {
                from,
                to,
                investor: BrokerInvestor::All,
                board: BrokerBoard::All,
            }
        }

        fn syms(list: &[&str]) -> Vec<Symbol> {
            list.iter().map(|s| Symbol::parse(s).unwrap()).collect()
        }

        fn universe() -> Vec<Issuer> {
            vec![Issuer {
                symbol: Symbol::parse("BBCA").unwrap(),
                name: Some("Bank Central Asia Tbk".into()),
            }]
        }

        #[tokio::test]
        async fn latest_session_skips_weekends_and_empty_days_and_names_brokers() {
            // Monday 19 Oct has no trades yet; the walk skips the weekend to Friday 16 Oct.
            let (f, be, _d) = fetcher(
                FixtureBackend::new()
                    .route("code=BBCA&start=10/19/2026", 200, EMPTY)
                    .route("code=BBCA&start=10/16/2026", 200, DAY),
            );
            let got = broker(
                &f,
                &universe(),
                &syms(&["BBCA"]),
                q(None, None),
                d(2026, 10, 19),
            )
            .await
            .unwrap();
            assert_eq!(be.call_count(), 2);
            let s = &got.data[0];
            assert_eq!((s.from, s.to), (d(2026, 10, 16), d(2026, 10, 16)));
            assert_eq!(s.name.as_deref(), Some("Bank Central Asia Tbk"));
            assert_eq!(
                s.buyers[0].name.as_deref(),
                Some("SEMESTA INDOVEST SEKURITAS")
            );
            assert_eq!(s.sellers[0].name.as_deref(), Some("MANDIRI SEKURITAS"));
            assert!(got.warnings.is_empty(), "{:?}", got.warnings);
        }

        #[tokio::test]
        async fn explicit_range_and_partial_failure() {
            let (f, _be, _d) = fetcher(
                FixtureBackend::new()
                    .route("code=BBCA&start=10/17/2026", 200, EMPTY)
                    .route("code=ZZZZ", 200, INVALID),
            );
            let day = d(2026, 10, 17);
            let got = broker(&f, &[], &syms(&["BBCA", "ZZZZ"]), q(None, Some(day)), day)
                .await
                .unwrap();
            assert_eq!(got.data.len(), 1);
            assert!(got.data[0].is_empty());
            assert!(
                got.warnings
                    .iter()
                    .any(|w| w.contains("no trades from 2026-10-17")),
                "{:?}",
                got.warnings
            );
            assert!(
                got.warnings
                    .iter()
                    .any(|w| w.contains("no broker summary for ZZZZ")),
                "{:?}",
                got.warnings
            );

            let err = broker(&f, &[], &syms(&["ZZZZ"]), q(Some(day), Some(day)), day)
                .await
                .unwrap_err();
            assert_eq!(err.exit_code(), 4);
        }

        #[tokio::test]
        async fn usage_errors() {
            let (f, be, _d) = fetcher(FixtureBackend::new());
            let day = d(2026, 10, 16);
            let err = broker(
                &f,
                &[],
                &syms(&["BBCA"]),
                q(Some(day), Some(d(2026, 10, 1))),
                day,
            )
            .await
            .unwrap_err();
            assert_eq!(err.exit_code(), 2);
            let many: Vec<String> = (0..11)
                .map(|i| format!("AA{}", (b'A' + i) as char))
                .collect();
            let many: Vec<&str> = many.iter().map(String::as_str).collect();
            let err = broker(&f, &[], &syms(&many), q(None, None), day)
                .await
                .unwrap_err();
            assert_eq!(err.exit_code(), 2);
            assert_eq!(be.call_count(), 0);
        }
    }

    mod bandar {
        use super::super::*;
        use crate::cache::Cache;
        use crate::http::{CacheMode, FixtureBackend};
        use crate::model::{BandarPhase, BandarSignal, Candle, History};
        use async_trait::async_trait;
        use chrono::{FixedOffset, TimeZone};

        const DAY: &str = include_str!("../tests/fixtures/ipot/brokersummary_bbca_20261016.html");
        const WEEK: &str =
            include_str!("../tests/fixtures/ipot/brokersummary_bbca_20261012_20261016.html");
        const BRPT: &str =
            include_str!("../tests/fixtures/ipot/brokersummary_brpt_20261012_20261016.html");
        const TLKM: &str =
            include_str!("../tests/fixtures/ipot/brokersummary_tlkm_20261012_20261016.html");

        fn d(m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, m, day).unwrap()
        }

        fn at(day: NaiveDate, h: u32, mi: u32) -> chrono::DateTime<FixedOffset> {
            crate::model::wib()
                .from_local_datetime(&day.and_hms_opt(h, mi, 0).unwrap())
                .unwrap()
        }

        /// Quotes on Friday 16 Oct; daily closes 6,050 on 9 Oct, then 5,175 by the 16th.
        struct Prices;

        #[async_trait]
        impl Provider for Prices {
            fn name(&self) -> &'static str {
                "fake"
            }
            async fn quotes(&self, syms: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
                let list = syms
                    .iter()
                    .map(|s| {
                        let mut q = Quote::empty(s.clone(), "fake");
                        q.price = Some(5_175.0);
                        q.previous_close = Some(6_075.0);
                        q.volume = Some(1_000_000);
                        q.market_time = Some(at(d(10, 16), 16, 14));
                        q
                    })
                    .collect();
                Ok(Sourced::fresh(list, "fake"))
            }
            async fn history(
                &self,
                s: &Symbol,
                p: Period,
                i: Interval,
            ) -> Result<Sourced<History>> {
                let closes = [
                    (d(10, 8), 6_100.0),
                    (d(10, 9), 6_050.0),
                    (d(10, 12), 6_000.0),
                    (d(10, 15), 6_075.0),
                ];
                let candles = closes
                    .iter()
                    .map(|(day, c)| Candle {
                        time: at(*day, 9, 0),
                        open: *c,
                        high: *c,
                        low: *c,
                        close: *c,
                        adj_close: None,
                        volume: 1,
                    })
                    .collect::<Vec<_>>();
                Ok(Sourced::fresh(
                    History {
                        symbol: s.clone(),
                        name: None,
                        currency: None,
                        period: p,
                        interval: i,
                        summary: History::summarise(&candles),
                        candles,
                    },
                    "fake",
                ))
            }
            async fn fundamentals(
                &self,
                _: &Symbol,
            ) -> Result<Sourced<crate::model::Fundamentals>> {
                unimplemented!()
            }
        }

        fn fetcher(be: FixtureBackend) -> (Arc<Fetcher>, Arc<FixtureBackend>, tempfile::TempDir) {
            let dir = tempfile::tempdir().unwrap();
            let be = Arc::new(be);
            let f = Arc::new(Fetcher::new(
                be.clone(),
                Cache::new(dir.path()),
                CacheMode::Normal,
            ));
            (f, be, dir)
        }

        fn q(period: BandarPeriod) -> BandarQuery {
            BandarQuery {
                period,
                from: None,
                to: None,
                top: 5,
                investor: BrokerInvestor::All,
                board: BrokerBoard::All,
                daily: false,
            }
        }

        fn syms(list: &[&str]) -> Vec<Symbol> {
            list.iter().map(|s| Symbol::parse(s).unwrap()).collect()
        }

        /// BRPT and TLKM over the week: the range page (its foreign net
        /// counts the 16th twice), plus the pages the net is rebuilt from.
        fn others(be: FixtureBackend) -> FixtureBackend {
            be.route("code=BRPT&start=10/12/2026&end=10/16/2026", 200, BRPT)
                .route("code=TLKM&start=10/12/2026&end=10/16/2026", 200, TLKM)
                .route(
                    "code=BRPT&start=10/12/2026&end=10/14/2026",
                    200,
                    include_str!(
                        "../tests/fixtures/ipot/brokersummary_brpt_20261012_20261014.html"
                    ),
                )
                .route(
                    "code=BRPT&start=10/15/2026&end=10/15/2026",
                    200,
                    include_str!("../tests/fixtures/ipot/brokersummary_brpt_20261015.html"),
                )
                .route(
                    "code=BRPT&start=10/16/2026&end=10/16/2026",
                    200,
                    include_str!("../tests/fixtures/ipot/brokersummary_brpt_20261016.html"),
                )
                .route(
                    "code=TLKM&start=10/12/2026&end=10/14/2026",
                    200,
                    include_str!(
                        "../tests/fixtures/ipot/brokersummary_tlkm_20261012_20261014.html"
                    ),
                )
                .route(
                    "code=TLKM&start=10/15/2026&end=10/15/2026",
                    200,
                    include_str!("../tests/fixtures/ipot/brokersummary_tlkm_20261015.html"),
                )
                .route(
                    "code=TLKM&start=10/16/2026&end=10/16/2026",
                    200,
                    include_str!("../tests/fixtures/ipot/brokersummary_tlkm_20261016.html"),
                )
        }

        fn days() -> FixtureBackend {
            let mut be = FixtureBackend::new()
                .route("code=BBCA&start=10/16/2026&end=10/16/2026", 200, DAY)
                .route("code=BBCA&start=10/12/2026&end=10/16/2026", 200, WEEK)
                .route(
                    "code=BBCA&start=10/12/2026&end=10/14/2026",
                    200,
                    include_str!(
                        "../tests/fixtures/ipot/brokersummary_bbca_20261012_20261014.html"
                    ),
                );
            for (day, html) in [
                (
                    "12",
                    include_str!("../tests/fixtures/ipot/brokersummary_bbca_20261012.html"),
                ),
                (
                    "13",
                    include_str!("../tests/fixtures/ipot/brokersummary_bbca_20261013.html"),
                ),
                (
                    "14",
                    include_str!("../tests/fixtures/ipot/brokersummary_bbca_20261014.html"),
                ),
                (
                    "15",
                    include_str!("../tests/fixtures/ipot/brokersummary_bbca_20261015.html"),
                ),
            ] {
                be = be.route(
                    &format!("code=BBCA&start=10/{day}/2026&end=10/{day}/2026"),
                    200,
                    html,
                );
            }
            be
        }

        #[tokio::test]
        async fn the_week_ends_on_the_latest_session_and_reads_markdown() {
            let (f, _be, _d) = fetcher(days());
            let got = super::super::bandar(
                &Prices,
                &f,
                &[],
                &syms(&["BBCA"]),
                q(BandarPeriod::Week1),
                d(10, 17),
            )
            .await
            .unwrap();
            let r = &got.data[0];
            assert_eq!(got.provider, "ipot");
            assert_eq!(
                (r.from, r.to, r.period.as_str()),
                (d(10, 12), d(10, 16), "1w")
            );
            assert_eq!(r.signal, BandarSignal::BigDistribution);
            assert_eq!(r.price, Some(5_175.0));
            let chg = r.price_change_pct.unwrap();
            assert!(
                (chg - (5_175.0 / 6_050.0 - 1.0) * 100.0).abs() < 1e-9,
                "{chg}"
            );
            assert_eq!(r.phase, Some(BandarPhase::Markdown));
            assert_eq!(
                r.buyers[0].name.as_deref(),
                Some("CLSA SEKURITAS INDONESIA")
            );
            assert!(r.daily.is_none());
            // The week page says -4.1 T (the 16th counted twice); the five
            // sessions one by one give -1.82 T, -18.4% of the 9.9 T traded.
            assert_eq!(r.foreign_net_value, Some(-1_823_100_000_000.0));
            let fpct = r.foreign_net_pct.unwrap();
            assert!((fpct - -1_823.1 / 9_900.0 * 100.0).abs() < 1e-9, "{fpct}");
            assert!(
                r.reasons
                    .iter()
                    .any(|x| x == "Foreign investors net sold 1.82T (-18.4% of value)."),
                "{:?}",
                r.reasons
            );

            // One session: the quote's previous close is the base.
            let got = super::super::bandar(
                &Prices,
                &f,
                &[],
                &syms(&["BBCA"]),
                q(BandarPeriod::Day1),
                d(10, 17),
            )
            .await
            .unwrap();
            let r = &got.data[0];
            assert_eq!((r.from, r.to), (d(10, 16), d(10, 16)));
            assert!((r.price_change_pct.unwrap() - -14.814814).abs() < 1e-4);
        }

        #[tokio::test]
        async fn the_daily_series_and_explicit_ranges() {
            let (f, be, _d) = fetcher(days());
            let mut query = q(BandarPeriod::Month3);
            query.from = Some(d(10, 12));
            query.to = Some(d(10, 16));
            query.daily = true;
            let got = super::super::bandar(&Prices, &f, &[], &syms(&["BBCA"]), query, d(10, 17))
                .await
                .unwrap();
            let r = &got.data[0];
            assert_eq!(r.period, "custom");
            let daily = r.daily.as_ref().unwrap();
            assert_eq!(daily.days.len(), 5);
            assert_eq!(daily.days[4].date, d(10, 16));
            assert!(daily.streak < 0);
            assert_eq!(
                be.call_count(),
                7,
                "the range, the 16th, the 15th and the rest for the foreign net, then the three other sessions"
            );
            assert_eq!(r.foreign_net_value, Some(-1_823_100_000_000.0));
            // Base for a range that starts on the 12th is the close of the 9th.
            assert!((r.price_change_pct.unwrap() - (5_175.0 / 6_050.0 - 1.0) * 100.0).abs() < 1e-9);
        }

        #[tokio::test]
        async fn usage_errors_cost_no_request() {
            let (f, be, _d) = fetcher(FixtureBackend::new());
            let mut bad = q(BandarPeriod::Week1);
            bad.top = 4;
            let err = super::super::bandar(&Prices, &f, &[], &syms(&["BBCA"]), bad, d(10, 17))
                .await
                .unwrap_err();
            assert_eq!(err.exit_code(), 2);
            let mut daily = q(BandarPeriod::Week1);
            daily.daily = true;
            let err = super::super::bandar(
                &Prices,
                &f,
                &[],
                &syms(&["BBCA", "BBRI", "BMRI", "BBNI"]),
                daily,
                d(10, 17),
            )
            .await
            .unwrap_err();
            assert!(err.to_string().contains("daily series"), "{err}");
            assert_eq!(be.call_count(), 0);
        }

        #[tokio::test]
        async fn the_screen_keeps_big_distribution_heaviest_first() {
            let (f, be, _d) = fetcher(others(days()));
            let opts = BandarScreenOptions {
                period: BandarPeriod::Week1,
                top: 5,
                preset: Some(Preset::BigDistribution),
                filters: bd::parse_filters("turnover>1B").unwrap(),
                sort: None,
                descending: None,
                limit: 0,
                scan: 100,
                symbols: syms(&["BBCA", "BRPT", "TLKM", "ZZZZ"]),
            };
            let got = bandar_screen(&Prices, &f, &[], &opts, d(10, 17))
                .await
                .unwrap();
            let s = &got.data;
            assert_eq!((s.from, s.to), (d(10, 12), d(10, 16)));
            assert_eq!(s.filters, ["acc<=-15", "turnover>1B"]);
            assert_eq!((s.scanned, s.checked), (4, 3));
            assert!(!s.descending);
            let order: Vec<&str> = s.rows.iter().map(|r| r.symbol.as_str()).collect();
            assert_eq!(order, ["BBCA", "TLKM"]);
            assert_eq!(s.rows[0].phase, Some(BandarPhase::Markdown));
            // Rebuilt from single sessions: TLKM -96.6 B - 17.9 B - 90.2 B
            // where its week page says -319.8 B.
            assert_eq!(s.rows[0].foreign_net_value, Some(-1_823_100_000_000.0));
            assert_eq!(s.rows[1].foreign_net_value, Some(-204_700_000_000.0));
            assert!(
                !got.warnings.iter().any(|w| w.contains("no foreign net")),
                "{:?}",
                got.warnings
            );
            assert!(
                got.warnings
                    .iter()
                    .any(|w| w == "1 of 4 scanned tickers had no broker summary"),
                "{:?}",
                got.warnings
            );
            // ZZZZ failed on the network: retried once more after the first pass.
            let zzzz = be.calls_matching("code=ZZZZ");
            assert!(zzzz >= 2, "{zzzz}");

            let err = bandar_screen(
                &Prices,
                &f,
                &[],
                &BandarScreenOptions { top: 2, ..opts },
                d(10, 17),
            )
            .await
            .unwrap_err();
            assert_eq!(err.exit_code(), 2);
        }

        #[tokio::test]
        async fn broker_activity_finds_one_member_across_the_scan() {
            let (f, _be, _d) = fetcher(others(days()));
            let opts = BrokerActivityOptions {
                code: " yp ".into(),
                period: BandarPeriod::Week1,
                scan: 100,
                symbols: syms(&["BBCA", "BRPT", "TLKM"]),
                limit: 0,
            };
            let got = bandar_broker(&Prices, &f, &[], &opts, d(10, 18))
                .await
                .unwrap();
            assert_eq!(got.provider, "ipot");
            let a = &got.data;
            assert_eq!(a.code, "YP");
            assert_eq!(a.name.as_deref(), Some("MIRAE ASSET SEKURITAS INDONESIA"));
            assert_eq!((a.from, a.to), (d(10, 12), d(10, 16)));
            assert_eq!((a.scanned, a.checked, a.failed), (3, 3, 0));
            // YP: a top-ten buyer of BBCA and TLKM, a top-ten seller of BRPT and TLKM.
            let mut bought: Vec<&str> = a.bought.iter().map(|r| r.symbol.as_str()).collect();
            bought.sort();
            assert_eq!(bought, ["BBCA", "TLKM"]);
            let mut sold: Vec<&str> = a.sold.iter().map(|r| r.symbol.as_str()).collect();
            sold.sort();
            assert_eq!(sold, ["BRPT", "TLKM"]);
            let v = |r: &BrokerStockRow| r.value.unwrap();
            assert!(a.bought.windows(2).all(|w| v(&w[0]) >= v(&w[1])));
            let bbca = a
                .bought
                .iter()
                .find(|r| r.symbol.as_str() == "BBCA")
                .unwrap();
            assert_eq!(bbca.rank, 9);
            assert_eq!((bbca.opposite_value, bbca.net_value), (None, None));
            assert!(bbca.share_pct.unwrap() > 0.0 && bbca.share_pct.unwrap() < 100.0);
            let tb = a
                .bought
                .iter()
                .find(|r| r.symbol.as_str() == "TLKM")
                .unwrap();
            let ts = a.sold.iter().find(|r| r.symbol.as_str() == "TLKM").unwrap();
            assert_eq!(tb.opposite_value, ts.value);
            assert_eq!(
                tb.net_value, ts.net_value,
                "net is buy minus sell on both sides"
            );
            assert_eq!(tb.net_value, Some(v(tb) - v(ts)));
            let total: f64 = a.bought.iter().map(v).sum();
            assert!((a.bought_value - total).abs() < 1e-6);

            let bad = BrokerActivityOptions {
                code: "MIRAE".into(),
                ..opts.clone()
            };
            assert_eq!(
                bandar_broker(&Prices, &f, &[], &bad, d(10, 18))
                    .await
                    .unwrap_err()
                    .exit_code(),
                2
            );
            let limited = BrokerActivityOptions { limit: 1, ..opts };
            let got = bandar_broker(&Prices, &f, &[], &limited, d(10, 18))
                .await
                .unwrap();
            assert_eq!((got.data.bought.len(), got.data.sold.len()), (1, 1));
            assert!(
                (got.data.bought_value - total).abs() < 1e-6,
                "sums cover every row"
            );
        }
    }
}
