//! Command-line surface. Each subcommand module owns its argument structs and
//! its handler; this module wires global flags into [`Context`].

use std::path::PathBuf;
use std::sync::Arc;

use clap::{CommandFactory, Parser, Subcommand};

use crate::cache::Cache;
use crate::config::{self, CliOverrides, FileConfig, Settings};
use crate::error::{IdxError, Result};
use crate::http::{Fetcher, ReqwestBackend};
use crate::output::{self, ColorMode, OutputFormat};
use crate::progress;
use crate::providers::{
    self, Provider, ProviderKind, ProviderOptions, Sourced, StockbitOptions, Ttls,
};

pub mod bandar;
pub mod broker;
pub mod cache_cmd;
pub mod config_cmd;
pub mod market;
pub mod mcp_cmd;
pub mod ownership;
pub mod stockbit;
pub mod stocks;
pub mod watch;

const LONG_ABOUT: &str = "\
Indonesian stock market (IDX / IHSG) analysis for humans and AI agents.

Tables go to stdout for people; add `-o json` for a stable, schema-documented
envelope that scripts and agents can parse. Errors go to stderr with a
meaningful exit code. Data comes from free public endpoints, cached locally.

Examples:
  idx stocks quote BBCA,BBRI,TLKM
  idx stocks history TLKM --period 1y --interval 1wk
  idx stocks fundamental BBCA              # ratios plus a plain-language verdict
  idx stocks technical BBRI                # RSI, MACD, moving averages, signal
  idx stocks compare BBCA BBRI BMRI        # side-by-side fundamentals
  idx stocks screen -f 'pe<10,div>5,mcap>5T' # screener over ~950 IDX tickers
  idx stocks news BBCA                     # Kontan + CNBC Indonesia headlines
  idx broker BBCA                          # which brokers bought and sold it
  idx bandar detect BBCA                   # top brokers accumulating or distributing?
  idx bandar screen --preset big-accumulation  # bandarmology screener
  idx stocks actions BBCA                  # dividends, splits, rights, RUPS
  idx market foreign                       # top foreign net buying and selling
  idx market calendar --type dividend      # cum, ex and payment dates ahead
  idx ownership import --latest            # KSEI >1% holders into local SQLite
  idx ownership ticker BBCA                # who holds more than 1%
  idx mcp                                  # MCP server over stdio for agents
  idx -o json stocks quote BBCA | jq '.data[0].price'
  idx --offline stocks quote BBCA        # serve from cache, never touch the network
  idx schema quotes                      # JSON Schema of the quote envelope";

const AFTER_HELP: &str = "\
Exit codes: 0 ok · 1 internal · 2 usage · 3 network/provider · 4 not found · 5 offline cache miss · 6 config
Env: IDX_PROVIDER, IDX_OUTPUT, IDX_COLOR / NO_COLOR, IDX_PROGRESS, IDX_TIMEOUT_SECS, IDX_CACHE_DIR, IDX_OFFLINE, IDX_NO_CACHE, IDX_CONFIG, IDX_MSN_API_KEY, IDX_OWNERSHIP_DB, IDX_HOLIDAYS
Docs: https://github.com/fzn0x/idx";

#[derive(Debug, Parser)]
#[command(
    name = "idx",
    version,
    about = "Indonesian stock market (IDX / IHSG) analysis CLI for humans and AI agents",
    long_about = LONG_ABOUT,
    after_help = AFTER_HELP,
    propagate_version = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalOpts,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, clap::Args)]
pub struct GlobalOpts {
    /// Output format: table for people, json for scripts and agents, csv for spreadsheets.
    #[arg(short = 'o', long, global = true, value_enum, value_name = "FORMAT")]
    pub output: Option<OutputFormat>,

    /// Data provider. `auto` tries each one in turn.
    #[arg(long, global = true, value_enum, value_name = "NAME")]
    pub provider: Option<ProviderKind>,

    /// Never touch the network; serve from the local cache even if stale.
    #[arg(long, global = true, conflicts_with = "no_cache")]
    pub offline: bool,

    /// Ignore cached responses (still writes fresh ones).
    #[arg(long, global = true)]
    pub no_cache: bool,

    /// Disable ANSI colours (also honours NO_COLOR).
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Per-request network timeout in seconds.
    #[arg(long, global = true, value_name = "SECS")]
    pub timeout: Option<u64>,

    /// Cache directory override.
    #[arg(long, global = true, value_name = "DIR")]
    pub cache_dir: Option<PathBuf>,

    /// Config file path.
    #[arg(long, global = true, value_name = "PATH", env = "IDX_CONFIG")]
    pub config: Option<PathBuf>,

    /// Verbose diagnostics on stderr (-v info, -vv debug, -vvv trace).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Quotes, history, profile, financials, earnings, fundamentals, technicals, compare, corporate actions, foreign flow.
    #[command(subcommand)]
    Stocks(stocks::StocksCmd),

