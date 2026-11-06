//! Session figures over intraday bars: the bars of the latest date and their
//! summary (open, high, low, close, volume and a volume-weighted average
//! price). Pure, no network.

use crate::model::{Candle, IntradaySummary};

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The bars of the last WIB date in `candles` (oldest first), so a source that
/// returns more than one session still yields one.
pub fn session_bars(candles: &[Candle]) -> &[Candle] {
    let Some(last) = candles.last() else {
        return candles;
    };
    let day = last.date();
    let start = candles
        .iter()
        .rposition(|c| c.date() != day)
        .map_or(0, |i| i + 1);
    &candles[start..]
}

/// Summary of one session's bars; `None` without bars. VWAP weights each
/// bar's typical price, (high + low + close) / 3, by its volume, so it
/// approximates the exchange's average price (value / volume), which needs
/// trade-level data; it is `None` when no bar carries volume.
pub fn summary(bars: &[Candle]) -> Option<IntradaySummary> {
    let first = bars.first()?;
    let last = bars.last()?;
    let high = bars.iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let low = bars.iter().map(|c| c.low).fold(f64::MAX, f64::min);
    let volume: u64 = bars.iter().map(|c| c.volume).sum();
    let weighted: f64 = bars
        .iter()
        .map(|c| (c.high + c.low + c.close) / 3.0 * c.volume as f64)
        .sum();
    let vwap = (volume > 0).then(|| round2(weighted / volume as f64));
    Some(IntradaySummary {
        first_bar: first.time,
        last_bar: last.time,
        bars: bars.len(),
        open: first.open,
        high,
        low,
        close: last.close,
        bars_volume: volume,
        vwap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{wib_from_unix, Interval, Period};
    use crate::providers::yahoo::parse_chart;

    const BBCA: &str = include_str!("../../tests/fixtures/yahoo/chart_bbca_1d_1m.json");
    const JKSE: &str = include_str!("../../tests/fixtures/yahoo/chart_jkse_1d_1m.json");
    const BBCA_2M: &str = include_str!("../../tests/fixtures/yahoo/chart_bbca_1d_2m.json");

    #[test]
    fn stock_session_by_the_minute() {
        let h = parse_chart(BBCA, Period::D1, Interval::Min1).unwrap();
        let bars = session_bars(&h.candles);
        assert_eq!(bars.len(), 333);
        let s = summary(bars).unwrap();
        assert_eq!(s.bars, 333);
        assert_eq!(s.first_bar.to_rfc3339(), "2026-11-05T09:00:00+07:00");
        assert_eq!(s.last_bar.to_rfc3339(), "2026-11-05T16:14:00+07:00");
        assert_eq!(
            (s.open, s.high, s.low, s.close),
            (5025.0, 5125.0, 5025.0, 5125.0)
        );
        // The day's volume is 166,336,900: the opening auction is in no bar.
        assert_eq!(s.bars_volume, 155_142_100);
        // idx.co.id's value / volume for that session is 5,088.5.
        assert_eq!(s.vwap, Some(5089.98));
    }

    #[test]
    fn during_the_session() {
        // 09:12:16 WIB on 11-06: 09:00 (volume 0), 09:01, a null 09:02 row and
        // the 09:02:16 last-trade snapshot, which is not a bar.
        let h = parse_chart(
            include_str!("../../tests/fixtures/yahoo/chart_bbca_1d_1m_session.json"),
            Period::D1,
            Interval::Min1,
        )
        .unwrap();
        let s = summary(session_bars(&h.candles)).unwrap();
        assert_eq!(s.bars, 2);
        assert_eq!(s.first_bar.to_rfc3339(), "2026-11-06T09:00:00+07:00");
        assert_eq!(s.last_bar.to_rfc3339(), "2026-11-06T09:01:00+07:00");
        assert_eq!(s.bars_volume, 697_300);
        assert_eq!(s.vwap, Some(5_033.33));
    }

    #[test]
    fn two_minute_bars_cover_the_same_session() {
        let h = parse_chart(BBCA_2M, Period::D1, Interval::Min2).unwrap();
        let s = summary(session_bars(&h.candles)).unwrap();
        assert_eq!(s.bars, 168);
        assert_eq!(s.bars_volume, 155_142_100);
        assert_eq!(s.vwap, Some(5089.76));
    }

    #[test]
    fn index_has_no_volume_so_no_vwap() {
        let h = parse_chart(JKSE, Period::D1, Interval::Min1).unwrap();
        let s = summary(session_bars(&h.candles)).unwrap();
        assert_eq!(s.bars, 320);
        assert_eq!(s.bars_volume, 0);
        assert_eq!(s.vwap, None);
        assert!((s.high - 5072.05).abs() < 0.01, "{}", s.high);
        assert!((s.low - 4995.46).abs() < 0.01, "{}", s.low);
        assert_eq!(s.last_bar.to_rfc3339(), "2026-11-05T16:00:00+07:00");
    }

    #[test]
    fn only_the_last_date_counts() {
        // 2026-11-04 and 2026-11-05, 09:00 WIB.
        let day = |secs: i64, close: f64| Candle {
            time: wib_from_unix(secs).unwrap(),
            open: close,
            high: close + 5.0,
            low: close - 5.0,
            close,
            adj_close: None,
            volume: 100,
        };
        let candles = vec![
            day(1_793_757_600, 4_900.0),
            day(1_793_757_660, 4_910.0),
            day(1_793_844_000, 5_000.0),
            day(1_793_844_060, 5_010.0),
            day(1_793_844_120, 5_020.0),
        ];
        let bars = session_bars(&candles);
        assert_eq!(bars.len(), 3);
        assert_eq!(bars[0].time.to_rfc3339(), "2026-11-05T09:00:00+07:00");
        let s = summary(bars).unwrap();
        assert_eq!((s.open, s.close, s.bars_volume), (5_000.0, 5_020.0, 300));
        assert_eq!(s.vwap, Some(5_010.0));
        assert!(session_bars(&[]).is_empty());
        assert!(summary(&[]).is_none());
    }
}
