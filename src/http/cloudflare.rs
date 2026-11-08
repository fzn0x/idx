//! Telling a Cloudflare refusal apart from an ordinary HTTP failure.
//!
//! Sites behind Cloudflare bot management (idx.co.id) answer some requests,
//! now and then from any network and more often from data centres, VPNs and
//! CI runners, with one of two pages:
//!
//! - a **challenge** ("Just a moment...", `window._cf_chl_opt`), which only a
//!   browser running its JavaScript can pass, though it often lifts within a
//!   minute; Cloudflare marks it with the response header
//!   `cf-mitigated: challenge`;
//! - a **block page** ("Attention Required! | Cloudflare", "Sorry, you have
//!   been blocked"), served without that header.
//!
//! Both become [`IdxError::Blocked`](crate::error::IdxError::Blocked) (JSON
//! code `blocked`, exit 3). Anything else keeps its HTTP status and is not
//! described as Cloudflare: a plain 503 is an outage, not bot protection.
//! The detector knows no source URL, so every source can use it.

/// What kind of refusal a response is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A managed or JavaScript challenge only a browser can solve; it often
    /// lifts within a minute, so a later request can pass without one.
    Challenge,
    /// A block page (firewall rule or bot score); no browser step passes it.
    Block,
}

impl Refusal {
    /// `a Cloudflare challenge` / `a Cloudflare block page`.
    pub fn noun(self) -> &'static str {
        match self {
            Refusal::Challenge => "a Cloudflare challenge",
            Refusal::Block => "a Cloudflare block page",
        }
    }
}

/// How much of the body is searched for the page markers.
const HEAD: usize = 8 * 1024;

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
}

/// Classifies a response. `headers` as the backend returns them (names in
/// any case); only the first 8 KiB of `body` are read. Rules, in order:
///
/// 1. header `cf-mitigated: challenge` (any status): a challenge;
/// 2. status 403, 429 or 503 with a block page's title or markers: a block
///    (checked before 3: block pages also load `/cdn-cgi/challenge-platform/`);
/// 3. status 403, 429 or 503 with a challenge page's markers: a challenge;
/// 4. status 403 with `server: cloudflare`: a block;
/// 5. a 2xx whose body is the challenge page itself (its title or
///    `_cf_chl_opt`): a challenge. Ordinary pages behind Cloudflare also load
///    `/cdn-cgi/challenge-platform/` scripts, so that alone never counts;
/// 6. anything else: `None`.
pub fn refusal(status: u16, headers: &[(String, String)], body: &[u8]) -> Option<Refusal> {
    if header(headers, "cf-mitigated").is_some_and(|v| v.eq_ignore_ascii_case("challenge")) {
        return Some(Refusal::Challenge);
    }
    let head = String::from_utf8_lossy(&body[..body.len().min(HEAD)]).to_ascii_lowercase();
    let refused = matches!(status, 403 | 429 | 503);
    if refused
        && (head.contains("attention required! | cloudflare")
            || head.contains("sorry, you have been blocked")
            || head.contains("cf-error-details"))
    {
        return Some(Refusal::Block);
    }
    let challenge_page = head.contains("<title>just a moment") || head.contains("_cf_chl_opt");
    if refused
        && (challenge_page || head.contains("cf-chl") || head.contains("challenges.cloudflare.com"))
    {
        return Some(Refusal::Challenge);
    }
    if status == 403
        && header(headers, "server").is_some_and(|v| v.eq_ignore_ascii_case("cloudflare"))
    {
        return Some(Refusal::Block);
    }
    if (200..300).contains(&status) && challenge_page {
        return Some(Refusal::Challenge);
    }
    None
}

/// `a Cloudflare challenge (HTTP 403, cf-mitigated: challenge)` or
/// `a Cloudflare block page (HTTP 403)`.
pub fn describe(r: Refusal, status: u16, headers: &[(String, String)]) -> String {
    let flagged =
        header(headers, "cf-mitigated").is_some_and(|v| v.eq_ignore_ascii_case("challenge"));
    let status = if status == 0 {
        String::new()
    } else {
        format!("HTTP {status}")
    };
    let detail = match (status.is_empty(), flagged) {
        (true, false) => String::new(),
        (true, true) => " (cf-mitigated: challenge)".to_string(),
        (false, false) => format!(" ({status})"),
        (false, true) => format!(" ({status}, cf-mitigated: challenge)"),
    };
    format!("{}{detail}", r.noun())
}

