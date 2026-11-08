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
`intraday`, `profile`, `fundamentals`, `growth`, `valuation`, `risk`,
`compare`, `financials`, `earnings`, `dividends`, `returns`, `market`,
`brief`, `technical`, `screen`, `news`, `sentiment`, `insights`,
`foreign-value`, `astro`, `ownership-holders`, `ownership-entity`, `ownership-changes`,
`ownership-foreign`, `ownership-foreign-flow`, `ownership-releases`,
`ownership-import`, `ownership-discover`, `ownership-search`,
`ownership-cross-holders`, `ownership-concentration`, `ownership-graph`,
`ownership-resolve-candidates`, `ownership-resolution`,
`ownership-resolutions`, `ownership-composition`, `ownership-snapshot-sync`,
`ownership-status`, `ownership-snapshot`, `ownership-institutions`,
`watchlist`, `stockbit-actions`, `stockbit-movements`, `stockbit-status`,
`error`, `quote`, `candle`.

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
| `idx.quotes.v1`, `idx.compare.v1`, `idx.returns.v1`, `idx.broker.v1`, `idx.bandar.v1`, `idx.ownership.releases.v1`, `idx.ownership.import.v1`, `idx.ownership.discover.v1`, `idx.ownership.resolution.v1`, `idx.ownership.resolutions.v1` | item of `data` |
| `idx.history.v1` | `data.candles[]` |
| `idx.intraday.v1` | `data.bars[]` (the quote and the summary stay in `-o json`) |
| `idx.financials.v1` | `data.periods[]` |
| `idx.dividends.v1` | `data.dividends[]` (yearly totals, splits and TTM figures stay in `-o json`) |
| `idx.screen.v1`, `idx.bandar.screen.v1`, `idx.ownership.foreign-flow.v1`, `idx.ownership.concentration.v1` | `data.rows[]` |
| `idx.watchlist.v1` | `data.symbols[]`, JSON strings (`idx -o jsonl watch list \| jq -r .`) |
| `idx.holidays.v1` | `data.holidays[]` |
| `idx.instruments.v1` | `data.instruments[]` |
| `idx.members.v1` | `data.members[]` |
| `idx.indices.v1` | `data.indices[]` |
| `idx.calendar.v1` | `data.events[]` |
| `idx.foreign.v1`, `idx.foreign.value.v1` | `data.days[]` |
| `idx.market.brokers.v1` | `data.brokers[]` |
| `idx.news.v1` | `data.items[]` |
| `idx.insights.v1` | `data.insights[]` (the peers stay in `-o json`) |
| `idx.ownership.holders.v1`, `idx.ownership.institutions.v1`, `idx.ownership.cross-holders.v1` | `data.holders[]` |
| `idx.ownership.entity.v1` | `data.holdings[]` |
| `idx.ownership.changes.v1` | `data.changes[]` |
| `idx.ownership.foreign.v1` | `data.points[]` |
| `idx.ownership.search.v1` | `data.matches[]` |
| `idx.ownership.resolve-candidates.v1` | `data.candidates[]` |
| `idx.stockbit.actions.v1` | `data.actions[]` |
| `idx.stockbit.movements.v1` | `data.movements[]` |
| `idx.profile.v1`, `idx.fundamentals.v1`, `idx.growth.v1`, `idx.valuation.v1`, `idx.risk.v1`, `idx.technical.v1`, `idx.brief.v1`, `idx.sentiment.v1`, `idx.ownership.snapshot.v1` | `data` on one line (a single record) |
| `idx.earnings.v1`, `idx.actions.v1`, `idx.market.v1`, `idx.market.foreign.v1`, `idx.market.foreign.value.v1`, `idx.bandar.broker.v1`, `idx.orderbook.v1`, `idx.ownership.graph.v1`, `idx.ownership.composition.v1`, `idx.ownership.snapshot-sync.v1`, `idx.ownership.status.v1`, `idx.astro.v1` | `data` on one line (several peer lists kept together) |

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
| `stale` | bool | The cached copy was older than its TTL and was used anyway (`--offline`, or the network failed or the source refused the request). |
| `stale_reason` | string? | Why `stale` is true: `offline` (`--offline`: the network was not tried) or `fetch_failed` (the live request failed, or the source's bot protection refused it, such as an idx.co.id challenge that stayed through the retries; a `live fetch failed (...); showing cached copy` warning names the failure). Omitted when `stale` is false. |
| `warnings` | string[] | Non-fatal notes. Omitted when empty. |
| `data` | object or array | The payload, see below. |

An answer built from several responses (a brief's sections, several
tickers, a range read one session at a time) carries all of them: `cached`
is true only when every response came from the cache, `stale` when any was,
and `fetched_at` is the oldest one's fetch time (a screen's per-company
fundamentals count toward `cached` and `stale` only). So `cached: false`
with a `fetched_at` from before the call means some parts were fetched for
it and the others came from the cache; the web terminal labels such an
answer partly cached.

## `idx.quotes.v1` (`stocks quote`)

`data` is an array of quote objects, one per symbol the source knew. Symbols
it did not know appear in `warnings`; if none were known the command fails
with exit code 4.

