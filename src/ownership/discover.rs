//! Finding and downloading the monthly files on idx.co.id.
//!
//! The "Data Kepemilikan Saham" page is a Nuxt app whose payload embeds
//! `{Description:"Pemegang Saham di Atas 1% per 31 Agustus 2026",
//! Prospectus:"https://www.idx.co.id/Media/...xlsx", ...}`
//! entries. We parse those pairs out of the HTML without a JS engine.
//!
//! idx.co.id sits behind Cloudflare bot management: some networks get a
//! challenge or a block page instead of the file ([`crate::http::cloudflare`]).
//! `fetch_browser` tries a browser-shaped request first, then, only when
//! Cloudflare refused it, shells out to a `curl-impersonate` binary when one
//! is installed. A refusal that remains is a `blocked` error (exit 3); any
//! other failure keeps its HTTP status. When idx.co.id fails, `discover`
//! falls back to the GitHub mirror that CI keeps in sync (see `mirror.rs`),
//! so `idx ownership import --latest` works without any extra tooling.
//! `file://` URLs read a saved page or file.

use std::path::Path;
use std::time::Duration;

use chrono::NaiveDate;

use crate::error::{IdxError, Result};
use crate::http::cloudflare::{self, Refusal};
use crate::model::DiscoveredFile;

use super::ingest::parse_indonesian_date;

pub const PAGE_URL: &str = "https://www.idx.co.id/id/perusahaan-tercatat/data-kepemilikan-saham/";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
/// Binaries tried in order; `IDX_CURL_IMPERSONATE` overrides the list.
const IMPERSONATE_BINARIES: &[&str] = &[
    "curl_chrome131",
    "curl_chrome124",
    "curl_chrome116",
    "curl-impersonate-chrome",
    "curl-impersonate",
];

/// What to do when Cloudflare refuses idx.co.id.
pub const MANUAL_HINT: &str = "Ways out: retry later or from another network; let `idx ownership import --latest` use the GitHub mirror (the default, or IDX_OWNERSHIP_SOURCE=mirror); install curl-impersonate (https://github.com/lexiforest/curl-impersonate) and put curl_chrome131 on PATH; or open https://www.idx.co.id/id/perusahaan-tercatat/data-kepemilikan-saham/ in a browser, download the workbook and run `idx ownership import --file <path>`";

pub(super) fn unescape_js(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Ok(code) = u32::from_str_radix(&hex, 16) {
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                    }
                }
            }
            Some('/') => out.push('/'),
            Some('"') => out.push('"'),
            Some('n') => out.push('\n'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Reads a JS string literal starting at `s` (which begins with a quote).
fn js_string(s: &str) -> Option<(String, usize)> {
    let quote = s.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut escaped = false;
    for (i, c) in s.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
        } else if c == quote {
            return Some((unescape_js(&s[1..i]), i + 1));
        }
    }
    None
}

/// `per 31 Agustus 2026` -> date.
pub fn as_of_from_title(title: &str) -> Option<NaiveDate> {
    let idx = title.to_lowercase().rfind(" per ")?;
    let tail = title[idx + 5..].trim().trim_end_matches('.');
    parse_indonesian_date(tail).or_else(|| super::ingest::parse_text_date(tail))
}

/// Every `Description`/`Prospectus` pair in the page, newest first. Only the
/// above-1 % register is kept; the above-5 % and investor-type workbooks live
/// on the same page but have other layouts.
pub fn parse_page(html: &str) -> Vec<DiscoveredFile> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("Description:") {
        rest = &rest[pos + "Description:".len()..];
        let Some((title, used)) = js_string(rest) else {
            continue;
        };
        let after = &rest[used..];
        // The URL follows within the same object literal.
        let Some(ppos) = after.find("Prospectus:") else {
            continue;
        };
        if ppos > 400 {
            continue;
        }
        let Some((url, _)) = js_string(&after[ppos + "Prospectus:".len()..]) else {
            continue;
        };
        let lower = title.to_lowercase();
        let is_above1 = lower.contains("di atas 1%")
            || lower.contains("di atas 1 %")
            || lower.contains("diatas 1%");
        if !is_above1 {
            continue;
        }
        let Some(as_of) = as_of_from_title(&title) else {
            continue;
        };
        let format = if url.to_lowercase().ends_with(".pdf") {
            "pdf"
        } else {
            "xlsx"
        };
        out.push(DiscoveredFile {
            as_of,
            title: title.trim().to_string(),
            url,
            format: format.to_string(),
            origin: "idx.co.id".to_string(),
            sha256: None,
            source_url: None,
            family: "above1".to_string(),
            importable: true,
        });
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.as_of));
    out.dedup_by(|a, b| a.as_of == b.as_of);
    out
}

