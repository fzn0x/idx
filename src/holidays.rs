//! IDX trading holidays (Kalender Libur Bursa), bundled from
//! `data/holidays.csv` and replaceable with `IDX_HOLIDAYS=<csv>`. idx.co.id
//! publishes the calendar only as one image per year, so the list is
//! transcribed and checked against the trading-day count IDX prints for
//! every month. Weekend holidays are left out, as IDX lists them.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc, Weekday};

use crate::error::{IdxError, Result};
use crate::model::Holiday;

const BUNDLED: &str = include_str!("../data/holidays.csv");

fn parse_csv(text: &str) -> Result<BTreeMap<NaiveDate, Holiday>> {
    let mut out = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.to_ascii_lowercase().starts_with("date,")
        {
            continue;
        }
        let mut parts = line.splitn(3, ',');
        let date = parts.next().unwrap_or("").trim();
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| {
            IdxError::Usage(format!(
                "holidays line {}: {date:?} is not a YYYY-MM-DD date",
                i + 1
            ))
        })?;
        let name = parts.next().map(str::trim).unwrap_or("").to_string();
        if name.is_empty() {
            return Err(IdxError::Usage(format!("holidays line {}: no name", i + 1)));
        }
        let name_en = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        out.insert(
            date,
            Holiday {
                date,
                weekday: weekday_name(date.weekday()).to_string(),
                name,
                name_en,
            },
        );
    }
    Ok(out)
}

fn weekday_name(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// The holiday list: `IDX_HOLIDAYS` when set (and readable), else the
/// bundled one. Loaded once per process.
fn table() -> &'static BTreeMap<NaiveDate, Holiday> {
    static TABLE: OnceLock<BTreeMap<NaiveDate, Holiday>> = OnceLock::new();
    TABLE.get_or_init(|| {
        if let Some(p) = std::env::var_os("IDX_HOLIDAYS") {
            match load_file(Path::new(&p)) {
                Ok(t) => return t,
                Err(e) => tracing::warn!(error = %e, "IDX_HOLIDAYS ignored"),
            }
        }
        parse_csv(BUNDLED).expect("bundled holidays.csv parses")
    })
}

fn load_file(path: &Path) -> Result<BTreeMap<NaiveDate, Holiday>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| IdxError::Usage(format!("holidays {}: {e}", path.display())))?;
    parse_csv(&text)
}

/// Every listed holiday, oldest first.
pub fn all() -> Vec<Holiday> {
    table().values().cloned().collect()
}

/// Years the list covers, oldest first.
pub fn years() -> Vec<i32> {
    let mut y: Vec<i32> = table().keys().map(|d| d.year()).collect();
    y.dedup();
    y
}

/// `true` when the list covers `year` (its holidays are known).
pub fn covers(year: i32) -> bool {
    table().keys().any(|d| d.year() == year)
}

/// The holiday on `date`, if any.
pub fn holiday(date: NaiveDate) -> Option<&'static Holiday> {
    table().get(&date)
}

/// A weekday that is not a listed holiday. For a year the list does not
/// cover, only weekends count as closed.
pub fn is_trading_day(date: NaiveDate) -> bool {
    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) && holiday(date).is_none()
}

/// The first trading day after `date`.
pub fn next_trading_day(date: NaiveDate) -> NaiveDate {
    let mut d = date.succ_opt().expect("date in range");
    while !is_trading_day(d) {
        d = d.succ_opt().expect("date in range");
    }
    d
}

/// The last trading day at or before `date`.
pub fn last_trading_day(date: NaiveDate) -> NaiveDate {
    let mut d = date;
    while !is_trading_day(d) {
        d = d.pred_opt().expect("date in range");
    }
    d
}

/// When an IDX trading day's session is over, in seconds after midnight WIB:
/// 16:15:00, the end of post-closing on the regular market, Monday to Friday
/// (IDX trading hours, Decree Kep-00158/BEI/10-2026; the web terminal's
/// clock shows the same timetable). Closing quotes carry 16:14 to 16:15.
pub const SESSION_END_SECS: u32 = 16 * 3600 + 15 * 60;

/// The latest trading day whose session has ended by `now`: today from
/// 16:15 WIB on a trading day, otherwise the trading day before. Weekends
/// and listed holidays are skipped, so a holiday never counts as a missed
/// session; for a year the list does not cover, only weekends are.
pub fn latest_session(now: DateTime<Utc>) -> NaiveDate {
    let local = now.with_timezone(&crate::model::wib());
    let today = local.date_naive();
    if is_trading_day(today) && local.time().num_seconds_from_midnight() >= SESSION_END_SECS {
        today
    } else {
        last_trading_day(today.pred_opt().expect("date in range"))
    }
}

