//! IPOT's "Corporate Action & RUPS" fragment for one stock, tested against
//! the recorded pages in `tests/fixtures/ipot/corporateaction_*`.
//!
//! `corporateaction.php?code=BBCA&start=MM/DD/YYYY` answers three tables:
//! - Corporate Actions: `#`, action type, amount, R1, R2, cum date,
//!   status, then hidden cells (`data-ca` 5 to 9) with the ex date,
//!   recording date, distribution date, "right end" and the announcement.
//! - RUPS: `#`, date, time, status, hidden venue and notice.
//! - Warrant & Right: `#`, code, W/R, exercise, shares, listing date,
//!   maturity date, status, hidden description.
//!
//! A template block (`id="detail_ca"`) follows with placeholder values
//! (`AAAA`, `2015-09-09`, lorem ipsum); it is cut off before parsing. The
//! source repeats some rows verbatim; repeats are dropped. `-` and empty
//! cells are `None`; an unknown code answers `Invalid stock code!`.

use chrono::NaiveDate;

use crate::model::{ActionKind, CorporateAction, ShareholderMeeting, WarrantRight};

/// What the page said.
#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    Invalid,
    Actions(Parsed),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    pub actions: Vec<CorporateAction>,
    pub meetings: Vec<ShareholderMeeting>,
    pub warrants_rights: Vec<WarrantRight>,
}

pub(super) fn decode(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Tags removed; paragraphs, list items and line breaks become line breaks;
/// spaces collapsed within lines; empty lines dropped.
fn plain(html: &str) -> String {
    let mut marked = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    while i < html.len() {
        if html.as_bytes()[i] == b'<' {
            let end = html[i..].find('>').map(|j| i + j + 1).unwrap_or(html.len());
            let tag = &lower[i..end];
            if tag.starts_with("</p")
                || tag.starts_with("<br")
                || tag.starts_with("<li")
                || tag.starts_with("</tr")
                || tag.starts_with("</li")
            {
                marked.push('\n');
            } else if tag.starts_with("</td") || tag.starts_with("</th") {
                marked.push(' ');
            }
            i = end;
        } else {
            let next = html[i..].find('<').map(|j| i + j).unwrap_or(html.len());
            marked.push_str(&html[i..next]);
            i = next;
        }
    }
    decode(&marked)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Position of the next `<tag` opening (followed by `>` or whitespace, so
/// `<th` does not match `<thead`) at or after `from`, in lower-case HTML.
fn find_open(lower: &str, open: &str, from: usize) -> Option<usize> {
    let mut at = from;
    while let Some(i) = lower.get(at..)?.find(open) {
        let p = at + i;
        match lower.as_bytes().get(p + open.len()) {
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') => return Some(p),
            _ => at = p + open.len(),
        }
    }
    None
}

/// Inner HTML of each top-level `<tag>` element in `html`. Elements nested
/// inside one (the announcements carry their own little tables) stay part
/// of their parent.
fn elements<'a>(html: &'a str, tag: &str) -> Vec<&'a str> {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(start) = find_open(&lower, &open, pos) {
        let Some(gt) = lower[start..].find('>') else {
            break;
        };
        let inner = start + gt + 1;
        let mut depth = 1;
        let mut cur = inner;
        loop {
            let next_open = find_open(&lower, &open, cur);
            let Some(next_close) = lower[cur..].find(&close).map(|i| cur + i) else {
                return out;
            };
            match next_open {
                Some(o) if o < next_close => {
                    depth += 1;
                    cur = o + open.len();
                }
                _ => {
                    depth -= 1;
                    cur = next_close + close.len();
                    if depth == 0 {
                        out.push(&html[inner..next_close]);
                        break;
                    }
                }
            }
        }
        pos = cur;
    }
    out
}

/// Inner HTML of every cell in a row.
fn tds(row: &str) -> Vec<&str> {
    elements(row, "td")
}

/// Rows of the `<tbody>` that follows `heading`.
fn rows<'a>(html: &'a str, heading: &str) -> Vec<&'a str> {
    let Some(h) = html.find(heading) else {
        return Vec::new();
    };
    let section = &html[h..];
    let Some(b) = section.find("<tbody>") else {
        return Vec::new();
    };
    let body = &section[b + "<tbody>".len()..];
    let body = &body[..body.find("</tbody>").unwrap_or(body.len())];
    elements(body, "tr")
}

fn text(cell: Option<&&str>) -> Option<String> {
    let t = plain(cell?);
    match t.trim() {
        "" | "-" => None,
        s => Some(s.to_string()),
    }
}

fn date(cell: Option<&&str>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text(cell)?.trim(), "%Y-%m-%d").ok()
}

fn number(cell: Option<&&str>) -> Option<f64> {
    text(cell)?.replace(',', "").trim().parse().ok()
}

/// 0 means "not applicable" in the amount and ratio columns.
fn nonzero(v: Option<f64>) -> Option<f64> {
    v.filter(|x| *x != 0.0)
}

