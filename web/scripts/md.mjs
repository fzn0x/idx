// Markdown -> HTML for idx's own documentation (README.md and docs/*.md).
//
// Not a general CommonMark implementation: it covers what those files use, and
// web/test/docs.test.mjs fails when a page renders with leftovers or unbalanced
// tags, so a new construct is noticed instead of silently mangled. No
// dependencies: CI runs `node --test` without `npm install`.
//
// Blocks: ATX headings (GitHub-style ids), paragraphs, fenced code, GFM
// tables (with `\|` escapes, also inside code spans), bullet and ordered
// lists (continuation lines, nested lists, loose items), blockquotes,
// thematic breaks. Raw HTML blocks (README's logo, badges and screenshots)
// are dropped; any other `<` is text and gets escaped.
// Inline: code spans, **strong**, *em*, [text](url), bare https URLs,
// backslash escapes, hard breaks.

const ESC = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" };
export const escapeHtml = (s) => s.replace(/[&<>"]/g, (c) => ESC[c]);

/** GitHub's heading ids (github-slugger): lowercase, punctuation dropped,
 *  each space a hyphen, `-1`, `-2` ... for repeats within one page. */
export function slugger() {
  const occurrences = new Map();
  return (text) => {
    const base = text
      .toLowerCase()
      .trim()
      .replace(/[^\p{L}\p{M}\p{N}\p{Pc} -]/gu, "")
      .replace(/ /g, "-");
    let slug = base;
    while (occurrences.has(slug)) {
      const n = occurrences.get(base) + 1;
      occurrences.set(base, n);
      slug = `${base}-${n}`;
    }
    occurrences.set(slug, 0);
    return slug;
  };
}

/** Plain text of an inline Markdown string (for ids and titles). */
export function inlineText(s) {
  return s
    .replace(/`+([^`]*?)`+/g, "$1")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/(^|[^*\w])\*([^*\s][^*]*?)\*(?![*\w])/g, "$1$2")
    .replace(/\\([\\`*_{}\[\]()#+\-.!|>])/g, "$1");
}

/**
 * Inline Markdown -> HTML. `href(url)` rewrites link targets (or returns null
 * to drop the link and keep its text).
 */
export function inline(src, href = (u) => u) {
  const slots = [];
  let s = inlineSlots(src, href, slots, true);
  // Restore the protected pieces; a link's text may hold code spans, so a
  // restored piece can itself hold placeholders.
  for (let depth = 0; depth < 8 && s.includes("\u0000"); depth++) s = s.replace(/\u0000(\d+)\u0000/g, (_, n) => slots[Number(n)]);
  return s;
}

/** A bare URL's end, as GitHub's autolinker finds it: trailing punctuation
 *  is not part of the URL, and neither is a `)` without its `(`. */
function trimUrl(url) {
  let tail = "";
  for (;;) {
    const last = url.at(-1);
    const unbalanced = last === ")" && url.split("(").length < url.split(")").length;
    if (!".,;:!?]".includes(last) && !unbalanced) return [url, tail];
    tail = last + tail;
    url = url.slice(0, -1);
  }
}

// Inline rendering with the protected pieces left as \0n\0 placeholders in
// `slots`, shared with the link texts rendered inside it.
function inlineSlots(src, href, slots, autolink) {
  const keep = (html) => `\u0000${slots.push(html) - 1}\u0000`;
  let s = src;
  // Code spans first: nothing inside them is Markdown.
  s = s.replace(/(`+)([\s\S]*?[^`])\1(?!`)/g, (_, ticks, code) => {
    const body = code.replace(/\n/g, " ");
    const trimmed = /^ .* $/.test(body) && body.trim() ? body.slice(1, -1) : body;
    return keep(`<code>${escapeHtml(trimmed)}</code>`);
  });
  // Backslash escapes.
  s = s.replace(/\\([\\`*_{}\[\]()#+\-.!|>])/g, (_, c) => keep(escapeHtml(c)));
  // Links: [text](url "title"?) with the text rendered inline (no autolinks:
  // an <a> inside an <a> is invalid).
  s = s.replace(/\[([^\]]+)\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g, (_, text, url) => {
    const inner = inlineSlots(text, href, slots, false);
    const target = href(url);
    return keep(target == null ? inner : `<a href="${escapeHtml(target)}">${inner}</a>`);
  });
  // Bare URLs (GitHub autolinks them). `*` ends one, so **https://...** works.
  if (autolink) {
    s = s.replace(/\bhttps?:\/\/[^\s<>"'`*\u0000]+/g, (m) => {
      const [url, tail] = trimUrl(m);
      return url.length > m.indexOf("//") + 2 ? keep(`<a href="${escapeHtml(url)}">${escapeHtml(url)}</a>`) + tail : m;
    });
  }
  s = escapeHtml(s);
  s = s.replace(/\*\*(?=\S)([\s\S]*?\S)\*\*/g, "<strong>$1</strong>");
  s = s.replace(/(^|[^*\w])\*(?=[^*\s])([^*]*?[^*\s])\*(?![*\w])/g, "$1<em>$2</em>");
  // Hard break: two trailing spaces before a newline.
  s = s.replace(/ {2,}\n/g, "<br>\n");
  return s;
}

const FENCE = /^( {0,3})(`{3,}|~{3,})\s*([^`\s]*)[^`]*$/;
const HEADING = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/;
const HR = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
const BULLET = /^( *)([-*+])[ \t]+(.*)$/;
const ORDERED = /^( *)(\d{1,9})([.)])[ \t]+(.*)$/;
const QUOTE = /^ {0,3}> ?(.*)$/;
export const HTML_BLOCK = /^ {0,3}<\/?(?:p|div|picture|source|img|a|details|summary|table|br|hr|h[1-6]|!--)(?=[\s>\/]|$)/i;

/** The Markdown without raw HTML blocks (outside code fences): what the
 *  HTML rendering drops, the Markdown pages drop too. */
export function dropHtmlBlocks(md) {
  const lines = md.split("\n");
  const out = [];
  let fence = null;
  for (let i = 0; i < lines.length; i++) {
    const f = lines[i].match(/^ {0,3}(`{3,}|~{3,})/);
    if (f) fence = fence ? (f[1][0] === fence ? null : fence) : f[1][0];
    if (!fence && !f && HTML_BLOCK.test(lines[i]) && (i === 0 || isBlank(lines[i - 1]))) {
      while (i < lines.length && !isBlank(lines[i])) i++;
      // i is on the blank line after the block; drop it when one is already out.
      if (out.length && isBlank(out[out.length - 1])) continue;
      i--;
      continue;
    }
    out.push(lines[i]);
  }
  return out.join("\n");
}
const TABLE_DELIM = /^\s*\|?\s*:?-+:?\s*(?:\|\s*:?-+:?\s*)*\|?\s*$/;

