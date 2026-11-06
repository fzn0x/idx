//! Pure parsers for MSN Money responses. Fixtures: `tests/fixtures/msn/`.
//!
//! Shapes worth knowing:
//! - Autosuggest returns `data.stocks[]` as JSON *strings*, each with
//!   `RT00S` (ticker), `ExMicCode` (exchange MIC) and `SecId` (MSN id).
//! - `Quotes?ids=a` returns `[Quote]`; `Quotes?ids=a,b` returns `[[Quote],[Quote]]`.
//!   Unknown ids are dropped silently.
//! - `Equities?ids=a` returns `[Equity]` with `company`, `analysis.keyMetrics`,
//!   `analysis.companyMetrics`, `analysis.estimate`, `analysis.shareStatistics`
//!   and `ownershipSummary` (institutions, funds).
//!   Percentages are already percent (21.8), ratios are ratios (0.0017).

use chrono::{DateTime, NaiveDate};
use serde_json::Value;

use crate::error::Result;
use crate::model::institutions::signal;
use crate::model::{
    wib, Analyst, Director, Dividend, Fundamentals, Growth, Health, HolderKind,
    InstitutionalHolder, InstitutionalOwnership, PerShare, Profile, Profitability, Quote, Symbol,
    Valuation,
};

pub const SOURCE_QUOTES: &str = "msn:quotes";
pub const SOURCE_EQUITIES: &str = "msn:equities";
/// Indonesia Stock Exchange MIC.
pub const IDX_MIC: &str = "XIDX";

fn f64_of(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|x| x.is_finite())
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn date(v: &Value, key: &str) -> Option<NaiveDate> {
    text(v, key).and_then(|s| {
        DateTime::parse_from_rfc3339(&s)
            .map(|d| d.date_naive())
            .ok()
            .or_else(|| NaiveDate::parse_from_str(&s[..s.len().min(10)], "%Y-%m-%d").ok())
    })
}

fn obj<'a>(v: &'a Value, path: &str) -> &'a Value {
    static NULL: Value = Value::Null;
    v.pointer(path).unwrap_or(&NULL)
}

/// MSN security id for `symbol` on the Indonesia Stock Exchange, if listed.
pub fn parse_autosuggest(body: &str, symbol: &Symbol) -> Result<Option<String>> {
    let v: Value = serde_json::from_str(body)?;
    let Some(stocks) = v.pointer("/data/stocks").and_then(Value::as_array) else {
        return Ok(None);
    };
    for s in stocks {
        let entry: Value = match s {
            Value::String(raw) => match serde_json::from_str(raw) {
                Ok(e) => e,
                Err(_) => continue,
            },
            other => other.clone(),
        };
        let ticker = text(&entry, "RT00S").or_else(|| text(&entry, "OS001"));
        let mic = text(&entry, "ExMicCode").or_else(|| text(&entry, "LS01Z"));
        if ticker.as_deref() == Some(symbol.as_str()) && mic.as_deref() == Some(IDX_MIC) {
            if let Some(id) = text(&entry, "SecId") {
                return Ok(Some(id));
            }
        }
    }
    Ok(None)
}

fn flatten(v: Value) -> Vec<Value> {
    match v {
        Value::Array(items) => items.into_iter().flat_map(flatten).collect(),
        Value::Object(_) => vec![v],
        _ => vec![],
    }
}

/// Quotes in the order MSN returned them (one per known id).
pub fn parse_quotes(body: &str) -> Result<Vec<Quote>> {
    let v: Value = serde_json::from_str(body)?;
    Ok(flatten(v)
        .iter()
        .filter_map(|q| {
            let symbol = Symbol::parse(&text(q, "symbol")?).ok()?;
            let mut out = Quote::empty(symbol, SOURCE_QUOTES);
            out.name = text(q, "displayName");
            out.currency = text(q, "currency");
            out.price = f64_of(q, "price");
            out.change = f64_of(q, "priceChange");
            out.change_pct = f64_of(q, "priceChangePercent");
            out.previous_close = f64_of(q, "pricePreviousClose");
            out.open = f64_of(q, "priceDayOpen");
            out.day_high = f64_of(q, "priceDayHigh");
            out.day_low = f64_of(q, "priceDayLow");
            out.volume = f64_of(q, "accumulatedVolume").map(|x| x.max(0.0) as u64);
            out.market_cap = f64_of(q, "marketCap");
            out.week52_high = f64_of(q, "price52wHigh");
            out.week52_low = f64_of(q, "price52wLow");
            out.market_time = text(q, "timeLastTraded")
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&wib()));
            Some(out)
        })
        .collect())
}

