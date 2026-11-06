//! Tables for `idx ownership search | cross-holders | concentration | graph
//! | resolve`.

use std::collections::HashMap;

use comfy_table::{Attribute, Cell};

use crate::model::{
    CandidateReason, ConcentrationReport, ConcentrationSort, CrossHoldersReport, GraphReport,
    HolderOrigin, Resolution, ResolutionAction, ResolutionKind, ResolutionReport,
    ResolveCandidates, SearchMatch, SearchMatchKind, SearchReport,
};
use crate::output::format as f;
use crate::output::table::{base_table, header, right};

fn pct(v: f64) -> String {
    format!("{}%", f::num(v, 2))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn showing(shown: usize, total: usize) -> String {
    if shown < total {
        format!(" · showing {shown} (use --limit 0 for all)")
    } else {
        String::new()
    }
}

fn kind(k: SearchMatchKind) -> &'static str {
    match k {
        SearchMatchKind::Exact => "exact",
        SearchMatchKind::Spelling => "spelling",
        SearchMatchKind::Prefix => "prefix",
        SearchMatchKind::Contains => "contains",
        SearchMatchKind::Words => "words",
    }
}

/// Spellings of a match other than its display name.
fn other_spellings(m: &SearchMatch) -> Vec<&str> {
    m.spellings
        .iter()
        .map(String::as_str)
        .filter(|s| *s != m.holder)
        .collect()
}

pub fn search(r: &SearchReport, color: bool) -> String {
    let mut out = format!(
        "\"{}\" · {} match{}{} · issuers counted as of {}\n",
        r.query,
        plural(r.total_matches, "holder", "holders"),
        if r.total_matches == 1 { "es" } else { "" },
        showing(r.matches.len(), r.total_matches),
        r.as_of
    );
    if r.matches.is_empty() {
        out.push_str("No holder matches. Try fewer or shorter words.\n");
        return out;
    }
    let show_spellings = r.matches.iter().any(|m| !other_spellings(m).is_empty());
    let mut cols = vec![
        "#",
        "Holder",
        "Match",
        "Issuers",
        "Total %",
        "Type",
        "L/A",
        "Last seen",
    ];
    if show_spellings {
        cols.push("Other spellings");
    }
    cols.push("Key");
    let mut t = base_table(color);
    t.set_header(header(&cols));
    for (i, m) in r.matches.iter().enumerate() {
        let mut row = vec![
            right((i + 1).to_string()),
            Cell::new(f::clip(&m.holder, 40)).add_attribute(Attribute::Bold),
            Cell::new(kind(m.matched)),
            right(m.issuers.to_string()),
            right(if m.issuers == 0 {
                f::DASH.to_string()
            } else {
                pct(m.total_pct)
            }),
            Cell::new(f::clip(&f::text(&m.investor_type), 18)),
            Cell::new(f::text(&m.local_foreign)),
            Cell::new(m.last_seen),
        ];
        if show_spellings {
            row.push(Cell::new(f::clip(&other_spellings(m).join(" | "), 40)));
        }
        row.push(Cell::new(f::clip(&m.entity_key, 40)));
        t.add_row(row);
    }
    out.push_str(&t.to_string());
    out.push_str(&format!(
        "\nTotal %: stakes in different issuers added up (a breadth measure). Next: idx ownership entity --exact \"{}\"\n",
        r.matches[0].entity_key
    ));
    out
}

