---
name: idx web surface
description: Halaman Bursa. The stock market page of an Indonesian morning newspaper, made live, large and calm, for readers who have followed the market for decades.
colors:
  paper: "#f3f4f1"
  sheet: "#ffffff"
  band: "#e8ebe6"
  rule: "#cfd4cd"
  edge: "#7b847c"
  ink: "#15191e"
  ink-2: "#454e58"
  ink-3: "#5a636e"
  navy: "#123a64"
  navy-2: "#0b2a4b"
  navy-soft: "#e1e8f1"
  on-navy: "#ffffff"
  on-navy-2: "#c9d6e6"
  on-navy-late: "#ffd48a"
  on-navy-down: "#ffb4ab"
  on-navy-focus: "#ffc94d"
  link: "#0d4c8c"
  up: "#11743a"
  up-bg: "#e2f1e7"
  down: "#b42318"
  down-bg: "#fbe6e3"
  flat: "#454e58"
  late: "#8a5300"
  late-bg: "#fbefd6"
  hl: "#ffe680"
  focus: "#1b5fd1"
  paper-night: "#0f151c"
  sheet-night: "#151d26"
  band-night: "#1c2631"
  rule-night: "#2b3643"
  edge-night: "#6c7a8a"
  ink-night: "#e9edf2"
  ink-2-night: "#b4bec9"
  ink-3-night: "#98a4b1"
  navy-night: "#173d66"
  navy-2-night: "#0f2d4f"
  navy-soft-night: "#1d3554"
  link-night: "#8ec2ff"
  up-night: "#4ade80"
  up-bg-night: "#12301f"
  down-night: "#ff8f85"
  down-bg-night: "#3b1b1a"
  flat-night: "#b4bec9"
  late-night: "#f2bd52"
  late-bg-night: "#3a2c0f"
  hl-night: "#5c4c0e"
  focus-night: "#ffc94d"
typography:
  wordmark:
    fontFamily: "Libre Franklin, Atkinson Hyperlegible Next, system-ui, sans-serif"
    fontSize: "1.5rem"
    fontWeight: 800
    lineHeight: 1.1
    letterSpacing: "-0.02em"
  page-title:
    fontFamily: "Libre Franklin, Atkinson Hyperlegible Next, system-ui, sans-serif"
    fontSize: "2.1rem"
    fontWeight: 800
    lineHeight: 1.2
    letterSpacing: "-0.01em"
  section-title:
    fontFamily: "Libre Franklin, Atkinson Hyperlegible Next, system-ui, sans-serif"
    fontSize: "1.2rem"
    fontWeight: 800
    lineHeight: 1.2
    letterSpacing: "-0.01em"
  panel-title:
    fontFamily: "Libre Franklin, Atkinson Hyperlegible Next, system-ui, sans-serif"
    fontSize: "1.05rem"
    fontWeight: 800
    lineHeight: 1.2
    letterSpacing: "-0.01em"
  lead-figure:
    fontFamily: "Atkinson Hyperlegible Next, Atkinson Hyperlegible, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "3.6rem"
    fontWeight: 800
    lineHeight: 1
    letterSpacing: "-0.02em"
    fontFeature: "'tnum'"
  lead-change:
    fontFamily: "Atkinson Hyperlegible Next, Atkinson Hyperlegible, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "1.3rem"
    fontWeight: 700
    fontFeature: "'tnum'"
  body:
    fontFamily: "Atkinson Hyperlegible Next, Atkinson Hyperlegible, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "1rem"
    fontWeight: 400
    lineHeight: 1.55
    fontFeature: "'tnum'"
  table:
    fontFamily: "Atkinson Hyperlegible Next, Atkinson Hyperlegible, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "0.95rem"
    fontWeight: 400
    lineHeight: 1.35
    fontFeature: "'tnum'"
  plate-code:
    fontFamily: "Atkinson Hyperlegible Next, Atkinson Hyperlegible, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "1.05rem"
    fontWeight: 800
    letterSpacing: "0.02em"
  label:
    fontFamily: "Atkinson Hyperlegible Next, Atkinson Hyperlegible, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "0.85rem"
    fontWeight: 700
  mono:
    fontFamily: "ui-monospace, Cascadia Mono, SF Mono, Menlo, Consolas, monospace"
    fontSize: "0.85rem"
    fontWeight: 400
    lineHeight: 1.5
rounded:
  sm: "3px"
  md: "4px"
  pill: "999px"
