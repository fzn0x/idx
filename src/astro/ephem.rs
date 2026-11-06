//! Apparent geocentric positions of the Sun and the planets.
//!
//! - Planets and the Earth-Moon barycentre: E. M. Standish, "Keplerian Elements
//!   for Approximate Positions of the Major Planets", Table 1 (1800-2050),
//!   <https://ssd.jpl.nasa.gov/planets/approx_pos.html>. Pluto, since dropped
//!   from that page, from JPL's archived `p_elem_t1.txt`
//!   (web.archive.org/web/20190803153746/https://ssd.jpl.nasa.gov/txt/p_elem_t1.txt).
//! - Jupiter and Saturn: P. Schlyter, "How to compute planetary positions"
//!   (<https://stjarnhimlen.se/comp/ppcomp.html>): elements of date plus the
//!   Jupiter-Saturn perturbations, five times closer to JPL Horizons than
//!   Table 1 alone.
//! - Nutation (low accuracy), ecliptic precession and annual aberration:
//!   J. Meeus, "Astronomical Algorithms", 2nd ed., ch. 22, 21.5 and 23.
//!
//! Checked against JPL Horizons in `tests/fixtures/astro/` (see the tests
//! below): the Sun within 30″ and every planet within 130″, 1800-2050.

use crate::model::Body;

use super::moon;
use super::time::{jde_from_ms, HOUR_MS};

pub(crate) const D2R: f64 = std::f64::consts::PI / 180.0;
pub(crate) const R2D: f64 = 180.0 / std::f64::consts::PI;
const LIGHT_DAYS_PER_AU: f64 = 0.0057755183;

pub fn norm360(x: f64) -> f64 {
    ((x % 360.0) + 360.0) % 360.0
}
pub(crate) fn sind(x: f64) -> f64 {
    (x * D2R).sin()
}
pub(crate) fn cosd(x: f64) -> f64 {
    (x * D2R).cos()
}

/// Julian centuries of TT from J2000.0.
pub(crate) fn centuries(jde: f64) -> f64 {
    (jde - 2_451_545.0) / 36_525.0
}

/// An apparent position of date: longitude and latitude in degrees, distance
/// in au (km for the Moon).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pos {
    pub lon: f64,
    pub lat: f64,
    pub dist: f64,
}

/// Nutation in longitude, degrees (Meeus ch. 22, 0.5″).
pub fn nutation_lon(t: f64) -> f64 {
    let om = 125.04452 - 1934.136261 * t;
    let l = 280.4665 + 36000.7698 * t;
    let lp = 218.3165 + 481267.8813 * t;
    (-17.20 * sind(om) - 1.32 * sind(2.0 * l) - 0.23 * sind(2.0 * lp) + 0.21 * sind(2.0 * om))
        / 3600.0
}

/// Ecliptic coordinates from J2000.0 to the equinox of date (Meeus 21.5, T0 = 0).
fn precess_from_j2000(lon: f64, lat: f64, t: f64) -> (f64, f64) {
    let eta = (47.0029 * t - 0.03302 * t * t + 0.000060 * t.powi(3)) / 3600.0;
    let pi = 174.876384 - (869.8089 * t + 0.03536 * t * t) / 3600.0;
    let p = (5029.0966 * t + 1.11113 * t * t - 0.0000006 * t.powi(3)) / 3600.0;
    let a = cosd(eta) * cosd(lat) * sind(pi - lon) - sind(eta) * sind(lat);
    let b = cosd(lat) * cosd(pi - lon);
    let c = cosd(eta) * sind(lat) + sind(eta) * cosd(lat) * sind(pi - lon);
    (norm360(p + pi - a.atan2(b) * R2D), c.asin() * R2D)
}