pub fn kind(label: &str) -> ActionKind {
    // The source spells it "Deviden" as often as "Dividend".
    let l = label.to_lowercase().replace("deviden", "dividen");
    if l.contains("reverse") {
        ActionKind::ReverseSplit
    } else if l.contains("split") {
        ActionKind::StockSplit
    } else if l.contains("bonus") {
        ActionKind::Bonus
    } else if l.contains("right") || l.contains("hmetd") {
        ActionKind::RightsIssue
    } else if l.contains("warrant") || l.contains("waran") {
        ActionKind::Warrant
    } else if (l.contains("dividen") || l.contains("dividend")) && l.contains("stock") {
        ActionKind::StockDividend
    } else if l.contains("dividen") || l.contains("dividend") {
        ActionKind::CashDividend
    } else {
        ActionKind::Other
    }
}

fn action(row: &str) -> Option<CorporateAction> {
    // hidden code, #, type, amount, R1, R2, cum, status, ex, rec, dist, end, text
    let c = tds(row);
    if c.len() < 8 {
        return None;
    }
    let label = text(c.get(2))?;
    let k = kind(&label);
    let cash = k == ActionKind::CashDividend;
    Some(CorporateAction {
        kind: k,
        label,
        amount: nonzero(number(c.get(3))),
        ratio_old: if cash {
            None
        } else {
            nonzero(number(c.get(4)))
        },
        ratio_new: if cash {
            None
        } else {
            nonzero(number(c.get(5)))
        },
        cum_date: date(c.get(6)),
        status: text(c.get(7)),
        ex_date: date(c.get(8)),
        recording_date: date(c.get(9)),
        distribution_date: date(c.get(10)),
        end_date: date(c.get(11)),
        details: text(c.get(12)),
    })
}

fn meeting(row: &str) -> Option<ShareholderMeeting> {
    // hidden code, #, date, time, status, venue, notice
    let c = tds(row);
    if c.len() < 5 {
        return None;
    }
    Some(ShareholderMeeting {
        date: date(c.get(2)),
        time: text(c.get(3)),
        status: text(c.get(4)),
        venue: text(c.get(5)),
        agenda: text(c.get(6)),
    })
}

fn warrant(row: &str) -> Option<WarrantRight> {
    // #, code, W/R, exercise, shares, listing, maturity, status, text
    let c = tds(row);
    if c.len() < 8 {
        return None;
    }
    Some(WarrantRight {
        code: text(c.get(1))?,
        kind: text(c.get(2))?,
        exercise_price: nonzero(number(c.get(3))),
        shares: nonzero(number(c.get(4))),
        listing_date: date(c.get(5)),
        maturity_date: date(c.get(6)),
        status: text(c.get(7)),
        details: text(c.get(8)),
    })
}

