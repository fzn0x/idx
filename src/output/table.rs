//! Table renderers. Pure functions returning strings so they are testable and
//! reusable by the MCP server (which can hand an agent the same text a human sees).

use comfy_table::{Attribute, Cell, CellAlignment, Color, ContentArrangement, Table};

use crate::model::{History, OrderBook, Quote};
use crate::output::format as f;

pub(crate) fn base_table(color: bool) -> Table {
    let mut t = Table::new();
    t.load_preset(comfy_table::presets::UTF8_FULL_CONDENSED)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Disabled);
    if color {
        t.enforce_styling();
    } else {
        t.force_no_tty();
    }
    t
}

pub(crate) fn right(s: String) -> Cell {
    Cell::new(s).set_alignment(CellAlignment::Right)
}

pub(crate) fn signed(s: String, v: Option<f64>, color: bool) -> Cell {
    let mut c = right(s);
    if color {
        match v {
            Some(x) if x > 0.0 => c = c.fg(Color::Green),
            Some(x) if x < 0.0 => c = c.fg(Color::Red),
            _ => {}
        }
    }
    c
}

pub(crate) fn header(cols: &[&str]) -> Vec<Cell> {
    cols.iter()
        .map(|c| Cell::new(c).add_attribute(Attribute::Bold))
        .collect()
}

/// One row per quote. Rich columns stay even when a row lacks them (shown as a dash).
pub fn quotes(rows: &[Quote], color: bool) -> String {
    let mut t = base_table(color);
    t.set_header(header(&[
        "Symbol", "Name", "Price", "Chg", "Chg %", "Open", "High", "Low", "Volume", "Mkt Cap",
        "P/E", "Div %",
    ]));
    for q in rows {
        t.add_row(vec![
            Cell::new(q.symbol.as_str()).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&f::text(&q.name), 28)),
            right(f::price(q.price)),
            signed(f::change(q.change), q.change, color),
            signed(f::pct(q.change_pct), q.change_pct, color),
            right(f::price(q.open)),
            right(f::price(q.day_high)),
            right(f::price(q.day_low)),
            right(f::int(q.volume)),
            right(f::compact(q.market_cap)),
            right(f::ratio(q.pe_trailing)),
            right(
                q.dividend_yield_pct
                    .map(|d| format!("{}%", f::num(d, 2)))
                    .unwrap_or_else(|| f::DASH.into()),
            ),
        ]);
    }
    let mut out = t.to_string();
    out.push('\n');
    if let Some(when) = rows.iter().filter_map(|q| q.market_time).max() {
        let state = rows
            .iter()
            .find_map(|q| q.market_state.clone())
            .map(|s| format!(" · market {}", s.to_lowercase()))
            .unwrap_or_default();
        out.push_str(&format!(
            "Last trade {} WIB{state} · delayed data\n",
            when.format("%Y-%m-%d %H:%M")
        ));
    }
    let behind: Vec<String> = rows
        .iter()
        .filter(|q| q.behind_session == Some(true))
        .map(|q| match q.market_time {
            Some(t) => format!(
                "{} (last trade {})",
                q.symbol,
                t.with_timezone(&crate::model::wib()).format("%Y-%m-%d")
            ),
            None => q.symbol.to_string(),
        })
        .collect();
    if !behind.is_empty() {
        out.push_str(&format!(
            "Behind the latest IDX session: {} · suspended, not traded since, or the source lags\n",
            behind.join(", ")
        ));
    }
    out
}

