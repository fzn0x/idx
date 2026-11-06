//! Bandarmology rules over a broker summary. No network.
//!
//! The summary lists the top 10 buyers and top 10 sellers, ranked by lots.
//! For N in 1, 3, 5 and 10 the top N buyers' value is set against the top N
//! sellers' value; the difference (the bandar value) over the range's total value is
//! the reading:
//!
//! | net of top N, % of value | signal           |
//! |--------------------------|------------------|
//! | >= +15                   | big_accumulation |
//! | +5 to +15                | accumulation     |
//! | -5 to +5                 | neutral          |
//! | -15 to -5                | distribution     |
//! | <= -15                   | big_distribution |
//!
//! Thresholds were set against the top 40 IDX stocks by value in October
//! 2026: on a single session about half read neutral and one in five big.
//! The phase reads the signal against the price over the same range
//! (within 2 % counts as flat), following the usual bandarmology reading:
//! accumulation while the price stalls is quiet buying, distribution while
//! it holds up is quiet selling. Heuristics, not advice.

use std::cmp::Ordering;

use chrono::NaiveDate;

use crate::analysis::screen::{compare, parse_number, split_clause, Op};
use crate::error::{IdxError, Result};
use crate::model::{
    BandarDaily, BandarDay, BandarPhase, BandarReport, BandarScreenRow, BandarSignal, BandarTier,
    BrokerFlow, BrokerSummary,
};
use crate::output::format as fmt;

/// Tiers every report carries.
pub const TIERS: [u32; 4] = [1, 3, 5, 10];
/// Tier behind the headline signal unless asked otherwise.
pub const DEFAULT_TOP: u32 = 5;
/// Net share of value, percent, from which a tier reads accumulation (or, negative, distribution).
pub const ACCUMULATION_PCT: f64 = 5.0;
/// Net share of value, percent, from which the reading is big.
pub const BIG_PCT: f64 = 15.0;
/// Price moves within this many percent count as flat for the phase.
pub const FLAT_PCT: f64 = 2.0;

pub fn check_top(top: u32) -> Result<u32> {
    if TIERS.contains(&top) {
        Ok(top)
    } else {
        Err(IdxError::Usage(format!(
            "top must be 1, 3, 5 or 10 (got {top})"
        )))
    }
}

pub fn signal(net_pct: Option<f64>) -> BandarSignal {
    match net_pct {
        Some(p) if p >= BIG_PCT => BandarSignal::BigAccumulation,
        Some(p) if p >= ACCUMULATION_PCT => BandarSignal::Accumulation,
        Some(p) if p <= -BIG_PCT => BandarSignal::BigDistribution,
        Some(p) if p <= -ACCUMULATION_PCT => BandarSignal::Distribution,
        _ => BandarSignal::Neutral,
    }
}

/// The signal against the price move; `null` when a signal has no price move to read.
pub fn phase(signal: BandarSignal, price_change_pct: Option<f64>) -> Option<BandarPhase> {
    if signal == BandarSignal::Neutral {
        return Some(BandarPhase::Neutral);
    }
    let chg = price_change_pct?;
    Some(if signal.is_accumulation() {
        if chg > FLAT_PCT {
            BandarPhase::Markup
        } else {
            BandarPhase::QuietAccumulation
        }
    } else if chg < -FLAT_PCT {
        BandarPhase::Markdown
    } else {
        BandarPhase::QuietDistribution
    })
}

fn pct_of(part: f64, whole: Option<f64>) -> Option<f64> {
    match whole {
        Some(w) if w > 0.0 => Some(part / w * 100.0),
        _ => None,
    }
}

/// Volume-weighted average over brokers: total value over total shares, the
/// shares taken from each broker's value and its exact published average
/// (lots of a million and more are rounded, averages never are).
fn vwap(flows: &[BrokerFlow]) -> Option<f64> {
    let (mut value, mut shares) = (0.0, 0.0);
    for f in flows {
        if let (Some(v), Some(a)) = (f.value, f.average_price) {
            if a > 0.0 && v > 0.0 {
                value += v;
                shares += v / a;
            }
        }
    }
    (shares > 0.0).then(|| value / shares)
}

