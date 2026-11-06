//! Javanese weton and the Chinese shio.
//!
//! - Weton: the seven-day week meets the five-day pasaran (Legi, Pahing, Pon,
//!   Wage, Kliwon). The pasaran is the Julian day number mod 5 (0 = Legi):
//!   17 August 1945 is JDN 2431685 and Jumat Legi, as Soekarno said that day.
//!   Neptu values after primbon tables (Minggu 5, Senin 4, Selasa 3, Rabu 7,
//!   Kamis 8, Jumat 6, Sabtu 9; Legi 5, Pahing 9, Pon 7, Wage 4, Kliwon 8).
//!   The civil date is used; Javanese reckoning starts the day at sunset.
//! - Shio: Imlek computed from new moons and solar terms in China Standard
//!   Time (UTC+8), the rule of the Chinese calendar since 1929: month 11 holds
//!   the winter solstice; with 13 months between two such months the first
//!   one without a principal term is the leap month; Imlek is the second new
//!   moon after the month-11 start (the third when month 11 or 12 is leap).
//!   Checked against the published dates 1924-2050 (Wikipedia's table and
//!   the Hong Kong Observatory's conversion tables); `null` outside.

use chrono::{Datelike, NaiveDate};

use crate::model::{Body, Shio, Weton};

use super::ephem::{norm360, position};
use super::events::{next_phase, prev_phase};
use super::time::{date_of_day, utc_ms, DAY_MS, HOUR_MS};

pub const DAYS_ID: [&str; 7] = [
    "Minggu", "Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu",
];
const DAYS_EN: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const NEPTU_DAY: [u8; 7] = [5, 4, 3, 7, 8, 6, 9];
pub const PASARAN: [&str; 5] = ["Legi", "Pahing", "Pon", "Wage", "Kliwon"];
const NEPTU_PASARAN: [u8; 5] = [5, 9, 7, 4, 8];

/// Julian day number of a civil date.
fn jdn(date: NaiveDate) -> i64 {
    date.num_days_from_ce() as i64 + 1_721_425
}

/// Index into [`PASARAN`] of `date`.
pub fn pasaran_index(date: NaiveDate) -> usize {
    jdn(date).rem_euclid(5) as usize
}

pub fn weton(date: NaiveDate) -> Weton {
    let n = jdn(date);
    let dow = (n + 1).rem_euclid(7) as usize; // 0 = Minggu
    let p = n.rem_euclid(5) as usize;
    Weton {
        day: DAYS_ID[dow].to_string(),
        day_en: DAYS_EN[dow].to_string(),
        pasaran: PASARAN[p].to_string(),
        neptu_day: NEPTU_DAY[dow],
        neptu_pasaran: NEPTU_PASARAN[p],
        neptu: NEPTU_DAY[dow] + NEPTU_PASARAN[p],
    }
}

const CST_MS: f64 = 8.0 * HOUR_MS;
/// Mean motion of the Sun, degrees per day.
const SUN_RATE: f64 = 0.98564736;

/// Day number (days since 1970-01-01) of an instant's date in UTC+8.
fn day_cst(ms: f64) -> i64 {
    ((ms + CST_MS) / DAY_MS).floor() as i64
}

/// The instant the Sun's apparent longitude reaches `target`, searching forward from `ms`.
pub fn sun_longitude_time(ms: f64, target: f64) -> f64 {
    let mut t = ms + norm360(target - position(Body::Sun, ms).lon) / SUN_RATE * DAY_MS;
    for _ in 0..10 {
        let d = norm360(target - position(Body::Sun, t).lon + 180.0) - 180.0;
        let step = d / SUN_RATE * DAY_MS;
        t += step;
        if step.abs() < 1000.0 {
            break;
        }
    }
    t
}

/// Start (UTC+8 day number) of the lunar month holding UTC+8 day `day`.
fn month_start_on_or_before(day: i64) -> i64 {
    let end_of_day = day as f64 * DAY_MS - CST_MS + DAY_MS - 1.0;
    day_cst(prev_phase(end_of_day, 0.0))
}

/// Imlek (first day of the first lunar month) of Gregorian year `year`, as a CST date.
pub fn chinese_new_year(year: i32) -> NaiveDate {
    let dec1 = |y: i32| utc_ms(NaiveDate::from_ymd_opt(y, 12, 1).expect("valid date"), 0);
    let ws1 = day_cst(sun_longitude_time(dec1(year - 1), 270.0));
    let ws2 = day_cst(sun_longitude_time(dec1(year), 270.0));
    let m11a = month_start_on_or_before(ws1);
    let m11b = month_start_on_or_before(ws2);
    let mut starts = vec![m11a];
    let mut t = m11a as f64 * DAY_MS - CST_MS + DAY_MS;
    loop {
        let nm = next_phase(t + 2.0 * DAY_MS, 0.0);
        let d = day_cst(nm);
        if d > m11b {
            break;
        }
        starts.push(d);
        t = nm;
        if d == m11b {
            break;
        }
    }
    let mut leap: Option<usize> = None;
    if starts.len() - 1 == 13 {
        for i in 1..starts.len() - 1 {
            // Does month i hold a principal term (the Sun at a multiple of 30°)?
            let start_ms = starts[i] as f64 * DAY_MS - CST_MS;
            let lon = position(Body::Sun, start_ms).lon;
            let next = (lon / 30.0).ceil() * 30.0;
            let zq = day_cst(sun_longitude_time(start_ms, next % 360.0));
            if zq >= starts[i + 1] {
                leap = Some(i);
                break;
            }
        }
    }
    let idx = if matches!(leap, Some(1) | Some(2)) {
        3
    } else {
        2
    };
    date_of_day(starts[idx])
}

