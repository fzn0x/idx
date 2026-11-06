// node --test web/test/*.mjs (no network). The monitor's session clock:
// IDX's regular-market phases (public/clock.js), Friday's longer break, and
// the next session over weekends and the bundled holidays.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { session, countdown, clockText, dayText, isTradingDay, nextTradingDay, TIMETABLE, wib } from "../public/clock.js";

const web = join(dirname(fileURLToPath(import.meta.url)), "..");
// The same rows /api/holidays serves, as the page holds them: date -> name.
const HOLIDAYS = new Map(JSON.parse(readFileSync(join(web, "src", "holidays.json"), "utf8")).map(([d, , en]) => [d, en]));
const at = (isoWib) => new Date(`${isoWib}+07:00`);

test("the timetable is IDX's: Session I to 12:00, Friday to 11:30, close at 16:15", () => {
  const ids = (l) => l.map((p) => p.id).join(" ");
  assert.equal(ids(TIMETABLE.weekday), "pre-open session-1 break session-2 pre-close post");
  const hm = (s) => `${String(Math.floor(s / 3600)).padStart(2, "0")}:${String((s % 3600) / 60).padStart(2, "0")}`;
  assert.deepEqual(TIMETABLE.weekday.map((p) => `${hm(p.from)}-${hm(p.to)}`), ["08:45-09:00", "09:00-12:00", "12:00-13:30", "13:30-15:50", "15:50-16:02", "16:02-16:15"]);
  assert.deepEqual(TIMETABLE.friday.map((p) => `${hm(p.from)}-${hm(p.to)}`), ["08:45-09:00", "09:00-11:30", "11:30-14:00", "14:00-15:50", "15:50-16:02", "16:02-16:15"]);
});

test("each phase of a Thursday, with what comes next", () => {
  const cases = [
    ["2026-11-05T08:50:00", "pre-open", "session-1", "2026-11-05T09:00:00", 600],
    ["2026-11-05T10:00:00", "session-1", "break", "2026-11-05T12:00:00", 7200],
    ["2026-11-05T12:30:00", "break", "session-2", "2026-11-05T13:30:00", 3600],
    ["2026-11-05T15:55:30", "pre-close", "post", "2026-11-05T16:02:00", 390],
    ["2026-11-05T16:10:00", "post", "closed", "2026-11-05T16:15:00", 300],
  ];
  for (const [t, phase, next, nextAt, until] of cases) {
    const s = session(at(t), HOLIDAYS);
    assert.equal(s.phase?.id, phase, t);
    assert.equal(s.next.id, next, t);
    assert.equal(s.next.at.getTime(), at(nextAt).getTime(), t);
    assert.equal(s.until, until, t);
  }
  assert.equal(session(at("2026-11-05T10:00:00")).trading, true);
  assert.equal(session(at("2026-11-05T12:30:00")).trading, false);
  assert.equal(session(at("2026-11-05T12:30:00")).open, true);
});

test("Friday breaks from 11:30 to 14:00", () => {
  const s = session(at("2026-11-06T11:45:00"), HOLIDAYS);
  assert.equal(s.phase.id, "break");
  assert.equal(s.next.at.getTime(), at("2026-11-06T14:00:00").getTime());
  assert.equal(session(at("2026-11-06T13:45:00")).phase.id, "break");
  assert.equal(session(at("2026-11-09T13:45:00")).phase.id, "session-2");
});

test("closed: before the pre-opening, after the close, at weekends and on holidays", () => {
  const early = session(at("2026-11-05T07:00:00"), HOLIDAYS);
  assert.equal(early.phase, null);
  assert.equal(early.next.at.getTime(), at("2026-11-05T08:45:00").getTime());
  assert.equal(early.until, 6300);

  const evening = session(at("2026-11-05T16:30:00"), HOLIDAYS);
  assert.equal(evening.open, false);
  assert.equal(evening.next.day, "2026-11-06");

  const saturday = session(at("2026-11-07T10:00:00"), HOLIDAYS);
  assert.equal(saturday.tradingDay, false);
  assert.equal(saturday.next.at.getTime(), at("2026-11-09T08:45:00").getTime());

  // 24 and 25 December 2026 are listed holidays (Thursday, Friday): Wednesday's close opens on Monday.
  const christmas = session(at("2026-12-24T10:00:00"), HOLIDAYS);
  assert.equal(christmas.phase, null);
  assert.equal(christmas.holiday, "Commemoration of Christmas Day");
  assert.equal(session(at("2026-12-23T17:00:00"), HOLIDAYS).next.day, "2026-12-28");
  // Without the holiday list only weekends close.
  assert.equal(session(at("2026-12-24T10:00:00")).phase.id, "session-1");
});

test("trading days, WIB dates and the clock faces", () => {
  assert.equal(isTradingDay("2026-12-31", HOLIDAYS), false);
  assert.equal(nextTradingDay("2026-12-30", HOLIDAYS), "2027-01-04");
  // 23:30 UTC is already the next morning in Jakarta.
  assert.deepEqual(wib(new Date("2026-11-05T23:30:00Z")), { iso: "2026-11-06", dow: 5, secs: 23400 });
  assert.equal(clockText(new Date("2026-11-05T23:30:05Z")), "06:30:05");
  assert.equal(countdown(3725), "01:02:05");
  assert.equal(countdown(2 * 86400 + 3 * 3600 + 7 * 60), "2d 03:07");
  assert.equal(countdown(-4), "00:00:00");
  assert.equal(dayText("2026-11-06"), "Fri 6 Nov 2026");
});
