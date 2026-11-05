# veredas-comissionador: test designer and runner

## Overview
A first visual and functional mockup of **veredas-comissionador**, the Veredas 2 testing
platform: predefined checks that run against plant data from **veredas-portal**, either
live or replayed over historical data, and end in an acceptance report. Tests are designed
in the platform's own designer, as text or as a flow chart. The flow chart exists so that
testers who are not software developers can read and build tests; text stays available
for everything that is easier to write than to draw.

This package covers three screens:
- **Designer · flow chart** (`Main.dc.html`)
- **Designer · text** (`Text.dc.html`)
- **Run & report** (`Run.dc.html`)

## About the design files
The `.dc.html` files are a **design reference created in HTML**: they show intended layout,
hierarchy, colour and the main concepts. They are not production code to copy. Open any of
them in a browser with `support.js` beside it. The screens link to each other: the Flow chart / Text
toggle, the Run button and the left rail move between them.

All sample content is invented for the mockup: the test, tag names, parcel ids, run numbers
and timings. `[TESTER]` on the sign button is a placeholder.

## Fidelity
**Mid-fidelity.** Layout, component structure and the concepts below are the intended
direction; exact spacing and the text syntax are a first draft to be refined.
The look follows the shared Console / IDE direction in `../README.md`: same neutrals,
green desktop accent, icon rail, titlebar, content header and status bar. Geist / Geist
Mono stand in for **Monaspace Xenon**.

## Concepts the design commits to
- **One test file, two views.** A test is one plain-text file (`tests/<id>.test`),
  diff-able in git. The flow chart and the text editor are two views of it; editing in one
  updates the other. Flow-chart positions live in a trailing `layout { … }` block that the
  flow view writes.
- **Blocks.** Write (gated), Wait, Read, Check, Branch, Script, Function, Report, plus
  start/end. Each block type has its own colour (table below).
- **Script boxes.** A step with no clean graphical form (a calculation, say) is written as
  code inside the chart.
- **Functions as blocks.** Custom functions are written once in text (`functions/*.fn`)
  with typed inputs and outputs, then placed in the chart as blocks, much like a function
  block in PLC ladder. Inputs are left ports, outputs right ports. A function declares what
  portal data it `reads` and may not write.
- **Branching on outcome.** A Branch block routes on any value a step produced: ranges or
  cases, each exit pointing to a step, a `retry <step> max N`, or a fail path. In the
  sample, `ratio >= 0.95` passes, `0.85 .. 0.95` retries `route` once, anything else fails.
- **Gated writes.** Write blocks are marked "gated". They go through portal's write surface
  (tag marked writable, write switch armed at run start). The inspector lists how many
  gated writes a test makes before it runs.
- **Live or replay.** The run-mode toggle runs a test against the live plant or replays it
  over a stored portal window. A failed live run can be replayed from its own window.
- **Runs are kept.** Each run keeps its step results, the portal frames it used and its
  report under `runs/<test>/<n>`; the run history lists them.

## Design tokens
All neutrals, accent, status and syntax colours come from `../README.md`. Additions for
this screen set:

| Use | Hex |
|---|---|
| Write · gated | `#FFC864` (warn: a write touches the plant) |
| Wait / Read | `#66B2FF` |
| Check | `#30D158` |
| Branch / Retry | `#FFB266` |
| Script | `#80DEDE` |
| Function | `#CC99FF` |
| Report | `#9A9AA3` |
| Fail path | `#FF6B6B` (text `#FF8F8F`) |
| Line-number gutter | `#5E8A6C` (lighter than `#3F6F4F`, to pass contrast) |
| Code comment | `#8A8A92` |

Blocks: card `#191A1D`, border `#2A2A30`, a 3px left stripe in the block colour, 10px
radius, a mono uppercase type label top-left and the step id top-right. The selected block
gets a 2px accent border. The canvas is `#141416` with a 20px dot grid.

## Screens

### Designer · flow chart (`Main.dc.html`)
- **Left:** test library grouped by area (status dot plus last result per test), a
  Functions group, and "+ New test".
- **Header:** path, test name, unsaved-changes pill; Flow chart / Text toggle; Live /
  Replay history toggle; Validate; primary "Run live".
- **Block palette:** a row of draggable block types above the canvas, with step and branch
  counts.
- **Canvas:** the test top to bottom. Pass continues down; Branch exits leave to the right
  (retry above, fail below).
- **Inspector (right):** the selected block's inputs (editable) and outputs, where it is
  defined in text, and a notice of the test's gated writes.
- **Status bar:** portal connection and tag count, write switch state, last run, file and
  git state.

### Designer · text (`Text.dc.html`)
- **Left pane:** the test file with line numbers and syntax colours. The line selected in
  the flow chart is highlighted.
- **Right pane:** the function definition the selected block uses, and how it maps to a
  block.
- **Problems bar:** parse state, counts (steps, branches, functions, gated writes), whether
  the layout matches the flow view, cursor position.

### Run & report (`Run.dc.html`)
- **Header:** run number, mode, time window and duration, a verdict pill; "Open in
  designer", "Replay this window", "Run again".
- **Steps:** one row per executed step (status, step id in its block colour, result,
  time), including the retry loop; skipped steps greyed out.
- **Signals during the run:** tracks for the tags and events the test touched, with a
  legend, and the parcel table. Warnings (no outcome within the timeout) are tinted.
- **Acceptance report:** criterion, measured value, result, evidence, tester note,
  Export PDF and Sign.
- **Run history (right):** earlier runs of the test with their mode and result.

## Text format (draft)
The syntax in the mockup is a sketch to make the idea concrete, not a settled grammar:

```
test "<name>" {
  id / area / portal / modes      # metadata
  use functions/<file>.fn         # imports
  let <name> = <value>
  step <id>  write <tag> := <value>
  step <id>  wait  <condition> timeout <duration>
  step <id>  call  <Function>(<arg>: <value>, …) -> <out>, …
  step <id>  script { … }
  step <id>  branch <value>
               <range or case> -> <step> | retry <step> max <n>
               else            -> <step>
  step <id>  report <values…>
  step <id>  stop fail
}
layout { <step> <col>,<row> … }   # written by the flow view

function <Name>(<in>: <type>, …) -> (<out>: <type>, …) {
  reads <portal data…>
  timeout <duration>
  …
}
```

## Open points
- The test and function grammar: keywords, types, expression language and error messages.
- How retries count across nested branches, and how they appear in the report.
- Who can sign a report, and whether a witness signature is needed for safety tests
  (the SAF group in the library).
- PDF layout of the acceptance report.
- Ladder-style function blocks: how many ports a block shows before it collapses.

## Files
- `Main.dc.html`, `Text.dc.html`, `Run.dc.html`: the three screens.
- `support.js`: runtime that lets the files open in a browser. Not part of the app.
