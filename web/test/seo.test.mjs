// node --test web/test/*.mjs (no network). Share-card and search metadata:
// the tags must agree with the files they point at and with each other.
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { PAGES as DOCS, htmlPath } from "../scripts/docs.mjs";

const pub = join(dirname(fileURLToPath(import.meta.url)), "..", "public");
const read = (p) => readFileSync(join(pub, p), "utf8");
const ORIGIN = "https://ihsg.midoriresearch.com";
const PAGES = { "index.html": `${ORIGIN}/`, "terminal.html": `${ORIGIN}/terminal` };
// The docs pages are generated at deploy time; docs.test.mjs checks their tags.
const DOC_URLS = DOCS.map((p) => `${ORIGIN}${htmlPath(p.slug)}`);

function meta(html, key) {
  const m = html.match(new RegExp(`<meta (?:property|name)="${key}" content="([^"]*)">`));
  return m?.[1];
}

function pngSize(p) {
  const b = readFileSync(join(pub, p));
  assert.equal(b.toString("latin1", 1, 4), "PNG", `${p} is not a PNG`);
  return [b.readUInt32BE(16), b.readUInt32BE(20)];
}

test("every indexable page carries a canonical URL and a large share card", () => {
  for (const [page, url] of Object.entries(PAGES)) {
    const html = read(page);
    assert.ok(html.includes(`<link rel="canonical" href="${url}">`), `${page} canonical`);
    assert.equal(meta(html, "og:url"), url, `${page} og:url`);
    for (const k of ["description", "og:title", "og:description", "og:image:alt"]) assert.ok(meta(html, k), `${page} ${k}`);
    assert.equal(meta(html, "twitter:card"), "summary_large_image", `${page} twitter:card`);

    const image = meta(html, "og:image");
    assert.ok(image?.startsWith(`${ORIGIN}/`), `${page} og:image must be absolute`);
    const file = image.slice(ORIGIN.length + 1);
    assert.deepEqual(pngSize(file), [Number(meta(html, "og:image:width")), Number(meta(html, "og:image:height"))], `${page} og:image size`);

    const icon = html.match(/<link rel="apple-touch-icon" href="\/([^"]+)">/)?.[1];
    assert.deepEqual(pngSize(icon), [180, 180], `${page} apple-touch-icon`);
  }
});

test("sitemap lists exactly the canonical pages and robots.txt points at it", () => {
  const locs = [...read("sitemap.xml").matchAll(/<loc>([^<]+)<\/loc>/g)].map((m) => m[1]);
  assert.deepEqual(locs.sort(), [...Object.values(PAGES), ...DOC_URLS].sort(), "a docs page added to PAGES in web/scripts/docs.mjs goes into public/sitemap.xml too");
  const robots = read("robots.txt");
  assert.match(robots, new RegExp(`^Sitemap: ${ORIGIN}/sitemap\\.xml$`, "m"));
  assert.match(robots, /^Disallow: \/api\/$/m);
});

test("landing page JSON-LD parses and describes the app", () => {
  const block = read("index.html").match(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/)?.[1];
  const graph = JSON.parse(block)["@graph"];
  const app = graph.find((n) => n["@type"] === "SoftwareApplication");
  assert.equal(app.url, `${ORIGIN}/`);
  assert.ok(existsSync(join(pub, new URL(app.image).pathname)), "JSON-LD image exists");
  for (const n of graph) for (const ref of [n.publisher?.["@id"]].filter(Boolean)) {
    assert.ok(graph.some((m) => m["@id"] === ref), `unresolved @id ${ref}`);
  }
});

test("the 404 page stays out of the index", () => {
  assert.equal(meta(read("404.html"), "robots"), "noindex");
});
