//! Screener filter language and evaluation.
//!
//! `--filter "pe<15,roe>15,sector=Banking,mcap>10T"`: comma-separated
//! clauses, each `key op value`. Numeric values accept `K`, `M`, `B`, `T`
//! suffixes and a trailing `%`. String keys (`sector`, `industry`) match
//! case-insensitively as a substring with `=` / `!=`. A missing value never
//! matches, and neither does a P/B or P/S over statements in another
//! currency than the price ([`numeric`]).
//!
//! Presets ([`ScreenPreset`]) are idx's own ready-made screens: a filter
//! string plus a sort key and direction, run through the same screener.

use std::cmp::Ordering;

use crate::error::{IdxError, Result};
use crate::model::{Fundamentals, MarketReturns, Quote, ScreenRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
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

/// Every key the screener understands. `needs_fundamentals` marks the ones
/// that require a per-symbol fundamentals call, `needs_returns` the ones that
/// need the market-wide return windows (one request).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Pe,
    Fpe,
    Pb,
    Mcap,
    Price,
    Div,
    Eps,
    Volume,
    Chg,
    /// Position in the 52-week range, 0 (low) to 100 (high).
    Range,
    Ret1w,
    Ret1m,
    Ret3m,
    Ret6m,
    RetYtd,
    Ret1y,
    Roe,
    Roa,
    Margin,
    OpMargin,
    De,
    Growth,
    EpsGrowth,
    Peg,
    Ps,
    Sector,
    Industry,
}

impl Key {
    pub fn parse(s: &str) -> Option<Key> {
        Some(match s.to_ascii_lowercase().as_str() {
            "pe" | "p/e" | "per" => Key::Pe,
            "fpe" | "forwardpe" | "fwdpe" => Key::Fpe,
            "pb" | "pbv" | "p/b" => Key::Pb,
            "mcap" | "marketcap" | "cap" => Key::Mcap,
            "price" | "close" => Key::Price,
            "div" | "yield" | "dividend" => Key::Div,
            "eps" => Key::Eps,
            "volume" | "vol" | "avgvolume" => Key::Volume,
            "chg" | "change" => Key::Chg,
            "range" | "pos52" | "52w" => Key::Range,
            "ret1w" => Key::Ret1w,
            "ret1m" => Key::Ret1m,
            "ret3m" => Key::Ret3m,
            "ret6m" => Key::Ret6m,
            "retytd" | "ytd" => Key::RetYtd,
            "ret1y" => Key::Ret1y,
            "roe" => Key::Roe,
            "roa" => Key::Roa,
            "margin" | "npm" | "netmargin" => Key::Margin,
            "opmargin" | "opm" => Key::OpMargin,
            "de" | "der" | "debt" => Key::De,
            "growth" | "revgrowth" | "revenuegrowth" => Key::Growth,
            "epsgrowth" | "earningsgrowth" => Key::EpsGrowth,
            "peg" => Key::Peg,
            "ps" | "p/s" => Key::Ps,
            "sector" => Key::Sector,
            "industry" => Key::Industry,
            _ => return None,
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Key::Pe => "pe",
            Key::Fpe => "fpe",
            Key::Pb => "pb",
            Key::Mcap => "mcap",
            Key::Price => "price",
            Key::Div => "div",
            Key::Eps => "eps",
            Key::Volume => "volume",
            Key::Chg => "chg",
            Key::Range => "range",
            Key::Ret1w => "ret1w",
            Key::Ret1m => "ret1m",
            Key::Ret3m => "ret3m",
            Key::Ret6m => "ret6m",
            Key::RetYtd => "retytd",
            Key::Ret1y => "ret1y",
            Key::Roe => "roe",
            Key::Roa => "roa",
            Key::Margin => "margin",
            Key::OpMargin => "opmargin",
            Key::De => "de",
            Key::Growth => "growth",
            Key::EpsGrowth => "epsgrowth",
            Key::Peg => "peg",
            Key::Ps => "ps",
            Key::Sector => "sector",
            Key::Industry => "industry",
        }
    }

    pub fn needs_fundamentals(&self) -> bool {
        matches!(
            self,
            Key::Roe
                | Key::Roa
                | Key::Margin
                | Key::OpMargin
                | Key::De
                | Key::Growth
                | Key::EpsGrowth
                | Key::Peg
                | Key::Ps
                | Key::Sector
                | Key::Industry
        )
    }

    /// The return-window keys: joined from one market-wide request.
    pub fn needs_returns(&self) -> bool {
        matches!(
            self,
            Key::Ret1w | Key::Ret1m | Key::Ret3m | Key::Ret6m | Key::RetYtd | Key::Ret1y
        )
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Key::Sector | Key::Industry)
    }

