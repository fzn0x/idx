//! Company profile, valuation ratios, financial statements and earnings.
//! These structs are the JSON contract for `stocks profile|fundamental|
//! financials|earnings|compare`. Every optional field is `null` when the
//! source did not return it.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Symbol;
use crate::error::{IdxError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Director {
    pub name: String,
    pub title: Option<String>,
}

/// Who the company is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Profile {
    pub symbol: Symbol,
    pub name: Option<String>,
    pub sector: Option<String>,
    pub industry: Option<String>,
    /// Long business description as published by the source.
    pub summary: Option<String>,
    pub employees: Option<u64>,
    pub founded_year: Option<u32>,
    pub website: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub exchange: Option<String>,
    pub currency: Option<String>,
    pub market_cap: Option<f64>,
    pub shares_outstanding: Option<u64>,
    /// Board and management when the source lists them (MSN does, Yahoo does not).
    #[serde(default)]
    pub directors: Vec<Director>,
    /// Endpoint that produced this record, e.g. `yahoo:quoteSummary`.
    pub source: String,
}

impl Profile {
    /// Fills `null`s and an empty board from `other`; keeps existing values.
    pub fn fill_from(&mut self, other: &Profile) {
        macro_rules! fill {
            ($f:ident) => {
                if self.$f.is_none() {
                    self.$f = other.$f.clone();
                }
            };
        }
        fill!(name);
        fill!(sector);
        fill!(industry);
        fill!(summary);
        fill!(employees);
        fill!(founded_year);
        fill!(website);
        fill!(city);
        fill!(country);
        fill!(exchange);
        fill!(currency);
        fill!(market_cap);
        fill!(shares_outstanding);
        if self.directors.is_empty() {
            self.directors = other.directors.clone();
        }
        if !self.source.contains(&other.source) {
            self.source = format!("{}, {}", self.source, other.source);
        }
    }

    /// Whether another provider could still add something useful.
    pub fn has_gaps(&self) -> bool {
        self.founded_year.is_none() || self.directors.is_empty() || self.sector.is_none()
    }

