//! Relative rotation (`stocks rotation`): where each symbol sits against a
//! benchmark on a relative rotation graph. Pure functions over daily closes.
//!
//! The JdK RS-Ratio and RS-Momentum (RRG Research, shown by StockCharts and
//! Optuma) are proprietary; this is the common open approximation, labelled
//! as one wherever it is shown:
//!
//! - RS = 100 x close / benchmark close, on the dates both have
//! - RS-Ratio = 100 + (RS - mean) / sd, over the last `window` bars
//! - ROC = 100 x (RS-Ratio / previous RS-Ratio - 1)
//! - RS-Momentum = 100 + (ROC - mean) / sd, over the last `window` bars
//!
//! with the sample standard deviation. Both axes are in standard deviations
//! around 100, not percent. A symbol's first point needs `2 x window` bars.

use chrono::{Datelike, NaiveDate};

use crate::model::{Quadrant, RotationInterval, RotationPoint, RotationRow, Symbol};

use super::returns::{common, Closes};

/// Bars in each rolling mean and standard deviation, by default.
pub const DEFAULT_WINDOW: usize = 14;
/// Points in each symbol's tail, by default.
pub const DEFAULT_TAIL: usize = 10;
/// The centre of both axes.
pub const CENTRE: f64 = 100.0;
/// The formulas in words, as `method.formula` carries them.
pub const FORMULA: &str = "RS = 100 x close / benchmark close on shared dates; RS-Ratio = 100 + (RS - mean) / sd over the window; ROC = 100 x (RS-Ratio / previous RS-Ratio - 1); RS-Momentum = 100 + (ROC - mean) / sd over the window; sample sd. An open approximation of the JdK RS-Ratio and RS-Momentum, whose formulas are proprietary.";

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0 + 0.0
}

/// The last bar of each week (ISO weeks, Monday to Sunday, of the bars' own
/// dates), oldest first. The current week counts with its latest bar so far;
/// a week without a bar has no point.
pub fn weekly(s: &[(NaiveDate, f64)]) -> Closes {
    let mut out: Closes = Vec::with_capacity(s.len() / 4 + 1);
    for &(d, c) in s {
        match out.last_mut() {
            Some(last) if last.0.iso_week() == d.iso_week() => *last = (d, c),
            _ => out.push((d, c)),
        }
    }
    out
}

/// Bars a tail of `tail` points needs: the first point needs `2 x window`.
pub fn bars_needed(window: usize, tail: usize) -> usize {
    2 * window + tail.max(1) - 1
}

/// Mean and sample standard deviation; `None` under two values or when the
/// values do not spread (a symbol that moved exactly with its benchmark).
fn mean_sd(xs: &[f64]) -> Option<(f64, f64)> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    let sd = (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt();
    (sd.is_finite() && sd > 1e-9 * mean.abs().max(1.0)).then_some((mean, sd))
}

/// Each bar's value normalised against the `window` bars that end on it:
/// `100 + (x - mean) / sd`. `None` until a full window of values exists.
fn normalised(xs: &[Option<f64>], window: usize) -> Vec<Option<f64>> {
    (0..xs.len())
        .map(|i| {
            if window == 0 || i + 1 < window {
                return None;
            }
            let w: Option<Vec<f64>> = xs[i + 1 - window..=i].iter().copied().collect();
            let (mean, sd) = mean_sd(&w?)?;
            Some(CENTRE + (xs[i]? - mean) / sd)
        })
        .collect()
}

/// The path of `stock` against `bench` (aligned, same dates, oldest first):
/// one point per bar from the first that has both axes, values to two
/// decimals, the quadrant read from those.
pub fn path(
    stock: &[(NaiveDate, f64)],
    bench: &[(NaiveDate, f64)],
    window: usize,
) -> Vec<RotationPoint> {
    debug_assert_eq!(stock.len(), bench.len());
    let rs: Vec<Option<f64>> = stock
        .iter()
        .zip(bench)
        .map(|(s, b)| (s.1 > 0.0 && b.1 > 0.0).then(|| 100.0 * s.1 / b.1))
        .collect();
    let ratio = normalised(&rs, window);
    let roc: Vec<Option<f64>> = (0..ratio.len())
        .map(|i| {
            let (prev, now) = (ratio[i.checked_sub(1)?]?, ratio[i]?);
            (prev != 0.0).then(|| 100.0 * (now / prev - 1.0))
        })
        .collect();
    let momentum = normalised(&roc, window);
    stock
        .iter()
        .zip(ratio.iter().zip(&momentum))
        .filter_map(|(&(date, _), (r, m))| {
            let (rs_ratio, rs_momentum) = (round2((*r)?), round2((*m)?));
            Some(RotationPoint {
                date,
                rs_ratio,
                rs_momentum,
                quadrant: Quadrant::of(rs_ratio, rs_momentum),
            })
        })
        .collect()
}

/// A symbol's reading: its last `tail` points and how many bars it shares
/// with the benchmark.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub tail: Vec<RotationPoint>,
    pub observations: usize,
}

