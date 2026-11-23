# The `.abaco` file: `veredas-abaco`, version 1

A workbook born in Ábaco, and at the same time the session that works on other files
(`docs/abaco-plan.md`, "The `.abaco` file"): one zip of plain-text parts, readable with `unzip`
and an editor. `veredas_abaco::abacofile` writes it (`write`, through a file beside the
destination renamed into place, the file it replaces kept as `<name>.bak`) and reads it back
(`read`, evaluated; nothing it carries runs). Its export to `.xlsx` is the fork's writer
(`Workbook::export`).

Every plain-text file of Veredas 2 states its schema (`docs/questions.md` Q10): each TOML part
opens with `format` and `version`, read before its body by the suite's one header reader (core's
`veredas_core::schema`, C-045), so a part of another format, or of a version this Ábaco does not
know, is refused by name, never guessed. A key nothing reads is refused too, at any depth of a
part, by its path (`styles.fonts.0.szz`); a tool's own records and state are tables whose keys
the tool chooses.

## The parts

| Part | Format | Holds |
|---|---|---|
| `manifest.toml` | `veredas-abaco` | who wrote the file, the workbook's name, its sheets in order (each with its part), its macros |
| `workbook.toml` | `veredas-abaco-workbook` | the workbook's settings, metadata, defined names, styles, theme, tables and linked workbooks |
| `sheets/NN-<name>.toml` | `veredas-abaco-sheet` | one sheet: its structure, then its cells, one line per cell |
| `macros/<name>.py` | (Python) | a macro the workspace carries, never run at open |
| `workspace/references.toml` | `veredas-abaco-references` | the foreign files the session works on, each fingerprinted |
| `workspace/decisions.toml` | `veredas-abaco-records` | what its tools decided, oldest first |
| `workspace/queue.toml` | `veredas-abaco-records` | what its tools queued, in order |
| `workspace/state/<tool>.toml` | `veredas-abaco-state` | one tool's own state |
| `workspace/log.txt` | (text) | the log, one entry per line |

A sheet's part is named by its place and a file-safe spelling of its name (`01-Loads.toml`; a
character other than a letter, a digit, `.`, `_`, `-` or a space becomes `_`); the manifest says
which part holds which sheet. A macro's and a tool's name is letters, digits and `_`, not opening
with a digit. The workspace's parts are written only when they hold something.

A part this Ábaco does not read is kept and written back as it came, so a newer Ábaco's additions
survive an older one's save. The same workbook writes the same bytes: every zip entry is dated at
the format's epoch (1980-01-01) with mode 0644.

## The manifest

| Key | Holds |
|---|---|
| `format`, `version` | `"veredas-abaco"`, `1` |
| `written_by` | the Ábaco that wrote it, `veredas-abaco <version>` |
| `name` | the workbook's name |
| `macros` | the macros' names, each a `macros/<name>.py` part |
| `[[sheets]]` | in order: `name`, the sheet's name, and `part`, the part that holds it |

## The workbook part
The engine's own structures, as the engine names them (`ironcalc_base::types`): `[settings]`,
`[metadata]`, `[[defined_names]]`, `[styles]`, `[theme]`, `[tables.<name>]`,
`[[external_books]]`. It is the part least meant for an editor's hand; Ábaco reads it back whole.
These structures pass through JSON on their way, so their keys come in TOML's sorted order, and
the engine's none (an unset alignment, a colour that is none) is written `nan`: TOML has no null,
and no number the engine keeps there is a NaN. A sheet part's structure is written the same way.

## A sheet part
The sheet's structure first, as the engine names it: `name`, `sheet_id`, `state`, `color`,
`dimension`, `frozen_rows`, `frozen_columns`, `show_grid_lines`, and the lists `[[cols]]`,
`[[rows]]`, `[[merged_cells]]`, `[[comments]]`, `[[conditional_formatting]]`, and `links`, the
hyperlinks as `[row, column, link]` in row-major order. Then the cells, row-major, in the `[cells]`
table, each keyed by its address:

| A cell | Holds |
|---|---|
| `{ n = 6.5 }` | a number: a whole number under 10^15 as an integer, any other in the shortest text that reads back as the same double |
| `{ s = "Span (m)" }` | a text, in TOML's own string |
| `{ b = true }` | a boolean |
| `{ e = "#N/A" }` | an error |
| `{ f = "B2*B1^2/8", v = 63.375 }` | a formula, without its `=`; `v` (or `e`, an error) is the value it last computed, for the reader: it is computed again when the file is opened |
| `{ array = "A1:A3*2", size = [1, 3] }` | a legacy array formula over its size, columns then rows |
| `{ dynamic = "SEQUENCE(3)" }` | a dynamic array formula; the cells it spills into are not written |
| `{ style = 5 }` | a cell that carries only its style |

