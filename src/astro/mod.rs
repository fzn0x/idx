//! Offline ephemeris for `market astro`: apparent positions of the Sun, Moon
//! and planets, lunar phases, stations and sign ingresses, Mercury's
//! retrograde periods, the Javanese weton, the Chinese shio and IHSG's own
//! chart. Pure computation, no network, no new crates; checked against JPL
//! Horizons, USNO and published calendars in `tests/fixtures/astro/`.
//!
//! `web/src/astro.js` is a line-for-line port: same tables, same steps, same
//! rounding, so the CLI, the MCP server and the web terminal agree.

use chrono::NaiveDate;

use crate::error::{IdxError, Result};
use crate::model::{
    Astro, AstroBody, AstroWindow, Body, IhsgChart, MoonPhase, MoonState, NatalChoice, ZodiacSign,
};

pub mod calendar;
pub mod ephem;
pub mod events;
pub mod moon;
pub mod natal;
pub mod time;

use time::{close_ms, day_start_ms, js_round, round_to, wib_minute, DAY_MS};

/// First date the planetary elements cover.
pub fn first_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(1800, 1, 1).expect("valid date")
}
/// Last date the planetary elements cover.
pub fn last_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2050, 12, 31).expect("valid date")
}
/// Most days of upcoming events one call lists.
pub const MAX_DAYS: u32 = 366;

pub const METHOD: &str = "Apparent geocentric positions in the tropical zodiac (ecliptic and \
equinox of date) at 16:00 WIB (UTC+7), the IDX close. Planets and the Earth from JPL's Keplerian \
elements (Standish, Table 1, 1800-2050; Pluto from JPL's archived table), Jupiter and Saturn from \
Schlyter's elements with the Jupiter-Saturn perturbations, the Moon from Meeus, Astronomical \
Algorithms, ch. 47; light time, aberration, nutation and precession applied; Delta T after \
Espenak and Meeus. Checked against JPL Horizons 1800-2050: Sun and Moon within 0.01°, planets \
within 0.04°; phase times within 2 minutes of USNO; stations within 3 hours; slow planets' \
ingresses within 12 hours, so one can land on the neighbouring date. Moon phase: eight 45° \
sectors of the Moon-Sun elongation centred on the exact phases. Weton: pasaran from the Julian \
day number (17 Aug 1945 = Jumat Legi) on the civil date (Javanese days begin at sunset). Shio: \
Imlek from new moons and solar terms in China Standard Time, checked against published dates \
1924-2050. Natal chart at noon WIB, time unknown: no houses, the Moon approximate; its dates \
(launch 1983-04-01, base 1982-08-10) come from secondary sources.";

pub const DISCLAIMER: &str = "Traditional astrology, not a forecast or investment advice: the \
positions are astronomy, their meanings are tradition, and the backtest only describes the past.";

/// A body's position as the payload prints it.
pub(crate) struct Placement {
    pub longitude: f64,
    pub sign: ZodiacSign,
    pub degree: f64,
    pub speed: f64,
    pub retrograde: bool,
}

/// Longitude to 2 decimals and the sign and degree of that rounded value, so the three agree.
pub(crate) fn place(lon: f64) -> (f64, ZodiacSign, f64) {
    let mut l = js_round(lon * 100.0) / 100.0;
    if l >= 360.0 {
        l -= 360.0;
    }
    let idx = ((l / 30.0).floor() as usize).min(11);
    (l, ZodiacSign::ALL[idx], round_to(l - 30.0 * idx as f64, 2))
}

pub(crate) fn body_at(body: Body, ms: f64) -> Placement {
    let p = ephem::position(body, ms);
    let v = ephem::speed(body, ms);
    let (longitude, sign, degree) = place(p.lon);
    Placement {
        longitude,
        sign,
        degree,
        speed: round_to(v, 3),
        retrograde: body.can_retrograde() && v < 0.0,
    }
}

