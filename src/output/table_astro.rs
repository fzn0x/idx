//! `market astro` as text: the sky table, the Moon, Mercury, weton and shio
//! lines, the events ahead, the transits to IHSG's chart and the backtest.

use chrono::{DateTime, FixedOffset};
use comfy_table::Cell;

use crate::model::{
    Astro, AstroBacktest, AstroEvent, AstroEventKind, BacktestGroup, IhsgChart, NatalChart,
};
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

/// `13°57'`: whole degrees and minutes, the minutes truncated so they never
/// round up into the next degree.
pub fn dms(degree: f64) -> String {
    let d = degree.floor();
    // The epsilon keeps 2.05 (stored as 2.04999...) at 2°03'.
    let m = ((degree - d) * 60.0 + 1e-9).floor().min(59.0);
    format!("{}°{:02}'", d as i64, m as i64)
}

fn hm(t: &DateTime<FixedOffset>) -> String {
    t.format("%Y-%m-%d %H:%M").to_string()
}

fn signed_speed(v: f64) -> String {
    if v > 0.0 {
        format!("+{v:.3}°")
    } else {
        format!("{v:.3}°")
    }
}

fn event_line(e: &AstroEvent) -> String {
    let sign = e.sign.name();
    let what = match e.kind {
        AstroEventKind::NewMoon => format!("New moon in {sign}"),
        AstroEventKind::FirstQuarter => format!("First quarter moon in {sign}"),
        AstroEventKind::FullMoon => format!("Full moon in {sign}"),
        AstroEventKind::LastQuarter => format!("Last quarter moon in {sign}"),
        AstroEventKind::StationRetrograde => {
            format!("{} stations retrograde in {sign}", e.body.name())
        }
        AstroEventKind::StationDirect => format!("{} stations direct in {sign}", e.body.name()),
        AstroEventKind::Ingress => format!("{} enters {sign}", e.body.name()),
    };
    format!("{}  {what}\n", hm(&e.at))
}

