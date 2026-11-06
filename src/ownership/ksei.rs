//! ksei.co.id as a source: the "Kepemilikan Efek" page that lists KSEI's
//! monthly composition archives, and the archives themselves.
//!
//! - Listing: `https://www.ksei.co.id/id/publikasi/data-dan-statistik/kepemilikan-efek`
//!   (`?page=N` for older months), a server-rendered Nuxt page, eight months
//!   per page, newest first. Each month appears twice: as a rendered link
//!   `<a href="https://www.ksei.co.id/storage/Download/BalanceposEfek20261030.zip">`
//!   under the title "Kepemilikan Efek 30 Oktober 2026", and in
//!   `window.__NUXT__` with `/`-escaped slashes. The page states
//!   `last_page:N`. Deep pages can take a minute or time out (2026-11-06), so
//!   only page 1 is required; a later page that fails ends the walk with a
//!   warning. The page's own JSON API (webcms.ksei.co.id) does not resolve
//!   publicly.
//! - Files: `https://www.ksei.co.id/storage/Download/BalanceposEfek<YYYYMMDD>.zip`
//!   (about 125 KB, S3 behind nginx, no bot protection, any user agent). The
//!   date is the month's last trading day, so it is read from the page, never
//!   derived from the calendar.
//!
//! The listing goes through the [`Fetcher`] (cached 12 h, stale copies served
//! offline); the archive is binary and fetched directly, at most
//! [`MAX_DOWNLOAD`] bytes. See [`super::composition`] for the format.

use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;

use crate::error::{IdxError, Result};
use crate::http::cloudflare;
use crate::http::{Fetched, Fetcher};
use crate::model::DiscoveredFile;
use crate::providers::Sourced;

use super::discover::{basename, unescape_js};

pub const PAGE_URL: &str =
    "https://www.ksei.co.id/id/publikasi/data-dan-statistik/kepemilikan-efek";
/// `origin` of the listed files.
pub const ORIGIN: &str = "ksei.co.id";
/// `provider` of envelopes built from ksei.co.id.
pub const PROVIDER: &str = "ksei";
/// Months on one listing page.
pub const PER_PAGE: usize = 8;
/// Listing pages walked at most (80 months).
pub const MAX_PAGES: u32 = 10;
/// The largest archive downloaded (KSEI's are about 0.13 MB).
pub const MAX_DOWNLOAD: u64 = 32 * 1024 * 1024;
const TTL_LISTING: Duration = Duration::from_secs(12 * 3600);
const NAME: &str = "BalanceposEfek";

pub fn page_url(n: u32) -> String {
    if n <= 1 {
        PAGE_URL.to_string()
    } else {
        format!("{PAGE_URL}?page={n}")
    }
}

/// `Kepemilikan Efek 30 Oktober 2026` nearest before `pos`, if close.
fn title_before(html: &str, pos: usize) -> Option<String> {
    let from = pos.saturating_sub(1200);
    let from = (from..pos).find(|i| html.is_char_boundary(*i))?;
    let window = &html[from..pos];
    let at = window.rfind("Kepemilikan Efek ")?;
    let rest = &window[at..];
    let end = rest.find(['<', '"']).unwrap_or(rest.len());
    let t = rest[..end].trim();
    (t.len() > "Kepemilikan Efek ".len()).then(|| t.to_string())
}

/// Every `BalanceposEfek<YYYYMMDD>.zip` on one listing page, newest first,
/// one per date (the rendered link and the Nuxt copy both match).
pub fn parse_listing(html: &str) -> Vec<DiscoveredFile> {
    let mut out: Vec<DiscoveredFile> = Vec::new();
    let mut from = 0;
    while let Some(rel) = html[from..].find(NAME) {
        let at = from + rel;
        from = at + NAME.len();
        let digits = &html[from..];
        if digits.len() < 12
            || !digits.as_bytes()[..8].iter().all(u8::is_ascii_digit)
            || !digits
                .get(8..12)
                .is_some_and(|ext| ext.eq_ignore_ascii_case(".zip"))
        {
            continue;
        }
        let Ok(as_of) = NaiveDate::parse_from_str(&digits[..8], "%Y%m%d") else {
            continue;
        };
        // The URL starts after the nearest quote before the name.
        let Some(start) = html[..at].rfind(['"', '\'']) else {
            continue;
        };
        if at - start > 400 {
            continue;
        }
        let url = unescape_js(&html[start + 1..from + 12]);
        if !url.contains("://") || out.iter().any(|f| f.as_of == as_of) {
            continue;
        }
        let title =
            title_before(html, start).unwrap_or_else(|| format!("Kepemilikan Efek {as_of}"));
        out.push(DiscoveredFile {
            as_of,
            title,
            url,
            format: "zip".into(),
            origin: ORIGIN.into(),
            sha256: None,
            source_url: None,
            family: "composition".into(),
            importable: true,
        });
    }
    out.sort_by_key(|f| std::cmp::Reverse(f.as_of));
    out
}