| field | type | notes |
| --- | --- | --- |
| `symbol` | string | IDX ticker without suffix, e.g. `BBCA`; a ticker from another Yahoo Finance market keeps its suffix, e.g. `D05.SI` (see below) |
| `name` | string? | Company name |
| `currency` | string? | `IDR` |
| `financial_currency` | string? | Currency of the financial statements when the source names it (Yahoo's `financialCurrency`; `null` from MSN, Stockbit and the chart fallback). Issuers such as BSSR, GEMS and ITMG report in `USD`: their `price_to_book` then divides the IDR price by a USD book value (BSSR 48,921.57 on 2026-11-06), published as reported, never converted. P/E, EPS and the dividend yield are in the price's terms |
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
| `behind_session` | bool? | `true` when `market_time` is dated (WIB) before the latest IDX session that has ended (16:15 WIB, the end of post-closing; weekends and the bundled IDX holidays are skipped): the stock is suspended or did not trade, or the source lags the close. `false` when the last trade is from that session or later. `null` when `market_time` is unknown (Stockbit, MSN rows without a trade) or the symbol does not trade on IDX (`^GSPC`, `^N225`, another market's `D05.SI`). Computed on every call, never cached. For years the holiday list does not cover only weekends are skipped, and unscheduled closures read as behind. |

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
- `interval` is the bar size the source actually returned. Yahoo can answer
  with coarser bars than asked; the payload then carries its real size and a
  warning says so. `--period max` with `1d` or `1wk` asks Yahoo for the whole
  history (`period1=0`), so it is daily or weekly (about 9,200 daily bars for
  `^JKSE`, from 1990-04-06), not the monthly bars `range=max` returns.
  The web terminal caches a history in one D1 row (2 MB): one that would not
  fit (all daily bars of an index older than IHSG) keeps its most recent
  candles, `summary` still covers them all, and a warning says so.
- `adj_close` is present when the source supplies split/dividend adjusted
  closes.
- `summary` is `null` when there are no candles.
- `interval` is one of `1m 2m 5m 15m 30m 1h 1d 1wk 1mo`. One-minute bars
  cover at most 5 days (`period` `1d` or `5d`) and two-minute bars at most
  `1mo`; a longer window is a usage error (exit 2), because Yahoo answers
  HTTP 422 beyond 8 days of 1m and 60 days of 2m data.

## `idx.intraday.v1` (`stocks intraday`)

The latest session of one stock or index by the minute (or the bar size asked
for): today's session while the market trades, otherwise the last one.
Yahoo's chart runs about 10 minutes behind the market (its quote says
`exchangeDataDelayedBy: 10`; on 2026-11-06 at 09:13:21 WIB the last row was
stamped 09:03:20), so its one-day window is empty for the first minutes after
the 09:00 open (no bars, and the previous session has dropped out; the first
row of the day came at 09:10 WIB). idx then reads the five-day window and
shows its last session, with a warning naming the date.

```json
{
  "symbol": "BBCA",
  "name": "PT Bank Central Asia Tbk",
  "session": "2026-11-05",
  "interval": "1m",
  "quote": { "symbol": "BBCA", "price": 5125.0, "change": 195.0, "change_pct": 3.955,
             "previous_close": 4930.0, "open": 5050.0, "volume": 166336900,
             "market_state": "PREPRE", "market_time": "2026-11-05T16:14:43+07:00", "...": "..." },
  "bars": [
    { "time": "2026-11-05T09:00:00+07:00", "open": 5025.0, "high": 5100.0, "low": 5025.0,
      "close": 5100.0, "adj_close": null, "volume": 0 }
  ],
  "summary": {
    "first_bar": "2026-11-05T09:00:00+07:00", "last_bar": "2026-11-05T16:14:00+07:00",
    "bars": 333, "open": 5025.0, "high": 5125.0, "low": 5025.0, "close": 5125.0,
    "bars_volume": 155142100, "vwap": 5089.98
  }
}
```

| field | type | notes |
| --- | --- | --- |
| `session` | date? | WIB date of the bars; `null` when there are none |
| `interval` | string | Bar size asked for: `1m` (default), `2m`, `5m`, `15m`, `30m`, `1h` |
| `quote` | object? | The `idx.quotes.v1` quote object as its source reports it; `null` (with a warning) when the quote could not be fetched |
| `bars` | array | Candles as in `idx.history.v1`, oldest first; `-o json` carries every bar (`--limit` only trims the table), the MCP tool keeps the last 120 by default |
| `summary` | object? | Over every bar of the session; `null` without bars |
| `summary.open`, `close` | number | First bar's open, last bar's close |
| `summary.high`, `low` | number | Highest high, lowest low |
| `summary.bars_volume` | integer | Sum of the bars' volume, shares |
| `summary.vwap` | number? | Volume-weighted average of each bar's typical price, (high + low + close) / 3, computed by idx and rounded to 2 decimals; `null` when no bar has volume |

- The summary describes the bars, the quote the session as the source reports
  it, and the two differ: on Yahoo the opening auction's volume is in no bar
  (the 09:00 bar shows volume 0, so `bars_volume` is below `quote.volume`), the
  first bar's open is not always the opening price, the pre-closing auction
  lands in the 16:00 bar, and 16:01 to 16:14 are trades at the closing price.
- Minutes without a trade have no bar; the lunch break has none (Monday to
  Thursday 12:00 to 13:30, Friday 11:30 to 14:00).
- VWAP is an approximation: IDX's average price is the regular market's value
  divided by its volume, which needs every trade. For BBCA on 2026-11-05 the
  bars give 5,089.98 against IDX's 5,088.5.
- Indices carry no volume on Yahoo, so their `vwap` is `null` and
  `bars_volume` is 0.
- Warnings: no quote (and why), the five-day retry when the 1-day window is
  empty (for some minutes after the open, or no trade today), no bars (the
  stock did not trade, or the source has none outside trading hours), and bars
  and quote from different dates.
- In session, and in multi-day windows after the close, Yahoo ends with a row
  stamped at the last trade time (e.g. 09:02:16 or 16:14:43); it is a snapshot,
  not a bar, and is left out, in `idx.history.v1` too. Bars sit on the
  interval's grid from the window's first bar (09:00), so a row off that grid,
  or one that repeats the time of the row before it, is the snapshot.
- With `--provider stockbit` the bars are Stockbit's realtime one-minute
  candles for the day (none outside the session).

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

`rows[].currency` and `rows[].financial_currency` (optional) are the price's
currency and the statements' when the source names them: Yahoo's `currency`
and `financialCurrency` from the quote, else from the fundamentals, as under
`idx.quotes.v1` and `idx.fundamentals.v1`. A row's `price_to_book` and
`price_to_sales` are both in `financial_currency` terms: a fundamentals report
over statements in another currency fills neither (under `auto`, an MSN report
for a company whose quote names USD statements, since MSN names no statement
currency).

Filters apply to the values as published, with one exception below.
`price_to_book` comes from the quote and is negative for a company whose
shareholders' equity is below zero, so `pb<1` matches those too; add `pb>0`
(`pb>0,pb<1`) to keep only companies trading below a positive book value. A
negative `debt_to_equity` means the same, so `de<0.5` matches it too; add
`de>0`.

The exception: when a row's `financial_currency` differs from its `currency`
(issuers such as BSSR, GEMS and ITMG report in USD), its `price_to_book` and
`price_to_sales` divide the IDR price by USD figures (BSSR's P/B 48,921.57 on
2026-11-06). The row keeps them as reported, but they are not ratios, so `pb`
and `ps` read them as missing: no `pb` or `ps` clause matches the row, low or
high, and a `pb` or `ps` sort puts it last, as for a row without the value.
Through 0.8.2 they were compared as published, so such a row passed `pb>3`
and led `--sort pb`. P/E, EPS and the dividend yield are in the price's terms
and filter as before. The table marks such a P/B with `*` and names those
companies under it.

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
`average_price` (`null` when nothing traded), `buy_value` and `sell_value`
(what the selected investors bought and sold, IDR). What the source's
totals mean depends on the filters:

- All investors: `total_value`, `total_lots` and `average_price` are the
  range's; `buy_value` and `sell_value` both equal the value traded.
- One side (`investor` `foreign` or `domestic`): the source's totals are that
  side's buying, so `total_value`, `total_lots` and `average_price` are
  `null`, `buy_value` is the side's buying as published, and `sell_value` is
  computed by idx from the foreign net: foreign `buy_value -
  foreign_net_value`, domestic `buy_value + foreign_net_value`. BBCA on
  2026-11-05: foreign investors bought 605.4 B and sold 301.5 B, domestic
  investors bought 242.8 B, of 848.3 B traded.
- A board filter: the source's foreign net covers every board whatever the
  filter, so `foreign_net_value` is `null`, and so is a side's `sell_value`.

The source abbreviates figures
of a million and more to one decimal (`1.6 M`); those arrive as plain numbers
at that precision and the summary carries `rounded: true`. Nothing is netted
per broker or estimated. A range without trades has empty lists and a warning.
Over a range that starts before the source's latest session and contains it,
the source's foreign net counts that session twice (its value, lots and lists
are right). When a range reached the last two trading days at the time it
was read, `foreign_net_value` is therefore the sum of the last two sessions
with trades, each read on its own, and of the rest of the range. That is
usually three more requests (four when the range reaches a session still
trading), up to six when the latest sessions had no trades (single sessions
are read back from the end, at most five, until two had trades), and more
only when a part of the range cached while it was current has to be split
again; most of them are cached. If those cannot be read it is `null` with a
warning.
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
reads `neutral` with a single reason. `foreign_net_value` over several
sessions is rebuilt from single sessions as described for `idx.broker.v1`;
`bandar screen` reads its ranges the same way, while `bandar broker`, which
shows no foreign net, reads only the range pages. With an
`investor` filter the brokers are that side's, while `total_value`,
`total_lots` and every percentage stay those of the stock's value traded,
read from the all-investor summary of the same range and board (one more
request per range, and per session with `--daily`).

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

