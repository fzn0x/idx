//! `idx stockbit ...`: manage the realtime Stockbit session and read the live
//! order book. Everything here uses the Stockbit provider regardless of the
//! configured default, since it is the only source with a live book.
//!
//! Once logged in, the other commands use Stockbit with `--provider stockbit`
//! (or `idx config set provider stockbit`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Subcommand;

use crate::error::{IdxError, Result};
use crate::model::Symbol;
use crate::output::json::SCHEMA_ORDERBOOK;
use crate::output::{self, table};
use crate::providers::stockbit::{login, SessionStore, Stockbit};
use crate::providers::Ttls;

use super::{emit, Context};

#[derive(Debug, Subcommand)]
pub enum StockbitCmd {
    /// Sign in to Stockbit in your browser and store the session (auto-captured).
    #[command(
        after_help = "Examples:\n  idx stockbit login                      # opens Chrome/Edge; sign in, idx captures the session\n  idx stockbit login --browser /path/to/chrome\n  idx stockbit login --manual             # paste the credentialStorage cookie instead\n\nThe token is your own Stockbit session. idx stores it locally and refreshes it\nfor you; it is never sent anywhere but Stockbit. Because Stockbit rotates the\nrefresh token on every refresh, do not share one session between idx and another\ntool (they would log each other out)."
    )]
    Login {
        /// Paste the token yourself instead of driving a browser (any browser).
        #[arg(long)]
        manual: bool,
        /// Browser executable (default: the first Chrome/Edge/Brave found; env IDX_BROWSER).
        #[arg(long, value_name = "PATH")]
        browser: Option<PathBuf>,
        /// Persistent browser profile directory, so a second login need not re-type.
        #[arg(long, value_name = "DIR")]
        profile: Option<PathBuf>,
        /// Seconds to wait for the login before giving up.
        #[arg(long, default_value_t = 300)]
        timeout: u64,
    },
    /// Show whether a session is stored and when its tokens expire (no token values).
    Status,
    /// Delete the stored session.
    Logout,
    /// Live order book (depth ladder) and foreign flow for one ticker.
    #[command(
        after_help = "Examples:\n  idx stockbit orderbook BBCA\n  idx stockbit orderbook BBRI --depth 5\n  idx -o json stockbit orderbook TLKM | jq '.data.bids[0]'\n\nRealtime via your Stockbit session. Sizes are shares; values IDR."
    )]
    Orderbook {
        /// IDX ticker, e.g. BBCA.
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Price levels per side to show (0 shows all).
        #[arg(long, short = 'd', default_value_t = 10)]
        depth: usize,
    },
}

fn store(ctx: &Context) -> Arc<SessionStore> {
    Arc::new(SessionStore::new(
        ctx.settings.stockbit_session.clone(),
        ctx.settings.stockbit_access_seed.clone(),
        ctx.settings.stockbit_refresh_seed.clone(),
    ))
}

fn provider(ctx: &Context, store: Arc<SessionStore>) -> Stockbit {
    let ttls = Ttls {
        quote: ctx.settings.quote_ttl(),
        history: ctx.settings.history_ttl(),
        ..Ttls::default()
    };
    Stockbit::new(ctx.fetcher.clone(), ttls, store)
}

pub async fn run(ctx: &Context, cmd: StockbitCmd) -> Result<()> {
    match cmd {
        StockbitCmd::Login {
            manual,
            browser,
            profile,
            timeout,
        } => login_cmd(ctx, manual, browser, profile, timeout).await,
        StockbitCmd::Status => status_cmd(ctx),
        StockbitCmd::Logout => {
            let removed = store(ctx).clear()?;
            let v = serde_json::json!({ "logged_out": removed });
            if output::print_value(ctx.settings.output, &v)? {
                Ok(())
            } else if removed {
                output::print_stdout("Logged out; the stored Stockbit session was deleted.\n")
            } else {
                output::print_stdout("No stored Stockbit session.\n")
            }
        }
        StockbitCmd::Orderbook { symbol, depth } => {
            let sym = Symbol::parse(&symbol)?;
            let sb = provider(ctx, store(ctx));
            let result = sb.orderbook(&sym).await?;
            emit(ctx, SCHEMA_ORDERBOOK, result, |d| {
                table::orderbook(d, depth, ctx.color)
            })
        }
    }
}

fn status_cmd(ctx: &Context) -> Result<()> {
    let status = store(ctx).status(chrono::Utc::now().timestamp());
    if output::print_value(ctx.settings.output, &status)? {
        return Ok(());
    }
    let dur = |secs: Option<i64>| match secs {
        Some(s) if s <= 0 => "expired".to_string(),
        Some(s) => format!("in {}h {}m", s / 3600, (s % 3600) / 60),
        None => "unknown".to_string(),
    };
    let mut out = String::new();
    out.push_str(&format!("session file : {}\n", status.session_file));
    if status.logged_in {
        out.push_str("logged in    : yes\n");
        out.push_str(&format!(
            "access token : {} (expires {})\n",
            if status.access.present {
                "present"
            } else {
                "missing"
            },
            dur(status.access.expires_in_secs)
        ));
        out.push_str(&format!(
            "refresh token: present (expires {})\n",
            dur(status.refresh.expires_in_secs)
        ));
    } else {
        out.push_str("logged in    : no — run `idx stockbit login`\n");
    }
    output::print_stdout(&out)
}

async fn login_cmd(
    ctx: &Context,
    manual: bool,
    browser: Option<PathBuf>,
    profile: Option<PathBuf>,
    timeout: u64,
) -> Result<()> {
    let store = store(ctx);
    if manual {
        return manual_login(ctx, &store);
    }

    let profile_dir = profile.unwrap_or_else(|| {
        ctx.settings
            .stockbit_session
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("stockbit-chrome-profile")
    });
    output::print_stderr(
        "Opening a browser for Stockbit. Sign in there; idx will capture the session automatically.",
    );
    let opts = login::LoginOptions {
        browser,
        profile_dir,
        timeout: Duration::from_secs(timeout),
    };
    let (session, how) = login::capture(&opts).await?;
    store.save(&session)?;
    let note = match how {
        login::Captured::Login => "captured your sign-in",
        login::Captured::AlreadySignedIn => "reused your already-signed-in browser session",
    };
    output::print_notice(&format!("Logged in: {note}."));
    status_cmd(ctx)
}

/// Read the `credentialStorage` cookie (or a bare refresh token) from stdin.
fn manual_login(ctx: &Context, store: &SessionStore) -> Result<()> {
    output::print_stderr(
        "Manual login:\n  1. Open https://stockbit.com and sign in.\n  2. DevTools (F12) → Application → Cookies → https://stockbit.com\n  3. Copy the value of the `credentialStorage` cookie.\n  4. Paste it below and press Enter.\n",
    );
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| IdxError::Usage(format!("could not read input: {e}")))?;
    let session = crate::providers::stockbit::auth::session_from_credential_blob(&line)
        .filter(|s| !s.refresh.trim().is_empty())
        .ok_or_else(|| {
            IdxError::Usage(
                "no refresh token found in that value; paste the whole credentialStorage cookie"
                    .into(),
            )
        })?;
    store.save(&session)?;
    output::print_notice("Saved the Stockbit session.");
    status_cmd(ctx)
}