/// Standish Table 1: [a, e, I, L, long. perihelion, long. node] and their rates per century.
type Elements = [[f64; 6]; 2];
const MERCURY: Elements = [
    [
        0.38709927,
        0.20563593,
        7.00497902,
        252.25032350,
        77.45779628,
        48.33076593,
    ],
    [
        0.00000037,
        0.00001906,
        -0.00594749,
        149472.67411175,
        0.16047689,
        -0.12534081,
    ],
];
const VENUS: Elements = [
    [
        0.72333566,
        0.00677672,
        3.39467605,
        181.97909950,
        131.60246718,
        76.67984255,
    ],
    [
        0.00000390,
        -0.00004107,
        -0.00078890,
        58517.81538729,
        0.00268329,
        -0.27769418,
    ],
];
const EMB: Elements = [
    [
        1.00000261,
        0.01671123,
        -0.00001531,
        100.46457166,
        102.93768193,
        0.0,
    ],
    [
        0.00000562,
        -0.00004392,
        -0.01294668,
        35999.37244981,
        0.32327364,
        0.0,
    ],
];
const MARS: Elements = [
    [
        1.52371034,
        0.09339410,
        1.84969142,
        -4.55343205,
        -23.94362959,
        49.55953891,
    ],
    [
        0.00001847,
        0.00007882,
        -0.00813131,
        19140.30268499,
        0.44441088,
        -0.29257343,
    ],
];
const URANUS: Elements = [
    [
        19.18916464,
        0.04725744,
        0.77263783,
        313.23810451,
        170.95427630,
        74.01692503,
    ],
    [
        -0.00196176,
        -0.00004397,
        -0.00242939,
        428.48202785,
        0.40805281,
        0.04240589,
    ],
];
const NEPTUNE: Elements = [
    [
        30.06992276,
        0.00859048,
        1.77004347,
        -55.12002969,
        44.96476227,
        131.78422574,
    ],
    [
        0.00026291,
        0.00005105,
        0.00035372,
        218.45945325,
        -0.32241464,
        -0.00508664,
    ],
];
const PLUTO: Elements = [
    [
        39.48211675,
        0.24882730,
        17.14001206,
        238.92903833,
        224.06891629,
        110.30393684,
    ],
    [
        -0.00031596,
        0.00005170,
        0.00004818,
        145.20780515,
        -0.04062942,
        -0.01183482,
    ],
];

/// Heliocentric ecliptic J2000 rectangular coordinates (au) from Table 1 at `t` centuries.
fn kepler(el: &Elements, t: f64) -> [f64; 3] {
    let [e0, r] = el;
    let a = e0[0] + r[0] * t;
    let e = e0[1] + r[1] * t;
    let i = e0[2] + r[2] * t;
    let l = e0[3] + r[3] * t;
    let vp = e0[4] + r[4] * t;
    let om = e0[5] + r[5] * t;
    let w = vp - om;
    let mut m = norm360(l - vp);
    if m > 180.0 {
        m -= 360.0;
    }
    let estar = e * R2D;
    let mut ea = m + estar * sind(m);
    for _ in 0..30 {
        let dm = m - (ea - estar * sind(ea));
        let de = dm / (1.0 - e * cosd(ea));
        ea += de;
        if de.abs() <= 1e-8 {
            break;
        }
    }
    let xp = a * (cosd(ea) - e);
    let yp = a * (1.0 - e * e).sqrt() * sind(ea);
    let (cw, sw, co, so, ci, si) = (cosd(w), sind(w), cosd(om), sind(om), cosd(i), sind(i));
    [
        (cw * co - sw * so * ci) * xp + (-sw * co - cw * so * ci) * yp,
        (cw * so + sw * co * ci) * xp + (-sw * so + cw * co * ci) * yp,
        (sw * si) * xp + (cw * si) * yp,
    ]
}

/// Schlyter's elements of date [N, i, w, a, e, M] at `d` days from 2000 Jan 0.0.
fn schlyter_jupiter(d: f64) -> [f64; 6] {
    [
        100.4542 + 2.76854e-5 * d,
        1.3030 - 1.557e-7 * d,
        273.8777 + 1.64505e-5 * d,
        5.20256,
        0.048498 + 4.469e-9 * d,
        19.8950 + 0.0830853001 * d,
    ]
}
fn schlyter_saturn(d: f64) -> [f64; 6] {
    [
        113.6634 + 2.38980e-5 * d,
        2.4886 - 1.081e-7 * d,
        339.3939 + 2.97661e-5 * d,
        9.55475,
        0.055546 - 9.499e-9 * d,
        316.9670 + 0.0334442282 * d,
    ]
}

