// What each terminal command waits on, for its loading line. `slow` is added
// once the wait passes SLOW_MS: what is known about that source's stalls and
// how the API bounds them. test/waits.test.mjs keeps the numbers in step with
// src/api.js (feed deadline) and src/ipot.js (404 retries).

export const SHOW_MS = 250; // answered sooner (cache, bundled data): no loading line
export const SLOW_MS = 5000;
export const FEED_TIMEOUT_S = 8;
export const IPOT_TRIES = 5;

const FEEDS_SLOW = `Kontan's feeds sometimes stall; a feed silent for ${FEED_TIMEOUT_S} s is dropped and named in a warning`;
const IPOT_SLOW = `IPOT sometimes answers 404 before the data; each one is retried, up to ${IPOT_TRIES} tries`;
const YAHOO = { on: "Yahoo Finance" };

export const WAITS = {
  quote: YAHOO, history: YAHOO, profile: YAHOO, financials: YAHOO, earnings: YAHOO, fundamental: YAHOO, technical: YAHOO, dividends: YAHOO,
  valuation: YAHOO,
  growth: { on: "Yahoo Finance: fundamentals and the annual statements" },
  risk: { on: "Yahoo Finance: fundamentals and a year of daily candles for the stock and IHSG" },
  compare: { on: "Yahoo Finance, one request per symbol" },
  returns: { on: "Yahoo Finance, one history per symbol and the benchmark" },
  screen: { on: "the board, Yahoo fundamentals and MSN's market list", slow: "fundamental filters fetch each matching ticker, up to --max-fundamentals (20); return keys read MSN's whole-market list once (cached 15 minutes)" },
  news: { on: "the Kontan and CNBC Indonesia feeds", slow: FEEDS_SLOW },
  sentiment: { on: "the Kontan and CNBC Indonesia feeds", slow: FEEDS_SLOW },
  insights: { on: "MSN Money insights and its peer list" },
  brief: { on: "quote, fundamentals, history, dividends, holders and news in parallel", slow: `${FEEDS_SLOW}; the rest of the brief still prints` },
  broker: { on: "IPOT broker summaries", slow: IPOT_SLOW },
  bandar: { on: "IPOT broker summaries, one per session", slow: IPOT_SLOW },
  "bandar screen": { on: "the nightly bandar scan" },
  "bandar broker": { on: "the nightly bandar scan" },
  actions: { on: "IPOT corporate actions", slow: IPOT_SLOW },
  market: { on: "the minute board" },
  ownership: { on: "the KSEI register" },
};

/** The entry for a command (aliases resolved) and its first argument; {} for local commands. */
export function waitFor(name, arg) {
  const sub = String(arg || "").toLowerCase();
  if (name === "bandar" && (sub === "screen" || sub === "broker")) return WAITS[`bandar ${sub}`];
  return WAITS[name] || {};
}