The web terminal answers `bandar screen` and `bandar broker` from the Worker's
scan of the last session instead of scanning live: from 16:45 WIB after each
close it waits for IPOT to publish the session's broker lists, then reads the
100 most traded stocks. The shapes are the same. When the newest trading
session is not the one shown, `warnings` say which one is shown and why (the
newer scan is waiting for IPOT, running, ended without IPOT publishing the
session or answering, read too few stocks, or starts at 16:45 WIB) and, from
two sessions behind, how many sessions behind it is. They also count the shown
session's stocks without a broker summary or not published by IPOT and say
when the scan reads those again or, when that would be at or after the next
session's 16:45 WIB, that they count as failed or not published then. They
also say when its stocks were picked from the quote board of another session
(a scan that started after the next open). Each stock counts once in
`scanned`. `fetched_at` is when the shown session's scan last stored a stock.
Before it stored one, it is when the session's record was filed, or when the
scan started while it has no record yet. For a session the Worker's earlier
code scanned to the end, it is when that scan ended; for one it kept no record
of, 16:45 WIB of the session, the earliest its scan could have read it.

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
trades, as the source reports them. `--days N` (MCP `days`) gives the latest
N sessions; `--from D [--to D]` (MCP `from`, `to`) the sessions of a range
among the latest 250. `--source ipot` (MCP `source: "ipot"`) answers with
`idx.foreign.value.v1` instead.

## `idx.foreign.value.v1` (`stocks foreign --source ipot`)

An object, from Indo Premier's broker summary (`idx schema foreign-value`):
`symbol`, `name`, `from` and `to` (first and last session shown), `days[]`
oldest first, one per session the stock traded: `date`, `value` (traded by
all investors on all boards, IDR), `lots`, `average_price`,
`foreign_buy_value` (as published), `foreign_sell_value`
(`foreign_buy_value - foreign_net_value`, computed by idx),
`foreign_net_value` (as published) and `foreign_share_pct` (`(buy + sell) /
(2 × value) × 100`, computed by idx); `totals` (`sessions`, `value`,
`foreign_buy_value`, `foreign_sell_value`, `foreign_net_value`,
`foreign_share_pct` over all the sessions, `net_buy_sessions`,
`net_sell_sessions`, `streak` as in `idx.foreign.v1`) and `rounded`. Every
figure is in rupiah across all boards, negotiated trades included, so it is
not comparable field by field with `idx.foreign.v1`'s regular-board shares;
the two share no field names. Each session costs two single-session requests
(all investors and foreign investors; a range's own foreign net is not used,
see `idx.broker.v1`), cached, at most 60 sessions. A day without trades is
skipped; a traded day on which no foreigner traded has zeros. The source
publishes a session after the close: while today trades it is left out with
a warning, and so is any day whose page shows trades without broker lists
(not published yet; such a page is cached for minutes, not the week). A
range with no published session with trades is not found (exit 4), and says
so when a session was left out for that reason; when a day failed, that
failure is the error instead, since the day may have traded. `--days N` with
fewer sessions with trades in the days read gives those, with a warning. It
rounds figures of a million and more to one decimal (`1.2 T` is ±4 %), and
`foreign_sell_value` carries the error of both its terms.

