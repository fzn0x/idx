//! `idx mcp`: serve over stdio (default) or HTTP, and `idx mcp setup` for
//! the registration recipe of every supported agent.

use clap::{Args, Subcommand};

use crate::error::{IdxError, Result};
use crate::mcp::clients::{self, Launcher};
use crate::output;

use super::{build_context, GlobalOpts};

#[derive(Debug, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub cmd: Option<McpCmd>,

    /// Serve MCP streamable HTTP on this address instead of stdio (e.g. 127.0.0.1:8765; port 0 picks one).
    #[arg(long, value_name = "ADDR")]
    pub http: Option<String>,

    /// Bearer token clients must send (required when binding a non-loopback address).
    #[arg(
        long,
        env = "IDX_MCP_TOKEN",
        hide_env_values = true,
        value_name = "TOKEN"
    )]
    pub token: Option<String>,

    /// Extra Host header values to accept with --http (a tunnel hostname, or * for any). Repeatable.
    #[arg(long = "allowed-host", value_name = "HOST")]
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum McpCmd {
    /// Print the registration command or config snippet for an agent, with this binary's absolute path filled in.
    #[command(
        after_help = "Examples:\n  idx mcp setup --list\n  idx mcp setup codex\n  idx mcp setup muse\n  idx mcp setup cursor --npx                       # launch through npx (npm package ihsg-mcp) instead of a path\n  idx mcp setup chatgpt --http https://x.trycloudflare.com/mcp --token s3cret\n  idx -o json mcp setup claude-code                # machine-readable steps"
    )]
    Setup {
        /// Client id or alias (see --list): claude-code, codex, muse, antigravity, gemini, copilot-cli, vscode, cursor, windsurf, kiro, jetbrains, zed, cline, continue, opencode, goose, warp, droid, vibe, qwen, crush, trae, augment, chatgpt.
        #[arg(value_name = "CLIENT", required_unless_present = "list")]
        client: Option<String>,
        /// List the supported clients.
        #[arg(long)]
        list: bool,
        /// Generate the remote (HTTP) form pointing at this URL instead of a local command.
        #[arg(long, value_name = "URL")]
        http: Option<String>,
        /// Bearer token to embed in the remote form.
        #[arg(long, value_name = "TOKEN", requires = "http")]
        token: Option<String>,
        /// Use `npx -y ihsg-mcp` (the npm MCP package; `cmd /c npx -y ihsg-mcp` on Windows) as the command instead of this binary's path.
        #[arg(long, conflicts_with = "http")]
        npx: bool,
    },
}

pub async fn run(global: &GlobalOpts, args: McpArgs) -> Result<()> {
    match args.cmd {
        Some(McpCmd::Setup {
            client,
            list,
            http,
            token,
            npx,
        }) => {
            if args.http.is_some() {
                return Err(IdxError::Usage(
                    "--http belongs to `idx mcp`; for a remote recipe use `idx mcp setup <client> --http URL`"
                        .into(),
                ));
            }
            let format = super::requested_format(global);
            if list {
                if output::print_value(format, &clients::CLIENTS)? {
                    return Ok(());
                }
                return output::print_stdout(&clients::list_text());
            }
            let id = client.unwrap_or_default();
            let Some(c) = clients::find(&id) else {
                return Err(IdxError::Usage(format!(
                    "unknown client {id:?}; run `idx mcp setup --list`"
                )));
            };
            let launcher = match (http, npx) {
                (Some(url), _) => Launcher::Http { url, token },
                (None, true) => Launcher::Npx,
                (None, false) => Launcher::Binary(current_binary()),
            };
            let s = clients::setup(c, &launcher);
            if output::print_value(format, &s)? {
                return Ok(());
            }
            output::print_stdout(&clients::render(&s))
        }
        None => {
            let ctx = build_context(global)?;
            let stockbit = crate::mcp::StockbitConfig {
                options: crate::providers::StockbitOptions {
                    session_path: ctx.settings.stockbit_session.clone(),
                    access_seed: ctx.settings.stockbit_access_seed.clone(),
                    refresh_seed: ctx.settings.stockbit_refresh_seed.clone(),
                },
                ttls: crate::providers::Ttls {
                    quote: ctx.settings.quote_ttl(),
                    history: ctx.settings.history_ttl(),
                    ..crate::providers::Ttls::default()
                },
            };
            match args.http {
                Some(bind) => {
                    crate::mcp::http::serve_http(
                        ctx.provider,
                        ctx.fetcher,
                        ctx.settings.ownership_db.clone(),
                        stockbit,
                        crate::mcp::http::HttpOptions {
                            bind,
                            token: args.token,
                            allowed_hosts: args.allowed_hosts,
                        },
                    )
                    .await
                }
                None => {
                    crate::mcp::serve(
                        ctx.provider,
                        ctx.fetcher,
                        ctx.settings.ownership_db.clone(),
                        stockbit,
                    )
                    .await
                }
            }
        }
    }
}

/// Absolute path of this executable, forward slashes kept as the OS gives them.
fn current_binary() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok().or(Some(p)))
        .map(|p| {
            let s = p.display().to_string();
            // canonicalize() on Windows yields \\?\C:\...; strip the prefix.
            s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
        })
        .unwrap_or_else(|| "idx".into())
}