    pub fn empty(symbol: Symbol, source: &str) -> Self {
        Profile {
            symbol,
            name: None,
            sector: None,
            industry: None,
            summary: None,
            employees: None,
            founded_year: None,
            website: None,
            city: None,
            country: None,
            exchange: None,
            currency: None,
            market_cap: None,
            shares_outstanding: None,
            directors: vec![],
            source: source.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Valuation {
    pub market_cap: Option<f64>,
    pub enterprise_value: Option<f64>,
    pub pe_trailing: Option<f64>,
    pub pe_forward: Option<f64>,
    /// Price/earnings-to-growth.
    pub peg: Option<f64>,
    pub price_to_book: Option<f64>,
    pub price_to_sales: Option<f64>,
    pub ev_to_ebitda: Option<f64>,
    pub ev_to_revenue: Option<f64>,
}

/// All margins and returns are percentages (`21.8` means 21.8 %).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Profitability {
    pub roe_pct: Option<f64>,
    pub roa_pct: Option<f64>,
    pub gross_margin_pct: Option<f64>,
    pub operating_margin_pct: Option<f64>,
    pub net_margin_pct: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Health {
    /// Total debt / shareholders' equity as a ratio (`0.6` = 60 %). `null` when
    /// the source has none, including MSN's placeholder for a company with
    /// negative equity.
    pub debt_to_equity: Option<f64>,
    pub current_ratio: Option<f64>,
    pub quick_ratio: Option<f64>,
    pub total_debt: Option<f64>,
    pub total_cash: Option<f64>,
}

/// Growth rates in percent. `revenue_growth_pct` and `earnings_growth_pct`
/// compare the latest quarter with the same quarter a year earlier.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Growth {
    pub revenue_growth_pct: Option<f64>,
    pub earnings_growth_pct: Option<f64>,
    pub revenue_cagr_5y_pct: Option<f64>,
    pub eps_growth_3y_pct: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Dividend {
    /// Trailing dividend yield in percent.
    pub yield_pct: Option<f64>,
    pub payout_ratio_pct: Option<f64>,
    pub ex_dividend_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PerShare {
    pub eps_ttm: Option<f64>,
    pub eps_forward: Option<f64>,
    pub book_value: Option<f64>,
    pub revenue: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Analyst {
    /// `strong_buy` | `buy` | `hold` | `sell` | `strong_sell` as reported.
    pub recommendation: Option<String>,
    pub target_mean: Option<f64>,
    pub target_high: Option<f64>,
    pub target_low: Option<f64>,
    pub analysts: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerdictLabel {
    Undervalued,
    Fair,
    Overvalued,
    InsufficientData,
}

impl VerdictLabel {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerdictLabel::Undervalued => "undervalued",
            VerdictLabel::Fair => "fairly valued",
            VerdictLabel::Overvalued => "overvalued",
            VerdictLabel::InsufficientData => "insufficient data",
        }
    }
}

/// Rule-based reading of the ratios. Heuristic, not advice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Verdict {
    pub label: VerdictLabel,
    /// Sum of the rule scores; positive favours cheapness and quality.
    pub score: i32,
    /// One line per rule that fired, in plain language.
    pub reasons: Vec<String>,
    /// How many of the rule inputs were available.
    pub inputs_available: usize,
}

/// Combined fundamentals report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Fundamentals {
    pub symbol: Symbol,
    pub name: Option<String>,
    /// Trading currency of the price.
    pub currency: Option<String>,
    /// Currency of the financial statements when the source says (Yahoo
    /// `financialData.financialCurrency`). When it differs from `currency`,
    /// the book, sales and EV multiples divide the price by figures in another
    /// currency, and book value, revenue per share, total debt and total cash
    /// are in this currency: none of them is filled from a source that names
    /// no statement currency.
    pub financial_currency: Option<String>,
    pub sector: Option<String>,
    pub industry: Option<String>,
    pub price: Option<f64>,
    pub beta: Option<f64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    pub valuation: Valuation,
    pub profitability: Profitability,
    pub health: Health,
    pub growth: Growth,
    pub dividend: Dividend,
    pub per_share: PerShare,
    pub analyst: Analyst,
    pub fiscal_year_end: Option<NaiveDate>,
    pub most_recent_quarter: Option<NaiveDate>,
    /// Plain-language valuation reading; `null` only when not computed.
    pub verdict: Option<Verdict>,
    /// Endpoints that contributed, in priority order.
    pub sources: Vec<String>,
}

impl Fundamentals {
    pub fn empty(symbol: Symbol, source: &str) -> Self {
        Fundamentals {
            symbol,
            name: None,
            currency: None,
            financial_currency: None,
            sector: None,
            industry: None,
            price: None,
            beta: None,
            week52_high: None,
            week52_low: None,
            valuation: Valuation::default(),
            profitability: Profitability::default(),
            health: Health::default(),
            growth: Growth::default(),
            dividend: Dividend::default(),
            per_share: PerShare::default(),
            analyst: Analyst::default(),
            fiscal_year_end: None,
            most_recent_quarter: None,
            verdict: None,
            sources: vec![source.to_string()],
        }
    }

    /// Fills every `null` field from `other`, keeping existing values.
    ///
    /// Book value and revenue per share, total debt and total cash are in the
    /// statement currency, and P/B, P/S and the EV multiples divide the price
    /// by them. Those, and `financial_currency` itself, are not filled from a
    /// record whose statements are in another currency, so every one of them
    /// published next to `financial_currency` is in that currency. A record
    /// that names no statement currency (MSN) is read as reporting in its
    /// price currency: under `auto` it fills none of them into a Yahoo record
    /// with USD statements.
    pub fn fill_from(&mut self, other: &Fundamentals) {
        macro_rules! fill {
            ($($path:ident).+) => {
                if self.$($path).+.is_none() {
                    self.$($path).+ = other.$($path).+.clone();
                }
            };
        }
        let foreign_statements = match (self.statement_currency(), other.statement_currency()) {
            (Some(ours), Some(theirs)) => !ours.eq_ignore_ascii_case(theirs),
            _ => false,
        };
        if !foreign_statements {
            fill!(financial_currency);
            fill!(valuation.price_to_book);
            fill!(valuation.price_to_sales);
            fill!(valuation.ev_to_ebitda);
            fill!(valuation.ev_to_revenue);
            fill!(health.total_debt);
            fill!(health.total_cash);
            fill!(per_share.book_value);
            fill!(per_share.revenue);
        }
        fill!(name);
        fill!(currency);
        fill!(sector);
        fill!(industry);
        fill!(price);
        fill!(beta);
        fill!(week52_high);
        fill!(week52_low);
        fill!(valuation.market_cap);
        fill!(valuation.enterprise_value);
        fill!(valuation.pe_trailing);
        fill!(valuation.pe_forward);
        fill!(valuation.peg);
        fill!(profitability.roe_pct);
        fill!(profitability.roa_pct);
        fill!(profitability.gross_margin_pct);
        fill!(profitability.operating_margin_pct);
        fill!(profitability.net_margin_pct);
        fill!(health.debt_to_equity);
        fill!(health.current_ratio);
        fill!(health.quick_ratio);
        fill!(growth.revenue_growth_pct);
        fill!(growth.earnings_growth_pct);
        fill!(growth.revenue_cagr_5y_pct);
        fill!(growth.eps_growth_3y_pct);
        fill!(dividend.yield_pct);
        fill!(dividend.payout_ratio_pct);
        fill!(dividend.ex_dividend_date);
        fill!(per_share.eps_ttm);
        fill!(per_share.eps_forward);
        fill!(analyst.recommendation);
        fill!(analyst.target_mean);
        fill!(analyst.target_high);
        fill!(analyst.target_low);
        fill!(analyst.analysts);
        fill!(fiscal_year_end);
        fill!(most_recent_quarter);
        for s in &other.sources {
            if !self.sources.contains(s) {
                self.sources.push(s.clone());
            }
        }
    }

    /// Fields the verdict and the compare table care about. Used to decide
    /// whether a second provider is worth asking.
    pub fn missing_key_fields(&self) -> usize {
        [
            self.valuation.pe_trailing,
            self.valuation.price_to_book,
            self.profitability.roe_pct,
            self.profitability.net_margin_pct,
            self.growth.revenue_growth_pct,
            self.dividend.yield_pct,
            self.health.debt_to_equity,
        ]
        .iter()
        .filter(|v| v.is_none())
        .count()
    }

    pub fn is_financial_sector(&self) -> bool {
        let s = self.sector.as_deref().unwrap_or("").to_ascii_lowercase();
        let i = self.industry.as_deref().unwrap_or("").to_ascii_lowercase();
        s.contains("financ") || i.contains("bank") || i.contains("insurance")
    }

    /// Whether shareholders' equity is below zero, read from the signs the
    /// sources report: `Some(true)` when book value per share, P/B or
    /// debt/equity is negative (P/B and debt/equity both divide by equity),
    /// `Some(false)` when at least one of them is reported and none is
    /// negative, `None` when none is reported. Yahoo blanks P/B, debt/equity
    /// and ROE for such a company but keeps `bookValue` negative; MSN keeps
    /// `bookValuePerShare` negative (GIAA, INAF, POLY, SRIL, WSKT on
    /// 2026-11-06). The sign does not depend on the statement currency.
    pub fn negative_equity(&self) -> Option<bool> {
        let signs = [
            self.per_share.book_value,
            self.valuation.price_to_book,
            self.health.debt_to_equity,
        ];
        if signs.iter().all(Option::is_none) {
            return None;
        }
        Some(signs.iter().flatten().any(|x| *x < 0.0))
    }

    /// The statements are in another currency than the price (both known),
    /// e.g. a coal miner or an airline that reports in USD while its shares
    /// trade in IDR.
    pub fn mixed_currency(&self) -> bool {
        currencies_differ(self.currency.as_deref(), self.financial_currency.as_deref())
    }

    /// The statement currency, or the price's when the source names none.
    /// `fill_from` and the screener's join use it, to keep one record's
    /// amounts in one unit.
    pub(crate) fn statement_currency(&self) -> Option<&str> {
        self.financial_currency
            .as_deref()
            .or(self.currency.as_deref())
    }
}

/// Whether the statements are in another currency than the price, both known
/// (case ignored): the test behind [`Fundamentals::mixed_currency`] and
/// `ScreenRow::mixed_currency`.
pub fn currencies_differ(price: Option<&str>, statements: Option<&str>) -> bool {
    match (price, statements) {
        (Some(price), Some(statements)) => !price.eq_ignore_ascii_case(statements),
        _ => false,
    }
}

/// Which statement series `stocks financials` returns.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum StatementPeriod {
    Annual,
    Quarterly,
    /// Trailing twelve months (flows only; balance sheet fields are null).
    Ttm,
}

impl StatementPeriod {
    pub fn as_str(&self) -> &'static str {
        match self {
            StatementPeriod::Annual => "annual",
            StatementPeriod::Quarterly => "quarterly",
            StatementPeriod::Ttm => "ttm",
        }
    }
}