fn dedup<T: PartialEq>(v: Vec<T>) -> Vec<T> {
    let mut out: Vec<T> = Vec::with_capacity(v.len());
    for x in v {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

pub fn parse(html: &str) -> Page {
    if html.trim().starts_with("Invalid stock code") {
        return Page::Invalid;
    }
    let html = &html[..html.find("id=\"detail_ca\"").unwrap_or(html.len())];
    let mut actions = dedup(
        rows(html, ">Corporate Actions</h5>")
            .into_iter()
            .filter_map(action)
            .collect(),
    );
    actions.sort_by_key(|a| std::cmp::Reverse(a.cum_date));
    let mut meetings = dedup(
        rows(html, ">RUPS</h5>")
            .into_iter()
            .filter_map(meeting)
            .collect(),
    );
    meetings.sort_by_key(|m| std::cmp::Reverse(m.date));
    let mut warrants_rights = dedup(
        rows(html, ">Warrant &amp; Right</h5>")
            .into_iter()
            .chain(rows(html, ">Warrant & Right</h5>"))
            .filter_map(warrant)
            .collect(),
    );
    warrants_rights.sort_by_key(|w| std::cmp::Reverse(w.listing_date));
    Page::Actions(Parsed {
        actions,
        meetings,
        warrants_rights,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BBCA: &str =
        include_str!("../../../tests/fixtures/ipot/corporateaction_bbca_20200101.html");
    const ENRG: &str =
        include_str!("../../../tests/fixtures/ipot/corporateaction_enrg_20250101.html");
    const MEGA: &str =
        include_str!("../../../tests/fixtures/ipot/corporateaction_mega_20250101.html");
    const EMPTY: &str =
        include_str!("../../../tests/fixtures/ipot/corporateaction_bbca_since_today_empty.html");
    const INVALID: &str =
        include_str!("../../../tests/fixtures/ipot/corporateaction_zzzz_invalid.html");

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn parsed(html: &str) -> Parsed {
        match parse(html) {
            Page::Actions(p) => p,
            Page::Invalid => panic!("invalid"),
        }
    }

    #[test]
    fn cash_dividends_and_split_with_repeats_dropped() {
        let p = parsed(BBCA);
        // The page lists 2021-04-07 and 2020-04-20 twice each.
        let cum: Vec<_> = p.actions.iter().map(|a| a.cum_date.unwrap()).collect();
        let mut unique = cum.clone();
        unique.dedup();
        assert_eq!(cum, unique, "sorted newest first, no repeats");
        let split = p
            .actions
            .iter()
            .find(|a| a.kind == ActionKind::StockSplit)
            .unwrap();
        assert_eq!(split.label, "Stock Split");
        assert_eq!((split.ratio_old, split.ratio_new), (Some(1.0), Some(5.0)));
        assert_eq!(split.amount, None, "0.00 means not applicable");
        assert_eq!(split.cum_date, Some(d(2021, 10, 12)));
        assert_eq!(split.ex_date, Some(d(2021, 10, 13)));
        assert_eq!(split.end_date, Some(d(2021, 10, 15)));
        // The schedule sits in a table nested inside the announcement cell.
        let schedule = split.details.as_deref().unwrap();
        assert!(schedule.contains("Nilai Nominal Baru"), "{schedule}");
        assert!(schedule.contains("13 Oktober 2021"), "{schedule}");
        let div = p
            .actions
            .iter()
            .find(|a| a.cum_date == Some(d(2021, 4, 7)))
            .unwrap();
        assert_eq!(div.kind, ActionKind::CashDividend);
        assert_eq!(div.label, "Cash Deviden");
        assert_eq!(div.amount, Some(432.0));
        assert_eq!((div.ratio_old, div.ratio_new), (None, None));
        assert_eq!(div.recording_date, Some(d(2021, 4, 9)));
        assert_eq!(div.distribution_date, Some(d(2021, 4, 28)));
        assert_eq!(div.end_date, None, "`-` is no date");
        assert_eq!(div.status.as_deref(), Some("Final"));
        let details = div.details.as_deref().unwrap();
        assert!(details.contains("Rp. 432"), "{details}");
        assert!(details.contains('\n'), "paragraphs kept apart");
        assert!(!details.contains('<'));

        let rups = &p.meetings[0];
        assert_eq!(rups.date, Some(d(2026, 3, 12)));
        assert_eq!(rups.time.as_deref(), Some("14:00:00"));
        assert!(rups.venue.as_deref().unwrap().contains("Menara BCA"));
        assert!(rups
            .agenda
            .as_deref()
            .unwrap()
            .contains("Agenda RUPS Tahunan"));
        assert!(p.meetings.windows(2).all(|w| w[0].date >= w[1].date));
        assert!(p.warrants_rights.is_empty());
        // Nothing from the template block.
        assert!(p.actions.iter().all(|a| a.cum_date != Some(d(2015, 9, 9))));
    }

    #[test]
    fn rights_issue_and_listed_right() {
        let p = parsed(ENRG);
        let r = &p.actions[0];
        assert_eq!(r.kind, ActionKind::RightsIssue);
        assert_eq!(r.amount, Some(310.0), "exercise price");
        assert_eq!((r.ratio_old, r.ratio_new), (Some(2.0), Some(1.0)));
        assert_eq!(r.end_date, Some(d(2026, 10, 21)), "end of rights trading");
        let w = &p.warrants_rights[0];
        assert_eq!(w.code, "ENRG-R");
        assert_eq!(w.kind, "Right");
        assert_eq!(w.exercise_price, Some(310.0));
        assert_eq!(w.shares, Some(13_282_271_875.0));
        assert_eq!(w.listing_date, Some(d(2026, 10, 8)));
        assert_eq!(w.maturity_date, Some(d(2026, 10, 21)));
        assert!(p
            .meetings
            .iter()
            .any(|m| m.status.as_deref() == Some("Pending")));
    }

    #[test]
    fn bonus_shares() {
        let p = parsed(MEGA);
        let b = p
            .actions
            .iter()
            .find(|a| a.kind == ActionKind::Bonus)
            .unwrap();
        assert_eq!(b.label, "Bonus");
        assert_eq!((b.ratio_old, b.ratio_new), (Some(1.0), Some(1.0)));
        assert_eq!(b.amount, None);
        assert_eq!(b.cum_date, Some(d(2026, 4, 9)));
        assert_eq!(b.distribution_date, Some(d(2026, 4, 30)));
    }

    #[test]
    fn empty_and_invalid_pages() {
        let p = parsed(EMPTY);
        assert!(p.actions.is_empty() && p.meetings.is_empty() && p.warrants_rights.is_empty());
        assert_eq!(parse(INVALID), Page::Invalid);
    }

    #[test]
    fn labels_map_to_kinds() {
        assert_eq!(kind("Cash Deviden"), ActionKind::CashDividend);
        assert_eq!(kind("Stock Dividend"), ActionKind::StockDividend);
        assert_eq!(kind("Reverse Split"), ActionKind::ReverseSplit);
        assert_eq!(kind("Right Issue"), ActionKind::RightsIssue);
        assert_eq!(kind("Something"), ActionKind::Other);
    }
}
