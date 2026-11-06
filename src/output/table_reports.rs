//! Report-style tables: profile, fundamentals, compare, financials, earnings
//! and technicals. Same conventions as `table.rs`: pure functions, dashes for
//! missing values, colour only when asked.

use comfy_table::{Attribute, Cell, Color, Table};

use crate::analysis::fundamental::summary_line;
use crate::model::{
    Brief, DividendHistory, Earnings, EarningsEstimate, FinancialPeriod, Financials, Fundamentals,
    MarketOverview, MarketRow, Performance, Profile, ReturnWindows, SignalLabel, StatementPeriod,
    Technical, Trend, VerdictLabel,
};
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

fn pct_opt(v: Option<f64>) -> String {
    v.map(|x| format!("{}%", f::num(x, 2)))
        .unwrap_or_else(|| f::DASH.into())
}

fn ratio_x(v: Option<f64>) -> String {
    v.map(|x| format!("{}x", f::num(x, 2)))
        .unwrap_or_else(|| f::DASH.into())
}

fn date_opt(d: Option<chrono::NaiveDate>) -> String {
    d.map(|d| d.to_string()).unwrap_or_else(|| f::DASH.into())
}

fn title_case(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Greedy word wrap for the summary paragraph.
pub fn wrap(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn kv_table(color: bool, rows: Vec<(String, Cell)>) -> Table {
    let mut t = base_table(color);
    t.load_preset(comfy_table::presets::UTF8_FULL_CONDENSED)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS);
    for (k, v) in rows {
        t.add_row(vec![Cell::new(k).add_attribute(Attribute::Bold), v]);
    }
    t
}

fn section(color: bool, name: &str) -> Cell {
    let c = Cell::new(name.to_uppercase()).add_attribute(Attribute::Bold);
    if color {
        c.fg(Color::Cyan)
    } else {
        c
    }
}

pub fn profile(p: &Profile, color: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} · {}\n",
        p.symbol,
        p.name.clone().unwrap_or_else(|| f::DASH.into())
    ));
    let rows = vec![
        ("Sector".into(), Cell::new(f::text(&p.sector))),
        ("Industry".into(), Cell::new(f::text(&p.industry))),
        ("Exchange".into(), Cell::new(f::text(&p.exchange))),
        ("Currency".into(), Cell::new(f::text(&p.currency))),
        ("Market cap".into(), Cell::new(f::compact(p.market_cap))),
        (
            "Shares outstanding".into(),
            Cell::new(f::int(p.shares_outstanding)),
        ),
        ("Employees".into(), Cell::new(f::int(p.employees))),
        (
            "Founded".into(),
            Cell::new(
                p.founded_year
                    .map(|y| y.to_string())
                    .unwrap_or_else(|| f::DASH.into()),
            ),
        ),
        (
            "Headquarters".into(),
            Cell::new(match (&p.city, &p.country) {
                (Some(c), Some(k)) => format!("{c}, {k}"),
                (Some(c), None) => c.clone(),
                (None, Some(k)) => k.clone(),
                _ => f::DASH.into(),
            }),
        ),
        ("Website".into(), Cell::new(f::text(&p.website))),
    ];
    out.push_str(&kv_table(color, rows).to_string());
    out.push('\n');
    if let Some(s) = &p.summary {
        out.push('\n');
        out.push_str(&wrap(s, 88));
    }
    if !p.directors.is_empty() {
        out.push('\n');
        out.push_str("Board and management\n");
        for d in p.directors.iter().take(10) {
            out.push_str(&format!(
                "  {}{}\n",
                d.name,
                d.title
                    .as_ref()
                    .map(|t| format!(" · {t}"))
                    .unwrap_or_default()
            ));
        }
        if p.directors.len() > 10 {
            out.push_str(&format!(
                "  … and {} more (see -o json)\n",
                p.directors.len() - 10
            ));
        }
    }
    out.push_str(&format!("Source: {}\n", p.source));
    out
}

fn verdict_cell(f: &Fundamentals, color: bool) -> Cell {
    let Some(v) = &f.verdict else {
        return Cell::new(f::DASH);
    };
    let c = Cell::new(v.label.as_str());
    if !color {
        return c;
    }
    match v.label {
        VerdictLabel::Undervalued => c.fg(Color::Green),
        VerdictLabel::Overvalued => c.fg(Color::Red),
        VerdictLabel::Fair => c.fg(Color::Yellow),
        VerdictLabel::InsufficientData => c,
    }
}

