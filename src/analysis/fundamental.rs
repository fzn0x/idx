//! Plain-language valuation verdict. A handful of transparent rules; every
//! rule that fires is reported as a reason, so the reader can disagree.

use crate::model::{Fundamentals, Verdict, VerdictLabel};

/// Scores the ratios and writes the verdict into `f.verdict`.
///
/// Two guards keep upside-down figures from scoring:
/// - Negative shareholders' equity ([`Fundamentals::negative_equity`]) is one
///   adverse rule (-2) and sets the P/B, ROE and debt/equity rules aside: a
///   negative P/B is not "below book", a negative debt/equity is not
///   "conservative", and a loss over negative equity makes ROE positive. The
///   set-aside figures still count in `inputs_available` (they were reported).
/// - When the statements are in another currency than the price
///   ([`Fundamentals::mixed_currency`]), the source's P/B divides one by the
///   other, so it is neither scored, counted nor listed as a reason;
///   `financial_currency` and the tables' `*` note say why it is missing.
pub fn verdict(f: &Fundamentals) -> Verdict {
    let mut score = 0;
    let mut reasons = Vec::new();
    let mut inputs = 0;
    let bank = f.is_financial_sector();
    let negative_equity = f.negative_equity() == Some(true);
    let mixed_currency = f.mixed_currency();
    let v = &f.valuation;
    let p = &f.profitability;

    if let Some(pe) = v.pe_trailing {
        inputs += 1;
        if pe <= 0.0 {
            score -= 1;
            reasons.push("negative trailing earnings".into());
        } else if pe < 10.0 {
            score += 2;
            reasons.push(format!("trailing P/E {pe:.1} is low (under 10)"));
        } else if pe < 15.0 {
            score += 1;
            reasons.push(format!("trailing P/E {pe:.1} is moderate (10 to 15)"));
        } else if pe > 35.0 {
            score -= 2;
            reasons.push(format!("trailing P/E {pe:.1} is high (over 35)"));
        } else if pe > 25.0 {
            score -= 1;
            reasons.push(format!("trailing P/E {pe:.1} is elevated (over 25)"));
        } else {
            reasons.push(format!("trailing P/E {pe:.1} is in a normal range"));
        }
    }

    if negative_equity {
        inputs += 1;
        score -= 2;
        reasons.push(
            "negative shareholders' equity (book value below zero): P/B, ROE and debt/equity rules skipped"
                .into(),
        );
    }

    if let Some(pb) = v.price_to_book {
        if negative_equity {
            // Set aside by the negative-equity rule.
            inputs += 1;
        } else if !mixed_currency {
            // A price over a book value in another currency is no ratio, so
            // a mixed-currency P/B gets nothing at all.
            inputs += 1;
            if pb < 1.0 {
                score += 1;
                reasons.push(format!("trades below book value (P/B {pb:.2})"));
            } else if pb > 4.0 {
                score -= 1;
                reasons.push(format!("P/B {pb:.2} is rich (over 4)"));
            }
        }
    }

    if let Some(peg) = v.peg {
        inputs += 1;
        if peg > 0.0 && peg < 1.0 {
            score += 1;
            reasons.push(format!("PEG {peg:.2} under 1: growth is cheap"));
        } else if peg > 2.0 {
            score -= 1;
            reasons.push(format!("PEG {peg:.2} over 2: paying up for growth"));
        }
    }

    if let Some(roe) = p.roe_pct {
        inputs += 1;
        let net_loss = p.net_margin_pct.is_some_and(|m| m < 0.0)
            || f.per_share.eps_ttm.is_some_and(|e| e < 0.0);
        // With negative equity the rule above already set ROE aside.
        if !negative_equity {
            if roe > 15.0 && net_loss {
                // ROE = earnings / equity: a positive ROE beside a loss means
                // the equity behind it is negative or the two figures cover
                // different periods.
                reasons.push(format!("ROE {roe:.1}% alongside a net loss: rule skipped"));
            } else if roe > 15.0 {
                score += 1;
                reasons.push(format!("ROE {roe:.1}% is strong (over 15%)"));
            } else if roe < 5.0 {
                score -= 1;
                reasons.push(format!("ROE {roe:.1}% is weak (under 5%)"));
            }
        }
    }

    if let Some(nm) = p.net_margin_pct {
        inputs += 1;
        if nm > 20.0 {
            score += 1;
            reasons.push(format!("net margin {nm:.1}% is wide"));
        } else if nm < 5.0 {
            score -= 1;
            reasons.push(format!("net margin {nm:.1}% is thin"));
        }
    }

    if !bank {
        if let Some(de) = f.health.debt_to_equity {
            inputs += 1;
            // With negative equity the rule above already set it aside.
            if !negative_equity {
                if de > 2.0 {
                    score -= 1;
                    reasons.push(format!("debt is {de:.1}x equity: heavily leveraged"));
                } else if de < 0.5 {
                    score += 1;
                    reasons.push(format!(
                        "debt is {de:.2}x equity: conservative balance sheet"
                    ));
                }
            }
        }
    } else {
        reasons.push("leverage and margin rules relaxed for a financial company".into());
    }

    if let Some(g) = f.growth.revenue_growth_pct {
        inputs += 1;
        if g > 10.0 {
            score += 1;
            reasons.push(format!("revenue growing {g:.1}% year on year"));
        } else if g < 0.0 {
            score -= 1;
            reasons.push(format!("revenue shrinking {:.1}% year on year", -g));
        }
    }

    if let Some(g) = f.growth.earnings_growth_pct {
        inputs += 1;
        if g > 10.0 {
            score += 1;
            reasons.push(format!("earnings growing {g:.1}% year on year"));
        } else if g < 0.0 {
            score -= 1;
            reasons.push(format!("earnings shrinking {:.1}% year on year", -g));
        }
    }

    if let Some(y) = f.dividend.yield_pct {
        inputs += 1;
        if y >= 4.0 {
            score += 1;
            reasons.push(format!("dividend yield {y:.1}% is generous"));
        }
    }

    if let (Some(price), Some(target)) = (f.price, f.analyst.target_mean) {
        if price > 0.0 {
            let upside = (target - price) / price * 100.0;
            if upside > 20.0 {
                score += 1;
                reasons.push(format!("analyst mean target implies {upside:.0}% upside"));
            } else if upside < -10.0 {
                score -= 1;
                reasons.push(format!(
                    "analyst mean target sits {:.0}% below the price",
                    -upside
                ));
            }
        }
    }

    let label = if inputs < 3 {
        VerdictLabel::InsufficientData
    } else if score >= 3 {
        VerdictLabel::Undervalued
    } else if score <= -3 {
        VerdictLabel::Overvalued
    } else {
        VerdictLabel::Fair
    };
    Verdict {
        label,
        score,
        reasons,
        inputs_available: inputs,
    }
}

