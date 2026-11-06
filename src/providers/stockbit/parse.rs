//! Pure parsers for Stockbit's exodus responses, tested against the recorded
//! fixtures in `tests/fixtures/stockbit/`. No field here is invented: a value
//! the response omits, or carries as `""` / `N/A` / `NA`, becomes `None`.
//!
//! Units, measured and fixed by the exchange:
//! - orderbook `volume` and the depth ladder's `volume` are **shares**;
//! - orderbook `fbuy` / `fsell` / `fnet` are IDR values, read only while
//!   `has_foreign_bs` is `true` (see [`foreign_withheld`]);
//! - chart candle `volume` is **lots**, converted to shares here (×100) so a
//!   Stockbit candle's volume matches Yahoo's and the orderbook's.

use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use crate::error::{IdxError, Result};
use crate::model::orderbook::{BookLevel, OrderBook};
use crate::model::{wib, wib_from_unix, Candle, History, Interval, Period, Quote, Symbol};

use super::NAME;

/// A number from a field that may be a JSON number or a numeric string, with
/// thousands separators and the source's various "missing" spellings handled.
pub fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t = s.trim().replace(',', "");
            if t.is_empty() || t.eq_ignore_ascii_case("n/a") || t.eq_ignore_ascii_case("na") {
                None
            } else {
                t.parse().ok()
            }
        }
        _ => None,
    }
}

fn nonneg_u64(v: &Value) -> Option<u64> {
    num(v).filter(|n| *n >= 0.0).map(|n| n as u64)
}

/// `true` when the body is Stockbit's not-found error (`error_type: NOT_FOUND`).
pub fn is_not_found(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error_type")
                .and_then(|e| e.as_str())
                .map(String::from)
        })
        .is_some_and(|t| t.eq_ignore_ascii_case("NOT_FOUND"))
}

/// The `message` of an error envelope, for surfacing a provider failure.
pub fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
        .unwrap_or_else(|| "unexpected response".into())
}

/// A quote from the orderbook endpoint (`/company-price-feed/v2/orderbook/...`),
/// which carries OHLC, volume, value and the foreign flow in one response.
pub fn parse_quote(symbol: &Symbol, body: &str) -> Result<Quote> {
    let v: Value = serde_json::from_str(body)?;
    let d = v
        .get("data")
        .ok_or_else(|| IdxError::provider(NAME, "quote response had no data"))?;
    let g = |k: &str| d.get(k).and_then(num);
    let mut q = Quote::empty(symbol.clone(), "stockbit:orderbook");
    q.name = d.get("name").and_then(|n| n.as_str()).map(String::from);
    q.currency = Some("IDR".into());
    q.price = g("lastprice").or_else(|| g("close"));
    q.change = g("change");
    q.change_pct = g("percentage_change");
    q.previous_close = g("previous");
    q.open = g("open");
    q.day_high = g("high");
    q.day_low = g("low");
    q.volume = d.get("volume").and_then(nonneg_u64);
    Ok(q)
}

/// The orderbook fields that carry foreign flow, IDR values.
const FOREIGN_FIELDS: [&str; 3] = ["fbuy", "fsell", "fnet"];

/// Whether `fbuy`/`fsell`/`fnet` describe the current session. Stockbit sets
/// `has_foreign_bs: false` while they still hold the previous session's
/// figures: BBCA's book in session 2 on 2026-11-03 carried 2026-11-02's
/// foreign net (-116.1B, IPOT's figure for that day; 2026-11-03 itself closed
/// at -28.8B). Only an explicit `true` counts; a missing or odd flag does not.
fn foreign_is_current(d: &Value) -> bool {
    d.get("has_foreign_bs").and_then(Value::as_bool) == Some(true)
}

/// When [`parse_orderbook`] left out foreign figures the book carries because
/// Stockbit does not mark them as current, the `has_foreign_bs` value it sent
/// instead (`false`, `null`, `"true"`, or `missing`), for the warning that
/// says why. `None` when nothing was withheld: the flag is `true`, or the book
/// has no foreign figures to withhold.
pub fn foreign_withheld(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let d = v.get("data")?;
    if foreign_is_current(d)
        || !FOREIGN_FIELDS
            .iter()
            .any(|k| d.get(*k).and_then(num).is_some())
    {
        return None;
    }
    Some(match d.get("has_foreign_bs") {
        Some(flag) => flag.to_string(),
        None => "missing".into(),
    })
}

