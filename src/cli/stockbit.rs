//! `idx stockbit ...`: manage the Stockbit session and read what only the
//! session reads: the live order book, corporate actions and the calendar,
//! insider and 5 % holder movements. Everything here uses the Stockbit
//! provider regardless of the configured default.
//!
//! Once logged in, the other commands use Stockbit with `--provider stockbit`
//! (or `idx config set provider stockbit`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use clap::Subcommand;

use crate::error::{IdxError, Result};
use crate::model::{CorporateEventKind, MovementFilter, MovementSource, Symbol};
use crate::output::json::{SCHEMA_ORDERBOOK, SCHEMA_STOCKBIT_ACTIONS, SCHEMA_STOCKBIT_MOVEMENTS};
use crate::output::{self, table, table_corporate, table_movements};
use crate::providers::stockbit::movement::MovementQuery;
use crate::providers::stockbit::signin::Accepted;
use crate::providers::stockbit::{login, SessionStore, Stockbit};
use crate::providers::{StockbitOptions, Ttls};
use crate::service;
use crate::universe;

use super::{emit, Context};

#[derive(Debug, Subcommand)]
pub enum StockbitCmd {
    /// Sign in to Stockbit and store the session: in a browser idx opens, or by pasting the session cookie.
    #[command(
        after_help = "Examples:\n  idx stockbit login                       # opens Chrome/Edge/Brave on Stockbit's sign-in page\n  idx stockbit login --browser /path/to/chrome\n  pbpaste | ssh you@host idx stockbit login   # or pipe the credentialStorage cookie\n  idx stockbit login --manual              # paste it into this terminal\n\nidx never asks for your Stockbit password: you sign in on stockbit.com and idx takes only the\nsession (the credentialStorage cookie), checks it with one request and stores it (0600); a\nsession Stockbit refuses leaves the previous one in place. When the stored session still works,\nlogin says so and stops (--force signs in anyway). Stockbit rotates the refresh token on every\nrefresh, so never share one session with another tool or browser (they would log each other out)."
    )]
    Login {
        /// Paste the session cookie into this terminal (also what happens when stdin is a pipe).
        #[arg(long, conflicts_with = "browser")]
        manual: bool,
        /// Browser executable (default: the first Chrome/Edge/Brave found; env IDX_BROWSER).
        #[arg(long, value_name = "PATH")]
        browser: Option<PathBuf>,
        /// Browser profile only idx uses (keeps Stockbit's device cookies, not the session).
        #[arg(long, value_name = "DIR")]
        profile: Option<PathBuf>,
        /// Seconds to wait for the sign-in.
        #[arg(long, default_value_t = 300)]
        timeout: u64,
        /// Sign in again even when the stored session still works.
        #[arg(long)]
        force: bool,
    },
    /// Show whether a session is stored and when its tokens expire (no token values).
    Status,
    /// Delete the stored session.
    Logout,
    /// Live order book (depth ladder) and foreign flow for one ticker.
    #[command(
        after_help = "Examples:\n  idx stockbit orderbook BBCA\n  idx stockbit orderbook BBRI --depth 5\n  idx -o json stockbit orderbook TLKM | jq '.data.bids[0]'\n\nRealtime via your Stockbit session. Sizes are shares; values IDR.\nForeign buy/sell/net stay empty, with a warning, while Stockbit marks them as\nnot current (it does during trading, when they are the previous session's)."
    )]
    Orderbook {
        /// IDX ticker, e.g. BBCA.
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Price levels per side to show (0 shows all).
        #[arg(long, short = 'd', default_value_t = 10)]
        depth: usize,
    },
    /// One stock's corporate actions from your Stockbit session: dividends, bonus shares, rights, warrants, splits, RUPS, public exposes, tender offers.
    #[command(
        after_help = "Examples:\n  idx stockbit actions BBCA\n  idx stockbit actions TLKM --kind dividend\n  idx stockbit actions BBRI --kind split,bonus,rights --from 2015-01-01\n  idx -o json stockbit actions BBCA --kind dividend | jq '.data.actions[] | {cum_date, dividend_per_share}'\n\nKinds: dividend, stock_dividend, bonus, rights, warrant, split, reverse_split, rups (meeting),\npublic_expose (expose), ipo, tender_offer (tender), or all. Default: every kind but ipo.\nStockbit keeps a stock's whole history (BBCA's cash dividends go back to 2009). One request per\nkind, a second apart, cached 6 h. Needs `idx stockbit login`. IPOT's view with the announcement\ntexts, no login: `idx stocks actions`."
    )]
    Actions {
        /// IDX ticker, e.g. BBCA.
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Kinds, comma separated (see below), or all.
        #[arg(long = "kind", short = 'k', value_name = "KINDS")]
        kinds: Option<String>,
        /// Only actions dated from this day (YYYY-MM-DD).
        #[arg(long, value_name = "DATE")]
        from: Option<NaiveDate>,
        /// Only actions dated up to this day (YYYY-MM-DD).
        #[arg(long, value_name = "DATE")]
        to: Option<NaiveDate>,
    },
    /// Corporate actions across IDX from your Stockbit session, in a date range (default: the next 14 days).
    #[command(
        after_help = "Examples:\n  idx stockbit calendar\n  idx stockbit calendar --kind dividend --days 30\n  idx stockbit calendar --kind ipo,tender_offer --from 2026-01-01 --to 2026-12-31\n  idx -o json stockbit calendar --kind dividend | jq '.data.actions[] | {symbol, cum_date, dividend_per_share}'\n\nA row's date is its cum date for dividends, bonuses, rights and splits (the ex date without one),\nelse the event's first day; rights trading, tender offers and IPO offers still open in the range\nare listed too. Default kinds: all but warrant (structured warrants list dozens a month; `--kind\nall` adds them). A range covers at most 366 days. One request per kind, a second apart, cached\n6 h. Needs `idx stockbit login`. The exchange's own calendar, no login: `idx market calendar`."
    )]
    Calendar {
        /// First day (YYYY-MM-DD); default today.
        #[arg(long, value_name = "DATE")]
        from: Option<NaiveDate>,
        /// Last day (YYYY-MM-DD); default --from plus --days.
        #[arg(long, value_name = "DATE", conflicts_with = "days")]
        to: Option<NaiveDate>,
        /// Days ahead of --from when --to is not given.
        #[arg(long, value_name = "N")]
        days: Option<u32>,
        /// Kinds, comma separated, or all (default: all but warrant).
        #[arg(long = "kind", short = 'k', value_name = "KINDS")]
        kinds: Option<String>,
    },
    /// Insider and 5 % holder movements from your Stockbit session: IDX insider disclosures and KSEI's 5 % holder changes.
    #[command(
        after_help = "Examples:\n  idx stockbit movements BBCA\n  idx stockbit movements --source ksei --action sell\n  idx stockbit movements --from 2026-10-01 --to 2026-10-31 --pages 5\n  idx stockbit movements --holder 1000004481        # one holder (HOLDER ID column)\n  idx -o json stockbit movements BBCA | jq '.data.movements[] | {date, holder, action, shares_change, price}'\n\nReported changes in a holder's stake, newest first: IDX insider disclosures (directors,\ncommissioners and other insiders, any size, with the trade price) and KSEI's daily changes of\nholders at 5 % and above (no price filed; Stockbit's estimate shows as ~price and stays apart in\nJSON as estimated_price). Without SYMBOL, the whole market. 50 rows a page, one request each, a\nsecond apart, cached 1 h. Filings, not signals: a share plan reads as many insiders buying at one\nprice, and rows appear days after the change. Needs `idx stockbit login`. The monthly register of\nholders above 1 %, no login: `idx ownership`."
    )]
    Movements {
        /// IDX ticker, e.g. BBCA; omit it for the whole market.
        #[arg(value_name = "SYMBOL")]
        symbol: Option<String>,
        /// Only changes from this day (YYYY-MM-DD).
        #[arg(long, value_name = "DATE")]
        from: Option<NaiveDate>,
        /// Only changes up to this day (YYYY-MM-DD).
        #[arg(long, value_name = "DATE")]
        to: Option<NaiveDate>,
        /// One holder, by the id in the HOLDER ID column (digits).
        #[arg(long, value_name = "ID")]
        holder: Option<String>,
        /// Only one kind of change.
        #[arg(long, value_enum)]
        action: Option<MovementFilter>,
        /// Only IDX insider disclosures or KSEI's 5 % holder changes.
        #[arg(long, value_enum)]
        source: Option<MovementSource>,
        /// Pages of 50 rows to read (1-10).
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=10))]
        pages: u32,
    },
}

