//! Exchange-wide data: trading holidays, instruments, indices, the
//! corporate-action calendar, foreign flow and broker activity. Shared by
//! the CLI and the MCP server like the rest of `service`.

use std::future::Future;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate};
use futures::stream::{self, StreamExt};

use crate::brokers;
use crate::error::{IdxError, Result};
use crate::holidays;
use crate::http::Fetcher;
use crate::model::{BrokerDayRow, BrokerRanking, BrokerSort};
use crate::model::{
    Calendar, CalendarCategory, CalendarEvent, Holidays, IndexBoard, Instrument, InstrumentList,
    Member, MemberList,
};
use crate::model::{CorporateActions, Symbol};
use crate::model::{
    ForeignDay, ForeignFlow, ForeignRow, ForeignSort, ForeignTotals, ForeignValueDay,
    ForeignValueFlow, ForeignValueTotals, MarketForeign,
};
use crate::providers::idx::{self, CalendarRange, StockDay};
use crate::providers::ipot;
use crate::providers::Sourced;
use crate::universe::{self, Issuer};

/// How far back `corporate_actions` reaches when no date is given.
pub const ACTIONS_DEFAULT_DAYS: u64 = 365;

/// One company's corporate actions, shareholders' meetings and listed
/// warrants and rights (IPOT) from `since` on (default: a year back),
/// upcoming events included.
pub async fn corporate_actions(
    fetcher: &Arc<Fetcher>,
    universe: &[Issuer],
    symbol: &Symbol,
    since: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<Sourced<CorporateActions>> {
    if symbol.is_index() {
        return Err(IdxError::Usage(format!(
            "{symbol} is an index; corporate actions belong to stocks"
        )));
    }
    let since = since.unwrap_or(today - chrono::Days::new(ACTIONS_DEFAULT_DAYS));
    let mut s = ipot::corporate_actions(fetcher, symbol, since).await?;
    s.data.name = universe
        .iter()
        .find(|i| &i.symbol == symbol)
        .and_then(|i| i.name.clone());
    Ok(s)
}

/// Provider name for data that ships inside the binary.
pub const BUNDLED: &str = "bundled";

/// The trading-holiday list for `year` (every listed year when `None`),
/// limited to dates from `today` on when `upcoming`.
pub fn holidays(year: Option<i32>, upcoming: bool, today: NaiveDate) -> Result<Sourced<Holidays>> {
    let covered = holidays::years();
    let mut warnings = Vec::new();
    if let Some(y) = year {
        if !(1990..=2100).contains(&y) {
            return Err(IdxError::Usage(format!("year {y} is out of range")));
        }
        if !holidays::covers(y) {
            warnings.push(format!(
                "no holiday list for {y}; covered: {}",
                covered
                    .iter()
                    .map(|y| y.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let list = holidays::all()
        .into_iter()
        .filter(|h| year.is_none_or(|y| h.date.year() == y))
        .filter(|h| !upcoming || h.date >= today)
        .collect();
    let mut s = Sourced::fresh(
        Holidays {
            year,
            trading_days: year
                .filter(|y| holidays::covers(*y))
                .map(holidays::trading_days),
            years_covered: covered,
            today,
            today_is_trading_day: holidays::is_trading_day(today),
            next_trading_day: holidays::next_trading_day(today),
            holidays: list,
        },
        BUNDLED,
    );
    s.warnings = warnings;
    Ok(s)
}

/// Trading days `latest_session` walks back over before giving up.
const SESSION_LOOKBACK: usize = 10;

/// Why `date` had no session, for messages.
fn closed_reason(date: NaiveDate) -> String {
    if let Some(h) = holidays::holiday(date) {
        format!("{date} is a trading holiday ({})", h.name)
    } else if matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
        format!("{date} is a {}", date.format("%A"))
    } else {
        format!("nothing was published for {date}")
    }
}

/// The day's list from `fetch`: for `date` when given, else for the latest
/// trading day at or before `today` that has one. Weekends and listed
/// holidays are skipped without a request. An empty explicit date is
/// `NotFound`.
async fn session<T, F, Fut>(
    date: Option<NaiveDate>,
    today: NaiveDate,
    fetch: F,
) -> Result<(NaiveDate, Sourced<Vec<T>>)>
where
    F: Fn(NaiveDate) -> Fut,
    Fut: Future<Output = Result<Sourced<Vec<T>>>>,
{
    if let Some(d) = date {
        if d > today {
            return Err(IdxError::Usage(format!("{d} is in the future")));
        }
        let s = fetch(d).await?;
        if s.data.is_empty() {
            return Err(IdxError::NotFound(format!(
                "no trading data: {}",
                closed_reason(d)
            )));
        }
        return Ok((d, s));
    }
    let mut day = holidays::last_trading_day(today);
    for _ in 0..SESSION_LOOKBACK {
        let s = fetch(day).await?;
        if !s.data.is_empty() {
            return Ok((day, s));
        }
        day = holidays::last_trading_day(day.pred_opt().expect("date in range"));
    }
    Err(IdxError::NotFound(format!(
        "no session published in the {SESSION_LOOKBACK} trading days to {today}"
    )))
}

fn matches_search(search: Option<&str>, code: &str, name: Option<&str>) -> bool {
    let Some(q) = search.map(str::trim).filter(|q| !q.is_empty()) else {
        return true;
    };
    let q = q.to_lowercase();
    code.to_lowercase().contains(&q) || name.is_some_and(|n| n.to_lowercase().contains(&q))
}

/// Listed stocks from idx.co.id, filtered by board and by a code or name
/// substring. When idx.co.id cannot be reached and nothing is cached, the
/// bundled universe stands in (codes and names only) with a warning.
pub async fn instruments(
    fetcher: &Arc<Fetcher>,
    board: Option<&str>,
    search: Option<&str>,
) -> Result<Sourced<InstrumentList>> {
    let mut s = match idx::securities(fetcher).await {
        Ok(s) => s,
        Err(e) => {
            let list: Vec<Instrument> = universe::load(None)?
                .into_iter()
                .map(|i| Instrument {
                    symbol: i.symbol,
                    name: i.name,
                    listing_date: None,
                    shares: None,
                    board: None,
                })
                .collect();
            let mut s = Sourced::fresh(list, BUNDLED);
            s.warnings.push(format!(
                "{e}; listing the bundled universe instead (codes and names only, no listing dates, shares or boards)"
            ));
            s
        }
    };
    let board = board.map(str::trim).filter(|b| !b.is_empty());
    let total = s.data.len();
    let mut list: Vec<Instrument> = std::mem::take(&mut s.data)
        .into_iter()
        .filter(|i| {
            board.is_none_or(|b| {
                i.board
                    .as_deref()
                    .is_some_and(|x| x.to_lowercase().contains(&b.to_lowercase()))
            })
        })
        .filter(|i| matches_search(search, i.symbol.as_str(), i.name.as_deref()))
        .collect();
    list.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    Ok(s.map(|_| InstrumentList {
        total,
        board: board.map(str::to_string),
        search: search
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(str::to_string),
        instruments: list,
    }))
}

/// Exchange members (broker codes) from idx.co.id, filtered by a code or
/// name substring; the bundled member list stands in when idx.co.id cannot
/// be reached and nothing is cached.
pub async fn members(fetcher: &Arc<Fetcher>, search: Option<&str>) -> Result<Sourced<MemberList>> {
    let mut s = match idx::members(fetcher).await {
        Ok(s) => s,
        Err(e) => {
            let list = brokers::all()
                .into_iter()
                .map(|(code, name)| Member {
                    code,
                    name: Some(name),
                    licenses: None,
                })
                .collect();
            let mut s = Sourced::fresh(list, BUNDLED);
            s.warnings.push(format!(
                "{e}; listing the bundled member list instead (no licences)"
            ));
            s
        }
    };
    let total = s.data.len();
    let mut list: Vec<Member> = std::mem::take(&mut s.data)
        .into_iter()
        .filter(|m| matches_search(search, &m.code, m.name.as_deref()))
        .collect();
    list.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(s.map(|_| MemberList {
        total,
        search: search
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(str::to_string),
        members: list,
    }))
}

/// Every index on `date` (default: the latest session), optionally only
/// the given codes (`LQ45`, `IDX30`; `IHSG` and `^JKSE` mean `COMPOSITE`).
pub async fn indices(
    fetcher: &Arc<Fetcher>,
    date: Option<NaiveDate>,
    codes: &[String],
    today: NaiveDate,
) -> Result<Sourced<IndexBoard>> {
    let (day, mut s) = session(date, today, |d| idx::index_summary(fetcher, d, today)).await?;
    let wanted: Vec<String> = codes
        .iter()
        .flat_map(|c| c.split(|ch: char| ch == ',' || ch.is_whitespace()))
        .map(|c| c.trim().to_ascii_uppercase())
        .filter(|c| !c.is_empty())
        .map(|c| match c.as_str() {
            "IHSG" | "^JKSE" | "JKSE" => "COMPOSITE".to_string(),
            _ => c,
        })
        .collect();
    let mut rows = std::mem::take(&mut s.data);
    if !wanted.is_empty() {
        for w in &wanted {
            if !rows.iter().any(|r| &r.code == w) {
                s.warnings.push(format!("no index {w} on {day}"));
            }
        }
        rows.retain(|r| wanted.contains(&r.code));
        if rows.is_empty() {
            return Err(IdxError::NotFound(format!(
                "none of {} on {day}",
                wanted.join(", ")
            )));
        }
    }
    Ok(s.map(|_| IndexBoard {
        date: day,
        indices: rows,
    }))
}

/// Longest range `calendar` covers in one call.
pub const CALENDAR_MAX_DAYS: i64 = 92;
/// Days `calendar` looks ahead when no end is given.
pub const CALENDAR_DEFAULT_DAYS: u32 = 14;

/// Which part of the exchange calendar to show.
#[derive(Debug, Clone, Default)]
pub struct CalendarQuery {
    /// One day; wins over `from`/`to`.
    pub date: Option<NaiveDate>,
    /// Start of the range; default today.
    pub from: Option<NaiveDate>,
    /// End of the range; default `from` plus `days`.
    pub to: Option<NaiveDate>,
    /// Look-ahead when `to` is not given; default [`CALENDAR_DEFAULT_DAYS`].
    pub days: Option<u32>,
    /// One issuer code (`BBCA`).
    pub symbol: Option<String>,
    /// Categories to keep; empty means [`CalendarCategory::DEFAULT`].
    pub categories: Vec<CalendarCategory>,
}

/// First day of every month that `from..=to` touches.
fn months(from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut m = from.with_day(1).expect("day 1 exists");
    while m <= to {
        out.push(m);
        m = m
            .checked_add_months(chrono::Months::new(1))
            .expect("date in range");
    }
    out
}

/// Exchange calendar entries (dividend and rights dates, shareholders'
/// meetings, public exposes, ...) over a day or a range, oldest first.
/// One request per day asked for, or per month the range touches.
pub async fn calendar(
    fetcher: &Arc<Fetcher>,
    q: &CalendarQuery,
    today: NaiveDate,
) -> Result<Sourced<Calendar>> {
    let (from, to) = match q.date {
        Some(d) => (d, d),
        None => {
            let from = q.from.unwrap_or(today);
            let days = q.days.unwrap_or(CALENDAR_DEFAULT_DAYS);
            let to =
                q.to.unwrap_or_else(|| from + chrono::Days::new(u64::from(days)));
            (from, to)
        }
    };
    if from > to {
        return Err(IdxError::Usage(format!(
            "the range starts ({from}) after it ends ({to})"
        )));
    }
    if (to - from).num_days() > CALENDAR_MAX_DAYS {
        return Err(IdxError::Usage(format!(
            "a calendar range covers at most {CALENDAR_MAX_DAYS} days"
        )));
    }
    let symbol = match q.symbol.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(crate::model::Symbol::parse(s)?.as_str().to_string()),
        None => None,
    };
    let ranges: Vec<CalendarRange> = if from == to {
        vec![CalendarRange::Day(from)]
    } else {
        months(from, to)
            .into_iter()
            .map(CalendarRange::Month)
            .collect()
    };
    let mut out: Option<Sourced<Vec<CalendarEvent>>> = None;
    for r in ranges {
        let s = idx::calendar(fetcher, r, symbol.as_deref()).await?;
        match &mut out {
            None => out = Some(s),
            Some(acc) => {
                acc.absorb(&s);
                acc.data.extend(s.data);
            }
        }
    }
    let mut s = out.expect("at least one range");
    let mut events: Vec<CalendarEvent> = std::mem::take(&mut s.data)
        .into_iter()
        .filter(|e| e.date >= from && e.date <= to)
        .collect();
    events.sort_by(|a, b| a.date.cmp(&b.date).then(a.id.cmp(&b.id)));
    // The source sometimes lists the same entry twice under two ids.
    let mut seen = std::collections::HashSet::new();
    events.retain(|e| seen.insert((e.date, e.code.clone(), e.description.clone())));
    let total = events.len();
    let categories = if q.categories.is_empty() {
        CalendarCategory::DEFAULT.to_vec()
    } else {
        q.categories.clone()
    };
    events.retain(|e| categories.contains(&e.category));
    Ok(s.map(|_| Calendar {
        from,
        to,
        symbol,
        categories,
        total,
        events,
    }))
}

/// Sessions `foreign_flow` shows by default.
pub const FOREIGN_DEFAULT_DAYS: usize = 20;
/// Most sessions `foreign_flow` fetches in one call (idx.co.id).
pub const FOREIGN_MAX_DAYS: usize = 250;
/// Most sessions `foreign_flow_ipot` reads: two requests each.
pub const FOREIGN_IPOT_MAX_SESSIONS: usize = 60;
/// Sessions `foreign_flow_ipot` reads at once.
const FOREIGN_IPOT_CONCURRENCY: usize = 4;

/// Which sessions a stock's foreign flow covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignRange {
    /// The latest `n` sessions.
    Latest(usize),
    /// Every session from `from` to `to`, both included.
    Dates { from: NaiveDate, to: NaiveDate },
}

impl ForeignRange {
    /// From the CLI's `--days`/`--from`/`--to` or the MCP and web
    /// `days`/`from`/`to`. Neither: the latest [`FOREIGN_DEFAULT_DAYS`]. `to`
    /// defaults to `today` and may not pass it; each source checks `days`
    /// against its own maximum.
    pub fn from_args(
        days: Option<usize>,
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
        today: NaiveDate,
    ) -> Result<Self> {
        if days.is_some() && (from.is_some() || to.is_some()) {
            return Err(IdxError::Usage(
                "give either the latest days or a from/to range, not both".into(),
            ));
        }
        match (from, to) {
            (None, None) => Ok(ForeignRange::Latest(days.unwrap_or(FOREIGN_DEFAULT_DAYS))),
            (None, Some(_)) => Err(IdxError::Usage(
                "to needs from, the first session of the range".into(),
            )),
            (Some(from), to) => {
                let to = to.unwrap_or(today);
                if from > to {
                    return Err(IdxError::Usage(format!(
                        "the range starts ({from}) after it ends ({to})"
                    )));
                }
                if to > today {
                    return Err(IdxError::Usage(format!(
                        "{to} is in the future (today is {today} in WIB)"
                    )));
                }
                Ok(ForeignRange::Dates { from, to })
            }
        }
    }
}

/// Trading days from `from` to `to` by the bundled holiday list, oldest first.
fn trading_days_between(from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    from.iter_days()
        .take_while(|d| *d <= to)
        .filter(|d| holidays::is_trading_day(*d))
        .collect()
}

fn stock_only(symbol: &Symbol) -> Result<()> {
    if symbol.is_index() {
        return Err(IdxError::Usage(format!(
            "{symbol} is an index; foreign flow is per stock (see `idx market foreign`)"
        )));
    }
    Ok(())
}

/// When idx.co.id refuses: the same question from Indo Premier, which
/// answers from anywhere, for at most [`FOREIGN_IPOT_MAX_SESSIONS`].
pub const IPOT_SOURCE_ADVICE: crate::error::Advice = crate::error::Advice {
    cli: "; or add --source ipot for Indo Premier's figures in rupiah (at most 60 sessions)",
    mcp: "; or call the tool again with source \"ipot\" for Indo Premier's figures in rupiah (at most 60 sessions)",
};

/// When idx.co.id refuses the market's foreign flow: the latest session's
/// foreign net per stock from Indo Premier's broker summaries.
pub const BANDAR_FNET_ADVICE: crate::error::Advice = crate::error::Advice {
    cli: "; or rank the most traded stocks by the latest session's foreign net in rupiah (Indo Premier) with idx bandar screen --period 1d --sort fnet",
    mcp: "; or rank the most traded stocks by the latest session's foreign net in rupiah (Indo Premier) with idx_bandar_screen, period \"1d\" and sort \"fnet\"",
};

/// idx.co.id refused the request and nothing was cached: add what else
/// answers the question. A Cloudflare refusal keeps its JSON code `blocked`.
fn with_alternative(e: IdxError, advice: crate::error::Advice) -> IdxError {
    match e {
        IdxError::Blocked {
            provider,
            refusal,
            message,
        } if provider == idx::NAME => IdxError::Blocked {
            provider,
            refusal,
            message: format!("{message}{}", advice.cli),
        },
        other => other,
    }
}

/// idx.co.id refused the request: point at the source that answers the same
/// question in rupiah from anywhere.
fn with_ipot_hint(e: IdxError) -> IdxError {
    with_alternative(e, IPOT_SOURCE_ADVICE)
}

/// `+1` net buying, `-1` net selling, `0` flat or unknown.
fn net_sign(net: Option<f64>) -> i32 {
    match net {
        Some(n) if n > 0.0 => 1,
        Some(n) if n < 0.0 => -1,
        _ => 0,
    }
}

/// Sessions on the latest session's side, in a row: `+3`, `-2`, or `0`
/// when the latest was flat.
fn streak(signs: &[i32]) -> i32 {
    match signs.last().copied().filter(|s| *s != 0) {
        Some(last) => signs.iter().rev().take_while(|s| **s == last).count() as i32 * last,
        None => 0,
    }
}

fn net(buy: Option<f64>, sell: Option<f64>) -> Option<f64> {
    Some(buy? - sell?)
}

fn at_close(net: Option<f64>, close: Option<f64>) -> Option<f64> {
    Some(net? * close?)
}

fn foreign_day(r: &StockDay) -> Option<ForeignDay> {
    let foreign_net = net(r.foreign_buy, r.foreign_sell);
    Some(ForeignDay {
        date: r.date()?,
        close: r.close,
        change: r.change,
        volume: r.volume,
        value: r.value,
        foreign_buy: r.foreign_buy,
        foreign_sell: r.foreign_sell,
        foreign_net,
        foreign_net_value_at_close: at_close(foreign_net, r.close),
    })
}

/// Sums and the same-side streak over `days` (oldest first).
pub fn foreign_totals(days: &[ForeignDay]) -> ForeignTotals {
    let sum = |f: fn(&ForeignDay) -> Option<f64>| days.iter().filter_map(f).sum::<f64>();
    let signs: Vec<i32> = days.iter().map(|d| net_sign(d.foreign_net)).collect();
    ForeignTotals {
        sessions: days.len(),
        foreign_buy: sum(|d| d.foreign_buy),
        foreign_sell: sum(|d| d.foreign_sell),
        foreign_net: sum(|d| d.foreign_net),
        foreign_net_value_at_close: sum(|d| d.foreign_net_value_at_close),
        net_buy_sessions: signs.iter().filter(|s| **s > 0).count(),
        net_sell_sessions: signs.iter().filter(|s| **s < 0).count(),
        streak: streak(&signs),
    }
}

/// Foreign buying and selling of one stock (idx.co.id daily stock summary),
/// oldest first: its latest sessions, or the sessions of a date range among
/// the latest [`FOREIGN_MAX_DAYS`].
pub async fn foreign_flow(
    fetcher: &Arc<Fetcher>,
    universe: &[Issuer],
    symbol: &Symbol,
    range: ForeignRange,
    today: NaiveDate,
) -> Result<Sourced<ForeignFlow>> {
    stock_only(symbol)?;
    let length = match range {
        ForeignRange::Latest(days) => {
            if days == 0 || days > FOREIGN_MAX_DAYS {
                return Err(IdxError::Usage(format!(
                    "days must be between 1 and {FOREIGN_MAX_DAYS}"
                )));
            }
            days
        }
        ForeignRange::Dates { from, .. } => {
            // The source serves the latest N sessions: enough to reach `from`,
            // with a little room for a closure the bundled list lacks.
            let sessions = trading_days_between(from, today).len();
            if sessions > FOREIGN_MAX_DAYS {
                return Err(IdxError::Usage(format!(
                    "idx.co.id serves the latest {FOREIGN_MAX_DAYS} sessions; {from} is older"
                )));
            }
            (sessions + 2).min(FOREIGN_MAX_DAYS)
        }
    };
    let (s, failure) = idx::trading_history(fetcher, symbol, length)
        .await
        .map_err(with_ipot_hint)?;
    let stock_name = s.data.iter().find_map(|r| r.stock_name.clone());
    let mut list: Vec<ForeignDay> = s.data.iter().filter_map(foreign_day).collect();
    list.sort_by_key(|d| d.date);
    list.dedup_by_key(|d| d.date);
    if let ForeignRange::Dates { from, to } = range {
        list.retain(|d| d.date >= from && d.date <= to);
        // A copy served for a failed fetch may stop before the range.
        if let Some(failure) = failure.filter(|_| list.is_empty()) {
            return Err(with_ipot_hint(
                failure.error(&format!("has no session of {symbol} from {from} to {to}")),
            ));
        }
        if list.is_empty() {
            return Err(IdxError::NotFound(format!(
                "idx.co.id has no sessions for {symbol} from {from} to {to}"
            )));
        }
    }
    let (Some(from), Some(to)) = (list.first().map(|d| d.date), list.last().map(|d| d.date)) else {
        return Err(IdxError::NotFound(format!(
            "no dated sessions for {symbol}"
        )));
    };
    let name = universe
        .iter()
        .find(|i| &i.symbol == symbol)
        .and_then(|i| i.name.clone())
        .or(stock_name);
    let totals = foreign_totals(&list);
    Ok(s.map(|_| ForeignFlow {
        symbol: symbol.clone(),
        name,
        from,
        to,
        days: list,
        totals,
    }))
}

/// Sums over `days` (oldest first), the foreign share of all their trading
/// and the same-side streak.
pub fn foreign_value_totals(days: &[ForeignValueDay]) -> ForeignValueTotals {
    let sum = |f: fn(&ForeignValueDay) -> Option<f64>| days.iter().filter_map(f).sum::<f64>();
    let signs: Vec<i32> = days.iter().map(|d| net_sign(d.foreign_net_value)).collect();
    // The share over the sessions that carry all three figures.
    let (mut sides, mut traded) = (0.0, 0.0);
    for d in days {
        if let (Some(b), Some(s), Some(v)) = (d.foreign_buy_value, d.foreign_sell_value, d.value) {
            sides += b + s;
            traded += v;
        }
    }
    ForeignValueTotals {
        sessions: days.len(),
        value: sum(|d| d.value),
        foreign_buy_value: sum(|d| d.foreign_buy_value),
        foreign_sell_value: sum(|d| d.foreign_sell_value),
        foreign_net_value: sum(|d| d.foreign_net_value),
        foreign_share_pct: (traded > 0.0).then(|| sides / (2.0 * traded) * 100.0),
        net_buy_sessions: signs.iter().filter(|s| **s > 0).count(),
        net_sell_sessions: signs.iter().filter(|s| **s < 0).count(),
        streak: streak(&signs),
    }
}

/// When the WIB clock passes this, today's session is over (post-trading
/// ends 16:15).
const SESSION_DONE: (u32, u32) = (16, 15);

type IpotDay = (NaiveDate, Result<Sourced<ipot::ForeignSession>>);

async fn ipot_days(
    fetcher: &Arc<Fetcher>,
    symbol: &Symbol,
    days: &[NaiveDate],
    today: NaiveDate,
) -> Vec<IpotDay> {
    stream::iter(days.iter().copied())
        .map(|d| async move { (d, ipot::foreign_day(fetcher, symbol, d, today).await) })
        .buffered(FOREIGN_IPOT_CONCURRENCY)
        .collect()
        .await
}

/// Foreign buying, selling and net of one stock in rupiah across all boards
/// (IPOT broker summary), oldest first: its latest sessions with trades, or
/// every session of a date range. Two single-session pages per trading day,
/// cached; a day without trades is skipped, a session the source has not
/// published yet and a day that fails are left out with a warning. When no
/// session with trades is left, a failure is the error (that day may have
/// traded), then a session not yet published. At most
/// [`FOREIGN_IPOT_MAX_SESSIONS`].
pub async fn foreign_flow_ipot(
    fetcher: &Arc<Fetcher>,
    universe: &[Issuer],
    symbol: &Symbol,
    range: ForeignRange,
    today: NaiveDate,
) -> Result<Sourced<ForeignValueFlow>> {
    stock_only(symbol)?;
    let max = FOREIGN_IPOT_MAX_SESSIONS;
    let mut got: Vec<IpotDay> = Vec::new();
    let (first, last) = match range {
        ForeignRange::Latest(n) => {
            if n == 0 || n > max {
                return Err(IdxError::Usage(format!(
                    "days must be between 1 and {max} with source ipot (two requests per session)"
                )));
            }
            // Newest first, back far enough for n sessions around holidays
            // and a suspension; each round asks only for what is missing.
            let oldest = today - chrono::Days::new(2 * n as u64 + 10);
            let mut candidates = trading_days_between(oldest, today);
            candidates.reverse();
            let (mut traded, mut next) = (0, 0);
            while traded < n && next < candidates.len() {
                let take = (n - traded).min(candidates.len() - next);
                let round = ipot_days(fetcher, symbol, &candidates[next..next + take], today).await;
                next += take;
                traded += round
                    .iter()
                    .filter(|(_, r)| {
                        matches!(r, Ok(s) if matches!(s.data, ipot::ForeignSession::Traded { .. }))
                    })
                    .count();
                let stop = round
                    .iter()
                    .any(|(_, r)| matches!(r, Err(IdxError::NotFound(_))))
                    || round.iter().all(|(_, r)| r.is_err()) && !got.iter().any(|(_, r)| r.is_ok());
                got.extend(round);
                // An unknown code, or nothing but failures so far: no use
                // walking further back.
                if stop {
                    break;
                }
            }
            (oldest, today)
        }
        ForeignRange::Dates { from, to } => {
            let days = trading_days_between(from, to.min(today));
            if days.len() > max {
                return Err(IdxError::Usage(format!(
                    "a foreign flow from ipot covers at most {max} sessions; {from} to {to} has {}",
                    days.len()
                )));
            }
            got = ipot_days(fetcher, symbol, &days, today).await;
            (from, to)
        }
    };

    let mut prov: Option<Sourced<()>> = None;
    let mut warnings = Vec::new();
    let mut first_err: Option<IdxError> = None;
    let mut pending: Option<NaiveDate> = None;
    let mut list: Vec<ForeignValueDay> = Vec::new();
    let mut rounded = false;
    for (date, r) in got {
        match r {
            Ok(s) => {
                if let Some(p) = &mut prov {
                    p.absorb(&s);
                }
                match &s.data {
                    ipot::ForeignSession::Traded { day, rounded: r } => {
                        rounded |= r;
                        list.push(day.clone());
                    }
                    ipot::ForeignSession::Pending => {
                        pending = pending.max(Some(date));
                        warnings.push(if date >= today {
                            format!("{symbol}: today's session ({date}) is left out: the source publishes the foreign figures after the close")
                        } else {
                            format!("{symbol}: the session of {date} is left out: the source shows its totals but has not published its broker lists and foreign figures yet")
                        });
                    }
                    ipot::ForeignSession::NoTrades => {}
                }
                if prov.is_none() {
                    prov = Some(s.map(|_| ()));
                }
            }
            Err(e @ IdxError::NotFound(_)) => return Err(e),
            Err(e) => {
                warnings.push(format!("{symbol}: the session of {date} failed: {e}"));
                first_err.get_or_insert(e);
            }
        }
    }
    let Some(mut s) = prov.filter(|_| !list.is_empty()) else {
        // A session that failed may have traded: say it failed, not that
        // nothing traded.
        if let Some(e) = first_err {
            return Err(e);
        }
        return Err(IdxError::NotFound(match pending {
            Some(date) if date >= today => format!(
                "no published session with trades for {symbol} from {first} to {last}: today's session ({date}) is published after the close"
            ),
            Some(date) => format!(
                "no published session with trades for {symbol} from {first} to {last}: the source has not published the session of {date} yet"
            ),
            None => format!("no sessions with trades for {symbol} from {first} to {last}"),
        }));
    };
    list.sort_by_key(|d| d.date);
    if let ForeignRange::Latest(n) = range {
        let extra = list.len().saturating_sub(n);
        list.drain(..extra);
        if list.len() < n {
            warnings.push(format!(
                "{symbol}: {} sessions with trades from {first} to {last}, fewer than the {n} asked for",
                list.len()
            ));
        }
    }
    let (from, to) = (list[0].date, list[list.len() - 1].date);
    let now = chrono::Utc::now().with_timezone(&crate::model::wib());
    let done = chrono::NaiveTime::from_hms_opt(SESSION_DONE.0, SESSION_DONE.1, 0).expect("time");
    if to == today && now.date_naive() == today && now.time() < done {
        warnings.push(format!(
            "{symbol}: today's session ({today}) is still trading; its figures will change"
        ));
    }
    for w in warnings {
        if !s.warnings.contains(&w) {
            s.warnings.push(w);
        }
    }
    let name = universe
        .iter()
        .find(|i| &i.symbol == symbol)
        .and_then(|i| i.name.clone());
    let totals = foreign_value_totals(&list);
    Ok(s.map(|_| ForeignValueFlow {
        symbol: symbol.clone(),
        name,
        from,
        to,
        days: list,
        totals,
        rounded,
    }))
}

fn foreign_row(r: &StockDay) -> Option<ForeignRow> {
    let foreign_net = net(r.foreign_buy, r.foreign_sell);
    Some(ForeignRow {
        symbol: Symbol::parse(&r.stock_code).ok()?,
        name: r.stock_name.clone(),
        close: r.close,
        change: r.change,
        change_pct: match (r.change, r.previous) {
            (Some(c), Some(p)) if p != 0.0 => Some(c / p * 100.0),
            _ => None,
        },
        volume: r.volume,
        value: r.value,
        foreign_buy: r.foreign_buy,
        foreign_sell: r.foreign_sell,
        foreign_net,
        foreign_net_value_at_close: at_close(foreign_net, r.close),
    })
}

/// Foreign net buying and selling across every stock for one session
/// (default: the latest published), the `top` largest each way (0 = all).
pub async fn market_foreign(
    fetcher: &Arc<Fetcher>,
    date: Option<NaiveDate>,
    top: usize,
    sort: ForeignSort,
    today: NaiveDate,
) -> Result<Sourced<MarketForeign>> {
    let (day, mut s) = session(date, today, |d| idx::stock_summary(fetcher, d, today))
        .await
        .map_err(|e| with_alternative(e, BANDAR_FNET_ADVICE))?;
    let rows: Vec<ForeignRow> = std::mem::take(&mut s.data)
        .iter()
        .filter_map(foreign_row)
        .collect();
    let key = |r: &ForeignRow| match sort {
        ForeignSort::Value => r.foreign_net_value_at_close,
        ForeignSort::Shares => r.foreign_net,
    };
    let sum = |f: fn(&ForeignRow) -> Option<f64>| rows.iter().filter_map(f).sum::<f64>();
    let mut buy: Vec<ForeignRow> = rows
        .iter()
        .filter(|r| key(r).is_some_and(|k| k > 0.0))
        .cloned()
        .collect();
    buy.sort_by(|a, b| {
        key(b)
            .partial_cmp(&key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut sell: Vec<ForeignRow> = rows
        .iter()
        .filter(|r| key(r).is_some_and(|k| k < 0.0))
        .cloned()
        .collect();
    sell.sort_by(|a, b| {
        key(a)
            .partial_cmp(&key(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if top > 0 {
        buy.truncate(top);
        sell.truncate(top);
    }
    let out = MarketForeign {
        date: day,
        stocks: rows.len(),
        foreign_buy: sum(|r| r.foreign_buy),
        foreign_sell: sum(|r| r.foreign_sell),
        foreign_net: sum(|r| r.foreign_net),
        foreign_net_value_at_close: sum(|r| r.foreign_net_value_at_close),
        sort,
        net_buy: buy,
        net_sell: sell,
    };
    Ok(s.map(|_| out))
}

/// The ranking of one session's `rows`: totals over all of them, each
/// member's share of all members' value, the members ranked by `sort`
/// (largest first; ties keep the order the rows came in) and the `top`
/// first kept (0 = all).
fn member_ranking(
    date: NaiveDate,
    mut rows: Vec<BrokerDayRow>,
    sort: BrokerSort,
    top: usize,
    rounded: bool,
) -> BrokerRanking {
    let total = |f: fn(&BrokerDayRow) -> Option<f64>| rows.iter().filter_map(f).sum::<f64>();
    let total_value = total(|r| r.value);
    let total_volume = total(|r| r.volume);
    let total_frequency = total(|r| r.frequency);
    let key = |r: &BrokerDayRow| {
        match sort {
            BrokerSort::Value => r.value,
            BrokerSort::Volume => r.volume,
            BrokerSort::Frequency => r.frequency,
        }
        .unwrap_or(f64::NEG_INFINITY)
    };
    rows.sort_by(|a, b| {
        key(b)
            .partial_cmp(&key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (i, r) in rows.iter_mut().enumerate() {
        r.rank = i as u32 + 1;
        r.value_share_pct = r
            .value
            .filter(|_| total_value > 0.0)
            .map(|v| v / total_value * 100.0);
    }
    let members = rows.len();
    if top > 0 {
        rows.truncate(top);
    }
    BrokerRanking {
        date,
        members,
        total_volume,
        total_value,
        total_frequency,
        sort,
        rounded,
        brokers: rows,
    }
}

/// `true` when `s`, idx.co.id's broker summary of `day`, does not show that
/// the trading day had no session: it has no rows and was read on `day`
/// itself (a copy keeps when it was fetched; an answer read now is read
/// `today`), when idx.co.id may not have published the session yet. Only
/// for a session that has ended by `ended`.
fn read_unpublished(
    s: &Sourced<Vec<idx::BrokerDay>>,
    day: NaiveDate,
    today: NaiveDate,
    ended: NaiveDate,
) -> bool {
    let read = if s.cached {
        s.fetched_at
            .with_timezone(&crate::model::wib())
            .date_naive()
    } else {
        today
    };
    s.data.is_empty() && day <= ended && holidays::is_trading_day(day) && read <= day
}

/// idx.co.id's broker summary of `day` as [`market_brokers`] walks to it.
/// Before idx.co.id publishes a session its summary is an empty list, so a
/// summary read on `day` itself without rows does not show that a trading
/// day had no session ([`read_unpublished`]). Once that session has ended
/// (`ended`), such a copy is read again: the cache keeps a past day's
/// summary for a closed session's 30 days, so it is asked for with today's
/// short lifetime instead. An empty answer read so is returned for the walk
/// to tell from a day without a session. A copy served because it could
/// not be read again (the live fetch failed, or offline) is an error rather
/// than a day without trading: the walk does not skip a session it could
/// not read.
async fn broker_day(
    fetcher: &Arc<Fetcher>,
    day: NaiveDate,
    today: NaiveDate,
    ended: NaiveDate,
) -> Result<Sourced<Vec<idx::BrokerDay>>> {
    let read = |s: &Sourced<Vec<idx::BrokerDay>>| s.fetched_at.with_timezone(&crate::model::wib());
    let early = |s: &Sourced<Vec<idx::BrokerDay>>| read_unpublished(s, day, today, ended);
    let mut s = idx::broker_summary(fetcher, day, today).await?;
    if early(&s) && s.cached && !s.stale && day < today {
        s = idx::broker_summary(fetcher, day, day).await?;
    }
    if !early(&s) || !s.stale {
        return Ok(s);
    }
    let clock = read(&s).format("%H:%M");
    if s.stale_reason == Some(crate::model::StaleReason::Offline) {
        return Err(IdxError::OfflineMiss(format!(
            "the broker summary for {day} is cached only as read at {clock} WIB that day, with no rows, which it has until idx.co.id publishes the session"
        )));
    }
    let why = s
        .warnings
        .iter()
        .find_map(|w| crate::providers::fetch_failed_cause(w))
        .unwrap_or("the live fetch failed");
    Err(IdxError::provider(
        idx::NAME,
        format!(
            "the broker summary for {day} could not be read again ({why}); the copy read at {clock} WIB that day has no rows, which it has until idx.co.id publishes the session"
        ),
    ))
}

/// Every exchange member's trading for one session (default: the latest
/// published), ranked by value, volume or frequency; the `top` first (0 =
/// all). Each member's figures count both its buying and its selling.
/// `today` counts as after its close; [`market_brokers_from`] knows the
/// hour (see [`members_until`]).
pub async fn market_brokers(
    fetcher: &Arc<Fetcher>,
    date: Option<NaiveDate>,
    top: usize,
    sort: BrokerSort,
    today: NaiveDate,
) -> Result<Sourced<BrokerRanking>> {
    let ended = holidays::last_trading_day(today);
    members_until(fetcher, date, top, sort, today, ended).await
}

/// [`market_brokers`] with `ended`, the latest session that has ended.
/// idx.co.id publishes a session's summary after its close, so with no
/// `date` the walk back starts there: before the close the day's own
/// summary is not asked for, and a refusal of that request cannot hide the
/// session before it. A session that could not be read is not skipped
/// either (see [`broker_day`]), and one idx.co.id has not published yet is
/// named: a walk past it says so, and a `date` of it is not found as such.
/// A `date` without a session (a weekend or a listed holiday) is not found
/// without a request, whatever a copy of it or idx.co.id would say.
async fn members_until(
    fetcher: &Arc<Fetcher>,
    date: Option<NaiveDate>,
    top: usize,
    sort: BrokerSort,
    today: NaiveDate,
    ended: NaiveDate,
) -> Result<Sourced<BrokerRanking>> {
    let (walked, unpublished) = idx_walk(fetcher, date, top, sort, today, ended).await;
    idx_answer(walked, date, unpublished, None)
}

/// idx.co.id's walk for [`members_until`]: the ranking, or the error that
/// ended the walk, and the session it found unpublished on the way (see
/// [`read_unpublished`]), if any.
async fn idx_walk(
    fetcher: &Arc<Fetcher>,
    date: Option<NaiveDate>,
    top: usize,
    sort: BrokerSort,
    today: NaiveDate,
    ended: NaiveDate,
) -> (Result<Sourced<BrokerRanking>>, Option<NaiveDate>) {
    if let Some(d) = date.filter(|d| *d <= today && !holidays::is_trading_day(*d)) {
        let none = IdxError::NotFound(format!("no trading data: {}", closed_reason(d)));
        return (Err(none), None);
    }
    let from = match date {
        Some(_) => today,
        None => ended,
    };
    let unpublished = std::sync::Mutex::new(None);
    let walked = session(date, from, |d| {
        let unpublished = &unpublished;
        async move {
            let s = broker_day(fetcher, d, today, ended).await?;
            if read_unpublished(&s, d, today, ended) {
                unpublished.lock().expect("not poisoned").get_or_insert(d);
            }
            Ok(s)
        }
    })
    .await;
    let unpublished = unpublished.into_inner().expect("not poisoned");
    let ranking = walked.map(|(day, mut s)| {
        let mut rows: Vec<BrokerDayRow> = std::mem::take(&mut s.data)
            .into_iter()
            .map(|r| BrokerDayRow {
                rank: 0,
                code: r.code.trim().to_ascii_uppercase(),
                name: r
                    .name
                    .map(|n| n.trim().to_string())
                    .filter(|n| !n.is_empty()),
                volume: r.volume,
                value: r.value,
                frequency: r.frequency,
                value_share_pct: None,
                buy_value: None,
                sell_value: None,
                net_value: None,
                kind: None,
            })
            .collect();
        // Members with the same figure rank by code.
        rows.sort_by(|a, b| a.code.cmp(&b.code));
        s.map(|_| member_ranking(day, rows, sort, top, false))
    });
    (ranking, unpublished)
}

/// idx.co.id's answer from [`idx_walk`], naming the session `unpublished`
/// when there is one: a walk past it to the session before says so in a
/// warning, a `date` of it is not found as unpublished, and an error that
/// ended the walk after it says it was passed. `note`, when given (why
/// IPOT could not stand in), follows.
fn idx_answer(
    walked: Result<Sourced<BrokerRanking>>,
    date: Option<NaiveDate>,
    unpublished: Option<NaiveDate>,
    note: Option<&str>,
) -> Result<Sourced<BrokerRanking>> {
    let after = note.map(|n| format!("; {n}")).unwrap_or_default();
    let Some(u) = unpublished else {
        return walked.map_err(|e| match note {
            Some(n) => with_note(e, n),
            None => e,
        });
    };
    match walked {
        Ok(mut s) => {
            let shown = s.data.date;
            s.warnings.insert(
                0,
                format!(
                    "idx.co.id has not published {u} yet; showing the session before, {shown}{after}"
                ),
            );
            Ok(s)
        }
        Err(IdxError::NotFound(_)) if date == Some(u) => Err(IdxError::NotFound(format!(
            "idx.co.id has not published its broker summary for {u} yet{after}"
        ))),
        Err(e) => Err(with_note(
            e,
            &format!("idx.co.id has not published {u} yet{after}"),
        )),
    }
}

/// IPOT's ranking as [`market_brokers`] gives idx.co.id's: members without
/// trades are left out, as idx.co.id leaves them out, and a code with trades
/// that is not on the IDX member list stays, named in a warning.
fn ipot_member_ranking(
    mut s: Sourced<ipot::topbroker::Ranking>,
    top: usize,
    sort: BrokerSort,
) -> Result<Sourced<BrokerRanking>> {
    let date = s.data.date;
    let mut unlisted = Vec::new();
    let rows: Vec<BrokerDayRow> = std::mem::take(&mut s.data.brokers)
        .into_iter()
        .filter(|b| b.traded())
        .map(|b| {
            if brokers::name(&b.code).is_none() {
                unlisted.push(match &b.name {
                    Some(n) => format!("{} ({n})", b.code),
                    None => b.code.clone(),
                });
            }
            BrokerDayRow {
                rank: 0,
                code: b.code,
                name: b.name,
                volume: b.volume,
                value: b.value,
                frequency: b.frequency,
                value_share_pct: None,
                buy_value: b.buy_value,
                sell_value: b.sell_value,
                net_value: b.net_value,
                kind: b.kind,
            }
        })
        .collect();
    if rows.is_empty() {
        return Err(IdxError::NotFound(format!(
            "IPOT's broker ranking for {date} has no trades yet"
        )));
    }
    if !unlisted.is_empty() {
        s.warnings.push(format!(
            "IPOT lists trades for codes that are not on the IDX member list: {}",
            unlisted.join(", ")
        ));
    }
    let rounded = s.data.rounded;
    Ok(s.map(|_| member_ranking(date, rows, sort, top, rounded)))
}

/// [`market_brokers`] from IPOT's ranking of its latest session
/// ([`ipot::top_brokers`]): the same members and totals, with each member's
/// value bought and sold, its net and its class, and figures the source
/// abbreviated to two decimals (`rounded`). Past sessions cannot be asked
/// for: a `date` before IPOT's session is a usage error. A day without a
/// session and one whose session has not ended are not found without a
/// request, as is a session after IPOT's, which it has not published yet.
pub async fn market_brokers_ipot(
    fetcher: &Arc<Fetcher>,
    date: Option<NaiveDate>,
    top: usize,
    sort: BrokerSort,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Sourced<BrokerRanking>> {
    let today = now.with_timezone(&crate::model::wib()).date_naive();
    if let Some(d) = date {
        if d > today {
            return Err(IdxError::Usage(format!("{d} is in the future")));
        }
        if !holidays::is_trading_day(d) {
            return Err(IdxError::NotFound(format!(
                "no trading data: {}",
                closed_reason(d)
            )));
        }
        if d > holidays::latest_session(now) {
            return Err(IdxError::NotFound(format!(
                "the session of {d} has not ended (it ends at 16:15 WIB), and IPOT's ranking of it cannot be dated before 19:00 WIB"
            )));
        }
    }
    let s = ipot::top_brokers(fetcher, now).await?;
    let held = s.data.date;
    match date {
        Some(d) if d < held => Err(IdxError::Usage(format!(
            "IPOT holds only its latest session, {held}, not {d}; source idx (idx.co.id) has the others"
        ))),
        Some(d) if d > held => Err(IdxError::NotFound(format!(
            "IPOT has not published {d} yet: its latest session is {held}; source idx (idx.co.id) publishes each session after its close"
        ))),
        _ => ipot_member_ranking(s, top, sort),
    }
}

/// `e` with `note` after its message; its code and exit stay.
fn with_note(e: IdxError, note: &str) -> IdxError {
    match e {
        IdxError::Blocked {
            provider,
            refusal,
            message,
        } => IdxError::Blocked {
            provider,
            refusal,
            message: format!("{message}; {note}"),
        },
        IdxError::Provider { provider, message } => IdxError::Provider {
            provider,
            message: format!("{message}; {note}"),
        },
        IdxError::Network(m) => IdxError::Network(format!("{m}; {note}")),
        IdxError::OfflineMiss(m) => IdxError::OfflineMiss(format!("{m}; {note}")),
        other => other,
    }
}

/// The member ranking from `source`. `auto` asks idx.co.id and, when it
/// refuses (code `blocked`), cannot be reached (`network`), fails
/// (`provider`), offline has no copy (`offline_miss`), or has not
/// published the session yet, answers from IPOT with a warning saying so,
/// if IPOT holds the session asked for: `date`, or with none the latest
/// session that has ended. When IPOT cannot stand in (another session, such
/// as an old copy served offline or after a failed fetch, or a failure),
/// idx.co.id's answer stands with IPOT's reason after it.
pub async fn market_brokers_from(
    fetcher: &Arc<Fetcher>,
    source: crate::model::BrokerSource,
    date: Option<NaiveDate>,
    top: usize,
    sort: BrokerSort,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Sourced<BrokerRanking>> {
    use crate::model::BrokerSource;
    let entered = chrono::Utc::now();
    let today = now.with_timezone(&crate::model::wib()).date_naive();
    let ended = holidays::latest_session(now);
    match source {
        BrokerSource::Idx => members_until(fetcher, date, top, sort, today, ended).await,
        BrokerSource::Ipot => market_brokers_ipot(fetcher, date, top, sort, now).await,
        BrokerSource::Auto => {
            let (walked, unpublished) = idx_walk(fetcher, date, top, sort, today, ended).await;
            // The session asked for: with no date, the latest that ended,
            // where idx.co.id's walk begins. IPOT stands in for it when
            // idx.co.id refused, could not be reached, failed or, offline,
            // had no copy, or has not published it yet.
            let want = date.unwrap_or(ended);
            let unpublished = unpublished.filter(|u| *u == want);
            let failed = matches!(
                walked,
                Err(IdxError::Blocked { .. }
                    | IdxError::Provider { .. }
                    | IdxError::Network(_)
                    | IdxError::OfflineMiss(_))
            );
            if !failed && unpublished.is_none() {
                return idx_answer(walked, date, None, None);
            }
            // IPOT is asked now, after idx.co.id's requests (and retries):
            // its pages are judged by this clock, not the call's.
            let asked = now + (chrono::Utc::now() - entered);
            let s = match ipot::top_brokers(fetcher, asked).await {
                Ok(s) => s,
                Err(e) => {
                    let note = format!("IPOT could not stand in: {e}");
                    return idx_answer(walked, date, unpublished, Some(&note));
                }
            };
            let held = s.data.date;
            // A copy of an older session is not the one asked for.
            if held != want {
                return idx_answer(walked, date, unpublished, Some(&ipot_holds_only(&s)));
            }
            let why = match (&walked, unpublished) {
                (_, Some(u)) => format!("idx.co.id has not published {u} yet"),
                (Err(IdxError::Blocked { refusal, .. }), None) => {
                    format!("idx.co.id answered with {}", refusal.noun())
                }
                (Err(IdxError::Network(m)), None) => {
                    format!("idx.co.id could not be reached ({m})")
                }
                (Err(IdxError::OfflineMiss(_)), None) => format!(
                    "offline, and no usable copy of idx.co.id's broker summary for {want} is cached"
                ),
                (Err(other), None) => format!("idx.co.id failed ({other})"),
                (Ok(_), None) => "idx.co.id has not answered it".to_string(),
            };
            // A copy of an older session than the latest is no latest session.
            let copy = copy_read_at(&s).filter(|_| held < ended);
            let mut s = match ipot_member_ranking(s, top, sort) {
                Ok(s) => s,
                Err(e) => {
                    let note = format!("IPOT could not stand in: {e}");
                    return idx_answer(walked, date, unpublished, Some(&note));
                }
            };
            let whose = match copy {
                Some(read) => format!("for {held}, from its copy read at {read}"),
                None => format!("for its latest session, {held}"),
            };
            s.warnings.insert(
                0,
                format!("{why}, so the ranking is IPOT's (Indo Premier) {whose}"),
            );
            Ok(s)
        }
    }
}

/// When a copy of IPOT's ranking served in place of a read (offline, or
/// after a failed fetch) was read, as `10:00 WIB on 2026-10-17`; `None`
/// for pages read now, or fresh in the cache.
fn copy_read_at<T>(s: &Sourced<T>) -> Option<String> {
    s.stale.then(|| {
        s.fetched_at
            .with_timezone(&crate::model::wib())
            .format("%H:%M WIB on %Y-%m-%d")
            .to_string()
    })
}

/// What IPOT holds when it cannot stand in for the session asked for, for
/// the note after idx.co.id's error: its latest session, or, for a copy
/// served in place of a read, the session that copy holds and, after a
/// failed fetch, why IPOT could not be read.
fn ipot_holds_only(s: &Sourced<ipot::topbroker::Ranking>) -> String {
    let held = s.data.date;
    let Some(read) = copy_read_at(s) else {
        return format!("IPOT, which stands in, holds only its latest session, {held}");
    };
    match s
        .warnings
        .iter()
        .find_map(|w| crate::providers::fetch_failed_cause(w))
    {
        Some(why) => format!(
            "IPOT, which stands in, could not be read ({why}), and its copy read at {read} holds only {held}"
        ),
        None => format!("IPOT's copy, read at {read}, holds only {held}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn holidays_for_a_year_with_trading_facts() {
        let s = holidays(Some(2026), false, d(2026, 10, 18)).unwrap();
        assert_eq!(s.provider, "bundled");
        assert_eq!(s.data.holidays.len(), 22);
        assert_eq!(s.data.trading_days, Some(239));
        assert!(!s.data.today_is_trading_day, "Sunday");
        assert_eq!(s.data.next_trading_day, d(2026, 10, 19));
        assert!(s.warnings.is_empty());

        let up = holidays(Some(2026), true, d(2026, 10, 18)).unwrap();
        let dates: Vec<_> = up.data.holidays.iter().map(|h| h.date).collect();
        assert_eq!(dates, [d(2026, 12, 24), d(2026, 12, 25), d(2026, 12, 31)]);
    }

    #[test]
    fn uncovered_year_warns_and_has_no_count() {
        let s = holidays(Some(2030), false, d(2026, 10, 18)).unwrap();
        assert!(s.data.holidays.is_empty());
        assert_eq!(s.data.trading_days, None);
        assert!(s.warnings[0].contains("no holiday list for 2030"));
        assert!(holidays(Some(10_000), false, d(2026, 10, 18)).is_err());
        let all = holidays(None, false, d(2026, 10, 18)).unwrap();
        assert_eq!(all.data.year, None);
        assert_eq!(all.data.trading_days, None);
        assert!(all.data.holidays.len() > 30);
    }

    use crate::cache::Cache;
    use crate::http::{CacheMode, FixtureBackend};

    const SECURITIES: &str = include_str!("../../tests/fixtures/idx/securities_excerpt.json");
    const MEMBERS: &str = include_str!("../../tests/fixtures/idx/brokersearch.json");
    const INDICES: &str = include_str!("../../tests/fixtures/idx/indexsummary_20261016.json");
    const NO_TRADING: &str =
        include_str!("../../tests/fixtures/idx/stocksummary_20261018_no_trading.json");

    pub(super) fn fetcher(
        be: FixtureBackend,
    ) -> (Arc<Fetcher>, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(be);
        let f = Arc::new(Fetcher::new(
            be.clone(),
            Cache::new(dir.path()),
            CacheMode::Normal,
        ));
        (f, be, dir)
    }

    #[tokio::test]
    async fn instruments_filter_by_board_and_search() {
        let (f, _be, _d) =
            fetcher(FixtureBackend::new().route("GetSecuritiesStock", 200, SECURITIES));
        let s = instruments(&f, Some("akselerasi"), None).await.unwrap();
        assert_eq!(s.provider, "idx");
        assert!(s.data.total > s.data.instruments.len());
        assert!(s
            .data
            .instruments
            .iter()
            .all(|i| i.board.as_deref() == Some("Akselerasi")));
        let s = instruments(&f, None, Some("central asia")).await.unwrap();
        let codes: Vec<_> = s
            .data
            .instruments
            .iter()
            .map(|i| i.symbol.as_str())
            .collect();
        assert_eq!(codes, ["BBCA"]);
        assert_eq!(s.data.search.as_deref(), Some("central asia"));
    }

    #[tokio::test]
    async fn directories_fall_back_to_the_bundled_lists() {
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route("GetSecuritiesStock", 403, "blocked")
                .route("GetBrokerSearch", 403, "blocked"),
        );
        let s = instruments(&f, None, Some("BBCA")).await.unwrap();
        assert_eq!(s.provider, "bundled");
        assert!(s.warnings[0].contains("Cloudflare"), "{:?}", s.warnings);
        assert_eq!(s.data.instruments[0].symbol.as_str(), "BBCA");
        assert_eq!(s.data.instruments[0].board, None);

        let m = members(&f, Some("cc")).await.unwrap();
        assert_eq!(m.provider, "bundled");
        assert!(m.data.members.iter().any(|x| x.code == "CC"));
        assert!(m.data.members.iter().all(|x| x.licenses.is_none()));
        assert_eq!(m.data.total, 89);
    }

    #[tokio::test]
    async fn members_from_idx_carry_licences() {
        let (f, _be, _d) = fetcher(FixtureBackend::new().route("GetBrokerSearch", 200, MEMBERS));
        let m = members(&f, None).await.unwrap();
        assert_eq!(m.data.members.len(), 89);
        assert!(m.data.members.windows(2).all(|w| w[0].code < w[1].code));
        assert!(m.data.members.iter().all(|x| x.licenses.is_some()));
    }

    #[tokio::test]
    async fn indices_walk_back_to_the_latest_session_and_filter_codes() {
        // Sunday 18 Oct: Saturday and Sunday are skipped without a request;
        // Friday 16 Oct has the data.
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route("GetIndexSummary?date=20261016", 200, INDICES)
                .route("GetIndexSummary", 200, NO_TRADING),
        );
        let s = indices(
            &f,
            None,
            &["lq45, ihsg".to_string(), "NOPE".to_string()],
            d(2026, 10, 18),
        )
        .await
        .unwrap();
        assert_eq!(s.data.date, d(2026, 10, 16));
        let codes: Vec<_> = s.data.indices.iter().map(|r| r.code.as_str()).collect();
        assert_eq!(codes, ["COMPOSITE", "LQ45"]);
        assert!(s.warnings.iter().any(|w| w.contains("NOPE")));
        assert_eq!(be.call_count(), 1);

        // Monday 19 Oct before the summary is out: falls back to Friday.
        let s = indices(&f, None, &[], d(2026, 10, 19)).await.unwrap();
        assert_eq!(s.data.date, d(2026, 10, 16));
        assert_eq!(s.data.indices.len(), 45);
        assert_eq!(be.calls_matching("date=20261019"), 1);

        let err = indices(&f, Some(d(2026, 8, 17)), &[], d(2026, 10, 19))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(err.to_string().contains("Proklamasi"), "{err}");
        let err = indices(&f, Some(d(2026, 10, 20)), &[], d(2026, 10, 19))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    const MONTH: &str = include_str!("../../tests/fixtures/idx/calendar_m_20261001_excerpt.json");
    const DAY: &str = include_str!("../../tests/fixtures/idx/calendar_d_20261019.json");

    #[tokio::test]
    async fn calendar_ranges_filter_by_date_and_default_categories() {
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route("range=m&date=20261001", 200, MONTH)
                .route("range=m&date=20261101", 200, "{\"Results\":[]}")
                .route("range=d&date=20261019", 200, DAY),
        );
        let q = CalendarQuery {
            from: Some(d(2026, 10, 15)),
            to: Some(d(2026, 10, 31)),
            ..Default::default()
        };
        let s = calendar(&f, &q, d(2026, 10, 18)).await.unwrap();
        assert_eq!(s.provider, "idx");
        assert_eq!(be.call_count(), 1, "one month");
        assert!(
            s.data.total > s.data.events.len(),
            "bonds and buybacks hidden"
        );
        assert!(s
            .data
            .events
            .iter()
            .all(|e| e.date >= d(2026, 10, 15) && e.date <= d(2026, 10, 31)));
        assert!(s.data.events.iter().all(|e| !matches!(
            e.category,
            CalendarCategory::Bond | CalendarCategory::Buyback
        )));
        assert!(s.data.events.windows(2).all(|w| w[0].date <= w[1].date));

        // Every category, across two months: one request per month.
        let q = CalendarQuery {
            from: Some(d(2026, 10, 20)),
            days: Some(20),
            categories: vec![CalendarCategory::Bond],
            ..Default::default()
        };
        let s = calendar(&f, &q, d(2026, 10, 18)).await.unwrap();
        assert_eq!(s.data.to, d(2026, 11, 9));
        assert!(!s.data.events.is_empty());
        assert!(s
            .data
            .events
            .iter()
            .all(|e| e.category == CalendarCategory::Bond));
        assert_eq!(be.calls_matching("date=20261101"), 1);

        // One day asks for the day.
        let q = CalendarQuery {
            date: Some(d(2026, 10, 19)),
            categories: CalendarCategory::ALL.to_vec(),
            ..Default::default()
        };
        let s = calendar(&f, &q, d(2026, 10, 18)).await.unwrap();
        assert_eq!(s.data.events.len(), 8);
        assert_eq!((s.data.from, s.data.to), (d(2026, 10, 19), d(2026, 10, 19)));
    }

    #[tokio::test]
    async fn calendar_passes_the_symbol_and_checks_the_range() {
        let (f, be, _d) =
            fetcher(FixtureBackend::new().route("GetCalendar", 200, "{\"Results\":[]}"));
        let q = CalendarQuery {
            symbol: Some("bbca".into()),
            ..Default::default()
        };
        let s = calendar(&f, &q, d(2026, 10, 18)).await.unwrap();
        assert_eq!(s.data.symbol.as_deref(), Some("BBCA"));
        assert!(s.data.events.is_empty());
        assert!(be.calls.lock().unwrap()[0].ends_with("&code=BBCA"));

        let too_long = CalendarQuery {
            from: Some(d(2026, 1, 1)),
            to: Some(d(2026, 6, 1)),
            ..Default::default()
        };
        assert_eq!(
            calendar(&f, &too_long, d(2026, 10, 18))
                .await
                .unwrap_err()
                .exit_code(),
            2
        );
        let backwards = CalendarQuery {
            from: Some(d(2026, 10, 20)),
            to: Some(d(2026, 10, 1)),
            ..Default::default()
        };
        assert!(calendar(&f, &backwards, d(2026, 10, 18)).await.is_err());
    }

    const ACTIONS: &str =
        include_str!("../../tests/fixtures/ipot/corporateaction_enrg_20250101.html");

    #[tokio::test]
    async fn corporate_actions_default_to_a_year_back_and_fill_the_name() {
        let (f, be, _d) = fetcher(FixtureBackend::new().route("corporateaction.php", 200, ACTIONS));
        let list = universe::load(None).unwrap();
        let sym = Symbol::parse("ENRG").unwrap();
        let s = corporate_actions(&f, &list, &sym, None, d(2026, 10, 18))
            .await
            .unwrap();
        assert_eq!(s.data.since, d(2025, 10, 18));
        assert!(be.calls.lock().unwrap()[0].ends_with("start=10/18/2025"));
        assert!(s.data.name.as_deref().unwrap().contains("ENERGI MEGA"));
        assert_eq!(s.data.warrants_rights[0].code, "ENRG-R");
        let idx = Symbol::parse("^JKSE").unwrap();
        assert!(corporate_actions(&f, &list, &idx, None, d(2026, 10, 18))
            .await
            .is_err());
    }

    const HISTORY: &str = include_str!("../../tests/fixtures/idx/tradinginfoss_bbca_excerpt.json");
    const STOCKS: &str =
        include_str!("../../tests/fixtures/idx/stocksummary_20261016_excerpt.json");

    #[tokio::test]
    async fn foreign_flow_is_oldest_first_with_totals_and_streak() {
        let (f, be, _d) = fetcher(FixtureBackend::new().route("GetTradingInfoSS", 200, HISTORY));
        let list = universe::load(None).unwrap();
        let sym = Symbol::parse("BBCA").unwrap();
        let today = d(2026, 10, 18);
        let s = foreign_flow(&f, &list, &sym, ForeignRange::Latest(30), today)
            .await
            .unwrap();
        assert!(be.calls.lock().unwrap()[0].contains("code=BBCA&start=0&length=30"));
        let flow = &s.data;
        assert_eq!(flow.days.len(), 30);
        assert_eq!(flow.to, d(2026, 10, 16));
        assert_eq!(flow.from, d(2026, 9, 7));
        assert!(flow.days.windows(2).all(|w| w[0].date < w[1].date));
        let last = flow.days.last().unwrap();
        assert_eq!(last.foreign_net, Some(309_392_800.0 - 757_227_900.0));
        assert_eq!(
            last.foreign_net_value_at_close,
            Some((309_392_800.0 - 757_227_900.0) * 5175.0)
        );
        let t = &flow.totals;
        assert_eq!(t.sessions, 30);
        assert_eq!(t.net_buy_sessions + t.net_sell_sessions, 30);
        let net: f64 = flow.days.iter().filter_map(|d| d.foreign_net).sum();
        assert!((t.foreign_net - net).abs() < 1e-3);
        assert!(t.streak < 0, "the last session was net selling");
        assert!(flow.name.as_deref().unwrap().contains("CENTRAL ASIA"));

        assert!(
            foreign_flow(&f, &list, &sym, ForeignRange::Latest(0), today)
                .await
                .is_err()
        );
        let index = Symbol::parse("^JKSE").unwrap();
        assert!(
            foreign_flow(&f, &list, &index, ForeignRange::Latest(5), today)
                .await
                .is_err()
        );

        // A range: enough of the latest sessions to reach `from`, then cut.
        let range = ForeignRange::Dates {
            from: d(2026, 10, 1),
            to: d(2026, 10, 9),
        };
        let s = foreign_flow(&f, &list, &sym, range, d(2026, 10, 16))
            .await
            .unwrap();
        // 12 trading days from 10-01 to 10-16, and two to spare.
        assert!(be.calls.lock().unwrap()[1].contains("code=BBCA&start=0&length=14"));
        assert_eq!((s.data.from, s.data.to), (d(2026, 10, 1), d(2026, 10, 9)));
        assert_eq!(s.data.days.len(), 7);
        assert_eq!(s.data.totals.sessions, 7);
        let old = ForeignRange::Dates {
            from: d(2025, 1, 2),
            to: d(2025, 1, 31),
        };
        let err = foreign_flow(&f, &list, &sym, old, today).await.unwrap_err();
        assert!(err.to_string().contains("latest 250 sessions"), "{err}");
        let empty = ForeignRange::Dates {
            from: d(2026, 10, 17),
            to: d(2026, 10, 18),
        };
        let err = foreign_flow(&f, &list, &sym, empty, today)
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
    }

    #[tokio::test]
    async fn a_blocked_idx_co_id_points_at_ipot() {
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route("GetTradingInfoSS", 403, "no")
                .route("GetStockSummary", 403, "no"),
        );
        let sym = Symbol::parse("BBCA").unwrap();
        let err = foreign_flow(&f, &[], &sym, ForeignRange::Latest(5), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert_eq!(err.code(), "blocked");
        assert!(err.to_string().contains("Cloudflare"), "{err}");
        assert!(
            err.to_string().ends_with(
                "; or add --source ipot for Indo Premier's figures in rupiah (at most 60 sessions)"
            ),
            "{err}"
        );
        // The market's foreign flow: the bandar screen's foreign net.
        let err = market_foreign(&f, None, 10, ForeignSort::Value, d(2026, 10, 16))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "blocked");
        assert!(
            err.to_string()
                .ends_with("with idx bandar screen --period 1d --sort fnet"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn the_ipot_advice_names_the_sessions_it_covers() {
        let (f, _be, _d) = fetcher(FixtureBackend::new().route("GetTradingInfoSS", 403, "no"));
        let sym = Symbol::parse("BBCA").unwrap();
        // More sessions than source ipot takes: the advice says how many it does.
        let err = foreign_flow(&f, &[], &sym, ForeignRange::Latest(120), d(2026, 10, 18))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "blocked");
        let limit = format!("at most {FOREIGN_IPOT_MAX_SESSIONS} sessions");
        assert!(err.to_string().contains(&limit), "{err}");
        assert!(IPOT_SOURCE_ADVICE.mcp.contains(&limit));
    }

    #[tokio::test]
    async fn a_refused_day_with_an_empty_copy_is_not_a_day_without_trading() {
        // 16 Oct after the close: today's summaries were cached empty two
        // hours ago, before they were out; the 15th's are cached; idx.co.id
        // now refuses today's.
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route("GetBrokerSummary?date=20261016", 403, "no")
                .route("GetStockSummary?date=20261016", 403, "no"),
        );
        let url = |endpoint: &str, date: &str| {
            format!("https://www.idx.co.id/primary/TradingSummary/{endpoint}?date={date}&start=0&length=9999")
        };
        let two_hours_ago = chrono::Utc::now() - chrono::Duration::seconds(7200);
        for endpoint in ["GetBrokerSummary", "GetStockSummary"] {
            f.cache()
                .put_at(&url(endpoint, "20261016"), 200, NO_TRADING, two_hours_ago)
                .unwrap();
        }
        f.cache()
            .put(&url("GetBrokerSummary", "20261015"), 200, BROKERS)
            .unwrap();
        let today = d(2026, 10, 16);
        // Not the 15th as if the 16th had no session, nor not_found for the
        // 16th: the refusal, as with nothing cached.
        for date in [None, Some(today)] {
            let err = market_brokers(&f, date, 5, BrokerSort::Value, today)
                .await
                .unwrap_err();
            assert_eq!((err.code(), err.exit_code()), ("blocked", 3), "{err}");
            assert!(err.to_string().contains(" WIB is empty. "), "{err}");
        }
        assert_eq!(be.calls_matching("date=20261015"), 0);
        // The market's foreign flow too, with its alternative.
        let err = market_foreign(&f, None, 10, ForeignSort::Value, today)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "blocked");
        assert!(
            err.to_string()
                .ends_with("with idx bandar screen --period 1d --sort fnet"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_refused_history_whose_copy_misses_the_range_is_no_empty_range() {
        // 19 Oct after the close: BBCA's history was cached two hours ago,
        // its newest session the 16th; idx.co.id now refuses.
        let (f, _be, _d) = fetcher(FixtureBackend::new().route("GetTradingInfoSS", 403, "no"));
        let url = |length: usize| {
            format!("https://www.idx.co.id/primary/ListedCompany/GetTradingInfoSS?code=BBCA&start=0&length={length}")
        };
        let two_hours_ago = chrono::Utc::now() - chrono::Duration::seconds(7200);
        for length in [3, 5] {
            f.cache()
                .put_at(&url(length), 200, HISTORY, two_hours_ago)
                .unwrap();
        }
        let sym = Symbol::parse("BBCA").unwrap();
        let today = d(2026, 10, 19);
        let range = |from| ForeignRange::Dates { from, to: today };
        // The 19th is not in the copy: the refusal, not "no sessions".
        let err = foreign_flow(&f, &[], &sym, range(today), today)
            .await
            .unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("blocked", 3), "{err}");
        let msg = err.to_string();
        assert!(
            msg.contains(" WIB has no session of BBCA from 2026-10-19 to 2026-10-19. "),
            "{msg}"
        );
        assert!(msg.ends_with(IPOT_SOURCE_ADVICE.cli), "{msg}");
        // A range the copy reaches is served from it, marked stale.
        let s = foreign_flow(&f, &[], &sym, range(d(2026, 10, 15)), today)
            .await
            .unwrap();
        assert!(s.stale && s.warnings.len() == 1, "{:?}", s.warnings);
        assert_eq!((s.data.from, s.data.to), (d(2026, 10, 15), d(2026, 10, 16)));
    }

    #[test]
    fn foreign_ranges_from_the_arguments() {
        let today = d(2026, 11, 6);
        let r = |days, from, to| ForeignRange::from_args(days, from, to, today);
        assert_eq!(
            r(None, None, None).unwrap(),
            ForeignRange::Latest(FOREIGN_DEFAULT_DAYS)
        );
        assert_eq!(r(Some(5), None, None).unwrap(), ForeignRange::Latest(5));
        assert_eq!(
            r(None, Some(d(2026, 11, 2)), None).unwrap(),
            ForeignRange::Dates {
                from: d(2026, 11, 2),
                to: today
            }
        );
        for (days, from, to, says) in [
            (Some(3), Some(d(2026, 11, 2)), None, "not both"),
            (Some(3), None, Some(d(2026, 11, 2)), "not both"),
            (None, None, Some(d(2026, 11, 2)), "to needs from"),
            (None, Some(d(2026, 11, 5)), Some(d(2026, 11, 2)), "starts"),
            (
                None,
                Some(d(2026, 1, 1)),
                Some(d(2099, 1, 1)),
                "2099-01-01 is in the future (today is 2026-11-06 in WIB)",
            ),
        ] {
            let err = r(days, from, to).unwrap_err();
            assert_eq!(err.exit_code(), 2);
            assert!(err.to_string().contains(says), "{err}");
        }
        // A holiday and a weekend cost no request.
        assert_eq!(
            trading_days_between(d(2026, 8, 14), d(2026, 8, 18)),
            [d(2026, 8, 14), d(2026, 8, 18)]
        );
    }

    /// BBCA's ten pages of 10-12..10-16 (all investors and foreign), the two
    /// of 11-02..11-05, and ZZZZ unknown.
    fn ipot_sessions() -> FixtureBackend {
        let mut be = FixtureBackend::new().route(
            "code=ZZZZ",
            200,
            include_str!("../../tests/fixtures/ipot/brokersummary_zzzz_invalid.html"),
        );
        for day in [
            "20261012", "20261013", "20261014", "20261015", "20261016", "20261102", "20261103",
            "20261104", "20261105",
        ] {
            let mdy = format!("{}/{}/{}", &day[4..6], &day[6..8], &day[..4]);
            let file = |suffix: &str| {
                std::fs::read_to_string(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/ipot")
                        .join(format!("brokersummary_bbca_{day}{suffix}.html")),
                )
                .unwrap()
            };
            be = be
                .route(
                    &format!("code=BBCA&start={mdy}&end={mdy}&fd=all&"),
                    200,
                    &file(""),
                )
                .route(
                    &format!("code=BBCA&start={mdy}&end={mdy}&fd=F&"),
                    200,
                    &file("_foreign"),
                );
        }
        be
    }

    #[tokio::test]
    async fn foreign_flow_from_ipot_in_rupiah() {
        let (f, be, _d) = fetcher(ipot_sessions());
        let list = universe::load(None).unwrap();
        let sym = Symbol::parse("BBCA").unwrap();
        let week = ForeignRange::Dates {
            from: d(2026, 10, 12),
            to: d(2026, 10, 16),
        };
        let s = foreign_flow_ipot(&f, &list, &sym, week, d(2026, 10, 17))
            .await
            .unwrap();
        assert_eq!(s.provider, "ipot");
        let fl = &s.data;
        assert!(fl.name.as_deref().unwrap().contains("CENTRAL ASIA"));
        assert_eq!((fl.from, fl.to), (d(2026, 10, 12), d(2026, 10, 16)));
        assert_eq!(fl.days.len(), 5);
        assert!(fl.rounded);
        assert!(fl.days.windows(2).all(|w| w[0].date < w[1].date));
        let t = &fl.totals;
        // Only single sessions: the week's own page would say -4.1 T.
        assert_eq!(t.foreign_buy_value, 4_364_700_000_000.0);
        assert_eq!(t.foreign_sell_value, 6_187_800_000_000.0);
        assert_eq!(t.foreign_net_value, -1_823_100_000_000.0);
        assert_eq!(t.value, 9_937_300_000_000.0);
        let share = t.foreign_share_pct.unwrap();
        assert!((share - 53.095).abs() < 0.001, "{share}");
        assert_eq!(
            (t.net_buy_sessions, t.net_sell_sessions, t.streak),
            (3, 2, -1)
        );
        assert_eq!(be.call_count(), 10, "two pages per session");
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);

        // 11-02..11-05 read before the open on 11-06.
        let nov = ForeignRange::Dates {
            from: d(2026, 11, 2),
            to: d(2026, 11, 5),
        };
        let t = foreign_flow_ipot(&f, &list, &sym, nov, d(2026, 11, 6))
            .await
            .unwrap()
            .data
            .totals;
        assert_eq!(t.foreign_buy_value, 3_314_700_000_000.0);
        assert_eq!(t.foreign_net_value, 292_300_000_000.0);
        assert_eq!(t.foreign_sell_value, 3_022_400_000_000.0);
        assert_eq!(t.value, 4_848_300_000_000.0);
        assert!((t.foreign_share_pct.unwrap() - 65.35).abs() < 0.01);
        assert_eq!(t.streak, 2);
    }

    #[tokio::test]
    async fn a_session_still_trading_is_left_out_with_a_warning() {
        let be = ipot_sessions()
            .route(
                "code=BBCA&start=11/06/2026&end=11/06/2026&fd=all&",
                200,
                include_str!(
                    "../../tests/fixtures/ipot/brokersummary_bbca_20261106_insession.html"
                ),
            )
            .route(
                "code=BBCA&start=11/06/2026&end=11/06/2026&fd=F&",
                200,
                include_str!(
                    "../../tests/fixtures/ipot/brokersummary_bbca_20261106_insession_foreign.html"
                ),
            );
        let (f, _be, _d) = fetcher(be);
        let sym = Symbol::parse("BBCA").unwrap();
        let s = foreign_flow_ipot(&f, &[], &sym, ForeignRange::Latest(3), d(2026, 11, 6))
            .await
            .unwrap();
        let dates: Vec<NaiveDate> = s.data.days.iter().map(|x| x.date).collect();
        assert_eq!(dates, [d(2026, 11, 3), d(2026, 11, 4), d(2026, 11, 5)]);
        assert!(
            s.warnings.iter().any(|w| w.contains(
                "today's session (2026-11-06) is left out: the source publishes the foreign figures after the close"
            )),
            "{:?}",
            s.warnings
        );

        // Only today, still trading: said so, not "no sessions with trades".
        let only_today = ForeignRange::Dates {
            from: d(2026, 11, 6),
            to: d(2026, 11, 6),
        };
        let err = foreign_flow_ipot(&f, &[], &sym, only_today, d(2026, 11, 6))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(
            err.to_string().contains(
                "no published session with trades for BBCA from 2026-11-06 to 2026-11-06: today's session (2026-11-06) is published after the close"
            ),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_past_session_not_yet_published_is_left_out_with_a_warning() {
        let sym = Symbol::parse("BBCA").unwrap();
        let day5 = d(2026, 11, 5);
        // 11-05 as if read before the source published it: totals, no broker
        // lists (the shape of 11-06 at 09:15 WIB).
        let be = ipot_sessions().route(
            &ipot::url(
                &sym,
                day5,
                day5,
                crate::model::BrokerInvestor::All,
                crate::model::BrokerBoard::All,
            ),
            200,
            include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261106_insession.html"),
        );
        let (f, be, _d) = fetcher(be);
        let range = ForeignRange::Dates {
            from: d(2026, 11, 4),
            to: day5,
        };
        let s = foreign_flow_ipot(&f, &[], &sym, range, d(2026, 11, 6))
            .await
            .unwrap();
        let dates: Vec<NaiveDate> = s.data.days.iter().map(|x| x.date).collect();
        assert_eq!(dates, [d(2026, 11, 4)]);
        assert!(
            s.warnings.iter().any(|w| w
                == "BBCA: the session of 2026-11-05 is left out: the source shows its totals but has not published its broker lists and foreign figures yet"),
            "{:?}",
            s.warnings
        );
        assert_eq!(be.calls_matching("start=11/05/2026&end=11/05/2026&fd=F"), 0);

        let only = ForeignRange::Dates {
            from: day5,
            to: day5,
        };
        let err = foreign_flow_ipot(&f, &[], &sym, only, d(2026, 11, 6))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(
            err.to_string()
                .contains("the source has not published the session of 2026-11-05 yet"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn fewer_latest_ipot_sessions_than_asked_is_a_warning() {
        // Every BBCA day besides the recorded ones has no trades.
        let (f, _be, _d) = fetcher(ipot_sessions().route(
            "code=BBCA&start=",
            200,
            include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261017_no_trades.html"),
        ));
        let sym = Symbol::parse("BBCA").unwrap();
        let s = foreign_flow_ipot(&f, &[], &sym, ForeignRange::Latest(7), d(2026, 10, 17))
            .await
            .unwrap();
        assert_eq!(s.data.days.len(), 5);
        assert!(
            s.warnings.iter().any(|w| w
                == "BBCA: 5 sessions with trades from 2026-09-23 to 2026-10-17, fewer than the 7 asked for"),
            "{:?}",
            s.warnings
        );
    }

    #[tokio::test]
    async fn the_latest_ipot_sessions_walk_back_past_the_weekend() {
        let (f, be, _d) = fetcher(ipot_sessions());
        let sym = Symbol::parse("BBCA").unwrap();
        // Saturday: the weekend costs nothing, Friday to Wednesday do.
        let s = foreign_flow_ipot(&f, &[], &sym, ForeignRange::Latest(3), d(2026, 10, 17))
            .await
            .unwrap();
        let dates: Vec<NaiveDate> = s.data.days.iter().map(|x| x.date).collect();
        assert_eq!(dates, [d(2026, 10, 14), d(2026, 10, 15), d(2026, 10, 16)]);
        assert_eq!(be.call_count(), 6);
        assert_eq!(s.data.totals.sessions, 3);

        let err = foreign_flow_ipot(&f, &[], &sym, ForeignRange::Latest(61), d(2026, 10, 17))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 2);
        let long = ForeignRange::Dates {
            from: d(2026, 6, 1),
            to: d(2026, 10, 16),
        };
        let err = foreign_flow_ipot(&f, &[], &sym, long, d(2026, 10, 17))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("at most 60 sessions"), "{err}");
        assert_eq!(be.call_count(), 6, "usage errors cost no request");
    }

    #[tokio::test]
    async fn a_failed_ipot_session_is_a_warning_and_an_unknown_code_not_found() {
        let (f, _be, _d) = fetcher(ipot_sessions());
        let sym = Symbol::parse("BBCA").unwrap();
        // 10-09 has no recorded page: the network fails for it.
        let range = ForeignRange::Dates {
            from: d(2026, 10, 9),
            to: d(2026, 10, 12),
        };
        let s = foreign_flow_ipot(&f, &[], &sym, range, d(2026, 10, 17))
            .await
            .unwrap();
        assert_eq!(s.data.days.len(), 1);
        assert!(
            s.warnings[0].starts_with("BBCA: the session of 2026-10-09 failed"),
            "{:?}",
            s.warnings
        );
        let zzzz = Symbol::parse("ZZZZ").unwrap();
        let err = foreign_flow_ipot(&f, &[], &zzzz, ForeignRange::Latest(3), d(2026, 10, 17))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        // Nothing read at all: the failure itself.
        let range = ForeignRange::Dates {
            from: d(2026, 9, 1),
            to: d(2026, 9, 2),
        };
        let err = foreign_flow_ipot(&f, &[], &sym, range, d(2026, 10, 17))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 3);

        // 10-08 read without trades, 10-09 failed: the failure, since that
        // day may have traded.
        let (f, _be, _d) = fetcher(ipot_sessions().route(
            "code=BBCA&start=10/08/2026&end=10/08/2026&",
            200,
            include_str!("../../tests/fixtures/ipot/brokersummary_bbca_20261017_no_trades.html"),
        ));
        let range = ForeignRange::Dates {
            from: d(2026, 10, 8),
            to: d(2026, 10, 9),
        };
        let err = foreign_flow_ipot(&f, &[], &sym, range, d(2026, 10, 17))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 3, "{err}");
    }

    #[test]
    fn ipot_totals_count_present_values_only() {
        let day = |net: Option<f64>, value: Option<f64>| ForeignValueDay {
            date: d(2026, 10, 1),
            value,
            lots: None,
            average_price: None,
            foreign_buy_value: Some(10.0),
            foreign_sell_value: net.map(|n| 10.0 - n),
            foreign_net_value: net,
            foreign_share_pct: None,
        };
        let t = foreign_value_totals(&[
            day(Some(-2.0), Some(20.0)),
            day(Some(4.0), Some(20.0)),
            day(None, Some(20.0)),
        ]);
        assert_eq!(t.sessions, 3);
        assert_eq!(t.value, 60.0);
        assert_eq!(t.foreign_net_value, 2.0);
        // (10 + 12 + 10 + 6) / (2 * 40): the third session has no selling.
        assert_eq!(t.foreign_share_pct, Some(38.0 / 80.0 * 100.0));
        assert_eq!(t.streak, 0, "the latest session has no net");
        assert_eq!(foreign_value_totals(&[]).foreign_share_pct, None);
    }

    #[test]
    fn streak_counts_the_latest_run_only() {
        let day = |n: f64| ForeignDay {
            date: d(2026, 10, 1),
            close: None,
            change: None,
            volume: None,
            value: None,
            foreign_buy: None,
            foreign_sell: None,
            foreign_net: Some(n),
            foreign_net_value_at_close: None,
        };
        let t = foreign_totals(&[day(-1.0), day(2.0), day(3.0), day(1.0)]);
        assert_eq!(t.streak, 3);
        assert_eq!((t.net_buy_sessions, t.net_sell_sessions), (3, 1));
        assert_eq!(foreign_totals(&[day(2.0), day(0.0)]).streak, 0);
        assert_eq!(foreign_totals(&[]).streak, 0);
    }

    #[tokio::test]
    async fn market_foreign_ranks_both_ways() {
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route("GetStockSummary?date=20261016", 200, STOCKS)
                .route("GetStockSummary", 200, NO_TRADING),
        );
        let s = market_foreign(&f, None, 5, ForeignSort::Value, d(2026, 10, 18))
            .await
            .unwrap();
        let m = &s.data;
        assert_eq!(m.date, d(2026, 10, 16));
        assert_eq!(m.stocks, 28);
        assert_eq!((m.net_buy.len(), m.net_sell.len()), (5, 5));
        let v = |r: &ForeignRow| r.foreign_net_value_at_close.unwrap();
        assert!(m.net_buy.windows(2).all(|w| v(&w[0]) >= v(&w[1])));
        assert!(m.net_sell.windows(2).all(|w| v(&w[0]) <= v(&w[1])));
        assert!(m.net_buy.iter().all(|r| v(r) > 0.0));
        assert!(m.net_sell.iter().all(|r| v(r) < 0.0));
        let bbca = m.net_sell.iter().find(|r| r.symbol.as_str() == "BBCA");
        assert!(
            bbca.is_some(),
            "BBCA was among the heaviest foreign selling"
        );
        let pct = bbca.unwrap().change_pct.unwrap();
        assert!((pct - (-900.0 / 6075.0 * 100.0)).abs() < 1e-9);

        let by_shares = market_foreign(&f, None, 0, ForeignSort::Shares, d(2026, 10, 18))
            .await
            .unwrap();
        let n = |r: &ForeignRow| r.foreign_net.unwrap();
        assert!(by_shares
            .data
            .net_buy
            .windows(2)
            .all(|w| n(&w[0]) >= n(&w[1])));
        assert_eq!(by_shares.data.sort, ForeignSort::Shares);
    }

    const TOP_BROKERS: &str = include_str!("../../tests/fixtures/ipot/topbroker_20261106.html");
    const BBCA_STATISTIC: &str =
        include_str!("../../tests/fixtures/ipot/statistic_bbca_20261106.html");
    const CHALLENGE: &str =
        include_str!("../../tests/fixtures/idx/cloudflare_challenge_synthetic.html");

    /// Sunday 2026-11-08 10:00 WIB: IPOT's latest session is 2026-11-06.
    fn sunday() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-11-08T10:00:00+07:00")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    /// A WIB wall-clock time.
    fn wib_at(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(&format!("{s}+07:00"))
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    /// IPOT's ranking `page` dated by BBCA's statistics of 2026-11-06.
    fn ipot_ranking(be: FixtureBackend, page: &str) -> FixtureBackend {
        be.route("ma-topbroker.php", 200, page).route(
            "statistic.php?code=BBCA",
            200,
            BBCA_STATISTIC,
        )
    }

    #[tokio::test]
    async fn market_brokers_from_ipot_count_as_idx_co_id_does() {
        use crate::model::{BrokerKind, BrokerSource};
        let (f, be, _d) = fetcher(ipot_ranking(FixtureBackend::new(), TOP_BROKERS));
        let s = market_brokers_from(&f, BrokerSource::Ipot, None, 0, BrokerSort::Value, sunday())
            .await
            .unwrap();
        assert_eq!(s.provider, "ipot");
        let r = &s.data;
        assert_eq!(r.date, d(2026, 11, 6));
        // idx.co.id's GetBrokerSummary of the session, read the same morning:
        // 88 members, 2,205,570 trades, IDR 20,648,010,571,340.
        assert_eq!((r.members, r.brokers.len()), (88, 88));
        assert_eq!(r.total_frequency, 2_205_570.0);
        assert!(
            (r.total_value / 20_648_010_571_340.0 - 1.0).abs() < 1e-4,
            "{}",
            r.total_value
        );
        assert!(r.rounded);
        let cp = &r.brokers[0];
        assert_eq!(
            (cp.rank, cp.code.as_str(), cp.kind),
            (1, "CP", Some(BrokerKind::Local))
        );
        assert_eq!(
            (cp.buy_value, cp.sell_value, cp.net_value),
            (Some(1.4e12), Some(1.39e12), Some(4.77e9))
        );
        let cc = r.brokers.iter().find(|b| b.code == "CC").unwrap();
        assert_eq!(cc.frequency, Some(211_212.0), "JB is counted in CC");
        assert!(r
            .brokers
            .iter()
            .all(|b| !matches!(b.code.as_str(), "JB" | "PS" | "ZZ")));
        let share: f64 = r.brokers.iter().filter_map(|b| b.value_share_pct).sum();
        assert!((share - 100.0).abs() < 1e-6);
        assert_eq!(
            s.warnings,
            ["JB ([CC] BJB Sekuritas Jawa Barat) is counted in CC, as idx.co.id counts it; IPOT lists it apart"]
        );
        assert_eq!(be.call_count(), 2);

        // By trades, which the source gives exact.
        let by = market_brokers_from(
            &f,
            BrokerSource::Ipot,
            Some(d(2026, 11, 6)),
            5,
            BrokerSort::Frequency,
            sunday(),
        )
        .await
        .unwrap();
        let fr = |x: &BrokerDayRow| x.frequency.unwrap();
        assert_eq!((by.data.members, by.data.brokers.len()), (88, 5));
        assert!(by.data.brokers.windows(2).all(|w| fr(&w[0]) >= fr(&w[1])));
        assert_eq!(by.data.brokers[0].code, "XL");
        assert_eq!(be.call_count(), 2, "from the cache");

        // IPOT holds one session; a date in the future costs no request.
        for (date, says) in [
            (
                d(2026, 10, 16),
                "IPOT holds only its latest session, 2026-11-06, not 2026-10-16",
            ),
            (d(2026, 11, 9), "2026-11-09 is in the future"),
        ] {
            let err = market_brokers_from(
                &f,
                BrokerSource::Ipot,
                Some(date),
                0,
                BrokerSort::Value,
                sunday(),
            )
            .await
            .unwrap_err();
            assert_eq!(err.exit_code(), 2);
            assert!(err.to_string().contains(says), "{err}");
        }
        assert_eq!(be.call_count(), 2);
    }

    #[tokio::test]
    async fn a_session_ipot_cannot_hold_yet_is_not_found() {
        use crate::model::BrokerSource;
        let (f, be, dir) = fetcher(ipot_ranking(FixtureBackend::new(), TOP_BROKERS));
        let ipot = |date, now| {
            market_brokers_from(
                &f,
                BrokerSource::Ipot,
                Some(date),
                0,
                BrokerSort::Value,
                now,
            )
        };
        // A weekend, and a session that has not ended: no request.
        for (date, now, says) in [
            (d(2026, 11, 7), sunday(), "no trading data: 2026-11-07 is a Saturday"),
            (d(2026, 11, 8), sunday(), "no trading data: 2026-11-08 is a Sunday"),
            (
                d(2026, 11, 9),
                wib_at("2026-11-09T08:00:00"),
                "the session of 2026-11-09 has not ended (it ends at 16:15 WIB), and IPOT's ranking of it cannot be dated before 19:00 WIB",
            ),
        ] {
            let err = ipot(date, now).await.unwrap_err();
            assert_eq!((err.code(), err.exit_code()), ("not_found", 4), "{date}");
            assert!(err.to_string().contains(says), "{err}");
        }
        assert_eq!(be.call_count(), 0);

        // Monday night both of IPOT's pages still show Friday. A date page
        // naming a session older than Monday's dates nothing on its own.
        let monday_night = wib_at("2026-11-09T19:30:00");
        let err = ipot(d(2026, 11, 9), monday_night).await.unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("provider", 3));
        assert!(err.to_string().contains("so it cannot be dated"), "{err}");
        // Once Friday's ranking has been accepted, on Sunday, the same pair
        // is Friday's: Monday is not there yet.
        let live = Arc::new(Fetcher::new(
            Arc::new(ipot_ranking(FixtureBackend::new(), TOP_BROKERS)),
            Cache::new(dir.path()),
            CacheMode::Bypass,
        ));
        let ipot_live = |date, now| {
            market_brokers_from(&live, BrokerSource::Ipot, date, 0, BrokerSort::Value, now)
        };
        ipot_live(None, sunday()).await.unwrap();
        let err = ipot_live(Some(d(2026, 11, 9)), monday_night)
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(
            err.to_string().contains(
                "IPOT has not published 2026-11-09 yet: its latest session is 2026-11-06"
            ),
            "{err}"
        );
        // A session before IPOT's stays a usage error: it never holds one.
        let err = ipot(d(2026, 11, 5), sunday()).await.unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[tokio::test]
    async fn auto_answers_from_ipot_when_idx_co_id_refuses() {
        use crate::model::BrokerSource;
        let be = FixtureBackend::new().route_with_headers(
            "GetBrokerSummary",
            403,
            CHALLENGE,
            &[("cf-mitigated", "challenge"), ("server", "cloudflare")],
        );
        let (f, be, _d) = fetcher(ipot_ranking(be, TOP_BROKERS));
        let auto = |date| {
            market_brokers_from(&f, BrokerSource::Auto, date, 5, BrokerSort::Value, sunday())
        };
        // The latest session: idx.co.id's 11-06 is challenged, IPOT stands in.
        let s = auto(None).await.unwrap();
        assert_eq!(s.provider, "ipot");
        assert_eq!(
            (s.data.date, s.data.members, s.data.brokers.len()),
            (d(2026, 11, 6), 88, 5)
        );
        assert_eq!(
            s.warnings[0],
            "idx.co.id answered with a Cloudflare challenge, so the ranking is IPOT's (Indo Premier) for its latest session, 2026-11-06"
        );
        // Asked first (how often a challenge is asked again is idx's own).
        assert!(be.calls_matching("GetBrokerSummary?date=20261106") > 0);
        assert_eq!(auto(Some(d(2026, 11, 6))).await.unwrap().provider, "ipot");

        // Another session: idx.co.id's refusal stands, saying why.
        let err = auto(Some(d(2026, 10, 16))).await.unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("blocked", 3));
        assert!(
            err.to_string()
                .contains("; IPOT, which stands in, holds only its latest session, 2026-11-06"),
            "{err}"
        );
        // Source idx never asks IPOT.
        let asked = be.calls_matching("indopremier");
        let err = market_brokers_from(&f, BrokerSource::Idx, None, 5, BrokerSort::Value, sunday())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "blocked");
        assert_eq!(be.calls_matching("indopremier"), asked);
    }

    /// `inner`, answering a URL that contains `slow` only after `pause` of
    /// wall time.
    struct Slow {
        inner: FixtureBackend,
        slow: &'static str,
        pause: std::time::Duration,
    }

    #[async_trait::async_trait]
    impl crate::http::Backend for Slow {
        async fn get(
            &self,
            url: &str,
            headers: &[(String, String)],
        ) -> Result<crate::http::RawResponse> {
            if url.contains(self.slow) {
                tokio::time::sleep(self.pause).await;
            }
            self.inner.get(url, headers).await
        }
    }

    #[tokio::test]
    async fn auto_judges_ipot_by_when_it_asks_after_idx_co_id() {
        use crate::model::BrokerSource;
        // idx.co.id takes 400 ms to refuse, from 300 ms before Monday's
        // first trades at 08:58: IPOT's pages come in the session's hours.
        let refusing = FixtureBackend::new().route_with_headers(
            "GetBrokerSummary",
            403,
            CHALLENGE,
            &[("cf-mitigated", "challenge"), ("server", "cloudflare")],
        );
        let slow = Slow {
            inner: ipot_ranking(refusing, TOP_BROKERS),
            slow: "GetBrokerSummary",
            pause: std::time::Duration::from_millis(400),
        };
        let dir = tempfile::tempdir().unwrap();
        let f = Arc::new(Fetcher::new(
            Arc::new(slow),
            Cache::new(dir.path()),
            CacheMode::Normal,
        ));
        let call = wib_at("2026-11-09T08:57:59") + chrono::Duration::milliseconds(700);
        let err = market_brokers_from(&f, BrokerSource::Auto, None, 5, BrokerSort::Value, call)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "blocked");
        // Whether IPOT was then asked or not (nothing is cached), its pages
        // would be read after 08:58.
        let says = err.to_string();
        assert!(says.contains("; IPOT could not stand in: ipot: "), "{says}");
        assert!(
            says.contains(" read at 08:58 WIB on 2026-11-09, in a trading day's hours"),
            "{says}"
        );
    }

    #[tokio::test]
    async fn a_session_idx_co_id_has_not_published_is_not_a_day_without_one() {
        use crate::model::BrokerSource;
        // Friday 19:30: idx.co.id answers Friday's summary with no rows, as
        // it does until it publishes the session; Thursday's is out.
        let night = wib_at("2026-11-06T19:30:00");
        let idx_co_id = || {
            FixtureBackend::new()
                .route("GetBrokerSummary?date=20261106", 200, NO_TRADING)
                .route("GetBrokerSummary?date=20261105", 200, BROKERS)
        };
        // Each call on its own cache: a copy stored here would carry the
        // wall clock's day, not this one's.
        let brokers = |be: FixtureBackend, source, date| async move {
            let (f, be, _d) = fetcher(be);
            let got = market_brokers_from(&f, source, date, 5, BrokerSort::Value, night).await;
            (got, be.calls_matching("indopremier"))
        };
        // Source idx: the session before, saying why.
        let (got, ipot) = brokers(idx_co_id(), BrokerSource::Idx, None).await;
        let s = got.unwrap();
        assert_eq!((s.provider, s.data.date), ("idx", d(2026, 11, 5)));
        assert_eq!(
            s.warnings,
            ["idx.co.id has not published 2026-11-06 yet; showing the session before, 2026-11-05"]
        );
        assert_eq!(ipot, 0);
        let (got, _) = brokers(idx_co_id(), BrokerSource::Idx, Some(d(2026, 11, 6))).await;
        let err = got.unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("not_found", 4));
        assert_eq!(
            err.to_string(),
            "not found: idx.co.id has not published its broker summary for 2026-11-06 yet"
        );

        // auto: IPOT, which holds Friday, stands in for it.
        for date in [None, Some(d(2026, 11, 6))] {
            let be = ipot_ranking(idx_co_id(), TOP_BROKERS);
            let s = brokers(be, BrokerSource::Auto, date).await.0.unwrap();
            assert_eq!(
                (s.provider, s.data.date),
                ("ipot", d(2026, 11, 6)),
                "{date:?}"
            );
            assert_eq!(
                s.warnings[0],
                "idx.co.id has not published 2026-11-06 yet, so the ranking is IPOT's (Indo Premier) for its latest session, 2026-11-06"
            );
        }
        // When IPOT cannot, idx.co.id's answer stands, with IPOT's reason.
        let s = brokers(idx_co_id(), BrokerSource::Auto, None)
            .await
            .0
            .unwrap();
        assert_eq!((s.provider, s.data.date), ("idx", d(2026, 11, 5)));
        assert!(
            s.warnings[0].starts_with(
                "idx.co.id has not published 2026-11-06 yet; showing the session before, 2026-11-05; IPOT could not stand in: network: fixture backend has no route for "
            ),
            "{:?}",
            s.warnings
        );
        let err = brokers(idx_co_id(), BrokerSource::Auto, Some(d(2026, 11, 6)))
            .await
            .0
            .unwrap_err();
        assert_eq!(err.code(), "not_found");
        assert!(
            err.to_string().starts_with(
                "not found: idx.co.id has not published its broker summary for 2026-11-06 yet; IPOT could not stand in: "
            ),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_day_without_a_session_costs_no_request() {
        use crate::model::BrokerSource;
        // idx.co.id would refuse anything asked, and nothing is cached.
        let (f, be, _d) = fetcher(FixtureBackend::new().route_with_headers(
            "GetBrokerSummary",
            403,
            CHALLENGE,
            &[("cf-mitigated", "challenge"), ("server", "cloudflare")],
        ));
        for source in [BrokerSource::Idx, BrokerSource::Auto] {
            for (date, says) in [
                (d(2026, 10, 31), "no trading data: 2026-10-31 is a Saturday"),
                (
                    d(2026, 8, 17),
                    "no trading data: 2026-08-17 is a trading holiday",
                ),
            ] {
                let err =
                    market_brokers_from(&f, source, Some(date), 5, BrokerSort::Value, sunday())
                        .await
                        .unwrap_err();
                assert_eq!((err.code(), err.exit_code()), ("not_found", 4), "{err}");
                assert!(err.to_string().contains(says), "{err}");
            }
            // A future one stays a usage error.
            let err = market_brokers_from(
                &f,
                source,
                Some(d(2026, 11, 14)),
                5,
                BrokerSort::Value,
                sunday(),
            )
            .await
            .unwrap_err();
            assert_eq!(err.exit_code(), 2, "{err}");
        }
        assert_eq!(be.call_count(), 0);
    }

    #[tokio::test]
    async fn auto_lets_ipot_stand_in_only_for_the_session_asked_for() {
        use crate::model::BrokerSource;
        // IPOT's pages as read on Saturday 2026-10-17, dated 2026-10-16;
        // idx.co.id has nothing for the latest session, 2026-11-06.
        let october = BBCA_STATISTIC.replacen("2026-11-06", "2026-10-16", 2);
        let seed = |dir: &std::path::Path| {
            for (url, page) in [
                (
                    "https://www.indopremier.com/module/saham/include/ma-topbroker.php",
                    TOP_BROKERS,
                ),
                (
                    "https://www.indopremier.com/module/saham/include/statistic.php?code=BBCA",
                    october.as_str(),
                ),
            ] {
                Cache::new(dir)
                    .put_fetched_at(url, 200, page, wib_at("2026-10-17T10:00:00"))
                    .unwrap();
            }
        };
        // Offline: idx.co.id's offline miss stands, saying what IPOT's copy
        // holds, not the latest session.
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path());
        let offline = Arc::new(Fetcher::new(
            Arc::new(FixtureBackend::new()),
            Cache::new(dir.path()),
            CacheMode::Offline,
        ));
        let err = market_brokers_from(
            &offline,
            BrokerSource::Auto,
            None,
            5,
            BrokerSort::Value,
            sunday(),
        )
        .await
        .unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("offline_miss", 5));
        assert!(
            err.to_string()
                .ends_with("; IPOT's copy, read at 10:00 WIB on 2026-10-17, holds only 2026-10-16"),
            "{err}"
        );
        // IPOT cannot be reached and idx.co.id is challenged: idx.co.id's
        // refusal stands, with why IPOT has only that copy.
        let (f, _be, dir) = fetcher(FixtureBackend::new().route_with_headers(
            "GetBrokerSummary",
            403,
            CHALLENGE,
            &[("cf-mitigated", "challenge"), ("server", "cloudflare")],
        ));
        seed(dir.path());
        let err = market_brokers_from(&f, BrokerSource::Auto, None, 5, BrokerSort::Value, sunday())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "blocked");
        let says = err.to_string();
        assert!(
            says.contains(
                "; IPOT, which stands in, could not be read (fixture backend has no route for "
            ),
            "{says}"
        );
        assert!(
            says.ends_with("), and its copy read at 10:00 WIB on 2026-10-17 holds only 2026-10-16"),
            "{says}"
        );
        // Asked for that session, the copy stands in, named as a copy.
        let s = market_brokers_from(
            &f,
            BrokerSource::Auto,
            Some(d(2026, 10, 16)),
            5,
            BrokerSort::Value,
            sunday(),
        )
        .await
        .unwrap();
        assert_eq!((s.provider, s.data.date), ("ipot", d(2026, 10, 16)));
        assert_eq!(
            s.warnings[0],
            "idx.co.id answered with a Cloudflare challenge, so the ranking is IPOT's (Indo Premier) for 2026-10-16, from its copy read at 10:00 WIB on 2026-10-17"
        );
    }

    #[tokio::test]
    async fn auto_names_why_ipot_could_not_be_read_when_its_copy_cannot_stand_in() {
        use crate::model::BrokerSource;
        // idx.co.id is challenged; IPOT cannot be reached and its copy was
        // read on Friday at 10:00, in the session's hours.
        let (f, _be, dir) = fetcher(FixtureBackend::new().route_with_headers(
            "GetBrokerSummary",
            403,
            CHALLENGE,
            &[("cf-mitigated", "challenge"), ("server", "cloudflare")],
        ));
        for (url, page) in [
            (
                "https://www.indopremier.com/module/saham/include/ma-topbroker.php",
                TOP_BROKERS,
            ),
            (
                "https://www.indopremier.com/module/saham/include/statistic.php?code=BBCA",
                BBCA_STATISTIC,
            ),
        ] {
            Cache::new(dir.path())
                .put_fetched_at(url, 200, page, wib_at("2026-11-06T10:00:00"))
                .unwrap();
        }
        let err = market_brokers_from(&f, BrokerSource::Auto, None, 5, BrokerSort::Value, sunday())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "blocked");
        assert!(
            err.to_string().contains(
                "; IPOT could not stand in: network: IPOT could not be read (fixture backend has no route for "
            ),
            "{err}"
        );
    }

    #[tokio::test]
    async fn auto_keeps_what_idx_co_id_answers_and_its_error_when_ipot_fails_too() {
        use crate::model::BrokerSource;
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route("GetBrokerSummary?date=20261016", 200, BROKERS)
                .route("GetBrokerSummary?date=20261015", 500, "oops")
                .route("ma-topbroker.php", 503, "busy"),
        );
        let auto = |date| {
            market_brokers_from(
                &f,
                BrokerSource::Auto,
                Some(date),
                5,
                BrokerSort::Value,
                sunday(),
            )
        };
        let s = auto(d(2026, 10, 16)).await.unwrap();
        assert_eq!(s.provider, "idx");
        assert!(s.warnings.is_empty() && !s.data.rounded);
        assert!(s
            .data
            .brokers
            .iter()
            .all(|b| b.buy_value.is_none() && b.net_value.is_none() && b.kind.is_none()));
        assert_eq!(be.calls_matching("indopremier"), 0);

        // A provider error falls back too; with IPOT failing, idx.co.id's
        // error stands.
        let err = auto(d(2026, 10, 15)).await.unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("provider", 3));
        assert!(
            err.to_string().contains(
                "HTTP 500; IPOT could not stand in: ipot: broker ranking for the market: HTTP 503"
            ),
            "{err}"
        );
    }

    #[tokio::test]
    async fn auto_stands_in_when_idx_co_id_cannot_be_reached_or_is_not_cached() {
        use crate::model::BrokerSource;
        // No route for GetBrokerSummary: the request fails as a network error.
        let (f, _be, _d) = fetcher(ipot_ranking(FixtureBackend::new(), TOP_BROKERS));
        let s = market_brokers_from(&f, BrokerSource::Auto, None, 5, BrokerSort::Value, sunday())
            .await
            .unwrap();
        assert_eq!((s.provider, s.data.date), ("ipot", d(2026, 11, 6)));
        assert!(
            s.warnings[0].starts_with("idx.co.id could not be reached (")
                && s.warnings[0].ends_with(
                    "), so the ranking is IPOT's (Indo Premier) for its latest session, 2026-11-06"
                ),
            "{:?}",
            s.warnings
        );
        // Nobody answers: idx.co.id's network error stands, saying why.
        let (f, _be, _d) = fetcher(FixtureBackend::new());
        let err = market_brokers_from(&f, BrokerSource::Auto, None, 5, BrokerSort::Value, sunday())
            .await
            .unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("network", 3));
        assert!(
            err.to_string().contains("; IPOT could not stand in: "),
            "{err}"
        );

        // Offline with only IPOT's pages cached, read on Saturday: they answer.
        let saturday = chrono::DateTime::parse_from_rfc3339("2026-11-07T10:00:00+07:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        for (url, page) in [
            (
                "https://www.indopremier.com/module/saham/include/ma-topbroker.php",
                TOP_BROKERS,
            ),
            (
                "https://www.indopremier.com/module/saham/include/statistic.php?code=BBCA",
                BBCA_STATISTIC,
            ),
        ] {
            cache.put_fetched_at(url, 200, page, saturday).unwrap();
        }
        let offline = Arc::new(Fetcher::new(
            Arc::new(FixtureBackend::new()),
            cache,
            CacheMode::Offline,
        ));
        let s = market_brokers_from(
            &offline,
            BrokerSource::Auto,
            None,
            5,
            BrokerSort::Value,
            sunday(),
        )
        .await
        .unwrap();
        assert_eq!(s.provider, "ipot");
        assert!(s.stale);
        assert_eq!(
            s.warnings[0],
            "offline, and no usable copy of idx.co.id's broker summary for 2026-11-06 is cached, so the ranking is IPOT's (Indo Premier) for its latest session, 2026-11-06"
        );
    }

    #[tokio::test]
    async fn market_brokers_start_at_the_latest_session_that_ended() {
        use crate::model::BrokerSource;
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route("GetBrokerSummary?date=20261105", 200, BROKERS)
                .route_with_headers(
                    "GetBrokerSummary?date=20261106",
                    403,
                    CHALLENGE,
                    &[("cf-mitigated", "challenge"), ("server", "cloudflare")],
                ),
        );
        // Friday 08:40: idx.co.id publishes the day's session after its
        // close, so it is not asked for, and Thursday's answers.
        for source in [BrokerSource::Idx, BrokerSource::Auto] {
            let s = market_brokers_from(
                &f,
                source,
                None,
                5,
                BrokerSort::Value,
                wib_at("2026-11-06T08:40:00"),
            )
            .await
            .unwrap();
            assert_eq!((s.provider, s.data.date), ("idx", d(2026, 11, 5)));
        }
        assert_eq!(be.calls_matching("date=20261106"), 0);
        assert_eq!(be.calls_matching("indopremier"), 0);
        // Once it has ended, the day's session is asked for first.
        let err = market_brokers_from(
            &f,
            BrokerSource::Idx,
            None,
            5,
            BrokerSort::Value,
            wib_at("2026-11-06T16:20:00"),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "blocked");
    }

    const FRIDAY_BROKERS: &str = "https://www.idx.co.id/primary/TradingSummary/GetBrokerSummary?date=20261106&start=0&length=9999";

    #[tokio::test]
    async fn a_session_whose_summary_could_not_be_read_is_not_skipped() {
        use crate::model::BrokerSource;
        // Read at 11:00, before idx.co.id published the session, Friday's
        // summary was empty; at 19:30 idx.co.id cannot be reached.
        let night = wib_at("2026-11-06T19:30:00");
        let early = |dir: &std::path::Path| {
            Cache::new(dir)
                .put_fetched_at(
                    FRIDAY_BROKERS,
                    200,
                    NO_TRADING,
                    wib_at("2026-11-06T11:00:00"),
                )
                .unwrap();
        };
        let thursday =
            || FixtureBackend::new().route("GetBrokerSummary?date=20261105", 200, BROKERS);
        let (f, be, dir) = fetcher(thursday());
        early(dir.path());
        let err = market_brokers_from(&f, BrokerSource::Idx, None, 5, BrokerSort::Value, night)
            .await
            .unwrap_err();
        // The failure and the copy's time, whichever layer words them.
        assert_eq!(err.exit_code(), 3);
        let says = err.to_string();
        assert!(
            says.contains("no route for") && says.contains("11:00 WIB"),
            "{says}"
        );
        assert_eq!(
            be.calls_matching("date=20261105"),
            0,
            "not Thursday instead"
        );

        // auto: IPOT stands in for the session idx.co.id could not read.
        let (f, _be, dir) = fetcher(ipot_ranking(thursday(), TOP_BROKERS));
        early(dir.path());
        let s = market_brokers_from(&f, BrokerSource::Auto, None, 5, BrokerSort::Value, night)
            .await
            .unwrap();
        assert_eq!((s.provider, s.data.date), ("ipot", d(2026, 11, 6)));
        assert!(
            s.warnings[0].starts_with("idx.co.id ")
                && s.warnings[0].contains("11:00 WIB")
                && s.warnings[0].ends_with(
                    ", so the ranking is IPOT's (Indo Premier) for its latest session, 2026-11-06"
                ),
            "{:?}",
            s.warnings
        );

        // Offline, that copy is not the session either.
        let dir = tempfile::tempdir().unwrap();
        early(dir.path());
        let offline = Arc::new(Fetcher::new(
            Arc::new(FixtureBackend::new()),
            Cache::new(dir.path()),
            CacheMode::Offline,
        ));
        let err = market_brokers_from(
            &offline,
            BrokerSource::Idx,
            None,
            5,
            BrokerSort::Value,
            night,
        )
        .await
        .unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("offline_miss", 5));
        assert!(
            err.to_string().contains(
                "the broker summary for 2026-11-06 is cached only as read at 11:00 WIB that day"
            ),
            "{err}"
        );
        // With IPOT's pages cached that evening, auto answers from them.
        let cache = Cache::new(dir.path());
        for (url, page) in [
            (
                "https://www.indopremier.com/module/saham/include/ma-topbroker.php",
                TOP_BROKERS,
            ),
            (
                "https://www.indopremier.com/module/saham/include/statistic.php?code=BBCA",
                BBCA_STATISTIC,
            ),
        ] {
            cache.put_fetched_at(url, 200, page, night).unwrap();
        }
        let s = market_brokers_from(
            &offline,
            BrokerSource::Auto,
            None,
            5,
            BrokerSort::Value,
            night,
        )
        .await
        .unwrap();
        assert_eq!(
            s.warnings[0],
            "offline, and no usable copy of idx.co.id's broker summary for 2026-11-06 is cached, so the ranking is IPOT's (Indo Premier) for its latest session, 2026-11-06"
        );
        // A Saturday read that Saturday has no session to miss, even offline.
        Cache::new(dir.path())
            .put_fetched_at(
                "https://www.idx.co.id/primary/TradingSummary/GetBrokerSummary?date=20261031&start=0&length=9999",
                200,
                NO_TRADING,
                wib_at("2026-10-31T11:00:00"),
            )
            .unwrap();
        let err = market_brokers_from(
            &offline,
            BrokerSource::Idx,
            Some(d(2026, 10, 31)),
            5,
            BrokerSort::Value,
            sunday(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "not_found");
        assert!(
            err.to_string().contains("2026-10-31 is a Saturday"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_summary_cached_before_its_session_was_published_is_read_again() {
        use crate::model::BrokerSource;
        // Thursday's summary read that morning, before idx.co.id published
        // it: on Friday it would be kept as a closed session for 30 days.
        let friday = wib_at("2026-11-06T07:00:00");
        let early = |dir: &std::path::Path| {
            Cache::new(dir)
                .put_fetched_at(
                    "https://www.idx.co.id/primary/TradingSummary/GetBrokerSummary?date=20261105&start=0&length=9999",
                    200,
                    NO_TRADING,
                    wib_at("2026-11-05T11:00:00"),
                )
                .unwrap();
        };
        let (f, be, dir) =
            fetcher(FixtureBackend::new().route("GetBrokerSummary?date=20261105", 200, BROKERS));
        early(dir.path());
        let latest =
            || market_brokers_from(&f, BrokerSource::Idx, None, 5, BrokerSort::Value, friday);
        let s = latest().await.unwrap();
        assert_eq!((s.data.date, s.data.members), (d(2026, 11, 5), 88));
        assert!(!s.cached);
        // The published copy is kept.
        assert!(latest().await.unwrap().cached);
        assert_eq!(be.call_count(), 1);

        // When it cannot be read again, the session is not skipped.
        let (f, _be, dir) = fetcher(FixtureBackend::new());
        early(dir.path());
        let err = market_brokers_from(&f, BrokerSource::Idx, None, 5, BrokerSort::Value, friday)
            .await
            .unwrap_err();
        // Thursday's failure and its copy's time, not Wednesday's.
        assert_eq!(err.exit_code(), 3);
        let says = err.to_string();
        assert!(
            says.contains("2026-11-05") && says.contains("11:00 WIB"),
            "{says}"
        );
    }

    #[tokio::test]
    async fn an_ipot_ranking_without_trades_is_not_found_and_other_codes_are_named() {
        use crate::model::BrokerSource;
        let page = |rows: &[(&str, &str, &str)]| {
            let body: String = rows
                .iter()
                .enumerate()
                .map(|(i, (code, name, v))| {
                    format!(
                        "<tr><td>{}</td><td><span class=\"text-local\">{code}</span></td><td><span class=\"text-local\">{name}</span></td><td>{v}</td><td>0</td><td>{v}</td><td>{v}</td><td>{v}</td><td>{v}</td></tr>",
                        i + 1
                    )
                })
                .collect();
            let head = "<thead><tr><th>#</th><th colspan=\"2\">Company</th><th>T.Val</th><th>N.Val</th><th>B.Val</th><th>S.Val</th><th>T.Vol</th><th>T.Freq</th></tr></thead>";
            format!("<table>{head}<tbody>{body}</tbody></table>")
        };
        let ranking_at = |html: String, now| async move {
            let (f, _be, _d) = fetcher(ipot_ranking(FixtureBackend::new(), &html));
            market_brokers_from(&f, BrokerSource::Ipot, None, 0, BrokerSort::Value, now).await
        };
        let ranking = |html: String| ranking_at(html, sunday());
        // Read on the morning of 2026-11-06, before its first trades.
        let morning = chrono::DateTime::parse_from_rfc3339("2026-11-06T08:00:00+07:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let none = || {
            page(&[
                ("PS", "PARAMITRA ALFA SEKURITAS", "0"),
                ("ZZ", "Kliring", "0"),
            ])
        };
        let err = ranking_at(none(), morning).await.unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(
            err.to_string()
                .contains("IPOT's broker ranking for 2026-11-06 has no trades yet"),
            "{err}"
        );
        // Read on the Sunday after, the session had trades: no date fits.
        let err = ranking(none()).await.unwrap_err();
        assert_eq!(err.code(), "provider");

        let s = ranking(page(&[
            ("Q9", "Unknown Sekuritas", "12"),
            ("CC", "MANDIRI SEKURITAS", "12"),
            ("PS", "PARAMITRA ALFA SEKURITAS", "0"),
        ]))
        .await
        .unwrap();
        assert_eq!(s.data.members, 2);
        assert!(!s.data.rounded, "small figures are exact");
        assert_eq!(
            s.warnings,
            ["IPOT lists trades for codes that are not on the IDX member list: Q9 (Unknown Sekuritas)"]
        );
        let codes: Vec<&str> = s.data.brokers.iter().map(|b| b.code.as_str()).collect();
        assert_eq!(codes, ["Q9", "CC"], "a tie keeps IPOT's order");
    }

    const BROKERS: &str = include_str!("../../tests/fixtures/idx/brokersummary_20261016.json");

    #[tokio::test]
    async fn market_brokers_rank_members_with_value_shares() {
        let (f, _be, _d) =
            fetcher(FixtureBackend::new().route("GetBrokerSummary?date=20261016", 200, BROKERS));
        let s = market_brokers(
            &f,
            Some(d(2026, 10, 16)),
            5,
            BrokerSort::Value,
            d(2026, 10, 18),
        )
        .await
        .unwrap();
        let r = &s.data;
        assert_eq!(r.members, 88);
        assert_eq!(r.brokers.len(), 5);
        assert_eq!(r.brokers[0].rank, 1);
        let v = |x: &BrokerDayRow| x.value.unwrap();
        assert!(r.brokers.windows(2).all(|w| v(&w[0]) >= v(&w[1])));
        assert!((r.total_value - 64_027_626_516_936.0).abs() < 1.0);
        let pct = r.brokers[0].value_share_pct.unwrap();
        assert!((pct - v(&r.brokers[0]) / r.total_value * 100.0).abs() < 1e-9);

        let all = market_brokers(
            &f,
            Some(d(2026, 10, 16)),
            0,
            BrokerSort::Frequency,
            d(2026, 10, 18),
        )
        .await
        .unwrap();
        assert_eq!(all.data.brokers.len(), 88);
        let share: f64 = all
            .data
            .brokers
            .iter()
            .filter_map(|b| b.value_share_pct)
            .sum();
        assert!((share - 100.0).abs() < 1e-6);
        let fr = |x: &BrokerDayRow| x.frequency.unwrap();
        assert!(all.data.brokers.windows(2).all(|w| fr(&w[0]) >= fr(&w[1])));
        assert_eq!(all.data.brokers.last().unwrap().rank, 88);
    }
}
