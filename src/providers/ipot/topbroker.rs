//! Pure parsers for IPOT's market-wide broker ranking and the page that
//! dates it. Tested against the recorded pages in `tests/fixtures/ipot/`.
//!
//! `ma-topbroker.php` (no parameters) is a 9-column table of every broker
//! in IPOT's latest session, ranked by value: `#`, code, name, `T.Val`
//! (value bought plus value sold), `N.Val` (bought minus sold), `B.Val`,
//! `S.Val`, `T.Vol` (shares bought plus sold) and `T.Freq` (trades). Cells
//! are read by position, so a page with other headings is refused.
//! Figures of a million and more are abbreviated to two decimals (`2.79 T`,
//! `-264.04 B`, `56.37 M`), smaller ones are exact (`278,800`, `43,020`; no
//! broker reached a million trades on 2026-11-06). `N.Val` carries its own
//! two decimals, so it is kept as published rather than taken from the
//! rounded `B.Val` and `S.Val`. Code
//! and name sit in a `<span class="text-local|text-foreign|text-bumn">`.
//! Brokers without trades are listed too, with zeros. A commented-out
//! template of placeholder rows ("Lorem Ipsum") follows the real ones, so
//! comments are removed before anything is read.
//!
//! The ranking carries no date. `statistic.php?code=BBCA` does: on its
//! `Today` row the dates of the day's high and low are the session's. A
//! commented-out copy of that table from 2015 follows the real one.

use chrono::NaiveDate;

use crate::error::{IdxError, Result};
use crate::model::BrokerKind;

use super::actions::decode;
use super::parse::{cells, kind, number, strip_tags};
use super::NAME;

/// The head of the ranking's table, label and span of each heading: the
/// rows are read by position, so a page headed otherwise is refused.
const COLUMNS: [(&str, usize); 8] = [
    ("#", 1),
    ("Company", 2),
    ("T.Val", 1),
    ("N.Val", 1),
    ("B.Val", 1),
    ("S.Val", 1),
    ("T.Vol", 1),
    ("T.Freq", 1),
];

/// One broker's row as the page shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct TopBroker {
    /// The page's `#`: its place by value.
    pub rank: u32,
    pub code: String,
    /// As the page publishes it.
    pub name: Option<String>,
    pub kind: Option<BrokerKind>,
    /// `T.Val`: value bought plus value sold, IDR.
    pub value: Option<f64>,
    /// `B.Val`: value bought, IDR.
    pub buy_value: Option<f64>,
    /// `S.Val`: value sold, IDR.
    pub sell_value: Option<f64>,
    /// `N.Val`: value bought minus value sold, IDR.
    pub net_value: Option<f64>,
    /// `T.Vol`: shares bought plus shares sold.
    pub volume: Option<f64>,
    /// `T.Freq`: trades, both sides counted.
    pub frequency: Option<f64>,
}

impl TopBroker {
    /// `false` for a broker the page lists without trades (all zeros).
    pub fn traded(&self) -> bool {
        [self.value, self.volume, self.frequency]
            .iter()
            .any(|v| v.is_some_and(|x| x != 0.0))
    }
}

/// Every broker in one session, in the page's order.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranking {
    /// The session, from [`session_date`].
    pub date: NaiveDate,
    pub brokers: Vec<TopBroker>,
    /// At least one figure was abbreviated (`2.79 T`).
    pub rounded: bool,
}

/// A row [`fold_listed_apart`] moved into the member it trades under.
#[derive(Debug, Clone, PartialEq)]
pub struct Folded {
    pub code: String,
    /// As published, the member's code in brackets first.
    pub name: String,
    /// The member's code.
    pub into: String,
}

/// `html` without its `<!-- ... -->` comments; an unclosed one runs to the end.
pub(super) fn uncommented(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start + 4..].find("-->") else {
            return out;
        };
        rest = &rest[start + 4 + end + 3..];
    }
    out.push_str(rest);
    out
}

/// The headings in the page's `<thead>`, each with its `colspan`.
pub(super) fn headings(html: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let Some(at) = html.find("<thead") else {
        return out;
    };
    let head = &html[at + 6..];
    let mut rest = &head[..head.find("</thead>").unwrap_or(head.len())];
    while let Some(i) = rest.find("<th") {
        let after = &rest[i + 3..];
        let (Some(open_end), Some(close)) = (after.find('>'), after.find("</th>")) else {
            break;
        };
        let attrs = &after[..open_end];
        if !attrs.is_empty() && !attrs.starts_with(char::is_whitespace) || close < open_end {
            rest = after;
            continue;
        }
        let span = attrs
            .find("colspan=\"")
            .and_then(|j| attrs[j + 9..].split('"').next()?.parse().ok())
            .unwrap_or(1);
        out.push((strip_tags(&after[open_end + 1..close]), span));
        rest = &after[close + 5..];
    }
    out
}