fn natal_block(n: &NatalChart, color: bool) -> String {
    let label = match n.chart {
        IhsgChart::Launch => "launch",
        IhsgChart::Base => "base date",
    };
    let mut out = format!(
        "\nTransits to IHSG's chart ({label} {}, noon WIB; orb 2°)\n",
        n.date
    );
    if n.transits.is_empty() {
        out.push_str("No aspects within 2° today.\n");
        return out;
    }
    let mut t = base_table(color);
    t.set_header(header(&["Transit", "Aspect", "Natal", "Orb", "Motion"]));
    for x in &n.transits {
        t.add_row(vec![
            Cell::new(x.transit.name()),
            Cell::new(x.aspect.as_str()),
            Cell::new(format!("natal {}", x.natal.name())),
            right(format!("{:.2}°", x.orb)),
            Cell::new(if x.applying { "applying" } else { "separating" }),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');
    out
}

fn opt(v: Option<f64>, decimals: usize, sign: bool) -> String {
    match v {
        None => f::DASH.to_string(),
        Some(x) if sign && x > 0.0 => format!("+{x:.decimals$}"),
        Some(x) => format!("{x:.decimals$}"),
    }
}

fn group_label(factor: &str, g: &BacktestGroup) -> (String, String) {
    let factor = match factor {
        "moon_phase" => "Moon phase",
        "mercury" => "Mercury",
        "pasaran" => "Pasaran",
        "sun_sign" => "Sun sign",
        other => other,
    };
    let mut name = g.group.replace('_', " ");
    if let Some(first) = name.get(0..1) {
        name = first.to_uppercase() + &name[1..];
    }
    (factor.to_string(), name)
}

fn backtest_block(b: &AstroBacktest, color: bool) -> String {
    let mut out = format!(
        "\nBacktest · {} daily close-to-close returns · {} to {} · {} sessions\n",
        b.symbol,
        b.from,
        b.to,
        f::int(Some(b.returns as u64))
    );
    out.push_str(&format!(
        "All sessions: mean {}%, median {}%, up {:.2}%\n",
        opt(Some(b.mean_pct), 4, true),
        opt(Some(b.median_pct), 4, true),
        b.win_rate_pct
    ));
    let mut t = base_table(color);
    t.set_header(header(&[
        "Factor", "Group", "Days", "Mean %", "Median %", "Up %", "t", "p",
    ]));
    for fac in &b.factors {
        for g in &fac.groups {
            let (factor, name) = group_label(&fac.factor, g);
            let p = g.p_value.map(|p| {
                if g.significant {
                    format!("{p:.4} *")
                } else {
                    format!("{p:.4}")
                }
            });
            t.add_row(vec![
                Cell::new(factor),
                Cell::new(name),
                right(f::int(Some(g.n as u64))),
                signed(opt(g.mean_pct, 4, true), g.mean_pct, color),
                right(opt(g.median_pct, 4, true)),
                right(opt(g.win_rate_pct, 1, false)),
                right(opt(g.t_stat, 2, false)),
                right(p.unwrap_or_else(|| f::DASH.to_string())),
            ]);
        }
    }
    out.push_str(&t.to_string());
    out.push('\n');
    let verdict = match b.significant {
        0 => "none pass".to_string(),
        1 => "1 passes (marked *)".to_string(),
        n => format!("{n} pass (marked *)"),
    };
    out.push_str(&format!(
        "{} tests against all other sessions (Welch's t); Bonferroni needs p < {} at {}%: {verdict}.\n",
        b.tests,
        b.bonferroni_alpha,
        b.alpha * 100.0
    ));
    out.push_str(
        "Sun signs follow the calendar, so they also carry month-of-year effects. Descriptive, not a trading signal.\n",
    );
    out
}

pub fn astro(a: &Astro, color: bool) -> String {
    let mut out = format!(
        "IHSG astro · {} {} {} · {} WIB\n",
        a.weton.day,
        a.weton.pasaran,
        a.date,
        a.at.format("%H:%M")
    );
    let mut t = base_table(color);
    t.set_header(header(&["Body", "Sign", "Position", "Speed/day"]));
    for b in &a.bodies {
        let pos = if b.retrograde {
            format!("{} R", dms(b.degree))
        } else {
            dms(b.degree)
        };
        t.add_row(vec![
            Cell::new(b.body.name()),
            Cell::new(b.sign.name()),
            Cell::new(pos),
            right(signed_speed(b.speed)),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');

    let m = &a.moon;
    out.push_str(&format!(
        "Moon: {}, {:.0}% lit, {:.1} days old · new moon {} · full moon {}\n",
        m.phase.label(),
        m.illumination * 100.0,
        m.age_days,
        hm(&m.next_new_moon),
        hm(&m.next_full_moon)
    ));
    let me = &a.mercury;
    let mut line = if me.retrograde {
        let mut s = "Mercury: retrograde".to_string();
        if let Some(since) = &me.retrograde_since {
            s.push_str(&format!(" since {}", hm(since)));
        }
        if let Some(until) = &me.retrograde_until {
            s.push_str(&format!(", direct {}", hm(until)));
        }
        s
    } else {
        "Mercury: direct".to_string()
    };
    if let Some(n) = &me.next_retrograde {
        line.push_str(&format!(" · next retrograde {}", n.from.date_naive()));
        if let Some(u) = &n.until {
            line.push_str(&format!(" to {}", u.date_naive()));
        }
    }
    out.push_str(&line);
    out.push('\n');
    let w = &a.weton;
    let shio = match &a.shio {
        Some(s) => format!(
            "Shio: {} ({} {}) since Imlek {}",
            s.animal_id, s.element, s.animal, s.year_start
        ),
        None => "Shio: not computed before 1924 or after 2050".to_string(),
    };
    out.push_str(&format!(
        "Weton: {} {}, neptu {} + {} = {} · {shio}\n",
        w.day, w.pasaran, w.neptu_day, w.neptu_pasaran, w.neptu
    ));

    let days = (a.upcoming.to - a.upcoming.from).num_days();
    out.push_str(&format!(
        "\nNext {days} day{}\n",
        if days == 1 { "" } else { "s" }
    ));
    if a.upcoming.events.is_empty() {
        out.push_str("No exact phases, stations or ingresses.\n");
    }
    for e in &a.upcoming.events {
        out.push_str(&event_line(e));
    }
    if let Some(n) = &a.natal {
        out.push_str(&natal_block(n, color));
    }
    match &a.backtest {
        Some(b) => {
            out.push_str(&backtest_block(b, color));
            out.push_str(&format!("{}\n", a.disclaimer));
        }
        None => out.push_str(&format!(
            "\n{} Add --backtest to see how IHSG's daily returns split by these factors.\n",
            a.disclaimer
        )),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NatalChoice;

    #[test]
    fn degrees_and_minutes_never_round_up() {
        assert_eq!(dms(13.96), "13°57'");
        assert_eq!(dms(29.999), "29°59'");
        assert_eq!(dms(0.0), "0°00'");
        assert_eq!(dms(2.05), "2°03'");
    }

    #[test]
    fn renders_every_block() {
        let (a, _) =
            crate::astro::sky("2026-11-06".parse().unwrap(), 30, NatalChoice::Launch).unwrap();
        let s = astro(&a, false);
        assert!(
            s.starts_with("IHSG astro · Jumat Pahing 2026-11-06 · 16:00 WIB\n"),
            "{s}"
        );
        assert!(s.contains("Mercury: retrograde since 2026-10-24"), "{s}");
        assert!(
            s.contains("Moon: waning crescent, 9% lit, 26.7 days old"),
            "{s}"
        );
        assert!(s.contains("Weton: Jumat Pahing, neptu 6 + 9 = 15 · Shio: Kuda (fire horse) since Imlek 2026-02-17"), "{s}");
        assert!(s.contains("Sun enters Sagittarius"), "{s}");
        assert!(
            s.contains("Transits to IHSG's chart (launch 1983-04-01"),
            "{s}"
        );
        assert!(s.contains("Add --backtest"), "{s}");
        assert!(s.contains("13°57'"), "{s}");
    }
}
