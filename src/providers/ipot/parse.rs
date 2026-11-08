//! Pure parsers for IPOT's broker summary fragment. Tested against the
//! recorded pages in `tests/fixtures/ipot/`.
//!
//! The fragment is a 9-column table: buyer code, lots, value, average,
//! rank, seller code, lots, value, average. Each code sits in a
//! `<span class="text-local|text-foreign|text-bumn">`; the footer holds
//! `T. Val`, `F. NVal`, `T.Lot` and `Avg`.

use chrono::NaiveDate;

use crate::error::{IdxError, Result};
use crate::model::{BrokerBoard, BrokerFlow, BrokerInvestor, BrokerKind, BrokerSummary, Symbol};

use super::NAME;

/// What one page said.
#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    /// `Invalid stock code!`: the source does not know the ticker.
    Invalid,
    Summary(Parsed),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    /// Range echoed back in the page's date inputs.
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub buyers: Vec<BrokerFlow>,
    pub sellers: Vec<BrokerFlow>,
    pub total_value: Option<f64>,
    pub total_lots: Option<f64>,
    pub foreign_net_value: Option<f64>,
    pub average_price: Option<f64>,
    /// At least one figure was abbreviated (`1.6 M`).
    pub rounded: bool,
}

impl Parsed {
    /// The summary for `symbol`; `from` and `to` stand in when the page did
    /// not echo its range. Names are left for the caller to fill in.
    ///
    /// What the footer means depends on the filters (verified 2026-11-06):
    /// for one investor side `T. Val`, `T.Lot` and `Avg` are that side's
    /// buying (BBCA 11-05: foreign 605.4 B + domestic 242.8 B = the 848.3 B
    /// traded), so they become `buy_value` and the totals stay `null`;
    /// `F. NVal` is the same on every page and ignores the board filter, so
    /// with a board filter there is no net for that board.
    pub fn into_summary(
        self,
        symbol: Symbol,
        from: NaiveDate,
        to: NaiveDate,
        investor: BrokerInvestor,
        board: BrokerBoard,
    ) -> BrokerSummary {
        let all = investor == BrokerInvestor::All;
        let foreign_net_value = self.foreign_net_value.filter(|_| board == BrokerBoard::All);
        BrokerSummary {
            symbol,
            name: None,
            from: self.from.unwrap_or(from),
            to: self.to.unwrap_or(to),
            investor,
            board,
            buyers: self.buyers,
            sellers: self.sellers,
            total_value: self.total_value.filter(|_| all),
            total_lots: self.total_lots.filter(|_| all),
            foreign_net_value,
            average_price: self.average_price.filter(|_| all),
            buy_value: self.total_value,
            sell_value: sell_value(investor, self.total_value, foreign_net_value),
            rounded: self.rounded,
        }
    }
}

/// What the selected investors sold, from what they bought and the foreign
/// net: every trade has a buyer and a seller, so all investors sold what
/// they bought, foreign investors sold their buying less the net, and
/// domestic investors their buying plus it.
pub fn sell_value(investor: BrokerInvestor, buy: Option<f64>, net: Option<f64>) -> Option<f64> {
    match investor {
        BrokerInvestor::All => buy,
        BrokerInvestor::Foreign => Some(buy? - net?),
        BrokerInvestor::Domestic => Some(buy? + net?),
    }
}

/// `1.6 M` -> `(1_600_000, true)`, `687,067` -> `(687_067, false)`,
/// `-2.3 T` -> `(-2.3e12, true)`; empty or unparsable -> `None`.
pub fn number(text: &str) -> Option<(f64, bool)> {
    let t = text.trim().replace(',', "");
    if t.is_empty() {
        return None;
    }
    let (num, mult) = match t.chars().last()?.to_ascii_uppercase() {
        'K' => (&t[..t.len() - 1], 1e3),
        'M' => (&t[..t.len() - 1], 1e6),
        'B' => (&t[..t.len() - 1], 1e9),
        'T' => (&t[..t.len() - 1], 1e12),
        _ => (t.as_str(), 1.0),
    };
    let v = num.trim().parse::<f64>().ok()?;
    // Round away float noise from the scaling (1.6 * 1e6).
    Some(((v * mult).round(), mult != 1.0))
}