impl std::fmt::Display for StatementPeriod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IncomeStatement {
    pub total_revenue: Option<f64>,
    pub gross_profit: Option<f64>,
    pub operating_income: Option<f64>,
    pub ebitda: Option<f64>,
    pub ebit: Option<f64>,
    pub interest_income: Option<f64>,
    pub interest_expense: Option<f64>,
    pub net_interest_income: Option<f64>,
    pub pretax_income: Option<f64>,
    pub tax_provision: Option<f64>,
    pub net_income: Option<f64>,
    pub net_income_common: Option<f64>,
    pub eps_basic: Option<f64>,
    pub eps_diluted: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BalanceSheet {
    pub total_assets: Option<f64>,
    pub total_liabilities: Option<f64>,
    pub stockholders_equity: Option<f64>,
    pub cash_and_equivalents: Option<f64>,
    pub total_debt: Option<f64>,
    pub net_debt: Option<f64>,
    pub shares_outstanding: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CashFlow {
    pub operating: Option<f64>,
    pub investing: Option<f64>,
    pub financing: Option<f64>,
    /// Negative when cash left the company, as reported.
    pub capital_expenditure: Option<f64>,
    pub free_cash_flow: Option<f64>,
    pub dividends_paid: Option<f64>,
}

/// One reporting period across the three statements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FinancialPeriod {
    pub end_date: NaiveDate,
    pub period: StatementPeriod,
    pub income: IncomeStatement,
    pub balance: BalanceSheet,
    pub cash_flow: CashFlow,
}

impl FinancialPeriod {
    pub fn new(end_date: NaiveDate, period: StatementPeriod) -> Self {
        FinancialPeriod {
            end_date,
            period,
            income: IncomeStatement::default(),
            balance: BalanceSheet::default(),
            cash_flow: CashFlow::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Financials {
    pub symbol: Symbol,
    pub currency: Option<String>,
    pub period: StatementPeriod,
    /// Oldest first.
    pub periods: Vec<FinancialPeriod>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EarningsQuarter {
    /// Source label such as `3Q2025`.
    pub period: String,
    pub end_date: Option<NaiveDate>,
    pub reported_date: Option<NaiveDate>,
    pub eps_actual: Option<f64>,
    pub eps_estimate: Option<f64>,
    /// Actual vs estimate in percent.
    pub surprise_pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RevenueEarnings {
    /// `2025` for a fiscal year, `3Q2025` for a quarter.
    pub period: String,
    pub revenue: Option<f64>,
    pub earnings: Option<f64>,
    pub profit_margin_pct: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NextEarnings {
    pub date: Option<NaiveDate>,
    pub date_is_estimate: bool,
    pub eps_avg: Option<f64>,
    pub eps_low: Option<f64>,
    pub eps_high: Option<f64>,
    pub eps_growth_pct: Option<f64>,
    pub revenue_avg: Option<f64>,
    pub revenue_low: Option<f64>,
    pub revenue_high: Option<f64>,
    pub analysts: Option<u32>,
}

/// Whether an earnings row covers a fiscal year or a quarter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PeriodKind {
    Annual,
    Quarterly,
}

/// Analyst consensus for one period, as the source publishes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EarningsEstimate {
    /// Source label: `0q` this quarter, `+1q` next quarter, `0y` this fiscal
    /// year, `+1y` next fiscal year.
    pub period: String,
    pub period_type: PeriodKind,
    /// Last day of the period.
    pub end_date: Option<NaiveDate>,
    pub eps_avg: Option<f64>,
    pub eps_low: Option<f64>,
    pub eps_high: Option<f64>,
    /// EPS of the same period a year earlier.
    pub eps_year_ago: Option<f64>,
    /// Consensus EPS versus a year earlier, in percent (`19.98`).
    pub eps_growth_pct: Option<f64>,
    /// Analysts behind the EPS consensus.
    pub eps_analysts: Option<u32>,
    pub revenue_avg: Option<f64>,
    pub revenue_low: Option<f64>,
    pub revenue_high: Option<f64>,
    pub revenue_year_ago: Option<f64>,
    /// Consensus revenue versus a year earlier, in percent.
    pub revenue_growth_pct: Option<f64>,
    /// Analysts behind the revenue consensus.
    pub revenue_analysts: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Earnings {
    pub symbol: Symbol,
    pub name: Option<String>,
    pub currency: Option<String>,
    /// Analyst consensus for this and next quarter and fiscal year, in source
    /// order (`0q`, `+1q`, `0y`, `+1y`).
    #[serde(default)]
    pub estimates: Vec<EarningsEstimate>,
    /// Reported quarters, oldest first.
    pub quarters: Vec<EarningsQuarter>,
    /// Fiscal-year revenue and earnings, oldest first.
    pub yearly: Vec<RevenueEarnings>,
    /// Quarterly revenue and earnings, oldest first.
    pub quarterly: Vec<RevenueEarnings>,
    pub next: Option<NextEarnings>,
    pub ex_dividend_date: Option<NaiveDate>,
    pub dividend_date: Option<NaiveDate>,
    pub source: String,
    /// What a filtered call kept (`stocks earnings --annual --quarterly
    /// --reported --estimates`, MCP `period` and `kind`); `null` when nothing
    /// was filtered. An empty list or a `null` next under a scope comes from
    /// the filter, not the source.
    #[serde(default)]
    pub scope: Option<EarningsScope>,
}

/// The rows a filtered earnings call kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EarningsScope {
    /// Fiscal-year rows: `yearly` and the `0y`/`+1y` consensus.
    pub annual: bool,
    /// Quarterly rows: `quarters`, `quarterly`, the `0q`/`+1q` consensus and `next`.
    pub quarterly: bool,
    /// Reported figures: `quarters`, `yearly` and `quarterly`.
    pub reported: bool,
    /// Analyst consensus: `estimates` and `next`.
    pub estimates: bool,
}

/// Row filter for earnings. Giving neither flag of a pair keeps both: with
/// neither `annual` nor `quarterly` every period is kept, with neither
/// `reported` nor `estimates` both kinds are.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EarningsFilter {
    pub annual: bool,
    pub quarterly: bool,
    pub reported: bool,
    pub estimates: bool,
}

impl EarningsFilter {
    /// The filter from MCP and web names: `period` `annual` | `quarterly`,
    /// `kind` `reported` | `estimates`; empty or `all` keeps both.
    pub fn from_names(period: Option<&str>, kind: Option<&str>) -> Result<Self> {
        let norm = |v: Option<&str>| v.map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
        let mut f = EarningsFilter::default();
        match norm(period).as_str() {
            "" | "all" => {}
            "annual" => f.annual = true,
            "quarterly" => f.quarterly = true,
            other => {
                return Err(IdxError::Usage(format!(
                    "invalid period {other:?}; use annual or quarterly (or leave it out for both)"
                )))
            }
        }
        match norm(kind).as_str() {
            "" | "all" => {}
            "reported" => f.reported = true,
            "estimates" => f.estimates = true,
            other => {
                return Err(IdxError::Usage(format!(
                    "invalid kind {other:?}; use reported or estimates (or leave it out for both)"
                )))
            }
        }
        Ok(f)
    }

    /// What the filter keeps, with each unset pair widened to both.
    pub fn scope(&self) -> EarningsScope {
        let periods = !self.annual && !self.quarterly;
        let kinds = !self.reported && !self.estimates;
        EarningsScope {
            annual: self.annual || periods,
            quarterly: self.quarterly || periods,
            reported: self.reported || kinds,
            estimates: self.estimates || kinds,
        }
    }

    /// `true` when the filter keeps every row.
    pub fn is_all(&self) -> bool {
        let s = self.scope();
        s.annual && s.quarterly && s.reported && s.estimates
    }

    /// Drops the rows the filter leaves out and records the scope; a no-op
    /// (scope stays `null`) when it keeps everything. Dividend dates, name,
    /// currency and source are always kept.
    pub fn apply(&self, e: &mut Earnings) {
        if self.is_all() {
            return;
        }
        let s = self.scope();
        if !(s.reported && s.quarterly) {
            e.quarters.clear();
            e.quarterly.clear();
        }
        if !(s.reported && s.annual) {
            e.yearly.clear();
        }
        if s.estimates {
            e.estimates.retain(|x| match x.period_type {
                PeriodKind::Annual => s.annual,
                PeriodKind::Quarterly => s.quarterly,
            });
        } else {
            e.estimates.clear();
        }
        if !(s.estimates && s.quarterly) {
            e.next = None;
        }
        e.scope = Some(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_from_only_fills_nulls() {
        let sym = Symbol::parse("BBCA").unwrap();
        let mut a = Fundamentals::empty(sym.clone(), "a");
        a.valuation.pe_trailing = Some(10.0);
        let mut b = Fundamentals::empty(sym, "b");
        b.valuation.pe_trailing = Some(99.0);
        b.valuation.price_to_book = Some(2.0);
        b.name = Some("Bank".into());
        a.fill_from(&b);
        assert_eq!(a.valuation.pe_trailing, Some(10.0));
        assert_eq!(a.valuation.price_to_book, Some(2.0));
        assert_eq!(a.name.as_deref(), Some("Bank"));
        assert_eq!(a.sources, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn fill_from_keeps_statement_figures_in_one_currency() {
        // GIAA on 2026-11-06: Yahoo has USD statements, MSN names none and its
        // per-share amounts are in IDR terms (book value -9.829, revenue
        // 515.77). The other MSN figures are made up.
        let sym = Symbol::parse("GIAA").unwrap();
        let mut yahoo = Fundamentals::empty(sym.clone(), "yahoo");
        yahoo.currency = Some("IDR".into());
        yahoo.financial_currency = Some("USD".into());
        yahoo.per_share.book_value = Some(-0.001);
        let mut msn = Fundamentals::empty(sym, "msn");
        msn.currency = Some("IDR".into());
        msn.per_share.book_value = Some(-9.829);
        msn.per_share.revenue = Some(515.77);
        msn.valuation.price_to_book = Some(-3.97);
        msn.valuation.price_to_sales = Some(0.2652);
        msn.valuation.ev_to_ebitda = Some(6.1);
        msn.valuation.ev_to_revenue = Some(1.4);
        msn.health.total_debt = Some(5.9e13);
        msn.health.total_cash = Some(7.4e12);
        msn.health.debt_to_equity = Some(-2.89);
        msn.growth.earnings_growth_pct = Some(-92.482);

        // `auto`: Yahoo first, MSN fills the nulls.
        let mut merged = yahoo.clone();
        merged.fill_from(&msn);
        assert_eq!(merged.financial_currency.as_deref(), Some("USD"));
        assert!(merged.mixed_currency());
        assert_eq!(merged.per_share.book_value, Some(-0.001));
        assert_eq!(merged.per_share.revenue, None, "no IDR amount labelled USD");
        assert_eq!(merged.health.total_debt, None);
        assert_eq!(merged.health.total_cash, None);
        assert_eq!(merged.valuation.price_to_book, None);
        assert_eq!(merged.valuation.price_to_sales, None);
        assert_eq!(merged.valuation.ev_to_ebitda, None);
        assert_eq!(merged.valuation.ev_to_revenue, None);
        // Ratios of two statement figures and growth rates do not depend on it.
        assert_eq!(merged.health.debt_to_equity, Some(-2.89));
        assert_eq!(merged.growth.earnings_growth_pct, Some(-92.482));
        assert_eq!(merged.sources, vec!["yahoo".to_string(), "msn".to_string()]);

        // The other way round, MSN's own amounts are not relabelled USD.
        let mut merged = msn.clone();
        merged.fill_from(&yahoo);
        assert_eq!(merged.financial_currency, None);
        assert!(!merged.mixed_currency());
        assert_eq!(merged.per_share.book_value, Some(-9.829));

        // Statements in the price's currency (BBCA): MSN fills them as before.
        let mut bbca = Fundamentals::empty(Symbol::parse("BBCA").unwrap(), "yahoo");
        bbca.currency = Some("IDR".into());
        bbca.financial_currency = Some("IDR".into());
        bbca.fill_from(&msn);
        assert_eq!(bbca.per_share.revenue, Some(515.77));
        assert_eq!(bbca.valuation.price_to_sales, Some(0.2652));
        assert_eq!(bbca.health.total_debt, Some(5.9e13));

        // Nothing known about either currency: fills as before.
        let mut unknown = Fundamentals::empty(Symbol::parse("GIAA").unwrap(), "a");
        let mut other = Fundamentals::empty(Symbol::parse("GIAA").unwrap(), "b");
        other.financial_currency = Some("USD".into());
        other.per_share.book_value = Some(-0.001);
        unknown.fill_from(&other);
        assert_eq!(unknown.financial_currency.as_deref(), Some("USD"));
        assert_eq!(unknown.per_share.book_value, Some(-0.001));
    }

    #[test]
    fn negative_equity_reads_any_sign() {
        let mut f = Fundamentals::empty(Symbol::parse("GIAA").unwrap(), "t");
        assert_eq!(f.negative_equity(), None, "nothing reported");
        f.per_share.book_value = Some(1200.0);
        assert_eq!(f.negative_equity(), Some(false));
        // Yahoo rounds a tiny USD book value to zero, sometimes with a sign.
        f.per_share.book_value = Some(-0.0);
        assert_eq!(f.negative_equity(), Some(false));
        f.per_share.book_value = Some(0.0);
        assert_eq!(f.negative_equity(), Some(false));
        f.per_share.book_value = Some(-0.001);
        assert_eq!(f.negative_equity(), Some(true));

        let mut f = Fundamentals::empty(Symbol::parse("INAF").unwrap(), "t");
        f.valuation.price_to_book = Some(-0.55);
        assert_eq!(f.negative_equity(), Some(true));
        let mut f = Fundamentals::empty(Symbol::parse("INAF").unwrap(), "t");
        f.health.debt_to_equity = Some(-2.89);
        f.per_share.book_value = Some(10.0);
        assert_eq!(f.negative_equity(), Some(true), "any negative sign wins");
    }

    #[test]
    fn mixed_currency_needs_both_currencies() {
        let mut f = Fundamentals::empty(Symbol::parse("ITMG").unwrap(), "t");
        assert!(!f.mixed_currency());
        f.currency = Some("IDR".into());
        assert!(!f.mixed_currency(), "statement currency unknown");
        f.financial_currency = Some("IDR".into());
        assert!(!f.mixed_currency());
        f.financial_currency = Some("usd".into());
        assert!(f.mixed_currency());
        f.currency = None;
        assert!(!f.mixed_currency(), "price currency unknown");
    }

    #[test]
    fn financial_sector_detection() {
        let mut f = Fundamentals::empty(Symbol::parse("BBCA").unwrap(), "t");
        f.sector = Some("Financial Services".into());
        assert!(f.is_financial_sector());
        f.sector = Some("Technology".into());
        f.industry = Some("Telecommunications Services".into());
        assert!(!f.is_financial_sector());
    }

    fn earnings() -> Earnings {
        let est = |period: &str, period_type| EarningsEstimate {
            period: period.into(),
            period_type,
            end_date: None,
            eps_avg: Some(1.0),
            eps_low: None,
            eps_high: None,
            eps_year_ago: None,
            eps_growth_pct: None,
            eps_analysts: None,
            revenue_avg: None,
            revenue_low: None,
            revenue_high: None,
            revenue_year_ago: None,
            revenue_growth_pct: None,
            revenue_analysts: None,
        };
        let re = |period: &str| RevenueEarnings {
            period: period.into(),
            revenue: Some(10.0),
            earnings: Some(1.0),
            profit_margin_pct: Some(10.0),
        };
        Earnings {
            symbol: Symbol::parse("TLKM").unwrap(),
            name: None,
            currency: Some("IDR".into()),
            estimates: vec![
                est("0q", PeriodKind::Quarterly),
                est("+1q", PeriodKind::Quarterly),
                est("0y", PeriodKind::Annual),
                est("+1y", PeriodKind::Annual),
            ],
            quarters: vec![EarningsQuarter {
                period: "3Q2026".into(),
                end_date: None,
                reported_date: None,
                eps_actual: Some(38.81),
                eps_estimate: Some(58.24),
                surprise_pct: Some(-33.36),
            }],
            yearly: vec![re("2024"), re("2025")],
            quarterly: vec![re("2Q2026"), re("3Q2026")],
            next: Some(NextEarnings::default()),
            ex_dividend_date: NaiveDate::from_ymd_opt(2026, 6, 18),
            dividend_date: None,
            source: "t".into(),
            scope: None,
        }
    }

    fn periods(e: &Earnings) -> Vec<&str> {
        e.estimates.iter().map(|x| x.period.as_str()).collect()
    }

    #[test]
    fn earnings_filter_keeps_the_asked_rows() {
        let f = |annual, quarterly, reported, estimates| EarningsFilter {
            annual,
            quarterly,
            reported,
            estimates,
        };
        // No flag, or both flags of each pair: nothing filtered, scope null.
        for filter in [f(false, false, false, false), f(true, true, true, true)] {
            let mut e = earnings();
            filter.apply(&mut e);
            assert_eq!(e, earnings());
            assert!(filter.is_all());
        }
        // --annual: yearly rows and the yearly consensus; no quarters, no next.
        let mut e = earnings();
        f(true, false, false, false).apply(&mut e);
        assert!(e.quarters.is_empty() && e.quarterly.is_empty());
        assert_eq!(e.yearly.len(), 2);
        assert_eq!(periods(&e), ["0y", "+1y"]);
        assert_eq!(e.next, None);
        assert_eq!(e.ex_dividend_date, NaiveDate::from_ymd_opt(2026, 6, 18));
        assert_eq!(
            e.scope,
            Some(EarningsScope {
                annual: true,
                quarterly: false,
                reported: true,
                estimates: true
            })
        );
        // --quarterly --estimates: the quarter consensus and the next date.
        let mut e = earnings();
        f(false, true, false, true).apply(&mut e);
        assert!(e.quarters.is_empty() && e.quarterly.is_empty() && e.yearly.is_empty());
        assert_eq!(periods(&e), ["0q", "+1q"]);
        assert!(e.next.is_some());
        // --reported: every reported row, no consensus.
        let mut e = earnings();
        f(false, false, true, false).apply(&mut e);
        assert_eq!(
            (e.quarters.len(), e.yearly.len(), e.quarterly.len()),
            (1, 2, 2)
        );
        assert!(e.estimates.is_empty());
        assert_eq!(e.next, None);
        assert!(e.scope.is_some_and(|s| s.reported && !s.estimates));
        // --quarterly --reported: reported quarters only.
        let mut e = earnings();
        f(false, true, true, false).apply(&mut e);
        assert_eq!(
            (e.quarters.len(), e.yearly.len(), e.quarterly.len()),
            (1, 0, 2)
        );
        assert!(e.estimates.is_empty() && e.next.is_none());
        // --annual --estimates
        let mut e = earnings();
        f(true, false, false, true).apply(&mut e);
        assert!(e.quarters.is_empty() && e.yearly.is_empty() && e.quarterly.is_empty());
        assert_eq!(periods(&e), ["0y", "+1y"]);
        assert_eq!(e.next, None);
    }

    #[test]
    fn earnings_filter_from_names() {
        let f = EarningsFilter::from_names(Some("Annual"), Some(" estimates ")).unwrap();
        assert!(f.annual && f.estimates && !f.quarterly && !f.reported);
        assert!(EarningsFilter::from_names(None, Some("all"))
            .unwrap()
            .is_all());
        assert!(EarningsFilter::from_names(Some(""), None).unwrap().is_all());
        let e = EarningsFilter::from_names(Some("weekly"), None).unwrap_err();
        assert_eq!(e.exit_code(), 2);
        let e = EarningsFilter::from_names(None, Some("forecast-ish")).unwrap_err();
        assert_eq!(e.exit_code(), 2);
    }

    #[test]
    fn statement_period_serialises_lowercase() {
        assert_eq!(
            serde_json::to_string(&StatementPeriod::Ttm).unwrap(),
            "\"ttm\""
        );
    }
}
