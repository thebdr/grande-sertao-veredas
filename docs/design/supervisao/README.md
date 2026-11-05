# veredas-supervisao: layout overview and dashboard objects

## Overview
A first mockup of **veredas-supervisao**, the Veredas 2 web interface. It shows plant
layouts and dashboards for a lot of live data, so the chrome is kept to a minimum and the
data gets the screen. Every value arrives through **veredas-portal**; every write rides
portal's gated surface.

Screens:
- **Layout overview** (`Main.dc.html`)
- **Dashboard objects** (`Objects.dc.html`): a sheet of the objects dashboards are built from

Open either in a browser with `support.js` beside it. All sample content (Sorter A, its
chutes, counts, times, tag paths) is invented; `[DEST]` is a placeholder for a chute's
destination.

## Fidelity
**Mid-fidelity.** Layout, object anatomy, states and the concepts below are the intended
direction; exact sizes and numbers are not. The look is the shared Console / IDE direction
in `../README.md`, with the **web magenta accent** (supervisão absorbs Veredas 1's
dashboard, which introduced it).

## Decisions the design builds on
- **2D layouts in SVG, 3D in glTF.** Both use the same object IDs, so one plain-text binding
  file drives either view. Configuration is plain text; 3D models are assets it points to.
- **Several sessions from one machine.** One supervisão host serves many browser sessions
  side by side.
- **Editing** happens in the browser or in the plain-text config files.

## Layout overview (`Main.dc.html`)
- **Top bar, one line:** wordmark (`veredas ›`, the web side carries it), plant name, view
  tabs (Layout, Dashboards, Tables, Alarms with a count), four headline figures, live clock.
- **Plant view:** fills the screen. Sample: a loop sorter with 24 chutes, 4 inductions and
  a scanner; parcels move along the loop (one, in magenta, is followed). Each chute shows its
  fill level and state; the loop shows speed, carriers, occupancy and laps to sort.
  Floating controls only: layer toggles (parcels, states, labels, heat map) top-left,
  legend bottom-left, zoom bottom-right. A tweak switches moving parcels off.
- **Side panel (collapsible):** the selected object (chute 12, full): when it went full,
  counters, a 30-minute inflow sparkline, a gated action shown disabled while portal's write
  switch is off; below it, a "Needs attention" list of everything not normal.
- **Alarm strip:** one line with the latest alarms and the layout's source file.

## Dashboard objects (`Objects.dc.html`)
Every object: one header line (name, plus its binding or state word), then the data only.
- **Value tile** in four states: normal, warn, alarm, stale (dashed border, greyed value,
  "last value 42 s ago").
- **Trend line** with a dashed limit, breach dots and a tooltip.
- **Bars** against a dashed target; the running interval is outlined.
- **State over time:** one bar per device for the last hour (running, stopped, fault).
- **State grid:** many small cells (chute fill %) with the same states as the layout.
- **Levels:** horizontal bars against threshold ticks, instead of round gauges.
- **Table** with inline bars and a state word per row.
- **Charts in cells:** per row a median, a sparkline of the median over the last hour, and
  a distribution curve on the same scale in every row, with σ and n. A curve that widens or
  tips flags a problem before the median moves.
- **Tracking accuracy (dedicated object).** For conveyor tracking: Δ = real − virtual
  position at each sensor, &gt;0 late, &lt;0 early.
  - Header totals: sensors, matched, share within ±100 mm, lost, created, drift along the line.
  - Profile: median Δ per sensor by position along the conveyor, with ±σ whiskers and the
    ±100 / ±200 mm bands.
  - Table per sensor: position, matched, median Δ, distribution (−300 … +300 mm, same scale
    every row), matches by band (≤ −200, −200…−100, ±100 with %, 100…200, ≥ 200), lost early,
    lost late, created; totals row.
  - Definitions as drawn: **lost early**, the real object passed before the match window
    opened; **lost late**, no real object within the window; **created**, a real object with
    no virtual one, so tracking made a new object. To be confirmed against the tracking
    implementation.
  - Bands and the distribution range come from the object's config.

## States
One vocabulary in every object, each a mark plus a word: running ●, near full ■,
stopped ■, full ■, fault !, alarm !, stale ◌ (dashed), disabled ▨ (hatched), selected ◆.
Warn states use `#FFC864`, faults and alarms `#FF6B6B` / `#FF8F8F`, near full `#B88F3E`,
neutral fill `#4A4E5C`. **Magenta is never a state**: it marks what is selected or followed,
and draws neutral data series.

## Open points
- The binding file format (object ID → portal value or table, plus per-object config such as
  bands and ranges).
- How a 3D view switches with its 2D layout, and what stays the same (selection, side panel).
- Session and permission model for several sessions on one host.
- Confirm the tracking definitions (lost early / late, created) and the default bands.
