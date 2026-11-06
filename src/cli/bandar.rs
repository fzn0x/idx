//! `idx bandar ...`: bandarmology on the broker summary.

use chrono::NaiveDate;

use crate::analysis::bandar::{self as bd, Preset};
use crate::error::Result;
use crate::model::{BandarPeriod, BrokerBoard, BrokerInvestor, Symbol};
use crate::output::json::{SCHEMA_BANDAR, SCHEMA_BANDAR_BROKER, SCHEMA_BANDAR_SCREEN};
use crate::output::table_bandar as tb;
use crate::service::{self, BandarQuery, BandarScreenOptions, BrokerActivityOptions};
use crate::universe;

use super::{emit, Context};

const DETECT_HELP: &str = "Examples:
  idx bandar detect BBCA                     # last week, top 5 buyers vs top 5 sellers
  idx bandar detect BBRI TLKM --period 1m
  idx bandar detect BBCA --daily             # plus the session-by-session movement
  idx bandar detect ASII --top 3 --period 1d
  idx bandar detect BBCA --from 2026-09-01 --to 2026-09-30
  idx bandar detect TLKM --investor foreign
  idx -o json bandar detect BBCA | jq '.data[0] | {signal, phase, net_pct}'

Reading: for N = 1, 3, 5 and 10 the top N buyers' value minus the top N sellers' value (the
bandar value; in lots the bandar volume), as a share of all value traded. +15% or more reads
big accumulation, +5% accumulation, -5% distribution, -15% big distribution. The phase sets
that against the price move: accumulation with the price within +2% is quiet accumulation,
distribution with the price within -2% quiet distribution. The top buyers' average price
against the last price says whether they sit on a gain. Heuristic, not advice.";

const SCREEN_HELP: &str = "Examples:
  idx bandar screen --preset big-accumulation          # last week, 100 most traded stocks
  idx bandar screen --preset quiet-accumulation --period 1m
  idx bandar screen --preset big-distribution --period 1d
  idx bandar screen -f 'acc>=5,fpct>=5,gain<0'          # bandar and foreigners buying, price under their average
  idx bandar screen --preset accumulation --symbols BBCA,BBRI,BMRI,BBNI
  idx bandar screen --sort fnet --limit 10
  idx -o json bandar screen --preset below-bandar | jq '.data.rows[].symbol'

