---
name: idx web surface
description: Papan Stasiun. The IDX session as a station departure board on black glass, flush panels split by 1px rules, one live circuit in idx mint.
colors:
  ground: "#0b0f14"
  rule: "#1b242d"
  ghost: "#17202a"
  raise: "#121a22"
  fill: "#34404c"
  edge: "#56636f"
  dim: "#8b98a5"
  lit: "#e6edf3"
  mint: "#4ade80"
  rose: "#f43f5e"
  amber: "#e3b341"
typography:
  led-index:
    fontFamily: "Doto, Martian Mono, ui-monospace, monospace"
    fontSize: "56px"
    fontWeight: 800
    lineHeight: 1
    letterSpacing: "0.04em"
    fontFeature: "'tnum'"
  led-clock:
    fontFamily: "Doto, Martian Mono, ui-monospace, monospace"
    fontSize: "28px"
    fontWeight: 800
    lineHeight: 1
    letterSpacing: "0.04em"
    fontFeature: "'tnum'"
  led-change:
    fontFamily: "Doto, Martian Mono, ui-monospace, monospace"
    fontSize: "22px"
    fontWeight: 800
    lineHeight: 1
    letterSpacing: "0.04em"
    fontFeature: "'tnum'"
  wordmark:
    fontFamily: "Martian Mono, ui-monospace, Cascadia Mono, Menlo, Consolas, monospace"
    fontSize: "16px"
    fontWeight: 700
    letterSpacing: "-0.03em"
  command:
    fontFamily: "Martian Mono, ui-monospace, Cascadia Mono, Menlo, Consolas, monospace"
    fontSize: "15px"
    fontWeight: 400
  body:
    fontFamily: "Martian Mono, ui-monospace, Cascadia Mono, Menlo, Consolas, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.55
    fontFeature: "'tnum'"
  panel-title:
    fontFamily: "Martian Mono, ui-monospace, Cascadia Mono, Menlo, Consolas, monospace"
    fontSize: "12px"
    fontWeight: 700
    letterSpacing: "0.14em"
  label:
    fontFamily: "Martian Mono, ui-monospace, Cascadia Mono, Menlo, Consolas, monospace"
    fontSize: "12px"
    fontWeight: 600
    letterSpacing: "0.1em"
    fontVariation: "'wdth' 87.5"
  data:
    fontFamily: "Martian Mono, ui-monospace, Cascadia Mono, Menlo, Consolas, monospace"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: "16px"
    fontFeature: "'tnum'"
    fontVariation: "'wdth' 75"
  machine-line:
    fontFamily: "Martian Mono, ui-monospace, Cascadia Mono, Menlo, Consolas, monospace"
    fontSize: "12px"
    fontWeight: 400
    letterSpacing: "0.16em"
    fontVariation: "'wdth' 75"
rounded:
  none: "0px"
spacing:
  gutter: "16px"
  gutter-narrow: "12px"
  top-bar: "40px"
  gate: "44px"
  status-bar: "28px"
  cell-x: "10px"
  row-y: "4px"
  board-row-y: "7px"
  panel-body: "12px 16px 14px"
  panel-head: "9px 16px 8px"