spacing:
  root: "18px"
  gutter: "24px"
  gutter-phone: "16px"
  max-width: "1320px"
  masthead: "68px"
  control: "58px"
  gate-input: "54px"
  status-line: "44px"
  module-gap: "24px"
  band-pad: "11px 18px"
  cell: "10px 14px"
  cell-edge: "18px"
  board-row-y: "11px"
  measure: "62ch"
  docs-measure: "31em"
  docs-rail: "300px"
  terminal-rail: "320px"
components:
  button-primary:
    backgroundColor: "{colors.navy}"
    textColor: "{colors.on-navy}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "8px 26px"
    height: "{spacing.control}"
  button-primary-hover:
    backgroundColor: "{colors.navy-2}"
  button-ghost:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    padding: "8px 26px"
    height: "{spacing.control}"
  button-ghost-hover:
    backgroundColor: "{colors.band}"
  button-small:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.md}"
    padding: "0 14px"
    height: "36px"
  button-small-hover:
    backgroundColor: "{colors.band}"
  search-input:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    padding: "0 18px"
    height: "{spacing.control}"
  masthead:
    backgroundColor: "{colors.navy}"
    textColor: "{colors.on-navy}"
    height: "{spacing.masthead}"
  nav-link:
    backgroundColor: "transparent"
    textColor: "{colors.on-navy}"
    typography: "{typography.body}"
    padding: "0 14px"
  nav-link-hover:
    backgroundColor: "{colors.navy-2}"
  lang-current:
    backgroundColor: "{colors.on-navy}"
    textColor: "{colors.navy}"
    typography: "{typography.label}"
    padding: "5px 11px"
  section-band:
    backgroundColor: "{colors.navy}"
    textColor: "{colors.on-navy}"
    typography: "{typography.section-title}"
    padding: "{spacing.band-pad}"
  module:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
  tab:
    backgroundColor: "transparent"
    textColor: "{colors.ink-2}"
    typography: "{typography.body}"
    padding: "12px 10px"
  tab-selected:
    backgroundColor: "{colors.navy-soft}"
    textColor: "{colors.navy}"
  table-head:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink-2}"
    typography: "{typography.label}"
    padding: "{spacing.cell}"
  table-row-hover:
    backgroundColor: "{colors.band}"
  change-chip-up:
    backgroundColor: "{colors.up-bg}"
    textColor: "{colors.up}"
    typography: "{typography.lead-change}"
    rounded: "{rounded.md}"
    padding: "6px 12px"
  change-chip-down:
    backgroundColor: "{colors.down-bg}"
    textColor: "{colors.down}"
    typography: "{typography.lead-change}"
    rounded: "{rounded.md}"
    padding: "6px 12px"
  state-pill:
    backgroundColor: "{colors.band}"
    textColor: "{colors.ink}"
    rounded: "{rounded.pill}"
    padding: "8px 14px"
  state-pill-open:
    backgroundColor: "{colors.up-bg}"
    textColor: "{colors.up}"
  suggestion-selected:
    backgroundColor: "{colors.navy-soft}"
    textColor: "{colors.ink}"
  status-line:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink-2}"
    typography: "{typography.label}"
    height: "{spacing.status-line}"
  kbd:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    rounded: "{rounded.sm}"
    padding: "0.15em 0.45em"
---

# Design System: idx web surface

## Overview

**Creative North Star: "Halaman Bursa"**

The site is the stock market page of an Indonesian morning newspaper, made live, large and calm. IHSG leads like the paper's headline figure, the closing table and the movers follow, and every move is printed with its sign, its triangle and its colour together. It is written first for retail investors over fifty who have read that page for decades, and second for developers who arrive from GitHub and scroll on to the install line, the three faces and the command table.

The ground is cool newsprint grey; tables, the IHSG box and the article sheets print on a whiter sheet; a navy masthead and navy section bands with white type carry the structure, exactly as a printed market page carries its column heads. Type is large (an 18px root) and figures are tabular everywhere. Density comes from ruled modules, not from small text: hairlines between rows, a 2px ink rule under every table head, a 4px ink rule over the lead and over the signed footer. Day is the default; night follows the device and keeps the same structure on a blue charcoal ground, with gains in idx mint.

The home page, the web terminal, the 404 page, the docs and the share card all share one shell: masthead, page content in navy-banded modules on white sheets, and the signed footer. The world refuses the dark trading dashboard and the white fintech card grid.