/// One-paragraph summary for the table footer. The noun follows the count:
/// "1 input", "8 inputs".
pub fn summary_line(f: &Fundamentals) -> String {
    let Some(v) = &f.verdict else {
        return String::new();
    };
    let head = match v.label {
        VerdictLabel::Undervalued => "Looks undervalued on these ratios",
        VerdictLabel::Fair => "Looks fairly valued on these ratios",
        VerdictLabel::Overvalued => "Looks expensive on these ratios",
        VerdictLabel::InsufficientData => "Not enough data for a reading",
    };
    format!(
        "{head} (score {:+}, {} input{}). Heuristic from public ratios, not investment advice.",
        v.score,
        v.inputs_available,
        if v.inputs_available == 1 { "" } else { "s" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Symbol;

    fn base() -> Fundamentals {
        Fundamentals::empty(Symbol::parse("TEST").unwrap(), "t")
    }

    #[test]
    fn cheap_quality_company_is_undervalued() {
        let mut f = base();
        f.valuation.pe_trailing = Some(8.0);
        f.valuation.price_to_book = Some(0.8);
        f.profitability.roe_pct = Some(18.0);
        f.dividend.yield_pct = Some(5.0);
        let v = verdict(&f);
        assert_eq!(v.label, VerdictLabel::Undervalued);
        assert!(v.score >= 3);
        assert!(v.reasons.iter().any(|r| r.contains("below book")));
    }

    #[test]
    fn expensive_company_is_overvalued() {
        let mut f = base();
        f.valuation.pe_trailing = Some(50.0);
        f.valuation.price_to_book = Some(9.0);
        f.valuation.peg = Some(3.0);
        f.growth.revenue_growth_pct = Some(-5.0);
        assert_eq!(verdict(&f).label, VerdictLabel::Overvalued);
    }

    #[test]
    fn too_little_data() {
        let mut f = base();
        f.valuation.pe_trailing = Some(5.0);
        let v = verdict(&f);
        assert_eq!(v.label, VerdictLabel::InsufficientData);
        assert_eq!(v.inputs_available, 1);
    }

    #[test]
    fn summary_line_agrees_with_the_input_count() {
        let mut f = base();
        f.valuation.pe_trailing = Some(12.9);
        f.verdict = Some(verdict(&f));
        assert_eq!(
            summary_line(&f),
            "Not enough data for a reading (score +1, 1 input). Heuristic from public ratios, not investment advice."
        );
        // A net margin between 5% and 20% counts without moving the score.
        f.profitability.net_margin_pct = Some(10.0);
        f.verdict = Some(verdict(&f));
        assert_eq!(
            summary_line(&f),
            "Not enough data for a reading (score +1, 2 inputs). Heuristic from public ratios, not investment advice."
        );
    }

    #[test]
    fn banks_skip_leverage_rule() {
        let mut f = base();
        f.sector = Some("Financial Services".into());
        f.health.debt_to_equity = Some(9.0);
        f.valuation.pe_trailing = Some(12.0);
        f.valuation.price_to_book = Some(2.0);
        f.profitability.roe_pct = Some(20.0);
        let v = verdict(&f);
        assert!(!v.reasons.iter().any(|r| r.contains("leveraged")));
        assert_eq!(v.label, VerdictLabel::Fair);
    }

    const NEGATIVE_EQUITY: &str =
        "negative shareholders' equity (book value below zero): P/B, ROE and debt/equity rules skipped";

    fn summary(v: &Verdict) -> (VerdictLabel, i32, usize) {
        (v.label, v.score, v.inputs_available)
    }

    fn rewards_leverage_or_book(v: &Verdict) -> bool {
        v.reasons
            .iter()
            .any(|r| r.contains("conservative") || r.contains("below book") || r.contains("strong"))
    }

    #[test]
    fn negative_equity_is_adverse_and_sets_pb_roe_and_leverage_aside() {
        // GIAA's merged inputs on 2026-11-06 (Yahoo, gaps filled by MSN).
        let mut f = base();
        f.price = Some(39.0);
        f.per_share.book_value = Some(-0.001);
        f.per_share.eps_ttm = Some(-23.54);
        f.profitability.net_margin_pct = Some(-9.375);
        f.growth.revenue_growth_pct = Some(8.2);
        f.growth.earnings_growth_pct = Some(-92.482);
        f.dividend.yield_pct = Some(0.0);
        f.analyst.target_mean = Some(130.0);
        let v = verdict(&f);
        assert_eq!(summary(&v), (VerdictLabel::Overvalued, -3, 5));
        assert_eq!(v.reasons[0], NEGATIVE_EQUITY);
        assert!(!rewards_leverage_or_book(&v), "{:?}", v.reasons);
    }

    #[test]
    fn negative_ratios_mean_negative_equity() {
        // A negative P/B is not "below book".
        let mut f = base();
        f.valuation.price_to_book = Some(-0.55);
        f.profitability.net_margin_pct = Some(-51.5);
        f.growth.revenue_growth_pct = Some(-17.6);
        let v = verdict(&f);
        assert_eq!(summary(&v), (VerdictLabel::Overvalued, -4, 4));
        assert!(!rewards_leverage_or_book(&v), "{:?}", v.reasons);

        // A negative debt/equity is not conservative, and the ROE it implies
        // (a loss over negative equity) is not strong.
        let mut f = base();
        f.health.debt_to_equity = Some(-2.89);
        f.profitability.roe_pct = Some(92.0);
        f.profitability.net_margin_pct = Some(-9.4);
        let v = verdict(&f);
        assert_eq!(summary(&v), (VerdictLabel::Overvalued, -3, 4));
        assert!(!rewards_leverage_or_book(&v), "{:?}", v.reasons);
        assert_eq!(v.reasons[0], NEGATIVE_EQUITY);
    }

    #[test]
    fn negative_equity_applies_to_banks_too() {
        let mut f = base();
        f.sector = Some("Financial Services".into());
        f.per_share.book_value = Some(-5.0);
        f.health.debt_to_equity = Some(-3.0);
        let v = verdict(&f);
        assert_eq!(v.score, -2);
        assert!(v.reasons.iter().any(|r| r == NEGATIVE_EQUITY));
        assert!(v.reasons.iter().any(|r| r.contains("financial company")));
    }

    #[test]
    fn zero_book_value_is_not_negative_equity() {
        for bv in [0.0, -0.0] {
            let mut f = base();
            f.per_share.book_value = Some(bv);
            f.health.debt_to_equity = Some(0.3);
            let v = verdict(&f);
            assert!(!v.reasons.iter().any(|r| r == NEGATIVE_EQUITY), "{bv}");
            assert!(v.reasons.iter().any(|r| r.contains("conservative")), "{bv}");
        }
    }

    #[test]
    fn positive_roe_next_to_a_loss_is_not_rewarded() {
        let mut f = base();
        f.valuation.pe_trailing = Some(12.0);
        f.profitability.roe_pct = Some(40.0);
        f.profitability.net_margin_pct = Some(-12.0);
        f.health.debt_to_equity = Some(0.3);
        let v = verdict(&f);
        assert!(v
            .reasons
            .iter()
            .any(|r| r == "ROE 40.0% alongside a net loss: rule skipped"));
        assert!(!v.reasons.iter().any(|r| r.contains("is strong")));
        assert_eq!(summary(&v), (VerdictLabel::Fair, 1, 4));

        // A loss read from EPS counts too.
        let mut f = base();
        f.profitability.roe_pct = Some(40.0);
        f.per_share.eps_ttm = Some(-3.0);
        assert!(verdict(&f).reasons[0].contains("alongside a net loss"));
    }

    #[test]
    fn pb_is_not_scored_when_statements_use_another_currency() {
        // ITMG on 2026-11-06: an IDR price over a USD book value.
        let mut f = base();
        f.currency = Some("IDR".into());
        f.financial_currency = Some("USD".into());
        f.valuation.pe_trailing = Some(7.46);
        f.valuation.price_to_book = Some(14103.31);
        f.profitability.roe_pct = Some(11.1);
        let v = verdict(&f);
        assert!(
            !v.reasons.iter().any(|r| r.contains("P/B")),
            "no P/B reason at all: {:?}",
            v.reasons
        );
        assert_eq!(v.inputs_available, 2, "the mixed P/B is not an input");

        // Same currency, same figures: the P/B rule runs as before.
        f.financial_currency = Some("IDR".into());
        let v = verdict(&f);
        assert!(v.reasons.iter().any(|r| r.contains("rich")));
        assert_eq!(v.inputs_available, 3);
    }

    mod fixtures {
        use super::*;
        use crate::providers::{msn, yahoo};

        const Y_BBCA: &str =
            include_str!("../../tests/fixtures/yahoo/quote_summary_fundamentals_bbca.json");
        const Y_GIAA: &str =
            include_str!("../../tests/fixtures/yahoo/quote_summary_fundamentals_giaa.json");
        const Y_ITMG: &str =
            include_str!("../../tests/fixtures/yahoo/quote_summary_fundamentals_itmg.json");
        const M_GIAA: &str = include_str!("../../tests/fixtures/msn/equities_giaa.json");

        fn from_yahoo(body: &str, s: &str) -> Fundamentals {
            yahoo::summary::parse_fundamentals(body, &Symbol::parse(s).unwrap()).unwrap()
        }

        #[test]
        fn giaa_merged_like_auto() {
            // `auto`: Yahoo first, MSN fills the nulls (Chain::fundamentals).
            let mut f = from_yahoo(Y_GIAA, "GIAA");
            let m = msn::parse::parse_fundamentals(M_GIAA, &Symbol::parse("GIAA").unwrap(), None)
                .unwrap()
                .unwrap();
            f.fill_from(&m);
            assert_eq!(f.health.debt_to_equity, None, "MSN placeholder dropped");
            assert_eq!(f.per_share.book_value, Some(-0.001));
            assert_eq!(f.growth.earnings_growth_pct, Some(-92.482), "filled by MSN");
            let v = verdict(&f);
            assert_eq!(summary(&v), (VerdictLabel::Overvalued, -3, 5));
            assert_eq!(
                v.reasons,
                vec![
                    NEGATIVE_EQUITY.to_string(),
                    "net margin -9.4% is thin".into(),
                    "earnings shrinking 92.5% year on year".into(),
                    "analyst mean target implies 233% upside".into(),
                ]
            );
        }

        #[test]
        fn giaa_yahoo_only_matches_the_web() {
            let v = verdict(&from_yahoo(Y_GIAA, "GIAA"));
            assert_eq!(summary(&v), (VerdictLabel::Fair, -2, 4));
            assert_eq!(v.reasons[0], NEGATIVE_EQUITY);
        }

        #[test]
        fn itmg_pb_is_set_aside() {
            // #25: the ITMG verdict lists no P/B reason.
            let v = verdict(&from_yahoo(Y_ITMG, "ITMG"));
            assert!(
                !v.reasons.iter().any(|r| r.contains("P/B")),
                "{:?}",
                v.reasons
            );
            assert_eq!(summary(&v), (VerdictLabel::Undervalued, 7, 8));
        }

        #[test]
        fn bbca_is_unchanged() {
            let v = verdict(&from_yahoo(Y_BBCA, "BBCA"));
            assert_eq!(summary(&v), (VerdictLabel::Undervalued, 5, 8));
            assert_eq!(
                v.reasons,
                vec![
                    "trailing P/E 12.9 is moderate (10 to 15)".to_string(),
                    "ROE 21.8% is strong (over 15%)".into(),
                    "net margin 53.1% is wide".into(),
                    "leverage and margin rules relaxed for a financial company".into(),
                    "dividend yield 6.3% is generous".into(),
                    "analyst mean target implies 58% upside".into(),
                ]
            );
        }
    }
}
