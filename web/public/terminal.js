// The web terminal: parses CLI-shaped commands, calls /api, prints statements.
import { api, fmt, table, wrapScroll, el, sparkline, signClass } from "/app.js";
import { priceChart } from "/chart.js";
import { TOOLS, CLI_ONLY_REASON, byName } from "/tools.js";
import { SHOW_MS, SLOW_MS, waitFor } from "/waits.js";

const out = document.getElementById("out");
const form = document.getElementById("prompt");
const input = document.getElementById("cmd");
const dateline = document.getElementById("dateline");

// ---- command table ---------------------------------------------------------

const COMMANDS = [
  { name: "quote", usage: "quote <SYMBOLS>", help: "delayed quotes, e.g. quote BBCA,BBRI; one symbol adds a price chart (1D to 5Y)" },
  { name: "history", usage: "history <SYMBOL> [period] [interval]", help: "OHLCV candles; periods 1d 5d 1mo 3mo 6mo 1y 2y 5y 10y ytd max, intervals 5m 15m 30m 1h 1d 1wk 1mo" },
  { name: "profile", usage: "profile <SYMBOL>", help: "sector, industry, description, size" },
  { name: "financials", usage: "financials <SYMBOL> [annual|quarterly|ttm]", help: "income statement, balance sheet, cash flow" },
  { name: "earnings", usage: "earnings <SYMBOL>", help: "quarterly EPS vs estimates, next date" },
  { name: "fundamental", usage: "fundamental <SYMBOL>", help: "ratios with a plain-language verdict" },
  { name: "technical", usage: "technical <SYMBOL> [period]", help: "RSI, MACD, averages, levels, signal" },
  { name: "compare", usage: "compare <SYMBOLS>", help: "side-by-side fundamentals" },
  { name: "screen", usage: "screen <FILTER> [--sort KEY] [--limit N]", help: 'e.g. screen pe<10,div>5,mcap>5T --sort div' },
  { name: "news", usage: "news [SYMBOL] [--limit N] [--company-only]", help: "headlines; with a symbol the company's first, then market context" },
  { name: "dividends", usage: "dividends <SYMBOL>", help: "payments, yearly totals, TTM yield, streak, splits" },
  { name: "returns", usage: "returns <SYMBOLS> [--benchmark ^JKSE|none]", help: "1w..5y returns, CAGR, volatility, drawdown, excess over IHSG" },
  { name: "brief", usage: "brief <SYMBOL>", help: "one page: quote, verdict, signal, returns, dividends, holders, headlines" },
  { name: "sentiment", usage: "sentiment <SYMBOL>", help: "lexicon sentiment over matching headlines" },
  { name: "broker", usage: "broker <SYMBOLS> [--date D | --from D --to D] [--investor foreign|domestic] [--board regular|cash|negotiated]", help: "which brokers bought and sold it: top 10 per side ranked by lots (IPOT), latest session by default" },
  { name: "bandar", usage: "bandar <SYMBOLS> [--period 1d|1w|1m|3m] [--top 1|3|5|10] [--daily] [--date D | --from D --to D] | bandar screen [--preset P] [--filter F] [--period P] [--sort KEY] [--limit N] | bandar broker <CODE> [--period P] [--limit N]", help: "bandarmology: are the top brokers accumulating or distributing? screen presets: big-accumulation, accumulation, distribution, big-distribution, quiet-accumulation, quiet-distribution, foreign-accumulation, below-bandar; broker: where one broker (YP, CC, ...) is a top-10 buyer or seller of the session's 100 most traded stocks" },
  { name: "actions", usage: "actions <SYMBOL> [--since YYYY-MM-DD]", help: "corporate actions (dividends, splits, bonus, rights with ratio and cum/ex/payment dates), RUPS, warrants and rights; upcoming included (IPOT)" },
  { name: "market", usage: "market", help: "IHSG, breadth, gainers, losers, most traded" },
  { name: "holidays", usage: "holidays [YEAR] [--all] [--upcoming]", help: "IDX trading holidays, the year's trading days, whether today is a trading day and the next one" },
  { name: "list", usage: "list [--search TEXT]", help: "listed stocks: codes and names (bundled; the CLI adds boards, listing dates and shares from idx.co.id)" },
  { name: "members", usage: "members [--search TEXT]", help: "exchange members: the broker codes broker and bandar report, with names" },
  { name: "tools", usage: "tools [idx_TOOL]", help: "the 33 MCP tools with their CLI and web commands; tools idx_quote runs one" },
  { name: "ownership", usage: "ownership ticker <SYMBOL> | entity <NAME> | changes <FROM> <TO> [SYMBOL] [--offset N] | foreign [SYMBOL] | releases", help: "KSEI holders above 1 %, synced monthly from IDX: holders, entities, monthly changes with a summary, foreign vs local" },
  { name: "json", usage: "json <command…>", help: "print the raw JSON envelope of any command" },
  { name: "help", usage: "help [command]", help: "this list" },
  { name: "clear", usage: "clear", help: "clear the sheet" },
];
const ALIASES = { q: "quote", h: "history", f: "fundamental", t: "technical", n: "news", s: "screen", c: "compare", d: "dividends", r: "returns", b: "brief", "?": "help", cls: "clear", own: "ownership" };

function tokenize(line) {
  const toks = [];
  const re = /"([^"]*)"|'([^']*)'|(\S+)/g;
  let m;
  while ((m = re.exec(line))) toks.push(m[1] ?? m[2] ?? m[3]);
  return toks;
}
function flags(toks) {
  const pos = [], opt = {};
  for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    if (t.startsWith("--")) {
      const [k, v] = t.slice(2).split("=");
      if (v != null) opt[k] = v;
      else if (i + 1 < toks.length && !toks[i + 1].startsWith("--")) opt[k] = toks[++i];
      else opt[k] = true;
    } else pos.push(t);
  }
  return { pos, opt };
}
const qs = (o) => Object.entries(o).filter(([, v]) => v != null && v !== "").map(([k, v]) => `${k}=${encodeURIComponent(v)}`).join("&");

// ---- rendering --------------------------------------------------------------

function kv(pairs) {
  const dl = el("dl", { class: "kv" });
  for (const [k, v] of pairs) if (v !== undefined) dl.append(el("dt", { text: k }), el("dd", { text: v == null ? "—" : String(v) }));
  return dl;
}
const pct = (v) => fmt.pctPlain(v);
const x = (v) => (v == null ? "—" : `${fmt.num(v, 2)}x`);

const newsTable = (items) => wrapScroll(table([
  { h: "WIB", f: (i) => (i.published ? fmt.wib(i.published).slice(5, 16) : "—"), l: true }, { h: "Source", k: "source", l: true },
  { h: "±", f: (i) => (i.sentiment > 0 ? "+" : i.sentiment < 0 ? "−" : "·"), cls: (i) => signClass(i.sentiment) },
  { h: "Headline", f: (i) => el("a", { href: i.url, target: "_blank", rel: "noopener", text: i.title }), l: true },
], items));