fn first_equity(body: &str) -> Result<Option<Value>> {
    let v: Value = serde_json::from_str(body)?;
    Ok(flatten(v).into_iter().next())
}

fn display_name(e: &Value) -> Option<String> {
    text(obj(e, "/localizedAttributes/en-us"), "displayName")
        .or_else(|| text(e, "displayName"))
        .or_else(|| text(obj(e, "/company"), "name"))
}

/// Profile from an `Equities` body; `quote` supplies market cap when given.
pub fn parse_profile(
    body: &str,
    symbol: &Symbol,
    quote: Option<&Quote>,
) -> Result<Option<Profile>> {
    let Some(e) = first_equity(body)? else {
        return Ok(None);
    };
    let c = obj(&e, "/company");
    let ss = obj(&e, "/analysis/shareStatistics");
    let mut p = Profile::empty(symbol.clone(), SOURCE_EQUITIES);
    p.name = display_name(&e);
    p.sector = text(c, "sector");
    p.industry = text(c, "industry");
    p.summary = text(obj(&e, "/localizedAttributes/en-us"), "description")
        .or_else(|| text(c, "description"));
    p.employees = f64_of(c, "employees").map(|x| x.max(0.0) as u64);
    p.founded_year = f64_of(c, "establishedYear").map(|x| x as u32);
    p.website = text(c, "website");
    p.city = text(obj(c, "/address"), "city");
    p.country = text(obj(c, "/address"), "country");
    p.exchange = text(&e, "exchangeName");
    p.currency = text(&e, "currency");
    p.market_cap = quote.and_then(|q| q.market_cap);
    p.shares_outstanding = f64_of(ss, "sharesOutstanding").map(|x| x.max(0.0) as u64);
    p.directors = c
        .get("directors")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|d| {
                    Some(Director {
                        name: text(d, "name")?,
                        title: text(d, "title"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(p))
}

/// `strongBuy` -> `strong_buy`, matching Yahoo's vocabulary.
fn snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Fundamentals from an `Equities` body plus (optionally) a quote for price,
/// market cap and the 52-week range.
pub fn parse_fundamentals(
    body: &str,
    symbol: &Symbol,
    quote: Option<&Quote>,
) -> Result<Option<Fundamentals>> {
    let Some(e) = first_equity(body)? else {
        return Ok(None);
    };
    let c = obj(&e, "/company");
    let km = obj(&e, "/analysis/keyMetrics");
    let cm = obj(&e, "/analysis/companyMetrics");
    let est = obj(&e, "/analysis/estimate");
    let ss = obj(&e, "/analysis/shareStatistics");

    let mut f = Fundamentals::empty(symbol.clone(), SOURCE_EQUITIES);
    f.name = display_name(&e);
    f.currency = text(&e, "currency");
    f.sector = text(c, "sector");
    f.industry = text(c, "industry");
    f.beta = f64_of(&e, "beta");
    if let Some(q) = quote {
        f.price = q.price;
        f.week52_high = q.week52_high;
        f.week52_low = q.week52_low;
        f.sources.push(SOURCE_QUOTES.to_string());
    }
    let eps = f64_of(km, "eps");
    f.valuation = Valuation {
        market_cap: quote.and_then(|q| q.market_cap),
        enterprise_value: f64_of(ss, "enterpriseValue"),
        // MSN publishes no trailing P/E; price over trailing EPS is the same number.
        pe_trailing: match (f.price, eps) {
            (Some(p), Some(e)) if e > 0.0 => Some(p / e),
            _ => None,
        },
        pe_forward: f64_of(km, "forwardPriceToEPS"),
        peg: f64_of(cm, "pEGrowthRatio"),
        price_to_book: f64_of(km, "priceToBookRatio"),
        price_to_sales: f64_of(cm, "priceSalesRatio"),
        ev_to_ebitda: None,
        ev_to_revenue: None,
    };
    f.profitability = Profitability {
        roe_pct: f64_of(km, "returnOnEquity"),
        roa_pct: f64_of(km, "returnOnAssets"),
        gross_margin_pct: None,
        operating_margin_pct: f64_of(cm, "operatingMargin"),
        net_margin_pct: f64_of(cm, "netProfitMargin")
            .or_else(|| f64_of(km, "latestNetProfitMargin")),
    };
    f.health = Health {
        debt_to_equity: f64_of(km, "debtToEquityRatio").or_else(|| f64_of(cm, "debtEquityRatio")),
        current_ratio: None,
        quick_ratio: None,
        total_debt: None,
        total_cash: None,
    };
    f.growth = Growth {
        revenue_growth_pct: f64_of(cm, "revenueQQLastYearGrowthRate"),
        earnings_growth_pct: f64_of(cm, "netIncomeQQLastYearGrowthRate"),
        revenue_cagr_5y_pct: f64_of(cm, "revenue5YearAverageGrowthRate"),
        eps_growth_3y_pct: f64_of(cm, "dilutedEPS3YearGrowth"),
    };
    f.dividend = Dividend {
        yield_pct: f64_of(ss, "dividendYield")
            .or_else(|| f64_of(cm, "trailingAnnualDividendYield")),
        payout_ratio_pct: f64_of(km, "payoutRatio"),
        ex_dividend_date: date(ss, "exDividendDate"),
    };
    f.per_share = PerShare {
        eps_ttm: eps,
        eps_forward: None,
        book_value: f64_of(km, "bookValuePerShare"),
        revenue: f64_of(km, "latestRevenuePerShare"),
    };
    f.analyst = Analyst {
        recommendation: text(est, "recommendation").map(|s| snake(&s)),
        target_mean: f64_of(est, "meanPriceTarget"),
        target_high: f64_of(est, "highPriceTarget"),
        target_low: f64_of(est, "lowPriceTarget"),
        analysts: f64_of(est, "numberOfPriceTargets")
            .or_else(|| f64_of(est, "numberOfAnalysts"))
            .map(|x| x.max(0.0) as u32),
    };
    Ok(Some(f))
}

/// One `institutionalOwnership` / `mutualFundOwnership` row. Rows without a
/// holder name are skipped. `marketValue` and `change` are deliberately not
/// read: they are holder-level totals in an unstated currency, not the value
/// of this position.
fn holder(h: &Value, kind: HolderKind, index: usize) -> Option<InstitutionalHolder> {
    let name = text(obj(h, "/entity"), "displayName")?;
    let shares = f64_of(h, "shares").map(|x| x.max(0.0) as u64);
    let shares_change = f64_of(h, "sharesChange").map(|x| x as i64);
    Some(InstitutionalHolder {
        kind,
        rank: f64_of(h, "rank")
            .filter(|r| *r >= 1.0)
            .map(|r| r as u32)
            .unwrap_or(index as u32 + 1),
        name,
        shares,
        shares_change,
        pct_outstanding: f64_of(h, "outstandingPercent"),
        pct_outstanding_change: f64_of(h, "changePercent"),
        pct_portfolio: match kind {
            HolderKind::Fund => f64_of(h, "percentPortfolio"),
            HolderKind::Institution => None,
        },
        report_date: date(h, "reportDate"),
        signal: signal(shares, shares_change),
    })
}

/// Institutional and fund holders from an `Equities` body's `ownershipSummary`.
/// `None` when the body holds no equity (404 `[]`); an equity without the
/// block gives an empty report. `insiderOwnershipPercent` is not read: it
/// contradicts the KSEI register (1.21 % for BBCA, whose controlling holder
/// has 54.94 %).
pub fn parse_institutions(body: &str, symbol: &Symbol) -> Result<Option<InstitutionalOwnership>> {
    let Some(e) = first_equity(body)? else {
        return Ok(None);
    };
    let o = obj(&e, "/ownershipSummary");
    let mut r = InstitutionalOwnership::empty(symbol.clone());
    r.name = display_name(&e);
    r.institutions_pct = f64_of(o, "institutionalOwnershipPercent");
    r.top10_institutions_pct = f64_of(o, "top10InstitutesPercent");
    r.funds_pct = f64_of(o, "mutualFundOwnershipPercent");
    r.float_pct = f64_of(o, "floatOwnershipPercent");
    for (key, kind) in [
        ("institutionalOwnership", HolderKind::Institution),
        ("mutualFundOwnership", HolderKind::Fund),
    ] {
        if let Some(rows) = o.get(key).and_then(Value::as_array) {
            r.holders.extend(
                rows.iter()
                    .enumerate()
                    .filter_map(|(i, h)| holder(h, kind, i)),
            );
        }
    }
    r.summarise();
    Ok(Some(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AS_BBCA: &str = include_str!("../../../tests/fixtures/msn/autosuggest_bbca.json");
    const AS_GOTO: &str =
        include_str!("../../../tests/fixtures/msn/autosuggest_goto_ambiguous.json");
    const AS_ZZZZ: &str =
        include_str!("../../../tests/fixtures/msn/autosuggest_zzzz_no_idx_match.json");
    const Q1: &str = include_str!("../../../tests/fixtures/msn/quotes_bbca.json");
    const Q3: &str = include_str!("../../../tests/fixtures/msn/quotes_bbca_bbri_tlkm.json");
    const QB: &str = include_str!("../../../tests/fixtures/msn/quotes_bbca_with_bogus_ids.json");
    const EQ_BBCA: &str = include_str!("../../../tests/fixtures/msn/equities_bbca.json");
    const EQ_TLKM: &str = include_str!("../../../tests/fixtures/msn/equities_tlkm.json");
    const EQ_404: &str = include_str!("../../../tests/fixtures/msn/equities_bogus_404.json");

    fn sym(s: &str) -> Symbol {
        Symbol::parse(s).unwrap()
    }

    #[test]
    fn autosuggest_picks_the_idx_listing() {
        assert_eq!(
            parse_autosuggest(AS_BBCA, &sym("BBCA")).unwrap().as_deref(),
            Some("bn91jc")
        );
        assert_eq!(
            parse_autosuggest(AS_GOTO, &sym("GOTO")).unwrap().as_deref(),
            Some("c5tv8m"),
            "XIDX row is not first for GOTO"
        );
        assert_eq!(parse_autosuggest(AS_ZZZZ, &sym("ZZZZ")).unwrap(), None);
    }

    #[test]
    fn quotes_single_and_nested() {
        let q = parse_quotes(Q1).unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].symbol.as_str(), "BBCA");
        assert_eq!(q[0].price, Some(5175.0));
        assert_eq!(q[0].previous_close, Some(6075.0));
        assert_eq!(q[0].change, Some(-900.0));
        assert!((q[0].change_pct.unwrap() + 14.8148).abs() < 1e-6);
        assert_eq!(q[0].volume, Some(1_241_098_200));
        assert_eq!(q[0].market_cap, Some(741_407_000_000_000.0));
        assert_eq!(q[0].week52_high, Some(8750.0));
        assert_eq!(
            q[0].market_time.unwrap().to_rfc3339(),
            "2026-10-16T16:15:00+07:00"
        );
        assert_eq!(q[0].source, "msn:quotes");
        assert_eq!(q[0].pe_trailing, None);

        let q = parse_quotes(Q3).unwrap();
        let syms: Vec<&str> = q.iter().map(|q| q.symbol.as_str()).collect();
        assert_eq!(syms, ["BBCA", "BBRI", "TLKM"]);

        let q = parse_quotes(QB).unwrap();
        assert_eq!(q.len(), 1, "bogus ids are dropped by MSN");
    }

    #[test]
    fn profile_bbca() {
        let quote = &parse_quotes(Q1).unwrap()[0];
        let p = parse_profile(EQ_BBCA, &sym("BBCA"), Some(quote))
            .unwrap()
            .unwrap();
        assert_eq!(p.name.as_deref(), Some("PT Bank Central Asia Tbk"));
        assert_eq!(p.sector.as_deref(), Some("Financials"));
        assert_eq!(p.industry.as_deref(), Some("Banking Services"));
        assert_eq!(p.employees, Some(27728));
        assert_eq!(p.founded_year, Some(1955));
        assert_eq!(p.city.as_deref(), Some("JAKARTA PUSAT"));
        assert_eq!(p.market_cap, Some(741_407_000_000_000.0));
        assert_eq!(p.shares_outstanding, Some(122_042_300_000));
        assert_eq!(p.directors.len(), 15);
        assert_eq!(p.directors[0].name, "Jahja Setiaatmadja");
        assert_eq!(p.source, "msn:equities");
        assert!(parse_profile(EQ_404, &sym("ZZZZ"), None).unwrap().is_none());
    }

    #[test]
    fn fundamentals_bbca_and_tlkm() {
        let quote = &parse_quotes(Q1).unwrap()[0];
        let f = parse_fundamentals(EQ_BBCA, &sym("BBCA"), Some(quote))
            .unwrap()
            .unwrap();
        assert_eq!(f.price, Some(5175.0));
        assert!((f.valuation.pe_trailing.unwrap() - 5175.0 / 471.8202).abs() < 1e-6);
        assert_eq!(f.valuation.pe_forward, Some(11.9878));
        assert_eq!(f.valuation.price_to_book, Some(2.66));
        assert_eq!(f.valuation.enterprise_value, Some(697_562_000_000_000.0));
        assert_eq!(f.profitability.roe_pct, Some(21.824));
        assert!((f.profitability.net_margin_pct.unwrap() - 51.3927).abs() < 1e-6);
        assert_eq!(f.health.debt_to_equity, Some(0.0017));
        assert_eq!(f.dividend.yield_pct, Some(6.2716));
        assert_eq!(f.dividend.payout_ratio_pct, Some(80.7399));
        assert_eq!(
            f.dividend.ex_dividend_date,
            Some(NaiveDate::from_ymd_opt(2026, 8, 31).unwrap())
        );
        assert_eq!(f.growth.revenue_cagr_5y_pct, Some(8.6252));
        assert_eq!(f.analyst.recommendation.as_deref(), Some("strong_buy"));
        assert_eq!(f.analyst.target_mean, Some(8193.38));
        assert_eq!(f.analyst.analysts, Some(24));
        assert_eq!(
            f.sources,
            vec!["msn:equities".to_string(), "msn:quotes".to_string()]
        );

        let t = parse_fundamentals(EQ_TLKM, &sym("TLKM"), None)
            .unwrap()
            .unwrap();
        assert_eq!(t.price, None);
        assert_eq!(t.valuation.pe_trailing, None, "no price, no derived P/E");
        assert_eq!(t.profitability.operating_margin_pct, Some(23.313));
        assert_eq!(t.health.debt_to_equity, Some(0.3882));
        assert_eq!(t.sector.as_deref(), Some("Technology"));
    }

    #[test]
    fn snake_case() {
        assert_eq!(snake("strongBuy"), "strong_buy");
        assert_eq!(snake("hold"), "hold");
    }

    const EQ_JTPE: &str =
        include_str!("../../../tests/fixtures/msn/equities_jtpe_all_signals.json");
    const EQ_STAR: &str =
        include_str!("../../../tests/fixtures/msn/equities_star_no_ownership.json");
    const EQ_ATAP: &str =
        include_str!("../../../tests/fixtures/msn/equities_atap_empty_holders.json");

    fn day(s: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
    }

    #[test]
    fn institutions_bbca() {
        use crate::model::{HolderFlow, HolderSignal};
        let r = parse_institutions(EQ_BBCA, &sym("BBCA")).unwrap().unwrap();
        assert_eq!(r.symbol.as_str(), "BBCA");
        assert_eq!(r.name.as_deref(), Some("PT Bank Central Asia Tbk"));
        assert_eq!(r.institutions_pct, Some(19.33));
        assert_eq!(r.top10_institutions_pct, Some(9.24));
        assert_eq!(r.funds_pct, Some(16.71));
        assert_eq!(r.float_pct, Some(38.2045));
        assert_eq!(r.holders.len(), 35);
        let kinds: Vec<HolderKind> = r.holders.iter().map(|h| h.kind).collect();
        assert!(kinds[..15].iter().all(|k| *k == HolderKind::Institution));
        assert!(kinds[15..].iter().all(|k| *k == HolderKind::Fund));
        let ranks: Vec<u32> = r.of_kind(HolderKind::Fund).map(|h| h.rank).collect();
        assert_eq!(ranks, (1..=20).collect::<Vec<u32>>());

        let h = &r.holders[0];
        assert_eq!(h.rank, 1);
        assert_eq!(h.name, "Fidelity Management & Research Company LLC");
        assert_eq!(h.shares, Some(2_550_310_952));
        assert_eq!(h.shares_change, Some(20_110_296));
        assert_eq!(h.pct_outstanding, Some(2.09));
        assert_eq!(h.pct_outstanding_change, Some(0.0165));
        assert_eq!(
            h.pct_portfolio, None,
            "institutions carry no portfolio weight"
        );
        assert_eq!(h.report_date, day("2026-09-30"));
        assert_eq!(h.signal, Some(HolderSignal::Buyer));
        let f = &r.holders[15];
        assert_eq!(f.name, "Statens Pensjonsfond Utland");
        assert_eq!(f.pct_portfolio, Some(0.0211));
        assert_eq!(f.report_date, day("2026-06-30"));

        assert_eq!(
            r.flow.institutions,
            HolderFlow {
                new: 1,
                buyers: 10,
                sellers: 4,
                exited: 0,
                unchanged: 0,
                net_shares_change: 142_132_958,
            }
        );
        assert_eq!(
            r.flow.funds,
            HolderFlow {
                new: 1,
                buyers: 11,
                sellers: 3,
                exited: 0,
                unchanged: 5,
                net_shares_change: 739_658_106,
            }
        );
        assert_eq!(r.oldest_report, day("2025-12-31"));
        assert_eq!(r.newest_report, day("2026-09-30"));

        // MSN's holder-level totals and insider share never reach the output.
        let v = serde_json::to_value(&r).unwrap();
        let keys: Vec<&str> = v["holders"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "kind",
                "rank",
                "name",
                "shares",
                "shares_change",
                "pct_outstanding",
                "pct_outstanding_change",
                "pct_portfolio",
                "report_date",
                "signal"
            ]
        );
        let top: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            top,
            [
                "symbol",
                "name",
                "institutions_pct",
                "top10_institutions_pct",
                "funds_pct",
                "float_pct",
                "oldest_report",
                "newest_report",
                "flow",
                "holders"
            ]
        );
    }

    #[test]
    fn institutions_jtpe_has_every_signal() {
        use crate::model::HolderSignal::*;
        let r = parse_institutions(EQ_JTPE, &sym("JTPE")).unwrap().unwrap();
        let signals = |k| r.of_kind(k).map(|h| h.signal.unwrap()).collect::<Vec<_>>();
        assert_eq!(
            signals(HolderKind::Institution),
            [Unchanged, Seller, Buyer, Exited]
        );
        assert_eq!(
            signals(HolderKind::Fund),
            [Unchanged, Seller, New, Exited, Exited]
        );
        let niche = &r.holders[3];
        assert_eq!(niche.name, "Niche Asset Management Limited");
        assert_eq!(
            (niche.shares, niche.shares_change),
            (Some(0), Some(-3_434_200))
        );
        assert_eq!(niche.report_date, day("2026-03-31"));
        assert_eq!(r.flow.institutions.net_shares_change, -3_556_219);
        assert_eq!(r.flow.funds.net_shares_change, -3_556_219);
        assert_eq!(r.flow.funds.exited, 2);
        assert_eq!(r.flow.funds.new, 1);
        assert_eq!(r.institutions_pct, Some(0.11));
        assert_eq!(r.float_pct, Some(6.0732));
        assert_eq!(r.oldest_report, day("2025-12-31"));
        assert_eq!(r.newest_report, day("2026-09-30"));
    }

    #[test]
    fn institutions_tlkm() {
        let r = parse_institutions(EQ_TLKM, &sym("TLKM")).unwrap().unwrap();
        let i = &r.flow.institutions;
        assert_eq!(
            (i.new, i.buyers, i.sellers, i.exited, i.unchanged),
            (0, 8, 6, 0, 1)
        );
        let f = &r.flow.funds;
        assert_eq!(
            (f.new, f.buyers, f.sellers, f.exited, f.unchanged),
            (1, 7, 8, 0, 4)
        );
        assert_eq!(r.holders[0].name, "Employees Provident Fund Board");
        assert_eq!(r.institutions_pct, Some(21.15));
    }

    #[test]
    fn institutions_without_holders() {
        // No ownershipSummary block at all.
        let star = parse_institutions(EQ_STAR, &sym("STAR")).unwrap().unwrap();
        assert!(star.holders.is_empty());
        assert_eq!(
            star.name.as_deref(),
            Some("PT Calculus Global Ventures Tbk")
        );
        assert_eq!(
            (star.institutions_pct, star.funds_pct, star.float_pct),
            (None, None, None)
        );
        assert_eq!(star.oldest_report, None);
        assert_eq!(star.flow, Default::default());
        // A block with empty lists keeps the figures it has.
        let atap = parse_institutions(EQ_ATAP, &sym("ATAP")).unwrap().unwrap();
        assert!(atap.holders.is_empty());
        assert_eq!(atap.float_pct, Some(15.0635));
        assert_eq!(atap.institutions_pct, None);
        // Unknown id: MSN answers 404 with `[]`.
        assert!(parse_institutions(EQ_404, &sym("ZZZZ")).unwrap().is_none());
    }
}
