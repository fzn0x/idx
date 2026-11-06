//! Pure parsers for Yahoo responses. No I/O here, so every function is unit
//! tested against the recorded fixtures in `tests/fixtures/yahoo/`.

use chrono::{FixedOffset, NaiveDate};
use serde_json::Value;

use crate::error::{IdxError, Result};
use crate::model::{
    wib, wib_from_unix, Candle, DividendEvent, DividendHistory, History, Interval, Period, Quote,
    SplitEvent, Symbol,
};

use super::NAME;

/// Thin accessor over a JSON object that tolerates missing or mistyped fields.
struct J<'a>(&'a Value);

impl J<'_> {
    fn f64(&self, k: &str) -> Option<f64> {
        self.0.get(k).and_then(Value::as_f64)
    }
    fn u64(&self, k: &str) -> Option<u64> {
        self.0
            .get(k)
            .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64)))
    }
    fn i64(&self, k: &str) -> Option<i64> {
        self.0.get(k).and_then(Value::as_i64)
    }
    fn str(&self, k: &str) -> Option<String> {
        self.0.get(k).and_then(Value::as_str).map(str::to_string)
    }
}

/// Human readable text from `chart.error` / `finance.error`, if any.
pub fn chart_error(body: &str) -> String {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return "unexpected response".to_string(),
    };
    let err = v
        .pointer("/chart/error")
        .or_else(|| v.pointer("/finance/error"))
        .or_else(|| v.pointer("/quoteResponse/error"))
        .or_else(|| v.pointer("/quoteSummary/error"));
    match err {
        Some(e) if !e.is_null() => {
            let code = e.get("code").and_then(Value::as_str).unwrap_or("error");
            let desc = e.get("description").and_then(Value::as_str).unwrap_or("");
            if desc.is_empty() {
                code.to_string()
            } else {
                format!("{code}: {desc}")
            }
        }
        _ => "unexpected response".to_string(),
    }
}

fn chart_result(body: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(body)?;
    if let Some(err) = v.pointer("/chart/error").filter(|e| !e.is_null()) {
        let code = err.get("code").and_then(Value::as_str).unwrap_or("");
        let msg = chart_error(body);
        return Err(if code.eq_ignore_ascii_case("not found") {
            IdxError::NotFound(msg)
        } else {
            IdxError::provider(NAME, msg)
        });
    }
    v.pointer("/chart/result/0")
        .cloned()
        .ok_or_else(|| IdxError::provider(NAME, "chart response has no result"))
}

/// Parses a `v8/finance/chart` body into candles.
pub fn parse_chart(body: &str, period: Period, interval: Interval) -> Result<History> {
    let r = chart_result(body)?;
    let meta = r.get("meta").cloned().unwrap_or(Value::Null);
    let m = J(&meta);
    let symbol = Symbol::from_yahoo(&m.str("symbol").unwrap_or_default());

    let ts: Vec<Option<i64>> = r
        .get("timestamp")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(Value::as_i64).collect())
        .unwrap_or_default();
    let q = r
        .pointer("/indicators/quote/0")
        .cloned()
        .unwrap_or(Value::Null);
    let series = |k: &str| -> Vec<Option<f64>> {
        q.get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().map(Value::as_f64).collect())
            .unwrap_or_default()
    };
    let (open, high, low, close, volume) = (
        series("open"),
        series("high"),
        series("low"),
        series("close"),
        series("volume"),
    );
    let adj: Vec<Option<f64>> = r
        .pointer("/indicators/adjclose/0/adjclose")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(Value::as_f64).collect())
        .unwrap_or_default();

    let mut candles = Vec::with_capacity(ts.len());
    for (i, t) in ts.iter().enumerate() {
        let (Some(t), Some(o), Some(h), Some(l), Some(c)) = (
            *t,
            open.get(i).copied().flatten(),
            high.get(i).copied().flatten(),
            low.get(i).copied().flatten(),
            close.get(i).copied().flatten(),
        ) else {
            continue; // holiday / missing bar
        };
        let Some(time) = wib_from_unix(t) else {
            continue;
        };
        candles.push(Candle {
            time,
            open: o,
            high: h,
            low: l,
            close: c,
            adj_close: adj.get(i).copied().flatten(),
            volume: volume
                .get(i)
                .copied()
                .flatten()
                .map(|v| v.max(0.0) as u64)
                .unwrap_or(0),
        });
    }
    candles.sort_by_key(|c| c.time);

    Ok(History {
        symbol,
        name: m.str("longName").or_else(|| m.str("shortName")),
        currency: m.str("currency"),
        period,
        interval,
        summary: History::summarise(&candles),
        candles,
    })
}