#[derive(Debug)]
pub(super) struct Cell {
    pub(super) class: Option<String>,
    pub(super) text: String,
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").trim().to_string()
}

pub(super) fn cells(row: &str) -> Vec<Cell> {
    let mut out = Vec::new();
    let mut rest = row;
    while let Some(start) = rest.find("<td") {
        let after = &rest[start..];
        let Some(open_end) = after.find('>') else {
            break;
        };
        let inner_start = open_end + 1;
        let Some(close) = after[inner_start..].find("</td>") else {
            break;
        };
        let inner = &after[inner_start..inner_start + close];
        let class = inner.find("class=\"").and_then(|i| {
            let v = &inner[i + 7..];
            v.find('"').map(|j| v[..j].to_string())
        });
        out.push(Cell {
            class,
            text: strip_tags(inner),
        });
        rest = &after[inner_start + close + 5..];
    }
    out
}

pub(super) fn kind(class: Option<&str>) -> Option<BrokerKind> {
    match class?.strip_prefix("text-")? {
        "local" => Some(BrokerKind::Local),
        "foreign" => Some(BrokerKind::Foreign),
        "bumn" => Some(BrokerKind::StateOwned),
        _ => None,
    }
}

/// `MM/DD/YYYY` from the value of the input with this id.
fn input_date(html: &str, id: &str) -> Option<NaiveDate> {
    let at = html.find(&format!("id=\"{id}\""))?;
    let tag_end = at + html[at..].find('>')?;
    let tag = &html[..tag_end];
    let tag = &tag[tag.rfind('<')?..];
    let v = &tag[tag.find("value=\"")? + 7..];
    NaiveDate::parse_from_str(&v[..v.find('"')?], "%m/%d/%Y").ok()
}

/// Text after `label` up to the next tag, e.g. `T. Val : 6.6 T`.
fn footer(html: &str, label: &str) -> Option<String> {
    let at = html.find(label)? + label.len();
    let rest = &html[at..];
    Some(rest[..rest.find('<')?].trim().to_string())
}