**Key Characteristics:**
- Cool newsprint paper, white sheets, navy masthead and navy section bands with white type.
- Libre Franklin 800 for heads; Atkinson Hyperlegible Next for text and every figure, tabular.
- An 18px root, 58px primary controls, AA contrast on every pairing, a 3px focus ring.
- Every move carries sign, triangle and colour; colour is never the only carrier.
- Every stock carries its name plate: the code over the company's name.
- Signature: fresh ink, a changed figure takes the highlighter and fades back to paper over 2.2s; nothing moves under reduced motion.
- Bahasa Indonesia at the plain paths, English under /en/, Indonesian number and time style (5.050,43; 16.14 WIB).

## Colors

A printed palette: newsprint, white sheet, ink in three strengths, one navy for structure and three signal colours whose meanings never overlap, each with a pale tint for chips. Every day token has a night twin (the `-night` keys) applied under `prefers-color-scheme: dark`; `on-navy` and `on-navy-2` hold in both modes.

### Primary
- **Masthead Navy** (navy): the masthead, every section band (`sec-h`), terminal result heads and rail heads, docs `##` bands, the primary button, a pressed chart range, the 404 code by day, the selected tab's label and underline by day. Pressed or hovered on navy goes to **Deep Navy** (navy-2). White **On Navy** (on-navy) and **On Navy Secondary** (on-navy-2, 8.1:1) carry all text on navy.
- **Selection Navy** (navy-soft): the selected tab, the selected search suggestion, the current docs page, a docs note (blockquote).

### Secondary
- **Gain Green** (up, 5.6:1 on the sheet; idx mint up-night at night): gains, rising chart lines and areas, advancing breadth, the open-market state and the live lamp. Its tint (up-bg) fills the IHSG change chip and the open state pill.
- **Loss Red** (down, 6.3:1; down-night at night): losses, falling chart lines, declining breadth, errors and the offline state. Its tint (down-bg) fills a falling change chip.
- **Flat** (flat): an unchanged move, printed with the diamond.

### Tertiary
- **Late Amber** (late, 6.2:1; late-night at night): delayed or stale only, the stale lamp and state word, a request that runs slow. On navy the stale age and stale result meta use on-navy-late; result errors on navy use on-navy-down.
- **Fresh Ink** (hl; hl-night at night): the highlighter. Marks a figure that changed in the minute refresh, the matched letters in a search suggestion and text selection. Never a static fill.

### Neutral
- **Newsprint** (paper): the page ground, the sticky terminal gate bar and the footer. Cool grey, never cream.
- **Sheet** (sheet): modules, tables, the IHSG box, the docs article and rail, inputs, the status line, the keys dialog.
- **Band** (band): hovered rows and buttons, the breadth track, code blocks and inline code in the docs, terminal table heads, the command-line label strip, volume bars.
- **Hairline** (rule): 1px rules between rows, modules, faces and notes; module and sheet outlines.
- **Control Edge** (edge, 3.6:1 on the sheet): the 2px outline of inputs and every outlined button, kbd outlines, chart axes, the unchanged share of the breadth bar.
- **Ink** (ink, 17:1): text and figures, the 4px rule over the lead and the footer, the 2px rule under table heads.
- **Ink Secondary** (ink-2, 7.9:1): sublines, column heads, company names in plates, help text.
- **Ink Tertiary** (ink-3, 5.9:1): placeholders, ranks, footnotes, the idle lamp.
- **Link Blue** (link): links and the ticker codes in name plates; the selected tab's underline at night and the 404 code at night.
- **Focus Blue** (focus, 5.4:1 on the paper; focus-night amber at night): the 3px focus ring. On navy the ring is always on-navy-focus.

### Named Rules
**The Sign Triangle Colour Rule.** A move is printed with its sign (+ or -), its triangle (▲ up, ▼ down, ◆ level) and its colour together. Colour alone never carries a move.

**The Amber Means Late Rule.** Amber appears only when something is delayed or stale. It is not a warning, a highlight or a second accent.

**The Highlighter Is Fresh Ink Rule.** The yellow highlighter marks what just changed or what matched; it fades back to paper and is never a permanent fill or a brand colour.

## Typography

**Display Font:** Libre Franklin 800 (with Atkinson Hyperlegible Next, system-ui)
**Body Font:** Atkinson Hyperlegible Next 400, 700, 800 (with Atkinson Hyperlegible, system-ui, -apple-system, Segoe UI, Roboto)
**Label/Mono Font:** the system monospace stack (ui-monospace, Cascadia Mono, SF Mono, Menlo, Consolas) for commands, code and the terminal's input and result titles

