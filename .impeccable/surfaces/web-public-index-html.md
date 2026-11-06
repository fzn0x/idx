---
version: 1
slug: "web-public-index-html"
primary_target: "web/public/index.html"
related_targets: ["web/public/terminal.html"]
---

# Surface: ihsg.midoriresearch.com home monitor (web/public/index.html) and the web terminal (web/public/terminal.html)

Scope: Persuade (the home page, now a live market monitor) with the terminal as an Operate surface in the same world.
Audience: IDX retail investors and analysts arriving from GitHub or a shared link; AI-agent builders evaluating the MCP server.
Job: see idx prove itself on a working market screen, understand in seconds that it is one free binary with three faces, then type a command, install, or star the repo.
Proof/content: the live board from /api/market (minute-refreshed), real CLI output and JSON, the real tool list from public/tools.js, the live GitHub star count through the Worker. No testimonials, logos or benchmarks exist and none are invented.
Constraints: English UI with the Indonesian section names kept (Papan Harga, Saham Teraktif, Keterangan, Catatan, Ikhtisar, Daftar Perintah); brand mark and idx green kept; the notes, the three-faces output and the full command table kept; keyboard-first with the mouse fully working; no em dashes anywhere in the copy; disclaimers one sentence; copy readable without JS; AA contrast; reduced motion honoured; CSP forbids inline scripts and only Google Fonts may serve fonts.

## Direction contract

THESIS: The IDX session read like a Jakarta KRL departure board run as an institutional terminal: fixed rows that flip in place, a station clock and timetable for the session, a command line at the gate. The product proves itself live before it explains itself. It refuses the dark developer-tool landing (headline and install beside a terminal screenshot, feature cards) and the neon-on-black terminal costume.

OWN-WORLD: Black glass ground #0b0f14 (the mark's own tile), flush panels split by 1px rules #1b242d; no cards, shadows, radius or glow. Lit text #e6edf3, dim #8b98a5, ghost #17202a. idx mint #4ade80 is the one live circuit (focus, caret, live state) and gains; rose #f43f5e losses and errors; amber #e3b341 only for delayed or stale. Doto dot-matrix only for the band's big figures; Martian Mono for everything else, condensed for tables, tabular figures. Panels open with a numbered Indonesian title, an English subtitle and an age stamp, and close with a machine line.

STORY: A visitor lands on a working market screen, watches IHSG, the session clock and the heavyweights board update in place, reads in Keterangan that idx gives one answer as table, JSON and MCP, then types a command or ticker at the gate, copies the install line, or stars the repo.

FIRST VIEWPORT: At 1440x900: a 40px top bar (mark, idx wordmark, Monitor / Terminal / Docs, Star button with live count at right); a full-width command line with typeahead; an LED band with IHSG in Doto near 56px, change, the WIB clock, session state and countdown, breadth, the index tape along its foot; then 01 PAPAN HARGA at two thirds width, about 16 rows, beside a right column stacking 02 SAHAM TERAKTIF (tabs) over 03 KETERANGAN (three lines, install and MCP lines with copy, open terminal); a sticky 28px status bar with live state, snapshot age, source and key hints.

FORM: Grounded candidate 6 of my ordered list (KRL and MRT Jakarta dot-matrix departure boards), seed key e925621a, mode persuade, fused with the user-pinned Bloomberg, TradingView and TUI brief. Raises: addressable panels and results, Back retraces the trail (HyperCard); every panel states its age (brine calendar); one live circuit in idx green (neon circuit); a machine line closing each panel (passport); one region owns the screen (zoo map). Signature interaction: the board flips; each minute changed cells swap instantly through a one-frame dot invert, never a tween, while the station clock ticks; under reduced motion values swap with no flash. Terminal: the same shell, the gate on top, each result a numbered panel with its machine line, newest under the gate, a rail of commands and history on wide screens.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

## Unresolved
- Settled 2026-11-06: the timetable and countdown follow IDX's regular-market hours from its "Trading Hours and Mechanism" page (Decree Kep-00158/BEI/10-2026, Rule II-A), checked against the live page; web/public/clock.js holds them.
- Ownership data on the web: only what the register mirror syncs; the terminal states the limitation.
- Screenshots for the README come from this build once reviewed.
