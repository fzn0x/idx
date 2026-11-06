//! IHSG's chart and today's aspects to it.
//!
//! IHSG was first published on 1 April 1983 (launch) with 10 August 1982 as
//! its base date (value 100), per Antara, Bisnis.com and CEIC; idx.co.id was
//! not reachable to confirm it. The time of day is unknown, so the chart is
//! cast for 12:00 WIB, without houses or ascendant, and its Moon (13° a day)
//! is approximate.

use crate::model::{Aspect, Body, IhsgChart, NatalBody, NatalChart, Transit};

use super::ephem::{norm360, position};
use super::time::{noon_ms, round_to, wib_minute, DAY_MS};
use super::{body_at, Placement};

/// Widest orb listed, degrees.
pub const TRANSIT_ORB: f64 = 2.0;

/// The chart and the aspects that the sky at `ms` makes to it.
pub fn natal(chart: IhsgChart, ms: f64) -> NatalChart {
    let date = chart.date();
    let at = noon_ms(date);
    let bodies: Vec<NatalBody> = Body::ALL
        .iter()
        .map(|&b| {
            let Placement {
                longitude,
                sign,
                degree,
                speed,
                retrograde,
            } = body_at(b, at);
            NatalBody {
                body: b,
                longitude,
                sign,
                degree,
                speed,
                retrograde,
                approximate: b == Body::Moon,
            }
        })
        .collect();
    let mut transits = Vec::new();
    for t in Body::ALL.into_iter().filter(|b| *b != Body::Moon) {
        let now = position(t, ms).lon;
        let later = position(t, ms + DAY_MS).lon;
        for n in bodies.iter().filter(|x| x.body != Body::Moon) {
            let sep = |lon: f64| {
                let d = norm360(lon - n.longitude).abs();
                if d > 180.0 {
                    360.0 - d
                } else {
                    d
                }
            };
            for aspect in Aspect::ALL {
                let orb = (sep(now) - aspect.angle()).abs();
                if orb <= TRANSIT_ORB {
                    transits.push(Transit {
                        transit: t,
                        aspect,
                        natal: n.body,
                        orb: round_to(orb, 2),
                        applying: (sep(later) - aspect.angle()).abs() < orb,
                    });
                }
            }
        }
    }
    // Stable: equal orbs keep the transit body order.
    transits.sort_by(|a, b| a.orb.total_cmp(&b.orb));
    NatalChart {
        chart,
        date,
        at: wib_minute(at),
        bodies,
        transits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::ephem::tests::{arcsec, horizons};
    use crate::astro::time::close_ms;
    use crate::model::ZodiacSign;

    #[test]
    fn launch_chart() {
        let today = close_ms("2026-11-06".parse().unwrap());
        let c = natal(IhsgChart::Launch, today);
        assert_eq!(c.date.to_string(), "1983-04-01");
        assert_eq!(c.at.to_rfc3339(), "1983-04-01T12:00:00+07:00");
        let sun = &c.bodies[0];
        assert_eq!((sun.longitude, sun.sign), (10.91, ZodiacSign::Aries));
        let moon = &c.bodies[1];
        assert_eq!((moon.sign, moon.approximate), (ZodiacSign::Scorpio, true));
        assert!(c.bodies.iter().filter(|b| b.approximate).count() == 1);
        let saturn = &c.bodies[6];
        assert_eq!(
            (saturn.sign, saturn.retrograde),
            (ZodiacSign::Scorpio, true)
        );
        assert!(c.transits.windows(2).all(|w| w[0].orb <= w[1].orb));
        assert!(c.transits.iter().all(|t| t.orb <= TRANSIT_ORB));
        let first = &c.transits[0];
        assert_eq!(
            (first.transit, first.aspect, first.natal),
            (Body::Saturn, Aspect::Trine, Body::Uranus)
        );
        // Horizons has the Sun's row for 1983-04-01 05:00 UT in the 1800-2050 list.
        let rows = horizons(
            include_str!("../../tests/fixtures/astro/horizons_sun_1800_2050.txt"),
            0,
        );
        let row = rows
            .iter()
            .find(|r| (r.ms - noon_ms(c.date)).abs() < 1000.0)
            .unwrap();
        assert!(arcsec(position(Body::Sun, row.ms).lon, row.lon) < 40.0);
    }

    #[test]
    fn base_chart() {
        let c = natal(IhsgChart::Base, close_ms("2026-11-06".parse().unwrap()));
        assert_eq!(c.date.to_string(), "1982-08-10");
        assert_eq!(c.bodies[0].sign, ZodiacSign::Leo);
        assert_eq!(c.bodies[1].sign, ZodiacSign::Aries);
    }
}