components:
  button-outline:
    backgroundColor: "transparent"
    textColor: "{colors.lit}"
    typography: "{typography.label}"
    rounded: "{rounded.none}"
    padding: "0 14px"
    height: "28px"
  button-outline-hover:
    backgroundColor: "{colors.lit}"
    textColor: "{colors.ground}"
  button-go:
    backgroundColor: "{colors.lit}"
    textColor: "{colors.ground}"
    typography: "{typography.panel-title}"
    rounded: "{rounded.none}"
    padding: "8px 12px"
  button-go-hover:
    backgroundColor: "{colors.ground}"
    textColor: "{colors.lit}"
  gate-input:
    backgroundColor: "transparent"
    textColor: "{colors.lit}"
    typography: "{typography.command}"
    rounded: "{rounded.none}"
    height: "{spacing.gate}"
  nav-link:
    backgroundColor: "transparent"
    textColor: "{colors.dim}"
    typography: "{typography.label}"
    padding: "0 12px"
    height: "{spacing.top-bar}"
  nav-link-current:
    textColor: "{colors.lit}"
  tab:
    backgroundColor: "transparent"
    textColor: "{colors.dim}"
    typography: "{typography.label}"
    padding: "6px 6px 5px"
  tab-selected:
    textColor: "{colors.lit}"
  panel-header:
    backgroundColor: "{colors.ground}"
    textColor: "{colors.lit}"
    typography: "{typography.panel-title}"
    padding: "{spacing.panel-head}"
  table-row-hover:
    backgroundColor: "{colors.raise}"
  suggestion-selected:
    backgroundColor: "{colors.lit}"
    textColor: "{colors.ground}"
  status-bar:
    backgroundColor: "{colors.ground}"
    textColor: "{colors.dim}"
    typography: "{typography.label}"
    height: "{spacing.status-bar}"
  key-cap:
    backgroundColor: "transparent"
    textColor: "{colors.lit}"
    padding: "0 4px"
    height: "16px"
---

# Design System: idx web surface

## Overview

**Creative North Star: "Papan Stasiun"**

The site is a Jakarta KRL departure board run as an institutional market terminal. One black glass ground, flush panels split by hairline rules, rows that hold still and flip in place when a value changes, a station clock and session timetable in a dot-matrix LED band, and a command line at the gate. The product proves itself live on a working market screen before it explains anything; explanation sits in the same panels, under the same rules, as the data.

Density is high and even. One working text size carries almost everything, and hierarchy comes from weight, caps tracking, the width axis, the numbered panel strips laid on a ghost-dot ground and the single step up into Doto in the band. Colour is near absent: the ground, five greys for structure and text, and three signal colours whose meanings never overlap. Mint is the live circuit and gains, rose is losses and errors, amber is lateness. Nothing glows, lifts, rounds or tweens.

The monitor (home), the web terminal and the 404 page share one shell: top bar, gate, content panels, fixed status bar. The terminal is the same board with the gate on top and each result printed as a numbered panel with its own machine line, newest first. The world rejects the dark developer-tool landing (headline and install line beside a terminal screenshot, feature cards) and the neon-on-black terminal costume.

**Key Characteristics:**
- Black glass ground with flush panels and 1px rules; no cards, shadows, radius or glow.
- One live circuit in idx mint; rose and amber each carry exactly one meaning.
- Doto dot-matrix for the LED band figures only; Martian Mono for everything else, condensed in tables, tabular figures throughout.
- Every panel is addressable: numbered Indonesian title, English subtitle, age stamp, closing machine line.
- Signature interaction: the board flip, a one-frame dot invert with no tween, and none at all under reduced motion.
- Keyboard first (`/`, number keys, `?`), with every control fully working by mouse and touch.

## Colors

A near-monochrome blue-black instrument palette in which colour appears only as signal.

### Primary
- **idx Mint** (mint): the one live circuit. Focus rings (2px outline, 2px offset), the gate's caret, the gate label and bottom rule while it has focus, the live lamp in the status bar and the session cell, the lamp beside a board row that moved, the typeahead's matched characters, text selection, the busy runner over a pending terminal result, and every gain (prices, changes, rising chart lines, advancing breadth dots). 11:1 on the ground.

### Secondary
- **Signal Rose** (rose): losses, falling chart lines, declining breadth, and the error and offline states of the status lamp and result meta. 5.3:1 on the ground and still AA on the hover tone.

### Tertiary
- **Late Amber** (amber): delayed or stale only. A stale age stamp, the stale status lamp, a stale result's meta, the terminal's clock once a request runs slow. 9.9:1 on the ground.