/// `last_page:5` from the page's Nuxt state.
pub fn last_page(html: &str) -> Option<u32> {
    let at = html.find("last_page:")? + "last_page:".len();
    let digits: String = html[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Maps a non-2xx answer (or a Cloudflare page) to the right error.
fn refused(url: &str, status: u16, headers: &[(String, String)], body: &[u8]) -> IdxError {
    let host = cloudflare::host(url);
    if let Some(r) = cloudflare::refusal(status, headers, body) {
        return IdxError::blocked(
            PROVIDER,
            r,
            format!(
                "{host} answered with {}. It usually passes later or from another network",
                cloudflare::describe(r, status, headers)
            ),
        );
    }
    if status == 404 {
        return IdxError::NotFound(format!("{url}: HTTP 404"));
    }
    IdxError::provider(PROVIDER, format!("{host} answered HTTP {status} for {url}"))
}

fn check(url: &str, f: &Fetched) -> Result<()> {
    if f.is_success() && cloudflare::refusal(f.status, &f.headers, f.body.as_bytes()).is_none() {
        return Ok(());
    }
    Err(refused(url, f.status, &f.headers, f.body.as_bytes()))
}

/// Walks the listing pages, newest first, until `enough` says so, a page adds
/// nothing, the last page or [`MAX_PAGES`]. Page 1 must answer; a later page
/// that fails ends the walk with a warning.
pub async fn list_until(
    fetcher: &Arc<Fetcher>,
    enough: impl Fn(&[DiscoveredFile]) -> bool,
) -> Result<Sourced<Vec<DiscoveredFile>>> {
    let headers = vec![(
        "Accept-Language".to_string(),
        "id-ID,id;q=0.9,en;q=0.8".to_string(),
    )];
    let url = page_url(1);
    let first = fetcher.get(&url, &headers, TTL_LISTING, None).await?;
    check(&url, &first)?;
    let mut files = parse_listing(&first.body);
    if files.is_empty() {
        return Err(IdxError::provider(
            PROVIDER,
            "the Kepemilikan Efek page lists no BalanceposEfek archive; the page layout may have changed",
        ));
    }
    let last = last_page(&first.body).unwrap_or(1);
    let mut s = Sourced::fresh(Vec::new(), PROVIDER);
    s.cached = first.cached;
    s.stale = first.stale;
    s.fetched_at = first.fetched_at;
    let mut n = 1;
    while !enough(&files) && n < last {
        if n >= MAX_PAGES {
            s.warnings.push(format!(
                "ksei.co.id lists {last} pages; idx reads the first {MAX_PAGES} ({} months)",
                MAX_PAGES as usize * PER_PAGE
            ));
            break;
        }
        n += 1;
        let url = page_url(n);
        let page = match fetcher.get(&url, &headers, TTL_LISTING, None).await {
            Ok(f) => check(&url, &f).map(|_| f),
            Err(e) => Err(e),
        };
        match page {
            Ok(f) => {
                let before = files.len();
                for file in parse_listing(&f.body) {
                    if !files.iter().any(|x| x.as_of == file.as_of) {
                        files.push(file);
                    }
                }
                if files.len() == before {
                    break;
                }
            }
            Err(e) => {
                s.warnings.push(format!(
                    "ksei.co.id listing page {n} failed ({e}); older months are not listed"
                ));
                break;
            }
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.as_of));
    s.data = files;
    Ok(s)
}

/// The newest `limit` archives (0: every page, up to [`MAX_PAGES`]).
pub async fn list(fetcher: &Arc<Fetcher>, limit: usize) -> Result<Sourced<Vec<DiscoveredFile>>> {
    let mut s = list_until(fetcher, |files| limit > 0 && files.len() >= limit).await?;
    if limit > 0 {
        s.data.truncate(limit);
    }
    Ok(s)
}

/// Whether `url` is served by ksei.co.id or names a KSEI archive, so
/// `import --url` fetches it plainly (no idx.co.id Referer, no
/// curl-impersonate).
pub fn is_ksei_url(url: &str) -> bool {
    let host = cloudflare::host(url).to_ascii_lowercase();
    let host = host.split(':').next().unwrap_or_default();
    host == "ksei.co.id"
        || host.ends_with(".ksei.co.id")
        || basename(url)
            .to_ascii_lowercase()
            .starts_with("balanceposefek")
}

/// Downloads an archive (or a saved TXT): `file://` reads a local file.
pub async fn download(url: &str, timeout: Duration) -> Result<Vec<u8>> {
    download_capped(url, timeout, MAX_DOWNLOAD).await
}

/// `32 MiB`, or `1024 bytes` below a mebibyte.
pub(super) fn size_limit(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{} MiB", n / (1024 * 1024))
    } else {
        format!("{n} bytes")
    }
}

fn too_big(url: &str, n: u64, cap: u64) -> IdxError {
    IdxError::provider(
        PROVIDER,
        format!(
            "{url} is {n} bytes or more, over the {} limit for a composition archive",
            size_limit(cap)
        ),
    )
}

fn check_body(url: &str, bytes: Vec<u8>) -> Result<Vec<u8>> {
    let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    if bytes.starts_with(b"PK\x03\x04") || body.starts_with(b"Date|Code|Type|") {
        return Ok(bytes);
    }
    let head: String = String::from_utf8_lossy(&bytes[..bytes.len().min(60)])
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    Err(IdxError::provider(
        PROVIDER,
        format!("{url} is not a KSEI ZIP or TXT archive (it starts with {head:?})"),
    ))
}

async fn download_capped(url: &str, timeout: Duration, cap: u64) -> Result<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        let path = if cfg!(windows) {
            path.trim_start_matches('/')
        } else {
            path
        };
        let len = std::fs::metadata(path)
            .map_err(|e| IdxError::Usage(format!("cannot read {path}: {e}")))?
            .len();
        if len > cap {
            return Err(too_big(url, len, cap));
        }
        let bytes =
            std::fs::read(path).map_err(|e| IdxError::Usage(format!("cannot read {path}: {e}")))?;
        return check_body(url, bytes);
    }
    let _wait = crate::progress::request(url);
    let client = reqwest::Client::builder()
        .user_agent(concat!(
            "idx/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/fzn0x/idx)"
        ))
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let mut resp = client.get(url).send().await?;
    let status = resp.status().as_u16();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_ascii_lowercase(),
                v.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    if resp.content_length().is_some_and(|n| n > cap) {
        return Err(too_big(url, resp.content_length().unwrap_or(cap), cap));
    }
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if bytes.len() as u64 + chunk.len() as u64 > cap {
            return Err(too_big(url, cap + 1, cap));
        }
        bytes.extend_from_slice(&chunk);
    }
    if !(200..300).contains(&status) || cloudflare::refusal(status, &headers, &bytes).is_some() {
        return Err(refused(url, status, &headers, &bytes));
    }
    check_body(url, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::http::{CacheMode, FixtureBackend};

    const PAGE: &str =
        include_str!("../../tests/fixtures/ownership/ksei_kepemilikan_efek_page_excerpt.html");

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn fetcher(be: FixtureBackend) -> (Arc<Fetcher>, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(be);
        let f = Arc::new(Fetcher::new(
            be.clone(),
            Cache::new(dir.path()),
            CacheMode::Normal,
        ));
        (f, be, dir)
    }

    /// The excerpt with every date moved back by `months` (page 2, 3, ...).
    fn older_page(months: u32) -> String {
        let mut page = PAGE.to_string();
        for f in parse_listing(PAGE) {
            let old = f.as_of.format("%Y%m%d").to_string();
            let shifted = f.as_of - chrono::Months::new(months);
            page = page.replace(&old, &shifted.format("%Y%m%d").to_string());
        }
        page
    }

    #[test]
    fn the_page_lists_eight_months_newest_first() {
        let files = parse_listing(PAGE);
        let dates: Vec<NaiveDate> = files.iter().map(|f| f.as_of).collect();
        assert_eq!(dates.len(), 8);
        assert_eq!(dates[0], d(2026, 10, 30));
        assert_eq!(
            dates[5],
            d(2026, 5, 29),
            "the last trading day, not the 31st"
        );
        assert_eq!(dates[7], d(2026, 3, 31));
        let f = &files[0];
        assert_eq!(f.title, "Kepemilikan Efek 30 Oktober 2026");
        assert_eq!(
            f.url,
            "https://www.ksei.co.id/storage/Download/BalanceposEfek20261030.zip"
        );
        assert_eq!(
            (
                f.format.as_str(),
                f.origin.as_str(),
                f.family.as_str(),
                f.importable
            ),
            ("zip", "ksei.co.id", "composition", true)
        );
        assert_eq!(last_page(PAGE), Some(5));
        // The Nuxt copy alone is enough.
        let nuxt = &PAGE[PAGE.find("window.__NUXT__").unwrap()..];
        let only = parse_listing(nuxt);
        assert_eq!(only.len(), 8);
        assert_eq!(only[0].url, files[0].url);
        assert_eq!(only[0].title, "Kepemilikan Efek 30 Oktober 2026");
        assert!(parse_listing("<html>nothing</html>").is_empty());
        assert!(parse_listing("BalanceposEfek2026.zip \"BalanceposEfek99999999.zip").is_empty());
    }

    #[tokio::test]
    async fn listing_walks_pages_until_the_limit() {
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route(&page_url(2), 200, &older_page(8))
                .route(PAGE_URL, 200, PAGE),
        );
        let s = list(&f, 8).await.unwrap();
        assert_eq!((s.data.len(), be.call_count()), (8, 1));
        assert_eq!(s.provider, "ksei");
        let s = list(&f, 10).await.unwrap();
        assert_eq!(s.data.len(), 10);
        assert_eq!(s.data[8].as_of, d(2026, 2, 28), "{:?}", s.data[8]);
        assert_eq!(be.calls_matching("?page=2"), 1);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    }

    #[tokio::test]
    async fn a_failing_later_page_keeps_page_one() {
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route(&page_url(2), 504, "Gateway Time-out")
                .route(PAGE_URL, 200, PAGE),
        );
        let s = list(&f, 0).await.unwrap();
        assert_eq!(s.data.len(), 8);
        assert!(s.warnings[0].contains("page 2 failed"), "{:?}", s.warnings);
        assert!(s.warnings[0].contains("HTTP 504"), "{:?}", s.warnings);
    }

    #[tokio::test]
    async fn page_one_errors() {
        let (f, _be, _d) = fetcher(FixtureBackend::new().route(PAGE_URL, 200, "<html></html>"));
        let err = list(&f, 8).await.unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("provider", 3));
        assert!(err.to_string().contains("layout"), "{err}");
        let (f, _be, _d) = fetcher(FixtureBackend::new().route(PAGE_URL, 404, "gone"));
        assert_eq!(list(&f, 8).await.unwrap_err().exit_code(), 4);
        let (f, _be, _d) = fetcher(FixtureBackend::new().route(PAGE_URL, 503, "maintenance"));
        let err = list(&f, 8).await.unwrap_err();
        assert_eq!(err.code(), "provider");
        assert!(!err.to_string().contains("Cloudflare"), "{err}");
        let (f, _be, _d) = fetcher(FixtureBackend::new().route_with_headers(
            PAGE_URL,
            403,
            "<title>Just a moment...</title>",
            &[("cf-mitigated", "challenge")],
        ));
        assert_eq!(list(&f, 8).await.unwrap_err().code(), "blocked");
    }

    #[test]
    fn ksei_urls_are_fetched_plainly() {
        assert!(is_ksei_url(
            "https://www.ksei.co.id/storage/Download/BalanceposEfek20261030.zip"
        ));
        assert!(is_ksei_url("https://ksei.co.id:443/x"));
        assert!(is_ksei_url(
            "https://mirror.example/BalanceposEfek20261030.zip"
        ));
        assert!(!is_ksei_url(
            "https://www.idx.co.id/Media/x/peng-2026-08-00017-satu-persen.xlsx"
        ));
        assert!(!is_ksei_url("https://notksei.co.id.example/x.zip"));
    }

    #[tokio::test]
    async fn downloads_are_capped_and_checked() {
        let dir = tempfile::tempdir().unwrap();
        let url = |name: &str| {
            format!(
                "file://{}",
                dir.path()
                    .join(name)
                    .display()
                    .to_string()
                    .replace('\\', "/")
            )
        };
        let zip = include_bytes!(
            "../../tests/fixtures/ownership/ksei_balancepos_20261030_excerpt_constructed.zip"
        );
        std::fs::write(dir.path().join("BalanceposEfek20261030.zip"), zip).unwrap();
        std::fs::write(dir.path().join("page.html"), b"<html>error</html>").unwrap();
        let t = Duration::from_secs(5);
        assert_eq!(
            download(&url("BalanceposEfek20261030.zip"), t)
                .await
                .unwrap(),
            zip
        );
        let err = download_capped(&url("BalanceposEfek20261030.zip"), t, 100)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("over the 100 bytes limit"),
            "{err}"
        );
        assert_eq!(size_limit(MAX_DOWNLOAD), "32 MiB");
        let err = download(&url("page.html"), t).await.unwrap_err();
        assert!(err.to_string().contains("not a KSEI ZIP"), "{err}");
        assert_eq!(
            download(&url("missing.zip"), t)
                .await
                .unwrap_err()
                .exit_code(),
            2
        );
    }
}