const isBlank = (l) => /^\s*$/.test(l);

function splitRow(line) {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|") && !s.endsWith("\\|")) s = s.slice(0, -1);
  const cells = [];
  let cur = "";
  for (let i = 0; i < s.length; i++) {
    if (s[i] === "\\" && s[i + 1] === "|") { cur += "|"; i++; continue; }
    if (s[i] === "|") { cells.push(cur.trim()); cur = ""; continue; }
    cur += s[i];
  }
  cells.push(cur.trim());
  return cells;
}

function startsBlock(line, next) {
  return (
    FENCE.test(line) || HEADING.test(line) || HR.test(line) || QUOTE.test(line) ||
    BULLET.test(line) || ORDERED.test(line) || HTML_BLOCK.test(line) ||
    (line.includes("|") && next !== undefined && TABLE_DELIM.test(next) && next.includes("-"))
  );
}

/**
 * Block Markdown -> HTML.
 * opts.href(url): link rewriting; opts.slug: a slugger() shared by the page;
 * opts.headings: array that receives {level, text, id}.
 */
export function render(src, opts = {}) {
  const href = opts.href ?? ((u) => u);
  const slug = opts.slug ?? slugger();
  const headings = opts.headings ?? [];
  const lines = src.replace(/\r\n?/g, "\n").replace(/\t/g, "    ").split("\n");
  const out = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    if (isBlank(line)) { i++; continue; }

    let m;
    if ((m = line.match(FENCE))) {
      const [, , fence, lang] = m;
      const body = [];
      i++;
      while (i < lines.length && !new RegExp(`^ {0,3}${fence[0]}{${fence.length},}\\s*$`).test(lines[i])) body.push(lines[i++]);
      i++; // closing fence (or end of file)
      const cls = lang ? ` class="language-${escapeHtml(lang)}"` : "";
      out.push(`<pre><code${cls}>${escapeHtml(body.join("\n"))}</code></pre>`);
      continue;
    }
    if ((m = line.match(HEADING))) {
      const level = m[1].length;
      const text = (m[2] ?? "").trim();
      const id = slug(inlineText(text));
      headings.push({ level, text: inlineText(text), id });
      out.push(`<h${level} id="${id}">${inline(text, href)}</h${level}>`);
      i++;
      continue;
    }
    if (HR.test(line)) { out.push("<hr>"); i++; continue; }
    if (HTML_BLOCK.test(line)) {
      // Raw HTML (README's logo, badges, screenshots): dropped up to the next blank line.
      while (i < lines.length && !isBlank(lines[i])) i++;
      continue;
    }
    if (line.includes("|") && i + 1 < lines.length && TABLE_DELIM.test(lines[i + 1]) && lines[i + 1].includes("-")) {
      const head = splitRow(line);
      // Column alignment as a class (no inline styles): c centre, r right, l left.
      const align = splitRow(lines[i + 1]).map((c) => (c.startsWith(":") && c.endsWith(":") ? "c" : c.endsWith(":") ? "r" : c.startsWith(":") ? "l" : null));
      i += 2;
      const rows = [];
      while (i < lines.length && !isBlank(lines[i]) && lines[i].includes("|")) rows.push(splitRow(lines[i++]));
      const cell = (tag, c, k) => `<${tag}${align[k] ? ` class="${align[k]}"` : ""}>${inline(c, href)}</${tag}>`;
      const pad = (r) => head.map((_, k) => r[k] ?? "");
      out.push(
        `<div class="tbl-wrap"><table class="tbl doc-tbl">` +
          `<thead><tr>${head.map((c, k) => cell("th", c, k)).join("")}</tr></thead>` +
          `<tbody>${rows.map((r) => `<tr>${pad(r).map((c, k) => cell("td", c, k)).join("")}</tr>`).join("")}</tbody>` +
          `</table></div>`,
      );
      continue;
    }
    if (QUOTE.test(line)) {
      const body = [];
      while (i < lines.length && !isBlank(lines[i]) && (QUOTE.test(lines[i]) || body.length)) {
        const q = lines[i].match(QUOTE);
        body.push(q ? q[1] : lines[i]);
        i++;
      }
      out.push(`<blockquote>${render(body.join("\n"), { href, slug, headings })}</blockquote>`);
      continue;
    }
    if (BULLET.test(line) || ORDERED.test(line)) {
      const [html, next] = list(lines, i, href, slug, headings);
      out.push(html);
      i = next;
      continue;
    }
    // Paragraph: until a blank line or the start of another block.
    const para = [line];
    i++;
    while (i < lines.length && !isBlank(lines[i]) && !startsBlock(lines[i], lines[i + 1])) para.push(lines[i++]);
    out.push(`<p>${inline(para.map((l) => l.replace(/^ +/, "")).join("\n"), href)}</p>`);
  }
  return out.join("\n");
}