pub fn cross_holders(r: &CrossHoldersReport, color: bool) -> String {
    let mut filters = Vec::new();
    if let Some(t) = &r.investor_type {
        filters.push(format!("type ~ {t}"));
    }
    if let Some(o) = r.origin {
        filters.push(
            match o {
                HolderOrigin::Foreign => "foreign (A)",
                HolderOrigin::Local => "local (L)",
            }
            .to_string(),
        );
    }
    let mut out = format!(
        "Holders above 1% in at least {} as of {}{} · {}{}\n",
        plural(r.min_issuers, "issuer", "issuers"),
        r.as_of,
        filters
            .iter()
            .map(|f| format!(" · {f}"))
            .collect::<String>(),
        plural(r.total_matches, "holder", "holders"),
        showing(r.holders.len(), r.total_matches)
    );
    if r.holders.is_empty() {
        out.push_str("No holder passes the filters.\n");
        return out;
    }
    let mut t = base_table(color);
    t.set_header(header(&[
        "#",
        "Holder",
        "Type",
        "L/A",
        "Issuers",
        "Total %",
        "Largest positions",
    ]));
    for (i, h) in r.holders.iter().enumerate() {
        let mut largest: Vec<String> = h
            .positions
            .iter()
            .take(4)
            .map(|p| format!("{} {}", p.symbol, pct(p.pct)))
            .collect();
        if h.positions.len() > 4 {
            largest.push("…".into());
        }
        t.add_row(vec![
            right((i + 1).to_string()),
            Cell::new(f::clip(&h.holder, 40)).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&f::text(&h.investor_type), 18)),
            Cell::new(f::text(&h.local_foreign)),
            right(h.issuers.to_string()),
            right(pct(h.total_pct)),
            Cell::new(largest.join(", ")),
        ]);
    }
    out.push_str(&t.to_string());
    out.push_str("\nTotal % adds stakes in different companies: a breadth measure, not a share of anything.\n");
    out.push_str("Custodian and nominee accounts (securities companies, private banks) hold for their clients.\n");
    out.push_str(
        "A holder printed two ways is counted twice: see idx ownership resolve candidates.\n",
    );
    out
}

pub fn concentration(r: &ConcentrationReport, color: bool) -> String {
    let by = match r.sort {
        ConcentrationSort::Hhi => "HHI",
        ConcentrationSort::Top1 => "largest stake",
        ConcentrationSort::Top3 => "top-3 stakes",
    };
    let mut out = format!(
        "Ownership concentration as of {} · by {by}, {} first · {}{}\n",
        r.as_of,
        if r.ascending { "least" } else { "most" },
        plural(r.total_matches, "issuer", "issuers"),
        showing(r.rows.len(), r.total_matches)
    );
    let mut cols = vec![
        "#",
        "Symbol",
        "Issuer",
        "HHI",
        "Top 1 %",
        "Top 3 %",
        "Listed %",
        "Undisclosed %",
    ];
    if r.controller_reported {
        cols.push("Control %");
    }
    cols.extend(["Holders", "Largest holder"]);
    let mut t = base_table(color);
    t.set_header(header(&cols));
    for (i, c) in r.rows.iter().enumerate() {
        let mut row = vec![
            right((i + 1).to_string()),
            Cell::new(&c.symbol).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&f::text(&c.issuer_name), 30)),
            right(f::num(c.hhi, 2)),
            right(pct(c.top1_pct)),
            right(pct(c.top3_pct)),
            right(pct(c.listed_pct)),
            right(pct(c.undisclosed_pct)),
        ];
        if r.controller_reported {
            row.push(right(
                c.controller_pct
                    .map(pct)
                    .unwrap_or_else(|| f::DASH.to_string()),
            ));
        }
        row.extend([
            right(c.holders_listed.to_string()),
            Cell::new(f::clip(&c.top_holder, 36)),
        ]);
        t.add_row(row);
    }
    out.push_str(&t.to_string());
    out.push_str("\nHHI is the sum of the squared stakes of the holders listed above 1% (0 to 10,000; 10,000 means\n");
    out.push_str("one holder owns everything). Holders at or below 1% are not published, so HHI covers the listed\n");
    out.push_str("holders only (a lower bound of the full figure) and Undisclosed is what the register does not show.\n");
    if r.controller_reported {
        out.push_str("Control %: stakes flagged CONTROLLER or AFFILIATE OF THE CONTROLLER.\n");
    }
    out
}