pub fn fundamentals(fd: &Fundamentals, color: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} · {} · {}{}\n",
        fd.symbol,
        fd.name.clone().unwrap_or_else(|| f::DASH.into()),
        f::text(&fd.sector),
        fd.industry
            .as_ref()
            .map(|i| format!(" / {i}"))
            .unwrap_or_default()
    ));
    let price_line = format!(
        "Price {} {} · 52w {} – {} · beta {}",
        f::price(fd.price),
        fd.currency.clone().unwrap_or_default(),
        f::price(fd.week52_low),
        f::price(fd.week52_high),
        f::ratio(fd.beta)
    );
    out.push_str(&price_line);
    out.push('\n');

    let v = &fd.valuation;
    let p = &fd.profitability;
    let h = &fd.health;
    let g = &fd.growth;
    let d = &fd.dividend;
    let ps = &fd.per_share;
    let a = &fd.analyst;
    let rows: Vec<(String, Cell)> = vec![
        ("".into(), section(color, "valuation")),
        ("Market cap".into(), right(f::compact(v.market_cap))),
        (
            "Enterprise value".into(),
            right(f::compact(v.enterprise_value)),
        ),
        ("P/E (trailing)".into(), right(f::ratio(v.pe_trailing))),
        ("P/E (forward)".into(), right(f::ratio(v.pe_forward))),
        ("PEG".into(), right(f::ratio(v.peg))),
        ("P/B".into(), right(f::ratio(v.price_to_book))),
        ("P/S".into(), right(f::ratio(v.price_to_sales))),
        ("EV/EBITDA".into(), right(f::ratio(v.ev_to_ebitda))),
        ("EV/Revenue".into(), right(f::ratio(v.ev_to_revenue))),
        ("".into(), section(color, "profitability")),
        ("ROE".into(), right(pct_opt(p.roe_pct))),
        ("ROA".into(), right(pct_opt(p.roa_pct))),
        ("Gross margin".into(), right(pct_opt(p.gross_margin_pct))),
        (
            "Operating margin".into(),
            right(pct_opt(p.operating_margin_pct)),
        ),
        ("Net margin".into(), right(pct_opt(p.net_margin_pct))),
        ("".into(), section(color, "balance sheet")),
        ("Debt / equity".into(), right(ratio_x(h.debt_to_equity))),
        ("Current ratio".into(), right(f::ratio(h.current_ratio))),
        ("Quick ratio".into(), right(f::ratio(h.quick_ratio))),
        ("Total debt".into(), right(f::compact(h.total_debt))),
        ("Total cash".into(), right(f::compact(h.total_cash))),
        ("".into(), section(color, "growth")),
        (
            "Revenue growth (yoy)".into(),
            signed(f::pct(g.revenue_growth_pct), g.revenue_growth_pct, color),
        ),
        (
            "Earnings growth (yoy)".into(),
            signed(f::pct(g.earnings_growth_pct), g.earnings_growth_pct, color),
        ),
        (
            "Revenue CAGR 5y".into(),
            signed(f::pct(g.revenue_cagr_5y_pct), g.revenue_cagr_5y_pct, color),
        ),
        (
            "EPS growth 3y".into(),
            signed(f::pct(g.eps_growth_3y_pct), g.eps_growth_3y_pct, color),
        ),
        ("".into(), section(color, "dividend")),
        ("Yield".into(), right(pct_opt(d.yield_pct))),
        ("Payout ratio".into(), right(pct_opt(d.payout_ratio_pct))),
        ("Ex-dividend".into(), right(date_opt(d.ex_dividend_date))),
        ("".into(), section(color, "per share")),
        ("EPS (ttm)".into(), right(f::price(ps.eps_ttm))),
        ("EPS (forward)".into(), right(f::price(ps.eps_forward))),
        ("Book value".into(), right(f::price(ps.book_value))),
        ("Revenue".into(), right(f::price(ps.revenue))),
        ("".into(), section(color, "analysts")),
        (
            "Recommendation".into(),
            right(
                a.recommendation
                    .as_deref()
                    .map(title_case)
                    .unwrap_or_else(|| f::DASH.into()),
            ),
        ),
        (
            "Target mean / low / high".into(),
            right(format!(
                "{} / {} / {}",
                f::price(a.target_mean),
                f::price(a.target_low),
                f::price(a.target_high)
            )),
        ),
        (
            "Analysts".into(),
            right(
                a.analysts
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| f::DASH.into()),
            ),
        ),
        ("".into(), section(color, "verdict")),
        ("Reading".into(), verdict_cell(fd, color)),
    ];
    out.push_str(&kv_table(color, rows).to_string());
    out.push('\n');
    if let Some(v) = &fd.verdict {
        out.push_str(&summary_line(fd));
        out.push('\n');
        for r in &v.reasons {
            out.push_str(&format!("  • {r}\n"));
        }
    }
    out.push_str(&format!(
        "Fiscal year end {} · latest quarter {} · sources: {}\n",
        date_opt(fd.fiscal_year_end),
        date_opt(fd.most_recent_quarter),
        fd.sources.join(", ")
    ));
    out
}