function list(lines, start, href, slug, headings) {
  const first = lines[start].match(BULLET) ?? lines[start].match(ORDERED);
  const indent = first[1].length;
  const ordered = !lines[start].match(BULLET);
  const items = [];
  let i = start;
  while (i < lines.length) {
    const l = lines[i];
    const m = ordered ? l.match(ORDERED) : l.match(BULLET);
    if (!m || m[1].length !== indent) break;
    const content = [ordered ? m[4] : m[3]];
    const contentIndent = indent + (ordered ? m[2].length + m[3].length + 1 : 2);
    i++;
    // Continuation and nested blocks: indented lines, or lazy lines that do
    // not start a new block; a blank line ends the item unless the next line
    // is indented under it.
    while (i < lines.length) {
      const c = lines[i];
      if (isBlank(c)) {
        const nxt = lines[i + 1];
        if (nxt !== undefined && !isBlank(nxt) && nxt.match(/^( *)/)[1].length >= contentIndent) { content.push(""); i++; continue; }
        break;
      }
      const lead = c.match(/^( *)/)[1].length;
      if (lead >= contentIndent) { content.push(c.slice(contentIndent)); i++; continue; }
      if (lead <= indent && (BULLET.test(c) || ORDERED.test(c))) break;
      if (startsBlock(c, lines[i + 1])) break;
      content.push(c.trimStart());
      i++;
    }
    items.push(content.join("\n"));
    // A blank line between items of the same list keeps the list going.
    if (isBlank(lines[i] ?? "x") && i + 1 < lines.length) {
      const nm = ordered ? lines[i + 1].match(ORDERED) : lines[i + 1].match(BULLET);
      if (nm && nm[1].length === indent) i++;
    }
  }
  const tag = ordered ? "ol" : "ul";
  const startAttr = ordered && Number(first[2]) !== 1 ? ` start="${Number(first[2])}"` : "";
  const lis = items.map((body) => {
    const html = render(body, { href, slug, headings });
    // Tight item: a lone first paragraph (alone, or before a nested list) is
    // unwrapped; an item with several paragraphs keeps them.
    const tight = html.replace(/^<p>((?:(?!<\/?p>)[\s\S])*)<\/p>(?=\n<(?:ul|ol)[ >]|$)/, "$1");
    return `<li>${tight}</li>`;
  });
  return [`<${tag}${startAttr}>${lis.join("")}</${tag}>`, i];
}