/// The full depth ladder plus the session figures and foreign flow; the
/// foreign flow only while Stockbit marks it as current (`has_foreign_bs`
/// exactly `true`), otherwise `None`.
pub fn parse_orderbook(symbol: &Symbol, body: &str) -> Result<OrderBook> {
    let v: Value = serde_json::from_str(body)?;
    let d = v
        .get("data")
        .ok_or_else(|| IdxError::provider(NAME, "orderbook response had no data"))?;
    let g = |k: &str| d.get(k).and_then(num);
    let current = foreign_is_current(d);
    let foreign = |k: &str| g(k).filter(|_| current);
    let levels = |key: &str| -> Vec<BookLevel> {
        d.get(key)
            .and_then(|a| a.as_array())
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| {
                        Some(BookLevel {
                            price: r.get("price").and_then(num)?,
                            shares: r.get("volume").and_then(nonneg_u64).unwrap_or(0),
                            orders: r.get("que_num").and_then(nonneg_u64).unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(OrderBook {
        symbol: symbol.clone(),
        name: d.get("name").and_then(|n| n.as_str()).map(String::from),
        last: g("lastprice").or_else(|| g("close")),
        change: g("change"),
        change_pct: g("percentage_change"),
        previous_close: g("previous"),
        open: g("open"),
        day_high: g("high"),
        day_low: g("low"),
        volume: d.get("volume").and_then(nonneg_u64),
        value: g("value"),
        bids: levels("bid"),
        asks: levels("offer"),
        foreign_buy: foreign("fbuy"),
        foreign_sell: foreign("fsell"),
        foreign_net: foreign("fnet"),
    })
}

/// Maps an idx look-back window to a Stockbit chart timeframe. Stockbit's
/// vocabulary is windows, not bar sizes, and it chooses the granularity: `1w`
/// is hourly, `1m`/`3m`/`ytd`/`1y` are daily, `3y`/`5y` are weekly. Returns the
/// timeframe and whether the previous-period flag is needed.
pub fn timeframe_for(period: Period, interval: Interval) -> (&'static str, bool) {
    // An intraday request maps to the hourly (1w) or same-day (1d) windows.
    if interval.is_intraday() {
        return match period {
            Period::D1 => ("1d", false),
            _ => ("1w", true),
        };
    }
    match period {
        Period::D1 => ("1d", false),
        Period::D5 => ("1w", true),
        Period::M1 => ("1m", false),
        Period::M3 => ("3m", false),
        Period::M6 | Period::Ytd => ("ytd", true),
        Period::Y1 => ("1y", false),
        Period::Y2 => ("3y", false),
        Period::Y5 | Period::Y10 | Period::Max => ("5y", false),
    }
}

/// Candles from `/charts/{SYM}/daily`. `prices[]` rows carry `date` (epoch ms,
/// the bar start in WIB), string OHLC and a `value` that is the close; `volume`
/// is in lots and is converted to shares. Rows without a usable close or time
/// are dropped. Returns oldest first (the source already orders them so).
pub fn parse_history(
    symbol: &Symbol,
    period: Period,
    interval: Interval,
    body: &str,
) -> Result<History> {
    let v: Value = serde_json::from_str(body)?;
    let d = v
        .get("data")
        .ok_or_else(|| IdxError::provider(NAME, "chart response had no data"))?;
    let rows = d
        .get("prices")
        .and_then(|p| p.as_array())
        .ok_or_else(|| IdxError::provider(NAME, "chart response had no prices"))?;
    let mut candles: Vec<Candle> = Vec::with_capacity(rows.len());
    for r in rows {
        let Some(close) = r.get("value").and_then(num) else {
            continue;
        };
        let Some(time) = candle_time(r) else { continue };
        // Line-type rows leave OHLC empty; fall back to the close so a candle is
        // never silently zero-filled.
        let open = r.get("open").and_then(num).unwrap_or(close);
        let high = r.get("high").and_then(num).unwrap_or(close);
        let low = r.get("low").and_then(num).unwrap_or(close);
        let lots = r.get("volume").and_then(nonneg_u64).unwrap_or(0);
        candles.push(Candle {
            time,
            open,
            high,
            low,
            close,
            adj_close: None,
            volume: lots.saturating_mul(100),
        });
    }
    let summary = History::summarise(&candles);
    Ok(History {
        symbol: symbol.clone(),
        name: None,
        currency: Some("IDR".into()),
        period,
        interval,
        candles,
        summary,
    })
}

/// A candle's start time: `date` is epoch milliseconds in most windows, but the
/// intraday (`1d`) window sends `date: "0"` and the clock in `formatted_date`
/// (`YYYY-MM-DD HH:MM:SS`, WIB). Both are read; a daily `formatted_date` is a
/// bare date.
fn candle_time(row: &Value) -> Option<DateTime<FixedOffset>> {
    if let Some(ms) = row.get("date").and_then(num) {
        if ms >= 1.0 {
            return wib_from_unix((ms / 1000.0) as i64);
        }
    }
    let f = row.get("formatted_date").and_then(|s| s.as_str())?.trim();
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(f, "%Y-%m-%d %H:%M:%S") {
        return dt.and_local_timezone(wib()).single();
    }
    let date = chrono::NaiveDate::parse_from_str(f, "%Y-%m-%d").ok()?;
    date.and_hms_opt(0, 0, 0)?
        .and_local_timezone(wib())
        .single()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(s: &str) -> Symbol {
        Symbol::parse(s).unwrap()
    }

    const ORDERBOOK: &str = include_str!("../../../tests/fixtures/stockbit/orderbook_bbca.json");
    const ORDERBOOK_IHSG: &str =
        include_str!("../../../tests/fixtures/stockbit/orderbook_ihsg.json");
    /// BBCA in session 2 on 2026-11-03, `has_foreign_bs: false`.
    const IN_SESSION: &str =
        include_str!("../../../tests/fixtures/stockbit/orderbook_bbca_in_session.json");
    const NOT_FOUND: &str = include_str!("../../../tests/fixtures/stockbit/orderbook_zzzz.json");
    const CHART_1M: &str =
        include_str!("../../../tests/fixtures/stockbit/chart_candle_bbca_1m.json");
    const CHART_1W: &str =
        include_str!("../../../tests/fixtures/stockbit/chart_candle_bbca_1w.json");
    const CHART_1Y: &str =
        include_str!("../../../tests/fixtures/stockbit/chart_candle_bbca_1y.json");
    const CHART_EMPTY: &str =
        include_str!("../../../tests/fixtures/stockbit/chart_candle_bbca_1d.json");

    #[test]
    fn number_coercion() {
        assert_eq!(num(&serde_json::json!("5,175")), Some(5175.0));
        assert_eq!(num(&serde_json::json!(5175)), Some(5175.0));
        assert_eq!(num(&serde_json::json!("N/A")), None);
        assert_eq!(num(&serde_json::json!("NA")), None);
        assert_eq!(num(&serde_json::json!("")), None);
    }

    #[test]
    fn quote_from_orderbook() {
        let q = parse_quote(&sym("BBCA"), ORDERBOOK).unwrap();
        assert_eq!(q.name.as_deref(), Some("Bank Central Asia Tbk."));
        assert_eq!(q.price, Some(5175.0));
        assert_eq!(q.change, Some(-900.0));
        assert_eq!(q.change_pct, Some(-14.81));
        assert_eq!(q.previous_close, Some(6075.0));
        assert_eq!(q.open, Some(5825.0));
        assert_eq!(q.day_high, Some(5825.0));
        assert_eq!(q.day_low, Some(5175.0));
        assert_eq!(q.volume, Some(1_241_098_200));
        assert_eq!(q.currency.as_deref(), Some("IDR"));
    }

    #[test]
    fn index_quote_from_orderbook() {
        let q = parse_quote(&sym("^JKSE"), ORDERBOOK_IHSG).unwrap();
        assert!((q.price.unwrap() - 5185.105).abs() < 1e-3);
        assert_eq!(q.previous_close, Some(5863.78));
    }

    #[test]
    fn full_orderbook_depth_and_foreign() {
        let ob = parse_orderbook(&sym("BBCA"), ORDERBOOK).unwrap();
        assert!(ob.bids.is_empty(), "BBCA closed at ARB with no bids");
        assert_eq!(ob.asks.len(), 85);
        assert_eq!(ob.asks[0].price, 5175.0);
        assert_eq!(ob.asks[0].shares, 264_462_500);
        // The offer ladder sums to the reported total (shares).
        let sum: u64 = ob.asks.iter().map(|l| l.shares).sum();
        assert_eq!(sum, 331_908_100);
        assert_eq!(ob.foreign_net, Some(-2_329_493_087_500.0));
        assert_eq!(ob.last, Some(5175.0));
    }

    #[test]
    fn foreign_flow_only_when_stockbit_marks_it_current() {
        // In session the flag is false and fbuy/fsell/fnet are the previous
        // session's: left out, while the ladder and today's figures still parse.
        let ob = parse_orderbook(&sym("BBCA"), IN_SESSION).unwrap();
        assert_eq!(
            (ob.foreign_buy, ob.foreign_sell, ob.foreign_net),
            (None, None, None)
        );
        assert_eq!(foreign_withheld(IN_SESSION).as_deref(), Some("false"));
        assert_eq!(ob.volume, Some(134_744_100));
        assert_eq!(ob.value, Some(666_978_580_500.0));
        assert_eq!(ob.last, Some(4860.0));
        assert_eq!(ob.previous_close, Some(4980.0));
        assert_eq!((ob.bids.len(), ob.asks.len()), (22, 64));
        assert_eq!(
            ob.bids[0],
            BookLevel {
                price: 4850.0,
                shares: 7_255_100,
                orders: 1_649
            }
        );
        assert_eq!(
            ob.asks[0],
            BookLevel {
                price: 4860.0,
                shares: 86_800,
                orders: 1
            }
        );
        // Both sides sum to total_bid_offer.*.raw_lot (shares).
        let bid: u64 = ob.bids.iter().map(|l| l.shares).sum();
        let ask: u64 = ob.asks.iter().map(|l| l.shares).sum();
        assert_eq!((bid, ask), (36_866_000, 70_567_200));

        // After the close (weekend recordings) the flag is true: kept, no warning.
        let ob = parse_orderbook(&sym("BBCA"), ORDERBOOK).unwrap();
        assert_eq!(ob.foreign_buy, Some(1_616_461_977_500.0));
        assert_eq!(ob.foreign_net, Some(-2_329_493_087_500.0));
        assert_eq!(foreign_withheld(ORDERBOOK), None);
        let ob = parse_orderbook(&sym("^JKSE"), ORDERBOOK_IHSG).unwrap();
        assert_eq!(ob.foreign_net, Some(-3_397_459_968_400.0));
        assert_eq!(foreign_withheld(ORDERBOOK_IHSG), None);
    }

    #[test]
    fn foreign_flow_needs_an_explicit_true_flag() {
        let with_flag = |flag: Option<Value>| {
            let mut v: Value = serde_json::from_str(ORDERBOOK).unwrap();
            let d = v["data"].as_object_mut().unwrap();
            match flag {
                Some(f) => d.insert("has_foreign_bs".into(), f),
                None => d.remove("has_foreign_bs"),
            };
            v.to_string()
        };
        for (flag, said) in [
            (None, "missing"),
            (Some(Value::Null), "null"),
            (Some(serde_json::json!("true")), "\"true\""),
            (Some(serde_json::json!(1)), "1"),
        ] {
            let body = with_flag(flag);
            let ob = parse_orderbook(&sym("BBCA"), &body).unwrap();
            assert_eq!(ob.foreign_net, None, "{said}");
            assert_eq!(ob.foreign_buy, None, "{said}");
            assert_eq!(foreign_withheld(&body).as_deref(), Some(said));
        }
        // Nothing to withhold: no foreign figures in a not-current book.
        let mut v: Value = serde_json::from_str(IN_SESSION).unwrap();
        for k in FOREIGN_FIELDS {
            v["data"].as_object_mut().unwrap().remove(k);
        }
        assert_eq!(foreign_withheld(&v.to_string()), None);
        assert_eq!(foreign_withheld("not json"), None);
    }

    #[test]
    fn not_found_detected() {
        assert!(is_not_found(NOT_FOUND));
        assert!(!is_not_found(ORDERBOOK));
        assert!(parse_quote(&sym("ZZZZ"), NOT_FOUND).is_err());
    }

    #[test]
    fn daily_candles() {
        let h = parse_history(&sym("BBCA"), Period::M1, Interval::Day1, CHART_1M).unwrap();
        assert_eq!(h.candles.len(), 23);
        let last = h.candles.last().unwrap();
        assert_eq!(last.close, 5175.0);
        assert_eq!(last.open, 5825.0);
        // 12,410,982 lots -> shares.
        assert_eq!(last.volume, 1_241_098_200);
        assert_eq!(last.date().to_string(), "2026-10-16");
        // Oldest first.
        assert!(h.candles[0].time < last.time);
    }

    #[test]
    fn hourly_candles_keep_the_clock() {
        let h = parse_history(&sym("BBCA"), Period::D5, Interval::Hour1, CHART_1W).unwrap();
        assert_eq!(h.candles.len(), 34);
        let first = &h.candles[0];
        assert_eq!(
            first.time.format("%Y-%m-%d %H:%M").to_string(),
            "2026-10-12 09:00"
        );
    }

    #[test]
    fn yearly_series_summarises() {
        let h = parse_history(&sym("BBCA"), Period::Y1, Interval::Day1, CHART_1Y).unwrap();
        assert_eq!(h.candles.len(), 240);
        let s = h.summary.unwrap();
        assert_eq!(s.candles, 240);
    }

    #[test]
    fn empty_chart_is_no_candles_not_an_error() {
        let h = parse_history(&sym("BBCA"), Period::D1, Interval::Day1, CHART_EMPTY).unwrap();
        assert!(h.candles.is_empty());
        assert!(h.summary.is_none());
    }

    #[test]
    fn timeframe_mapping() {
        assert_eq!(timeframe_for(Period::M1, Interval::Day1), ("1m", false));
        assert_eq!(timeframe_for(Period::D5, Interval::Day1), ("1w", true));
        assert_eq!(timeframe_for(Period::Y5, Interval::Day1).0, "5y");
        assert_eq!(timeframe_for(Period::Y1, Interval::Hour1), ("1w", true));
        assert_eq!(timeframe_for(Period::D1, Interval::Min5), ("1d", false));
    }
}