impl StockbitCmd {
    /// Commands that only read data, and so show the loading line; the
    /// session commands print their own prompts.
    pub fn shows_progress(&self) -> bool {
        matches!(
            self,
            StockbitCmd::Orderbook { .. }
                | StockbitCmd::Actions { .. }
                | StockbitCmd::Calendar { .. }
                | StockbitCmd::Movements { .. }
        )
    }
}

/// The process's one store for the session file (shared with the provider
/// `--provider stockbit` builds), so refreshes queue on one lock.
fn store(ctx: &Context) -> Arc<SessionStore> {
    StockbitOptions {
        session_path: ctx.settings.stockbit_session.clone(),
        access_seed: ctx.settings.stockbit_access_seed.clone(),
        refresh_seed: ctx.settings.stockbit_refresh_seed.clone(),
    }
    .store()
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
            force,
        } => login_cmd(ctx, manual, browser, profile, timeout, force).await,
        StockbitCmd::Status => status_cmd(ctx),
        StockbitCmd::Logout => {
            let removed = store(ctx).clear()?;
            if ctx.json() {
                output::print_stdout(&output::json::to_pretty(
                    &serde_json::json!({ "logged_out": removed }),
                )?)
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
        StockbitCmd::Actions {
            symbol,
            kinds,
            from,
            to,
        } => {
            let sym = Symbol::parse(&symbol)?;
            let kinds = CorporateEventKind::parse_list(
                kinds.as_deref(),
                &CorporateEventKind::STOCK_DEFAULT,
            )?;
            let list = universe::load(None).unwrap_or_default();
            let sb = provider(ctx, store(ctx));
            let result = service::stockbit_actions(&sb, &list, &sym, &kinds, from, to).await?;
            emit(ctx, SCHEMA_STOCKBIT_ACTIONS, result, |d| {
                table_corporate::corporate_events(d, ctx.color)
            })
        }
        StockbitCmd::Calendar {
            from,
            to,
            days,
            kinds,
        } => {
            let q = service::StockbitCalendarQuery {
                from,
                to,
                days,
                kinds: CorporateEventKind::parse_list(
                    kinds.as_deref(),
                    &CorporateEventKind::CALENDAR_DEFAULT,
                )?,
            };
            let sb = provider(ctx, store(ctx));
            let result = service::stockbit_calendar(&sb, &q, service::today_wib()).await?;
            emit(ctx, SCHEMA_STOCKBIT_ACTIONS, result, |d| {
                table_corporate::corporate_events(d, ctx.color)
            })
        }
        StockbitCmd::Movements {
            symbol,
            from,
            to,
            holder,
            action,
            source,
            pages,
        } => {
            let q = MovementQuery {
                symbol: symbol.as_deref().map(Symbol::parse).transpose()?,
                from,
                to,
                holder_id: holder.map(|h| h.trim().to_string()),
                action,
                source,
            };
            q.validate()?;
            let sb = provider(ctx, store(ctx));
            let result = service::stockbit_movements(&sb, &q, pages).await?;
            emit(ctx, SCHEMA_STOCKBIT_MOVEMENTS, result, |d| {
                table_movements::holder_movements(d, ctx.color)
            })
        }
    }
}