pub fn compare(list: &[Fundamentals], color: bool) -> String {
    let mut t = base_table(color);
    let mut cols: Vec<&str> = vec!["Metric"];
    for fd in list {
        cols.push(fd.symbol.as_str());
    }
    t.set_header(header(&cols));

    macro_rules! row {
        ($label:expr, $f:expr) => {{
            let mut cells = vec![Cell::new($label).add_attribute(Attribute::Bold)];
            for fd in list {
                cells.push($f(fd));
            }
            t.add_row(cells);
        }};
    }
    row!("Price", |fd: &Fundamentals| right(f::price(fd.price)));
    row!("Market cap", |fd: &Fundamentals| right(f::compact(
        fd.valuation.market_cap
    )));
    row!("P/E (ttm)", |fd: &Fundamentals| right(f::ratio(
        fd.valuation.pe_trailing
    )));
    row!("P/E (fwd)", |fd: &Fundamentals| right(f::ratio(
        fd.valuation.pe_forward
    )));
    row!("PEG", |fd: &Fundamentals| right(f::ratio(fd.valuation.peg)));
    row!("P/B", |fd: &Fundamentals| right(f::ratio(
        fd.valuation.price_to_book
    )));
    row!("P/S", |fd: &Fundamentals| right(f::ratio(
        fd.valuation.price_to_sales
    )));
    row!("EV/EBITDA", |fd: &Fundamentals| right(f::ratio(
        fd.valuation.ev_to_ebitda
    )));
    row!("ROE", |fd: &Fundamentals| right(pct_opt(
        fd.profitability.roe_pct
    )));
    row!("ROA", |fd: &Fundamentals| right(pct_opt(
        fd.profitability.roa_pct
    )));
    row!("Op margin", |fd: &Fundamentals| right(pct_opt(
        fd.profitability.operating_margin_pct
    )));
    row!("Net margin", |fd: &Fundamentals| right(pct_opt(
        fd.profitability.net_margin_pct
    )));
    row!("Debt/equity", |fd: &Fundamentals| right(ratio_x(
        fd.health.debt_to_equity
    )));
    row!("Rev growth", |fd: &Fundamentals| signed(
        f::pct(fd.growth.revenue_growth_pct),
        fd.growth.revenue_growth_pct,
        color
    ));
    row!("EPS growth", |fd: &Fundamentals| signed(
        f::pct(fd.growth.earnings_growth_pct),
        fd.growth.earnings_growth_pct,
        color
    ));
    row!("Div yield", |fd: &Fundamentals| right(pct_opt(
        fd.dividend.yield_pct
    )));
    row!("Payout", |fd: &Fundamentals| right(pct_opt(
        fd.dividend.payout_ratio_pct
    )));
    row!("Beta", |fd: &Fundamentals| right(f::ratio(fd.beta)));
    row!("Target (mean)", |fd: &Fundamentals| right(f::price(
        fd.analyst.target_mean
    )));
    row!("Upside", |fd: &Fundamentals| {
        let up = match (fd.price, fd.analyst.target_mean) {
            (Some(p), Some(t)) if p > 0.0 => Some((t - p) / p * 100.0),
            _ => None,
        };
        signed(f::pct(up), up, color)
    });
    row!("Rating", |fd: &Fundamentals| right(
        fd.analyst
            .recommendation
            .as_deref()
            .map(title_case)
            .unwrap_or_else(|| f::DASH.into())
    ));
    row!("Verdict", |fd: &Fundamentals| verdict_cell(fd, color));

    let mut out = t.to_string();
    out.push('\n');
    out.push_str("Verdicts are heuristics from public ratios, not investment advice.\n");
    out
}

fn money(v: Option<f64>) -> Cell {
    right(f::compact(v))
}

pub struct StatementFlags {
    pub income: bool,
    pub balance: bool,
    pub cash_flow: bool,
}

pub fn financials(fin: &Financials, flags: &StatementFlags, color: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} · {} statements · {} periods · {}\n",
        fin.symbol,
        fin.period,
        fin.periods.len(),
        fin.currency.clone().unwrap_or_default()
    ));
    let periods: &[FinancialPeriod] = &fin.periods;
    let mut cols: Vec<String> = vec!["".into()];
    for p in periods {
        cols.push(match fin.period {
            StatementPeriod::Annual => format!("FY{}", p.end_date.format("%Y")),
            _ => p.end_date.format("%Y-%m-%d").to_string(),
        });
    }
    let col_refs: Vec<&str> = cols.iter().map(String::as_str).collect();

    let mut t = base_table(color);
    t.set_header(header(&col_refs));
    macro_rules! line {
        ($label:expr, $get:expr) => {{
            let vals: Vec<Option<f64>> = periods.iter().map($get).collect();
            if vals.iter().any(Option::is_some) {
                let mut cells = vec![Cell::new($label)];
                cells.extend(vals.into_iter().map(money));
                t.add_row(cells);
            }
        }};
    }
    macro_rules! sect {
        ($name:expr) => {{
            let mut cells = vec![section(color, $name)];
            cells.extend(periods.iter().map(|_| Cell::new("")));
            t.add_row(cells);
        }};
    }
    if flags.income {
        sect!("income statement");
        line!("Total revenue", |p: &FinancialPeriod| p
            .income
            .total_revenue);
        line!("Gross profit", |p: &FinancialPeriod| p.income.gross_profit);
        line!("Operating income", |p: &FinancialPeriod| p
            .income
            .operating_income);
        line!("EBITDA", |p: &FinancialPeriod| p.income.ebitda);
        line!("EBIT", |p: &FinancialPeriod| p.income.ebit);
        line!("Interest income", |p: &FinancialPeriod| p
            .income
            .interest_income);
        line!("Interest expense", |p: &FinancialPeriod| p
            .income
            .interest_expense);
        line!("Net interest income", |p: &FinancialPeriod| p
            .income
            .net_interest_income);
        line!("Pretax income", |p: &FinancialPeriod| p
            .income
            .pretax_income);
        line!("Tax", |p: &FinancialPeriod| p.income.tax_provision);
        line!("Net income", |p: &FinancialPeriod| p.income.net_income);
        {
            let vals: Vec<Option<f64>> = periods.iter().map(|p| p.income.eps_diluted).collect();
            if vals.iter().any(Option::is_some) {
                let mut cells = vec![Cell::new("EPS (diluted)")];
                cells.extend(vals.into_iter().map(|v| right(f::price(v))));
                t.add_row(cells);
            }
        }
    }
    if flags.balance && fin.period != StatementPeriod::Ttm {
        sect!("balance sheet");
        line!("Total assets", |p: &FinancialPeriod| p.balance.total_assets);
        line!("Total liabilities", |p: &FinancialPeriod| p
            .balance
            .total_liabilities);
        line!("Shareholders' equity", |p: &FinancialPeriod| p
            .balance
            .stockholders_equity);
        line!("Cash & equivalents", |p: &FinancialPeriod| p
            .balance
            .cash_and_equivalents);
        line!("Total debt", |p: &FinancialPeriod| p.balance.total_debt);
        line!("Net debt", |p: &FinancialPeriod| p.balance.net_debt);
        line!("Shares outstanding", |p: &FinancialPeriod| p
            .balance
            .shares_outstanding);
    }
    if flags.cash_flow {
        sect!("cash flow");
        line!("Operating", |p: &FinancialPeriod| p.cash_flow.operating);
        line!("Investing", |p: &FinancialPeriod| p.cash_flow.investing);
        line!("Financing", |p: &FinancialPeriod| p.cash_flow.financing);
        line!("Capital expenditure", |p: &FinancialPeriod| p
            .cash_flow
            .capital_expenditure);
        line!("Free cash flow", |p: &FinancialPeriod| p
            .cash_flow
            .free_cash_flow);
        line!("Dividends paid", |p: &FinancialPeriod| p
            .cash_flow
            .dividends_paid);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    out.push_str(&format!(
        "Values in {} as reported (T = trillion, B = billion). Source: {}\n",
        fin.currency
            .clone()
            .unwrap_or_else(|| "source currency".into()),
        fin.source
    ));
    out
}

