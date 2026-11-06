//! Tables for `idx stockbit actions` and `idx stockbit calendar`. Same
//! conventions as `table.rs`: pure functions, dashes for missing values.

use comfy_table::{Attribute, Cell};

use crate::model::{CorporateEvent, CorporateEventKind, CorporateEvents};
use crate::output::format as f;
use crate::output::table::{base_table, header};

/// A per-share amount with as many decimals as it has (up to four): `25`,
/// `3.8`, `0.0012`, `1,191.09`.
fn amount(x: f64) -> String {
    let fixed = format!("{x:.4}");
    let decimals = fixed
        .split_once('.')
        .map(|(_, d)| d.trim_end_matches('0').len())
        .unwrap_or(0);
    f::num(x, decimals)
}

fn idr(v: Option<f64>) -> Option<String> {
    v.map(|p| format!("IDR {}", amount(p)))
}

fn window(start: Option<chrono::NaiveDate>, end: Option<chrono::NaiveDate>) -> Option<String> {
    match (start, end) {
        (Some(s), Some(e)) => Some(format!("{s} to {e}")),
        (Some(s), None) => Some(format!("from {s}")),
        (None, Some(e)) => Some(format!("until {e}")),
        (None, None) => None,
    }
}

fn ratio(e: &CorporateEvent) -> Option<String> {
    match (e.ratio_old, e.ratio_new) {
        (Some(o), Some(n)) => Some(format!("{}:{}", amount(o), amount(n))),
        _ => None,
    }
}

fn shares(e: &CorporateEvent) -> Option<String> {
    e.shares.map(|s| match e.shares_pct {
        Some(p) => format!("{} shares ({} %)", f::num(s, 0), amount(p)),
        None => format!("{} shares", f::num(s, 0)),
    })
}

/// The terms of one action in a line.
pub fn detail(e: &CorporateEvent) -> String {
    use CorporateEventKind as K;
    let parts: Vec<Option<String>> = match e.kind {
        K::Dividend => vec![e.dividend_per_share.map(|v| {
            format!(
                "{} {} per share",
                e.currency.as_deref().unwrap_or("?"),
                amount(v)
            )
        })],
        K::StockDividend | K::Bonus | K::Split | K::ReverseSplit => {
            vec![ratio(e), e.factor.map(|x| format!("factor {}", amount(x)))]
        }
        K::Rights => vec![
            match (e.ratio_old, e.ratio_new) {
                (Some(o), Some(n)) => Some(match idr(e.price) {
                    Some(p) => format!("{} rights per {} held at {p}", amount(n), amount(o)),
                    None => format!("{} rights per {} held", amount(n), amount(o)),
                }),
                _ => idr(e.price).map(|p| format!("at {p}")),
            },
            window(e.trading_start, e.trading_end).map(|w| format!("trading {w}")),
        ],
        K::Warrant => vec![
            idr(e.price).map(|p| format!("exercise {p}")),
            window(e.exercise_start, e.exercise_end),
        ],
        K::Rups | K::PublicExpose => vec![e.time.clone(), e.venue.clone()],
        K::Ipo => vec![
            idr(e.price),
            shares(e),
            window(e.offer_start, e.offer_end).map(|w| format!("offer {w}")),
        ],
        K::TenderOffer => vec![
            match (idr(e.price), shares(e)) {
                (Some(p), Some(s)) => Some(format!("{p} for {s}")),
                (p, s) => p.or(s),
            },
            window(e.offer_start, e.offer_end).map(|w| format!("open {w}")),
            e.distribution_date.map(|d| format!("paid {d}")),
        ],
    };
    let mut line: Vec<String> = parts.into_iter().flatten().collect();
    if let Some(n) = &e.note {
        line.push(n.clone());
    }
    if line.is_empty() {
        f::DASH.to_string()
    } else {
        line.join(", ")
    }
}

fn kind_label(k: CorporateEventKind) -> String {
    k.as_str().replace('_', " ")
}

fn date(d: Option<chrono::NaiveDate>) -> String {
    d.map(|d| d.to_string())
        .unwrap_or_else(|| f::DASH.to_string())
}

