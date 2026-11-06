//! Performance statistics over daily candles. Pure functions, unit tested
//! with synthetic series.

use chrono::{Datelike, Duration, Months, NaiveDate};

use crate::model::{Candle, DayMove, Drawdown, History, Performance, ReturnWindows};

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Closes with their dates, oldest first, one per calendar day.
pub(crate) fn series(candles: &[Candle]) -> Vec<(NaiveDate, f64)> {
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