/// `0q` -> "this quarter (0q)" and so on; other labels as the source wrote them.
fn estimate_label(period: &str) -> String {
    let name = match period {
        "0q" => "this quarter",
        "+1q" => "next quarter",
        "0y" => "this year",
        "+1y" => "next year",
        other => return other.to_string(),
    };
    format!("{name} ({period})")
}

/// Analyst consensus per period: EPS average with its range and analyst
/// count, growth on a year earlier, and the revenue consensus.
fn estimates_table(rows: &[EarningsEstimate], color: bool) -> String {
    let mut t = base_table(color);
    t.set_header(header(&[
        "Consensus",
        "Period end",
        "EPS avg",
        "EPS range",
        "Analysts",
        "EPS growth",
        "Revenue avg",
        "Revenue growth",
    ]));
    for r in rows {
        let range = match (r.eps_low, r.eps_high) {
            (None, None) => f::DASH.to_string(),
            (lo, hi) => format!("{} – {}", f::price(lo), f::price(hi)),
        };
        t.add_row(vec![
            Cell::new(estimate_label(&r.period)).add_attribute(Attribute::Bold),
            Cell::new(date_opt(r.end_date)),
            right(f::price(r.eps_avg)),
            right(range),
            right(
                r.eps_analysts
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| f::DASH.into()),
            ),
            signed(f::pct(r.eps_growth_pct), r.eps_growth_pct, color),
            money(r.revenue_avg),
            signed(f::pct(r.revenue_growth_pct), r.revenue_growth_pct, color),
        ]);
    }
    let mut out = t.to_string();
    out.push('\n');
    out
}

pub fn earnings(e: &Earnings, color: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} · {} · EPS in {}\n",
        e.symbol,
        e.name.clone().unwrap_or_else(|| f::DASH.into()),
        e.currency.clone().unwrap_or_default()
    ));
    if !e.quarters.is_empty() {
        let mut t = base_table(color);
        t.set_header(header(&[
            "Quarter",
            "Period end",
            "Reported",
            "EPS actual",
            "EPS est.",
            "Surprise",
        ]));
        for q in &e.quarters {
            t.add_row(vec![
                Cell::new(&q.period).add_attribute(Attribute::Bold),
                Cell::new(date_opt(q.end_date)),
                Cell::new(date_opt(q.reported_date)),
                right(f::price(q.eps_actual)),
                right(f::price(q.eps_estimate)),
                signed(f::pct(q.surprise_pct), q.surprise_pct, color),
            ]);
        }
        out.push_str(&t.to_string());
        out.push('\n');
    }
    if !e.yearly.is_empty() || !e.quarterly.is_empty() {
        let mut t = base_table(color);
        t.set_header(header(&["Period", "Revenue", "Earnings", "Margin"]));
        for r in e.yearly.iter().chain(e.quarterly.iter()) {
            let margin = r
                .profit_margin_pct
                .or_else(|| match (r.revenue, r.earnings) {
                    (Some(rev), Some(earn)) if rev != 0.0 => Some(earn / rev * 100.0),
                    _ => None,
                });
            t.add_row(vec![
                Cell::new(&r.period).add_attribute(Attribute::Bold),
                money(r.revenue),
                money(r.earnings),
                right(pct_opt(margin)),
            ]);
        }
        out.push_str(&t.to_string());
        out.push('\n');
    }
    if !e.estimates.is_empty() {
        out.push_str(&estimates_table(&e.estimates, color));
    }
    if let Some(n) = &e.next {
        out.push_str(&format!(
            "Next earnings {}{} · EPS consensus {} (range {} – {}, {} analysts) · revenue consensus {}\n",
            date_opt(n.date),
            if n.date_is_estimate { " (estimated)" } else { "" },
            f::price(n.eps_avg),
            f::price(n.eps_low),
            f::price(n.eps_high),
            n.analysts
                .map(|a| a.to_string())
                .unwrap_or_else(|| f::DASH.into()),
            f::compact(n.revenue_avg),
        ));
    }
    if e.ex_dividend_date.is_some() || e.dividend_date.is_some() {
        out.push_str(&format!(
            "Ex-dividend {} · payment {}\n",
            date_opt(e.ex_dividend_date),
            date_opt(e.dividend_date)
        ));
    }
    out.push_str(&format!("Source: {}\n", e.source));
    out
}

