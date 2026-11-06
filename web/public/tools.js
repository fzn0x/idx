// The 37 MCP tools, each with its CLI command and, where the web terminal can
// run it, a web command. `web: null` marks the CLI-and-MCP-only tools: the
// idx.co.id ones (idx.co.id refuses requests from this site's host) and the
// Stockbit ones (they carry a personal account session).
// The landing page table and test/tools.test.mjs keep to this list, and the
// test checks it against docs/mcp.md.

export const TOOLS = [
  { tool: "idx_brief", cli: "idx stocks brief BBCA", web: "brief BBCA", what: "one page: quote, verdict, signal, returns, dividends, holders, headlines" },
  { tool: "idx_market", cli: "idx market overview", web: "market", what: "IHSG, breadth, gainers, losers, most traded" },
  { tool: "idx_quote", cli: "idx stocks quote BBCA,BBRI", web: "quote BBCA", what: "delayed quotes; one symbol adds a price chart" },
  { tool: "idx_history", cli: "idx stocks history BBCA --period 3mo", web: "history BBCA 3mo", what: "OHLCV candles" },
  { tool: "idx_profile", cli: "idx stocks profile TLKM", web: "profile TLKM", what: "sector, industry, description, management" },
  { tool: "idx_financials", cli: "idx stocks financials ASII", web: "financials ASII", what: "income statement, balance sheet, cash flow" },
  { tool: "idx_earnings", cli: "idx stocks earnings TLKM", web: "earnings TLKM", what: "EPS vs estimates, next earnings date" },
  { tool: "idx_dividends", cli: "idx stocks dividends TLKM", web: "dividends TLKM", what: "payments, yearly totals, TTM yield, streak" },
  { tool: "idx_returns", cli: "idx stocks returns BBCA BBRI BMRI", web: "returns BBCA BBRI BMRI", what: "1w..5y returns, CAGR, drawdown, excess over IHSG" },
  { tool: "idx_fundamental", cli: "idx stocks fundamental BBCA", web: "fundamental BBCA", what: "ratios with a plain-language verdict" },
  { tool: "idx_technical", cli: "idx stocks technical BBRI", web: "technical BBRI", what: "RSI, MACD, averages, levels, signal" },
  { tool: "idx_compare", cli: "idx stocks compare BBCA BBRI BMRI", web: "compare BBCA BBRI BMRI", what: "side-by-side fundamentals" },
  { tool: "idx_screen", cli: "idx stocks screen -f 'pe<10,div>5,mcap>5T'", web: "screen pe<10,div>5,mcap>5T --sort div", what: "screener over ~950 tickers" },
  { tool: "idx_news", cli: "idx stocks news TLKM", web: "news TLKM", what: "Kontan and CNBC Indonesia headlines" },
  { tool: "idx_sentiment", cli: "idx stocks sentiment BBRI", web: "sentiment BBRI", what: "lexicon sentiment over the headlines" },
  { tool: "idx_corporate_actions", cli: "idx stocks actions BBCA", web: "actions BBCA", what: "dividends, splits, bonus, rights, RUPS, warrants" },
  { tool: "idx_foreign", cli: "idx stocks foreign BBCA", web: null, what: "foreign buying and selling per session" },
  { tool: "idx_instruments", cli: "idx stocks list --search bank", web: "list --search bank", what: "listed stocks (the web lists codes and names)" },
  { tool: "idx_market_foreign", cli: "idx market foreign", web: null, what: "top foreign net buying and selling" },
  { tool: "idx_market_brokers", cli: "idx market brokers", web: null, what: "exchange members ranked by the day's value" },
  { tool: "idx_indices", cli: "idx market indices", web: null, what: "every IDX index for one session" },
  { tool: "idx_calendar", cli: "idx market calendar", web: null, what: "dividend and rights dates, RUPS, public exposes" },
  { tool: "idx_holidays", cli: "idx market holidays --upcoming", web: "holidays --upcoming", what: "trading holidays and the next trading day" },
  { tool: "idx_members", cli: "idx market members", web: "members", what: "exchange members: broker codes and names" },
  { tool: "idx_broker", cli: "idx broker BBCA", web: "broker BBCA", what: "top 10 buyers and sellers of a stock" },
  { tool: "idx_bandar", cli: "idx bandar detect BBCA --daily", web: "bandar BBCA --daily", what: "bandarmology: accumulation or distribution" },
  { tool: "idx_bandar_screen", cli: "idx bandar screen --preset big-accumulation", web: "bandar screen --preset big-accumulation", what: "the most traded stocks the top brokers accumulate" },
  { tool: "idx_bandar_broker", cli: "idx bandar broker YP", web: "bandar broker YP", what: "where one broker is a top buyer or seller" },
  { tool: "idx_ownership_ticker", cli: "idx ownership ticker BBCA", web: "ownership ticker BBCA", what: "holders above 1 %" },
  { tool: "idx_ownership_entity", cli: "idx ownership entity \"Dwimuria Investama\"", web: "ownership entity \"Dwimuria Investama\"", what: "every position of a holder" },
  { tool: "idx_ownership_changes", cli: "idx ownership changes --from 2026-08-31 --to 2026-09-30", web: "ownership changes 2026-08-31 2026-09-30", what: "monthly changes with a summary" },
  { tool: "idx_ownership_foreign", cli: "idx ownership foreign", web: "ownership foreign", what: "disclosed foreign vs local stakes" },
  { tool: "idx_ownership_releases", cli: "idx ownership releases", web: "ownership releases", what: "imported monthly registers" },
  { tool: "idx_orderbook", cli: "idx stockbit orderbook BBCA", web: null, what: "realtime order book: full depth ladder and foreign flow (your Stockbit session)" },
  { tool: "idx_stockbit_status", cli: "idx stockbit status", web: null, what: "is a Stockbit session stored, and when its tokens expire" },
  { tool: "idx_stockbit_login", cli: "idx stockbit login", web: null, what: "sign in to Stockbit in your browser, once" },
  { tool: "idx_stockbit_actions", cli: "idx stockbit actions BBCA", web: null, what: "a stock's corporate actions since listing, or the market's calendar, by kind (your Stockbit session)" },
];

export const CLI_ONLY_REASON = "idx.co.id refuses requests from this site's host (a Cloudflare Worker), so this tool runs in the CLI and the MCP server only.";

export const byName = (name) => TOOLS.find((t) => t.tool === String(name || "").trim().toLowerCase());
