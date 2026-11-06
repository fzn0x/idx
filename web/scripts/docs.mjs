// Builds the documentation of ihsg.midoriresearch.com from the repository's
// README.md and docs/*.md, so the site never says something the repository
// does not:
//
//   /docs/, /docs/<slug>             HTML for people (docs/index.html, docs/<slug>.html)
//   /docs/index.md, /docs/<slug>.md  the same pages as Markdown, for agents
//   /llms.txt                        index in the llmstxt.org format
//   /llms-full.txt                   every page in one file
//
// wrangler.toml's [build] step runs this before every `wrangler dev` and
// `wrangler deploy`; the outputs are gitignored, and web/public/docs/ is
// rewritten from scratch each time (keep hand-written files out of it).
// `node web/scripts/docs.mjs` writes them by hand. web/test/docs.test.mjs
// renders every page from the current sources and checks tags, links and
// anchors, so a docs edit that breaks the site fails CI rather than the deploy.
//
// The pages take the site's station-board look from public/docs.css alone
// (they load no other stylesheet and no script).

import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { dropHtmlBlocks, escapeHtml, inlineText, render, slugger } from "./md.mjs";

export const ORIGIN = "https://ihsg.midoriresearch.com";
export const REPO = "https://github.com/fzn0x/idx";
export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** Every page, in navigation and llms.txt order. Titles come from each file's
 *  first `# ` heading (README has none, so it gets `title`); `panel` is the
 *  page's Indonesian panel name, as the monitor names its panels. Descriptions
 *  say what a page answers; no counts, they would go stale.
 *  A new docs/*.md file must be added here (docs.test.mjs fails otherwise). */
export const PAGES = [
  { slug: "index", src: "README.md", title: "idx", nav: "Overview", panel: "Ikhtisar", desc: "What idx is, how to install it, every command, output for scripts, the MCP server in brief, bandarmology and the data sources." },
  { slug: "agent-guide", src: "docs/agent-guide.md", nav: "For agents", panel: "Panduan agen", desc: "How an AI agent should call idx (MCP or the CLI), the JSON contract and exit codes in one place, recipes, and an agent-led install." },
  { slug: "mcp", src: "docs/mcp.md", nav: "MCP server", panel: "Server MCP", desc: "Registering idx with each agent, idx mcp setup, the HTTP transport, and every MCP tool with its arguments." },
  { slug: "json-output", src: "docs/json-output.md", nav: "JSON output", panel: "Keluaran JSON", desc: "The envelope, every payload schema field by field, errors, exit codes and the compatibility promise." },
  { slug: "configuration", src: "docs/configuration.md", nav: "Configuration", panel: "Konfigurasi", desc: "Config file, IDX_* environment variables, cache location and lifetimes." },
  { slug: "ownership", src: "docs/ownership.md", nav: "Ownership data", panel: "Kepemilikan", desc: "The KSEI register of holders above 1 %: source files, the GitHub mirror, the local database and its queries." },
  { slug: "stockbit", src: "docs/stockbit.md", nav: "Stockbit realtime", panel: "Stockbit", desc: "The opt-in realtime provider: logging in, the order book, the MCP tools and where the session lives." },
];

export const SUMMARY =
  "Indonesian stock market (IDX / IHSG) analysis for people and AI agents: one open-source binary with a CLI that prints tables, a JSON mode for scripts (`idx -o json`) and an MCP server for agents (`idx mcp`), built on free public data with no API keys.";

const FACTS = [
  "Tickers are IDX codes (`BBCA`, `TLKM`); the `.JK` suffix is optional and `^JKSE` is the IHSG composite.",
  "`idx -o json <command>` prints one envelope on stdout: `schema`, `provider`, `fetched_at`, `cached`, `stale`, `warnings`, `data`. Errors go to stderr as `{\"error\":{\"code\":...,\"message\":...},\"exit_code\":N}`.",
  "Exit codes: 0 ok, 1 internal, 2 usage, 3 network or provider, 4 not found, 5 offline with nothing cached, 6 config or a provider that cannot serve the command.",
  "`idx mcp` serves the commands as typed MCP tools over stdio (`idx mcp --http` for URL-only clients); `idx mcp setup <client>` prints the registration for each agent.",
  "A value the source does not publish is `null`; nothing is estimated. Verdicts and signals are labelled rule-based heuristics, not investment advice.",
  "Every page below is Markdown; the same page as HTML drops the `.md` (`/docs/mcp`).",
];

