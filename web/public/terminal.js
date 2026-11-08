// The web terminal: parses CLI-shaped commands, calls /api, prints statements.
// The shell (help, states, loading lines, errors) speaks the page's language;
// the results keep the CLI's English column names and words on both pages.
import { api, fmt, NA, table, wrapScroll, el, sparkline, signClass, machineLine, loadUniverse, copyButton, cacheState, CACHE_STATES } from "/app.js";
import { priceChart } from "/chart.js";
import { TOOLS, byName, whatOf } from "/tools.js";
import { routeLine, tokenize, shownLine, ALIASES, INSTALL, AGENT } from "/route.js";
import { SHOW_MS, SLOW_MS, WAITS, FEED_TIMEOUT_S, IPOT_TRIES, waitFor } from "/waits.js";
import { mountGate, suggestions } from "/gate.js";
import { initShell, hotkeys, lamp } from "/shell.js";
import { session, hhmm, wib, boardState } from "/clock.js";
import { ID, t, here, otherPath, PHASES, DAYS_SHORT, TIME_SEP } from "/i18n.js";

const out = document.getElementById("out");
const form = document.getElementById("prompt");
const input = document.getElementById("cmd");

// ---- command table ---------------------------------------------------------

const COMMANDS = [
  { name: "quote", usage: "quote <SYMBOLS>", help: "delayed quotes, e.g. quote BBCA,BBRI; one symbol adds a price chart (1D to 5Y)" },
  { name: "history", usage: "history <SYMBOL> [period] [interval]", help: "OHLCV candles; periods 1d 5d 1mo 3mo 6mo 1y 2y 5y 10y ytd max, intervals 1m 2m 5m 15m 30m 1h 1d 1wk 1mo (1m up to 5d, 2m up to 1mo)" },
  { name: "intraday", usage: "intraday <SYMBOL> [interval]", help: "the latest session by the minute: OHLCV bars, VWAP, change against the previous close (Yahoo, about 10 minutes behind); intervals 1m 2m 5m 15m 30m 1h" },
  { name: "profile", usage: "profile <SYMBOL>", help: "sector, industry, description, size" },
  { name: "financials", usage: "financials <SYMBOL> [annual|quarterly|ttm] [--statement income,balance,cash]", help: "income statement, balance sheet, cash flow; --statement shows only those" },
  { name: "earnings", usage: "earnings <SYMBOL> [--annual|--quarterly] [--reported|--estimates]", help: "quarterly EPS vs estimates, revenue and earnings, analyst consensus, next date; the flags keep only those rows (--history and --forecast work too)" },
  { name: "fundamental", usage: "fundamental <SYMBOL> [--lens growth|valuation|risk]", help: "ratios with a plain-language verdict; --lens gives one of the readings below instead" },
  { name: "growth", usage: "growth <SYMBOL>", help: "growth lens: latest quarter, forward EPS, fiscal-year revenue and net income, each rule with its points" },
  { name: "valuation", usage: "valuation <SYMBOL>", help: "valuation lens: P/E, PEG, P/B, P/S, EV/EBITDA, dividend yield and analyst target, each scored" },
  { name: "risk", usage: "risk <SYMBOL>", help: "risk lens: negative equity, leverage, liquidity, losses, and beta, volatility and largest fall against IHSG" },
  { name: "technical", usage: "technical <SYMBOL> [period]", help: "RSI, MACD, averages, levels, signal" },
  { name: "compare", usage: "compare <SYMBOLS>", help: "side-by-side fundamentals" },
  { name: "screen", usage: "screen [FILTER] [--preset P] [--sort KEY] [--asc|--desc] [--returns] [--limit N]", help: "e.g. screen pe<10,div>5,mcap>5T --sort div; --returns adds the return windows when no filter or sort needs them; presets: top-performers, worst-performers, high-dividend, low-pe, 52w-high, 52w-low, high-volume, large-cap; keys include range (52-week position, 0 to 100) and ret1w ret1m ret3m ret6m retytd ret1y (price return %, as MSN reports it)" },
  { name: "news", usage: "news [SYMBOL] [--limit N] [--company-only]", help: "headlines; with a symbol the company's first, then market context" },
  { name: "dividends", usage: "dividends <SYMBOL>", help: "payments, yearly totals, TTM yield, streak, splits" },
  { name: "returns", usage: "returns <SYMBOLS> [--benchmark ^JKSE|none]", help: "1w..5y returns, CAGR, volatility, drawdown, excess over IHSG" },
  { name: "brief", usage: "brief <SYMBOL>", help: "one page: quote, verdict, signal, returns, dividends, holders, headlines" },
  { name: "sentiment", usage: "sentiment <SYMBOL>", help: "lexicon sentiment over matching headlines" },
  { name: "insights", usage: "insights <SYMBOL> [--all]", help: "MSN Money's rated statements on valuation, earnings, growth, performance and health, against peers, the industry and the company's own history; --all adds the ones MSN could not rate" },
  { name: "broker", usage: "broker <SYMBOLS> [--date D | --from D --to D] [--investor foreign|domestic] [--board regular|cash|negotiated]", help: "which brokers bought and sold it: top 10 per side ranked by lots (IPOT), latest session by default" },
  { name: "foreign", usage: "foreign <SYMBOL> [--days N | --from D --to D]", help: "foreign buying and selling per session in rupiah across all boards (IPOT): value, foreign share, totals and the net-buy/net-sell streak; 10 sessions by default, at most 20" },
  { name: "bandar", usage: "bandar <SYMBOLS> [--period 1d|1w|1m|3m] [--top 1|3|5|10] [--daily] [--date D | --from D --to D] | bandar screen [--preset P] [--filter F] [--period P] [--sort KEY] [--limit N] | bandar broker <CODE> [--period P] [--limit N]", help: "bandarmology: are the top brokers accumulating or distributing? screen presets: big-accumulation, accumulation, distribution, big-distribution, quiet-accumulation, quiet-distribution, foreign-accumulation, below-bandar; broker: where one broker (YP, CC, ...) is a top-10 buyer or seller of the session's 100 most traded stocks" },
  { name: "actions", usage: "actions <SYMBOL> [--since YYYY-MM-DD]", help: "corporate actions (dividends, splits, bonus, rights with ratio and cum/ex/payment dates), RUPS, warrants and rights; upcoming included (IPOT)" },
  { name: "market", usage: "market", help: "IHSG, breadth, gainers, losers, most traded" },
  { name: "holidays", usage: "holidays [YEAR] [--all] [--upcoming]", help: "IDX trading holidays, the year's trading days, whether today is a trading day and the next one" },
  { name: "astro", usage: "astro [DATE] [--days N] [--natal launch|base|none] [--backtest] [--period 1y|2y|5y|10y]", help: "IHSG astrology: planets, Moon phase, Mercury retrograde, weton and shio for a date (16:00 WIB), upcoming phases and ingresses, transits to IHSG's chart; --backtest tests them on IHSG's daily returns (5y by default). Traditional astrology, not a forecast" },
  { name: "list", usage: "list [--search TEXT]", help: "listed stocks: codes and names (bundled; the CLI adds boards, listing dates and shares from idx.co.id)" },
  { name: "members", usage: "members [--search TEXT]", help: "exchange members: the broker codes broker and bandar report, with names" },
  { name: "tools", usage: "tools [idx_TOOL]", help: "the MCP tools with their CLI and web commands; tools idx_quote runs one" },
  { name: "ownership", usage: "ownership ticker <SYMBOL> | entity <NAME> | search <NAME> | cross-holders [--type T] [--origin foreign|local] [--min-issuers N] | concentration [SYMBOLS] [--by hhi|top1|top3] [--asc] | graph <ROOT> [--depth N] [--min-pct P] [--holder] | changes <FROM> <TO> [SYMBOL] [--offset N] | foreign [SYMBOL] | releases", help: "KSEI holders above 1 %, synced monthly from IDX: holders, entities, holder search, holders across issuers, concentration (HHI), the ownership network, monthly changes with a summary, foreign vs local" },
  { name: "json", usage: "json <command…>", help: "print the raw JSON envelope of any command" },
  { name: "help", usage: "help [command]", help: "this list" },
  { name: "clear", usage: "clear", help: "clear the sheet" },
];

// What each command does, in Bahasa Indonesia, for help, the rail and the gate; the usage lines stay as typed.
const HELP_ID = {
  quote: "harga tertunda, misalnya quote BBCA,BBRI; satu kode menambah grafik harga (1D sampai 5Y)",
  history: "candle OHLCV; periode 1d 5d 1mo 3mo 6mo 1y 2y 5y 10y ytd max, interval 1m 2m 5m 15m 30m 1h 1d 1wk 1mo (1m sampai 5d, 2m sampai 1mo)",
  intraday: "sesi terakhir per menit: bar OHLCV, VWAP, perubahan terhadap penutupan sebelumnya (Yahoo, tertinggal sekitar 10 menit); interval 1m 2m 5m 15m 30m 1h",
  profile: "sektor, industri, deskripsi, ukuran perusahaan",
  financials: "laba rugi, neraca, arus kas; --statement hanya menampilkan yang disebut",
  earnings: "EPS per kuartal dibanding perkiraan, pendapatan dan laba, konsensus analis, tanggal laporan berikutnya; flag menyaring barisnya (--history dan --forecast juga bisa)",
  fundamental: "rasio dengan kesimpulan dalam bahasa sederhana; --lens memberi salah satu bacaan di bawah",
  growth: "lensa pertumbuhan: kuartal terakhir, EPS ke depan, pendapatan dan laba bersih per tahun buku, tiap aturan dengan poinnya",
  valuation: "lensa valuasi: P/E, PEG, P/B, P/S, EV/EBITDA, imbal hasil dividen dan target analis, masing-masing dinilai",
  risk: "lensa risiko: ekuitas negatif, utang, likuiditas, kerugian, serta beta, volatilitas dan penurunan terdalam dibanding IHSG",
  technical: "RSI, MACD, rata-rata bergerak, level, sinyal",
  compare: "fundamental beberapa saham berdampingan",
  screen: "misalnya screen pe<10,div>5,mcap>5T --sort div; --returns menambah kolom imbal hasil bila tidak ada filter atau urutan yang memerlukannya; preset: top-performers, worst-performers, high-dividend, low-pe, 52w-high, 52w-low, high-volume, large-cap; kuncinya antara lain range (posisi dalam rentang 52 minggu, 0 sampai 100) dan ret1w ret1m ret3m ret6m retytd ret1y (imbal hasil harga %, menurut MSN)",
  news: "judul berita; dengan kode saham, berita perusahaan lebih dulu, lalu konteks pasar",
  dividends: "pembayaran, total per tahun, imbal hasil TTM, tahun berturut-turut, pemecahan saham",
  returns: "imbal hasil 1w..5y, CAGR, volatilitas, penurunan terdalam, selisih terhadap IHSG",
  brief: "satu halaman: harga, kesimpulan, sinyal, imbal hasil, dividen, pemegang saham, berita",
  sentiment: "sentimen kamus kata atas judul berita yang cocok",
  insights: "pernyataan bernilai dari MSN Money tentang valuasi, laba, pertumbuhan, kinerja dan kesehatan, dibanding emiten sejenis, industrinya dan riwayat perusahaan sendiri; --all menambah yang tidak bisa dinilai MSN",
  broker: "broker mana yang membeli dan menjual: 10 teratas tiap sisi menurut lot (IPOT), sesi terakhir bila tanpa tanggal",
  foreign: "beli dan jual asing per sesi dalam rupiah di semua papan (IPOT): nilai, porsi asing, total dan rentetan beli bersih atau jual bersih; 10 sesi bila tanpa pilihan, paling banyak 20",
  bandar: "bandarmologi: apakah broker teratas sedang akumulasi atau distribusi? preset screen: big-accumulation, accumulation, distribution, big-distribution, quiet-accumulation, quiet-distribution, foreign-accumulation, below-bandar; broker: di saham mana satu broker (YP, CC, ...) masuk 10 pembeli atau penjual teratas dari 100 saham teramai sesi itu",
  actions: "aksi korporasi (dividen, pemecahan saham, bonus, rights dengan rasio dan tanggal cum/ex/pembayaran), RUPS, waran dan rights; termasuk yang akan datang (IPOT)",
  market: "IHSG, jumlah saham naik dan turun, saham naik, saham turun, saham teramai",
  holidays: "hari libur bursa, jumlah hari perdagangan setahun, apakah hari ini hari perdagangan dan kapan yang berikutnya",
  astro: "astrologi IHSG: planet, fase Bulan, Merkurius retrograde, weton dan shio untuk satu tanggal (16.00 WIB), fase dan ingres mendatang, transit ke peta IHSG; --backtest mengujinya pada imbal hasil harian IHSG (5y bila tanpa pilihan). Astrologi tradisional, bukan ramalan",
  list: "saham tercatat: kode dan nama (bawaan; CLI menambah papan, tanggal pencatatan dan jumlah saham dari idx.co.id)",
  members: "anggota bursa: kode broker yang dipakai broker dan bandar, dengan namanya",
  tools: "alat MCP dengan perintah CLI dan web-nya; tools idx_quote menjalankan satu alat",
  ownership: "pemegang saham di atas 1 % dari KSEI, disinkronkan bulanan dari IDX: pemegang, entitas, cari pemegang, pemegang di banyak emiten, konsentrasi (HHI), jaringan kepemilikan, perubahan bulanan dengan ringkasan, asing dan lokal",
  json: "cetak amplop JSON mentah dari perintah apa pun",
  help: "daftar ini",
  clear: "bersihkan lembar hasil",
};
const helpOf = (c) => (ID && HELP_ID[c.name]) || c.help;

/** Errors the terminal raises itself: a usage slip, worded for the page. */
const usageError = (message) => Object.assign(new Error(message), { code: "usage" });
const usageLine = (usage) => usageError(t(`cara pakai: ${usage}`, `usage: ${usage}`));
const CLI_ONLY = t("hanya CLI dan MCP", "CLI and MCP only");
/** The head of a notice (route.js says which kind): where the line runs instead of here. */
const NOTICE_META = { "cli-mcp": CLI_ONLY, cli: t("hanya CLI", "CLI only"), flag: t("flag CLI", "CLI flag") };

// Switches never take the next word as their value: `screen --returns mcap>10T`
// keeps mcap>10T as a filter clause instead of losing it to --returns.
const BOOL_FLAGS = new Set(["all", "asc", "company-only", "daily", "desc", "returns", "upcoming"]);
function flags(toks) {
  const pos = [], opt = {};
  for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    if (t.startsWith("--")) {
      const [k, v] = t.slice(2).split("=");
      if (v != null) opt[k] = v;
      else if (!BOOL_FLAGS.has(k) && i + 1 < toks.length && !toks[i + 1].startsWith("--")) opt[k] = toks[++i];
      else opt[k] = true;
    } else pos.push(t);
  }
  return { pos, opt };
}
const qs = (o) => Object.entries(o).filter(([, v]) => v != null && v !== "").map(([k, v]) => `${k}=${encodeURIComponent(v)}`).join("&");

// ---- rendering --------------------------------------------------------------

function kv(pairs) {
  const dl = el("dl", { class: "kv" });
  for (const [k, v] of pairs) if (v !== undefined) dl.append(el("dt", { text: k }), el("dd", { text: v == null ? NA : String(v) }));
  return dl;
}
const pct = (v) => fmt.pctPlain(v);
const ESTIMATE_NAMES = { "0q": "this quarter", "+1q": "next quarter", "0y": "this year", "+1y": "next year" };
// "showing annual estimates" and the like (scope_label in src/output/table_reports.rs).
function scopeLabel(s) {
  const period = s.annual && !s.quarterly ? "annual " : s.quarterly && !s.annual ? "quarterly " : "";
  const kind = s.reported && !s.estimates ? "reported figures" : s.estimates && !s.reported ? "estimates" : "rows";
  return `showing ${period}${kind}`;
}
const x = (v) => (v == null ? NA : `${fmt.num(v, 2)}x`);
// Yahoo's marketState in words, as the CLI prints it (market_state in src/output/format.rs).
const STATES = { REGULAR: "open", PRE: "pre-market", POST: "after hours", PREPRE: "closed", POSTPOST: "closed", CLOSED: "closed" };
const marketState = (code) => STATES[code.toUpperCase()] || code.toLowerCase();

const newsTable = (items) => wrapScroll(table([
  { h: "WIB", f: (i) => (i.published ? fmt.wib(i.published).slice(5, 16) : NA), l: true }, { h: "Source", k: "source", l: true },
  { h: "±", f: (i) => (i.sentiment > 0 ? "+" : i.sentiment < 0 ? "−" : "·"), cls: (i) => signClass(i.sentiment) },
  { h: "Headline", f: (i) => el("a", { href: i.url, target: "_blank", rel: "noopener", text: i.title }), l: true },
], items));

// Statements in another currency than the price (USD statements, IDR price):
// the source's book, sales and EV multiples then divide one by the other.
// Same test as Fundamentals::mixed_currency in the CLI.
const mixedCurrency = (f) => !!(f.currency && f.financial_currency) && f.currency.toUpperCase() !== f.financial_currency.toUpperCase();
const mixedRatio = (f, v) => (v != null && mixedCurrency(f) ? `${fmt.ratio(v)}*` : fmt.ratio(v));
const inStatementCurrency = (f, v, s) => (v != null && mixedCurrency(f) ? `${s} ${f.financial_currency}` : s);

