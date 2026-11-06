// node --test web/test/*.mjs (no network). The docs pages, llms.txt and
// llms-full.txt are generated from README.md and docs/*.md at deploy time
// (web/scripts/docs.mjs); this renders them from the current sources and
// checks what a broken page would get wrong.
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { render, inline, slugger } from "../scripts/md.mjs";
import { PAGES, ORIGIN, REPO, ROOT, build, exitCodes, exitCodesFact, htmlPath, mdPath, llmsTxt, resolveHref } from "../scripts/docs.mjs";

const files = build(ROOT);
const html = (slug) => files.get(`docs/${slug}.html`);
const body = (slug) => html(slug).split('<article class="doc-body">')[1].split("</article>")[0];
const readRepo = (p) => readFileSync(join(ROOT, p), "utf8").replace(/\r\n?/g, "\n");

test("md: headings get GitHub ids, repeats are numbered", () => {
  const headings = [];
  const out = render("# JSON output\n\n## `idx.quotes.v1` (`stocks quote`)\n\n## Errors\n\n## Errors", { headings });
  assert.match(out, /<h2 id="idxquotesv1-stocks-quote"><code>idx.quotes.v1<\/code> \(<code>stocks quote<\/code>\)<\/h2>/);
  assert.deepEqual(headings.map((h) => h.id), ["json-output", "idxquotesv1-stocks-quote", "errors", "errors-1"]);
  const s = slugger();
  assert.equal(s("Kiro (IDE and CLI; formerly Amazon Q Developer CLI)"), "kiro-ide-and-cli-formerly-amazon-q-developer-cli");
});

test("md: code is escaped, never parsed", () => {
  assert.equal(render("```json\n{\"a\": \"<b>\"}\n```"), '<pre><code class="language-json">{&quot;a&quot;: &quot;&lt;b&gt;&quot;}</code></pre>');
  // A code span across a line break, and text that looks like a tag (docs/mcp.md).
  assert.equal(render("header `Authorization: Bearer\n<token>`: ChatGPT"), "<p>header <code>Authorization: Bearer &lt;token&gt;</code>: ChatGPT</p>");
  assert.equal(inline("**bold** and *em* and `**not**`"), "<strong>bold</strong> and <em>em</em> and <code>**not**</code>");
});

test("md: tables honour \\| inside and outside code spans, alignment is a class", () => {
  const out = render("| a | b |\n| --- | --- |\n| `x \\| y` | `annual` \\| `ttm` |");
  assert.match(out, /<td><code>x \| y<\/code><\/td><td><code>annual<\/code> \| <code>ttm<\/code><\/td>/);
  assert.match(render("| a | b |\n| :-: | --: |\n| 1 | 2 |"), /<td class="c">1<\/td><td class="r">2<\/td>/);
});

test("md: lists, continuation lines, blockquotes, links, bare URLs", () => {
  assert.equal(render("1. one\n   more\n2. two"), "<ol><li>one\nmore</li><li>two</li></ol>");
  assert.equal(render("- a\n  - b\n- c"), "<ul><li>a\n<ul><li>b</li></ul></li><li>c</li></ul>");
  // A loose item keeps both paragraphs, and the markup stays balanced.
  assert.equal(render("- a\n\n  more\n- b"), "<ul><li><p>a</p>\n<p>more</p></li><li>b</li></ul>");
  assert.equal(render("> quoted\n> text"), "<blockquote><p>quoted\ntext</p></blockquote>");
  assert.equal(inline("[docs](mcp.md) and https://x.example/a."), '<a href="mcp.md">docs</a> and <a href="https://x.example/a">https://x.example/a</a>.');
  assert.equal(inline("**https://ihsg.midoriresearch.com/terminal**"), '<strong><a href="https://ihsg.midoriresearch.com/terminal">https://ihsg.midoriresearch.com/terminal</a></strong>');
  assert.equal(inline("(see https://x.example/a_(b))"), '(see <a href="https://x.example/a_(b)">https://x.example/a_(b)</a>)');
  assert.equal(render('<p align="center">\n  <img src="x.png">\n</p>\n\ntext'), "<p>text</p>");
});

