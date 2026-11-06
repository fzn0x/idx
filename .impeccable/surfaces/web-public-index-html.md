---
version: 1
slug: "web-public-index-html"
primary_target: "web/public/index.html"
related_targets: ["web/public/terminal.html","web/public/en/index.html","web/public/en/terminal.html"]
---

# Surface: ihsg.midoriresearch.com home (web/public/index.html, en/index.html) and the web terminal (terminal.html, en/terminal.html)

Scope: Operate (the home page is where older investors read the market and find a stock); the terminal is an Operate surface for power users in the same world.
Audience: Indonesian retail investors, many over 50, who have read the market page of the morning paper for decades; they check IHSG and their stocks on a phone by day and on a laptop or tablet at home. Second: developers and AI-agent builders who arrive from GitHub.
Job: read today's IHSG, the session, the heavyweights and the movers at a glance, in Bahasa Indonesia; find a stock by code or company name; developers find the install line, MCP and the command table further down.
Action/task: search a stock (the result opens in the terminal), switch tabs between Naik, Turun and Teramai, switch language, open the terminal, copy an install line.
Proof/content: the live board from /api/market, the IHSG minute tape, breadth, the session timetable from clock.js, real CLI output and JSON, the tool list from public/tools.js, the live star count. Nothing invented: no testimonials, logos or benchmarks.
Constraints: Bahasa Indonesia at the plain paths and English under /en/ with a switch on every page; Indonesian number and time style (5.050,43; 16.14 WIB; M = miliar, T = triliun); light by day and dark when the device asks; body text at 18px or more; colour never the only carrier of a move (sign and triangle too); AA contrast; reduced motion honoured; CSP forbids inline scripts and only Google Fonts may serve fonts; no em dashes; the M3 Expressive mark of 2026-11-06 and idx mint kept; the notes, the three-faces output and the full command table kept; not affiliated with PT Bursa Efek Indonesia, said plainly.

## Direction contract

THESIS: The market read the way older Indonesian investors have always read it: the morning paper's market page, made live, large and calm. IHSG leads like the paper's headline figure, the closing table and the movers follow, and every move is printed with its sign, its triangle and its colour. It refuses the dark trading dashboard and the white fintech card grid.

OWN-WORLD: Day: cool newsprint grey #f3f4f1 with white sheets #ffffff, ink #15191e, secondary #454e58, hairlines #cfd4cd, a navy masthead and navy section bands #123a64 with white type, gains #11743a, losses #b42318, late #8a5300, fresh-ink highlighter #ffe680. Night: blue charcoal #0f151c, sheets #151d26, ink #e9edf2, navy #173d66, gains in idx mint #4ade80, losses #ff8f85. Libre Franklin 800 for headings; Atkinson Hyperlegible Next for text and every figure, tabular. 4px corners, a 4px ink rule over the lead, 2px ink rules under table heads.

STORY: A reader lands, sees today's date and whether the exchange is open, reads IHSG and its move, glances at breadth and the session chart, scans the heavyweights by name, flips to the day's gainers, then types a company name into the search and opens it. A developer scrolls on to the install line, the three faces and the command table.

FIRST VIEWPORT: At 1440x900: a 68px navy masthead (mark, idx and Midori Research, Pasar / Terminal / Panduan, ID-EN switch, Star with count); the page head with "Pasar Saham Indonesia", the date line and the session pill; a 58px search with its Cari button; the IHSG box across the width, five twelfths figure (close at 3.6rem, the move in a tinted chip, breadth to scale, the session and clock) beside seven twelfths of session chart, with the freshness line under it; the top of Saham Unggulan below.

FORM: Grounded candidate 4 of my ordered list (the Indonesian morning paper's stock market page), seed key bc07890b, mode operate, assigned by the roll and chosen by the owner. Raises: one reading size at a fixed measure (acetate tab manual); ruled modules without whitespace inflation (Japanese high-density web); every bar to scale (Labanotation); one plot owns the field, the IHSG chart (catalog sleeve); every stock carries its name plate, code over full name (character catalog); one tonal ramp shared by day and night (exposure record). Signature move: fresh ink, a figure that changed in the minute refresh is marked with the highlighter and fades back to paper over two seconds; under reduced motion the value only changes.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

## Unresolved
- The timetable and countdown follow IDX's regular-market hours (Decree Kep-00158/BEI/10-2026, Rule II-A); web/public/clock.js holds them.
- Ownership data on the web: only what the register mirror syncs; the terminal states the limitation.
- The terminal's command results keep the CLI's English column names on both languages; its shell, help and states are translated.
- The docs pages stay in English (generated from README and docs/); their shell follows this world.
