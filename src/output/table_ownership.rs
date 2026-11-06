//! Tables for `idx ownership ...`.

use std::path::Path;

use chrono::NaiveDate;
use comfy_table::{Attribute, Cell, Color};

use crate::model::{
    ChangeKind, ChangesReport, DiscoveredFile, EntityReport, ForeignFlowReport, ForeignReport,
    HoldersReport, Holding, ImportReport, Release,
};
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

fn pct(v: f64) -> String {
    format!("{}%", f::num(v, 2))
}

fn pct_opt(v: Option<f64>) -> String {
    v.map(pct).unwrap_or_else(|| f::DASH.into())
}

fn shares(v: i64) -> String {
    f::num(v as f64, 0)
}

/// CONTROLLER_OR_AFFILIATE in a narrow column; blank when not flagged.
fn control(v: &Option<String>) -> String {
    match v.as_deref() {
        Some("CONTROLLER") => "controller".into(),
        Some("AFFILIATE OF THE CONTROLLER") => "affiliate".into(),
        Some(other) => f::clip(&other.to_lowercase(), 12),
        None => String::new(),
    }
}

/// Register lines summed into a position (blank for releases imported by an
/// old idx, which never show the column).
fn accounts(v: Option<u32>) -> String {
    v.map(|n| n.to_string()).unwrap_or_default()
}

fn several_accounts(list: &[Holding]) -> bool {
    list.iter().any(|h| h.accounts.unwrap_or(1) > 1)
}

const ACCTS_NOTE: &str = "Accts: register lines summed (KSEI lists a holder once per SID and once per scrip account without one).\n";
const CONTROL_NOTE: &str =
    "Control: CONTROLLER_OR_AFFILIATE as the issuer reported it to IDX for the previous month.\n";

pub fn holders(r: &HoldersReport, color: bool) -> String {
    let mut out = format!(
        "{} · {} · holders above 1% as of {} · {} listed = {}\n",
        r.symbol,
        r.issuer_name.clone().unwrap_or_else(|| f::DASH.into()),
        r.as_of,
        r.holders.len(),
        pct(r.listed_pct)
    );
    let show_accts = several_accounts(&r.holders);
    let show_control = r.controller_reported;
    let mut cols = vec!["#", "Holder", "Type", "L/A"];
    if show_control {
        cols.push("Control");
    }
    if show_accts {
        cols.push("Accts");
    }
    cols.extend(["Shares", "%"]);
    let mut t = base_table(color);
    t.set_header(header(&cols));
    for (i, h) in r.holders.iter().enumerate() {
        let mut row = vec![
            right((i + 1).to_string()),
            Cell::new(f::clip(&h.holder, 48)).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&f::text(&h.investor_type), 22)),
            Cell::new(f::text(&h.local_foreign)),
        ];
        if show_control {
            row.push(Cell::new(control(&h.controller_or_affiliate)));
        }
        if show_accts {
            row.push(right(accounts(h.accounts)));
        }
        row.extend([right(shares(h.shares_total)), right(pct(h.pct))]);
        t.add_row(row);
    }
    out.push_str(&t.to_string());
    out.push_str("\nSource: KSEI via IDX monthly register, imported locally. Holders below 1% are not published.\n");
    if show_accts {
        out.push_str(ACCTS_NOTE);
    }
    if show_control {
        out.push_str(CONTROL_NOTE);
    }
    out
}