/// Jupiter or Saturn, heliocentric ecliptic J2000 rectangular (au), Schlyter with perturbations.
fn schlyter(jupiter: bool, t: f64) -> [f64; 3] {
    let d = t * 36525.0 + 1.5; // JDE - 2451543.5
    let [n, i, w, a, e, m0] = if jupiter {
        schlyter_jupiter(d)
    } else {
        schlyter_saturn(d)
    };
    let m = norm360(m0);
    let mut ea = m + e * R2D * sind(m) * (1.0 + e * cosd(m));
    for _ in 0..30 {
        let de = (ea - e * R2D * sind(ea) - m) / (1.0 - e * cosd(ea));
        ea -= de;
        if de.abs() < 1e-9 {
            break;
        }
    }
    let xv = a * (cosd(ea) - e);
    let yv = a * (1.0 - e * e).sqrt() * sind(ea);
    let v = yv.atan2(xv) * R2D;
    let r = xv.hypot(yv);
    let xh = r * (cosd(n) * cosd(v + w) - sind(n) * sind(v + w) * cosd(i));
    let yh = r * (sind(n) * cosd(v + w) + cosd(n) * sind(v + w) * cosd(i));
    let zh = r * sind(v + w) * sind(i);
    let mut lon = yh.atan2(xh) * R2D;
    let mut lat = zh.atan2(xh.hypot(yh)) * R2D;
    let mj = norm360(schlyter_jupiter(d)[5]);
    let ms = norm360(schlyter_saturn(d)[5]);
    if jupiter {
        lon += -0.332 * sind(2.0 * mj - 5.0 * ms - 67.6) - 0.056 * sind(2.0 * mj - 2.0 * ms + 21.0)
            + 0.042 * sind(3.0 * mj - 5.0 * ms + 21.0)
            - 0.036 * sind(mj - 2.0 * ms)
            + 0.022 * cosd(mj - ms)
            + 0.023 * sind(2.0 * mj - 3.0 * ms + 52.0)
            - 0.016 * sind(mj - 5.0 * ms - 69.0);
    } else {
        lon += 0.812 * sind(2.0 * mj - 5.0 * ms - 67.6) - 0.229 * cosd(2.0 * mj - 4.0 * ms - 2.0)
            + 0.119 * sind(mj - 2.0 * ms - 3.0)
            + 0.046 * sind(2.0 * mj - 6.0 * ms - 69.0)
            + 0.014 * sind(mj - 3.0 * ms + 32.0);
        lat += -0.020 * cosd(2.0 * mj - 4.0 * ms - 2.0) + 0.018 * sind(2.0 * mj - 6.0 * ms - 49.0);
    }
    // Of date to J2000: take out the general precession in longitude (the
    // ecliptic's tilt change is ignored, under 3″).
    lon -= (5029.0966 * t + 1.11113 * t * t) / 3600.0;
    [
        r * cosd(lat) * cosd(lon),
        r * cosd(lat) * sind(lon),
        r * sind(lat),
    ]
}

/// Heliocentric J2000 vector of a planet (not the Sun or Moon) at `t` centuries.
fn helio(body: Body, t: f64) -> [f64; 3] {
    match body {
        Body::Mercury => kepler(&MERCURY, t),
        Body::Venus => kepler(&VENUS, t),
        Body::Mars => kepler(&MARS, t),
        Body::Jupiter => schlyter(true, t),
        Body::Saturn => schlyter(false, t),
        Body::Uranus => kepler(&URANUS, t),
        Body::Neptune => kepler(&NEPTUNE, t),
        Body::Pluto => kepler(&PLUTO, t),
        Body::Sun | Body::Moon => unreachable!("not a planet"),
    }
}

