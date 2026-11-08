//! Tables for exchange-wide data: holidays, instruments, members, indices,
//! the corporate-action calendar, corporate actions, foreign flow and broker
//! activity. Same conventions as `table.rs`: pure functions, dashes for
//! missing values, colour only when asked.

use comfy_table::{Attribute, Cell};

use crate::model::{ActionKind, CorporateActions};
use crate::model::{BrokerRanking, BrokerSort};
use crate::model::{Calendar, CalendarCategory, Holidays, IndexBoard, InstrumentList, MemberList};
use crate::model::{ForeignFlow, ForeignRow, ForeignSort, ForeignValueFlow, MarketForeign};
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

/// Exchange members ranked for one session, as `provider` (`idx` or `ipot`)
/// published them. Rows with a class or a net (IPOT's) get those columns.
pub fn market_brokers(r: &BrokerRanking, provider: &str, color: bool) -> String {
    let by = match r.sort {
        BrokerSort::Value => "value",
        BrokerSort::Volume => "volume",
        BrokerSort::Frequency => "frequency",
    };
    let mut out = format!(
        "Exchange members · {} · {} active · by {by} · all members' value {} (both sides)\n",
        r.date,
        r.members,
        f::compact(Some(r.total_value)),
    );
    let sides = r
        .brokers
        .iter()
        .any(|b| b.net_value.is_some() || b.kind.is_some());
    let columns: &[&str] = if sides {
        &[
            "#",
            "Code",
            "Member",
            "Class",
            "Value",
            "Share",
            "Net",
            "Volume",
            "Frequency",
        ]
    } else {
        &[
            "#",
            "Code",
            "Member",
            "Value",
            "Share",
            "Volume",
            "Frequency",
        ]
    };
    let mut t = base_table(color && sides);
    t.set_header(header(columns));
    for b in &r.brokers {
        let share = b
            .value_share_pct
            .map(|p| format!("{}%", f::num(p, 2)))
            .unwrap_or_else(|| f::DASH.to_string());
        let mut row = vec![
            right(b.rank.to_string()),
            Cell::new(&b.code).add_attribute(Attribute::Bold),
            Cell::new(f::clip(
                b.name.as_deref().unwrap_or(f::DASH),
                if sides { 28 } else { 34 },
            )),
        ];
        if sides {
            row.extend([
                Cell::new(crate::output::table_broker::kind_label(b.kind)),
                right(f::compact(b.value)),
                right(share),
                signed(f::compact(b.net_value), b.net_value, color),
            ]);
        } else {
            row.extend([right(f::compact(b.value)), right(share)]);
        }
        row.extend([right(f::compact(b.volume)), right(f::compact(b.frequency))]);
        t.add_row(row);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    let source = match provider {
        "idx" => "idx.co.id",
        "ipot" => "IPOT (Indo Premier)",
        other => other,
    };
    let net = if sides {
        ", and its net (bought minus sold; -o json, csv and jsonl add the value bought and sold)"
    } else {
        ""
    };
    out.push_str(&format!("Each member's buying and selling together, as {source} publishes them{net}; shares sum to 100% of all members' value, which is twice the market's.\n"));
    if r.rounded {
        out.push_str(&format!(
            "{source} abbreviates figures of a million and more to two decimals (2.79T), so they and the totals carry that precision.\n"
        ));
    }
    out
}

/// One stock's foreign buying and selling, newest session first.
pub fn foreign_flow(fl: &ForeignFlow, color: bool) -> String {
    let mut out = format!(
        "{}{} · foreign flow · {} to {} · {} sessions\n",
        fl.symbol,
        fl.name
            .as_ref()
            .map(|n| format!(" · {n}"))
            .unwrap_or_default(),
        fl.from,
        fl.to,
        fl.totals.sessions
    );
    let mut t = base_table(color);
    t.set_header(header(&[
        "Date",
        "Close",
        "Change",
        "Volume",
        "Foreign buy",
        "Foreign sell",
        "Net (shares)",
        "Net × close",
    ]));
    for d in fl.days.iter().rev() {
        t.add_row(vec![
            Cell::new(d.date.to_string()),
            right(f::price(d.close)),
            signed(f::change(d.change), d.change, color),
            right(f::compact(d.volume)),
            right(f::compact(d.foreign_buy)),
            right(f::compact(d.foreign_sell)),
            signed(f::compact(d.foreign_net), d.foreign_net, color),
            signed(
                f::compact(d.foreign_net_value_at_close),
                d.foreign_net_value_at_close,
                color,
            ),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    let tt = &fl.totals;
    let streak = streak_words(tt.streak);
    out.push_str(&format!(
        "Total · foreign buy {} · sell {} · net {} shares (≈ {} at close) · {} net-buy / {} net-sell sessions · {streak}\n",
        f::compact(Some(tt.foreign_buy)),
        f::compact(Some(tt.foreign_sell)),
        f::compact(Some(tt.foreign_net)),
        f::compact(Some(tt.foreign_net_value_at_close)),
        tt.net_buy_sessions,
        tt.net_sell_sessions,
    ));
    out.push_str("Foreign figures are shares as IDX publishes them; net × close gives a rupiah scale, not the traded value.\n");
    out
}

fn streak_words(streak: i32) -> String {
    match streak {
        0 => "flat last session".to_string(),
        s if s > 0 => format!("{s} net-buy session(s) in a row"),
        s => format!("{} net-sell session(s) in a row", -s),
    }
}

fn signed_compact(v: Option<f64>) -> String {
    match v {
        Some(x) if x > 0.0 => format!("+{}", f::compact(Some(x))),
        other => f::compact(other),
    }
}

fn pct1(v: Option<f64>) -> String {
    v.map(|p| format!("{}%", f::num(p, 1)))
        .unwrap_or_else(|| f::DASH.to_string())
}

/// One stock's foreign flow in rupiah from IPOT, newest session first.
pub fn foreign_value_flow(fl: &ForeignValueFlow, color: bool) -> String {
    let mut out = format!(
        "{}{} · foreign flow (IPOT, all boards) · {} to {} · {} sessions\n",
        fl.symbol,
        fl.name
            .as_ref()
            .map(|n| format!(" · {n}"))
            .unwrap_or_default(),
        fl.from,
        fl.to,
        fl.totals.sessions
    );
    let mut t = base_table(color);
    t.set_header(header(&[
        "Date",
        "Value",
        "Avg",
        "Foreign buy",
        "Foreign sell",
        "Foreign net",
        "Foreign %",
    ]));
    for d in fl.days.iter().rev() {
        t.add_row(vec![
            Cell::new(d.date.to_string()),
            right(f::compact(d.value)),
            right(f::price(d.average_price)),
            right(f::compact(d.foreign_buy_value)),
            right(f::compact(d.foreign_sell_value)),
            signed(
                signed_compact(d.foreign_net_value),
                d.foreign_net_value,
                color,
            ),
            right(pct1(d.foreign_share_pct)),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    let tt = &fl.totals;
    out.push_str(&format!(
        "Total · value {} · foreign buy {} · sell {} · net {} · foreign share {} · {} net-buy / {} net-sell sessions · {}\n",
        f::compact(Some(tt.value)),
        f::compact(Some(tt.foreign_buy_value)),
        f::compact(Some(tt.foreign_sell_value)),
        signed_compact(Some(tt.foreign_net_value)),
        pct1(tt.foreign_share_pct),
        tt.net_buy_sessions,
        tt.net_sell_sessions,
        streak_words(tt.streak),
    ));
    out.push_str("Rupiah across all boards (negotiated trades included) from IPOT's broker summary; foreign % is foreign buying plus selling over twice the value traded.");
    if fl.rounded {
        out.push_str(" The source rounds figures of a million and more to one decimal.");
    }
    out.push_str(" For shares on the regular board as IDX publishes them, use --source idx.\n");
    out
}

fn foreign_list(out: &mut String, title: &str, rows: &[ForeignRow], color: bool) {
    out.push_str(title);
    out.push('\n');
    if rows.is_empty() {
        out.push_str("None.\n");
        return;
    }
    let mut t = base_table(color);
    t.set_header(header(&[
        "#",
        "Symbol",
        "Name",
        "Close",
        "%",
        "Foreign buy",
        "Foreign sell",
        "Net (shares)",
        "Net × close",
    ]));
    for (i, r) in rows.iter().enumerate() {
        t.add_row(vec![
            right((i + 1).to_string()),
            Cell::new(r.symbol.as_str()).add_attribute(Attribute::Bold),
            Cell::new(f::clip(r.name.as_deref().unwrap_or(f::DASH), 26)),
            right(f::price(r.close)),
            signed(f::pct(r.change_pct), r.change_pct, color),
            right(f::compact(r.foreign_buy)),
            right(f::compact(r.foreign_sell)),
            signed(f::compact(r.foreign_net), r.foreign_net, color),
            signed(
                f::compact(r.foreign_net_value_at_close),
                r.foreign_net_value_at_close,
                color,
            ),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
}

/// Market-wide foreign net buying and selling for one session.
pub fn market_foreign(m: &MarketForeign, color: bool) -> String {
    let mut out = format!(
        "Foreign flow · {} · {} stocks · foreign buy {} · sell {} · net {} shares (≈ {} at close)\n",
        m.date,
        m.stocks,
        f::compact(Some(m.foreign_buy)),
        f::compact(Some(m.foreign_sell)),
        f::compact(Some(m.foreign_net)),
        f::compact(Some(m.foreign_net_value_at_close)),
    );
    let by = match m.sort {
        ForeignSort::Value => "by net × close",
        ForeignSort::Shares => "by net shares",
    };
    foreign_list(
        &mut out,
        &format!("Foreign net buying · {by}"),
        &m.net_buy,
        color,
    );
    foreign_list(
        &mut out,
        &format!("Foreign net selling · {by}"),
        &m.net_sell,
        color,
    );
    out.push_str("Foreign figures are shares as IDX publishes them; net × close gives a rupiah scale, not the traded value.\n");
    out
}

fn day(d: Option<chrono::NaiveDate>) -> String {
    d.map(|x| x.to_string())
        .unwrap_or_else(|| f::DASH.to_string())
}

fn action_label(k: ActionKind) -> &'static str {
    match k {
        ActionKind::CashDividend => "cash dividend",
        ActionKind::StockDividend => "stock dividend",
        ActionKind::StockSplit => "stock split",
        ActionKind::ReverseSplit => "reverse split",
        ActionKind::Bonus => "bonus shares",
        ActionKind::RightsIssue => "rights issue",
        ActionKind::Warrant => "warrant",
        ActionKind::Other => "other",
    }
}

/// Corporate actions, meetings and warrants/rights of one company.
pub fn corporate_actions(c: &CorporateActions, _color: bool) -> String {
    let mut out = format!(
        "{}{} · since {}\n",
        c.symbol,
        c.name
            .as_ref()
            .map(|n| format!(" · {n}"))
            .unwrap_or_default(),
        c.since
    );
    out.push_str("Corporate actions\n");
    if c.actions.is_empty() {
        out.push_str("None in this period.\n");
    } else {
        let mut t = base_table(false);
        t.set_header(header(&[
            "Action",
            "Amount",
            "Ratio",
            "Cum",
            "Ex",
            "Recording",
            "Payment",
            "Status",
        ]));
        for a in &c.actions {
            let ratio = match (a.ratio_old, a.ratio_new) {
                (Some(o), Some(n)) => format!("{}:{}", f::price(Some(o)), f::price(Some(n))),
                _ => f::DASH.to_string(),
            };
            t.add_row(vec![
                Cell::new(action_label(a.kind)).add_attribute(Attribute::Bold),
                right(f::price(a.amount)),
                right(ratio),
                Cell::new(day(a.cum_date)),
                Cell::new(day(a.ex_date)),
                Cell::new(day(a.recording_date)),
                Cell::new(day(a.distribution_date)),
                Cell::new(a.status.as_deref().unwrap_or(f::DASH)),
            ]);
        }
        out.push_str(&t.to_string());
        out.push('\n');
    }
    out.push_str("Shareholders' meetings (RUPS)\n");
    if c.meetings.is_empty() {
        out.push_str("None in this period.\n");
    } else {
        let mut t = base_table(false);
        t.set_header(header(&["Date", "Time", "Status", "Venue"]));
        for m in &c.meetings {
            t.add_row(vec![
                Cell::new(day(m.date)),
                Cell::new(m.time.as_deref().unwrap_or(f::DASH)),
                Cell::new(m.status.as_deref().unwrap_or(f::DASH)),
                Cell::new(f::clip(m.venue.as_deref().unwrap_or(f::DASH), 60)),
            ]);
        }
        out.push_str(&t.to_string());
        out.push('\n');
    }
    if !c.warrants_rights.is_empty() {
        out.push_str("Warrants and rights\n");
        let mut t = base_table(false);
        t.set_header(header(&[
            "Code", "Type", "Exercise", "Issued", "Listing", "Maturity", "Status",
        ]));
        for w in &c.warrants_rights {
            t.add_row(vec![
                Cell::new(&w.code).add_attribute(Attribute::Bold),
                Cell::new(&w.kind),
                right(f::price(w.exercise_price)),
                right(f::compact(w.shares)),
                Cell::new(day(w.listing_date)),
                Cell::new(day(w.maturity_date)),
                Cell::new(w.status.as_deref().unwrap_or(f::DASH)),
            ]);
        }
        out.push_str(&t.to_string());
        out.push('\n');
    }
    out.push_str("Amount: IDR per share (dividend, or a rights issue's exercise price). Ratio: old:new. Source: IPOT. JSON carries each announcement.\n");
    out
}

fn category_label(c: CalendarCategory) -> &'static str {
    match c {
        CalendarCategory::Dividend => "dividend",
        CalendarCategory::Rights => "rights",
        CalendarCategory::Bonus => "bonus",
        CalendarCategory::Split => "split",
        CalendarCategory::Warrant => "warrant",
        CalendarCategory::Rups => "RUPS",
        CalendarCategory::PublicExpose => "expose",
        CalendarCategory::Buyback => "buyback",
        CalendarCategory::Listing => "listing",
        CalendarCategory::Bond => "bond",
        CalendarCategory::Other => "other",
    }
}

/// Exchange calendar entries, one row each.
pub fn calendar(c: &Calendar, _color: bool) -> String {
    let mut out = if c.from == c.to {
        format!("IDX calendar · {}", c.from)
    } else {
        format!("IDX calendar · {} to {}", c.from, c.to)
    };
    if let Some(s) = &c.symbol {
        out.push_str(&format!(" · {s}"));
    }
    out.push_str(&format!(
        " · {} of {} entries ({})\n",
        c.events.len(),
        c.total,
        c.categories
            .iter()
            .map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if c.events.is_empty() {
        out.push_str("No entries.\n");
        return out;
    }
    let mut t = base_table(false);
    t.set_header(header(&["Date", "Code", "Type", "Step", "When", "Entry"]));
    for e in &c.events {
        let when = match (e.meeting_at, e.expose_date) {
            (Some(m), _) => m.format("%Y-%m-%d %H:%M").to_string(),
            (None, Some(x)) => x.to_string(),
            (None, None) => f::DASH.to_string(),
        };
        t.add_row(vec![
            Cell::new(e.date.to_string()),
            Cell::new(&e.code).add_attribute(Attribute::Bold),
            Cell::new(category_label(e.category)),
            Cell::new(e.milestone.as_deref().unwrap_or(f::DASH)),
            Cell::new(when),
            Cell::new(f::clip(&e.description, 64)),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    out.push_str("Type and step are classified by idx from the entry's wording; --type all adds bonds and buyback reports.\n");
    out
}

/// Holiday list plus today's trading status.
pub fn holidays(h: &Holidays, _color: bool) -> String {
    let mut out = String::new();
    let scope = match h.year {
        Some(y) => format!("IDX trading holidays {y}"),
        None => "IDX trading holidays".to_string(),
    };
    out.push_str(&scope);
    if let Some(n) = h.trading_days {
        out.push_str(&format!(" · {n} trading days"));
    }
    out.push('\n');
    out.push_str(&format!(
        "Today {} is {} · next trading day {}\n",
        h.today,
        if h.today_is_trading_day {
            "a trading day"
        } else {
            "not a trading day"
        },
        h.next_trading_day
    ));
    if h.holidays.is_empty() {
        out.push_str("No holidays listed.\n");
        return out;
    }
    let mut t = base_table(false);
    t.set_header(header(&["Date", "Day", "Holiday", "English"]));
    for d in &h.holidays {
        t.add_row(vec![
            Cell::new(d.date.to_string()),
            Cell::new(&d.weekday),
            Cell::new(&d.name),
            Cell::new(d.name_en.as_deref().unwrap_or(f::DASH)),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    out.push_str(&format!(
        "Covers {}. Weekend holidays are not listed; IDX can add a holiday later.\n",
        h.years_covered
            .iter()
            .map(|y| y.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out
}

/// Listed stocks.
pub fn instruments(l: &InstrumentList, _color: bool) -> String {
    let mut out = String::new();
    let mut scope = format!("{} of {} listed stocks", l.instruments.len(), l.total);
    if let Some(b) = &l.board {
        scope.push_str(&format!(" · board {b}"));
    }
    if let Some(q) = &l.search {
        scope.push_str(&format!(" · matching {q:?}"));
    }
    out.push_str(&scope);
    out.push('\n');
    if l.instruments.is_empty() {
        return out;
    }
    let mut t = base_table(false);
    t.set_header(header(&["Symbol", "Name", "Board", "Listed", "Shares"]));
    for i in &l.instruments {
        t.add_row(vec![
            Cell::new(i.symbol.as_str()).add_attribute(Attribute::Bold),
            Cell::new(f::clip(i.name.as_deref().unwrap_or(f::DASH), 34)),
            Cell::new(i.board.as_deref().unwrap_or(f::DASH)),
            Cell::new(
                i.listing_date
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| f::DASH.to_string()),
            ),
            right(f::compact(i.shares)),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    out
}

/// Exchange members.
pub fn members(l: &MemberList, _color: bool) -> String {
    let mut out = format!("{} of {} exchange members", l.members.len(), l.total);
    if let Some(q) = &l.search {
        out.push_str(&format!(" · matching {q:?}"));
    }
    out.push('\n');
    if l.members.is_empty() {
        return out;
    }
    let mut t = base_table(false);
    t.set_header(header(&["Code", "Member", "Licences"]));
    for m in &l.members {
        t.add_row(vec![
            Cell::new(&m.code).add_attribute(Attribute::Bold),
            Cell::new(m.name.as_deref().unwrap_or(f::DASH)),
            Cell::new(
                m.licenses
                    .as_ref()
                    .map(|l| l.join(", "))
                    .unwrap_or_else(|| f::DASH.to_string()),
            ),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    out
}

/// The day's index board.
pub fn indices(b: &IndexBoard, color: bool) -> String {
    let mut out = format!("IDX indices · {}\n", b.date);
    let mut t = base_table(color);
    t.set_header(header(&[
        "Index",
        "Close",
        "Change",
        "%",
        "High",
        "Low",
        "Stocks",
        "Value",
        "Market cap",
    ]));
    for r in &b.indices {
        t.add_row(vec![
            Cell::new(&r.code).add_attribute(Attribute::Bold),
            right(f::ratio(r.close)),
            signed(
                f::change(r.change.map(|c| (c * 100.0).round() / 100.0)),
                r.change,
                color,
            ),
            signed(f::pct(r.change_pct), r.change_pct, color),
            right(f::ratio(r.high)),
            right(f::ratio(r.low)),
            right(
                r.constituents
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| f::DASH.to_string()),
            ),
            right(f::compact(r.value)),
            right(f::compact(r.market_cap)),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    out.push_str(
        "COMPOSITE is IHSG (^JKSE). Value and market cap in IDR, as published by idx.co.id.\n",
    );
    out
}
