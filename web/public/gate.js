// The gate: the command line both pages share, an ARIA 1.2 combobox. Typing
// opens a listbox of suggestions (commands, tickers, earlier lines); Up and
// Down move through it, or recall earlier lines while it is closed; Tab
// completes; Enter runs; Esc closes the list, then clears the line, then
// leaves it. The page decides what running a line means (onRun).
import { el } from "/app.js";
import { t } from "/i18n.js";

/** The words beside each suggestion, in the page's language. */
const KIND = { command: t("perintah", "command"), ticker: t("saham", "ticker"), history: t("riwayat", "history"), example: t("contoh", "example") };
const AGAIN = t("jalankan lagi", "run again");

const TICKERISH = /^[a-z]{1,4}$/i;

/**
 * Suggestions for a partly typed line.
 * commands: [{ name, label?, help, value, run, symbols }]; tickers: [{ symbol, name }];
 * history: earlier lines, newest first. Each suggestion is
 * { kind, label, detail, value, run, match } where value is the whole new line.
 */
export function suggestions(line, { commands = [], tickers = [], history = [], examples = [], limit = 8 } = {}) {
  const raw = line.replace(/^\s+/, "");
  const lower = raw.toLowerCase();
  const out = [], seen = new Set();
  const push = (s) => { if (out.length < limit && !seen.has(s.value)) { seen.add(s.value); out.push(s); } };
  const ticker = (s, value, match) => push({ kind: "ticker", label: s.symbol, detail: s.name || "", value, run: true, match });

  if (!raw) {
    for (const h of history.slice(0, 5)) push({ kind: "history", label: h, detail: AGAIN, value: h, run: true, match: 0 });
    for (const e of examples) push({ kind: "example", label: e, detail: "", value: e, run: true, match: 0 });
    return out;
  }
  for (const h of history) if (h.toLowerCase().startsWith(lower) && h.toLowerCase() !== lower) push({ kind: "history", label: h, detail: AGAIN, value: h, run: true, match: raw.length });

  const toks = raw.split(/\s+/);
  const trailing = /\s$/.test(raw);
  if (toks.length === 1 && !trailing) {
    const t = toks[0], tl = t.toLowerCase(), tu = t.toUpperCase();
    for (const c of commands) if (c.name.startsWith(tl)) push({ kind: "command", label: c.label ?? c.name, detail: c.help, value: c.value, run: c.run, match: Math.min(t.length, (c.label ?? c.name).length) });
    if (TICKERISH.test(t)) for (const s of tickers) if (s.symbol.startsWith(tu)) ticker(s, `quote ${s.symbol}`, t.length);
    if (t.length >= 3) for (const s of tickers) if (s.name && s.name.toLowerCase().includes(tl)) ticker(s, `quote ${s.symbol}`, 0);
    return out;
  }
  // An argument: tickers for a command that takes symbols.
  const cmd = commands.find((c) => c.name === toks[0].toLowerCase());
  const cur = trailing ? "" : toks[toks.length - 1];
  const prev = trailing ? toks[toks.length - 1] : toks[toks.length - 2];
  if (cmd?.symbols && TICKERISH.test(cur) && !String(prev).startsWith("--")) {
    const head = raw.slice(0, raw.length - cur.length);
    for (const s of tickers) if (s.symbol.startsWith(cur.toUpperCase())) ticker(s, `${head}${s.symbol}`, cur.length);
  }
  return out;
}

/**
 * Mounts the combobox on an <input> inside a <form>, with a <ul role=listbox>
 * and a polite status line. suggest(line) returns suggestions; onRun(line)
 * runs one; history() gives earlier lines, oldest first.
 */
export function mountGate({ form, input, list, status, suggest, onRun, history = () => [], onFocus }) {
  let items = [], active = -1, recall = null, draft = "";
  const optId = (i) => `${list.id}-o${i}`;

  // A phone gets the short placeholder (data-short-placeholder) so the first control reads whole.
  const long = input.placeholder, short = input.dataset.shortPlaceholder;
  if (short) {
    const narrow = matchMedia("(max-width: 46rem)");
    const fit = () => { input.placeholder = narrow.matches ? short : long; };
    fit();
    narrow.addEventListener("change", fit);
  }

  function paint() {
    list.replaceChildren(...items.map((s, i) => {
      const key = el("span", { class: "key" });
      if (s.match > 0) key.append(el("mark", { text: s.label.slice(0, s.match) }), s.label.slice(s.match));
      else key.textContent = s.label;
      const li = el("li", { role: "option", id: optId(i), "aria-selected": String(i === active) }, [el("span", { class: "kind", text: KIND[s.kind] || s.kind }), key, el("span", { class: "what", text: s.detail })]);
      li.addEventListener("mousedown", (ev) => ev.preventDefault());
      li.addEventListener("click", () => accept(s, s.run));
      return li;
    }));
    const isOpen = items.length > 0;
    list.hidden = !isOpen;
    input.setAttribute("aria-expanded", String(isOpen));
    if (active >= 0) {
      input.setAttribute("aria-activedescendant", optId(active));
      list.children[active]?.scrollIntoView({ block: "nearest" });
    } else input.removeAttribute("aria-activedescendant");
  }
  function show(next) {
    items = next;
    active = -1;
    paint();
    if (status) status.textContent = items.length ? t(`${items.length} saran; panah atas dan bawah untuk memilih`, `${items.length} suggestion${items.length === 1 ? "" : "s"}; up and down to choose`) : "";
  }
  const close = () => { if (items.length || !list.hidden) show([]); };
  const refresh = () => show(suggest(input.value));

  function accept(s, run) {
    input.value = s.value;
    close();
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
    if (run) submit();
  }
  function submit() {
    const line = input.value.trim();
    close();
    recall = null;
    if (line) onRun(line);
  }

  form.addEventListener("submit", (ev) => { ev.preventDefault(); submit(); });
  input.addEventListener("input", () => { recall = null; refresh(); });
  input.addEventListener("focus", () => onFocus?.());
  input.addEventListener("blur", () => setTimeout(() => { if (document.activeElement !== input) close(); }, 0));
  input.addEventListener("keydown", (ev) => {
    const open = items.length > 0;
    const hist = history();
    switch (ev.key) {
      case "ArrowDown":
        ev.preventDefault();
        if (open) { active = active < items.length - 1 ? active + 1 : 0; paint(); }
        else if (recall != null) { recall += 1; if (recall >= hist.length) { recall = null; input.value = draft; } else input.value = hist[recall]; }
        else refresh();
        break;
      case "ArrowUp":
        ev.preventDefault();
        if (open) { active = active > 0 ? active - 1 : items.length - 1; paint(); }
        else if (hist.length) {
          if (recall == null) { draft = input.value; recall = hist.length; }
          recall = Math.max(0, recall - 1);
          input.value = hist[recall];
        }
        break;
      case "Tab":
        if (open && !ev.shiftKey) { ev.preventDefault(); accept(items[Math.max(0, active)], false); if (/\s$/.test(input.value)) refresh(); }
        break;
      case "Enter":
        if (open && active >= 0) { ev.preventDefault(); accept(items[active], items[active].run); }
        break;
      case "Escape":
        // Close the list, then clear the line, then leave it so the single-key shortcuts work.
        if (open) { ev.preventDefault(); close(); }
        else if (input.value) { ev.preventDefault(); input.value = ""; recall = null; }
        else input.blur();
        break;
      default:
    }
  });
  return { close, focus: () => input.focus(), set: (v) => { input.value = v; close(); } };
}
