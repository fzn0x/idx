// The station shell both pages share: the Star button's live count, the
// hotkeys, the keys dialog, the pause switch for everything that moves, and
// the status bar's state lamp.
import { api } from "/app.js";

const $ = (id) => document.getElementById(id);

const compactCount = (v) => (v < 1000 ? String(v) : `${(v / 1000).toFixed(v < 10_000 ? 1 : 0).replace(/\.0$/, "")}k`);

/** Fills the Star button with the repository's count from /api/repo; on any failure it stays a plain Star link. */
export async function starCount() {
  const link = $("star"), n = $("star-n");
  if (!link || !n) return;
  try {
    const { data } = await api("repo");
    if (!Number.isInteger(data?.stars)) return;
    n.textContent = compactCount(data.stars);
    n.hidden = false;
    link.setAttribute("aria-label", `Star fzn0x/idx on GitHub, ${data.stars} stars`);
    link.title = `${data.stars} stars${Number.isInteger(data.forks) ? `, ${data.forks} forks` : ""} on GitHub`;
  } catch {
    // The count is a courtesy; the link works without it.
  }
}

const typing = (t) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

/**
 * Single-key shortcuts that never fire while typing, with a modifier held, or
 * behind an open dialog. `map` keys are KeyboardEvent.key values; `digit`
 * takes 1 to 9.
 */
export function hotkeys(map) {
  document.addEventListener("keydown", (ev) => {
    if (ev.defaultPrevented || ev.ctrlKey || ev.metaKey || ev.altKey || ev.isComposing) return;
    if (typing(ev.target) || document.querySelector("dialog[open]")) return;
    const fn = /^[1-9]$/.test(ev.key) && map.digit ? () => map.digit(Number(ev.key)) : map[ev.key] || map[ev.key.toLowerCase()];
    if (fn && fn(ev) !== false) ev.preventDefault();
  });
}

/** The keys dialog: a native <dialog>, opened with ? and closed with Esc, Close or a click outside. */
export function keysDialog() {
  const d = $("keys");
  if (!d) return () => {};
  $("keys-close")?.addEventListener("click", () => d.close());
  d.addEventListener("click", (ev) => { if (ev.target === d) d.close(); });
  return () => { if (!d.open) d.showModal(); };
}

let paused = false;
const pauseFns = new Set();
export const isPaused = () => paused;
export const onPause = (fn) => pauseFns.add(fn);

/** Stops (or restarts) the minute refresh, the clocks and the flips; the switch is the status bar's Pause button and P. */
export function togglePause(force) {
  paused = force ?? !paused;
  const b = $("pause");
  if (b) { b.setAttribute("aria-pressed", String(paused)); b.textContent = paused ? "Resume" : "Pause"; }
  for (const fn of pauseFns) fn(paused);
}

/** The status bar's lamp: live, closed, stale, offline or paused, with its words. */
export function lamp(kind, text) {
  const s = $("st-state");
  if (!s) return;
  s.dataset.state = kind;
  s.textContent = text;
}

/**
 * Moves to a panel by id: scrolls it into view, focuses it and leaves a
 * history entry, so Back retraces the jumps.
 */
export function jump(id) {
  const p = $(id);
  if (!p) return;
  if (location.hash !== `#${id}`) history.pushState({ panel: id }, "", `#${id}`);
  p.scrollIntoView({ block: "start" });
  p.focus({ preventScroll: true });
}

/** Wires the shared shell: Star count, Pause button, keys dialog and its button. Returns the dialog opener. */
export function initShell() {
  $("pause")?.addEventListener("click", () => togglePause());
  starCount();
  const openKeys = keysDialog();
  $("keys-open")?.addEventListener("click", openKeys);
  return openKeys;
}