export const htmlPath = (slug) => (slug === "index" ? "/docs/" : `/docs/${slug}`);
export const mdPath = (slug) => `/docs/${slug}.md`;

const read = (root, p) => readFileSync(join(root, p), "utf8").replace(/\r\n?/g, "\n");
const two = (n) => String(n).padStart(2, "0");

/** Cargo.toml's package version: the docs follow main, this says which. */
export function version(root = ROOT) {
  return read(root, "Cargo.toml").match(/^version = "([^"]+)"/m)?.[1] ?? "unknown";
}

/** Link target in `page`'s source -> site URL (html) or absolute Markdown URL (md).
 *  Links to another page stay on this site; links to any other repository
 *  file go to it on GitHub. A same-page `#anchor` stays relative in the HTML
 *  and names its page in the Markdown, which llms-full.txt joins into one
 *  file. Only http(s) and mailto links are kept (null drops the link and
 *  keeps its text). */
export function resolveHref(url, page, mode) {
  if (url.startsWith("#")) return mode === "html" ? url : `${ORIGIN}${mdPath(page.slug)}${url}`;
  if (/^[a-z][a-z0-9+.-]*:/i.test(url)) return /^(?:https?|mailto):/i.test(url) ? url : null;
  const [path, hash] = url.split("#");
  const anchor = hash ? `#${hash}` : "";
  const resolved = posix.normalize(posix.join(posix.dirname(page.src), path));
  const target = PAGES.find((p) => p.src === resolved);
  if (target) return mode === "html" ? `${htmlPath(target.slug)}${anchor}` : `${ORIGIN}${mdPath(target.slug)}${anchor}`;
  return `${REPO}/blob/main/${resolved}${anchor}`;
}

/** Rewrites `](target)` links outside code fences and code spans. */
function rewriteMarkdownLinks(md, page) {
  let fence = null;
  return md
    .split("\n")
    .map((line) => {
      const f = line.match(/^ {0,3}(`{3,}|~{3,})/);
      if (f) {
        if (!fence) fence = f[1][0];
        else if (f[1][0] === fence) fence = null;
        return line;
      }
      if (fence) return line;
      return line
        .split(/(`+[^`]*`+)/)
        .map((part, k) => (k % 2 ? part : part.replace(/\]\(\s*<?([^)\s>]+)>?\)/g, (_, u) => `](${resolveHref(u, page, "md") ?? u})`)))
        .join("");
    })
    .join("\n");
}

/** README starts with an HTML header (logo, tagline, badges, screenshot) and a
 *  rule; the pages keep only its words, as a lead paragraph. */
function stripReadmeHeader(md) {
  const lines = md.split("\n");
  const lead = [];
  let i = 0;
  while (i < lines.length) {
    const l = lines[i];
    if (/^\s*$/.test(l) || /^ {0,3}([-*_])(?:\s*\1){2,}\s*$/.test(l)) { i++; continue; }
    if (!/^\s*</.test(l)) break;
    const block = [];
    while (i < lines.length && !/^\s*$/.test(lines[i])) block.push(lines[i++]);
    const html = block.join(" ");
    if (!/<img|<picture/i.test(html)) {
      const text = html.replace(/<br\s*\/?>/gi, " ").replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
      if (text) lead.push(text);
    }
  }
  return { lead: lead.join(" "), rest: lines.slice(i).join("\n") };
}

/** One page's Markdown: `clean` keeps the source's links (for the HTML),
 *  `md` is what agents get: a title, absolute links, no HTML header. */