pub fn graph(r: &GraphReport, color: bool) -> String {
    let label = r
        .issuers
        .iter()
        .find(|i| i.id == r.root)
        .map(|i| i.symbol.clone())
        .or_else(|| {
            r.holders
                .iter()
                .find(|h| h.id == r.root)
                .map(|h| h.holder.clone())
        })
        .unwrap_or_else(|| r.root.clone());
    let mut out = format!(
        "Ownership network around {label} as of {} · {} · {} · {} · {}{}{}\n",
        r.as_of,
        plural(r.depth, "hop", "hops"),
        plural(r.holders.len(), "holder", "holders"),
        plural(r.issuers.len(), "issuer", "issuers"),
        plural(r.links.len(), "link", "links"),
        if r.min_pct > 0.0 {
            format!(" · stakes of {}% or more", r.min_pct)
        } else {
            String::new()
        },
        if r.truncated { " · truncated" } else { "" }
    );
    let holders: HashMap<&str, &crate::model::GraphHolder> =
        r.holders.iter().map(|h| (h.id.as_str(), h)).collect();
    let mut t = base_table(color);
    t.set_header(header(&["Hops", "Symbol", "Holder", "Type", "L/A", "%"]));
    for l in &r.links {
        let h = holders.get(l.from.as_str());
        t.add_row(vec![
            right(l.hops.to_string()),
            Cell::new(&l.symbol).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&l.holder, 44)),
            Cell::new(f::clip(
                &h.and_then(|h| h.investor_type.clone())
                    .unwrap_or_else(|| f::DASH.into()),
                18,
            )),
            Cell::new(
                h.and_then(|h| h.local_foreign.clone())
                    .unwrap_or_else(|| f::DASH.into()),
            ),
            right(pct(l.pct)),
        ]);
    }
    out.push_str(&t.to_string());
    out.push_str(&format!(
        "\nEach row is a holding above 1%. Graphviz: idx ownership graph {} --dot | dot -Tsvg -o graph.svg\n",
        if label.contains(' ') {
            format!("\"{label}\"")
        } else {
            label
        }
    ));
    out
}

pub fn resolve_candidates(r: &ResolveCandidates, color: bool) -> String {
    let mut out = format!(
        "Holders that may be one · {}{} · {} compared, newest {}\n",
        plural(r.total_matches, "pair", "pairs"),
        showing(r.candidates.len(), r.total_matches),
        plural(r.releases_compared, "pair of months", "pairs of months"),
        r.as_of
    );
    if r.candidates.is_empty() {
        out.push_str("No candidates.\n");
        return out;
    }
    let mut t = base_table(color);
    t.set_header(header(&[
        "#",
        "Keep",
        "Merge",
        "Evidence",
        "Listed together",
    ]));
    for (i, c) in r.candidates.iter().enumerate() {
        let mut evidence: Vec<String> = c
            .renamed
            .iter()
            .map(|e| {
                format!(
                    "renamed in {} {}→{} ({})",
                    plural(e.symbols.len(), "issuer", "issuers"),
                    e.from,
                    e.to,
                    e.symbols.join(", ")
                )
            })
            .collect();
        if c.reasons.contains(&CandidateReason::SimilarName) {
            evidence.push("similar name".into());
        }
        let together = c
            .listed_together
            .iter()
            .map(|m| format!("{} {}", m.symbol, m.as_of))
            .collect::<Vec<_>>()
            .join(", ");
        t.add_row(vec![
            right((i + 1).to_string()),
            Cell::new(format!(
                "{} ({})",
                f::clip(&c.keep.holder, 36),
                c.keep.issuers
            ))
            .add_attribute(Attribute::Bold),
            Cell::new(format!(
                "{} ({}, last {})",
                f::clip(&c.merge.holder, 36),
                c.merge.issuers,
                c.merge.last_seen
            )),
            Cell::new(evidence.join(" · ")),
            Cell::new(if together.is_empty() {
                String::new()
            } else {
                f::clip(&together, 40)
            }),
        ]);
    }
    out.push_str(&t.to_string());
    let first = &r.candidates[0];
    out.push_str(&format!(
        "\n(n): issuers held above 1% in the newest release. To merge the first pair:\n  idx ownership resolve merge \"{}\" \"{}\" --dry-run\n",
        first.keep.entity_key, first.merge.entity_key
    ));
    out.push_str("These are suggestions: an identical share count can also be a real block sale (MANDOM CORPORATION\n");
    out.push_str("to KALON MIDCO HOLDINGS CO LTD), and holders listed together are often different people.\n");
    out
}