**Character:** a newspaper's heavy grotesque for heads over a face drawn for low-vision readers, which also sets every figure, so a price reads as clearly as a word. Both load from Google Fonts only (Atkinson Hyperlegible Next 400, 700, 800; Libre Franklin 800), as the CSP requires; the mono face is the reader's own.

The root is 112.5% (18px) at every width, phones included, so body text never reads below 18px. Body text never drops below 0.85rem (about 15px); figures are tabular on the body (`font-variant-numeric: tabular-nums`).

### Hierarchy
- **Wordmark** (Libre Franklin 800, 1.5rem, -0.02em): the `idx` beside the mark in the masthead, over "oleh Midori Research" at 0.75rem 600 in on-navy-2.
- **Page title** (Libre Franklin 800, 2.1rem, line-height 1.2): "Pasar Saham Indonesia", the docs page title; 1.7rem on phones. The 404 code is Libre Franklin 800 at 4.5rem.
- **Section title** (Libre Franklin 800, 1.2rem): the name in a navy band, with a subline in 0.9rem on-navy-2 and an age stamp pushed right. Docs `##` bands set 1.35rem.
- **Panel title** (Libre Franklin 800, 1.05rem): terminal rail and dialog heads. Terminal result heads set the command line in the mono face at 1rem 700.
- **Lead figure** (Atkinson Hyperlegible Next 800, 3.6rem, line-height 1, tabular): the IHSG close; 3rem on phones. The change beside it is 1.3rem 700 in a tinted chip; the session name 1.35rem 800.
- **Body** (Atkinson Hyperlegible Next 400, 1rem, line-height 1.55): running text at a 62ch measure (docs 31em, about 66 characters, line-height 1.6).
- **Table** (Atkinson Hyperlegible Next, 0.95rem, line-height 1.35): every table; heads 0.85rem 700 in ink-2; terminal tables 0.9rem.
- **Plate code** (Atkinson Hyperlegible Next 800, 1.05rem, 0.02em): a ticker code over its company name (0.85rem ink-2).
- **Label** (Atkinson Hyperlegible Next 700, 0.85rem): small labels, buttons in strips, meta and age stamps. Sentence case, never tracked caps.
- **Mono** (0.85rem, line-height 1.5): install lines, CLI and JSON output, command cells.

### Named Rules
**The Reading Size Rule.** One reading size (18px) at a fixed measure. Hierarchy steps up through Libre Franklin heads and the lead figure, never down into small grey text.

**The Figures Are Text Rule.** Every figure, from the IHSG close to a table cell, is set in Atkinson Hyperlegible Next with tabular numerals. No display face and no mono face for market numbers.

## Layout

A centred page of 1320px maximum width with a 24px gutter (16px under 48rem). On the home page, in order: the 68px navy masthead; the page head (title, the date line, the session state pill); the search with its label, a 58px field and the Cari button; the IHSG lead module across the width, five twelfths figure (close, change chip, as-of line, breadth, session and clock) beside seven twelfths of minute chart; the freshness line; then two columns at 8fr and 5fr with a 24px gap (Saham Unggulan beside Pergerakan Saham), then Tentang idx beside Pasang di komputer, then full-width Contoh Keluaran, Catatan and Daftar Perintah, and the signed footer.

The terminal puts a sticky gate bar (on paper, rule beneath) under the masthead, results in the left column beside a 320px sticky rail of commands and history, and a fixed 44px status line at the foot. The docs set a 300px sticky page list beside the article sheet. The 404 page is one module, 760px wide.

Modules pad 16px to 22px; navy bands 11px by 18px; table cells 10px by 14px with 18px at the outer edges; board rows 11px. Modules sit 24px apart.

Breakpoints: under 72rem the columns, the lead, the three faces and the terminal stack to one column (the lead figure splits into two columns over the chart) and the status line drops its source; under 48rem the gutter and root step down, the nav wraps into a scrolling row under the brand with 48px links, the search button goes full width, the board keeps only name plate, price and percent (with the triangle beside the percent), the command and help tables stack each row, and the status line keeps the lamp and the age; under 24rem the brand drops its subline. The docs collapse their page list into a wrapped row under 64rem.

**The Ruled Module Rule.** Content lives in modules: a white sheet with a 1px hairline outline, opened by a navy band. Rows inside are split by hairlines, never by gaps or alternating fills.

