# The supervisão file and the binding file: `veredas-supervisao` and `veredas-binding`, version 1

Supervisão (`docs/supervisao-plan.md`) runs one or more plants' portal sessions and shows each
through its objects. Two plain-text files say everything it does. The **supervisão file** names
the address it serves and the sessions it runs, each a portal project file
(`docs/project-schema.md`) and a binding file. The **binding file** names a session's objects by
ID, binds each to a series the session carries, says how its value reads as a state (a mark and a
word) and how the layout shows it, names the writes it offers, points to the SVG layout whose
elements carry the same IDs, and lists the dashboards and their panels. No plant is named
anywhere: a plant's supervisão is a folder of files like the ones below, kept outside the suite's
code, and nothing of a plant's engineering exports is read (`docs/questions.md` Q18).

`veredas_supervisao::config::load` reads the supervisão file, every project and binding file it
points at, and every layout a binding points at, and checks each against the others, so a
supervisão file that loads is a host every session of which can start; the executable
`veredas-supervisao <supervisao.toml>` starts it (`--check` reads, checks and prints the plan
without starting anything).

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed, the header read
by core's one reader (`veredas_core::schema`).

## The worked example: a loop with three outlets, two parcels and an infeed

A controller serves the loop's speed, two parcels' places along it, three outlets' fill levels, an
infeed's state and an outlet's release command over OPC UA; the sorter's log records every sort
with its outlet, its travel time and the read rate. The project below reads both; the binding
shows the infeed, the outlets and the parcels on the layout, the speed and the read rate in the
top bar, and one dashboard with a panel of every kind. `crates/veredas-supervisao/tests/files.rs`
writes the four blocks beside each other under the names the files give (`supervisao.toml`,
`loop/project.toml`, `loop/binding.toml`, `loop/layout.svg`), loads them, checks every statement
below against what it read, and runs the executable's check on them; it holds a copy of this text,
so an edit here is an edit there.

The supervisão file:

```toml
format = "veredas-supervisao"
version = 1
bind = "127.0.0.1:8902"

[[session]]
name = "loop"
project = "loop/project.toml"
binding = "loop/binding.toml"
```

The session's project, a portal project file like any other (`docs/project-schema.md`):

```toml
format = "veredas-project"
version = 1
name = "the loop"

[session]
store = "ram"
writes_armed = false

[[log]]
id = "sorter"
file = "sorts.log"
timestamp = { pattern = "^(?P<ts>\\S+ \\S+)", formats = ["%Y-%m-%d %H:%M:%S%.3f"] }

[[rule]]
name = "sorts"
pattern = "SORT outlet=(?P<outlet>\\d+) travel=(?P<travel>[0-9.]+) rate=(?P<rate>[0-9.]+)"
scope = "sorter"
types = { outlet = "int", travel = "float", rate = "float" }

[[opc]]
source_id = "plc"
endpoint_url = "opc.tcp://127.0.0.1:4855/"
poll_ms = 200
rules = [
  { name = "plc_main", tags = [
    { node_id = "ns=2;s=speed", series = "speed", type = "float" },
    { node_id = "ns=2;s=p1", series = "p1", type = "float" },
    { node_id = "ns=2;s=p2", series = "p2", type = "float" },
    { node_id = "ns=2;s=fill1", series = "fill1", type = "float" },
    { node_id = "ns=2;s=fill2", series = "fill2", type = "float" },
    { node_id = "ns=2;s=fill3", series = "fill3", type = "float" },
    { node_id = "ns=2;s=infeed", series = "infeed", type = "int" },
    { node_id = "ns=2;s=release3", series = "release3", type = "int", writable = true },
  ] },
]

[[table]]
name = "outlets"
sql = "SELECT outlet, count(*) AS sorted, avg(travel) AS travel FROM sorts GROUP BY outlet"
```

The binding file:

