//! Parsers for `v10/finance/quoteSummary` bodies: profile, fundamentals and
//! earnings. Yahoo wraps most numbers as `{"raw": 1.23, "fmt": "1.23"}`; the
//! helpers below read `raw` and treat an empty object as missing.
//!
//! Fixtures: `tests/fixtures/yahoo/quote_summary_*.json`.

use chrono::NaiveDate;
use serde_json::Value;

use crate::error::{IdxError, Result};
use crate::model::{
    Analyst, Director, Dividend, Earnings, EarningsQuarter, Fundamentals, Growth, Health,
    NextEarnings, PerShare, Profile, Profitability, RevenueEarnings, Symbol, Valuation,
};

use super::NAME;

pub const SOURCE: &str = "yahoo:quoteSummary";

/// Modules for `profile` and `fundamental` (one request serves both).
pub const FUNDAMENTAL_MODULES: &str =
    "assetProfile,summaryDetail,defaultKeyStatistics,financialData,price";
/// Modules for `earnings`.
pub const EARNINGS_MODULES: &str = "earnings,earningsHistory,earningsTrend,calendarEvents,price";

fn raw(v: &Value, key: &str) -> Option<f64> {
    let x = v.get(key)?;
    match x {
        Value::Number(n) => n.as_f64(),
        Value::Object(_) => x.get("raw").and_then(Value::as_f64),
        _ => None,
    }
}

fn pct(v: &Value, key: &str) -> Option<f64> {
    raw(v, key).map(|x| x * 100.0)
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn date(v: &Value, key: &str) -> Option<NaiveDate> {
    let secs = raw(v, key)? as i64;
    chrono::DateTime::from_timestamp(secs, 0).map(|d| d.date_naive())
}

/// Unwraps `quoteSummary.result[0]`, mapping Yahoo's error block to our errors.
pub fn result(body: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(body)?;
    if let Some(err) = v.pointer("/quoteSummary/error").filter(|e| !e.is_null()) {
        let code = err.get("code").and_then(Value::as_str).unwrap_or("");
        let desc = err
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("unexpected response");
        return Err(if code.eq_ignore_ascii_case("not found") {
            IdxError::NotFound(desc.to_string())
        } else {
            IdxError::provider(NAME, format!("{code}: {desc}"))
        });
    }
    v.pointer("/quoteSummary/result/0")
        .cloned()
        .ok_or_else(|| IdxError::provider(NAME, "quoteSummary response has no result"))
}

fn module<'a>(r: &'a Value, name: &str) -> &'a Value {
    static NULL: Value = Value::Null;
    r.get(name).unwrap_or(&NULL)
}

