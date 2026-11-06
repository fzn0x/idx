// The IDX session as a station timetable: which phase the regular market is
// in at a moment, when that phase ends, and when trading resumes.
//
// Hours are IDX's own, for the regular market (Pasar Reguler), set by the
// Decree of the Board of Directors of PT Bursa Efek Indonesia
// Kep-00158/BEI/10-2026 (Rule II-A) and published as "Trading Hours and
// Mechanism", https://www.idx.co.id/en/products-services/trading-hours-and-mechanism/
// (read 2026-11-06). They run on JATS time, which is WIB (UTC+7):
//   pre-opening   08:45:00-08:59:59 (input to 08:57:59, matching from 08:58:00)
//   session I     09:00:00-12:00:00 Mon-Thu, 09:00:00-11:30:00 Fri
//   session II    13:30:00-15:49:59 Mon-Thu, 14:00:00-15:49:59 Fri
//   pre-closing   15:50:00-16:01:59 (input to 15:59:59, matching from 16:00:00)
//   post-closing  16:02:00-16:15:00
// Holidays are the bundled IDX calendar (src/holidays.json) that
// /api/holidays serves; weekends always close. Pure functions, tested in
// test/session.test.mjs.

const at = (h, m = 0) => (h * 60 + m) * 60; // seconds after midnight, WIB
const WIB_MS = 7 * 3600e3;

const phases = (s1End, s2Start) => [
  { id: "pre-open", name: "Pre-opening", from: at(8, 45), to: at(9) },
  { id: "session-1", name: "Session I", from: at(9), to: s1End },
  { id: "break", name: "Break", from: s1End, to: s2Start },
  { id: "session-2", name: "Session II", from: s2Start, to: at(15, 50) },
  { id: "pre-close", name: "Pre-closing", from: at(15, 50), to: at(16, 2) },
  { id: "post", name: "Post-closing", from: at(16, 2), to: at(16, 15) },
];

/** The regular market's day: Monday to Thursday, and Friday's longer break. */
export const TIMETABLE = { weekday: phases(at(12), at(13, 30)), friday: phases(at(11, 30), at(14)) };

const isoOf = (ms) => new Date(ms).toISOString().slice(0, 10);
const dowOf = (iso) => new Date(`${iso}T00:00:00Z`).getUTCDay();
const addDays = (iso, n) => isoOf(Date.parse(`${iso}T00:00:00Z`) + n * 86400e3);
/** The instant `secs` after midnight WIB on `iso`. */
const instant = (iso, secs) => new Date(Date.parse(`${iso}T00:00:00Z`) - WIB_MS + secs * 1000);

/** WIB calendar date, weekday (0 = Sunday) and seconds after midnight of a moment. */
export function wib(now) {
  const ms = now.getTime() + WIB_MS;
  const iso = isoOf(ms);
  return { iso, dow: dowOf(iso), secs: Math.floor((ms - Date.parse(`${iso}T00:00:00Z`)) / 1000) };
}

/** A weekday that is not a listed holiday. `holidays` maps ISO dates to names. */
export const isTradingDay = (iso, holidays = new Map()) => {
  const d = dowOf(iso);
  return d !== 0 && d !== 6 && !holidays.has(iso);
};

export function nextTradingDay(iso, holidays = new Map()) {
  let d = addDays(iso, 1);
  for (let i = 0; i < 60 && !isTradingDay(d, holidays); i++) d = addDays(d, 1);
  return d;
}

const dayPhases = (iso) => (dowOf(iso) === 5 ? TIMETABLE.friday : TIMETABLE.weekday);

/**
 * Where the session stands at `now`: the phase (null when closed), the next
 * event and the seconds until it. `trading` is true in the two continuous
 * sessions; `open` in any phase from pre-opening to post-closing.
 */
export function session(now, holidays = new Map()) {
  const { iso, dow, secs } = wib(now);
  const tradingDay = isTradingDay(iso, holidays);
  const list = dayPhases(iso);
  const phase = tradingDay ? list.find((p) => secs >= p.from && secs < p.to) || null : null;
  let next;
  if (phase) {
    const i = list.indexOf(phase);
    const after = list[i + 1];
    next = after ? { id: after.id, name: after.name, at: instant(iso, after.from) } : { id: "closed", name: "Close", at: instant(iso, phase.to) };
  } else {
    const day = tradingDay && secs < list[0].from ? iso : nextTradingDay(iso, holidays);
    next = { id: "pre-open", name: "Pre-opening", at: instant(day, dayPhases(day)[0].from), day };
  }
  return {
    date: iso,
    weekday: dow,
    holiday: holidays.get(iso) || null,
    tradingDay,
    phase,
    open: phase != null,
    trading: phase?.id === "session-1" || phase?.id === "session-2",
    next,
    until: Math.max(0, Math.round((next.at.getTime() - now.getTime()) / 1000)),
  };
}

const p2 = (n) => String(n).padStart(2, "0");

/** HH:MM:SS under a day; 2d 14:05 beyond. */
export function countdown(secs) {
  const s = Math.max(0, Math.floor(secs));
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
  return d ? `${d}d ${p2(h)}:${p2(m)}` : `${p2(h)}:${p2(m)}:${p2(s % 60)}`;
}

/** HH:MM:SS of a moment in WIB. */
export const clockText = (now) => { const s = wib(now).secs; return `${p2(Math.floor(s / 3600))}:${p2(Math.floor((s % 3600) / 60))}:${p2(s % 60)}`; };

/** HH:MM of seconds after midnight. */
export const hhmm = (secs) => `${p2(Math.floor(secs / 3600))}:${p2(Math.floor((secs % 3600) / 60))}`;

const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/** "Fri 6 Nov 2026" for an ISO date. */
export const dayText = (iso) => `${DAYS[dowOf(iso)]} ${Number(iso.slice(8, 10))} ${MONTHS[Number(iso.slice(5, 7)) - 1]} ${iso.slice(0, 4)}`;

/** "Wed 15:50" for a moment, in WIB. */
export const stamp = (date) => { const w = wib(date); return `${DAYS[w.dow]} ${hhmm(w.secs)}`; };
