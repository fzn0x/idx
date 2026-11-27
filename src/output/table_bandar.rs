//! `idx bandar` tables: the reading per ticker and the screen. Same
//! conventions as `table.rs`: pure functions, dashes for missing values,
//! colour only when asked.

use comfy_table::{Attribute, Cell, Color};

use crate::analysis::bandar::{ACCUMULATION_PCT, BIG_PCT};
use crate::model::{
    BandarDaily, BandarPhase, BandarReport, BandarScreen, BandarSignal, BrokerBoard, BrokerFlow,
    BrokerInvestor,
};
use crate::model::{BrokerActivity, BrokerStockRow};
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

pub fn signal_label(s: BandarSignal) -> &'static str {
    match s {
        BandarSignal::BigAccumulation => "big acc",
        BandarSignal::Accumulation => "acc",
        BandarSignal::Neutral => "neutral",
        BandarSignal::Distribution => "dist",
        BandarSignal::BigDistribution => "big dist",
    }
}

fn signal_cell(s: BandarSignal, color: bool) -> Cell {
    let mut c = Cell::new(signal_label(s));
    if matches!(
        s,
        BandarSignal::BigAccumulation | BandarSignal::BigDistribution
    ) {
        c = c.add_attribute(Attribute::Bold);
    }
    if color {
        if s.is_accumulation() {
            c = c.fg(Color::Green);
        } else if s.is_distribution() {
            c = c.fg(Color::Red);
        }
    }
    c
}

fn phase_label(p: Option<BandarPhase>) -> String {
    p.map(|p| p.as_str().replace('_', " "))
        .unwrap_or_else(|| f::DASH.into())
}

fn pct1(v: Option<f64>) -> String {
    match v {
        None => f::DASH.into(),
        Some(x) if x > 0.0 => format!("+{x:.1}%"),
        Some(x) => format!("{x:.1}%"),
    }
}

fn signed_compact(v: Option<f64>) -> String {
    match v {
        Some(x) if x > 0.0 => format!("+{}", f::compact(Some(x))),
        other => f::compact(other),
    }
}

fn range(r: &BandarReport) -> String {
    let span = if r.from == r.to {
        r.from.to_string()
    } else {
        format!("{} to {}", r.from, r.to)
    };
    format!("{span} ({})", r.period)
}

fn who(investor: BrokerInvestor, board: BrokerBoard) -> String {
    let who = match investor {
        BrokerInvestor::All => "all investors",
        BrokerInvestor::Foreign => "foreign investors",
        BrokerInvestor::Domestic => "domestic investors",
    };
    let board = match board {
        BrokerBoard::All => "all boards",
        BrokerBoard::Regular => "regular board",
        BrokerBoard::Cash => "cash board",
        BrokerBoard::Negotiated => "negotiated board",
    };
    format!("{who} · {board}")
}

