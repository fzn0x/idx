//! Performance statistics over daily candles. Pure functions, unit tested
//! with synthetic series.

use chrono::{Datelike, Duration, Months, NaiveDate};

use crate::model::{Candle, DayMove, Drawdown, History, Performance, ReturnWindows};

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Closes with their dates, oldest first, one per calendar day.
pub fn series(candles: &[Candle]) -> Vec<(NaiveDate, f64)> {
    let mut out: Vec<(NaiveDate, f64)> = Vec::with_capacity(candles.len());
    for c in candles {
        let d = c.date();
        match out.last_mut() {
            Some(last) if last.0 == d => last.1 = c.close,
            _ => out.push((d, c.close)),
        }
    }
    out
}

/// Last close on or before `target`. When `target` is earlier than the first
/// candle, the first candle is used if it is within `tolerance` days, so a
/// 5y window still resolves on a range that starts a few days late.
fn close_on_or_before(s: &[(NaiveDate, f64)], target: NaiveDate, tolerance: i64) -> Option<f64> {
    let idx = s.partition_point(|(d, _)| *d <= target);
    if idx == 0 {
        let (first, close) = s.first()?;
        return ((*first - target).num_days() <= tolerance).then_some(*close);
    }
    Some(s[idx - 1].1)
}

fn pct_from(base: Option<f64>, price: f64) -> Option<f64> {
    base.filter(|b| *b > 0.0)
        .map(|b| round2((price / b - 1.0) * 100.0))
}

/// Total returns over the standard windows ending at the last candle.
pub fn windows(s: &[(NaiveDate, f64)]) -> ReturnWindows {
    let Some(&(as_of, price)) = s.last() else {
        return ReturnWindows::default();
    };
    let at = |d: Option<NaiveDate>, tol: i64| d.and_then(|d| close_on_or_before(s, d, tol));
    let months = |n: u32| as_of.checked_sub_months(Months::new(n));
    let prev_year_end = NaiveDate::from_ymd_opt(as_of.year() - 1, 12, 31);
    ReturnWindows {
        w1: pct_from(at(Some(as_of - Duration::days(7)), 0), price),
        m1: pct_from(at(months(1), 0), price),
        m3: pct_from(at(months(3), 0), price),
        m6: pct_from(at(months(6), 0), price),
        ytd: pct_from(at(prev_year_end, 0), price),
        y1: pct_from(at(months(12), 7), price),
        y3: pct_from(at(months(36), 10), price),
        y5: pct_from(at(months(60), 10), price),
    }
}

fn cagr(s: &[(NaiveDate, f64)], years: u32) -> Option<f64> {
    let &(as_of, price) = s.last()?;
    let target = as_of.checked_sub_months(Months::new(years * 12))?;
    let idx = s.partition_point(|(d, _)| *d <= target);
    let (start_date, start) = if idx == 0 {
        let f = s.first()?;
        if (f.0 - target).num_days() > 10 {
            return None;
        }
        *f
    } else {
        s[idx - 1]
    };
    let elapsed = (as_of - start_date).num_days() as f64 / 365.25;
    if start <= 0.0 || elapsed < 0.5 {
        return None;
    }
    Some(round2(((price / start).powf(1.0 / elapsed) - 1.0) * 100.0))
}

/// Largest peak-to-trough fall over `s`.
pub fn max_drawdown(s: &[(NaiveDate, f64)]) -> Option<Drawdown> {
    let mut peak = *s.first()?;
    let mut best: Option<(f64, NaiveDate, NaiveDate)> = None;
    for &(d, c) in s {
        if c > peak.1 {
            peak = (d, c);
        }
        let dd = (c / peak.1 - 1.0) * 100.0;
        if best.is_none_or(|(b, _, _)| dd < b) {
            best = Some((dd, peak.0, d));
        }
    }
    let (pct, peak_date, trough_date) = best?;
    if pct >= 0.0 {
        return None;
    }
    let peak_close = s.iter().find(|(d, _)| *d == peak_date)?.1;
    let recovered = s.iter().any(|(d, c)| *d > trough_date && *c >= peak_close);
    Some(Drawdown {
        pct: round2(pct),
        peak_date,
        trough_date,
        recovered,
    })
}

