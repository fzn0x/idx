//! `idx stockbit actions` / `idx stockbit calendar`: corporate actions as one
//! structured row each, from the user's own Stockbit session.
//!
//! Unlike `idx.actions.v1` (IPOT's announcement records, with their texts) and
//! `idx.calendar.v1` (idx.co.id's milestones, with their descriptions), these
//! rows carry the dates and terms of each action and no text. Every field the
//! source does not give is `null`.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Symbol;
use crate::error::{IdxError, Result};

/// What a corporate action is.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
    clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum CorporateEventKind {
    /// Cash dividend.
    Dividend,
    /// Dividend paid in shares.
    #[value(name = "stock_dividend", alias = "stock-dividend")]
    StockDividend,
    /// Bonus shares.
    Bonus,
    /// Rights issue (HMETD).
    #[value(alias = "rights_issue", alias = "rights-issue")]
    Rights,
    /// Warrants, the company's own and structured warrants (listed under
    /// their own codes).
    #[value(alias = "warrants")]
    Warrant,
    /// Stock split.
    #[value(alias = "stock_split", alias = "stock-split")]
    Split,
    /// Reverse stock split.
    #[value(name = "reverse_split", alias = "reverse-split")]
    ReverseSplit,
    /// Shareholders' meeting (RUPS).
    #[value(alias = "meeting")]
    Rups,
    /// Public expose.
    #[value(name = "public_expose", alias = "public-expose", alias = "expose")]
    PublicExpose,
    /// Initial public offering.
    Ipo,
    /// Tender offer.
    #[value(name = "tender_offer", alias = "tender-offer", alias = "tender")]
    TenderOffer,
}

impl CorporateEventKind {
    pub const ALL: [CorporateEventKind; 11] = [
        CorporateEventKind::Dividend,
        CorporateEventKind::StockDividend,
        CorporateEventKind::Bonus,
        CorporateEventKind::Rights,
        CorporateEventKind::Warrant,
        CorporateEventKind::Split,
        CorporateEventKind::ReverseSplit,
        CorporateEventKind::Rups,
        CorporateEventKind::PublicExpose,
        CorporateEventKind::Ipo,
        CorporateEventKind::TenderOffer,
    ];

    /// One stock's actions when no kind is asked for: all but the IPO.
    pub const STOCK_DEFAULT: [CorporateEventKind; 10] = [
        CorporateEventKind::Dividend,
        CorporateEventKind::StockDividend,
        CorporateEventKind::Bonus,
        CorporateEventKind::Rights,
        CorporateEventKind::Warrant,
        CorporateEventKind::Split,
        CorporateEventKind::ReverseSplit,
        CorporateEventKind::Rups,
        CorporateEventKind::PublicExpose,
        CorporateEventKind::TenderOffer,
    ];

    /// The market calendar when no kind is asked for: all but warrants (most
    /// market-wide warrant rows are structured warrants, dozens a month).
    pub const CALENDAR_DEFAULT: [CorporateEventKind; 10] = [
        CorporateEventKind::Dividend,
        CorporateEventKind::StockDividend,
        CorporateEventKind::Bonus,
        CorporateEventKind::Rights,
        CorporateEventKind::Split,
        CorporateEventKind::ReverseSplit,
        CorporateEventKind::Rups,
        CorporateEventKind::PublicExpose,
        CorporateEventKind::Ipo,
        CorporateEventKind::TenderOffer,
    ];

