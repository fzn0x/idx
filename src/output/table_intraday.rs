//! Table for `stocks intraday`: the quote line, the bars' summary with VWAP,
//! a sparkline of the closes and the most recent bars.

use crate::model::Intraday;
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

/// Cells in the sparkline of the session's closes.
const SPARK_WIDTH: usize = 60;

/// `limit` keeps the most recent N bars in the table (JSON carries all);
/// `provider` picks the closing note (Stockbit is realtime, the rest delayed).
pub fn intraday(d: &Intraday, limit: Option<usize>, provider: &str, color: bool) -> String {
    let mut out = String::new();
    let mut head = vec![d.symbol.to_string()];
    if let Some(name) = d.name.as_deref().filter(|n| !n.is_empty()) {
        head.push(name.to_string());
    }
    if let Some(day) = d.session {
        head.push(format!("session {day}"));
    }
    head.push(format!("{} bars", d.interval));
    if let Some(state) = d.quote.as_ref().and_then(|q| q.market_state.as_deref()) {
        head.push(format!("market {}", state.to_lowercase()));
    }
    out.push_str(&head.join(" · "));
    out.push('\n');

    if let Some(q) = &d.quote {
        let mut line = vec![format!(
            "Last {} {} ({})",
            f::price(q.price),
            f::change(q.change),
            f::pct(q.change_pct)
        )];
        line.push(format!("prev close {}", f::price(q.previous_close)));
        line.push(format!("open {}", f::price(q.open)));
        line.push(format!("high {}", f::price(q.day_high)));
        line.push(format!("low {}", f::price(q.day_low)));
        if let Some(v) = q.volume.filter(|v| *v > 0) {
            line.push(format!("volume {}", f::compact(Some(v as f64))));
        }
        if let Some(t) = q.market_time {
            line.push(format!("last trade {} WIB", t.format("%Y-%m-%d %H:%M")));
        }
        out.push_str(&line.join(" · "));
        out.push('\n');
    }

    let Some(s) = &d.summary else {
        out.push_str("No bars for this session.\n");
        return out;
    };
    let mut line = vec![
        format!(
            "Bars {} to {} WIB",
            s.first_bar.format("%H:%M"),
            s.last_bar.format("%H:%M")
        ),
        format!("{} bar{}", s.bars, if s.bars == 1 { "" } else { "s" }),
        format!("open {}", f::price(Some(s.open))),
        format!("high {}", f::price(Some(s.high))),
        format!("low {}", f::price(Some(s.low))),
        format!("close {}", f::price(Some(s.close))),
    ];
    if s.bars_volume > 0 {
        line.push(format!("volume {}", f::compact(Some(s.bars_volume as f64))));
    }
    line.push(match s.vwap {
        Some(v) => format!("VWAP {}", f::price(Some(v))),
        None => "no VWAP (the bars carry no volume)".to_string(),
    });
    out.push_str(&line.join(" · "));
    out.push('\n');
    let closes: Vec<f64> = d.bars.iter().map(|c| c.close).collect();
    out.push_str(&f::sparkline(&closes, SPARK_WIDTH));
    out.push('\n');

    let mut t = base_table(color);
    t.set_header(header(&[
        "Time (WIB)",
        "Open",
        "High",
        "Low",
        "Close",
        "Chg %",
        "Volume",
    ]));
    let skip = limit.map(|n| d.bars.len().saturating_sub(n)).unwrap_or(0);
    let mut prev: Option<f64> = skip.checked_sub(1).map(|i| d.bars[i].close);
    for c in d.bars.iter().skip(skip) {
        let chg = prev
            .filter(|p| *p != 0.0)
            .map(|p| (c.close - p) / p * 100.0);
        t.add_row(vec![
            comfy_table::Cell::new(c.time.format("%H:%M").to_string()),
            right(f::price(Some(c.open))),
            right(f::price(Some(c.high))),
            right(f::price(Some(c.low))),
            right(f::price(Some(c.close))),
            signed(f::pct(chg), chg, color),
            right(f::int(Some(c.volume))),
        ]);
        prev = Some(c.close);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    if skip > 0 {
        out.push_str(&format!(
            "Showing the last {} of {} bars (--limit 0 for all; -o json carries every bar)\n",
            d.bars.len() - skip,
            d.bars.len()
        ));
    }
    out.push_str(match provider {
        "stockbit" => "Realtime from your Stockbit session; minutes without a trade have no bar.\n",
        "yahoo" => "Yahoo's chart, about 10 minutes behind the market; minutes without a trade have no bar;\nthe opening auction's volume is in no bar, so the bars' volume is below the day's.\n",
        _ => "Delayed as published; minutes without a trade have no bar.\n",
    });
    out.push_str(
        "VWAP is computed by idx from the bars ((high + low + close) / 3, weighted by volume), an approximation\nof IDX's average price (value / volume).\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Interval, Intraday, Period, Quote, Symbol};
    use crate::providers::yahoo::{parse_chart, parse_v7_quotes};

    fn bbca() -> Intraday {
        let h = parse_chart(
            include_str!("../../tests/fixtures/yahoo/chart_bbca_1d_1m.json"),
            Period::D1,
            Interval::Min1,
        )
        .unwrap();
        let q: Quote = parse_v7_quotes(include_str!(
            "../../tests/fixtures/yahoo/quote_v7_bbca.json"
        ))
        .unwrap()
        .remove(0);
        let bars = h.candles;
        Intraday {
            symbol: Symbol::parse("BBCA").unwrap(),
            name: Some("Bank Central Asia Tbk".into()),
            session: bars.last().map(|c| c.date()),
            interval: Interval::Min1,
            quote: Some(q),
            summary: crate::analysis::intraday::summary(&bars),
            bars,
        }
    }

    #[test]
    fn stock_table_has_quote_summary_and_recent_bars() {
        let t = intraday(&bbca(), Some(5), "yahoo", false);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(
            lines[0],
            "BBCA · Bank Central Asia Tbk · session 2026-11-05 · 1m bars · market prepre"
        );
        assert_eq!(
            lines[1],
            "Last 5,125 +195 (+3.96%) · prev close 4,930 · open 5,050 · high 5,125 · low 5,025 · volume 166.3M · last trade 2026-11-05 16:14 WIB"
        );
        assert_eq!(
            lines[2],
            "Bars 09:00 to 16:14 WIB · 333 bars · open 5,025 · high 5,125 · low 5,025 · close 5,125 · volume 155.1M · VWAP 5,089.98"
        );
        assert_eq!(lines[3].chars().count(), 60);
        assert!(t.contains("16:14"), "{t}");
        assert!(!t.contains("16:09"), "only the last 5 bars: {t}");
        assert!(t.contains("Showing the last 5 of 333 bars"));
        assert!(t.contains("opening auction's volume is in no bar"));
        assert!(t.contains("VWAP is computed by idx"));
    }

    #[test]
    fn empty_session_and_realtime_note() {
        let mut d = bbca();
        d.bars.clear();
        d.summary = None;
        d.session = None;
        d.quote = None;
        let t = intraday(&d, None, "stockbit", false);
        assert_eq!(
            t,
            "BBCA · Bank Central Asia Tbk · 1m bars\nNo bars for this session.\n"
        );
        let t = intraday(&bbca(), None, "stockbit", false);
        assert!(t.contains("Realtime from your Stockbit session"));
        assert!(!t.contains("Showing the last"));

        // One bar without volume, as at 09:11 WIB: singular, and no VWAP.
        let mut d = bbca();
        d.bars.truncate(1);
        d.summary = crate::analysis::intraday::summary(&d.bars);
        let t = intraday(&d, None, "yahoo", false);
        assert!(
            t.contains("Bars 09:00 to 09:00 WIB · 1 bar · open 5,025"),
            "{t}"
        );
        assert!(t.contains("no VWAP (the bars carry no volume)"), "{t}");
    }
}