/// Aligns `stock` and `bench` daily closes on the dates both have, resamples
/// them to weeks in weekly mode, and keeps the last `tail` points of the path.
pub fn analyse(
    stock: &[(NaiveDate, f64)],
    bench: &[(NaiveDate, f64)],
    interval: RotationInterval,
    window: usize,
    tail: usize,
) -> Reading {
    let (s, b) = common(stock, bench);
    let (s, b) = match interval {
        RotationInterval::Week => (weekly(&s), weekly(&b)),
        RotationInterval::Day => (s, b),
    };
    let points = path(&s, &b, window);
    let skip = points.len().saturating_sub(tail);
    Reading {
        tail: points[skip..].to_vec(),
        observations: s.len(),
    }
}

/// The row a reading becomes: the latest point, and its change over the tail.
pub fn row(symbol: Symbol, name: Option<String>, r: Reading) -> RotationRow {
    let last = r.tail.last();
    let change = |f: fn(&RotationPoint) -> f64| match (r.tail.first(), last) {
        (Some(a), Some(b)) if r.tail.len() > 1 => Some(round2(f(b) - f(a))),
        _ => None,
    };
    RotationRow {
        symbol,
        name,
        as_of: last.map(|p| p.date),
        rs_ratio: last.map(|p| p.rs_ratio),
        rs_momentum: last.map(|p| p.rs_momentum),
        quadrant: last.map(|p| p.quadrant),
        rs_ratio_change: change(|p| p.rs_ratio),
        rs_momentum_change: change(|p| p.rs_momentum),
        observations: r.observations,
        tail: r.tail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// Consecutive calendar days from `start`.
    fn days(start: NaiveDate, closes: &[f64]) -> Closes {
        closes
            .iter()
            .enumerate()
            .map(|(i, c)| (start + Duration::days(i as i64), *c))
            .collect()
    }

    #[test]
    fn weekly_keeps_each_weeks_last_bar_across_the_new_year() {
        let s = vec![
            (d(2025, 12, 22), 1.0), // Mon, ISO 2025-W52
            (d(2025, 12, 24), 2.0), // Wed
            (d(2025, 12, 29), 3.0), // Mon, ISO 2026-W01
            (d(2026, 1, 2), 4.0),   // Fri, same ISO week as Dec 29
            // no bar in 2026-W02 (a holiday week)
            (d(2026, 1, 12), 5.0), // Mon, W03, the current week so far
        ];
        let w = weekly(&s);
        assert_eq!(
            w,
            vec![
                (d(2025, 12, 24), 2.0),
                (d(2026, 1, 2), 4.0),
                (d(2026, 1, 12), 5.0)
            ]
        );
        assert!(weekly(&[]).is_empty());
    }

    #[test]
    fn the_first_point_needs_two_windows_of_bars() {
        // A stock that wobbles against a flat benchmark: every window spreads.
        let stock: Vec<f64> = (0..40)
            .map(|i| 100.0 + (i as f64 * 0.7).sin() * 5.0 + i as f64 * 0.1)
            .collect();
        let s = days(d(2026, 1, 1), &stock);
        let b = days(d(2026, 1, 1), &[1000.0; 40]);
        let p = path(&s, &b, 5);
        // RS-Ratio from bar 4, ROC from bar 5, RS-Momentum from bar 9.
        assert_eq!(p.len(), 40 - 9);
        assert_eq!(p[0].date, s[9].0);
        assert_eq!(bars_needed(5, 1), 10);
        assert_eq!(bars_needed(14, 10), 37);
        // The tail keeps the last points, in order, ending on the last bar.
        let r = analyse(&s, &b, RotationInterval::Day, 5, 10);
        assert_eq!(r.tail.len(), 10);
        assert_eq!(r.tail.last().unwrap().date, s[39].0);
        assert_eq!(r.observations, 40);
        assert_eq!(r.tail, p[p.len() - 10..]);
    }

    #[test]
    fn no_spread_no_point() {
        // A stock that moves exactly with its benchmark has a flat RS.
        let bench: Vec<f64> = (0..40)
            .map(|i| 1000.0 + (i as f64 * 0.9).cos() * 30.0)
            .collect();
        let stock: Vec<f64> = bench.iter().map(|x| x * 2.5).collect();
        let r = analyse(
            &days(d(2026, 1, 1), &stock),
            &days(d(2026, 1, 1), &bench),
            RotationInterval::Day,
            5,
            10,
        );
        assert!(r.tail.is_empty());
        assert_eq!(r.observations, 40);
        // A steady linear RS gives a constant RS-Ratio, so its momentum has no spread either.
        let linear: Vec<f64> = (0..40).map(|i| 100.0 + i as f64).collect();
        let r = analyse(
            &days(d(2026, 1, 1), &linear),
            &days(d(2026, 1, 1), &[100.0; 40]),
            RotationInterval::Day,
            5,
            10,
        );
        assert!(r.tail.is_empty());
        let row = row(Symbol::parse("BBCA").unwrap(), None, r);
        assert_eq!(
            (row.as_of, row.rs_ratio, row.quadrant, row.rs_ratio_change),
            (None, None, None, None)
        );
    }

    #[test]
    fn a_short_history_gives_a_short_tail_and_dates_must_be_shared() {
        let stock: Vec<f64> = (0..12).map(|i| 100.0 + (i as f64).sin() * 4.0).collect();
        let s = days(d(2026, 1, 1), &stock);
        let b = days(d(2026, 1, 1), &[500.0; 12]);
        // Window 5: points from bar 9, so 3 of the 10 asked for.
        assert_eq!(analyse(&s, &b, RotationInterval::Day, 5, 10).tail.len(), 3);
        // The benchmark starts two days late: two fewer shared bars, one point.
        let late = days(d(2026, 1, 3), &[500.0; 10]);
        let r = analyse(&s, &late, RotationInterval::Day, 5, 10);
        assert_eq!(r.observations, 10);
        assert_eq!(r.tail.len(), 1);
        let one = row(Symbol::parse("TLKM").unwrap(), None, r);
        assert!(one.rs_ratio.is_some());
        assert_eq!(one.rs_ratio_change, None, "one point has no change");
    }

    #[test]
    fn quadrants_turn_around_one_hundred() {
        assert_eq!(Quadrant::of(101.0, 101.0), Quadrant::Leading);
        assert_eq!(Quadrant::of(101.0, 99.0), Quadrant::Weakening);
        assert_eq!(Quadrant::of(99.0, 99.0), Quadrant::Lagging);
        assert_eq!(Quadrant::of(99.0, 101.0), Quadrant::Improving);
        assert_eq!(
            Quadrant::of(100.0, 100.0),
            Quadrant::Leading,
            "100 is the right and upper side"
        );
        assert_eq!(Quadrant::of(100.0, 99.99), Quadrant::Weakening);
    }

    #[test]
    fn one_point_by_hand() {
        // Window 3, flat benchmark at 100, so RS is the close.
        // RS:        10, 12, 11, 14, 13, 17
        // RS-Ratio:   -,  -, r2, r3, r4, r5   (100 + z of the last of each 3)
        let s = days(d(2026, 3, 2), &[10.0, 12.0, 11.0, 14.0, 13.0, 17.0]);
        let b = days(d(2026, 3, 2), &[100.0; 6]);
        let z = |w: [f64; 3]| {
            let m = (w[0] + w[1] + w[2]) / 3.0;
            let sd = (w.iter().map(|x| (x - m).powi(2)).sum::<f64>() / 2.0).sqrt();
            100.0 + (w[2] - m) / sd
        };
        let (r2, r3, r4, r5) = (
            z([10.0, 12.0, 11.0]),
            z([12.0, 11.0, 14.0]),
            z([11.0, 14.0, 13.0]),
            z([14.0, 13.0, 17.0]),
        );
        let roc = |a: f64, b: f64| 100.0 * (b / a - 1.0);
        let m5 = z([roc(r2, r3), roc(r3, r4), roc(r4, r5)]);
        let p = path(&s, &b, 3);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].date, d(2026, 3, 7));
        assert_eq!(p[0].rs_ratio, round2(r5));
        assert_eq!(p[0].rs_momentum, round2(m5));
        assert_eq!(p[0].quadrant, Quadrant::of(round2(r5), round2(m5)));
    }

    /// On the recorded one-year charts, weekly, window 14, tail 10: one
    /// stock in each quadrant as of 2026-11-04 (^JKSE's last bar).
    #[test]
    fn recorded_charts_land_in_all_four_quadrants() {
        use crate::analysis::returns::series;
        use crate::model::{Interval, Period};
        use crate::providers::yahoo;
        let chart = |name: &str| {
            let body = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/yahoo")
                    .join(name),
            )
            .unwrap();
            series(
                &yahoo::parse_chart(&body, Period::Y1, Interval::Day1)
                    .unwrap()
                    .candles,
            )
        };
        let jkse = chart("chart_jkse_1y_1d.json");
        let read = |s: &str| {
            let r = analyse(
                &chart(&format!("chart_{s}_1y_1d.json")),
                &jkse,
                RotationInterval::Week,
                DEFAULT_WINDOW,
                DEFAULT_TAIL,
            );
            let last = r.tail.last().unwrap().clone();
            (
                r.tail.len(),
                last.date.to_string(),
                last.rs_ratio,
                last.rs_momentum,
                last.quadrant,
            )
        };
        let got = [read("bbca"), read("tlkm"), read("giaa"), read("itmg")];
        assert_eq!(
            got[0],
            (10, "2026-11-04".into(), 100.63, 100.63, Quadrant::Leading)
        );
        assert_eq!(
            got[1],
            (10, "2026-11-04".into(), 99.33, 99.5, Quadrant::Lagging)
        );
        assert_eq!(
            got[2],
            (10, "2026-11-04".into(), 98.86, 100.06, Quadrant::Improving)
        );
        assert_eq!(
            got[3],
            (10, "2026-11-04".into(), 101.76, 98.66, Quadrant::Weakening)
        );
    }
}
