//! Institutional and fund holders of one stock, as a market-data vendor lists
//! them (MSN Money's ownership data), for `idx ownership institutions`. Unlike
//! the KSEI register (`model::ownership`), every holder reports on its own date
//! and the lists stop at the largest few. The buy/sell `signal` and the `flow`
//! counts are computed by idx from the shares and the change the source gives.
//! These structs are the JSON contract (`idx.ownership.institutions.v1`).

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Symbol;

/// Which of the source's two lists a holder row comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HolderKind {
    /// An investment manager or other institution, with its whole position.
    Institution,
    /// One fund. Its shares are also part of its manager's institutional
    /// position, so the two lists must never be added together.
    Fund,
}

/// What a holder did since its previous report, computed by idx.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HolderSignal {
    /// Opened the position: the change equals the whole holding.
    New,
    /// Added shares.
    Buyer,
    /// Sold some shares and still holds the rest.
    Seller,
    /// Sold out: holds 0 shares after a sale.
    Exited,
    /// Same number of shares as at its previous report.
    Unchanged,
}

impl HolderSignal {
    pub fn as_str(&self) -> &'static str {
        match self {
            HolderSignal::New => "new",
            HolderSignal::Buyer => "buyer",
            HolderSignal::Seller => "seller",
            HolderSignal::Exited => "exited",
            HolderSignal::Unchanged => "unchanged",
        }
    }
}

/// The signal for a holding of `shares` that changed by `change` shares since
/// the holder's previous report. `None` unless both are known.
pub fn signal(shares: Option<u64>, change: Option<i64>) -> Option<HolderSignal> {
    let (shares, change) = (shares?, change?);
    let held = i64::try_from(shares).ok()?;
    Some(if held == 0 && change < 0 {
        HolderSignal::Exited
    } else if held > 0 && change == held {
        HolderSignal::New
    } else if change > 0 {
        HolderSignal::Buyer
    } else if change < 0 {
        HolderSignal::Seller
    } else {
        HolderSignal::Unchanged
    })
}

/// One institution or fund holding the stock, as of its own report date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InstitutionalHolder {
    /// `institution` or `fund`.
    pub kind: HolderKind,
    /// Position in the source's list, largest holding first (1-based, per kind).
    pub rank: u32,
    /// Holder name as the source prints it.
    pub name: String,
    /// Shares held at `report_date`.
    pub shares: Option<u64>,
    /// Shares bought (positive) or sold (negative) since the holder's previous report.
    pub shares_change: Option<i64>,
    /// Holding as a percent of shares outstanding, e.g. `2.09` for 2.09 %.
    pub pct_outstanding: Option<f64>,
    /// `shares_change` as percentage points of shares outstanding, e.g. `0.0165`.
    pub pct_outstanding_change: Option<f64>,
    /// Funds only: weight of this stock in the fund's portfolio, in percent
    /// (`0.0211` is 0.0211 %). `null` for institutions.
    pub pct_portfolio: Option<f64>,
    /// The holder's own report date; holders report on different dates.
    pub report_date: Option<NaiveDate>,
    /// Computed by idx: `new`, `buyer`, `seller`, `exited` or `unchanged`;
    /// `null` when `shares` or `shares_change` is missing.
    pub signal: Option<HolderSignal>,
}

/// Signal counts over one list, computed by idx.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HolderFlow {
    pub new: usize,
    pub buyers: usize,
    pub sellers: usize,
    pub exited: usize,
    pub unchanged: usize,
    /// Sum of `shares_change` over the list's holders that report one. Each
    /// holder's change runs from its own previous report, so this is a flow
    /// indicator, not a change over one period.
    pub net_shares_change: i64,
}

impl HolderFlow {
    /// Counts the signals of `holders` and sums their changes.
    pub fn of<'a>(holders: impl IntoIterator<Item = &'a InstitutionalHolder>) -> Self {
        let mut f = HolderFlow::default();
        for h in holders {
            match h.signal {
                Some(HolderSignal::New) => f.new += 1,
                Some(HolderSignal::Buyer) => f.buyers += 1,
                Some(HolderSignal::Seller) => f.sellers += 1,
                Some(HolderSignal::Exited) => f.exited += 1,
                Some(HolderSignal::Unchanged) => f.unchanged += 1,
                None => {}
            }
            f.net_shares_change = f
                .net_shares_change
                .saturating_add(h.shares_change.unwrap_or(0));
        }
        f
    }
}

/// [`HolderFlow`] for each list. Never add the two: funds overlap their managers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstitutionalFlow {
    pub institutions: HolderFlow,
    pub funds: HolderFlow,
}

