//! Tables for `stocks growth | valuation | risk`: the inputs grouped by
//! theme, the reading, one line per rule with its points, then the notes on
//! rules that could not run. Same conventions as `table_reports.rs`.

use comfy_table::{Attribute, Cell, Color};

use crate::model::{
    GrowthLabel, GrowthLens, LensRule, Reading, RiskLabel, RiskLens, ValuationLabel, ValuationLens,
};
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};
use crate::output::table_reports::{kv_table, section};

const DISCLAIMER: &str = "Heuristic from public data, not investment advice.";

fn pct_plain(v: Option<f64>) -> String {
    v.map(|x| format!("{}%", f::num(x, 2)))
        .unwrap_or_else(|| f::DASH.into())
}

fn times(v: Option<f64>) -> String {
    v.map(|x| format!("{}x", f::num(x, 2)))
        .unwrap_or_else(|| f::DASH.into())
}

fn yes_no(v: Option<bool>) -> String {
    match v {
        Some(true) => "yes".into(),
        Some(false) => "no".into(),
        None => f::DASH.into(),
    }
}

/// Good, middle or bad, for the colour of the reading.
#[derive(Clone, Copy)]
enum Tone {
    Good,
    Middle,
    Bad,
    None,
}

fn reading_cell<L>(r: &Reading<L>, label: &str, tone: Tone, color: bool) -> Cell {
    let c = Cell::new(format!(
        "{} (score {:+}, {} rule{})",
        label.to_uppercase(),
        r.score,
        r.inputs_available,
        if r.inputs_available == 1 { "" } else { "s" }
    ))
    .add_attribute(Attribute::Bold);
    if !color {
        return c;
    }
    match tone {
        Tone::Good => c.fg(Color::Green),
        Tone::Middle => c.fg(Color::Yellow),
        Tone::Bad => c.fg(Color::Red),
        Tone::None => c,
    }
}

/// One line per rule (`  +1  reason`), then the notes.
fn rules_block(rules: &[LensRule], notes: &[String]) -> String {
    let mut out = String::new();
    for r in rules {
        out.push_str(&format!("  {:+}  {}\n", r.points, r.reason));
    }
    for n in notes {
        out.push_str(&format!("  ·   skipped: {n}\n"));
    }
    out
}

fn title(symbol: &str, name: &Option<String>, lens: &str) -> String {
    format!(
        "{symbol} · {} · {lens}\n",
        name.clone().unwrap_or_else(|| f::DASH.into())
    )
}

fn footer(direction: &str, sources: &[String]) -> String {
    let src = if sources.is_empty() {
        String::new()
    } else {
        format!(" Sources: {}.", sources.join(", "))
    };
    format!("{direction} {DISCLAIMER}{src}\n")
}

pub fn valuation(v: &ValuationLens, color: bool) -> String {
    let mut out = title(v.symbol.as_str(), &v.name, "valuation");
    let mixed = match (&v.financial_currency, &v.currency) {
        (Some(s), Some(p)) if !s.eq_ignore_ascii_case(p) => Some((s.as_str(), p.as_str())),
        _ => None,
    };
    out.push_str(&format!(
        "Price {} {}{}{}\n",
        f::price(v.price),
        v.currency.clone().unwrap_or_default(),
        mixed
            .map(|(s, _)| format!(" · statements in {s}"))
            .unwrap_or_default(),
        if v.financial_company {
            " · financial company"
        } else {
            ""
        }
    ));
    let star = |s: String| {
        if mixed.is_some() && s != f::DASH {
            format!("{s}*")
        } else {
            s
        }
    };
    let (label, tone) = match v.reading.label {
        ValuationLabel::Cheap => ("cheap", Tone::Good),
        ValuationLabel::Fair => ("fair", Tone::Middle),
        ValuationLabel::Expensive => ("expensive", Tone::Bad),
        ValuationLabel::InsufficientData => ("insufficient data", Tone::None),
    };
    let rows: Vec<(String, Cell)> = vec![
        ("".into(), section(color, "multiples")),
        ("P/E (trailing)".into(), right(f::ratio(v.pe_trailing))),
        ("P/E (forward)".into(), right(f::ratio(v.pe_forward))),
        ("PEG".into(), right(f::ratio(v.peg))),
        ("P/B".into(), right(star(f::ratio(v.price_to_book)))),
        ("P/S".into(), right(star(f::ratio(v.price_to_sales)))),
        ("EV/EBITDA".into(), right(star(f::ratio(v.ev_to_ebitda)))),
        ("EV/Revenue".into(), right(star(f::ratio(v.ev_to_revenue)))),
        ("Negative equity".into(), right(yes_no(v.negative_equity))),
        ("".into(), section(color, "income")),
        (
            "Dividend yield".into(),
            right(pct_plain(v.dividend_yield_pct)),
        ),
        ("Payout ratio".into(), right(pct_plain(v.payout_ratio_pct))),
        ("".into(), section(color, "analysts")),
        ("Target (mean)".into(), right(f::price(v.target_mean))),
        (
            "Analysts".into(),
            right(
                v.analysts
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| f::DASH.into()),
            ),
        ),
        (
            "Upside".into(),
            signed(f::pct(v.target_upside_pct), v.target_upside_pct, color),
        ),
        (
            "Reading".into(),
            reading_cell(&v.reading, label, tone, color),
        ),
    ];
    out.push_str(&kv_table(color, rows).to_string());
    out.push('\n');
    out.push_str(&rules_block(&v.reading.rules, &v.reading.notes));
    if let Some((s, p)) = mixed {
        out.push_str(&format!(
            "* Statements in {s}, price in {p}: these multiples divide the price by {s} figures, \
             so they are shown as reported and not scored.\n"
        ));
    }
    out.push_str(&footer("Points favour cheapness.", &v.sources));
    out
}

