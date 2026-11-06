//! HTTP fetching with the cache in front of it.
//!
//! Providers never talk to `reqwest` directly: they ask the [`Fetcher`] for a
//! URL with a TTL. The fetcher consults the [`Cache`], honours `--offline` and
//! `--no-cache`, and falls back to a stale entry when the network fails. The
//! [`Backend`] trait lets tests substitute recorded fixtures for the network.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::cache::Cache;
use crate::error::{IdxError, Result};

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub body: String,
    /// Lower-cased header names.
    pub headers: Vec<(String, String)>,
}

impl RawResponse {
    pub fn header_values(&self, name: &str) -> Vec<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .filter(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

#[async_trait]
pub trait Backend: Send + Sync {
    async fn get(&self, url: &str, headers: &[(String, String)]) -> Result<RawResponse>;

    /// POST with a JSON body: session endpoints (token refresh, never cached)
    /// and read-only query endpoints that take their parameters in the body
    /// (cached by [`Fetcher::post`] under an explicit key).
    async fn post(
        &self,
        url: &str,
        _headers: &[(String, String)],
        _body: &str,
    ) -> Result<RawResponse> {
        Err(IdxError::Other(format!(
            "this backend cannot POST to {url}"
        )))
    }
}

pub struct ReqwestBackend {
    client: reqwest::Client,
}

impl ReqwestBackend {
    pub fn new(timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(10))
            .gzip(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        Ok(ReqwestBackend { client })
    }
}

#[async_trait]
impl Backend for ReqwestBackend {
    async fn get(&self, url: &str, headers: &[(String, String)]) -> Result<RawResponse> {
        let mut req = self
            .client
            .get(url)
            .header("Accept", "application/json, text/plain, */*");
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        tracing::debug!(url, "GET");
        let resp = req.send().await?;
        read_response(url, resp).await
    }

    async fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &str,
    ) -> Result<RawResponse> {
        let mut req = self
            .client
            .post(url)
            .header("Accept", "application/json, text/plain, */*")
            .header("Content-Type", "application/json")
            .body(body.to_string());
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        tracing::debug!(url, "POST");
        let resp = req.send().await?;
        read_response(url, resp).await
    }
}

async fn read_response(url: &str, resp: reqwest::Response) -> Result<RawResponse> {
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
    let body = resp.text().await?;
    tracing::debug!(url, status, bytes = body.len(), "response");
    Ok(RawResponse {
        status,
        body,
        headers,
    })
}

/// In-memory backend for tests: exact-URL matches first, then substring matches.
#[derive(Default)]
pub struct FixtureBackend {
    routes: Mutex<Vec<(String, RawResponse)>>,
    pub calls: Mutex<Vec<String>>,
    /// Request headers, one entry per call, in call order.
    pub headers: Mutex<Vec<Vec<(String, String)>>>,
}

impl FixtureBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn route(self, url_part: &str, status: u16, body: &str) -> Self {
        self.routes.lock().unwrap().push((
            url_part.to_string(),
            RawResponse {
                status,
                body: body.to_string(),
                headers: vec![],
            },
        ));
        self
    }

    pub fn route_with_headers(
        self,
        url_part: &str,
        status: u16,
        body: &str,
        headers: &[(&str, &str)],
    ) -> Self {
        self.routes.lock().unwrap().push((
            url_part.to_string(),
            RawResponse {
                status,
                body: body.to_string(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
                    .collect(),
            },
        ));
        self
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// Requests whose URL contains `part`.
    pub fn calls_matching(&self, part: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.contains(part))
            .count()
    }
}

impl FixtureBackend {
    fn answer(&self, url: &str, headers: &[(String, String)]) -> Result<RawResponse> {
        self.calls.lock().unwrap().push(url.to_string());
        self.headers.lock().unwrap().push(headers.to_vec());
        let routes = self.routes.lock().unwrap();
        if let Some((_, r)) = routes.iter().find(|(u, _)| u == url) {
            return Ok(r.clone());
        }
        if let Some((_, r)) = routes.iter().find(|(u, _)| url.contains(u.as_str())) {
            return Ok(r.clone());
        }
        Err(IdxError::Network(format!(
            "fixture backend has no route for {url}"
        )))
    }
}

#[async_trait]
impl Backend for FixtureBackend {
    async fn get(&self, url: &str, headers: &[(String, String)]) -> Result<RawResponse> {
        self.answer(url, headers)
    }

    /// Recorded as `POST <url>` so tests can tell it from a GET.
    async fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        _body: &str,
    ) -> Result<RawResponse> {
        self.answer(&format!("POST {url}"), headers)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// Serve fresh cache, otherwise fetch; fall back to stale on network error.
    Normal,
    /// Never touch the network. Serve anything cached, fresh or stale.
    Offline,
    /// Always fetch; still write the result to the cache.
    Bypass,
}

/// A response plus where it came from.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
    pub cached: bool,
    pub stale: bool,
    pub fetched_at: DateTime<Utc>,
    /// Why the live fetch failed when a stale copy is served instead.
    pub failure: Option<String>,
}

