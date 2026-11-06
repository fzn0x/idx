//! The astro backtest: IHSG's close-to-close daily returns grouped by lunar
//! phase, Mercury retrograde or direct, pasaran and Sun sign, each group
//! tested against all other sessions (Welch's t, Bonferroni over all groups).
//! Descriptive statistics of the past, not a signal.

use chrono::{DateTime, NaiveDate, Timelike, Utc};

use crate::astro::calendar::{pasaran_index, PASARAN};
use crate::astro::ephem::{norm360, sun_apparent};
use crate::astro::events::mercury_retrograde_intervals;
use crate::astro::moon;
use crate::astro::time::{close_ms, jde_from_ms, round_to, DAY_MS};
use crate::error::{IdxError, Result};
use crate::model::{
    wib, AstroBacktest, AstroPeriod, BacktestFactor, BacktestGroup, History, Interval, MoonPhase,
    ZodiacSign,
};

use super::returns::series;
use super::stats;

/// Fewest returns the backtest runs on.
pub const MIN_RETURNS: usize = 30;
pub const ALPHA: f64 = 0.05;

pub const METHOD: &str = "Close-to-close daily returns of the index, each session assigned the \
factor values at 16:00 WIB of its date (the Moon's phase sector, Mercury retrograde or direct, \
the pasaran, the Sun's sign). Each group is compared with all other sessions by Welch's t \
(two-sided p from Student's t, Welch-Satterthwaite degrees of freedom), Bonferroni over all \
groups. Daily returns are fat-tailed and their volatility clusters, so p-values are approximate; \
Sun signs track calendar months and carry month-of-year effects. Descriptive statistics, not a \
trading signal.";

/// One session's factor values: indexes into the group lists below.
struct Features {
    phase: usize,
    retrograde: bool,
    pasaran: usize,
    sign: usize,
}

fn features(dates: &[NaiveDate]) -> Vec<Features> {
    let (Some(first), Some(last)) = (dates.first(), dates.last()) else {
        return vec![];
    };
    let iv = mercury_retrograde_intervals(
        close_ms(*first) - 40.0 * DAY_MS,
        close_ms(*last) + 40.0 * DAY_MS,
    );
    let mut k = 0;
    dates
        .iter()
        .map(|&d| {
            let ms = close_ms(d);
            let jde = jde_from_ms(ms);
            let sun = sun_apparent(jde).lon;
            let el = norm360(moon::longitude(jde) - sun);
            while k < iv.len() && iv[k].1 <= ms {
                k += 1;
            }
            let phase = MoonPhase::of(el);
            Features {
                phase: MoonPhase::ALL.iter().position(|p| *p == phase).unwrap_or(0),
                retrograde: k < iv.len() && iv[k].0 <= ms,
                pasaran: pasaran_index(d),
                sign: ZodiacSign::ALL
                    .iter()
                    .position(|s| *s == ZodiacSign::of(sun))
                    .unwrap_or(0),
            }
        })
        .collect()
}

fn group(name: &str, members: &[f64], rest: &[f64]) -> BacktestGroup {
    let n = members.len();
    let m = (n > 0).then(|| stats::mean(members));
    let w = stats::welch(members, rest);
    BacktestGroup {
        group: name.to_string(),
        n,
        mean_pct: m.map(|v| round_to(v, 4)),
        median_pct: (n > 0).then(|| round_to(stats::median(members), 4)),
        stdev_pct: m
            .filter(|_| n > 1)
            .map(|m| round_to(stats::variance(members, m).sqrt(), 4)),
        win_rate_pct: (n > 0).then(|| {
            round_to(
                members.iter().filter(|x| **x > 0.0).count() as f64 / n as f64 * 100.0,
                2,
            )
        }),
        t_stat: w.map(|(t, _, _)| round_to(t, 3)),
        p_value: w.map(|(_, _, p)| p),
        significant: false,
    }
}

/// Removes the last candle when the history was fetched on that session's
/// date (Jakarta) before 17:00 WIB: until the close and the post-trading
/// session are over, its price is not a close. `fetched_at` is when the data
/// was fetched, not the clock now, so a cached mid-session price read in the
/// evening (`--offline`, a long TTL, a long-running MCP server) is still left
/// out. Returns the date left out.
pub fn drop_unfinished(h: &mut History, fetched_at: DateTime<Utc>) -> Option<NaiveDate> {
    let at = fetched_at.with_timezone(&wib());
    let last = h.candles.last()?.date();
    if last == at.date_naive() && at.hour() < 17 {
        h.candles.pop();
        return Some(last);
    }
    None
}