// The note under a screen whose table marks a P/B, worded as the CLI's
// (src/output/table_news.rs): the marked companies in table order, grouped
// by their statement and price currencies; null when none is marked.
function screenCurrencyNote(rows) {
  const groups = new Map();
  for (const r of rows) {
    if (r.price_to_book == null || !mixedCurrency(r)) continue;
    const pair = `statements in ${r.financial_currency}, price in ${r.currency}`;
    groups.set(pair, [...(groups.get(pair) || []), r.symbol]);
  }
  if (!groups.size) return null;
  const named = [...groups].map(([pair, symbols]) => `${symbols.join(", ")} (${pair})`).join("; ");
  return `* P/B divides the price by statement figures in another currency, so it is not comparable: ${named}. Filters and sorts on pb and ps read such P/B and P/S as missing.`;
}

// Growth, valuation and risk lenses (idx.growth.v1, idx.valuation.v1,
// idx.risk.v1): the inputs, the reading, every rule with its points, then
// the rules that could not run.
const LENS_TONE = { cheap: "up", growing: "up", low: "up", expensive: "dn", shrinking: "dn", high: "dn" };
const yesNo = (b) => (b == null ? null : b ? "yes" : "no");
function lensView(d, lens, groups, direction, after = []) {
  const frag = document.createDocumentFragment();
  const r = d.reading;
  frag.append(el("p", { class: "prose" }, [el("b", { text: d.name || d.symbol }), ` · ${d.symbol} · ${lens}`]));
  const grid = el("div", { class: "three" });
  for (const [title, pairs] of groups) grid.append(el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: title }), kv(pairs)]));
  frag.append(grid, ...after);
  frag.append(el("p", { class: "prose" }, [el("span", { class: `pill ${LENS_TONE[r.label] || ""}`, text: r.label.replace("_", " ") }), ` score ${r.score >= 0 ? "+" : ""}${r.score}, ${r.inputs_available} rule${r.inputs_available === 1 ? "" : "s"} · ${direction}`]));
  frag.append(el("ul", { class: "reasons" }, r.rules.map((x) => el("li", { text: `${x.points >= 0 ? "+" : ""}${x.points}  ${x.reason}` }))));
  if (r.notes.length) frag.append(el("ul", { class: "reasons" }, r.notes.map((n) => el("li", { text: `skipped: ${n}` }))));
  frag.append(el("p", { class: "warn", text: `Heuristic from public data, not investment advice.${d.sources.length ? ` Sources: ${d.sources.join(", ")}` : ""}` }));
  return frag;
}

const BANDAR_NOTE = "Bandarmology from the IPOT broker summary: the top N buyers against the top N sellers (ranked by lots), net value as a share of the value traded: +15% big accumulation, +5% accumulation, -5% distribution, -15% big distribution. Heuristic, not advice.";

// Ownership columns that only appear when they carry data: Accts when a holder
// has several register lines, Control when the release has the 30 Oct 2026 flag.
const CONTROL_COL = { h: "Control", f: (r) => ({ CONTROLLER: "controller", "AFFILIATE OF THE CONTROLLER": "affiliate" })[r.controller_or_affiliate] ?? (r.controller_or_affiliate || "").toLowerCase(), l: true };
function ownCols(rows, controllerReported) {
  const cols = [], notes = [];
  if (controllerReported) { cols.push(CONTROL_COL); notes.push("Control: CONTROLLER_OR_AFFILIATE as the issuer reported it to IDX for the previous month."); }
  if (rows.some((r) => (r.accounts ?? 1) > 1)) { cols.push({ h: "Accts", f: (r) => (r.accounts == null ? NA : String(r.accounts)) }); notes.push("Accts: register lines summed (KSEI lists a holder once per SID and once per scrip account without one)."); }
  return { cols, notes };
}
const changeKind = (r) => {
  const a = r.accounts_from, b = r.accounts_to;
  if (a != null && b != null && a !== b) return `${r.kind} (${a}→${b} accts)`;
  const n = Math.max(a ?? 0, b ?? 0);
  return n > 1 ? `${r.kind} (${n} accts)` : r.kind;
};