impl Fetched {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub struct Fetcher {
    backend: Arc<dyn Backend>,
    cache: Cache,
    mode: CacheMode,
}

impl Fetcher {
    pub fn new(backend: Arc<dyn Backend>, cache: Cache, mode: CacheMode) -> Self {
        Fetcher {
            backend,
            cache,
            mode,
        }
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    pub fn mode(&self) -> CacheMode {
        self.mode
    }

    pub fn is_offline(&self) -> bool {
        self.mode == CacheMode::Offline
    }

    /// Uncached request. Errors immediately in offline mode.
    pub async fn get_uncached(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<RawResponse> {
        if self.is_offline() {
            return Err(IdxError::OfflineMiss(format!("{url} is not cached")));
        }
        let _wait = crate::progress::request(url);
        self.backend.get(url, headers).await
    }

    /// Uncached POST, for session endpoints. Errors immediately in offline mode.
    pub async fn post_uncached(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &str,
    ) -> Result<RawResponse> {
        if self.is_offline() {
            return Err(IdxError::OfflineMiss(format!("{url} needs the network")));
        }
        let _wait = crate::progress::request(url);
        self.backend.post(url, headers, body).await
    }

    /// Cached GET. `cache_key` defaults to the URL; pass something else when
    /// the URL carries volatile parts (session tokens) that should not split
    /// the cache.
    pub async fn get(
        &self,
        url: &str,
        headers: &[(String, String)],
        ttl: Duration,
        cache_key: Option<&str>,
    ) -> Result<Fetched> {
        self.fetch(url, headers, ttl, cache_key, None, None).await
    }

    /// Cached POST for read-only query endpoints that take their parameters in
    /// the body (MSN's screener). `cache_key` is required: it names the request
    /// (the URL without secrets plus a label for the body), because the URL
    /// alone does not.
    pub async fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &str,
        ttl: Duration,
        cache_key: &str,
    ) -> Result<Fetched> {
        self.fetch(url, headers, ttl, Some(cache_key), None, Some(body))
            .await
    }

    /// Like [`get`](Self::get), but a network failure is remembered for
    /// `retry_after`: until then the source is not asked again and the call
    /// serves the stale copy, or fails, at once. For optional sources that can
    /// go silent, such as one news feed among several, where every caller
    /// would otherwise wait out the timeout.
    pub async fn get_with_backoff(
        &self,
        url: &str,
        headers: &[(String, String)],
        ttl: Duration,
        cache_key: Option<&str>,
        retry_after: Duration,
    ) -> Result<Fetched> {
        self.fetch(url, headers, ttl, cache_key, Some(retry_after), None)
            .await
    }

    /// The shared path: cache, offline, backoff and stale fallback. `body`
    /// makes the live request a POST.
    async fn fetch(
        &self,
        url: &str,
        headers: &[(String, String)],
        ttl: Duration,
        cache_key: Option<&str>,
        retry_after: Option<Duration>,
        body: Option<&str>,
    ) -> Result<Fetched> {
        let key = cache_key.unwrap_or(url);
        let hit = if self.mode == CacheMode::Bypass {
            None
        } else {
            self.cache.get(key, ttl)
        };

        if let Some(h) = &hit {
            if h.fresh {
                tracing::debug!(key, age_secs = h.age.as_secs(), "cache hit");
                return Ok(from_hit(h, false));
            }
        }

        if self.is_offline() {
            return match hit {
                Some(h) => {
                    tracing::info!(
                        key,
                        age_secs = h.age.as_secs(),
                        "offline: serving stale cache"
                    );
                    Ok(from_hit(&h, true))
                }
                None => Err(IdxError::OfflineMiss(format!("no cached copy of {key}"))),
            };
        }

        // `--no-cache` asks for the network, so only the normal mode skips.
        let mark = retry_after
            .filter(|_| self.mode == CacheMode::Normal)
            .map(|r| (format!("failed:{key}"), r));
        if let Some((mark, retry)) = &mark {
            if let Some(m) = self.cache.get(mark, *retry).filter(|m| m.fresh) {
                let why = format!(
                    "{}; next try in {}s",
                    m.stored.body,
                    retry.saturating_sub(m.age).as_secs()
                );
                return match hit {
                    Some(h) => {
                        tracing::info!(key, %why, "source failed recently: serving stale cache");
                        let mut f = from_hit(&h, true);
                        f.failure = Some(why);
                        Ok(f)
                    }
                    None => Err(IdxError::Network(why)),
                };
            }
        }

        let live = {
            let _wait = crate::progress::request(url);
            match body {
                Some(b) => self.backend.post(url, headers, b).await,
                None => self.backend.get(url, headers).await,
            }
        };
        match live {
            Ok(resp) => {
                if (200..300).contains(&resp.status) {
                    if let Err(e) = self.cache.put(key, resp.status, &resp.body) {
                        tracing::warn!(error = %e, "could not write cache");
                    }
                }
                Ok(Fetched {
                    status: resp.status,
                    body: resp.body,
                    headers: resp.headers,
                    cached: false,
                    stale: false,
                    fetched_at: Utc::now(),
                    failure: None,
                })
            }
            Err(e) => {
                // The bare message: it is wrapped again when replayed.
                let why = match &e {
                    IdxError::Network(m) => m.clone(),
                    other => other.to_string(),
                };
                if let Some((mark, _)) = &mark {
                    if let Err(w) = self.cache.put(mark, 0, &why) {
                        tracing::warn!(error = %w, "could not write cache");
                    }
                }
                match hit {
                    Some(h) => {
                        tracing::warn!(error = %e, key, "network failed: serving stale cache");
                        let mut f = from_hit(&h, true);
                        f.failure = Some(why);
                        Ok(f)
                    }
                    None => Err(e),
                }
            }
        }
    }
}

fn from_hit(h: &crate::cache::Hit, stale: bool) -> Fetched {
    Fetched {
        status: h.stored.status,
        body: h.stored.body.clone(),
        headers: vec![],
        cached: true,
        stale,
        fetched_at: h.stored.fetched_at,
        failure: None,
    }
}

/// Helper for building `Cookie:` headers from `set-cookie` responses.
pub fn cookie_pairs(set_cookie_values: &[&str]) -> String {
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut order = Vec::new();
    for v in set_cookie_values {
        if let Some(pair) = v.split(';').next() {
            if let Some((k, val)) = pair.split_once('=') {
                let k = k.trim().to_string();
                if !seen.contains_key(&k) {
                    order.push(k.clone());
                }
                seen.insert(k, val.trim().to_string());
            }
        }
    }
    order
        .into_iter()
        .map(|k| format!("{k}={}", seen[&k]))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fetcher(backend: FixtureBackend, mode: CacheMode) -> (Fetcher, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (
            Fetcher::new(Arc::new(backend), Cache::new(dir.path()), mode),
            dir,
        )
    }

    #[tokio::test]
    async fn caches_successful_responses() {
        let be = FixtureBackend::new().route("example.test/a", 200, "{\"x\":1}");
        let (f, _d) = fetcher(be, CacheMode::Normal);
        let a = f
            .get("https://example.test/a", &[], Duration::from_secs(60), None)
            .await
            .unwrap();
        assert!(!a.cached);
        let b = f
            .get("https://example.test/a", &[], Duration::from_secs(60), None)
            .await
            .unwrap();
        assert!(b.cached && !b.stale);
        assert_eq!(b.body, "{\"x\":1}");
    }

    #[tokio::test]
    async fn does_not_cache_errors() {
        let be = FixtureBackend::new().route("example.test/e", 500, "boom");
        let (f, _d) = fetcher(be, CacheMode::Normal);
        f.get("https://example.test/e", &[], Duration::from_secs(60), None)
            .await
            .unwrap();
        assert!(f
            .cache()
            .get("https://example.test/e", Duration::from_secs(60))
            .is_none());
    }

    #[tokio::test]
    async fn offline_serves_stale_or_errors() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache.put("https://example.test/s", 200, "old").unwrap();
        let f = Fetcher::new(Arc::new(FixtureBackend::new()), cache, CacheMode::Offline);
        let got = f
            .get("https://example.test/s", &[], Duration::ZERO, None)
            .await
            .unwrap();
        assert!(got.cached && got.body == "old");
        let err = f
            .get("https://example.test/missing", &[], Duration::ZERO, None)
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 5);
        assert_eq!(
            f.get_uncached("https://x", &[])
                .await
                .unwrap_err()
                .exit_code(),
            5
        );
    }