fn status_cmd(ctx: &Context) -> Result<()> {
    let status = store(ctx).status(chrono::Utc::now().timestamp());
    if ctx.json() {
        return output::print_stdout(&output::json::to_pretty(&status)?);
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
        out.push_str("logged in    : no; run `idx stockbit login`\n");
    }
    output::print_stdout(&out)
}

/// How `idx stockbit login` takes the session in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// From stdin: `--manual`, or stdin is not a terminal (a pipe).
    Paste,
    /// A browser idx launches on this machine (`None`: the first one found).
    Browser(Option<PathBuf>),
}

/// The sign-in method for the flags given; pure, so it is tested.
pub fn choose_method(manual: bool, browser: Option<PathBuf>, stdin_is_tty: bool) -> Method {
    if manual || (!stdin_is_tty && browser.is_none()) {
        Method::Paste
    } else {
        Method::Browser(browser)
    }
}

const PASTE_HELP: &str = "Paste stockbit.com's credentialStorage cookie and press Enter.
  1. Sign in at https://stockbit.com (a private window keeps your everyday browser's session apart).
  2. DevTools (F12) > Application > Cookies > https://stockbit.com
  3. Copy the value of credentialStorage.
idx checks it with Stockbit before storing it, and never asks for your password.";

/// The session the user pastes: one line from a terminal, else all of stdin.
fn read_paste(stdin_is_tty: bool) -> Result<String> {
    use std::io::Read;
    let mut raw = String::new();
    if stdin_is_tty {
        output::print_stderr(PASTE_HELP);
        std::io::stdin().read_line(&mut raw)
    } else {
        std::io::stdin().read_to_string(&mut raw)
    }
    .map_err(|e| IdxError::Usage(format!("could not read the session from stdin: {e}")))?;
    if raw.trim().is_empty() {
        return Err(IdxError::Usage(
            "nothing on stdin: run `idx stockbit login` in a terminal, or pipe the credentialStorage cookie (pbpaste | ssh host idx stockbit login)".into(),
        ));
    }
    Ok(raw)
}

