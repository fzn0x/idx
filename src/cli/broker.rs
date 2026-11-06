//! `idx broker ...`: which brokers bought and sold a ticker.

use chrono::NaiveDate;
use serde_json::{json, Value};

use crate::error::Result;
use crate::model::{BrokerBoard, BrokerInvestor, BrokerSummary, Symbol};
use crate::output::json::SCHEMA_BROKER;
use crate::output::table_broker as tb;
use crate::output::{self, OutputFormat};
use crate::service::{self, BrokerQuery};
use crate::universe;

use super::{emit, notices, Context};

#[derive(Debug, clap::Args)]
pub struct BrokerArgs {
    /// One or more IDX tickers (up to 10), comma or space separated.
    #[arg(value_name = "SYMBOL", required = true, num_args = 1..)]
    pub symbols: Vec<String>,
    /// One trading day, YYYY-MM-DD. Default: the latest session with trades.
    #[arg(long, short = 'd', value_name = "DATE", conflicts_with_all = ["from", "to"])]
    pub date: Option<NaiveDate>,
    /// Start of a range, YYYY-MM-DD (the end defaults to today).
    #[arg(long, value_name = "DATE")]
    pub from: Option<NaiveDate>,
    /// End of a range, YYYY-MM-DD.
    #[arg(long, value_name = "DATE")]
    pub to: Option<NaiveDate>,
    /// Whose trades to count.
    #[arg(long, value_enum, default_value_t = BrokerInvestor::All)]
    pub investor: BrokerInvestor,
    /// Market board to count.
    #[arg(long, value_enum, default_value_t = BrokerBoard::All)]
    pub board: BrokerBoard,
    /// Rows per side in table mode (JSON, JSONL and CSV carry all the source published).
    #[arg(long, short = 'n', default_value_t = 10)]
    pub top: usize,
}

/// One CSV row per broker per side, so a spreadsheet gets a flat table.
fn csv_rows(list: &[BrokerSummary]) -> Value {
    let mut rows = Vec::new();
    for s in list {
        for (side, flows) in [("buy", &s.buyers), ("sell", &s.sellers)] {
            for f in flows {
                rows.push(json!({
                    "symbol": s.symbol,
                    "from": s.from,
                    "to": s.to,
                    "investor": s.investor,
                    "board": s.board,
                    "side": side,
                    "rank": f.rank,
                    "code": f.code,
                    "name": f.name,
                    "kind": f.kind,
                    "lots": f.lots,
                    "value": f.value,
                    "average_price": f.average_price,
                }));
            }
        }
    }
    Value::Array(rows)
}

pub async fn run(ctx: &Context, a: BrokerArgs) -> Result<()> {
    let symbols = Symbol::parse_list(&a.symbols)?;
    let (from, to) = match a.date {
        Some(d) => (Some(d), Some(d)),
        None => (a.from, a.to),
    };
    let query = BrokerQuery {
        from,
        to,
        investor: a.investor,
        board: a.board,
    };
    let list = universe::load(None).unwrap_or_default();
    let result =
        service::broker(&ctx.fetcher, &list, &symbols, query, service::today_wib()).await?;
    if ctx.settings.output == OutputFormat::Csv {
        notices(ctx, &result);
        return output::print_stdout(&output::csv::render(&csv_rows(&result.data)));
    }
    emit(ctx, SCHEMA_BROKER, result, |d| {
        tb::broker(d, a.top, ctx.color)
    })
}