    /// The JSON name.
    pub fn as_str(&self) -> &'static str {
        match self {
            CorporateEventKind::Dividend => "dividend",
            CorporateEventKind::StockDividend => "stock_dividend",
            CorporateEventKind::Bonus => "bonus",
            CorporateEventKind::Rights => "rights",
            CorporateEventKind::Warrant => "warrant",
            CorporateEventKind::Split => "split",
            CorporateEventKind::ReverseSplit => "reverse_split",
            CorporateEventKind::Rups => "rups",
            CorporateEventKind::PublicExpose => "public_expose",
            CorporateEventKind::Ipo => "ipo",
            CorporateEventKind::TenderOffer => "tender_offer",
        }
    }

    /// Entitlement kinds: the row's date is its cum date.
    pub fn is_entitlement(&self) -> bool {
        matches!(
            self,
            CorporateEventKind::Dividend
                | CorporateEventKind::StockDividend
                | CorporateEventKind::Bonus
                | CorporateEventKind::Rights
                | CorporateEventKind::Split
                | CorporateEventKind::ReverseSplit
        )
    }

    /// `dividend,rights`, `all`, or nothing (then `default`) into kinds, in
    /// the order given, without repeats. Accepts the aliases (`meeting`,
    /// `expose`, `tender`, hyphens for underscores).
    pub fn parse_list(
        raw: Option<&str>,
        default: &[CorporateEventKind],
    ) -> Result<Vec<CorporateEventKind>> {
        let mut out = Vec::new();
        for piece in raw
            .unwrap_or("")
            .split(|c: char| c == ',' || c.is_whitespace())
        {
            let p = piece.trim();
            if p.is_empty() {
                continue;
            }
            let add: Vec<CorporateEventKind> = if p.eq_ignore_ascii_case("all") {
                Self::ALL.to_vec()
            } else {
                vec![
                    <CorporateEventKind as clap::ValueEnum>::from_str(p, true).map_err(|_| {
                        IdxError::Usage(format!(
                            "unknown corporate action kind {p:?}; use all or {}",
                            Self::ALL.map(|k| k.as_str()).join(", ")
                        ))
                    })?,
                ]
            };
            for k in add {
                if !out.contains(&k) {
                    out.push(k);
                }
            }
        }
        if out.is_empty() {
            out = default.to_vec();
        }
        Ok(out)
    }
}

impl std::fmt::Display for CorporateEventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One corporate action. Dates are the exchange's (WIB); prices IDR per share.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CorporateEvent {
    /// The listed code as the source gives it; for a structured warrant this
    /// is the warrant's own code (e.g. `ASIIZPCN7A`), not the underlying's.
    pub symbol: Symbol,
    /// Issuer name when the source gives one (IPOs, tender offers, some RUPS).
    pub name: Option<String>,
    pub kind: CorporateEventKind,
    /// The row's date: the cum date for entitlements (dividends, bonus,
    /// rights, splits; the ex date when there is no cum date), the first
    /// trading day for warrants (else the first exercise day), the meeting or
    /// expose day, the IPO's listing day, a tender offer's first day.
    pub date: NaiveDate,
    pub cum_date: Option<NaiveDate>,
    pub ex_date: Option<NaiveDate>,
    /// Recording date; for a RUPS, the date that decides who may attend.
    pub recording_date: Option<NaiveDate>,
    /// Payment or distribution date; a tender offer's settlement.
    pub distribution_date: Option<NaiveDate>,
    /// Cash dividend per share, in `currency`.
    pub dividend_per_share: Option<f64>,
    /// ISO currency code of the dividend (`IDR`, `USD`); dividends only.
    pub currency: Option<String>,
    /// Old : new as published (1 : 5 for a five-for-one split; 655 held : 104
    /// rights).
    pub ratio_old: Option<f64>,
    pub ratio_new: Option<f64>,
    /// The source's price adjustment factor.
    pub factor: Option<f64>,
    /// IDR per share: the rights or warrant exercise price, the tender offer
    /// price, the final IPO price.
    pub price: Option<f64>,
    /// Rights or warrant trading window.
    pub trading_start: Option<NaiveDate>,
    pub trading_end: Option<NaiveDate>,
    /// Warrant exercise window.
    pub exercise_start: Option<NaiveDate>,
    pub exercise_end: Option<NaiveDate>,
    /// IPO offering or tender offer period.
    pub offer_start: Option<NaiveDate>,
    pub offer_end: Option<NaiveDate>,
    /// IPO listing date.
    pub listing_date: Option<NaiveDate>,
    /// Shares offered (IPO) or sought (tender offer).
    pub shares: Option<f64>,
    /// The same as a percentage of the company's shares.
    pub shares_pct: Option<f64>,
    /// RUPS or expose time as published (WIB), e.g. `10:30` or `13:00:00`.
    pub time: Option<String>,
    /// Venue as published, whitespace collapsed.
    pub venue: Option<String>,
    /// The source's note, when not empty.
    pub note: Option<String>,
    /// The source's row id.
    pub id: Option<String>,
}