pub fn foreign(r: &ForeignReport, color: bool) -> String {
    let last = r.points.last();
    let mut out = format!(
        "{} · {} · disclosed foreign stake{} · {} release{}\n",
        r.symbol,
        r.issuer_name.clone().unwrap_or_else(|| f::DASH.into()),
        last.map(|p| format!(
            " {} of the {} listed above 1% as of {}",
            pct(p.foreign_pct),
            pct(p.listed_pct),
            p.as_of
        ))
        .unwrap_or_default(),
        r.points.len(),
        if r.points.len() == 1 { "" } else { "s" }
    );
    let mut t = base_table(color);
    t.set_header(header(&[
        "As of",
        "Foreign %",
        "Δ pp",
        "Local %",
        "Unflagged %",
        "Foreign holders",
        "Local holders",
        "Listed %",
    ]));
    for p in &r.points {
        t.add_row(vec![
            Cell::new(p.as_of).add_attribute(Attribute::Bold),
            right(pct(p.foreign_pct)),
            signed(f::change(p.foreign_change_pp), p.foreign_change_pp, color),
            right(pct(p.local_pct)),
            right(pct(p.unflagged_pct)),
            right(p.foreign_holders.to_string()),
            right(p.local_holders.to_string()),
            right(pct(p.listed_pct)),
        ]);
    }
    out.push_str(&t.to_string());
    out.push_str(
        "\nSums over holders above 1% only (KSEI L/A flag); holders below 1% are not published.\n",
    );
    out
}

pub fn foreign_flow(r: &ForeignFlowReport, color: bool) -> String {
    let mut out = format!(
        "Disclosed foreign stake {} → {} · {} issuers · {} inflows · {} outflows · net {}{} pp{}\n",
        r.from,
        r.to,
        r.issuers,
        r.inflows,
        r.outflows,
        if r.net_pp > 0.0 { "+" } else { "" },
        f::num(r.net_pp, 2),
        if r.rows.len() < r.issuers {
            format!(" · showing {} (use --limit 0 for all)", r.rows.len())
        } else {
            String::new()
        }
    );
    let mut t = base_table(color);
    t.set_header(header(&[
        "Symbol",
        "Issuer",
        "Foreign % from",
        "Foreign % to",
        "Δ pp",
        "Foreign holders",
    ]));
    for x in &r.rows {
        t.add_row(vec![
            Cell::new(&x.symbol).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&f::text(&x.issuer_name), 36)),
            right(pct_opt(x.foreign_pct_from)),
            right(pct(x.foreign_pct_to)),
            signed(f::change(Some(x.change_pp)), Some(x.change_pp), color),
            right(x.foreign_holders_to.to_string()),
        ]);
    }
    out.push_str(&t.to_string());
    out.push_str("\nSums over holders above 1% only; an issuer new to the register counts its whole foreign stake as inflow.\n");
    out
}

pub fn entity(r: &EntityReport, color: bool) -> String {
    let mut out = format!(
        "\"{}\" · {} matching holder{} · as of {}\n",
        r.query,
        r.entities.len(),
        if r.entities.len() == 1 { "" } else { "s" },
        r.as_of
    );
    for e in &r.entities {
        out.push_str(&format!("  {e}\n"));
    }
    let show_accts = several_accounts(&r.holdings);
    let show_control = r.controller_reported;
    let mut cols = vec!["Symbol", "Issuer", "Holder (as printed)", "Type"];
    if show_control {
        cols.push("Control");
    }
    if show_accts {
        cols.push("Accts");
    }
    cols.extend(["Shares", "%"]);
    let mut t = base_table(color);
    t.set_header(header(&cols));
    for h in &r.holdings {
        let mut row = vec![
            Cell::new(&h.symbol).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&f::text(&h.issuer_name), 30)),
            Cell::new(f::clip(&h.holder_raw, 40)),
            Cell::new(f::clip(&f::text(&h.investor_type), 18)),
        ];
        if show_control {
            row.push(Cell::new(control(&h.controller_or_affiliate)));
        }
        if show_accts {
            row.push(right(accounts(h.accounts)));
        }
        row.extend([right(shares(h.shares_total)), right(pct(h.pct))]);
        t.add_row(row);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    if show_accts {
        out.push_str(
            "Accts: register lines summed; the name as printed is the largest line's spelling.\n",
        );
    }
    if show_control {
        out.push_str(CONTROL_NOTE);
    }
    out
}

