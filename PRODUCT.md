# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Decided by the owner in the project brief: the hosted surface is a Cloudflare
Worker at `ihsg.midoriresearch.com` with static assets and a D1 database that a
cron trigger refreshes every minute; access is custom-domain only
(`workers_dev = false`). The CLI/MCP binary is Rust. No front-end framework was
requested; the web terminal and landing page are plain HTML/CSS/JS served by
the Worker (chosen by the maintainer in this session, not asked separately,
because the brief fixed the Worker + assets shape).

## Users

- **Retail investors and analysts following the Indonesia Stock Exchange** who
  want quotes, fundamentals, technicals, shareholder registers and news for IDX
  tickers without paying for a terminal. They work in a shell or a browser and
  already know tickers like BBCA, BBRI, TLKM.
- **AI agents and scripts** (Claude Code, Cursor, cron jobs) that call the same
  capabilities through `idx -o json` or the MCP server and need stable,
  schema-documented JSON and meaningful exit codes.
- Web visitors arrive from the GitHub README or a shared link and want to try
  the tool before installing anything (owner's brief: "landing page has a CTA to
  the web terminal"). Confirmed in this session: the web surface is
  terminal-first, in English.

## Product Purpose

`idx` is one open-source binary (CLI + MCP server) and a hosted web terminal
for Indonesian stock market analysis built only on free public data: Yahoo
Finance, MSN Money, IDX's monthly KSEI shareholder register, and Kontan / CNBC
Indonesia feeds. Success means a person or an agent gets a trustworthy answer
("who holds more than 1 % of BBCA", "which banks trade under 10x earnings") in
one command, with provenance, and the hosted terminal shows the same answers
without an install.

## Positioning

The only IDX tool that serves humans and agents from one code path: tables,
JSON and MCP tools return the same envelope, every field traces to a source,
nothing is estimated, and the KSEI ">1 % holders" register is queryable
locally with normalised entity names. Free, MIT, no API keys.

## Operating Context

- CLI in a terminal (Linux, macOS, Windows); MCP inside coding agents.
- Hosted terminal in a desktop or phone browser; data comes from D1 snapshots
  refreshed every minute by the Worker's cron plus on-demand fetches that are
  cached in D1.
- Prices are delayed as published by the sources; the register is monthly.
- Indonesian market conventions: tickers are four letters, prices in whole
  rupiah, trading hours in WIB (UTC+7), IHSG is the composite index (`^JKSE`).

## Capabilities and Constraints

- Commands: `stocks quote|history|profile|financials|earnings|fundamental|
  technical|compare|screen|news|sentiment`, `ownership import|ticker|entity|
  changes|releases|discover`, `mcp`, `schema`, `config`, `cache`.
- Hard rules carried into the web surface: never invent fields (null when the
  source lacks it), label verdicts and signals as heuristics, show provenance
  (`provider`, `fetched_at`, `cached`, `stale`).
- idx.co.id is behind Cloudflare; the web terminal cannot fetch the ownership
  register live. Ownership data on the web is only what the maintainer imports
  (undecided: whether to publish a snapshot; until then the web terminal states
  the limitation).
- The web terminal must not expose Worker-side secrets; everything it shows is
  public data.
- Accessible without JavaScript for the landing copy; the terminal needs JS.

## Brand Commitments

- Name: `idx` (lower-case wordmark). Assets: `assets/logo.svg` (with
  `logo-light.svg` / `logo-dark.svg`), `assets/mark.svg` = `web/public/mark.svg`
  (Material 3 Expressive mark, owner's brief of 2026-11-06: a nine-lobed
  cookie in idx mint `#4ade80` holding three ink `#0b0f14` candles).
- Publisher: Midori Research (midoriresearch.com), contact
  desk@midoriresearch.com. MIT licence.
- Voice (from the CLI and README): plain, exact, no hype; disclaimers are
  one sentence and never apologetic.
- Binding visual constraint volunteered by the owner: none beyond the logo.

## Evidence on Hand

- Real CLI output and JSON samples in `README.md` and `docs/`.
- Live data at runtime; recorded fixtures under `tests/fixtures/`.
- No testimonials, customer logos, press or benchmarks exist; the web surface
  must not fabricate any. Screenshots of the CLI and terminal are to be
  produced from the real tool for the README and landing page.

## Product Principles

1. One answer, three faces: table, JSON and MCP tool say the same thing.
2. Provenance over polish: every number names its source and age.
3. Free forever: no keys, no accounts, no paywall; self-hostable.
4. Heuristics are labelled: verdicts, signals and sentiment explain their
   rules and never read as advice.
5. Works offline and degrades gracefully when a source breaks.

## Accessibility & Inclusion

Keyboard-first terminal; colour never the only carrier of meaning (signs and
labels accompany red/green); WCAG AA contrast on the dark theme; respects
`prefers-reduced-motion`.