type Response = (u16, Vec<(String, String)>, Vec<u8>);

async fn fetch_plain(url: &str, timeout: Duration) -> Result<Response> {
    let _wait = crate::progress::request(url);
    let client = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(timeout)
        .gzip(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let resp = client
        .get(url)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "id-ID,id;q=0.9,en-US;q=0.8,en;q=0.7")
        .header("Referer", PAGE_URL)
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "same-origin")
        .header("Upgrade-Insecure-Requests", "1")
        .send()
        .await?;
    let status = resp.status().as_u16();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_ascii_lowercase(),
                v.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let bytes = resp.bytes().await?.to_vec();
    Ok((status, headers, bytes))
}

fn impersonate_candidates() -> Vec<String> {
    if let Ok(custom) = std::env::var("IDX_CURL_IMPERSONATE") {
        if !custom.trim().is_empty() {
            return vec![custom];
        }
    }
    IMPERSONATE_BINARIES.iter().map(|s| s.to_string()).collect()
}

fn fetch_impersonated(url: &str, timeout: Duration) -> Option<Result<(u16, Vec<u8>)>> {
    for bin in impersonate_candidates() {
        let tmp = std::env::temp_dir().join(format!("idx-dl-{}.bin", std::process::id()));
        let out = std::process::Command::new(&bin)
            .args([
                "-sS",
                "-L",
                "--max-time",
                &timeout.as_secs().to_string(),
                "-H",
                &format!("Referer: {PAGE_URL}"),
                "-o",
            ])
            .arg(&tmp)
            .args(["-w", "%{http_code}"])
            .arg(url)
            .output();
        match out {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Some(Err(IdxError::Network(format!("{bin}: {e}")))),
            Ok(o) => {
                let code: u16 = String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse()
                    .unwrap_or(0);
                let body = std::fs::read(&tmp).unwrap_or_default();
                let _ = std::fs::remove_file(&tmp);
                tracing::info!(binary = %bin, status = code, "fetched through curl-impersonate");
                if !o.status.success() && code == 0 {
                    return Some(Err(IdxError::Network(format!(
                        "{bin} failed: {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    ))));
                }
                return Some(Ok((code, body)));
            }
        }
    }
    None
}

/// The provider named in a `blocked` error for `host`.
fn provider_for(host: &str) -> &'static str {
    let h = host
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if h == "idx.co.id" || h.ends_with(".idx.co.id") {
        "idx"
    } else if h == "ksei.co.id" || h.ends_with(".ksei.co.id") {
        super::ksei::PROVIDER
    } else {
        "ownership"
    }
}

/// The `blocked` error for a refusal of `url`: `what` is the described
/// refusal, `tail` what was tried.
pub fn blocked_error(url: &str, refusal: Refusal, what: &str, tail: &str) -> IdxError {
    let host = cloudflare::host(url);
    IdxError::blocked(
        provider_for(host),
        refusal,
        format!(
            "{host} answered with {what}{tail}. It usually passes later or from an ordinary home or office connection (data centres, VPNs and CI runners are often refused). {MANUAL_HINT}"
        ),
    )
}

/// A non-2xx answer that is not a Cloudflare refusal.
fn http_error(url: &str, status: u16, tail: &str) -> IdxError {
    if status == 404 {
        return IdxError::NotFound(format!("{url}: HTTP 404{tail}"));
    }
    IdxError::Network(format!(
        "{} answered HTTP {status}{tail}",
        cloudflare::host(url)
    ))
}

/// Downloads `url` as a browser would. See the module docs for the strategy.
pub async fn fetch_browser(url: &str, timeout: Duration) -> Result<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        let path = if cfg!(windows) {
            path.trim_start_matches('/')
        } else {
            path
        };
        return std::fs::read(path)
            .map_err(|e| IdxError::Usage(format!("cannot read {path}: {e}")));
    }
    let (status, headers, body) = fetch_plain(url, timeout).await?;
    let Some(first) = cloudflare::refusal(status, &headers, &body) else {
        if (200..300).contains(&status) {
            return Ok(body);
        }
        // Not Cloudflare: curl-impersonate would get the same answer.
        return Err(http_error(url, status, ""));
    };
    tracing::info!(status, refusal = ?first, "plain request was refused, trying curl-impersonate");
    let _wait = crate::progress::request(url);
    match fetch_impersonated(url, timeout) {
        Some(Ok((code, body))) => {
            // curl reports no headers here; after a refusal a bare 403 is one too.
            let again = cloudflare::refusal(code, &[], &body).or((code == 403).then_some(first));
            match again {
                None if (200..300).contains(&code) => Ok(body),
                None => Err(http_error(url, code, " through curl-impersonate")),
                Some(r) => Err(blocked_error(
                    url,
                    r,
                    &cloudflare::describe(r, code, &[]),
                    " even through curl-impersonate",
                )),
            }
        }
        Some(Err(e)) => Err(e),
        None => Err(blocked_error(
            url,
            first,
            &cloudflare::describe(first, status, &headers),
            " and no curl-impersonate binary was found",
        )),
    }
}

fn source_setting() -> String {
    std::env::var("IDX_OWNERSHIP_SOURCE")
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The above-1 % files on the idx.co.id page (or `IDX_OWNERSHIP_PAGE_URL`).
async fn idx_page(timeout: Duration) -> Result<Vec<DiscoveredFile>> {
    let page_url = std::env::var("IDX_OWNERSHIP_PAGE_URL").unwrap_or_else(|_| PAGE_URL.to_string());
    let bytes = fetch_browser(&page_url, timeout).await?;
    Ok(parse_page(&String::from_utf8_lossy(&bytes)))
}

/// Why the mirror is used instead, for the note.
fn fallback_reason(err: &IdxError) -> String {
    match err {
        IdxError::Blocked { refusal, .. } => {
            format!("idx.co.id answered with {}", refusal.noun())
        }
        e => format!("idx.co.id unavailable ({})", short(&e.to_string())),
    }
}

/// `err` with the mirror's failure appended, keeping a `blocked` error
/// `blocked` so scripts still see why idx.co.id failed.
fn with_mirror_failure(err: IdxError, mirror: &IdxError) -> IdxError {
    match err {
        IdxError::Blocked {
            provider,
            refusal,
            message,
        } => IdxError::Blocked {
            provider,
            refusal,
            message: format!("{message}. The GitHub mirror also failed: {mirror}"),
        },
        e => IdxError::Network(format!("{e}. The GitHub mirror also failed: {mirror}")),
    }
}

/// The above-1 % files from the parsed page, or from the mirror when the
/// page failed or listed none.
async fn above1_from(
    page: Result<Vec<DiscoveredFile>>,
    timeout: Duration,
) -> Result<(Vec<DiscoveredFile>, Vec<String>)> {
    let direct = page.and_then(|files| {
        if files.is_empty() {
            Err(IdxError::provider(
                "idx",
                "the Data Kepemilikan Saham page had no \"Pemegang Saham di Atas 1%\" entries; the page layout may have changed",
            ))
        } else {
            Ok(files)
        }
    });
    let err = match direct {
        Ok(files) => return Ok((files, vec![])),
        Err(e) => e,
    };
    // A page the caller pointed at explicitly is not second-guessed.
    let explicit_page = std::env::var("IDX_OWNERSHIP_PAGE_URL").is_ok();
    let Some(base) = super::mirror::base().filter(|_| !explicit_page && source_setting() != "idx")
    else {
        return Err(err);
    };
    tracing::info!(error = %err, mirror = %base, "idx.co.id unavailable, using the GitHub mirror");
    match super::mirror::index(&base, timeout).await {
        Ok(index) => {
            let files = super::mirror::to_discovered(&base, &index);
            if files.is_empty() {
                return Err(err);
            }
            Ok((
                files,
                vec![format!(
                    "{}; using the GitHub mirror {base}, which holds the same IDX files (SHA-256 checked on download)",
                    fallback_reason(&err)
                )],
            ))
        }
        Err(me) => Err(with_mirror_failure(err, &me)),
    }
}

async fn mirror_only(timeout: Duration) -> Result<(Vec<DiscoveredFile>, Vec<String>)> {
    let base = super::mirror::base().ok_or_else(|| {
        IdxError::Config("IDX_OWNERSHIP_SOURCE=mirror but IDX_OWNERSHIP_MIRROR=off".into())
    })?;
    let index = super::mirror::index(&base, timeout).await?;
    Ok((
        super::mirror::to_discovered(&base, &index),
        vec![format!(
            "IDX_OWNERSHIP_SOURCE=mirror: listing from {base} (same IDX files, SHA-256 checked on download)"
        )],
    ))
}

/// Fetches the listing page and returns the above-1 % files, newest first.
/// Returns the files plus notes (for example that the mirror was used).
///
/// `IDX_OWNERSHIP_SOURCE`: `auto` (default: idx.co.id, then the mirror),
/// `idx` (never the mirror) or `mirror` (skip idx.co.id).
pub async fn discover(timeout: Duration) -> Result<(Vec<DiscoveredFile>, Vec<String>)> {
    if source_setting() == "mirror" {
        return mirror_only(timeout).await;
    }
    above1_from(idx_page(timeout).await, timeout).await
}

fn short(s: &str) -> String {
    let first = s.split(". ").next().unwrap_or(s);
    if first.chars().count() > 120 {
        format!("{}…", first.chars().take(120).collect::<String>())
    } else {
        first.to_string()
    }
}

/// File name component of a URL or path, for provenance.
pub fn basename(source: &str) -> String {
    Path::new(source.split('?').next().unwrap_or(source))
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| source.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../../tests/fixtures/ownership/idx_page_excerpt.html");

    #[test]
    fn page_lists_above1_files_newest_first() {
        let files = parse_page(PAGE);
        let dates: Vec<String> = files.iter().map(|f| f.as_of.to_string()).collect();
        assert_eq!(dates, ["2026-08-31", "2026-07-31", "2026-02-27"]);
        assert_eq!(
            files[0].url,
            "https://www.idx.co.id/Media/fahlw1o2/peng-2026-08-00017-satu-persen.xlsx"
        );
        assert_eq!(files[0].format, "xlsx");
        assert!(files[0]
            .title
            .starts_with("Pemegang Saham di Atas 1% per 31 Agustus"));
        assert!(
            !files.iter().any(|f| f.url.contains("lima-persen")),
            "above-5% files are filtered out"
        );
        assert!(files.iter().all(|f| f.family == "above1" && f.importable));
    }

    #[test]
    fn title_dates_and_js_strings() {
        assert_eq!(
            as_of_from_title("Pemegang Saham di Atas 1% per 31 Juli 2026"),
            Some(NaiveDate::from_ymd_opt(2026, 7, 31).unwrap())
        );
        assert_eq!(
            unescape_js("https:\\u002F\\u002Fx.y\\u002Fa.xlsx"),
            "https://x.y/a.xlsx"
        );
        assert_eq!(js_string("\"a\\\"b\" rest").unwrap().0, "a\"b");
        assert_eq!(basename("https://x/y/z.xlsx?x=1"), "z.xlsx");
    }

    #[tokio::test]
    async fn file_urls_read_saved_pages() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/ownership/idx_page_excerpt.html"
        )
        .replace('\\', "/");
        let bytes = fetch_browser(&format!("file://{path}"), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(parse_page(&String::from_utf8_lossy(&bytes)).len(), 3);
        assert!(
            fetch_browser("file:///no/such/page.html", Duration::from_secs(5))
                .await
                .is_err()
        );
    }

    #[test]
    fn the_fallback_note_says_which_refusal() {
        let e = blocked_error(
            PAGE_URL,
            Refusal::Challenge,
            "a Cloudflare challenge (HTTP 403)",
            "",
        );
        assert_eq!((e.code(), e.exit_code()), ("blocked", 3));
        assert_eq!(
            fallback_reason(&e),
            "idx.co.id answered with a Cloudflare challenge"
        );
        let e = blocked_error(
            PAGE_URL,
            Refusal::Block,
            "a Cloudflare block page (HTTP 403)",
            "",
        );
        assert_eq!(
            fallback_reason(&e),
            "idx.co.id answered with a Cloudflare block page"
        );
        assert!(
            e.to_string()
                .starts_with("idx: www.idx.co.id answered with a Cloudflare block page"),
            "{e}"
        );
        assert!(e.to_string().contains("IDX_OWNERSHIP_SOURCE=mirror"), "{e}");
        let outage = http_error("https://www.idx.co.id/x", 503, "");
        assert_eq!(outage.code(), "network");
        assert!(fallback_reason(&outage).contains("HTTP 503"));
        assert!(!outage.to_string().contains("Cloudflare"));
        assert_eq!(
            http_error("https://h.example/x.xlsx", 404, "").exit_code(),
            4
        );
        // The mirror failing too keeps the error `blocked`.
        let both = with_mirror_failure(e, &IdxError::Network("down".into()));
        assert_eq!(both.code(), "blocked");
        assert!(
            both.to_string()
                .ends_with("The GitHub mirror also failed: network: down"),
            "{both}"
        );
        assert_eq!(provider_for("127.0.0.1:18765"), "ownership");
        assert_eq!(provider_for("www.ksei.co.id"), "ksei");
    }
}
