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

use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{IdxError, Result};
use crate::http::Fetcher;

/// Where `/login/refresh` lives. Exodus is Stockbit's market-data backend.
pub const REFRESH_URL: &str = "https://exodus.stockbit.com/login/refresh";

/// Refresh the access token this long before it actually expires, so a request
/// never races the expiry.
const REFRESH_BUFFER_SECS: i64 = 60 * 60;

/// The persisted session. Only the two tokens; expiries are decoded from them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    #[serde(default)]
    pub access: String,
    #[serde(default)]
    pub refresh: String,
}

impl Session {
    pub fn is_empty(&self) -> bool {
        self.refresh.trim().is_empty() && self.access.trim().is_empty()
    }
}

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
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("exp").and_then(|e| e.as_i64())
}

/// A readable, non-secret summary of a token for `status` and logs: never the
/// token itself.
#[derive(Debug, Clone, Serialize)]
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

/// Session status without exposing any token.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// `true` when a refresh token is present (whether or not it is still live).
    pub logged_in: bool,
    pub access: TokenInfo,
    pub refresh: TokenInfo,
    /// Absolute path of the session file.
    pub session_file: String,
}

/// Reads and writes the session file and performs the refresh. One per
/// session file per process (`StockbitOptions::store` hands out the same
/// one); the inflight lock collapses concurrent refreshes.
pub struct SessionStore {
    path: PathBuf,
    /// Seed tokens from the environment, used only when the file is empty.
    seed: Session,
    inflight: Mutex<()>,
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
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
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
            access: TokenInfo::of(&s.access, now),
            refresh: TokenInfo::of(&s.refresh, now),
            session_file: self.path.display().to_string(),
        }
    }

    /// A valid access token, refreshing proactively when the current one is
    /// within the buffer of expiry, already expired, or missing. Returns a
    /// usage error when there is no session at all.
    pub async fn valid_access(&self, fetcher: &Fetcher, now: i64) -> Result<String> {
        let current = self.load();
        if current.is_empty() {
            return Err(IdxError::Usage(
                "no Stockbit session; run `idx stockbit login` (or set IDX_STOCKBIT_REFRESH_TOKEN)"
                    .into(),
            ));
        }
        let fresh = current
            .access
            .split('.')
            .nth(1)
            .and_then(|_| token_expiry(&current.access))
            .is_some_and(|exp| now < exp - REFRESH_BUFFER_SECS);
        if fresh {
            return Ok(current.access);
        }
        // Serialise refreshes: the second caller re-reads the file the first
        // wrote instead of spending the rotating refresh token again.
        let _guard = self.inflight.lock().await;
        let after = self.load();
        if after.access != current.access
            && token_expiry(&after.access).is_some_and(|exp| now < exp - REFRESH_BUFFER_SECS)
        {
            return Ok(after.access);
        }
        match self.refresh(fetcher, &after).await {
            Ok(new) => {
                self.save(&new)?;
                Ok(new.access)
            }
            Err(e) if !after.access.trim().is_empty() => {
                // A transient refresh failure must not block a request while the
                // access token is still usable; surface it only when it is not.
                tracing::warn!(error = %e, "stockbit refresh failed; using the current access token");
                Ok(after.access)
            }
            Err(e) => Err(e),
        }
    }

    /// Exchanges the refresh token for a new pair. Does not write the file.
    pub async fn refresh(&self, fetcher: &Fetcher, session: &Session) -> Result<Session> {
        let refresh = session.refresh.trim();
        if refresh.is_empty() {
            return Err(IdxError::Usage(
                "no Stockbit refresh token; run `idx stockbit login`".into(),
            ));
        }
        let headers = super::request_headers(refresh);
        let resp = fetcher.post_uncached(REFRESH_URL, &headers, "{}").await?;
        if resp.status == 401 || resp.status == 403 {
            return Err(IdxError::provider(
                super::NAME,
                "refresh token rejected (expired or revoked); run `idx stockbit login` again",
            ));
        }
        if !(200..300).contains(&resp.status) {
            return Err(IdxError::provider(
                super::NAME,
                format!("refresh failed: HTTP {}", resp.status),
            ));
        }
        parse_refresh(&resp.body, session)
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