export function pageMarkdown(page, root = ROOT) {
  let clean = read(root, page.src);
  let title = page.title;
  if (page.src === "README.md") {
    const { lead, rest } = stripReadmeHeader(clean);
    clean = `# ${title}\n\n${lead ? `> ${lead}\n\n` : ""}${dropHtmlBlocks(rest.trimStart())}`;
  } else {
    title = inlineText(clean.match(/^# (.+)$/m)?.[1] ?? page.nav);
  }
  return { title, clean, md: `${rewriteMarkdownLinks(clean, page).trimEnd()}\n` };
}

/** The page list; the current page opens into its numbered `##` sections. */
function railHtml(current, headings) {
  const pages = PAGES.map((p, k) => {
    const here = p === current;
    const sections = headings.filter((h) => h.level === 2);
    const toc = here && sections.length
      ? `<ul class="toc">${sections.map((h, j) => `<li><a href="#${h.id}"><span class="no">${two(j + 1)}</span><span class="t">${escapeHtml(h.text)}</span></a></li>`).join("")}</ul>`
      : "";
    return `<li><a href="${htmlPath(p.slug)}"${here ? ' aria-current="page"' : ""}><span class="no">${two(k + 1)}</span><span class="t">${escapeHtml(p.nav)}</span></a>${toc}</li>`;
  }).join("");
  const machine = [
    [mdPath(current.slug), "This page as Markdown"],
    ["/llms.txt", "llms.txt, the index"],
    ["/llms-full.txt", "llms-full.txt, every page"],
    [`${REPO}/blob/main/${current.src}`, "Source on GitHub"],
  ].map(([href, text]) => `<li><a href="${href}"><span class="t">${text}</span></a></li>`).join("");
  return `<nav class="rail" aria-label="Documentation">
  <p class="ph"><span class="ttl" lang="id">Halaman</span><span class="sub">Pages</span></p>
  <ul class="pages">${pages}</ul>
  <p class="ph"><span class="ttl" lang="id">Untuk mesin</span><span class="sub">Machine-readable</span></p>
  <ul class="pages plain">${machine}</ul>
</nav>`;
}

/** One page as HTML: the site's top bar, a page rail, the article. */
export function pageHtml(page, root = ROOT) {
  const { title, clean } = pageMarkdown(page, root);
  const ver = version(root);
  const headings = [];
  // The HTML links to site pages; the Markdown carries absolute .md URLs.
  const body = render(clean, { href: (u) => resolveHref(u, page, "html"), slug: slugger(), headings });
  const canonical = `${ORIGIN}${htmlPath(page.slug)}`;
  const full = page.slug === "index" ? "idx documentation" : `${title} · idx docs`;
  const desc = escapeHtml(page.desc);
  const no = two(PAGES.indexOf(page) + 1);
  // The closing passport string, as every panel on the site ends.
  const mline = `IDX<<DOCS<<${page.slug.toUpperCase().replace(/-/g, "<")}<<V${ver}`;
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(full)}</title>
<meta name="description" content="${desc}">
<link rel="canonical" href="${canonical}">
<link rel="alternate" type="text/markdown" href="${mdPath(page.slug)}" title="${escapeHtml(title)} as Markdown">
<meta name="robots" content="index,follow,max-image-preview:large">
<meta name="theme-color" content="#0b0f14">
<meta property="og:type" content="article">
<meta property="og:site_name" content="idx · Midori Research">
<meta property="og:locale" content="en_US">
<meta property="og:title" content="${escapeHtml(full)}">
<meta property="og:description" content="${desc}">
<meta property="og:url" content="${canonical}">
<meta property="og:image" content="${ORIGIN}/og.png">
<meta property="og:image:type" content="image/png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="idx: IDX stocks for people, scripts and AI agents. idx stocks quote BBCA prints a table, idx -o json a stable JSON envelope, idx mcp an MCP server.">
<meta name="twitter:card" content="summary_large_image">
<link rel="icon" href="/mark.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Martian+Mono:wdth,wght@75..100,300..700&display=swap">
<link rel="stylesheet" href="/docs.css">
</head>
<body class="docs-page">
<a class="skip" href="#doc">Skip to the page</a>
<header class="topbar">
  <a class="brand" href="/" aria-label="idx home"><img src="/mark.svg" alt="" width="22" height="22"><span>idx</span></a>
  <nav class="nav" aria-label="Site">
    <a href="/">Monitor</a>
    <a href="/terminal">Terminal</a>
    <a href="/docs/" aria-current="page">Docs</a>
  </nav>
  <a class="md" href="${mdPath(page.slug)}" aria-label="This page as Markdown">Markdown</a>
</header>
<div class="docs">
${railHtml(page, headings)}
<main class="doc" id="doc">
<header class="ph"><span class="no">${no}</span><span class="ttl" lang="id">${escapeHtml(page.panel)}</span><span class="sub">${escapeHtml(page.src)}</span><span class="age">Version <b>${escapeHtml(ver)}</b></span></header>
<article class="doc-body">
${body}
</article>
<p class="mline"><span class="sr-only">End of ${escapeHtml(title)}, idx documentation for version ${escapeHtml(ver)}.</span><span aria-hidden="true">${escapeHtml(mline)}</span></p>
</main>
</div>
<footer class="sign">
  <div>
    <p class="place">Jakarta. <b>Midori Research</b> · MIT licence · <a href="mailto:desk@midoriresearch.com">desk@midoriresearch.com</a></p>
    <p class="links"><a href="/">Monitor</a> <a href="/terminal">Web terminal</a> <a href="${REPO}">Source on GitHub</a> <a href="${REPO}/blob/main/CHANGELOG.md">Changelog</a></p>
  </div>
  <p class="disc">These pages are generated from the repository's README.md and docs/ and follow its main branch. idx is research tooling, not investment advice, and is not affiliated with IDX, KSEI, Yahoo, Microsoft or Stockbit.</p>
</footer>
<div class="statusbar">
  <span class="st-state" data-state="idle">Docs</span>
  <span class="st-age">Version <b>${escapeHtml(ver)}</b><span class="st-more"> · follows main</span></span>
  <span class="st-src">${escapeHtml(page.src)}</span>
  <span class="st-right"><a href="${mdPath(page.slug)}">Markdown</a> <a href="/llms.txt">llms.txt</a></span>
</div>
</body>
</html>
`;
}

export function llmsTxt(root = ROOT) {
  const lines = ["# idx", "", `> ${SUMMARY}`, "", `Version ${version(root)}; these pages follow the main branch of ${REPO} (\`idx version\` prints the installed one).`, ""];
  for (const f of FACTS) lines.push(`- ${f}`);
  lines.push("", "## Docs", "");
  for (const p of PAGES) {
    const { title } = pageMarkdown(p, root);
    lines.push(`- [${p.slug === "index" ? "Overview" : title}](${ORIGIN}${mdPath(p.slug)}): ${p.desc}`);
  }
  lines.push(
    "",
    "## Optional",
    "",
    `- [All pages in one file](${ORIGIN}/llms-full.txt): the pages above, concatenated`,
    `- [Web terminal](${ORIGIN}/terminal): the same commands in a browser, no install`,
    `- [Source code](${REPO}): Rust, MIT licence; AGENTS.md describes the architecture`,
    `- [Changelog](${REPO}/blob/main/CHANGELOG.md): what changed in each release`,
    "",
  );
  return lines.join("\n");
}

export function llmsFullTxt(root = ROOT) {
  const parts = [`# idx documentation\n\n> ${SUMMARY}\n\nVersion ${version(root)}. Generated from README.md and docs/*.md of ${REPO}.\n`];
  for (const p of PAGES) parts.push(`---\nSource: ${ORIGIN}${mdPath(p.slug)}\n\n${pageMarkdown(p, root).md}`);
  return parts.join("\n");
}

/** Every generated file: path under web/public -> content. */
export function build(root = ROOT) {
  const files = new Map();
  for (const p of PAGES) {
    files.set(`docs/${p.slug}.md`, pageMarkdown(p, root).md);
    files.set(`docs/${p.slug}.html`, pageHtml(p, root));
  }
  files.set("llms.txt", llmsTxt(root));
  files.set("llms-full.txt", llmsFullTxt(root));
  return files;
}

export function write(outDir = join(ROOT, "web", "public"), root = ROOT) {
  rmSync(join(outDir, "docs"), { recursive: true, force: true });
  const files = build(root);
  for (const [rel, content] of files) {
    mkdirSync(dirname(join(outDir, rel)), { recursive: true });
    writeFileSync(join(outDir, rel), content);
  }
  return files;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const files = write(process.argv[2] ? resolve(process.argv[2]) : undefined);
  console.log(`docs: wrote ${files.size} files into ${process.argv[2] ?? "web/public"}`);
}
