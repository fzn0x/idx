//! Command-line surface. Each subcommand module owns its argument structs and
//! its handler; this module wires global flags into [`Context`].

use std::path::PathBuf;
use std::sync::Arc;

use clap::{CommandFactory, Parser, Subcommand};

use crate::cache::Cache;
use crate::config::{self, CliOverrides, FileConfig, Settings};
use crate::error::{IdxError, Result};
use crate::http::{Fetcher, ReqwestBackend};
use crate::output::{self, ColorMode, OutputFormat, OutputSetting};
use crate::progress;
use crate::providers::{
    self, HistoryProviderKind, Provider, ProviderKind, ProviderOptions, Sourced, StockbitOptions,
    Ttls,
};

pub mod bandar;
pub mod broker;
pub mod cache_cmd;
pub mod config_cmd;
pub mod input;
pub mod market;
pub mod mcp_cmd;
pub mod ownership;
pub mod ownership_analytics;
pub mod stockbit;
pub mod stocks;
pub mod watch;

const LONG_ABOUT: &str = "\
Indonesian stock market (IDX / IHSG) analysis for humans and AI agents.

Tables go to stdout for people; add `-o json` for a stable, schema-documented
envelope that scripts and agents can parse (`-o jsonl` for one record per line,
`-o auto` for a table on a terminal and JSON when piped). Errors go to stderr
with a meaningful exit code; `-q` keeps stderr to errors. Data comes from free
public endpoints, cached locally.

Examples:
  idx stocks quote BBCA,BBRI,TLKM
  idx stocks history TLKM --period 1y --interval 1wk
  idx stocks fundamental BBCA              # ratios plus a plain-language verdict
  idx stocks risk TLKM                     # beta, volatility and drawdown against IHSG
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
  idx -o jsonl stocks screen -f 'pe<10' -n 5 | idx stocks compare -   # one record per line, - reads stdin
  idx --offline stocks quote BBCA        # serve from cache, never touch the network
  idx schema quotes                      # JSON Schema of the quote envelope";

const AFTER_HELP: &str = "\
Exit codes: 0 ok · 1 internal · 2 usage · 3 network/provider (JSON code blocked: bot protection refused this network) · 4 not found · 5 offline cache miss · 6 config
Env: IDX_PROVIDER, IDX_HISTORY_PROVIDER, IDX_OUTPUT, IDX_COLOR / NO_COLOR, IDX_PROGRESS, IDX_QUIET, IDX_EXCHANGE, IDX_TIMEOUT_SECS, IDX_CACHE_DIR, IDX_OFFLINE, IDX_NO_CACHE, IDX_CONFIG, IDX_MSN_API_KEY, IDX_OWNERSHIP_DB, IDX_HOLIDAYS
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
    /// Output format: table for people, json for scripts and agents, jsonl for one record per line, csv for spreadsheets, auto for a table on a terminal and json when piped.
    #[arg(short = 'o', long, global = true, value_enum, value_name = "FORMAT")]
    pub output: Option<OutputSetting>,

    /// Data provider. `auto` tries each one in turn.
    #[arg(long, global = true, value_enum, value_name = "NAME")]
    pub provider: Option<ProviderKind>,

    /// Where candles come from (history, technical, returns, brief, bandar price moves): auto follows --provider; yahoo or stockbit pin it.
    #[arg(long, global = true, value_enum, value_name = "NAME")]
    pub history_provider: Option<HistoryProviderKind>,

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

    /// Hide warnings, notes and the loading line on stderr (also IDX_QUIET=1). Errors still print, and -o json keeps warnings in the envelope.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Quotes, history, profile, financials, earnings, fundamentals, growth/valuation/risk lenses, technicals, compare, corporate actions, foreign flow.
    #[command(subcommand)]
    Stocks(stocks::StocksCmd),

    /// Who owns a stock: KSEI's register of holders above 1 % and its investor-type composition (import monthly files; holders, entities, search, cross-holders, concentration, network graph, changes, composition; resolve spellings) and institutional and fund holders from MSN.
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
  echo BBCA BBRI | idx broker -               # tickers from stdin

Source: Indo Premier (IPOT) broker summary, the top 10 buyers and sellers ranked by lots, with
lots, value, average price and each broker's class (local, foreign, BUMN). Broker names come from
the IDX member list bundled with idx. The source rounds figures of a million and more to one
decimal (1.6 M); JSON marks those summaries with rounded=true.