    pub const ALL: &'static str = "pe, fpe, pb, ps, peg, mcap, price, div, eps, volume, chg, range, ret1w, ret1m, ret3m, ret6m, retytd, ret1y, roe, roa, margin, opmargin, de, growth, epsgrowth, sector, industry";
}

/// idx's ready-made screens. Each is a filter (possibly empty) plus a sort
/// key and direction; `-f`, `--sort` and `--asc`/`--desc` add to or override
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ScreenPreset {
    /// Best 1-year price return first.
    TopPerformers,
    /// Worst 1-year price return first.
    WorstPerformers,
    /// Dividend payers, highest yield first.
    HighDividend,
    /// Positive P/E, lowest first.
    LowPe,
    /// Price in the top 5 % of its 52-week range.
    #[value(name = "52w-high")]
    High52w,
    /// Price in the bottom 5 % of its 52-week range.
    #[value(name = "52w-low")]
    Low52w,
    /// Highest 3-month average volume first.
    HighVolume,
    /// Largest market cap first.
    LargeCap,
}

impl ScreenPreset {
    pub const NAMES: &'static str = "top-performers, worst-performers, high-dividend, low-pe, 52w-high, 52w-low, high-volume, large-cap";

    pub fn name(&self) -> &'static str {
        match self {
            ScreenPreset::TopPerformers => "top-performers",
            ScreenPreset::WorstPerformers => "worst-performers",
            ScreenPreset::HighDividend => "high-dividend",
            ScreenPreset::LowPe => "low-pe",
            ScreenPreset::High52w => "52w-high",
            ScreenPreset::Low52w => "52w-low",
            ScreenPreset::HighVolume => "high-volume",
            ScreenPreset::LargeCap => "large-cap",
        }
    }

    /// Clauses the preset adds; empty for the pure rankings.
    pub fn filter(&self) -> &'static str {
        match self {
            ScreenPreset::HighDividend => "div>0",
            ScreenPreset::LowPe => "pe>0",
            ScreenPreset::High52w => "range>=95",
            ScreenPreset::Low52w => "range<=5",
            ScreenPreset::TopPerformers
            | ScreenPreset::WorstPerformers
            | ScreenPreset::HighVolume
            | ScreenPreset::LargeCap => "",
        }
    }

    pub fn sort(&self) -> Key {
        match self {
            ScreenPreset::TopPerformers | ScreenPreset::WorstPerformers => Key::Ret1y,
            ScreenPreset::HighDividend => Key::Div,
            ScreenPreset::LowPe => Key::Pe,
            ScreenPreset::High52w | ScreenPreset::Low52w => Key::Range,
            ScreenPreset::HighVolume => Key::Volume,
            ScreenPreset::LargeCap => Key::Mcap,
        }
    }

    /// The "worst" and "low" screens list the smallest values first.
    pub fn descending(&self) -> bool {
        !matches!(
            self,
            ScreenPreset::WorstPerformers | ScreenPreset::LowPe | ScreenPreset::Low52w
        )
    }
}

impl std::str::FromStr for ScreenPreset {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <ScreenPreset as clap::ValueEnum>::from_str(&s.trim().replace('_', "-"), true).map_err(
            |_| IdxError::Usage(format!("unknown preset {s:?}; use {}", ScreenPreset::NAMES)),
        )
    }
}