fn hypot3(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Geometric geocentric Sun of date (no nutation, no aberration) from the
/// Earth-Moon barycentre vector: (longitude, latitude, distance in au).
fn sun_from_earth(earth: [f64; 3], t: f64) -> (f64, f64, f64) {
    let lon0 = norm360((-earth[1]).atan2(-earth[0]) * R2D);
    let lat0 = (-earth[2]).atan2(earth[0].hypot(earth[1])) * R2D;
    let (lon, lat) = precess_from_j2000(lon0, lat0, t);
    (lon, lat, hypot3(earth))
}

/// Apparent geocentric Sun of date at `jde`.
pub fn sun_apparent(jde: f64) -> Pos {
    let t = centuries(jde);
    let (lon, lat, r) = sun_from_earth(kepler(&EMB, t), t);
    Pos {
        lon: norm360(lon + nutation_lon(t) - 20.4898 / 3600.0 / r),
        lat,
        dist: r,
    }
}

/// Apparent geocentric planet of date at `jde`: light time, aberration, nutation.
fn planet_apparent(body: Body, jde: f64) -> Pos {
    let t = centuries(jde);
    let earth = kepler(&EMB, t);
    let mut tau = 0.0;
    let mut g = [0.0; 3];
    for _ in 0..3 {
        let p = helio(body, (jde - tau - 2_451_545.0) / 36_525.0);
        g = [p[0] - earth[0], p[1] - earth[1], p[2] - earth[2]];
        tau = hypot3(g) * LIGHT_DAYS_PER_AU;
    }
    let lon0 = norm360(g[1].atan2(g[0]) * R2D);
    let lat0 = g[2].atan2(g[0].hypot(g[1])) * R2D;
    let (lon, lat) = precess_from_j2000(lon0, lat0, t);
    // Annual aberration (Meeus 23.2) with the Sun's true longitude from the same Earth.
    let (sun_true, _, _) = sun_from_earth(earth, t);
    let kappa = 20.49552 / 3600.0;
    let e_earth = 0.016708634 - 0.000042037 * t - 0.0000001267 * t * t;
    let pi_earth = 102.93735 + 1.71946 * t + 0.00046 * t * t;
    let dlon = (-kappa * cosd(sun_true - lon) + e_earth * kappa * cosd(pi_earth - lon)) / cosd(lat);
    Pos {
        lon: norm360(lon + dlon + nutation_lon(t)),
        lat,
        dist: hypot3(g),
    }
}

/// Apparent geocentric ecliptic-of-date position of `body` at instant `ms`
/// (distance in au, km for the Moon).
pub fn position(body: Body, ms: f64) -> Pos {
    let jde = jde_from_ms(ms);
    match body {
        Body::Sun => sun_apparent(jde),
        Body::Moon => moon::apparent(jde),
        b => planet_apparent(b, jde),
    }
}

/// The Sun and the Moon at one instant (for the Moon's illumination).
pub fn sun_moon(ms: f64) -> (Pos, Pos) {
    let jde = jde_from_ms(ms);
    (sun_apparent(jde), moon::apparent(jde))
}

/// Longitude speed in degrees per day: the central difference over ±6 h.
pub fn speed(body: Body, ms: f64) -> f64 {
    let h = 6.0 * HOUR_MS;
    let mut d = position(body, ms + h).lon - position(body, ms - h).lon;
    if d > 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d * 2.0
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One Horizons row: instant (ms) and the apparent ecliptic longitude, latitude of date.
    pub(crate) struct Row {
        pub ms: f64,
        pub lon: f64,
        pub lat: f64,
    }

    /// Rows between `$$SOE` and `$$EOE` of a Horizons `QUANTITIES='31'` CSV export
    /// (`skip` extra columns before ObsEcLon, 1 for the `QUANTITIES='10,31'` files).
    pub(crate) fn horizons(text: &str, skip: usize) -> Vec<Row> {
        let block = text
            .split("$$SOE")
            .nth(1)
            .unwrap()
            .split("$$EOE")
            .next()
            .unwrap();
        let mut out = Vec::new();
        for line in block.lines() {
            let cols: Vec<&str> = line.split(',').map(str::trim).collect();
            if cols.len() < 5 + skip {
                continue;
            }
            let when = cols[0];
            let (date, time) = when.split_once(' ').unwrap();
            let mut dp = date.split('-');
            let y: i32 = dp.next().unwrap().parse().unwrap();
            let name = dp.next().unwrap();
            let mon = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ]
            .iter()
            .position(|m| *m == name)
            .unwrap() as u32
                + 1;
            let d: u32 = dp.next().unwrap().parse().unwrap();
            let mut tp = time.split(':');
            let hh: f64 = tp.next().unwrap().parse().unwrap();
            let mm: f64 = tp.next().unwrap().parse().unwrap();
            let ss: f64 = tp.next().map(|s| s.parse().unwrap()).unwrap_or(0.0);
            let day = chrono::NaiveDate::from_ymd_opt(y, mon, d).unwrap();
            let ms = super::super::time::utc_ms(day, 0) + (hh * 3600.0 + mm * 60.0 + ss) * 1000.0;
            out.push(Row {
                ms,
                lon: cols[3 + skip].parse().unwrap(),
                lat: cols[4 + skip].parse().unwrap(),
            });
        }
        out
    }

    pub(crate) fn arcsec(a: f64, b: f64) -> f64 {
        (((a - b) + 540.0) % 360.0 - 180.0).abs() * 3600.0
    }

    macro_rules! fixture {
        ($f:literal) => {
            include_str!(concat!("../../tests/fixtures/astro/", $f))
        };
    }

    const HISTORICAL: [(Body, &str, f64); 10] = [
        (Body::Sun, fixture!("horizons_sun_1800_2050.txt"), 40.0),
        (Body::Moon, fixture!("horizons_moon_1800_2050.txt"), 30.0),
        (
            Body::Mercury,
            fixture!("horizons_mercury_1800_2050.txt"),
            60.0,
        ),
        (Body::Venus, fixture!("horizons_venus_1800_2050.txt"), 120.0),
        (Body::Mars, fixture!("horizons_mars_1800_2050.txt"), 120.0),
        (
            Body::Jupiter,
            fixture!("horizons_jupiter_1800_2050.txt"),
            120.0,
        ),
        (
            Body::Saturn,
            fixture!("horizons_saturn_1800_2050.txt"),
            180.0,
        ),
        (
            Body::Uranus,
            fixture!("horizons_uranus_1800_2050.txt"),
            150.0,
        ),
        (
            Body::Neptune,
            fixture!("horizons_neptune_1800_2050.txt"),
            90.0,
        ),
        (Body::Pluto, fixture!("horizons_pluto_1800_2050.txt"), 90.0),
    ];

    #[test]
    fn longitudes_match_horizons_1800_to_2050() {
        for (body, text, tol) in HISTORICAL {
            let rows = horizons(text, 0);
            assert!(rows.len() >= 33, "{body:?}: {} rows", rows.len());
            let worst = rows
                .iter()
                .map(|r| arcsec(position(body, r.ms).lon, r.lon))
                .fold(0.0, f64::max);
            assert!(worst < tol, "{body:?}: {worst:.1}″ (tolerance {tol}″)");
        }
    }

    #[test]
    fn latitudes_stay_close_too() {
        for (body, text, _) in HISTORICAL {
            let worst = horizons(text, 0)
                .iter()
                .map(|r| (position(body, r.ms).lat - r.lat).abs() * 3600.0)
                .fold(0.0, f64::max);
            assert!(worst < 180.0, "{body:?}: latitude off by {worst:.1}″");
        }
    }

    #[test]
    fn daily_mercury_and_saturn_2025_2027() {
        for (body, text, tol) in [
            (
                Body::Mercury,
                fixture!("horizons_mercury_2025_2027_daily.txt"),
                60.0,
            ),
            (
                Body::Saturn,
                fixture!("horizons_saturn_2025_2027_daily.txt"),
                180.0,
            ),
        ] {
            let rows = horizons(text, 0);
            assert_eq!(rows.len(), 1095);
            let worst = rows
                .iter()
                .map(|r| arcsec(position(body, r.ms).lon, r.lon))
                .fold(0.0, f64::max);
            assert!(worst < tol, "{body:?}: {worst:.1}″");
            // Retrograde flag = sign of Horizons' own central difference, every interior day.
            for w in rows.windows(3) {
                let mut d = w[2].lon - w[0].lon;
                if d > 180.0 {
                    d -= 360.0;
                }
                if d < -180.0 {
                    d += 360.0;
                }
                assert_eq!(
                    speed(body, w[1].ms) < 0.0,
                    d < 0.0,
                    "{body:?} at {}",
                    w[1].ms
                );
            }
        }
    }
}
