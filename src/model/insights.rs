//! `stocks insights` output: MSN Money's rule-based statements about one company.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Symbol;

/// How MSN rated one statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InsightStatus {
    Good,
    Bad,
    Neutral,
    /// MSN could not rate it (its `NA`), usually for missing data.
    NotRated,
}

impl InsightStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            InsightStatus::Good => "good",
            InsightStatus::Bad => "bad",
            InsightStatus::Neutral => "neutral",
            InsightStatus::NotRated => "not_rated",
        }
    }
}

/// What the company's value was measured against. Derived by idx from MSN's
/// benchmark object: an aggregate over the peer list, an aggregate over the
/// industry, the company's own earlier quarters or years, the current price
/// (analyst target) or a fixed threshold (analyst rating).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InsightBenchmark {
    Peers,
    Industry,
    OwnHistory,
    Price,
    Threshold,
}

impl InsightBenchmark {
    pub fn as_str(&self) -> &'static str {
        match self {
            InsightBenchmark::Peers => "peers",
            InsightBenchmark::Industry => "industry",
            InsightBenchmark::OwnHistory => "own_history",
            InsightBenchmark::Price => "price",
            InsightBenchmark::Threshold => "threshold",
        }
    }
}

/// One rated statement, as MSN wrote it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Insight {
    /// `Valuation`, `Earnings`, `Growth`, `Performance` or `Health`.
    pub category: String,
    /// Short rule name, e.g. `P/B ratio`. The same name can appear twice with
    /// different benchmarks (peers and industry).
    pub name: String,
    pub statement: Option<String>,
    pub short_statement: Option<String>,
    pub status: InsightStatus,
    /// `lower_is_better` or `higher_is_better` (MSN's criteria, snake-cased).
    pub criteria: Option<String>,
    /// The company's figure as MSN reports it. Units differ by statement:
    /// multiples (P/E 10.2), fractions (growth 0.038), rupiah (revenue).
    /// Null when MSN could not rate the statement and sent 0 for both figures.
    pub value: Option<f64>,
    /// The benchmark's figure, same units as `value`.
    pub benchmark_value: Option<f64>,
    pub compared_with: Option<InsightBenchmark>,
}

/// MSN Money insights for one ticker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Insights {
    pub symbol: Symbol,
    pub name: Option<String>,
    /// When MSN last recomputed the statements (UTC, whole seconds).
    pub updated: Option<DateTime<Utc>>,
    pub good: usize,
    pub bad: usize,
    pub neutral: usize,
    pub not_rated: usize,
    /// IDX tickers in MSN's peer group, in MSN's order. Empty when the peer
    /// lookup failed (a warning says so).
    pub peers: Vec<Symbol>,
    /// Every statement in MSN's order (category by category).
    pub insights: Vec<Insight>,
}

impl Insights {
    /// Recounts `good`, `bad`, `neutral` and `not_rated` from the items.
    pub fn recount(&mut self) {
        let n = |s: InsightStatus| self.insights.iter().filter(|i| i.status == s).count();
        let (good, bad, neutral, not_rated) = (
            n(InsightStatus::Good),
            n(InsightStatus::Bad),
            n(InsightStatus::Neutral),
            n(InsightStatus::NotRated),
        );
        self.good = good;
        self.bad = bad;
        self.neutral = neutral;
        self.not_rated = not_rated;
    }
}
