//! Pure parsers for MSN Money responses. Fixtures: `tests/fixtures/msn/`.
//!
//! Shapes worth knowing:
//! - Autosuggest returns `data.stocks[]` as JSON *strings*, each with
//!   `RT00S` (ticker), `ExMicCode` (exchange MIC) and `SecId` (MSN id).
//! - `Quotes?ids=a` returns `[Quote]`; `Quotes?ids=a,b` returns `[[Quote],[Quote]]`.
//!   Unknown ids are dropped silently.
//! - `Equities?ids=a` returns `[Equity]` with `company`, `analysis.keyMetrics`,
//!   `analysis.companyMetrics`, `analysis.estimate`, `analysis.shareStatistics`.
//!   Percentages are already percent (21.8), ratios are ratios (0.0017).

use chrono::{DateTime, NaiveDate};
use serde_json::Value;

use crate::error::Result;
use crate::model::{
    wib, Analyst, Director, Dividend, Fundamentals, Growth, Health, PerShare, Profile,
    Profitability, Quote, Symbol, Valuation,
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
    const QW: &str = include_str!("../../../tests/fixtures/msn/quotes_bbca_wika_kaef.json");
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
    fn a_suspended_stock_has_no_last_trade_so_no_session_flag() {
        let q = parse_quotes(QW).unwrap();
        let by = |s: &str| q.iter().find(|x| x.symbol.as_str() == s).unwrap();
        let wika = by("WIKA");
        assert_eq!((wika.price, wika.market_time), (None, None));
        // Recorded before the 2026-11-06 open: the latest session is the 5th.
        let fifth = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let sixth = fifth.succ_opt().unwrap();
        assert_eq!(wika.behind(fifth), None, "unknown, not false");
        assert_eq!(by("BBCA").behind(fifth), Some(false));
        assert_eq!(
            by("KAEF").market_time.unwrap().to_rfc3339(),
            "2026-11-05T16:15:00+07:00"
        );
        // The same row read after the 6th's close is behind.
        assert_eq!(by("KAEF").behind(sixth), Some(true));
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
}