/// Trading days in `year`: weekdays minus listed holidays.
pub fn trading_days(year: i32) -> u32 {
    let mut d = NaiveDate::from_ymd_opt(year, 1, 1).expect("valid year");
    let mut n = 0;
    while d.year() == year {
        if is_trading_day(d) {
            n += 1;
        }
        d = d.succ_opt().expect("date in range");
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn bundled_list_matches_the_trading_days_idx_publishes() {
        // The totals printed under each year's image on idx.co.id.
        assert_eq!(trading_days(2026), 239);
        assert_eq!(trading_days(2027), 241);
        assert!(covers(2026) && covers(2027) && !covers(2025));
        assert_eq!(years(), [2026, 2027]);
        for h in all() {
            assert!(
                !matches!(h.date.weekday(), Weekday::Sat | Weekday::Sun),
                "{} falls on a weekend",
                h.date
            );
        }
    }

    #[test]
    fn trading_day_helpers_skip_weekends_and_holidays() {
        let h = holiday(d(2026, 8, 17)).unwrap();
        assert_eq!(h.name, "Proklamasi Kemerdekaan");
        assert_eq!(h.name_en.as_deref(), Some("Independence Day of Indonesia"));
        assert_eq!(h.weekday, "Monday");
        assert!(!is_trading_day(d(2026, 8, 17)));
        assert!(!is_trading_day(d(2026, 10, 18)), "Sunday");
        assert!(is_trading_day(d(2026, 10, 19)));
        // Friday 2026-08-14, the weekend and the Monday holiday.
        assert_eq!(next_trading_day(d(2026, 8, 14)), d(2026, 8, 18));
        assert_eq!(last_trading_day(d(2026, 8, 17)), d(2026, 8, 14));
        // Idul Fitri 2026: Friday 20 March through Tuesday 24 March.
        assert_eq!(next_trading_day(d(2026, 3, 19)), d(2026, 3, 25));
    }

    #[test]
    fn latest_session_skips_weekends_holidays_and_waits_for_the_close() {
        let at = |s: &str| {
            DateTime::parse_from_rfc3339(&format!("{s}+07:00"))
                .unwrap()
                .with_timezone(&Utc)
        };
        for (now, want, why) in [
            (
                "2026-11-06T06:52:00",
                d(2026, 11, 5),
                "Friday before the open",
            ),
            (
                "2026-11-06T10:00:00",
                d(2026, 11, 5),
                "Friday during the session",
            ),
            (
                "2026-11-06T16:14:59",
                d(2026, 11, 5),
                "one second before the end",
            ),
            ("2026-11-06T16:15:00", d(2026, 11, 6), "post-closing over"),
            ("2026-11-07T12:00:00", d(2026, 11, 6), "Saturday"),
            ("2026-11-08T23:59:59", d(2026, 11, 6), "Sunday night"),
            (
                "2026-11-09T00:30:00",
                d(2026, 11, 6),
                "Monday after midnight",
            ),
            (
                "2026-11-09T16:15:00",
                d(2026, 11, 9),
                "Monday after the close",
            ),
            ("2026-08-17T17:00:00", d(2026, 8, 14), "Independence Day"),
            (
                "2026-08-18T08:00:00",
                d(2026, 8, 14),
                "the morning after it",
            ),
            (
                "2026-03-25T08:00:00",
                d(2026, 3, 17),
                "after Nyepi and Idul Fitri",
            ),
            ("2026-12-24T17:00:00", d(2026, 12, 23), "cuti bersama Natal"),
            (
                "2027-01-04T08:00:00",
                d(2026, 12, 30),
                "12-31 and 01-01 closed",
            ),
            ("2027-01-05T17:00:00", d(2027, 1, 4), "Isra Mikraj"),
        ] {
            assert_eq!(latest_session(at(now)), want, "{now}: {why}");
        }
        // The same instant written in UTC: 09:15Z is 16:15 WIB.
        let utc = DateTime::parse_from_rfc3339("2026-11-06T09:15:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(latest_session(utc), d(2026, 11, 6));
    }

    #[test]
    fn parser_reads_optional_english_names_and_rejects_bad_rows() {
        let t = parse_csv("# c\ndate,name,name_en\n2030-01-01,Tahun Baru\n").unwrap();
        let h = &t[&d(2030, 1, 1)];
        assert_eq!(h.name, "Tahun Baru");
        assert_eq!(h.name_en, None);
        assert!(parse_csv("01-01-2030,Tahun Baru\n").is_err());
        assert!(parse_csv("2030-01-01,\n").is_err());
    }
}