/// The sky for `date` (positions at 16:00 WIB), `days` of upcoming events
/// and the chosen natal chart; the backtest is added by the service. Also
/// returns warnings (an events window cut at the end of 2050).
pub fn sky(date: NaiveDate, days: u32, natal: NatalChoice) -> Result<(Astro, Vec<String>)> {
    if date < first_date() || date > last_date() {
        return Err(IdxError::Usage(format!(
            "astro covers {} to {}, the range of the planetary elements; got {date}",
            first_date(),
            last_date()
        )));
    }
    if !(1..=MAX_DAYS).contains(&days) {
        return Err(IdxError::Usage(format!(
            "--days must be 1 to {MAX_DAYS}; got {days}"
        )));
    }
    let mut warnings = Vec::new();
    let ms = close_ms(date);
    let from = day_start_ms(date);
    let mut to = from + days as f64 * DAY_MS;
    let limit = day_start_ms(last_date()) + DAY_MS;
    if to > limit {
        to = limit;
        warnings.push(format!(
            "events listed to {} only, the end of the planetary elements",
            last_date()
        ));
    }
    let el = events::elongation(ms);
    let prev_new = events::prev_phase(ms, 0.0);
    let bodies = Body::ALL
        .iter()
        .map(|&b| {
            let p = body_at(b, ms);
            AstroBody {
                body: b,
                longitude: p.longitude,
                sign: p.sign,
                degree: p.degree,
                speed: p.speed,
                retrograde: p.retrograde,
            }
        })
        .collect();
    let astro = Astro {
        date,
        at: wib_minute(ms),
        bodies,
        moon: MoonState {
            phase: MoonPhase::of(el),
            elongation: round_to(el, 2) % 360.0,
            illumination: round_to(events::illumination(ms), 3),
            age_days: round_to((ms - prev_new) / DAY_MS, 1),
            previous_new_moon: wib_minute(prev_new),
            next_new_moon: wib_minute(events::next_phase(ms, 0.0)),
            next_full_moon: wib_minute(events::next_phase(ms, 180.0)),
        },
        mercury: events::mercury(ms),
        weton: calendar::weton(date),
        shio: calendar::shio(date),
        upcoming: AstroWindow {
            from: wib_minute(from),
            to: wib_minute(to),
            events: events::upcoming(from, to),
        },
        natal: match natal {
            NatalChoice::Launch => Some(natal::natal(IhsgChart::Launch, ms)),
            NatalChoice::Base => Some(natal::natal(IhsgChart::Base, ms)),
            NatalChoice::None => None,
        },
        backtest: None,
        method: METHOD.to_string(),
        disclaimer: DISCLAIMER.to_string(),
    };
    Ok((astro, warnings))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{AstroEventKind, ZodiacSign};

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn sky_on_2026_11_06() {
        let (a, w) = sky(d("2026-11-06"), 30, NatalChoice::Launch).unwrap();
        assert!(w.is_empty());
        assert_eq!(a.at.to_rfc3339(), "2026-11-06T16:00:00+07:00");
        let row = |b: Body| a.bodies.iter().find(|x| x.body == b).unwrap();
        // Values of the prototype the plan validated (truth is Horizons; see ephem's tests).
        assert_eq!(
            (row(Body::Sun).longitude, row(Body::Sun).sign),
            (223.96, ZodiacSign::Scorpio)
        );
        assert_eq!(row(Body::Sun).degree, 13.96);
        assert_eq!(row(Body::Mercury).longitude, 219.92);
        assert!(row(Body::Mercury).retrograde);
        assert!(row(Body::Venus).retrograde);
        assert_eq!(
            (row(Body::Saturn).longitude, row(Body::Saturn).sign),
            (8.96, ZodiacSign::Aries)
        );
        assert!(row(Body::Saturn).retrograde);
        assert_eq!(
            (row(Body::Pluto).longitude, row(Body::Pluto).retrograde),
            (303.17, false)
        );
        assert!(!row(Body::Sun).retrograde && !row(Body::Moon).retrograde);
        assert_eq!(a.moon.phase, MoonPhase::WaningCrescent);
        assert!((a.moon.illumination - 0.087).abs() <= 0.001);
        assert_eq!(a.moon.age_days, 26.7);
        assert_eq!(
            (
                a.weton.day.as_str(),
                a.weton.pasaran.as_str(),
                a.weton.neptu
            ),
            ("Jumat", "Pahing", 15)
        );
        let shio = a.shio.as_ref().unwrap();
        assert_eq!(
            (shio.animal.as_str(), shio.element.as_str()),
            ("horse", "fire")
        );
        assert_eq!(shio.year_start, d("2026-02-17"));
        assert!(a.mercury.retrograde);
        // USNO: new moon 2026-11-09 07:02 UT, first quarter 11-17 11:48, full 11-24 14:53,
        // last quarter 12-01 06:08 (each within 2 minutes after rounding).
        let at = |k: AstroEventKind| a.upcoming.events.iter().find(|e| e.kind == k).unwrap().at;
        let near = |got: chrono::DateTime<chrono::FixedOffset>, want: &str| {
            let w = chrono::DateTime::parse_from_rfc3339(want).unwrap();
            assert!((got - w).num_minutes().abs() <= 2, "{got} vs {want}");
        };
        near(at(AstroEventKind::NewMoon), "2026-11-09T14:02:00+07:00");
        near(
            at(AstroEventKind::FirstQuarter),
            "2026-11-17T18:48:00+07:00",
        );
        near(at(AstroEventKind::FullMoon), "2026-11-24T21:53:00+07:00");
        near(at(AstroEventKind::LastQuarter), "2026-12-01T13:08:00+07:00");
        let sun_in = a
            .upcoming
            .events
            .iter()
            .find(|e| e.kind == AstroEventKind::Ingress && e.body == Body::Sun)
            .unwrap();
        assert_eq!(
            (sun_in.sign, sun_in.at.date_naive()),
            (ZodiacSign::Sagittarius, d("2026-11-22"))
        );
        assert!(a.upcoming.events.windows(2).all(|w| w[0].at <= w[1].at));
        assert_eq!(a.upcoming.events.len(), 9);
        let natal = a.natal.unwrap();
        assert_eq!(natal.transits.len(), 6);
        assert!(a.backtest.is_none());
    }

    /// `tests/fixtures/astro/golden_*.json` hold this payload for `web/test/astro.test.mjs`,
    /// which asserts the JS port computes the very same one. `IDX_WRITE_GOLDEN=1` rewrites it.
    pub(crate) fn golden(name: &str, value: &serde_json::Value) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/astro")
            .join(name);
        if std::env::var_os("IDX_WRITE_GOLDEN").is_some() {
            let mut text = serde_json::to_string_pretty(value).unwrap();
            text.push('\n');
            std::fs::write(&path, text).unwrap();
        }
        let want: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value, &want, "{name} differs; the web port reads it too");
    }

    #[test]
    fn sky_matches_the_golden_payload() {
        let (a, _) = sky(d("2026-11-06"), 30, NatalChoice::Launch).unwrap();
        golden(
            "golden_sky_2026-11-06.json",
            &serde_json::to_value(&a).unwrap(),
        );
    }

    #[test]
    fn rounding_keeps_sign_and_degree_consistent() {
        assert_eq!(place(29.999), (30.0, ZodiacSign::Taurus, 0.0));
        assert_eq!(place(359.996), (0.0, ZodiacSign::Aries, 0.0));
        assert_eq!(place(223.9612), (223.96, ZodiacSign::Scorpio, 13.96));
    }

    #[test]
    fn range_checks() {
        for (date, days) in [
            ("1799-12-31", 30),
            ("2051-01-01", 30),
            ("2026-11-06", 0),
            ("2026-11-06", 367),
        ] {
            let e = sky(d(date), days, NatalChoice::None).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{date} {days}");
        }
        let (a, w) = sky(d("2050-12-20"), 30, NatalChoice::None).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(a.upcoming.to.to_rfc3339(), "2051-01-01T00:00:00+07:00");
        assert!(a.natal.is_none());
        let (a, _) = sky(d("1800-01-01"), 1, NatalChoice::Base).unwrap();
        assert!(a.shio.is_none());
    }
}