/// Candles oldest-first; `limit` keeps only the most recent N rows (JSON is never limited).
pub fn history(h: &History, limit: Option<usize>, color: bool) -> String {
    let mut out = String::new();
    let name = h.name.clone().unwrap_or_default();
    out.push_str(&format!(
        "{} {}· {} · {} candles\n",
        h.symbol,
        if name.is_empty() {
            String::new()
        } else {
            format!("· {name} ")
        },
        format_args!("{} / {}", h.period, h.interval),
        h.candles.len()
    ));
    if let Some(s) = &h.summary {
        let fmt = if h.interval.is_intraday() {
            "%Y-%m-%d %H:%M"
        } else {
            "%Y-%m-%d"
        };
        out.push_str(&format!(
            "{} → {} · close {} → {} ({}) · high {} · low {} · avg vol {}\n",
            s.start.format(fmt),
            s.end.format(fmt),
            f::price(Some(s.first_close)),
            f::price(Some(s.last_close)),
            f::pct(Some(s.change_pct)),
            f::price(Some(s.high)),
            f::price(Some(s.low)),
            f::compact(Some(s.avg_volume as f64)),
        ));
    }

    let mut t = base_table(color);
    let intraday = h.interval.is_intraday();
    let mut cols = vec![
        if intraday { "Time (WIB)" } else { "Date" },
        "Open",
        "High",
        "Low",
        "Close",
        "Chg %",
        "Volume",
    ];
    let has_adj = h
        .candles
        .iter()
        .any(|c| c.adj_close.is_some_and(|a| (a - c.close).abs() > 1e-9));
    if has_adj {
        cols.push("Adj Close");
    }
    t.set_header(header(&cols));

    let skip = limit
        .map(|n| h.candles.len().saturating_sub(n))
        .unwrap_or(0);
    let mut prev_close: Option<f64> = if skip > 0 {
        Some(h.candles[skip - 1].close)
    } else {
        None
    };
    for c in h.candles.iter().skip(skip) {
        let chg = prev_close.map(|p| {
            if p != 0.0 {
                (c.close - p) / p * 100.0
            } else {
                0.0
            }
        });
        let mut row = vec![
            Cell::new(if intraday {
                c.time.format("%Y-%m-%d %H:%M").to_string()
            } else {
                c.date().to_string()
            }),
            right(f::price(Some(c.open))),
            right(f::price(Some(c.high))),
            right(f::price(Some(c.low))),
            right(f::price(Some(c.close))),
            signed(f::pct(chg), chg, color),
            right(f::int(Some(c.volume))),
        ];
        if has_adj {
            row.push(right(f::price(c.adj_close)));
        }
        t.add_row(row);
        prev_close = Some(c.close);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    if skip > 0 {
        out.push_str(&format!(
            "Showing last {} of {} candles (use --limit 0 for all)\n",
            h.candles.len() - skip,
            h.candles.len()
        ));
    }
    out
}

/// The order book: the depth ladder (bids left, asks right), a header line with
/// the last price and change, and the session's foreign flow. `depth` caps how
/// many levels per side are shown (0 shows all).
pub fn orderbook(ob: &OrderBook, depth: usize, color: bool) -> String {
    let mut out = String::new();
    let name = ob
        .name
        .as_deref()
        .map(|n| format!(" · {n}"))
        .unwrap_or_default();
    out.push_str(&format!(
        "{}{} · last {} ({}) · prev {} · O {} H {} L {}\n",
        ob.symbol,
        name,
        f::price(ob.last),
        f::pct(ob.change_pct),
        f::price(ob.previous_close),
        f::price(ob.open),
        f::price(ob.day_high),
        f::price(ob.day_low),
    ));

    let n = ob.bids.len().max(ob.asks.len());
    let n = if depth == 0 { n } else { n.min(depth) };
    let mut t = base_table(color);
    t.set_header(header(&[
        "Bid Vol", "Bid Ord", "Bid", "Ask", "Ask Ord", "Ask Vol",
    ]));
    let cell = |s: String| right(s);
    for i in 0..n {
        let bid = ob.bids.get(i);
        let ask = ob.asks.get(i);
        let bid_price = bid.map(|l| l.price);
        let ask_price = ask.map(|l| l.price);
        t.add_row(vec![
            cell(
                bid.map(|l| f::int(Some(l.shares)))
                    .unwrap_or_else(|| f::DASH.into()),
            ),
            cell(
                bid.map(|l| f::int(Some(l.orders)))
                    .unwrap_or_else(|| f::DASH.into()),
            ),
            signed(f::price(bid_price), bid_price.map(|_| 1.0), color),
            signed(f::price(ask_price), ask_price.map(|_| -1.0), color),
            cell(
                ask.map(|l| f::int(Some(l.orders)))
                    .unwrap_or_else(|| f::DASH.into()),
            ),
            cell(
                ask.map(|l| f::int(Some(l.shares)))
                    .unwrap_or_else(|| f::DASH.into()),
            ),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');

    out.push_str(&format!(
        "Volume {} · value {} · foreign buy {} / sell {} / net {}\n",
        f::int(ob.volume),
        f::compact(ob.value),
        f::compact(ob.foreign_buy),
        f::compact(ob.foreign_sell),
        f::compact(ob.foreign_net),
    ));
    out.push_str("Sizes in shares · values IDR · realtime via Stockbit\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, Interval, Period, Symbol};

    fn quote() -> Quote {
        let mut q = Quote::empty(Symbol::parse("BBCA").unwrap(), "t");
        q.name = Some("PT Bank Central Asia Tbk".into());
        q.price = Some(5175.0);
        q.change = Some(-900.0);
        q.change_pct = Some(-14.814815);
        q.volume = Some(1_241_098_200);
        q.market_cap = Some(635_706_095_435_776.0);
        q.pe_trailing = Some(10.963983);
        q.dividend_yield_pct = Some(6.27);
        q
    }

    #[test]
    fn quotes_table_contains_values_and_dashes() {
        let s = quotes(&[quote()], false);
        assert!(s.contains("BBCA"));
        assert!(s.contains("5,175"));
        assert!(s.contains("-14.81%"));
        assert!(s.contains("635.7T"));
        assert!(s.contains("10.96"));
        assert!(s.contains("6.27%"));
        assert!(
            s.contains(f::DASH),
            "missing open/high/low should render as a dash"
        );
        assert!(!s.contains("\x1b["), "no ANSI without color");
    }

    #[test]
    fn quotes_table_notes_rows_behind_the_session() {
        let at = |s: &str| Some(chrono::DateTime::parse_from_rfc3339(s).unwrap());
        let mut wika = quote();
        wika.symbol = Symbol::parse("WIKA").unwrap();
        wika.market_time = at("2025-02-17T16:14:58+07:00");
        wika.behind_session = Some(true);
        let mut bbca = quote();
        bbca.market_time = at("2026-11-06T16:14:43+07:00");
        bbca.behind_session = Some(false);
        let s = quotes(&[bbca.clone(), wika], false);
        assert!(
            s.contains("Behind the latest IDX session: WIKA (last trade 2025-02-17) · suspended, not traded since, or the source lags"),
            "{s}"
        );
        assert!(s.contains("Last trade 2026-11-06 16:14 WIB"), "{s}");
        assert!(!quotes(&[bbca], false).contains("Behind the latest"));
        assert!(!quotes(&[quote()], false).contains("Behind the latest"));
    }

    #[test]
    fn quotes_table_colours_when_asked() {
        let s = quotes(&[quote()], true);
        assert!(s.contains("\x1b["));
    }

    #[test]
    fn history_table_limits_rows() {
        let t0 = crate::model::wib_from_unix(1_789_524_000).unwrap();
        let candles: Vec<Candle> = (0..10)
            .map(|i| Candle {
                time: t0 + chrono::Duration::days(i),
                open: 100.0 + i as f64,
                high: 101.0 + i as f64,
                low: 99.0 + i as f64,
                close: 100.0 + i as f64,
                adj_close: None,
                volume: 1000,
            })
            .collect();
        let h = History {
            symbol: Symbol::parse("TEST").unwrap(),
            name: Some("Test Co".into()),
            currency: Some("IDR".into()),
            period: Period::M1,
            interval: Interval::Day1,
            summary: History::summarise(&candles),
            candles,
        };
        let s = history(&h, Some(3), false);
        assert!(s.contains("Showing last 3 of 10"));
        assert!(s.contains("2026-09-25"));
        assert!(!s.contains("2026-09-17 "), "older rows hidden");
        // First visible row closes at 107 against the hidden 106: +0.94%.
        assert!(
            s.contains("+0.94%"),
            "change vs the hidden previous close is still computed:\n{s}"
        );
        let all = history(&h, None, false);
        assert!(!all.contains("Showing last"));
        assert!(all.contains("TEST · Test Co · 1mo / 1d · 10 candles"));
    }
}
