# Astro fixtures

Reference data for `market astro` (`src/astro/`, `src/analysis/astro.rs`) and
its web port (`web/src/astro.js`, `web/test/astro.test.mjs`). Recorded
2026-11-06 between 06:50 and 07:40 WIB with plain GET requests, no
credentials. The upstream files are byte for byte what the services
returned; the two kinds marked below are built by idx.

## JPL Horizons (NASA/JPL Solar System Dynamics)

`https://ssd.jpl.nasa.gov/api/horizons.api` with `format=text`,
`EPHEM_TYPE='OBSERVER'`, `CENTER='500@399'` (geocentric), `QUANTITIES='31'`
(observer ecliptic-of-date longitude and latitude of the apparent position:
light time, gravitational deflection, stellar aberration) and
`CSV_FORMAT='YES'`. Ephemeris DE441.

| file | request | notes |
| --- | --- | --- |
| `horizons_<body>_1800_2050.txt` (10 bodies) | `COMMAND` 10 (Sun), 301, 199, 299, 499, 599, 699, 799, 899, 999; `TLIST_TYPE='JD'` with 33 instants from 1800 to 2050 | Neptune and Pluto start at 1800-01-03 (Horizons has no ephemeris for them before 1800-01-01/02 23:59 UT); the list includes 1945-08-17, 1982-08-10 05:00 UT and 1983-04-01 05:00 UT (IHSG's base and launch dates at noon WIB) |
| `horizons_mercury_2025_2027_daily.txt`, `horizons_saturn_2025_2027_daily.txt` | `START_TIME='2025-01-01'`, `STOP_TIME='2027-12-31'`, `STEP_SIZE='1 d'` (00:00 UT) | 1,095 rows each: Mercury's 18 stations and Saturn's stations and ingresses |
| `horizons_moon_2026_illum.txt` | `COMMAND='301'`, 2026-01-01 09:00 to 2026-12-31 09:00 UT, `STEP_SIZE='1 d'`, `QUANTITIES='10,31'` | `Illu%` (illuminated fraction) before the longitude and latitude |

## U.S. Naval Observatory

`https://aa.usno.navy.mil/api/moon/phases/year?year=Y` for 1982, 2025, 2026
and 2027: `usno_moon_phases_<Y>.json`, 198 phases in all, times in UT to the
minute.

## Imlek dates (constructed by idx)

`imlek_1924_2050_constructed.txt` lists the first day of each Chinese year,
1924-2050 (127 dates), copied by hand from two published sources:

- 1924-2043: the start dates in the table of the English Wikipedia article
  "Chinese zodiac" (read 2026-11-06);
- 2044-2050: the "1st Lunar Month" line of the Hong Kong Observatory's
  Gregorian-Lunar calendar conversion tables,
  `https://www.hko.gov.hk/en/gts/time/calendar/text/files/T<YEAR>e.txt`.

## Golden payloads (constructed by idx)

`golden_sky_2026-11-06.json` is the `data` of
`idx -o json market astro --date 2026-11-06` (30 days, launch chart), and
`golden_backtest_jkse_2y.json` the backtest over
`../yahoo/chart_jkse_2y_1d.json`. Both are produced by the Rust code; the Rust
tests and the web tests each assert their own output equals them, which keeps
the CLI and the web terminal identical. `IDX_WRITE_GOLDEN=1 cargo test astro`
rewrites them after a deliberate change.

## Weton sources (in the tests, no file)

The 16 weton dates in `src/astro/calendar.rs` and `web/test/astro.test.mjs`
come from published Javanese calendars, read 2026-11-06: 1633-07-08 Jumat
Legi (1 Sura 1555, Kompas TV, detik.com); 1945-08-17 Jumat Legi (Soekarno's
own words, via goodnewsfromindonesia.id and republika.co.id); 2026 dates from
metrotvnews.com, kalenderku.id, kabar24.bisnis.com, katadata.co.id and
pdiperjuanganbali.id. Neptu values follow katadata.co.id (citing primbon.com)
and kalenderku.id.