/// `#, Company (2 wide), T.Val` for messages.
fn show(columns: &[(String, usize)]) -> String {
    if columns.is_empty() {
        return "none".into();
    }
    columns
        .iter()
        .map(|(label, span)| match span {
            1 => label.clone(),
            n => format!("{label} ({n} wide)"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The ranking on a `ma-topbroker.php` page, for the session `date`: the
/// page does not say which (see [`session_date`]). A page whose head is not
/// [`COLUMNS`] is refused before any row is read.
pub fn ranking(html: &str, date: NaiveDate) -> Result<Ranking> {
    let html = uncommented(html);
    let (Some(start), Some(end)) = (html.find("<tbody>"), html.find("</tbody>")) else {
        return Err(IdxError::provider(
            NAME,
            "unexpected broker ranking page (no table)",
        ));
    };
    let head = headings(&html);
    let want: Vec<(String, usize)> = COLUMNS.iter().map(|(l, s)| (l.to_string(), *s)).collect();
    if head != want {
        return Err(IdxError::provider(
            NAME,
            format!(
                "unexpected broker ranking layout: the columns read {}, not {}",
                show(&head),
                show(&want)
            ),
        ));
    }
    let mut rounded = false;
    let mut brokers: Vec<TopBroker> = Vec::new();
    for row in html[start..end].split("</tr>") {
        let c = cells(row);
        if c.len() < 9 {
            continue;
        }
        let code = c[1].text.to_ascii_uppercase();
        if code.is_empty() {
            continue;
        }
        let rank = c[0].text.parse().unwrap_or(brokers.len() as u32 + 1);
        let mut num = |i: usize| {
            number(&c[i].text).map(|(v, r)| {
                rounded |= r;
                v
            })
        };
        brokers.push(TopBroker {
            rank,
            name: Some(decode(&c[2].text).trim().to_string()).filter(|n| !n.is_empty()),
            kind: kind(c[1].class.as_deref()),
            value: num(3),
            net_value: num(4),
            buy_value: num(5),
            sell_value: num(6),
            volume: num(7),
            frequency: num(8),
            code,
        });
    }
    Ok(Ranking {
        date,
        brokers,
        rounded,
    })
}

/// The session on a `statistic.php` page: the date on its `Today` row,
/// where the day's high and its low were both made. `None` without that row
/// (an unknown code answers `Invalid stock code!`) or when its two dates
/// differ.
pub fn session_date(html: &str) -> Option<NaiveDate> {
    let html = uncommented(html);
    let body = &html[html.find("<tbody>")?..];
    let row = body.split("</tr>").map(cells).find(|c| {
        c.first()
            .is_some_and(|x| x.text.eq_ignore_ascii_case("today"))
    })?;
    let date = |i: usize| NaiveDate::parse_from_str(row.get(i)?.text.trim(), "%Y-%m-%d").ok();
    let (high, low) = (date(1)?, date(3)?);
    (high == low).then_some(high)
}

/// A fingerprint of the ranking's rows, codes and figures as published (not
/// names), to tell the same ranking read again from another session's.
pub fn digest(r: &Ranking) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for b in &r.brokers {
        h.update(format!(
            "{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}
",
            b.code, b.value, b.net_value, b.buy_value, b.sell_value, b.volume, b.frequency
        ));
    }
    hex::encode(h.finalize())
}

/// `CC` from `[CC] BJB Sekuritas Jawa Barat`.
fn member_in_brackets(name: &str) -> Option<String> {
    let rest = name.trim_start().strip_prefix('[')?;
    let code = rest[..rest.find(']')?].trim();
    let plausible =
        (1..=4).contains(&code.len()) && code.chars().all(|c| c.is_ascii_alphanumeric());
    plausible.then(|| code.to_ascii_uppercase())
}

/// `sum + part`; unknown when either is.
fn add(sum: &mut Option<f64>, part: Option<f64>) {
    *sum = sum.zip(part).map(|(a, b)| a + b);
}

/// Moves every row IPOT lists apart for a broker that trades under another
/// member's code into that member, which is how idx.co.id counts it. Such a
/// row names the member in brackets: `JB`, `[CC] BJB Sekuritas Jawa Barat`
/// (2026-11-06: CC's 210,936 trades and JB's 276 make the 211,212 idx.co.id
/// reports for CC). A row whose member is not on the page stays as it is.
/// Returns the rows moved that had trades.
pub fn fold_listed_apart(r: &mut Ranking) -> Vec<Folded> {
    let mut folded = Vec::new();
    let mut i = 0;
    while i < r.brokers.len() {
        let into = r.brokers[i]
            .name
            .as_deref()
            .and_then(member_in_brackets)
            .filter(|m| *m != r.brokers[i].code)
            .and_then(|m| r.brokers.iter().position(|b| b.code == m));
        let Some(at) = into else {
            i += 1;
            continue;
        };
        let row = r.brokers.remove(i);
        let m = &mut r.brokers[if at > i { at - 1 } else { at }];
        add(&mut m.value, row.value);
        add(&mut m.buy_value, row.buy_value);
        add(&mut m.sell_value, row.sell_value);
        add(&mut m.net_value, row.net_value);
        add(&mut m.volume, row.volume);
        add(&mut m.frequency, row.frequency);
        if row.traded() {
            folded.push(Folded {
                into: m.code.clone(),
                name: row.name.clone().unwrap_or_default(),
                code: row.code,
            });
        }
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOP: &str = include_str!("../../../tests/fixtures/ipot/topbroker_20261106.html");
    const STATISTIC: &str =
        include_str!("../../../tests/fixtures/ipot/statistic_bbca_20261106.html");

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn top() -> Ranking {
        ranking(TOP, d(2026, 11, 6)).unwrap()
    }

    fn row<'a>(r: &'a Ranking, code: &str) -> &'a TopBroker {
        r.brokers.iter().find(|b| b.code == code).unwrap()
    }

    #[test]
    fn every_broker_of_the_session_in_the_pages_order() {
        let r = top();
        assert_eq!(r.date, d(2026, 11, 6));
        assert_eq!(r.brokers.len(), 91);
        assert!(r
            .brokers
            .iter()
            .enumerate()
            .all(|(i, b)| b.rank == i as u32 + 1));
        assert!(r.rounded);

        let first = &r.brokers[0];
        assert_eq!(
            (first.code.as_str(), first.name.as_deref(), first.kind),
            ("CP", Some("KB Valbury Sekuritas"), Some(BrokerKind::Local))
        );
        assert_eq!(first.value, Some(2_790_000_000_000.0));
        assert_eq!(first.net_value, Some(4_770_000_000.0));
        assert_eq!(first.buy_value, Some(1_400_000_000_000.0));
        assert_eq!(first.sell_value, Some(1_390_000_000_000.0));
        assert_eq!(first.volume, Some(6_480_000_000.0));
        assert_eq!(first.frequency, Some(43_020.0), "trades are exact");

        assert_eq!(r.brokers[1].kind, Some(BrokerKind::Foreign), "AK");
        let cc = row(&r, "CC");
        assert_eq!(cc.kind, Some(BrokerKind::StateOwned));
        assert_eq!(cc.net_value, Some(-264_040_000_000.0), "net selling");
        assert_eq!(cc.sell_value, Some(1_070_000_000_000.0));
        let gi = row(&r, "GI");
        assert_eq!(gi.value, Some(56_370_000.0));
        assert_eq!(gi.volume, Some(278_800.0), "small figures are exact");

        // The last two traded nothing; the name keeps its ampersand.
        let last = r.brokers.last().unwrap();
        assert_eq!((last.rank, last.code.as_str()), (91, "ZZ"));
        assert_eq!(last.name.as_deref(), Some("Kliring & Penjaminan Efek Ind."));
        assert_eq!(
            (last.value, last.net_value, last.frequency),
            (Some(0.0), Some(0.0), Some(0.0))
        );
        assert!(!last.traded() && !row(&r, "PS").traded());
        assert_eq!(r.brokers.iter().filter(|b| b.traded()).count(), 89);
    }

    #[test]
    fn the_commented_template_is_not_read() {
        let r = top();
        assert!(r.brokers.iter().all(|b| b.code != "AA"));
        assert!(r
            .brokers
            .iter()
            .all(|b| !b.name.as_deref().unwrap_or("").contains("Lorem")));
        // The template alone, as the page would show it without rows.
        let start = TOP.find("<!--tr>").unwrap();
        let head = &TOP[TOP.find("<thead>").unwrap()..TOP.find("</thead>").unwrap() + 8];
        let empty = format!(
            "<table>{head}<tbody>{}</tbody></table>",
            &TOP[start..TOP.find("</tbody>").unwrap()]
        );
        assert!(ranking(&empty, d(2026, 11, 6)).unwrap().brokers.is_empty());
        let err = ranking("<html>maintenance</html>", d(2026, 11, 6)).unwrap_err();
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn a_page_whose_columns_moved_is_refused() {
        let swapped = TOP
            .replacen(">B.Val<", ">\u{0}<", 1)
            .replacen(">S.Val<", ">B.Val<", 1)
            .replacen(">\u{0}<", ">S.Val<", 1);
        let err = ranking(&swapped, d(2026, 11, 6)).unwrap_err().to_string();
        assert!(
            err.ends_with(
                "the columns read #, Company (2 wide), T.Val, N.Val, S.Val, B.Val, T.Vol, T.Freq, not #, Company (2 wide), T.Val, N.Val, B.Val, S.Val, T.Vol, T.Freq"
            ),
            "{err}"
        );
        for (page, why) in [
            (swapped, "bought and sold swapped"),
            (
                TOP.replacen(">S.Val</th>", ">Avg</th><th>S.Val</th>", 1),
                "a column added",
            ),
            (
                TOP.replacen("<th colspan=\"2\" align=\"left\">", "<th>", 1),
                "code and name in one column",
            ),
            (
                TOP.replacen("<thead>", "", 1).replacen("</thead>", "", 1),
                "no head",
            ),
        ] {
            let Err(err) = ranking(&page, d(2026, 11, 6)) else {
                panic!("{why}: read as a ranking");
            };
            assert_eq!(err.exit_code(), 3, "{why}");
            assert!(
                err.to_string().contains("unexpected broker ranking layout"),
                "{why}: {err}"
            );
        }
    }

    #[test]
    fn the_session_is_the_date_on_the_today_row() {
        assert_eq!(session_date(STATISTIC), Some(d(2026, 11, 6)));
        // The commented-out 2015 table has a Today row of its own.
        let old = &STATISTIC[STATISTIC.find("<!--\n").unwrap()..];
        assert_eq!(session_date(&format!("<tbody>{old}")), None);
        assert_eq!(
            session_date(&format!("<tbody>{}", old.replacen("<!--", "", 1))),
            Some(d(2015, 9, 15))
        );
        assert_eq!(session_date("Invalid stock code!"), None);
        // The first date on the page is the Today row's high.
        let split = STATISTIC.replacen("2026-11-06", "2026-11-05", 1);
        assert_eq!(session_date(&split), None, "high and low on different days");
    }

    #[test]
    fn a_broker_trading_under_another_members_code_is_counted_in_it() {
        let mut r = top();
        let jb = row(&r, "JB").clone();
        assert_eq!(jb.name.as_deref(), Some("[CC] BJB Sekuritas Jawa Barat"));
        let cc = row(&r, "CC").clone();
        let folded = fold_listed_apart(&mut r);
        assert_eq!(
            folded,
            [Folded {
                code: "JB".into(),
                name: "[CC] BJB Sekuritas Jawa Barat".into(),
                into: "CC".into(),
            }]
        );
        assert_eq!(r.brokers.len(), 90);
        assert!(r.brokers.iter().all(|b| b.code != "JB"));
        let merged = row(&r, "CC");
        // idx.co.id's 211,212 trades for CC on 2026-11-06.
        assert_eq!(merged.frequency, Some(211_212.0));
        assert_eq!(merged.value, Some(1_880_000_000_000.0 + 1_490_000_000.0));
        assert_eq!(merged.volume, Some(4_470_000_000.0 + 38_660_000.0));
        assert_eq!(merged.net_value, Some(-264_040_000_000.0 + 1_130_000_000.0));
        assert_eq!(
            (merged.rank, merged.name.as_deref(), merged.kind),
            (cc.rank, cc.name.as_deref(), cc.kind)
        );

        // A member that is not on the page: the row stays as published.
        let mut r = top();
        r.brokers.retain(|b| b.code != "CC");
        assert!(fold_listed_apart(&mut r).is_empty());
        assert!(r.brokers.iter().any(|b| b.code == "JB"));
        assert_eq!(member_in_brackets(" [cc] x").as_deref(), Some("CC"));
        assert_eq!(member_in_brackets("[] x"), None);
        assert_eq!(member_in_brackets("[not a code] x"), None);
        assert_eq!(member_in_brackets("MANDIRI SEKURITAS"), None);
    }
}
