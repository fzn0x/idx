//! Taking a Stockbit session in. Every way in (idx's browser window, the
//! local page, a paste on stdin) ends in [`Stockbit::accept_session`]: read
//! the two tokens out of what the user sent, store them (0600), check them
//! with one authorised request, and put the previous session back when
//! Stockbit refuses them. The value is never logged, echoed or put in an
//! error message, and idx never sees the account's password: the user signs
//! in on stockbit.com, and only the `credentialStorage` cookie that sign-in
//! leaves behind (or a bare refresh token) reaches idx.

use crate::error::IdxError;

use super::auth::{self, session_from_credential_blob, RefreshError, Status};
use super::{parse, rejected, request_headers, Stockbit, CDN_BLOCKED, EXODUS, NAME};

/// The authorised GET that proves a session works: small, read-only, and
/// the one Stockbit's own app polls.
fn session_url() -> String {
    format!("{EXODUS}/company-price-feed/market-time/session")
}

/// Why a session check did not pass.
#[derive(Debug)]
pub enum SessionCheck {
    /// Stockbit refused the session: the access token after a refresh, or
    /// the refresh token itself. Sending it again cannot help.
    Refused(String),
    /// The check could not be made (network, offline, CDN block, an odd
    /// answer); the session may be fine.
    Failed(IdxError),
}

/// Why a pasted session was not taken.
#[derive(Debug)]
pub enum AcceptError {
    /// Not a `credentialStorage` cookie or a refresh token.
    NotASession,
    /// Stockbit refused it; the previous session is back in place.
    Refused(String),
    /// It could not be checked; the previous session is back in place.
    Failed(IdxError),
}

impl From<AcceptError> for IdxError {
    fn from(e: AcceptError) -> Self {
        match e {
            AcceptError::NotASession => IdxError::Usage(
                "not a Stockbit session: paste the value of stockbit.com's credentialStorage cookie, or a refresh token"
                    .into(),
            ),
            AcceptError::Refused(why) => IdxError::provider(
                NAME,
                format!("Stockbit refused this session ({why}); sign in again and send the new one"),
            ),
            AcceptError::Failed(e) => e,
        }
    }
}

/// A session idx took.
#[derive(Debug, Clone)]
pub struct Accepted {
    pub status: Status,
    /// The same session was already stored and works; nothing was written.
    pub unchanged: bool,
    /// Stored although the check that followed a refresh failed (the pasted
    /// refresh token was already spent, so the new pair is kept).
    pub warning: Option<String>,
}

fn refused(e: RefreshError) -> SessionCheck {
    match e {
        RefreshError::Rejected => {
            SessionCheck::Refused("Stockbit rejected the refresh token".into())
        }
        RefreshError::Failed(e) => SessionCheck::Failed(e),
    }
}

impl Stockbit {
    /// One authorised GET (never answered from the cache) that proves the
    /// stored session works, refreshing it first when the access token is
    /// missing or near expiry, and once more when Stockbit refuses it.
    pub async fn verify(&self) -> std::result::Result<(), SessionCheck> {
        let now = chrono::Utc::now().timestamp();
        let mut s = self.store().load();
        if s.is_empty() {
            return Err(SessionCheck::Refused("no session is stored".into()));
        }
        let mut rotated = false;
        if !auth::fresh(&s.access, now) {
            s = self
                .store()
                .rotate(self.fetcher(), &s, now)
                .await
                .map_err(refused)?;
            rotated = true;
        }
        loop {
            let resp = self
                .fetcher()
                .get_uncached(&session_url(), &request_headers(&s.access))
                .await
                .map_err(SessionCheck::Failed)?;
            if (200..300).contains(&resp.status) {
                return Ok(());
            }
            if rejected(resp.status, &resp.body) {
                if rotated {
                    return Err(SessionCheck::Refused(format!(
                        "HTTP {} for a freshly refreshed session",
                        resp.status
                    )));
                }
                s = self
                    .store()
                    .rotate(self.fetcher(), &s, now)
                    .await
                    .map_err(refused)?;
                rotated = true;
                continue;
            }
            if resp.status == 403 {
                return Err(SessionCheck::Failed(IdxError::provider(NAME, CDN_BLOCKED)));
            }
            return Err(SessionCheck::Failed(IdxError::provider(
                NAME,
                format!(
                    "session check HTTP {}: {}",
                    resp.status,
                    parse::error_message(&resp.body)
                ),
            )));
        }
    }

