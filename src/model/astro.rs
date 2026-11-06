//! `idx.astro.v1`: the sky over IDX on a date, the Javanese weton and Chinese
//! shio, IHSG's own chart and, on request, a backtest of IHSG's daily returns
//! against those factors. Positions are astronomy computed on this machine
//! (`crate::astro`); their meanings are tradition, which v1 leaves out.

use chrono::{DateTime, FixedOffset, NaiveDate};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{IdxError, Result};
use crate::model::{Period, Symbol};

/// A body of the sky, in the order the payload lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Body {
    Sun,
    Moon,
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
    Pluto,
}

impl Body {
    pub const ALL: [Body; 10] = [
        Body::Sun,
        Body::Moon,
        Body::Mercury,
        Body::Venus,
        Body::Mars,
        Body::Jupiter,
        Body::Saturn,
        Body::Uranus,
        Body::Neptune,
        Body::Pluto,
    ];

    /// Capitalised English name, for tables.
    pub fn name(self) -> &'static str {
        match self {
            Body::Sun => "Sun",
            Body::Moon => "Moon",
            Body::Mercury => "Mercury",
            Body::Venus => "Venus",
            Body::Mars => "Mars",
            Body::Jupiter => "Jupiter",
            Body::Saturn => "Saturn",
            Body::Uranus => "Uranus",
            Body::Neptune => "Neptune",
            Body::Pluto => "Pluto",
        }
    }

    /// The Sun and the Moon never move backwards along the ecliptic.
    pub fn can_retrograde(self) -> bool {
        !matches!(self, Body::Sun | Body::Moon)
    }
}

/// Tropical zodiac sign: 30° of ecliptic longitude each, Aries from 0°.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZodiacSign {
    Aries,
    Taurus,
    Gemini,
    Cancer,
    Leo,
    Virgo,
    Libra,
    Scorpio,
    Sagittarius,
    Capricorn,
    Aquarius,
    Pisces,
}

impl ZodiacSign {
    pub const ALL: [ZodiacSign; 12] = [
        ZodiacSign::Aries,
        ZodiacSign::Taurus,
        ZodiacSign::Gemini,
        ZodiacSign::Cancer,
        ZodiacSign::Leo,
        ZodiacSign::Virgo,
        ZodiacSign::Libra,
        ZodiacSign::Scorpio,
        ZodiacSign::Sagittarius,
        ZodiacSign::Capricorn,
        ZodiacSign::Aquarius,
        ZodiacSign::Pisces,
    ];

    /// The sign holding ecliptic longitude `lon` (any value; reduced to [0, 360)).
    pub fn of(lon: f64) -> ZodiacSign {
        let l = ((lon % 360.0) + 360.0) % 360.0;
        ZodiacSign::ALL[((l / 30.0).floor() as usize).min(11)]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ZodiacSign::Aries => "aries",
            ZodiacSign::Taurus => "taurus",
            ZodiacSign::Gemini => "gemini",
            ZodiacSign::Cancer => "cancer",
            ZodiacSign::Leo => "leo",
            ZodiacSign::Virgo => "virgo",
            ZodiacSign::Libra => "libra",
            ZodiacSign::Scorpio => "scorpio",
            ZodiacSign::Sagittarius => "sagittarius",
            ZodiacSign::Capricorn => "capricorn",
            ZodiacSign::Aquarius => "aquarius",
            ZodiacSign::Pisces => "pisces",
        }
    }

    /// Capitalised name, for tables.
    pub fn name(self) -> &'static str {
        match self {
            ZodiacSign::Aries => "Aries",
            ZodiacSign::Taurus => "Taurus",
            ZodiacSign::Gemini => "Gemini",
            ZodiacSign::Cancer => "Cancer",
            ZodiacSign::Leo => "Leo",
            ZodiacSign::Virgo => "Virgo",
            ZodiacSign::Libra => "Libra",
            ZodiacSign::Scorpio => "Scorpio",
            ZodiacSign::Sagittarius => "Sagittarius",
            ZodiacSign::Capricorn => "Capricorn",
            ZodiacSign::Aquarius => "Aquarius",
            ZodiacSign::Pisces => "Pisces",
        }
    }
}

/// Eight equal 45° sectors of the Moon-Sun elongation, centred on 0°, 45°, ..., 315°.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MoonPhase {
    NewMoon,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    FullMoon,
    WaningGibbous,
    LastQuarter,
    WaningCrescent,
}

impl MoonPhase {
    pub const ALL: [MoonPhase; 8] = [
        MoonPhase::NewMoon,
        MoonPhase::WaxingCrescent,
        MoonPhase::FirstQuarter,
        MoonPhase::WaxingGibbous,
        MoonPhase::FullMoon,
        MoonPhase::WaningGibbous,
        MoonPhase::LastQuarter,
        MoonPhase::WaningCrescent,
    ];