/// Annualised standard deviation of daily log returns, percent.
pub fn volatility(s: &[(NaiveDate, f64)]) -> Option<f64> {
    let r: Vec<f64> = s
        .windows(2)
        .filter(|w| w[0].1 > 0.0 && w[1].1 > 0.0)
        .map(|w| (w[1].1 / w[0].1).ln())
        .collect();
    if r.len() < 20 {
        return None;
    }
    let mean = r.iter().sum::<f64>() / r.len() as f64;
    let var = r.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (r.len() - 1) as f64;
    Some(round2(var.sqrt() * (252f64).sqrt() * 100.0))
}

/// Fewest matched daily returns behind a beta: about six months of sessions.
pub const MIN_BETA_OBSERVATIONS: usize = 120;

/// Daily closes with their dates, oldest first (see [`series`]).
pub type Closes = Vec<(NaiveDate, f64)>;

/// The dates both series have, oldest first, as two aligned series. Index
/// and stock bars can lag each other by a session (Yahoo's `^JKSE` daily bar
/// often arrives a day after the shares'), so statistics that compare the
/// two run on these.
pub fn common(a: &[(NaiveDate, f64)], b: &[(NaiveDate, f64)]) -> (Closes, Closes) {
    let (mut i, mut j) = (0, 0);
    let (mut out_a, mut out_b) = (Vec::new(), Vec::new());
    while i < a.len() && j < b.len() {
        match a[i].0.cmp(&b[j].0) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out_a.push(a[i]);
                out_b.push(b[j]);
                i += 1;
                j += 1;
            }
        }
    }
    (out_a, out_b)
}

/// Beta and correlation of `stock` against `bench`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BetaStats {
    /// Sample covariance over the benchmark's sample variance, 3 dp; `None`
    /// under [`MIN_BETA_OBSERVATIONS`] or when the benchmark never moved.
    pub beta: Option<f64>,
    /// Pearson correlation, 3 dp; `None` when beta is, or the stock never moved.
    pub correlation: Option<f64>,
    /// Matched daily returns used.
    pub observations: usize,
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Beta from simple daily returns between consecutive dates of two aligned
/// series (see [`common`]); a pair is skipped when either base close is not
/// above zero.
pub fn beta(stock: &[(NaiveDate, f64)], bench: &[(NaiveDate, f64)]) -> BetaStats {
    let pairs: Vec<(f64, f64)> = stock
        .windows(2)
        .zip(bench.windows(2))
        .filter(|(s, b)| s[0].1 > 0.0 && b[0].1 > 0.0)
        .map(|(s, b)| (s[1].1 / s[0].1 - 1.0, b[1].1 / b[0].1 - 1.0))
        .collect();
    let n = pairs.len();
    let none = BetaStats {
        beta: None,
        correlation: None,
        observations: n,
    };
    if n < MIN_BETA_OBSERVATIONS {
        return none;
    }
    let ms = pairs.iter().map(|p| p.0).sum::<f64>() / n as f64;
    let mb = pairs.iter().map(|p| p.1).sum::<f64>() / n as f64;
    let dof = (n - 1) as f64;
    let cov = pairs.iter().map(|p| (p.0 - ms) * (p.1 - mb)).sum::<f64>() / dof;
    let var_b = pairs.iter().map(|p| (p.1 - mb).powi(2)).sum::<f64>() / dof;
    let var_s = pairs.iter().map(|p| (p.0 - ms).powi(2)).sum::<f64>() / dof;
    if var_b <= 0.0 {
        return none;
    }
    BetaStats {
        beta: Some(round3(cov / var_b)),
        correlation: (var_s > 0.0).then(|| round3(cov / (var_b * var_s).sqrt())),
        observations: n,
    }
}

fn extremes(s: &[(NaiveDate, f64)]) -> (Option<DayMove>, Option<DayMove>) {
    let mut best: Option<DayMove> = None;
    let mut worst: Option<DayMove> = None;
    for w in s.windows(2) {
        if w[0].1 <= 0.0 {
            continue;
        }
        let pct = (w[1].1 / w[0].1 - 1.0) * 100.0;
        if best.as_ref().is_none_or(|b| pct > b.pct) {
            best = Some(DayMove {
                date: w[1].0,
                pct: round2(pct),
            });
        }
        if worst.as_ref().is_none_or(|b| pct < b.pct) {
            worst = Some(DayMove {
                date: w[1].0,
                pct: round2(pct),
            });
        }
    }
    (best, worst)
}