### Neutral
- **Black Glass** (ground): the page, every panel, the gate, the status bar and the dialog. Taken from the mark's own tile.
- **Hairline** (rule): the 1px splits between panels, rows, cells and bars.
- **Ghost Dot** (ghost): unlit LED dots on the band and the stacked panel strips, volume bars in charts.
- **Raised Row** (raise): the hover tone for table rows, rail entries and suggestions.
- **Filler** (fill): the `<` run that pads every machine line, scrollbar thumbs.
- **Control Edge** (edge): the 1px outline of buttons, key caps and the Star control, link underlines, chart axes. 3.1:1 on the ground.
- **Dim** (dim): subtitles, labels, column heads, secondary values and the placeholder. 6.6:1.
- **Lit** (lit): primary text and figures, the inverted fill of hovered, pressed and selected controls. 16:1.

### Named Rules
**The One Live Circuit Rule.** Mint means live, focused, connected or up. It is never decoration, never a background fill, never a brand wash across a panel.

**The Amber Means Late Rule.** Amber appears only when something is delayed or stale. It is not a warning colour, a highlight or a second accent.

**The Lit Inversion Rule.** Hover, pressed, selected and the primary call to action are shown by inverting to a lit fill with ground text. No tints, no coloured hover states.

## Typography

**Display Font:** Doto (with Martian Mono, ui-monospace fallback), dot-matrix, weight 800
**Body Font:** Martian Mono (with ui-monospace, Cascadia Mono, Menlo, Consolas)
**Label/Mono Font:** Martian Mono, its width axis set to 87.5% for labels and 75% for tables and machine lines

**Character:** A departure board's LED figures over a single engineered monospace. Doto is the lamp; Martian Mono is the timetable, and its width axis does the work a second family would do elsewhere. Both load from Google Fonts only (Doto ROND and weights 100 and 700 to 900; Martian Mono wdth 75 to 100, wght 300 to 700), as the CSP requires.

### Hierarchy
- **LED index** (Doto 800, 56px, line-height 1): the IHSG level in the band. Steps to 48px under 80rem and 44px under 46rem.
- **LED clock** (Doto 800, 28px): the WIB clock and the session countdown; 26px then 24px on smaller screens.
- **LED change** (Doto 800, 22px): the index change beside the level; 20px on phones. The 404 status figure uses Doto at 72px.
- **Wordmark** (Martian Mono 700, 16px, -0.03em): the `idx` beside the mark, nowhere else.
- **Command** (Martian Mono 400, 15px): the gate input; 16px on phones so iOS does not zoom.
- **Body** (Martian Mono 400, 13px, line-height 1.55): running text, notes (max 78ch), terminal prose (max 84ch), terminal result titles in sentence case.
- **Panel title** (Martian Mono 700, 12px, 0.14em, uppercase): the Indonesian panel names.
- **Label** (Martian Mono 600, 12px, 0.1em to 0.12em, uppercase, 87.5% width): nav, tabs, buttons, column heads, band cell labels, status words.
- **Data** (Martian Mono 400, 12px, line-height 16px, 75% width, tabular): every table. Symbols set in 700 with 0.04em.
- **Machine line** (Martian Mono 400, 12px, 0.16em, uppercase, 75% width): the passport string that closes each panel.

### Named Rules
**The Single Size Rule.** 12px is the working size for labels, data, titles and machine lines. Hierarchy comes from weight, caps tracking, the width axis, the numbered panel strips and the Doto step, never from a ladder of Martian Mono sizes. Only running text (13px), the command input (15px) and the wordmark (16px) sit outside it.

**The Dot Matrix Is For Figures Rule.** Doto sets the band's big figures (index, change, clock, countdown) and the 404 status code. Never body text, labels, headings or buttons.

**The Tabular Rule.** Figures are tabular everywhere (`font-variant-numeric: tabular-nums` on the body), so a flipping value never shifts its column.

## Layout

A full-bleed shell of stacked horizontal bands, with no max-width container on the monitor and terminal:

