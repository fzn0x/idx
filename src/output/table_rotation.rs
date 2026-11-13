//! Table for `stocks rotation`: one row per symbol with its latest point and
//! the change over its tail, then the four quadrants laid out as the graph
//! draws them. Same conventions as `table_reports.rs`.

use comfy_table::{Attribute, Cell, Color};

use crate::model::{Quadrant, Rotation, RotationInterval};
use crate::output::format as f;
use crate::output::table::{base_table, header, right, signed};

fn quadrant_cell(q: Option<Quadrant>, color: bool) -> Cell {
    let Some(q) = q else {
        return Cell::new(f::DASH);
    };
    let c = Cell::new(q.as_str());
    if !color {
        return c;
    }
    // StockCharts' colours: leading green, weakening yellow, lagging red, improving blue.
    c.fg(match q {
        Quadrant::Leading => Color::Green,
        Quadrant::Weakening => Color::Yellow,
        Quadrant::Lagging => Color::Red,
        Quadrant::Improving => Color::Blue,
    })
}

fn axis(v: Option<f64>) -> String {
    v.map(|x| f::num(x, 2)).unwrap_or_else(|| f::DASH.into())
}

fn change(v: Option<f64>) -> String {
    v.map(|x| format!("{}{}", if x > 0.0 { "+" } else { "" }, f::num(x, 2)))
        .unwrap_or_else(|| f::DASH.into())
}

pub fn rotation(r: &Rotation, color: bool) -> String {
    let m = &r.method;
    let every = match r.interval {
        RotationInterval::Week => "weekly",
        RotationInterval::Day => "daily",
    };
    let bench = match &r.benchmark_name {
        Some(n) => format!("{} ({n})", r.benchmark),
        None => r.benchmark.to_string(),
    };
    let as_of = r
        .rows
        .iter()
        .filter_map(|x| x.as_of)
        .max()
        .map(|d| format!(" · as of {d}"))
        .unwrap_or_default();
    let mut out = format!(
        "Relative rotation vs {bench} · {every}, window {}, tail {}{as_of}\n",
        m.window, m.tail
    );
    out.push_str(
        "Open approximation of the JdK RS-Ratio and RS-Momentum; values differ from StockCharts and Optuma.\n",
    );
    let bars = match r.interval {
        RotationInterval::Week => "Weeks",
        RotationInterval::Day => "Sessions",
    };
    let mut t = base_table(color);
    t.set_header(header(&[
        "Symbol",
        "Quadrant",
        "RS-Ratio",
        "RS-Momentum",
        "Ratio chg",
        "Momentum chg",
        "As of",
        bars,
    ]));
    for row in &r.rows {
        t.add_row(vec![
            Cell::new(row.symbol.as_str()).add_attribute(Attribute::Bold),
            quadrant_cell(row.quadrant, color),
            right(axis(row.rs_ratio)),
            right(axis(row.rs_momentum)),
            signed(change(row.rs_ratio_change), row.rs_ratio_change, color),
            signed(
                change(row.rs_momentum_change),
                row.rs_momentum_change,
                color,
            ),
            Cell::new(
                row.as_of
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| f::DASH.into()),
            ),
            right(row.observations.to_string()),
        ]);
    }
    out.push_str(&t.to_string());
    out.push('\n');

    // The graph's four quadrants, where it draws them: momentum up, ratio right.
    let names = |q: Quadrant| {
        let list: Vec<&str> = r
            .rows
            .iter()
            .filter(|x| x.quadrant == Some(q))
            .map(|x| x.symbol.as_str())
            .collect();
        if list.is_empty() {
            f::DASH.to_string()
        } else {
            list.join(" ")
        }
    };
    if r.rows.iter().any(|x| x.quadrant.is_some()) {
        let mut g = base_table(color);
        let cell = |q: Quadrant| Cell::new(format!("{}: {}", q.as_str(), names(q)));
        let band = |s: &str| Cell::new(s).add_attribute(Attribute::Bold);
        g.add_row(vec![
            band("RS-Momentum >= 100"),
            cell(Quadrant::Improving),
            cell(Quadrant::Leading),
        ]);
        g.add_row(vec![
            band("RS-Momentum < 100"),
            cell(Quadrant::Lagging),
            cell(Quadrant::Weakening),
        ]);
        g.add_row(vec![
            Cell::new(""),
            Cell::new("RS-Ratio < 100").add_attribute(Attribute::Bold),
            Cell::new("RS-Ratio >= 100").add_attribute(Attribute::Bold),
        ]);
        out.push_str(&g.to_string());
        out.push('\n');
    }
    out.push_str(
        "Axes in standard deviations around 100, not percent. Rotation runs clockwise: improving, leading, weakening, lagging. Not investment advice.\n",
    );
    out
}
