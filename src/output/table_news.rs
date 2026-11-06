//! Tables for `stocks screen`, `stocks news` and `stocks sentiment`.

use comfy_table::{Attribute, Cell, Color};

use crate::analysis::screen::{split_clause, Key};
use crate::model::{
    NewsItem, NewsReport, ScreenReport, ScreenRow, SentimentLabel, SentimentReport,
};
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

fn pct_opt(v: Option<f64>) -> String {
    v.map(|x| format!("{}%", f::num(x, 2)))
        .unwrap_or_else(|| f::DASH.into())
}

/// Keys of the filter clauses, in order.
fn filter_keys(r: &ScreenReport) -> Vec<Key> {
    r.filters
        .iter()
        .filter_map(|c| split_clause(c).ok().and_then(|(k, _, _)| Key::parse(k)))
        .collect()
}

/// The return window to show: the sort key's, else the first one filtered
/// on, else one year.
fn return_column(r: &ScreenReport) -> (&'static str, fn(&ScreenRow) -> Option<f64>) {
    let sort = Key::parse(&r.sort).filter(Key::needs_returns);
    let key = sort
        .or_else(|| filter_keys(r).into_iter().find(Key::needs_returns))
        .unwrap_or(Key::Ret1y);
    match key {
        Key::Ret1w => ("1W %", |x| x.return_1w_pct),
        Key::Ret1m => ("1M %", |x| x.return_1m_pct),
        Key::Ret3m => ("3M %", |x| x.return_3m_pct),
        Key::Ret6m => ("6M %", |x| x.return_6m_pct),
        Key::RetYtd => ("YTD %", |x| x.return_ytd_pct),
        _ => ("1Y %", |x| x.return_1y_pct),
    }
}

pub fn screen(r: &ScreenReport, color: bool) -> String {
    let what = match (&r.preset, r.filters.is_empty()) {
        (Some(p), true) => p.clone(),
        (Some(p), false) => format!("{p}: {}", r.filters.join(", ")),
        (None, true) => "(no filters)".to_string(),
        (None, false) => r.filters.join(", "),
    };
    let mut out = format!(
        "Screen {} · {} of {} tickers quoted · {} passed quote filters{} · {} match{}\n",
        what,
        r.quoted,
        r.universe,
        r.matched_quote_level,
        if r.fundamentals_checked > 0 {
            format!(
                " · fundamentals checked for {}{}",
                r.fundamentals_checked,
                if r.fundamentals_capped {
                    " (capped; raise --max-fundamentals)"
                } else {
                    ""
                }
            )
        } else {
            String::new()
        },
        r.total_matches,
        if r.total_matches == 1 { "" } else { "es" }
    );
    let with_f = r.rows.iter().any(|x| x.fundamentals_checked);
    let ret = r.returns_source.as_ref().map(|_| return_column(r));
    let with_range =
        Key::parse(&r.sort) == Some(Key::Range) || filter_keys(r).contains(&Key::Range);
    let mut cols = vec![
        "Symbol", "Name", "Price", "Chg %", "Mkt Cap", "P/E", "P/B", "Div %",
    ];
    if let Some((label, _)) = ret {
        cols.push(label);
    }
    if with_range {
        cols.push("52w pos");
    }
    if with_f {
        cols.extend(["ROE", "Net mgn", "D/E", "Sector"]);
    }
    let mut t = base_table(color);
    t.set_header(header(&cols));
    for x in &r.rows {
        let mut cells = vec![
            Cell::new(x.symbol.as_str()).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&f::text(&x.name), 26)),
            right(f::price(x.price)),
            signed(f::pct(x.change_pct), x.change_pct, color),
            right(f::compact(x.market_cap)),
            right(f::ratio(x.pe_trailing)),
            right(f::ratio(x.price_to_book)),
            right(pct_opt(x.dividend_yield_pct)),
        ];
        if let Some((_, get)) = ret {
            cells.push(signed(f::pct(get(x)), get(x), color));
        }
        if with_range {
            cells.push(right(
                x.range_52w_pct
                    .map(|v| f::num(v, 1))
                    .unwrap_or_else(|| f::DASH.into()),
            ));
        }
        if with_f {
            cells.push(right(pct_opt(x.roe_pct)));
            cells.push(right(pct_opt(x.net_margin_pct)));
            cells.push(right(
                x.debt_to_equity
                    .map(|d| format!("{}x", f::num(d, 2)))
                    .unwrap_or_else(|| f::DASH.into()),
            ));
            cells.push(Cell::new(f::clip(&f::text(&x.sector), 20)));
        }
        t.add_row(cells);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    if r.rows.len() < r.total_matches {
        out.push_str(&format!(
            "Showing {} of {} (use --limit 0 for all)\n",
            r.rows.len(),
            r.total_matches
        ));
    }
    out.push_str(&format!(
        "Sorted by {} {}. Filter keys: pe fpe pb ps peg mcap price div eps volume chg range ret1w ret1m ret3m ret6m retytd ret1y roe roa margin opmargin de growth epsgrowth sector industry.\n",
        r.sort,
        if r.descending { "desc" } else { "asc" },
    ));
    if let Some(src) = &r.returns_source {
        out.push_str(&format!(
            "Return windows: price return % as reported by {src}.\n"
        ));
    }
    if with_range {
        out.push_str(
            "52w pos: where the price sits in its 52-week range, 0 at the low, 100 at the high.\n",
        );
    }
    out
}

