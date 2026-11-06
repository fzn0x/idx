//! Table for `idx ownership institutions` (MSN's institutional and fund holders).

use comfy_table::{Attribute, Cell, Color};

use crate::model::{HolderFlow, HolderKind, HolderSignal, InstitutionalOwnership};
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

fn pct(v: Option<f64>, decimals: usize) -> String {
    v.map(|x| format!("{}%", f::num(x, decimals)))
        .unwrap_or_else(|| f::DASH.into())
}

fn plus(v: f64, decimals: usize) -> String {
    let s = f::num(v, decimals);
    if v > 0.0 {
        format!("+{s}")
    } else {
        s
    }
}

fn signal_cell(s: Option<HolderSignal>, color: bool) -> Cell {
    let Some(s) = s else {
        return Cell::new(f::DASH);
    };
    let mut c = Cell::new(s.as_str());
    if color {
        match s {
            HolderSignal::New | HolderSignal::Buyer => c = c.fg(Color::Green),
            HolderSignal::Seller | HolderSignal::Exited => c = c.fg(Color::Red),
            HolderSignal::Unchanged => {}
        }
    }
    c
}

fn flow_line(label: &str, n: usize, fl: &HolderFlow) -> String {
    format!(
        "{label} ({n}): {} new, {} buying, {} selling, {} exited, {} unchanged · net {} shares\n",
        fl.new,
        fl.buyers,
        fl.sellers,
        fl.exited,
        fl.unchanged,
        plus(fl.net_shares_change as f64, 0)
    )
}

fn holders_table(r: &InstitutionalOwnership, kind: HolderKind, color: bool) -> String {
    let fund = kind == HolderKind::Fund;
    let mut cols = vec![
        "#", "Holder", "Shares", "% out", "Change", "Δ pp", "Reported", "Signal",
    ];
    if fund {
        cols.insert(4, "% of fund");
    }
    let mut t = base_table(color);
    t.set_header(header(&cols));
    for h in r.of_kind(kind) {
        let change = h.shares_change.map(|c| c as f64);
        let mut row = vec![
            right(h.rank.to_string()),
            Cell::new(f::clip(&h.name, 48)).add_attribute(Attribute::Bold),
            right(f::int(h.shares)),
            right(pct(h.pct_outstanding, 2)),
            signed(
                change.map(|c| plus(c, 0)).unwrap_or_else(|| f::DASH.into()),
                change,
                color,
            ),
            signed(
                h.pct_outstanding_change
                    .map(|c| plus(c, 4))
                    .unwrap_or_else(|| f::DASH.into()),
                h.pct_outstanding_change,
                color,
            ),
            Cell::new(
                h.report_date
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| f::DASH.into()),
            ),
            signal_cell(h.signal, color),
        ];
        if fund {
            row.insert(4, right(pct(h.pct_portfolio, 4)));
        }
        t.add_row(row);
    }
    let mut out = t.to_string();
    out.push('\n');
    out
}

pub fn institutions(r: &InstitutionalOwnership, color: bool) -> String {
    let mut out = format!(
        "{} · {} · institutional and fund holders (MSN)\n",
        r.symbol,
        r.name.clone().unwrap_or_else(|| f::DASH.into())
    );
    // Only the figures the source gave; a stock without any skips the line.
    let mut parts = Vec::new();
    if r.institutions_pct.is_some() {
        let top10 = r
            .top10_institutions_pct
            .map(|t| format!(" (top 10: {})", pct(Some(t), 2)))
            .unwrap_or_default();
        parts.push(format!(
            "institutions hold {} of shares outstanding{top10}",
            pct(r.institutions_pct, 2)
        ));
    }
    if r.funds_pct.is_some() {
        parts.push(format!("funds {}", pct(r.funds_pct, 2)));
    }
    if r.float_pct.is_some() {
        parts.push(format!("free float {}", pct(r.float_pct, 2)));
    }
    match (r.oldest_report, r.newest_report) {
        (Some(a), Some(b)) if a == b => parts.push(format!("reported {a}")),
        (Some(a), Some(b)) => parts.push(format!("reported {a} to {b}")),
        _ => {}
    }
    if !parts.is_empty() {
        let line = parts.join(" · ");
        let mut chars = line.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
            out.push('\n');
        }
    }
    if r.holders.is_empty() {
        out.push_str(&format!(
            "MSN lists no institutional or fund holders for {}.\n",
            r.symbol
        ));
        return out;
    }
    for (kind, label, fl) in [
        (
            HolderKind::Institution,
            "Institutions",
            &r.flow.institutions,
        ),
        (HolderKind::Fund, "Funds", &r.flow.funds),
    ] {
        let n = r.of_kind(kind).count();
        if n == 0 {
            continue;
        }
        out.push('\n');
        out.push_str(&flow_line(label, n, fl));
        out.push_str(&holders_table(r, kind, color));
    }
    out.push_str(
        "\nSource: MSN Money ownership data, each holder as of its own report date; change is since\n\
         that holder's previous report. Funds are part of their managers' institutional positions:\n\
         do not add the two lists.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Symbol;
    use crate::providers::msn::parse::parse_institutions;

    const EQ_JTPE: &str = include_str!("../../tests/fixtures/msn/equities_jtpe_all_signals.json");
    const EQ_STAR: &str = include_str!("../../tests/fixtures/msn/equities_star_no_ownership.json");
    const EQ_ATAP: &str = include_str!("../../tests/fixtures/msn/equities_atap_empty_holders.json");

    #[test]
    fn renders_both_lists_with_signals() {
        let r = parse_institutions(EQ_JTPE, &Symbol::parse("JTPE").unwrap())
            .unwrap()
            .unwrap();
        let t = institutions(&r, false);
        assert!(
            t.starts_with(
                "JTPE · PT Jasuindo Tiga Perkasa Tbk · institutional and fund holders (MSN)\n"
            ),
            "{t}"
        );
        assert!(t.contains("Institutions hold 0.11% of shares outstanding (top 10: 0.11%) · funds 0.11% · free float 6.07% · reported 2025-12-31 to 2026-09-30"), "{t}");
        assert!(t.contains("Institutions (4): 0 new, 1 buying, 1 selling, 1 exited, 1 unchanged · net -3,556,219 shares"), "{t}");
        assert!(t.contains("Funds (5): 1 new, 0 buying, 1 selling, 2 exited, 1 unchanged · net -3,556,219 shares"), "{t}");
        assert!(t.contains("% of fund"));
        assert!(t.contains("Niche Asset Management Limited"));
        assert!(t.contains("-3,434,200"));
        assert!(t.contains("+482,400"));
        assert!(t.contains("-0.0501"));
        for s in ["new", "buyer", "seller", "exited", "unchanged"] {
            assert!(t.contains(&format!(" {s} ")), "{s}: {t}");
        }
        assert!(t.contains("do not add the two lists"));
    }

    #[test]
    fn empty_report_says_so() {
        let r = parse_institutions(EQ_STAR, &Symbol::parse("STAR").unwrap())
            .unwrap()
            .unwrap();
        let t = institutions(&r, false);
        assert_eq!(
            t,
            "STAR · PT Calculus Global Ventures Tbk · institutional and fund holders (MSN)\n\
             MSN lists no institutional or fund holders for STAR.\n"
        );
        let atap = parse_institutions(EQ_ATAP, &Symbol::parse("ATAP").unwrap())
            .unwrap()
            .unwrap();
        let t = institutions(&atap, false);
        assert!(t.contains("\nFree float 15.06%\nMSN lists no"), "{t}");
    }
}