test("md: link text keeps its code spans and never nests a link", () => {
  assert.equal(inline("[`idx schema` and **more**](json-output.md#errors)"), '<a href="json-output.md#errors"><code>idx schema</code> and <strong>more</strong></a>');
  assert.equal(inline("[https://a.example/x](https://a.example/x)"), '<a href="https://a.example/x">https://a.example/x</a>');
});

test("links: pages stay on the site, other files go to GitHub, odd schemes are dropped", () => {
  const mcp = PAGES.find((p) => p.slug === "mcp");
  assert.equal(resolveHref("json-output.md#errors", mcp, "html"), "/docs/json-output#errors");
  assert.equal(resolveHref("json-output.md#errors", mcp, "md"), `${ORIGIN}/docs/json-output.md#errors`);
  assert.equal(resolveHref("../README.md", mcp, "html"), "/docs/");
  assert.equal(resolveHref("../web/", mcp, "md"), `${REPO}/blob/main/web/`);
  assert.equal(resolveHref("#tools", mcp, "html"), "#tools");
  // llms-full.txt joins every page, so a same-page anchor must name its page.
  assert.equal(resolveHref("#tools", mcp, "md"), `${ORIGIN}/docs/mcp.md#tools`);
  assert.equal(resolveHref("javascript:alert(1)", mcp, "html"), null);
  assert.equal(render("[x](javascript:void)", { href: (u) => resolveHref(u, mcp, "html") }), "<p>x</p>");
});

test("every docs/*.md file is a page, and every page renders", () => {
  const docs = readdirSync(join(ROOT, "docs")).filter((f) => f.endsWith(".md")).map((f) => `docs/${f}`);
  const listed = PAGES.map((p) => p.src).filter((s) => s !== "README.md");
  for (const d of docs) assert.ok(listed.includes(d), `${d} is not on the site: add it to PAGES in web/scripts/docs.mjs`);
  for (const s of listed) assert.ok(docs.includes(s), `${s} is in PAGES but does not exist`);
  assert.equal(new Set(PAGES.map((p) => p.slug)).size, PAGES.length, "slugs are unique");
  for (const p of PAGES) {
    assert.ok(files.get(`docs/${p.slug}.md`).startsWith("# "), `${p.slug}.md starts with a title`);
    assert.ok(html(p.slug).includes('<main class="doc" id="doc">'), p.slug);
    for (const k of ["slug", "src", "nav", "panel", "desc"]) assert.ok(p[k], `${p.slug} ${k}`);
  }
});

test("rendered pages carry no Markdown leftovers, balanced tags and well-formed tables", () => {
  const VOID = new Set(["br", "hr", "img", "meta", "link", "source", "input"]);
  for (const p of PAGES) {
    const text = body(p.slug).replace(/<pre>[\s\S]*?<\/pre>/g, "").replace(/<code>[\s\S]*?<\/code>/g, "");
    for (const bad of ["**", "](", "```", "<p>|", "<p>#", "\\|", "undefined", "\u0000"]) assert.ok(!text.includes(bad), `${p.slug}: leftover ${JSON.stringify(bad)}`);
    const stack = [];
    for (const [, close, tag] of html(p.slug).replace(/<!doctype[^>]*>/i, "").matchAll(/<(\/?)([a-z][a-z0-9]*)\b[^>]*>/gi)) {
      const t = tag.toLowerCase();
      if (VOID.has(t)) continue;
      if (!close) stack.push(t);
      else assert.equal(stack.pop(), t, `${p.slug}: </${t}> closes the wrong element`);
    }
    assert.deepEqual(stack, [], `${p.slug}: unclosed elements`);
    for (const table of html(p.slug).match(/<table[\s\S]*?<\/table>/g) ?? []) {
      const cols = (table.match(/<thead><tr>([\s\S]*?)<\/tr>/)[1].match(/<th/g) ?? []).length;
      for (const row of table.match(/<tbody>[\s\S]*<\/tbody>/)[0].match(/<tr>[\s\S]*?<\/tr>/g) ?? []) {
        assert.equal((row.match(/<td/g) ?? []).length, cols, `${p.slug}: ${row.slice(0, 80)}`);
      }
    }
  }
});

