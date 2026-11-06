//! Error type shared by every layer, with a stable mapping to process exit codes.
//!
//! Exit codes are part of the public contract (scripts and agents rely on them):
//!
//! | code | meaning                                   |
//! |------|-------------------------------------------|
//! | 0    | success                                   |
//! | 1    | internal / unexpected error               |
//! | 2    | usage error (bad flags, bad symbol)       |
//! | 3    | network or provider failure (JSON code `network`, `provider`, or `blocked` when the source's bot protection refused this network) |
//! | 4    | symbol or record not found                |
//! | 5    | `--offline` and nothing usable in cache   |
//! | 6    | configuration error                       |

use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum IdxError {
    #[error("{0}")]
    Usage(String),
    #[error("config: {0}")]
    Config(String),
    #[error("network: {0}")]
    Network(String),
    #[error("{provider}: {message}")]
    Provider {
        provider: &'static str,
        message: String,
    },
    /// The source's bot protection (Cloudflare) refused this client: a
    /// challenge only a browser can pass, or a block page. Nothing is wrong
    /// with the request; a later run, another network, a mirror or a manual
    /// download can work. Exit 3 like any network failure, JSON code
    /// `blocked`.
    #[error("{provider}: {message}")]
    Blocked {
        provider: &'static str,
        refusal: crate::http::cloudflare::Refusal,
        message: String,
    },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("offline: {0}")]
    OfflineMiss(String),
    /// The selected provider cannot serve this command (another one may).
    #[error("{provider} does not provide {what}; use --provider auto")]
    Unsupported {
        provider: &'static str,
        what: &'static str,
    },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

impl IdxError {
    pub fn exit_code(&self) -> i32 {
        match self {
            IdxError::Usage(_) => 2,
            IdxError::Network(_) | IdxError::Provider { .. } | IdxError::Blocked { .. } => 3,
            IdxError::NotFound(_) => 4,
            IdxError::OfflineMiss(_) => 5,
            IdxError::Config(_) | IdxError::Unsupported { .. } => 6,
            IdxError::Io(_) | IdxError::Other(_) => 1,
        }
    }

    /// Short machine-readable code used in JSON error output.
    pub fn code(&self) -> &'static str {
        match self {
            IdxError::Usage(_) => "usage",
            IdxError::Config(_) => "config",
            IdxError::Network(_) => "network",
            IdxError::Provider { .. } => "provider",
            IdxError::Blocked { .. } => "blocked",
            IdxError::NotFound(_) => "not_found",
            IdxError::OfflineMiss(_) => "offline_miss",
            IdxError::Unsupported { .. } => "unsupported",
            IdxError::Io(_) => "io",
            IdxError::Other(_) => "internal",
        }
    }

    pub fn unsupported(provider: &'static str, what: &'static str) -> Self {
        IdxError::Unsupported { provider, what }
    }

    pub fn provider(provider: &'static str, message: impl fmt::Display) -> Self {
        IdxError::Provider {
            provider,
            message: message.to_string(),
        }
    }

    pub fn blocked(
        provider: &'static str,
        refusal: crate::http::cloudflare::Refusal,
        message: impl fmt::Display,
    ) -> Self {
        IdxError::Blocked {
            provider,
            refusal,
            message: message.to_string(),
        }
    }
}

impl From<serde_json::Error> for IdxError {
    fn from(e: serde_json::Error) -> Self {
        IdxError::Other(format!("invalid JSON: {e}"))
    }
}

impl From<reqwest::Error> for IdxError {
    /// reqwest's own message ("error sending request for url (...)") hides
    /// the cause; name a timeout as such and append the innermost cause.
    /// URLs lose their query string: it can hold a session crumb or a key.
    fn from(e: reqwest::Error) -> Self {
        let url = e.url().map(|u| u.as_str().to_string());
        if e.is_timeout() {
            return IdxError::Network(match url {
                Some(u) => format!("timed out waiting for {}", crate::http::redact_urls(&u)),
                None => "timed out".into(),
            });
        }
        let mut cause: &dyn std::error::Error = &e;
        while let Some(next) = cause.source() {
            cause = next;
        }
        let msg = e.to_string();
        let root = cause.to_string();
        let text = if msg.contains(&root) {
            msg
        } else {
            format!("{msg}: {root}")
        };
        IdxError::Network(crate::http::redact_urls(&text))
    }
}

pub type Result<T, E = IdxError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::cloudflare::Refusal;

    #[tokio::test]
    async fn network_errors_leave_the_query_string_out() {
        // An https-only client refuses an http URL before any connection.
        let e = reqwest::Client::builder()
            .https_only(true)
            .build()
            .unwrap()
            .get("http://x.test/v7/finance/quote?symbols=BBCA.JK&crumb=SECRET")
            .send()
            .await
            .unwrap_err();
        assert!(e.to_string().contains("crumb=SECRET"), "{e}");
        let ours = IdxError::from(e);
        assert_eq!(ours.exit_code(), 3);
        let text = ours.to_string();
        assert!(text.contains("http://x.test/v7/finance/quote"), "{text}");
        assert!(
            !text.contains("SECRET") && !text.contains("crumb"),
            "{text}"
        );
    }

    #[test]
    fn blocked_is_exit_3_with_its_own_code() {
        let e = IdxError::blocked(
            "idx",
            Refusal::Challenge,
            "www.idx.co.id answered with a Cloudflare challenge",
        );
        assert_eq!(e.exit_code(), 3);
        assert_eq!(e.code(), "blocked");
        assert_eq!(
            e.to_string(),
            "idx: www.idx.co.id answered with a Cloudflare challenge"
        );
    }
}
