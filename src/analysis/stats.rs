//! Small-sample statistics: mean, median, sample standard deviation, Welch's
//! t-test and the two-sided p-value of Student's t. The special functions
//! follow Numerical Recipes (`gammln`, `betacf`, `betai`); `web/src/astro.js`
//! carries the same code.

pub fn mean(a: &[f64]) -> f64 {
    a.iter().sum::<f64>() / a.len() as f64
}

/// Sample variance (n - 1 in the denominator) around `m`.
pub fn variance(a: &[f64], m: f64) -> f64 {
    a.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (a.len() as f64 - 1.0)
}

pub fn median(a: &[f64]) -> f64 {
    let mut s = a.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    if n % 2 == 1 {
        s[(n - 1) / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// ln Γ(x), Lanczos approximation.
fn ln_gamma(x: f64) -> f64 {
    const C: [f64; 6] = [
        76.18009172947146,
        -86.50532032941677,
        24.01409824083091,
        -1.231739572450155,
        0.1208650973866179e-2,
        -0.5395239384953e-5,
    ];
    let mut y = x;
    let mut tmp = x + 5.5;
    tmp -= (x + 0.5) * tmp.ln();
    let mut ser = 1.000000000190015;
    for c in C {
        y += 1.0;
        ser += c / y;
    }
    -tmp + (2.5066282746310005 * ser / x).ln()
}

/// Continued fraction of the incomplete beta function (modified Lentz).
fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    const MAXIT: usize = 300;
    const EPS: f64 = 3e-14;
    const FPMIN: f64 = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAXIT {
        let m = m as f64;
        let m2 = 2.0 * m;
        let mut aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

/// Regularized incomplete beta I_x(a, b).
fn beta_inc(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * beta_cf(a, b, x) / a
    } else {
        1.0 - bt * beta_cf(b, a, 1.0 - x) / b
    }
}

/// Two-sided p-value of Student's t with `df` degrees of freedom.
pub fn student_t_two_sided(t: f64, df: f64) -> f64 {
    beta_inc(df / 2.0, 0.5, df / (df + t * t))
}

/// Welch's t-test of `a` against `b`: (t, Welch-Satterthwaite df, two-sided p);
/// `None` with fewer than 2 values on either side or no variance at all.
pub fn welch(a: &[f64], b: &[f64]) -> Option<(f64, f64, f64)> {
    if a.len() < 2 || b.len() < 2 {
        return None;
    }
    let (m1, m2) = (mean(a), mean(b));
    let v1 = variance(a, m1) / a.len() as f64;
    let v2 = variance(b, m2) / b.len() as f64;
    let se = (v1 + v2).sqrt();
    if se.is_nan() || se <= 0.0 {
        return None;
    }
    let t = (m1 - m2) / se;
    let df = (v1 + v2).powi(2)
        / (v1.powi(2) / (a.len() as f64 - 1.0) + v2.powi(2) / (b.len() as f64 - 1.0));
    Some((t, df, student_t_two_sided(t, df)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r5(x: f64) -> f64 {
        (x * 1e5).round() / 1e5
    }

    #[test]
    fn p_values_of_student_t() {
        assert_eq!(r5(student_t_two_sided(2.0, 10.0)), 0.07339);
        assert_eq!(r5(student_t_two_sided(1.96, 1e6)), 0.05);
        assert_eq!(r5(student_t_two_sided(3.0, 5.0)), 0.0301);
        assert_eq!(student_t_two_sided(0.0, 8.0), 1.0);
    }

    #[test]
    fn welch_and_moments() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(mean(&[1.0, 2.0, 3.0]), 2.0);
        assert_eq!(variance(&[1.0, 2.0, 3.0], 2.0), 1.0);
        assert!(welch(&[1.0], &[1.0, 2.0]).is_none());
        assert!(welch(&[1.0, 1.0], &[2.0, 2.0]).is_none(), "no variance");
        let (t, df, p) = welch(&[1.0, 2.0, 3.0, 4.0], &[2.0, 4.0, 6.0, 8.0, 10.0]).unwrap();
        assert!((t - -2.251436).abs() < 1e-6, "{t}");
        assert!((df - 5.520788).abs() < 1e-6, "{df}");
        assert!((p - 0.069134).abs() < 1e-6, "{p}");
    }
}
