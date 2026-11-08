// The 47 MCP tools, each with its CLI command and, where the web terminal can
// run it, a web command. `web: null` marks the CLI-and-MCP-only tools: the
// idx.co.id ones (idx.co.id refuses requests from this site's host), the KSEI
// composition one (its archive is imported into the CLI's local database), the
// ownership snapshot tool (it acts on the machine's own SQLite file), the
// Stockbit ones (they carry a personal account session) and the ones
// whose source has not been checked from this site's host yet (`unchecked`
// names that source). `why`, when set, is the tool's own reason;
// cliOnlyReason() says which.
// The landing page table and test/tools.test.mjs keep to this list, and the
// test checks it against docs/mcp.md. The CLI-and-MCP-only tools' `what` and
// `why` also have their Bahasa Indonesia words (WHAT_ID, WHY_ID below), for the
// notice the terminal shows when one of them is typed (route.js).
import { LANG } from "./i18n.js";

export const TOOLS = [
  { tool: "idx_brief", cli: "idx stocks brief BBCA", web: "brief BBCA", what: "one page: quote, verdict, signal, returns, dividends, holders, headlines" },
  { tool: "idx_market", cli: "idx market overview", web: "market", what: "IHSG, breadth, gainers, losers, most traded" },
  { tool: "idx_quote", cli: "idx stocks quote BBCA,BBRI", web: "quote BBCA", what: "delayed quotes; one symbol adds a price chart" },
  { tool: "idx_history", cli: "idx stocks history BBCA --period 3mo", web: "history BBCA 3mo", what: "OHLCV candles" },
  { tool: "idx_intraday", cli: "idx stocks intraday BBCA", web: "intraday BBCA", what: "the latest session by the minute: OHLCV bars, VWAP, change vs the previous close" },
  { tool: "idx_profile", cli: "idx stocks profile TLKM", web: "profile TLKM", what: "sector, industry, description, management" },
  { tool: "idx_financials", cli: "idx stocks financials ASII", web: "financials ASII", what: "income statement, balance sheet, cash flow" },
  { tool: "idx_earnings", cli: "idx stocks earnings TLKM", web: "earnings TLKM", what: "EPS vs estimates, next earnings date" },
  { tool: "idx_dividends", cli: "idx stocks dividends TLKM", web: "dividends TLKM", what: "payments, yearly totals, TTM yield, streak" },
  { tool: "idx_returns", cli: "idx stocks returns BBCA BBRI BMRI", web: "returns BBCA BBRI BMRI", what: "1w..5y returns, CAGR, drawdown, excess over IHSG" },
  { tool: "idx_fundamental", cli: "idx stocks fundamental BBCA", web: "fundamental BBCA", what: "ratios with a plain-language verdict; lens growth, valuation or risk for a focused reading" },
  { tool: "idx_technical", cli: "idx stocks technical BBRI", web: "technical BBRI", what: "RSI, MACD, averages, levels, signal" },
  { tool: "idx_compare", cli: "idx stocks compare BBCA BBRI BMRI", web: "compare BBCA BBRI BMRI", what: "side-by-side fundamentals" },
  { tool: "idx_screen", cli: "idx stocks screen -f 'pe<10,div>5,mcap>5T'", web: "screen pe<10,div>5,mcap>5T --sort div", what: "screener over ~950 tickers" },
  { tool: "idx_news", cli: "idx stocks news TLKM", web: "news TLKM", what: "Kontan and CNBC Indonesia headlines" },
  { tool: "idx_sentiment", cli: "idx stocks sentiment BBRI", web: "sentiment BBRI", what: "lexicon sentiment over the headlines" },
  { tool: "idx_insights", cli: "idx stocks insights BBCA", web: "insights BBCA", what: "MSN Money's rated statements vs peers and industry" },
  { tool: "idx_corporate_actions", cli: "idx stocks actions BBCA", web: "actions BBCA", what: "dividends, splits, bonus, rights, RUPS, warrants" },
  { tool: "idx_foreign", cli: "idx stocks foreign BBCA", web: "foreign BBCA", what: "foreign buying and selling per session (the web reads IPOT, in rupiah)" },
  { tool: "idx_instruments", cli: "idx stocks list --search bank", web: "list --search bank", what: "listed stocks (the web lists codes and names)" },
  { tool: "idx_market_foreign", cli: "idx market foreign", web: null, what: "top foreign net buying and selling" },
  { tool: "idx_market_brokers", cli: "idx market brokers", web: "brokers", what: "exchange members ranked by the day's value (the web reads IPOT's latest session)" },
  { tool: "idx_indices", cli: "idx market indices", web: null, what: "every IDX index for one session" },
  { tool: "idx_calendar", cli: "idx market calendar", web: null, what: "dividend and rights dates, RUPS, public exposes" },
  { tool: "idx_holidays", cli: "idx market holidays --upcoming", web: "holidays --upcoming", what: "trading holidays and the next trading day" },
  { tool: "idx_members", cli: "idx market members", web: "members", what: "exchange members: broker codes and names" },
  { tool: "idx_astro", cli: "idx market astro", web: "astro", what: "planets, Moon phase, Mercury retrograde, weton and shio for IHSG, with a returns backtest" },
  { tool: "idx_broker", cli: "idx broker BBCA", web: "broker BBCA", what: "top 10 buyers and sellers of a stock" },
  { tool: "idx_bandar", cli: "idx bandar detect BBCA --daily", web: "bandar BBCA --daily", what: "bandarmology: accumulation or distribution" },
  { tool: "idx_bandar_screen", cli: "idx bandar screen --preset big-accumulation", web: "bandar screen --preset big-accumulation", what: "the most traded stocks the top brokers accumulate" },
  { tool: "idx_bandar_broker", cli: "idx bandar broker YP", web: "bandar broker YP", what: "where one broker is a top buyer or seller" },
  { tool: "idx_ownership_ticker", cli: "idx ownership ticker BBCA", web: "ownership ticker BBCA", what: "holders above 1 %" },
  { tool: "idx_ownership_entity", cli: "idx ownership entity \"Dwimuria Investama\"", web: "ownership entity \"Dwimuria Investama\"", what: "every position of a holder" },
  { tool: "idx_ownership_changes", cli: "idx ownership changes --from 2026-08-31 --to 2026-09-30", web: "ownership changes 2026-08-31 2026-09-30", what: "monthly changes with a summary" },
  { tool: "idx_ownership_foreign", cli: "idx ownership foreign", web: "ownership foreign", what: "disclosed foreign vs local stakes" },
  { tool: "idx_ownership_releases", cli: "idx ownership releases", web: "ownership releases", what: "imported monthly registers" },
  { tool: "idx_ownership_search", cli: "idx ownership search salim", web: "ownership search salim", what: "find a holder by name: key, spellings, issuers held" },
  { tool: "idx_ownership_rankings", cli: "idx ownership cross-holders", web: "ownership cross-holders", what: "holders in the most issuers, or issuers by concentration (HHI)" },
  { tool: "idx_ownership_graph", cli: "idx ownership graph BBCA", web: "ownership graph BBCA", what: "network of holders and issuers around a ticker or holder" },
  { tool: "idx_ownership_composition", cli: "idx ownership composition BBCA", web: null, what: "local vs foreign shares by investor type (KSEI monthly archive)", why: "KSEI's composition archive is downloaded from ksei.co.id into the CLI's local ownership database, so this tool runs in the CLI and the MCP server only for now." },
  { tool: "idx_ownership_snapshot", cli: "idx ownership sync", web: null, what: "install or check the prebuilt ownership database (action sync or status)", why: "This tool installs and checks the ownership database file on your own machine; the web terminal keeps its own copy of the register in step with the same GitHub mirror, so there is nothing to sync here." },
  { tool: "idx_ownership_institutions", cli: "idx ownership institutions BBCA", web: null, unchecked: "MSN Money", what: "institutions and funds holding it: buying, selling, new, exited (MSN)" },
  { tool: "idx_orderbook", cli: "idx stockbit orderbook BBCA", web: null, what: "realtime order book: full depth ladder and foreign flow (your Stockbit session)" },
  { tool: "idx_stockbit_status", cli: "idx stockbit status", web: null, what: "is a Stockbit session stored, and when its tokens expire" },
  { tool: "idx_stockbit_login", cli: "idx stockbit login", web: null, what: "sign in to Stockbit in a browser or through a local page, once" },
  { tool: "idx_stockbit_actions", cli: "idx stockbit actions BBCA", web: null, what: "a stock's corporate actions since listing, or the market's calendar, by kind (your Stockbit session)" },
  { tool: "idx_stockbit_movements", cli: "idx stockbit movements BBCA", web: null, what: "insider and 5 % holder movements: who bought or sold, how many shares, at what price (your Stockbit session)" },
];