With --investor foreign or domestic the source's totals are that side's buying, so idx prints what
the side bought and sold (buy_value, sell_value) instead of the value traded. The source's foreign
net covers every board, so a --board filter leaves it out. Over a range that reaches the latest
sessions the foreign net is rebuilt from single sessions (the source counts the latest one twice).")]
    Broker(broker::BrokerArgs),

    /// Bandarmology: are the top brokers accumulating or distributing? `detect` per ticker, `screen` across the most traded, `broker` for one broker.
    #[command(subcommand)]
    Bandar(bandar::BandarCmd),

    /// Realtime via your own Stockbit session: `login` in the browser, `status`, `logout`, live `orderbook`.
    #[command(
        after_help = "Examples:\n  idx stockbit login                 # sign in once in your browser\n  idx stockbit status\n  idx stockbit orderbook BBCA        # live depth ladder + foreign flow\n  idx --provider stockbit stocks quote BBCA   # realtime quote\n  idx --provider stockbit stocks history BBCA --period 1mo\n\nOpt-in realtime from Stockbit's own backend, carrying your Stockbit session\n(the one exception to idx's free-public-data rule). Keep this session separate\nfrom any other tool's: a refresh rotates the token and retires the old one."
    )]
    #[command(subcommand)]
    Stockbit(stockbit::StockbitCmd),

    /// Quote your watchlist, or manage it (add, remove, list, clear, returns).
    #[command(
        after_help = "Examples:\n  idx watch add BBCA BBRI TLKM\n  idx -o jsonl stocks screen -f \"div>6\" | idx watch add -   # the screen's tickers\n  idx watch                 # quotes for the list\n  idx watch returns         # 1w..5y returns vs IHSG\n  idx watch rm TLKM\n\nStored as plain text next to the config file (or $IDX_WATCHLIST)."
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
        after_help = "Serve:\n  idx mcp                                   # stdio, what every local agent launches\n  idx mcp --http 127.0.0.1:8765 --token T   # streamable HTTP at /mcp for URL-only clients\n\nRegister (prints the exact command or snippet, with this binary's absolute path):\n  idx mcp setup --list\n  idx mcp setup claude-code | codex | muse | antigravity | gemini | copilot-cli | vscode | cursor | windsurf |\n                kiro | jetbrains | zed | cline | continue | opencode | goose | warp | droid | vibe | qwen |\n                crush | trae | augment | chatgpt\n\n44 tools returning the same JSON as -o json: idx_brief (start here), idx_market, idx_quote, idx_history,\nidx_intraday, idx_profile, idx_financials, idx_earnings, idx_dividends, idx_returns, idx_fundamental, idx_technical,\nidx_compare, idx_screen, idx_news, idx_sentiment, idx_insights, idx_corporate_actions, idx_foreign, idx_instruments,\nidx_market_foreign, idx_market_brokers, idx_indices, idx_calendar, idx_holidays, idx_members, idx_broker,\nidx_bandar, idx_bandar_screen, idx_bandar_broker,