pub fn tier(s: &BrokerSummary, top: u32) -> BandarTier {
    let n = top as usize;
    let buyers = &s.buyers[..n.min(s.buyers.len())];
    let sellers = &s.sellers[..n.min(s.sellers.len())];
    let sum = |fl: &[BrokerFlow], f: fn(&BrokerFlow) -> Option<f64>| -> f64 {
        fl.iter().filter_map(f).sum()
    };
    let buy_value = sum(buyers, |f| f.value);
    let sell_value = sum(sellers, |f| f.value);
    let buy_lots = sum(buyers, |f| f.lots);
    let sell_lots = sum(sellers, |f| f.lots);
    let net_value = buy_value - sell_value;
    let net_pct = pct_of(net_value, s.total_value);
    BandarTier {
        top,
        buy_value,
        sell_value,
        net_value,
        buy_lots,
        sell_lots,
        net_lots: buy_lots - sell_lots,
        net_pct,
        buy_share_pct: pct_of(buy_value, s.total_value),
        sell_share_pct: pct_of(sell_value, s.total_value),
        buy_average: vwap(buyers),
        sell_average: vwap(sellers),
        signal: signal(net_pct),
    }
}

/// Price change in percent from `base` to `price`.
pub fn change_pct(base: Option<f64>, price: Option<f64>) -> Option<f64> {
    match (base, price) {
        (Some(b), Some(p)) if b > 0.0 => Some((p / b - 1.0) * 100.0),
        _ => None,
    }
}

fn idr(v: f64) -> String {
    fmt::compact(Some(v))
}

fn pct1(v: f64) -> String {
    format!("{}{:.1}%", if v > 0.0 { "+" } else { "" }, v)
}

fn words(s: BandarSignal) -> &'static str {
    match s {
        BandarSignal::BigAccumulation => "big accumulation",
        BandarSignal::Accumulation => "accumulation",
        BandarSignal::Neutral => "neutral",
        BandarSignal::Distribution => "distribution",
        BandarSignal::BigDistribution => "big distribution",
    }
}

fn threshold(s: BandarSignal) -> String {
    match s {
        BandarSignal::BigAccumulation => format!("at +{BIG_PCT:.0}% or more"),
        BandarSignal::Accumulation => format!("+{ACCUMULATION_PCT:.0}% to +{BIG_PCT:.0}%"),
        BandarSignal::Neutral => format!("within ±{ACCUMULATION_PCT:.0}%"),
        BandarSignal::Distribution => format!("-{ACCUMULATION_PCT:.0}% to -{BIG_PCT:.0}%"),
        BandarSignal::BigDistribution => format!("at -{BIG_PCT:.0}% or less"),
    }
}

