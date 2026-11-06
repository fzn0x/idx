//! Lunar phases, stations, sign ingresses and Mercury's retrograde periods.
//! Instants are milliseconds since the Unix epoch (see `time`).

use crate::model::{
    AstroEvent, AstroEventKind, Body, MercuryState, MoonPhase, RetrogradePeriod, ZodiacSign,
};

use super::ephem::{cosd, norm360, position, speed, sun_moon};
use super::time::{wib_minute, DAY_MS, HOUR_MS};

/// Mean rate of the Moon-Sun elongation, degrees per day.
const SYNODIC_RATE: f64 = 12.190749;

/// Moon minus Sun apparent longitude at `ms`, degrees [0, 360).
pub fn elongation(ms: f64) -> f64 {
    norm360(position(Body::Moon, ms).lon - position(Body::Sun, ms).lon)
}

pub fn phase_at(ms: f64) -> MoonPhase {
    MoonPhase::of(elongation(ms))
}

/// Illuminated fraction of the Moon's disc (Meeus 48.2, 48.3).
pub fn illumination(ms: f64) -> f64 {
    let (s, m) = sun_moon(ms);
    let psi = (cosd(m.lat) * cosd(m.lon - s.lon)).acos();
    let r = s.dist * 149_597_870.7;
    let i = (r * psi.sin()).atan2(m.dist - r * psi.cos());
    (1.0 + i.cos()) / 2.0
}

/// The instant the elongation reaches `target` (0, 90, 180, 270), searching forward from `ms`.
pub fn next_phase(ms: f64, target: f64) -> f64 {
    let mut t = ms;
    let diff = norm360(target - elongation(t));
    t += diff / SYNODIC_RATE * DAY_MS;
    for _ in 0..8 {
        let d = norm360(target - elongation(t) + 180.0) - 180.0;
        let step = d / SYNODIC_RATE * DAY_MS;
        t += step;
        if step.abs() < 500.0 {
            break;
        }
    }
    t
}

/// The last instant at or before `ms` the elongation was `target`.
pub fn prev_phase(ms: f64, target: f64) -> f64 {
    let back = norm360(elongation(ms) - target);
    let mut t = next_phase(ms - (back / SYNODIC_RATE + 2.0) * DAY_MS, target);
    if t > ms {
        t = next_phase(t - 33.0 * DAY_MS, target);
    }
    t
}

/// Where a planet's longitude speed changes sign in [from, to), scanned at
/// `step_days` and bisected to under 30 s.
pub fn stations(body: Body, from: f64, to: f64, step_days: f64) -> Vec<(f64, AstroEventKind)> {
    let mut out = Vec::new();
    let step = step_days * DAY_MS;
    let mut t0 = from;
    let mut v0 = speed(body, t0);
    let mut t1 = from + step;
    while t1 <= to + step {
        let v1 = speed(body, t1);
        if (v0 < 0.0) != (v1 < 0.0) {
            let (mut a, mut b, mut va) = (t0, t1, v0);
            let mut i = 0;
            while i < 40 && b - a > 30_000.0 {
                let m = (a + b) / 2.0;
                let vm = speed(body, m);
                if (va < 0.0) == (vm < 0.0) {
                    a = m;
                    va = vm;
                } else {
                    b = m;
                }
                i += 1;
            }
            let at = (a + b) / 2.0;
            if at >= from && at < to {
                let kind = if v1 < 0.0 {
                    AstroEventKind::StationRetrograde
                } else {
                    AstroEventKind::StationDirect
                };
                out.push((at, kind));
            }
        }
        t0 = t1;
        v0 = v1;
        t1 += step;
    }
    out
}

fn sign_at(body: Body, ms: f64) -> ZodiacSign {
    ZodiacSign::of(position(body, ms).lon)
}