fn kind_name(k: ResolutionKind) -> &'static str {
    match k {
        ResolutionKind::Merge => "merge",
        ResolutionKind::Map => "map",
    }
}

fn combined_text(r: &Resolution) -> String {
    r.combined
        .iter()
        .map(|m| format!("{} {}", m.symbol, m.as_of))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Results of `resolve merge | map | undo`: one sentence per rule.
pub fn resolution_reports(list: &[ResolutionReport]) -> String {
    let mut out = String::new();
    for r in list {
        let rule = &r.resolution;
        let id = rule.id.map(|i| format!(" (#{i})")).unwrap_or_default();
        let what = match (r.action, rule.kind) {
            (ResolutionAction::Merge, _) => format!("{} into {}{id}", rule.subject, rule.holder),
            (ResolutionAction::Map, _) => {
                format!("spelling \"{}\" to {}{id}", rule.subject, rule.holder)
            }
            (ResolutionAction::Undo, ResolutionKind::Merge) => {
                format!("merge{id} of {} into {}", rule.subject, rule.holder)
            }
            (ResolutionAction::Undo, ResolutionKind::Map) => {
                format!("map{id} of \"{}\" to {}", rule.subject, rule.holder)
            }
        };
        let verb = match (r.action, r.dry_run) {
            (ResolutionAction::Undo, true) => "Would undo",
            (ResolutionAction::Undo, false) => "Undid",
            (ResolutionAction::Merge, true) => "Would merge",
            (ResolutionAction::Merge, false) => "Merged",
            (ResolutionAction::Map, true) => "Would map",
            (ResolutionAction::Map, false) => "Mapped",
        };
        let months = if r.releases.is_empty() {
            "no imported release".to_string()
        } else {
            r.releases
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "{verb} {what}: {}, {}, {} in {months}{}{}\n",
            plural(r.spellings_moved, "spelling", "spellings"),
            plural(r.lines_moved, "register line", "register lines"),
            plural(r.positions_moved, "position", "positions"),
            if r.positions_combined > 0 {
                format!(
                    " ({} {})",
                    plural(r.positions_combined, "issuer-month", "issuer-months"),
                    if r.action == ResolutionAction::Undo {
                        "split again"
                    } else {
                        "combined"
                    }
                )
            } else {
                String::new()
            },
            if r.dry_run {
                " (dry run, nothing written)"
            } else {
                ""
            }
        ));
    }
    out
}

/// `resolve list`.
pub fn resolutions(list: &[Resolution], color: bool) -> String {
    if list.is_empty() {
        return "No resolution rules yet. Suggestions: idx ownership resolve candidates\n".into();
    }
    let mut t = base_table(color);
    t.set_header(header(&[
        "#", "Kind", "Subject", "Holder", "Created", "Combined",
    ]));
    for r in list {
        t.add_row(vec![
            right(r.id.map(|i| format!("#{i}")).unwrap_or_default()),
            Cell::new(kind_name(r.kind)),
            Cell::new(f::clip(&r.subject, 40)).add_attribute(Attribute::Bold),
            Cell::new(f::clip(&r.holder, 40)),
            Cell::new(&r.created_at),
            Cell::new(f::clip(&combined_text(r), 40)),
        ]);
    }
    let mut out = format!(
        "{} in this database, applied to every imported release and every later import\n",
        plural(list.len(), "resolution rule", "resolution rules")
    );
    out.push_str(&t.to_string());
    out.push_str("\nUndo one with: idx ownership resolve undo <#>\n");
    out
}