fn reasons(r: &BandarReport, head: &BandarTier) -> Vec<String> {
    let mut out = Vec::new();
    let n = r.top;
    let Some(total) = r.total_value.filter(|v| *v > 0.0) else {
        out.push(format!("No trades from {} to {}.", r.from, r.to));
        return out;
    };
    out.push(format!(
        "Top {n} buyers bought {}, top {n} sellers sold {}: net {}, {} of the {} traded ({}, {}).",
        idr(head.buy_value),
        idr(head.sell_value),
        idr(head.net_value),
        head.net_pct.map(pct1).unwrap_or_else(|| fmt::DASH.into()),
        idr(total),
        words(head.signal),
        threshold(head.signal),
    ));
    let by_tier: Vec<String> = r
        .tiers
        .iter()
        .map(|t| {
            format!(
                "top {} {}",
                t.top,
                t.net_pct.map(pct1).unwrap_or_else(|| fmt::DASH.into())
            )
        })
        .collect();
    out.push(format!("Net by tier: {}.", by_tier.join(", ")));
    if let (Some(b), Some(s)) = (head.buy_share_pct, head.sell_share_pct) {
        out.push(format!(
            "The top {n} buyers took {b:.1}% of the value, the top {n} sellers {s:.1}%: {}.",
            if b > s {
                "buying is more concentrated than selling"
            } else if b < s {
                "selling is more concentrated than buying"
            } else {
                "buying and selling are equally concentrated"
            }
        ));
    }
    if let Some(f) = r.foreign_net_value {
        let share = r
            .foreign_net_pct
            .map(|p| format!(" ({} of value)", pct1(p)))
            .unwrap_or_default();
        out.push(if f > 0.0 {
            format!("Foreign investors net bought {}{share}.", idr(f))
        } else if f < 0.0 {
            format!("Foreign investors net sold {}{share}.", idr(-f))
        } else {
            "Foreign investors were flat on balance.".into()
        });
    }
    match (r.phase, r.price_change_pct) {
        (Some(p), Some(chg)) if p != BandarPhase::Neutral => out.push(format!(
            "Price {} over the range: {} with the price {} reads as {}.",
            pct1(chg),
            words(r.signal),
            if chg > FLAT_PCT {
                format!("up more than {FLAT_PCT:.0}%")
            } else if chg < -FLAT_PCT {
                format!("down more than {FLAT_PCT:.0}%")
            } else {
                format!("within ±{FLAT_PCT:.0}%")
            },
            p.as_str().replace('_', " "),
        )),
        (None, _) => out.push("No price move for the range, so no phase.".into()),
        _ => {}
    }
    if let (Some(avg), Some(price), Some(g)) = (r.bandar_average, r.price, r.bandar_gain_pct) {
        out.push(format!(
            "The top {n} buyers paid {} on average against the last price {}: {}.",
            fmt::price(Some(avg.round())),
            fmt::price(Some(price)),
            if g >= 0.0 {
                format!("a {g:.1}% gain")
            } else {
                format!("{:.1}% under water", -g)
            }
        ));
    }
    if r.rounded {
        out.push("The source rounds figures of a million and more to one decimal.".into());
    }
    out
}

/// The full reading of one summary. `price` is the last price of the
/// range's final session and `price_change_pct` the move over the range.
pub fn report(
    s: &BrokerSummary,
    period: &str,
    top: u32,
    price: Option<f64>,
    price_change_pct: Option<f64>,
) -> BandarReport {
    let tiers: Vec<BandarTier> = TIERS.iter().map(|n| tier(s, *n)).collect();
    let head = tiers
        .iter()
        .find(|t| t.top == top)
        .cloned()
        .unwrap_or_else(|| tier(s, top));
    let traded = s.total_value.is_some_and(|v| v > 0.0);
    let signal = head.signal;
    let bandar_gain_pct = change_pct(head.buy_average, price);
    let n = top as usize;
    let mut r = BandarReport {
        symbol: s.symbol.clone(),
        name: s.name.clone(),
        period: period.to_string(),
        from: s.from,
        to: s.to,
        investor: s.investor,
        board: s.board,
        top,
        signal,
        phase: if traded {
            phase(signal, price_change_pct)
        } else {
            Some(BandarPhase::Neutral)
        },
        net_value: head.net_value,
        net_lots: head.net_lots,
        net_pct: head.net_pct,
        price,
        price_change_pct,
        bandar_average: head.buy_average,
        bandar_gain_pct,
        total_value: s.total_value,
        total_lots: s.total_lots,
        foreign_net_value: s.foreign_net_value,
        foreign_net_pct: s.foreign_net_value.and_then(|f| pct_of(f, s.total_value)),
        tiers,
        buyers: s.buyers.iter().take(n).cloned().collect(),
        sellers: s.sellers.iter().take(n).cloned().collect(),
        daily: None,
        reasons: Vec::new(),
        rounded: s.rounded,
    };
    r.reasons = reasons(&r, &head);
    r
}