pub fn growth(g: &GrowthLens, color: bool) -> String {
    let mut out = title(g.symbol.as_str(), &g.name, "growth");
    out.push_str(&format!(
        "Latest quarter {} · statements in {}\n",
        g.most_recent_quarter
            .map(|d| d.to_string())
            .unwrap_or_else(|| f::DASH.into()),
        f::text(&g.statements_currency)
    ));
    let (label, tone) = match g.reading.label {
        GrowthLabel::Growing => ("growing", Tone::Good),
        GrowthLabel::Stable => ("stable", Tone::Middle),
        GrowthLabel::Mixed => ("mixed", Tone::Middle),
        GrowthLabel::Shrinking => ("shrinking", Tone::Bad),
        GrowthLabel::InsufficientData => ("insufficient data", Tone::None),
    };
    let pct = |v: Option<f64>| signed(f::pct(v), v, color);
    let rows: Vec<(String, Cell)> = vec![
        ("".into(), section(color, "latest quarter (year on year)")),
        ("Revenue growth".into(), pct(g.revenue_growth_pct)),
        ("Earnings growth".into(), pct(g.earnings_growth_pct)),
        ("".into(), section(color, "earnings per share")),
        ("EPS (ttm)".into(), right(f::price(g.eps_ttm))),
        ("EPS (forward)".into(), right(f::price(g.eps_forward))),
        ("Forward vs trailing".into(), pct(g.eps_forward_growth_pct)),
        ("".into(), section(color, "multi-year")),
        ("Revenue CAGR".into(), pct(g.revenue_cagr_pct)),
        ("Net income CAGR".into(), pct(g.net_income_cagr_pct)),
        ("Revenue 5y (source)".into(), pct(g.revenue_cagr_5y_pct)),
        ("EPS 3y (source)".into(), pct(g.eps_growth_3y_pct)),
        (
            "Reading".into(),
            reading_cell(&g.reading, label, tone, color),
        ),
    ];
    out.push_str(&kv_table(color, rows).to_string());
    out.push('\n');
    if !g.annual.is_empty() {
        let mut t = base_table(color);
        t.set_header(header(&[
            "Fiscal year",
            "Revenue",
            "Change",
            "Net income",
            "Change",
        ]));
        for y in &g.annual {
            t.add_row(vec![
                Cell::new(format!("FY{} ({})", y.end_date.format("%Y"), y.end_date)),
                right(f::compact(y.revenue)),
                signed(f::pct(y.revenue_change_pct), y.revenue_change_pct, color),
                right(f::compact(y.net_income)),
                signed(
                    f::pct(y.net_income_change_pct),
                    y.net_income_change_pct,
                    color,
                ),
            ]);
        }
        out.push_str(&t.to_string());
        out.push('\n');
    }
    out.push_str(&rules_block(&g.reading.rules, &g.reading.notes));
    out.push_str(&footer("Points favour growth.", &g.sources));
    out
}

