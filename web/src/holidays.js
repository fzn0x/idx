// IDX trading holidays: the web twin of src/holidays.rs. holidays.json is
// data/holidays.csv as [date, name, name_en] rows (a test keeps them equal);
// weekend holidays are not listed, as IDX publishes them.

import HOLIDAYS from "./holidays.json" with { type: "json" };

const BY_DATE = new Map(HOLIDAYS.map(([date, name, nameEn]) => [date, { name, name_en: nameEn || null }]));
const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

const dow = (iso) => new Date(`${iso}T00:00:00Z`).getUTCDay();
const addDays = (iso, n) => new Date(Date.parse(`${iso}T00:00:00Z`) + n * 86400e3).toISOString().slice(0, 10);

export const weekday = (iso) => WEEKDAYS[dow(iso)];

/** Years the list covers, oldest first. */
export const years = () => [...new Set(HOLIDAYS.map(([d]) => Number(d.slice(0, 4))))].sort((a, b) => a - b);
export const covers = (year) => years().includes(year);

/** A weekday that is not a listed holiday (for an uncovered year only weekends close). */
export const isTradingDay = (iso) => dow(iso) !== 0 && dow(iso) !== 6 && !BY_DATE.has(iso);

/** The listed holiday on `iso`, { name, name_en }, or null (holidays::holiday). */
export const holiday = (iso) => BY_DATE.get(iso) ?? null;

/** Why `iso` has no session, like closed_reason in src/service/exchange.rs. */
export function closedReason(iso) {
  const h = holiday(iso);
  if (h) return `${iso} is a trading holiday (${h.name})`;
  return dow(iso) === 0 || dow(iso) === 6 ? `${iso} is a ${weekday(iso)}` : `nothing was published for ${iso}`;
}

export function nextTradingDay(iso) {
  let d = addDays(iso, 1);
  while (!isTradingDay(d)) d = addDays(d, 1);
  return d;
}

/** The last trading day before `iso`. */
export function previousTradingDay(iso) {
  let d = addDays(iso, -1);
  while (!isTradingDay(d)) d = addDays(d, -1);
  return d;
}

// When an IDX trading day's session is over: 16:15:00 WIB, the end of
// post-closing (IDX trading hours, Decree Kep-00158/BEI/10-2026; the same
// constant as SESSION_END_SECS in src/holidays.rs and the end of the
// timetable the terminal's clock shows).
export const SESSION_END_SECS = 16 * 3600 + 15 * 60;
const WIB_MS = 7 * 3600e3;

/** WIB calendar date (YYYY-MM-DD) of an instant (a Date or anything Date.parse reads). */
export function wibDate(t) {
  const ms = t instanceof Date ? t.getTime() : Date.parse(t);
  return new Date(ms + WIB_MS).toISOString().slice(0, 10);
}

/**
 * The latest trading day whose session has ended by `now`: today from 16:15
 * WIB on a trading day, otherwise the trading day before (weekends and listed
 * holidays skipped). The twin of holidays::latest_session.
 */
export function latestSession(now = new Date()) {
  const w = new Date(now.getTime() + WIB_MS);
  const today = w.toISOString().slice(0, 10);
  const secs = w.getUTCHours() * 3600 + w.getUTCMinutes() * 60 + w.getUTCSeconds();
  return isTradingDay(today) && secs >= SESSION_END_SECS ? today : previousTradingDay(today);
}

/** Plain codes (.JK listings) and IDX's own indices (^JK...) follow IDX sessions; ^GSPC or USDIDR=X do not. */
export const tradesOnIdx = (symbol) => {
  const s = String(symbol || "");
  return s.startsWith("^") ? s.startsWith("^JK") : !s.includes("=");
};

/** `true` when a last trade at `marketTime` is dated (WIB) before `latest`; null without a time. */
export const behindDate = (marketTime, latest) => (marketTime ? wibDate(marketTime) < latest : null);

/** A quote's behind_session: null off IDX or without market_time, as Quote::behind in Rust. */
export const behindSession = (q, latest) => (q && tradesOnIdx(q.symbol) ? behindDate(q.market_time, latest) : null);

/** Weekdays minus listed holidays in `year`. */
export function tradingDays(year) {
  let n = 0;
  for (let d = `${year}-01-01`; d.startsWith(String(year)); d = addDays(d, 1)) if (isTradingDay(d)) n++;
  return n;
}

/** The idx.holidays.v1 payload for `year` (every year when null) plus warnings, like service::holidays. */
export function holidays(year, upcoming, today) {
  const covered = years();
  const warnings = [];
  if (year != null && !covers(year)) warnings.push(`no holiday list for ${year}; covered: ${covered.join(", ")}`);
  const list = HOLIDAYS
    .filter(([d]) => year == null || Number(d.slice(0, 4)) === year)
    .filter(([d]) => !upcoming || d >= today)
    .map(([date, name, nameEn]) => ({ date, weekday: weekday(date), name, name_en: nameEn || null }));
  return {
    data: {
      year: year ?? null,
      years_covered: covered,
      trading_days: year != null && covers(year) ? tradingDays(year) : null,
      today,
      today_is_trading_day: isTradingDay(today),
      next_trading_day: nextTradingDay(today),
      holidays: list,
    },
    warnings,
  };
}
