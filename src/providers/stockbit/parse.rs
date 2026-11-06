//! Pure parsers for Stockbit's exodus responses, tested against the recorded
//! fixtures in `tests/fixtures/stockbit/`. No field here is invented: a value
//! the response omits, or carries as `""` / `N/A` / `NA`, becomes `None`.
//!
//! Units, measured and fixed by the exchange:
//! - orderbook `volume` and the depth ladder's `volume` are **shares**;
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

/// The full depth ladder plus the session figures and foreign flow.
pub fn parse_orderbook(symbol: &Symbol, body: &str) -> Result<OrderBook> {
    let v: Value = serde_json::from_str(body)?;
    let d = v
        .get("data")
        .ok_or_else(|| IdxError::provider(NAME, "orderbook response had no data"))?;
    let g = |k: &str| d.get(k).and_then(num);
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
        foreign_buy: g("fbuy"),
        foreign_sell: g("fsell"),
        foreign_net: g("fnet"),
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

/// The warning `history` adds when Stockbit's granularity for `tf` is not
/// the `interval` asked for: `1w` is hourly, `1d` one-minute, `3y`/`5y`
/// weekly. `None` when the bars are what was asked for.
pub fn granularity_note(tf: &str, interval: Interval) -> Option<String> {
    let real = match tf {
        "1w" if interval == Interval::Hour1 => return None,
        "1w" => "hourly",
        "1d" if interval == Interval::Min1 => return None,
        "1d" => "intraday (one-minute)",
        "3y" | "5y" => "weekly",
        _ => return None,
    };
    Some(format!(
        "Stockbit returned {real} bars for this window, not {interval} bars; each candle's time reflects the real granularity"
    ))
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
        assert_eq!(timeframe_for(Period::D1, Interval::Min1), ("1d", false));
    }

    #[test]
    fn granularity_warning_only_when_the_bars_differ() {
        assert_eq!(granularity_note("1d", Interval::Min1), None);
        let w = granularity_note("1d", Interval::Min5).unwrap();
        assert!(
            w.contains("intraday (one-minute)") && w.contains("not 5m bars"),
            "{w}"
        );
        assert!(granularity_note("1d", Interval::Day1).is_some());
        assert!(granularity_note("1w", Interval::Min2)
            .unwrap()
            .contains("hourly"));
        assert_eq!(granularity_note("1w", Interval::Hour1), None);
        assert!(granularity_note("5y", Interval::Day1)
            .unwrap()
            .contains("weekly"));
        assert_eq!(granularity_note("1y", Interval::Day1), None);
    }
}