Presets: big-accumulation (acc>=15), accumulation (acc>=5), distribution (acc<=-5),
big-distribution (acc<=-15), quiet-accumulation (phase=quiet_accumulation),
quiet-distribution (phase=quiet_distribution), foreign-accumulation (acc>=5,fpct>=5),
below-bandar (acc>=5,gain<0).
Filter keys: acc (top-N net, % of value), net (IDR), lots, fnet (foreign net, IDR),
fpct (foreign net, % of value), gain (price vs top buyers' average, %), chg (price move, %),
turnover (value traded), conc (top buyers' share of value, %), price, signal=..., phase=...
Operators < <= > >= = !=; numbers accept K/M/B/T and %.

One broker-summary request per scanned stock (cached; closed sessions for a week), and on
1w/1m/3m up to three more for its foreign net (the source counts its latest session twice over
a range, so the net is rebuilt from single sessions) and one history request per row the other
filters leave.";

const BROKER_HELP: &str = "Examples:
  idx bandar broker YP                       # last week, the 100 most traded stocks
  idx bandar broker CC --period 1m --scan 200
  idx bandar broker AK --symbols BBCA,BBRI,BMRI,TLKM
  idx -o json bandar broker ZP | jq '.data.bought[] | {symbol, value, share_pct}'

Where one exchange member sits among the top ten buyers or sellers (IPOT broker summary) of the
scanned stocks over the period ending on the latest session: rank, lots, value, average price
and its share of the stock's value. Only the top ten per side are published, so a stock where
the broker traded below tenth place does not show, and net is given only where both sides are
visible. Codes: `idx market members`; each member's whole-day totals: `idx market brokers`.

One broker-summary request per scanned stock, shared with `idx bandar screen` (cached).";

#[derive(Debug, clap::Subcommand)]
pub enum BandarCmd {
    /// Bandar detector: accumulation or distribution by the top brokers, per ticker.
    #[command(after_help = DETECT_HELP)]
    Detect(DetectArgs),
    /// Bandarmology screener: the most traded stocks the top brokers accumulate or distribute.
    #[command(after_help = SCREEN_HELP)]
    Screen(ScreenArgs),
    /// Broker activity: which of the most traded stocks one broker is a top buyer or seller of.
    #[command(after_help = BROKER_HELP)]
    Broker(BrokerArgs),
}

#[derive(Debug, clap::Args)]
pub struct BrokerArgs {
    /// Exchange-member code, e.g. YP or CC.
    #[arg(value_name = "CODE")]
    pub code: String,
    /// Look-back ending at the latest session with trades.
    #[arg(long, short = 'p', value_enum, default_value_t = BandarPeriod::Week1)]
    pub period: BandarPeriod,
    /// Rows per side; 0 = all.
    #[arg(long, short = 'l', default_value_t = 20)]
    pub limit: usize,
    /// How many of the most traded stocks (by value, latest session) to scan.
    #[arg(long, default_value_t = service::BANDAR_SCAN_DEFAULT)]
    pub scan: usize,
    /// Scan these tickers instead of the most traded.
    #[arg(long, value_name = "SYMBOLS", value_delimiter = ',')]
    pub symbols: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub struct DetectArgs {
    /// One or more IDX tickers (up to 10; 3 with --daily), comma or space separated.
    #[arg(value_name = "SYMBOL", required = true, num_args = 1..)]
    pub symbols: Vec<String>,
    /// Look-back ending at the latest session with trades.
    #[arg(long, short = 'p', value_enum, default_value_t = BandarPeriod::Week1)]
    pub period: BandarPeriod,
    /// One trading day instead of a period, YYYY-MM-DD.
    #[arg(long, short = 'd', value_name = "DATE", conflicts_with_all = ["from", "to"])]
    pub date: Option<NaiveDate>,
    /// Start of an explicit range, YYYY-MM-DD (the end defaults to today).
    #[arg(long, value_name = "DATE")]
    pub from: Option<NaiveDate>,
    /// End of an explicit range, YYYY-MM-DD.
    #[arg(long, value_name = "DATE")]
    pub to: Option<NaiveDate>,
    /// Brokers per side behind the headline signal: 1, 3, 5 or 10.
    #[arg(long, short = 'n', default_value_t = bd::DEFAULT_TOP)]
    pub top: u32,
    /// Whose trades to count.
    #[arg(long, value_enum, default_value_t = BrokerInvestor::All)]
    pub investor: BrokerInvestor,
    /// Market board to count.
    #[arg(long, value_enum, default_value_t = BrokerBoard::All)]
    pub board: BrokerBoard,
    /// Add the session-by-session movement (one request per session).
    #[arg(long)]
    pub daily: bool,
}

#[derive(Debug, clap::Args)]
pub struct ScreenArgs {
    /// Ready-made screen.
    #[arg(long, value_enum)]
    pub preset: Option<Preset>,
    /// Extra clauses, e.g. "acc>=5,fpct>0,gain<0" (combined with the preset).
    #[arg(long, short = 'f', value_name = "FILTER")]
    pub filter: Option<String>,
    /// Look-back ending at the latest session with trades.
    #[arg(long, short = 'p', value_enum, default_value_t = BandarPeriod::Week1)]
    pub period: BandarPeriod,
    /// Brokers per side: 1, 3, 5 or 10.
    #[arg(long, short = 'n', default_value_t = bd::DEFAULT_TOP)]
    pub top: u32,
    /// Sort key (any numeric filter key). Default acc.
    #[arg(long, value_name = "KEY")]
    pub sort: Option<String>,
    /// Sort ascending (the distribution presets do by default).
    #[arg(long, conflicts_with = "desc")]
    pub asc: bool,
    /// Sort descending.
    #[arg(long)]
    pub desc: bool,
    /// Rows to show; 0 = all.
    #[arg(long, short = 'l', default_value_t = 25)]
    pub limit: usize,
    /// How many of the most traded stocks (by value, latest session) to scan.
    #[arg(long, default_value_t = service::BANDAR_SCAN_DEFAULT)]
    pub scan: usize,
    /// Scan these tickers instead of the most traded.
    #[arg(long, value_name = "SYMBOLS", value_delimiter = ',')]
    pub symbols: Vec<String>,
}

pub async fn run(ctx: &Context, cmd: BandarCmd) -> Result<()> {
    let list = universe::load(None).unwrap_or_default();
    let today = service::today_wib();
    match cmd {
        BandarCmd::Detect(a) => {
            let symbols = Symbol::parse_list(&a.symbols)?;
            let (from, to) = match a.date {
                Some(d) => (Some(d), Some(d)),
                None => (a.from, a.to),
            };
            let q = BandarQuery {
                period: a.period,
                from,
                to,
                top: a.top,
                investor: a.investor,
                board: a.board,
                daily: a.daily,
            };
            let result = service::bandar(
                ctx.provider.as_ref(),
                &ctx.fetcher,
                &list,
                &symbols,
                q,
                today,
            )
            .await?;
            emit(ctx, SCHEMA_BANDAR, result, |d| tb::bandar(d, ctx.color))
        }
        BandarCmd::Screen(a) => {
            let opts = BandarScreenOptions {
                period: a.period,
                top: a.top,
                preset: a.preset,
                filters: bd::parse_filters(a.filter.as_deref().unwrap_or(""))?,
                sort: bd::parse_sort(a.sort.as_deref())?,
                descending: if a.asc {
                    Some(false)
                } else if a.desc {
                    Some(true)
                } else {
                    None
                },
                limit: a.limit,
                scan: a.scan,
                symbols: if a.symbols.is_empty() {
                    Vec::new()
                } else {
                    Symbol::parse_list(&a.symbols)?
                },
            };
            let result =
                service::bandar_screen(ctx.provider.as_ref(), &ctx.fetcher, &list, &opts, today)
                    .await?;
            emit(ctx, SCHEMA_BANDAR_SCREEN, result, |d| {
                tb::screen(d, ctx.color)
            })
        }
        BandarCmd::Broker(a) => {
            let opts = BrokerActivityOptions {
                code: a.code,
                period: a.period,
                scan: a.scan,
                symbols: if a.symbols.is_empty() {
                    Vec::new()
                } else {
                    Symbol::parse_list(&a.symbols)?
                },
                limit: a.limit,
            };
            let result =
                service::bandar_broker(ctx.provider.as_ref(), &ctx.fetcher, &list, &opts, today)
                    .await?;
            emit(ctx, SCHEMA_BANDAR_BROKER, result, |d| {
                tb::broker_activity(d, ctx.color)
            })
        }
    }
}
