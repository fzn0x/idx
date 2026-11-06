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
// Each page in Bahasa Indonesia at its plain path and in English under /en/.
const PAGES = {
  "index.html": `${ORIGIN}/`, "en/index.html": `${ORIGIN}/en/`,
  "terminal.html": `${ORIGIN}/terminal`, "en/terminal.html": `${ORIGIN}/en/terminal`,
};
const PAIRS = [["index.html", "en/index.html"], ["terminal.html", "en/terminal.html"]];
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

const alternates = (html) => Object.fromEntries([...html.matchAll(/<link rel="alternate" hreflang="([a-z-]+)" href="([^"]+)">/g)].map((m) => [m[1], m[2]]));

test("every page names its language, and each language pair links both ways", () => {
  for (const [id, en] of PAIRS) {
    const [hi, he] = [read(id), read(en)];
    assert.match(hi, /^<!doctype html>\r?\n<html lang="id">/, id);
    assert.match(he, /^<!doctype html>\r?\n<html lang="en">/, en);
    assert.equal(meta(hi, "og:locale"), "id_ID", id);
    assert.equal(meta(he, "og:locale"), "en_US", en);
    const want = { id: PAGES[id], en: PAGES[en], "x-default": PAGES[id] };
    assert.deepEqual(alternates(hi), want, `${id} hreflang`);
    assert.deepEqual(alternates(he), want, `${en} hreflang`);
    // The language switch on each page points at the other one.
    assert.ok(hi.includes(`href="${new URL(PAGES[en]).pathname}" hreflang="en"`), `${id} links to ${en}`);
    assert.ok(he.includes(`href="${new URL(PAGES[id]).pathname}" hreflang="id"`), `${en} links to ${id}`);
  }
});

test("titles name the publisher and stay within what search results show", () => {
  for (const page of Object.keys(PAGES)) {
    const title = read(page).match(/<title>([^<]+)<\/title>/)?.[1];
    assert.ok(title, `${page} title`);
    assert.match(title, /Midori Research$/, `${page} title ends with the publisher`);
    assert.ok(title.length <= 70, `${page} title is ${title.length} characters`);
    const d = meta(read(page), "description");
    assert.ok(d.length >= 70 && d.length <= 170, `${page} description is ${d.length} characters`);
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

test("the 404 pages stay out of the index", () => {
  assert.equal(meta(read("404.html"), "robots"), "noindex");
  assert.equal(meta(read("en/404.html"), "robots"), "noindex");
});

test("the sitemap pairs each page with its other language", () => {
  const xml = read("sitemap.xml");
  for (const [id, en] of PAIRS) for (const page of [id, en]) {
    const at = xml.indexOf(`<loc>${PAGES[page]}</loc>`);
    assert.ok(at > 0, `${page} in the sitemap`);
    const block = xml.slice(at, xml.indexOf("</url>", at));
    assert.ok(block.includes(`hreflang="id" href="${PAGES[id]}"`) && block.includes(`hreflang="en" href="${PAGES[en]}"`), `${page} alternates`);
  }
});