/// The host part of a URL, for messages (`www.idx.co.id`); the URL itself
/// when it has none.
pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let authority = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    if authority.is_empty() {
        url
    } else {
        authority
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = include_str!("../../tests/fixtures/idx/cloudflare_blocked.html");
    const CHALLENGE: &str =
        include_str!("../../tests/fixtures/idx/cloudflare_challenge_synthetic.html");

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_challenge_header_wins_whatever_the_status() {
        let flagged = h(&[("CF-Mitigated", "challenge"), ("server", "cloudflare")]);
        assert_eq!(
            refusal(403, &flagged, CHALLENGE.as_bytes()),
            Some(Refusal::Challenge)
        );
        assert_eq!(refusal(200, &flagged, b"{}"), Some(Refusal::Challenge));
        assert_eq!(
            describe(Refusal::Challenge, 403, &flagged),
            "a Cloudflare challenge (HTTP 403, cf-mitigated: challenge)"
        );
    }

    #[test]
    fn block_and_challenge_pages_without_headers() {
        // The real block page also loads /cdn-cgi/challenge-platform/: it
        // must still read as a block, not a challenge.
        assert!(BLOCK.contains("challenge-platform"));
        assert_eq!(refusal(403, &[], BLOCK.as_bytes()), Some(Refusal::Block));
        assert_eq!(
            refusal(403, &[], CHALLENGE.as_bytes()),
            Some(Refusal::Challenge)
        );
        assert_eq!(
            refusal(503, &[], CHALLENGE.as_bytes()),
            Some(Refusal::Challenge)
        );
        assert_eq!(
            describe(Refusal::Block, 403, &[]),
            "a Cloudflare block page (HTTP 403)"
        );
        assert_eq!(describe(Refusal::Block, 0, &[]), "a Cloudflare block page");
    }

    #[test]
    fn a_bare_cloudflare_403_is_a_block() {
        assert_eq!(
            refusal(403, &h(&[("Server", "cloudflare")]), b"forbidden"),
            Some(Refusal::Block)
        );
        // Without any Cloudflare evidence a 403 is just a 403.
        assert_eq!(refusal(403, &[], b""), None);
        assert_eq!(refusal(403, &h(&[("server", "nginx")]), b"forbidden"), None);
    }

    #[test]
    fn outages_and_files_are_not_refusals() {
        let nginx = "<html><head><title>503 Service Temporarily Unavailable</title></head><body><center><h1>503 Service Temporarily Unavailable</h1></center><hr><center>nginx</center></body></html>";
        assert_eq!(
            refusal(503, &h(&[("server", "nginx")]), nginx.as_bytes()),
            None
        );
        // A Cloudflare-proxied origin outage is not bot protection either.
        assert_eq!(
            refusal(503, &h(&[("server", "cloudflare")]), b"origin down"),
            None
        );
        assert_eq!(refusal(200, &[], b"PK\x03\x04\x14\x00"), None);
        assert_eq!(refusal(404, &[], b"not found"), None);
        // An ordinary page loading Cloudflare's scripts is a page.
        let page = b"<html><title>Data Kepemilikan Saham</title><script src=\"/cdn-cgi/challenge-platform/scripts/jsd/main.js\"></script>";
        assert_eq!(
            refusal(200, &[("server".into(), "cloudflare".into())], page),
            None
        );
        // A 2xx carrying the challenge page is still a challenge.
        assert_eq!(
            refusal(200, &[], b"<html><title>Just a moment...</title>"),
            Some(Refusal::Challenge)
        );
    }

    #[test]
    fn hosts_for_messages() {
        assert_eq!(
            host("https://www.idx.co.id/id/perusahaan-tercatat/x/"),
            "www.idx.co.id"
        );
        assert_eq!(host("http://127.0.0.1:18765/challenge"), "127.0.0.1:18765");
        assert_eq!(host("https://u:p@ksei.co.id?x=1"), "ksei.co.id");
        assert_eq!(host("no-scheme.example/path"), "no-scheme.example");
    }
}
