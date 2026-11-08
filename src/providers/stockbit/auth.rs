//! Stockbit session: the bearer tokens, where they are stored, and how they
//! are refreshed.
//!
//! Stockbit authorises every request with `Authorization: Bearer <JWT>`. There
//! is no API key and no cookie. Two tokens matter:
//!
//! - the **access** token, good for ~24 h, sent on every data request;
//! - the **refresh** token, good for ~7 days, spent once to mint a new access
//!   token.
//!
//! `POST /login/refresh` (bearer = the refresh token, empty JSON body) returns
//! a *new* access token **and a new refresh token**, retiring the one
//! presented. That rotation is why one session must have exactly one refresher:
//! if two processes share a token pair they retire each other's credential and
//! both get logged out. See `docs/stockbit.md`.
//!
//! The session file is the source of truth; the env vars only seed an empty
//! one. The file is written with mode `0600` on Unix.
//!
//! Refreshes are serialised twice over: an in-process lock (one
//! [`SessionStore`] per session file per process, see
//! `providers::StockbitOptions::store`) and a lock file beside the session
//! (`stockbit-session.lock`) for other idx processes (a CLI command next to a
//! running MCP server). Inside both, the file is read again, so a refresh
//! another holder just made is used instead of spending the new refresh token.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{IdxError, Result};
use crate::http::Fetcher;

/// Where `/login/refresh` lives. Exodus is Stockbit's market-data backend.
pub const REFRESH_URL: &str = "https://exodus.stockbit.com/login/refresh";

/// Refresh the access token this long before it actually expires, so a request
/// never races the expiry.
const REFRESH_BUFFER_SECS: i64 = 60 * 60;

/// How long another process may hold the refresh lock before it counts as
/// abandoned (a crashed process leaves the file behind).
const LOCK_STALE: Duration = Duration::from_secs(60);
/// How long to wait for another process's refresh before giving up.
const LOCK_WAIT: Duration = Duration::from_secs(20);

/// The persisted session. Only the two tokens; expiries are decoded from them.
/// `Debug` never prints them.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    #[serde(default)]
    pub access: String,
    #[serde(default)]
    pub refresh: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shown = |t: &str| {
            if t.trim().is_empty() {
                "<empty>"
            } else {
                "<redacted>"
            }
        };
        f.debug_struct("Session")
            .field("access", &shown(&self.access))
            .field("refresh", &shown(&self.refresh))
            .finish()
    }
}

impl Session {
    pub fn is_empty(&self) -> bool {
        self.refresh.trim().is_empty() && self.access.trim().is_empty()
    }
}

/// Why a refresh did not produce a new pair.
#[derive(Debug)]
pub enum RefreshError {
    /// Stockbit answered 401, or 403 with a JSON body: the refresh token is
    /// expired, revoked or already spent. Retrying it cannot help.
    Rejected,
    /// Anything else (network, offline, an unexpected answer); a later try may work.
    Failed(IdxError),
}

impl From<RefreshError> for IdxError {
    fn from(e: RefreshError) -> Self {
        match e {
            RefreshError::Rejected => IdxError::provider(super::NAME, REFRESH_REJECTED),
            RefreshError::Failed(e) => e,
        }
    }
}

/// The message of a refresh Stockbit refused; `super::is_fatal` keys on it.
pub const REFRESH_REJECTED: &str =
    "refresh token rejected (expired or revoked); run `idx stockbit login` again";

/// Strips a leading `Bearer ` and surrounding whitespace.
pub fn normalize_token(raw: &str) -> String {
    let t = raw.trim();
    t.strip_prefix("Bearer ")
        .or_else(|| t.strip_prefix("bearer "))
        .unwrap_or(t)
        .trim()
        .to_string()
}

/// Seconds since the Unix epoch that a JWT expires (`exp`), without verifying
/// the signature — we only ever carry these, never issue them. `None` when the
/// token is not a JWT or has no numeric `exp`.
pub fn token_expiry(token: &str) -> Option<i64> {
    claims(token)?.get("exp").and_then(|e| e.as_i64())
}

/// A JWT's claims (its middle segment, base64url JSON), without verifying the
/// signature. `None` when the token is not a JWT.
fn claims(token: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    match serde_json::from_slice(&bytes).ok()? {
        serde_json::Value::Object(map) => Some(map),
        _ => None,
    }
}