/// Sign changes of `body` in [from, to), scanned every 6 hours and bisected 30 times.
pub fn ingresses(body: Body, from: f64, to: f64) -> Vec<(f64, ZodiacSign)> {
    let mut out = Vec::new();
    let step = 6.0 * HOUR_MS;
    let mut s0 = sign_at(body, from);
    let mut t = from;
    while t < to {
        let end = (t + step).min(to);
        let s1 = sign_at(body, end);
        if s1 != s0 {
            let (mut lo, mut hi) = (t, end);
            for _ in 0..30 {
                let mid = (lo + hi) / 2.0;
                if sign_at(body, mid) == s0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            out.push(((lo + hi) / 2.0, s1));
            s0 = s1;
        }
        t += step;
    }
    out
}

const PHASES: [(AstroEventKind, f64); 4] = [
    (AstroEventKind::NewMoon, 0.0),
    (AstroEventKind::FirstQuarter, 90.0),
    (AstroEventKind::FullMoon, 180.0),
    (AstroEventKind::LastQuarter, 270.0),
];
const PLANETS: [Body; 8] = [
    Body::Mercury,
    Body::Venus,
    Body::Mars,
    Body::Jupiter,
    Body::Saturn,
    Body::Uranus,
    Body::Neptune,
    Body::Pluto,
];

/// Exact phases, stations (Mercury to Pluto) and ingresses (Sun, Mercury to
/// Pluto; the Moon changes sign every two or three days and is left out) in
/// [from, to), oldest first.
pub fn upcoming(from: f64, to: f64) -> Vec<AstroEvent> {
    let mut out: Vec<(f64, AstroEventKind, Body, ZodiacSign)> = Vec::new();
    for (kind, target) in PHASES {
        let mut t = next_phase(from, target);
        if t < from {
            t = next_phase(from + DAY_MS, target);
        }
        while t < to {
            out.push((t, kind, Body::Moon, sign_at(Body::Moon, t)));
            t = next_phase(t + 25.0 * DAY_MS, target);
        }
    }
    for b in PLANETS {
        for (at, kind) in stations(b, from, to, 1.0) {
            out.push((at, kind, b, sign_at(b, at)));
        }
    }
    for b in std::iter::once(Body::Sun).chain(PLANETS) {
        for (at, sign) in ingresses(b, from, to) {
            out.push((at, AstroEventKind::Ingress, b, sign));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter()
        .map(|(at, kind, body, sign)| AstroEvent {
            at: wib_minute(at),
            kind,
            body,
            sign,
        })
        .collect()
}

/// Mercury at `ms`: the current retrograde period if any, and the next one.
pub fn mercury(ms: f64) -> MercuryState {
    let back = stations(Body::Mercury, ms - 130.0 * DAY_MS, ms, 1.0);
    let fwd = stations(Body::Mercury, ms, ms + 260.0 * DAY_MS, 1.0);
    let retro = speed(Body::Mercury, ms) < 0.0;
    let last_sr = back
        .iter()
        .rev()
        .find(|s| s.1 == AstroEventKind::StationRetrograde);
    let next_sd = fwd.iter().find(|s| s.1 == AstroEventKind::StationDirect);
    let next_sr = fwd
        .iter()
        .find(|s| s.1 == AstroEventKind::StationRetrograde);
    let sd_after = next_sr.and_then(|sr| {
        fwd.iter()
            .find(|s| s.1 == AstroEventKind::StationDirect && s.0 > sr.0)
    });
    MercuryState {
        retrograde: retro,
        retrograde_since: last_sr.filter(|_| retro).map(|s| wib_minute(s.0)),
        retrograde_until: next_sd.filter(|_| retro).map(|s| wib_minute(s.0)),
        next_retrograde: next_sr.map(|sr| RetrogradePeriod {
            from: wib_minute(sr.0),
            until: sd_after.map(|s| wib_minute(s.0)),
        }),
    }
}

/// Mercury's retrograde intervals [station retrograde, station direct)
/// covering [from, to], from a scan at 5-day steps (enough for Mercury,
/// whose retrogrades last about three weeks).
pub fn mercury_retrograde_intervals(from: f64, to: f64) -> Vec<(f64, f64)> {
    let st = stations(Body::Mercury, from, to, 5.0);
    let mut iv = Vec::new();
    let mut open = (speed(Body::Mercury, from) < 0.0).then_some(from);
    for (at, kind) in st {
        if kind == AstroEventKind::StationRetrograde {
            open = Some(at);
        } else if let Some(o) = open.take() {
            iv.push((o, at));
        }
    }
    if let Some(o) = open {
        iv.push((o, to));
    }
    iv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::ephem::tests::horizons;

    fn usno(text: &str) -> Vec<(f64, f64)> {
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        v["phasedata"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let (hh, mm) = p["time"].as_str().unwrap().split_once(':').unwrap();
                let day = chrono::NaiveDate::from_ymd_opt(
                    p["year"].as_i64().unwrap() as i32,
                    p["month"].as_u64().unwrap() as u32,
                    p["day"].as_u64().unwrap() as u32,
                )
                .unwrap();
                let ms = super::super::time::utc_ms(day, 0)
                    + (hh.parse::<f64>().unwrap() * 60.0 + mm.parse::<f64>().unwrap()) * 60_000.0;
                let target = match p["phase"].as_str().unwrap() {
                    "New Moon" => 0.0,
                    "First Quarter" => 90.0,
                    "Full Moon" => 180.0,
                    "Last Quarter" => 270.0,
                    other => panic!("{other}"),
                };
                (ms, target)
            })
            .collect()
    }

    #[test]
    fn phases_within_two_minutes_of_usno() {
        let mut n = 0;
        for text in [
            include_str!("../../tests/fixtures/astro/usno_moon_phases_1982.json"),
            include_str!("../../tests/fixtures/astro/usno_moon_phases_2025.json"),
            include_str!("../../tests/fixtures/astro/usno_moon_phases_2026.json"),
            include_str!("../../tests/fixtures/astro/usno_moon_phases_2027.json"),
        ] {
            for (ms, target) in usno(text) {
                let ours = next_phase(ms - 3.0 * DAY_MS, target);
                assert!(
                    (ours - ms).abs() <= 120_000.0,
                    "{target}° at {ms}: off by {} s",
                    (ours - ms) / 1000.0
                );
                let prev = prev_phase(ms + 3.0 * DAY_MS, target);
                assert!(
                    (prev - ours).abs() < 2000.0,
                    "prev_phase finds the same instant"
                );
                n += 1;
            }
        }
        assert_eq!(n, 198);
    }

    #[test]
    fn illumination_matches_horizons_2026() {
        let text = include_str!("../../tests/fixtures/astro/horizons_moon_2026_illum.txt");
        let block = text
            .split("$$SOE")
            .nth(1)
            .unwrap()
            .split("$$EOE")
            .next()
            .unwrap();
        let illu: Vec<f64> = block
            .lines()
            .filter_map(|l| l.split(',').nth(3).map(str::trim))
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<f64>().unwrap() / 100.0)
            .collect();
        let rows = horizons(text, 1);
        assert_eq!(rows.len(), 365);
        assert_eq!(illu.len(), 365);
        for (r, k) in rows.iter().zip(illu) {
            let ours = illumination(r.ms);
            assert!((ours - k).abs() < 0.0005, "{}: {ours} vs {k}", r.ms);
        }
    }

    /// Stations from Horizons' daily longitudes: where the central difference changes sign,
    /// linearly interpolated.
    fn horizons_stations(text: &str) -> Vec<(f64, AstroEventKind)> {
        let rows = horizons(text, 0);
        let v: Vec<f64> = (1..rows.len() - 1)
            .map(|i| {
                let mut d = rows[i + 1].lon - rows[i - 1].lon;
                if d > 180.0 {
                    d -= 360.0;
                }
                if d < -180.0 {
                    d += 360.0;
                }
                d / 2.0
            })
            .collect();
        let mut out = Vec::new();
        for i in 1..v.len() {
            if (v[i - 1] < 0.0) != (v[i] < 0.0) {
                let f = v[i - 1] / (v[i - 1] - v[i]);
                let kind = if v[i] < 0.0 {
                    AstroEventKind::StationRetrograde
                } else {
                    AstroEventKind::StationDirect
                };
                out.push((rows[i].ms + f * DAY_MS, kind));
            }
        }
        out
    }

    #[test]
    fn mercury_and_saturn_stations_match_horizons() {
        for (body, text, tol_h, count) in [
            (
                Body::Mercury,
                include_str!("../../tests/fixtures/astro/horizons_mercury_2025_2027_daily.txt"),
                1.0,
                18,
            ),
            (
                Body::Saturn,
                include_str!("../../tests/fixtures/astro/horizons_saturn_2025_2027_daily.txt"),
                3.0,
                6,
            ),
        ] {
            let hz = horizons_stations(text);
            assert_eq!(hz.len(), count, "{body:?}");
            let rows = horizons(text, 0);
            let ours = stations(body, rows[1].ms, rows[rows.len() - 2].ms, 1.0);
            assert_eq!(ours.len(), hz.len(), "{body:?}");
            for (o, h) in ours.iter().zip(&hz) {
                assert_eq!(o.1, h.1);
                assert!(
                    (o.0 - h.0).abs() <= tol_h * HOUR_MS,
                    "{body:?} {:?}: {} h",
                    h.1,
                    (o.0 - h.0) / HOUR_MS
                );
            }
        }
    }

    #[test]
    fn saturn_ingresses_2025_2026() {
        // Horizons crossings (linear interpolation of the daily series), UT.
        let at = |y, m, d, hh, mm| {
            super::super::time::utc_ms(chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap(), 0)
                + (hh as f64 * 60.0 + mm as f64) * 60_000.0
        };
        let expected = [
            (at(2025, 5, 25, 3, 37), ZodiacSign::Aries),
            (at(2025, 9, 1, 8, 3), ZodiacSign::Pisces),
            (at(2026, 2, 14, 0, 11), ZodiacSign::Aries),
        ];
        let found = ingresses(Body::Saturn, at(2025, 1, 1, 0, 0), at(2027, 1, 1, 0, 0));
        assert_eq!(found.len(), 3, "{found:?}");
        for ((ms, sign), (want, want_sign)) in found.iter().zip(expected) {
            assert_eq!(*sign, want_sign);
            assert!(
                (ms - want).abs() <= 12.0 * HOUR_MS,
                "{} h",
                (ms - want) / HOUR_MS
            );
        }
    }

    #[test]
    fn mercury_on_2026_11_06() {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        let m = mercury(super::super::time::close_ms(d));
        assert!(m.retrograde);
        // Horizons: station retrograde 2026-10-24 06:46 UT, direct 2026-11-13 16:13 UT.
        let since = m.retrograde_since.unwrap();
        let until = m.retrograde_until.unwrap();
        let hz_since = chrono::DateTime::parse_from_rfc3339("2026-10-24T06:46:00Z").unwrap();
        let hz_until = chrono::DateTime::parse_from_rfc3339("2026-11-13T16:13:00Z").unwrap();
        assert!((since - hz_since).num_minutes().abs() <= 60);
        assert!((until - hz_until).num_minutes().abs() <= 60);
        let next = m.next_retrograde.unwrap();
        assert_eq!(next.from.date_naive().to_string(), "2027-02-10");
        assert_eq!(next.until.unwrap().date_naive().to_string(), "2027-03-03");
    }

    #[test]
    fn retrograde_intervals_agree_with_the_daily_speed() {
        let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let from = super::super::time::close_ms(d);
        let to = from + 900.0 * DAY_MS;
        let iv = mercury_retrograde_intervals(from - 40.0 * DAY_MS, to + 40.0 * DAY_MS);
        let mut k = 0;
        for day in 0..900 {
            let ms = from + day as f64 * DAY_MS;
            while k < iv.len() && iv[k].1 <= ms {
                k += 1;
            }
            let retro = k < iv.len() && iv[k].0 <= ms;
            assert_eq!(retro, speed(Body::Mercury, ms) < 0.0, "day {day}");
        }
    }
}