/// The exchange's UTC offset (`meta.gmtoffset`, WIB when it is missing).
/// Dates of a session are taken in it: a daily bar is stamped at its local
/// open and `regularMarketTime` at the last trade, and in WIB a Toronto close
/// (16:00 EST) already falls on the next day. For IDX it is WIB.
fn exchange_offset(m: &J) -> FixedOffset {
    m.i64("gmtoffset")
        .and_then(|s| i32::try_from(s).ok())
        .and_then(FixedOffset::east_opt)
        .unwrap_or_else(wib)
}

/// The exchange's calendar date of a unix time.
fn local_date(secs: i64, offset: FixedOffset) -> Option<NaiveDate> {
    wib_from_unix(secs).map(|t| t.with_timezone(&offset).date_naive())
}

/// Parses the `events` block of a chart response (`events=div,splits`) into
/// the raw dividend history; computed fields stay empty. Dates are the
/// exchange's own (see [`exchange_offset`]).
pub fn parse_events(body: &str) -> Result<DividendHistory> {
    let r = chart_result(body)?;
    let meta = r.get("meta").cloned().unwrap_or(Value::Null);
    let m = J(&meta);
    let symbol = Symbol::from_yahoo(&m.str("symbol").unwrap_or_default());
    let offset = exchange_offset(&m);
    let day = |secs: Option<i64>| secs.and_then(|s| local_date(s, offset));

    let mut dividends: Vec<DividendEvent> = r
        .pointer("/events/dividends")
        .and_then(Value::as_object)
        .map(|o| {
            o.values()
                .filter_map(|e| {
                    let e = J(e);
                    Some(DividendEvent {
                        date: day(e.i64("date"))?,
                        amount: e.f64("amount")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    dividends.sort_by_key(|d| d.date);

    let mut splits: Vec<SplitEvent> = r
        .pointer("/events/splits")
        .and_then(Value::as_object)
        .map(|o| {
            o.values()
                .filter_map(|e| {
                    let e = J(e);
                    let numerator = e.f64("numerator")?;
                    let denominator = e.f64("denominator")?;
                    Some(SplitEvent {
                        date: day(e.i64("date"))?,
                        numerator,
                        denominator,
                        ratio: e
                            .str("splitRatio")
                            .unwrap_or_else(|| format!("{numerator}:{denominator}")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    splits.sort_by_key(|s| s.date);

    Ok(DividendHistory::raw(
        symbol,
        m.str("longName").or_else(|| m.str("shortName")),
        m.str("currency"),
        m.f64("regularMarketPrice"),
        day(m.i64("regularMarketTime")),
        dividends,
        splits,
    ))
}

/// Builds a reduced quote from a short (`range=5d`) chart response.
pub fn parse_chart_quote(body: &str) -> Result<Quote> {
    let r = chart_result(body)?;
    let meta = r.get("meta").cloned().unwrap_or(Value::Null);
    let m = J(&meta);
    let symbol = Symbol::from_yahoo(&m.str("symbol").unwrap_or_default());
    let mut q = Quote::empty(symbol, "yahoo:chart");

    q.name = m.str("longName").or_else(|| m.str("shortName"));
    q.currency = m.str("currency");
    q.price = m.f64("regularMarketPrice");
    q.change_pct = m.f64("regularMarketChangePercent");
    q.change = m.f64("fulldayChange");
    q.day_high = m.f64("regularMarketDayHigh");
    q.day_low = m.f64("regularMarketDayLow");
    q.volume = m.u64("regularMarketVolume");
    q.week52_high = m.f64("fiftyTwoWeekHigh");
    q.week52_low = m.f64("fiftyTwoWeekLow");
    q.market_time = m.i64("regularMarketTime").and_then(wib_from_unix);

    // Previous close and today's open come from the bars themselves. The
    // current session's bar may be incomplete (null close) so we take the last
    // close strictly before the market-time date rather than "second to last".
    // Both dates are the exchange's own, not WIB's (see `exchange_offset`).
    let offset = exchange_offset(&m);
    let ts: Vec<Option<i64>> = r
        .get("timestamp")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(Value::as_i64).collect())
        .unwrap_or_default();
    let closes: Vec<Option<f64>> = r
        .pointer("/indicators/quote/0/close")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(Value::as_f64).collect())
        .unwrap_or_default();
    let market_date = q.market_time.map(|t| t.with_timezone(&offset).date_naive());
    q.previous_close = match market_date {
        Some(md) => ts
            .iter()
            .zip(closes.iter())
            .filter_map(|(t, c)| Some(((*t)?, (*c)?)))
            .filter(|(t, _)| local_date(*t, offset).is_some_and(|d| d < md))
            .map(|(_, c)| c)
            .next_back(),
        None => {
            let valid: Vec<f64> = closes.iter().flatten().copied().collect();
            if valid.len() >= 2 {
                Some(valid[valid.len() - 2])
            } else {
                None
            }
        }
    };
    if q.change.is_none() {
        if let (Some(p), Some(prev)) = (q.price, q.previous_close) {
            q.change = Some(p - prev);
        }
    }
    q.open = r
        .pointer("/indicators/quote/0/open")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().rev().find_map(Value::as_f64));
    Ok(q)
}

/// Parses a `v7/finance/quote` body. Unknown symbols are simply absent.
pub fn parse_v7_quotes(body: &str) -> Result<Vec<Quote>> {
    let v: Value = serde_json::from_str(body)?;
    if let Some(err) = v
        .pointer("/quoteResponse/error")
        .or_else(|| v.pointer("/finance/error"))
    {
        if !err.is_null() {
            return Err(IdxError::provider(NAME, chart_error(body)));
        }
    }
    let rows = v
        .pointer("/quoteResponse/result")
        .and_then(Value::as_array)
        .ok_or_else(|| IdxError::provider(NAME, "quote response has no result array"))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let j = J(row);
        let Some(sym) = j.str("symbol") else { continue };
        let mut q = Quote::empty(Symbol::from_yahoo(&sym), "yahoo:quote");
        q.name = j.str("longName").or_else(|| j.str("shortName"));
        q.currency = j.str("currency");
        q.price = j.f64("regularMarketPrice");
        q.change = j.f64("regularMarketChange");
        q.change_pct = j.f64("regularMarketChangePercent");
        q.previous_close = j.f64("regularMarketPreviousClose");
        q.open = j.f64("regularMarketOpen");
        q.day_high = j.f64("regularMarketDayHigh");
        q.day_low = j.f64("regularMarketDayLow");
        q.volume = j.u64("regularMarketVolume");
        q.avg_volume_3m = j.u64("averageDailyVolume3Month");
        q.market_cap = j.f64("marketCap");
        q.pe_trailing = j.f64("trailingPE");
        q.pe_forward = j.f64("forwardPE");
        q.eps_ttm = j.f64("epsTrailingTwelveMonths");
        q.price_to_book = j.f64("priceToBook");
        q.dividend_yield_pct = j.f64("dividendYield");
        q.week52_high = j.f64("fiftyTwoWeekHigh");
        q.week52_low = j.f64("fiftyTwoWeekLow");
        q.sma50 = j.f64("fiftyDayAverage");
        q.sma200 = j.f64("twoHundredDayAverage");
        q.shares_outstanding = j.u64("sharesOutstanding");
        q.market_state = j.str("marketState");
        q.market_time = j.i64("regularMarketTime").and_then(wib_from_unix);
        out.push(q);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHART_1MO: &str = include_str!("../../../tests/fixtures/yahoo/chart_bbca_1mo_1d.json");
    const CHART_1WK: &str = include_str!("../../../tests/fixtures/yahoo/chart_tlkm_1y_1wk.json");
    const CHART_1H: &str = include_str!("../../../tests/fixtures/yahoo/chart_bbri_5d_1h.json");
    const CHART_404: &str = include_str!("../../../tests/fixtures/yahoo/chart_not_found.json");
    const V7: &str = include_str!("../../../tests/fixtures/yahoo/quote_v7_bbca_bbri_tlkm.json");
    const V7_401: &str = include_str!("../../../tests/fixtures/yahoo/quote_v7_unauthorized.json");

    #[test]
    fn chart_daily() {
        let h = parse_chart(CHART_1MO, Period::M1, Interval::Day1).unwrap();
        assert_eq!(h.symbol.as_str(), "BBCA");
        assert_eq!(h.name.as_deref(), Some("PT Bank Central Asia Tbk"));
        assert_eq!(h.currency.as_deref(), Some("IDR"));
        // 23 timestamps, but the current session's bar has a null close and is skipped.
        assert_eq!(h.candles.len(), 22);
        let first = &h.candles[0];
        assert_eq!(first.time.to_rfc3339(), "2026-09-16T09:00:00+07:00");
        assert_eq!(first.open, 6350.0);
        assert_eq!(first.close, 6325.0);
        let last = h.candles.last().unwrap();
        assert_eq!(last.close, 6075.0);
        let s = h.summary.unwrap();
        assert_eq!(s.candles, 22);
        assert!((s.change_pct - (6075.0 / 6325.0 - 1.0) * 100.0).abs() < 1e-9);
        assert_eq!(s.high, 6425.0);
        assert_eq!(s.low, 5850.0);
    }

    #[test]
    fn chart_weekly_has_adjclose() {
        let h = parse_chart(CHART_1WK, Period::Y1, Interval::Week1).unwrap();
        assert_eq!(h.symbol.as_str(), "TLKM");
        assert_eq!(h.candles.len(), 54);
        assert!(h.candles.iter().all(|c| c.adj_close.is_some()));
        assert!(h
            .candles
            .iter()
            .any(|c| (c.adj_close.unwrap() - c.close).abs() > 1e-9));
        assert!(h.candles.windows(2).all(|w| w[0].time < w[1].time));
    }

    #[test]
    fn chart_intraday() {
        let h = parse_chart(CHART_1H, Period::D5, Interval::Hour1).unwrap();
        assert_eq!(h.symbol.as_str(), "BBRI");
        // 40 timestamps, 34 complete bars.
        assert_eq!(h.candles.len(), 34);
        assert_eq!(h.candles[0].time.to_rfc3339(), "2026-10-12T09:00:00+07:00");
    }

    #[test]
    fn chart_not_found_maps_to_not_found() {
        let e = parse_chart(CHART_404, Period::M1, Interval::Day1).unwrap_err();
        assert_eq!(e.exit_code(), 4);
        assert_eq!(
            chart_error(CHART_404),
            "Not Found: No data found, symbol may be delisted"
        );
        assert_eq!(chart_error("garbage"), "unexpected response");
    }

    #[test]
    fn chart_quote_fallback() {
        let q = parse_chart_quote(CHART_1MO).unwrap();
        assert_eq!(q.symbol.as_str(), "BBCA");
        assert_eq!(q.source, "yahoo:chart");
        assert_eq!(q.price, Some(5175.0));
        assert_eq!(q.change, Some(-900.0));
        assert_eq!(q.previous_close, Some(6075.0));
        assert_eq!(q.open, Some(5825.0));
        assert_eq!(q.day_high, Some(5825.0));
        assert_eq!(q.volume, Some(1_241_098_200));
        assert_eq!(q.week52_high, Some(8750.0));
        assert!(
            q.market_cap.is_none(),
            "chart meta has no market cap; must stay null"
        );
        assert_eq!(
            q.market_time.unwrap().to_rfc3339(),
            "2026-10-16T16:14:58+07:00"
        );
    }

    #[test]
    fn chart_quote_fallback_dates_sessions_on_the_exchanges_clock() {
        // RY.TO recorded after Toronto's close on 2026-11-05: the last trade
        // (16:00 EST) is 04:00 WIB on 2026-11-06, while that session's bar is
        // stamped 09:30 EST (21:30 WIB on the 5th). In WIB the session's own
        // bar would count as "before", giving today's close as the previous.
        let body = include_str!("../../../tests/fixtures/yahoo/chart_ryto_5d_1d.json");
        let q = parse_chart_quote(body).unwrap();
        assert_eq!(q.symbol.as_str(), "RY.TO");
        assert_eq!(q.currency.as_deref(), Some("CAD"));
        assert_eq!(q.price, Some(285.86));
        assert_eq!(q.previous_close, Some(292.57000732421875));
        assert_eq!(q.change, Some(-6.71));
        // change (from meta) and previous_close (from the bars) agree.
        let implied = q.price.unwrap() - q.previous_close.unwrap();
        assert!((implied - q.change.unwrap()).abs() < 1e-4, "{implied}");
        assert_eq!(
            q.market_time.unwrap().to_rfc3339(),
            "2026-11-06T04:00:00+07:00",
            "the instant stays in WIB"
        );
        // Same rule for the dividends' price date.
        let h = parse_events(body).unwrap();
        assert_eq!(h.price_date.unwrap().to_string(), "2026-11-05");
        // IDX is unchanged: WIB is its own clock.
        let bbca = parse_events(include_str!(
            "../../../tests/fixtures/yahoo/chart_events_bbca_max.json"
        ))
        .unwrap();
        assert_eq!(bbca.price_date.unwrap().to_string(), "2026-10-16");
    }

    #[test]
    fn v7_full_row() {
        let qs = parse_v7_quotes(V7).unwrap();
        assert_eq!(qs.len(), 3);
        let q = &qs[0];
        assert_eq!(q.symbol.as_str(), "BBCA");
        assert_eq!(q.source, "yahoo:quote");
        assert_eq!(q.name.as_deref(), Some("PT Bank Central Asia Tbk"));
        assert_eq!(q.price, Some(5175.0));
        assert_eq!(q.change, Some(-900.0));
        assert!((q.change_pct.unwrap() + 14.814815).abs() < 1e-6);
        assert_eq!(q.previous_close, Some(6075.0));
        assert_eq!(q.open, Some(5825.0));
        assert_eq!(q.volume, Some(1_241_098_200));
        assert_eq!(q.avg_volume_3m, Some(125_370_055));
        assert_eq!(q.market_cap, Some(635_706_095_435_776.0));
        assert!((q.pe_trailing.unwrap() - 10.963983).abs() < 1e-6);
        assert_eq!(q.eps_ttm, Some(472.0));
        assert_eq!(q.dividend_yield_pct, Some(6.27));
        assert_eq!(q.sma50, Some(6318.0));
        assert_eq!(q.shares_outstanding, Some(122_841_751_300));
        assert_eq!(q.market_state.as_deref(), Some("CLOSED"));
        assert_eq!(
            q.market_time.unwrap().to_rfc3339(),
            "2026-10-16T16:14:58+07:00"
        );
        assert_eq!(qs[1].symbol.as_str(), "BBRI");
        assert_eq!(qs[2].symbol.as_str(), "TLKM");
    }

    #[test]
    fn v7_error_body() {
        let e = parse_v7_quotes(V7_401).unwrap_err();
        assert_eq!(e.exit_code(), 3);
        assert!(e.to_string().contains("Invalid Crumb"));
    }

    #[test]
    fn events_parse_dividends_and_splits() {
        let body = include_str!("../../../tests/fixtures/yahoo/chart_events_bbca_max.json");
        let h = parse_events(body).unwrap();
        assert_eq!(h.symbol.as_str(), "BBCA");
        assert_eq!(h.name.as_deref(), Some("PT Bank Central Asia Tbk"));
        assert_eq!(h.currency.as_deref(), Some("IDR"));
        assert_eq!(h.price, Some(5175.0));
        assert_eq!(h.dividends.len(), 47);
        assert_eq!(h.splits.len(), 2);
        let last = h.dividends.last().unwrap();
        assert_eq!(
            (last.date.to_string().as_str(), last.amount),
            ("2026-08-31", 25.0)
        );
        assert_eq!(h.dividends[0].date.to_string(), "2004-07-01");
        assert_eq!(h.splits[1].ratio, "5:1");
        assert_eq!(h.splits[1].date.to_string(), "2021-10-13");
        assert!(h.years.is_empty(), "computed fields are filled by analysis");
        assert!(parse_events(include_str!(
            "../../../tests/fixtures/yahoo/chart_not_found.json"
        ))
        .is_err());
    }
}