export const CLI_ONLY_REASON = "idx.co.id refuses requests from this site's host (a Cloudflare Worker), so this tool runs in the CLI and the MCP server only.";
const CLI_ONLY_REASON_ID = "idx.co.id menolak permintaan dari server situs ini (sebuah Cloudflare Worker), jadi alat ini hanya berjalan di CLI dan server MCP.";

const STOCKBIT_REASON = "The Stockbit tools carry your own account session (captured once by `idx stockbit login`), so they run in the CLI and the MCP server only, never this public page.";
const STOCKBIT_REASON_ID = "Alat Stockbit membawa sesi akun Anda sendiri (diambil sekali dengan `idx stockbit login`), jadi hanya berjalan di CLI dan server MCP, tidak pernah di halaman publik ini.";

/** What each CLI-and-MCP-only tool gives, as the Indonesian landing page says it. */
const WHAT_ID = {
  idx_market_foreign: "beli dan jual bersih asing terbesar",
  idx_indices: "semua indeks bursa untuk satu sesi",
  idx_calendar: "tanggal dividen dan rights, RUPS, paparan publik",
  idx_ownership_composition: "saham lokal dan asing menurut jenis investor (arsip bulanan KSEI)",
  idx_ownership_snapshot: "pasang atau periksa basis data kepemilikan siap pakai (aksi sync atau status)",
  idx_ownership_institutions: "institusi dan reksa dana pemegangnya: membeli, menjual, baru, keluar (MSN)",
  idx_orderbook: "antrean beli dan jual waktu nyata dan aliran asing (sesi Stockbit Anda)",
  idx_stockbit_status: "apakah sesi Stockbit tersimpan, dan kapan tokennya berakhir",
  idx_stockbit_login: "masuk ke Stockbit sekali, lewat peramban atau halaman lokal",
  idx_stockbit_actions: "aksi korporasi suatu saham sejak tercatat, atau kalender pasar, menurut jenis (sesi Stockbit Anda)",
  idx_stockbit_movements: "transaksi orang dalam dan pemegang 5 %: siapa membeli atau menjual, berapa lembar, di harga berapa (sesi Stockbit Anda)",
};

