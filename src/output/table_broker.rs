//! `idx broker` tables: top buyers and top sellers per ticker. Same
//! conventions as `table.rs`: pure functions, dashes for missing values,
//! colour only when asked.

use comfy_table::{Attribute, Cell, Color};

use crate::model::{BrokerBoard, BrokerFlow, BrokerInvestor, BrokerKind, BrokerSummary};
use crate::output::format as f;
use crate::output::table::{base_table, header, right};

fn kind_label(k: Option<BrokerKind>) -> &'static str {
    match k {
        Some(BrokerKind::Local) => "local",
        Some(BrokerKind::Foreign) => "foreign",
        Some(BrokerKind::StateOwned) => "BUMN",
        None => f::DASH,
    }
}

fn side(out: &mut String, title: &str, rows: &[BrokerFlow], top: usize, buy: bool, color: bool) {
    out.push_str(title);
    out.push('\n');
    let mut t = base_table(color);
    t.set_header(header(&[
        "#", "Code", "Broker", "Class", "Lots", "Value", "Avg",
    ]));
    for r in rows.iter().take(top) {
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

/// One block per ticker; `top` limits rows per side.
pub fn broker(list: &[BrokerSummary], top: usize, color: bool) -> String {
    let mut out = String::new();
    for (i, s) in list.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "{}{} · {} · {}\n",
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
        side(&mut out, "Top buyers", &s.buyers, top, true, color);
        side(&mut out, "Top sellers", &s.sellers, top, false, color);
    }
    out.push_str(
        "Source: IPOT broker summary, top 10 buyers and sellers ranked by lots. One lot = 100 shares.",
    );
    if list.iter().any(|s| s.rounded) {
        out.push_str(" The source rounds figures of a million and more to one decimal.");
    }
    if list.iter().any(|s| s.board != BrokerBoard::All) {
        out.push_str(
            " The source's foreign net covers every board, so a board filter leaves it out.",
        );
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