fn flows(list: &[BrokerFlow]) -> String {
    if list.is_empty() {
        return f::DASH.into();
    }
    list.iter()
        .map(|b| {
            format!(
                "{} {} @{}",
                b.code,
                f::compact(b.value),
                f::price(b.average_price)
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn daily(out: &mut String, d: &BandarDaily, top: u32, color: bool) {
    let mut t = base_table(color);
    t.set_header(header(&[
        "Date",
        "Value",
        &format!("Top {top} net"),
        "Net %",
        "Signal",
        "Cumulative",
        "Foreign net",
    ]));
    for day in &d.days {
        t.add_row(vec![
            Cell::new(day.date.to_string()),
            right(f::compact(day.total_value)),
            signed(
                signed_compact(Some(day.net_value)),
                Some(day.net_value),
                color,
            ),
            signed(pct1(day.net_pct), day.net_pct, color),
            signal_cell(day.signal, color),
            signed(
                signed_compact(Some(day.cumulative_net_value)),
                Some(day.cumulative_net_value),
                color,
            ),
            signed(
                signed_compact(day.foreign_net_value),
                day.foreign_net_value,
                color,
            ),
        ]);
    }
    out.push_str("Bandar movement\n");
    out.push_str(&t.to_string());
    out.push('\n');
    let streak = match d.streak {
        0 => "the latest session was neutral".to_string(),
        n if n > 0 => format!(
            "{n} accumulation session{} in a row",
            if n == 1 { "" } else { "s" }
        ),
        n => format!(
            "{} distribution session{} in a row",
            -n,
            if n == -1 { "" } else { "s" }
        ),
    };
    out.push_str(&format!(
        "{} accumulation · {} distribution · {} neutral sessions · {streak}\n",
        d.accumulation_days, d.distribution_days, d.neutral_days
    ));
}

/// `true` when `source` (the envelope's provider) is Stockbit, which lists
/// every broker; IPOT publishes the top ten per side.
fn every_broker(source: &str) -> bool {
    source == crate::providers::stockbit::NAME
}

/// The source as the notes name it.
fn source_name(source: &str) -> &'static str {
    if every_broker(source) {
        "Stockbit"
    } else {
        "IPOT"
    }
}

fn rule_note(source: &str) -> String {
    format!(
        "Bandarmology from the {} broker summary: the top N buyers against the top N sellers, net as a share of the value traded (+{BIG_PCT:.0}% big accumulation, +{ACCUMULATION_PCT:.0}% accumulation, -{ACCUMULATION_PCT:.0}% distribution, -{BIG_PCT:.0}% big distribution). Heuristic, not advice.",
        source_name(source)
    )
}

/// One block per ticker; `source` is the envelope's provider.
pub fn bandar(list: &[BandarReport], source: &str, color: bool) -> String {
    let mut out = String::new();
    for (i, r) in list.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "{}{} · {} · {}\n",
            r.symbol,
            r.name
                .as_ref()
                .map(|n| format!(" · {n}"))
                .unwrap_or_default(),
            range(r),
            who(r.investor, r.board),
        ));
        if r.total_value.is_none_or(|v| v <= 0.0) {
            out.push_str(&format!("No trades from {} to {}.\n", r.from, r.to));
            continue;
        }
        out.push_str(&format!(
            "{} · top {} net {} ({} of {} traded) · phase {} · price {} ({})\n",
            r.signal.as_str().replace('_', " ").to_uppercase(),
            r.top,
            signed_compact(Some(r.net_value)),
            pct1(r.net_pct),
            f::compact(r.total_value),
            phase_label(r.phase),
            f::price(r.price),
            pct1(r.price_change_pct),
        ));
        out.push_str(&format!(
            "Top {} buyers' average {} ({} vs price) · foreign net {} ({})\n",
            r.top,
            f::price(r.bandar_average.map(f64::round)),
            pct1(r.bandar_gain_pct),
            signed_compact(r.foreign_net_value),
            pct1(r.foreign_net_pct),
        ));
        let mut t = base_table(color);
        t.set_header(header(&[
            "Top",
            "Buy value",
            "Sell value",
            "Net value",
            "Net lots",
            "Net %",
            "Buy share",
            "Buy avg",
            "Sell avg",
            "Signal",
        ]));
        for tier in &r.tiers {
            let mut top = right(tier.top.to_string());
            if tier.top == r.top {
                top = top.add_attribute(Attribute::Bold);
            }
            t.add_row(vec![
                top,
                right(f::compact(Some(tier.buy_value))),
                right(f::compact(Some(tier.sell_value))),
                signed(
                    signed_compact(Some(tier.net_value)),
                    Some(tier.net_value),
                    color,
                ),
                signed(
                    signed_compact(Some(tier.net_lots)),
                    Some(tier.net_lots),
                    color,
                ),
                signed(pct1(tier.net_pct), tier.net_pct, color),
                right(pct1(tier.buy_share_pct).trim_start_matches('+').to_string()),
                right(f::price(tier.buy_average.map(f64::round))),
                right(f::price(tier.sell_average.map(f64::round))),
                signal_cell(tier.signal, color),
            ]);
        }
        out.push_str(&t.to_string());
        out.push('\n');
        out.push_str(&format!("Buyers:  {}\n", flows(&r.buyers)));
        out.push_str(&format!("Sellers: {}\n", flows(&r.sellers)));
        if let Some(d) = &r.daily {
            daily(&mut out, d, r.top, color);
        }
        for line in &r.reasons {
            out.push_str(&format!("· {line}\n"));
        }
    }
    out.push_str(&rule_note(source));
    out.push('\n');
    out
}

