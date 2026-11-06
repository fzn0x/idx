//! The GitHub mirror of IDX's monthly register.
//!
//! `.github/workflows/ownership.yml` fetches the IDX "Data Kepemilikan Saham"
//! page through a browser-impersonating client three times a day and
//! publishes every "Pemegang Saham di Atas 1%" workbook, byte for byte, to
//! the `ownership-data` prerelease of the repository, together with
//! `index.json` (as-of date, original IDX URL, SHA-256, row counts) and JSON
//! parts the web terminal loads into D1.
//!
//! The CLI uses it when idx.co.id refuses the request (Cloudflare) and no
//! `curl-impersonate` binary is installed. Files are checked against the
//! SHA-256 in the index before they are imported. `IDX_OWNERSHIP_MIRROR`
//! points at another mirror; `off` disables the fallback.

use std::time::Duration;

use chrono::NaiveDate;
use serde::Deserialize;

use crate::error::{IdxError, Result};
use crate::model::DiscoveredFile;

pub const DEFAULT_MIRROR: &str = "https://github.com/fzn0x/idx/releases/download/ownership-data";
pub const INDEX_SCHEMA: &str = "idx.ownership.mirror.v1";
pub const ORIGIN: &str = "github-mirror";

#[derive(Debug, Clone, Deserialize)]
pub struct MirrorIndex {
    pub schema: String,
    #[serde(default)]
    pub generated_at: Option<String>,
    pub releases: Vec<MirrorRelease>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MirrorRelease {
    pub as_of: NaiveDate,
    #[serde(default)]
    pub title: Option<String>,
    pub format: String,
    /// The file's address on idx.co.id.
    #[serde(default)]
    pub source_url: Option<String>,
    /// Asset name on the mirror release.
    pub file: String,
    pub sha256: String,
    #[serde(default)]
    pub rows: Option<u64>,
}

/// Mirror base URL, or `None` when disabled with `IDX_OWNERSHIP_MIRROR=off`.
pub fn base() -> Option<String> {
    match std::env::var("IDX_OWNERSHIP_MIRROR") {
        Ok(v) if v.trim().is_empty() || v.eq_ignore_ascii_case("off") => None,
        Ok(v) => Some(v.trim().trim_end_matches('/').to_string()),
        Err(_) => Some(DEFAULT_MIRROR.to_string()),
    }
}

async fn get(url: &str, timeout: Duration) -> Result<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        let path = if cfg!(windows) {
            path.trim_start_matches('/')
        } else {
            path
        };
        return std::fs::read(path)
            .map_err(|e| IdxError::Usage(format!("cannot read {path}: {e}")));
    }
    let _wait = crate::progress::request(url);
    let client = reqwest::Client::builder()
        .user_agent(concat!(
            "idx/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/fzn0x/idx)"
        ))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    let resp = client.get(url).send().await?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Err(IdxError::NotFound(format!("{url}: not on the mirror")));
    }
    if !status.is_success() {
        return Err(IdxError::Network(format!("{url}: HTTP {status}")));
    }
    Ok(resp.bytes().await?.to_vec())
}

pub fn parse_index(bytes: &[u8]) -> Result<MirrorIndex> {
    let index: MirrorIndex = serde_json::from_slice(bytes)
        .map_err(|e| IdxError::provider("mirror", format!("invalid index.json: {e}")))?;
    if index.schema != INDEX_SCHEMA {
        return Err(IdxError::provider(
            "mirror",
            format!("unsupported mirror schema {:?}; upgrade idx", index.schema),
        ));
    }
    Ok(index)
}

pub async fn index(base: &str, timeout: Duration) -> Result<MirrorIndex> {
    parse_index(&get(&format!("{base}/index.json"), timeout).await?)
}

/// The mirrored releases as discovery results, newest first.
pub fn to_discovered(base: &str, index: &MirrorIndex) -> Vec<DiscoveredFile> {
    let mut out: Vec<DiscoveredFile> = index
        .releases
        .iter()
        .map(|r| DiscoveredFile {
            as_of: r.as_of,
            title: r
                .title
                .clone()
                .unwrap_or_else(|| format!("Pemegang Saham di Atas 1% per {}", r.as_of)),
            url: format!("{base}/{}", r.file),
            format: r.format.clone(),
            origin: ORIGIN.to_string(),
            sha256: Some(r.sha256.clone()),
            source_url: r.source_url.clone(),
            // The mirror holds the above-1 % register only.
            family: "above1".to_string(),
            importable: true,
        })
        .collect();
    out.sort_by_key(|f| std::cmp::Reverse(f.as_of));
    out
}

/// Downloads a mirrored file and checks it against the index's SHA-256.
pub async fn download(url: &str, sha256: Option<&str>, timeout: Duration) -> Result<Vec<u8>> {
    let bytes = get(url, timeout).await?;
    if let Some(expected) = sha256 {
        let actual = super::sha256_hex(&bytes);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(IdxError::provider(
                "mirror",
                format!("{url}: SHA-256 {actual} does not match the index ({expected})"),
            ));
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = r#"{
      "schema": "idx.ownership.mirror.v1",
      "generated_at": "2026-10-17T09:00:00Z",
      "releases": [
        {"as_of": "2026-08-31", "format": "xlsx", "file": "ksei-above1-2026-08-31.xlsx",
         "sha256": "abc", "source_url": "https://www.idx.co.id/Media/x/peng.xlsx", "rows": 7321},
        {"as_of": "2026-09-30", "title": "Pemegang Saham di Atas 1% per 30 September 2026",
         "format": "xlsx", "file": "ksei-above1-2026-09-30.xlsx", "sha256": "def"}
      ]
    }"#;

    #[test]
    fn index_maps_to_discovery_newest_first() {
        let ix = parse_index(INDEX.as_bytes()).unwrap();
        let files = to_discovered("https://m.example/r", &ix);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].as_of.to_string(), "2026-09-30");
        assert_eq!(
            files[0].url,
            "https://m.example/r/ksei-above1-2026-09-30.xlsx"
        );
        assert_eq!(files[0].origin, ORIGIN);
        assert_eq!(files[1].sha256.as_deref(), Some("abc"));
        assert_eq!(
            files[1].source_url.as_deref(),
            Some("https://www.idx.co.id/Media/x/peng.xlsx")
        );
        assert!(files[1].title.contains("2026-08-31"));
        assert!(parse_index(br#"{"schema":"idx.ownership.mirror.v9","releases":[]}"#).is_err());
    }

    #[tokio::test]
    async fn download_verifies_sha256() {
        let dir = std::env::temp_dir().join(format!("idx-mirror-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.bin");
        std::fs::write(&f, b"hello").unwrap();
        let url = format!("file://{}", f.display().to_string().replace('\\', "/"));
        let good = super::super::sha256_hex(b"hello");
        assert_eq!(
            download(&url, Some(&good), Duration::from_secs(5))
                .await
                .unwrap(),
            b"hello"
        );
        assert!(download(&url, Some("00"), Duration::from_secs(5))
            .await
            .is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