test("every internal link resolves to a page, a file or an anchor that exists", () => {
  const ids = new Map(PAGES.map((p) => [htmlPath(p.slug), new Set([...html(p.slug).matchAll(/ id="([^"]+)"/g)].map((m) => m[1]))]));
  const pub = join(ROOT, "web", "public");
  const routes = new Set(["/", "/terminal", "/en/", "/en/terminal"]);
  const exists = (path) => routes.has(path) || ids.has(path) || files.has(path.slice(1)) || existsSync(join(pub, path.slice(1)));
  for (const p of PAGES) {
    for (const [, raw] of html(p.slug).matchAll(/<(?:a|link)\b[^>]*\bhref="([^"]+)"/g)) {
      // Absolute links to this site (agent-guide spells them out) must work too.
      const href = raw.startsWith(`${ORIGIN}/`) ? raw.slice(ORIGIN.length) : raw;
      if (/^(https?:|mailto:)/.test(href)) continue;
      const [path, anchor] = href.split("#");
      const page = path === "" ? htmlPath(p.slug) : path;
      assert.ok(page.startsWith("/"), `${p.slug}: relative link ${href}`);
      assert.ok(exists(page), `${p.slug}: unknown target ${href}`);
      if (anchor) assert.ok(ids.get(page)?.has(anchor), `${p.slug}: missing anchor ${href}`);
    }
    for (const [, src] of html(p.slug).matchAll(/<img\b[^>]*\bsrc="([^"]+)"/g)) assert.ok(exists(src), `${p.slug}: image ${src}`);
  }
  for (const [name, text] of files) {
    if (!name.endsWith(".md") && !name.endsWith(".txt")) continue;
    assert.doesNotMatch(text, /\]\((?!https?:|mailto:)[^)]+\)/, `${name}: relative link left`);
    assert.doesNotMatch(text, /<p align|<img |<picture/, `${name}: README header left`);
    for (const [, url] of text.matchAll(/\]\((https:\/\/ihsg\.midoriresearch\.com[^)\s]*)\)/g)) {
      const { pathname, hash } = new URL(url);
      assert.ok(exists(pathname), `${name}: ${url}`);
      if (hash && pathname.endsWith(".md")) {
        const target = PAGES.find((q) => mdPath(q.slug) === pathname);
        assert.ok(ids.get(htmlPath(target.slug)).has(hash.slice(1)), `${name}: missing anchor ${url}`);
      }
    }
  }
});