fn sentiment_cell(v: f64, color: bool) -> Cell {
    let label = if v > 0.0 {
        "+"
    } else if v < 0.0 {
        "−"
    } else {
        "·"
    };
    let c = Cell::new(label);
    if !color {
        return c;
    }
    if v > 0.0 {
        c.fg(Color::Green)
    } else if v < 0.0 {
        c.fg(Color::Red)
    } else {
        c
    }
}

fn item_rows(t: &mut comfy_table::Table, items: &[&NewsItem], color: bool) {
    for it in items {
        t.add_row(vec![
            Cell::new(
                it.published
                    .map(|d| d.format("%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| f::DASH.into()),
            ),
            Cell::new(&it.source),
            sentiment_cell(it.sentiment, color),
            Cell::new(f::clip(&it.title, 78)),
        ]);
    }
}

pub fn news(r: &NewsReport, color: bool) -> String {
    let company: Vec<&NewsItem> = r.items.iter().filter(|i| i.scope == "company").collect();
    let market: Vec<&NewsItem> = r.items.iter().filter(|i| i.scope != "company").collect();
    let mut out = match &r.symbol {
        Some(s) => format!(
            "News for {}{} · {} of {} scanned headlines mention the company{}{} · sources: {}\n",
            s,
            r.matched_name
                .as_ref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default(),
            r.company_matches,
            r.scanned,
            if company.len() < r.company_matches {
                format!(" (showing {})", company.len())
            } else {
                String::new()
            },
            if r.company_only {
                " · market context omitted"
            } else {
                ""
            },
            r.sources.join(", ")
        ),
        None => format!(
            "Market headlines · {} shown of {} · sources: {}\n",
            r.items.len(),
            r.scanned,
            r.sources.join(", ")
        ),
    };
    let cols = ["WIB", "Source", "±", "Headline"];
    if r.symbol.is_some() {
        if company.is_empty() {
            out.push_str("No headline mentions the company in the current feeds.\n");
        } else {
            let mut t = base_table(color);
            t.set_header(header(&cols));
            item_rows(&mut t, &company, color);
            out.push_str(&t.to_string());
            out.push('\n');
        }
        if !market.is_empty() {
            out.push_str(&format!("Market context · {} headlines\n", market.len()));
            let mut t = base_table(color);
            t.set_header(header(&cols));
            item_rows(&mut t, &market, color);
            out.push_str(&t.to_string());
            out.push('\n');
        }
    } else {
        let mut t = base_table(color);
        t.set_header(header(&cols));
        item_rows(&mut t, &market, color);
        out.push_str(&t.to_string());
        out.push('\n');
    }
    for it in company.iter().chain(market.iter()) {
        out.push_str(&format!("  {}\n", it.url));
    }
    out.push_str(
        "± is a lexicon read of the headline, not a recommendation. Links above, in table order.\n",
    );
    out
}

pub fn sentiment(r: &SentimentReport, color: bool) -> String {
    let label = Cell::new(r.label.as_str().to_uppercase()).add_attribute(Attribute::Bold);
    let label = if color {
        match r.label {
            SentimentLabel::Positive => label.fg(Color::Green),
            SentimentLabel::Negative => label.fg(Color::Red),
            _ => label,
        }
    } else {
        label
    };
    let mut out = format!(
        "{}{} · sentiment {} (score {:+.2}) · {} headlines: {} positive, {} negative, {} neutral{}\n",
        r.symbol,
        r.matched_name
            .as_ref()
            .map(|n| format!(" ({n})"))
            .unwrap_or_default(),
        label.content(),
        r.score,
        r.items,
        r.positive,
        r.negative,
        r.neutral,
        match (r.oldest, r.newest) {
            (Some(a), Some(b)) => format!(" · {} → {}", a.format("%Y-%m-%d"), b.format("%Y-%m-%d")),
            _ => String::new(),
        }
    );
    if !r.top_positive_terms.is_empty() || !r.top_negative_terms.is_empty() {
        let fmt = |v: &Vec<(String, usize)>| {
            v.iter()
                .map(|(t, n)| format!("{t} ×{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "Terms · positive: {} · negative: {}\n",
            if r.top_positive_terms.is_empty() {
                f::DASH.to_string()
            } else {
                fmt(&r.top_positive_terms)
            },
            if r.top_negative_terms.is_empty() {
                f::DASH.to_string()
            } else {
                fmt(&r.top_negative_terms)
            },
        ));
    }
    if !r.headlines.is_empty() {
        let mut t = base_table(color);
        t.set_header(header(&["WIB", "Source", "±", "Headline"]));
        let heads: Vec<&NewsItem> = r.headlines.iter().collect();
        item_rows(&mut t, &heads, color);
        out.push_str(&t.to_string());
        out.push('\n');
    } else {
        out.push_str(
            "No matching headlines in the current feeds; try --source all or check back later.\n",
        );
    }
    out.push_str("Method: lexicon count over Indonesian and English finance terms. Indicative only, not advice.\n");
    out
}