    /// Checks a session the user sent (stockbit.com's `credentialStorage`
    /// cookie, URL-encoded or not, or a bare refresh token) and stores it.
    /// When Stockbit refuses it, or it cannot be checked, the previous
    /// session file is put back (or removed when there was none).
    pub async fn accept_session(&self, raw: &str) -> std::result::Result<Accepted, AcceptError> {
        let new = session_from_credential_blob(raw)
            .filter(|s| !s.refresh.trim().is_empty())
            .ok_or(AcceptError::NotASession)?;
        let status = |s: &Stockbit| s.store().status(chrono::Utc::now().timestamp());
        let current = self.store().load();
        if current.refresh.trim() == new.refresh.trim() && self.verify().await.is_ok() {
            return Ok(Accepted {
                status: status(self),
                unchanged: true,
                warning: None,
            });
        }
        let previous = self.store().read_raw();
        self.store().save(&new).map_err(AcceptError::Failed)?;
        let saved = self.store().read_raw();
        let restore = |s: &Stockbit| {
            if let Err(e) = s.store().restore_raw(previous.as_deref()) {
                tracing::warn!(error = %e, "could not put the previous Stockbit session back");
            }
        };
        match self.verify().await {
            Ok(()) => Ok(Accepted {
                status: status(self),
                unchanged: false,
                warning: None,
            }),
            Err(SessionCheck::Refused(why)) => {
                restore(self);
                Err(AcceptError::Refused(why))
            }
            Err(SessionCheck::Failed(e)) if self.store().read_raw() != saved => Ok(Accepted {
                status: status(self),
                unchanged: false,
                warning: Some(format!(
                    "stored the session Stockbit issued, but the check after the refresh failed ({e})"
                )),
            }),
            Err(SessionCheck::Failed(e)) => {
                restore(self);
                Err(AcceptError::Failed(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::http::{CacheMode, Fetcher, FixtureBackend};
    use crate::providers::stockbit::SessionStore;
    use crate::providers::Ttls;
    use std::sync::Arc;

    const COOKIE: &str =
        include_str!("../../../tests/fixtures/stockbit/credential_storage_cookie_constructed.txt");
    const REFRESHED: &str =
        include_str!("../../../tests/fixtures/stockbit/login_refresh_constructed.json");
    const SESSION: &str = include_str!("../../../tests/fixtures/stockbit/market_session.json");
    const UNAUTHORIZED: &str = include_str!("../../../tests/fixtures/stockbit/unauthorized.json");
    /// Fresh until 2100; signed by nobody.
    const FRESH: &str = "eyJhbGciOiJub25lIn0.eyJleHAiOjQxMDI0NDQ4MDB9.sig";

    fn provider(
        be: FixtureBackend,
        mode: CacheMode,
    ) -> (Stockbit, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(be);
        let fetcher = Arc::new(Fetcher::new(
            be.clone(),
            Cache::new(dir.path().join("cache")),
            mode,
        ));
        let store = Arc::new(SessionStore::new(
            dir.path().join("session.json"),
            None,
            None,
        ));
        (Stockbit::new(fetcher, Ttls::default(), store), be, dir)
    }

    /// A paste whose access token is fresh, so no refresh is needed.
    fn fresh_paste(refresh: &str) -> String {
        format!(
            r#"{{"state":{{"access":{{"token":"{FRESH}"}},"refresh":{{"token":"{refresh}"}}}}}}"#
        )
    }

    #[tokio::test]
    async fn a_good_paste_is_stored_and_checked() {
        let (sb, be, _d) = provider(
            FixtureBackend::new().route("market-time/session", 200, SESSION),
            CacheMode::Normal,
        );
        let got = sb
            .accept_session(&fresh_paste("eyJ0.eyJleHAiOjF9.new"))
            .await
            .unwrap();
        assert!(!got.unchanged && got.warning.is_none());
        assert!(got.status.logged_in);
        assert_eq!(sb.store().load().refresh, "eyJ0.eyJleHAiOjF9.new");
        assert_eq!(be.calls_matching("market-time/session"), 1);
        assert_eq!(be.calls_matching("login/refresh"), 0);
        // The same session again: nothing written.
        let again = sb
            .accept_session(&fresh_paste("eyJ0.eyJleHAiOjF9.new"))
            .await
            .unwrap();
        assert!(again.unchanged);
    }

    #[tokio::test]
    async fn a_refused_paste_puts_the_previous_session_back() {
        let (sb, be, dir) = provider(
            FixtureBackend::new()
                .route("market-time/session", 401, UNAUTHORIZED)
                .route("login/refresh", 401, UNAUTHORIZED),
            CacheMode::Normal,
        );
        // No session before: none after.
        let e = sb.accept_session(COOKIE).await.unwrap_err();
        assert!(matches!(e, AcceptError::Refused(_)), "{e:?}");
        assert!(!dir.path().join("session.json").exists());
        let msg = IdxError::from(e).to_string();
        assert!(msg.contains("Stockbit refused this session"), "{msg}");
        assert!(!msg.contains("eyJ"), "{msg}");
        // A previous session: its exact bytes come back.
        let before = br#"{"access":"a","refresh":"previous"}"#;
        std::fs::write(dir.path().join("session.json"), before).unwrap();
        let e = sb
            .accept_session(&fresh_paste("eyJ0.eyJleHAiOjF9.other"))
            .await
            .unwrap_err();
        assert!(matches!(e, AcceptError::Refused(_)), "{e:?}");
        assert_eq!(
            std::fs::read(dir.path().join("session.json")).unwrap(),
            before
        );
        assert!(be.calls_matching("login/refresh") >= 1);
    }

    #[tokio::test]
    async fn a_refresh_token_alone_is_refreshed_then_kept_when_the_check_fails() {
        // The bare refresh token is spent on the refresh, so the new pair is
        // kept even though the GET that follows fails.
        let (sb, be, _d) = provider(
            FixtureBackend::new().route("login/refresh", 200, REFRESHED),
            CacheMode::Normal,
        );
        let got = sb.accept_session("eyJ0.eyJleHAiOjF9.only").await.unwrap();
        assert!(got
            .warning
            .as_deref()
            .is_some_and(|w| w.contains("check after the refresh failed")));
        assert_ne!(sb.store().load().refresh, "eyJ0.eyJleHAiOjF9.only");
        assert_eq!(be.calls_matching("login/refresh"), 1);
    }

    #[tokio::test]
    async fn garbage_and_offline_store_nothing() {
        let (sb, be, dir) = provider(FixtureBackend::new(), CacheMode::Normal);
        for junk in ["", "hello", "%7B%22state%22%3A%7B%7D%7D", "eyJ0.only-two"] {
            let e = sb.accept_session(junk).await.unwrap_err();
            assert!(matches!(e, AcceptError::NotASession), "{junk}");
            assert_eq!(IdxError::from(e).exit_code(), 2);
        }
        assert_eq!(be.call_count(), 0);
        assert!(!dir.path().join("session.json").exists());

        let (sb, _be, dir) = provider(FixtureBackend::new(), CacheMode::Offline);
        let e = sb.accept_session(COOKIE).await.unwrap_err();
        assert_eq!(IdxError::from(e).exit_code(), 5);
        assert!(!dir.path().join("session.json").exists());
    }

    #[tokio::test]
    async fn a_cdn_block_is_a_failed_check_not_a_refusal() {
        let blocked = include_str!("../../../tests/fixtures/stockbit/blocked_403.html");
        let (sb, be, dir) = provider(
            FixtureBackend::new().route("market-time/session", 403, blocked),
            CacheMode::Normal,
        );
        let e = sb
            .accept_session(&fresh_paste("eyJ0.eyJleHAiOjF9.x"))
            .await
            .unwrap_err();
        assert!(
            matches!(&e, AcceptError::Failed(f) if f.to_string().contains("CDN")),
            "{e:?}"
        );
        assert!(!dir.path().join("session.json").exists());
        assert_eq!(be.calls_matching("login/refresh"), 0);
    }
}