/// One stock's actions (`symbol` set) or the market calendar.
pub fn corporate_events(c: &CorporateEvents, _color: bool) -> String {
    let calendar = c.symbol.is_none();
    let rows = if c.actions.len() == c.total {
        format!("{} rows", c.actions.len())
    } else {
        format!("{} of {} rows", c.actions.len(), c.total)
    };
    let kinds = c
        .kinds
        .iter()
        .map(|k| k.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = match &c.symbol {
        Some(s) => {
            let mut t = s.to_string();
            if let Some(n) = &c.name {
                t.push_str(&format!(" · {n}"));
            }
            t.push_str(" · Stockbit corporate actions");
            if let Some(r) = window(c.from, c.to) {
                t.push_str(&format!(" · {r}"));
            }
            format!("{t} · {rows} ({kinds})\n")
        }
        None => format!(
            "Stockbit corporate calendar · {} · {rows} ({kinds})\n",
            window(c.from, c.to).unwrap_or_default()
        ),
    };
    if c.actions.is_empty() {
        out.push_str("No actions.\n");
        return out;
    }
    let mut t = base_table(false);
    let mut head = vec!["Date"];
    if calendar {
        head.push("Code");
    }
    head.extend(["Kind", "Cum", "Ex", "Recording", "Payment", "Detail"]);
    t.set_header(header(&head));
    for e in &c.actions {
        let mut row = vec![Cell::new(e.date.to_string())];
        if calendar {
            row.push(Cell::new(e.symbol.as_str()).add_attribute(Attribute::Bold));
        }
        row.extend([
            Cell::new(kind_label(e.kind)),
            Cell::new(date(e.cum_date)),
            Cell::new(date(e.ex_date)),
            Cell::new(date(e.recording_date)),
            Cell::new(date(e.distribution_date)),
            Cell::new(f::clip(&detail(e), 64)),
        ]);
        t.add_row(row);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    out.push_str("Source: Stockbit (your session). Ratio: old:new. Date: the cum date for entitlements, else the event's first day.\n");
    if calendar {
        out.push_str(
            "Rights trading, tender offers and IPO offers still open in the range are listed too.\n",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Symbol;
    use chrono::NaiveDate;
    use CorporateEventKind as K;

    fn d(s: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
    }

    fn ev(kind: K) -> CorporateEvent {
        CorporateEvent::new(
            Symbol::parse("BMAS").unwrap(),
            kind,
            d("2026-11-23").unwrap(),
        )
    }

    #[test]
    fn details_per_kind() {
        let mut e = ev(K::Dividend);
        e.dividend_per_share = Some(0.0012);
        e.currency = Some("USD".into());
        assert_eq!(detail(&e), "USD 0.0012 per share");
        e.dividend_per_share = Some(1191.09);
        e.currency = Some("IDR".into());
        assert_eq!(detail(&e), "IDR 1,191.09 per share");

        let mut e = ev(K::Rights);
        (e.ratio_old, e.ratio_new, e.price) = (Some(655.0), Some(104.0), Some(350.0));
        (e.trading_start, e.trading_end) = (d("2026-11-27"), d("2026-12-03"));
        assert_eq!(
            detail(&e),
            "104 rights per 655 held at IDR 350, trading 2026-11-27 to 2026-12-03"
        );

        let mut e = ev(K::Split);
        (e.ratio_old, e.ratio_new, e.factor) = (Some(1.0), Some(5.0), Some(5.0));
        assert_eq!(detail(&e), "1:5, factor 5");

        let mut e = ev(K::Warrant);
        e.price = Some(800.0);
        (e.exercise_start, e.exercise_end) = (d("2027-04-16"), d("2029-10-15"));
        assert_eq!(detail(&e), "exercise IDR 800, 2027-04-16 to 2029-10-15");

        let mut e = ev(K::Ipo);
        (e.price, e.shares, e.shares_pct) = (Some(170.0), Some(2_525_000_000.0), Some(20.02));
        (e.offer_start, e.offer_end) = (d("2026-07-02"), d("2026-07-08"));
        assert_eq!(
            detail(&e),
            "IDR 170, 2,525,000,000 shares (20.02 %), offer 2026-07-02 to 2026-07-08"
        );

        let mut e = ev(K::TenderOffer);
        (e.price, e.shares, e.shares_pct) = (Some(3600.0), Some(238_442_293.0), Some(4.95));
        (e.offer_start, e.offer_end, e.distribution_date) =
            (d("2026-10-13"), d("2026-11-11"), d("2026-11-18"));
        assert_eq!(
            detail(&e),
            "IDR 3,600 for 238,442,293 shares (4.95 %), open 2026-10-13 to 2026-11-11, paid 2026-11-18"
        );

        let mut e = ev(K::Rups);
        e.time = Some("10:30".into());
        e.venue = Some("Menara BCA".into());
        assert_eq!(detail(&e), "10:30, Menara BCA");
        assert_eq!(detail(&ev(K::ReverseSplit)), f::DASH);
    }

    #[test]
    fn titles_and_columns() {
        let mut e = ev(K::Rights);
        e.cum_date = d("2026-11-23");
        let c = CorporateEvents {
            symbol: None,
            name: None,
            from: d("2026-11-06"),
            to: d("2026-11-20"),
            kinds: vec![K::Rights],
            total: 5,
            actions: vec![e],
        };
        let t = corporate_events(&c, false);
        assert!(
            t.starts_with(
                "Stockbit corporate calendar · 2026-11-06 to 2026-11-20 · 1 of 5 rows (rights)"
            ),
            "{t}"
        );
        assert!(
            t.contains("Code") && t.contains("BMAS") && t.contains("still open"),
            "{t}"
        );
        let c = CorporateEvents {
            symbol: Some(Symbol::parse("BBCA").unwrap()),
            name: Some("Bank Central Asia Tbk".into()),
            from: None,
            to: None,
            kinds: vec![K::Dividend],
            total: 0,
            actions: vec![],
        };
        let t = corporate_events(&c, false);
        assert!(
            t.starts_with(
                "BBCA · Bank Central Asia Tbk · Stockbit corporate actions · 0 rows (dividend)"
            ),
            "{t}"
        );
        assert!(t.contains("No actions."));
    }
}