```toml
format = "veredas-binding"
version = 1
title = "The loop"
layout = "layout.svg"
headline = ["speed", "read-rate"]

[[object]]
id = "speed"
label = "Loop speed"
value = { source = "plc", series = "speed" }
unit = "m/s"
decimals = 1
states = [ { below = 0.5, state = "warn", word = "slow" } ]
normal = "running"

[[object]]
id = "read-rate"
label = "Read rate"
value = { source = "sorter", series = "rate" }
unit = "%"
decimals = 1
states = [ { below = 98.0, state = "alarm", word = "under 98 %" } ]
stale_after_ms = 30000

[[object]]
id = "infeed"
label = "Infeed"
value = { source = "plc", series = "infeed" }
states = [
  { equals = 0, state = "warn", word = "stopped" },
  { equals = 2, state = "alarm", word = "fault" },
]
normal = "running"

[[object]]
id = "outlet-1"
label = "Outlet 1"
value = { source = "plc", series = "fill1" }
unit = "%"
decimals = 0
show = "level"
states = [ { at_least = 100, state = "warn", word = "full" }, { at_least = 80, state = "near", word = "near full" } ]

[[object]]
id = "outlet-2"
label = "Outlet 2"
value = { source = "plc", series = "fill2" }
unit = "%"
decimals = 0
show = "level"
states = [ { at_least = 100, state = "warn", word = "full" }, { at_least = 80, state = "near", word = "near full" } ]

[[object]]
id = "outlet-3"
label = "Outlet 3"
value = { source = "plc", series = "fill3" }
unit = "%"
decimals = 0
show = "level"
states = [ { at_least = 100, state = "warn", word = "full" }, { at_least = 80, state = "near", word = "near full" } ]
actions = [ { label = "Release", source = "plc", tag = "ns=2;s=release3", value = "1" } ]

[[object]]
id = "parcel-1"
label = "Parcel 1"
value = { source = "plc", series = "p1" }
show = "move"
path = "loop"

[[object]]
id = "parcel-2"
label = "Parcel 2"
value = { source = "plc", series = "p2" }
show = "move"
path = "loop"

[[dashboard]]
name = "Overview"
columns = 4

[[dashboard.panel]]
kind = "value"
object = "read-rate"
target = 99.0

[[dashboard.panel]]
kind = "trend"
object = "read-rate"
minutes = 60
limit = { below = 98.0 }
span = 2

[[dashboard.panel]]
kind = "bars"
object = "speed"
minutes = 60
bucket_minutes = 5
aggregate = "mean"
target = 1.5

[[dashboard.panel]]
kind = "timeline"
objects = ["infeed"]
minutes = 60

[[dashboard.panel]]
kind = "grid"
objects = ["outlet-*"]

[[dashboard.panel]]
kind = "levels"
objects = ["outlet-*"]
ticks = [80, 90]

[[dashboard.panel]]
kind = "table"
table = "outlets"
columns = [ { name = "outlet" }, { name = "sorted", bar = true }, { name = "travel", decimals = 1 } ]
sort = { column = "sorted", descending = true }

[[dashboard.panel]]
kind = "cells"
table = "sorts"
key = "outlet"
value = "travel"
minutes = 60
spread = 10.0
```

The layout the binding points at, an SVG whose elements carry the objects' IDs:

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 220">
  <path id="loop" d="M 80 40 H 320 A 70 70 0 0 1 320 180 H 80 A 70 70 0 0 1 80 40 Z" fill="none" stroke="#26262A" stroke-width="14"/>
  <rect id="infeed" x="20" y="96" width="40" height="28"/>
  <rect id="outlet-1" x="130" y="4" width="24" height="28"/>
  <rect id="outlet-2" x="190" y="4" width="24" height="28"/>
  <rect id="outlet-3" x="250" y="4" width="24" height="28"/>
  <circle id="parcel-1" r="5"/>
  <circle id="parcel-2" r="5"/>
