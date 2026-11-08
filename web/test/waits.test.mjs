// node --test web/test/*.mjs. The terminal's loading line says what a command
// waits on and, once slow, how the API bounds that source. Every network
// command needs an entry, and the numbers it quotes must be the API's.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { WAITS, waitFor, FEED_TIMEOUT_S, IPOT_TRIES, SHOW_MS, SLOW_MS } from "../public/waits.js";

const web = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (...p) => readFileSync(join(web, ...p), "utf8");

// Answered in the browser or from bundled data: nothing to wait on.
const LOCAL = new Set(["help", "clear", "json", "tools", "holidays", "list", "members"]);

test("every network command says what it waits on", () => {
  const names = [...read("public", "terminal.js").matchAll(/\{ name: "([a-z]+)", usage:/g)].map((m) => m[1]);
  assert.ok(names.length > 20);
  for (const n of names.filter((n) => !LOCAL.has(n))) assert.ok(WAITS[n]?.on, `no loading text for ${n}`);
  for (const w of Object.values(WAITS)) assert.ok(w.on.length > 5);
});

test("bandar screen and broker read the scan of the last session, bandar itself waits on IPOT", () => {
  assert.match(waitFor("bandar", "screen").on, /scan of the last session/);
  assert.match(waitFor("bandar", "BROKER").on, /scan of the last session/);
  assert.match(waitFor("bandar", "BBCA").on, /IPOT/);
  assert.match(waitFor("bandar", "BBCA").slow, /404/);
  assert.match(waitFor("market", "foreign").on, /bandar scan of the last session, and IPOT's market statistics/);
  assert.match(waitFor("market").on, /minute board/);
  assert.deepEqual(waitFor("holidays"), {});
  assert.deepEqual(waitFor("nope"), {});
});

test("the slow notes quote the API's own limits", () => {
  assert.match(read("src", "api.js"), new RegExp(`const FEED_TIMEOUT_MS = ${FEED_TIMEOUT_S * 1000};`));
  assert.match(read("src", "ipot.js"), new RegExp(`attempts = ${IPOT_TRIES},`));
  assert.match(WAITS.news.slow, new RegExp(`${FEED_TIMEOUT_S} s`));
  assert.match(WAITS.broker.slow, new RegExp(`${IPOT_TRIES} tries`));
  assert.ok(SHOW_MS < 1000 && SLOW_MS >= 3000);
});
