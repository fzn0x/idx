//! `stocks rotation`: where each symbol sits on a relative rotation graph
//! against a benchmark. Every number is computed by idx from the provider's
//! daily closes (`analysis::rotation`), with an open approximation of the
//! JdK RS-Ratio and RS-Momentum; the original formulas are proprietary, so
//! the values will not match StockCharts or Optuma.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Symbol;

/// The four quadrants of a relative rotation graph, around 100 on both axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Quadrant {
    /// RS-Ratio and RS-Momentum both at or above 100: outperforming and gaining.
    Leading,
    /// RS-Ratio at or above 100, RS-Momentum below: outperforming, losing pace.
    Weakening,
    /// Both below 100: underperforming and losing pace.
    Lagging,
    /// RS-Ratio below 100, RS-Momentum at or above: underperforming, gaining pace.
    Improving,
}

impl Quadrant {
    /// The quadrant of a point; 100 counts as the right and upper side.
    pub fn of(rs_ratio: f64, rs_momentum: f64) -> Quadrant {
        match (rs_ratio >= 100.0, rs_momentum >= 100.0) {
            (true, true) => Quadrant::Leading,
            (true, false) => Quadrant::Weakening,
            (false, false) => Quadrant::Lagging,
            (false, true) => Quadrant::Improving,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Quadrant::Leading => "leading",
            Quadrant::Weakening => "weakening",
            Quadrant::Lagging => "lagging",
            Quadrant::Improving => "improving",
        }
    }
}

/// Bars the rotation is computed on.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
pub enum RotationInterval {
    /// The last session of each week (Monday to Sunday, WIB); the current week
    /// counts with its latest session so far.
    #[serde(rename = "1wk")]
    #[value(name = "1wk", alias = "week", alias = "weekly")]
    Week,
    /// Every session.
    #[serde(rename = "1d")]
    #[value(name = "1d", alias = "day", alias = "daily")]
    Day,
}

impl RotationInterval {
    pub fn as_str(self) -> &'static str {
        match self {
            RotationInterval::Week => "1wk",
            RotationInterval::Day => "1d",
        }
    }

    /// `week` or `session`, for messages.
    pub fn bar(self) -> &'static str {
        match self {
            RotationInterval::Week => "week",
            RotationInterval::Day => "session",
        }
    }
}

/// How the axes were computed, so a reader can reproduce them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RotationMethod {
    /// Always `true`: an open approximation of the JdK RS-Ratio and
    /// RS-Momentum, whose formulas are proprietary. Values differ from
    /// StockCharts' and Optuma's.
    pub approximation: bool,
    /// Bars in each rolling mean and standard deviation (`--window`).
    pub window: usize,
    /// Points kept in each symbol's tail (`--tail`).
    pub tail: usize,
    /// The centre of both axes.
    pub centre: f64,
    /// The formulas, in words.
    pub formula: String,
}

/// One point of a symbol's path on the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RotationPoint {
    /// The bar's date: the last session of its week in weekly mode.
    pub date: NaiveDate,
    /// Relative strength against the benchmark, normalised around 100.
    pub rs_ratio: f64,
    /// Rate of change of `rs_ratio`, normalised around 100.
    pub rs_momentum: f64,
    pub quadrant: Quadrant,
}

/// One symbol on the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RotationRow {
    pub symbol: Symbol,
    pub name: Option<String>,
    /// Date of the latest point; `null` when the history is too short.
    pub as_of: Option<NaiveDate>,
    /// The latest point's values; `null` when the history is too short or
    /// the symbol moved exactly with the benchmark (no spread to normalise).
    pub rs_ratio: Option<f64>,
    pub rs_momentum: Option<f64>,
    pub quadrant: Option<Quadrant>,
    /// Latest minus the tail's first point, in axis points; `null` with
    /// fewer than two points.
    pub rs_ratio_change: Option<f64>,
    pub rs_momentum_change: Option<f64>,
    /// Bars the symbol and the benchmark share, after resampling.
    pub observations: usize,
    /// The path, oldest first, ending at `as_of`: up to `method.tail` points.
    pub tail: Vec<RotationPoint>,
}

/// A relative rotation graph of several symbols against one benchmark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rotation {
    /// E.g. `^JKSE` (IHSG).
    pub benchmark: Symbol,
    pub benchmark_name: Option<String>,
    pub interval: RotationInterval,
    pub method: RotationMethod,
    /// In the order the symbols were given.
    pub rows: Vec<RotationRow>,
}