    /// The sector holding elongation `elong` (degrees).
    pub fn of(elong: f64) -> MoonPhase {
        let e = ((elong + 22.5) % 360.0 + 360.0) % 360.0;
        MoonPhase::ALL[((e / 45.0).floor() as usize) % 8]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            MoonPhase::NewMoon => "new_moon",
            MoonPhase::WaxingCrescent => "waxing_crescent",
            MoonPhase::FirstQuarter => "first_quarter",
            MoonPhase::WaxingGibbous => "waxing_gibbous",
            MoonPhase::FullMoon => "full_moon",
            MoonPhase::WaningGibbous => "waning_gibbous",
            MoonPhase::LastQuarter => "last_quarter",
            MoonPhase::WaningCrescent => "waning_crescent",
        }
    }

    /// Lower-case words, for tables ("waning crescent").
    pub fn label(self) -> &'static str {
        match self {
            MoonPhase::NewMoon => "new moon",
            MoonPhase::WaxingCrescent => "waxing crescent",
            MoonPhase::FirstQuarter => "first quarter",
            MoonPhase::WaxingGibbous => "waxing gibbous",
            MoonPhase::FullMoon => "full moon",
            MoonPhase::WaningGibbous => "waning gibbous",
            MoonPhase::LastQuarter => "last quarter",
            MoonPhase::WaningCrescent => "waning crescent",
        }
    }
}

/// Something that happens at an instant: an exact lunar phase, a station or a sign ingress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AstroEventKind {
    NewMoon,
    FirstQuarter,
    FullMoon,
    LastQuarter,
    /// The body turns retrograde.
    StationRetrograde,
    /// The body turns direct again.
    StationDirect,
    /// The body enters `sign`.
    Ingress,
}

/// Major aspects: the angle between two longitudes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Aspect {
    Conjunction,
    Sextile,
    Square,
    Trine,
    Opposition,
}

impl Aspect {
    pub const ALL: [Aspect; 5] = [
        Aspect::Conjunction,
        Aspect::Sextile,
        Aspect::Square,
        Aspect::Trine,
        Aspect::Opposition,
    ];

    pub fn angle(self) -> f64 {
        match self {
            Aspect::Conjunction => 0.0,
            Aspect::Sextile => 60.0,
            Aspect::Square => 90.0,
            Aspect::Trine => 120.0,
            Aspect::Opposition => 180.0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Aspect::Conjunction => "conjunction",
            Aspect::Sextile => "sextile",
            Aspect::Square => "square",
            Aspect::Trine => "trine",
            Aspect::Opposition => "opposition",
        }
    }
}

/// Which IHSG chart the transits refer to (`--natal`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum NatalChoice {
    /// 1 April 1983, the day IHSG was first published.
    Launch,
    /// 10 August 1982, the base date (value 100).
    Base,
    /// No natal chart.
    None,
}

impl std::str::FromStr for NatalChoice {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <NatalChoice as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            IdxError::Usage(format!(
                "invalid natal chart {s:?}; use launch, base or none"
            ))
        })
    }
}

/// The IHSG chart in the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IhsgChart {
    /// 1983-04-01, the day IHSG was first published.
    Launch,
    /// 1982-08-10, the base date (value 100).
    Base,
}

impl IhsgChart {
    pub fn date(self) -> NaiveDate {
        match self {
            IhsgChart::Launch => NaiveDate::from_ymd_opt(1983, 4, 1),
            IhsgChart::Base => NaiveDate::from_ymd_opt(1982, 8, 10),
        }
        .expect("valid date")
    }

    pub fn as_str(self) -> &'static str {
        match self {
            IhsgChart::Launch => "launch",
            IhsgChart::Base => "base",
        }
    }
}

/// Look-back of the returns backtest (`--period`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
pub enum AstroPeriod {
    #[value(name = "1y")]
    #[serde(rename = "1y")]
    Y1,
    #[value(name = "2y")]
    #[serde(rename = "2y")]
    Y2,
    #[value(name = "5y")]
    #[serde(rename = "5y")]
    Y5,
    #[value(name = "10y")]
    #[serde(rename = "10y")]
    Y10,
    /// All of the source's history (^JKSE on Yahoo: from 1990-04-06).
    #[value(name = "max")]
    #[serde(rename = "max")]
    Max,
}

impl AstroPeriod {
    pub fn as_str(self) -> &'static str {
        match self {
            AstroPeriod::Y1 => "1y",
            AstroPeriod::Y2 => "2y",
            AstroPeriod::Y5 => "5y",
            AstroPeriod::Y10 => "10y",
            AstroPeriod::Max => "max",
        }
    }

    /// The history look-back it asks the provider for.
    pub fn history_period(self) -> Period {
        match self {
            AstroPeriod::Y1 => Period::Y1,
            AstroPeriod::Y2 => Period::Y2,
            AstroPeriod::Y5 => Period::Y5,
            AstroPeriod::Y10 => Period::Y10,
            AstroPeriod::Max => Period::Max,
        }
    }
}