impl CorporateEvent {
    /// A row with only the identity filled in.
    pub fn new(symbol: Symbol, kind: CorporateEventKind, date: NaiveDate) -> Self {
        CorporateEvent {
            symbol,
            name: None,
            kind,
            date,
            cum_date: None,
            ex_date: None,
            recording_date: None,
            distribution_date: None,
            dividend_per_share: None,
            currency: None,
            ratio_old: None,
            ratio_new: None,
            factor: None,
            price: None,
            trading_start: None,
            trading_end: None,
            exercise_start: None,
            exercise_end: None,
            offer_start: None,
            offer_end: None,
            listing_date: None,
            shares: None,
            shares_pct: None,
            time: None,
            venue: None,
            note: None,
            id: None,
        }
    }
}

/// One stock's corporate actions (`stockbit actions`) or the market's in a
/// date range (`stockbit calendar`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CorporateEvents {
    /// The stock asked for; `null` for the market calendar.
    pub symbol: Option<Symbol>,
    /// Company name from idx's bundled issuer list (one stock only).
    pub name: Option<String>,
    /// First day of the range idx applied; `null` when open.
    pub from: Option<NaiveDate>,
    /// Last day of the range idx applied; `null` when open.
    pub to: Option<NaiveDate>,
    /// The kinds read, one Stockbit request each.
    pub kinds: Vec<CorporateEventKind>,
    /// Rows the source returned for these kinds before the date range.
    pub total: usize,
    /// Newest first for one stock, oldest first for the calendar.
    pub actions: Vec<CorporateEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use CorporateEventKind as K;

    #[test]
    fn kinds_parse_with_aliases_and_defaults() {
        assert_eq!(
            K::parse_list(Some("meeting, expose,tender"), &K::ALL).unwrap(),
            [K::Rups, K::PublicExpose, K::TenderOffer]
        );
        assert_eq!(
            K::parse_list(Some("stock-dividend,REVERSE_SPLIT,rights_issue"), &K::ALL).unwrap(),
            [K::StockDividend, K::ReverseSplit, K::Rights]
        );
        assert_eq!(
            K::parse_list(Some("dividend,dividend"), &K::ALL).unwrap(),
            [K::Dividend]
        );
        assert_eq!(K::parse_list(Some("all"), &[]).unwrap(), K::ALL);
        assert_eq!(
            K::parse_list(None, &K::STOCK_DEFAULT).unwrap(),
            K::STOCK_DEFAULT
        );
        assert_eq!(
            K::parse_list(Some(" "), &K::CALENDAR_DEFAULT).unwrap(),
            K::CALENDAR_DEFAULT
        );
        assert!(!K::STOCK_DEFAULT.contains(&K::Ipo));
        assert!(!K::CALENDAR_DEFAULT.contains(&K::Warrant));
        let e = K::parse_list(Some("dividend,nope"), &K::ALL).unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert!(e.to_string().contains("tender_offer"), "{e}");
        for k in K::ALL {
            assert_eq!(K::parse_list(Some(k.as_str()), &[]).unwrap(), [k]);
            assert_eq!(
                serde_json::to_value(k).unwrap(),
                serde_json::json!(k.as_str())
            );
        }
    }
}
