//! Company profile, valuation ratios, financial statements and earnings.
//! These structs are the JSON contract for `stocks profile|fundamental|
//! financials|earnings|compare`. Every optional field is `null` when the
//! source did not return it.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Symbol;

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
    /// Total debt / shareholders' equity as a ratio (`0.6` = 60 %).
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
    pub currency: Option<String>,
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
    pub fn fill_from(&mut self, other: &Fundamentals) {
        macro_rules! fill {
            ($($path:ident).+) => {
                if self.$($path).+.is_none() {
                    self.$($path).+ = other.$($path).+.clone();
                }
            };
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
        fill!(valuation.price_to_book);
        fill!(valuation.price_to_sales);
        fill!(valuation.ev_to_ebitda);
        fill!(valuation.ev_to_revenue);
        fill!(profitability.roe_pct);
        fill!(profitability.roa_pct);
        fill!(profitability.gross_margin_pct);
        fill!(profitability.operating_margin_pct);
        fill!(profitability.net_margin_pct);
        fill!(health.debt_to_equity);
        fill!(health.current_ratio);
        fill!(health.quick_ratio);
        fill!(health.total_debt);
        fill!(health.total_cash);
        fill!(growth.revenue_growth_pct);
        fill!(growth.earnings_growth_pct);
        fill!(growth.revenue_cagr_5y_pct);
        fill!(growth.eps_growth_3y_pct);
        fill!(dividend.yield_pct);
        fill!(dividend.payout_ratio_pct);
        fill!(dividend.ex_dividend_date);
        fill!(per_share.eps_ttm);
        fill!(per_share.eps_forward);
        fill!(per_share.book_value);
        fill!(per_share.revenue);
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
    fn financial_sector_detection() {
        let mut f = Fundamentals::empty(Symbol::parse("BBCA").unwrap(), "t");
        f.sector = Some("Financial Services".into());
        assert!(f.is_financial_sector());
        f.sector = Some("Technology".into());
        f.industry = Some("Telecommunications Services".into());
        assert!(!f.is_financial_sector());
    }

    #[test]
    fn statement_period_serialises_lowercase() {
        assert_eq!(
            serde_json::to_string(&StatementPeriod::Ttm).unwrap(),
            "\"ttm\""
        );
    }
}