pub fn brief(b: &Brief, color: bool) -> String {
    let mut out = format!(
        "{}{} · brief · {}\n",
        b.symbol,
        b.name
            .as_ref()
            .map(|n| format!(" · {n}"))
            .unwrap_or_default(),
        b.generated_at.format("%Y-%m-%d %H:%M UTC")
    );
    for k in &b.key_points {
        out.push_str(&format!("• {k}\n"));
    }
    if let Some(f) = &b.fundamentals {
        out.push_str(&format!(
            "Fundamentals · {} / {} · P/E {} · fwd {} · P/B {} · EV/EBITDA {} · ROE {} · net margin {} · D/E {} · yield {}\n",
            f::text(&f.sector),
            f::text(&f.industry),
            f::ratio(f.valuation.pe_trailing),
            f::ratio(f.valuation.pe_forward),
            f::ratio(f.valuation.price_to_book),
            f::ratio(f.valuation.ev_to_ebitda),
            pct_opt(f.profitability.roe_pct),
            pct_opt(f.profitability.net_margin_pct),
            ratio_x(f.health.debt_to_equity),
            pct_opt(f.dividend.yield_pct),
        ));
        if let Some(v) = &f.verdict {
            for r in v.reasons.iter().take(4) {
                out.push_str(&format!("    - {r}\n"));
            }
        }
    }
    if let Some(t) = &b.technical {
        let ma = &t.moving_averages;
        out.push_str(&format!(
            "Technical · RSI {} · MACD hist {} · SMA50 {} · SMA200 {} · 52w {} – {} · support {} · resistance {}\n",
            f::ratio(t.rsi14),
            t.macd
                .as_ref()
                .map(|m| f::num(m.histogram, 2))
                .unwrap_or_else(|| f::DASH.into()),
            f::price(ma.sma50),
            f::price(ma.sma200),
            f::price(t.low_52w),
            f::price(t.high_52w),
            t.levels
                .support
                .iter()
                .map(|v| f::price(Some(*v)))
                .collect::<Vec<_>>()
                .join("/"),
            t.levels
                .resistance
                .iter()
                .map(|v| f::price(Some(*v)))
                .collect::<Vec<_>>()
                .join("/"),
        ));
        for r in t.signal.reasons.iter().take(3) {
            out.push_str(&format!("    - {r}\n"));
        }
    }
    if let Some(r) = &b.returns {
        let w = &r.returns;
        out.push_str(&format!(
            "Returns · 1m {} · 3m {} · 6m {} · YTD {} · 1y {} · 3y {} · 5y {} · vol 1y {} · max DD 1y {}\n",
            f::pct(w.m1),
            f::pct(w.m3),
            f::pct(w.m6),
            f::pct(w.ytd),
            f::pct(w.y1),
            f::pct(w.y3),
            f::pct(w.y5),
            pct_opt(r.volatility_1y_pct),
            f::pct(r.max_drawdown_1y.as_ref().map(|d| d.pct)),
        ));
        if let Some(bm) = &r.benchmark {
            let e = &bm.excess;
            out.push_str(&format!(
                "    excess vs {} · 1m {} · 3m {} · YTD {} · 1y {} · 3y {} pp\n",
                bm.symbol,
                f::pct(e.m1),
                f::pct(e.m3),
                f::pct(e.ytd),
                f::pct(e.y1),
                f::pct(e.y3),
            ));
        }
    }
    if let Some(o) = &b.ownership {
        let mut t = base_table(color);
        t.set_header(header(&["Holder", "Type", "L/A", "%"]));
        for h in &o.top_holders {
            t.add_row(vec![
                Cell::new(f::clip(&h.holder, 44)).add_attribute(Attribute::Bold),
                Cell::new(f::clip(&f::text(&h.investor_type), 16)),
                Cell::new(f::text(&h.local_foreign)),
                right(format!("{}%", f::num(h.pct, 2))),
            ]);
        }
        out.push_str(&format!(
            "Holders above 1% as of {} · {} listed = {}% · foreign {}%\n",
            o.as_of,
            o.holders_listed,
            f::num(o.listed_pct, 2),
            f::num(o.foreign_pct, 2)
        ));
        out.push_str(&t.to_string());
        out.push('\n');
    }
    if let Some(n) = &b.news {
        if !n.headlines.is_empty() {
            out.push_str(&format!(
                "Headlines · {} mention the company · sentiment {} ({:+.2})\n",
                n.company_matches,
                n.sentiment.as_str(),
                n.score
            ));
            for h in &n.headlines {
                out.push_str(&format!(
                    "    {} {} {}  {}\n",
                    h.published
                        .map(|d| d.format("%m-%d").to_string())
                        .unwrap_or_else(|| "--".into()),
                    if h.sentiment > 0.0 {
                        "+"
                    } else if h.sentiment < 0.0 {
                        "−"
                    } else {
                        "·"
                    },
                    f::clip(&h.title, 90),
                    h.url
                ));
            }
        }
    }
    if !b.sections_failed.is_empty() {
        out.push_str(&format!(
            "Missing sections: {} (see warnings)\n",
            b.sections_failed.join(", ")
        ));
    }
    out.push_str(
        "Verdicts, signals and sentiment are transparent rule scores, not investment advice.\n",
    );
    out
}

