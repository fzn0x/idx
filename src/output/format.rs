//! Number formatting for tables. JSON never goes through here.

pub const DASH: &str = "—";

/// Thousands separators, fixed decimals.
pub fn num(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return DASH.to_string();
    }
    let s = format!("{:.*}", decimals, v.abs());
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s.as_str(), None),
    };
    let mut grouped = String::with_capacity(int.len() + int.len() / 3);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let mut out = String::new();
    if v < 0.0 {
        out.push('-');
    }
    out.push_str(&grouped);
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    out
}

/// IDR prices are whole rupiah; show decimals only when they exist.
pub fn price(v: Option<f64>) -> String {
    match v {
        None => DASH.to_string(),
        Some(x) if (x - x.round()).abs() < 1e-9 => num(x, 0),
        Some(x) => num(x, 2),
    }
}

/// Signed absolute change, same rule as `price`.
pub fn change(v: Option<f64>) -> String {
    match v {
        None => DASH.to_string(),
        Some(x) => {
            let body = price(Some(x.abs()));
            if x > 0.0 {
                format!("+{body}")
            } else if x < 0.0 {
                format!("-{body}")
            } else {
                body
            }
        }
    }
}

/// `+1.23%`, `-14.81%`, `0.00%`.
pub fn pct(v: Option<f64>) -> String {
    match v {
        None => DASH.to_string(),
        Some(x) if x > 0.0 => format!("+{}%", num(x, 2)),
        Some(x) => format!("{}%", num(x, 2)),
    }
}

pub fn int(v: Option<u64>) -> String {
    v.map(|x| num(x as f64, 0))
        .unwrap_or_else(|| DASH.to_string())
}

/// Two decimals, or a dash.
pub fn ratio(v: Option<f64>) -> String {
    v.map(|x| num(x, 2)).unwrap_or_else(|| DASH.to_string())
}

/// Compact magnitude: 635.71T, 12.3B, 950.0M, 12.5K.
pub fn compact(v: Option<f64>) -> String {
    let Some(x) = v else { return DASH.to_string() };
    let a = x.abs();
    let (scaled, suffix) = if a >= 1e12 {
        (x / 1e12, "T")
    } else if a >= 1e9 {
        (x / 1e9, "B")
    } else if a >= 1e6 {
        (x / 1e6, "M")
    } else if a >= 1e3 {
        (x / 1e3, "K")
    } else {
        return num(x, 0);
    };
    let decimals = if scaled.abs() >= 100.0 { 1 } else { 2 };
    format!("{}{suffix}", num(scaled, decimals))
}

pub fn text(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| DASH.to_string())
}

/// Truncates with an ellipsis so names do not blow up table width.
pub fn clip(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// A one-line chart of `values` in block characters, at most `width` cells:
/// longer series are resampled (each cell is the last value of its stretch),
/// the lowest value maps to the lowest block and the highest to the full one.
/// Empty input gives an empty string; a flat series sits at mid height.
pub fn sparkline(values: &[f64], width: usize) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let values: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    let n = values.len();
    if n == 0 || width == 0 {
        return String::new();
    }
    let cells = n.min(width);
    let points: Vec<f64> = (0..cells)
        .map(|i| values[((i + 1) * n).div_ceil(cells) - 1])
        .collect();
    let lo = points.iter().copied().fold(f64::MAX, f64::min);
    let hi = points.iter().copied().fold(f64::MIN, f64::max);
    points
        .iter()
        .map(|v| {
            if hi - lo < 1e-12 {
                BLOCKS[3]
            } else {
                let level = ((v - lo) / (hi - lo) * 7.0).round() as usize;
                BLOCKS[level.min(7)]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping() {
        assert_eq!(num(5175.0, 0), "5,175");
        assert_eq!(num(1234567.891, 2), "1,234,567.89");
        assert_eq!(num(-900.0, 0), "-900");
        assert_eq!(num(12.0, 0), "12");
        assert_eq!(num(f64::NAN, 2), DASH);
    }

    #[test]
    fn price_and_change() {
        assert_eq!(price(Some(5175.0)), "5,175");
        assert_eq!(price(Some(5175.5)), "5,175.50");
        assert_eq!(price(None), DASH);
        assert_eq!(change(Some(-900.0)), "-900");
        assert_eq!(change(Some(25.0)), "+25");
        assert_eq!(change(Some(0.0)), "0");
    }

    #[test]
    fn percentages_and_compact() {
        assert_eq!(pct(Some(-14.814815)), "-14.81%");
        assert_eq!(pct(Some(6.27)), "+6.27%");
        assert_eq!(pct(Some(0.0)), "0.00%");
        assert_eq!(compact(Some(635_706_095_435_776.0)), "635.7T");
        assert_eq!(compact(Some(12_300_000_000.0)), "12.30B");
        assert_eq!(compact(Some(950_000_000.0)), "950.0M");
        assert_eq!(compact(Some(12_500.0)), "12.50K");
        assert_eq!(compact(Some(999.0)), "999");
        assert_eq!(compact(None), DASH);
    }

    #[test]
    fn sparklines() {
        assert_eq!(sparkline(&[], 10), "");
        assert_eq!(sparkline(&[1.0, 2.0, 3.0], 0), "");
        assert_eq!(sparkline(&[1.0, 8.0], 10), "▁█");
        assert_eq!(sparkline(&[5.0, 5.0, 5.0], 10), "▄▄▄");
        let ramp: Vec<f64> = (0..8).map(f64::from).collect();
        assert_eq!(sparkline(&ramp, 8), "▁▂▃▄▅▆▇█");
        // 100 points into 10 cells: each cell is the last value of its tenth.
        let long: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = sparkline(&long, 10);
        assert_eq!(s.chars().count(), 10);
        assert!(s.starts_with('▁') && s.ends_with('█'), "{s}");
        assert_eq!(sparkline(&[1.0, f64::NAN, 3.0], 5), "▁█");
    }

    #[test]
    fn clipping() {
        assert_eq!(clip("Bank Central Asia", 30), "Bank Central Asia");
        assert_eq!(clip("PT Bank Central Asia Tbk", 10), "PT Bank C…");
    }
}