**The One Plot Rule.** One chart owns the home page, the IHSG minute chart in the lead. Everything else is a table or a line of text.

## Elevation & Depth

Flat and printed. Depth comes from the paper and sheet step, the 1px hairline outline and the ink rules, not from shadows. Two functional shadows exist and nowhere else: the search suggestion list floats over the page, and a docs table wider than its sheet shows a soft edge shadow that says it scrolls. The keys dialog sits on a flat backdrop.

### Shadow Vocabulary
- **Suggestion float** (`box-shadow: 0 10px 28px rgb(0 0 0 / 0.16)`): the search suggestion list only.
- **Scroll edge** (radial ink at 26%, 14px wide, fixed while sheet-coloured covers scroll): a docs table that overflows.

### Named Rules
**The Print Rule.** Nothing lifts off the page. A module is a sheet with a rule, not a card with a shadow; only a floating list may cast one.

## Shapes

Gently squared corners (4px) on modules, inputs, buttons, chips, the language switch and the Star control; 3px on key caps, the breadth bar and inline code. The state pill and the terminal's verdict pills are full pills (999px); lamps are 10px to 11px circles. Navy bands take the top corners of their module. Rules carry the structure: a 4px ink rule over the lead and over the footer, 2px ink under table heads (1px in terminal results), 1px hairlines everywhere else. Outlined controls use a 2px edge. The current nav link and the selected tab carry a 4px underline. Icons are thin stroked SVG (the star, the arrow) at 1.6px to 2px.

The brand mark is a fixed identity asset in Material 3 Expressive form: a nine-lobed cookie in idx mint holding three ink candles that rise left to right. It keeps its own shape and is never imitated by interface elements.

## Components

### Buttons
Plain, large, standard.
- **Shape:** gently squared (4px).
- **Primary (Cari, Jalankan, Buka terminal):** navy fill, white 1.1rem 700 label, 58px tall (54px in the terminal gate), 8px by 26px; a stroked arrow may follow the label.
- **Hover / Focus:** hover deepens to navy-2; focus is the 3px focus ring at 2px offset. No transitions.
- **Ghost:** transparent with a 2px control edge and ink label at the same height; hover fills with band.
- **Small (Salin, Jeda pembaruan, Pintasan, hints, chart ranges):** sheet fill, 2px control edge, 0.85rem to 0.9rem 700, 32px to 36px tall; hover fills with band; a pressed chart range turns navy. Hints for CLI-only commands use a dashed edge and ink-2.

### Inputs / Fields
- **Search:** a labelled 58px field on the sheet, 2px control edge, 4px corners, 1.15rem text, ink-3 placeholder; the Cari button sits beside it (below it on phones). Help text under it in ink-2.
- **Focus:** the edge turns focus blue with a 3px ring of focus at 30%.
- **Suggestions:** a sheet list under the field with a 2px edge and the one suggestion shadow; each row prints the kind, the code in 700 with the matched letters on the highlighter, and the name in ink-2; hover fills with band, the selected row with navy-soft.
- **Terminal gate:** the same field at 54px in the mono face, with a "Perintah" label in Libre Franklin and a key hint.

### Navigation
- **Masthead:** navy, 68px; mark and wordmark, Pasar / Terminal / Panduan links in white 1rem 700 at full bar height with a 4px white underline on the current page and navy-2 on hover; then the language switch and the Star control pushed right. Focus on navy uses on-navy-focus.
- **Language switch:** an ID / EN pair outlined in on-navy-2; the current language is a white cell with navy text. The docs, which exist only in English, show a single link to the Indonesian site.
- **Star:** outlined in on-navy-2, a star icon and label, then the live count in its own cell; hover inverts to white with navy text.
- **Tabs (Naik, Turun, Teramai):** equal-width 1rem 700 labels in ink-2 on a hairline; the selected tab takes navy-soft, navy text and a 4px navy underline (at night ink text and a link-blue underline).

### Modules and section bands
The unit of the world. A sheet with a 1px hairline outline and 4px corners, opened by a navy band (`sec-h`) holding the section title, a subline and an age stamp ("Diperbarui" with the value in white, on-navy-late when stale). A module may close with a foot line in ink-3 above a hairline.

