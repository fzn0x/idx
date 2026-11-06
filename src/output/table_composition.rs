//! Table for `idx ownership composition`.

use comfy_table::{Attribute, Cell};

use crate::model::CompositionReport;
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

fn pct(v: Option<f64>) -> String {
    v.map(|x| format!("{}%", f::num(x, 2)))
        .unwrap_or_else(|| f::DASH.into())
}

fn shares(v: i64) -> String {
    f::num(v as f64, 0)
}

pub fn composition(r: &CompositionReport, color: bool) -> String {
    let mut out = format!(
        "{} · KSEI composition {} · price {}\n",
        r.symbol,
        r.as_of,
        f::price(r.price)
    );
    out.push_str(&format!(
        "Shares listed {} · in KSEI accounts (scripless) {} = {} of listed; the rest is held as certificates (scrip)\n",
        shares(r.shares_listed),
        shares(r.shares_scripless),
        pct(r.scripless_pct)
    ));
    out.push_str(&format!(
        "Local {} · Foreign {} of the scripless shares\n\n",
        pct(r.local_pct),
        pct(r.foreign_pct)
    ));
    let mut t = base_table(color);
    t.set_header(header(&[
        "Investor type",
        "Local shares",
        "Local %",
        "Foreign shares",
        "Foreign %",
    ]));
    for ty in &r.by_type {
        t.add_row(vec![
            Cell::new(format!("{} {}", ty.code, ty.investor_type.to_lowercase())),
            right(shares(ty.local_shares)),
            right(pct(ty.local_pct)),
            right(shares(ty.foreign_shares)),
            right(pct(ty.foreign_pct)),
        ]);
    }
    t.add_row(vec![
        Cell::new("Total").add_attribute(Attribute::Bold),
        right(shares(r.local_shares)),
        right(pct(r.local_pct)),
        right(shares(r.foreign_shares)),
        right(pct(r.foreign_pct)),
    ]);
    out.push_str(&t.to_string());
    out.push('\n');
    if r.history.len() > 1 {
        let mut h = base_table(color);
        h.set_header(header(&[
            "As of",
            "Foreign %",
            "Δ pp",
            "Local %",
            "Scripless %",
        ]));
        for p in &r.history {
            h.add_row(vec![
                Cell::new(p.as_of).add_attribute(Attribute::Bold),
                right(pct(p.foreign_pct)),
                signed(f::change(p.foreign_change_pp), p.foreign_change_pp, color),
                right(pct(p.local_pct)),
                right(pct(p.scripless_pct)),
            ]);
        }
        out.push_str("\nImported months\n");
        out.push_str(&h.to_string());
        out.push('\n');
    }
    out.push_str(
        "Percent of the shares in KSEI accounts; every holder is counted, none is named.\n",
    );
    out.push_str(&format!("Source: {}\n", r.source));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CompositionPoint, CompositionType};
    use chrono::NaiveDate;

    #[test]
    fn renders_the_split_types_and_history() {
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        let r = CompositionReport {
            symbol: "BBCA".into(),
            as_of: d(10, 30),
            price: Some(4980.0),
            shares_listed: 123_275_050_000,
            shares_scripless: 52_453_691_120,
            scripless_pct: Some(42.5501),
            local_shares: 17_194_269_088,
            foreign_shares: 35_259_422_032,
            local_pct: Some(32.7799),
            foreign_pct: Some(67.2201),
            by_type: vec![CompositionType {
                code: "ID".into(),
                investor_type: "INDIVIDUAL".into(),
                local_shares: 12_030_567_470,
                foreign_shares: 324_800_480,
                local_pct: Some(22.9356),
                foreign_pct: Some(0.6192),
            }],
            history: vec![
                CompositionPoint {
                    as_of: d(9, 30),
                    local_pct: Some(31.1),
                    foreign_pct: Some(68.9),
                    scripless_pct: Some(42.55),
                    foreign_change_pp: None,
                },
                CompositionPoint {
                    as_of: d(10, 30),
                    local_pct: Some(32.7799),
                    foreign_pct: Some(67.2201),
                    scripless_pct: Some(42.5501),
                    foreign_change_pp: Some(-1.6799),
                },
            ],
            source: "BalanceposEfek20261030.zip".into(),
        };
        let out = composition(&r, false);
        for needle in [
            "BBCA · KSEI composition 2026-10-30",
            "42.55% of listed",
            "Local 32.78% · Foreign 67.22%",
            "ID individual",
            "22.94%",
            "Total",
            "Imported months",
            "2026-09-30",
            "-1.68",
            "Source: BalanceposEfek20261030.zip",
        ] {
            assert!(out.contains(needle), "{needle} in\n{out}");
        }
    }
}