pub fn parse_profile(body: &str, symbol: &Symbol) -> Result<Profile> {
    let r = result(body)?;
    let ap = module(&r, "assetProfile");
    let price = module(&r, "price");
    let dks = module(&r, "defaultKeyStatistics");
    let sd = module(&r, "summaryDetail");
    let mut p = Profile::empty(symbol.clone(), SOURCE);
    p.name = text(price, "longName").or_else(|| text(price, "shortName"));
    p.sector = text(ap, "sector");
    p.industry = text(ap, "industry");
    p.summary = text(ap, "longBusinessSummary");
    p.employees = raw(ap, "fullTimeEmployees").map(|x| x.max(0.0) as u64);
    p.website = text(ap, "website");
    p.city = text(ap, "city");
    p.country = text(ap, "country");
    p.exchange = text(price, "exchangeName");
    p.currency = text(price, "currency");
    p.market_cap = raw(price, "marketCap").or_else(|| raw(sd, "marketCap"));
    p.shares_outstanding = raw(dks, "sharesOutstanding").map(|x| x.max(0.0) as u64);
    p.directors = ap
        .get("companyOfficers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    Some(Director {
                        name: text(o, "name")?,
                        title: text(o, "title"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(p)
}

/// Yahoo reports gross and EBITDA margins as exactly `0` for companies without
/// cost of goods sold (banks). That is "not applicable", not a margin.
fn margin_or_none(v: &Value, key: &str) -> Option<f64> {
    pct(v, key).filter(|m| *m != 0.0)
}

pub fn parse_fundamentals(body: &str, symbol: &Symbol) -> Result<Fundamentals> {
    let r = result(body)?;
    let ap = module(&r, "assetProfile");
    let price = module(&r, "price");
    let dks = module(&r, "defaultKeyStatistics");
    let sd = module(&r, "summaryDetail");
    let fd = module(&r, "financialData");

    let mut f = Fundamentals::empty(symbol.clone(), SOURCE);
    f.name = text(price, "longName").or_else(|| text(price, "shortName"));
    f.currency = text(price, "currency").or_else(|| text(fd, "financialCurrency"));
    f.sector = text(ap, "sector");
    f.industry = text(ap, "industry");
    f.price = raw(fd, "currentPrice").or_else(|| raw(price, "regularMarketPrice"));
    f.beta = raw(sd, "beta");
    f.week52_high = raw(sd, "fiftyTwoWeekHigh");
    f.week52_low = raw(sd, "fiftyTwoWeekLow");
    f.valuation = Valuation {
        market_cap: raw(price, "marketCap").or_else(|| raw(sd, "marketCap")),
        enterprise_value: raw(dks, "enterpriseValue"),
        pe_trailing: raw(sd, "trailingPE"),
        pe_forward: raw(sd, "forwardPE"),
        peg: raw(dks, "pegRatio"),
        price_to_book: raw(dks, "priceToBook"),
        price_to_sales: raw(sd, "priceToSalesTrailing12Months"),
        ev_to_ebitda: raw(dks, "enterpriseToEbitda"),
        ev_to_revenue: raw(dks, "enterpriseToRevenue"),
    };
    f.profitability = Profitability {
        roe_pct: pct(fd, "returnOnEquity"),
        roa_pct: pct(fd, "returnOnAssets"),
        gross_margin_pct: margin_or_none(fd, "grossMargins"),
        operating_margin_pct: pct(fd, "operatingMargins"),
        net_margin_pct: pct(fd, "profitMargins"),
    };
    f.health = Health {
        // Yahoo reports debt/equity as a percent (59.98 = 0.5998x).
        debt_to_equity: raw(fd, "debtToEquity").map(|x| x / 100.0),
        current_ratio: raw(fd, "currentRatio"),
        quick_ratio: raw(fd, "quickRatio"),
        total_debt: raw(fd, "totalDebt"),
        total_cash: raw(fd, "totalCash"),
    };
    f.growth = Growth {
        revenue_growth_pct: pct(fd, "revenueGrowth"),
        earnings_growth_pct: pct(fd, "earningsGrowth"),
        revenue_cagr_5y_pct: None,
        eps_growth_3y_pct: None,
    };
    f.dividend = Dividend {
        yield_pct: pct(sd, "dividendYield"),
        payout_ratio_pct: pct(sd, "payoutRatio"),
        ex_dividend_date: date(sd, "exDividendDate"),
    };
    f.per_share = PerShare {
        eps_ttm: raw(dks, "trailingEps"),
        eps_forward: raw(dks, "forwardEps"),
        book_value: raw(dks, "bookValue"),
        revenue: raw(fd, "revenuePerShare"),
    };
    f.analyst = Analyst {
        recommendation: text(fd, "recommendationKey"),
        target_mean: raw(fd, "targetMeanPrice"),
        target_high: raw(fd, "targetHighPrice"),
        target_low: raw(fd, "targetLowPrice"),
        analysts: raw(fd, "numberOfAnalystOpinions").map(|x| x.max(0.0) as u32),
    };
    f.fiscal_year_end = date(dks, "lastFiscalYearEnd");
    f.most_recent_quarter = date(dks, "mostRecentQuarter");
    Ok(f)
}

fn period_label(v: &Value) -> String {
    match v.get("date") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// An amount Yahoo may have padded: values it has not loaded come as
/// `{"raw": 0, "fmt": null}`, while a reported figure carries its formatted
/// form. A padded value is missing, not zero.
fn amount(v: &Value, key: &str) -> Option<f64> {
    let x = v.get(key)?;
    let padded = x.get("fmt").is_some_and(Value::is_null)
        && x.get("raw").and_then(Value::as_f64) == Some(0.0);
    if padded {
        None
    } else {
        raw(v, key)
    }
}

/// Yahoo pads quarters it has not loaded yet with zeros (`{"raw": 0, "fmt":
/// null}` for revenue or earnings); a company with revenue and exactly zero
/// earnings is a gap in the feed, not a result. The margin is kept only when
/// both figures are there.
fn revenue_earnings(arr: Option<&Vec<Value>>) -> Vec<RevenueEarnings> {
    arr.map(|a| {
        a.iter()
            .map(|e| {
                let revenue = amount(e, "revenue");
                let earnings = amount(e, "earnings")
                    .filter(|x| !(*x == 0.0 && revenue.is_some_and(|r| r != 0.0)));
                let profit_margin_pct = match (revenue, earnings) {
                    (Some(r), Some(_)) if r != 0.0 => pct(e, "profitMargin"),
                    _ => None,
                };
                RevenueEarnings {
                    period: period_label(e),
                    revenue,
                    earnings,
                    profit_margin_pct,
                }
            })
            .collect()
    })
    .unwrap_or_default()
}

pub fn parse_earnings(body: &str, symbol: &Symbol) -> Result<Earnings> {
    let r = result(body)?;
    let price = module(&r, "price");
    let e = module(&r, "earnings");
    let cal = module(&r, "calendarEvents");
    let trend = module(&r, "earningsTrend");

    let quarters = e
        .pointer("/earningsChart/quarterly")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|q| EarningsQuarter {
                    period: period_label(q),
                    end_date: date(q, "periodEndDate"),
                    reported_date: date(q, "reportedDate"),
                    eps_actual: raw(q, "actual"),
                    eps_estimate: raw(q, "estimate"),
                    surprise_pct: match q.get("surprisePct") {
                        Some(Value::String(s)) => s.trim().parse().ok(),
                        Some(other) => other.as_f64(),
                        None => None,
                    },
                })
                .collect()
        })
        .unwrap_or_default();

    let cal_e = module(cal, "earnings");
    let next_date = cal_e
        .get("earningsDate")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|d| d.get("raw"))
        .and_then(Value::as_f64)
        .and_then(|s| chrono::DateTime::from_timestamp(s as i64, 0))
        .map(|d| d.date_naive());
    let current_q = trend.get("trend").and_then(Value::as_array).and_then(|a| {
        a.iter()
            .find(|t| t.get("period").and_then(Value::as_str) == Some("0q"))
    });
    let next = if cal_e.is_null() && current_q.is_none() {
        None
    } else {
        Some(NextEarnings {
            date: next_date,
            date_is_estimate: cal_e
                .get("isEarningsDateEstimate")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            eps_avg: raw(cal_e, "earningsAverage"),
            eps_low: raw(cal_e, "earningsLow"),
            eps_high: raw(cal_e, "earningsHigh"),
            eps_growth_pct: current_q.and_then(|t| pct(t, "growth")),
            revenue_avg: raw(cal_e, "revenueAverage"),
            revenue_low: raw(cal_e, "revenueLow"),
            revenue_high: raw(cal_e, "revenueHigh"),
            analysts: current_q
                .and_then(|t| raw(module(t, "earningsEstimate"), "numberOfAnalysts"))
                .map(|x| x.max(0.0) as u32),
        })
    };

    Ok(Earnings {
        symbol: symbol.clone(),
        name: text(price, "longName").or_else(|| text(price, "shortName")),
        currency: text(e, "financialCurrency").or_else(|| text(price, "currency")),
        quarters,
        yearly: revenue_earnings(
            e.pointer("/financialsChart/yearly")
                .and_then(Value::as_array),
        ),
        quarterly: revenue_earnings(
            e.pointer("/financialsChart/quarterly")
                .and_then(Value::as_array),
        ),
        next,
        ex_dividend_date: date(cal, "exDividendDate"),
        dividend_date: date(cal, "dividendDate"),
        source: SOURCE.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BBCA: &str =
        include_str!("../../../tests/fixtures/yahoo/quote_summary_fundamentals_bbca.json");
    const TLKM: &str =
        include_str!("../../../tests/fixtures/yahoo/quote_summary_fundamentals_tlkm.json");
    const EARN_BBCA: &str =
        include_str!("../../../tests/fixtures/yahoo/quote_summary_statements_bbca.json");
    const EARN_TLKM: &str =
        include_str!("../../../tests/fixtures/yahoo/quote_summary_earnings_tlkm.json");
    const EARN_TLKM_20261106: &str =
        include_str!("../../../tests/fixtures/yahoo/quote_summary_earnings_tlkm_20261106.json");
    const NOT_FOUND: &str =
        include_str!("../../../tests/fixtures/yahoo/quote_summary_not_found.json");

    fn sym(s: &str) -> Symbol {
        Symbol::parse(s).unwrap()
    }

    #[test]
    fn profile_bbca() {
        let p = parse_profile(BBCA, &sym("BBCA")).unwrap();
        assert_eq!(p.name.as_deref(), Some("PT Bank Central Asia Tbk"));
        assert_eq!(p.sector.as_deref(), Some("Financial Services"));
        assert_eq!(p.industry.as_deref(), Some("Banks - Regional"));
        assert_eq!(p.employees, Some(27728));
        assert_eq!(p.city.as_deref(), Some("Jakarta"));
        assert_eq!(p.exchange.as_deref(), Some("Jakarta"));
        assert_eq!(p.currency.as_deref(), Some("IDR"));
        assert!(p.market_cap.unwrap() > 1e14);
        assert_eq!(p.shares_outstanding, Some(122_841_751_300));
        assert!(p
            .summary
            .as_deref()
            .unwrap()
            .starts_with("PT Bank Central Asia Tbk provides"));
        assert_eq!(p.source, "yahoo:quoteSummary");
    }

    #[test]
    fn fundamentals_bbca_bank() {
        let f = parse_fundamentals(BBCA, &sym("BBCA")).unwrap();
        assert_eq!(f.price, Some(5175.0));
        assert!((f.valuation.pe_trailing.unwrap() - 12.870893).abs() < 1e-6);
        assert!((f.valuation.price_to_book.unwrap() - 2.3506572).abs() < 1e-6);
        assert_eq!(f.valuation.ev_to_ebitda, None, "banks have no EBITDA");
        assert!((f.profitability.roe_pct.unwrap() - 21.818).abs() < 1e-3);
        assert_eq!(f.profitability.gross_margin_pct, None, "0 means n/a");
        assert!((f.profitability.net_margin_pct.unwrap() - 53.118).abs() < 1e-3);
        assert_eq!(f.health.debt_to_equity, None);
        assert!((f.dividend.yield_pct.unwrap() - 6.27).abs() < 1e-9);
        assert!((f.dividend.payout_ratio_pct.unwrap() - 75.42).abs() < 1e-6);
        assert_eq!(
            f.dividend.ex_dividend_date,
            Some(NaiveDate::from_ymd_opt(2026, 8, 31).unwrap())
        );
        assert_eq!(f.per_share.eps_ttm, Some(402.07));
        assert_eq!(f.analyst.recommendation.as_deref(), Some("strong_buy"));
        assert_eq!(f.analyst.analysts, Some(24));
        assert_eq!(
            f.fiscal_year_end,
            Some(NaiveDate::from_ymd_opt(2025, 12, 31).unwrap())
        );
        assert!(f.is_financial_sector());
        assert_eq!(f.sources, vec!["yahoo:quoteSummary".to_string()]);
    }

    #[test]
    fn fundamentals_tlkm_non_bank() {
        let f = parse_fundamentals(TLKM, &sym("TLKM")).unwrap();
        assert!((f.health.debt_to_equity.unwrap() - 0.59982).abs() < 1e-6);
        assert_eq!(f.health.current_ratio, Some(0.791));
        assert_eq!(f.valuation.ev_to_ebitda, Some(3.562));
        assert!((f.profitability.gross_margin_pct.unwrap() - 59.959).abs() < 1e-3);
        assert!(!f.is_financial_sector());
    }

    #[test]
    fn earnings_bbca() {
        let e = parse_earnings(EARN_BBCA, &sym("BBCA")).unwrap();
        assert_eq!(e.currency.as_deref(), Some("IDR"));
        assert_eq!(e.quarters.len(), 4);
        assert_eq!(e.quarters[0].period, "3Q2025");
        assert_eq!(e.quarters[0].eps_actual, Some(117.0));
        assert!((e.quarters[0].surprise_pct.unwrap() - 1.9).abs() < 1e-9);
        assert_eq!(
            e.quarters[0].reported_date,
            Some(NaiveDate::from_ymd_opt(2025, 10, 20).unwrap())
        );
        assert_eq!(e.yearly.len(), 4);
        assert_eq!(e.yearly[0].period, "2022");
        assert_eq!(e.yearly[3].earnings, Some(57_537_287_000_000.0));
        let n = e.next.unwrap();
        assert_eq!(n.date, Some(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()));
        assert!(n.date_is_estimate);
        assert_eq!(n.eps_avg, Some(122.67));
        assert_eq!(n.analysts, Some(3));
        assert!((n.eps_growth_pct.unwrap() - 4.85).abs() < 1e-3);
        assert_eq!(
            e.ex_dividend_date,
            Some(NaiveDate::from_ymd_opt(2026, 8, 31).unwrap())
        );
        assert_eq!(e.dividend_date, None);
    }

    #[test]
    fn earnings_tlkm_parses() {
        let e = parse_earnings(EARN_TLKM, &sym("TLKM")).unwrap();
        assert!(!e.quarters.is_empty());
        assert!(!e.yearly.is_empty());
    }

    #[test]
    fn padded_revenue_is_null_with_no_margin() {
        // Recorded 2026-11-06: 1Q2026 revenue is {"raw":0,"fmt":null} while its
        // earnings are reported; it used to print as 0 revenue at a 0.00% margin.
        for body in [EARN_TLKM_20261106, EARN_TLKM] {
            let e = parse_earnings(body, &sym("TLKM")).unwrap();
            let q = e.quarterly.iter().find(|q| q.period == "1Q2026").unwrap();
            assert_eq!(q.revenue, None);
            assert_eq!(q.profit_margin_pct, None);
            assert_eq!(q.earnings, Some(4_344_000_000_000.0));
            let q = e.quarterly.iter().find(|q| q.period == "2Q2026").unwrap();
            assert_eq!(q.revenue, Some(38_689_000_000_000.0));
            assert!((q.profit_margin_pct.unwrap() - 17.503685).abs() < 1e-6);
        }
        let e = parse_earnings(EARN_TLKM_20261106, &sym("TLKM")).unwrap();
        assert_eq!(e.quarterly.len(), 4);
        assert_eq!(e.quarterly[3].period, "3Q2026");
        assert!(e.quarterly[3].revenue.is_some());
        // BBCA's padded 2Q2025 earnings stay null, its revenue stays.
        let e = parse_earnings(EARN_BBCA, &sym("BBCA")).unwrap();
        let q = e.quarterly.iter().find(|q| q.period == "2Q2025").unwrap();
        assert_eq!(q.earnings, None);
        assert_eq!(q.profit_margin_pct, None);
        assert_eq!(q.revenue, Some(28_589_918_000_000.0));
    }

    #[test]
    fn not_found_maps_to_exit_4() {
        let err = parse_fundamentals(NOT_FOUND, &sym("ZZZZ")).unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(err.to_string().contains("ZZZZ.JK"), "{err}");
    }
}