### The lead (IHSG)
The headline figure. A module with a 4px ink rule on top, split five twelfths to seven twelfths by a hairline. Left: "IHSG" over its full name, the close at 3.6rem 800, the change in a tinted chip with sign and triangle, the as-of time, the breadth (a 14px bar of gain, loss and unchanged to scale, with counts and triangles), and the session with the WIB clock. Right: the minute chart, a 2.5px line in gain or loss with a 14% area, a dashed ink-3 previous-close reference, hairline grid, 15px labels haloed in sheet, and the last value in 800. A freshness line under the module carries the live lamp, the age, the source and the pause button.

### Tables and name plates
Full width on the sheet. Figures right-aligned, the first column left; heads in ink-2 0.85rem 700 over a 2px ink rule; rows split by hairlines, the last row open; hover fills with band. Every stock is a name plate: the code in link blue at 1.05rem 800 over the company's short name (PT and Tbk removed) in ink-2. Changes print sign, triangle and colour in 700. A table that overflows fades its right 2.5rem and shows a scroll cue.

### Fresh ink (signature)
Each minute refresh patches the board, the movers and the lead cell by cell. A cell whose value changed takes the highlighter (hl) and fades back to transparent over 2.2s (`cubic-bezier(0.2, 0.7, 0.2, 1)`). Under `prefers-reduced-motion: reduce` the value only changes. The pause control stops the refresh.

### State pill and lamps
The session state is a full pill on band with an 11px ink-3 dot; when the exchange is open it turns up-bg with a gain-green dot and label. Live lamps (10px circles) are gain green for live, amber for stale, red for offline or error, an outlined ring for paused, ink-3 for idle.

### Copy blocks and output
An install line is a bordered block: a band strip with its label and a Salin button over the command in mono on the sheet. Contoh Keluaran prints the same answer as table, JSON and agent tool in three faces split by hairlines.

### Key caps
Sheet fill, 1px control edge, 3px corners, 0.8em 600. Hidden on coarse pointers.

### Terminal
The gate bar under the masthead, results stacked newest first, each a module with a navy head (a two-digit sequence number and the command line in mono, meta pushed right, on-navy-late when stale, on-navy-down on error), a body of tables, key-value lists, prose or the quote chart, and a source line in ink-3 above a hairline. The rail lists commands and history on a sheet with navy heads. The fixed 44px status line on the sheet carries the state lamp and word, the snapshot age, the source, the Pintasan button and key caps. The keys dialog is a sheet with a 2px edge on a flat backdrop.

### Not found
One 760px module: a navy band with the title, the 404 code in Libre Franklin 800 at 4.5rem in navy (link blue at night), a sentence, a primary button back to the market page and ghost buttons to the terminal and the docs.

### Docs
The same masthead and footer over a 300px sticky page list (navy head, the current page on navy-soft in 700, its sections indented in ink-2) beside the article sheet. The title is a page title with the README tagline as a 1.2rem standfirst; every `##` opens a full-width navy band; code sits on band; notes on navy-soft; tables are the home page's tables, left-aligned.

## Do's and Don'ts

### Do:
- **Do** print every move with its sign, its triangle (▲ ▼ ◆) and its colour together.
- **Do** open every module with a navy band and keep rows on the white sheet split by 1px hairlines; put the 4px ink rule over the lead and the footer and a 2px ink rule under table heads.
- **Do** set every figure in Atkinson Hyperlegible Next with tabular numerals, and every head in Libre Franklin 800.
- **Do** keep the 18px root, body text at 0.85rem or more, and primary controls at 58px.
- **Do** print numbers and times in the page's language: 5.050,43, T, M, jt, rb and 16.14 WIB in Indonesian; 5,050.43, T, B, M, K and 16:14 WIB in English; "t/a" or "n/a" where the source has no value.
- **Do** carry every stock as a name plate, the code over the company's name.
- **Do** mark a changed figure with fresh ink fading over 2.2s, and only change the value under reduced motion.
- **Do** keep amber for delayed or stale, the highlighter for what just changed or matched, and the 3px focus ring on everything focusable.
- **Do** keep controls standard: real buttons, links that look like links, labelled fields.

### Don't:
- **Don't** use side stripes (a coloured bar on one edge of a row, card or note) to mark state.
- **Don't** add decorative numerals, kickers, eyebrows or tracked uppercase labels.
- **Don't** use glass, blur, glows, gradients on surfaces or shadows beyond the suggestion list and the docs scroll edge.
- **Don't** build the dark trading dashboard or the white fintech card grid.
- **Don't** let colour alone carry a move, a state or an error.
- **Don't** write em dashes in copy.
- **Don't** load fonts from anywhere but Google Fonts.