/// What to print once a session is taken.
fn report(ctx: &Context, acc: &Accepted) -> Result<()> {
    if acc.unchanged {
        output::print_stderr("That session is already stored and works; nothing changed.");
    } else {
        output::print_stderr("Signed in: Stockbit accepted the session and idx stored it.");
    }
    if let Some(w) = &acc.warning {
        output::print_stderr(&format!("warning: {w}"));
    }
    status_cmd(ctx)
}

/// `true` (after saying so) when the stored session still works, so a
/// sign-in would only replace a good session.
async fn already_signed_in(ctx: &Context, sb: &Stockbit) -> Result<bool> {
    if !sb.store().status(chrono::Utc::now().timestamp()).logged_in {
        return Ok(false);
    }
    if sb.verify().await.is_err() {
        return Ok(false);
    }
    output::print_stderr(
        "Already signed in to Stockbit; run `idx stockbit logout` to switch accounts, or pass --force.",
    );
    status_cmd(ctx).map(|_| true)
}

/// The browser profile idx's sign-in window uses: beside the session file.
fn default_profile(session: &std::path::Path) -> PathBuf {
    session
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("stockbit-chrome-profile")
}

async fn login_cmd(
    ctx: &Context,
    manual: bool,
    browser: Option<PathBuf>,
    profile: Option<PathBuf>,
    timeout: u64,
    force: bool,
) -> Result<()> {
    use std::io::IsTerminal;
    let sb = Arc::new(provider(ctx, store(ctx)));
    let stdin_is_tty = std::io::stdin().is_terminal();
    match choose_method(manual, browser, stdin_is_tty) {
        Method::Paste => {
            let raw = read_paste(stdin_is_tty)?;
            let acc = sb.accept_session(&raw).await?;
            report(ctx, &acc)
        }
        Method::Browser(browser) => {
            if ctx.fetcher.is_offline() {
                return Err(IdxError::OfflineMiss(
                    "`idx stockbit login` checks the session with Stockbit, so it needs the network"
                        .into(),
                ));
            }
            if !force && already_signed_in(ctx, &sb).await? {
                return Ok(());
            }
            let opts = login::LoginOptions {
                browser,
                profile_dir: profile
                    .unwrap_or_else(|| default_profile(&ctx.settings.stockbit_session)),
                timeout: Duration::from_secs(timeout),
            };
            output::print_stderr(
                "Opening a browser on Stockbit's sign-in page. Sign in there; idx takes the session once Stockbit accepts it.",
            );
            let say = |s: &str| output::print_stderr(s);
            let acc = login::capture(
                &opts,
                |v: String| {
                    let sb = sb.clone();
                    async move { sb.accept_session(&v).await }
                },
                &say,
            )
            .await?;
            report(ctx, &acc)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sign_in_method_follows_the_flags_and_stdin() {
        let chrome = Some(PathBuf::from("/opt/chrome"));
        assert_eq!(choose_method(true, None, true), Method::Paste);
        assert_eq!(choose_method(false, None, false), Method::Paste);
        assert_eq!(choose_method(false, None, true), Method::Browser(None));
        assert_eq!(
            choose_method(false, chrome.clone(), false),
            Method::Browser(chrome.clone())
        );
        assert_eq!(
            choose_method(false, chrome.clone(), true),
            Method::Browser(chrome)
        );
    }
}