/// The backtest over a daily history (oldest first, one close per WIB date).
pub fn backtest(h: &History, period: AstroPeriod) -> Result<AstroBacktest> {
    let closes = series(&h.candles);
    if closes.len() > 2 {
        let mut gaps: Vec<i64> = closes
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).num_days())
            .collect();
        gaps.sort_unstable();
        if h.interval != Interval::Day1 || gaps[gaps.len() / 2] > 4 {
            return Err(IdxError::provider(
                "backtest",
                format!(
                    "history for {} came back coarser than daily ({} bars)",
                    h.symbol, h.interval
                ),
            ));
        }
    }
    if closes.len() < MIN_RETURNS + 1 {
        return Err(IdxError::NotFound(format!(
            "{} has {} daily closes; the backtest needs at least {}",
            h.symbol,
            closes.len(),
            MIN_RETURNS + 1
        )));
    }
    let dates: Vec<NaiveDate> = closes.iter().map(|c| c.0).collect();
    let feats = features(&dates);
    // Return i is the move into session i + 1, so it takes that session's factors.
    let rets: Vec<(f64, &Features)> = closes
        .windows(2)
        .zip(&feats[1..])
        .map(|(w, f)| ((w[1].1 / w[0].1 - 1.0) * 100.0, f))
        .collect();
    let all: Vec<f64> = rets.iter().map(|r| r.0).collect();

    let split = |pred: &dyn Fn(&Features) -> bool| -> (Vec<f64>, Vec<f64>) {
        let mut inside = Vec::new();
        let mut rest = Vec::new();
        for (r, f) in &rets {
            if pred(f) {
                inside.push(*r);
            } else {
                rest.push(*r);
            }
        }
        (inside, rest)
    };
    let mut factors = vec![
        BacktestFactor {
            factor: "moon_phase".into(),
            groups: MoonPhase::ALL
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let (g, r) = split(&|f| f.phase == i);
                    group(p.as_str(), &g, &r)
                })
                .collect(),
        },
        BacktestFactor {
            factor: "mercury".into(),
            groups: [("retrograde", true), ("direct", false)]
                .iter()
                .map(|(name, retro)| {
                    let (g, r) = split(&|f| f.retrograde == *retro);
                    group(name, &g, &r)
                })
                .collect(),
        },
        BacktestFactor {
            factor: "pasaran".into(),
            groups: PASARAN
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let (g, r) = split(&|f| f.pasaran == i);
                    group(&p.to_lowercase(), &g, &r)
                })
                .collect(),
        },
        BacktestFactor {
            factor: "sun_sign".into(),
            groups: ZodiacSign::ALL
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let (g, r) = split(&|f| f.sign == i);
                    group(s.as_str(), &g, &r)
                })
                .collect(),
        },
    ];
    let tests = factors
        .iter()
        .flat_map(|f| &f.groups)
        .filter(|g| g.p_value.is_some())
        .count();
    let threshold = ALPHA / tests.max(1) as f64;
    let mut significant = 0;
    for g in factors.iter_mut().flat_map(|f| f.groups.iter_mut()) {
        g.significant = g.p_value.is_some_and(|p| p < threshold);
        significant += g.significant as usize;
        g.p_value = g.p_value.map(|p| round_to(p, 6));
    }
    Ok(AstroBacktest {
        symbol: h.symbol.clone(),
        period,
        from: dates[1],
        to: *dates.last().expect("checked above"),
        returns: all.len(),
        mean_pct: round_to(stats::mean(&all), 4),
        median_pct: round_to(stats::median(&all), 4),
        win_rate_pct: round_to(
            all.iter().filter(|x| **x > 0.0).count() as f64 / all.len() as f64 * 100.0,
            2,
        ),
        tests,
        alpha: ALPHA,
        bonferroni_alpha: round_to(threshold, 6),
        significant,
        factors,
        method: METHOD.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Period;
    use crate::providers::yahoo::parse_chart;

    const JKSE_2Y: &str = include_str!("../../tests/fixtures/yahoo/chart_jkse_2y_1d.json");

    fn g<'a>(b: &'a AstroBacktest, factor: &str, name: &str) -> &'a BacktestGroup {
        b.factors
            .iter()
            .find(|f| f.factor == factor)
            .unwrap()
            .groups
            .iter()
            .find(|x| x.group == name)
            .unwrap()
    }

    #[test]
    fn two_years_of_jkse() {
        let h = parse_chart(JKSE_2Y, Period::Y2, Interval::Day1).unwrap();
        let b = backtest(&h, AstroPeriod::Y2).unwrap();
        assert_eq!(b.symbol.as_str(), "^JKSE");
        assert_eq!(
            (b.from.to_string(), b.to.to_string()),
            ("2024-11-06".into(), "2026-11-04".into())
        );
        assert_eq!(b.returns, 473);
        assert_eq!(
            (b.mean_pct, b.median_pct, b.win_rate_pct),
            (-0.0763, -0.0187, 49.89)
        );
        assert_eq!(
            (b.tests, b.bonferroni_alpha, b.significant),
            (27, 0.001852, 0)
        );
        let m = g(&b, "mercury", "retrograde");
        assert_eq!(
            (m.n, m.mean_pct, m.p_value),
            (93, Some(-0.1281), Some(0.709909))
        );
        let m = g(&b, "mercury", "direct");
        assert_eq!(
            (m.n, m.mean_pct, m.p_value),
            (380, Some(-0.0636), Some(0.709909))
        );
        let ns: Vec<usize> = ["legi", "pahing", "pon", "wage", "kliwon"]
            .iter()
            .map(|p| g(&b, "pasaran", p).n)
            .collect();
        assert_eq!(ns, [95, 95, 98, 92, 93]);
        let k = g(&b, "pasaran", "kliwon");
        assert_eq!((k.mean_pct, k.p_value), (Some(-0.3225), Some(0.148345)));
        let nm = g(&b, "moon_phase", "new_moon");
        assert_eq!(
            (nm.n, nm.mean_pct, nm.p_value),
            (50, Some(0.1162), Some(0.291622))
        );
        let fm = g(&b, "moon_phase", "full_moon");
        assert_eq!(
            (fm.n, fm.mean_pct, fm.p_value),
            (59, Some(-0.276), Some(0.296163))
        );
        let c = g(&b, "sun_sign", "capricorn");
        assert_eq!(
            (c.n, c.mean_pct, c.p_value),
            (32, Some(0.2634), Some(0.023102))
        );
        let p = g(&b, "sun_sign", "pisces");
        assert_eq!(
            (p.n, p.mean_pct, p.p_value),
            (40, Some(-0.5863), Some(0.066919))
        );
        let total: usize = b.factors[0].groups.iter().map(|x| x.n).sum();
        assert_eq!(total, 473);
        crate::astro::tests::golden(
            "golden_backtest_jkse_2y.json",
            &serde_json::to_value(&b).unwrap(),
        );
    }

    #[test]
    fn all_of_jkse_since_1990() {
        let max = include_str!("../../tests/fixtures/yahoo/chart_jkse_period1_max_1d.json");
        let h = parse_chart(max, Period::Max, Interval::Day1).unwrap();
        let b = backtest(&h, AstroPeriod::Max).unwrap();
        assert_eq!(
            (b.from.to_string(), b.to.to_string()),
            ("1990-04-09".into(), "2026-11-04".into())
        );
        assert_eq!(b.returns, 8904);
        assert_eq!(
            (b.mean_pct, b.median_pct, b.win_rate_pct),
            (0.0324, 0.0554, 52.72)
        );
        assert_eq!((b.tests, b.significant), (27, 0));
        let fm = g(&b, "moon_phase", "full_moon");
        assert!(fm.mean_pct.unwrap() < 0.0 && fm.p_value.unwrap() < 0.05);
        assert!(g(&b, "mercury", "retrograde").p_value.unwrap() > 0.9);
    }

    #[test]
    fn a_session_still_trading_is_left_out() {
        let mut h = parse_chart(JKSE_2Y, Period::Y2, Interval::Day1).unwrap();
        let n = h.candles.len();
        // The last priced bar is 2026-11-04.
        let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        assert_eq!(drop_unfinished(&mut h, at("2026-11-04T10:00:00Z")), None);
        assert_eq!(drop_unfinished(&mut h, at("2026-11-05T02:00:00Z")), None);
        assert_eq!(
            drop_unfinished(&mut h, at("2026-11-04T02:30:00Z")).map(|d| d.to_string()),
            Some("2026-11-04".to_string())
        );
        assert_eq!(h.candles.len(), n - 1);
    }

    #[test]
    fn monthly_or_short_history_is_refused() {
        let monthly = include_str!("../../tests/fixtures/yahoo/chart_jkse_range_max_1d.json");
        let h = parse_chart(monthly, Period::Max, Interval::Day1).unwrap();
        assert_eq!(backtest(&h, AstroPeriod::Max).unwrap_err().exit_code(), 3);
        let short = include_str!("../../tests/fixtures/yahoo/chart_bbca_1mo_1d.json");
        let h = parse_chart(short, Period::M1, Interval::Day1).unwrap();
        assert_eq!(backtest(&h, AstroPeriod::Y1).unwrap_err().exit_code(), 4);
    }
}