test("the landing page, the terminal and the 404 page link only to docs that exist", () => {
  const pub = join(ROOT, "web", "public");
  const ids = new Map(PAGES.map((p) => [htmlPath(p.slug), new Set([...html(p.slug).matchAll(/ id="([^"]+)"/g)].map((m) => m[1]))]));
  let seen = 0;
  for (const page of readdirSync(pub).filter((f) => f.endsWith(".html"))) {
    for (const [, href] of readFileSync(join(pub, page), "utf8").matchAll(/href="(\/(?:docs|llms)[^"]*)"/g)) {
      seen++;
      const [path, anchor] = href.split("#");
      assert.ok(ids.has(path) || files.has(path.slice(1)), `${page}: ${href}`);
      if (anchor) assert.ok(ids.get(path)?.has(anchor), `${page}: missing anchor ${href}`);
    }
  }
  assert.ok(seen > 0, "the landing page links to the docs");
});

test("pages carry canonical, share and Markdown alternate tags, and work under the CSP", () => {
  const meta = (h, k) => h.match(new RegExp(`<meta (?:property|name)="${k}" content="([^"]*)">`))?.[1];
  for (const p of PAGES) {
    const h = html(p.slug);
    const url = `${ORIGIN}${htmlPath(p.slug)}`;
    assert.ok(h.includes(`<link rel="canonical" href="${url}">`), p.slug);
    assert.equal(meta(h, "og:url"), url);
    for (const k of ["description", "og:title", "og:description", "og:image", "og:image:alt"]) assert.ok(meta(h, k), `${p.slug} ${k}`);
    assert.equal(meta(h, "og:image"), `${ORIGIN}/og.png`);
    assert.equal(meta(h, "twitter:card"), "summary_large_image");
    assert.ok(h.includes(`<link rel="alternate" type="text/markdown" href="${mdPath(p.slug)}"`), p.slug);
    // script-src 'self' and no inline styles: the pages need neither.
    assert.doesNotMatch(h, /<script\b|\son[a-z]+=|\sstyle="/i, `${p.slug}: script, handler or inline style`);
    for (const [, href] of h.matchAll(/<link rel="stylesheet" href="([^"]+)"/g)) {
      assert.ok(href === "/docs.css" || href.startsWith("https://fonts.googleapis.com/"), `${p.slug}: stylesheet ${href}`);
    }
  }
});

test("llms.txt follows llmstxt.org and lists every page", () => {
  const t = llmsTxt(ROOT);
  const lines = t.split("\n");
  assert.equal(lines[0], "# idx");
  assert.match(lines[2], /^> \S/);
  const sections = lines.filter((l) => l.startsWith("## "));
  assert.deepEqual(sections, ["## Docs", "## Optional"]);
  const links = [...t.matchAll(/^- \[([^\]]+)\]\(([^)]+)\): \S/gm)].map((m) => m[2]);
  for (const p of PAGES) assert.ok(links.includes(`${ORIGIN}${mdPath(p.slug)}`), p.slug);
  for (const l of links.filter((u) => u.startsWith(ORIGIN))) assert.ok(files.has(new URL(l).pathname.slice(1)) || ["/terminal"].includes(new URL(l).pathname), l);
  assert.doesNotMatch(t, /\b\d+ (tools|clients|commands)\b/, "no counts that go stale");
});

test("llms.txt states the exit codes and error codes of docs/json-output.md", () => {
  const codes = exitCodes(readRepo("docs/json-output.md"), "## Errors");
  assert.ok(codes.size >= 6);
  const line = llmsTxt(ROOT).split("\n").find((l) => l.startsWith("- Exit codes"));
  assert.ok(line, "llms.txt has an exit-code line");
  assert.match(line, /\b0 ok\b/);
  for (const [exit, names] of codes) {
    assert.ok(names.length, `exit ${exit} has error codes`);
    assert.ok(line.includes(`${exit} ${names.map((c) => `\`${c}\``).join(", ")}`), `llms.txt: exit ${exit} with ${names.join(", ")}`);
  }
});

test("llms.txt follows a new error code in docs/json-output.md", () => {
  const md = readRepo("docs/json-output.md").replace("| 3 | `network`, `provider` |", "| 3 | `network`, `provider`, `blocked` |");
  assert.match(exitCodesFact(md), /; 3 `network`, `provider`, `blocked`;/);
  assert.throws(() => exitCodesFact("# JSON output\n\nno table\n"), /Errors/);
});

test("the agent guide's exit codes match docs/json-output.md", () => {
  const want = exitCodes(readRepo("docs/json-output.md"), "## Errors");
  const got = exitCodes(readRepo("docs/agent-guide.md"), "## Exit codes");
  assert.deepEqual(got.get("0"), [], "exit 0 is success");
  got.delete("0");
  assert.deepEqual(got, want, "docs/agent-guide.md and docs/json-output.md list different exit codes");
});

test("llms-full.txt holds every page once, in order", () => {
  const full = files.get("llms-full.txt");
  let at = 0;
  for (const p of PAGES) {
    const k = full.indexOf(`Source: ${ORIGIN}${mdPath(p.slug)}\n`, at);
    assert.ok(k > at, p.slug);
    assert.equal(full.indexOf(`Source: ${ORIGIN}${mdPath(p.slug)}\n`, k + 1), -1, `${p.slug} twice`);
    at = k;
  }
});
