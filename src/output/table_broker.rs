//! `idx broker` tables: top buyers and top sellers per ticker. Same
//! conventions as `table.rs`: pure functions, dashes for missing values,
//! colour only when asked.

use comfy_table::{Attribute, Cell, Color};

use crate::model::{
    BrokerBoard, BrokerFlow, BrokerInvestor, BrokerKind, BrokerSummary, BrokerSummarySource,
};
use crate::output::format as f;
use crate::output::table::{base_table, header, right};

pub(crate) fn kind_label(k: Option<BrokerKind>) -> &'static str {
    match k {
        Some(BrokerKind::Local) => "local",
        Some(BrokerKind::Foreign) => "foreign",
        Some(BrokerKind::StateOwned) => "BUMN",
        None => f::DASH,
    }
}

/// One side's table under its title. `top` limits the rows (0: all); the
/// title says how many there are when more exist than are shown: in the
/// list, or by the source's count.
fn side(
    out: &mut String,
    title: &str,
    rows: &[BrokerFlow],
    count: Option<u32>,
    top: usize,
    buy: bool,
    color: bool,
) {
    let shown = if top == 0 {
        rows.len()
    } else {
        rows.len().min(top)
    };
    let total = count.map_or(rows.len(), |n| (n as usize).max(rows.len()));
    out.push_str(title);
    if shown < total {
        out.push_str(&format!(" ({shown} of {total})"));
    }
    out.push('\n');
    let mut t = base_table(color);
    t.set_header(header(&[
        "#", "Code", "Broker", "Class", "Lots", "Value", "Avg",
    ]));
    for r in rows.iter().take(shown) {
        let mut code = Cell::new(&r.code).add_attribute(Attribute::Bold);
        if color {
            code = code.fg(if buy { Color::Green } else { Color::Red });
        }
        t.add_row(vec![
            right(r.rank.to_string()),
            code,
            Cell::new(f::clip(r.name.as_deref().unwrap_or(f::DASH), 32)),
            Cell::new(kind_label(r.kind)),
            right(f::compact(r.lots)),
            right(f::compact(r.value)),
            right(f::price(r.average_price)),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
}

fn range(s: &BrokerSummary) -> String {
    if s.from == s.to {
        s.from.to_string()
    } else {
        format!("{} to {}", s.from, s.to)
    }
}

fn filters(s: &BrokerSummary) -> String {
    let who = match s.investor {
        BrokerInvestor::All => "all investors",
        BrokerInvestor::Foreign => "foreign investors",
        BrokerInvestor::Domestic => "domestic investors",
    };
    let board = match s.board {
        BrokerBoard::All => "all boards",
        BrokerBoard::Regular => "regular board",
        BrokerBoard::Cash => "cash board",
        BrokerBoard::Negotiated => "negotiated board",
    };
    format!("{who} · {board}")
}

fn signed_compact(v: Option<f64>) -> String {
    match v {
        Some(x) if x > 0.0 => format!("+{}", f::compact(Some(x))),
        other => f::compact(other),
    }
}

/// The range totals; for one investor side what that side bought and sold
/// (the source's totals there are its buying, not the value traded).
fn totals_line(s: &BrokerSummary) -> String {
    let side = match s.investor {
        BrokerInvestor::All => {
            return format!(
                "Value {} · lots {} · avg {} · foreign net {}\n",
                f::compact(s.total_value),
                f::compact(s.total_lots),
                f::price(s.average_price),
                f::compact(s.foreign_net_value),
            )
        }
        BrokerInvestor::Foreign => "Foreign",
        BrokerInvestor::Domestic => "Domestic",
    };
    format!(
        "{side} investors bought {} · sold {} · foreign net {}\n",
        f::compact(s.buy_value),
        f::compact(s.sell_value),
        signed_compact(s.foreign_net_value),
    )
}

const IPOT_SOURCE: &str = "IPOT broker summary, top 10 buyers and sellers ranked by lots";
const STOCKBIT_SOURCE: &str =
    "Stockbit broker summary from your session: every broker that bought and sold, ordered by lots";

/// One block per ticker; `top` limits rows per side (0: all). A call that
/// mixes sources names each block's.
pub fn broker(list: &[BrokerSummary], top: usize, color: bool) -> String {
    let from = |src: BrokerSummarySource| list.iter().any(|s| s.source == src);
    let (ipot, stockbit) = (
        from(BrokerSummarySource::Ipot),
        from(BrokerSummarySource::Stockbit),
    );
    let mixed = ipot && stockbit;
    let mut out = String::new();
    for (i, s) in list.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mark = match s.source {
            _ if !mixed => "",
            BrokerSummarySource::Ipot => " · IPOT",
            BrokerSummarySource::Stockbit => " · Stockbit",
        };
        out.push_str(&format!(
            "{}{} · {} · {}{mark}\n",
            s.symbol,
            s.name
                .as_ref()
                .map(|n| format!(" · {n}"))
                .unwrap_or_default(),
            range(s),
            filters(s),
        ));
        if s.is_empty() {
            out.push_str(&format!("No trades from {} to {}.\n", s.from, s.to));
            continue;
        }
        out.push_str(&totals_line(s));
        side(
            &mut out,
            "Top buyers",
            &s.buyers,
            s.buyer_count,
            top,
            true,
            color,
        );
        side(
            &mut out,
            "Top sellers",
            &s.sellers,
            s.seller_count,
            top,
            false,
            color,
        );
    }
    let sources = match (ipot, stockbit) {
        (true, true) => format!("{IPOT_SOURCE}; {STOCKBIT_SOURCE}"),
        (false, true) => STOCKBIT_SOURCE.to_string(),
        _ => IPOT_SOURCE.to_string(),
    };
    out.push_str(&format!("Source: {sources}. One lot = 100 shares."));
    // Only IPOT rounds, and only IPOT's foreign net ignores the board.
    let ipot_rows = || {
        list.iter()
            .filter(|s| s.source == BrokerSummarySource::Ipot)
    };
    if ipot_rows().any(|s| s.rounded) {
        let who = if mixed { "IPOT" } else { "The source" };
        out.push_str(&format!(
            " {who} rounds figures of a million and more to one decimal."
        ));
    }
    if ipot_rows().any(|s| s.board != BrokerBoard::All) {
        let whose = if mixed { "IPOT's" } else { "The source's" };
        out.push_str(&format!(
            " {whose} foreign net covers every board, so a board filter leaves it out."
        ));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Symbol;
    use chrono::NaiveDate;

    fn flow(rank: u32, code: &str, name: Option<&str>, kind: BrokerKind) -> BrokerFlow {
        BrokerFlow {
            rank,
            code: code.into(),
            name: name.map(Into::into),
            kind: Some(kind),
            lots: Some(1_600_000.0),
            value: Some(841_200_000_000.0),
            average_price: Some(5_191.0),
        }
    }

    fn summary(buyers: Vec<BrokerFlow>, sellers: Vec<BrokerFlow>) -> BrokerSummary {
        let day = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        BrokerSummary {
            symbol: Symbol::parse("BBCA").unwrap(),
            name: Some("Bank Central Asia Tbk".into()),
            from: day,
            to: day,
            investor: BrokerInvestor::All,
            board: BrokerBoard::All,
            buyers,
            sellers,
            total_value: Some(6.6e12),
            total_lots: Some(12.6e6),
            foreign_net_value: Some(-2.3e12),
            average_price: Some(5_228.0),
            buy_value: Some(6.6e12),
            sell_value: Some(6.6e12),
            rounded: true,
            buyer_count: None,
            seller_count: None,
            buyers_complete: false,
            sellers_complete: false,
            source: crate::model::BrokerSummarySource::Ipot,
        }
    }

    #[test]
    fn one_investor_side_says_what_it_bought_and_sold() {
        let mut s = summary(vec![flow(1, "AK", None, BrokerKind::Foreign)], vec![]);
        s.investor = BrokerInvestor::Foreign;
        (s.total_value, s.total_lots, s.average_price) = (None, None, None);
        (s.buy_value, s.sell_value, s.foreign_net_value) =
            (Some(605.4e9), Some(301.5e9), Some(303.9e9));
        let out = broker(&[s.clone()], 10, false);
        assert!(
            out.contains("Foreign investors bought 605.4B · sold 301.5B · foreign net +303.9B\n"),
            "{out}"
        );
        assert!(!out.contains("· lots"), "no value-traded line: {out}");
        s.board = BrokerBoard::Regular;
        (s.sell_value, s.foreign_net_value) = (None, None);
        let out = broker(&[s], 10, false);
        assert!(out.contains("a board filter leaves it out"), "{out}");
    }

    #[test]
    fn renders_both_sides_and_the_rounding_note() {
        let s = summary(
            vec![
                flow(
                    1,
                    "MG",
                    Some("SEMESTA INDOVEST SEKURITAS"),
                    BrokerKind::Local,
                ),
                flow(2, "XL", None, BrokerKind::Local),
            ],
            vec![flow(
                1,
                "CC",
                Some("MANDIRI SEKURITAS"),
                BrokerKind::StateOwned,
            )],
        );
        let out = broker(&[s], 1, false);
        assert!(
            out.starts_with(
                "BBCA · Bank Central Asia Tbk · 2026-10-16 · all investors · all boards\n"
            ),
            "{out}"
        );
        assert!(
            out.contains("Value 6.60T · lots 12.60M · avg 5,228 · foreign net -2.30T"),
            "{out}"
        );
        assert!(out.contains("SEMESTA INDOVEST SEKURITAS"));
        assert!(!out.contains("XL"), "top=1 keeps one row per side");
        assert!(out.contains("BUMN"));
        assert!(out.contains("841.2B"));
        assert!(out.contains("rounds figures of a million"));
    }

    fn stockbit(n: u32) -> BrokerSummary {
        let rows: Vec<BrokerFlow> = (1..=n)
            .map(|i| flow(i, &format!("B{i}"), None, BrokerKind::Local))
            .collect();
        let mut s = summary(rows.clone(), rows);
        s.rounded = false;
        s.source = crate::model::BrokerSummarySource::Stockbit;
        (s.buyer_count, s.seller_count) = (Some(n), Some(n));
        (s.buyers_complete, s.sellers_complete) = (true, true);
        s
    }

    #[test]
    fn a_list_longer_than_the_rows_shown_says_how_many_there_are() {
        let out = broker(&[stockbit(12)], 10, false);
        assert!(out.contains("Top buyers (10 of 12)\n"), "{out}");
        assert!(out.contains("Top sellers (10 of 12)\n"), "{out}");
        assert!(!out.contains("B11"), "{out}");
        assert!(
            out.contains("Source: Stockbit broker summary from your session: every broker that bought and sold, ordered by lots. One lot = 100 shares."),
            "{out}"
        );
        assert!(!out.contains("IPOT"), "{out}");
        // -n 0 shows every row.
        let out = broker(&[stockbit(12)], 0, false);
        assert!(out.contains("B12") && out.contains("Top buyers\n"), "{out}");
        // A cut list says how many the source counted.
        let mut cut = stockbit(5);
        (cut.buyer_count, cut.buyers_complete) = (Some(59), false);
        let out = broker(&[cut], 10, false);
        assert!(out.contains("Top buyers (5 of 59)\n"), "{out}");
        assert!(out.contains("Top sellers\n"), "{out}");
        // IPOT's ten rows of an unknown number stay as they were.
        let ipot = summary(vec![flow(1, "AK", None, BrokerKind::Foreign)], vec![]);
        let out = broker(&[ipot], 10, false);
        assert!(out.contains("Top buyers\n"), "{out}");
    }

    #[test]
    fn a_call_mixing_sources_names_each_block_and_both() {
        let ipot = summary(vec![flow(1, "AK", None, BrokerKind::Foreign)], vec![]);
        let mut sb = stockbit(2);
        sb.board = BrokerBoard::Regular;
        let out = broker(&[ipot.clone(), sb.clone()], 10, false);
        assert!(out.contains("all boards · IPOT\n"), "{out}");
        assert!(out.contains("regular board · Stockbit\n"), "{out}");
        assert!(out.contains("Source: IPOT broker summary, top 10 buyers and sellers ranked by lots; Stockbit broker summary from your session: every broker that bought and sold, ordered by lots."), "{out}");
        assert!(
            out.contains("IPOT rounds figures"),
            "IPOT's summary is rounded: {out}"
        );
        assert!(
            !out.contains("a board filter leaves it out"),
            "Stockbit's foreign net is the board's: {out}"
        );
        let out = broker(&[sb], 10, false);
        assert!(!out.contains(" · Stockbit\n"), "one source, no mark: {out}");
    }

    #[test]
    fn empty_range_says_so() {
        let mut s = summary(vec![], vec![]);
        s.rounded = false;
        let out = broker(&[s], 10, false);
        assert!(
            out.contains("No trades from 2026-10-16 to 2026-10-16."),
            "{out}"
        );
        assert!(!out.contains("Top buyers"));
        assert!(!out.contains("rounds figures"));
    }
}
