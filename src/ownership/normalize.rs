//! Name normalisation. The raw register spells the same holder several ways
//! (`PT ALAMTRI RESOURCES INDONESIA TBK`, `ALAMTRI RESOURCES INDONESIA TBK PT`,
//! `Alamtri  Resources Indonesia, Tbk.`). `entity_key` maps all of them to one
//! key; `display_name` keeps a tidy upper-case form for tables.

/// Legal-form tokens that move around or get dropped between filings.
const DROPPED_TOKENS: &[&str] = &["PT", "TBK", "PERSERO", "PT.", "TBK."];

fn tokens(raw: &str) -> Vec<String> {
    raw.to_uppercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Grouping key: upper-case alphanumerics, single spaces, legal forms removed.
pub fn entity_key(raw: &str) -> String {
    let toks: Vec<String> = tokens(raw)
        .into_iter()
        .filter(|t| !DROPPED_TOKENS.contains(&t.as_str()))
        .collect();
    if toks.is_empty() {
        tokens(raw).join(" ")
    } else {
        toks.join(" ")
    }
}

/// Upper-case, single-spaced, punctuation trimmed; keeps the legal form.
pub fn display_name(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| c == ',' || c == '.' || c == ';')
        .to_uppercase()
}

/// KSEI investor type: two-letter codes (Feb/Mar 2026 files) become the
/// official names; words are upper-cased as printed.
pub fn investor_type(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    Some(
        match t.to_uppercase().as_str() {
            "CP" => "CORPORATE",
            "ID" => "INDIVIDUAL",
            "IB" => "FINANCIAL INSTITUTION",
            "MF" => "MUTUAL FUND",
            "SC" => "SECURITIES COMPANY",
            "IS" => "INSURANCE",
            "PF" => "PENSION FUND",
            "FD" => "FOUNDATION",
            "OT" => "OTHERS",
            other => return Some(other.split_whitespace().collect::<Vec<_>>().join(" ")),
        }
        .to_string(),
    )
}

/// `L` / `A` (or the words local/lokal/foreign/asing) to a single letter.
pub fn local_foreign(raw: &str) -> Option<String> {
    match raw.trim().to_uppercase().as_str() {
        "" => None,
        "L" | "LOKAL" | "LOCAL" | "D" | "DOMESTIK" | "DOMESTIC" => Some("L".into()),
        "A" | "ASING" | "FOREIGN" | "F" => Some("A".into()),
        other => Some(other.to_string()),
    }
}

/// `CONTROLLER_OR_AFFILIATE` (registers from 30 Oct 2026): single-spaced and
/// upper-cased as printed, blank -> `None`. Known values: `CONTROLLER`,
/// `AFFILIATE OF THE CONTROLLER`.
pub fn controller_or_affiliate(raw: &str) -> Option<String> {
    opt(raw).map(|t| t.to_uppercase())
}

/// `None` for blank cells, trimmed text otherwise.
pub fn opt(raw: &str) -> Option<String> {
    let t = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_share_a_key() {
        let k = entity_key("PT ALAMTRI RESOURCES INDONESIA TBK");
        assert_eq!(k, "ALAMTRI RESOURCES INDONESIA");
        assert_eq!(entity_key("ALAMTRI RESOURCES INDONESIA TBK PT"), k);
        assert_eq!(entity_key("Alamtri  Resources Indonesia, Tbk."), k);
        assert_eq!(entity_key("PT. Alamtri Resources Indonesia"), k);
        assert_ne!(entity_key("ALAMTRI RESOURCES"), k);
    }

    #[test]
    fn key_never_empty() {
        assert_eq!(entity_key("PT"), "PT");
        assert_eq!(entity_key("PT Tbk"), "PT TBK");
    }

    #[test]
    fn display_and_types() {
        assert_eq!(
            display_name("  Adaro   Strategic Investments, "),
            "ADARO STRATEGIC INVESTMENTS"
        );
        assert_eq!(investor_type("CP").as_deref(), Some("CORPORATE"));
        assert_eq!(
            investor_type("Private Bank").as_deref(),
            Some("PRIVATE BANK")
        );
        assert_eq!(investor_type(" "), None);
        assert_eq!(local_foreign("Asing").as_deref(), Some("A"));
        assert_eq!(local_foreign("l").as_deref(), Some("L"));
        assert_eq!(opt("   "), None);
        assert_eq!(
            controller_or_affiliate("  affiliate of  the controller ").as_deref(),
            Some("AFFILIATE OF THE CONTROLLER")
        );
        assert_eq!(
            controller_or_affiliate("CONTROLLER").as_deref(),
            Some("CONTROLLER")
        );
        assert_eq!(controller_or_affiliate(" \u{a0}"), None);
    }
}