/// What a stored token can still do at `now`, judged by its own `exp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenState {
    /// No token is stored.
    Missing,
    /// A JWT whose `exp` is still ahead, or one without an `exp`: nothing in
    /// it says it has expired, so Stockbit's answer decides.
    Live,
    /// A JWT whose `exp` has passed: Stockbit refuses it.
    Expired,
    /// Not a JWT, so not a token Stockbit issued: it cannot work.
    NotJwt,
}

impl TokenState {
    fn of(token: &str, now: i64) -> Self {
        if token.trim().is_empty() {
            TokenState::Missing
        } else if claims(token).is_none() {
            TokenState::NotJwt
        } else if token_expiry(token).is_some_and(|exp| now >= exp) {
            TokenState::Expired
        } else {
            TokenState::Live
        }
    }
}

/// A readable, non-secret summary of a token for `status` and logs: never the
/// token itself.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TokenInfo {
    pub present: bool,
    /// Unix seconds the token expires, `null` when it is absent or not a JWT.
    pub expires_at: Option<i64>,
    /// Seconds until expiry (negative when already expired).
    pub expires_in_secs: Option<i64>,
}

impl TokenInfo {
    fn of(token: &str, now: i64) -> Self {
        let present = !token.trim().is_empty();
        let expires_at = if present { token_expiry(token) } else { None };
        TokenInfo {
            present,
            expires_at,
            expires_in_secs: expires_at.map(|e| e - now),
        }
    }
}

/// Session status without exposing any token (`idx schema stockbit-status`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Status {
    /// `true` when a refresh token is present (whether or not it is still live).
    pub logged_in: bool,
    /// `true` when idx can renew the access token: a refresh token is stored
    /// and its `exp` has not passed (one without an `exp` counts as
    /// unexpired; a token that is not a JWT, as unusable).
    #[serde(default)]
    pub renewable: bool,
    pub access: TokenInfo,
    pub refresh: TokenInfo,
    /// Absolute path of the session file.
    pub session_file: String,
    /// A sign-in the MCP server started with `idx_stockbit_login` and its
    /// progress; absent otherwise (the CLI never sets it).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sign_in: Option<SignInProgress>,
}

/// Progress of a sign-in started from MCP (`idx_stockbit_login`). Carries
/// no token, only where the user goes next.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SignInProgress {
    /// `waiting` (the page is open), `signed_in`, `failed`, or
    /// `already_signed_in` (the stored session works; nothing was opened).
    pub state: String,
    /// `page` (a page on 127.0.0.1 for the user's own browser), `browser`
    /// (a browser idx opened on this machine), or `none`.
    pub method: String,
    /// The page to open while `waiting` (`page` method).
    pub url: Option<String>,
    /// The page's port on 127.0.0.1.
    pub port: Option<u16>,
    /// The `ssh -N -L` command that forwards the page, when this server is
    /// reached over SSH.
    pub ssh_forward: Option<String>,
    /// Seconds until the page stops listening.
    pub expires_in_secs: Option<i64>,
    /// `true` when a page started earlier was still open and is returned
    /// instead of a new one.
    pub already_open: bool,
    /// The latest note, e.g. why Stockbit refused a pasted session.
    pub warning: Option<String>,
    /// Why the sign-in failed (`failed`).
    pub error: Option<String>,
}

/// Reads and writes the session file and performs the refresh. One per
/// session file per process (`providers::StockbitOptions::store` hands out the
/// same one); the inflight lock and the lock file collapse concurrent refreshes.
pub struct SessionStore {
    path: PathBuf,
    /// Seed tokens from the environment, used only when the file is empty.
    seed: Session,
    inflight: Mutex<()>,
    /// A refresh token Stockbit refused in this process (its SHA-256), so it
    /// is not presented again by every later request.
    refused: std::sync::Mutex<Option<[u8; 32]>>,
    /// When the last paced live request on this session went out. Kept here,
    /// not on the provider, so every provider over this store (MCP builds one
    /// per tool call) is paced against the others.
    pacer: Mutex<Option<tokio::time::Instant>>,
}