`style` is an index into the workbook part's `styles.cell_xfs`, written when it is not 0. A cell
states exactly one of `n`, `s`, `b`, `e`, `f`, `array` and `dynamic`, or none (a style alone); `v`
goes only with `f`, `size` only with `array`. Its key is the address as written here (`B1`, never
`b1` or `$B$1`); its `style` one the workbook part holds. An error is written as the engine shows
it (`#N/IMPL` among them) and read in that spelling or the engine's English name.

## The workspace parts
`workspace/references.toml` lists `[[reference]]`: `role` (what the file is to its tool),
`path`, and, as recorded, `size` in bytes, `modified` (RFC 3339, UTC, whole seconds) and
`sha256` (lowercase hex) of its content. A reference recorded with a hash knows when its file
moved on (`Reference::changed`).

`workspace/decisions.toml` and `workspace/queue.toml` list `[[record]]`, each a TOML table whose
keys the tool chooses. `workspace/state/<tool>.toml` holds the tool's `[state]` table. TOML has no
null: a record holds none.

`workspace/log.txt` holds one entry per line, oldest first, with `\`, a tab, a line feed and a
carriage return written `\\`, `\t`, `\n`, `\r`; a backslash before anything else is refused.

## The worked example: a beam check
A workbook named `Beam check`, its one sheet `Loads`: a span and a load, the moment they give
(`=B2*B1^2/8`, 63.375 kNm), a flag, a ratio that divides by zero, three numbers a dynamic array
spills (`=SEQUENCE(3)` at D1; D2 and D3 are computed, not written), its label in bold and one cell
bold alone; a macro; the loads file the session reads, fingerprinted; a check's decision, its next
step queued, its state; two lines of log. `crates/veredas-abaco/tests/abacofile.rs` builds it
through the API, writes it, holds each block below against the part written (the first line, a
comment, names the part; `<version>` is the writing Ábaco's), and reads it back whole.

```toml
# manifest.toml
format = "veredas-abaco"
version = 1
written_by = "veredas-abaco <version>"
name = "Beam check"
macros = ["beam_report"]

[[sheets]]
name = "Loads"
part = "sheets/01-Loads.toml"
```

The sheet's part; `color = nan` is a sheet with no tab colour (the engine's none: TOML has no
null), the lists empty for a new sheet.

```toml
# sheets/01-Loads.toml
format = "veredas-abaco-sheet"
version = 1
color = nan
cols = []
comments = []
conditional_formatting = []
dimension = "A1"
frozen_columns = 0
frozen_rows = 0
links = []
merged_cells = []
name = "Loads"
rows = []
sheet_id = 1
show_grid_lines = true
state = "Visible"

[cells]
A1 = { s = "Span (m)" }
B1 = { n = 6.5 }
D1 = { dynamic = "SEQUENCE(3)" }
A2 = { s = "Load (kN/m)" }
B2 = { n = 12 }
A3 = { s = "Moment (kNm)", style = 1 }
B3 = { f = "B2*B1^2/8", v = 63.375 }
A4 = { s = "Checked" }
B4 = { b = true }
A5 = { s = "Ratio" }
B5 = { f = "B3/0", e = "#DIV/0!" }
C5 = { style = 1 }
```

The session's parts:

```toml
# workspace/references.toml
format = "veredas-abaco-references"
version = 1

[[reference]]
role = "loads_csv"
path = "loads.csv"
size = 4
modified = "2026-11-23T10:04:05Z"
sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
```

```toml
# workspace/decisions.toml
format = "veredas-abaco-records"
version = 1

[[record]]
cell = "Loads!B3"
tool = "check"
verdict = "ok"
```

```toml
# workspace/state/check.toml
format = "veredas-abaco-state"
version = 1

[state]
last = "B3"
runs = 2
```

`workspace/queue.toml` lists the queued step as `decisions.toml` lists the decision, and
`workspace/log.txt` holds the two entries, the tab in the second escaped:

```text
opened Beam check
B5: #DIV/0!\tleft for the engineer
```

## Reading

`read` takes every entry of the zip, reads the manifest and the
workbook part, then each sheet the manifest names, its stamp first. It places values and styles as
the file states them, enters the formulas once the model stands (each keeping the kind the file
states and its own style), reads the macros the manifest names and the workspace's parts that are
there, keeps every other part, and evaluates the workbook. A refusal names the file, the part and,
for a cell, its address: a part that is missing or not UTF-8, a stamp of another format or of
another version, a misspelled key, a cell stating two kinds, `v` without `f`, an array of no size,
an error code that is none, an address that is none or not written as here, a style the workbook
does not hold, a number that is no finite number (`nan`, `inf`), a value of the wrong type, a
manifest with no sheet, a sheet whose part names another sheet than the manifest, a reference's
hash or date not in its form, a log line with a bad escape.