/// `10T`, `2.5B`, `500M`, `15%`, `0.8`.
pub fn parse_number(s: &str) -> Option<f64> {
    let t = s
        .trim()
        .trim_end_matches('%')
        .trim()
        .replace(['_', ','], "");
    if t.is_empty() {
        return None;
    }
    let (num, mult) = match t.chars().last()?.to_ascii_uppercase() {
        'K' => (&t[..t.len() - 1], 1e3),
        'M' => (&t[..t.len() - 1], 1e6),
        'B' => (&t[..t.len() - 1], 1e9),
        'T' => (&t[..t.len() - 1], 1e12),
        _ => (t.as_str(), 1.0),
    };
    num.trim().parse::<f64>().ok().map(|v| v * mult)
}

/// Splits one clause into its key, operator and value text: `"pe<=15"` ->
/// `("pe", Le, "15")`. The leftmost operator wins, the longer one on a tie.
pub fn split_clause(clause: &str) -> Result<(&str, Op, &str)> {
    let ops = [
        ("<=", Op::Le),
        (">=", Op::Ge),
        ("!=", Op::Ne),
        ("<", Op::Lt),
        (">", Op::Gt),
        ("=", Op::Eq),
    ];
    let Some((pos, (sym, op))) = ops
        .iter()
        .filter_map(|(s, o)| clause.find(s).map(|p| (p, (*s, *o))))
        .min_by_key(|(p, (s, _))| (*p, std::cmp::Reverse(s.len())))
    else {
        return Err(IdxError::Usage(format!(
            "filter {clause:?} needs an operator (<, <=, >, >=, =, !=)"
        )));
    };
    Ok((clause[..pos].trim(), op, clause[pos + sym.len()..].trim()))
}

/// `v op target` for numbers; equality within 1e-9.
pub fn compare(v: f64, op: Op, target: f64) -> bool {
    match op {
        Op::Lt => v < target,
        Op::Le => v <= target,
        Op::Gt => v > target,
        Op::Ge => v >= target,
        Op::Eq => (v - target).abs() < 1e-9,
        Op::Ne => (v - target).abs() >= 1e-9,
    }
}