## `idx.market.foreign.v1` (`market foreign`)

An object for one session: `date`, `stocks`, the market sums `foreign_buy`,
`foreign_sell`, `foreign_net` (shares) and `foreign_net_value_at_close`,
`sort` (`value` | `shares`), and `net_buy[]` / `net_sell[]` (largest first):
`symbol`, `name`, `close`, `change`, `change_pct`, `volume`, `value`,
`foreign_buy`, `foreign_sell`, `foreign_net`, `foreign_net_value_at_close`.

## `idx.market.foreign.value.v1` (`market foreign --source ipot`)

The same question from Indo Premier, in rupiah, in a shape of its own: never
mixed with `idx.market.foreign.v1`, whose figures are idx.co.id's shares.
An object for one session: `date`, `scanned` (the stocks read: the 100 most
traded by value on the latest quotes, the bandar screen's scan, not every
stock), `checked` (those with a broker summary that had trades), `market`,
`net_buy[]` / `net_sell[]` (largest first) and `rounded`. Each row:
`symbol`, `name`, `price`, `change_pct` (from the previous session's close),
`value` (traded on all boards, IDR), `foreign_net_value` (the broker
summary's `F. NVal`, all boards, as published) and `foreign_net_pct` (of
`value`, computed). `market` is IPOT's market statistics for the session:
`value` (its `TOTAL`: regular, negotiated and cash), `foreign_buy_value`,
`foreign_sell_value` and `foreign_net_value`, as published; IPOT does not say
which boards its foreign figures cover. `provider` is `ipot`.

The session is the latest one IPOT has published broker summaries for (the
bandar screen's `1d`); a `--date` before it is a usage error and a later one
`not_found`, and `--sort shares` (idx.co.id's net shares) a usage error. The
market statistics page carries no date and is dated as IPOT's broker ranking
is (see `idx.market.brokers.v1` below), with its own copy of BBCA's
statistics page: when it cannot be dated, or is another session's than the
stocks', `market` is `null` and a warning says why, so on a trading day from
08:58 to 19:00 WIB it is `null` unless a copy read outside those hours can
answer. Figures of a million and more are abbreviated, to one decimal on
the broker summaries and two on the market statistics (`rounded: true`).

## `idx.market.brokers.v1` (`market brokers`)

An object for one session: `date`, `members` (with trades), `total_volume`,
`total_value`, `total_frequency`, `sort` (`value` | `volume` | `frequency`),
`rounded` and `brokers[]`: `rank`, `code`, `name` (as the source publishes
it), `volume`, `value`, `frequency`, `value_share_pct`, then `buy_value`,
`sell_value`, `net_value` (bought minus sold) and `kind` (`local` |
`foreign` | `state_owned`), which only IPOT publishes: `null` from
idx.co.id. A member's buying and selling count together, so `total_value`
is twice the market's value.

`provider` says which source answered: `idx`, idx.co.id's daily broker
summary (any session), or `ipot`, IPOT's ranking of its latest session
(`--source ipot`, MCP `source: "ipot"`). `auto`, the default, asks
idx.co.id first; when idx.co.id refuses (Cloudflare), cannot be reached,
fails, offline has no copy, or has not published the session yet, IPOT
answers with a warning saying why, but only if it holds the session asked
for: the `--date`, or with none the latest session that has ended.
Otherwise idx.co.id's answer stands, with what IPOT holds or why it could
not be read. IPOT could answer for no `--date` or the latest session that
has ended, outside a trading day's 08:58 to 19:00 WIB. There `auto` does
not wait out a Cloudflare challenge: idx.co.id is asked once, without the
30 s of retries, and IPOT answers at once (a cached copy of idx.co.id's
summary still answers first, marked stale); only if IPOT cannot stand in
after all is idx.co.id asked again with the retries. `--source idx` (MCP
`source: "idx"`) always retries, and its `blocked` error names
`--source ipot` (MCP `source: "ipot"`) where IPOT could answer. A session
idx.co.id has not published yet (its summary read on its own day has no
rows) is named whatever the source: with no `--date` the session before
comes with a warning, and a `--date` of it is `not_found`. IPOT takes no
date: a `--date` before its session,
or after today, is a usage error; a later one up to today, or a day without
a session, is `not_found`; and under `auto` idx.co.id's error stands. It
abbreviates figures of a million and more to two decimals (`2.79 T`, so
`rounded: true`) and publishes `net_value` with its own two decimals, so it
can differ from `buy_value - sell_value`. Members without trades are left
out, as idx.co.id leaves them out; a broker IPOT lists apart under another
member's code (`JB`, `[CC] BJB Sekuritas Jawa Barat`) is counted in that
member, as idx.co.id counts it, with a warning.

IPOT's ranking page carries no date: the session comes from a second page,
the `Today` row of BBCA's statistics, read with the ranking (within a
minute). When the two pages roll over to a new session has not been
measured, so a ranking read on a trading day from 08:58 to 19:00 WIB cannot
be dated: in those hours the source answers a `provider` error (a copy read
then is `offline_miss` when served offline, and `network`, naming the
failure, when served because IPOT could not be read), under `auto`
idx.co.id's error stands, and IPOT is not asked at all unless a copy read
outside those hours is cached. Until the rollover is measured, both pages
are assumed to have rolled over by 19:00 WIB. A date page that names an
older session than the latest that had ended when the pages were read is
then refused, unless the pair is the very one accepted for that session
before. A ranking that has not rolled over by 19:00 while its date page has
is caught only when this cache remembers the previous session's ranking,
from an earlier IPOT read: the same figures under a new date are refused.

The web terminal's `brokers` (`/api/market/brokers`) answers from IPOT
only, with this shape and the `--source ipot` rules and messages: idx.co.id
refuses the site's Cloudflare Worker, so `source=idx` is a `usage` error
there, and `source=auto` reads IPOT with a warning saying so. The Worker
keeps the two pages as one pair and replaces it only when both were read
again, so the copy that stands in for a failed read (`stale: true`) is
always a pair read together.

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

## `idx.astro.v1` (`market astro`)

```json
{
  "date": "2026-11-06",
  "at": "2026-11-06T16:00:00+07:00",
  "bodies": [
    { "body": "sun", "longitude": 223.96, "sign": "scorpio", "degree": 13.96, "speed": 1.003, "retrograde": false },
    { "body": "mercury", "longitude": 219.92, "sign": "scorpio", "degree": 9.92, "speed": -1.22, "retrograde": true }
  ],
  "moon": { "phase": "waning_crescent", "elongation": 325.98, "illumination": 0.087, "age_days": 26.7,
            "previous_new_moon": "2026-10-10T22:50:00+07:00", "next_new_moon": "2026-11-09T14:02:00+07:00",
            "next_full_moon": "2026-11-24T21:54:00+07:00" },
  "mercury": { "retrograde": true, "retrograde_since": "2026-10-24T14:17:00+07:00",
               "retrograde_until": "2026-11-13T22:56:00+07:00",
               "next_retrograde": { "from": "2027-02-10T00:33:00+07:00", "until": "2027-03-03T19:41:00+07:00" } },
  "weton": { "day": "Jumat", "day_en": "Friday", "pasaran": "Pahing", "neptu_day": 6, "neptu_pasaran": 9, "neptu": 15 },
  "shio": { "animal": "horse", "animal_id": "Kuda", "element": "fire", "year_start": "2026-02-17" },
  "upcoming": { "from": "2026-11-06T00:00:00+07:00", "to": "2026-12-06T00:00:00+07:00",
                "events": [ { "at": "2026-11-09T14:02:00+07:00", "kind": "new_moon", "body": "moon", "sign": "scorpio" } ] },
  "natal": { "chart": "launch", "date": "1983-04-01", "at": "1983-04-01T12:00:00+07:00", "bodies": [ ... ],
             "transits": [ { "transit": "saturn", "aspect": "trine", "natal": "uranus", "orb": 0.03, "applying": false } ] },
  "backtest": null,
  "method": "Apparent geocentric positions in the tropical zodiac ...",
  "disclaimer": "Traditional astrology, not a forecast or investment advice: ..."
}
```

- Computed on the machine (no network) for 16:00 WIB on `date`, the IDX
  close; `provider` is `computed` and `cached` is `false`. With `--backtest`
  the envelope carries the `^JKSE` history's provenance instead (`provider`
  `yahoo`, `cached`, `stale`, `fetched_at`, warnings).
- `bodies` lists the Sun, Moon, Mercury, Venus, Mars, Jupiter, Saturn,
  Uranus, Neptune and Pluto in that order: apparent geocentric ecliptic
  longitude of date (tropical zodiac) to 2 decimals, the `sign` and `degree`
  of that rounded longitude, `speed` in degrees per day (3 decimals, negative
  while retrograde) and `retrograde` (always `false` for the Sun and Moon).
  It is the only top-level list, so `-o csv market astro` prints it.
- `moon.phase` is one of eight 45° sectors of the Moon-Sun elongation
  centred on the exact phases (`new_moon`, `waxing_crescent`,
  `first_quarter`, `waxing_gibbous`, `full_moon`, `waning_gibbous`,
  `last_quarter`, `waning_crescent`), so `full_moon` spans about 3.7 days;
  the exact instants are in `upcoming.events`. `illumination` is the lit
  fraction 0-1 (3 decimals), `age_days` the days since the previous new moon.
- `mercury`: `retrograde_since` and `retrograde_until` are the stations
  around the current retrograde (`null` when direct); `next_retrograde` the
  next period starting after `at`.
- `weton`: the civil date's day and pasaran (Legi, Pahing, Pon, Wage,
  Kliwon) with their neptu. Javanese days start at sunset; idx uses the
  civil date.