</svg>
```

Loaded, this host serves one session, `loop`, at `/s/loop/` on `127.0.0.1:8902`. Its binding has
eight objects, two of which (`speed`, `read-rate`) have no element in the layout and show in the
headline and the dashboard alone; one dashboard of eight panels; one write, `Release`, offered on
`outlet-3`, posting `1` to the tag `ns=2;s=release3` of `plc` through portal's surface, refused there
while the project leaves its writes off.

## The supervisão file: keys

Each key states in parentheses that it is required, or the value it takes when left out. A path is
relative to the file's folder.

- `format` (required): always `"veredas-supervisao"`.
- `version` (required): this reader knows `1`.
- `bind` (`"127.0.0.1:8902"`): the address and port the host serves, a port of `0` letting the
  system choose. Supervisão has no accounts (Q20): whoever reaches the address sees every session,
  and posts to a session's write routes, which portal refuses unless the project armed its writes;
  an address beyond the loopback opens that to every machine that reaches it.
- `session` (required, at least one): `[[session]]` tables, served in the file's order.

**A session** (`[[session]]`):
- `name` (required): the session's path segment, `/s/<name>/`: a lowercase ASCII letter or digit,
  then lowercase letters, digits and `-`, at most 32; no two sessions share one.
- `project` (required): the path of the session's portal project file, loaded as portal loads it
  (its refusals are portal's, named with the session). The project's own `[server]` address is not
  bound: the session's portal surface is served at `/s/<name>/portal/`.
- `binding` (required): the path of the session's binding file.

## The binding file: keys

A path is relative to the binding file's folder.

**The top level.**
- `format` (required): always `"veredas-binding"`.
- `version` (required): this reader knows `1`.
- `title` (the project's name): the session's name on the screen.
- `layout` (none): the path of the SVG layout. Without one the session has dashboards and no
  layout view.
- `headline` (none): object IDs shown in the top bar, in order.
- `object` (none): `[[object]]` tables.
- `dashboard` (none): `[[dashboard]]` tables.

**An object** (`[[object]]`): something on the plant with a value.
- `id` (required): an ASCII letter, then ASCII letters, digits, `-`, `_` and `.`, at most 64; no two
  objects share one, whatever the case. The layout's element with this `id` shows the object.
- `label` (`id`): the object's name on the screen.
- `value` (required): the series it reads, an inline table:
  - `source` (required): a source of the session's project (a log, an OPC source, a channel).
  - `series` (required): a series that source carries: a capture rule's group (not its key group)
    for a log the rule reads, an OPC tag's or bit's series for an OPC source, a message's field for
    a channel; typed as portal types it (a type's word in any case), and, where several rules or
    messages carry it, numeric only where every one of them types it so.
  - `key` (none): the one key read, for a numeric series whose samples carry keys (a text or a
    time series has no value per key to read); absent, the latest sample whatever its key, or, for
    a marker, one marker per key.
- `unit` (`""`): shown after the value.
- `decimals` (none): the places a number is shown with, 0 to 6; absent, as it reads, at most three
  places, trailing zeros dropped.
- `show` (`"state"`): how the layout shows the object: `state` (its element takes the state's
  class, and its mark and word when not normal), `level` (as `state`, and its element fills from
  the bottom by the value's place in `range`, a shape or a text within its outline, a group its
  box), `value` (as `state`, and the value written over its element) or `move` (its element is a
  marker placed along `path` by the value's place in `range`). A level and a marker need a numeric
  series.
- `range` (`[0, 100]`): the low and the high of a level or a marker, low below high, both finite.
- `path` (required for `move`, refused otherwise): the `id` of the layout element the marker
  follows, a `path`, `line`, `polyline`, `polygon`, `rect`, `circle` or `ellipse`.
- `states` (none): the rules, in order; the first that holds names the state; none holding, the
  state is normal.
- `normal` (`"ok"`): the word of the normal state.
- `stale_after_ms` (none): a freshness bound above 0: with it, the object is stale when no sample of its
  series arrived within that many milliseconds, even while its source is live; for a marker per
  key, a key with no sample within it is no longer shown (absent, after 10000 ms).
- `actions` (none): the writes the object offers, as inline tables.

**A state rule** (an entry of `states`): exactly one comparison, a state and a word.
- `equals`, `above`, `below`, `at_least`, `at_most` (one, required): the value compared; `equals`
  takes a number (compared as numbers) or a string (compared as text, exactly), the others a
  finite number.
- `state` (required): `near`, `warn`, `alarm` or `disabled`; normal is the absence of a rule, and
  stale is no rule's: it is judged on the arrival clock.
- `word` (the state's name): the word shown with the mark.
- `source`, `series`, `key` (the object's): another series the rule compares instead of the
  object's value, by the same rules as `value`'s keys.

The states read with a mark and a word everywhere: normal ●, near ■, warn ■, alarm !, disabled ▨,
stale ◌, and the page's own selected ◆. Stale overrides the rules: an object with no value yet, whose source is away (idle past the
project's window, retrying, ended or failed, or waiting again once it has delivered: an OPC source
reconnecting, a channel whose peer left), or whose value is older than its `stale_after_ms`, is
stale, a last value kept and greyed.

**An action** (an entry of `actions`): a write the side panel offers, posted to portal's write
surface (`POST /s/<name>/portal/api/opc/write`), never written by supervisão itself.
- `label` (required): the button's text.
- `source` (required): an OPC source of the project.
- `tag` (required): the tag's node id as the project's OPC rule writes it (portal names a tag by it), a tag the rule marks writable.
- `value` (required): the value written, as text, coerced by portal to the tag's type.

The page shows an action gated (the warn colour) and disabled while the session's switch is off;
portal answers a post 403 while it is off, whoever sends it.

**A dashboard** (`[[dashboard]]`):
- `name` (required): its name on the screen, unique in the file.
- `columns` (`4`): the grid's columns, 1 to 12.
- `panel` (none): `[[dashboard.panel]]` tables, laid out in order.

**A panel** (`[[dashboard.panel]]`): every panel has a `kind` (required), a `title` (the object's
label for a panel of one object, the table's name for a table's, else `"State over time"`,
`"States"` or `"Levels"`) and a `span` (`1`: the columns it takes, 1 to the
dashboard's `columns`). An entry of `objects` ending in `*` names every object whose ID starts with
what precedes it, in the binding's order, and must name at least one. A window is the last
`minutes` up to now on the store's time axis; `minutes` and `bucket_minutes` are above 0. A
trend, bars, a timeline and levels chart numbers: their objects read numeric series. A column a
panel names is checked against its table's where the project states them (a capture rule's, an OPC
rule's, a message's, each beside `source`, `key`, `index_ms`, `ts`, `arrival_ms` and `seq`); a
view's are the engine's, judged when the panel is computed.
- `value`: a tile with the object's value, unit, state and a sparkline. `object` (required),
  `minutes` (`30`: the sparkline's window), `target` (none: the tile says how far the value is from
  it, in percent).
- `trend`: the object's series over a window with a dashed limit and its breaches marked.
  `object` (required), `minutes` (`60`), `limit` (none): `{ above = x }` or `{ below = x }`; a
  breach is a point past it.
- `bars`: the object's series in buckets of `bucket_minutes` aligned on the clock, the bucket now
  running marked, against a dashed `target`. `object` (required), `minutes` (`60`),
  `bucket_minutes` (`5`), `aggregate` (`"mean"`: `mean`, `sum`, `count`, `min`, `max` or `last`),
  `target` (none).
- `timeline`: per object one bar of its state over the window, each state lasting until the next
  sample, the one holding at the window's start (its last sample within the day before) drawn from
  the start. `objects` (required), `minutes` (`60`). The objects' series must be numeric.
- `grid`: one small cell per object with its value and state. `objects` (required).
- `levels`: one horizontal bar per object, its value's place in the object's `range`, with ticks.
  `objects` (required), `ticks` (none: values in the objects' units).
- `table`: rows of a table of the session, newest first. `table` (required: a capture rule, an OPC
  rule, a channel message or a view of the project), `columns` (none: every column), each
  `{ name, label, bar, decimals, states }` (`name` required; `bar` true draws the value as a bar
  against the column's largest in the page; `states` rules as an object's), `sort` (none:
  `{ column, descending }`, applied to the rows shown, a column of the panel's, or of the table's
  when the panel names none), `limit` (`50`, 1 to 1000). A column's `decimals` are 0 to 6; its
  rules compare its own value.
- `cells`: charts in cells, one row per value of a key column of a table over a window: the
  median, a sparkline of the median per `bucket_minutes`, the distribution from median − `spread`
  to median + `spread` in 20 bins, σ and n. `table` (required), `key` (required), `value`
  (required: a numeric column), `minutes` (`60`), `bucket_minutes` (`5`), `spread` (required,
  above 0). It reads at most the newest 50 000 rows of the window, and says so when it did.

## Refusals

A misspelled key anywhere; a header of another format or version; a session name of another shape
or twice; a project or binding file that cannot be read or does not load (with portal's reason
for a project); an object ID of another shape or twice whatever the case; a source the project
lacks; a series its source does not carry (with the series it does carry); a level or a marker on
a series that is not numeric; places past 6; a freshness bound of 0; a `path` on an object that
does not move, a marker without one, or one naming no element of the layout or an element no
marker can follow; a `range` empty or not finite; a rule with no comparison or two, a state outside
the four, a comparison that is not a finite number; an action whose source is no OPC source of the
project or whose tag no rule marks writable, or whose label is empty; a headline or panel naming no
object, an `objects` pattern matching none; an empty dashboard name, or one twice; columns outside 1
to 12; a panel without a kind, of an unknown kind, or with a key its kind does not take; a span
past the columns; `minutes` or `bucket_minutes` of 0; a target, a limit or a tick that is not a
finite number, a limit naming both sides, an unknown aggregate; a trend, bars, a timeline or levels
over a series that is not numeric; a table the project lacks; a table panel's limit outside 1 to
1000, a column with an empty name, places past 6 or a rule over another series, a sort column the
panel or its table lacks; a column, key or value the table lacks where the project states its
columns, a value that is not numeric; a `spread` not above 0; a layout that cannot be read, is not
an SVG document (its root an `svg` in the SVG namespace), or holds a script, foreign content (an
element outside the SVG namespace), an event handler or a script link (a `javascript:` URL in any
attribute, read as a browser reads a URL): each is refused when the file loads, with its place
named, before any session starts.