const BANDAR_NOTE = "Bandarmology from the IPOT broker summary: the top N buyers against the top N sellers (ranked by lots), net value as a share of the value traded: +15% big accumulation, +5% accumulation, -5% distribution, -15% big distribution. Heuristic, not advice.";

// Ownership columns that only appear when they carry data: Accts when a holder
// has several register lines, Control when the release has the 30 Oct 2026 flag.
const CONTROL_COL = { h: "Control", f: (r) => ({ CONTROLLER: "controller", "AFFILIATE OF THE CONTROLLER": "affiliate" })[r.controller_or_affiliate] ?? (r.controller_or_affiliate || "").toLowerCase(), l: true };
function ownCols(rows, controllerReported) {
  const cols = [], notes = [];
  if (controllerReported) { cols.push(CONTROL_COL); notes.push("Control: CONTROLLER_OR_AFFILIATE as the issuer reported it to IDX for the previous month."); }
  if (rows.some((r) => (r.accounts ?? 1) > 1)) { cols.push({ h: "Accts", f: (r) => (r.accounts == null ? "" : String(r.accounts)) }); notes.push("Accts: register lines summed (KSEI lists a holder once per SID and once per scrip account without one)."); }
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
    if (d.length === 1) frag.append(priceChart(d[0]));
    return frag;
  },
  history(d) {
    const rows = d.candles.slice(-40);
    const frag = document.createDocumentFragment();
    if (d.summary) {
      const s = d.summary;
      frag.append(el("p", { class: "prose", text: `${d.symbol} · ${d.name || ""} · ${d.period} / ${d.interval} · ${s.candles} candles · close ${fmt.price(s.first_close)} → ${fmt.price(s.last_close)} (${fmt.pct(s.change_pct)}) · high ${fmt.price(s.high)} · low ${fmt.price(s.low)}` }));
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
    frag.append(el("p", { class: "prose", html: `<b>${f.name || f.symbol}</b> · ${f.sector || "—"}${f.industry ? " / " + f.industry : ""} · price ${fmt.price(f.price)} · 52w ${fmt.price(f.week52_low)} – ${fmt.price(f.week52_high)} · beta ${fmt.ratio(f.beta)}` }));
    const grid = el("div", { class: "three" });
    const stmt = (title, pairs) => el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: title }), kv(pairs)]);
    grid.append(
      stmt("Valuation", [["Market cap", fmt.compact(v.market_cap)], ["Enterprise value", fmt.compact(v.enterprise_value)], ["P/E trailing", fmt.ratio(v.pe_trailing)], ["P/E forward", fmt.ratio(v.pe_forward)], ["PEG", fmt.ratio(v.peg)], ["P/B", fmt.ratio(v.price_to_book)], ["P/S", fmt.ratio(v.price_to_sales)], ["EV/EBITDA", fmt.ratio(v.ev_to_ebitda)]]),
      stmt("Profitability & health", [["ROE", pct(p.roe_pct)], ["ROA", pct(p.roa_pct)], ["Gross margin", pct(p.gross_margin_pct)], ["Operating margin", pct(p.operating_margin_pct)], ["Net margin", pct(p.net_margin_pct)], ["Debt / equity", x(h.debt_to_equity)], ["Current ratio", fmt.ratio(h.current_ratio)], ["Total cash", fmt.compact(h.total_cash)]]),
      stmt("Growth, dividend, analysts", [["Revenue growth", fmt.pct(g.revenue_growth_pct)], ["Earnings growth", fmt.pct(g.earnings_growth_pct)], ["Dividend yield", pct(d.yield_pct)], ["Payout", pct(d.payout_ratio_pct)], ["Ex-dividend", d.ex_dividend_date], ["EPS (ttm)", fmt.price(f.per_share.eps_ttm)], ["Rating", a.recommendation ? a.recommendation.replace("_", " ") : null], ["Target mean", fmt.price(a.target_mean)]]),
    );
    frag.append(grid);
    if (f.verdict) {
      const vv = f.verdict;
      const cls = vv.label === "undervalued" ? "up" : vv.label === "overvalued" ? "dn" : "";
      frag.append(el("p", { class: "prose", style: "margin-top:.75rem" }, [el("span", { class: `pill ${cls}`, text: vv.label.replace("_", " ") }), ` score ${vv.score > 0 ? "+" : ""}${vv.score}, ${vv.inputs_available} inputs · heuristic from public ratios, not investment advice`]));
      frag.append(el("ul", { class: "reasons" }, vv.reasons.map((r) => el("li", { text: r }))));
    }
    frag.append(el("p", { class: "warn", text: `Sources: ${f.sources.join(", ")}` }));
    return frag;
  },
  technical(t) {
    const frag = document.createDocumentFragment();
    const ma = t.moving_averages;
    const vs = (v) => (v == null ? "—" : `${fmt.price(v)} (${fmt.pct(v ? ((t.price - v) / v) * 100 : null)})`);
    const cls = t.signal.label === "buy" ? "up" : t.signal.label === "sell" ? "dn" : "";
    frag.append(el("p", { class: "prose", html: `<b>${t.name || t.symbol}</b> · ${t.candles_used} candles (${t.period}) · as of ${fmt.date(t.as_of)} · price ${fmt.price(t.price)} · 1d ${fmt.pct(t.change_pct_1d)} · 1m ${fmt.pct(t.change_pct_1m)} · 3m ${fmt.pct(t.change_pct_3m)} · 52w ${fmt.price(t.low_52w)} – ${fmt.price(t.high_52w)}` }));
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
  financials(f) {
    const cols = [{ h: "", k: "label", l: true }, ...f.periods.map((p, i) => ({ h: f.period === "annual" ? `FY${p.end_date.slice(0, 4)}` : p.end_date, f: (r) => r.vals[i], cls: "num" }))];
    const lines = [
      ["Total revenue", "income", "total_revenue"], ["Gross profit", "income", "gross_profit"], ["Operating income", "income", "operating_income"], ["EBITDA", "income", "ebitda"],
      ["Net interest income", "income", "net_interest_income"], ["Pretax income", "income", "pretax_income"], ["Net income", "income", "net_income"], ["EPS (diluted)", "income", "eps_diluted", "price"],
      ["Total assets", "balance", "total_assets"], ["Total liabilities", "balance", "total_liabilities"], ["Shareholders' equity", "balance", "stockholders_equity"], ["Cash & equivalents", "balance", "cash_and_equivalents"], ["Total debt", "balance", "total_debt"],
      ["Operating cash flow", "cash_flow", "operating"], ["Capital expenditure", "cash_flow", "capital_expenditure"], ["Free cash flow", "cash_flow", "free_cash_flow"], ["Dividends paid", "cash_flow", "dividends_paid"],
    ];
    const rows = lines.map(([label, s, k, kind]) => ({ label, vals: f.periods.map((p) => (kind === "price" ? fmt.price(p[s][k]) : fmt.compact(p[s][k]))) })).filter((r) => r.vals.some((v) => v !== "—"));
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", text: `${f.symbol} · ${f.period} statements · ${f.periods.length} periods · ${f.currency || ""} · T = trillion, B = billion` }));
    frag.append(wrapScroll(table(cols, rows)));
    return frag;
  },
  earnings(e) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>${e.name || e.symbol}</b> · EPS in ${e.currency || "IDR"}` }));
    if (e.quarters.length) frag.append(wrapScroll(table([{ h: "Quarter", k: "period", l: true, cls: "sym" }, { h: "Period end", f: (q) => fmt.date(q.end_date), l: true }, { h: "Reported", f: (q) => fmt.date(q.reported_date), l: true }, { h: "EPS actual", f: (q) => fmt.price(q.eps_actual) }, { h: "EPS est.", f: (q) => fmt.price(q.eps_estimate) }, { h: "Surprise", f: (q) => fmt.pct(q.surprise_pct), cls: (q) => `num ${signClass(q.surprise_pct)}` }], e.quarters)));
    const re = [...e.yearly, ...e.quarterly];
    if (re.length) frag.append(wrapScroll(table([{ h: "Period", k: "period", l: true, cls: "sym" }, { h: "Revenue", f: (r) => fmt.compact(r.revenue) }, { h: "Earnings", f: (r) => fmt.compact(r.earnings) }, { h: "Margin", f: (r) => pct(r.profit_margin_pct) }], re)));
    if (e.next) frag.append(el("p", { class: "prose", text: `Next earnings ${e.next.date || "—"}${e.next.date_is_estimate ? " (estimated)" : ""} · EPS consensus ${fmt.price(e.next.eps_avg)} (${fmt.price(e.next.eps_low)} – ${fmt.price(e.next.eps_high)}, ${e.next.analysts ?? "—"} analysts) · revenue consensus ${fmt.compact(e.next.revenue_avg)}` }));
    if (e.ex_dividend_date) frag.append(el("p", { class: "prose", text: `Ex-dividend ${e.ex_dividend_date} · payment ${e.dividend_date || "—"}` }));
    return frag;
  },
  compare(list) {
    const cols = [{ h: "Metric", k: "label", l: true }, ...list.map((f, i) => ({ h: f.symbol, f: (r) => r.vals[i], cls: "num" }))];
    const rowsDef = [
      ["Price", (f) => fmt.price(f.price)], ["Market cap", (f) => fmt.compact(f.valuation.market_cap)], ["P/E (ttm)", (f) => fmt.ratio(f.valuation.pe_trailing)], ["P/E (fwd)", (f) => fmt.ratio(f.valuation.pe_forward)],
      ["P/B", (f) => fmt.ratio(f.valuation.price_to_book)], ["EV/EBITDA", (f) => fmt.ratio(f.valuation.ev_to_ebitda)], ["ROE", (f) => pct(f.profitability.roe_pct)], ["Net margin", (f) => pct(f.profitability.net_margin_pct)],
      ["Debt/equity", (f) => x(f.health.debt_to_equity)], ["Rev growth", (f) => fmt.pct(f.growth.revenue_growth_pct)], ["Div yield", (f) => pct(f.dividend.yield_pct)], ["Target (mean)", (f) => fmt.price(f.analyst.target_mean)],
      ["Verdict", (f) => f.verdict?.label.replace("_", " ") || "—"],
    ];
    return wrapScroll(table(cols, rowsDef.map(([label, fn]) => ({ label, vals: list.map(fn) }))));
  },
  screen(s) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", text: `${s.filters.join(", ") || "no filters"} · ${s.quoted} of ${s.universe} quoted · ${s.matched_quote_level} passed quote filters${s.fundamentals_checked ? ` · fundamentals checked for ${s.fundamentals_checked}${s.fundamentals_capped ? " (capped)" : ""}` : ""} · ${s.total_matches} match${s.total_matches === 1 ? "" : "es"} · sorted by ${s.sort}` }));
    const withF = s.rows.some((r) => r.fundamentals_checked);
    const cols = [
      { h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Name", k: "name", l: true, cls: "name" }, { h: "Price", f: (r) => fmt.price(r.price) },
      { h: "Chg %", f: (r) => fmt.pct(r.change_pct), cls: (r) => `num ${signClass(r.change_pct)}` }, { h: "Mkt Cap", f: (r) => fmt.compact(r.market_cap) },
      { h: "P/E", f: (r) => fmt.ratio(r.pe_trailing) }, { h: "P/B", f: (r) => fmt.ratio(r.price_to_book) }, { h: "Div %", f: (r) => pct(r.dividend_yield_pct) },
    ];
    if (withF) cols.push({ h: "ROE", f: (r) => pct(r.roe_pct) }, { h: "Net mgn", f: (r) => pct(r.net_margin_pct) }, { h: "D/E", f: (r) => x(r.debt_to_equity) }, { h: "Sector", k: "sector", l: true });
    frag.append(wrapScroll(table(cols, s.rows)));
    if (s.rows.length < s.total_matches) frag.append(el("p", { class: "warn", text: `Showing ${s.rows.length} of ${s.total_matches}; add --limit 0 for all.` }));
    return frag;
  },
  news(n) {
    const frag = document.createDocumentFragment();
    const company = n.items.filter((i) => i.scope === "company"), market = n.items.filter((i) => i.scope !== "company");
    if (n.symbol) {
      frag.append(el("p", { class: "prose", text: `${n.company_matches ?? company.length} of ${n.scanned} scanned headlines mention ${n.symbol}${n.matched_name ? ` or ${n.matched_name}` : ""} · ${n.sources.join(", ")}` }));
      if (company.length) frag.append(newsTable(company)); else frag.append(el("p", { class: "warn", text: "No headline mentions the company in the current feeds." }));
      if (market.length) { frag.append(el("h3", { class: "stmt-title", text: `Market context · ${market.length} headlines` })); frag.append(newsTable(market)); }
    } else {
      frag.append(el("p", { class: "prose", text: `${n.items.length} of ${n.scanned} headlines · ${n.sources.join(", ")}` }));
      frag.append(newsTable(n.items));
    }
    frag.append(el("p", { class: "warn", text: "± is a lexicon read of the headline, not a recommendation." }));
    return frag;
  },
  dividends(h) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>${h.name || h.symbol}</b> · ${h.currency || "IDR"} · price ${fmt.price(h.price)} · TTM dividends ${h.ttm_total == null ? "—" : fmt.num(h.ttm_total, 2)} (${h.ttm_payments} payment${h.ttm_payments === 1 ? "" : "s"}) · TTM yield ${pct(h.ttm_yield_pct)} · ${h.consecutive_years} consecutive year${h.consecutive_years === 1 ? "" : "s"}${h.last ? ` · last ${h.last.date} ${fmt.num(h.last.amount, 2)}` : ""}` }));
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
    for (const p of list) frag.append(el("p", { class: "prose", text: `${p.symbol} · 52w ${fmt.price(p.low_52w)} – ${fmt.price(p.high_52w)} (${fmt.pct(p.pct_from_52w_high)} from high${p.days_since_52w_high != null ? `, ${p.days_since_52w_high} days ago` : ""}) · CAGR 5y ${fmt.pct(p.cagr_5y_pct)} · best day ${p.best_day_1y ? `${fmt.pct(p.best_day_1y.pct)} on ${p.best_day_1y.date}` : "—"} · worst day ${p.worst_day_1y ? `${fmt.pct(p.worst_day_1y.pct)} on ${p.worst_day_1y.date}` : "—"} · max drawdown since ${p.start}: ${p.max_drawdown_all ? `${fmt.pct(p.max_drawdown_all.pct)} (${p.max_drawdown_all.peak_date} → ${p.max_drawdown_all.trough_date}${p.max_drawdown_all.recovered ? ", recovered" : ""})` : "—"}` }));
    frag.append(el("p", { class: "warn", text: "Computed from provider closes; a window is — when the history does not reach back that far." }));
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
    if (b.news?.headlines?.length) { frag.append(el("h3", { class: "stmt-title", text: `Headlines · ${b.news.company_matches} mention the company · sentiment ${b.news.sentiment}` })); frag.append(newsTable(b.news.headlines)); }
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
      frag.append(el("p", { class: "prose", text: `Disclosed foreign stake ${r.from} → ${r.to} · ${r.issuers} issuers · ${r.inflows} inflows · ${r.outflows} outflows · net ${fmt.change(r.net_pp)} pp` }));
      frag.append(wrapScroll(table([{ h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Issuer", k: "issuer_name", l: true, cls: "name" }, { h: "Foreign % from", f: (x) => pct(x.foreign_pct_from) }, { h: "Foreign % to", f: (x) => pct(x.foreign_pct_to) }, { h: "Δ pp", f: (x) => fmt.change(x.change_pp), cls: (x) => `num ${signClass(x.change_pp)}` }, { h: "Foreign holders", k: "foreign_holders_to" }], r.rows)));
    }
    frag.append(el("p", { class: "warn", text: "Sums over holders above 1 % only (KSEI L/A flag); holders below 1 % are not published." }));
    return frag;
  },
  bandar(list) {
    const frag = document.createDocumentFragment();
    const SIG = { big_accumulation: "big acc", accumulation: "acc", neutral: "neutral", distribution: "dist", big_distribution: "big dist" };
    const sigCls = (g) => (g.endsWith("accumulation") ? "up" : g.endsWith("distribution") ? "dn" : "");
    const p1 = (v) => (v == null ? "—" : `${v > 0 ? "+" : ""}${v.toFixed(1)}%`);
    const net = (v) => `${v > 0 ? "+" : ""}${fmt.compact(v)}`;
    const KIND = { local: "local", foreign: "foreign", state_owned: "BUMN" };
    const side = (title, rows) => el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: title }), wrapScroll(table([
      { h: "Code", k: "code", l: true, cls: "sym" }, { h: "Broker", f: (r) => r.name, l: true, cls: "name" }, { h: "Class", f: (r) => KIND[r.kind] || "—", l: true },
      { h: "Lots", f: (r) => fmt.compact(r.lots) }, { h: "Value", f: (r) => fmt.compact(r.value) }, { h: "Avg", f: (r) => fmt.price(r.average_price) },
    ], rows))]);
    for (const r of list) {
      const range = r.from === r.to ? r.from : `${r.from} to ${r.to}`;
      frag.append(el("p", { class: "prose" }, [el("b", { text: r.symbol }), `${r.name ? ` · ${r.name}` : ""} · ${range} (${r.period}) · ${r.investor} investors · ${r.board === "all" ? "all boards" : `${r.board} board`}`]));
      if (!(r.total_value > 0)) { frag.append(el("p", { class: "warn", text: `No trades from ${r.from} to ${r.to}.` })); continue; }
      frag.append(el("p", { class: "prose" }, [
        el("span", { class: `pill ${sigCls(r.signal)}`, text: r.signal.replace("_", " ") }),
        ` top ${r.top} net `, el("span", { class: signClass(r.net_value), text: net(r.net_value) }), ` (${p1(r.net_pct)} of ${fmt.compact(r.total_value)} traded) · phase ${r.phase ? r.phase.replace("_", " ") : "—"} · price ${fmt.price(r.price)} (`,
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
        { h: "Net %", f: (t) => p1(t.net_pct), cls: (t) => `num ${signClass(t.net_pct)}` }, { h: "Buy share", f: (t) => (t.buy_share_pct == null ? "—" : `${t.buy_share_pct.toFixed(1)}%`) },
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
    const p1 = (v) => (v == null ? "—" : `${v > 0 ? "+" : ""}${v.toFixed(1)}%`);
    frag.append(el("p", { class: "prose" }, [el("b", { text: `Bandar screen${s.preset ? ` · ${s.preset}` : ""}` }), ` · ${s.period} ${s.from} to ${s.to} · top ${s.top} · scanned ${s.scanned} · ${s.checked} with trades · ${s.total_matches} match${s.total_matches === 1 ? "" : "es"}`]));
    frag.append(el("p", { class: "prose", text: `Filters: ${s.filters.length ? s.filters.join(", ") : "none"} · sorted by ${s.sort} ${s.descending ? "descending" : "ascending"}` }));
    if (!s.rows.length) frag.append(el("p", { class: "warn", text: "No stock matched." }));
    else frag.append(wrapScroll(table([
      { h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Signal", f: (r) => SIG[r.signal], cls: (r) => sigCls(r.signal) },
      { h: "Net %", f: (r) => p1(r.net_pct), cls: (r) => `num ${signClass(r.net_pct)}` }, { h: "Net value", f: (r) => `${r.net_value > 0 ? "+" : ""}${fmt.compact(r.net_value)}`, cls: (r) => `num ${signClass(r.net_value)}` },
      { h: "Chg %", f: (r) => fmt.pct(r.price_change_pct), cls: (r) => `num ${signClass(r.price_change_pct)}` }, { h: "Phase", f: (r) => (r.phase ? r.phase.replace("_", " ") : "—"), l: true },
      { h: "Foreign %", f: (r) => p1(r.foreign_net_pct), cls: (r) => `num ${signClass(r.foreign_net_pct)}` },
      { h: "Bandar avg", f: (r) => fmt.price(r.bandar_average == null ? null : Math.round(r.bandar_average)) }, { h: "Gain", f: (r) => p1(r.bandar_gain_pct), cls: (r) => `num ${signClass(r.bandar_gain_pct)}` },
      { h: "Price", f: (r) => fmt.price(r.price) }, { h: "Top buyers", f: (r) => r.top_buyers.join(" "), l: true },
    ], s.rows)));
    if (s.rows.length < s.total_matches) frag.append(el("p", { class: "warn", text: `Showing ${s.rows.length} of ${s.total_matches} matches; --limit 0 for all.` }));
    frag.append(el("p", { class: "warn", text: `${BANDAR_NOTE} The web screen reads a nightly scan of the session's 100 most traded stocks (after 16:45 WIB); the CLI scans live.` }));
    return frag;
  },
  broker(list) {
    const frag = document.createDocumentFragment();
    const KIND = { local: "local", foreign: "foreign", state_owned: "BUMN" };
    const cols = [
      { h: "#", k: "rank" },
      { h: "Code", k: "code", l: true, cls: "sym" },
      { h: "Broker", f: (r) => r.name, l: true, cls: "name" },
      { h: "Class", f: (r) => KIND[r.kind] || "—", l: true },
      { h: "Lots", f: (r) => fmt.compact(r.lots) },
      { h: "Value", f: (r) => fmt.compact(r.value) },
      { h: "Avg", f: (r) => fmt.price(r.average_price) },
    ];
    for (const s of list) {
      const range = s.from === s.to ? s.from : `${s.from} to ${s.to}`;
      frag.append(el("p", { class: "prose", html: `<b>${s.symbol}</b>${s.name ? ` · ${s.name}` : ""} · ${range} · ${s.investor} investors · ${s.board === "all" ? "all boards" : `${s.board} board`}` }));
      if (!s.buyers.length && !s.sellers.length) { frag.append(el("p", { class: "warn", text: `No trades from ${s.from} to ${s.to}.` })); continue; }
      frag.append(el("p", { class: "prose", html: `Value ${fmt.compact(s.total_value)} · lots ${fmt.compact(s.total_lots)} · avg ${fmt.price(s.average_price)} · foreign net <span class="${signClass(s.foreign_net_value)}">${fmt.compact(s.foreign_net_value)}</span>` }));
      const grid = el("div", { class: "two" });
      grid.append(
        el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: "Top buyers" }), wrapScroll(table(cols, s.buyers))]),
        el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: "Top sellers" }), wrapScroll(table(cols, s.sellers))]),
      );
      frag.append(grid);
    }
    frag.append(el("p", { class: "warn", text: `Source: IPOT broker summary, top 10 buyers and sellers ranked by lots. One lot = 100 shares.${list.some((s) => s.rounded) ? " The source rounds figures of a million and more to one decimal." : ""}` }));
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
      { h: "Ratio", f: (a) => (a.ratio_old != null && a.ratio_new != null ? `${fmt.price(a.ratio_old)}:${fmt.price(a.ratio_new)}` : "—") },
      { h: "Cum", f: (a) => a.cum_date || "—", l: true }, { h: "Ex", f: (a) => a.ex_date || "—", l: true },
      { h: "Recording", f: (a) => a.recording_date || "—", l: true }, { h: "Payment", f: (a) => a.distribution_date || "—", l: true },
      { h: "Status", f: (a) => a.status || "—", l: true },
    ], c.actions)));
    frag.append(el("h3", { class: "stmt-title", text: "Shareholders' meetings (RUPS)" }));
    if (!c.meetings.length) frag.append(el("p", { class: "warn", text: "None in this period." }));
    else frag.append(wrapScroll(table([
      { h: "Date", f: (m) => m.date || "—", l: true, cls: "sym" }, { h: "Time", f: (m) => m.time || "—", l: true },
      { h: "Status", f: (m) => m.status || "—", l: true }, { h: "Venue", f: (m) => m.venue || "—", l: true, cls: "name" },
    ], c.meetings)));
    if (c.warrants_rights.length) {
      frag.append(el("h3", { class: "stmt-title", text: "Warrants and rights" }));
      frag.append(wrapScroll(table([
        { h: "Code", k: "code", l: true, cls: "sym" }, { h: "Type", k: "kind", l: true },
        { h: "Exercise", f: (w) => fmt.price(w.exercise_price) }, { h: "Issued", f: (w) => fmt.compact(w.shares) },
        { h: "Listing", f: (w) => w.listing_date || "—", l: true }, { h: "Maturity", f: (w) => w.maturity_date || "—", l: true },
        { h: "Status", f: (w) => w.status || "—", l: true },
      ], c.warrants_rights)));
    }
    frag.append(el("p", { class: "warn", text: "Amount: IDR per share (the dividend, or a rights issue's exercise price). Ratio: old:new. Source: IPOT; json actions … carries each announcement and RUPS agenda." }));
    return frag;
  },
  instruments(l) {
    const frag = document.createDocumentFragment();
    const shown = l.instruments.slice(0, 200);
    frag.append(el("p", { class: "prose", text: `${l.instruments.length} of ${l.total} listed stocks${l.search ? ` · matching "${l.search}"` : ""}` }));
    if (shown.length) frag.append(wrapScroll(table([{ h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Name", f: (i) => i.name || "—", l: true, cls: "name" }], shown)));
    if (l.instruments.length > shown.length) frag.append(el("p", { class: "warn", text: `Showing the first ${shown.length}; narrow with --search or use json list for all.` }));
    return frag;
  },
  members(l) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", text: `${l.members.length} of ${l.total} exchange members${l.search ? ` · matching "${l.search}"` : ""}` }));
    if (l.members.length) frag.append(wrapScroll(table([{ h: "Code", k: "code", l: true, cls: "sym" }, { h: "Member", f: (m) => m.name || "—", l: true, cls: "name" }], l.members)));
    return frag;
  },
  bandarBroker(a) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>${a.code}</b>${a.name ? ` · ${a.name}` : ""} · ${a.period} (${a.from} to ${a.to}) · ${a.scanned} scanned, ${a.checked} with trades` }));
    frag.append(el("p", { class: "prose", text: `Visible buying ${fmt.compact(a.bought_value)} in ${a.bought.length} stocks · visible selling ${fmt.compact(a.sold_value)} in ${a.sold.length} stocks` }));
    const cols = [
      { h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Name", f: (r) => r.name || "—", l: true, cls: "name" },
      { h: "Rank", k: "rank" }, { h: "Lots", f: (r) => fmt.compact(r.lots) }, { h: "Value", f: (r) => fmt.compact(r.value) },
      { h: "Avg", f: (r) => fmt.price(r.average_price) }, { h: "Share", f: (r) => (r.share_pct == null ? "—" : `${fmt.num(r.share_pct, 1)}%`) },
      { h: "Other side", f: (r) => fmt.compact(r.opposite_value) }, { h: "Net", f: (r) => fmt.compact(r.net_value), cls: (r) => `num ${signClass(r.net_value)}` },
    ];
    const grid = el("div", { class: "two" });
    grid.append(
      el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: "Top-ten buyer of" }), a.bought.length ? wrapScroll(table(cols, a.bought)) : el("p", { class: "warn", text: "Not among the top ten buyers of any scanned stock." })]),
      el("div", { class: "stmt" }, [el("h3", { class: "stmt-title", text: "Top-ten seller of" }), a.sold.length ? wrapScroll(table(cols, a.sold)) : el("p", { class: "warn", text: "Not among the top ten sellers of any scanned stock." })]),
    );
    frag.append(grid);
    frag.append(el("p", { class: "warn", text: "From the nightly scan of the session's 100 most traded stocks (IPOT broker summary). Only the top ten per side are published; net shows where both sides are visible." }));
    return frag;
  },
  tools() {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", text: `${TOOLS.length} MCP tools (idx mcp). ${TOOLS.filter((t) => t.web).length} run here; the other ${TOOLS.filter((t) => !t.web).length} (the idx.co.id exchange tools, the Stockbit realtime tools and the local ownership database tool) run in the CLI and MCP only. Click a tool to run it, or to see its CLI command.` }));
    frag.append(wrapScroll(table([
      { h: "Tool", k: "tool", l: true, cls: "sym" }, { h: "CLI", k: "cli", l: true },
      { h: "Web terminal", f: (t) => t.web || "CLI and MCP only", l: true }, { h: "What", k: "what", l: true, cls: "name" },
    ], TOOLS)));
    frag.append(toolButtons());
    return frag;
  },
  cliOnly(t) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>${t.tool}</b> · ${t.what}` }));
    frag.append(el("pre", { text: `${t.cli}\n${t.cli.replace(/^idx /, "idx -o json ")}` }));
    const stockbit = t.tool === "idx_orderbook" || t.tool.startsWith("idx_stockbit");
    const reason = t.why || (stockbit
      ? "Stockbit realtime carries your own account session (captured once by `idx stockbit login`), so it runs in the CLI and the MCP server only, never this public page."
      : CLI_ONLY_REASON);
    frag.append(el("p", { class: "warn", text: `${reason} Install: npm install -g ihsg-cli, or the binaries on GitHub (fzn0x/idx).` }));
    return frag;
  },
  holidays(h) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>IDX trading holidays${h.year != null ? ` ${h.year}` : ""}</b>${h.trading_days != null ? ` · ${h.trading_days} trading days` : ""} · today ${h.today} is ${h.today_is_trading_day ? "a trading day" : "not a trading day"} · next trading day ${h.next_trading_day}` }));
    if (!h.holidays.length) frag.append(el("p", { class: "warn", text: "No holidays listed." }));
    else frag.append(wrapScroll(table([
      { h: "Date", k: "date", l: true, cls: "sym" }, { h: "Day", k: "weekday", l: true },
      { h: "Holiday", k: "name", l: true, cls: "name" }, { h: "English", f: (d) => d.name_en || "—", l: true },
    ], h.holidays)));
    frag.append(el("p", { class: "warn", text: `Covers ${h.years_covered.join(", ")}, from the calendar idx.co.id publishes. Weekend holidays are not listed; IDX can add a holiday later.` }));
    return frag;
  },
  sentiment(s) {
    const frag = document.createDocumentFragment();
    const cls = s.label === "positive" ? "up" : s.label === "negative" ? "dn" : "";
    frag.append(el("p", { class: "prose" }, [el("span", { class: `pill ${cls}`, text: s.label.replace("_", " ") }), ` score ${s.score > 0 ? "+" : ""}${fmt.num(s.score, 2)} · ${s.items} headlines: ${s.positive} positive, ${s.negative} negative, ${s.neutral} neutral${s.oldest ? ` · ${fmt.date(s.oldest)} → ${fmt.date(s.newest)}` : ""}`]));
    const terms = (l) => (l.length ? l.map(([t, n]) => `${t} ×${n}`).join(", ") : "—");
    frag.append(el("p", { class: "prose", text: `Terms · positive: ${terms(s.top_positive_terms)} · negative: ${terms(s.top_negative_terms)}` }));
    if (s.headlines.length) frag.append(newsTable(s.headlines));
    frag.append(el("p", { class: "warn", text: "Lexicon count over Indonesian and English finance terms. Indicative only." }));
    return frag;
  },
  market(m) {
    const frag = document.createDocumentFragment();
    frag.append(el("p", { class: "prose", html: `<b>IHSG ${fmt.price(m.index?.price)}</b> ${fmt.change(m.index?.change)} (${fmt.pct(m.index?.change_pct)}) · advancing ${m.advancing} · declining ${m.declining} · unchanged ${m.unchanged} · ${m.quoted} quoted · snapshot ${fmt.wib(m.refreshed_at)} · ${m.market_open ? "market open" : "market closed"}` }));
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
    frag.append(el("p", { class: "prose", text: `Changes ${c.from} → ${c.to}${c.symbol ? ` · ${c.symbol}` : ""} · ${c.total_matches} match${c.total_matches === 1 ? "" : "es"}${s ? ` · ${s.issuers} issuers · ${s.holders} holders` : ""}` }));
    if (s) {
      frag.append(el("p", { class: "prose", text: `Summary · ${s.new_holders} new · ${s.exits} exited · ${s.increased} increased · ${s.decreased} decreased · net ${pp(s.net_pp)} (foreign ${pp(s.foreign_net_pp)}, local ${pp(s.local_net_pp)})` }));
      if (s.by_investor_type.length) frag.append(el("p", { class: "prose", text: `By type · ${s.by_investor_type.slice(0, 6).map((t) => `${t.investor_type} ${pp(t.net_pp)} (${t.rows})`).join(" · ")}` }));
      if (!c.symbol && s.issuer_flow.length) frag.append(el("p", { class: "prose", text: `Issuer flow · ${s.issuer_flow.slice(0, 8).map((i) => `${i.symbol} ${pp(i.net_pp)} (foreign ${pp(i.foreign_net_pp)})`).join(" · ")}` }));
    }
    if (c.changes.length < c.total_matches) frag.append(el("p", { class: "warn", text: `Rows ${(c.offset || 0) + 1}–${(c.offset || 0) + c.changes.length} of ${c.total_matches} · next page: --offset ${(c.offset || 0) + c.changes.length} · all: --limit 0` }));
    const flagged = c.changes.some((r) => r.controller_or_affiliate);
    frag.append(wrapScroll(table([{ h: "Symbol", k: "symbol", l: true, cls: "sym" }, { h: "Holder", k: "holder", l: true }, { h: "Type", k: "investor_type", l: true }, { h: "L/A", k: "local_foreign", l: true }, ...(flagged ? [CONTROL_COL] : []), { h: "Kind", f: changeKind, l: true }, { h: "% from", f: (r) => pct(r.pct_from) }, { h: "% to", f: (r) => pct(r.pct_to) }, { h: "Δ pp", f: (r) => fmt.change(r.pct_change), cls: (r) => `num ${signClass(r.pct_change)}` }, { h: "Δ shares", f: (r) => fmt.change(r.shares_change), cls: (r) => `num ${signClass(r.shares_change)}` }], c.changes)));
    return frag;
  },
  releases(list) {
    return wrapScroll(table([{ h: "As of", k: "as_of", l: true, cls: "sym" }, { h: "Format", k: "format", l: true }, { h: "Rows", f: (r) => fmt.int(r.rows) }, { h: "Positions", f: (r) => fmt.int(r.positions) }, { h: "Source", k: "source", l: true }], list));
  },
};

// ---- command execution -----------------------------------------------------

function need(pos, n, usage) { if (pos.length < n) throw Object.assign(new Error(`usage: ${usage}`), { code: "usage" }); }

async function run(line) {
  const toks = tokenize(line);
  if (!toks.length) return null;
  let name = toks[0].toLowerCase();
  name = ALIASES[name] || name;
  let rawJson = false;
  if (name === "json") { rawJson = true; toks.shift(); name = (toks[0] || "").toLowerCase(); name = ALIASES[name] || name; }
  const { pos, opt } = flags(toks.slice(1));
  const cmd = COMMANDS.find((c) => c.name === name);
  if (!cmd) throw Object.assign(new Error(`unknown command "${toks[0]}"; type help`), { code: "usage" });
  const sym = (i = 0) => pos[i];
  let env, render;
  switch (name) {
    case "help": return { help: pos[0] };
    case "clear": return { clear: true };
    case "quote": need(pos, 1, cmd.usage); env = await api(`quote?symbols=${encodeURIComponent(pos.join(","))}`); render = R.quotes; break;
    case "history": need(pos, 1, cmd.usage); env = await api(`history?${qs({ symbol: sym(), period: pos[1] || opt.period || "1y", interval: pos[2] || opt.interval || "1d", limit: rawJson ? opt.limit || 0 : 0 })}`); render = R.history; break;
    case "profile": need(pos, 1, cmd.usage); env = await api(`profile?symbol=${encodeURIComponent(sym())}`); render = R.profile; break;
    case "financials": need(pos, 1, cmd.usage); env = await api(`financials?${qs({ symbol: sym(), period: pos[1] || opt.period || "annual" })}`); render = R.financials; break;
    case "earnings": need(pos, 1, cmd.usage); env = await api(`earnings?symbol=${encodeURIComponent(sym())}`); render = R.earnings; break;
    case "fundamental": need(pos, 1, cmd.usage); env = await api(`fundamental?symbol=${encodeURIComponent(sym())}`); render = R.fundamentals; break;
    case "technical": need(pos, 1, cmd.usage); env = await api(`technical?${qs({ symbol: sym(), period: pos[1] || opt.period || "1y" })}`); render = R.technical; break;
    case "compare": need(pos, 1, cmd.usage); env = await api(`compare?symbols=${encodeURIComponent(pos.join(","))}`); render = R.compare; break;
    case "screen": need(pos, 1, cmd.usage); env = await api(`screen?${qs({ filter: pos.join(","), sort: opt.sort, limit: opt.limit, max_fundamentals: opt["max-fundamentals"], asc: opt.asc ? "1" : null })}`); render = R.screen; break;
    case "news": env = await api(`news?${qs({ symbol: sym(), limit: opt.limit || 20, company_only: opt["company-only"] ? "1" : null })}`); render = R.news; break;
    case "dividends": need(pos, 1, cmd.usage); env = await api(`dividends?symbol=${encodeURIComponent(sym())}`); render = R.dividends; break;
    case "returns": need(pos, 1, cmd.usage); env = await api(`returns?${qs({ symbols: pos.join(","), benchmark: opt.benchmark })}`); render = R.returns; break;
    case "brief": need(pos, 1, cmd.usage); env = await api(`brief?symbol=${encodeURIComponent(sym())}`); render = R.brief; break;
    case "sentiment": need(pos, 1, cmd.usage); env = await api(`sentiment?symbol=${encodeURIComponent(sym())}`); render = R.sentiment; break;
    case "broker": need(pos, 1, cmd.usage); env = await api(`broker?${qs({ symbols: pos.join(","), date: opt.date, from: opt.from, to: opt.to, investor: opt.investor, board: opt.board })}`); render = R.broker; break;
    case "bandar": {
      if ((pos[0] || "").toLowerCase() === "screen") {
        env = await api(`bandar/screen?${qs({ preset: opt.preset, filter: opt.filter, period: opt.period, top: opt.top, sort: opt.sort, asc: opt.asc ? "1" : opt.desc ? "0" : null, limit: opt.limit })}`);
        render = R.bandarScreen;
      } else if ((pos[0] || "").toLowerCase() === "broker") {
        need(pos, 2, "bandar broker <CODE> [--period 1d|1w|1m|3m] [--limit N]");
        env = await api(`bandar/broker?${qs({ code: pos[1], period: opt.period, limit: opt.limit })}`);
        render = R.bandarBroker;
      } else {
        const syms = (pos[0] || "").toLowerCase() === "detect" ? pos.slice(1) : pos;
        need(syms, 1, cmd.usage);
        env = await api(`bandar?${qs({ symbols: syms.join(","), period: opt.period, top: opt.top, daily: opt.daily ? "1" : null, date: opt.date, from: opt.from, to: opt.to, investor: opt.investor, board: opt.board })}`);
        render = R.bandar;
      }
      break;
    }
    case "market": env = await api("market"); render = R.market; break;
    case "list": env = await api(`instruments?${qs({ search: opt.search || pos.join(" ") || null })}`); render = R.instruments; break;
    case "members": env = await api(`members?${qs({ search: opt.search || pos.join(" ") || null })}`); render = R.members; break;
    case "tools": {
      if (!pos[0]) return { local: `${TOOLS.length} MCP tools`, node: R.tools() };
      const t = byName(pos[0]);
      if (!t) throw Object.assign(new Error(`no tool "${pos[0]}"; type tools for the list`), { code: "usage" });
      if (t.web) return run(rawJson ? `json ${t.web}` : t.web);
      return { local: "CLI and MCP only", node: R.cliOnly(t) };
    }
    case "actions": need(pos, 1, cmd.usage); env = await api(`actions?${qs({ symbol: sym(), since: opt.since })}`); render = R.actions; break;
    case "holidays": {
      // `holidays --upcoming 2027`: the flag parser hands the year to the flag.
      const year = pos[0] || [opt.upcoming, opt.all].find((v) => typeof v === "string" && /^\d{4}$/.test(v)) || opt.year;
      env = await api(`holidays?${qs({ year, all: opt.all ? "1" : null, upcoming: opt.upcoming ? "1" : null })}`);
      render = R.holidays;
      break;
    }
    case "ownership": {
      const sub = (pos[0] || "").toLowerCase();
      if (sub === "ticker") { need(pos, 2, cmd.usage); env = await api(`ownership/ticker?${qs({ symbol: pos[1], as_of: opt["as-of"] })}`); render = R.holders; }
      else if (sub === "entity") { need(pos, 2, cmd.usage); env = await api(`ownership/entity?${qs({ name: pos.slice(1).join(" "), as_of: opt["as-of"] })}`); render = R.entity; }
      else if (sub === "changes") { need(pos, 3, cmd.usage); env = await api(`ownership/changes?${qs({ from: pos[1], to: pos[2], symbol: pos[3], min_change: opt["min-change"], limit: opt.limit, offset: opt.offset })}`); render = R.changes; }
      else if (sub === "foreign") { env = await api(`ownership/foreign?${qs({ symbol: pos[1], from: opt.from, to: opt.to, limit: opt.limit })}`); render = R.foreign; }
      else if (sub === "releases") { env = await api("ownership/releases"); render = R.releases; }
      else throw Object.assign(new Error(`usage: ${cmd.usage}`), { code: "usage" });
      break;
    }
  }
  return { env, render: rawJson ? (d, e) => el("pre", { class: "json", text: JSON.stringify(e, null, 2) }) : render };
}

// ---- sheet ---------------------------------------------------------------------

function entry(cmdLine) {
  const sec = el("section", { class: "entry" });
  const cap = el("div", { class: "caption" }, [el("span", { class: "cmd", text: `idx ${cmdLine}` }), el("span", { class: "meta", text: "…" })]);
  const body = el("div", { class: "body" });
  sec.append(cap, body);
  out.append(sec);
  return { sec, cap, body, meta: cap.lastChild };
}

function reveal(sec) {
  sec.scrollIntoView({ block: "end", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
}

/** What a typed line waits on (public/waits.js). */
function waitsOf(line) {
  const toks = tokenize(line);
  if (toks[0]?.toLowerCase() === "json") toks.shift();
  const name = (toks[0] || "").toLowerCase();
  return waitFor(ALIASES[name] || name, toks[1]);
}

/**
 * The loading state of an entry while its request is out: the entry's rule
 * runs and a line says what it waits on, with a clock. Returns a stop
 * function that takes both down and gives the elapsed milliseconds.
 */
function pending(e, line) {
  const w = waitsOf(line);
  const t0 = performance.now();
  const clock = el("span", { class: "clock", "aria-hidden": "true" });
  const status = el("p", { class: "loading", role: "status" }, [el("span", { text: `waiting on ${w.on || "the source"}` }), clock]);
  let slow = false, every = 0;
  const tick = () => {
    const ms = performance.now() - t0;
    clock.textContent = ` · ${Math.floor(ms / 1000)}s`;
    if (!slow && ms >= SLOW_MS) {
      slow = true;
      status.append(el("span", { class: "slow", text: ` · ${w.slow || "slower than usual; the source may be busy"}` }));
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
function runLine(s) { hist.push(s); hIdx = hist.length; execute(s); }

/** One button per MCP tool: a web tool runs its example, an idx.co.id tool shows its CLI command. */
function toolButtons() {
  const box = el("div", { class: "hints tools" });
  box.append(...TOOLS.map((t) => el("button", {
    type: "button", text: t.tool, class: t.web ? "" : "cli-only",
    title: `${t.what} · ${t.web ? `runs: ${t.web}` : "CLI and MCP only"}`,
    onclick: () => runLine(t.web || `tools ${t.tool}`),
  })));
  return box;
}

function renderHelp(which) {
  const frag = document.createDocumentFragment();
  const list = which ? COMMANDS.filter((c) => c.name === (ALIASES[which] || which)) : COMMANDS;
  if (!list.length) frag.append(el("p", { class: "err", text: `no command "${which}"` }));
  frag.append(wrapScroll(table([{ h: "Command", k: "usage", l: true, cls: "sym" }, { h: "What it does", k: "help", l: true }], list.map((c) => ({ usage: c.usage, help: c.help })))));
  frag.append(el("p", { class: "warn", text: "Aliases: q h f t n s c d r b own. Prefix any command with json for the raw envelope. Symbols accept BBCA or BBCA.JK; several symbols are comma or space separated." }));
  if (!which) {
    frag.append(el("h3", { class: "stmt-title", text: `MCP tools · ${TOOLS.length} · click to run (dashed: CLI and MCP only)` }));
    frag.append(toolButtons());
  }
  return frag;
}

async function execute(line) {
  const e = entry(line);
  input.value = "";
  const stop = pending(e, line);
  const took = (ms) => (ms >= 1000 ? ` · took ${(ms / 1000).toFixed(1)}s` : "");
  try {
    const r = await run(line);
    const ms = stop();
    if (!r) { e.sec.remove(); return; }
    if (r.clear) { out.replaceChildren(); return; }
    if ("help" in r) { e.meta.textContent = ""; e.body.append(renderHelp(r.help)); return; }
    if (r.local) { e.meta.textContent = r.local; e.body.append(r.node); return; }
    const env = r.env;
    e.meta.textContent = `${env.provider} · ${env.cached ? "cached" : "live"}${env.stale ? " · stale" : ""} · fetched ${fmt.wib(env.fetched_at.replace("Z", "+00:00")).replace(" WIB", " UTC")}${took(ms)}`;
    for (const w of env.warnings || []) e.body.append(el("p", { class: "warn", text: w }));
    e.body.append(r.render(env.data, env));
  } catch (err) {
    const ms = stop();
    e.meta.textContent = `${err.code ? `error · ${err.code}` : "error"}${took(ms)}`;
    e.body.append(el("p", { class: "err", text: err.message }));
    if (err.code === "usage") e.body.append(el("p", { class: "warn", text: "Type help for the command list." }));
  }
  reveal(e.sec);
  input.focus();
}

// history + completion
const hist = [];
let hIdx = -1;
form.addEventListener("submit", (ev) => {
  ev.preventDefault();
  const line = input.value.trim();
  if (!line) return;
  hist.push(line); hIdx = hist.length;
  execute(line);
});
input.addEventListener("keydown", (ev) => {
  if (ev.key === "ArrowUp" && hist.length) { ev.preventDefault(); hIdx = Math.max(0, hIdx - 1); input.value = hist[hIdx]; }
  else if (ev.key === "ArrowDown" && hist.length) { ev.preventDefault(); hIdx = Math.min(hist.length, hIdx + 1); input.value = hist[hIdx] || ""; }
  else if (ev.key === "Tab" && input.value && !input.value.includes(" ")) {
    const hit = COMMANDS.map((c) => c.name).find((n) => n.startsWith(input.value.toLowerCase()));
    if (hit) { ev.preventDefault(); input.value = `${hit} `; }
  }
});

// hints + deep links (?cmd=)
const SAMPLE = ["market", "brief BBCA", "quote BBCA", "returns BBCA BBRI BMRI", "dividends TLKM", "fundamental BBCA", "technical BBRI", "screen pe<10,div>5,mcap>5T --sort div", "news TLKM", "broker BBCA", "bandar BBCA --daily", "bandar screen --preset big-accumulation", "bandar broker YP", "actions BBCA", "holidays --upcoming", "compare BBCA BBRI BMRI", "ownership ticker BBCA", "tools", "help"];
document.getElementById("hints").append(...SAMPLE.map((s) => el("button", { type: "button", text: s, onclick: () => runLine(s) })));

api("market").then((m) => { dateline.textContent = `${fmt.wib(m.data.as_of || m.data.refreshed_at)} · ${m.data.market_open ? "market open" : "market closed"} · ${m.data.quoted} quoted`; }).catch(() => { dateline.textContent = "snapshot unavailable"; });

const initial = new URLSearchParams(location.search).get("cmd");
if (initial) execute(initial);