- `shio`: the Chinese year holding the date (`year_start` is its Imlek, a
  China Standard Time date); `null` before Imlek 1924 and after 2050.
- `upcoming.events` (`kind`: `new_moon`, `first_quarter`, `full_moon`,
  `last_quarter`, `station_retrograde`, `station_direct`, `ingress`) from
  00:00 WIB on `date` for `--days` days (1-366; the web terminal takes up to
  90), oldest first, rounded to the minute. `sign` is the sign the body is
  in, or the one it enters. The Moon's own sign changes are not listed.
- `natal` (`null` with `--natal none`): IHSG's chart at 12:00 WIB on
  `1983-04-01` (`launch`, the default) or `1982-08-10` (`base`), dates from
  secondary sources. `bodies` adds `approximate` (`true` for the Moon only:
  the time of day is unknown). `transits` lists the aspects
  (`conjunction`, `sextile`, `square`, `trine`, `opposition`) of the Sun and
  Mercury to Pluto at `at` to the natal Sun and Mercury to Pluto within 2°,
  closest first; `applying` means the orb shrinks over the next day.
- `backtest` (`null` unless asked): `symbol` (`^JKSE`), `period` (`1y`,
  `2y`, `5y`, `10y`, `max`; the web terminal stops at `10y`), `from` and
  `to` (session dates of the first and last return), `returns`, `mean_pct`,
  `median_pct` (4 decimals), `win_rate_pct` (2 decimals), `tests`, `alpha`
  (0.05), `bonferroni_alpha` (`alpha / tests`, 6 decimals), `significant`
  (groups below it), `factors[]` and `method`. Each factor (`moon_phase`,
  `mercury`, `pasaran`, `sun_sign`) lists its `groups[]`: `group`, `n`,
  `mean_pct`, `median_pct`, `stdev_pct` (4 decimals; `null` when `n` is 0,
  `stdev_pct` when `n` < 2), `win_rate_pct`, `t_stat` (Welch's t against all
  other sessions, 3 decimals) and `p_value` (two-sided, 6 decimals), `null`
  with fewer than two sessions on either side, and `significant`. Returns
  are close to close, one close per WIB date; a session that was still
  trading when the history was fetched (on its date, before 17:00 WIB) is
  left out with a warning, however much later the cached copy is read. A
  history coarser than daily is a provider error; fewer than 31 closes is
  `not_found`.
