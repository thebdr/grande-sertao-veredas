# Design references

Mockups for Veredas 2 components. Each folder holds `.dc.html` screens and a `support.js`
runtime: open any screen in a browser with `support.js` beside it. They are **design
references, not production code**: they show layout, hierarchy, colour and the concepts a
component commits to. All sample data in them is invented.

| Folder | Component | Screens |
|---|---|---|
| `comissionador/` | veredas-comissionador | Designer (flow chart), Designer (text), Run & report |
| `supervisao/` | veredas-supervisao | Layout overview, Dashboard objects |

## Shared look
Both follow the Veredas "Console / IDE" direction set in Veredas 1's
`design_handoff_veredas_redesign`. The tokens they use are repeated here so this repo
stands on its own.

**Type.** Sans for chrome (titles, labels, buttons), mono for data (values, tags, code).
The mockups use Geist and Geist Mono as stand-ins; the desktop apps ship Monaspace Xenon.
Scale: screen title 18/600 · section title 13.5/600 · body 12–13 · label 11.5 · caption
10–11 · mono data 11.5–12.5 · big value 26 mono · column headers 10 mono 600, uppercase,
.06–.1em tracking. Uppercase is typed, never made by a stylesheet (`text-transform`): an app's own
heading words are written in capitals in its source; a label an engineer wrote, a column name and a
symbol are drawn exactly as written (σ stays σ, never Σ; `I (mA)` never `I (MA)`). Q28, decided
2026-11-16.

**Neutrals (dark).**

| Role | Hex |
|---|---|
| Rail / input / code background | `#0F0F11` |
| Titlebar | `#111113` |
| Chrome, side panels, status bar | `#161618` |
| Canvas / page ground | `#141416` |
| Content ground | `#1B1B1E` |
| Card | `#191A1D` |
| Raised surface, buttons | `#26262A` |
| Hairlines | `#222226`, `#232329`, `#26262A`; borders `#2A2A30`, inputs `#2C2C33`, outline `#333339` |
| Text | `#E8E8EC`; strong `#FFFFFF`; data plain `#D4D4DA` |
| Text weak | `#9A9AA3`, `#8A8A92`; field label `#7F8088` |
| Muted (decorative only) | `#6F6F77` |

**Accent.** One for the whole suite, lavanda (`docs/questions.md` Q22, reversed on 2026-11-15).

| Tone | Colour | Use |
|---|---|---|
| Accent | `#B9A9E8` | selection, focus, primary actions |
| Soft | `#2B2838` | the ground of a selected row, tab or button |
| Outline | `#494360` | the edge of a selected or hovered control |
| Text on the accent | `#12101C` | a label on a button filled with the accent |
| Tint | `#CFC3F2` | neutral data series and hover (the assistant's proposal, not ruled) |

The mockups in this folder were drawn before the reversal. Their green (`#30D158`, comissionador)
and their magenta (`#AC007C`, supervisão) read as the accent wherever they mark selection, focus or
a primary action.

**Status.** Warn `#FFC864` (near-full fill `#B88F3E`), danger `#FF6B6B` (text `#FF8F8F`),
neutral pill `rgba(154,160,166,.14)`.

**Syntax.** plain `#D4D4DA` · keyword `#6FB1FF` · key/type `#FFD780` · string `#7EE787` ·
number `#D2A8FF` · escape `#FFB86C` · punctuation and comments `#8A8A92`.

**Rules both follow.**
- Every state has a mark and a word, so it reads without colour.
- The accent marks selection, focus and primary actions; it is never a plant state.
- Green means a good state and is never the accent (`docs/questions.md` Q22). Which state is
  "good" is open (Q45), so no state takes it yet.
- Writes that touch the plant are shown as gated (warn colour) wherever they appear.
- Radii: widgets 6 px, cards 8–12 px, pills 5–7 px, chips 999 px. Spacing on an 8 px rhythm.