/// Session-by-session reading; empty sessions (holidays) are left out.
pub fn daily(sessions: &[BrokerSummary], top: u32) -> BandarDaily {
    let mut days = Vec::new();
    let mut cum = 0.0;
    for s in sessions.iter().filter(|s| !s.is_empty()) {
        let t = tier(s, top);
        cum += t.net_value;
        days.push(BandarDay {
            date: s.to,
            total_value: s.total_value,
            net_value: t.net_value,
            net_pct: t.net_pct,
            signal: t.signal,
            cumulative_net_value: cum,
            foreign_net_value: s.foreign_net_value,
        });
    }
    let count = |f: fn(&BandarSignal) -> bool| days.iter().filter(|d| f(&d.signal)).count() as u32;
    let mut streak = 0i32;
    for d in days.iter().rev() {
        let dir = if d.signal.is_accumulation() {
            1
        } else if d.signal.is_distribution() {
            -1
        } else {
            0
        };
        if dir == 0 || (streak != 0 && dir != streak.signum()) {
            break;
        }
        streak += dir;
    }
    BandarDaily {
        accumulation_days: count(BandarSignal::is_accumulation),
        distribution_days: count(BandarSignal::is_distribution),
        neutral_days: count(|s| *s == BandarSignal::Neutral),
        streak,
        cumulative_net_value: cum,
        days,
    }
}

pub fn screen_row(r: &BandarReport) -> BandarScreenRow {
    let head = r.tiers.iter().find(|t| t.top == r.top);
    BandarScreenRow {
        symbol: r.symbol.clone(),
        name: r.name.clone(),
        signal: r.signal,
        phase: r.phase,
        net_pct: r.net_pct,
        net_value: r.net_value,
        net_lots: r.net_lots,
        price: r.price,
        price_change_pct: r.price_change_pct,
        bandar_average: r.bandar_average,
        bandar_gain_pct: r.bandar_gain_pct,
        buy_share_pct: head.and_then(|t| t.buy_share_pct),
        total_value: r.total_value,
        foreign_net_value: r.foreign_net_value,
        foreign_net_pct: r.foreign_net_pct,
        top_buyers: r.buyers.iter().map(|f| f.code.clone()).collect(),
        top_sellers: r.sellers.iter().map(|f| f.code.clone()).collect(),
        rounded: r.rounded,
    }
}

// ---- screen filters ------------------------------------------------------------

/// Keys `bandar screen` filters and sorts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Acc,
    Net,
    Lots,
    Fnet,
    Fpct,
    Gain,
    Chg,
    Turnover,
    Conc,
    Price,
    Signal,
    Phase,
}

impl Key {
    pub const ALL: &'static str =
        "acc, net, lots, fnet, fpct, gain, chg, turnover, conc, price, signal, phase";

    pub fn parse(s: &str) -> Option<Key> {
        Some(match s.to_ascii_lowercase().as_str() {
            "acc" | "net%" | "netpct" | "bandar" => Key::Acc,
            "net" | "value" | "bval" => Key::Net,
            "lots" | "bvol" => Key::Lots,
            "fnet" | "foreign" => Key::Fnet,
            "fpct" | "fnet%" | "foreign%" => Key::Fpct,
            "gain" => Key::Gain,
            "chg" | "change" => Key::Chg,
            "turnover" | "tval" => Key::Turnover,
            "conc" | "share" => Key::Conc,
            "price" => Key::Price,
            "signal" => Key::Signal,
            "phase" => Key::Phase,
            _ => return None,
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Key::Acc => "acc",
            Key::Net => "net",
            Key::Lots => "lots",
            Key::Fnet => "fnet",
            Key::Fpct => "fpct",
            Key::Gain => "gain",
            Key::Chg => "chg",
            Key::Turnover => "turnover",
            Key::Conc => "conc",
            Key::Price => "price",
            Key::Signal => "signal",
            Key::Phase => "phase",
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Key::Signal | Key::Phase)
    }

    /// Needs the price move over the range (a history call per stock on
    /// multi-day periods).
    pub fn needs_move(&self) -> bool {
        matches!(self, Key::Chg | Key::Phase)
    }
}

/// A sort key: any numeric filter key; `None` for no key given.
pub fn parse_sort(s: Option<&str>) -> Result<Option<Key>> {
    match s.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => match Key::parse(s) {
            Some(k) if !k.is_text() => Ok(Some(k)),
            _ => Err(IdxError::Usage(format!(
                "unknown sort key {s:?}; use one of acc, net, lots, fnet, fpct, gain, chg, turnover, conc, price"
            ))),
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub key: Key,
    pub op: Op,
    pub value: Value,
    pub raw: String,
}

fn signal_name(s: &str) -> Option<&'static str> {
    Some(
        match s.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "big_accumulation" | "bigacc" | "big_acc" => "big_accumulation",
            "accumulation" | "acc" => "accumulation",
            "neutral" => "neutral",
            "distribution" | "dist" => "distribution",
            "big_distribution" | "bigdist" | "big_dist" => "big_distribution",
            _ => return None,
        },
    )
}