    /// KSEI shareholder register (holders above 1 %): import monthly files, query holders, entities, changes.
    #[command(subcommand)]
    Ownership(ownership::OwnershipCmd),

    /// Whole market: overview, indices, foreign flow, top brokers, the exchange calendar, holidays, members.
    #[command(subcommand)]
    Market(market::MarketCmd),

    /// Broker summary: which brokers bought and sold a ticker (top 10 per side, IPOT).
    #[command(after_help = "Examples:
  idx broker BBCA                       # latest session with trades
  idx broker BBCA --date 2026-10-16
  idx broker BBRI --from 2026-10-12 --to 2026-10-16
  idx broker TLKM,ASII --investor foreign
  idx broker BBCA --board regular -n 5
  idx -o json broker BBCA | jq '.data[0].buyers[0]'
  idx -o csv broker BBCA BBRI > brokers.csv   # one row per broker per side

Source: Indo Premier (IPOT) broker summary, the top 10 buyers and sellers ranked by lots, with
lots, value, average price and each broker's class (local, foreign, BUMN). Broker names come from
the IDX member list bundled with idx. The source rounds figures of a million and more to one
decimal (1.6 M); JSON marks those summaries with rounded=true.")]
    Broker(broker::BrokerArgs),

    /// Bandarmology: are the top brokers accumulating or distributing? `detect` per ticker, `screen` across the most traded, `broker` for one broker.
    #[command(subcommand)]
    Bandar(bandar::BandarCmd),

    /// Your own Stockbit session: `login` in the browser, `status`, `logout`; live `orderbook`; corporate `actions` and `calendar`.
    #[command(
        after_help = "Examples:\n  idx stockbit login                 # sign in once in your browser\n  idx stockbit status\n  idx stockbit orderbook BBCA        # live depth ladder + foreign flow\n  idx stockbit actions BBCA          # a stock's corporate actions since listing\n  idx stockbit calendar --kind dividend --days 30\n  idx --provider stockbit stocks quote BBCA   # realtime quote\n  idx --provider stockbit stocks history BBCA --period 1mo\n\nOpt-in realtime from Stockbit's own backend, carrying your Stockbit session\n(the one exception to idx's free-public-data rule). Keep this session separate\nfrom any other tool's: a refresh rotates the token and retires the old one."
    )]
    #[command(subcommand)]
    Stockbit(stockbit::StockbitCmd),

    /// Quote your watchlist, or manage it (add, remove, list, clear, returns).
    #[command(
        after_help = "Examples:\n  idx watch add BBCA BBRI TLKM\n  idx watch                 # quotes for the list\n  idx watch returns         # 1w..5y returns vs IHSG\n  idx watch rm TLKM\n\nStored as plain text next to the config file (or $IDX_WATCHLIST)."
    )]
    Watch(watch::WatchArgs),

    /// Read or write the config file.
    #[command(subcommand)]
    Config(config_cmd::ConfigCmd),

    /// Inspect or clear the local response cache.
    #[command(subcommand)]
    Cache(cache_cmd::CacheCmd),

    /// MCP server over stdio (default) or HTTP; `mcp setup <client>` prints the registration recipe for any agent.
    #[command(
        after_help = "Serve:\n  idx mcp                                   # stdio, what every local agent launches\n  idx mcp --http 127.0.0.1:8765 --token T   # streamable HTTP at /mcp for URL-only clients\n\nRegister (prints the exact command or snippet, with this binary's absolute path):\n  idx mcp setup --list\n  idx mcp setup claude-code | codex | muse | antigravity | gemini | copilot-cli | vscode | cursor | windsurf |\n                kiro | jetbrains | zed | cline | continue | opencode | goose | warp | droid | vibe | qwen |\n                crush | trae | augment | chatgpt\n\n37 tools returning the same JSON as -o json: idx_brief (start here), idx_market, idx_quote, idx_history,\nidx_profile, idx_financials, idx_earnings, idx_dividends, idx_returns, idx_fundamental, idx_technical,\nidx_compare, idx_screen, idx_news, idx_sentiment, idx_corporate_actions, idx_foreign, idx_instruments,\nidx_market_foreign, idx_market_brokers, idx_indices, idx_calendar, idx_holidays, idx_members, idx_broker,\nidx_bandar, idx_bandar_screen, idx_bandar_broker, idx_ownership_ticker|entity|changes|foreign|releases,\nand the Stockbit tools on your own session: idx_orderbook, idx_stockbit_actions, idx_stockbit_status,\nidx_stockbit_login.\nLogs go to stderr; stdout carries only the protocol."
    )]
    Mcp(mcp_cmd::McpArgs),

    /// Print the JSON Schema for a JSON output shape (run without a name to list them).
    Schema {
        /// quotes | history | profile | fundamentals | compare | financials | earnings | technical | screen | news | sentiment | actions | foreign | instruments | market-foreign | market-brokers | indices | calendar | holidays | members | broker | bandar | bandar-screen | bandar-broker | ownership-* | stockbit-actions | stockbit-status | error | quote | candle
        name: Option<String>,
    },

    /// Generate shell completions (bash, zsh, fish, powershell, elvish).
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Print version and build information.
    Version,
}

