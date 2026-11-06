//! Table for `stocks insights`: MSN's statements grouped by category.

use comfy_table::{Attribute, Cell, Color, Table};

use crate::model::{InsightStatus, Insights};

fn mark(s: InsightStatus, color: bool) -> Cell {
    let (text, fg) = match s {
        InsightStatus::Good => ("✓", Some(Color::Green)),
        InsightStatus::Bad => ("✗", Some(Color::Red)),
        InsightStatus::Neutral => ("=", None),
        InsightStatus::NotRated => ("·", None),
    };
    let c = Cell::new(text);
    match (color, fg, s) {
        (true, Some(fg), _) => c.fg(fg),
        (true, None, InsightStatus::NotRated) => c.add_attribute(Attribute::Dim),
        _ => c,
    }
}

fn borderless(color: bool) -> Table {
    let mut t = Table::new();
    t.load_preset(comfy_table::presets::NOTHING)
        .set_content_arrangement(comfy_table::ContentArrangement::Disabled);
    if color {
        t.enforce_styling();
    } else {
        t.force_no_tty();
    }
    t
}

/// Statements MSN could not rate are listed only with `all`.
pub fn insights(r: &Insights, all: bool, color: bool) -> String {
    let mut out = format!(
        "{}{} · MSN insights{}\n",
        r.symbol,
        r.name
            .as_deref()
            .map(|n| format!(" · {n}"))
            .unwrap_or_default(),
        r.updated
            .map(|u| format!(", updated {}", u.format("%Y-%m-%d %H:%M UTC")))
            .unwrap_or_default()
    );
    out.push_str(&format!(
        "{} good · {} bad · {} neutral · {} not rated{}\n",
        r.good,
        r.bad,
        r.neutral,
        r.not_rated,
        if r.peers.is_empty() {
            String::new()
        } else {
            let list: Vec<&str> = r.peers.iter().map(|s| s.as_str()).collect();
            format!(" · peers {}", list.join(", "))
        }
    ));
    let mut categories: Vec<&str> = Vec::new();
    for i in &r.insights {
        if !categories.contains(&i.category.as_str()) {
            categories.push(&i.category);
        }
    }
    for cat in categories {
        let rows: Vec<_> = r
            .insights
            .iter()
            .filter(|i| i.category == cat && (all || i.status != InsightStatus::NotRated))
            .collect();
        if rows.is_empty() {
            continue;
        }
        out.push('\n');
        out.push_str(&cat.to_uppercase());
        out.push('\n');
        let mut t = borderless(color);
        for i in rows {
            let text = i
                .statement
                .as_deref()
                .or(i.short_statement.as_deref())
                .unwrap_or(&i.name);
            t.add_row(vec![mark(i.status, color), Cell::new(text)]);
        }
        for line in t.to_string().lines() {
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    if r.insights.is_empty() {
        out.push_str("MSN lists no statements for this company.\n");
    }
    out.push('\n');
    if !all && r.not_rated > 0 {
        out.push_str(&format!(
            "{} statement{} MSN could not rate {} hidden; --all lists {}.\n",
            r.not_rated,
            if r.not_rated == 1 { "" } else { "s" },
            if r.not_rated == 1 { "is" } else { "are" },
            if r.not_rated == 1 { "it" } else { "them" },
        ));
    }
    out.push_str(
        "Rule-based statements by MSN Money (✓ good, ✗ bad, = neutral, · not rated), measured against\nMSN's peers, the industry, the company's own history or the price. Not investment advice.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Insight, InsightBenchmark, Symbol};

    fn item(cat: &str, status: InsightStatus, text: &str) -> Insight {
        Insight {
            category: cat.into(),
            name: "n".into(),
            statement: Some(text.into()),
            short_statement: None,
            status,
            criteria: None,
            value: None,
            benchmark_value: None,
            compared_with: Some(InsightBenchmark::Peers),
        }
    }

    fn report() -> Insights {
        let mut r = Insights {
            symbol: Symbol::parse("BBCA").unwrap(),
            name: Some("Bank Central Asia Tbk PT".into()),
            updated: None,
            good: 0,
            bad: 0,
            neutral: 0,
            not_rated: 0,
            peers: vec![Symbol::parse("BBKP").unwrap()],
            insights: vec![
                item("Valuation", InsightStatus::Bad, "P/B above peers"),
                item("Valuation", InsightStatus::NotRated, "no earnings data"),
                item("Health", InsightStatus::NotRated, "no current ratio"),
                item("Earnings", InsightStatus::Good, "earnings up"),
            ],
        };
        r.recount();
        r
    }

    #[test]
    fn groups_by_category_and_hides_not_rated() {
        let s = insights(&report(), false, false);
        assert!(
            s.contains("1 good · 1 bad · 0 neutral · 2 not rated · peers BBKP"),
            "{s}"
        );
        assert!(s.contains("VALUATION") && s.contains("EARNINGS"), "{s}");
        assert!(
            !s.contains("HEALTH"),
            "a category with only unrated items is left out: {s}"
        );
        assert!(!s.contains("no earnings data"), "{s}");
        assert!(
            s.contains("2 statements MSN could not rate are hidden"),
            "{s}"
        );
        assert!(s.find("VALUATION").unwrap() < s.find("EARNINGS").unwrap());
        assert!(!s.contains("\x1b["));

        let s = insights(&report(), true, false);
        assert!(
            s.contains("HEALTH") && s.contains("no earnings data"),
            "{s}"
        );
        assert!(!s.contains("hidden"), "{s}");
        assert!(insights(&report(), false, true).contains("\x1b["));
    }
}