/// `true` while `token` is a JWT that is still good for the refresh buffer.
pub(super) fn fresh(token: &str, now: i64) -> bool {
    token
        .split('.')
        .nth(1)
        .and_then(|_| token_expiry(token))
        .is_some_and(|exp| now < exp - REFRESH_BUFFER_SECS)
}

/// SHA-256 of a token, for remembering one without holding it.
pub(super) fn digest(token: &str) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(token.trim().as_bytes()).into()
}

/// Holds `<session>.lock` for one refresh; removes it when dropped.
struct LockFile(PathBuf);

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl LockFile {
    /// Creates the lock file, waiting for another holder up to [`LOCK_WAIT`]
    /// and taking over one older than [`LOCK_STALE`].
    async fn acquire(path: PathBuf) -> Result<LockFile> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let deadline = tokio::time::Instant::now() + LOCK_WAIT;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(LockFile(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if stale {
                        tracing::warn!("taking over a stale Stockbit refresh lock");
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Err(IdxError::provider(
                            super::NAME,
                            format!(
                                "another idx process is refreshing the Stockbit session (lock {}); try again",
                                path.display()
                            ),
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl SessionStore {
    /// `seed_access`/`seed_refresh` come from `IDX_STOCKBIT_*`; either may be
    /// empty. The file, when present, always wins over them.
    pub fn new(path: PathBuf, seed_access: Option<String>, seed_refresh: Option<String>) -> Self {
        SessionStore {
            path,
            seed: Session {
                access: seed_access.map(|s| normalize_token(&s)).unwrap_or_default(),
                refresh: seed_refresh
                    .map(|s| normalize_token(&s))
                    .unwrap_or_default(),
            },
            inflight: Mutex::new(()),
            refused: std::sync::Mutex::new(None),
            pacer: Mutex::new(None),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The time of this session's last paced live request; held while a
    /// paced request waits and goes out, so paced requests queue.
    pub(super) fn pacer(&self) -> &Mutex<Option<tokio::time::Instant>> {
        &self.pacer
    }

    /// The session file's raw bytes, `None` when there is none. For putting a
    /// previous session back after Stockbit refuses a new one.
    pub fn read_raw(&self) -> Option<Vec<u8>> {
        std::fs::read(&self.path).ok()
    }

    /// Puts back what [`read_raw`](Self::read_raw) returned: the bytes, or no
    /// file at all.
    pub fn restore_raw(&self, raw: Option<&[u8]>) -> Result<()> {
        match raw {
            Some(bytes) => {
                let tmp = self.path.with_extension("json.tmp");
                std::fs::write(&tmp, bytes)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
                }
                std::fs::rename(&tmp, &self.path)?;
                Ok(())
            }
            None => self.clear().map(|_| ()),
        }
    }

    /// The file's session, falling back to the env seed field by field.
    pub fn load(&self) -> Session {
        let mut s = match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str::<Session>(&text).unwrap_or_default(),
            Err(_) => Session::default(),
        };
        s.access = normalize_token(&s.access);
        s.refresh = normalize_token(&s.refresh);
        if s.access.is_empty() {
            s.access = self.seed.access.clone();
        }
        if s.refresh.is_empty() {
            s.refresh = self.seed.refresh.clone();
        }
        s
    }

    /// Writes the session file atomically with `0600` permissions on Unix.
    pub fn save(&self, session: &Session) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(session)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, body.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn clear(&self) -> Result<bool> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub fn status(&self, now: i64) -> Status {
        let s = self.load();
        Status {
            logged_in: !s.refresh.trim().is_empty(),
            renewable: TokenState::of(&s.refresh, now) == TokenState::Live,
            access: TokenInfo::of(&s.access, now),
            refresh: TokenInfo::of(&s.refresh, now),
            session_file: self.path.display().to_string(),
            sign_in: None,
        }
    }

    /// A valid access token, refreshing proactively when the current one is
    /// within the buffer of expiry, already expired, or missing. Returns a
    /// usage error when there is no session at all.
    pub async fn valid_access(&self, fetcher: &Fetcher, now: i64) -> Result<String> {
        self.valid_session(fetcher, now).await.map(|s| s.access)
    }

    /// Like [`valid_access`](Self::valid_access), but the whole session the
    /// access token belongs to, for handing back to [`rotate`](Self::rotate)
    /// when Stockbit rejects that token.
    pub async fn valid_session(&self, fetcher: &Fetcher, now: i64) -> Result<Session> {
        let current = self.load();
        if current.is_empty() {
            return Err(IdxError::Usage(
                "no Stockbit session; run `idx stockbit login` (or set IDX_STOCKBIT_REFRESH_TOKEN)"
                    .into(),
            ));
        }
        if fresh(&current.access, now) {
            return Ok(current);
        }
        match self.rotate(fetcher, &current, now).await {
            Ok(new) => Ok(new),
            Err(e) if !current.access.trim().is_empty() => {
                // A failed refresh must not block a request while the access
                // token may still be usable; Stockbit's answer decides.
                tracing::warn!(error = ?e, "stockbit refresh failed; using the current access token");
                Ok(current)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Replaces `seen` (the session a caller found stale or Stockbit
    /// rejected) with a fresh pair, and stores it. Serialised in this process
    /// and across idx processes; inside the locks the file is read again, and
    /// a pair another holder already rotated is returned instead of spending
    /// its refresh token a second time.
    pub async fn rotate(
        &self,
        fetcher: &Fetcher,
        seen: &Session,
        now: i64,
    ) -> std::result::Result<Session, RefreshError> {
        let _guard = self.inflight.lock().await;
        let _lock = LockFile::acquire(self.path.with_extension("lock"))
            .await
            .map_err(RefreshError::Failed)?;
        let after = self.load();
        if after != *seen && fresh(&after.access, now) {
            return Ok(after);
        }
        let new = self.refresh_checked(fetcher, &after).await?;
        self.save(&new).map_err(RefreshError::Failed)?;
        Ok(new)
    }

    /// Exchanges the refresh token for a new pair. Does not write the file.
    pub async fn refresh(&self, fetcher: &Fetcher, session: &Session) -> Result<Session> {
        self.refresh_checked(fetcher, session)
            .await
            .map_err(IdxError::from)
    }

    /// [`refresh`](Self::refresh) that tells a refused token from a failure.
    /// A token Stockbit refused once in this process is not presented again.
    pub async fn refresh_checked(
        &self,
        fetcher: &Fetcher,
        session: &Session,
    ) -> std::result::Result<Session, RefreshError> {
        let refresh = session.refresh.trim();
        if refresh.is_empty() {
            return Err(RefreshError::Failed(IdxError::Usage(
                "no Stockbit refresh token; run `idx stockbit login`".into(),
            )));
        }
        let key = digest(refresh);
        if *self.refused.lock().unwrap_or_else(|e| e.into_inner()) == Some(key) {
            return Err(RefreshError::Rejected);
        }
        let headers = super::request_headers(refresh);
        let resp = fetcher
            .post_uncached(REFRESH_URL, &headers, "{}")
            .await
            .map_err(RefreshError::Failed)?;
        if super::rejected(resp.status, &resp.body) {
            *self.refused.lock().unwrap_or_else(|e| e.into_inner()) = Some(key);
            return Err(RefreshError::Rejected);
        }
        if resp.status == 403 {
            // An HTML 403 is the CDN refusing this client, not a verdict on
            // the token: report the block and keep the token usable.
            return Err(RefreshError::Failed(IdxError::provider(
                super::NAME,
                super::CDN_BLOCKED,
            )));
        }
        if !(200..300).contains(&resp.status) {
            return Err(RefreshError::Failed(IdxError::provider(
                super::NAME,
                format!("refresh failed: HTTP {}", resp.status),
            )));
        }
        parse_refresh(&resp.body, session).map_err(RefreshError::Failed)
    }
}

/// Reads the new token pair out of a `/login/refresh` response. The envelope
/// has moved across API versions (`data.access.token`, once `data.data...`),
/// so this searches structurally for `access`/`refresh` holding a JWT, rather
/// than following a fixed path. A refresh that rotates only the access token
/// keeps the presented refresh token.
pub fn parse_refresh(body: &str, previous: &Session) -> Result<Session> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| IdxError::provider(super::NAME, format!("refresh response not JSON: {e}")))?;
    let access = find_slot_token(&v, "access");
    let refresh = find_slot_token(&v, "refresh");
    match access {
        Some(access) => Ok(Session {
            access: normalize_token(&access),
            refresh: refresh
                .map(|r| normalize_token(&r))
                .unwrap_or_else(|| previous.refresh.clone()),
        }),
        None => Err(IdxError::provider(
            super::NAME,
            "refresh response carried no access token",
        )),
    }
}

/// `true` for a three-segment base64url string starting `eyJ` — the shape of
/// every JWT we carry. Not a signature check.
fn looks_like_jwt(v: &str) -> bool {
    let mut parts = v.split('.');
    matches!(parts.next(), Some(h) if h.starts_with("eyJ"))
        && parts.next().is_some_and(|p| !p.is_empty())
        && parts.next().is_some_and(|p| !p.is_empty())
}

/// Finds the JWT under a key named `slot`, in either shape Stockbit has used:
/// a bare string, or an object with a `token` field. Walks the whole tree.
fn find_slot_token(v: &serde_json::Value, slot: &str) -> Option<String> {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(hit) = map.get(slot) {
                if let Some(s) = hit.as_str().filter(|s| looks_like_jwt(s)) {
                    return Some(s.to_string());
                }
                if let Some(tok) = hit.get("token").and_then(|t| t.as_str()) {
                    if looks_like_jwt(tok) {
                        return Some(tok.to_string());
                    }
                }
            }
            map.values().find_map(|child| find_slot_token(child, slot))
        }
        serde_json::Value::Array(items) => items.iter().find_map(|c| find_slot_token(c, slot)),
        _ => None,
    }
}

/// Pulls both tokens out of the website's `credentialStorage` cookie value
/// (URL-encoded JSON `{state:{access,refresh,...}}`), accepting the bare-JWT
/// and `{token}` shapes. Used by the login capture's already-signed-in path
/// and by `login --manual`.
pub fn session_from_credential_blob(raw: &str) -> Option<Session> {
    let decoded = percent_decode(raw.trim());
    for candidate in [decoded.as_str(), raw.trim()] {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(candidate) {
            if let Some(access) = find_slot_token(&v, "access") {
                return Some(Session {
                    access: normalize_token(&access),
                    refresh: find_slot_token(&v, "refresh")
                        .map(|r| normalize_token(&r))
                        .unwrap_or_default(),
                });
            }
        }
    }
    // A bare refresh JWT pasted on its own.
    let t = normalize_token(raw);
    looks_like_jwt(&t).then(|| Session {
        access: String::new(),
        refresh: t,
    })
}

/// Minimal `%XX` decoder (the cookie value is percent-encoded JSON). Avoids a
/// url-crate dependency for one small decode.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFRESH: &str =
        include_str!("../../../tests/fixtures/stockbit/login_refresh_constructed.json");
    const COOKIE: &str =
        include_str!("../../../tests/fixtures/stockbit/credential_storage_cookie_constructed.txt");

    #[test]
    fn normalizes_bearer_prefix() {
        assert_eq!(normalize_token("  Bearer abc.def.ghi "), "abc.def.ghi");
        assert_eq!(normalize_token("abc"), "abc");
    }

    #[test]
    fn decodes_jwt_expiry() {
        let access = find_slot_token(&serde_json::from_str(REFRESH).unwrap(), "access").unwrap();
        // The constructed token expires 24h after 1792298608.
        assert_eq!(token_expiry(&access), Some(1792298608 + 86400));
        assert_eq!(token_expiry("not-a-jwt"), None);
    }

    #[test]
    fn parses_refresh_response() {
        let prev = Session {
            access: "old".into(),
            refresh: "old-refresh".into(),
        };
        let s = parse_refresh(REFRESH, &prev).unwrap();
        assert!(looks_like_jwt(&s.access));
        assert!(looks_like_jwt(&s.refresh));
        assert_ne!(s.refresh, "old-refresh");
    }

    #[test]
    fn refresh_response_without_new_refresh_keeps_the_old_one() {
        let prev = Session {
            access: "old".into(),
            refresh: "keep-me".into(),
        };
        let body = r#"{"data":{"access":{"token":"eyJ0.eyJleHAiOjF9.sig"}}}"#;
        let s = parse_refresh(body, &prev).unwrap();
        assert_eq!(s.refresh, "keep-me");
    }

    #[test]
    fn rejects_refresh_without_access() {
        let body = r#"{"data":{"message":"nope"}}"#;
        assert!(parse_refresh(body, &Session::default()).is_err());
    }

    #[test]
    fn reads_the_credential_cookie() {
        let s = session_from_credential_blob(COOKIE).unwrap();
        assert!(looks_like_jwt(&s.access));
        assert!(looks_like_jwt(&s.refresh));
    }

    #[test]
    fn reads_a_bare_refresh_jwt() {
        let s = session_from_credential_blob("eyJ0.eyJleHAiOjF9.sig").unwrap();
        assert_eq!(s.access, "");
        assert!(looks_like_jwt(&s.refresh));
    }

    #[test]
    fn debug_never_prints_a_token() {
        let s = session_from_credential_blob(COOKIE).unwrap();
        let shown = format!("{s:?} {:?}", Session::default());
        assert!(!shown.contains("eyJ"), "{shown}");
        assert!(
            shown.contains("<redacted>") && shown.contains("<empty>"),
            "{shown}"
        );
    }

    fn fetcher_with(be: std::sync::Arc<crate::http::FixtureBackend>, dir: &Path) -> Fetcher {
        Fetcher::new(
            be,
            crate::cache::Cache::new(dir.join("cache")),
            crate::http::CacheMode::Normal,
        )
    }

    #[tokio::test]
    async fn rotate_uses_a_pair_another_holder_stored() {
        let dir = tempfile::tempdir().unwrap();
        let be = std::sync::Arc::new(crate::http::FixtureBackend::new().route(
            "login/refresh",
            200,
            REFRESH,
        ));
        let fetcher = fetcher_with(be.clone(), dir.path());
        let store = SessionStore::new(dir.path().join("session.json"), None, None);
        let stale = Session {
            access: String::new(),
            refresh: "eyJ0.eyJleHAiOjF9.old".into(),
        };
        store.save(&stale).unwrap();
        // The constructed pair is issued at 1792298608; "now" is just after.
        let now = 1_792_298_700;
        let first = store.rotate(&fetcher, &stale, now).await.unwrap();
        assert_ne!(first.refresh, stale.refresh);
        assert_eq!(store.load(), first);
        // A second caller that saw the same stale session gets the stored
        // pair; the new refresh token is not spent again.
        let second = store.rotate(&fetcher, &stale, now).await.unwrap();
        assert_eq!(second, first);
        assert_eq!(be.calls_matching("login/refresh"), 1);
        assert!(!dir.path().join("session.lock").exists(), "lock released");
    }

    #[tokio::test]
    async fn a_refused_refresh_token_is_not_presented_twice() {
        let dir = tempfile::tempdir().unwrap();
        let be = std::sync::Arc::new(crate::http::FixtureBackend::new().route(
            "login/refresh",
            401,
            r#"{"error_type":"UNAUTHORIZED","message":"Unauthorized"}"#,
        ));
        let fetcher = fetcher_with(be.clone(), dir.path());
        let store = SessionStore::new(dir.path().join("session.json"), None, None);
        let s = Session {
            access: String::new(),
            refresh: "eyJ0.eyJleHAiOjF9.sig".into(),
        };
        for _ in 0..3 {
            assert!(matches!(
                store.refresh_checked(&fetcher, &s).await,
                Err(RefreshError::Rejected)
            ));
        }
        assert_eq!(be.calls_matching("login/refresh"), 1);
        let e = IdxError::from(RefreshError::Rejected);
        assert_eq!(e.exit_code(), 3);
        assert!(e.to_string().contains("idx stockbit login"), "{e}");
    }

    #[tokio::test]
    async fn an_html_403_on_refresh_is_a_block_not_a_refused_token() {
        const BLOCKED: &str = include_str!("../../../tests/fixtures/stockbit/blocked_403.html");
        let dir = tempfile::tempdir().unwrap();
        let be = std::sync::Arc::new(crate::http::FixtureBackend::new().route(
            "login/refresh",
            403,
            BLOCKED,
        ));
        let fetcher = fetcher_with(be.clone(), dir.path());
        let store = SessionStore::new(dir.path().join("session.json"), None, None);
        let s = Session {
            access: String::new(),
            refresh: "eyJ0.eyJleHAiOjF9.sig".into(),
        };
        // Not remembered as refused: each try is sent, and each says CDN.
        for n in 1..=2 {
            match store.refresh_checked(&fetcher, &s).await {
                Err(RefreshError::Failed(e)) => {
                    assert!(e.to_string().contains("HTTP 403 from its CDN"), "{e}");
                    assert!(super::super::is_fatal(&e));
                }
                other => panic!("expected a CDN failure, got {other:?}"),
            }
            assert_eq!(be.calls_matching("login/refresh"), n);
        }
    }

    #[tokio::test]
    async fn a_held_lock_file_is_waited_for_and_a_stale_one_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.lock");
        let held = LockFile::acquire(path.clone()).await.unwrap();
        let waiter = tokio::spawn({
            let path = path.clone();
            async move { LockFile::acquire(path).await.map(|_| ()) }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!waiter.is_finished(), "waits while the lock is held");
        drop(held);
        waiter.await.unwrap().unwrap();
        // A lock left behind by a crashed process is taken over.
        std::fs::write(&path, b"").unwrap();
        let old = std::time::SystemTime::now() - LOCK_STALE - Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let taken = LockFile::acquire(path.clone()).await.unwrap();
        drop(taken);
        assert!(!path.exists());
    }

    #[test]
    fn restore_puts_back_the_bytes_or_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("session.json"), None, None);
        assert_eq!(store.read_raw(), None);
        store.restore_raw(None).unwrap();
        store.restore_raw(Some(br#"{"refresh":"x"}"#)).unwrap();
        assert_eq!(store.read_raw().unwrap(), br#"{"refresh":"x"}"#);
        store.restore_raw(None).unwrap();
        assert!(!dir.path().join("session.json").exists());
    }

    /// 2026-11-08 00:00 UTC, the "now" of the status tests.
    const NOW: i64 = 1_794_096_000;
    const DAY: i64 = 86_400;

    /// A synthetic JWT, signed by nobody, carrying `claims`.
    fn jwt(claims: &str) -> String {
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!("{}.{}.sig", b64(r#"{"alg":"none"}"#), b64(claims))
    }

    /// A synthetic JWT whose `exp` is `exp`.
    fn expiring(exp: i64) -> String {
        jwt(&format!(r#"{{"exp":{exp}}}"#))
    }

    /// The status of a stored session at `now`.
    fn status_of(access: &str, refresh: &str, now: i64) -> Status {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("session.json"), None, None);
        store
            .save(&Session {
                access: access.into(),
                refresh: refresh.into(),
            })
            .unwrap();
        store.status(now)
    }

    #[test]
    fn renewable_needs_a_refresh_token_whose_exp_has_not_passed() {
        let live = expiring(NOW + DAY);
        for (refresh, renewable) in [
            (expiring(NOW + 6 * DAY), true),
            (expiring(NOW - 13 * DAY), false),
            // `exp` is the first second Stockbit refuses the token.
            (expiring(NOW), false),
            // Nothing in a JWT without `exp` says it has expired.
            (jwt(r#"{"sub":"1"}"#), true),
            ("not-a-jwt".to_string(), false),
            (String::new(), false),
        ] {
            let got = status_of(&live, &refresh, NOW);
            assert_eq!(got.renewable, renewable, "{refresh:?}");
        }
    }

    #[test]
    fn save_load_roundtrip_and_env_seed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let store = SessionStore::new(path.clone(), None, Some("eyJ0.seed.sig".into()));
        // Empty file: the env seed fills the refresh token.
        assert_eq!(store.load().refresh, "eyJ0.seed.sig");
        store
            .save(&Session {
                access: "eyJ0.a.sig".into(),
                refresh: "eyJ0.r.sig".into(),
            })
            .unwrap();
        // File wins over the seed.
        let loaded = store.load();
        assert_eq!(loaded.access, "eyJ0.a.sig");
        assert_eq!(loaded.refresh, "eyJ0.r.sig");
        assert!(store.clear().unwrap());
        assert_eq!(store.load().refresh, "eyJ0.seed.sig");
    }
}