fn phase_name(s: &str) -> Option<&'static str> {
    Some(
        match s.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "quiet_accumulation" | "quietacc" => "quiet_accumulation",
            "markup" => "markup",
            "quiet_distribution" | "quietdist" => "quiet_distribution",
            "markdown" => "markdown",
            "neutral" => "neutral",
            _ => return None,
        },
    )
}

/// Parses `"acc>=5,fpct>0,phase=quiet_accumulation"`.
pub fn parse_filters(spec: &str) -> Result<Vec<Filter>> {
    let mut out = Vec::new();
    for clause in spec.split([',', ';']).map(str::trim) {
        if clause.is_empty() {
            continue;
        }
        let (key_s, op, val_s) = split_clause(clause)?;
        let key = Key::parse(key_s).ok_or_else(|| {
            IdxError::Usage(format!(
                "unknown bandar filter key {key_s:?}; use one of {}",
                Key::ALL
            ))
        })?;
        let value = if key.is_text() {
            if !matches!(op, Op::Eq | Op::Ne) {
                return Err(IdxError::Usage(format!(
                    "{} only supports = and !=",
                    key.name()
                )));
            }
            let name = if key == Key::Signal {
                signal_name(val_s).ok_or_else(|| IdxError::Usage(format!(
                    "unknown signal {val_s:?}; use big_accumulation, accumulation, neutral, distribution or big_distribution"
                )))?
            } else {
                phase_name(val_s).ok_or_else(|| IdxError::Usage(format!(
                    "unknown phase {val_s:?}; use quiet_accumulation, markup, quiet_distribution, markdown or neutral"
                )))?
            };
            Value::Text(name.to_string())
        } else {
            Value::Num(parse_number(val_s).ok_or_else(|| {
                IdxError::Usage(format!(
                    "filter {clause:?}: {val_s:?} is not a number (try 5, -15, 10B, 2%)"
                ))
            })?)
        };
        out.push(Filter {
            key,
            op,
            value,
            raw: clause.to_string(),
        });
    }
    Ok(out)
}

pub fn numeric(r: &BandarScreenRow, key: Key) -> Option<f64> {
    match key {
        Key::Acc => r.net_pct,
        Key::Net => Some(r.net_value),
        Key::Lots => Some(r.net_lots),
        Key::Fnet => r.foreign_net_value,
        Key::Fpct => r.foreign_net_pct,
        Key::Gain => r.bandar_gain_pct,
        Key::Chg => r.price_change_pct,
        Key::Turnover => r.total_value,
        Key::Conc => r.buy_share_pct,
        Key::Price => r.price,
        Key::Signal | Key::Phase => None,
    }
}

/// A row passes when every filter holds. Missing values never match.
pub fn matches(r: &BandarScreenRow, filters: &[Filter]) -> bool {
    filters.iter().all(|f| match &f.value {
        Value::Num(target) => numeric(r, f.key).is_some_and(|v| compare(v, f.op, *target)),
        Value::Text(want) => {
            let have = match f.key {
                Key::Signal => Some(r.signal.as_str()),
                _ => r.phase.map(|p| p.as_str()),
            };
            match have {
                None => false,
                Some(h) => (h == want) == (f.op == Op::Eq),
            }
        }
    })
}

