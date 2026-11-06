//! Table for `idx stockbit movements`. Same conventions as `table.rs`.

use comfy_table::{Attribute, Cell};

use crate::model::{HolderMovement, HolderMovements};
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

/// `+147,933`, `-10,000,000`, `0`.
fn shares(v: Option<i64>) -> String {
    match v {
        None => f::DASH.to_string(),
        Some(n) if n > 0 => format!("+{}", f::num(n as f64, 0)),
        Some(n) => f::num(n as f64, 0),
    }
}

/// A percentage as the source rounds it, up to four decimals.
fn pct(v: f64) -> String {
    let fixed = format!("{v:.4}");
    let decimals = fixed
        .split_once('.')
        .map(|(_, d)| d.trim_end_matches('0').len().max(2))
        .unwrap_or(2);
    f::num(v, decimals)
}

/// `6.66% (+0.07)`.
fn stake(m: &HolderMovement) -> String {
    let to = m.pct_to.map(|p| format!("{}%", pct(p)));
    let change = m.pct_change.map(|c| {
        if c > 0.0 {
            format!("+{}", pct(c))
        } else {
            pct(c)
        }
    });
    match (to, change) {
        (Some(t), Some(c)) => format!("{t} ({c})"),
        (Some(t), None) => t,
        (None, Some(c)) => format!("({c})"),
        (None, None) => f::DASH.to_string(),
    }
}

/// The filed figure, or the estimate marked with `~`.
fn filed_or_estimated(
    filed: Option<f64>,
    estimated: Option<f64>,
    fmt: fn(Option<f64>) -> String,
) -> String {
    match (filed, estimated) {
        (Some(_), _) => fmt(filed),
        (None, Some(_)) => format!("~{}", fmt(estimated)),
        (None, None) => f::DASH.to_string(),
    }
}

fn role(m: &HolderMovement) -> String {
    if m.roles.is_empty() {
        f::DASH.to_string()
    } else {
        m.roles.join(", ")
    }
}

pub fn holder_movements(r: &HolderMovements, color: bool) -> String {
    let scope = match &r.symbol {
        Some(s) => s.to_string(),
        None => "across IDX".to_string(),
    };
    let pages = format!(
        "{} page{}, {}",
        r.pages_read,
        if r.pages_read == 1 { "" } else { "s" },
        if r.more { "more remain" } else { "no more" }
    );
    let mut out = format!(
        "Shareholder movements · {scope} · {} rows ({pages})",
        r.movements.len()
    );
    let mut filters = Vec::new();
    if r.from.is_some() || r.to.is_some() {
        filters.push(format!(
            "{} to {}",
            r.from.map(|d| d.to_string()).unwrap_or_else(|| "…".into()),
            r.to.map(|d| d.to_string()).unwrap_or_else(|| "…".into())
        ));
    }
    if let Some(h) = &r.holder_id {
        filters.push(format!("holder {h}"));
    }
    if let Some(a) = r.action {
        filters.push(format!("{a:?}").to_ascii_lowercase());
    }
    if let Some(s) = r.source {
        filters.push(format!("{s:?}").to_ascii_uppercase());
    }
    if !filters.is_empty() {
        out.push_str(&format!(" · {}", filters.join(", ")));
    }
    out.push('\n');
    if r.movements.is_empty() {
        out.push_str("No movements.\n");
        return out;
    }
    let mut t = base_table(color);
    t.set_header(header(&[
        "Date",
        "Code",
        "Holder",
        "Role",
        "Action",
        "Shares",
        "Stake",
        "Price",
        "Value",
        "Src",
        "Holder ID",
    ]));
    for m in &r.movements {
        let change = signed(
            shares(m.shares_change),
            m.shares_change.map(|n| n as f64),
            color,
        );
        t.add_row(vec![
            Cell::new(m.date.to_string()),
            Cell::new(m.symbol.as_str()).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&m.holder, 36)),
            Cell::new(role(m)),
            Cell::new(m.action.as_deref().unwrap_or(f::DASH).replace('_', " ")),
            change,
            right(stake(m)),
            right(filed_or_estimated(m.price, m.estimated_price, f::price)),
            right(filed_or_estimated(m.value, m.estimated_value, f::compact)),
            Cell::new(
                m.source
                    .as_deref()
                    .map(str::to_ascii_uppercase)
                    .unwrap_or_else(|| f::DASH.to_string()),
            ),
            Cell::new(m.holder_id.as_deref().unwrap_or(f::DASH)),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    out.push_str("Source: Stockbit (your session). IDX: insider disclosures, any size, with the trade price. KSEI: holders at 5 % and above, no price filed;\n");
    out.push_str("~ marks Stockbit's estimate (estimated_price and estimated_value in JSON). Filings, not signals: a share plan reads as many insiders buying at one price.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Symbol;
    use chrono::NaiveDate;

    fn row() -> HolderMovement {
        HolderMovement {
            date: NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
            symbol: Symbol::parse("ATLA").unwrap(),
            holder: "RUDI R. SUTANTRA".into(),
            holder_id: Some("1000004481".into()),
            roles: vec![],
            foreign: Some(false),
            action: Some("sell".into()),
            source: Some("ksei".into()),
            shares_from: Some(2_530_254_200),
            shares_to: Some(2_520_254_200),
            shares_change: Some(-10_000_000),
            pct_from: Some(40.81),
            pct_to: Some(40.65),
            pct_change: Some(-0.16),
            price: None,
            estimated_price: Some(21.0),
            value: None,
            estimated_value: Some(210_000_000.0),
            purpose: None,
            broker: Some("AG".into()),
            broker_group: Some("foreign".into()),
        }
    }

    #[test]
    fn estimates_carry_a_tilde_and_stakes_their_change() {
        let mut m = row();
        let r = HolderMovements {
            symbol: None,
            from: None,
            to: None,
            holder_id: None,
            action: Some(crate::model::MovementFilter::Sell),
            source: Some(crate::model::MovementSource::Ksei),
            pages_read: 1,
            more: true,
            movements: vec![m.clone()],
        };
        let t = holder_movements(&r, false);
        assert!(
            t.starts_with(
                "Shareholder movements · across IDX · 1 rows (1 page, more remain) · sell, KSEI"
            ),
            "{t}"
        );
        for cell in [
            "-10,000,000",
            "40.65% (-0.16)",
            "~21",
            "~210.0M",
            "KSEI",
            "1000004481",
        ] {
            assert!(t.contains(cell), "{cell}: {t}");
        }
        m.pct_to = Some(0.0003);
        m.pct_change = Some(0.0001);
        assert_eq!(stake(&m), "0.0003% (+0.0001)");
        m.price = Some(6982.0);
        assert_eq!(filed_or_estimated(m.price, None, f::price), "6,982");
        assert_eq!(shares(Some(147_933)), "+147,933");
        assert_eq!(shares(Some(0)), "0");
    }
}