idx_ownership_ticker|entity|search|rankings|graph|changes|foreign|releases|composition|snapshot|institutions,\nand the realtime Stockbit tools idx_orderbook, idx_stockbit_status, idx_stockbit_login.\nLogs go to stderr; stdout carries only the protocol."
    )]
    Mcp(mcp_cmd::McpArgs),

    /// Print the JSON Schema for a JSON output shape (run without a name to list them).
    Schema {
        /// quotes | history | profile | fundamentals | growth | valuation | risk | compare | financials | earnings | technical | screen | news | sentiment | insights | actions | foreign | instruments | market-foreign | market-brokers | indices | calendar | holidays | members | broker | bandar | bandar-screen | bandar-broker | ownership-* | error | quote | candle
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

/// Prints warnings and the stale notice to stderr (table, csv and jsonl
/// modes; JSON carries them in the envelope). The notice says when the copy was
/// fetched; after a failed fetch the cause is in the warnings above it.
pub fn notices<T>(ctx: &Context, s: &Sourced<T>) {
    if ctx.json() {
        return;
    }
    for w in &s.warnings {
        output::print_notice(&format!("warning: {w}"));
    }
    if let Some(line) = stale_notice(s) {
        output::print_notice(&line);
    }
}

fn stale_notice<T>(s: &Sourced<T>) -> Option<String> {
    if !s.stale {
        return None;
    }
    let when = s
        .fetched_at
        .with_timezone(&crate::model::wib())
        .format("%Y-%m-%d %H:%M WIB");
    let why = match s.stale_reason {
        Some(crate::model::StaleReason::Offline) => " (--offline)",
        _ => "",
    };
    Some(format!(
        "warning: showing stale cached data from {when}{why}"
    ))
}

/// Writes a result in the chosen format: the JSON envelope, one JSON line per
/// record (the schema's row rule), CSV rows of the payload's main list, or the
/// given table.
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
        OutputFormat::Jsonl => {
            notices(ctx, &s);
            let v = serde_json::to_value(&s.data)?;
            output::print_stdout(&output::jsonl::render(schema, &v)?)
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
        history_provider: global.history_provider,
        output: global.output,
        stdout_is_terminal: std::io::IsTerminal::is_terminal(&std::io::stdout()),
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
        history: settings.history_provider,
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
/// [`progress`](crate::progress). Never offline, never with `--quiet`, never
/// under `-v`, never when stderr is not a terminal (scripts, agents).
fn spinner(ctx: &Context, global: &GlobalOpts) -> Option<progress::Spinner> {
    let terminal = std::io::IsTerminal::is_terminal(&std::io::stderr());
    if !wants_spinner(
        ctx.fetcher.is_offline(),
        output::is_quiet(),
        terminal,
        global.verbose,
        &|k| std::env::var(k).ok(),
    ) {
        return None;
    }
    progress::start()
}

/// Whether to show the loading line; `--quiet` wins over `IDX_PROGRESS=always`.
fn wants_spinner(
    offline: bool,
    quiet: bool,
    stderr_is_terminal: bool,
    verbose: u8,
    env: &dyn Fn(&str) -> Option<String>,
) -> bool {
    !offline && !quiet && progress::wanted(stderr_is_terminal, verbose, env)
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
            // No spinner: `login` drives a browser and prints its own prompts.
            let ctx = build_context(&cli.global)?;
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
            let format = requested_format(&cli.global);
            let names = output::json::schema_names();
            match name {
                None => {
                    if output::print_value(format, &names)? {
                        return Ok(());
                    }
                    output::print_stdout(&format!(
                        "Available schemas: {}\nUsage: idx schema <name>\n",
                        names.join(", ")
                    ))
                }
                Some(n) => {
                    let schema = output::json::schema_for(&n)?;
                    // A JSON Schema is JSON in every format; jsonl makes it one line.
                    if output::print_value(format, &schema)? {
                        return Ok(());
                    }
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
            let info = VersionInfo::current();
            if output::print_value(requested_format(&cli.global), &info)? {
                Ok(())
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

/// The output format of commands that build no [`Context`] (`version`,
/// `schema`, `config`, `mcp setup`) and of error reports: the flag,
/// `IDX_OUTPUT`, then the config file's `output`, skipping a layer that does
/// not parse ([`config::requested_output`]), with `auto` resolved.
pub fn requested_format(global: &GlobalOpts) -> OutputFormat {
    lenient_format(global.output, global.config.clone())
}

fn lenient_format(flag: Option<OutputSetting>, config: Option<PathBuf>) -> OutputFormat {
    let path = config.unwrap_or_else(config::default_config_path);
    config::requested_output(
        flag,
        &path,
        &|k| std::env::var(k).ok(),
        std::io::IsTerminal::is_terminal(&std::io::stdout()),
    )
}

/// Prints an error the way the chosen output format expects: one JSON line
/// in json and jsonl modes, `error:` (and a `hint:`) otherwise.
pub fn report_error(global: Option<&GlobalOpts>, err: &IdxError) {
    let format = match global {
        Some(g) => requested_format(g),
        None => lenient_format(None, std::env::var_os("IDX_CONFIG").map(PathBuf::from)),
    };
    if format.is_json() {
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
        assert_eq!(cli.global.output, Some(OutputSetting::Json));
        assert!(cli.global.offline);
        let cli = Cli::parse_from([
            "idx", "-o", "table", "stocks", "history", "TLKM", "--period", "5y",
        ]);
        assert_eq!(cli.global.output, Some(OutputSetting::Table));
        assert!(matches!(cli.command, Command::Stocks(_)));
        for (v, want) in [
            ("auto", OutputSetting::Auto),
            ("jsonl", OutputSetting::Jsonl),
        ] {
            let cli = Cli::parse_from(["idx", "-o", v, "version"]);
            assert_eq!(cli.global.output, Some(want));
        }
    }

    #[test]
    fn quiet_flag_parses_and_conflicts_with_verbose() {
        let cli = Cli::parse_from(["idx", "stocks", "quote", "BBCA", "-q"]);
        assert!(cli.global.quiet);
        let cli = Cli::parse_from(["idx", "--quiet", "version"]);
        assert!(cli.global.quiet);
        assert!(Cli::try_parse_from(["idx", "-q", "-v", "version"]).is_err());
        assert!(Cli::try_parse_from(["idx", "--quiet", "-vv", "version"]).is_err());
    }

    #[test]
    fn quiet_hides_the_loading_line_even_when_forced() {
        let always = |k: &str| (k == "IDX_PROGRESS").then(|| "always".to_string());
        assert!(wants_spinner(false, false, false, 0, &always));
        assert!(!wants_spinner(false, true, false, 0, &always));
        assert!(!wants_spinner(false, true, true, 0, &|_| None));
        assert!(wants_spinner(false, false, true, 0, &|_| None));
        assert!(!wants_spinner(true, false, true, 0, &always), "offline");
    }

    #[test]
    fn history_provider_is_a_global_flag() {
        let cli = Cli::parse_from([
            "idx",
            "stocks",
            "history",
            "BBCA",
            "--history-provider",
            "stockbit",
        ]);
        assert_eq!(
            cli.global.history_provider,
            Some(HistoryProviderKind::Stockbit)
        );
        assert!(Cli::try_parse_from(["idx", "--history-provider", "msn", "version"]).is_err());
    }

    #[test]
    fn offline_and_no_cache_conflict() {
        assert!(Cli::try_parse_from(["idx", "--offline", "--no-cache", "version"]).is_err());
    }
}