fn market_list(out: &mut String, title: &str, rows: &[MarketRow], value: bool, color: bool) {
    if rows.is_empty() {
        return;
    }
    out.push_str(title);
    out.push('\n');
    let mut t = base_table(color);
    let mut cols = vec!["Symbol", "Name", "Price", "Chg %"];
    cols.push(if value { "Value" } else { "Mkt Cap" });
    t.set_header(header(&cols));
    for r in rows {
        t.add_row(vec![
            Cell::new(r.symbol.as_str()).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&f::text(&r.name), 28)),
            right(f::price(r.price)),
            signed(f::pct(r.change_pct), r.change_pct, color),
            right(f::compact(if value { r.value } else { r.market_cap })),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
}

pub fn market(m: &MarketOverview, color: bool) -> String {
    let mut out = String::new();
    match &m.index {
        Some(ix) => out.push_str(&format!(
            "{} {} · {} ({}) · {}{}\n",
            ix.name.clone().unwrap_or_else(|| ix.symbol.to_string()),
            f::price(ix.price),
            f::change(ix.change),
            f::pct(ix.change_pct),
            m.market_state
                .clone()
                .unwrap_or_else(|| "state unknown".into()),
            m.as_of
                .map(|t| format!(" · last trade {}", t.format("%Y-%m-%d %H:%M WIB")))
                .unwrap_or_default()
        )),
        None => out.push_str("Index quote unavailable\n"),
    }
    out.push_str(&format!(
        "Breadth · {} advancing · {} declining · {} unchanged · {} of {} quoted · turnover {}\n",
        m.advancing,
        m.declining,
        m.unchanged,
        m.quoted,
        m.universe,
        f::compact(Some(m.turnover))
    ));
    market_list(
        &mut out,
        &format!(
            "Gainers · market cap ≥ {}",
            f::compact(Some(m.liquid_min_market_cap))
        ),
        &m.gainers,
        false,
        color,
    );
    market_list(&mut out, "Losers", &m.losers, false, color);
    market_list(
        &mut out,
        "Most traded by value",
        &m.most_active,
        true,
        color,
    );
    market_list(&mut out, "Heavyweights", &m.heavyweights, false, color);
    out.push_str("Delayed quotes as published by the source.\n");
    out
}

fn window_cells(w: &ReturnWindows, color: bool) -> Vec<Cell> {
    w.values()
        .iter()
        .map(|v| signed(f::pct(*v), *v, color))
        .collect()
}

pub fn returns(list: &[Performance], color: bool) -> String {
    let Some(first) = list.first() else {
        return "No data.\n".to_string();
    };
    let mut out = format!(
        "Price returns · as of {} · five years of daily closes · dividends excluded\n",
        first.as_of
    );
    let mut cols = vec!["Symbol", "Price"];
    cols.extend(ReturnWindows::LABELS.iter().map(|l| match *l {
        "ytd" => "YTD",
        other => other,
    }));
    cols.extend(["CAGR 3y", "Vol 1y", "MaxDD 1y"]);
    let mut t = base_table(color);
    t.set_header(header(&cols));
    for p in list {
        let mut cells = vec![
            Cell::new(p.symbol.as_str()).add_attribute(Attribute::Bold),
            right(f::price(Some(p.price))),
        ];
        cells.extend(window_cells(&p.returns, color));
        cells.push(signed(f::pct(p.cagr_3y_pct), p.cagr_3y_pct, color));
        cells.push(right(pct_opt(p.volatility_1y_pct)));
        let dd = p.max_drawdown_1y.as_ref().map(|d| d.pct);
        cells.push(signed(f::pct(dd), dd, color));
        t.add_row(cells);
    }
    if let Some(b) = list.iter().find_map(|p| p.benchmark.as_ref()) {
        let mut cells = vec![
            Cell::new(format!(
                "{}{}",
                b.symbol,
                b.name
                    .as_ref()
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default()
            ))
            .add_attribute(Attribute::Italic),
            right(f::DASH.to_string()),
        ];
        cells.extend(window_cells(&b.returns, color));
        cells.extend([
            right(f::DASH.into()),
            right(f::DASH.into()),
            right(f::DASH.into()),
        ]);
        t.add_row(cells);
    }
    out.push_str(&t.to_string());
    out.push('\n');

    if list.iter().any(|p| p.benchmark.is_some()) {
        let bench = list
            .iter()
            .find_map(|p| p.benchmark.as_ref())
            .map(|b| b.symbol.to_string())
            .unwrap_or_default();
        out.push_str(&format!("Excess over {bench} · percentage points\n"));
        let mut cols = vec!["Symbol"];
        cols.extend(ReturnWindows::LABELS.iter().map(|l| match *l {
            "ytd" => "YTD",
            other => other,
        }));
        let mut t = base_table(color);
        t.set_header(header(&cols));
        for p in list {
            let Some(b) = &p.benchmark else { continue };
            let mut cells = vec![Cell::new(p.symbol.as_str()).add_attribute(Attribute::Bold)];
            cells.extend(window_cells(&b.excess, color));
            t.add_row(cells);
        }
        out.push_str(&t.to_string());
        out.push('\n');
    }

    for p in list {
        let dd = |d: &Option<crate::model::Drawdown>| {
            d.as_ref()
                .map(|d| {
                    format!(
                        "{} ({} → {}{})",
                        f::pct(Some(d.pct)),
                        d.peak_date,
                        d.trough_date,
                        if d.recovered { ", recovered" } else { "" }
                    )
                })
                .unwrap_or_else(|| f::DASH.into())
        };
        let day = |m: &Option<crate::model::DayMove>| {
            m.as_ref()
                .map(|m| format!("{} on {}", f::pct(Some(m.pct)), m.date))
                .unwrap_or_else(|| f::DASH.into())
        };
        out.push_str(&format!(
            "{} · 52w {} – {} ({} from high{}) · CAGR 5y {} · best day {} · worst day {} · max drawdown since {}: {}\n",
            p.symbol,
            f::price(p.low_52w),
            f::price(p.high_52w),
            f::pct(p.pct_from_52w_high),
            p.days_since_52w_high
                .map(|d| format!(", {d} days ago"))
                .unwrap_or_default(),
            f::pct(p.cagr_5y_pct),
            day(&p.best_day_1y),
            day(&p.worst_day_1y),
            p.start,
            dd(&p.max_drawdown_all),
        ));
    }
    out.push_str("Computed by idx from provider closes; a window is — when the history does not reach back that far.\n");
    out
}

pub fn dividends(h: &DividendHistory, limit: usize, color: bool) -> String {
    let mut out = format!(
        "{}{} · {} · price {}{} · TTM dividends {} ({} payment{}) · TTM yield {} · {} consecutive year{}{}\n",
        h.symbol,
        h.name.as_ref().map(|n| format!(" · {n}")).unwrap_or_default(),
        h.currency.as_deref().unwrap_or("IDR"),
        f::price(h.price),
        h.price_date.map(|d| format!(" ({d})")).unwrap_or_default(),
        h.ttm_total.map(|v| f::num(v, 2)).unwrap_or_else(|| f::DASH.into()),
        h.ttm_payments,
        if h.ttm_payments == 1 { "" } else { "s" },
        pct_opt(h.ttm_yield_pct),
        h.consecutive_years,
        if h.consecutive_years == 1 { "" } else { "s" },
        h.last
            .as_ref()
            .map(|d| format!(" · last {} {}", d.date, f::num(d.amount, 2)))
            .unwrap_or_default(),
    );
    if h.dividends.is_empty() {
        out.push_str("No dividend events reported by the source.\n");
        return out;
    }
    out.push_str("Yearly totals (ex-date basis, per share)\n");
    let mut t = base_table(color);
    t.set_header(header(&["Year", "Payments", "Total", "vs prev"]));
    let years: Vec<_> = h.years.iter().rev().take(10).collect();
    for y in years.into_iter().rev() {
        t.add_row(vec![
            Cell::new(y.year).add_attribute(Attribute::Bold),
            right(y.payments.to_string()),
            right(f::num(y.total, 2)),
            signed(f::pct(y.change_pct), y.change_pct, color),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    let shown = limit.min(h.dividends.len());
    out.push_str(&format!(
        "Payments · last {shown} of {}\n",
        h.dividends.len()
    ));
    let mut t = base_table(color);
    t.set_header(header(&["Ex-date", "Amount"]));
    for d in h.dividends.iter().rev().take(shown) {
        t.add_row(vec![Cell::new(d.date), right(f::num(d.amount, 2))]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    if !h.splits.is_empty() {
        out.push_str(&format!(
            "Splits · {}\n",
            h.splits
                .iter()
                .map(|s| format!("{} {}", s.date, s.ratio))
                .collect::<Vec<_>>()
                .join(" · ")
        ));
    }
    out.push_str(
        "Amounts as reported by the source (split-adjusted on Yahoo). TTM yield = TTM total / last price, computed by idx.\n",
    );
    out
}

fn signal_cell(t: &Technical, color: bool) -> Cell {
    let c = Cell::new(format!(
        "{} (score {:+})",
        t.signal.label.as_str().to_uppercase(),
        t.signal.score
    ))
    .add_attribute(Attribute::Bold);
    if !color {
        return c;
    }
    match t.signal.label {
        SignalLabel::Buy => c.fg(Color::Green),
        SignalLabel::Sell => c.fg(Color::Red),
        SignalLabel::Neutral => c.fg(Color::Yellow),
    }
}

pub fn technical(t: &Technical, color: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} · {} · {} candles ({}) · as of {}\n",
        t.symbol,
        t.name.clone().unwrap_or_else(|| f::DASH.into()),
        t.candles_used,
        t.period,
        t.as_of.format("%Y-%m-%d")
    ));
    out.push_str(&format!(
        "Price {} · 1d {} · 1w {} · 1m {} · 3m {} · 52w {} – {}\n",
        f::price(Some(t.price)),
        f::pct(t.change_pct_1d),
        f::pct(t.change_pct_1w),
        f::pct(t.change_pct_1m),
        f::pct(t.change_pct_3m),
        f::price(t.low_52w),
        f::price(t.high_52w)
    ));
    let ma = &t.moving_averages;
    let vs = |v: Option<f64>| -> Cell {
        match v {
            Some(x) => {
                let diff = if x != 0.0 {
                    Some((t.price - x) / x * 100.0)
                } else {
                    None
                };
                signed(
                    format!("{} ({})", f::price(Some(x)), f::pct(diff)),
                    diff,
                    color,
                )
            }
            None => right(f::DASH.into()),
        }
    };
    let trend_cell = {
        let c = Cell::new(t.trend.as_str());
        if color {
            match t.trend {
                Trend::Uptrend => c.fg(Color::Green),
                Trend::Downtrend => c.fg(Color::Red),
                _ => c,
            }
        } else {
            c
        }
    };
    let rows: Vec<(String, Cell)> = vec![
        ("Trend".into(), trend_cell),
        ("Signal".into(), signal_cell(t, color)),
        ("".into(), section(color, "momentum")),
        ("RSI (14)".into(), right(f::ratio(t.rsi14))),
        (
            "MACD line / signal / hist".into(),
            match &t.macd {
                Some(m) => signed(
                    format!(
                        "{} / {} / {}",
                        f::num(m.line, 2),
                        f::num(m.signal, 2),
                        f::num(m.histogram, 2)
                    ),
                    Some(m.histogram),
                    color,
                ),
                None => right(f::DASH.into()),
            },
        ),
        ("ATR (14)".into(), right(f::price(t.atr14))),
        ("".into(), section(color, "moving averages (price vs)")),
        ("SMA 20".into(), vs(ma.sma20)),
        ("SMA 50".into(), vs(ma.sma50)),
        ("SMA 200".into(), vs(ma.sma200)),
        ("EMA 20".into(), vs(ma.ema20)),
        ("EMA 50".into(), vs(ma.ema50)),
        ("EMA 200".into(), vs(ma.ema200)),
        ("".into(), section(color, "bollinger (20, 2 sd)")),
        (
            "Lower / middle / upper".into(),
            right(match &t.bollinger {
                Some(b) => format!(
                    "{} / {} / {}",
                    f::price(Some(b.lower)),
                    f::price(Some(b.middle)),
                    f::price(Some(b.upper))
                ),
                None => f::DASH.into(),
            }),
        ),
        (
            "%B / bandwidth".into(),
            right(match &t.bollinger {
                Some(b) => format!(
                    "{} / {}%",
                    f::num(b.percent_b, 2),
                    f::num(b.bandwidth_pct, 1)
                ),
                None => f::DASH.into(),
            }),
        ),
        ("".into(), section(color, "volume")),
        (
            "Last / 20d avg".into(),
            right(format!(
                "{} / {} ({})",
                f::int(t.volume.last),
                f::int(t.volume.avg20),
                t.volume
                    .ratio
                    .map(|r| format!("{}x", f::num(r, 2)))
                    .unwrap_or_else(|| f::DASH.into())
            )),
        ),
        ("".into(), section(color, "levels")),
        (
            "Support".into(),
            right(if t.levels.support.is_empty() {
                f::DASH.into()
            } else {
                t.levels
                    .support
                    .iter()
                    .map(|v| f::price(Some(*v)))
                    .collect::<Vec<_>>()
                    .join(" · ")
            }),
        ),
        (
            "Resistance".into(),
            right(if t.levels.resistance.is_empty() {
                f::DASH.into()
            } else {
                t.levels
                    .resistance
                    .iter()
                    .map(|v| f::price(Some(*v)))
                    .collect::<Vec<_>>()
                    .join(" · ")
            }),
        ),
    ];
    out.push_str(&kv_table(color, rows).to_string());
    out.push('\n');
    out.push_str("Why:\n");
    for r in &t.signal.reasons {
        out.push_str(&format!("  • {r}\n"));
    }
    out.push_str("Signal is a rule-based read of daily candles, not investment advice.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fundamentals, Symbol};

    #[test]
    fn wrap_breaks_lines() {
        let w = wrap("one two three four five six", 9);
        assert_eq!(w, "one two\nthree\nfour five\nsix\n");
    }

    #[test]
    fn fundamentals_table_renders_sections_and_dashes() {
        let mut fd = Fundamentals::empty(Symbol::parse("BBCA").unwrap(), "t");
        fd.valuation.pe_trailing = Some(10.96);
        fd.profitability.roe_pct = Some(21.8);
        fd.verdict = Some(crate::analysis::fundamental::verdict(&fd));
        let s = fundamentals(&fd, false);
        assert!(s.contains("VALUATION"));
        assert!(s.contains("10.96"));
        assert!(s.contains("21.80%"));
        assert!(s.contains("insufficient data"));
        assert!(s.contains(f::DASH));
    }

    #[test]
    fn compare_table_has_one_column_per_symbol() {
        let a = Fundamentals::empty(Symbol::parse("BBCA").unwrap(), "t");
        let b = Fundamentals::empty(Symbol::parse("BBRI").unwrap(), "t");
        let s = compare(&[a, b], false);
        assert!(s.contains("BBCA"));
        assert!(s.contains("BBRI"));
        assert!(s.contains("P/E (ttm)"));
    }
}