1. **Top bar** (40px): mark and wordmark, Monitor / Terminal / Docs nav, the Star control with a live count pushed right.
2. **Gate** (44px input row): `PERINTAH` label, the command input with typeahead, Run, a key hint. Sticky to the top on the terminal.
3. **LED band**: a grid of 1.8fr plus three 1fr cells (index, WIB clock, session, breadth) on the ghost-dot ground, with the full-width index tape along its foot.
4. **Content**: on the monitor, 01 Papan Harga at two thirds beside a right column stacking 02 Saham Teraktif over 03 Keterangan, then a full-width stack (04 Ikhtisar, 05 Catatan, 06 Daftar Perintah) and the signature footer. On the terminal, results run in the left column beside a 300px sticky rail of commands and history.
5. **Status bar** (28px, fixed to the bottom): live lamp, snapshot age, source, Pause, Keys, key hints.

The gutter is 16px (12px under 46rem) and every text edge in every panel keeps it, while panel edges meet the rule flush. Panel bodies pad 12px top, 14px bottom; headers 9px over 8px (12px over 11px on the stacked panels). Table cells pad 4px by 10px, board rows 7px, movers 3px.

Breakpoints: under 80rem the band's index cell narrows to 1.3fr and the LED figures step down; under 64rem the monitor and terminal collapse to one column, the index cell spans the band, the three-faces and notes go single column, and the status bar drops the source note; under 46rem the gutter tightens, the gate label and key hint hide, the band goes two columns with breadth spanning, and the board drops rank, name, volume and value. A desktop screen up to 1080px tall shows the movers' top five beside the board; stacked, all ten.

**The Flush Panel Rule.** Panels are flush rectangles divided by 1px hairline rules. Wrappers that hold panels and tabs carry no padding of their own; only the text inside them keeps the gutter.

**The One Region Rule.** One region owns the screen: the board on the monitor, the newest result under the gate on the terminal. Everything else is a column or a stack beside or below it.

## Elevation & Depth

Flat. There are no shadows, no glows and no tonal elevation steps. Depth is expressed only by the 1px rule between regions and by the ghost-dot texture (4px pitch radial dots in the ghost colour) that marks the LED band and the stacked panel strips as the lamp surface. The keys dialog sits on a ground-coloured scrim at 78% opacity, outlined in the control edge, not lifted.

**The No Lift Rule.** Nothing rises off the glass. State is shown by inversion, colour of the circuit or a rule turning mint, never by a shadow or a raised surface.

## Shapes

Square everything: zero radius on panels, buttons, inputs, key caps, tabs, dialogs and lamps. Status lamps are 7px squares, the moved-row lamp a 5px square, breadth and tape are dots on a 4px grid, the busy runner a 3px dot strip. Outlines are 1px in the control edge; selected nav links and tabs carry a 2px lit bottom rule. Icons are thin stroked SVG (1.5 to 1.6px, mitre joins): the star and the arrow.

The brand mark (a rounded tile with gradient candles) is a fixed identity asset and keeps its own shape; it is the one exemption from the square rule and is never imitated by interface elements.

## Components

### Buttons
Engraved, outlined, inverting.
- **Shape:** square (0px), 1px control-edge outline.
- **Outline (Run, Copy, Pause, Keys, Close):** transparent fill, lit label in the label style; 28px tall in the gate, 22px for Copy, 20px in the status bar.
- **Hover:** inverts to a lit fill with ground text and a lit border. No transition.
- **Go (Open the terminal):** the one filled control, lit fill with ground text, 700 caps, a stroked arrow at the right; hover inverts back to ground.
- **Focus:** a 2px mint outline at 2px offset, as for every focusable element.
- **Hints and range buttons:** the terminal's suggested-command chips and chart range buttons share the outline style; pressed range buttons invert; CLI-only hints use a dashed edge and dim text.

### Gate (command line)
- **Style:** a full-width row, no box; the input is borderless and transparent, lit text, mint caret, dim placeholder.
- **Focus:** the row's bottom rule and the `PERINTAH` label turn mint.
- **Typeahead:** a flush list under the gate, rows split by rules, kind in dim caps, key in lit 600 with matched characters in mint, description dim and condensed; hover raises, the selected row inverts to lit.