/// Parses `"pe<15, roe>=15, sector=bank"` into filters.
pub fn parse_filters(spec: &str) -> Result<Vec<Filter>> {
    let mut out = Vec::new();
    for clause in spec.split([',', ';']).map(str::trim) {
        if clause.is_empty() {
            continue;
        }
        let (key_s, op, val_s) = split_clause(clause)?;
        let Some(key) = Key::parse(key_s) else {
            return Err(IdxError::Usage(format!(
                "unknown filter key {key_s:?}; use one of {}",
                Key::ALL
            )));
        };
        let value = if key.is_text() {
            if !matches!(op, Op::Eq | Op::Ne) {
                return Err(IdxError::Usage(format!(
                    "{} only supports = and !=",
                    key.name()
                )));
            }
            if val_s.is_empty() {
                return Err(IdxError::Usage(format!("{} needs a value", key.name())));
            }
            Value::Text(val_s.trim_matches('"').to_string())
        } else {
            match parse_number(val_s) {
                Some(n) => Value::Num(n),
                None => {
                    return Err(IdxError::Usage(format!(
                        "filter {clause:?}: {val_s:?} is not a number (try 15, 2.5B, 10T, 3%)"
                    )))
                }
            }
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

/// The value a filter or the sort reads for `key`. P/B and P/S that divide
/// the price by statements in another currency ([`ScreenRow::mixed_currency`])
/// are not ratios, so they read as missing: no `pb` or `ps` clause matches
/// them, and a `pb` or `ps` sort puts them last. The row keeps them as
/// reported.
pub fn numeric(row: &ScreenRow, key: Key) -> Option<f64> {
    let ratio = |v: Option<f64>| v.filter(|_| !row.mixed_currency());
    match key {
        Key::Pe => row.pe_trailing,
        Key::Fpe => row.pe_forward,
        Key::Pb => ratio(row.price_to_book),
        Key::Mcap => row.market_cap,
        Key::Price => row.price,
        Key::Div => row.dividend_yield_pct,
        Key::Eps => row.eps_ttm,
        Key::Volume => row.avg_volume_3m.map(|v| v as f64),
        Key::Chg => row.change_pct,
        Key::Range => row.range_52w_pct,
        Key::Ret1w => row.return_1w_pct,
        Key::Ret1m => row.return_1m_pct,
        Key::Ret3m => row.return_3m_pct,
        Key::Ret6m => row.return_6m_pct,
        Key::RetYtd => row.return_ytd_pct,
        Key::Ret1y => row.return_1y_pct,
        Key::Roe => row.roe_pct,
        Key::Roa => row.roa_pct,
        Key::Margin => row.net_margin_pct,
        Key::OpMargin => row.operating_margin_pct,
        Key::De => row.debt_to_equity,
        Key::Growth => row.revenue_growth_pct,
        Key::EpsGrowth => row.earnings_growth_pct,
        Key::Peg => row.peg,
        Key::Ps => ratio(row.price_to_sales),
        Key::Sector | Key::Industry => None,
    }
}

fn text(row: &ScreenRow, key: Key) -> Option<&str> {
    match key {
        Key::Sector => row.sector.as_deref(),
        Key::Industry => row.industry.as_deref(),
        _ => None,
    }
}

/// A row passes when every filter holds. Missing values never match.
pub fn matches(row: &ScreenRow, filters: &[Filter]) -> bool {
    filters.iter().all(|f| match &f.value {
        Value::Num(target) => match numeric(row, f.key) {
            None => false,
            Some(v) => compare(v, f.op, *target),
        },
        Value::Text(needle) => match text(row, f.key) {
            None => false,
            Some(hay) => {
                let hit = hay.to_lowercase().contains(&needle.to_lowercase());
                if f.op == Op::Ne {
                    !hit
                } else {
                    hit
                }
            }
        },
    })
}

/// Sorts by `key`; rows without a value go last whatever the direction.
pub fn sort_rows(rows: &mut [ScreenRow], key: Key, descending: bool) {
    rows.sort_by(|a, b| {
        let (x, y) = (numeric(a, key), numeric(b, key));
        match (x, y) {
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
        }
    });
}

/// Where `price` sits between `low` and `high`, in percent (0 at the low,
/// 100 at the high). Not clamped: a price outside a stale range shows as such.
pub fn range_position(price: Option<f64>, low: Option<f64>, high: Option<f64>) -> Option<f64> {
    match (price, low, high) {
        (Some(p), Some(l), Some(h)) if h > l => Some((p - l) / (h - l) * 100.0),
        _ => None,
    }
}

pub fn row_from_quote(q: &Quote, name_hint: Option<&str>) -> ScreenRow {
    ScreenRow {
        symbol: q.symbol.clone(),
        name: q.name.clone().or_else(|| name_hint.map(str::to_string)),
        currency: q.currency.clone(),
        financial_currency: q.financial_currency.clone(),
        price: q.price,
        change_pct: q.change_pct,
        market_cap: q.market_cap,
        pe_trailing: q.pe_trailing,
        pe_forward: q.pe_forward,
        price_to_book: q.price_to_book,
        dividend_yield_pct: q.dividend_yield_pct,
        eps_ttm: q.eps_ttm,
        avg_volume_3m: q.avg_volume_3m,
        week52_high: q.week52_high,
        week52_low: q.week52_low,
        range_52w_pct: range_position(q.price, q.week52_low, q.week52_high),
        return_1w_pct: None,
        return_1m_pct: None,
        return_3m_pct: None,
        return_6m_pct: None,
        return_ytd_pct: None,
        return_1y_pct: None,
        fundamentals_checked: false,
        sector: None,
        industry: None,
        roe_pct: None,
        roa_pct: None,
        net_margin_pct: None,
        operating_margin_pct: None,
        debt_to_equity: None,
        revenue_growth_pct: None,
        earnings_growth_pct: None,
        peg: None,
        price_to_sales: None,
    }
}

/// Copies the source's return windows into the row.
pub fn apply_returns(row: &mut ScreenRow, r: &MarketReturns) {
    row.return_1w_pct = r.w1;
    row.return_1m_pct = r.m1;
    row.return_3m_pct = r.m3;
    row.return_6m_pct = r.m6;
    row.return_ytd_pct = r.ytd;
    row.return_1y_pct = r.y1;
}

/// Joins a company's fundamentals into its row. P/B and P/S divide the price
/// by statement figures, so they come from one set of statements, as in
/// `Fundamentals::fill_from`: a report whose statement currency (or, when it
/// names none, its price's) is not the row's `financial_currency` fills
/// neither, nor that currency.
pub fn apply_fundamentals(row: &mut ScreenRow, f: &Fundamentals) {
    row.fundamentals_checked = true;
    row.sector = f.sector.clone();
    row.industry = f.industry.clone();
    row.roe_pct = f.profitability.roe_pct;
    row.roa_pct = f.profitability.roa_pct;
    row.net_margin_pct = f.profitability.net_margin_pct;
    row.operating_margin_pct = f.profitability.operating_margin_pct;
    row.debt_to_equity = f.health.debt_to_equity;
    row.revenue_growth_pct = f.growth.revenue_growth_pct;
    row.earnings_growth_pct = f.growth.earnings_growth_pct;
    row.peg = f.valuation.peg;
    let foreign = match (row.financial_currency.as_deref(), f.statement_currency()) {
        (Some(ours), Some(theirs)) => !ours.eq_ignore_ascii_case(theirs),
        _ => false,
    };
    if !foreign {
        row.price_to_sales = f.valuation.price_to_sales;
        if row.price_to_book.is_none() {
            row.price_to_book = f.valuation.price_to_book;
        }
        if row.financial_currency.is_none() {
            row.financial_currency = f.financial_currency.clone();
        }
    }
    if row.currency.is_none() {
        row.currency = f.currency.clone();
    }
    if row.pe_trailing.is_none() {
        row.pe_trailing = f.valuation.pe_trailing;
    }
    if row.dividend_yield_pct.is_none() {
        row.dividend_yield_pct = f.dividend.yield_pct;
    }
    if row.name.is_none() {
        row.name = f.name.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Symbol;

    fn row(sym: &str, pe: Option<f64>, mcap: Option<f64>) -> ScreenRow {
        let mut q = Quote::empty(Symbol::parse(sym).unwrap(), "t");
        q.pe_trailing = pe;
        q.market_cap = mcap;
        row_from_quote(&q, None)
    }

    #[test]
    fn parses_clauses_and_suffixes() {
        let f = parse_filters("pe<15, roe>=15,sector=Bank, mcap>10T, div>3%").unwrap();
        assert_eq!(f.len(), 5);
        assert_eq!(f[0].key, Key::Pe);
        assert_eq!(f[0].op, Op::Lt);
        assert_eq!(f[1].op, Op::Ge);
        assert_eq!(f[2].value, Value::Text("Bank".into()));
        assert_eq!(f[3].value, Value::Num(10e12));
        assert_eq!(f[4].value, Value::Num(3.0));
        assert!(f.iter().any(|x| x.key.needs_fundamentals()));
        assert_eq!(parse_number("2.5B"), Some(2.5e9));
        assert_eq!(parse_number("1,500"), Some(1500.0));
    }

    #[test]
    fn rejects_bad_clauses() {
        assert_eq!(parse_filters("pe 15").unwrap_err().exit_code(), 2);
        assert_eq!(parse_filters("nope<1").unwrap_err().exit_code(), 2);
        assert_eq!(parse_filters("pe<abc").unwrap_err().exit_code(), 2);
        assert_eq!(parse_filters("sector>1").unwrap_err().exit_code(), 2);
    }

    #[test]
    fn matching_and_sorting() {
        let f = parse_filters("pe<15,mcap>1T").unwrap();
        assert!(matches(&row("A", Some(10.0), Some(2e12)), &f));
        assert!(!matches(&row("B", Some(20.0), Some(2e12)), &f));
        assert!(
            !matches(&row("C", None, Some(2e12)), &f),
            "missing never matches"
        );
        let mut r = row("D", Some(5.0), Some(2e12));
        r.sector = Some("Financial Services".into());
        assert!(matches(&r, &parse_filters("sector=financial").unwrap()));
        assert!(!matches(&r, &parse_filters("sector!=financial").unwrap()));

        let mut rows = vec![
            row("X", Some(12.0), None),
            row("Y", Some(8.0), Some(1.0)),
            row("Z", None, Some(5.0)),
        ];
        sort_rows(&mut rows, Key::Pe, false);
        let order: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(order, ["Y", "X", "Z"]);
        sort_rows(&mut rows, Key::Mcap, true);
        let order: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(order, ["Z", "Y", "X"]);
    }

    #[test]
    fn new_keys_and_aliases() {
        for (s, k) in [
            ("range", Key::Range),
            ("pos52", Key::Range),
            ("52W", Key::Range),
            ("ret1w", Key::Ret1w),
            ("ret1m", Key::Ret1m),
            ("ret3m", Key::Ret3m),
            ("ret6m", Key::Ret6m),
            ("retytd", Key::RetYtd),
            ("ytd", Key::RetYtd),
            ("RET1Y", Key::Ret1y),
        ] {
            assert_eq!(Key::parse(s), Some(k), "{s}");
            assert_eq!(Key::parse(k.name()), Some(k));
            assert!(!k.needs_fundamentals(), "{s}");
            assert!(Key::ALL.contains(k.name()), "{s}");
        }
        assert!(Key::Ret1y.needs_returns() && !Key::Range.needs_returns());
        assert!(!Key::Pe.needs_returns() && !Key::Roe.needs_returns());
        let f = parse_filters("ret1y>50, 52w>=95").unwrap();
        assert_eq!((f[0].key, f[1].key), (Key::Ret1y, Key::Range));
        assert_eq!(f[1].op, Op::Ge);
    }

    #[test]
    fn range_position_and_returns() {
        let mut q = Quote::empty(Symbol::parse("BBCA").unwrap(), "t");
        (q.price, q.week52_low, q.week52_high) = (Some(5125.0), Some(4740.0), Some(8700.0));
        let mut r = row_from_quote(&q, None);
        assert!((r.range_52w_pct.unwrap() - 9.722222).abs() < 1e-5);
        assert_eq!(range_position(Some(10.0), Some(10.0), Some(10.0)), None);
        assert_eq!(range_position(None, Some(1.0), Some(2.0)), None);
        assert_eq!(range_position(Some(3.0), Some(1.0), Some(2.0)), Some(200.0));
        assert_eq!(r.return_1y_pct, None);
        apply_returns(
            &mut r,
            &MarketReturns {
                symbol: q.symbol.clone(),
                source: "t",
                w1: Some(3.7449),
                m1: None,
                m3: Some(-20.5426),
                m6: None,
                ytd: Some(-36.5325),
                y1: Some(-41.092),
            },
        );
        assert_eq!(numeric(&r, Key::Ret1y), Some(-41.092));
        assert_eq!(numeric(&r, Key::Ret1w), Some(3.7449));
        assert_eq!(numeric(&r, Key::Ret1m), None);
        assert_eq!(numeric(&r, Key::RetYtd), Some(-36.5325));
        assert!(matches(&r, &parse_filters("ret1y<-40,range<10").unwrap()));
        assert!(
            !matches(&r, &parse_filters("ret1m>0").unwrap()),
            "missing never matches"
        );
    }

    #[test]
    fn presets_expand_to_clauses_and_order() {
        use std::str::FromStr;
        let all = [
            (
                ScreenPreset::TopPerformers,
                "top-performers",
                "",
                Key::Ret1y,
                true,
            ),
            (
                ScreenPreset::WorstPerformers,
                "worst-performers",
                "",
                Key::Ret1y,
                false,
            ),
            (
                ScreenPreset::HighDividend,
                "high-dividend",
                "div>0",
                Key::Div,
                true,
            ),
            (ScreenPreset::LowPe, "low-pe", "pe>0", Key::Pe, false),
            (
                ScreenPreset::High52w,
                "52w-high",
                "range>=95",
                Key::Range,
                true,
            ),
            (
                ScreenPreset::Low52w,
                "52w-low",
                "range<=5",
                Key::Range,
                false,
            ),
            (
                ScreenPreset::HighVolume,
                "high-volume",
                "",
                Key::Volume,
                true,
            ),
            (ScreenPreset::LargeCap, "large-cap", "", Key::Mcap, true),
        ];
        for (p, name, filter, sort, desc) in all {
            assert_eq!(p.name(), name);
            assert_eq!(p.filter(), filter, "{name}");
            assert_eq!(p.sort(), sort, "{name}");
            assert_eq!(p.descending(), desc, "{name}");
            assert_eq!(ScreenPreset::from_str(name).unwrap(), p);
            assert!(ScreenPreset::NAMES.contains(name));
            parse_filters(p.filter()).unwrap();
        }
        assert_eq!(
            ScreenPreset::from_str("52W_HIGH").unwrap(),
            ScreenPreset::High52w
        );
        let err = ScreenPreset::from_str("nope").unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("52w-low"), "{err}");
    }

    /// A quote row as Yahoo's v7 sends it: IDR price, statements in `statements`.
    fn reporting_in(sym: &str, statements: Option<&str>, pb: Option<f64>) -> ScreenRow {
        let mut q = Quote::empty(Symbol::parse(sym).unwrap(), "yahoo:quote");
        q.currency = Some("IDR".into());
        q.financial_currency = statements.map(str::to_string);
        q.price_to_book = pb;
        row_from_quote(&q, None)
    }

    /// A fundamentals report with its currencies and book and sales multiples.
    fn report(currency: &str, statements: Option<&str>, pb: f64, ps: f64) -> Fundamentals {
        let mut f = Fundamentals::empty(Symbol::parse("ITMG").unwrap(), "t");
        f.currency = Some(currency.into());
        f.financial_currency = statements.map(str::to_string);
        f.valuation.price_to_book = Some(pb);
        f.valuation.price_to_sales = Some(ps);
        f.profitability.roe_pct = Some(11.12);
        f
    }

    #[test]
    fn rows_carry_the_price_and_statement_currencies() {
        // BSSR on 2026-11-06: the IDR price over a USD book value.
        let bssr = reporting_in("BSSR", Some("USD"), Some(48921.57));
        assert_eq!(bssr.currency.as_deref(), Some("IDR"));
        assert_eq!(bssr.financial_currency.as_deref(), Some("USD"));
        assert!(bssr.mixed_currency());
        assert!(!reporting_in("BBCA", Some("idr"), Some(2.25)).mixed_currency());
        assert!(
            !reporting_in("BSSR", None, Some(48921.57)).mixed_currency(),
            "statement currency unknown"
        );
        let mut r = bssr.clone();
        r.currency = None;
        assert!(!r.mixed_currency(), "price currency unknown");
    }

    #[test]
    fn fundamentals_fill_the_currencies_from_one_set_of_statements() {
        // ITMG's Yahoo report: USD statements, P/B 14103.308, P/S 13807.103.
        let yahoo = report("IDR", Some("USD"), 14103.308, 13807.103);

        // A quote that names no statement currency (and has no P/B, as MSN's):
        // the report fills both currencies and both multiples.
        let mut q = Quote::empty(Symbol::parse("ITMG").unwrap(), "msn:quotes");
        q.price = Some(24475.0);
        let mut r = row_from_quote(&q, None);
        apply_fundamentals(&mut r, &yahoo);
        assert_eq!(r.currency.as_deref(), Some("IDR"));
        assert_eq!(r.financial_currency.as_deref(), Some("USD"));
        assert_eq!(
            (r.price_to_book, r.price_to_sales),
            (Some(14103.308), Some(13807.103))
        );
        assert!(r.mixed_currency());

        // Yahoo's quote over the same statements keeps its own P/B.
        let mut r = reporting_in("ITMG", Some("USD"), Some(14204.875));
        apply_fundamentals(&mut r, &yahoo);
        assert_eq!(
            (r.price_to_book, r.price_to_sales),
            (Some(14204.875), Some(13807.103))
        );

        // A report that names no statement currency (MSN) reads as reporting
        // in its price's, as in Fundamentals::fill_from: it fills neither
        // multiple into a row over USD statements, and keeps the row's currency.
        let msn = report("IDR", None, 0.86, 0.84);
        let mut r = reporting_in("ITMG", Some("USD"), None);
        apply_fundamentals(&mut r, &msn);
        assert_eq!((r.price_to_book, r.price_to_sales), (None, None));
        assert_eq!(r.financial_currency.as_deref(), Some("USD"));
        assert_eq!(r.roe_pct, Some(11.12), "ratios within the statements stay");
        assert!(r.fundamentals_checked);

        // Over IDR statements it fills both, and names no currency it was not given.
        let mut r = reporting_in("BBCA", Some("IDR"), None);
        apply_fundamentals(&mut r, &msn);
        assert_eq!(
            (r.price_to_book, r.price_to_sales),
            (Some(0.86), Some(0.84))
        );
        assert_eq!(r.financial_currency.as_deref(), Some("IDR"));
        let mut r = reporting_in("BBCA", None, None);
        apply_fundamentals(&mut r, &msn);
        assert_eq!(r.price_to_sales, Some(0.84));
        assert_eq!(r.financial_currency, None);
    }

    #[test]
    fn pb_and_ps_pass_over_multiples_that_mix_currencies() {
        let mut bssr = reporting_in("BSSR", Some("USD"), Some(48921.57));
        (bssr.price_to_sales, bssr.pe_trailing) = (Some(9876.5), Some(7.95));
        let mut bbca = reporting_in("BBCA", Some("IDR"), Some(2.25));
        bbca.price_to_sales = Some(5.82);
        let unnamed = reporting_in("TLKM", None, Some(2.1));

        assert_eq!(
            (numeric(&bssr, Key::Pb), numeric(&bssr, Key::Ps)),
            (None, None)
        );
        assert_eq!(numeric(&bbca, Key::Pb), Some(2.25));
        assert_eq!(numeric(&bbca, Key::Ps), Some(5.82));
        assert_eq!(numeric(&unnamed, Key::Pb), Some(2.1), "no currency named");
        // Not a ratio: it matches no pb or ps clause, low or high, like a missing value.
        for spec in ["pb>0", "pb<1", "pb>1000", "pb!=1", "ps>0", "ps<1"] {
            assert!(!matches(&bssr, &parse_filters(spec).unwrap()), "{spec}");
        }
        assert!(matches(&bbca, &parse_filters("pb>1,ps<10").unwrap()));
        // P/E is in the price's terms and still filters.
        assert!(matches(&bssr, &parse_filters("pe<10").unwrap()));

        // Sorted last either way; the row keeps its value as reported.
        let mut rows = vec![bssr.clone(), bbca.clone(), unnamed.clone()];
        sort_rows(&mut rows, Key::Pb, true);
        let order: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(order, ["BBCA", "TLKM", "BSSR"]);
        sort_rows(&mut rows, Key::Pb, false);
        let order: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(order, ["TLKM", "BBCA", "BSSR"]);
        assert_eq!(rows[2].price_to_book, Some(48921.57));
    }
}