pub fn risk(r: &RiskLens, color: bool) -> String {
    let mut out = title(r.symbol.as_str(), &r.name, "risk");
    let (label, tone) = match r.reading.label {
        RiskLabel::Low => ("low", Tone::Good),
        RiskLabel::Moderate => ("moderate", Tone::Middle),
        RiskLabel::High => ("high", Tone::Bad),
        RiskLabel::InsufficientData => ("insufficient data", Tone::None),
    };
    let mut rows: Vec<(String, Cell)> = vec![
        ("".into(), section(color, "balance sheet")),
        ("Negative equity".into(), right(yes_no(r.negative_equity))),
        ("Debt / equity".into(), right(times(r.debt_to_equity))),
        ("Current ratio".into(), right(f::ratio(r.current_ratio))),
        ("Quick ratio".into(), right(f::ratio(r.quick_ratio))),
        (
            "Financial company".into(),
            right(if r.financial_company { "yes" } else { "no" }.into()),
        ),
        ("".into(), section(color, "earnings")),
        (
            "Net margin".into(),
            signed(pct_plain(r.net_margin_pct), r.net_margin_pct, color),
        ),
        ("EPS (ttm)".into(), right(f::price(r.eps_ttm))),
    ];
    if let Some(p) = &r.price {
        let pair = |s: String, b: String| {
            if b == f::DASH {
                s
            } else {
                format!("{s} (IHSG {b})")
            }
        };
        rows.extend([
            (
                "".into(),
                section(color, &format!("price vs {} (1y daily)", p.benchmark)),
            ),
            (
                "Window".into(),
                right(format!(
                    "{} – {} · {} daily returns",
                    p.from, p.to, p.observations
                )),
            ),
            (
                "Zero returns".into(),
                right(match p.zero_returns {
                    Some(z) => format!("{z} of {} (close unchanged)", p.observations),
                    None => f::DASH.into(),
                }),
            ),
            (
                "Beta".into(),
                right(match (p.beta, p.correlation) {
                    (Some(b), Some(c)) => format!("{b:.2} (correlation {c:.2})"),
                    (Some(b), None) => format!("{b:.2}"),
                    _ => f::DASH.into(),
                }),
            ),
            (
                "Volatility (a year)".into(),
                right(pair(
                    pct_plain(p.volatility_pct),
                    pct_plain(p.benchmark_volatility_pct),
                )),
            ),
            (
                "Largest fall".into(),
                right(match &p.max_drawdown {
                    Some(d) => format!(
                        "{} · {} to {}",
                        pair(
                            format!("{}%", f::num(d.pct, 2)),
                            pct_plain(p.benchmark_max_drawdown_pct)
                        ),
                        d.peak_date,
                        d.trough_date
                    ),
                    None => pair("none".into(), pct_plain(p.benchmark_max_drawdown_pct)),
                }),
            ),
        ]);
    }
    rows.push((
        "Reading".into(),
        reading_cell(&r.reading, label, tone, color),
    ));
    out.push_str(&kv_table(color, rows).to_string());
    out.push('\n');
    out.push_str(&rules_block(&r.reading.rules, &r.reading.notes));
    out.push_str(&footer("Points add risk.", &r.sources));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::lens;
    use crate::model::{Fundamentals, Symbol};

    fn itmg_like() -> Fundamentals {
        let mut fd = Fundamentals::empty(Symbol::parse("ITMG").unwrap(), "yahoo:quoteSummary");
        fd.name = Some("Indo Tambangraya Megah Tbk".into());
        fd.currency = Some("IDR".into());
        fd.financial_currency = Some("USD".into());
        fd.price = Some(24300.0);
        fd.valuation.pe_trailing = Some(7.46);
        fd.valuation.peg = Some(0.25);
        fd.valuation.price_to_book = Some(14103.308);
        fd.dividend.yield_pct = Some(7.2);
        fd
    }

    #[test]
    fn valuation_table_lists_rules_notes_and_the_currency_mark() {
        let s = valuation(&lens::valuation(&itmg_like()), false);
        assert!(
            s.starts_with("ITMG · Indo Tambangraya Megah Tbk · valuation\n"),
            "{s}"
        );
        assert!(s.contains("statements in USD"), "{s}");
        assert!(s.contains("14,103.31*"), "{s}");
        assert!(s.contains("CHEAP (score +4, 3 rules)"), "{s}");
        assert!(
            s.contains("  +2  trailing P/E 7.5 is low (under 10)"),
            "{s}"
        );
        assert!(
            s.contains("  ·   skipped: P/B 14103.31 not scored: statements in USD, price in IDR"),
            "{s}"
        );
        assert!(s.contains("* Statements in USD, price in IDR"), "{s}");
        assert!(s.contains("Points favour cheapness."), "{s}");
        assert!(s.contains("Sources: yahoo:quoteSummary."), "{s}");
    }

    #[test]
    fn growth_and_risk_tables_render_empty_inputs() {
        let sym = Symbol::parse("TEST").unwrap();
        let g = growth(&lens::growth(&sym, None, None), false);
        assert!(g.contains("INSUFFICIENT DATA (score +0, 0 rules)"), "{g}");
        assert!(!g.contains("Fiscal year"), "no statements, no year table");
        assert!(g.contains("skipped: revenue growth in the latest quarter: not reported"));
        let r = risk(&lens::risk(&sym, None, None, None), false);
        assert!(r.contains("BALANCE SHEET"), "{r}");
        assert!(!r.contains("PRICE VS"), "{r}");
        assert!(r.contains("skipped: price risk: no daily candles for the stock"));
        assert!(r.contains("Points add risk."));
    }
}