/// Sorts by `key`; rows without a value go last whatever the direction.
pub fn sort_rows(rows: &mut [BandarScreenRow], key: Key, descending: bool) {
    rows.sort_by(|a, b| match (numeric(a, key), numeric(b, key)) {
        (None, None) => a.symbol.cmp(&b.symbol),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let o = x.partial_cmp(&y).unwrap_or(Ordering::Equal);
            if descending {
                o.reverse()
            } else {
                o
            }
        }
    });
}

/// Ready-made screens, each a filter string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Preset {
    /// Top-N net of +15 % of value or more.
    BigAccumulation,
    /// Top-N net of +5 % of value or more.
    Accumulation,
    /// Top-N net of -5 % of value or less.
    Distribution,
    /// Top-N net of -15 % of value or less.
    BigDistribution,
    /// Accumulation while the price is flat or down.
    QuietAccumulation,
    /// Distribution while the price is flat or up.
    QuietDistribution,
    /// Accumulation with foreign investors net buying 5 % of value or more.
    ForeignAccumulation,
    /// Accumulation with the price under the top buyers' average.
    BelowBandar,
}

impl Preset {
    pub fn name(&self) -> &'static str {
        match self {
            Preset::BigAccumulation => "big-accumulation",
            Preset::Accumulation => "accumulation",
            Preset::Distribution => "distribution",
            Preset::BigDistribution => "big-distribution",
            Preset::QuietAccumulation => "quiet-accumulation",
            Preset::QuietDistribution => "quiet-distribution",
            Preset::ForeignAccumulation => "foreign-accumulation",
            Preset::BelowBandar => "below-bandar",
        }
    }

    pub fn filter(&self) -> &'static str {
        match self {
            Preset::BigAccumulation => "acc>=15",
            Preset::Accumulation => "acc>=5",
            Preset::Distribution => "acc<=-5",
            Preset::BigDistribution => "acc<=-15",
            Preset::QuietAccumulation => "phase=quiet_accumulation",
            Preset::QuietDistribution => "phase=quiet_distribution",
            Preset::ForeignAccumulation => "acc>=5,fpct>=5",
            Preset::BelowBandar => "acc>=5,gain<0",
        }
    }

    /// Distribution screens list the heaviest selling first.
    pub fn ascending(&self) -> bool {
        matches!(
            self,
            Preset::Distribution | Preset::BigDistribution | Preset::QuietDistribution
        )
    }
}

impl std::str::FromStr for Preset {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <Preset as clap::ValueEnum>::from_str(&s.replace('_', "-"), true).map_err(|_| {
            IdxError::Usage(format!(
                "unknown preset {s:?}; use big-accumulation, accumulation, distribution, big-distribution, quiet-accumulation, quiet-distribution, foreign-accumulation or below-bandar"
            ))
        })
    }
}