pub fn changes(r: &ChangesReport, color: bool) -> String {
    let s = &r.summary;
    let pp = |v: f64| format!("{}{} pp", if v > 0.0 { "+" } else { "" }, f::num(v, 2));
    let mut out = format!(
        "Changes {} → {}{} · {} match{} · {} issuer{} · {} holder{}\n",
        r.from,
        r.to,
        r.symbol
            .as_ref()
            .map(|s| format!(" · {s}"))
            .unwrap_or_default(),
        r.total_matches,
        if r.total_matches == 1 { "" } else { "es" },
        s.issuers,
        if s.issuers == 1 { "" } else { "s" },
        s.holders,
        if s.holders == 1 { "" } else { "s" },
    );
    out.push_str(&format!(
        "Summary · {} new · {} exited · {} increased · {} decreased · net {} (foreign {}, local {})\n",
        s.new_holders,
        s.exits,
        s.increased,
        s.decreased,
        pp(s.net_pp),
        pp(s.foreign_net_pp),
        pp(s.local_net_pp)
    ));
    if !s.by_investor_type.is_empty() {
        out.push_str(&format!(
            "By type · {}\n",
            s.by_investor_type
                .iter()
                .take(6)
                .map(|t| format!("{} {} ({})", t.investor_type, pp(t.net_pp), t.rows))
                .collect::<Vec<_>>()
                .join(" · ")
        ));
    }
    if r.symbol.is_none() && !s.issuer_flow.is_empty() {
        out.push_str(&format!(
            "Issuer flow · {}\n",
            s.issuer_flow
                .iter()
                .take(8)
                .map(|i| format!(
                    "{} {} (foreign {})",
                    i.symbol,
                    pp(i.net_pp),
                    pp(i.foreign_net_pp)
                ))
                .collect::<Vec<_>>()
                .join(" · ")
        ));
    }
    if r.changes.len() < r.total_matches {
        let first = r.offset + 1;
        let last = r.offset + r.changes.len();
        out.push_str(&format!(
            "Rows {first}–{last} of {} · next page: --offset {} · all rows: --limit 0\n",
            r.total_matches, last
        ));
    }
    let show_control = r
        .changes
        .iter()
        .any(|c| c.controller_or_affiliate.is_some());
    let mut cols = vec!["Symbol", "Holder", "Type", "L/A"];
    if show_control {
        cols.push("Control");
    }
    cols.extend(["Kind", "% from", "% to", "Δ pp", "Δ shares"]);
    let mut t = base_table(color);
    t.set_header(header(&cols));
    for c in &r.changes {
        let word = match c.kind {
            ChangeKind::Changed => "changed",
            ChangeKind::New => "new",
            ChangeKind::Exited => "exited",
        };
        // Holders on several register lines say so: "changed (1→2 accts)".
        let kind = Cell::new(match (c.accounts_from, c.accounts_to) {
            (Some(a), Some(b)) if a != b => format!("{word} ({a}→{b} accts)"),
            (Some(n), _) | (_, Some(n)) if n > 1 => format!("{word} ({n} accts)"),
            _ => word.to_string(),
        });
        let kind = if color {
            match c.kind {
                ChangeKind::New => kind.fg(Color::Green),
                ChangeKind::Exited => kind.fg(Color::Red),
                ChangeKind::Changed => kind,
            }
        } else {
            kind
        };
        let mut row = vec![
            Cell::new(&c.symbol).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&c.holder, 40)),
            Cell::new(f::clip(&f::text(&c.investor_type), 12)),
            Cell::new(f::text(&c.local_foreign)),
        ];
        if show_control {
            row.push(Cell::new(control(&c.controller_or_affiliate)));
        }
        row.extend([
            kind,
            right(pct_opt(c.pct_from)),
            right(pct_opt(c.pct_to)),
            signed(f::change(Some(c.pct_change)), Some(c.pct_change), color),
            signed(
                f::change(Some(c.shares_change as f64)),
                Some(c.shares_change as f64),
                color,
            ),
        ]);
        t.add_row(row);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    if show_control {
        out.push_str(CONTROL_NOTE);
    }
    out
}

