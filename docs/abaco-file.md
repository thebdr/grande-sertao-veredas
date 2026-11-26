# The `.abaco` file: `veredas-abaco`, version 1

A workbook born in Ábaco, and at the same time the session that works on other files
(`docs/abaco-plan.md`, "The `.abaco` file"): one zip of plain-text parts, readable with `unzip`
and an editor. `veredas_abaco::abacofile` writes it (`write`, through a file beside the
destination renamed into place, the file it replaces kept as `<name>.bak`; a write that fails
leaves the destination as it was and nothing beside it; the temporary file is one of its own, made
new under a name of its own (`.veredas-<process>-<n>.tmp`, so a long destination name is written up
to the file system's limit less the `.bak` four bytes), and the names are the system's whatever
their encoding; two writes of one destination at once are not serialised, a limit) and reads it
back
(`read`, evaluated; nothing it carries runs). Its export to `.xlsx` is the fork's writer
(`Workbook::export`).

Every plain-text file of Veredas 2 states its schema (`docs/questions.md` Q10): each TOML part
opens with `format` and `version`, read before its body by the suite's one header reader (core's
`veredas_core::schema`, C-045), so a part of another format, or of a version this Ábaco does not
know, is refused by name, never guessed. A key nothing reads is refused too, at any depth of a
part, by its path (`styles.fonts.0.szz`), and so is one that replaced a field the part then lacks
(a link's `url` for its `target`: `links.0.2.url`), found by reading the object that lacks it
alone (a link, a format's rule: inside one, the reader reads from a buffer that marks no key)
with the field given, as each kind of value and each variant of a tag, the reading that leaves
fewest keys unread deciding, never by its spelling; a field deleted, a tag too, no key standing
for it, is refused in the reader's words (``missing field `min` ``). A tool's own records and state are tables whose keys
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

A part, a sheet's or a macro's, is listed once: one listed twice is refused by the manifest.

## The workbook part
The engine's own structures, as the engine names them (`ironcalc_base::types`): `[settings]`,
`[metadata]`, `[[defined_names]]`, `[styles]`, `[theme]`, `[tables.<name>]`,
`[[external_books]]`. It is the part least meant for an editor's hand; Ábaco reads it back whole.
These structures pass through JSON on their way, so their keys come in TOML's sorted order, and
the engine's none (an unset alignment, a colour that is none) is written `nan`: TOML has no null.
JSON writes a number that is no finite number as its null too; the engine keeps one only from a
malformed workbook (a column's width of `NaN`), and the writer refuses it by its path
(`cols.0.width`) rather than write a file its reader refuses. A TOML date-time is no value of the
engine's, refused by its path. A sheet part's structure is written the same way.

Both ends hold the workbook to what the engine takes: sheet names of 1 to 31 characters with none
of `\ / * ? : [ ]`, told apart in any case, each sheet's id its own; a row's `s` and a column's
`style` one of the workbook's styles; each style's font, fill and border one of the workbook's,
and its number format's id none below 0, a named style's too (applying it copies them into a
cell's style); at least one sheet; a defined name spelled as Excel spells
one (MS-XLSX's: a letter, `_`, `\` or any character beyond ASCII first, then those, digits, `.` and
`?`, so no `@B1`,
`(B1)` or `B1+0`, and neither `TRUE` nor `FALSE`) that does not read as a cell (`B1`, `$B$1`,
`R1C1`, `R[1]C[-1]`, in any case, Unicode's capitals among them, `ß1` being SS1, `Sheet1!B1`, a
range such as `A:A`, one with white space),
stated once in its scope. The writer refuses, by the cell's address, what the reader would refuse
of a cell (a style the workbook does not hold, a legacy array formula over merged cells, which the
engine lets a merge leave but never enters, of no size, past the sheet's edge or over another's
area, a cell with content inside another's area, an empty formula, a formula the engine cannot
read, which it would read back repaired: `SUM(A1:A3` as `SUM(A1:A3)`, a formula whose spelling the
reader would read as another formula: a locale whose decimal is a comma spells an array's rows with
`/`, which the engine's lexer does not read), and a reference not
in its form, settings the engine refuses, a table the reader would refuse (its cells, its header and totals
rows, its sheet), a workspace part TOML's reader would not read back (a
tool's table nested past its depth), and a part it kept that a written one would overwrite, so
every file it writes reads back. A name local to a sheet the workbook does not hold is kept: the engine
holds one for a chart sheet of the `.xlsx` the workbook came from, which no formula reads.

## A sheet part
The sheet's structure first, as the engine names it: `name`, `sheet_id`, `state`, `color`,
`dimension`, `frozen_rows`, `frozen_columns`, `show_grid_lines`, and the lists `[[cols]]`,
`[[rows]]`, `[[merged_cells]]`, `[[comments]]`, `[[conditional_formatting]]`, and `links`, the
hyperlinks as `[row, column, link]` in row-major order. Then the cells, row-major, in the `[cells]`
table, each keyed by its address:

| A cell | Holds |
|---|---|
| `{ n = 6.5 }` | a number: a whole number under 10^15 as an integer, any other in the shortest text that reads back as the same double |
| `{ s = "Span (m)" }` | a text, in a TOML basic string on its line: `"`, `\`, a tab, a line break and every other control character escaped (`"a\nb"`, `"\u0007"`), the C1 controls (U+0080 to U+009F) and Unicode's line and paragraph separators (U+2028, U+2029) too, so no reader splits the line |
| `{ b = true }` | a boolean |
| `{ e = "#N/A" }` | an error |
| `{ f = "B2*B1^2/8", v = 63.375 }` | a formula, without its `=`; `v` (or `e`, an error) is the value it last computed, for the reader: it is computed again when the file is opened |
| `{ array = "A1:A3*2", size = [1, 3] }` | a legacy array formula over its size, columns then rows |
| `{ dynamic = "SEQUENCE(3)" }` | a dynamic array formula; the cells it spills into are not written unless their style is not 0 |
| `{ style = 5 }` | a cell that carries only its style |

`style` is an index into the workbook part's `styles.cell_xfs`, written when it is not 0. A cell an
array spills into is written only for its style; one not written holds style 0, whatever style its
row or column has (which the engine would give a cell it creates). A formula is spelled as the
reader reads it, in English and the locale the workbook's settings name, whatever language or locale
the model shows, and so it reads back as the same formula: the brackets that change its meaning kept (`(A1&A2)*1`,
`(A1+A2)%`, `A1+(A2+A3)`), and a number, in an array constant too, in Excel's 15 digits when they
hold it, otherwise the shortest spelling that does (`0.30000000000000004`). Of two dynamic arrays whose spills meet, the one that spills is decided
on reading row by row, as the engine opens any workbook, not by the order they were entered in (a
limit). A cell
states exactly one of `n`, `s`, `b`, `e`, `f`, `array` and `dynamic`, or none (a style alone); `v`
goes only with `f`, `size` only with `array`. A formula's `v` is a finite number, a text or a
boolean, and never stands beside its `e`. An integer `n` or `v` is one a double holds exactly
(`9007199254740993` is refused, not rounded). Its key is the address as written here (`B1`, never
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
carriage return written `\\`, `\t`, `\n`, `\r`, and every other control character and Unicode
line break (U+000B, U+000C, U+001C to U+001E, U+0085, U+2028, U+2029 among them) as `\uXXXX`, so no
reader splits an entry; a backslash before anything else, or a `\u` without four hex digits naming
a character, is refused.

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
another version, a misspelled key, a cell stating two kinds, `v` without `f`, a legacy array of no
size, past the sheet's edge (refused before its corner is reckoned) or over another's area, a
formula of white space alone, a formula the engine cannot read as the file states it (`1+`, `=A1`,
`SUM(A1:A3`, which the engine would enter as its error or repaired, `SUM(A1:A3)`), read by a
parser of the file's locale and English with no repair,
an error code that is none, an address that is none or not written as here, a style the workbook
does not hold (a cell's, a row's, a column's), a number that is no finite number (`nan`, `inf`)
or an integer no double holds, a value of the wrong type (a date-time among them, a formula's `v`
that is a table), a formula's `v` beside its `e`, a manifest with no sheet, a sheet whose part
names another sheet than the manifest, a sheet name or a defined name the engine refuses, a table whose
cells are no range or a range with its corners out of order, whose header or totals rows are
none of Excel's (0 or 1) or leave no data row, whose columns are not its range's width, or whose
sheet is none of the workbook's as the sheet spells its name (`tables.Table3.reference`), a cell's
link stated twice (`links.1`), a linked workbook's cached cell stated twice, or its sheet stated twice (in any case the engine
takes as one), a cell stated inside a
legacy array's area (but a style-only cell it spills into), settings
the engine refuses (a time zone, a locale: the workbook part named), a reference's hash or date
not in its form, a log line with a bad escape.