pub fn parse(html: &str) -> Result<Page> {
    if html
        .trim()
        .to_ascii_lowercase()
        .contains("invalid stock code")
    {
        return Ok(Page::Invalid);
    }
    let (Some(body_start), Some(body_end)) = (html.find("<tbody>"), html.find("</tbody>")) else {
        return Err(IdxError::provider(
            NAME,
            "unexpected broker summary page (no table)",
        ));
    };
    let mut p = Parsed {
        from: input_date(html, "startDateBrokerSummary"),
        to: input_date(html, "endDateBrokerSummary"),
        ..Parsed::default()
    };
    let mut rounded = false;
    let mut num = |c: &Cell| {
        number(&c.text).map(|(v, r)| {
            rounded |= r;
            v
        })
    };
    for (i, row) in html[body_start..body_end].split("</tr>").enumerate() {
        let c = cells(row);
        if c.len() < 9 {
            continue;
        }
        let rank = c[4].text.parse::<u32>().unwrap_or(i as u32 + 1);
        for (side, at) in [(&mut p.buyers, 0), (&mut p.sellers, 5)] {
            let code = c[at].text.to_ascii_uppercase();
            if code.is_empty() {
                continue;
            }
            side.push(BrokerFlow {
                rank,
                code,
                name: None,
                kind: kind(c[at].class.as_deref()),
                lots: num(&c[at + 1]),
                value: num(&c[at + 2]),
                average_price: num(&c[at + 3]),
            });
        }
    }
    let mut total = |label: &str| {
        footer(html, label).and_then(|t| {
            number(&t).map(|(v, r)| {
                rounded |= r;
                v
            })
        })
    };
    p.total_value = total("T. Val :");
    p.foreign_net_value = total("F. NVal :");
    p.total_lots = total("T.Lot :");
    // The page prints `Avg : 0` when nothing traded; there is no average then.
    p.average_price = total("Avg :").filter(|_| p.total_lots.is_some_and(|l| l > 0.0));
    p.rounded = rounded;
    Ok(Page::Summary(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: &str = include_str!("../../../tests/fixtures/ipot/brokersummary_bbca_20261016.html");
    const RANGE: &str =
        include_str!("../../../tests/fixtures/ipot/brokersummary_bbca_20261012_20261016.html");
    const EMPTY: &str =
        include_str!("../../../tests/fixtures/ipot/brokersummary_bbca_20261017_no_trades.html");
    const INVALID: &str =
        include_str!("../../../tests/fixtures/ipot/brokersummary_zzzz_invalid.html");

    fn summary(html: &str) -> Parsed {
        match parse(html).unwrap() {
            Page::Summary(p) => p,
            Page::Invalid => panic!("expected a summary"),
        }
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn numbers_as_published() {
        assert_eq!(number("1.6 M"), Some((1_600_000.0, true)));
        assert_eq!(number("841.2 B"), Some((841_200_000_000.0, true)));
        assert_eq!(number("-2.3 T"), Some((-2_300_000_000_000.0, true)));
        assert_eq!(number("687,067 "), Some((687_067.0, false)));
        assert_eq!(number("5,191 "), Some((5_191.0, false)));
        assert_eq!(number("0"), Some((0.0, false)));
        assert_eq!(number(" "), None);
        assert_eq!(number("n/a"), None);
    }

    #[test]
    fn parses_one_day() {
        let p = summary(DAY);
        assert_eq!(
            (p.from, p.to),
            (Some(d(2026, 10, 16)), Some(d(2026, 10, 16)))
        );
        assert_eq!((p.buyers.len(), p.sellers.len()), (10, 10));

        let b = &p.buyers[0];
        assert_eq!(
            (b.rank, b.code.as_str(), b.kind),
            (1, "MG", Some(BrokerKind::Local))
        );
        assert_eq!(b.lots, Some(1_600_000.0));
        assert_eq!(b.value, Some(841_200_000_000.0));
        assert_eq!(b.average_price, Some(5_191.0));
        assert_eq!(b.name, None, "names are the service's job");

        let s = &p.sellers[0];
        assert_eq!(
            (s.code.as_str(), s.kind),
            ("CC", Some(BrokerKind::StateOwned))
        );
        assert_eq!(s.value, Some(2_200_000_000_000.0));
        assert_eq!(p.buyers[3].kind, Some(BrokerKind::Foreign));
        assert_eq!(p.sellers[3].code, "BB");
        assert_eq!(p.sellers[3].lots, Some(687_067.0), "small lots are exact");
        assert_eq!(p.sellers[9].rank, 10);

        assert_eq!(p.total_value, Some(6_600_000_000_000.0));
        assert_eq!(p.foreign_net_value, Some(-2_300_000_000_000.0));
        assert_eq!(p.total_lots, Some(12_600_000.0));
        assert_eq!(p.average_price, Some(5_228.0));
        assert!(p.rounded);
    }

    #[test]
    fn parses_a_range() {
        let p = summary(RANGE);
        assert_eq!(
            (p.from, p.to),
            (Some(d(2026, 10, 12)), Some(d(2026, 10, 16)))
        );
        assert_eq!(p.total_value, Some(9_900_000_000_000.0));
        assert_eq!(p.average_price, Some(5_471.0));
        assert_eq!(p.buyers.len(), 10);
    }

    #[test]
    fn no_trades_is_an_empty_summary() {
        let p = summary(EMPTY);
        assert!(p.buyers.is_empty() && p.sellers.is_empty());
        assert_eq!(p.total_value, Some(0.0));
        assert_eq!(p.total_lots, Some(0.0));
        assert_eq!(p.average_price, None, "Avg : 0 means no average");
        assert!(!p.rounded);
        assert_eq!(p.to, Some(d(2026, 10, 17)));
    }

    fn side(html: &str, investor: BrokerInvestor, board: BrokerBoard) -> BrokerSummary {
        let day = d(2026, 10, 16);
        summary(html).into_summary(Symbol::parse("BBCA").unwrap(), day, day, investor, board)
    }

    #[test]
    fn one_investor_side_is_its_buying_and_selling_not_a_total() {
        use BrokerBoard as B;
        use BrokerInvestor as I;
        let foreign =
            include_str!("../../../tests/fixtures/ipot/brokersummary_bbca_20261016_foreign.html");
        let domestic =
            include_str!("../../../tests/fixtures/ipot/brokersummary_bbca_20261016_domestic.html");
        let regular = include_str!(
            "../../../tests/fixtures/ipot/brokersummary_bbca_20261016_foreign_regular.html"
        );

        let all = side(DAY, I::All, B::All);
        assert_eq!(all.total_value, Some(6_600_000_000_000.0));
        assert_eq!(
            (all.buy_value, all.sell_value),
            (all.total_value, all.total_value)
        );

        // Foreign: T. Val 1.7 T is their buying; F. NVal -2.3 T, so they sold 4.0 T.
        let f = side(foreign, I::Foreign, B::All);
        assert_eq!(
            (f.total_value, f.total_lots, f.average_price),
            (None, None, None)
        );
        assert_eq!(f.buy_value, Some(1_700_000_000_000.0));
        assert_eq!(f.sell_value, Some(4_000_000_000_000.0));
        assert_eq!(f.foreign_net_value, Some(-2_300_000_000_000.0));
        assert_eq!(f.buyers.len(), 10, "the broker lists stay");

        // Domestic: bought 4.9 T, sold 4.9 T - 2.3 T = 2.6 T.
        let dm = side(domestic, I::Domestic, B::All);
        assert_eq!(dm.total_value, None);
        assert_eq!(dm.buy_value, Some(4_900_000_000_000.0));
        assert_eq!(dm.sell_value, Some(2_600_000_000_000.0));

        // Regular board: the page still shows the all-board net, so none is kept.
        let r = side(regular, I::Foreign, B::Regular);
        assert_eq!(r.buy_value, Some(1_600_000_000_000.0));
        assert_eq!((r.sell_value, r.foreign_net_value), (None, None));
        let r = side(regular, I::All, B::Regular);
        assert_eq!(r.foreign_net_value, None);
        assert_eq!(
            r.total_value, r.sell_value,
            "every board trade has both sides"
        );

        // BBCA 11-05, probed 2026-11-06: foreign 605.4 B + domestic 242.8 B
        // bought the 848.3 B traded; foreign investors sold 301.5 B.
        let f = side(
            include_str!("../../../tests/fixtures/ipot/brokersummary_bbca_20261105_foreign.html"),
            I::Foreign,
            B::All,
        );
        assert_eq!(f.buy_value, Some(605_400_000_000.0));
        assert_eq!(f.sell_value, Some(301_500_000_000.0));
        let dm = side(
            include_str!("../../../tests/fixtures/ipot/brokersummary_bbca_20261105_domestic.html"),
            I::Domestic,
            B::All,
        );
        assert_eq!(dm.buy_value, Some(242_800_000_000.0));
        assert_eq!(dm.sell_value, Some(546_700_000_000.0));
    }

    #[test]
    fn invalid_code_and_garbage() {
        assert_eq!(parse(INVALID).unwrap(), Page::Invalid);
        let err = parse("<html>maintenance</html>").unwrap_err();
        assert_eq!(err.exit_code(), 3);
    }
}
