//! Pure parser for IPOT's market statistics page, tested against the
//! recorded page in `tests/fixtures/ipot/`.
//!
//! `ma-marketstatistic.php` (no parameters) sums up IPOT's latest session
//! in two tables under one head each: by board (`Regular`, `Negotiated`,
//! `Cash` and `TOTAL`) and foreign investors' side (`F. Buy`, `F. Sell`,
//! `F. TOTAL` and `F. NET`), each row with its `Value` (IDR), `Lot` and
//! `Freq`. Figures of a million and more are abbreviated to two decimals
//! (`10.32 T`, `329.33 B`), smaller ones are exact (`1,295`). A `Crossing`
//! row is commented out, and the index figures above the tables are
//! placeholders a script fills in (`00,000`), so neither is read.
//!
//! The page does not say which boards the foreign rows cover. For
//! 2026-11-06 its board rows matched idx.co.id's daily stock summary
//! (regular 7.83 T against 7,825.6 B; negotiated and cash 2.49 T against
//! the non-regular 2,493.0 B), while its foreign lots (121.98 M bought,
//! 165.65 M sold) were above idx.co.id's regular-board foreign shares
//! (96.78 M and 143.90 M lots).
//!
//! The page carries no date: [`super::topbroker::session_date`] dates it,
//! as it dates the broker ranking.

use chrono::NaiveDate;

use crate::error::{IdxError, Result};

use super::parse::{cells, number};
use super::topbroker::{headings, uncommented};
use super::NAME;

/// The heads of the page's two tables, in order: rows are read by label,
/// their figures by position, so a page headed otherwise is refused.
const HEADS: [[&str; 4]; 2] = [
    ["Board", "Value", "Lot", "Freq"],
    ["Foreign", "Value", "Lot", "Freq"],
];

/// The rows read, by label.
const TOTAL: &str = "TOTAL";
const BUY: &str = "F. Buy";
const SELL: &str = "F. Sell";
const NET: &str = "F. NET";

/// The market's trading in one session as the page sums it up.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketSummary {
    /// The session, from [`super::topbroker::session_date`].
    pub date: NaiveDate,
    /// `TOTAL`: value traded on the regular, negotiated and cash boards, IDR.
    pub value: Option<f64>,
    /// `F. Buy`: value foreign investors bought, IDR.
    pub foreign_buy_value: Option<f64>,
    /// `F. Sell`: value foreign investors sold, IDR.
    pub foreign_sell_value: Option<f64>,
    /// `F. NET`: value foreign investors bought minus sold, IDR, as
    /// published (it carries its own two decimals).
    pub foreign_net_value: Option<f64>,
    /// At least one figure was abbreviated (`10.32 T`).
    pub rounded: bool,
}

impl MarketSummary {
    /// `false` for a page without trades (no value traded or bought).
    pub fn traded(&self) -> bool {
        [self.value, self.foreign_buy_value, self.foreign_sell_value]
            .iter()
            .any(|v| v.is_some_and(|x| x > 0.0))
    }
}

/// `Board, Value, Lot, Freq` for messages.
fn show(heads: &[Vec<String>]) -> String {
    if heads.is_empty() {
        return "none".into();
    }
    heads
        .iter()
        .map(|h| h.join(", "))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The market summary on a `ma-marketstatistic.php` page, for the session
/// `date`: the page does not say which. A page whose heads are not
/// [`HEADS`], or that lacks one of the rows read, is refused.
pub fn summary(html: &str, date: NaiveDate) -> Result<MarketSummary> {
    let html = uncommented(html);
    let heads: Vec<Vec<String>> = html
        .match_indices("<thead")
        .map(|(i, _)| {
            headings(&html[i..])
                .into_iter()
                .map(|(label, _)| label)
                .collect()
        })
        .collect();
    let want: Vec<Vec<String>> = HEADS
        .iter()
        .map(|h| h.iter().map(|l| l.to_string()).collect())
        .collect();
    if heads != want {
        return Err(IdxError::provider(
            NAME,
            format!(
                "unexpected market summary layout: the tables are headed {}, not {}",
                show(&heads),
                show(&want)
            ),
        ));
    }
    let mut rounded = false;
    let mut value = |label: &str| -> Result<Option<f64>> {
        let row = html
            .split("</tr>")
            .map(cells)
            .find(|c| c.len() >= 4 && c[0].text == label)
            .ok_or_else(|| {
                IdxError::provider(
                    NAME,
                    format!("unexpected market summary page (no {label} row)"),
                )
            })?;
        Ok(number(&row[1].text).map(|(v, r)| {
            rounded |= r;
            v
        }))
    };
    let (total, buy, sell, net) = (value(TOTAL)?, value(BUY)?, value(SELL)?, value(NET)?);
    Ok(MarketSummary {
        date,
        value: total,
        foreign_buy_value: buy,
        foreign_sell_value: sell,
        foreign_net_value: net,
        rounded,
    })
}

/// A fingerprint of the figures as published, to tell the same summary read
/// again from another session's.
pub fn digest(s: &MarketSummary) -> String {
    use sha2::{Digest, Sha256};
    let text = format!(
        "{:?}|{:?}|{:?}|{:?}",
        s.value, s.foreign_buy_value, s.foreign_sell_value, s.foreign_net_value
    );
    hex::encode(Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../../../tests/fixtures/ipot/marketstatistic_20261106.html");

    fn friday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 6).unwrap()
    }

    #[test]
    fn the_session_total_and_the_foreign_side() {
        let s = summary(PAGE, friday()).unwrap();
        assert_eq!(s.date, friday());
        assert_eq!(s.value, Some(10_320_000_000_000.0));
        assert_eq!(s.foreign_buy_value, Some(3_760_000_000_000.0));
        assert_eq!(s.foreign_sell_value, Some(3_430_000_000_000.0));
        // As published: buy less sell is 330 B at the page's precision.
        assert_eq!(s.foreign_net_value, Some(329_330_000_000.0));
        assert!(s.rounded);
        assert!(s.traded());
    }

    #[test]
    fn a_net_sale_keeps_its_sign_and_an_empty_page_has_no_trades() {
        let sold = PAGE.replacen(">329.33 B<", ">-1.20 T<", 1);
        let s = summary(&sold, friday()).unwrap();
        assert_eq!(s.foreign_net_value, Some(-1_200_000_000_000.0));
        let zero = PAGE
            .replace(">10.32 T<", ">0<")
            .replace(">3.76 T<", ">0<")
            .replace(">3.43 T<", ">0<");
        assert!(!summary(&zero, friday()).unwrap().traded());
        assert_ne!(digest(&s), digest(&summary(PAGE, friday()).unwrap()));
    }

    #[test]
    fn a_page_headed_otherwise_or_without_a_row_is_refused() {
        let moved = PAGE.replacen(
            r#"<th align="right">Value</th>"#,
            r#"<th align="right">Lot</th>"#,
            1,
        );
        let err = summary(&moved, friday()).unwrap_err();
        assert_eq!(err.code(), "provider");
        assert_eq!(
            err.to_string(),
            "ipot: unexpected market summary layout: the tables are headed Board, Lot, Lot, Freq; Foreign, Value, Lot, Freq, not Board, Value, Lot, Freq; Foreign, Value, Lot, Freq"
        );
        let no_net = PAGE.replacen(">F. NET<", ">F. NETT<", 1);
        assert_eq!(
            summary(&no_net, friday()).unwrap_err().to_string(),
            "ipot: unexpected market summary page (no F. NET row)"
        );
        assert!(summary("Invalid stock code!", friday()).is_err());
    }
}
