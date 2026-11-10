# The bench file: `veredas-bench`, version 1

A bench is the plant a consumer meets before the real one (`docs/veredas-plan.md`,
"veredas-maquinar: the bench"): the log emitters, the OPC UA servers and the telegram channels
it talks to, each configured, none programmed. One bench file lists them all by name, and the
device's own files sit beside it: core's channel file (`docs/channel-schema.md`), the catalog
(`docs/catalog-schema.md`) and, when a controller rides a channel, its rules file
(`docs/controller-schema.md`). `veredas_maquinar::bench::load` reads the bench file and every
file it points at and checks each part as its own module checks it, so a bench file that loads
is a bench every part of which can start; `Bench::start` then runs every part behind one handle,
emitters first, then servers, then channels, each kind in the file's order (TOML groups a
kind's tables under one key, so a file keeps no order across kinds), and the executable
`veredas-maquinar <bench.toml>` runs it from the command line (`--check` reads, checks and
prints the plan without starting anything). No device is named anywhere: a
plant's bench is a folder of files like the ones below, kept outside the suite's code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed.

## The worked example: a scanner's log, a controller's OPC UA server, and a machine talking to its unit

A scanner writes a line every 200 ms into a daily file that rotates every 1000 lines; a
controller serves two tags over OPC UA on a port the machine chooses, a speed drawn between 1.0
and 2.0 every 500 ms and a run flag a client may write, and reacts to the flag written true by
setting the speed to 2.0 after 100 ms; and the machine of the controller file's worked example
listens on port 5020 for the unit of that example, which dials it. The channel file, the
catalog and the two rules files are those of the channel and controller references, written
beside this file under the names it gives; the emitter writes under `logs`, beside it too.
`crates/veredas-maquinar/tests/bench_file.rs` writes those files, loads this block, starts it,
sees every part do its work, and runs the executable on it; it holds a copy of this text, so an
edit here is an edit there.

```toml
format = "veredas-bench"
version = 1
seed = 7
out_dir = "logs"

[[emitter]]
name = "scanner"
template = "{ts} SCAN code={code} weight={weight}"
ts_format = "%Y-%m-%d %H:%M:%S%.3f"
values = [ { name = "code", low = 1000, high = 9999 }, { name = "weight", low = 0.1, high = 30.0, decimals = 2 } ]
interval_ms = 200
file = "%Y%m%d_scanner{n}.log"
max_lines = 1000

[[opc]]
name = "plc"
port = 0
tags = [
  { id = "speed", type = "float", value = "1.5", randomize = { low = 1.0, high = 2.0, decimals = 2, interval_ms = 500 } },
  { id = "run", type = "bool", access = "read-write" },
]
reactions = [ { on_write = "run", equals = "true", tag = "speed", then = [ { after_ms = 100, set = "2.0" } ] } ]

[[channel]]
name = "machine"
role = "listen"
port = 5020
channel = "channel.toml"
catalog = "catalog.toml"
controller = "machine.toml"

[[channel]]
name = "unit"
role = "connect"
port = 5020
channel = "channel.toml"
catalog = "catalog.toml"
controller = "unit.toml"
messages = [ { message = "status", fields = [ { name = "running", value = 1 }, { name = "speed", value = 50 } ] } ]
```

## Keys

Each key states in parentheses that it is required, or the value it takes when left out. Every
part has a name, exact, with no surrounding whitespace, and no two parts share one. A path is
relative to the bench file's folder.

**The top level**.
- `format` (required): always `"veredas-bench"`.
- `version` (required): this reader knows `1`. A file of another format or an unknown version is
  refused, never guessed.
- `seed` (none): the seed of every part's draws, for a run that repeats: each emitter without a
  seed of its own draws from this seed plus its index, each channel and its controller from
  this seed; the clock when absent.
- `out_dir` (`"."`): where the emitters write, created when there is an emitter.
- `emitter` (none): `[[emitter]]` tables.
- `opc` (none): `[[opc]]` tables.
- `channel` (none): `[[channel]]` tables.

**An emitter** (`[[emitter]]`): a log a plant would write, line by line into rotating files.
- `name` (required): the part's name.
- `template` (required): the line, `{ts}` the timestamp and `{name}` a value, anything else
  verbatim.
- `ts_format` (required): the `chrono` format `{ts}` renders with.
- `values` (none): the values drawn fresh for every line, as inline tables.
- `interval_ms` (required): milliseconds between lines; `0` writes as fast as it can.
- `file` (required): the file name, `chrono` specifiers and `{n}` for the rotation index; a name
  that is empty, has a path separator or is reserved by the platform is refused.
- `max_lines` (`0`): rotate to the next index after this many lines; `0` never rotates on
  count.
- `encoding` (`"utf-8"`): `"utf-8"` or `"utf-16le"`, the latter with a byte-order mark.
- `seed` (none): a fixed seed replays the same values; absent, the bench's seed plus the
  emitter's index, else the clock.

