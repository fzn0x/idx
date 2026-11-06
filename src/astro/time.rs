//! Instants and Julian days. An instant is milliseconds since the Unix epoch
//! as an `f64`, the representation `web/src/astro.js` uses too, so the CLI
//! and the web terminal compute the same numbers.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};

use crate::model::wib;

pub const DAY_MS: f64 = 86_400_000.0;
pub const HOUR_MS: f64 = 3_600_000.0;
const WIB_MS: f64 = 7.0 * HOUR_MS;

/// `Math.round` of JavaScript (halves go up), so both ports round alike.
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// Rounds to `places` decimals the way the web port does, without a negative zero.
pub fn round_to(x: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    let r = js_round(x * k) / k;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

pub fn jd_from_ms(ms: f64) -> f64 {
    ms / DAY_MS + 2_440_587.5
}

/// Delta T = TT - UT in seconds for decimal year `y` (Espenak and Meeus, 2006).
pub fn delta_t(y: f64) -> f64 {
    if y < 1800.0 {
        return 13.72; // outside the supported range; callers reject such dates
    }
    if y < 1860.0 {
        let t = y - 1800.0;
        return 13.72 - 0.332447 * t + 0.0068612 * t.powi(2) + 0.0041116 * t.powi(3)
            - 0.00037436 * t.powi(4)
            + 0.0000121272 * t.powi(5)
            - 0.0000001699 * t.powi(6)
            + 0.000000000875 * t.powi(7);
    }
    if y < 1900.0 {
        let t = y - 1860.0;
        return 7.62 + 0.5737 * t - 0.251754 * t.powi(2) + 0.01680668 * t.powi(3)
            - 0.0004473624 * t.powi(4)
            + t.powi(5) / 233174.0;
    }
    if y < 1920.0 {
        let t = y - 1900.0;
        return -2.79 + 1.494119 * t - 0.0598939 * t.powi(2) + 0.0061966 * t.powi(3)
            - 0.000197 * t.powi(4);
    }
    if y < 1941.0 {
        let t = y - 1920.0;
        return 21.20 + 0.84493 * t - 0.076100 * t.powi(2) + 0.0020936 * t.powi(3);
    }
    if y < 1961.0 {
        let t = y - 1950.0;
        return 29.07 + 0.407 * t - t.powi(2) / 233.0 + t.powi(3) / 2547.0;
    }
    if y < 1986.0 {
        let t = y - 1975.0;
        return 45.45 + 1.067 * t - t.powi(2) / 260.0 - t.powi(3) / 718.0;
    }
    if y < 2005.0 {
        let t = y - 2000.0;
        return 63.86 + 0.3345 * t - 0.060374 * t.powi(2)
            + 0.0017275 * t.powi(3)
            + 0.000651814 * t.powi(4)
            + 0.00002373599 * t.powi(5);
    }
    if y < 2050.0 {
        let t = y - 2000.0;
        return 62.92 + 0.32217 * t + 0.005589 * t.powi(2);
    }
    -20.0 + 32.0 * ((y - 1820.0) / 100.0).powi(2) - 0.5628 * (2150.0 - y)
}

/// Julian ephemeris day (TT) of an instant.
pub fn jde_from_ms(ms: f64) -> f64 {
    let jd = jd_from_ms(ms);
    let year = 2000.0 + (jd - 2_451_544.5) / 365.2425;
    jd + delta_t(year) / 86_400.0
}

/// `date` at `hour`:00 UTC.
pub fn utc_ms(date: NaiveDate, hour: u32) -> f64 {
    date.and_hms_opt(hour, 0, 0)
        .expect("valid hour")
        .and_utc()
        .timestamp_millis() as f64
}

/// 16:00 WIB on `date` (09:00 UTC), the IDX close: the instant the sky is computed for.
pub fn close_ms(date: NaiveDate) -> f64 {
    utc_ms(date, 9)
}

/// 12:00 WIB on `date` (05:00 UTC), for charts whose time of day is unknown.
pub fn noon_ms(date: NaiveDate) -> f64 {
    utc_ms(date, 5)
}

/// 00:00 WIB on `date`.
pub fn day_start_ms(date: NaiveDate) -> f64 {
    utc_ms(date, 0) - WIB_MS
}

/// An instant rounded to the minute, in WIB.
pub fn wib_minute(ms: f64) -> DateTime<FixedOffset> {
    let m = js_round(ms / 60_000.0) as i64 * 60_000;
    Utc.timestamp_millis_opt(m)
        .single()
        .expect("instant in range")
        .with_timezone(&wib())
}

/// The calendar date (in UTC+8, China Standard Time) of day number `day` since 1970-01-01.
pub fn date_of_day(day: i64) -> NaiveDate {
    NaiveDate::from_num_days_from_ce_opt((day + 719_163) as i32).expect("date in range")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn julian_days() {
        let d = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        assert_eq!(jd_from_ms(utc_ms(d, 12)), 2_451_545.0);
        assert_eq!(
            wib_minute(close_ms(d)).to_rfc3339(),
            "2000-01-01T16:00:00+07:00"
        );
        assert_eq!(
            wib_minute(day_start_ms(d)).to_rfc3339(),
            "2000-01-01T00:00:00+07:00"
        );
        assert_eq!(date_of_day(0).to_string(), "1970-01-01");
        // About 69 s observed in 2026; the 2005-2050 branch gives 75 s.
        assert!((delta_t(2026.0) - 75.0).abs() < 1.0);
        assert!((delta_t(1983.25) - 53.4).abs() < 1.0);
    }

    #[test]
    fn rounding_matches_javascript() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(round_to(-0.0004, 3).to_string(), "0");
        assert_eq!(round_to(223.956, 2), 223.96);
        assert_eq!(round_to(-1.2204, 3), -1.22);
    }
}