/// Dates of the weekdays in `from..=to`.
pub fn weekdays(from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    use chrono::Datelike;
    from.iter_days()
        .take_while(|d| *d <= to)
        .filter(|d| !matches!(d.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BrokerBoard, BrokerInvestor, Symbol};
    use crate::providers::ipot::parse;

    fn load(html: &str, sym: &str) -> BrokerSummary {
        let day = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        match parse::parse(html).unwrap() {
            parse::Page::Summary(p) => p.into_summary(
                Symbol::parse(sym).unwrap(),
                day,
                day,
                BrokerInvestor::All,
                BrokerBoard::All,
            ),
            parse::Page::Invalid => panic!("invalid page"),
        }
    }

    const DAY: &str = include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261016.html");
    const WEEK: &str =
        include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261012_20261016.html");
    const EMPTY: &str =
        include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261017_no_trades.html");

    #[test]
    fn signals_and_phases_follow_the_table() {
        assert_eq!(signal(Some(15.0)), BandarSignal::BigAccumulation);
        assert_eq!(signal(Some(14.99)), BandarSignal::Accumulation);
        assert_eq!(signal(Some(5.0)), BandarSignal::Accumulation);
        assert_eq!(signal(Some(4.99)), BandarSignal::Neutral);
        assert_eq!(signal(Some(-5.0)), BandarSignal::Distribution);
        assert_eq!(signal(Some(-15.0)), BandarSignal::BigDistribution);
        assert_eq!(signal(None), BandarSignal::Neutral);
        let acc = BandarSignal::Accumulation;
        assert_eq!(phase(acc, Some(2.0)), Some(BandarPhase::QuietAccumulation));
        assert_eq!(phase(acc, Some(2.01)), Some(BandarPhase::Markup));
        let dist = BandarSignal::BigDistribution;
        assert_eq!(
            phase(dist, Some(-2.0)),
            Some(BandarPhase::QuietDistribution)
        );
        assert_eq!(phase(dist, Some(-14.5)), Some(BandarPhase::Markdown));
        assert_eq!(phase(dist, None), None);
        assert_eq!(
            phase(BandarSignal::Neutral, None),
            Some(BandarPhase::Neutral)
        );
    }

    #[test]
    fn tiers_of_one_session() {
        let s = load(DAY, "BBCA");
        let t1 = tier(&s, 1);
        assert_eq!(t1.buy_value, 841_200_000_000.0);
        let t5 = tier(&s, 5);
        assert_eq!(t5.top, 5);
        assert!((t5.net_value - (t5.buy_value - t5.sell_value)).abs() < 1.0);
        let pct = t5.net_pct.unwrap();
        assert!((pct - t5.net_value / 6.6e12 * 100.0).abs() < 1e-9);
        assert!((pct - -24.3).abs() < 0.1, "{pct}");
        assert_eq!(t5.signal, BandarSignal::BigDistribution);
        let avg = t5.buy_average.unwrap();
        assert!(avg > 5_000.0 && avg < 6_000.0, "{avg}");
        let t10 = tier(&s, 10);
        assert!(t10.buy_share_pct.unwrap() > t5.buy_share_pct.unwrap());
    }

    /// The arithmetic of `report` on the raw week page. That page's foreign
    /// net (-4.1 T) counts the 16th twice; the service rebuilds it from
    /// single sessions (-1.82 T, see `ipot::broker_summary_range` and the
    /// service's bandar tests), but `report` reads whatever summary it gets.
    #[test]
    fn report_reads_the_week() {
        let s = load(WEEK, "BBCA");
        let r = report(&s, "1w", 5, Some(5_175.0), Some(-14.46));
        assert_eq!(r.period, "1w");
        assert_eq!(r.tiers.iter().map(|t| t.top).collect::<Vec<_>>(), TIERS);
        assert_eq!(r.signal, BandarSignal::BigDistribution);
        assert_eq!(r.phase, Some(BandarPhase::Markdown));
        assert_eq!(r.buyers.len(), 5);
        assert_eq!(r.sellers.len(), 5);
        assert!((r.net_pct.unwrap() - -19.5).abs() < 0.1, "{:?}", r.net_pct);
        assert!((r.foreign_net_pct.unwrap() - -41.4).abs() < 0.1);
        // web/test/bandar.test.mjs pins the same text for the JS port.
        assert_eq!(
            r.reasons,
            [
                "Top 5 buyers bought 4.23T, top 5 sellers sold 6.16T: net -1.93T, -19.5% of the 9.90T traded (big distribution, at -15% or less).",
                "Net by tier: top 1 -17.5%, top 3 -18.8%, top 5 -19.5%, top 10 -11.9%.",
                "The top 5 buyers took 42.7% of the value, the top 5 sellers 62.2%: selling is more concentrated than buying.",
                "Foreign investors net sold 4.10T (-41.4% of value).",
                "Price -14.5% over the range: big distribution with the price down more than 2% reads as markdown.",
                "The top 5 buyers paid 5,515 on average against the last price 5,175: 6.2% under water.",
                "The source rounds figures of a million and more to one decimal.",
            ]
        );

        let r = report(&s, "1w", 1, None, None);
        assert_eq!(r.buyers.len(), 1);
        assert_eq!(r.phase, None);
        assert_eq!(r.bandar_gain_pct, None);
        assert!(r.reasons.iter().any(|l| l.contains("no phase")));
    }

    #[test]
    fn no_trades_is_neutral() {
        let s = load(EMPTY, "BBCA");
        let r = report(&s, "1d", 5, Some(5_175.0), Some(0.0));
        assert_eq!(r.signal, BandarSignal::Neutral);
        assert_eq!(r.phase, Some(BandarPhase::Neutral));
        assert_eq!(r.net_pct, None);
        assert_eq!(
            r.reasons,
            vec!["No trades from 2026-10-17 to 2026-10-17.".to_string()]
        );
    }

    #[test]
    fn daily_series_counts_and_streaks() {
        let days: Vec<BrokerSummary> = [
            include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261012.html"),
            include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261013.html"),
            include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261014.html"),
            include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261015.html"),
            DAY,
            EMPTY,
        ]
        .iter()
        .map(|h| load(h, "BBCA"))
        .collect();
        let d = daily(&days, 5);
        assert_eq!(d.days.len(), 5, "the empty Saturday is left out");
        assert_eq!(
            d.days[0].date,
            NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()
        );
        assert_eq!(
            d.accumulation_days + d.distribution_days + d.neutral_days,
            5
        );
        let sum: f64 = d.days.iter().map(|x| x.net_value).sum();
        assert!((d.cumulative_net_value - sum).abs() < 1.0);
        assert!((d.days[4].cumulative_net_value - sum).abs() < 1.0);
        assert!(d.streak < 0, "16 Oct is a distribution day: {}", d.streak);
    }

    #[test]
    fn filters_presets_and_sorting() {
        let f = parse_filters("acc>=5, fpct>0 ;signal=bigacc,phase!=markup").unwrap();
        assert_eq!(f.len(), 4);
        assert_eq!(f[0].key, Key::Acc);
        assert_eq!(f[2].value, Value::Text("big_accumulation".into()));
        assert!(parse_filters("pe<10").is_err());
        assert!(parse_filters("signal>acc").is_err());
        assert!(parse_filters("signal=up").is_err());
        assert!(parse_filters("acc>lots").is_err());
        for p in [
            Preset::BigAccumulation,
            Preset::Accumulation,
            Preset::Distribution,
            Preset::BigDistribution,
            Preset::QuietAccumulation,
            Preset::QuietDistribution,
            Preset::ForeignAccumulation,
            Preset::BelowBandar,
        ] {
            assert!(parse_filters(p.filter()).is_ok(), "{}", p.name());
            assert_eq!(p.name().parse::<Preset>().unwrap(), p);
        }
        assert_eq!(
            "big_accumulation".parse::<Preset>().unwrap(),
            Preset::BigAccumulation
        );

        let s = load(WEEK, "BBCA");
        let row = screen_row(&report(&s, "1w", 5, Some(5_175.0), Some(-14.46)));
        assert_eq!(row.top_buyers.len(), 5);
        assert!(matches(&row, &parse_filters("acc<=-15").unwrap()));
        assert!(matches(
            &row,
            &parse_filters("phase=markdown,signal=big_dist").unwrap()
        ));
        assert!(!matches(
            &row,
            &parse_filters(Preset::Accumulation.filter()).unwrap()
        ));
        assert!(matches(&row, &parse_filters("phase!=markup").unwrap()));
        let mut other = row.clone();
        other.symbol = Symbol::parse("BBRI").unwrap();
        other.net_pct = Some(3.0);
        let mut none = row.clone();
        none.symbol = Symbol::parse("AAAA").unwrap();
        none.net_pct = None;
        let mut rows = vec![none, row, other];
        sort_rows(&mut rows, Key::Acc, true);
        let order: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(order, ["BBRI", "BBCA", "AAAA"]);
    }

    #[test]
    fn weekdays_skip_weekends() {
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        assert_eq!(
            weekdays(d(10, 9), d(10, 13)),
            vec![d(10, 9), d(10, 12), d(10, 13)]
        );
        assert!(check_top(4).is_err());
        assert_eq!(check_top(10).unwrap(), 10);
    }
}