/// Everything a handler needs. Built once per invocation.
pub struct Context {
    pub settings: Settings,
    pub provider: Arc<dyn Provider>,
    pub fetcher: Arc<Fetcher>,
    pub color: bool,
}

impl Context {
    pub fn json(&self) -> bool {
        self.settings.output == OutputFormat::Json
    }
}

/// Prints warnings and the stale notice to stderr (table and csv modes;
/// JSON carries them in the envelope).
pub fn notices<T>(ctx: &Context, s: &Sourced<T>) {
    if ctx.json() {
        return;
    }
    for w in &s.warnings {
        output::print_stderr(&format!("warning: {w}"));
    }
    if s.stale {
        output::print_stderr("warning: showing stale cached data");
    }
}

/// Writes a result in the chosen format: the JSON envelope, CSV rows of the
/// payload's main list, or the given table.
pub fn emit<T: serde::Serialize>(
    ctx: &Context,
    schema: &str,
    s: Sourced<T>,
    table: impl FnOnce(&T) -> String,
) -> Result<()> {
    match ctx.settings.output {
        OutputFormat::Json => {
            let env = output::json::Envelope::from_sourced(schema, s);
            output::print_stdout(&output::json::to_pretty(&env)?)
        }
        OutputFormat::Csv => {
            notices(ctx, &s);
            let v = serde_json::to_value(&s.data)?;
            output::print_stdout(&output::csv::render(&v))
        }
        OutputFormat::Table => {
            notices(ctx, &s);
            output::print_stdout(&table(&s.data))
        }
    }
}

pub fn resolve_settings(global: &GlobalOpts) -> Result<Settings> {
    let config_path = global
        .config
        .clone()
        .unwrap_or_else(config::default_config_path);
    let file = FileConfig::load(&config_path)?;
    let overrides = CliOverrides {
        provider: global.provider,
        output: global.output,
        no_color: global.no_color,
        offline: global.offline,
        no_cache: global.no_cache,
        timeout_secs: global.timeout,
        cache_dir: global.cache_dir.clone(),
    };
    config::resolve(&overrides, &file, config_path, &|k| std::env::var(k).ok())
}

pub fn build_context(global: &GlobalOpts) -> Result<Context> {
    let settings = resolve_settings(global)?;
    let backend = ReqwestBackend::new(settings.timeout())?;
    let fetcher = Arc::new(Fetcher::new(
        Arc::new(backend),
        Cache::new(&settings.cache_dir),
        settings.cache_mode,
    ));
    let opts = ProviderOptions {
        ttls: Ttls {
            quote: settings.quote_ttl(),
            history: settings.history_ttl(),
            fundamentals: settings.fundamentals_ttl(),
            financials: settings.financials_ttl(),
            ..Ttls::default()
        },
        msn_api_key: settings.msn_api_key.clone(),
        stockbit: StockbitOptions {
            session_path: settings.stockbit_session.clone(),
            access_seed: settings.stockbit_access_seed.clone(),
            refresh_seed: settings.stockbit_refresh_seed.clone(),
        },
    };
    let provider: Arc<dyn Provider> =
        Arc::from(providers::build(settings.provider, fetcher.clone(), opts)?);
    let color = settings
        .color
        .enabled(std::io::IsTerminal::is_terminal(&std::io::stdout()));
    Ok(Context {
        settings,
        provider,
        fetcher,
        color,
    })
}

/// The loading line on stderr while a network command waits; see
/// [`progress`](crate::progress). Never offline, never under `-v`, never when
/// stderr is not a terminal (scripts, agents).
fn spinner(ctx: &Context, global: &GlobalOpts) -> Option<progress::Spinner> {
    let terminal = std::io::IsTerminal::is_terminal(&std::io::stderr());
    if ctx.fetcher.is_offline()
        || !progress::wanted(terminal, global.verbose, &|k| std::env::var(k).ok())
    {
        return None;
    }
    progress::start()
}