/// `ownership institutions`: who holds the stock among institutions and funds,
/// and whether each bought or sold since its previous report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InstitutionalOwnership {
    pub symbol: Symbol,
    pub name: Option<String>,
    /// Percent of shares outstanding held by all institutions, as the source
    /// reports it (not the sum of the listed rows).
    pub institutions_pct: Option<f64>,
    /// Percent of shares outstanding held by the ten largest institutions.
    pub top10_institutions_pct: Option<f64>,
    /// Percent of shares outstanding held by funds. Funds sit inside their
    /// managers' institutional holdings: do not add it to `institutions_pct`.
    pub funds_pct: Option<f64>,
    /// Free float as a percent of shares outstanding, as the source reports it.
    pub float_pct: Option<f64>,
    /// Earliest `report_date` among the holders.
    pub oldest_report: Option<NaiveDate>,
    /// Latest `report_date` among the holders.
    pub newest_report: Option<NaiveDate>,
    /// Signal counts per list, computed by idx.
    pub flow: InstitutionalFlow,
    /// Institutions first, then funds, each in the source's rank order.
    pub holders: Vec<InstitutionalHolder>,
}

impl InstitutionalOwnership {
    /// A report with no figures and no holders.
    pub fn empty(symbol: Symbol) -> Self {
        InstitutionalOwnership {
            symbol,
            name: None,
            institutions_pct: None,
            top10_institutions_pct: None,
            funds_pct: None,
            float_pct: None,
            oldest_report: None,
            newest_report: None,
            flow: InstitutionalFlow::default(),
            holders: vec![],
        }
    }

    /// Recomputes `flow` and the report-date range from `holders`.
    pub fn summarise(&mut self) {
        let of = |k: HolderKind| HolderFlow::of(self.holders.iter().filter(|h| h.kind == k));
        self.flow = InstitutionalFlow {
            institutions: of(HolderKind::Institution),
            funds: of(HolderKind::Fund),
        };
        let dates = self.holders.iter().filter_map(|h| h.report_date);
        self.oldest_report = dates.clone().min();
        self.newest_report = dates.max();
    }

    /// Holders of one kind, in rank order.
    pub fn of_kind(&self, kind: HolderKind) -> impl Iterator<Item = &InstitutionalHolder> {
        self.holders.iter().filter(move |h| h.kind == kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_rule() {
        use HolderSignal::*;
        let cases = [
            (Some(5_821_200), Some(0), Some(Unchanged)),
            (Some(1_256_881), Some(-174_219), Some(Seller)),
            (Some(482_400), Some(52_200), Some(Buyer)),
            (Some(482_400), Some(482_400), Some(New)),
            (Some(0), Some(-3_434_200), Some(Exited)),
            (Some(0), Some(0), Some(Unchanged)),
            (None, Some(10), None),
            (Some(10), None, None),
            (None, None, None),
        ];
        for (shares, change, want) in cases {
            assert_eq!(signal(shares, change), want, "{shares:?} {change:?}");
        }
        assert_eq!(serde_json::to_string(&New).unwrap(), "\"new\"");
        assert_eq!(Exited.as_str(), "exited");
    }

    #[test]
    fn flow_counts_and_sums_per_kind() {
        let row = |kind, shares: u64, change: i64, date: &str| InstitutionalHolder {
            kind,
            rank: 1,
            name: "x".into(),
            shares: Some(shares),
            shares_change: Some(change),
            pct_outstanding: None,
            pct_outstanding_change: None,
            pct_portfolio: None,
            report_date: NaiveDate::parse_from_str(date, "%Y-%m-%d").ok(),
            signal: signal(Some(shares), Some(change)),
        };
        let mut r = InstitutionalOwnership::empty(Symbol::parse("JTPE").unwrap());
        r.holders = vec![
            row(HolderKind::Institution, 10, 10, "2026-09-30"),
            row(HolderKind::Institution, 0, -5, "2025-12-31"),
            row(HolderKind::Fund, 7, 2, "2026-06-30"),
        ];
        r.summarise();
        assert_eq!(
            r.flow.institutions,
            HolderFlow {
                new: 1,
                exited: 1,
                net_shares_change: 5,
                ..Default::default()
            }
        );
        assert_eq!(r.flow.funds.buyers, 1);
        assert_eq!(r.oldest_report.unwrap().to_string(), "2025-12-31");
        assert_eq!(r.newest_report.unwrap().to_string(), "2026-09-30");
        assert_eq!(r.of_kind(HolderKind::Fund).count(), 1);

        // Absurd upstream changes saturate instead of overflowing.
        r.holders = vec![
            row(HolderKind::Fund, 1, i64::MAX, "2026-06-30"),
            row(HolderKind::Fund, 1, i64::MAX, "2026-06-30"),
        ];
        r.summarise();
        assert_eq!(r.flow.funds.net_shares_change, i64::MAX);
    }
}