const ANIMALS: [&str; 12] = [
    "rat", "ox", "tiger", "rabbit", "dragon", "snake", "horse", "goat", "monkey", "rooster", "dog",
    "pig",
];
const ANIMALS_ID: [&str; 12] = [
    "Tikus", "Kerbau", "Macan", "Kelinci", "Naga", "Ular", "Kuda", "Kambing", "Monyet", "Ayam",
    "Anjing", "Babi",
];
const ELEMENTS: [&str; 10] = [
    "wood", "wood", "fire", "fire", "earth", "earth", "metal", "metal", "water", "water",
];

/// The Chinese year holding `date`; `None` outside the checked range 1924-2050.
pub fn shio(date: NaiveDate) -> Option<Shio> {
    let y = date.year();
    if !(1924..=2050).contains(&y) {
        return None;
    }
    let mut ly = y;
    let mut start = chinese_new_year(y);
    if date < start {
        ly = y - 1;
        if ly < 1924 {
            return None;
        }
        start = chinese_new_year(ly);
    }
    let a = (ly - 4).rem_euclid(12) as usize;
    let s = (ly - 4).rem_euclid(10) as usize;
    Some(Shio {
        animal: ANIMALS[a].to_string(),
        animal_id: ANIMALS_ID[a].to_string(),
        element: ELEMENTS[s].to_string(),
        year_start: start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn weton_matches_published_dates() {
        // (date, day, pasaran, neptu): see the sources in tests/fixtures/astro/README.md.
        let cases = [
            ("1633-07-08", "Jumat", "Legi", 11),
            ("1945-08-17", "Jumat", "Legi", 11),
            ("2026-01-01", "Kamis", "Pon", 15),
            ("2026-01-02", "Jumat", "Wage", 10),
            ("2026-01-03", "Sabtu", "Kliwon", 17),
            ("2026-01-10", "Sabtu", "Pahing", 18),
            ("2026-01-17", "Sabtu", "Wage", 13),
            ("2026-01-31", "Sabtu", "Pon", 16),
            ("2026-05-03", "Minggu", "Kliwon", 13),
            ("2026-07-06", "Senin", "Wage", 8),
            ("2026-07-07", "Selasa", "Kliwon", 11),
            ("2026-07-15", "Rabu", "Pon", 14),
            ("2026-10-20", "Selasa", "Kliwon", 11),
            ("2026-10-21", "Rabu", "Legi", 12),
            ("2026-10-23", "Jumat", "Pon", 13),
            ("2026-10-25", "Minggu", "Kliwon", 13),
        ];
        for (date, day, pasaran, neptu) in cases {
            let w = weton(d(date));
            assert_eq!(
                (w.day.as_str(), w.pasaran.as_str(), w.neptu),
                (day, pasaran, neptu),
                "{date}"
            );
        }
        let w = weton(d("2026-11-06"));
        assert_eq!(
            (w.day.as_str(), w.day_en.as_str(), w.pasaran.as_str()),
            ("Jumat", "Friday", "Pahing")
        );
        assert_eq!((w.neptu_day, w.neptu_pasaran, w.neptu), (6, 9, 15));
    }

    #[test]
    fn imlek_matches_the_published_list_1924_2050() {
        let list = include_str!("../../tests/fixtures/astro/imlek_1924_2050_constructed.txt");
        let dates: Vec<NaiveDate> = list
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| l.trim().parse().unwrap())
            .collect();
        assert_eq!(dates.len(), 127);
        for want in dates {
            assert_eq!(chinese_new_year(want.year()), want);
        }
    }

    #[test]
    fn shio_of_dates() {
        let s = shio(d("2026-02-16")).unwrap();
        assert_eq!((s.animal.as_str(), s.element.as_str()), ("snake", "wood"));
        assert_eq!(s.year_start, d("2025-01-29"));
        let s = shio(d("2026-02-17")).unwrap();
        assert_eq!(
            (s.animal.as_str(), s.animal_id.as_str(), s.element.as_str()),
            ("horse", "Kuda", "fire")
        );
        assert_eq!(s.year_start, d("2026-02-17"));
        let s = shio(d("1998-05-21")).unwrap();
        assert_eq!((s.animal.as_str(), s.element.as_str()), ("tiger", "earth"));
        assert!(shio(d("1924-01-01")).is_none());
        assert!(shio(d("1924-02-05")).is_some());
        assert!(shio(d("1900-06-01")).is_none());
        assert_eq!(shio(d("2050-12-31")).unwrap().animal, "horse");
    }
}