/// Builds the report from daily candles (ideally a 5y window). `None` when
/// there are no candles.
pub fn analyse(h: &History) -> Option<Performance> {
    let s = series(&h.candles);
    let &(as_of, price) = s.last()?;
    let year_ago = as_of - Duration::days(365);
    let last_year: Vec<(NaiveDate, f64)> =
        s.iter().copied().filter(|(d, _)| *d > year_ago).collect();
    let (best_day_1y, worst_day_1y) = extremes(&last_year);
    let high_52w = last_year
        .iter()
        .map(|x| x.1)
        .fold(None, |m: Option<f64>, c| Some(m.map_or(c, |m| m.max(c))));
    let low_52w = last_year
        .iter()
        .map(|x| x.1)
        .fold(None, |m: Option<f64>, c| Some(m.map_or(c, |m| m.min(c))));
    let high_date = high_52w.and_then(|hi| last_year.iter().rev().find(|x| x.1 == hi).map(|x| x.0));
    Some(Performance {
        symbol: h.symbol.clone(),
        name: h.name.clone(),
        currency: h.currency.clone(),
        as_of,
        price,
        start: s.first()?.0,
        candles_used: s.len(),
        returns: windows(&s),
        cagr_3y_pct: cagr(&s, 3),
        cagr_5y_pct: cagr(&s, 5),
        volatility_1y_pct: volatility(&last_year),
        max_drawdown_1y: max_drawdown(&last_year),
        max_drawdown_all: max_drawdown(&s),
        best_day_1y,
        worst_day_1y,
        high_52w,
        low_52w,
        pct_from_52w_high: high_52w.map(|hi| round2((price / hi - 1.0) * 100.0)),
        pct_from_52w_low: low_52w
            .filter(|l| *l > 0.0)
            .map(|lo| round2((price / lo - 1.0) * 100.0)),
        days_since_52w_high: high_date.map(|d| (as_of - d).num_days()),
        benchmark: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{wib, Interval, Period, Symbol};
    use chrono::TimeZone;

    fn hist(closes: &[(i32, u32, u32, f64)]) -> History {
        let candles = closes
            .iter()
            .map(|&(y, m, d, c)| Candle {
                time: wib().with_ymd_and_hms(y, m, d, 9, 0, 0).unwrap(),
                open: c,
                high: c,
                low: c,
                close: c,
                adj_close: None,
                volume: 1,
            })
            .collect::<Vec<_>>();
        History {
            symbol: Symbol::parse("T").unwrap(),
            name: None,
            currency: None,
            period: Period::Y5,
            interval: Interval::Day1,
            summary: History::summarise(&candles),
            candles,
        }
    }

    #[test]
    fn windows_resolve_on_calendar_dates() {
        let h = hist(&[
            (2021, 10, 18, 50.0),  // ~5y back, within tolerance of 2021-10-17
            (2023, 10, 16, 80.0),  // 3y
            (2025, 10, 15, 100.0), // 1y (tolerance 7d)
            (2025, 12, 31, 120.0), // ytd base
            (2026, 4, 17, 110.0),  // 6m
            (2026, 7, 17, 125.0),  // 3m
            (2026, 9, 17, 130.0),  // 1m
            (2026, 10, 10, 128.0), // 1w
            (2026, 10, 17, 140.0),
        ]);
        let p = analyse(&h).unwrap();
        let r = &p.returns;
        assert_eq!(r.w1, Some(9.38));
        assert_eq!(r.m1, Some(7.69));
        assert_eq!(r.m3, Some(12.0));
        assert_eq!(r.m6, Some(27.27));
        assert_eq!(r.ytd, Some(16.67));
        assert_eq!(r.y1, Some(40.0));
        assert_eq!(r.y3, Some(75.0));
        assert_eq!(r.y5, Some(180.0));
        assert_eq!(p.cagr_3y_pct, Some(20.48)); // 3.003 years elapsed
        assert!(p.cagr_5y_pct.unwrap() > 22.0 && p.cagr_5y_pct.unwrap() < 23.0);
        assert_eq!(p.high_52w, Some(140.0));
        assert_eq!(p.pct_from_52w_high, Some(0.0));
        assert_eq!(p.days_since_52w_high, Some(0));
        // Benchmark arithmetic.
        let bench = ReturnWindows {
            y1: Some(10.0),
            ..Default::default()
        };
        assert_eq!(r.minus(&bench).y1, Some(30.0));
        assert_eq!(r.minus(&bench).m1, None);
    }

    fn days(start: (i32, u32, u32), closes: &[f64]) -> Vec<(NaiveDate, f64)> {
        let d0 = NaiveDate::from_ymd_opt(start.0, start.1, start.2).unwrap();
        closes
            .iter()
            .enumerate()
            .map(|(i, c)| (d0 + Duration::days(i as i64), *c))
            .collect()
    }

    /// An index path with some variety, starting at 1000.
    fn index_path(n: usize) -> Vec<f64> {
        let mut out = vec![1000.0];
        for i in 1..n {
            let r = [0.01, -0.005, 0.002, -0.012, 0.007][i % 5];
            out.push(out[i - 1] * (1.0 + r));
        }
        out
    }

    #[test]
    fn beta_of_a_doubled_index_is_two() {
        let idx = index_path(200);
        let stock: Vec<f64> = {
            let mut out = vec![500.0];
            for i in 1..idx.len() {
                let r = idx[i] / idx[i - 1] - 1.0;
                out.push(out[i - 1] * (1.0 + 2.0 * r));
            }
            out
        };
        let b = beta(&days((2026, 1, 1), &stock), &days((2026, 1, 1), &idx));
        assert_eq!(b.observations, 199);
        assert_eq!(b.beta, Some(2.0));
        assert_eq!(b.correlation, Some(1.0));
        // The same series against itself.
        let s = days((2026, 1, 1), &idx);
        assert_eq!(beta(&s, &s).beta, Some(1.0));
    }

    #[test]
    fn beta_needs_enough_returns_and_a_moving_index() {
        let idx = index_path(120); // 119 returns
        let s = days((2026, 1, 1), &idx);
        let b = beta(&s, &s);
        assert_eq!((b.beta, b.correlation, b.observations), (None, None, 119));
        let flat = days((2026, 1, 1), &[100.0; 200]);
        let moving = days((2026, 1, 1), &index_path(200));
        assert_eq!(beta(&moving, &flat).beta, None, "index never moved");
        let b = beta(&flat, &moving);
        assert_eq!(b.beta, Some(0.0), "a flat stock has beta 0");
        assert_eq!(b.correlation, None, "and no correlation");
    }

    #[test]
    fn common_keeps_only_shared_dates() {
        let a = days((2026, 1, 1), &[1.0, 2.0, 3.0, 4.0]); // Jan 1..4
        let b = days((2026, 1, 2), &[20.0, 30.0, 40.0, 50.0]); // Jan 2..5
        let (ca, cb) = common(&a, &b);
        let dates: Vec<String> = ca.iter().map(|x| x.0.to_string()).collect();
        assert_eq!(dates, ["2026-01-02", "2026-01-03", "2026-01-04"]);
        assert_eq!(ca.iter().map(|x| x.1).collect::<Vec<_>>(), [2.0, 3.0, 4.0]);
        assert_eq!(
            cb.iter().map(|x| x.1).collect::<Vec<_>>(),
            [20.0, 30.0, 40.0]
        );
        // A gap in one series drops that date from both.
        let gappy: Vec<_> = a.iter().copied().filter(|x| x.1 != 3.0).collect();
        let (ca, cb) = common(&gappy, &b);
        assert_eq!(ca.len(), 2);
        assert_eq!(cb.iter().map(|x| x.1).collect::<Vec<_>>(), [20.0, 40.0]);
        assert_eq!(common(&a, &[]).0.len(), 0);
    }

    #[test]
    fn drawdown_and_extremes() {
        let h = hist(&[
            (2026, 1, 5, 100.0),
            (2026, 1, 6, 120.0),
            (2026, 1, 7, 90.0),
            (2026, 1, 8, 60.0),
            (2026, 1, 9, 121.0),
            (2026, 1, 12, 110.0),
        ]);
        let p = analyse(&h).unwrap();
        let dd = p.max_drawdown_all.unwrap();
        assert_eq!(dd.pct, -50.0);
        assert_eq!(dd.peak_date.to_string(), "2026-01-06");
        assert_eq!(dd.trough_date.to_string(), "2026-01-08");
        assert!(dd.recovered);
        assert_eq!(p.best_day_1y.unwrap().pct, 101.67);
        assert_eq!(p.worst_day_1y.unwrap().pct, -33.33);
        assert_eq!(p.volatility_1y_pct, None, "too few days");
        assert!(p.returns.y1.is_none(), "series too short for a 1y window");
        let flat = hist(&[(2026, 1, 5, 100.0), (2026, 1, 6, 100.0)]);
        assert!(analyse(&flat).unwrap().max_drawdown_all.is_none());
    }
}