pub fn releases(list: &[Release], db: &Path, color: bool) -> String {
    let count = |v: Option<u64>| {
        v.map(|n| f::num(n as f64, 0))
            .unwrap_or_else(|| f::DASH.into())
    };
    let show_other = list.iter().any(|r| r.other_date_rows.unwrap_or(0) > 0);
    let legacy = list.iter().any(|r| r.positions.is_none());
    let mut cols = vec!["As of", "Format", "Rows", "Positions"];
    if show_other {
        cols.push("Other dates");
    }
    cols.extend(["Issuers", "Holders", "Imported", "Source"]);
    let mut t = base_table(color);
    t.set_header(header(&cols));
    for r in list {
        let mut row = vec![
            Cell::new(r.as_of.to_string()).add_attribute(Attribute::Bold),
            Cell::new(&r.format),
            right(f::num(r.rows as f64, 0)),
            right(count(r.positions)),
        ];
        if show_other {
            row.push(right(count(r.other_date_rows)));
        }
        row.extend([
            right(f::num(r.issuers as f64, 0)),
            right(f::num(r.entities as f64, 0)),
            Cell::new(r.imported_at.get(..10).unwrap_or(&r.imported_at)),
            Cell::new(f::clip(&r.source, 48)),
        ]);
        t.add_row(row);
    }
    let mut out = format!("{t}\n");
    out.push_str("Rows: register lines. Positions: holders per ticker after summing each holder's accounts.\n");
    if show_other {
        out.push_str(
            "Other dates: lines dated other than the release, kept but left out of its holdings.\n",
        );
    }
    if legacy {
        out.push_str(&format!(
            "Positions {}: imported by idx 0.8.1 or older, which kept one account per holder; run `idx ownership import --all --force`.\n",
            f::DASH
        ));
    }
    out.push_str(&format!("Database: {}\n", db.display()));
    out
}

pub fn imports(list: &[ImportReport]) -> String {
    let mut out = String::new();
    for r in list {
        let rel = &r.release;
        let other = rel.other_date_rows.unwrap_or(0);
        let positions = match rel.positions {
            Some(p) => {
                let summed = rel.rows.saturating_sub(other).saturating_sub(p);
                format!(
                    " → {} positions{}",
                    f::num(p as f64, 0),
                    if summed > 0 {
                        format!(
                            " ({summed} extra account line{} summed)",
                            if summed == 1 { "" } else { "s" }
                        )
                    } else {
                        String::new()
                    }
                )
            }
            None => String::new(),
        };
        out.push_str(&format!(
            "Imported {} ({}): {} rows{}{}, {} issuers, {} holders{} → {}\n",
            rel.as_of,
            rel.format,
            f::num(rel.rows as f64, 0),
            positions,
            if other > 0 {
                format!(
                    ", {other} row{} dated otherwise left out",
                    if other == 1 { "" } else { "s" }
                )
            } else {
                String::new()
            },
            rel.issuers,
            f::num(rel.entities as f64, 0),
            if r.replaced {
                " (replaced earlier import)"
            } else {
                ""
            },
            r.database
        ));
    }
    if list.is_empty() {
        out.push_str("Nothing imported.\n");
    }
    out
}

pub fn discovered(files: &[DiscoveredFile], have: &[NaiveDate], color: bool) -> String {
    let mut t = base_table(color);
    t.set_header(header(&[
        "As of", "Format", "Imported", "From", "Title", "URL",
    ]));
    for d in files {
        t.add_row(vec![
            Cell::new(d.as_of.to_string()).add_attribute(Attribute::Bold),
            Cell::new(&d.format),
            Cell::new(if have.contains(&d.as_of) { "yes" } else { "no" }),
            Cell::new(&d.origin),
            Cell::new(f::clip(&d.title, 48)),
            Cell::new(&d.url),
        ]);
    }
    format!("{t}\n")
}