- `method` and `disclaimer` are fixed text. There is no interpretive text
  in v1; meanings, if added, will be a new optional field.

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
- `beta` is the source's own figure. Yahoo's does not track IHSG for IDX
  shares (BBCA 0.066 on 2026-11-06, against 0.95 from a year of daily closes
  against `^JKSE`); `stocks risk` computes beta against IHSG itself.
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

## Lenses: `idx.growth.v1`, `idx.valuation.v1`, `idx.risk.v1` (`stocks growth|valuation|risk`)

Three focused readings next to the blended verdict, also returned by
`stocks fundamental SYMBOL --lens growth|valuation|risk` and by MCP
`idx_fundamental` with `lens`. Each `data` carries the inputs it read (the
source's values as reported, `null` when absent), the figures idx computed
from them, a `reading` and `sources`:

```json
"reading": {
  "label": "moderate", "score": 2, "inputs_available": 6,
  "rules": [
    { "rule": "liquidity", "points": 1, "reason": "current ratio 0.79 is under 1" },
    { "rule": "volatility", "points": 1, "reason": "volatility 49.2% a year is 1.68x IHSG's 29.3% (over 1.5x)" }
  ],
  "notes": []
}
```

(TLKM's risk reading on the recorded fixtures, two of its six rules shown.
A rule that could not run lands in `notes`, for example
`"leverage and liquidity: not scored for a financial company"` for a bank.)

- `rules[]` lists every rule that ran, with its `points` (0 still ran and
  counts) and a `reason` carrying the figure and the threshold; `score` is
  their sum and `inputs_available` their number. `notes[]` names every rule
  that could not run and why (not reported, a financial company, statements
  in another currency, negative equity, fewer than 3 analysts, fewer than
  three fiscal years, benchmark unavailable, a price that barely moved).
  Fewer than three rules give `insufficient_data`.
- Rule ids are stable; the thresholds are heuristics, not advice. The
  valuation lens can disagree with the verdict of `stocks fundamental`,
  which also weighs quality and growth.
- A lens that needs several inputs fetches them concurrently. A missing one
  is named in `warnings` and the reading runs with fewer rules; the call
  fails only when the essential inputs all fail (growth: fundamentals and
  statements; risk: fundamentals and the stock's candles). It fails with
  `not_found` when the fundamentals or the stock's candles say the ticker is
  unknown; a statements "not found" (a listed ticker can have none) does not
  count, so the fundamentals' own error (a provider failure, an offline cache
  miss) is the one reported.

### `idx.valuation.v1`: points favour cheapness

`symbol`, `name`, `currency`, `financial_currency`, `financial_company`,
`negative_equity`, `price`, `pe_trailing`, `pe_forward`, `peg`,
`price_to_book`, `price_to_sales`, `ev_to_ebitda`, `ev_to_revenue` (shown,
not scored), `dividend_yield_pct`, `payout_ratio_pct`, `target_mean`,
`analysts`, `target_upside_pct` (computed: target over price, minus one),
`reading`, `sources`. Label: `cheap` (score 3 or more), `expensive` (-3 or
less), `fair`, `insufficient_data`.

| rule | points |
| --- | --- |
| `pe_trailing` | under 10: +2; 10 to 15: +1; 15 to 25: 0; over 25: -1; over 35: -2; not positive: 0 |
| `pe_forward` | only without a trailing P/E, above 0: under 10: +1; over 25: -1; else 0 |
| `peg` | above 0: under 1: +1; over 2: -1; else 0 |
| `price_to_book` | under 1: +1; over 4: -1; else 0. Not scored with negative equity or mixed currencies |
| `price_to_sales` | under 1: +1; over 5: -1; else 0. Not for a financial company or mixed currencies |
| `ev_to_ebitda` | above 0: under 6: +1; over 15: -1; else 0. Not for a financial company or mixed currencies |
| `dividend_yield` | 4 % or more: +1, but 0 when the payout exceeds 100 %; under 4 %: 0 |
| `analyst_target` | 3 or more analysts: upside over +20 %: +1; under -10 %: -1; else 0 |

### `idx.growth.v1`: points favour growth

`symbol`, `name`, `statements_currency`, `most_recent_quarter`,
`revenue_growth_pct` and `earnings_growth_pct` (latest quarter against the
same quarter a year earlier, as reported), `revenue_cagr_5y_pct` and
`eps_growth_3y_pct` (MSN's, when it contributed), `eps_ttm`, `eps_forward`,
`eps_forward_growth_pct` (computed, both EPS above zero), `annual[]` (one per
fiscal year, oldest first: `end_date`, `revenue`, `net_income`,
`revenue_change_pct`, `net_income_change_pct`, the changes computed against
the year before), `revenue_cagr_pct` and `net_income_cagr_pct` (computed from
the oldest to the newest fiscal year with a value, at least three such years,
both ends above zero), `reading`, `sources`. Label: `growing` (score 2 or
more), `shrinking` (-2 or less), `mixed` (points both ways), `stable`,
`insufficient_data`.

| rule | points |
| --- | --- |
| `revenue_growth`, `earnings_growth` | latest quarter over +10 %: +1; below 0 %: -1; else 0 (a figure beyond ±100 % is marked "from a small or negative base") |
| `eps_forward_growth` | forward EPS against trailing over +10 %: +1; below -10 %: -1; else 0 |
| `revenue_cagr` | idx's CAGR over three or more fiscal years: over +10 %: +1; below 0 %: -1; else 0 |
| `revenue_cagr_5y` | only with fewer than three fiscal years: MSN's 5-year revenue rate, as reported, same bands |
| `revenue_consistency` | revenue grew every year: +1; in fewer than half the years: -1; else 0 (three or more fiscal years) |
| `net_income_cagr` | idx's CAGR over three or more fiscal years: over +10 %: +1; below 0 %: -1; else 0. A profit turning into a loss: -1; a loss turning into a profit: +1 |
| `eps_growth_3y` | only with fewer than three fiscal years: MSN's 3-year EPS growth, as reported, same bands |

### `idx.risk.v1`: points add risk

`symbol`, `name`, `currency`, `financial_company`, `negative_equity`,
`debt_to_equity`, `current_ratio`, `quick_ratio`, `net_margin_pct`, `eps_ttm`,
`price` (`null` without daily candles for the stock), `reading`, `sources`.
`price` is computed by idx from a year of daily closes on the dates the stock
and IHSG both traded (Yahoo's `^JKSE` bar often lags the shares by a session):
`benchmark` (`^JKSE`), `from`, `to`, `observations` (matched daily returns),
`zero_returns` (how many of them left the stock's close unchanged; `null`
without IHSG), `beta` and `correlation` (from simple daily returns, 3 dp,
`null` under 120 observations), `volatility_pct` and
`benchmark_volatility_pct` (annualised standard deviation of daily log
returns), `max_drawdown` (`pct`, `peak_date`, `trough_date`, `recovered`;
`null` when the price never fell) and `benchmark_max_drawdown_pct`. Without
IHSG the price rules become notes and `price` carries the stock's own
volatility and drawdown over its own dates.
Label: `high` (score 3 or more), `low` (-2 or less), `moderate`,
`insufficient_data`.

| rule | points |
| --- | --- |
| `negative_equity` | +2 (any sector); the leverage rule is then skipped |
| `leverage` | debt/equity over 2x: +1; under 0.5x: -1; else 0. Not for a financial company |
| `liquidity` | current ratio under 1: +1; 1.5 or more: -1; else 0. Not for a financial company |
| `losses` | negative net margin or EPS over the trailing twelve months: +1; else 0 |
| `beta` | against IHSG, 120 or more daily returns: over 1.3: +1; under 0.7: -1; else 0 |
| `volatility` | the stock's over IHSG's: over 1.5x: +1; under 0.8x: -1; else 0 |
| `drawdown` | the stock's largest fall minus IHSG's: 15 points or more deeper: +1; 15 points or more shallower: -1; else 0 |
| `illiquidity` | more than half of 20 or more matched daily returns are zero: +1, and `beta`, `volatility` and `drawdown` are not scored |

Price risk is relative to IHSG so that a market-wide fall (IHSG fell 46 %
peak to trough in the year to 2026-11-04) does not make every stock look
risky. A price that does not move would look calm on all three price rules
(beta 0, no volatility, no fall), yet a suspended share, one stuck at the
floor price or one that barely trades is hard to sell: when the close is
unchanged on more than half the matched sessions, the `illiquidity` rule
scores instead and the three become a note (their figures stay in `price`).
Below that line thin trading still biases a daily beta toward 0 and
understates volatility; `zero_returns`, `observations` and `correlation`
show how much they rest on.


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
- Every period carries all three statements. `--statement income,cash` narrows
  the table only: JSON and CSV keep the three, since a blanked statement would
  read as data the source does not have.

## `idx.earnings.v1` (`stocks earnings`)

`data`: `estimates[]`, `quarters[]` (`period` like `3Q2025`, `end_date`,
`reported_date`, `eps_actual`, `eps_estimate`, `surprise_pct`), `yearly[]` and
`quarterly[]` (`period`, `revenue`, `earnings`, `profit_margin_pct`), `next`
(`date`, `date_is_estimate`, `eps_avg`/`eps_low`/`eps_high`, `eps_growth_pct`,
`revenue_avg`/`revenue_low`/`revenue_high`, `analysts`), `ex_dividend_date`,
`dividend_date`, `source`, `scope`.

- `estimates[]` is the analyst consensus for this and next quarter and fiscal
  year, in source order: `period` (`0q`, `+1q`, `0y`, `+1y`), `period_type`
  (`quarterly` or `annual`), `end_date`, `eps_avg`/`eps_low`/`eps_high`,
  `eps_year_ago`, `eps_growth_pct` (percent, versus a year earlier),
  `eps_analysts`, `revenue_avg`/`revenue_low`/`revenue_high`,
  `revenue_year_ago`, `revenue_growth_pct`, `revenue_analysts`. An optional
  addition to `idx.earnings.v1`: envelopes from older versions lack it. A
  period with no figure and no analyst is left out, so a ticker no analyst
  covers has an empty list.
- Yahoo pads figures it has not loaded as zeros; idx reports those as `null`
  (a quarter's `revenue`, `earnings` and `profit_margin_pct`, a consensus
  amount in `estimates[]` or `next`), never as `0`. Analyst counts keep their
  zero. `profit_margin_pct` is set only when both revenue and earnings are.
- `scope` is `null` unless rows were filtered out (`stocks earnings --annual
  --quarterly --reported --estimates`, MCP `period` and `kind`). Then it is
  `{ "annual", "quarterly", "reported", "estimates" }`, each `true` when those
  rows were kept, and an empty list or a `null` `next` comes from the filter,
  not the source. Giving neither flag of a pair keeps both. `quarters[]` and
  `quarterly[]` are reported quarterly rows, `yearly[]` reported annual rows,
  `estimates[]` follow their `period_type`, and `next` counts as a quarterly
  estimate. The filter applies to JSON and CSV alike.

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

## `idx.stockbit.actions.v1` (`stockbit actions`, `stockbit calendar`)

From the user's own Stockbit session; `provider` is `stockbit`. `data`:
`symbol` (the stock asked for; `null` for the calendar), `name` (from the
bundled issuer list), `from` / `to` (the range idx applied; `null` when open),
`kinds` (each one request), `total` (rows Stockbit returned for these kinds
before the range), and `actions[]`, newest first for a stock and oldest first
for the calendar. Each action: `symbol` (a structured warrant's own code for
those), `name` (IPOs, tender offers), `kind` (`dividend`, `stock_dividend`,
`bonus`, `rights`, `warrant`, `split`, `reverse_split`, `rups`,
`public_expose`, `ipo`, `tender_offer`), `date` (the cum date for
entitlements, the ex date without one, else the event's first day),
`cum_date`, `ex_date`, `recording_date` (for a RUPS, the date that decides who
may attend), `distribution_date` (payment; a tender offer's settlement),
`dividend_per_share` and `currency` (ISO code), `ratio_old` / `ratio_new` (as
published), `factor` (Stockbit's price adjustment factor), `price` (IDR per
share: rights or warrant exercise, tender offer, final IPO price),
`trading_start` / `trading_end` (rights and warrants), `exercise_start` /
`exercise_end` (warrants), `offer_start` / `offer_end` (IPO offering, tender
offer), `listing_date`, `shares` and `shares_pct` (offered or sought),
`time` (RUPS or expose time as published, WIB), `venue`, `note`, `id`
(Stockbit's row id). Placeholders (empty strings, zeros, `0001-01-01`) are
`null`. A kind that fails is a `warnings` entry.

## `idx.stockbit.movements.v1` (`stockbit movements`)

From the user's own Stockbit session; `provider` is `stockbit`. `data`: the
filters asked for (`symbol`, `from`, `to`, `holder_id`, `action` as `buy`,
`sell`, `transfer`, `cross` or `corporate_action`, `source` as `idx` or
`ksei`; `null` when not given), `pages_read`, `more` (rows remain past the
pages read, or a later page failed, which `warnings` names), and
`movements[]`, newest first: `date`, `symbol`, `holder`, `holder_id` (the
source's id for the holder, the same on all of that holder's rows), `roles`
(`director`, `commissioner`, or another role lowercased), `foreign` (`true`,
`false`, `null`), `action` (the source's label, lowercased: `buy`, `sell`,
`transfer`, `cross`, `right_issue`, `private_placement`, `tender_offer`, ...),
`source` (`idx` insider disclosure, `ksei` holder at 5 % and above),
`shares_from`, `shares_to`, `shares_change` (exact, signed), `pct_from`,
`pct_to`, `pct_change` (percent of the company as the source rounds it),
`price` and `value` (IDR, as filed; `null` when the source only estimates),
`estimated_price` and `estimated_value` (the source's estimate when no price
was filed; `null` otherwise), `purpose` (as filed), `broker` and
`broker_group` (seen on KSEI rows).

## `stockbit status` (`idx schema stockbit-status`)

A bare object, not an envelope, and never a token value (the MCP tool
`idx_stockbit_status` returns the same): `logged_in` (`true` when the stored
session can work now: its refresh token has not expired, or its access token
has not; each token's own `exp` decides, one without an `exp` counts as
unexpired and one that is not a JWT as unusable), `renewable` (`true` when idx
can renew the access token because the refresh token has not expired; while
`logged_in` is `true` and `renewable` is `false`, the session stops working
when the access token expires), `expired` (`true` when a session is stored but
its tokens have expired), `reason` (why the session cannot work or cannot be
renewed, and that `idx stockbit login` renews it; `null` while it works and
renews), `access` and `refresh` (each `present`, `expires_at` in Unix seconds
and `expires_in_secs`, negative once expired; both `null` when the token is
absent, not a JWT or without an `exp`), and `session_file` (the absolute
path). Nothing here asks Stockbit: a token it revoked, or another tool spent,
still counts until its `exp`, or until this process sees Stockbit refuse it (an
`idx mcp` server remembers a refresh token Stockbit refused: `renewable` turns
`false`, `logged_in` follows the access token alone and `reason` says so).
`sign_in` is MCP only: the reply of `idx_stockbit_login`, and
in `idx_stockbit_status` the latest page sign-in that tool started in the same
server; it is absent otherwise (the CLI never sets it). Its fields: `state` (`waiting`, `signed_in`, `failed` or
`already_signed_in`), `method` (`page`, `browser` or `none`), `url` and `port`
(the page on 127.0.0.1 while `waiting`), `ssh_forward` (an `ssh -N -L` command
when the server is reached over SSH), `expires_in_secs` (until the page stops
listening), `already_open` (an earlier page was still open and is returned),
`warning` (the latest note, such as why Stockbit refused a pasted session) and
`error` (why it `failed`).

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
| 3 | `network`, `provider`, `blocked` | network failure or the source returned an error; `blocked`: the source's bot protection (Cloudflare) refused the request with a challenge or a block page. For idx.co.id's data a challenged request is retried for 30 s first (`market brokers` under `auto`, where IPOT could answer, asks idx.co.id once and lets IPOT stand in, and retries only if IPOT cannot), and a cached copy, when there is one, is served marked stale instead of the error. A challenge usually passes within a minute: retry then, and from another network if it keeps refusing. A block page can last longer: retry later, or from another network if it persists. Data centres, VPNs and CI runners are refused more often. The message names any other source for the same question (`--source ipot` for a stock's foreign flow and for the latest session's foreign net of the most traded stocks, `market brokers --source ipot` for the member ranking of IPOT's latest session when that is the session asked for); for the ownership register, the GitHub mirror |
| 4 | `not_found` | symbol or record does not exist |
| 5 | `offline_miss` | `--offline` and nothing cached |
| 6 | `config`, `unsupported` | bad config file or env value; or the selected provider cannot serve this command (`--provider auto` can) |

## Compatibility promise

- Adding an optional field to a payload does not bump the `schema` version.
- Renaming, removing or changing the type/unit of a field does, and the old
  version is kept for at least one minor release behind a flag.
- Numbers are plain JSON numbers (no strings), timestamps are RFC 3339.