    #[tokio::test]
    async fn network_failure_falls_back_to_stale() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache.put("https://example.test/s", 200, "old").unwrap();
        let f = Fetcher::new(Arc::new(FixtureBackend::new()), cache, CacheMode::Normal);
        let got = f
            .get("https://example.test/s", &[], Duration::ZERO, None)
            .await
            .unwrap();
        assert!(got.stale);
    }

    #[tokio::test]
    async fn backoff_remembers_a_network_failure() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(FixtureBackend::new());
        let f = Fetcher::new(backend.clone(), Cache::new(dir.path()), CacheMode::Normal);
        let (url, ttl, retry) = (
            "https://silent.test/rss",
            Duration::from_secs(60),
            Duration::from_secs(300),
        );
        let first = f
            .get_with_backoff(url, &[], ttl, None, retry)
            .await
            .unwrap_err();
        assert!(first.to_string().contains("no route"), "{first}");
        let second = f
            .get_with_backoff(url, &[], ttl, None, retry)
            .await
            .unwrap_err();
        assert!(second.to_string().contains("next try in"), "{second}");
        assert_eq!(
            second.to_string().matches("network:").count(),
            1,
            "{second}"
        );
        assert_eq!(backend.call_count(), 1);
        // Plain get never skips.
        let _ = f.get(url, &[], ttl, None).await;
        assert_eq!(backend.call_count(), 2);
    }

    #[tokio::test]
    async fn backoff_serves_stale_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache.put("https://silent.test/rss", 200, "old").unwrap();
        let backend = Arc::new(FixtureBackend::new());
        let f = Fetcher::new(backend.clone(), cache, CacheMode::Normal);
        for _ in 0..2 {
            let got = f
                .get_with_backoff(
                    "https://silent.test/rss",
                    &[],
                    Duration::ZERO,
                    None,
                    Duration::from_secs(300),
                )
                .await
                .unwrap();
            assert!(got.stale);
            assert_eq!(got.body, "old");
            assert!(got.failure.unwrap().contains("no route"));
        }
        assert_eq!(backend.call_count(), 1);
    }

    #[tokio::test]
    async fn bypass_always_fetches() {
        let be = FixtureBackend::new().route("example.test/a", 200, "new");
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache.put("https://example.test/a", 200, "old").unwrap();
        let f = Fetcher::new(Arc::new(be), cache, CacheMode::Bypass);
        let got = f
            .get(
                "https://example.test/a",
                &[],
                Duration::from_secs(600),
                None,
            )
            .await
            .unwrap();
        assert_eq!(got.body, "new");
    }

    #[tokio::test]
    async fn post_is_cached_under_its_key() {
        let be = Arc::new(FixtureBackend::new().route("POST https://q.test/screen", 200, "[1]"));
        let dir = tempfile::tempdir().unwrap();
        let f = Fetcher::new(be.clone(), Cache::new(dir.path()), CacheMode::Normal);
        let ttl = Duration::from_secs(60);
        let key = "https://q.test/screen?body=all";
        let a = f
            .post("https://q.test/screen?apikey=secret", &[], "{}", ttl, key)
            .await
            .unwrap();
        assert!(!a.cached);
        let b = f
            .post("https://q.test/screen?apikey=other", &[], "{}", ttl, key)
            .await
            .unwrap();
        assert!(b.cached && !b.stale);
        assert_eq!(b.body, "[1]");
        assert_eq!(be.calls_matching("POST "), 1);
        assert_eq!(be.call_count(), 1);
        // The key, not the URL with its secret, is what the cache stores.
        assert!(f.cache().get(key, ttl).is_some());
        assert!(f
            .cache()
            .get("https://q.test/screen?apikey=secret", ttl)
            .is_none());
    }

    #[tokio::test]
    async fn offline_serves_a_cached_post() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache.put("screen:all", 200, "old").unwrap();
        let be = Arc::new(FixtureBackend::new());
        let f = Fetcher::new(be.clone(), cache, CacheMode::Offline);
        let got = f
            .post(
                "https://q.test/screen",
                &[],
                "{}",
                Duration::ZERO,
                "screen:all",
            )
            .await
            .unwrap();
        assert!(got.cached && got.stale);
        assert_eq!(got.body, "old");
        let err = f
            .post(
                "https://q.test/screen",
                &[],
                "{}",
                Duration::ZERO,
                "screen:none",
            )
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 5);
        assert_eq!(be.call_count(), 0, "offline never reaches the backend");
    }

    #[tokio::test]
    async fn failed_post_serves_stale() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache.put("screen:all", 200, "old").unwrap();
        // No route: the live POST fails like a network error.
        let be = Arc::new(FixtureBackend::new());
        let f = Fetcher::new(be.clone(), cache, CacheMode::Normal);
        let got = f
            .post(
                "https://q.test/screen",
                &[],
                "{}",
                Duration::ZERO,
                "screen:all",
            )
            .await
            .unwrap();
        assert!(got.stale);
        assert_eq!(got.body, "old");
        assert!(got.failure.unwrap().contains("no route"));
        assert_eq!(be.calls_matching("POST https://q.test/screen"), 1);
    }

    #[test]
    fn cookie_header_building() {
        let c = cookie_pairs(&["A3=abc; Domain=.yahoo.com; Secure", "B=1; Path=/", "A3=zzz"]);
        assert_eq!(c, "A3=zzz; B=1");
    }
}
