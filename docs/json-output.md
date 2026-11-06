# JSON output

Add `-o json` (or `IDX_OUTPUT=json`, or `output = "json"` in the config file)
and every command prints one JSON document to stdout. Errors go to stderr as
JSON too. Nothing else is ever written to stdout in JSON mode, so
`idx -o json ... | jq` is safe. `-o jsonl` prints one record per line instead
(see [JSON Lines](#json-lines--o-jsonl)). `-o auto` prints a table when stdout
is a terminal and this JSON document when it is piped or captured; it is
opt-in, the default stays `table`. Commands without an envelope (`version`,
`schema`, `config get`, `cache stats|clear`, `mcp setup`, `stockbit
status|logout`) follow the same flag, `IDX_OUTPUT` and config file.

Machine-readable schemas: `idx schema <name>` with `quotes`, `history`,
`profile`, `fundamentals`, `compare`, `financials`, `earnings`, `dividends`,
`returns`, `market`, `brief`, `technical`, `screen`, `news`, `sentiment`,
`insights`, `ownership-holders`, `ownership-entity`, `ownership-changes`,
`ownership-foreign`, `ownership-foreign-flow`, `ownership-releases`,
`ownership-import`, `ownership-discover`, `ownership-search`,
`ownership-cross-holders`, `ownership-concentration`, `ownership-graph`,
`ownership-resolve-candidates`, `ownership-resolution`,
`ownership-resolutions`, `ownership-composition`, `ownership-snapshot-sync`,
`ownership-status`, `ownership-snapshot`, `ownership-institutions`,
`watchlist`, `error`, `quote`, `candle`.

`-o csv` prints the payload's main list (candles, rows, items, holders,
changes, dividends, ...) as comma-separated rows with a header; nested objects
become dotted columns (`valuation.pe_trailing`), arrays of scalars join with
`|`, and a payload with no list becomes one row. Warnings still go to stderr.

## JSON Lines (`-o jsonl`)

`-o jsonl` (or `IDX_OUTPUT=jsonl`, or `output = "jsonl"`) writes one compact
JSON value per line to stdout, each ending in `\n`, and nothing else. Each
line is exactly the object `-o json` carries at that path: same keys, same
order, nulls kept. No new schema: every line is a shape `idx schema <name>`
already documents.

- Which list becomes lines is fixed per schema (table below), not guessed from
  the data. An empty list prints nothing and exits 0.
- The envelope is not printed. Its `warnings` and the stale notice go to stderr
  as `warning: ...` lines, as in table mode (`--quiet` hides them). Provenance
  (`provider`, `fetched_at`, `cached`, `stale`) and header fields such as a
  screen's funnel counts or the changes summary are only in `-o json`.
- Errors are the same one-line JSON on stderr as with `-o json`.
- Commands without an envelope print one line per item of an array, else one
  line. `*path` commands and `completions` keep their plain output.
- Table-only flags (`history --limit`, `broker --top`, `dividends --limit`,
  `financials --statement`) never trim JSON, JSONL or CSV.

| schema | one line per |
| --- | --- |
| `idx.quotes.v1`, `idx.compare.v1`, `idx.returns.v1`, `idx.broker.v1`, `idx.bandar.v1`, `idx.ownership.releases.v1`, `idx.ownership.import.v1`, `idx.ownership.discover.v1` | item of `data` |
| `idx.history.v1` | `data.candles[]` |
| `idx.financials.v1` | `data.periods[]` |
| `idx.dividends.v1` | `data.dividends[]` (yearly totals, splits and TTM figures stay in `-o json`) |
| `idx.screen.v1`, `idx.bandar.screen.v1`, `idx.ownership.foreign-flow.v1` | `data.rows[]` |
| `idx.watchlist.v1` | `data.symbols[]`, JSON strings (`idx -o jsonl watch list \| jq -r .`) |
| `idx.holidays.v1` | `data.holidays[]` |
| `idx.instruments.v1` | `data.instruments[]` |
| `idx.members.v1` | `data.members[]` |
| `idx.indices.v1` | `data.indices[]` |
| `idx.calendar.v1` | `data.events[]` |
| `idx.foreign.v1` | `data.days[]` |
| `idx.market.brokers.v1` | `data.brokers[]` |
| `idx.news.v1` | `data.items[]` |
| `idx.ownership.holders.v1` | `data.holders[]` |
| `idx.ownership.entity.v1` | `data.holdings[]` |
| `idx.ownership.changes.v1` | `data.changes[]` |
| `idx.ownership.foreign.v1` | `data.points[]` |
| `idx.profile.v1`, `idx.fundamentals.v1`, `idx.technical.v1`, `idx.brief.v1`, `idx.sentiment.v1` | `data` on one line (a single record) |
| `idx.earnings.v1`, `idx.actions.v1`, `idx.market.v1`, `idx.market.foreign.v1`, `idx.bandar.broker.v1`, `idx.orderbook.v1` | `data` on one line (several peer lists kept together) |

```sh
idx -o jsonl stocks history BBCA --period 1mo | jq -c '{time, close}'
idx -o jsonl market holidays --year 2026 | wc -l          # 22
```

The same rule reads idx's own output back: `-` in a ticker list accepts an
`-o json` envelope or `-o jsonl` lines on stdin and takes each row's `symbol`
(JSON strings are tickers), so
`idx -o jsonl stocks screen -f "pe<10" -n 5 | idx stocks compare -` works
without jq. A bare ticker read this way is an IDX one whatever `IDX_EXCHANGE`
says, because that is what idx meant when it wrote it (`D05.SI` keeps its
suffix); the same holds for a watchlist file (first line `# idx watchlist`).
Other text on stdin is read like typed arguments and follows `IDX_EXCHANGE`.

## Envelope

Every successful command wraps its payload:

```json
{
  "schema": "idx.quotes.v1",
  "provider": "yahoo",
  "fetched_at": "2026-10-16T17:45:07Z",
  "cached": false,
  "stale": false,
  "warnings": ["no data for ZZZZ"],
  "data": ...
}
```

| field | type | meaning |
| --- | --- | --- |
| `schema` | string | Payload shape and version. Changes only when a field is renamed or removed. |
| `provider` | string | Source that produced the data (`yahoo`, later `msn`, ...). |
| `fetched_at` | RFC 3339 UTC | When the underlying response left the network. For cached responses this is the original fetch time. |
| `cached` | bool | Served from the local cache. |
| `stale` | bool | The cached copy was older than its TTL and was used anyway (`--offline`, or the network failed). |
| `stale_reason` | string? | Why `stale` is true: `offline` (`--offline`: the network was not tried) or `fetch_failed` (the live request failed; a `live fetch failed (...); showing cached copy` warning names the failure). Omitted when `stale` is false. |
| `warnings` | string[] | Non-fatal notes. Omitted when empty. |
| `data` | object or array | The payload, see below. |

## `idx.quotes.v1` (`stocks quote`)

`data` is an array of quote objects, one per symbol the source knew. Symbols
it did not know appear in `warnings`; if none were known the command fails
with exit code 4.

| field | type | notes |
| --- | --- | --- |
| `symbol` | string | IDX ticker without suffix, e.g. `BBCA`; a ticker from another Yahoo Finance market keeps its suffix, e.g. `D05.SI` (see below) |
| `name` | string? | Company name |
| `currency` | string? | `IDR` |
| `price` | number? | Last trade |
| `change`, `change_pct` | number? | Versus previous close; `change_pct` is in percent (`-14.81`) |
| `previous_close`, `open`, `day_high`, `day_low` | number? | |
| `volume`, `avg_volume_3m` | integer? | Shares |
| `market_cap` | number? | In `currency` |
| `pe_trailing`, `pe_forward`, `eps_ttm`, `price_to_book` | number? | |
| `dividend_yield_pct` | number? | Percent, e.g. `6.27` |
| `week52_high`, `week52_low`, `sma50`, `sma200` | number? | |
| `shares_outstanding` | integer? | |
| `market_state` | string? | `REGULAR`, `CLOSED`, ... |
| `market_time` | RFC 3339? | Last trade time in WIB (`+07:00`) |
| `source` | string | Endpoint that produced the row: `yahoo:quote` (full) or `yahoo:chart` (reduced fallback) |
| `behind_session` | bool? | `true` when `market_time` is dated (WIB) before the latest IDX session that has ended (16:15 WIB, the end of post-closing; weekends and the bundled IDX holidays are skipped): the stock is suspended or did not trade, or the source lags the close. `false` when the last trade is from that session or later. `null` when `market_time` is unknown (Stockbit, MSN rows without a trade) or the symbol does not trade on IDX (`^GSPC`, `^N225`). Computed on every call, never cached. For years the holiday list does not cover only weekends are skipped, and unscheduled closures read as behind. |

`?` means the value is `null` when the source did not return it. Nothing is
estimated. When `source` is `yahoo:chart`, expect `market_cap`, P/E, dividend
and average-volume fields to be `null`.

Other markets: the Yahoo-backed commands (`stocks quote`, `history`,
`profile`, `financials`, `earnings`, `dividends`, `returns`, `fundamental`,
`technical`, `compare`) also take a ticker with another Yahoo Finance suffix,
or a bare one with `IDX_EXCHANGE` set. In every payload an IDX ticker stays
bare (`BBCA`) and another market's keeps its suffix (`D05.SI`); `currency` is
what Yahoo reports (`SGD`), and times are still given in WIB (`+07:00`): the
instant is right, the wall clock is Jakarta's, not the exchange's. Dates that
name a trading session (the chart fallback's `previous_close`, and `price_date`
and the dividend dates in `idx.dividends.v1`) are taken on the exchange's own
clock, so a Toronto close at 16:00 EST counts for that day, not the next.

## `idx.history.v1` (`stocks history`)

```json
{
  "symbol": "TLKM",
  "name": "...",
  "currency": "IDR",
  "period": "1y",
  "interval": "1wk",
  "candles": [
    { "time": "2025-10-13T09:00:00+07:00", "open": 2900.0, "high": 2960.0, "low": 2850.0,
      "close": 2880.0, "adj_close": 2804.6, "volume": 512345600 }
  ],
  "summary": {
    "start": "2025-10-13T09:00:00+07:00", "end": "2026-10-16T09:00:00+07:00",
    "first_close": 2880.0, "last_close": 1900.0, "change": -980.0, "change_pct": -34.03,
    "high": 3990.0, "low": 1900.0, "avg_volume": 578600000, "candles": 54
  }
}
```

- `candles` is oldest first and is **never** truncated by `--limit` (that flag
  only affects the table).
- Bars with a missing open/high/low/close (holidays, or the current session
  before its first trade) are dropped.
- `adj_close` is present when the source supplies split/dividend adjusted
  closes.
- `summary` is `null` when there are no candles.

## `idx.screen.v1` (`stocks screen`)

`data.rows[]` carry the quote-level fields (`price`, `change_pct`,
`market_cap`, `pe_trailing`, `pe_forward`, `price_to_book`,
`dividend_yield_pct`, `eps_ttm`, `avg_volume_3m`, 52-week range) and, when
`fundamentals_checked` is true, `sector`, `industry`, `roe_pct`, `roa_pct`,
`net_margin_pct`, `operating_margin_pct`, `debt_to_equity`,
`revenue_growth_pct`, `earnings_growth_pct`, `peg`, `price_to_sales`.
`data.universe`, `quoted`, `matched_quote_level`, `fundamentals_checked`,
`fundamentals_capped` and `total_matches` describe the funnel, so a reader can
tell a genuine "no match" from a capped fundamentals pass.

Filters apply to the values as published. `price_to_book` comes from the quote
and is negative for a company whose shareholders' equity is below zero, so
`pb<1` matches those too; add `pb>0` (`pb>0,pb<1`) to keep only companies
trading below a positive book value. A negative `debt_to_equity` means the
same, so `de<0.5` matches it too; add `de>0`. For an issuer that reports in
USD, `price_to_book` and `price_to_sales` divide the IDR price by USD figures,
so `pb` and `ps` filters say little about those (see `financial_currency`
under `idx.fundamentals.v1`).

Added in 0.8.2, all optional:

- `data.preset`: the preset name (`top-performers`, `worst-performers`,
  `high-dividend`, `low-pe`, `52w-high`, `52w-low`, `high-volume`,
  `large-cap`) or `null`. Presets are idx's own definitions: a clause list plus
  a sort key and direction (`top-performers` sorts by `ret1y` descending,
  `low-pe` is `pe>0` ascending, `52w-high` is `range>=95`, `52w-low`
  `range<=5`, ...; `idx stocks screen --help` lists them). `data.filters`
  lists every clause applied, the preset's first.
- `rows[].range_52w_pct`: where the price sits in the 52-week range, computed
  by idx from the quote, 0 at the low and 100 at the high (not clamped; `null`
  without all three figures or when the high equals the low). Filter and sort
  key `range`.
- `rows[].return_1w_pct`, `return_1m_pct`, `return_3m_pct`, `return_6m_pct`,
  `return_ytd_pct`, `return_1y_pct`: price returns in percent as MSN reports
  them (keys `ret1w`, `ret1m`, `ret3m`, `ret6m`, `retytd`, `ret1y`). They come
  from one request for MSN's whole-market screener list, made only when a
  filter, the sort or `--returns` needs them, and are `null` otherwise or when
  MSN lists no value. A filter or sort that needs them fails rather than
  returning an empty screen when the list cannot be fetched.
- `data.returns_source`: `msn:screener` when the return windows were joined,
  else `null`.

## `idx.news.v1` (`stocks news`) and `idx.sentiment.v1` (`stocks sentiment`)

News items: `title`, `url`, `source` (`kontan/investasi`,
`cnbcindonesia/market`, ...), `published` (WIB), `summary`, `matched_by`
(`ticker` | `name` | `all`), `scope` (`company` when the item mentions the
requested ticker, else `market`), `sentiment` (-1..+1), `positive_terms`,
`negative_terms`. With a symbol, `items` lists company headlines first and
then market headlines up to the limit; `company_matches` counts the company
headlines before the limit and `company_only` records whether market context
was suppressed. The sentiment report (company headlines only) adds `label`
(`positive` | `negative` | `neutral` | `no_coverage`), `score`, counts, the
covered period, top terms and the strongest `headlines`, plus `method`
describing the lexicon rule. MSN Money's crowd sentiment is not part of it:
`Finance/SentimentBrowser` returns no data for IDX tickers in any market
tried (checked 2026-11-06), so `idx.sentiment.v1` stays headline-based.

## `idx.insights.v1` (`stocks insights`)

MSN Money's rule-based statements about one company:

```json
{
  "symbol": "BBCA", "name": "Bank Central Asia Tbk PT", "updated": "2026-11-05T17:51:31Z",
  "good": 17, "bad": 4, "neutral": 0, "not_rated": 9,
  "peers": ["BBKP", "BABP", "SUPA", "INPC"],
  "insights": [
    { "category": "Valuation", "name": "P/B ratio",
      "statement": "BBCA is trading at a P/B multiple (2.10x) above weighted average of its peers (0.56x)",
      "short_statement": "...", "status": "bad", "criteria": "lower_is_better",
      "value": 2.09778355810005, "benchmark_value": 0.564114114036551, "compared_with": "peers" }
  ]
}
```

- `insights[]` keeps MSN's order: `Valuation`, `Earnings`, `Growth`,
  `Performance`, `Health`. The same `name` can appear twice with different
  benchmarks (peers and industry).
- `status`: `good`, `bad`, `neutral` or `not_rated` (MSN's `NA`, usually
  missing data; an unknown rating is also listed as `not_rated`, with a
  warning). `criteria`: `lower_is_better` or `higher_is_better`.
- `value` and `benchmark_value` are as MSN reports them and their units differ
  by statement: multiples (P/E 10.2), fractions (growth 0.038), rupiah
  (revenue). For a statement MSN could not rate, a 0/0 pair is its placeholder
  for missing data and is reported as `null`.
- `compared_with` is derived by idx from MSN's benchmark object: `peers` (the
  peer group's aggregate), `industry` (the industry aggregate), `own_history`
  (the company's earlier quarters or years), `price` (analyst target against
  the current price) or `threshold` (a fixed level, e.g. the analyst rating
  scale); `null` for a benchmark idx does not recognise.
- `peers`: the IDX tickers in MSN's peer group, resolved with one
  `Finance/Quotes` call; empty with a warning when that lookup fails.
- `updated`: when MSN last recomputed the statements (UTC), about once a day.

An unknown ticker exits 4; a rejected MSN key exits 3 and names
`IDX_MSN_API_KEY`.

## `idx.dividends.v1` (`stocks dividends`)

`dividends[]` (`date` = ex-date, `amount` per share as reported, split-adjusted
on Yahoo) and `splits[]` (`date`, `numerator`, `denominator`, `ratio`) oldest
first, with `price` and `price_date` from the same response. Computed by idx
and documented as such: `years[]` (`year`, `payments`, `total`, `change_pct`),
`last`, `ttm_total`, `ttm_payments`, `ttm_yield_pct` (= TTM total / price),
`consecutive_years`, `as_of`.

## `idx.broker.v1` (`broker`)

Array, one summary per requested ticker, as Indo Premier (IPOT) publishes it:
`symbol`, `name` (bundled universe), `from` and `to` (the range the source
applied, WIB dates), `investor` (`all` | `foreign` | `domestic`), `board`
(`all` | `regular` | `cash` | `negotiated`), `buyers[]` and `sellers[]` (top 10
ranked by lots, most first: `rank`, `code`, `name` from the bundled IDX member
list or `null`, `kind` = `local` | `foreign` | `state_owned` or `null`, `lots`
(1 lot = 100 shares), `value` in IDR, `average_price`), and the range totals
`total_value`, `total_lots`, `foreign_net_value` (foreign buy minus sell),
`average_price` (`null` when nothing traded). The source abbreviates figures
of a million and more to one decimal (`1.6 M`); those arrive as plain numbers
at that precision and the summary carries `rounded: true`. Nothing is netted
per broker or estimated. A range without trades has empty lists and a warning.
With `-o csv` the rows are one broker per side: `symbol, from, to, investor,
board, side (buy|sell), rank, code, name, kind, lots, value, average_price`.

## `idx.bandar.v1` (`bandar detect`)

Array, one reading per requested ticker, derived from the IPOT broker summary
(see `idx.broker.v1`) and the price; the rules are in `src/analysis/bandar.rs`.
`symbol`, `name`, `period` (`1d` | `1w` | `1m` | `3m`, or `custom` for
explicit dates), `from`, `to`, `investor`, `board`, `top` (N behind the
headline: 1, 3, 5 or 10), `signal` (`big_accumulation` | `accumulation` |
`neutral` | `distribution` | `big_distribution`: top-N net at +15 %, +5 %,
-5 %, -15 % of value), `phase` (`quiet_accumulation` | `markup` |
`quiet_distribution` | `markdown` | `neutral`, `null` without a price move),
`net_value` (IDR), `net_lots` and `net_pct` (of value) of the top-N tier,
`price` (last price of the final session: the quote's when the range ends on
the quote's session, otherwise the close), `price_change_pct` (from the close
before `from`), `bandar_average` (volume-weighted average of the top-N
buyers) and `bandar_gain_pct` (`price` against it), `total_value`,
`total_lots`, `foreign_net_value`, `foreign_net_pct`, `tiers[]` (one per N in
1, 3, 5, 10: `top`, `buy_value`, `sell_value`, `net_value`, `buy_lots`,
`sell_lots`, `net_lots`, `net_pct`, `buy_share_pct`, `sell_share_pct`,
`buy_average`, `sell_average`, `signal`), `buyers[]` and `sellers[]` (the
top N, shaped as in `idx.broker.v1`), `daily` (`null` unless `--daily`:
`days[]` of `date`, `total_value`, `net_value`, `net_pct`, `signal`,
`cumulative_net_value`, `foreign_net_value`, then `accumulation_days`,
`distribution_days`, `neutral_days`, `streak` (+3 = three accumulation
sessions in a row, -2 = two distribution sessions) and
`cumulative_net_value`), `reasons[]` and `rounded`. A range without trades
reads `neutral` with a single reason.

## `idx.bandar.screen.v1` (`bandar screen`)

An object: `period`, `from`, `to`, `top`, `preset` (or `null`), `filters`
(every clause, the preset's first), `sort`, `descending`, `scanned` (tickers
considered: the most traded by value in the latest session, or the ones
given), `checked` (those with trades), `total_matches` and `rows[]`: `symbol`,
`name`, `signal`, `phase`, `net_pct`, `net_value`, `net_lots`, `price`,
`price_change_pct`, `bandar_average`, `bandar_gain_pct`, `buy_share_pct`,
`total_value`, `foreign_net_value`, `foreign_net_pct`, `top_buyers[]` and
`top_sellers[]` (broker codes), `rounded`. Filter keys: `acc` (`net_pct`),
`net`, `lots`, `fnet`, `fpct`, `gain`, `chg`, `turnover`, `conc`
(`buy_share_pct`), `price`, `signal=`, `phase=`. With `-o csv` the rows are
the screen rows.

## `idx.bandar.broker.v1` (`bandar broker`)

An object: `code`, `name` (bundled member list), `period`, `from`, `to`,
`scanned`, `checked` (scanned stocks with trades), `failed`, `bought_value`
and `sold_value` (sums of `value` over every row, before `--limit`), then
`bought[]` (stocks where the broker is a top-10 buyer) and `sold[]` (top-10
seller), largest value first: `symbol`, `name`, `rank` on that side, `lots`,
`value`, `average_price`, `share_pct` (of all value traded in the stock),
`opposite_value` (the broker's value on the other side when it is also top 10
there, else `null`) and `net_value` (buy minus sell, in both lists, when both
sides are visible). Only the top 10 per side are published, so a broker below
tenth place in a stock is absent; nothing is estimated for it.

## `idx.actions.v1` (`stocks actions`)

An object, from IPOT: `symbol`, `name`, `since` (the source's filter;
upcoming events are included), `actions[]` newest cum date first (`kind` =
`cash_dividend` | `stock_dividend` | `stock_split` | `reverse_split` | `bonus`
| `rights_issue` | `warrant` | `other`, `label` as published, `amount` (IDR
per share: the dividend, or a rights issue's exercise price; `null` where the
source shows 0), `ratio_old` and `ratio_new` (old : new, `null` for cash
dividends), `cum_date`, `ex_date`, `recording_date`, `distribution_date`,
`end_date` (end of a rights trading period, or the last step of a split or
bonus), `status`, `details`), `meetings[]` (RUPS: `date`, `time`, `status`,
`venue`, `agenda`) and `warrants_rights[]` (`code`, `kind`, `exercise_price`,
`shares`, `listing_date`, `maturity_date`, `status`, `details`).

## `idx.foreign.v1` (`stocks foreign`)

An object, from idx.co.id's daily stock summary: `symbol`, `name`, `from`,
`to`, `days[]` oldest first (`date`, `close`, `change`, `volume`, `value`,
`foreign_buy`, `foreign_sell`, `foreign_net` in shares, and
`foreign_net_value_at_close`) and `totals` (`sessions`, `foreign_buy`,
`foreign_sell`, `foreign_net`, `foreign_net_value_at_close`,
`net_buy_sessions`, `net_sell_sessions`, `streak`: +3 = three net-buy
sessions in a row, -2 = two net-sell sessions, 0 when the latest was flat).
IDX publishes foreign trading in shares only; `foreign_net_value_at_close` is
net shares × close, a rupiah scale for comparing stocks, not the value
foreigners traded at. `volume` and `value` exclude non-regular (negotiated)
trades, as the source reports them.

## `idx.market.foreign.v1` (`market foreign`)

An object for one session: `date`, `stocks`, the market sums `foreign_buy`,
`foreign_sell`, `foreign_net` (shares) and `foreign_net_value_at_close`,
`sort` (`value` | `shares`), and `net_buy[]` / `net_sell[]` (largest first):
`symbol`, `name`, `close`, `change`, `change_pct`, `volume`, `value`,
`foreign_buy`, `foreign_sell`, `foreign_net`, `foreign_net_value_at_close`.

## `idx.market.brokers.v1` (`market brokers`)

An object for one session: `date`, `members` (with trades), `total_volume`,
`total_value`, `total_frequency`, `sort` (`value` | `volume` | `frequency`)
and `brokers[]`: `rank`, `code`, `name` (as idx.co.id publishes it),
`volume`, `value`, `frequency`, `value_share_pct`. A member's buying and
selling count together, so `total_value` is twice the market's value.

## `idx.indices.v1` (`market indices`)

`date` and `indices[]` in the source's order (IHSG first): `code`
(`COMPOSITE` is IHSG), `previous`, `high`, `low`, `close`, `change`,
`change_pct` (computed), `constituents`, `volume`, `value`, `frequency`,
`market_cap`.

## `idx.calendar.v1` (`market calendar`)

`from`, `to`, `symbol` (filter or `null`), `categories` (shown), `total`
(entries in the range before the category filter) and `events[]` oldest
first: `date`, `code` (a ticker, or an issuer such as `DJPU` for government
bonds), `category` (`dividend` | `rights` | `bonus` | `split` | `warrant` |
`rups` | `public_expose` | `buyback` | `listing` | `bond` | `other`),
`milestone` (`cum`, `ex`, `recording`, `payment`, `distribution`, `listing`,
`trading_start`, `trading_end`, `plan`, `notice`, `result`, `material`,
`coupon`, `principal`, `maturity`, `report`, or `null`), `description` as
published, `location`, `meeting_at` (RUPS date and time, WIB), `expose_date`,
`source_type` and `source_step` (the source's own fields) and `id`. Category
and milestone are classified by idx from the wording; entries the source
lists twice are shown once.

## `idx.holidays.v1` (`market holidays`)

`year` (or `null` for every year), `years_covered`, `trading_days` (weekdays
minus holidays in `year`), `today`, `today_is_trading_day`,
`next_trading_day` and `holidays[]` (`date`, `weekday`, `name`, `name_en`).
Bundled; `provider` is `bundled`.

## `idx.instruments.v1` (`stocks list`) and `idx.members.v1` (`market members`)

`idx.instruments.v1`: `total`, `board` and `search` (filters or `null`) and
`instruments[]` (`symbol`, `name`, `listing_date`, `shares`, `board`).
`idx.members.v1`: `total`, `search` and `members[]` (`code`, `name`,
`licenses[]`). When idx.co.id cannot be reached and nothing is cached, the
bundled lists stand in (`provider: bundled`, a warning, and `null` for what
they lack).

## `idx.returns.v1` (`stocks returns`)

Array, one `Performance` per requested ticker: `as_of`, `price`, `start`,
`candles_used`, `returns` (`1w 1m 3m 6m ytd 1y 3y 5y`, percent, `null` when
the history is too short), `cagr_3y_pct`, `cagr_5y_pct`, `volatility_1y_pct`
(annualised stdev of daily log returns), `max_drawdown_1y` and
`max_drawdown_all` (`pct`, `peak_date`, `trough_date`, `recovered`),
`best_day_1y`, `worst_day_1y`, `high_52w`, `low_52w`, `pct_from_52w_high`,
`pct_from_52w_low`, `days_since_52w_high`, and `benchmark` (`symbol`, `name`,
`returns`, `excess` in percentage points) or `null`. Everything is computed
from five years of daily closes; dividends are excluded.

## `idx.market.v1` (`market overview`)

`as_of`, `market_state`, `behind_session` (`true` when even the newest quote,
`as_of`, is dated before the latest IDX session that has ended; same rule as
the quote field), `index` (a `MarketRow` for `^JKSE`), `universe`,
`quoted`, `advancing`, `declining`, `unchanged`, `turnover`,
`liquid_min_market_cap`, and the lists `gainers`, `losers`, `most_active`,
`heavyweights` of `MarketRow` (`symbol`, `name`, `price`, `change`,
`change_pct`, `volume`, `value`, `market_cap`). The web terminal serves the
same shape plus `index_tape`, `refreshed_at` and `market_open` (false on
weekends and IDX holidays). Its quote, market and screen answers come from a
board a cron refreshes every minute in trading hours and every 15 minutes
outside them; a board older than that allows (10 or 35 minutes) is served
with `stale: true`, `stale_reason: "fetch_failed"` and a warning giving its
time.

## `idx.brief.v1` (`stocks brief`)

`symbol`, `name`, `generated_at`, `key_points[]` (plain-language lines), then
one section per topic, each either `null` or the dedicated command's object:
`quote` (one `Quote`, with `behind_session`; a key point names the last trade
date when it is `true`), `fundamentals` (`idx.fundamentals.v1` data),
`technical`, `returns` (one `Performance`), `dividends` (`ttm_total`,
`ttm_payments`, `ttm_yield_pct`, `consecutive_years`, `last`,
`payments_on_record`), `ownership` (`as_of`, `listed_pct`, `foreign_pct`,
`holders_listed`, `top_holders[]` with the same fields as `ownership ticker`),
`news` (`company_matches`, `sentiment`,
`score`, `headlines[]`), and `sections_failed[]` naming the `null` sections,
with the reason in the envelope's `warnings`.

## `idx.ownership.*.v1` (`ownership ...`)

`ticker` (`idx.ownership.holders.v1`) and `entity` (`idx.ownership.entity.v1`)
list `Holding` rows: one holder's position in one issuer, the holder's
register lines (one per SID, one per scrip account without an SID) summed.
Since 0.8.2 each row adds `accounts` (lines summed; `null` for months imported
by idx 0.8.1 or older, which kept one line) and `controller_or_affiliate`
(`CONTROLLER`, `AFFILIATE OF THE CONTROLLER` or `null`, from the register as
of 30 Oct 2026); both reports add `controller_reported` (whether the release
has that column). `changes` rows add `accounts_from`, `accounts_to` and
`controller_or_affiliate` (later release, earlier one for exits). `releases`
rows keep `rows` (register lines) and add `positions`, `other_date_rows` and
`controller_column` (`null` for months imported by idx 0.8.1 or older).
Notes about the months read (imported by an old idx, rows dated otherwise)
come in the envelope's `warnings`. The versions stay `v1`: every change is an
added field, and the values now match the documented "one entity's position".

The analytics read the same positions of one release (`as_of`; percentages on
the 0-100 scale, rounded to 4 decimals at the end):

- `search` (`idx.ownership.search.v1`): `query`, `as_of`, `total_matches`,
  `matches[]` with `entity_key` (pass it to `entity --exact` or `graph`),
  `holder`, `matched` (`exact`, `spelling`, `prefix`, `contains`, `words`),
  `spellings[]` (every spelling recorded, any month), `issuers` and
  `total_pct` in `as_of` (0 when absent that month), `investor_type`,
  `local_foreign` (largest position; `null` when absent) and `last_seen`.
- `cross-holders` (`idx.ownership.cross-holders.v1`): the filters applied
  (`min_issuers`, `investor_type`, `origin`), `total_matches` and `holders[]`
  with `issuers`, `total_pct` (a breadth measure, not a share of anything),
  `max_pct` and `positions[]` (`symbol`, `issuer_name`, `shares_total`, `pct`).
- `concentration` (`idx.ownership.concentration.v1`): `sort` (`hhi`, `top1`,
  `top3`), `ascending`, `total_matches`, `controller_reported` and `rows[]`
  with `holders_listed`, `top_holder`, `top1_pct`, `top3_pct`, `listed_pct`,
  `undisclosed_pct`, `hhi` (sum of the listed holders' squared stakes,
  0-10,000) and `controller_pct` (`null` when the release has no controller
  column).
- `graph` (`idx.ownership.graph.v1`): `root` (`ticker:<SYMBOL>` or
  `holder:<entity_key>`), `depth`, `min_pct`, `max_nodes`, `truncated`,
  `issuers[]` and `holders[]` with their `hops`, then `links[]` (`from`, `to`,
  `holder`, `symbol`, `shares_total`, `pct`, `hops`). `links` comes last, so
  `-o csv` prints the links.
- `resolve candidates` (`idx.ownership.resolve-candidates.v1`): `keep` and
  `merge` (`entity_key`, `holder`, `issuers`, `last_seen`), `reasons`
  (`renamed`, `similar_name`), `renamed[]` (`from`, `to`, `symbols`) and
  `listed_together[]` (`as_of`, `symbol`).
- `resolve merge|map|undo` (`idx.ownership.resolution.v1`): a list of
  `action`, `dry_run`, `resolution` (`id`, `kind`, `subject`, `holder`,
  `entity_key`, `combined[]`, `created_at`), `spellings_moved`,
  `lines_moved`, `positions_moved`, `positions_combined` and `releases[]`.
  `resolve list` (`idx.ownership.resolutions.v1`) lists the stored
  `resolution` objects.

`discover` (`idx.ownership.discover.v1`) entries add `family` (`above1`,
`above5`, `investor-type`, `classification` or `composition`) and
`importable` (`true` for `above1` and `composition`); `origin` can now also be
`ksei.co.id`. The envelope's `provider` names where the entries came from:
`idx` (idx.co.id), `github-mirror` (the mirror, when idx.co.id refused),
`ksei` (ksei.co.id only, as with `--family composition`), or `idx+ksei` and
`github-mirror+ksei` when the families were listed from both.
`import` (`idx.ownership.import.v1`) reports add `family`
(`above1` or `composition`) and `other_securities` (composition only: rows of
other security types left out). For a composition month `release.rows` and
`release.issuers` count the stocks stored, `release.entities` is 0 and
`positions`, `other_date_rows` and `controller_column` are `null`. Both stay
`v1` (added fields).

## `idx.ownership.composition.v1` (`ownership composition`)

KSEI's monthly "Kepemilikan Efek" archive for one stock (see
[ownership.md](ownership.md#ksei-composition-kepemilikan-efek)). `provider` is
`ksei` when the call downloaded the month, `local-db` otherwise. With `-o csv`
the rows are the nine investor types, whatever the number of imported months:
`symbol, as_of, code, investor_type, local_shares, foreign_shares, local_pct,
foreign_pct`.

```json
{
  "symbol": "BBCA", "as_of": "2026-10-30", "price": 4980.0,
  "shares_listed": 123275050000, "shares_scripless": 52453691120, "scripless_pct": 42.5501,
  "local_shares": 17194269088, "foreign_shares": 35259422032,
  "local_pct": 32.7799, "foreign_pct": 67.2201,
  "by_type": [
    {"code": "ID", "investor_type": "INDIVIDUAL", "local_shares": 12030567470,
     "foreign_shares": 324800480, "local_pct": 22.9356, "foreign_pct": 0.6192}
  ],
  "history": [
    {"as_of": "2026-09-30", "local_pct": 31.1106, "foreign_pct": 68.8894, "scripless_pct": 42.5501, "foreign_change_pp": null},
    {"as_of": "2026-10-30", "local_pct": 32.7799, "foreign_pct": 67.2201, "scripless_pct": 42.5501, "foreign_change_pp": -1.6693}
  ],
  "source": "https://www.ksei.co.id/storage/Download/BalanceposEfek20261030.zip"
}
```

`shares_scripless` is the local total plus the foreign total: the shares held
in KSEI accounts. Shares still held as certificates are outside the archive.
Every `*_pct` but `scripless_pct` is a percent of `shares_scripless` (4
decimals, `null` when it is 0); `scripless_pct` is `shares_scripless` over
`shares_listed`. `by_type` always lists the nine types in KSEI's order (`IS`,
`CP`, `PF`, `IB`, `ID`, `MF`, `SC`, `FD`, `OT`). `history` is every imported
month for the stock, oldest first; `foreign_change_pp` is the change from the
previous imported month. `price` is `null` when KSEI left it blank.

## `idx.ownership.snapshot-sync.v1` (`ownership sync`) and `idx.ownership.status.v1` (`ownership status`)

`sync` (provider `github-mirror`): `action` (`installed`, `updated`,
`imported` or `up_to_date`), `database`, `snapshot` (the mirror's snapshot for
this idx's schema: `file`, `generated_at`, `db_schema_version`, `oldest`,
`newest`, `releases`, `bytes`; `null` when the mirror has none), the months
`installed` from it, `imported` from the monthly files (missing or
re-published months), `kept` (only in the
local database, kept without `--force`) and `dropped` (replaced with
`--force`), `backup` (the `.bak` path or `null`), and `releases` / `newest`
after the sync. The name is not `idx.ownership.sync.v1` because the web
terminal's `/api/ownership/status` already answers with that one (the state of
its D1 loader).

`status` (provider `local-db`): `database`, `exists`, `bytes`,
`db_schema_version`, `supported_schema_version`, `releases`, `oldest`,
`newest`, `installed_snapshot` (from the file's `snapshot_info` table, `null`
for a database built by local imports), `remote` (`mirror`,
`index_generated_at`, `releases`, `newest`, `snapshot`; `null` offline, with
`IDX_OWNERSHIP_MIRROR=off` or when the mirror cannot be read), `missing`,
`outdated` (months held as the installed snapshot carried them while
`index.json` lists a re-published file; sync imports them again),
`local_only` and `sync` (`up_to_date`, `install`, `update`, `import` or `null`
without `remote`). Status never creates the database file.

`idx.ownership.snapshot.v1` is the manifest CI publishes as
`snapshot-v{V}.json` next to the snapshot file (and the `data` of the hidden
`ownership build-snapshot -o json`); see
[ownership.md](ownership.md#snapshot-and-sync).

## `idx.watchlist.v1` (`watch add|rm|list|clear`)

`path` and `symbols[]`. `idx watch` itself returns `idx.quotes.v1` and
`idx watch returns` returns `idx.returns.v1`.

## `idx.profile.v1` (`stocks profile`)

`data` is one object: `symbol`, `name`, `sector`, `industry`, `summary`,
`employees`, `founded_year`, `website`, `city`, `country`, `exchange`,
`currency`, `market_cap`, `shares_outstanding`, `directors[]` (`name`,
`title`; populated by MSN, empty from Yahoo) and `source`.

## `idx.fundamentals.v1` (`stocks fundamental`)

`data` is one object grouped by theme. Percent fields end in `_pct` and are
already percentages (`21.8` = 21.8 %). Ratios are plain multiples
(`debt_to_equity: 0.6` = 60 % of equity).

```json
{
  "symbol": "BBCA", "name": "PT Bank Central Asia Tbk", "currency": "IDR",
  "financial_currency": "IDR", "sector": "Financial Services", "industry": "Banks - Regional",
  "price": 5175.0, "beta": -0.017, "week52_high": 8750.0, "week52_low": 4820.0,
  "valuation": { "market_cap": 6.357e14, "enterprise_value": 6.099e14, "pe_trailing": 12.87,
                 "pe_forward": 10.73, "peg": 1.52, "price_to_book": 2.35, "price_to_sales": 5.82,
                 "ev_to_ebitda": null, "ev_to_revenue": 5.58 },
  "profitability": { "roe_pct": 21.82, "roa_pct": 3.67, "gross_margin_pct": null,
                     "operating_margin_pct": 66.75, "net_margin_pct": 53.12 },
  "health": { "debt_to_equity": null, "current_ratio": null, "quick_ratio": null,
              "total_debt": 5.2e13, "total_cash": 7.8e13 },
  "growth": { "revenue_growth_pct": 2.5, "earnings_growth_pct": 0.4,
              "revenue_cagr_5y_pct": 8.63, "eps_growth_3y_pct": 12.21 },
  "dividend": { "yield_pct": 6.27, "payout_ratio_pct": 75.42, "ex_dividend_date": "2026-08-31" },
  "per_share": { "eps_ttm": 402.07, "eps_forward": 482.48, "book_value": 2201.5, "revenue": 888.2 },
  "analyst": { "recommendation": "strong_buy", "target_mean": 8197.5, "target_high": 10100.0,
               "target_low": 5900.0, "analysts": 24 },
  "fiscal_year_end": "2025-12-31", "most_recent_quarter": "2026-06-30",
  "verdict": { "label": "fair", "score": 2, "reasons": ["trailing P/E 12.9 is moderate (10 to 15)", "..."],
               "inputs_available": 7 },
  "sources": ["yahoo:quoteSummary", "msn:equities", "msn:quotes"]
}
```

- `auto` asks Yahoo first and fills remaining `null`s from MSN; `sources`
  lists who contributed. Values are never averaged across sources.
- Yahoo reports gross and EBITDA margins as `0` for banks; that is published
  as `null` because it means "not applicable", not 0 %.
- MSN writes `±999.9999` in a ratio field (forward P/E, PEG, P/B, P/S,
  debt/equity) where it cannot compute one, notably debt/equity when
  shareholders' equity is negative; that is published as `null`.
- `currency` is the price's currency. `financial_currency` is the currency of
  the financial statements when the source names it (Yahoo
  `financialCurrency`; MSN does not, so it is `null` under `--provider msn`).
  Coal, oil and gas, and airline issuers such as ITMG, MEDC and GIAA report in
  USD: their `price_to_book`, `price_to_sales`, `ev_to_ebitda` and
  `ev_to_revenue` then divide the IDR price by USD figures (ITMG P/B 14103 on
  2026-11-06), and `per_share.book_value`, `per_share.revenue`,
  `health.total_debt` and `health.total_cash` are in USD. idx publishes them
  as reported, never converted; the verdict does not score P/B then, and the
  tables mark those multiples with `*`. Under `auto` none of these eight
  fields is filled from MSN, which names no statement currency, so each of
  them is Yahoo's or `null`. P/E, EPS and the dividend yield are consistent.
- `verdict.label` is `undervalued` | `fair` | `overvalued` |
  `insufficient_data` (fewer than three rule inputs). Every rule that fired is
  in `reasons`. It is a heuristic, not advice.
- A negative `per_share.book_value` (or a negative `price_to_book` or
  `debt_to_equity`) means shareholders' equity is below zero. The verdict then
  scores one adverse rule (-2, "negative shareholders' equity ...") and sets
  the P/B, ROE and debt/equity rules aside; those figures still count in
  `inputs_available`. An ROE above 15 % next to a net loss (negative net
  margin or EPS) is not rewarded either. A P/B that mixes currencies is
  neither scored nor counted, and no reason is listed for it.

## `idx.compare.v1` (`stocks compare`)

`data` is an array of `idx.fundamentals.v1` objects in the order requested.
Symbols that failed are listed in `warnings`.

## `idx.financials.v1` (`stocks financials`)

```json
{
  "symbol": "BBCA", "currency": "IDR", "period": "annual",
  "periods": [
    { "end_date": "2025-12-31", "period": "annual",
      "income": { "total_revenue": 1.14e14, "gross_profit": null, "operating_income": null,
                  "ebitda": null, "ebit": null, "interest_income": 9.8e13, "interest_expense": 1.0e13,
                  "net_interest_income": 8.8e13, "pretax_income": 7.1e13, "tax_provision": 1.4e13,
                  "net_income": 5.75e13, "net_income_common": 5.75e13, "eps_basic": 467.0, "eps_diluted": 467.0 },
      "balance": { "total_assets": 1.59e15, "total_liabilities": 1.31e15, "stockholders_equity": 2.81e14,
                   "cash_and_equivalents": 9.1e13, "total_debt": 2.4e12, "net_debt": null, "shares_outstanding": 1.23e11 },
      "cash_flow": { "operating": 7.75e13, "investing": -3.4e13, "financing": -4.2e13,
                     "capital_expenditure": -2.45e12, "free_cash_flow": 7.5e13, "dividends_paid": -3.76e13 } }
  ],
  "source": "yahoo:timeseries"
}
```

- `period` is `annual` (about four fiscal years), `quarterly` (about five
  quarters) or `ttm` (trailing twelve months; `balance` fields are `null`).
- `periods` is oldest first. Amounts are in `currency` units as reported;
  outflows are negative.
- Banks have no EBITDA, gross profit or operating income series at the source,
  so those are `null`.

## `idx.earnings.v1` (`stocks earnings`)

`data`: `quarters[]` (`period` like `3Q2025`, `end_date`, `reported_date`,
`eps_actual`, `eps_estimate`, `surprise_pct`), `yearly[]` and `quarterly[]`
(`period`, `revenue`, `earnings`, `profit_margin_pct`), `next` (`date`,
`date_is_estimate`, `eps_avg`/`eps_low`/`eps_high`, `eps_growth_pct`,
`revenue_avg`/`revenue_low`/`revenue_high`, `analysts`), `ex_dividend_date`,
`dividend_date`, `source`.

## `idx.technical.v1` (`stocks technical`)

Computed locally from daily candles (`provider` names where the candles came
from). `data`: `as_of`, `period`, `candles_used`, `price`,
`change_pct_1d|1w|1m|3m`, `high_52w`, `low_52w`,
`moving_averages` (`sma20|50|200`, `ema20|50|200`), `rsi14`,
`macd` (`line`, `signal`, `histogram`), `bollinger` (`upper`, `middle`,
`lower`, `percent_b`, `bandwidth_pct`), `atr14`, `volume` (`last`, `avg20`,
`ratio`), `levels` (`support[]`, `resistance[]`, nearest first),
`trend` (`uptrend` | `downtrend` | `sideways` | `unknown`) and
`signal` (`label` `buy` | `sell` | `neutral`, `score`, `reasons[]`).
Indicators that need more history than was available are `null`.

## `idx.orderbook.v1` (`stockbit orderbook`)

Realtime via the Stockbit provider (your own session; see
[stockbit.md](stockbit.md)). `provider` is `stockbit`. `data`: `symbol`,
`name`, `last`, `change`, `change_pct`, `previous_close`, `open`, `day_high`,
`day_low`, `volume` (shares), `value` (IDR), `bids[]` and `asks[]` (each a
level `{price, shares, orders}`, best price first), and the session foreign
flow `foreign_buy`, `foreign_sell`, `foreign_net` (IDR). Stockbit flags these
three with `has_foreign_bs`; while it is not `true` (seen during trading, when
the fields still hold the previous session's figures) they are `null` and
`warnings` says why. Sizes are shares (one IDX lot is 100 shares). A level the
book does not show, or a field the source omits, is absent/`null`.

## `idx.ownership.institutions.v1` (`ownership institutions`)

From MSN Money's ownership data (`provider` is `msn`), live, no import.
`data`: `symbol`, `name`, `institutions_pct`, `top10_institutions_pct`,
`funds_pct` (percent of shares outstanding, as MSN reports them),
`float_pct` (free float, percent of shares outstanding), `oldest_report`,
`newest_report`, `flow` (`institutions` and `funds`, each `new`, `buyers`,
`sellers`, `exited`, `unchanged`, `net_shares_change`) and `holders[]`
(institutions first, then funds): `kind` (`institution` | `fund`), `rank`,
`name`, `shares`, `shares_change` (since that holder's previous report),
`pct_outstanding`, `pct_outstanding_change` (percentage points),
`pct_portfolio` (funds only, percent of the fund), `report_date` (each
holder's own) and `signal` (`new` | `buyer` | `seller` | `exited` |
`unchanged`, computed by idx; `null` when `shares` or `shares_change` is
missing). Funds overlap their managers: never add the two lists. A stock MSN
lists no holders for has empty `holders` and a warning. `-o csv` prints one
row per holder; with no holders it prints the holder header alone, so files
for several symbols share one layout. See [ownership.md](ownership.md#institutional-and-fund-holders-msn).

## Errors

Written to stderr, exit code non-zero. With `-o json`, `-o jsonl` or `-o auto`
when piped (from the flag, `IDX_OUTPUT` or the config file, the flag first) an
error is one JSON line; otherwise it is `error: ...` text, with a `hint:` line
where one helps:

```json
{"error":{"code":"not_found","message":"not found: no quotes for ZZZZ"},"exit_code":4}
```

| exit | `code` | when |
| --- | --- | --- |
| 1 | `internal`, `io` | unexpected failure |
| 2 | `usage` | bad flags or symbol |
| 3 | `network`, `provider`, `blocked` | network failure or the source returned an error; `blocked`: the source's bot protection (Cloudflare) refused this network with a challenge or a block page. Retry later or from another network, use cached data (`--offline`), or for the ownership register the GitHub mirror |
| 4 | `not_found` | symbol or record does not exist |
| 5 | `offline_miss` | `--offline` and nothing cached |
| 6 | `config`, `unsupported` | bad config file or env value; or the selected provider cannot serve this command (`--provider auto` can) |

## Compatibility promise

- Adding an optional field to a payload does not bump the `schema` version.
- Renaming, removing or changing the type/unit of a field does, and the old
  version is kept for at least one minor release behind a flag.
- Numbers are plain JSON numbers (no strings), timestamps are RFC 3339.