impl std::str::FromStr for AstroPeriod {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <AstroPeriod as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            IdxError::Usage(format!(
                "invalid backtest period {s:?}; use 1y, 2y, 5y, 10y or max"
            ))
        })
    }
}

impl std::fmt::Display for AstroPeriod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `market astro`: the payload of `idx.astro.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Astro {
    /// The date asked for (Jakarta calendar date).
    pub date: NaiveDate,
    /// The instant positions are computed for: 16:00 WIB (UTC+7) on `date`, the IDX close.
    pub at: DateTime<FixedOffset>,
    /// Sun, Moon, Mercury, Venus, Mars, Jupiter, Saturn, Uranus, Neptune, Pluto, in that
    /// order. The only top-level list, so `-o csv` prints it.
    pub bodies: Vec<AstroBody>,
    pub moon: MoonState,
    pub mercury: MercuryState,
    pub weton: Weton,
    /// `null` before Imlek 1924 and after 2050 (the range checked against published Imlek dates).
    pub shio: Option<Shio>,
    /// Exact lunar phases, stations and sign ingresses from 00:00 WIB on `date` for `--days` days.
    pub upcoming: AstroWindow,
    /// IHSG's chart and today's aspects to it; `null` with `--natal none`.
    pub natal: Option<NatalChart>,
    /// `null` unless the backtest was asked for.
    pub backtest: Option<AstroBacktest>,
    /// How the numbers were computed, with their checked accuracy.
    pub method: String,
    /// Fixed sentence: traditional astrology, not a forecast or investment advice.
    pub disclaimer: String,
}

/// One body's apparent geocentric position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AstroBody {
    pub body: Body,
    /// Apparent geocentric ecliptic longitude of date (tropical zodiac), degrees [0, 360), 2 decimals.
    pub longitude: f64,
    /// Sign of the rounded longitude.
    pub sign: ZodiacSign,
    /// Degrees into the sign [0, 30), 2 decimals.
    pub degree: f64,
    /// Change in longitude, degrees per day, 3 decimals; negative while retrograde.
    pub speed: f64,
    /// Longitude decreasing; always false for the Sun and Moon.
    pub retrograde: bool,
}

/// The Moon's phase and the lunations around `at`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MoonState {
    /// Sector of the elongation: new_moon | waxing_crescent | first_quarter | waxing_gibbous |
    /// full_moon | waning_gibbous | last_quarter | waning_crescent (45° each, centred on the
    /// exact phases, so full_moon spans about 3.7 days).
    pub phase: MoonPhase,
    /// Moon minus Sun longitude, degrees [0, 360), 2 decimals.
    pub elongation: f64,
    /// Illuminated fraction of the disc 0-1, 3 decimals.
    pub illumination: f64,
    /// Days since the previous new moon, 1 decimal.
    pub age_days: f64,
    pub previous_new_moon: DateTime<FixedOffset>,
    pub next_new_moon: DateTime<FixedOffset>,
    pub next_full_moon: DateTime<FixedOffset>,
}

/// Mercury retrograde, the factor traders ask about most.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MercuryState {
    pub retrograde: bool,
    /// Station retrograde that began the current period; `null` when direct.
    pub retrograde_since: Option<DateTime<FixedOffset>>,
    /// Station direct that ends it; `null` when direct.
    pub retrograde_until: Option<DateTime<FixedOffset>>,
    /// The next period that starts after `at`.
    pub next_retrograde: Option<RetrogradePeriod>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RetrogradePeriod {
    /// Station retrograde.
    pub from: DateTime<FixedOffset>,
    /// Station direct; `null` if it falls past the search window.
    pub until: Option<DateTime<FixedOffset>>,
}

/// Javanese weton: the seven-day week with the five-day pasaran, and their neptu.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Weton {
    /// Senin, Selasa, Rabu, Kamis, Jumat, Sabtu or Minggu.
    pub day: String,
    /// The same day in English.
    pub day_en: String,
    /// Legi, Pahing, Pon, Wage or Kliwon.
    pub pasaran: String,
    pub neptu_day: u8,
    pub neptu_pasaran: u8,
    /// neptu_day + neptu_pasaran (7-18).
    pub neptu: u8,
}

/// Chinese zodiac year (shio) holding the date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Shio {
    /// rat, ox, tiger, rabbit, dragon, snake, horse, goat, monkey, rooster, dog or pig.
    pub animal: String,
    /// The animal in Indonesian (Tikus, Kerbau, Macan, Kelinci, Naga, Ular, Kuda, Kambing,
    /// Monyet, Ayam, Anjing, Babi).
    pub animal_id: String,
    /// Element of the year's heavenly stem: wood, fire, earth, metal or water.
    pub element: String,
    /// Imlek that began this Chinese year (China Standard Time date).
    pub year_start: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AstroWindow {
    pub from: DateTime<FixedOffset>,
    pub to: DateTime<FixedOffset>,
    /// Oldest first.
    pub events: Vec<AstroEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AstroEvent {
    /// Rounded to the minute, WIB.
    pub at: DateTime<FixedOffset>,
    pub kind: AstroEventKind,
    /// `moon` for the four phases.
    pub body: Body,
    /// Sign the body is in (the sign entered, for an ingress).
    pub sign: ZodiacSign,
}

/// IHSG's own chart: the sky at noon WIB on the chosen date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NatalChart {
    pub chart: IhsgChart,
    /// 1983-04-01 (launch) or 1982-08-10 (base). Both from secondary sources (Antara,
    /// Bisnis.com, CEIC); not yet confirmed against an IDX publication.
    pub date: NaiveDate,
    /// 12:00 WIB: the time of day is unknown, so there are no houses or ascendant.
    pub at: DateTime<FixedOffset>,
    pub bodies: Vec<NatalBody>,
    /// Aspects of the Sun..Pluto at `at` of the payload (the Moon left out) to the natal
    /// Sun..Pluto (the Moon left out) within 2°, closest first.
    pub transits: Vec<Transit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NatalBody {
    pub body: Body,
    /// Degrees [0, 360), 2 decimals.
    pub longitude: f64,
    pub sign: ZodiacSign,
    /// Degrees into the sign, 2 decimals.
    pub degree: f64,
    /// Degrees per day, 3 decimals.
    pub speed: f64,
    pub retrograde: bool,
    /// True for the Moon only: it moves about 13° a day and the time of day is unknown.
    pub approximate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Transit {
    /// Body in today's sky.
    pub transit: Body,
    pub aspect: Aspect,
    /// Body in IHSG's chart.
    pub natal: Body,
    /// Degrees from exact, 2 decimals.
    pub orb: f64,
    /// The orb is shrinking (smaller a day later).
    pub applying: bool,
}

/// IHSG daily returns grouped by lunar phase, Mercury retrograde, pasaran and Sun sign.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AstroBacktest {
    pub symbol: Symbol,
    pub period: AstroPeriod,
    /// Session date of the first return.
    pub from: NaiveDate,
    /// Session date of the last return.
    pub to: NaiveDate,
    /// Close-to-close daily returns used.
    pub returns: usize,
    /// Mean daily return over all sessions, percent, 4 decimals.
    pub mean_pct: f64,
    /// Median daily return, percent, 4 decimals.
    pub median_pct: f64,
    /// Share of sessions that closed up, percent, 2 decimals.
    pub win_rate_pct: f64,
    /// Groups tested, each against all other sessions.
    pub tests: usize,
    /// Family-wise significance level, 0.05.
    pub alpha: f64,
    /// alpha / tests (Bonferroni), 6 decimals.
    pub bonferroni_alpha: f64,
    /// Groups whose p-value is below the Bonferroni threshold.
    pub significant: usize,
    /// moon_phase, mercury, pasaran, sun_sign, in that order.
    pub factors: Vec<BacktestFactor>,
    /// How the returns were built and tested, with the caveats.
    pub method: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BacktestFactor {
    /// moon_phase | mercury | pasaran | sun_sign
    pub factor: String,
    pub groups: Vec<BacktestGroup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BacktestGroup {
    /// new_moon .. waning_crescent | retrograde, direct | legi, pahing, pon, wage, kliwon |
    /// aries .. pisces
    pub group: String,
    /// Sessions in the group.
    pub n: usize,
    /// Mean daily return, percent, 4 decimals; `null` when n = 0.
    pub mean_pct: Option<f64>,
    /// Median daily return, percent, 4 decimals; `null` when n = 0.
    pub median_pct: Option<f64>,
    /// Sample standard deviation, percent, 4 decimals; `null` when n < 2.
    pub stdev_pct: Option<f64>,
    /// Share of the group's sessions that closed up, percent, 2 decimals.
    pub win_rate_pct: Option<f64>,
    /// Welch's t of the group mean against all other sessions, 3 decimals; `null` with fewer
    /// than 2 sessions on either side.
    pub t_stat: Option<f64>,
    /// Two-sided p from Student's t with Welch-Satterthwaite degrees of freedom, 6 decimals.
    pub p_value: Option<f64>,
    /// p_value below the Bonferroni threshold.
    pub significant: bool,
}