const R = {
  quotes(d) {
    const frag = document.createDocumentFragment();
    frag.append(wrapScroll(table([
      { h: "Symbol", k: "symbol", l: true, cls: "sym" },
      { h: "Price", f: (r) => fmt.price(r.price), cls: "num" },
      { h: "Chg %", f: (r) => fmt.pct(r.change_pct), cls: (r) => `num ${signClass(r.change_pct)}` },
      { h: "Chg", f: (r) => fmt.change(r.change), cls: (r) => `num ${signClass(r.change)}` },
      { h: "Name", k: "name", l: true, cls: "name" }, { h: "Open", f: (r) => fmt.price(r.open) },
      { h: "High", f: (r) => fmt.price(r.day_high) }, { h: "Low", f: (r) => fmt.price(r.day_low) }, { h: "Volume", f: (r) => fmt.int(r.volume) },
      { h: "Mkt Cap", f: (r) => fmt.compact(r.market_cap) }, { h: "P/E", f: (r) => fmt.ratio(r.pe_trailing) }, { h: "Div %", f: (r) => pct(r.dividend_yield_pct) },
    ], d)));
    const behind = d.filter((r) => r.behind_session === true).map((r) => (r.market_time ? `${r.symbol} (last trade ${r.market_time.slice(0, 10)})` : r.symbol));
    if (behind.length) frag.append(el("p", { class: "warn", text: `Behind the latest IDX session: ${behind.join(", ")} · suspended, not traded since, or the source lags` }));
    if (d.length === 1) frag.append(priceChart(d[0]));
    return frag;
  },
  history(d) {
    const rows = d.candles.slice(-40);
    const frag = document.createDocumentFragment();
    if (d.summary) {
      const s = d.summary;
      frag.append(el("p", { class: "prose", text: `${d.symbol} · ${d.name || ""} · ${d.period} / ${d.interval} · ${s.candles} candles · close ${fmt.price(s.first_close)} to ${fmt.price(s.last_close)} (${fmt.pct(s.change_pct)}) · high ${fmt.price(s.high)} · low ${fmt.price(s.low)}` }));
      frag.append(sparkline(d.candles.map((c, i) => [i, c.close]), { cls: "spark" }));
    }
    let prev = d.candles.length > 40 ? d.candles[d.candles.length - 41].close : null;
    const withChg = rows.map((c) => { const chg = prev ? ((c.close - prev) / prev) * 100 : null; prev = c.close; return { ...c, chg }; });
    frag.append(wrapScroll(table([
      { h: d.interval.endsWith("m") || d.interval === "1h" ? "Time (WIB)" : "Date", f: (c) => (d.interval.endsWith("m") || d.interval === "1h" ? fmt.wib(c.time).replace(" WIB", "") : fmt.date(c.time)), l: true },
      { h: "Open", f: (c) => fmt.price(c.open) }, { h: "High", f: (c) => fmt.price(c.high) }, { h: "Low", f: (c) => fmt.price(c.low) }, { h: "Close", f: (c) => fmt.price(c.close) },
      { h: "Chg %", f: (c) => fmt.pct(c.chg), cls: (c) => `num ${signClass(c.chg)}` }, { h: "Volume", f: (c) => fmt.int(c.volume) },
    ], withChg)));
    if (d.candles.length > 40) frag.append(el("p", { class: "warn", text: `Showing the last 40 of ${d.candles.length} candles; use json history … for all.` }));
    return frag;
  },
  intraday(d) {
    const frag = document.createDocumentFragment();
    const q = d.quote, s = d.summary;
    const head = [d.symbol, d.name, d.session && `session ${d.session}`, `${d.interval} bars`, q?.market_state && `market ${marketState(q.market_state)}`].filter(Boolean);
    frag.append(el("p", { class: "prose", text: head.join(" · ") }));
    if (q) {
      frag.append(el("p", { class: "prose" }, [
        `Last ${fmt.price(q.price)} `, el("span", { class: signClass(q.change), text: `${fmt.change(q.change)} (${fmt.pct(q.change_pct)})` }),
        ` · prev close ${fmt.price(q.previous_close)} · open ${fmt.price(q.open)} · high ${fmt.price(q.day_high)} · low ${fmt.price(q.day_low)}${q.volume ? ` · volume ${fmt.compact(q.volume)}` : ""}${q.market_time ? ` · last trade ${fmt.wib(q.market_time)}` : ""}`,
      ]));
    }
    const source = "Yahoo's chart, about 10 minutes behind the market. Minutes without a trade have no bar; the opening auction's volume is in no bar, so the bars' volume is below the day's.";
    if (!s) { frag.append(el("p", { class: "warn", text: "No bars for this session." }), el("p", { class: "warn", text: source })); return frag; }
    frag.append(el("p", { class: "prose", text: `Bars ${s.first_bar.slice(11, 16)} to ${s.last_bar.slice(11, 16)} WIB · ${s.bars} bar${s.bars === 1 ? "" : "s"} · open ${fmt.price(s.open)} · high ${fmt.price(s.high)} · low ${fmt.price(s.low)} · close ${fmt.price(s.close)}${s.bars_volume ? ` · volume ${fmt.compact(s.bars_volume)}` : ""} · ${s.vwap == null ? "no VWAP (the bars carry no volume)" : `VWAP ${fmt.price(s.vwap)}`}` }));
    frag.append(sparkline(d.bars.map((c, i) => [i, c.close]), { cls: "spark" }));
    const rows = d.bars.slice(-30);
    let prev = d.bars.length > 30 ? d.bars[d.bars.length - 31].close : null;
    const withChg = rows.map((c) => { const chg = prev ? ((c.close - prev) / prev) * 100 : null; prev = c.close; return { ...c, chg }; });
    frag.append(wrapScroll(table([
      { h: "Time (WIB)", f: (c) => c.time.slice(11, 16), l: true },
      { h: "Open", f: (c) => fmt.price(c.open) }, { h: "High", f: (c) => fmt.price(c.high) }, { h: "Low", f: (c) => fmt.price(c.low) }, { h: "Close", f: (c) => fmt.price(c.close) },
      { h: "Chg %", f: (c) => fmt.pct(c.chg), cls: (c) => `num ${signClass(c.chg)}` }, { h: "Volume", f: (c) => fmt.int(c.volume) },
    ], withChg)));
    if (d.bars.length > 30) frag.append(el("p", { class: "warn", text: `Showing the last 30 of ${d.bars.length} bars; json intraday ${d.symbol} carries all.` }));
    frag.append(el("p", { class: "warn", text: `${source} VWAP is computed from the bars' typical prices, an approximation of IDX's average price.` }));
    return frag;
  },
  profile(p) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>${p.name || p.symbol}</b>` }));
    frag.append(kv([["Sector", p.sector], ["Industry", p.industry], ["Exchange", p.exchange], ["Market cap", fmt.compact(p.market_cap)], ["Shares outstanding", fmt.int(p.shares_outstanding)], ["Employees", fmt.int(p.employees)], ["Headquarters", [p.city, p.country].filter(Boolean).join(", ") || null], ["Website", p.website]]));
    if (p.summary) frag.append(el("p", { class: "prose", text: p.summary }));
    if (p.directors?.length) frag.append(el("p", { class: "prose", text: `Board and management: ${p.directors.slice(0, 8).map((d) => d.title ? `${d.name} (${d.title})` : d.name).join("; ")}${p.directors.length > 8 ? "; …" : ""}` }));
    return frag;
  },
  fundamentals(f) {
    const frag = document.createDocumentFragment();
    const v = f.valuation, p = f.profitability, h = f.health, g = f.growth, d = f.dividend, a = f.analyst;
    frag.append(el("p", { class: "prose", html: `<b>${f.name || f.symbol}</b> · ${f.sector || NA}${f.industry ? " / " + f.industry : ""} · price ${fmt.price(f.price)} · 52w ${fmt.price(f.week52_low)} to ${fmt.price(f.week52_high)} · beta ${fmt.ratio(f.beta)}` }));
    const grid = el("div", { class: "three" });
    const stmt = (title, pairs) => el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: title }), kv(pairs)]);
    grid.append(
      stmt("Valuation", [["Market cap", fmt.compact(v.market_cap)], ["Enterprise value", fmt.compact(v.enterprise_value)], ["P/E trailing", fmt.ratio(v.pe_trailing)], ["P/E forward", fmt.ratio(v.pe_forward)], ["PEG", fmt.ratio(v.peg)], ["P/B", mixedRatio(f, v.price_to_book)], ["P/S", mixedRatio(f, v.price_to_sales)], ["EV/EBITDA", mixedRatio(f, v.ev_to_ebitda)]]),
      stmt("Profitability & health", [["ROE", pct(p.roe_pct)], ["ROA", pct(p.roa_pct)], ["Gross margin", pct(p.gross_margin_pct)], ["Operating margin", pct(p.operating_margin_pct)], ["Net margin", pct(p.net_margin_pct)], ["Debt / equity", x(h.debt_to_equity)], ["Current ratio", fmt.ratio(h.current_ratio)], ["Total cash", inStatementCurrency(f, h.total_cash, fmt.compact(h.total_cash))]]),
      stmt("Growth, dividend, analysts", [["Revenue growth", fmt.pct(g.revenue_growth_pct)], ["Earnings growth", fmt.pct(g.earnings_growth_pct)], ["Dividend yield", pct(d.yield_pct)], ["Payout", pct(d.payout_ratio_pct)], ["Ex-dividend", d.ex_dividend_date], ["EPS (ttm)", fmt.price(f.per_share.eps_ttm)], ["Rating", a.recommendation ? a.recommendation.replace("_", " ") : null], ["Target mean", fmt.price(a.target_mean)]]),
    );
    frag.append(grid);
    if (mixedCurrency(f)) frag.append(el("p", { class: "warn", text: `* Statements in ${f.financial_currency}, price in ${f.currency}: P/B, P/S and EV multiples divide the price by ${f.financial_currency} figures, so they are not comparable, and the verdict does not score P/B.` }));
    if (f.verdict) {
      const vv = f.verdict;
      const cls = vv.label === "undervalued" ? "up" : vv.label === "overvalued" ? "dn" : "";
      frag.append(el("p", { class: "prose", style: "margin-top:.75rem" }, [el("span", { class: `pill ${cls}`, text: vv.label.replace("_", " ") }), ` score ${vv.score > 0 ? "+" : ""}${vv.score}, ${vv.inputs_available} input${vv.inputs_available === 1 ? "" : "s"} · heuristic from public ratios, not investment advice`]));
      frag.append(el("ul", { class: "reasons" }, vv.reasons.map((r) => el("li", { text: r }))));
    }
    frag.append(el("p", { class: "warn", text: `Sources: ${f.sources.join(", ")}` }));
    return frag;
  },
  valuation(v) {
    const m = (x) => mixedRatio(v, x);
    const after = mixedCurrency(v) ? [el("p", { class: "warn", text: `* Statements in ${v.financial_currency}, price in ${v.currency}: these multiples divide the price by ${v.financial_currency} figures, so they are shown as reported and not scored.` })] : [];
    return lensView(v, "valuation", [
      ["Multiples", [["P/E trailing", fmt.ratio(v.pe_trailing)], ["P/E forward", fmt.ratio(v.pe_forward)], ["PEG", fmt.ratio(v.peg)], ["P/B", m(v.price_to_book)], ["P/S", m(v.price_to_sales)], ["EV/EBITDA", m(v.ev_to_ebitda)], ["EV/Revenue", m(v.ev_to_revenue)]]],
      ["Price and income", [["Price", fmt.price(v.price)], ["Dividend yield", pct(v.dividend_yield_pct)], ["Payout", pct(v.payout_ratio_pct)], ["Negative equity", yesNo(v.negative_equity)], ["Financial company", yesNo(v.financial_company)]]],
      ["Analysts", [["Target mean", fmt.price(v.target_mean)], ["Analysts", fmt.int(v.analysts)], ["Upside", fmt.pct(v.target_upside_pct)]]],
    ], "points favour cheapness", after);
  },
  growth(g) {
    const chg = (k) => ({ h: "Change", f: (y) => fmt.pct(y[k]), cls: (y) => `num ${signClass(y[k])}` });
    const after = g.annual.length ? [wrapScroll(table([
      { h: "Fiscal year", f: (y) => `FY${y.end_date.slice(0, 4)}`, l: true }, { h: `Revenue${g.statements_currency ? ` (${g.statements_currency})` : ""}`, f: (y) => fmt.compact(y.revenue) }, chg("revenue_change_pct"),
      { h: "Net income", f: (y) => fmt.compact(y.net_income) }, chg("net_income_change_pct"),
    ], g.annual))] : [];
    return lensView(g, "growth", [
      ["Latest quarter (year on year)", [["Revenue growth", fmt.pct(g.revenue_growth_pct)], ["Earnings growth", fmt.pct(g.earnings_growth_pct)], ["Quarter end", g.most_recent_quarter]]],
      ["Earnings per share", [["EPS (ttm)", fmt.price(g.eps_ttm)], ["EPS (forward)", fmt.price(g.eps_forward)], ["Forward vs trailing", fmt.pct(g.eps_forward_growth_pct)]]],
      ["Multi-year", [["Revenue CAGR", fmt.pct(g.revenue_cagr_pct)], ["Net income CAGR", fmt.pct(g.net_income_cagr_pct)], ["Revenue 5y (source)", fmt.pct(g.revenue_cagr_5y_pct)], ["EPS 3y (source)", fmt.pct(g.eps_growth_3y_pct)]]],
    ], "points favour growth", after);
  },
  risk(r) {
    const p = r.price, dd = p?.max_drawdown;
    return lensView(r, "risk", [
      ["Balance sheet", [["Negative equity", yesNo(r.negative_equity)], ["Debt / equity", x(r.debt_to_equity)], ["Current ratio", fmt.ratio(r.current_ratio)], ["Quick ratio", fmt.ratio(r.quick_ratio)], ["Financial company", yesNo(r.financial_company)], ["Net margin", pct(r.net_margin_pct)], ["EPS (ttm)", fmt.price(r.eps_ttm)]]],
      [`Price vs ${p?.benchmark || "^JKSE"} (1y daily)`, p ? [["Window", `${p.from} to ${p.to}`], ["Daily returns", fmt.int(p.observations)], ["Zero returns", p.zero_returns == null ? null : `${fmt.int(p.zero_returns)} of ${fmt.int(p.observations)}`], ["Beta", fmt.ratio(p.beta)], ["Correlation", fmt.ratio(p.correlation)]] : [["Daily candles", null]]],
      ["Volatility and falls", p ? [["Volatility", pct(p.volatility_pct)], ["IHSG volatility", pct(p.benchmark_volatility_pct)], ["Largest fall", dd ? pct(dd.pct) : "none"], ["Peak to trough", dd ? `${dd.peak_date} to ${dd.trough_date}` : null], ["IHSG largest fall", pct(p.benchmark_max_drawdown_pct)]] : []],
    ], "points add risk");
  },
  technical(t) {
    const frag = document.createDocumentFragment();
    const ma = t.moving_averages;
    const vs = (v) => (v == null ? NA : `${fmt.price(v)} (${fmt.pct(v ? ((t.price - v) / v) * 100 : null)})`);
    const cls = t.signal.label === "buy" ? "up" : t.signal.label === "sell" ? "dn" : "";
    frag.append(el("p", { class: "prose", html: `<b>${t.name || t.symbol}</b> · ${t.candles_used} candles (${t.period}) · as of ${fmt.date(t.as_of)} · price ${fmt.price(t.price)} · 1d ${fmt.pct(t.change_pct_1d)} · 1m ${fmt.pct(t.change_pct_1m)} · 3m ${fmt.pct(t.change_pct_3m)} · 52w ${fmt.price(t.low_52w)} to ${fmt.price(t.high_52w)}` }));
    frag.append(el("p", { class: "prose" }, [el("span", { class: `pill ${cls}`, text: `${t.signal.label} ${t.signal.score > 0 ? "+" : ""}${t.signal.score}` }), ` · trend ${t.trend}`]));
    const grid = el("div", { class: "three" });
    const stmt = (title, pairs) => el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: title }), kv(pairs)]);
    grid.append(
      stmt("Momentum", [["RSI (14)", fmt.ratio(t.rsi14)], ["MACD line", t.macd ? fmt.num(t.macd.line, 2) : null], ["MACD signal", t.macd ? fmt.num(t.macd.signal, 2) : null], ["Histogram", t.macd ? fmt.num(t.macd.histogram, 2) : null], ["ATR (14)", fmt.price(t.atr14)], ["Volume / 20d", t.volume.ratio == null ? null : `${fmt.num(t.volume.ratio, 2)}x`]]),
      stmt("Averages (price vs)", [["SMA 20", vs(ma.sma20)], ["SMA 50", vs(ma.sma50)], ["SMA 200", vs(ma.sma200)], ["EMA 20", vs(ma.ema20)], ["EMA 50", vs(ma.ema50)], ["EMA 200", vs(ma.ema200)]]),
      stmt("Bands & levels", [["Bollinger lower", t.bollinger ? fmt.price(t.bollinger.lower) : null], ["Bollinger upper", t.bollinger ? fmt.price(t.bollinger.upper) : null], ["%B", t.bollinger ? fmt.num(t.bollinger.percent_b, 2) : null], ["Support", t.levels.support.map(fmt.price).join(" · ") || null], ["Resistance", t.levels.resistance.map(fmt.price).join(" · ") || null]]),
    );
    frag.append(grid);
    frag.append(el("ul", { class: "reasons" }, t.signal.reasons.map((r) => el("li", { text: r }))));
    frag.append(el("p", { class: "warn", text: "Rule-based read of daily candles, not investment advice." }));
    return frag;
  },
  financials(f, only = null) {
    const cols = [{ h: "", k: "label", l: true }, ...f.periods.map((p, i) => ({ h: f.period === "annual" ? `FY${p.end_date.slice(0, 4)}` : p.end_date, f: (r) => r.vals[i], cls: "num" }))];
    const lines = [
      ["Total revenue", "income", "total_revenue"], ["Gross profit", "income", "gross_profit"], ["Operating income", "income", "operating_income"], ["EBITDA", "income", "ebitda"],
      ["Net interest income", "income", "net_interest_income"], ["Pretax income", "income", "pretax_income"], ["Net income", "income", "net_income"], ["EPS (diluted)", "income", "eps_diluted", "price"],
      ["Total assets", "balance", "total_assets"], ["Total liabilities", "balance", "total_liabilities"], ["Shareholders' equity", "balance", "stockholders_equity"], ["Cash & equivalents", "balance", "cash_and_equivalents"], ["Total debt", "balance", "total_debt"],
      ["Operating cash flow", "cash_flow", "operating"], ["Capital expenditure", "cash_flow", "capital_expenditure"], ["Free cash flow", "cash_flow", "free_cash_flow"], ["Dividends paid", "cash_flow", "dividends_paid"],
    ].filter(([, s]) => !only || only.has(s));
    const rows = lines.map(([label, s, k, kind]) => ({ label, vals: f.periods.map((p) => (kind === "price" ? fmt.price(p[s][k]) : fmt.compact(p[s][k]))) })).filter((r) => r.vals.some((v) => v !== NA));
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", text: `${f.symbol} · ${f.period} statements · ${f.periods.length} periods · ${f.currency || ""} · T = trillion, B = billion` }));
    frag.append(wrapScroll(table(cols, rows)));
    return frag;
  },
  earnings(e) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>${e.name || e.symbol}</b> · EPS in ${e.currency || "IDR"}${e.scope ? ` · ${scopeLabel(e.scope)}` : ""}` }));
    if (e.quarters.length) frag.append(wrapScroll(table([{ h: "Quarter", k: "period", l: true, cls: "sym" }, { h: "Period end", f: (q) => fmt.date(q.end_date), l: true }, { h: "Reported", f: (q) => fmt.date(q.reported_date), l: true }, { h: "EPS actual", f: (q) => fmt.price(q.eps_actual) }, { h: "EPS est.", f: (q) => fmt.price(q.eps_estimate) }, { h: "Surprise", f: (q) => fmt.pct(q.surprise_pct), cls: (q) => `num ${signClass(q.surprise_pct)}` }], e.quarters)));
    const re = [...e.yearly, ...e.quarterly];
    if (re.length) frag.append(wrapScroll(table([{ h: "Period", k: "period", l: true, cls: "sym" }, { h: "Revenue", f: (r) => fmt.compact(r.revenue) }, { h: "Earnings", f: (r) => fmt.compact(r.earnings) }, { h: "Margin", f: (r) => pct(r.profit_margin_pct) }], re)));
    const est = e.estimates || [];
    if (est.length) frag.append(wrapScroll(table([
      { h: "Consensus", f: (r) => `${ESTIMATE_NAMES[r.period] || r.period} (${r.period})`, l: true, cls: "sym" }, { h: "Period end", f: (r) => fmt.date(r.end_date), l: true },
      { h: "EPS avg", f: (r) => fmt.price(r.eps_avg) }, { h: "EPS low", f: (r) => fmt.price(r.eps_low) }, { h: "EPS high", f: (r) => fmt.price(r.eps_high) }, { h: "Analysts", f: (r) => fmt.int(r.eps_analysts) },
      { h: "EPS growth", f: (r) => fmt.pct(r.eps_growth_pct), cls: (r) => `num ${signClass(r.eps_growth_pct)}` }, { h: "Revenue avg", f: (r) => fmt.compact(r.revenue_avg) },
      { h: "Revenue growth", f: (r) => fmt.pct(r.revenue_growth_pct), cls: (r) => `num ${signClass(r.revenue_growth_pct)}` },
    ], est)));
    if (e.next) frag.append(el("p", { class: "prose", text: `Next earnings ${e.next.date || NA}${e.next.date_is_estimate ? " (estimated)" : ""} · EPS consensus ${fmt.price(e.next.eps_avg)} (${fmt.price(e.next.eps_low)} to ${fmt.price(e.next.eps_high)}, ${e.next.analysts ?? NA} analysts) · revenue consensus ${fmt.compact(e.next.revenue_avg)}` }));
    if (e.ex_dividend_date) frag.append(el("p", { class: "prose", text: `Ex-dividend ${e.ex_dividend_date} · payment ${e.dividend_date || NA}` }));
    return frag;
  },
  compare(list) {
    const cols = [{ h: "Metric", k: "label", l: true }, ...list.map((f, i) => ({ h: f.symbol, f: (r) => r.vals[i], cls: "num" }))];
    const rowsDef = [
      ["Price", (f) => fmt.price(f.price)], ["Market cap", (f) => fmt.compact(f.valuation.market_cap)], ["P/E (ttm)", (f) => fmt.ratio(f.valuation.pe_trailing)], ["P/E (fwd)", (f) => fmt.ratio(f.valuation.pe_forward)],
      ["P/B", (f) => mixedRatio(f, f.valuation.price_to_book)], ["EV/EBITDA", (f) => mixedRatio(f, f.valuation.ev_to_ebitda)], ["ROE", (f) => pct(f.profitability.roe_pct)], ["Net margin", (f) => pct(f.profitability.net_margin_pct)],
      ["Debt/equity", (f) => x(f.health.debt_to_equity)], ["Rev growth", (f) => fmt.pct(f.growth.revenue_growth_pct)], ["Div yield", (f) => pct(f.dividend.yield_pct)], ["Target (mean)", (f) => fmt.price(f.analyst.target_mean)],
      ["Verdict", (f) => f.verdict?.label.replace("_", " ") || NA],
    ];
    const t = wrapScroll(table(cols, rowsDef.map(([label, fn]) => ({ label, vals: list.map(fn) }))));
    const mixed = list.filter(mixedCurrency).map((f) => `${f.symbol} (statements in ${f.financial_currency}, price in ${f.currency})`);
    if (!mixed.length) return t;
    const frag = document.createDocumentFragment();
    frag.append(t, el("p", { class: "warn", text: `* P/B and EV/EBITDA divide the price by statement figures in another currency, so they are not comparable: ${mixed.join("; ")}.` }));
    return frag;
  },
  screen(s) {
    const frag = document.createDocumentFragment();
    const what = [s.preset, s.filters.join(", ")].filter(Boolean).join(": ") || "no filters";
    frag.append(el("p", { class: "prose", text: `${what} · ${s.quoted} of ${s.universe} quoted · ${s.matched_quote_level} passed quote filters${s.fundamentals_checked ? ` · fundamentals checked for ${s.fundamentals_checked}${s.fundamentals_capped ? " (capped)" : ""}` : ""} · ${s.total_matches} match${s.total_matches === 1 ? "" : "es"} · sorted by ${s.sort} ${s.descending ? "desc" : "asc"}` }));
    const withF = s.rows.some((r) => r.fundamentals_checked);
    const keyOf = (clause) => (clause.match(/^\s*([^<>=!]+?)\s*(<=|>=|!=|<|>|=)/) || [])[1]?.toLowerCase();
    const filterKeys = s.filters.map(keyOf);
    // One return column: the sort key's window, else the first one filtered on, else 1 year.
    const WINDOWS = { ret1w: ["1W %", "return_1w_pct"], ret1m: ["1M %", "return_1m_pct"], ret3m: ["3M %", "return_3m_pct"], ret6m: ["6M %", "return_6m_pct"], retytd: ["YTD %", "return_ytd_pct"], ytd: ["YTD %", "return_ytd_pct"], ret1y: ["1Y %", "return_1y_pct"] };
    const win = WINDOWS[s.sort] || WINDOWS[filterKeys.find((k) => WINDOWS[k])] || WINDOWS.ret1y;
    const withRange = s.sort === "range" || filterKeys.some((k) => ["range", "pos52", "52w"].includes(k));
    const cols = [
      { h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Name", k: "name", l: true, cls: "name" }, { h: "Price", f: (r) => fmt.price(r.price) },
      { h: "Chg %", f: (r) => fmt.pct(r.change_pct), cls: (r) => `num ${signClass(r.change_pct)}` }, { h: "Mkt Cap", f: (r) => fmt.compact(r.market_cap) },
      { h: "P/E", f: (r) => fmt.ratio(r.pe_trailing) }, { h: "P/B", f: (r) => mixedRatio(r, r.price_to_book) }, { h: "Div %", f: (r) => pct(r.dividend_yield_pct) },
    ];
    if (s.returns_source) cols.push({ h: win[0], f: (r) => fmt.pct(r[win[1]]), cls: (r) => `num ${signClass(r[win[1]])}` });
    if (withRange) cols.push({ h: "52w pos", f: (r) => fmt.num(r.range_52w_pct, 1) });
    if (withF) cols.push({ h: "ROE", f: (r) => pct(r.roe_pct) }, { h: "Net mgn", f: (r) => pct(r.net_margin_pct) }, { h: "D/E", f: (r) => x(r.debt_to_equity) }, { h: "Sector", k: "sector", l: true });
    frag.append(wrapScroll(table(cols, s.rows)));
    if (s.rows.length < s.total_matches) frag.append(el("p", { class: "warn", text: `Showing ${s.rows.length} of ${s.total_matches}; add --limit 0 for all.` }));
    if (s.returns_source) frag.append(el("p", { class: "warn", text: `Return windows: price return % as reported by ${s.returns_source}.` }));
    if (withRange) frag.append(el("p", { class: "warn", text: "52w pos: where the price sits in its 52-week range, 0 at the low, 100 at the high." }));
    const mixed = screenCurrencyNote(s.rows);
    if (mixed) frag.append(el("p", { class: "warn", text: mixed }));
    return frag;
  },
  insights(r, all) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose" }, [
      el("b", { text: r.symbol }), `${r.name ? ` · ${r.name}` : ""} · MSN insights${r.updated ? `, updated ${r.updated.slice(0, 16).replace("T", " ")} UTC` : ""}`,
    ]));
    frag.append(el("p", { class: "prose", text: `${r.good} good · ${r.bad} bad · ${r.neutral} neutral · ${r.not_rated} not rated${r.peers.length ? ` · peers ${r.peers.join(", ")}` : ""}` }));
    const MARK = { good: "✓", bad: "✗", neutral: "=", not_rated: "·" };
    const VS = { peers: "peers", industry: "industry", own_history: "own history", price: "price", threshold: "threshold" };
    const categories = [...new Set(r.insights.map((i) => i.category))];
    for (const cat of categories) {
      const items = r.insights.filter((i) => i.category === cat && (all || i.status !== "not_rated"));
      if (!items.length) continue;
      frag.append(el("h3", { class: "stmt-title", text: cat }));
      frag.append(wrapScroll(table([
        { h: "", f: (i) => MARK[i.status] || "·", cls: (i) => signClass(i.status === "good" ? 1 : i.status === "bad" ? -1 : 0) },
        { h: "Statement", f: (i) => i.statement || i.short_statement || i.name, l: true },
        { h: "vs", f: (i) => VS[i.compared_with] || fmt.text(null), l: true },
      ], items)));
    }
    if (!r.insights.length) frag.append(el("p", { class: "warn", text: "MSN lists no statements for this company." }));
    if (!all && r.not_rated) frag.append(el("p", { class: "warn", text: `${r.not_rated} statement${r.not_rated === 1 ? "" : "s"} MSN could not rate ${r.not_rated === 1 ? "is" : "are"} hidden; add --all.` }));
    frag.append(el("p", { class: "warn", text: "Rule-based statements by MSN Money (✓ good, ✗ bad, = neutral, · not rated). Not investment advice." }));
    return frag;
  },
  news(n) {
    const frag = document.createDocumentFragment();
    const company = n.items.filter((i) => i.scope === "company"), market = n.items.filter((i) => i.scope !== "company");
    if (n.symbol) {
      // Verb and noun follow their counts, as in the CLI's news table.
      const matches = n.company_matches ?? company.length;
      frag.append(el("p", { class: "prose", text: `${matches} of ${n.scanned} scanned headlines ${matches === 1 ? "mentions" : "mention"} ${n.symbol}${n.matched_name ? ` or ${n.matched_name}` : ""} · ${n.sources.join(", ")}` }));
      if (company.length) frag.append(newsTable(company)); else frag.append(el("p", { class: "warn", text: "No headline mentions the company in the current feeds." }));
      if (market.length) { frag.append(el("h3", { class: "stmt-title", text: `Market context · ${market.length} headline${market.length === 1 ? "" : "s"}` })); frag.append(newsTable(market)); }
    } else {
      frag.append(el("p", { class: "prose", text: `${n.items.length} of ${n.scanned} headlines · ${n.sources.join(", ")}` }));
      frag.append(newsTable(n.items));
    }
    frag.append(el("p", { class: "warn", text: "± is a lexicon read of the headline, not a recommendation." }));
    return frag;
  },
  dividends(h) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>${h.name || h.symbol}</b> · ${h.currency || "IDR"} · price ${fmt.price(h.price)} · TTM dividends ${h.ttm_total == null ? NA : fmt.num(h.ttm_total, 2)} (${h.ttm_payments} payment${h.ttm_payments === 1 ? "" : "s"}) · TTM yield ${pct(h.ttm_yield_pct)} · ${h.consecutive_years} consecutive year${h.consecutive_years === 1 ? "" : "s"}${h.last ? ` · last ${h.last.date} ${fmt.num(h.last.amount, 2)}` : ""}` }));
    if (!h.dividends.length) { frag.append(el("p", { class: "warn", text: "No dividend events reported by the source." })); return frag; }
    const grid = el("div", { class: "three" });
    grid.append(
      el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: "Yearly totals" }), wrapScroll(table([{ h: "Year", k: "year", l: true, cls: "sym" }, { h: "Payments", k: "payments" }, { h: "Total", f: (y) => fmt.num(y.total, 2) }, { h: "vs prev", f: (y) => fmt.pct(y.change_pct), cls: (y) => `num ${signClass(y.change_pct)}` }], h.years.slice(-10)))]),
      el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: `Payments · last 12 of ${h.dividends.length}` }), wrapScroll(table([{ h: "Ex-date", k: "date", l: true }, { h: "Amount", f: (d) => fmt.num(d.amount, 2) }], [...h.dividends].reverse().slice(0, 12)))]),
    );
    frag.append(grid);
    if (h.splits.length) frag.append(el("p", { class: "prose", text: `Splits · ${h.splits.map((s) => `${s.date} ${s.ratio}`).join(" · ")}` }));
    frag.append(el("p", { class: "warn", text: "Amounts as reported by the source (split-adjusted on Yahoo). TTM yield = TTM total / last price, computed here." }));
    return frag;
  },
  returns(list) {
    const frag = document.createDocumentFragment();
    const W = ["1w", "1m", "3m", "6m", "ytd", "1y", "3y", "5y"];
    const wc = (key) => W.map((w) => ({ h: w.toUpperCase(), f: (r) => fmt.pct(r[key]?.[w]), cls: (r) => `num ${signClass(r[key]?.[w])}` }));
    frag.append(el("p", { class: "prose", text: `Price returns · as of ${list[0].as_of} · five years of daily closes · dividends excluded` }));
    const rows = list.map((p) => ({ ...p, w: p.returns }));
    const bench = list.find((p) => p.benchmark)?.benchmark;
    if (bench) rows.push({ symbol: `${bench.symbol}${bench.name ? ` (${bench.name})` : ""}`, price: null, w: bench.returns, cagr_3y_pct: null, volatility_1y_pct: null, max_drawdown_1y: null });
    frag.append(wrapScroll(table([{ h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Price", f: (r) => fmt.price(r.price) }, ...wc("w"), { h: "CAGR 3y", f: (r) => fmt.pct(r.cagr_3y_pct), cls: (r) => `num ${signClass(r.cagr_3y_pct)}` }, { h: "Vol 1y", f: (r) => pct(r.volatility_1y_pct) }, { h: "MaxDD 1y", f: (r) => fmt.pct(r.max_drawdown_1y?.pct), cls: "num dn" }], rows)));
    if (bench) {
      frag.append(el("h3", { class: "stmt-title", text: `Excess over ${bench.symbol} · percentage points` }));
      frag.append(wrapScroll(table([{ h: "Symbol", k: "symbol", l: true, cls: "sym" }, ...wc("e")], list.filter((p) => p.benchmark).map((p) => ({ symbol: p.symbol, e: p.benchmark.excess })))));
    }
    for (const p of list) frag.append(el("p", { class: "prose", text: `${p.symbol} · 52w ${fmt.price(p.low_52w)} to ${fmt.price(p.high_52w)} (${fmt.pct(p.pct_from_52w_high)} from high${p.days_since_52w_high != null ? `, ${p.days_since_52w_high} days ago` : ""}) · CAGR 5y ${fmt.pct(p.cagr_5y_pct)} · best day ${p.best_day_1y ? `${fmt.pct(p.best_day_1y.pct)} on ${p.best_day_1y.date}` : NA} · worst day ${p.worst_day_1y ? `${fmt.pct(p.worst_day_1y.pct)} on ${p.worst_day_1y.date}` : NA} · max drawdown since ${p.start}: ${p.max_drawdown_all ? `${fmt.pct(p.max_drawdown_all.pct)} (${p.max_drawdown_all.peak_date} to ${p.max_drawdown_all.trough_date}${p.max_drawdown_all.recovered ? ", recovered" : ""})` : NA}` }));
    frag.append(el("p", { class: "warn", text: "Computed from provider closes; a window is n/a when the history does not reach back that far." }));
    return frag;
  },
  brief(b) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>${b.name || b.symbol}</b> · brief · ${fmt.wib(b.generated_at.replace("Z", "+00:00")).replace(" WIB", " UTC")}` }));
    frag.append(el("ul", { class: "reasons" }, b.key_points.map((k) => el("li", { text: k }))));
    if (b.fundamentals) { frag.append(el("h3", { class: "stmt-title", text: "Fundamentals" })); frag.append(R.fundamentals(b.fundamentals)); }
    if (b.technical) { frag.append(el("h3", { class: "stmt-title", text: "Technical" })); frag.append(R.technical(b.technical)); }
    if (b.returns) { frag.append(el("h3", { class: "stmt-title", text: "Returns" })); frag.append(R.returns([b.returns])); }
    if (b.ownership) { frag.append(el("h3", { class: "stmt-title", text: `Holders above 1 % as of ${b.ownership.as_of} · ${b.ownership.holders_listed} listed = ${pct(b.ownership.listed_pct)} · foreign ${pct(b.ownership.foreign_pct)}` })); frag.append(wrapScroll(table([{ h: "Holder", k: "holder", l: true, cls: "sym" }, { h: "Type", k: "investor_type", l: true }, { h: "L/A", k: "local_foreign", l: true }, { h: "%", f: (r) => pct(r.pct) }], b.ownership.top_holders))); }
    if (b.news?.headlines?.length) { frag.append(el("h3", { class: "stmt-title", text: `Headlines · ${b.news.company_matches} ${b.news.company_matches === 1 ? "mentions" : "mention"} the company · sentiment ${b.news.sentiment}` })); frag.append(newsTable(b.news.headlines)); }
    if (b.sections_failed.length) frag.append(el("p", { class: "warn", text: `Missing sections: ${b.sections_failed.join(", ")} (see warnings above).` }));
    frag.append(el("p", { class: "warn", text: "Verdicts, signals and sentiment are transparent rule scores, not investment advice." }));
    return frag;
  },
  foreign(r) {
    const frag = document.createDocumentFragment();
    if (r.points) {
      const last = r.points[r.points.length - 1];
      frag.append(el("p", { class: "prose", html: `<b>${r.symbol}</b> · ${r.issuer_name || ""} · disclosed foreign stake ${pct(last.foreign_pct)} of the ${pct(last.listed_pct)} listed above 1 % as of ${last.as_of} · ${r.points.length} release${r.points.length === 1 ? "" : "s"}` }));
      frag.append(wrapScroll(table([{ h: "As of", k: "as_of", l: true, cls: "sym" }, { h: "Foreign %", f: (p) => pct(p.foreign_pct) }, { h: "Δ pp", f: (p) => fmt.change(p.foreign_change_pp), cls: (p) => `num ${signClass(p.foreign_change_pp)}` }, { h: "Local %", f: (p) => pct(p.local_pct) }, { h: "Foreign holders", k: "foreign_holders" }, { h: "Local holders", k: "local_holders" }, { h: "Listed %", f: (p) => pct(p.listed_pct) }], r.points)));
    } else {
      frag.append(el("p", { class: "prose", text: `Disclosed foreign stake ${r.from} to ${r.to} · ${r.issuers} issuers · ${r.inflows} inflows · ${r.outflows} outflows · net ${fmt.change(r.net_pp)} pp` }));
      frag.append(wrapScroll(table([{ h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Issuer", k: "issuer_name", l: true, cls: "name" }, { h: "Foreign % from", f: (x) => pct(x.foreign_pct_from) }, { h: "Foreign % to", f: (x) => pct(x.foreign_pct_to) }, { h: "Δ pp", f: (x) => fmt.change(x.change_pp), cls: (x) => `num ${signClass(x.change_pp)}` }, { h: "Foreign holders", k: "foreign_holders_to" }], r.rows)));
    }
    frag.append(el("p", { class: "warn", text: "Sums over holders above 1 % only (KSEI L/A flag); holders below 1 % are not published." }));
    return frag;
  },
  bandar(list) {
    const frag = document.createDocumentFragment();
    const SIG = { big_accumulation: "big acc", accumulation: "acc", neutral: "neutral", distribution: "dist", big_distribution: "big dist" };
    const sigCls = (g) => (g.endsWith("accumulation") ? "up" : g.endsWith("distribution") ? "dn" : "");
    const p1 = (v) => (v == null ? NA : `${v > 0 ? "+" : ""}${v.toFixed(1)}%`);
    const net = (v) => `${v > 0 ? "+" : ""}${fmt.compact(v)}`;
    const KIND = { local: "local", foreign: "foreign", state_owned: "BUMN" };
    const side = (title, rows) => el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: title }), wrapScroll(table([
      { h: "Code", k: "code", l: true, cls: "sym" }, { h: "Broker", f: (r) => r.name, l: true, cls: "name" }, { h: "Class", f: (r) => KIND[r.kind] || NA, l: true },
      { h: "Lots", f: (r) => fmt.compact(r.lots) }, { h: "Value", f: (r) => fmt.compact(r.value) }, { h: "Avg", f: (r) => fmt.price(r.average_price) },
    ], rows))]);
    for (const r of list) {
      const range = r.from === r.to ? r.from : `${r.from} to ${r.to}`;
      frag.append(el("p", { class: "prose" }, [el("b", { text: r.symbol }), `${r.name ? ` · ${r.name}` : ""} · ${range} (${r.period}) · ${r.investor} investors · ${r.board === "all" ? "all boards" : `${r.board} board`}`]));
      if (!(r.total_value > 0)) { frag.append(el("p", { class: "warn", text: `No trades from ${r.from} to ${r.to}.` })); continue; }
      frag.append(el("p", { class: "prose" }, [
        el("span", { class: `pill ${sigCls(r.signal)}`, text: r.signal.replace("_", " ") }),
        ` top ${r.top} net `, el("span", { class: signClass(r.net_value), text: net(r.net_value) }), ` (${p1(r.net_pct)} of ${fmt.compact(r.total_value)} traded) · phase ${r.phase ? r.phase.replace("_", " ") : NA} · price ${fmt.price(r.price)} (`,
        el("span", { class: signClass(r.price_change_pct), text: fmt.pct(r.price_change_pct) }), ")",
      ]));
      frag.append(el("p", { class: "prose" }, [
        `Top ${r.top} buyers' average ${fmt.price(r.bandar_average == null ? null : Math.round(r.bandar_average))} (`, el("span", { class: signClass(r.bandar_gain_pct), text: p1(r.bandar_gain_pct) }),
        " vs price) · foreign net ", el("span", { class: signClass(r.foreign_net_value), text: net(r.foreign_net_value) }), ` (${p1(r.foreign_net_pct)})`,
      ]));
      frag.append(wrapScroll(table([
        { h: "Top", f: (t) => (t.top === r.top ? el("b", { text: String(t.top) }) : String(t.top)), l: true },
        { h: "Buy value", f: (t) => fmt.compact(t.buy_value) }, { h: "Sell value", f: (t) => fmt.compact(t.sell_value) },
        { h: "Net value", f: (t) => net(t.net_value), cls: (t) => `num ${signClass(t.net_value)}` }, { h: "Net lots", f: (t) => net(t.net_lots), cls: (t) => `num ${signClass(t.net_lots)}` },
        { h: "Net %", f: (t) => p1(t.net_pct), cls: (t) => `num ${signClass(t.net_pct)}` }, { h: "Buy share", f: (t) => (t.buy_share_pct == null ? NA : `${t.buy_share_pct.toFixed(1)}%`) },
        { h: "Buy avg", f: (t) => fmt.price(t.buy_average == null ? null : Math.round(t.buy_average)) }, { h: "Sell avg", f: (t) => fmt.price(t.sell_average == null ? null : Math.round(t.sell_average)) },
        { h: "Signal", f: (t) => SIG[t.signal], cls: (t) => sigCls(t.signal) },
      ], r.tiers)));
      const grid = el("div", { class: "two" });
      grid.append(side(`Top ${r.top} buyers`, r.buyers), side(`Top ${r.top} sellers`, r.sellers));
      frag.append(grid);
      if (r.daily) {
        const d = r.daily;
        frag.append(el("h3", { class: "stmt-title", text: `Bandar movement · cumulative top ${r.top} net` }));
        if (d.days.length > 1) frag.append(sparkline(d.days.map((x, i) => [i, x.cumulative_net_value]), { cls: "spark" }));
        frag.append(wrapScroll(table([
          { h: "Date", k: "date", l: true }, { h: "Value", f: (x) => fmt.compact(x.total_value) },
          { h: `Top ${r.top} net`, f: (x) => net(x.net_value), cls: (x) => `num ${signClass(x.net_value)}` }, { h: "Net %", f: (x) => p1(x.net_pct), cls: (x) => `num ${signClass(x.net_pct)}` },
          { h: "Signal", f: (x) => SIG[x.signal], cls: (x) => sigCls(x.signal) }, { h: "Cumulative", f: (x) => net(x.cumulative_net_value), cls: (x) => `num ${signClass(x.cumulative_net_value)}` },
          { h: "Foreign net", f: (x) => net(x.foreign_net_value), cls: (x) => `num ${signClass(x.foreign_net_value)}` },
        ], d.days)));
        const streak = d.streak === 0 ? "the latest session was neutral" : `${Math.abs(d.streak)} ${d.streak > 0 ? "accumulation" : "distribution"} session${Math.abs(d.streak) === 1 ? "" : "s"} in a row`;
        frag.append(el("p", { class: "prose", text: `${d.accumulation_days} accumulation · ${d.distribution_days} distribution · ${d.neutral_days} neutral sessions · ${streak}` }));
      }
      frag.append(el("ul", { class: "reasons" }, r.reasons.map((x) => el("li", { text: x }))));
    }
    frag.append(el("p", { class: "warn", text: BANDAR_NOTE }));
    return frag;
  },
  bandarScreen(s) {
    const frag = document.createDocumentFragment();
    const SIG = { big_accumulation: "big acc", accumulation: "acc", neutral: "neutral", distribution: "dist", big_distribution: "big dist" };
    const sigCls = (g) => (g.endsWith("accumulation") ? "up" : g.endsWith("distribution") ? "dn" : "");
    const p1 = (v) => (v == null ? NA : `${v > 0 ? "+" : ""}${v.toFixed(1)}%`);
    frag.append(el("p", { class: "prose" }, [el("b", { text: `Bandar screen${s.preset ? ` · ${s.preset}` : ""}` }), ` · ${s.period} ${s.from} to ${s.to} · top ${s.top} · scanned ${s.scanned} · ${s.checked} with trades · ${s.total_matches} match${s.total_matches === 1 ? "" : "es"}`]));
    frag.append(el("p", { class: "prose", text: `Filters: ${s.filters.length ? s.filters.join(", ") : "none"} · sorted by ${s.sort} ${s.descending ? "descending" : "ascending"}` }));
    if (!s.rows.length) frag.append(el("p", { class: "warn", text: "No stock matched." }));
    else frag.append(wrapScroll(table([
      { h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Signal", f: (r) => SIG[r.signal], cls: (r) => sigCls(r.signal) },
      { h: "Net %", f: (r) => p1(r.net_pct), cls: (r) => `num ${signClass(r.net_pct)}` }, { h: "Net value", f: (r) => `${r.net_value > 0 ? "+" : ""}${fmt.compact(r.net_value)}`, cls: (r) => `num ${signClass(r.net_value)}` },
      { h: "Chg %", f: (r) => fmt.pct(r.price_change_pct), cls: (r) => `num ${signClass(r.price_change_pct)}` }, { h: "Phase", f: (r) => (r.phase ? r.phase.replace("_", " ") : NA), l: true },
      { h: "Foreign %", f: (r) => p1(r.foreign_net_pct), cls: (r) => `num ${signClass(r.foreign_net_pct)}` },
      { h: "Bandar avg", f: (r) => fmt.price(r.bandar_average == null ? null : Math.round(r.bandar_average)) }, { h: "Gain", f: (r) => p1(r.bandar_gain_pct), cls: (r) => `num ${signClass(r.bandar_gain_pct)}` },
      { h: "Price", f: (r) => fmt.price(r.price) }, { h: "Top buyers", f: (r) => r.top_buyers.join(" "), l: true },
    ], s.rows)));
    if (s.rows.length < s.total_matches) frag.append(el("p", { class: "warn", text: `Showing ${s.rows.length} of ${s.total_matches} matches; --limit 0 for all.` }));
    frag.append(el("p", { class: "warn", text: `${BANDAR_NOTE} The web screen reads the Worker's scan of the last session's 100 most traded stocks, which from 16:45 WIB waits for IPOT to publish that session and then reads it; the warnings say when an older session is shown. The CLI scans live.` }));
    return frag;
  },
  broker(list) {
    const frag = document.createDocumentFragment();
    const KIND = { local: "local", foreign: "foreign", state_owned: "BUMN" };
    const cols = [
      { h: "#", k: "rank" },
      { h: "Code", k: "code", l: true, cls: "sym" },
      { h: "Broker", f: (r) => r.name, l: true, cls: "name" },
      { h: "Class", f: (r) => KIND[r.kind] || NA, l: true },
      { h: "Lots", f: (r) => fmt.compact(r.lots) },
      { h: "Value", f: (r) => fmt.compact(r.value) },
      { h: "Avg", f: (r) => fmt.price(r.average_price) },
    ];
    for (const s of list) {
      const range = s.from === s.to ? s.from : `${s.from} to ${s.to}`;
      frag.append(el("p", { class: "prose", html: `<b>${s.symbol}</b>${s.name ? ` · ${s.name}` : ""} · ${range} · ${s.investor} investors · ${s.board === "all" ? "all boards" : `${s.board} board`}` }));
      if (!s.buyers.length && !s.sellers.length) { frag.append(el("p", { class: "warn", text: `No trades from ${s.from} to ${s.to}.` })); continue; }
      // One investor side: the source's totals are that side's buying, so it reads as bought and sold.
      const totals = s.investor === "all"
        ? `Value ${fmt.compact(s.total_value)} · lots ${fmt.compact(s.total_lots)} · avg ${fmt.price(s.average_price)}`
        : `${s.investor === "foreign" ? "Foreign" : "Domestic"} investors bought ${fmt.compact(s.buy_value)} · sold ${fmt.compact(s.sell_value)}`;
      frag.append(el("p", { class: "prose", html: `${totals} · foreign net <span class="${signClass(s.foreign_net_value)}">${fmt.compact(s.foreign_net_value)}</span>` }));
      const grid = el("div", { class: "two" });
      grid.append(
        el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: "Top buyers" }), wrapScroll(table(cols, s.buyers))]),
        el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: "Top sellers" }), wrapScroll(table(cols, s.sellers))]),
      );
      frag.append(grid);
    }
    frag.append(el("p", { class: "warn", text: `Source: IPOT broker summary, top 10 buyers and sellers ranked by lots. One lot = 100 shares.${list.some((s) => s.rounded) ? " The source rounds figures of a million and more to one decimal." : ""}${list.some((s) => s.board !== "all") ? " The source's foreign net covers every board, so a board filter leaves it out." : ""}` }));
    return frag;
  },
  foreignFlow(fl) {
    const frag = document.createDocumentFragment();
    const signed = (v) => (v > 0 ? `+${fmt.compact(v)}` : fmt.compact(v));
    const share = (v) => (v == null ? null : `${fmt.num(v, 1)}%`);
    const t = fl.totals;
    frag.append(el("p", { class: "prose", html: `<b>${fl.symbol}</b>${fl.name ? ` · ${fl.name}` : ""} · foreign flow (IPOT, all boards) · ${fl.from} to ${fl.to} · ${t.sessions} sessions` }));
    frag.append(wrapScroll(table([
      { h: "Date", k: "date", l: true },
      { h: "Value", f: (d) => fmt.compact(d.value) },
      { h: "Avg", f: (d) => fmt.price(d.average_price) },
      { h: "Foreign buy", f: (d) => fmt.compact(d.foreign_buy_value) },
      { h: "Foreign sell", f: (d) => fmt.compact(d.foreign_sell_value) },
      { h: "Foreign net", f: (d) => signed(d.foreign_net_value), cls: (d) => `num ${signClass(d.foreign_net_value)}` },
      { h: "Foreign %", f: (d) => share(d.foreign_share_pct) },
    ], [...fl.days].reverse())));
    const streak = t.streak === 0 ? "flat last session" : t.streak > 0 ? `${t.streak} net-buy session(s) in a row` : `${-t.streak} net-sell session(s) in a row`;
    frag.append(el("p", { class: "prose", html: `Total · value ${fmt.compact(t.value)} · foreign buy ${fmt.compact(t.foreign_buy_value)} · sell ${fmt.compact(t.foreign_sell_value)} · net <span class="${signClass(t.foreign_net_value)}">${signed(t.foreign_net_value)}</span> · foreign share ${share(t.foreign_share_pct) ?? fmt.pctPlain(null)} · ${t.net_buy_sessions} net-buy / ${t.net_sell_sessions} net-sell sessions · ${streak}` }));
    frag.append(el("p", { class: "warn", text: `Rupiah across all boards (negotiated trades included) from IPOT's broker summary, two requests per session; foreign % is foreign buying plus selling over twice the value traded.${fl.rounded ? " The source rounds figures of a million and more to one decimal." : ""} The CLI also reads idx.co.id's share counts (--source idx).` }));
    return frag;
  },
  actions(c) {
    const frag = document.createDocumentFragment();
    const KIND = { cash_dividend: "cash dividend", stock_dividend: "stock dividend", stock_split: "stock split", reverse_split: "reverse split", bonus: "bonus shares", rights_issue: "rights issue", warrant: "warrant", other: "other" };
    frag.append(el("p", { class: "prose", html: `<b>${c.symbol}</b>${c.name ? ` · ${c.name}` : ""} · since ${c.since}` }));
    frag.append(el("h3", { class: "stmt-title", text: "Corporate actions" }));
    if (!c.actions.length) frag.append(el("p", { class: "warn", text: "None in this period." }));
    else frag.append(wrapScroll(table([
      { h: "Action", f: (a) => KIND[a.kind] || a.label, l: true, cls: "sym" },
      { h: "Amount", f: (a) => fmt.price(a.amount) },
      { h: "Ratio", f: (a) => (a.ratio_old != null && a.ratio_new != null ? `${fmt.price(a.ratio_old)}:${fmt.price(a.ratio_new)}` : NA) },
      { h: "Cum", f: (a) => a.cum_date || NA, l: true }, { h: "Ex", f: (a) => a.ex_date || NA, l: true },
      { h: "Recording", f: (a) => a.recording_date || NA, l: true }, { h: "Payment", f: (a) => a.distribution_date || NA, l: true },
      { h: "Status", f: (a) => a.status || NA, l: true },
    ], c.actions)));
    frag.append(el("h3", { class: "stmt-title", text: "Shareholders' meetings (RUPS)" }));
    if (!c.meetings.length) frag.append(el("p", { class: "warn", text: "None in this period." }));
    else frag.append(wrapScroll(table([
      { h: "Date", f: (m) => m.date || NA, l: true, cls: "sym" }, { h: "Time", f: (m) => m.time || NA, l: true },
      { h: "Status", f: (m) => m.status || NA, l: true }, { h: "Venue", f: (m) => m.venue || NA, l: true, cls: "name" },
    ], c.meetings)));
    if (c.warrants_rights.length) {
      frag.append(el("h3", { class: "stmt-title", text: "Warrants and rights" }));
      frag.append(wrapScroll(table([
        { h: "Code", k: "code", l: true, cls: "sym" }, { h: "Type", k: "kind", l: true },
        { h: "Exercise", f: (w) => fmt.price(w.exercise_price) }, { h: "Issued", f: (w) => fmt.compact(w.shares) },
        { h: "Listing", f: (w) => w.listing_date || NA, l: true }, { h: "Maturity", f: (w) => w.maturity_date || NA, l: true },
        { h: "Status", f: (w) => w.status || NA, l: true },
      ], c.warrants_rights)));
    }
    frag.append(el("p", { class: "warn", text: "Amount: IDR per share (the dividend, or a rights issue's exercise price). Ratio: old:new. Source: IPOT; json actions … carries each announcement and RUPS agenda." }));
    return frag;
  },
  instruments(l) {
    const frag = document.createDocumentFragment();
    const shown = l.instruments.slice(0, 200);
    frag.append(el("p", { class: "prose", text: `${l.instruments.length} of ${l.total} listed stocks${l.search ? ` · matching "${l.search}"` : ""}` }));
    if (shown.length) frag.append(wrapScroll(table([{ h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Name", f: (i) => i.name || NA, l: true, cls: "name" }], shown)));
    if (l.instruments.length > shown.length) frag.append(el("p", { class: "warn", text: `Showing the first ${shown.length}; narrow with --search or use json list for all.` }));
    return frag;
  },
  members(l) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", text: `${l.members.length} of ${l.total} exchange members${l.search ? ` · matching "${l.search}"` : ""}` }));
    if (l.members.length) frag.append(wrapScroll(table([{ h: "Code", k: "code", l: true, cls: "sym" }, { h: "Member", f: (m) => m.name || NA, l: true, cls: "name" }], l.members)));
    return frag;
  },
  bandarBroker(a) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>${a.code}</b>${a.name ? ` · ${a.name}` : ""} · ${a.period} (${a.from} to ${a.to}) · ${a.scanned} scanned, ${a.checked} with trades` }));
    frag.append(el("p", { class: "prose", text: `Visible buying ${fmt.compact(a.bought_value)} in ${a.bought.length} stocks · visible selling ${fmt.compact(a.sold_value)} in ${a.sold.length} stocks` }));
    const cols = [
      { h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Name", f: (r) => r.name || NA, l: true, cls: "name" },
      { h: "Rank", k: "rank" }, { h: "Lots", f: (r) => fmt.compact(r.lots) }, { h: "Value", f: (r) => fmt.compact(r.value) },
      { h: "Avg", f: (r) => fmt.price(r.average_price) }, { h: "Share", f: (r) => (r.share_pct == null ? NA : `${fmt.num(r.share_pct, 1)}%`) },
      { h: "Other side", f: (r) => fmt.compact(r.opposite_value) }, { h: "Net", f: (r) => fmt.compact(r.net_value), cls: (r) => `num ${signClass(r.net_value)}` },
    ];
    const grid = el("div", { class: "two" });
    grid.append(
      el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: "Top-ten buyer of" }), a.bought.length ? wrapScroll(table(cols, a.bought)) : el("p", { class: "warn", text: "Not among the top ten buyers of any scanned stock." })]),
      el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: "Top-ten seller of" }), a.sold.length ? wrapScroll(table(cols, a.sold)) : el("p", { class: "warn", text: "Not among the top ten sellers of any scanned stock." })]),
    );
    frag.append(grid);
    frag.append(el("p", { class: "warn", text: "From the Worker's scan of the last session's 100 most traded stocks (IPOT broker summary), read once IPOT publishes that session. Only the top ten per side are published; net shows where both sides are visible." }));
    return frag;
  },
  tools() {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", text: `${TOOLS.length} MCP tools (idx mcp). ${TOOLS.filter((t) => t.web).length} run here; the other ${TOOLS.filter((t) => !t.web).length} (the idx.co.id exchange tools, the KSEI composition tool, the local ownership database tool, MSN's institutional holders and the Stockbit tools) run in the CLI and MCP only. Click a tool to run it, or to see its CLI command.` }));
    frag.append(wrapScroll(table([
      { h: "Tool", k: "tool", l: true, cls: "sym" }, { h: "CLI", k: "cli", l: true },
      { h: "Web terminal", f: (t) => t.web || "CLI and MCP only", l: true }, { h: "What", k: "what", l: true, cls: "name" },
    ], TOOLS)));
    frag.append(toolButtons());
    return frag;
  },
  // A line this page cannot run (route.js): what it is, why it runs elsewhere, the
  // CLI line in the user's words, how to get the CLI, and what runs here instead.
  notice(p) {
    const frag = document.createDocumentFragment();
    const words = (x) => (x ? t(x.id, x.en) : "");
    if (p.kind === "flag") {
      frag.append(el("p", { class: "prose" }, [t("Terminal web tidak punya ", "The web terminal has no "), ...codeList(p.flags, t(" atau ", " or ")), p.command ? t(` untuk ${p.command}.`, ` for ${p.command}.`) : "."]));
    } else {
      frag.append(el("p", { class: "prose" }, [el("b", { text: p.path }), ...(p.tool ? [t(" · alat MCP ", " · MCP tool "), el("code", { text: p.tool })] : []), ` · ${words(p.what)}`]));
    }
    if (p.why) frag.append(el("p", { class: "prose" }, inlineCode(words(p.why))));
    const instead = p.near.length ? [el("h3", { class: "stmt-title", text: p.kind === "flag" ? t("Ketik di sini", "Type here") : t("Di halaman ini", "On this page") }), nearTable(p.near)] : [];
    if (p.kind === "flag") frag.append(...instead);
    if (p.cli.length) {
      const labels = [t("Di CLI", "In the CLI"), t("Untuk skrip dan agen, dalam JSON", "For scripts and agents, as JSON")];
      frag.append(el("div", { class: "cmdline" }, p.cli.flatMap((line, i) => [el("p", { class: "lbl" }, [labels[i], copyButton(line)]), el("pre", { text: line })])));
    }
    const agent = p.kind === "cli-mcp" || p.agent;
    frag.append(el("p", { class: "warn install" }, [
      t("Pasang CLI: ", "Install the CLI: "), el("code", { text: INSTALL }),
      ...(agent ? [t("; untuk agen, tanpa memasang: ", "; for an agent, without installing: "), el("code", { text: AGENT })] : []),
      t("; cara lain di ", "; other ways in "), el("a", { href: "/docs/#install", hreflang: "en", text: t("panduan (bahasa Inggris)", "the guide") }), ".",
    ]));
    if (p.tools) frag.append(toolButtons(TOOLS.filter((x) => p.tools.includes(x.tool))));
    if (p.kind !== "flag") frag.append(...instead);
    return frag;
  },
  holidays(h) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>IDX trading holidays${h.year != null ? ` ${h.year}` : ""}</b>${h.trading_days != null ? ` · ${h.trading_days} trading days` : ""} · today ${h.today} is ${h.today_is_trading_day ? "a trading day" : "not a trading day"} · next trading day ${h.next_trading_day}` }));
    if (!h.holidays.length) frag.append(el("p", { class: "warn", text: "No holidays listed." }));
    else frag.append(wrapScroll(table([
      { h: "Date", k: "date", l: true, cls: "sym" }, { h: "Day", k: "weekday", l: true },
      { h: "Holiday", k: "name", l: true, cls: "name" }, { h: "English", f: (d) => d.name_en || NA, l: true },
    ], h.holidays)));
    frag.append(el("p", { class: "warn", text: `Covers ${h.years_covered.join(", ")}, from the calendar idx.co.id publishes. Weekend holidays are not listed; IDX can add a holiday later.` }));
    return frag;
  },
  astro(a) {
    const frag = document.createDocumentFragment();
    const name = (s) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, " ");
    const hm = (iso) => (iso ? `${iso.slice(0, 10)} ${iso.slice(11, 16)}` : fmt.text(null));
    // Minutes truncated, never rounded up into the next degree (as the CLI prints them).
    const dms = (deg) => { const d = Math.floor(deg); return `${d}°${String(Math.min(59, Math.floor((deg - d) * 60 + 1e-9))).padStart(2, "0")}'`; };
    const signed = (v, d) => (v > 0 ? `+${fmt.num(v, d)}` : fmt.num(v, d));
    const orNa = (v, f) => (v == null ? fmt.text(null) : f(v));
    frag.append(el("p", { class: "prose", html: `<b>IHSG astro · ${a.weton.day} ${a.weton.pasaran} ${a.date}</b> · positions at ${a.at.slice(11, 16)} WIB, the IDX close` }));
    frag.append(wrapScroll(table([
      { h: "Body", f: (b) => name(b.body), l: true, cls: "sym" }, { h: "Sign", f: (b) => name(b.sign), l: true },
      { h: "Position", f: (b) => `${dms(b.degree)}${b.retrograde ? " R" : ""}`, l: true }, { h: "Speed/day", f: (b) => `${signed(b.speed, 3)}°`, cls: (b) => `num ${signClass(b.speed)}` },
    ], a.bodies)));
    const m = a.moon, me = a.mercury;
    frag.append(el("p", { class: "prose", text: `Moon: ${m.phase.replace(/_/g, " ")}, ${fmt.num(m.illumination * 100, 0)}% lit, ${fmt.num(m.age_days, 1)} days old · new moon ${hm(m.next_new_moon)} · full moon ${hm(m.next_full_moon)}` }));
    let merc = me.retrograde ? `Mercury: retrograde${me.retrograde_since ? ` since ${hm(me.retrograde_since)}` : ""}${me.retrograde_until ? `, direct ${hm(me.retrograde_until)}` : ""}` : "Mercury: direct";
    if (me.next_retrograde) merc += ` · next retrograde ${me.next_retrograde.from.slice(0, 10)}${me.next_retrograde.until ? ` to ${me.next_retrograde.until.slice(0, 10)}` : ""}`;
    frag.append(el("p", { class: "prose", text: merc }));
    const w = a.weton, sh = a.shio;
    frag.append(el("p", { class: "prose", text: `Weton: ${w.day} ${w.pasaran}, neptu ${w.neptu_day} + ${w.neptu_pasaran} = ${w.neptu} · Shio: ${sh ? `${sh.animal_id} (${sh.element} ${sh.animal}) since Imlek ${sh.year_start}` : "not computed before 1924 or after 2050"}` }));
    const days = Math.round((Date.parse(a.upcoming.to) - Date.parse(a.upcoming.from)) / 86400e3);
    const what = (e) => {
      const sign = name(e.sign);
      if (e.kind === "ingress") return `${name(e.body)} enters ${sign}`;
      if (e.kind === "station_retrograde") return `${name(e.body)} stations retrograde in ${sign}`;
      if (e.kind === "station_direct") return `${name(e.body)} stations direct in ${sign}`;
      return `${name(e.kind)}${e.kind === "new_moon" || e.kind === "full_moon" ? "" : " moon"} in ${sign}`;
    };
    frag.append(el("h3", { class: "stmt-title", text: `Next ${days} day${days === 1 ? "" : "s"}` }));
    if (a.upcoming.events.length) frag.append(wrapScroll(table([{ h: "WIB", f: (e) => hm(e.at), l: true, cls: "sym" }, { h: "Event", f: what, l: true }], a.upcoming.events)));
    else frag.append(el("p", { class: "warn", text: "No exact phases, stations or ingresses." }));
    if (a.natal) {
      frag.append(el("h3", { class: "stmt-title", text: `Transits to IHSG's chart (${a.natal.chart === "launch" ? "launch" : "base date"} ${a.natal.date}, noon WIB; orb 2°)` }));
      if (a.natal.transits.length) frag.append(wrapScroll(table([
        { h: "Transit", f: (t) => name(t.transit), l: true, cls: "sym" }, { h: "Aspect", k: "aspect", l: true }, { h: "Natal", f: (t) => `natal ${name(t.natal)}`, l: true },
        { h: "Orb", f: (t) => `${fmt.num(t.orb, 2)}°` }, { h: "Motion", f: (t) => (t.applying ? "applying" : "separating"), l: true },
      ], a.natal.transits)));
      else frag.append(el("p", { class: "warn", text: "No aspects within 2° today." }));
    }
    const b = a.backtest;
    if (b) {
      frag.append(el("h3", { class: "stmt-title", text: `Backtest · ${b.symbol} daily close-to-close returns · ${b.from} to ${b.to} · ${fmt.int(b.returns)} sessions` }));
      frag.append(el("p", { class: "prose", text: `All sessions: mean ${signed(b.mean_pct, 4)}%, median ${signed(b.median_pct, 4)}%, up ${fmt.num(b.win_rate_pct, 2)}%` }));
      const FACTORS = { moon_phase: "Moon phase", mercury: "Mercury", pasaran: "Pasaran", sun_sign: "Sun sign" };
      const rows = b.factors.flatMap((f) => f.groups.map((g) => ({ ...g, factor: FACTORS[f.factor] || f.factor })));
      frag.append(wrapScroll(table([
        { h: "Factor", k: "factor", l: true }, { h: "Group", f: (g) => name(g.group), l: true, cls: "sym" }, { h: "Days", f: (g) => fmt.int(g.n) },
        { h: "Mean %", f: (g) => orNa(g.mean_pct, (v) => signed(v, 4)), cls: (g) => `num ${signClass(g.mean_pct)}` },
        { h: "Median %", f: (g) => orNa(g.median_pct, (v) => signed(v, 4)) }, { h: "Up %", f: (g) => orNa(g.win_rate_pct, (v) => fmt.num(v, 1)) },
        { h: "t", f: (g) => orNa(g.t_stat, (v) => fmt.num(v, 2)) }, { h: "p", f: (g) => orNa(g.p_value, (v) => `${fmt.num(v, 4)}${g.significant ? " *" : ""}`) },
      ], rows)));
      const verdict = b.significant === 0 ? "none pass" : `${b.significant} pass (marked *)`;
      frag.append(el("p", { class: "prose", text: `${b.tests} tests against all other sessions (Welch's t); Bonferroni needs p < ${b.bonferroni_alpha} at ${b.alpha * 100}%: ${verdict}. Sun signs follow the calendar, so they also carry month-of-year effects. Descriptive, not a trading signal.` }));
    }
    frag.append(el("p", { class: "warn", text: `${a.disclaimer}${b ? "" : " Add --backtest to see how IHSG's daily returns split by these factors; the web runs 1y to 10y, the CLI all of it since 1990."}` }));
    return frag;
  },
  sentiment(s) {
    const frag = document.createDocumentFragment();
    const cls = s.label === "positive" ? "up" : s.label === "negative" ? "dn" : "";
    frag.append(el("p", { class: "prose" }, [el("span", { class: `pill ${cls}`, text: s.label.replace("_", " ") }), ` score ${s.score > 0 ? "+" : ""}${fmt.num(s.score, 2)} · ${s.items} headlines: ${s.positive} positive, ${s.negative} negative, ${s.neutral} neutral${s.oldest ? ` · ${fmt.date(s.oldest)} to ${fmt.date(s.newest)}` : ""}`]));
    const terms = (l) => (l.length ? l.map(([t, n]) => `${t} ×${n}`).join(", ") : NA);
    frag.append(el("p", { class: "prose", text: `Terms · positive: ${terms(s.top_positive_terms)} · negative: ${terms(s.top_negative_terms)}` }));
    if (s.headlines.length) frag.append(newsTable(s.headlines));
    frag.append(el("p", { class: "warn", text: "Lexicon count over Indonesian and English finance terms. Indicative only." }));
    return frag;
  },
  market(m) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>IHSG ${fmt.price(m.index?.price)}</b> ${fmt.change(m.index?.change)} (${fmt.pct(m.index?.change_pct)}) · advancing ${m.advancing} · declining ${m.declining} · unchanged ${m.unchanged} · ${m.quoted} quoted · snapshot ${fmt.wibAt(m.refreshed_at)} · ${boardWords(m)}` }));
    if (m.behind_session) frag.append(el("p", { class: "warn", text: `Behind the latest IDX session: the newest quote on the board is from ${m.as_of.slice(0, 10)}` }));
    if (m.index_tape.length > 1) frag.append(sparkline(m.index_tape, { cls: "spark" }));
    const grid = el("div", { class: "three" });
    const mk = (title, rows, valueCol) => el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: title }), wrapScroll(table([{ h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Price", f: (r) => fmt.price(r.price) }, valueCol], rows))]);
    const chg = { h: "Chg %", f: (r) => fmt.pct(r.change_pct), cls: (r) => `num ${signClass(r.change_pct)}` };
    grid.append(mk("Gainers", m.gainers, chg), mk("Losers", m.losers, chg), mk("Most traded by value", m.most_active, { h: "Value", f: (r) => fmt.compact((r.volume || 0) * (r.price || 0)) }));
    frag.append(grid);
    return frag;
  },
  holders(h) {
    const frag = document.createDocumentFragment();
    const extra = ownCols(h.holders, h.controller_reported);
    frag.append(el("p", { class: "prose", html: `<b>${h.symbol}</b> · ${h.issuer_name || ""} · holders above 1 % as of ${h.as_of} · ${h.holders.length} listed = ${pct(h.listed_pct)}` }));
    frag.append(wrapScroll(table([{ h: "#", f: (r, i) => "", l: true }, { h: "Holder", k: "holder", l: true, cls: "sym" }, { h: "Type", k: "investor_type", l: true }, { h: "L/A", k: "local_foreign", l: true }, ...extra.cols, { h: "Shares", f: (r) => fmt.int(r.shares_total) }, { h: "%", f: (r) => pct(r.pct) }], h.holders.map((r, i) => ({ ...r, n: i + 1 })))));
    for (const n of extra.notes) frag.append(el("p", { class: "warn", text: n }));
    return frag;
  },
  entity(e) {
    const frag = document.createDocumentFragment();
    const extra = ownCols(e.holdings, e.controller_reported);
    frag.append(el("p", { class: "prose", text: `"${e.query}" · ${e.entities.length} matching holder${e.entities.length === 1 ? "" : "s"} · as of ${e.as_of}: ${e.entities.join("; ")}` }));
    frag.append(wrapScroll(table([{ h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Issuer", k: "issuer_name", l: true }, { h: "Holder (as printed)", k: "holder_raw", l: true }, { h: "Type", k: "investor_type", l: true }, ...extra.cols, { h: "Shares", f: (r) => fmt.int(r.shares_total) }, { h: "%", f: (r) => pct(r.pct) }], e.holdings)));
    for (const n of extra.notes) frag.append(el("p", { class: "warn", text: n }));
    return frag;
  },
  changes(c) {
    const frag = document.createDocumentFragment();
    const s = c.summary;
    const pp = (v) => `${v > 0 ? "+" : ""}${fmt.num(v, 2)} pp`;
    frag.append(el("p", { class: "prose", text: `Changes ${c.from} to ${c.to}${c.symbol ? ` · ${c.symbol}` : ""} · ${c.total_matches} match${c.total_matches === 1 ? "" : "es"}${s ? ` · ${s.issuers} issuers · ${s.holders} holders` : ""}` }));
    if (s) {
      frag.append(el("p", { class: "prose", text: `Summary · ${s.new_holders} new · ${s.exits} exited · ${s.increased} increased · ${s.decreased} decreased · net ${pp(s.net_pp)} (foreign ${pp(s.foreign_net_pp)}, local ${pp(s.local_net_pp)})` }));
      if (s.by_investor_type.length) frag.append(el("p", { class: "prose", text: `By type · ${s.by_investor_type.slice(0, 6).map((t) => `${t.investor_type} ${pp(t.net_pp)} (${t.rows})`).join(" · ")}` }));
      if (!c.symbol && s.issuer_flow.length) frag.append(el("p", { class: "prose", text: `Issuer flow · ${s.issuer_flow.slice(0, 8).map((i) => `${i.symbol} ${pp(i.net_pp)} (foreign ${pp(i.foreign_net_pp)})`).join(" · ")}` }));
    }
    if (c.changes.length < c.total_matches) frag.append(el("p", { class: "warn", text: `Rows ${(c.offset || 0) + 1}-${(c.offset || 0) + c.changes.length} of ${c.total_matches} · next page: --offset ${(c.offset || 0) + c.changes.length} · all: --limit 0` }));
    const flagged = c.changes.some((r) => r.controller_or_affiliate);
    frag.append(wrapScroll(table([{ h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Holder", k: "holder", l: true }, { h: "Type", k: "investor_type", l: true }, { h: "L/A", k: "local_foreign", l: true }, ...(flagged ? [CONTROL_COL] : []), { h: "Kind", f: changeKind, l: true }, { h: "% from", f: (r) => pct(r.pct_from) }, { h: "% to", f: (r) => pct(r.pct_to) }, { h: "Δ pp", f: (r) => fmt.change(r.pct_change), cls: (r) => `num ${signClass(r.pct_change)}` }, { h: "Δ shares", f: (r) => fmt.change(r.shares_change), cls: (r) => `num ${signClass(r.shares_change)}` }], c.changes)));
    return frag;
  },
  ownSearch(r) {
    const frag = document.createDocumentFragment();
    const shown = r.matches.length < r.total_matches ? ` · showing ${r.matches.length}` : "";
    frag.append(el("p", { class: "prose", text: `"${r.query}" · ${r.total_matches} holder${r.total_matches === 1 ? " matches" : "s match"}${shown} · issuers counted as of ${r.as_of}` }));
    if (!r.matches.length) return frag;
    frag.append(wrapScroll(table([
      { h: "Holder", k: "holder", l: true, cls: "sym" }, { h: "Match", k: "matched", l: true }, { h: "Issuers", f: (m) => String(m.issuers) },
      { h: "Total %", f: (m) => (m.issuers ? pct(m.total_pct) : null) }, { h: "Type", k: "investor_type", l: true }, { h: "L/A", k: "local_foreign", l: true },
      { h: "Spellings", f: (m) => m.spellings.join(" | "), l: true }, { h: "Key", k: "entity_key", l: true },
    ], r.matches)));
    const top = r.matches[0];
    frag.append(el("p", { class: "warn", text: `Total %: stakes in different issuers added up (a breadth measure). Next: ownership entity ${top.entity_key} --as-of ${top.issuers ? r.as_of : top.last_seen}` }));
    return frag;
  },
  crossHolders(r) {
    const frag = document.createDocumentFragment();
    const filters = [r.investor_type ? `type ~ ${r.investor_type}` : null, r.origin ? (r.origin === "foreign" ? "foreign (A)" : "local (L)") : null].filter(Boolean);
    const shown = r.holders.length < r.total_matches ? ` · showing ${r.holders.length} (--limit 0 for all)` : "";
    frag.append(el("p", { class: "prose", text: `Holders above 1 % in at least ${r.min_issuers} issuer${r.min_issuers === 1 ? "" : "s"} as of ${r.as_of}${filters.map((f) => ` · ${f}`).join("")} · ${r.total_matches} holder${r.total_matches === 1 ? "" : "s"}${shown}` }));
    if (!r.holders.length) return frag;
    const largest = (h) => h.positions.slice(0, 4).map((p) => `${p.symbol} ${pct(p.pct)}`).join(", ") + (h.positions.length > 4 ? ", …" : "");
    frag.append(wrapScroll(table([
      { h: "#", f: (h) => String(h.n) }, { h: "Holder", k: "holder", l: true, cls: "sym" }, { h: "Type", k: "investor_type", l: true }, { h: "L/A", k: "local_foreign", l: true },
      { h: "Issuers", f: (h) => String(h.issuers) }, { h: "Total %", f: (h) => pct(h.total_pct) }, { h: "Largest positions", f: largest, l: true },
    ], r.holders.map((h, i) => ({ ...h, n: i + 1 })))));
    frag.append(el("p", { class: "warn", text: "Total % adds stakes in different companies: a breadth measure, not a share of anything. Custodian and nominee accounts (securities companies, private banks) hold for their clients. A holder printed two ways counts twice." }));
    return frag;
  },
  concentration(r) {
    const frag = document.createDocumentFragment();
    const by = { hhi: "HHI", top1: "largest stake", top3: "top-3 stakes" }[r.sort] || r.sort;
    const shown = r.rows.length < r.total_matches ? ` · showing ${r.rows.length} (--limit 0 for all)` : "";
    frag.append(el("p", { class: "prose", text: `Ownership concentration as of ${r.as_of} · by ${by}, ${r.ascending ? "least" : "most"} first · ${r.total_matches} issuer${r.total_matches === 1 ? "" : "s"}${shown}` }));
    frag.append(wrapScroll(table([
      { h: "#", f: (c) => String(c.n) }, { h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Issuer", k: "issuer_name", l: true }, { h: "HHI", f: (c) => fmt.num(c.hhi, 2) },
      { h: "Top 1 %", f: (c) => pct(c.top1_pct) }, { h: "Top 3 %", f: (c) => pct(c.top3_pct) }, { h: "Listed %", f: (c) => pct(c.listed_pct) }, { h: "Undisclosed %", f: (c) => pct(c.undisclosed_pct) },
      ...(r.controller_reported ? [{ h: "Control %", f: (c) => pct(c.controller_pct) }] : []),
      { h: "Holders", f: (c) => String(c.holders_listed) }, { h: "Largest holder", k: "top_holder", l: true },
    ], r.rows.map((c, i) => ({ ...c, n: i + 1 })))));
    frag.append(el("p", { class: "warn", text: `HHI is the sum of the squared stakes of the holders listed above 1 % (0 to 10,000). Holders at or below 1 % are not published, so HHI covers the listed holders only (a lower bound) and Undisclosed is what the register does not show.${r.controller_reported ? " Control %: stakes flagged CONTROLLER or AFFILIATE OF THE CONTROLLER." : ""}` }));
    return frag;
  },
  graph(g) {
    const frag = document.createDocumentFragment();
    const label = g.issuers.find((i) => i.id === g.root)?.symbol || g.holders.find((h) => h.id === g.root)?.holder || g.root;
    const n = (k, one, many) => `${k} ${k === 1 ? one : many}`;
    frag.append(el("p", { class: "prose", text: `Ownership network around ${label} as of ${g.as_of} · ${n(g.depth, "hop", "hops")} · ${n(g.holders.length, "holder", "holders")} · ${n(g.issuers.length, "issuer", "issuers")} · ${n(g.links.length, "link", "links")}${g.min_pct > 0 ? ` · stakes of ${g.min_pct}% or more` : ""}${g.truncated ? " · truncated" : ""}` }));
    const holders = new Map(g.holders.map((h) => [h.id, h]));
    frag.append(wrapScroll(table([
      { h: "Hops", f: (l) => String(l.hops) }, { h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Holder", k: "holder", l: true },
      { h: "Type", f: (l) => holders.get(l.from)?.investor_type ?? null, l: true }, { h: "L/A", f: (l) => holders.get(l.from)?.local_foreign ?? null, l: true }, { h: "%", f: (l) => pct(l.pct) },
    ], g.links)));
    frag.append(el("p", { class: "warn", text: "Each row is a holding above 1 %. The CLI draws it too: idx ownership graph … --dot | dot -Tsvg" }));
    return frag;
  },
  releases(list) {
    return wrapScroll(table([{ h: "As of", k: "as_of", l: true, cls: "sym" }, { h: "Format", k: "format", l: true }, { h: "Rows", f: (r) => fmt.int(r.rows) }, { h: "Positions", f: (r) => fmt.int(r.positions) }, { h: "Source", k: "source", l: true }], list));
  },
};

/** Words as code, joined the way the page's language lists them: a, b and c. */
function codeList(words, last = t(" dan ", " and ")) {
  return words.flatMap((w, i) => [...(i ? [i === words.length - 1 ? last : ", "] : []), el("code", { text: w })]);
}

/** A sentence with its `backticked` words set as code. */
const inlineCode = (s) => String(s).split("`").map((part, i) => (i % 2 ? el("code", { text: part }) : part));

/**
 * A command line as its words, each kept whole (styles.css): it wraps at its spaces, never inside a word (foreign-accumulation,
 * 2026-08-31). A word over 40 characters (only a typed one: route.js's own are at most 20) stays plain text and may break at its
 * hyphens, as before: kept whole, a word that long can widen the table past its sheet at a desktop's narrowest two columns.
 */
const commandWords = (line) => line.split(" ").flatMap((w, i) => [...(i ? [" "] : []), w.length > 40 ? w : el("span", { class: "word", text: w })]);

/** The commands a notice offers instead, each a link that runs here, beside what it gives (help's table). */
function nearTable(list) {
  const tbl = table([
    { h: t("Perintah", "Command"), f: (n) => el("a", { href: `${here("/terminal")}?cmd=${encodeURIComponent(n.line)}` }, commandWords(n.line)), l: true, cls: "use" },
    { h: t("Kegunaan", "What it does"), f: (n) => t(n.id, n.en), l: true, cls: "help" },
  ], list);
  tbl.classList.add("tbl-help");
  return wrapScroll(tbl);
}

/** What a rewritten line ran as, and the CLI's own flags it left to the CLI. */
function noteLine(n) {
  const p = el("p", { class: "warn" }, [t("Dijalankan di sini sebagai ", "Run here as "), el("code", { text: n.line })]);
  if (n.ignored.length) p.append("; ", ...codeList(n.ignored), t(" hanya berlaku di CLI", n.ignored.length === 1 ? " applies to the CLI only" : " apply to the CLI only"));
  p.append(".");
  return p;
}

// ---- command execution -----------------------------------------------------

// `--statement income,balance,cash` (aliases cashflow, cash-flow; all = every
// statement) as the set of statement keys to show, or null for all of them.
const STATEMENTS = { income: "income", balance: "balance", cash: "cash_flow", cashflow: "cash_flow", "cash-flow": "cash_flow" };
function statements(v) {
  if (v == null || v === true) return null;
  const keys = new Set();
  for (const name of String(v).toLowerCase().split(",").map((x) => x.trim()).filter(Boolean)) {
    if (name === "all") return null;
    if (!STATEMENTS[name]) throw usageError(t(`laporan "${name}" tidak dikenal; pakai income, balance, cash atau all`, `invalid statement "${name}"; use income, balance, cash or all`));
    keys.add(STATEMENTS[name]);
  }
  return keys.size ? keys : null;
}

function need(pos, n, usage) { if (pos.length < n) throw usageLine(usage); }

// A command's request goes to the Worker even when the browser holds a copy
// from a moment ago (the language switch, Run again, Back), so the head and
// the machine line describe an answer sent for this run.
const ask = (path) => api(path, { fresh: true });

/**
 * Runs a typed line. route.js reads it first: a line in the CLI's words runs in
 * the page's (the result carries a note saying so), and one this page cannot
 * run answers with a notice instead of an error or another command's output.
 */
async function run(line) {
  const plan = routeLine(line);
  if (plan.kind !== "web") return { local: NOTICE_META[plan.kind], node: R.notice(plan) };
  const r = await runWeb(plan.line);
  if (r && plan.note) r.note = plan.note;
  return r;
}

async function runWeb(line) {
  const toks = tokenize(line);
  if (!toks.length) return null;
  let name = toks[0].toLowerCase();
  name = ALIASES[name] || name;
  let rawJson = false;
  if (name === "json") { rawJson = true; toks.shift(); name = (toks[0] || "").toLowerCase(); name = ALIASES[name] || name; }
  const { pos, opt } = flags(toks.slice(1));
  const cmd = COMMANDS.find((c) => c.name === name);
  if (!cmd) {
    // A bare ticker (or several) at the gate means quote.
    const near = COMMANDS.find((c) => c.name.startsWith(name));
    const known = new Set((await loadUniverse()).map((u) => u.symbol));
    const tickerish = toks.every((t) => /^[A-Za-z]{4}(\.JK)?$/i.test(t));
    const listed = known.size > 0 && toks.every((t) => known.has(t.toUpperCase().replace(/\.JK$/, "")));
    if (tickerish && (listed || (!known.size && !near))) return run(`${rawJson ? "json " : ""}quote ${toks.join(",")}`);
    throw usageError(t(
      `perintah "${toks[0]}" tidak dikenal${near ? `; mungkin maksudnya ${near.name}?` : ""}; ketik help`,
      `unknown command "${toks[0]}"${near ? `; did you mean ${near.name}?` : ""}; type help`));
  }
  const sym = (i = 0) => pos[i];
  let env, render;
  switch (name) {
    case "help": return { help: pos[0] };
    case "clear": return { clear: true };
    case "quote": need(pos, 1, cmd.usage); env = await ask(`quote?symbols=${encodeURIComponent(pos.join(","))}`); render = R.quotes; break;
    case "history": need(pos, 1, cmd.usage); env = await ask(`history?${qs({ symbol: sym(), period: pos[1] || opt.period || "1y", interval: pos[2] || opt.interval || "1d", limit: rawJson ? opt.limit || 0 : 0 })}`); render = R.history; break;
    case "intraday": need(pos, 1, cmd.usage); env = await ask(`intraday?${qs({ symbol: sym(), interval: pos[1] || opt.interval || "1m", limit: rawJson ? opt.limit || 0 : 0 })}`); render = R.intraday; break;
    case "profile": need(pos, 1, cmd.usage); env = await ask(`profile?symbol=${encodeURIComponent(sym())}`); render = R.profile; break;
    case "financials": {
      need(pos, 1, cmd.usage);
      const only = statements(opt.statement);
      env = await ask(`financials?${qs({ symbol: sym(), period: pos[1] || opt.period || "annual" })}`);
      render = (d) => R.financials(d, only);
      break;
    }
    case "earnings": {
      // Boolean flags: a word the flag parser took as a flag's value is the symbol.
      for (const k of ["annual", "quarterly", "reported", "estimates", "history", "forecast"]) if (typeof opt[k] === "string") { pos.push(opt[k]); opt[k] = true; }
      need(pos, 1, cmd.usage);
      const reported = opt.reported || opt.history, estimates = opt.estimates || opt.forecast;
      const period = opt.annual && !opt.quarterly ? "annual" : opt.quarterly && !opt.annual ? "quarterly" : null;
      const kind = reported && !estimates ? "reported" : estimates && !reported ? "estimates" : null;
      env = await ask(`earnings?${qs({ symbol: sym(), period, kind })}`);
      render = R.earnings;
      break;
    }
    case "fundamental": need(pos, 1, cmd.usage); env = await ask(`fundamental?${qs({ symbol: sym(), lens: opt.lens })}`); render = opt.lens ? R[String(opt.lens).toLowerCase()] : R.fundamentals; break;
    case "growth": case "valuation": case "risk": need(pos, 1, cmd.usage); env = await ask(`fundamental?${qs({ symbol: sym(), lens: name })}`); render = R[name]; break;
    case "technical": need(pos, 1, cmd.usage); env = await ask(`technical?${qs({ symbol: sym(), period: pos[1] || opt.period || "1y" })}`); render = R.technical; break;
    case "compare": need(pos, 1, cmd.usage); env = await ask(`compare?symbols=${encodeURIComponent(pos.join(","))}`); render = R.compare; break;
    case "screen": env = await ask(`screen?${qs({ filter: pos.join(","), preset: opt.preset, sort: opt.sort, limit: opt.limit, max_fundamentals: opt["max-fundamentals"], asc: opt.asc ? "1" : opt.desc ? "0" : null, returns: opt.returns ? "1" : null })}`); render = R.screen; break;
    case "news": env = await ask(`news?${qs({ symbol: sym(), limit: opt.limit || 20, company_only: opt["company-only"] ? "1" : null })}`); render = R.news; break;
    case "dividends": need(pos, 1, cmd.usage); env = await ask(`dividends?symbol=${encodeURIComponent(sym())}`); render = R.dividends; break;
    case "returns": need(pos, 1, cmd.usage); env = await ask(`returns?${qs({ symbols: pos.join(","), benchmark: opt.benchmark })}`); render = R.returns; break;
    case "brief": need(pos, 1, cmd.usage); env = await ask(`brief?symbol=${encodeURIComponent(sym())}`); render = R.brief; break;
    case "sentiment": need(pos, 1, cmd.usage); env = await ask(`sentiment?symbol=${encodeURIComponent(sym())}`); render = R.sentiment; break;
    case "insights": {
      need(pos, 1, cmd.usage);
      const all = opt.all != null;
      env = await ask(`insights?symbol=${encodeURIComponent(sym())}`);
      render = (d) => R.insights(d, all);
      break;
    }
    case "broker": need(pos, 1, cmd.usage); env = await ask(`broker?${qs({ symbols: pos.join(","), date: opt.date, from: opt.from, to: opt.to, investor: opt.investor, board: opt.board })}`); render = R.broker; break;
    case "foreign": need(pos, 1, cmd.usage); env = await ask(`foreign?${qs({ symbol: sym(), days: opt.days, from: opt.from, to: opt.to })}`); render = R.foreignFlow; break;
    case "bandar": {
      if ((pos[0] || "").toLowerCase() === "screen") {
        env = await ask(`bandar/screen?${qs({ preset: opt.preset, filter: opt.filter, period: opt.period, top: opt.top, sort: opt.sort, asc: opt.asc ? "1" : opt.desc ? "0" : null, limit: opt.limit })}`);
        render = R.bandarScreen;
      } else if ((pos[0] || "").toLowerCase() === "broker") {
        need(pos, 2, "bandar broker <CODE> [--period 1d|1w|1m|3m] [--limit N]");
        env = await ask(`bandar/broker?${qs({ code: pos[1], period: opt.period, limit: opt.limit })}`);
        render = R.bandarBroker;
      } else {
        const syms = (pos[0] || "").toLowerCase() === "detect" ? pos.slice(1) : pos;
        need(syms, 1, cmd.usage);
        env = await ask(`bandar?${qs({ symbols: syms.join(","), period: opt.period, top: opt.top, daily: opt.daily ? "1" : null, date: opt.date, from: opt.from, to: opt.to, investor: opt.investor, board: opt.board })}`);
        render = R.bandar;
      }
      break;
    }
    case "market": env = await ask("market"); await calendar; render = R.market; break;
    case "list": env = await ask(`instruments?${qs({ search: opt.search || pos.join(" ") || null })}`); render = R.instruments; break;
    case "members": env = await ask(`members?${qs({ search: opt.search || pos.join(" ") || null })}`); render = R.members; break;
    case "tools": {
      if (!pos[0]) return { local: t(`${TOOLS.length} alat MCP`, `${TOOLS.length} MCP tools`), node: R.tools() };
      const tool = byName(pos[0]);
      if (!tool) throw usageError(t(`tidak ada alat "${pos[0]}"; ketik tools untuk daftarnya`, `no tool "${pos[0]}"; type tools for the list`));
      if (tool.web) return run(rawJson ? `json ${tool.web}` : tool.web);
      return { local: CLI_ONLY, node: R.notice(routeLine(tool.tool)) };
    }
    case "actions": need(pos, 1, cmd.usage); env = await ask(`actions?${qs({ symbol: sym(), since: opt.since })}`); render = R.actions; break;
    case "holidays": {
      // `holidays --upcoming 2027`: the flag parser hands the year to the flag.
      const year = pos[0] || [opt.upcoming, opt.all].find((v) => typeof v === "string" && /^\d{4}$/.test(v)) || opt.year;
      env = await ask(`holidays?${qs({ year, all: opt.all ? "1" : null, upcoming: opt.upcoming ? "1" : null })}`);
      render = R.holidays;
      break;
    }
    case "astro": {
      const date = pos[0] || opt.date;
      env = await ask(`astro?${qs({ date, days: opt.days, natal: opt.natal, backtest: opt.backtest ? "1" : null, period: opt.period })}`);
      render = R.astro;
      break;
    }
    case "ownership": {
      const sub = (pos[0] || "").toLowerCase();
      if (sub === "ticker") { need(pos, 2, cmd.usage); env = await ask(`ownership/ticker?${qs({ symbol: pos[1], as_of: opt["as-of"] })}`); render = R.holders; }
      else if (sub === "entity") { need(pos, 2, cmd.usage); env = await ask(`ownership/entity?${qs({ name: pos.slice(1).join(" "), as_of: opt["as-of"] })}`); render = R.entity; }
      else if (sub === "changes") { need(pos, 3, cmd.usage); env = await ask(`ownership/changes?${qs({ from: pos[1], to: pos[2], symbol: pos[3], min_change: opt["min-change"], limit: opt.limit, offset: opt.offset })}`); render = R.changes; }
      else if (sub === "foreign") { env = await ask(`ownership/foreign?${qs({ symbol: pos[1], from: opt.from, to: opt.to, limit: opt.limit })}`); render = R.foreign; }
      else if (sub === "releases") { env = await ask("ownership/releases"); render = R.releases; }
      else if (sub === "search") { need(pos, 2, cmd.usage); env = await ask(`ownership/search?${qs({ q: pos.slice(1).join(" "), limit: opt.limit, as_of: opt["as-of"] })}`); render = R.ownSearch; }
      else if (sub === "cross-holders") { env = await ask(`ownership/cross-holders?${qs({ min_issuers: opt["min-issuers"] ?? opt["min-tickers"], type: opt.type, origin: opt.origin, limit: opt.limit, as_of: opt["as-of"] })}`); render = R.crossHolders; }
      else if (sub === "concentration") {
        // `--asc BBCA` hands the symbol to the flag; take it back.
        const syms = [...pos.slice(1), ...[opt.asc, opt.ascending].filter((v) => typeof v === "string")];
        env = await ask(`ownership/concentration?${qs({ symbols: syms.join(","), by: opt.by, asc: opt.asc || opt.ascending ? "1" : null, limit: opt.limit, as_of: opt["as-of"] })}`);
        render = R.concentration;
      } else if (sub === "graph") {
        const root = [...pos.slice(1), ...(typeof opt.holder === "string" ? [opt.holder] : [])];
        need(root, 1, cmd.usage);
        env = await ask(`ownership/graph?${qs({ root: root.join(" "), depth: opt.depth, min_pct: opt["min-pct"], max_nodes: opt["max-nodes"], holder: opt.holder ? "1" : null, as_of: opt["as-of"] })}`);
        render = R.graph;
      }
      else throw usageLine(cmd.usage);
      break;
    }
  }
  return { env, render: rawJson ? (d, e) => el("pre", { class: "json", text: JSON.stringify(e, null, 2) }) : render };
}

// ---- sheet ---------------------------------------------------------------------

// Each result is a numbered panel, newest under the gate, closed by its
// machine line. The URL follows the newest result, so Back retraces them.
let seq = 0;
const coarse = matchMedia("(pointer: coarse)").matches;

function entry(cmdLine) {
  const n = ++seq;
  const sec = el("section", { class: "panel result", id: `r${n}`, tabindex: "-1", "aria-labelledby": `r${n}-h` });
  const meta = el("p", { class: "meta", text: t("berjalan", "running") });
  const cap = el("header", { class: "ph" }, [el("h2", { id: `r${n}-h` }, [el("span", { class: "no", text: String(n).padStart(2, "0") }), shownLine(cmdLine)]), meta]);
  const body = el("div", { class: "body" });
  const mline = el("p", { class: "mline" });
  sec.append(cap, body, mline);
  out.prepend(sec);
  return { n, sec, cap, body, meta, mline };
}

function reveal(sec) {
  sec.scrollIntoView({ block: "start", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
}

function focusResult(p) {
  if (!p) return false;
  p.scrollIntoView({ block: "start" });
  p.focus({ preventScroll: true });
}

// What each command waits on, in Bahasa Indonesia: the same entries as
// public/waits.js (which keeps the English and the API's limits), by command.
const FEEDS_SLOW_ID = `umpan Kontan kadang macet; umpan yang diam ${FEED_TIMEOUT_S} detik dilewati dan disebut dalam peringatan`;
const IPOT_SLOW_ID = `IPOT kadang menjawab 404 sebelum datanya siap; setiap jawaban itu diulang, paling banyak ${IPOT_TRIES} kali`;
const FEEDS_ID = "umpan berita Kontan dan CNBC Indonesia";
const IPOT_ID = "ringkasan broker IPOT";
const WAITS_ID = {
  quote: { on: "Yahoo Finance" },
  growth: { on: "Yahoo Finance: fundamental dan laporan tahunan" },
  risk: { on: "Yahoo Finance: fundamental dan harga harian setahun untuk saham ini dan IHSG" },
  compare: { on: "Yahoo Finance, satu permintaan per kode" },
  returns: { on: "Yahoo Finance, satu riwayat per kode dan pembandingnya" },
  screen: { on: "papan harga, fundamental Yahoo dan daftar pasar MSN", slow: "filter fundamental mengambil setiap saham yang cocok, sampai --max-fundamentals (20); kunci imbal hasil membaca daftar seluruh pasar dari MSN sekali (disimpan 15 menit)" },
  news: { on: FEEDS_ID, slow: FEEDS_SLOW_ID },
  sentiment: { on: FEEDS_ID, slow: FEEDS_SLOW_ID },
  insights: { on: "insight MSN Money dan daftar emiten sejenisnya" },
  brief: { on: "harga, fundamental, riwayat, dividen, pemegang saham dan berita sekaligus", slow: `${FEEDS_SLOW_ID}; bagian lain ringkasan tetap tampil` },
  broker: { on: IPOT_ID, slow: IPOT_SLOW_ID },
  foreign: { on: `${IPOT_ID}, dua per sesi`, slow: IPOT_SLOW_ID },
  bandar: { on: `${IPOT_ID}, satu per sesi`, slow: IPOT_SLOW_ID },
  "bandar screen": { on: "pindaian bandar sesi terakhir" },
  "bandar broker": { on: "pindaian bandar sesi terakhir" },
  actions: { on: "aksi korporasi dari IPOT", slow: IPOT_SLOW_ID },
  market: { on: "papan harga per menit" },
  ownership: { on: "daftar pemegang saham KSEI" },
  astro: { on: "posisi langit yang dihitung API; --backtest juga membaca riwayat ^JKSE dari Yahoo Finance (disimpan 6 jam)" },
};

/** What a typed line waits on (public/waits.js), in the page's language. */
function waitsOf(line) {
  const toks = tokenize(line);
  if (toks[0]?.toLowerCase() === "json") toks.shift();
  const name = (toks[0] || "").toLowerCase();
  const w = waitFor(ALIASES[name] || name, toks[1]);
  if (!ID) return w;
  const key = Object.keys(WAITS).find((k) => WAITS[k] === w);
  const id = WAITS_ID[key] || {};
  return { on: id.on || w.on, slow: id.slow || w.slow };
}

/**
 * The loading state of an entry while its request is out: a run of lit dots
 * crosses the panel's top rule and a line says what it waits on, with a clock.
 * Returns a stop function that takes both down and gives the elapsed ms.
 */
function pending(e, line) {
  const w = waitsOf(line);
  const t0 = performance.now();
  const clock = el("span", { class: "clock", "aria-hidden": "true" });
  const on = w.on || t("sumbernya", "the source");
  const status = el("p", { class: "loading", role: "status" }, [el("span", { text: t(`menunggu ${on}`, `waiting on ${on}`) }), clock]);
  let slow = false, every = 0;
  const tick = () => {
    const ms = performance.now() - t0;
    const s = Math.floor(ms / 1000);
    clock.textContent = t(` · ${s} detik`, ` · ${s}s`);
    if (!slow && ms >= SLOW_MS) {
      slow = true;
      status.append(el("span", { class: "slow", text: ` · ${w.slow || t("lebih lambat dari biasanya; sumbernya mungkin sedang sibuk", "slower than usual; the source may be busy")}` }));
      reveal(e.sec);
    }
  };
  const show = setTimeout(() => { e.sec.classList.add("busy"); e.body.append(status); tick(); every = setInterval(tick, 1000); reveal(e.sec); }, SHOW_MS);
  return () => {
    clearTimeout(show);
    clearInterval(every);
    status.remove();
    e.sec.classList.remove("busy");
    return performance.now() - t0;
  };
}

/** Runs a line as if typed: kept in the history. */
function runLine(s) { execute(s); }

/** One button per MCP tool: a web tool runs its example, a CLI-and-MCP-only tool shows its notice. */
function toolButtons(list = TOOLS) {
  const box = el("div", { class: "hints tools" });
  box.append(...list.map((tool) => el("button", {
    type: "button", text: tool.tool, class: tool.web ? "" : "cli-only",
    title: `${whatOf(tool)} · ${tool.web ? t(`menjalankan: ${tool.web}`, `runs: ${tool.web}`) : CLI_ONLY}`,
    onclick: () => runLine(tool.web || `tools ${tool.tool}`),
  })));
  return box;
}

function renderHelp(which) {
  const frag = document.createDocumentFragment();
  const list = which ? COMMANDS.filter((c) => c.name === (ALIASES[which] || which)) : COMMANDS;
  if (!list.length) frag.append(el("p", { class: "err", text: t(`tidak ada perintah "${which}"`, `no command "${which}"`) }));
  const usage = table([{ h: t("Perintah", "Command"), k: "usage", l: true, cls: "use" }, { h: t("Kegunaan", "What it does"), k: "help", l: true, cls: "help" }], list.map((c) => ({ usage: c.usage, help: helpOf(c) })));
  usage.classList.add("tbl-help");
  frag.append(wrapScroll(usage));
  frag.append(el("p", { class: "warn", text: t(
    "Alias: q h f t n s c d r b own. Awali perintah apa pun dengan json untuk melihat amplop JSON mentahnya. Kode saham boleh BBCA atau BBCA.JK; beberapa kode dipisah koma atau spasi. Kode saham saja menjalankan quote. Nama kolom hasil memakai istilah bahasa Inggris CLI.",
    "Aliases: q h f t n s c d r b own. Prefix any command with json for the raw envelope. Symbols accept BBCA or BBCA.JK; several symbols are comma or space separated. A bare ticker runs quote.") }));
  frag.append(el("p", { class: "warn", text: t(
    "Baris dari CLI juga berjalan di sini (idx stocks quote BBCA, idx -o json market overview); perintah yang hanya ada di CLI atau MCP menunjukkan baris CLI-nya dan perintah terdekat di halaman ini.",
    "Lines pasted from the CLI run here too (idx stocks quote BBCA, idx -o json market overview); a command that runs only in the CLI or MCP shows its CLI line and the nearest command on this page.") }));
  if (!which) {
    frag.append(el("h3", { class: "stmt-title", text: t(`Alat MCP · ${TOOLS.length} · klik untuk menjalankan (garis putus-putus: hanya CLI dan MCP)`, `MCP tools · ${TOOLS.length} · click to run (dashed: CLI and MCP only)`) }));
    frag.append(toolButtons());
  }
  return frag;
}

const local = (e, what) => machineLine(e.mline, ["local", what], t("Dijawab di peramban, tanpa permintaan.", "Answered in the browser, no request."));

/**
 * A result's head, machine-line fields and screen-reader sentence: the
 * provider, how the answer was served (cacheState: cached, partly cached or
 * live), stale, and when it was fetched, which for a partly cached answer is
 * its oldest part. The machine line keeps the English word on both pages.
 */
function provenance(env) {
  const s = CACHE_STATES[cacheState(env)];
  const how = t(s.id, s.en), fetched = t(s.fetched.id, s.fetched.en);
  return {
    meta: `${env.provider} · ${how}${env.stale ? t(" · kedaluwarsa", " · stale") : ""} · ${fetched} ${fmt.when(env.fetched_at)}`,
    fields: [env.schema, env.provider, s.en, env.stale ? "stale" : null, env.fetched_at],
    sentence: t(
      `Skema ${env.schema}, penyedia ${env.provider}, ${how}${env.stale ? ", kedaluwarsa" : ""}, ${fetched} ${env.fetched_at}.`,
      `Schema ${env.schema}, provider ${env.provider}, ${how}${env.stale ? ", stale" : ""}, ${fetched} ${env.fetched_at}.`),
  };
}

/** The language switch follows the newest command, so the other language opens on the same line. */
function syncLang() {
  const a = document.querySelector(".lang a[hreflang]");
  if (a) a.setAttribute("href", `${otherPath()}${location.search}`);
}

async function execute(line, { push = "push" } = {}) {
  const e = entry(line);
  input.value = "";
  remember(line);
  const url = `${here("/terminal")}?cmd=${encodeURIComponent(line)}#r${e.n}`;
  if (push === "push") history.pushState({ n: e.n }, "", url);
  else if (push === "replace") history.replaceState({ n: e.n }, "", url);
  syncLang();
  // The loading line names what the line waits on in the page's words (a CLI line's web form).
  const plan = routeLine(line);
  const stop = pending(e, plan.kind === "web" ? plan.line : "");
  const took = (ms) => (ms < 1000 ? "" : t(` · butuh ${fmt.num(ms / 1000, 1)} detik`, ` · took ${(ms / 1000).toFixed(1)}s`));
  try {
    const r = await run(line);
    const ms = stop();
    if (!r) { e.sec.remove(); return; }
    if (r.clear) { out.replaceChildren(); return; }
    if (r.note) e.body.append(noteLine(r.note));
    if ("help" in r) { e.meta.textContent = t("lokal · tanpa permintaan", "local · no request"); e.body.append(renderHelp(r.help)); local(e, "help"); }
    else if (r.local) { e.meta.textContent = r.local; e.body.append(r.node); local(e, r.local); }
    else {
      const env = r.env, p = provenance(env);
      e.meta.textContent = `${p.meta}${took(ms)}`;
      e.meta.classList.toggle("stale", !!env.stale);
      for (const w of env.warnings || []) e.body.append(el("p", { class: "warn", text: w }));
      e.body.append(r.render(env.data, env));
      machineLine(e.mline, p.fields, p.sentence);
    }
  } catch (err) {
    const ms = stop();
    const exit = err.exit ?? (err.code === "usage" ? 2 : 3);
    const failed = t("gagal", "error");
    e.meta.textContent = `${err.code ? `${failed} · ${err.code}` : failed}${took(ms)}`;
    e.meta.classList.add("err");
    e.body.append(el("p", { class: "err", text: err.message }));
    if (err.code === "usage") e.body.append(el("p", { class: "warn", text: t("Ketik help untuk daftar perintah.", "Type help for the command list.") }));
    machineLine(e.mline, ["error", err.code || "provider", `exit ${exit}`], t(
      `Gagal: ${err.code || "provider"}, kode keluar ${exit}.`,
      `Error ${err.code || "provider"}, exit code ${exit}.`));
  }
  reveal(e.sec);
  if (!coarse) input.focus();
}

// ---- history: kept in this browser, recalled with the arrows and in the rail ----

const HISTORY_KEY = "idx.history";
const HISTORY_MAX = 50;
const hist = (() => {
  try { return JSON.parse(localStorage.getItem(HISTORY_KEY) || "[]").filter((x) => typeof x === "string").slice(-HISTORY_MAX); } catch { return []; }
})();

function remember(line) {
  if (hist[hist.length - 1] !== line) hist.push(line);
  if (hist.length > HISTORY_MAX) hist.splice(0, hist.length - HISTORY_MAX);
  try { localStorage.setItem(HISTORY_KEY, JSON.stringify(hist)); } catch { /* the history stays in this tab */ }
  paintHistory();
}

function paintHistory() {
  const recent = [...hist].reverse().slice(0, 20);
  document.getElementById("rail-hist").replaceChildren(...recent.map((l, i) => el("li", {}, [
    el("button", { type: "button", title: t(`Jalankan lagi: ${l}`, `Run again: ${l}`), onclick: () => runLine(l) }, [el("span", { class: "d", text: String(i + 1).padStart(2, "0") }), el("span", { class: "c", text: l })]),
  ])));
  document.getElementById("rail-empty").hidden = recent.length > 0;
}

// ---- the gate, the rail and the keys ------------------------------------------

// hints + deep links (?cmd=)
const SAMPLE = ["market", "brief BBCA", "quote BBCA", "returns BBCA BBRI BMRI", "dividends TLKM", "fundamental BBCA", "technical BBRI", "screen pe<10,div>5,mcap>5T --sort div", "news TLKM", "broker BBCA", "bandar BBCA --daily", "bandar screen --preset big-accumulation", "bandar broker YP", "actions BBCA", "holidays --upcoming", "astro", "compare BBCA BBRI BMRI", "ownership ticker BBCA", "tools", "help"];
document.getElementById("hints").append(...SAMPLE.map((s) => el("button", { type: "button", text: s, onclick: () => runLine(s) })));

const takesArgs = (usage) => /<[^>]+>/.test(usage.split(" | ")[0]);
const EXAMPLE = new Map();
for (const s of [...SAMPLE, ...TOOLS.map((t) => t.web).filter(Boolean)]) { const k = s.split(" ")[0]; if (!EXAMPLE.has(k)) EXAMPLE.set(k, s); }

/** A command from the rail: run it when it needs nothing, else load its example into the gate. */
function pick(c) {
  if (!takesArgs(c.usage)) return runLine(c.name);
  input.value = EXAMPLE.get(c.name) || `${c.name} `;
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
}
// The rail is narrow: each command gets a label of a few words here; help prints the full text.
const RAIL = {
  quote: "quotes and chart", history: "OHLCV candles", intraday: "minute bars", profile: "company profile", financials: "statements",
  earnings: "EPS vs estimates", fundamental: "ratios, verdict", growth: "growth lens", valuation: "valuation lens",
  risk: "risk vs IHSG", technical: "indicators, signal", compare: "side by side", insights: "MSN insights",
  foreign: "foreign flow", astro: "IHSG astrology",
  screen: "filter the market", news: "headlines", dividends: "payments, yield", returns: "returns vs IHSG",
  brief: "one-page summary", sentiment: "headline mood", broker: "top brokers", bandar: "bandarmology",
  actions: "corporate actions", market: "IHSG and movers", holidays: "trading holidays", list: "listed stocks",
  members: "broker codes", tools: "MCP tools", ownership: "KSEI holders", json: "raw JSON envelope",
  help: "every command", clear: "clear the sheet",
};
const RAIL_ID = {
  quote: "harga dan grafik", history: "candle OHLCV", intraday: "bar per menit", profile: "profil perusahaan", financials: "laporan keuangan",
  earnings: "EPS vs perkiraan", fundamental: "rasio, kesimpulan", growth: "lensa pertumbuhan", valuation: "lensa valuasi",
  risk: "risiko vs IHSG", technical: "indikator, sinyal", compare: "berdampingan", insights: "insight MSN",
  foreign: "aliran asing", astro: "astrologi IHSG",
  screen: "saring pasar", news: "berita", dividends: "dividen, imbal hasil", returns: "imbal hasil vs IHSG",
  brief: "ringkasan saham", sentiment: "suasana berita", broker: "broker teratas", bandar: "bandarmologi",
  actions: "aksi korporasi", market: "IHSG dan penggerak", holidays: "libur bursa", list: "saham tercatat",
  members: "kode broker", tools: "alat MCP", ownership: "pemegang KSEI", json: "amplop JSON mentah",
  help: "semua perintah", clear: "bersihkan lembar",
};
const railOf = (c) => (ID ? RAIL_ID : RAIL)[c.name] || helpOf(c);
document.getElementById("rail-cmds").append(...COMMANDS.map((c) => el("li", {}, [
  el("button", { type: "button", title: `${c.usage}: ${helpOf(c)}`, onclick: () => pick(c) }, [el("span", { class: "c", text: c.name }), el("span", { class: "d", text: railOf(c) })]),
])));
paintHistory();

const GATE = COMMANDS.map((c) => ({ name: c.name, help: helpOf(c), value: takesArgs(c.usage) ? `${c.name} ` : c.name, run: !takesArgs(c.usage), symbols: /SYMBOL/.test(c.usage) }));
let tickers = [];
mountGate({
  form, input, list: document.getElementById("gate-list"), status: document.getElementById("gate-status"),
  history: () => hist,
  suggest: (line) => suggestions(line, { commands: GATE, tickers, history: [...hist].reverse(), examples: SAMPLE.slice(0, 6) }),
  onRun: (line) => execute(line),
  onFocus: () => loadUniverse().then((u) => { tickers = u; }),
});
if (document.activeElement === input) loadUniverse().then((u) => { tickers = u; });

/** J and K walk the results (newest first); digits jump to a result by its number. */
function step(d) {
  const list = [...out.querySelectorAll(".result")];
  if (!list.length) return;
  const at = list.indexOf(document.activeElement?.closest?.(".result"));
  const i = at < 0 ? (d > 0 ? 0 : list.length - 1) : Math.min(list.length - 1, Math.max(0, at + d));
  focusResult(list[i]);
}
const openKeys = initShell();
hotkeys({
  "/": () => input.focus(),
  "?": openKeys,
  j: () => step(1),
  k: () => step(-1),
  digit: (n) => focusResult(document.getElementById(`r${n}`)),
});

// Links to /terminal?cmd= (or /en/terminal?cmd=) inside results (tickers, tool commands) run here, in place.
out.addEventListener("click", (ev) => {
  const a = ev.target.closest?.(`a[href^="${here("/terminal")}?cmd="]`);
  if (!a || ev.button !== 0 || ev.ctrlKey || ev.metaKey || ev.shiftKey || ev.altKey) return;
  ev.preventDefault();
  runLine(new URL(a.href).searchParams.get("cmd"));
});

addEventListener("popstate", (ev) => {
  syncLang();
  const p = ev.state?.n ? document.getElementById(`r${ev.state.n}`) : null;
  if (p) return focusResult(p);
  const cmd = new URLSearchParams(location.search).get("cmd");
  if (cmd) execute(cmd, { push: "replace" });
  else scrollTo({ top: 0 });
});

// ---- status bar: the minute board's state, read once a minute ------------------

let holidays = new Map();
// The IDX holiday calendar, read once; the market panel waits for it so a holiday never reads as a session.
// Each holiday carries the name the exchange gives it in the page's language.
const calendar = api("holidays?all=1&upcoming=1")
  .then((h) => { holidays = new Map(h.data.holidays.map((x) => [x.date, ID ? x.name || x.name_en : x.name_en || x.name])); })
  .catch(() => {});

/** Where the session stands, in the page's words (clock.js names the phases in English; i18n.js in both). */
function phaseWords(s) {
  if (s.phase) return PHASES[s.phase.id] || s.phase.name;
  if (s.holiday) return t(`Tutup, ${s.holiday}`, `Closed, ${s.holiday}`);
  return s.tradingDay ? t("Tutup", "Closed") : t("Tutup, akhir pekan", "Closed, weekend");
}

/** The market panel's state words, from the session classification the monitor's band uses. */
function boardWords(m) {
  const last = m.as_of ? new Date(m.as_of) : null;
  const b = boardState(new Date(m.refreshed_at), last, holidays);
  if (!b.lastTrade) return phaseWords(b.session);
  const w = wib(last);
  return `${phaseWords(b.session)} · ${t(`transaksi terakhir ${DAYS_SHORT[w.dow]} ${w.iso}`, b.lastTrade)}`;
}

const LAMP = { live: t("Langsung", "Live"), closed: t("Bursa tutup", "Closed"), stale: t("Tertunda", "Stale"), offline: t("Terputus", "Offline") };

/** What follows the count: the phase while a session runs, otherwise only why the exchange is shut,
 *  since the lamp already says it is (nothing after hours on a trading day). */
function trailWords(s) {
  if (s.open) return phaseWords(s);
  if (s.holiday) return s.holiday;
  return s.tradingDay ? "" : t("akhir pekan", "weekend");
}

/** The status bar: the lamp and the snapshot's age, read with the board once a minute; a phone keeps just those. */
async function board() {
  try {
    const m = await api("market");
    const at = new Date(m.data.refreshed_at);
    const s = session(new Date(), holidays);
    const age = Math.max(0, (Date.now() - at) / 1000);
    const kind = !s.open ? "closed" : age > 180 ? "stale" : "live";
    lamp(kind, LAMP[kind]);
    const trail = trailWords(s);
    document.getElementById("st-age").replaceChildren(t("Papan ", "Snapshot "), el("b", { text: `${hhmm(wib(at).secs).replace(":", TIME_SEP)} WIB` }), t(` · ${fmt.ago(age)} lalu`, ` · ${fmt.ago(age)} ago`),
      el("span", { class: "st-next", text: t(` · ${fmt.int(m.data.quoted)} saham`, ` · ${fmt.int(m.data.quoted)} quoted`) + (trail ? ` · ${trail}` : "") }));
  } catch {
    lamp("offline", LAMP.offline);
  }
  setTimeout(board, 60_000);
}
calendar.then(board);

// ---- deep links (?cmd=) -----------------------------------------------------------

const initial = new URLSearchParams(location.search).get("cmd");
if (initial) execute(initial, { push: "replace" });