### Navigation
- **Top bar links:** dim caps labels, 12px padding, full bar height; hover lifts to lit; the current page is lit with a 2px lit bottom rule.
- **Tabs (Saham Teraktif):** equal-width caps labels split by rules, the selected tab lit with a 2px lit bottom rule.
- **Star control:** outlined, label plus a condensed count cell split by a rule; hover inverts. Below 26rem the label drops to the star icon beside the count, and below 22rem the top bar shows the mark without the wordmark, so the bar fits a 320px screen.

### Panels
The addressable unit of the world.
- **Header strip:** number in dim 600, the Indonesian title in 700 caps at 0.14em, an English subtitle in dim condensed text that truncates, and an age stamp pushed right (`AGE` in dim caps, the value lit, amber when stale). Stacked panels lay the strip on the ghost-dot ground.
- **Body:** gutter padding, flush to the rules.
- **Machine line:** the closing strip, a caps passport string such as `IDX.MARKET.V1<<YAHOO<<MINUTE<SNAPSHOT` padded to the edge by a run of `<` in the filler colour. The string is aria-hidden and paired with a normal-case screen-reader sentence.
- **Focus:** panels are reachable by number keys and take a mint outline inset 2px.

### Tables
- **Style:** full width, collapsed, 12px data at 75% width, right-aligned figures, first column left; dim caps column heads; rows split by rules, the last row open.
- **Hover:** the row takes the raised tone.
- **Board:** 7px rows, bold symbols, a 5px mint lamp before the rank of any row whose values moved in the last refresh (its rank gains an accessible label saying so).
- **Overflow:** a horizontally scrolling table fades its right 2.5rem and shows a caps scroll cue.

### LED band
The station clock. Index level, change, WIB clock and session countdown in Doto over a ghost-dot ground; a square session lamp (mint when a session is on); breadth as a 12px bar of mint, rose and dim dots with lit counts; the index tape as a dot line, lit dots mint when rising and rose when falling.

### The board flip (signature)
Each minute refresh patches the board, movers and band cell by cell. A cell whose text changed shows a single 140ms frame of inverted dots (lit ground, ground-coloured 3px dot grid, ground text) through `steps(1, end)`, then its new value. There is no tween, fade or slide. Under `prefers-reduced-motion: reduce` the value swaps with no flash, and the Pause control (or P) stops the refresh, the clocks and the flips together.

### Status bar
Fixed 28px strip. A square lamp and caps state word: mint for live, amber for stale, rose for offline and error, an outlined square for paused, dim for idle. Then the snapshot age (lit value), the source note (first to give way on narrow screens), and outlined buttons with key caps at the right.

### Key caps
Outlined 1px in the control edge, 18px minimum width, 16px line, lit 600 at 12px. Used inline in hints, the status bar and the keys dialog.

## Do's and Don'ts

### Do:
- **Do** split every region with a 1px rule in the hairline colour and let panels meet it flush.
- **Do** open every content panel with number, Indonesian title, English subtitle and age stamp, and close it with an aria-hidden machine line plus a screen-reader sentence.
- **Do** keep mint for live state, focus, the caret and gains; rose for losses and errors; amber for delayed or stale only.
- **Do** show hover, pressed and selected by inverting to a lit fill with ground text.
- **Do** set figures tabular and tables at the 75% width; set labels at 87.5%.
- **Do** change live values with the one-frame flip (140ms, `steps(1, end)`) and swap them with no flash under reduced motion.
- **Do** keep 12px as the working Martian Mono size and build hierarchy from weight, caps tracking and the Doto step.

### Don't:
- **Don't** use cards, shadows, glows, colour gradients on surfaces (the ghost-dot texture is the only pattern) or any border radius on interface elements.
- **Don't** set Doto anywhere but the LED band figures and the 404 status code.
- **Don't** tween, fade or slide a changing value; the board flips or it swaps.
- **Don't** use mint as a background wash or decoration, or amber as a general highlight.
- **Don't** add a ladder of Martian Mono sizes to create hierarchy.
- **Don't** build a headline-and-screenshot developer landing, feature cards, or neon-on-black terminal styling.
- **Don't** load fonts from anywhere but Google Fonts.