**A value** (an entry of `values`): drawn within `[low, high]`, rendered to `decimals`.
- `name` (required): the placeholder's name.
- `low` (required), `high` (required): finite, `low` not above `high`.
- `decimals` (`0`): the places rendered, at most 15.

**An OPC UA server** (`[[opc]]`): a controller's tags on a real endpoint.
- `name` (required): the part's name.
- `bind` (`"127.0.0.1"`): the address the server binds and advertises, never `0.0.0.0`.
- `port` (`4855`): `0` lets the machine choose a free port.
- `user` (none): a user the server accepts by name and password; anonymous is always accepted.
- `max_nodes_per_read` (none): the most nodes one read may carry; absent, the stack's own limit.
- `max_sessions` (none): the most sessions held at once.
- `tags` (none): the tags, as inline tables.
- `reactions` (none): the reactions, as inline tables.

**A user** (the value of `user`).
- `name` (required), `password` (required): shown in no status or log.

**A tag** (an entry of `tags`).
- `id` (required): the string identifier in the bench's namespace; the node id is
  `ns=<index>;s=<id>`.
- `name` (`id`): the browse and display name a client sees.
- `type` (required): `int` (an Int32), `float` (a Double), `bool` or `text`.
- `access` (`"read"`): `read`, `write` (a command the controller takes but does not report) or
  `read-write`.
- `value` (the type's zero): the initial value as text, parsed per the type.
- `randomize` (none): a fresh value within a range on an interval; a text tag takes none.
- `ramp` (none): a value stepping through a range on an interval and wrapping, for plant
  motion (a place along a loop, a level filling); an int or a float, never with `randomize`.

**A draw** (the value of `randomize`).
- `low` (`0`), `high` (`0`): the range; an int draws the whole numbers in it, a bool flips.
- `decimals` (`0`): a float's places.
- `interval_ms` (`1000`): between draws; at least the server's tick of 20 ms.

**A ramp** (the value of `ramp`): each step adds `step` to what the tag holds, wrapped by the
range's width while past `high`; a value outside the range (a client's write, a plant-side set)
starts again at `low`, one inside it moves the ramp on from there.
- `low` (required), `high` (required): the range, finite, `low` below `high`, its width finite;
  the tag's initial value inside it.
- `step` (`1`): above 0 and at most the range's width (with places, counted in their units); an
  int's range and step are whole numbers.
- `decimals` (none): a float's places, which a float ramp names (an int takes none): every step is
  counted in units of the last place, so the ramp reaches its high exactly, and `low`, `high` and
  `step` must be whole numbers of those units, small enough to count exactly (at most 2^51 of
  them: with fifteen places, a range within ±2.25).
- `interval_ms` (`1000`): between steps; at least the server's tick of 20 ms.

**A reaction** (an entry of `reactions`): what the plant does when a client writes a tag.
- `on_write` (required): the tag watched.
- `equals` (none): the value that fires it, as text parsed per the tag's type; absent, any
  change fires it.
- `tag` (`on_write`): the tag the steps set.
- `then` (required): the steps, as inline tables, each at its delay from the moment of firing.

**A step** (an entry of `then`).
- `after_ms` (`0`): the delay.
- `set` (required): the value set, as text parsed per the target tag's type.

**A channel** (`[[channel]]`): one telegram link, listening or dialling.
- `name` (required): the part's name.
- `role` (required): `listen` (wait for the peer to connect) or `connect` (dial it).
- `host` (`"127.0.0.1"`): the address to bind, or the host to dial.
- `port` (required): the port to bind or dial.
- `channel` (required): the path of core's channel file.
- `catalog` (required): the path of the catalog file; its ids must ride in the channel file's
  framing, as the catalog's own check says (`docs/catalog-schema.md`, `id_field`).
- `controller` (none): the path of the controller rules file, read against the catalog; none
  for a channel that only sends its own messages.
- `messages` (none): the channel's own messages, as inline tables.

**A message** (an entry of `messages`): a message of the catalog the channel sends on an
interval, or on demand.
- `message` (required): its name in the catalog.
- `every_ms` (`0`): the interval; `0` is on demand only.
- `fields` (none): its fields, one for every name the message writes, each a field of the
  controller file's reference (`docs/controller-schema.md`, "A field") with the sources a plain
  channel has: no `from`, no `register`, no `decide`.

A misspelled key in any table, a part without a name or with a name another part has, an
emitter's file name or timestamp format that is bad, a value's range that is empty or not
finite, anything the OPC UA server's own checks refuse, a file a channel points at that cannot be
read or does not load, an `id_field` that does not ride in the framing, a controller file that
does not load against the catalog, a message the catalog lacks or whose fields leave a name
unwritten or use a source a plain channel lacks, is refused when the bench file loads, with the
place named, before anything binds, opens or spawns. A part that cannot start (a port taken, a
folder that cannot be created) stops the parts started before it and is named.