fn activity_side(
    out: &mut String,
    title: &str,
    rows: &[BrokerStockRow],
    buy: bool,
    every: bool,
    color: bool,
) {
    out.push_str(title);
    out.push('\n');
    if rows.is_empty() {
        out.push_str(if every {
            "Not on this side in any scanned stock.\n"
        } else {
            "Not among the top ten in any scanned stock.\n"
        });
        return;
    }
    let mut t = base_table(color);
    t.set_header(header(&[
        "Symbol",
        "Name",
        "Rank",
        "Lots",
        "Value",
        "Avg",
        "Share",
        "Other side",
        "Net",
    ]));
    for r in rows {
        let mut sym = Cell::new(r.symbol.as_str()).add_attribute(Attribute::Bold);
        if color {
            sym = sym.fg(if buy { Color::Green } else { Color::Red });
        }
        t.add_row(vec![
            sym,
            Cell::new(f::clip(r.name.as_deref().unwrap_or(f::DASH), 26)),
            right(r.rank.to_string()),
            right(f::compact(r.lots)),
            right(f::compact(r.value)),
            right(f::price(r.average_price)),
            right(
                r.share_pct
                    .map(|p| format!("{}%", f::num(p, 1)))
                    .unwrap_or_else(|| f::DASH.to_string()),
            ),
            right(f::compact(r.opposite_value)),
            signed(f::compact(r.net_value), r.net_value, color),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
}

/// One broker's appearances across the scanned stocks; `source` is the
/// envelope's provider.
pub fn broker_activity(a: &BrokerActivity, source: &str, color: bool) -> String {
    let every = every_broker(source);
    let mut out = format!(
        "{}{} · {} ({} to {}) · {} scanned, {} with trades\n",
        a.code,
        a.name
            .as_ref()
            .map(|n| format!(" · {n}"))
            .unwrap_or_default(),
        a.period,
        a.from,
        a.to,
        a.scanned,
        a.checked,
    );
    out.push_str(&format!(
        "{} {} in {} stocks · {} {} in {} stocks\n",
        if every { "Buying" } else { "Visible buying" },
        f::compact(Some(a.bought_value)),
        a.bought.len(),
        if every { "selling" } else { "visible selling" },
        f::compact(Some(a.sold_value)),
        a.sold.len(),
    ));
    let (buyer, seller) = if every {
        ("Buyer of", "Seller of")
    } else {
        ("Top-ten buyer of", "Top-ten seller of")
    };
    activity_side(&mut out, buyer, &a.bought, true, every, color);
    activity_side(&mut out, seller, &a.sold, false, every, color);
    out.push_str(if every {
        "Every broker that bought or sold is listed per stock. Source: Stockbit.\n"
    } else {
        "Only the top ten buyers and sellers per stock are published; net shows where both sides are visible. Source: IPOT.\n"
    });
    out
}

/// The screen's rows; `source` is the envelope's provider.
pub fn screen(s: &BandarScreen, source: &str, color: bool) -> String {
    let mut out = format!(
        "Bandar screen{} · {} {} to {} · top {} · scanned {} · {} with trades · {} match{}\n",
        s.preset
            .as_ref()
            .map(|p| format!(" · {p}"))
            .unwrap_or_default(),
        s.period.as_str(),
        s.from,
        s.to,
        s.top,
        s.scanned,
        s.checked,
        s.total_matches,
        if s.total_matches == 1 { "" } else { "es" },
    );
    out.push_str(&format!(
        "Filters: {} · sorted by {} {}\n",
        if s.filters.is_empty() {
            "none".to_string()
        } else {
            s.filters.join(", ")
        },
        s.sort,
        if s.descending {
            "descending"
        } else {
            "ascending"
        },
    ));
    if s.rows.is_empty() {
        out.push_str("No stock matched.\n");
    } else {
        let mut t = base_table(color);
        t.set_header(header(&[
            "Symbol",
            "Signal",
            "Net %",
            "Net value",
            "Chg %",
            "Phase",
            "Foreign %",
            "Bandar avg",
            "Gain",
            "Price",
            "Top buyers",
        ]));
        for r in &s.rows {
            t.add_row(vec![
                Cell::new(r.symbol.as_str()).add_attribute(Attribute::Bold),
                signal_cell(r.signal, color),
                signed(pct1(r.net_pct), r.net_pct, color),
                signed(signed_compact(Some(r.net_value)), Some(r.net_value), color),
                signed(pct1(r.price_change_pct), r.price_change_pct, color),
                Cell::new(phase_label(r.phase)),
                signed(pct1(r.foreign_net_pct), r.foreign_net_pct, color),
                right(f::price(r.bandar_average.map(f64::round))),
                signed(pct1(r.bandar_gain_pct), r.bandar_gain_pct, color),
                right(f::price(r.price)),
                Cell::new(r.top_buyers.join(" ")),
            ]);
        }
        out.push_str(&t.to_string());
        out.push('\n');
        if s.rows.len() < s.total_matches {
            out.push_str(&format!(
                "Showing {} of {} matches; raise --limit or set 0 for all.\n",
                s.rows.len(),
                s.total_matches
            ));
        }
    }
    out.push_str(&rule_note(source));
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::bandar as bd;
    use crate::model::{BandarPeriod, BrokerSummary, Symbol};
    use crate::providers::ipot::parse;
    use chrono::NaiveDate;

    fn week() -> BrokerSummary {
        let html =
            include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261012_20261016.html");
        let day = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        match parse::parse(html).unwrap() {
            parse::Page::Summary(p) => p.into_summary(
                Symbol::parse("BBCA").unwrap(),
                day,
                day,
                BrokerInvestor::All,
                BrokerBoard::All,
            ),
            parse::Page::Invalid => unreachable!(),
        }
    }

    #[test]
    fn report_block() {
        let mut s = week();
        s.name = Some("Bank Central Asia Tbk".into());
        let r = bd::report(&s, "1w", 5, Some(5_175.0), Some(-14.46));
        let out = bandar(&[r], "ipot", false);
        assert!(
            out.starts_with("BBCA · Bank Central Asia Tbk · 2026-10-12 to 2026-10-16 (1w) · all investors · all boards\n"),
            "{out}"
        );
        assert!(out.contains("BIG DISTRIBUTION · top 5 net -"), "{out}");
        assert!(
            out.contains("phase markdown · price 5,175 (-14.5%)"),
            "{out}"
        );
        assert!(out.contains("Buy share"));
        assert!(out.contains("big dist"));
        assert!(out.contains("Buyers:  "));
        assert!(out.contains("· Top 5 buyers bought "));
        assert!(out.contains("Heuristic, not advice."));
    }

    #[test]
    fn screen_block() {
        let s = week();
        let row = bd::screen_row(&bd::report(&s, "1w", 5, Some(5_175.0), Some(-14.46)));
        let screen_ = BandarScreen {
            period: BandarPeriod::Week1,
            from: s.from,
            to: s.to,
            top: 5,
            preset: Some("big-distribution".into()),
            filters: vec!["acc<=-15".into()],
            sort: "acc".into(),
            descending: false,
            scanned: 100,
            checked: 98,
            total_matches: 2,
            rows: vec![row],
        };
        let out = screen(&screen_, "ipot", false);
        assert!(out.starts_with("Bandar screen · big-distribution · 1w 2026-10-12 to 2026-10-16 · top 5 · scanned 100 · 98 with trades · 2 matches\n"), "{out}");
        assert!(out.contains("Filters: acc<=-15 · sorted by acc ascending"));
        assert!(out.contains("markdown"));
        assert!(out.contains("Showing 1 of 2 matches"));
    }
}