/** The tools' own reasons (`why`) in Bahasa Indonesia. */
const WHY_ID = {
  idx_ownership_composition: "Arsip komposisi KSEI diunduh dari ksei.co.id ke basis data kepemilikan lokal milik CLI, jadi untuk sementara alat ini hanya berjalan di CLI dan server MCP.",
  idx_ownership_snapshot: "Alat ini memasang dan memeriksa berkas basis data kepemilikan di komputer Anda sendiri; terminal web menyimpan salinan daftarnya sendiri yang mengikuti mirror GitHub yang sama, jadi tidak ada yang perlu disinkronkan di sini.",
};

/** What a tool gives, in the page's language (or `lang`): Indonesian where WHAT_ID has it. */
export const whatOf = (t, lang = LANG) => (lang === "id" && WHAT_ID[t.tool]) || t.what;

/** Why a tool with `web: null` runs in the CLI and MCP only, in the page's language (or `lang`). */
export function cliOnlyReason(t, lang = LANG) {
  const id = lang === "id";
  if (t.why) return (id && WHY_ID[t.tool]) || t.why;
  if (t.tool === "idx_orderbook" || t.tool.startsWith("idx_stockbit")) return id ? STOCKBIT_REASON_ID : STOCKBIT_REASON;
  if (t.unchecked) return id
    ? `${t.unchecked} belum diperiksa dari server situs ini (sebuah Cloudflare Worker), jadi untuk sementara alat ini hanya berjalan di CLI dan server MCP.`
    : `${t.unchecked} has not been checked from this site's host (a Cloudflare Worker) yet, so this tool runs in the CLI and the MCP server only for now.`;
  return id ? CLI_ONLY_REASON_ID : CLI_ONLY_REASON;
}

export const byName = (name) => TOOLS.find((t) => t.tool === String(name || "").trim().toLowerCase());