/// Entry point used by `main`. Returns the error for `main` to print and map
/// to an exit code.
pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Stocks(cmd) => {
            let ctx = build_context(&cli.global)?;
            let _spinner = spinner(&ctx, &cli.global);
            stocks::run(&ctx, cmd).await
        }
        Command::Ownership(cmd) => {
            let ctx = build_context(&cli.global)?;
            let _spinner = spinner(&ctx, &cli.global);
            ownership::run(&ctx, cmd).await
        }
        Command::Market(cmd) => {
            let ctx = build_context(&cli.global)?;
            let _spinner = spinner(&ctx, &cli.global);
            market::run(&ctx, cmd).await
        }
        Command::Broker(args) => {
            let ctx = build_context(&cli.global)?;
            let _spinner = spinner(&ctx, &cli.global);
            broker::run(&ctx, args).await
        }
        Command::Bandar(cmd) => {
            let ctx = build_context(&cli.global)?;
            let _spinner = spinner(&ctx, &cli.global);
            bandar::run(&ctx, cmd).await
        }
        Command::Stockbit(cmd) => {
            // The data commands show the loading line; `login` prints its
            // own prompts and `status`/`logout` are instant.
            let ctx = build_context(&cli.global)?;
            let _spinner = cmd
                .shows_progress()
                .then(|| spinner(&ctx, &cli.global))
                .flatten();
            stockbit::run(&ctx, cmd).await
        }
        Command::Watch(args) => {
            let ctx = build_context(&cli.global)?;
            let _spinner = spinner(&ctx, &cli.global);
            watch::run(&ctx, args).await
        }
        Command::Mcp(args) => mcp_cmd::run(&cli.global, args).await,
        Command::Config(cmd) => config_cmd::run(&cli.global, cmd),
        Command::Cache(cmd) => cache_cmd::run(&cli.global, cmd),
        Command::Schema { name } => {
            let names = output::json::schema_names();
            match name {
                None => {
                    let json = cli.global.output == Some(OutputFormat::Json);
                    if json {
                        output::print_stdout(&output::json::to_pretty(&names)?)
                    } else {
                        output::print_stdout(&format!(
                            "Available schemas: {}\nUsage: idx schema <name>\n",
                            names.join(", ")
                        ))
                    }
                }
                Some(n) => {
                    let schema = output::json::schema_for(&n)?;
                    output::print_stdout(&output::json::to_pretty(&schema)?)
                }
            }
        }
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            let mut buf = Vec::new();
            clap_complete::generate(shell, &mut cmd, "idx", &mut buf);
            output::print_stdout(&String::from_utf8_lossy(&buf))
        }
        Command::Version => {
            let json = cli.global.output == Some(OutputFormat::Json);
            let info = VersionInfo::current();
            if json {
                output::print_stdout(&output::json::to_pretty(&info)?)
            } else {
                output::print_stdout(&format!(
                    "idx {} ({} {})\nproviders: {}\nconfig: {}\ncache:  {}\n",
                    info.version,
                    info.target,
                    info.profile,
                    info.providers.join(", "),
                    config::default_config_path().display(),
                    config::default_cache_dir().display(),
                ))
            }
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct VersionInfo {
    pub version: &'static str,
    pub target: &'static str,
    pub profile: &'static str,
    pub providers: Vec<&'static str>,
}

impl VersionInfo {
    pub fn current() -> Self {
        VersionInfo {
            version: env!("CARGO_PKG_VERSION"),
            target: std::env::consts::ARCH,
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            providers: providers::AUTO_ORDER.to_vec(),
        }
    }
}

/// Prints an error the way the chosen output format expects.
pub fn report_error(global: Option<&GlobalOpts>, err: &IdxError) {
    let json = global
        .map(|g| g.output == Some(OutputFormat::Json))
        .unwrap_or(false)
        || std::env::var("IDX_OUTPUT").is_ok_and(|v| v.eq_ignore_ascii_case("json"));
    if json {
        let env = output::json::ErrorEnvelope::from(err);
        output::print_stderr(&serde_json::to_string(&env).unwrap_or_default());
    } else {
        output::print_stderr(&format!("error: {err}"));
        if let IdxError::OfflineMiss(_) = err {
            output::print_stderr("hint: run once without --offline to populate the cache");
        }
    }
}

pub fn color_mode_for_tests() -> ColorMode {
    ColorMode::Never
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_global_flags_anywhere() {
        let cli = Cli::parse_from(["idx", "stocks", "quote", "BBCA", "-o", "json", "--offline"]);
        assert_eq!(cli.global.output, Some(OutputFormat::Json));
        assert!(cli.global.offline);
        let cli = Cli::parse_from([
            "idx", "-o", "table", "stocks", "history", "TLKM", "--period", "5y",
        ]);
        assert_eq!(cli.global.output, Some(OutputFormat::Table));
        assert!(matches!(cli.command, Command::Stocks(_)));
    }

    #[test]
    fn offline_and_no_cache_conflict() {
        assert!(Cli::try_parse_from(["idx", "--offline", "--no-cache", "version"]).is_err());
    }
}
