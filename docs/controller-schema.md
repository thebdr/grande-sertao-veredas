# The controller file: `veredas-controller`, version 1

What a controller does on its wire is configured, not programmed (`docs/questions.md` Q8). The
channel file (`docs/channel-schema.md`) frames the link and the catalog file
(`docs/catalog-schema.md`) names the messages; the controller file says what the bench's
simulated controller does with them: the items it announces at a rate and awaits an answer
for, the outcome it reports when an item's travel ends, the answers it gives to messages, the
commands it obeys, the messages it sends on an interval, and the destinations it serves, each
with a state word of named bits that may refuse an item. `veredas_maquinar::controller::load`
reads it against the catalog its messages come from, into `Rules`; a `Controller` runs them,
pure and tick-driven, and a bench channel drives it from the wire. The same file format plays
either side of a conversation: the machine that announces, or the stand-in for the unit that
answers it. No device is named anywhere: a device's rules are a file like the ones below, kept
with its channel and catalog files, outside the suite's code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed.

## The worked example: a machine, the unit it talks to, and their catalog

The catalog below extends the catalog file's worked example with three messages: a `command`
the machine obeys (id 24), a `heartbeat` it sends per destination (id 25) and a `status` line
of its registers (id 26). The machine announces an item every 100 ms while a peer is connected
and its `running` register is not 0; one item in 10 carries the fault `noread` (its `read` flag
`N` and an empty `code`). It awaits an `answer` keyed by `item`, whose `dest` names the
destination: a `flag` of 2 sends the item round again after a lap of 2000 ms, a `flag` of 3
parks it, anything else delivers it, and 500 ms later the machine reports the `outcome`, drawn
from a mix of 9 `sorted` to 1 `lost`, unless the destination's state forces one: a destination
with its `full` bit set forces `FULL` and the item comes round again, one with `fault` set
forces `FAULT`. Destinations 1 to 4 rest with no bit set; destination 4 starts `full`. The
`command` message's `code` sets the registers: 1 starts the machine, 0 stops it, 7 sets `speed`
to 70. A `heartbeat` goes out every 1000 ms for each destination with its state word, and a
`status` line every 5000 ms with the registers.

```toml
format = "veredas-catalog"
version = 1
id_field = "id"
byte_order = "little"

[[message]]
name = "announce"
id = 21
len = 32
fields = [
  { name = "item", at = 0, ty = "U32" },
  { name = "lane", at = 4, ty = "U8" },
  { name = "read", at = 5, len = 1, ty = "Ascii" },
  { name = "code", at = 8, len = 16, ty = "Ascii" },
  { name = "weight", at = 24, ty = "U16" },
]

[[message]]
name = "answer"
id = 22
len = 8
fields = [
  { name = "item", at = 0, ty = "U32" },
  { name = "dest", at = 4, ty = "U16" },
  { name = "flag", at = 6, ty = "U8" },
]

[[message]]
name = "outcome"
id = 23
template = "ITEM={item};DEST={dest};RESULT={result}"
payload = "^ITEM=(?P<item>\\d+);DEST=(?P<dest>\\d+);RESULT=(?P<result>\\w+)$"

[[message]]
name = "command"
id = 24
len = 2
fields = [
  { name = "code", at = 0, ty = "U8" },
]

[[message]]
name = "heartbeat"
id = 25
len = 4
fields = [
  { name = "dest", at = 0, ty = "U16" },
  { name = "state", at = 2, ty = "U16" },
]

[[message]]
name = "status"
id = 26
template = "RUN={running};SPEED={speed}"
payload = "^RUN=(?P<running>\\d+);SPEED=(?P<speed>\\d+)$"
```

The machine:

```toml
format = "veredas-controller"
version = 1
seed = 7

[registers]
running = 1
speed = 50

[destinations]
list = [1, 2, 3, 4]
bits = { full = 1, fault = 2 }
resting = []
initial = { "4" = ["full"] }
blocking = [
  { bit = "full", set = { result = "FULL" }, requeue = true },
  { bit = "fault", set = { result = "FAULT" } },
]

[items]
message = "announce"
interval_ms = 100
key = "item"
gate = "running"
lap_ms = 2000
fields = [
  { name = "item", counter = { first = 1 } },
  { name = "lane", cycle = [1, 2] },
  { name = "read", value = "Y" },
  { name = "code", template = "PKG{item}" },
  { name = "weight", range = [100, 900] },
]
faults = [
  { name = "noread", percent = 10, set = { read = "N", code = "" } },
]

[items.reply]
message = "answer"
key = "item"
destination = "dest"
when = [
  { field = "flag", equals = 2, action = "requeue" },
  { field = "flag", equals = 3, action = "park" },
]

[items.outcome]
message = "outcome"
after_ms = 500
fields = [
  { name = "item", from = "item" },
  { name = "dest", from = "dest" },
  { name = "result", value = "SORTED" },
]
mix = [
  { name = "sorted", weight = 9 },
  { name = "lost", weight = 1, set = { result = "LOST" } },
]

[[commands]]
on = "command"
field = "code"
when = [
  { equals = 1, set = { running = 1 } },
  { equals = 0, set = { running = 0 } },
  { equals = 7, set = { speed = 70 } },
]

[[cyclic]]
message = "heartbeat"
every_ms = 1000
per_destination = true
fields = [
  { name = "dest", from = "destination" },
  { name = "state", from = "state" },
]

[[cyclic]]
message = "status"
every_ms = 5000
fields = [
  { name = "running", register = "running" },
  { name = "speed", register = "speed" },
]
```

The unit answers every `announce` at once with an `answer` for the same `item`: an item whose
`code` is empty goes to destination 0, any other to destination 1, 2 or 3 in the shares 2, 1
and 1, the same destination again for the same `code`, and to 9, the reject, when every one of
those is full. The unit keeps its own `[destinations]`, so the bench may mark one `full` on it
too and the spread then avoids it. The `flag` is 1.

```toml
format = "veredas-controller"
version = 1
seed = 11

[destinations]
list = [1, 2, 3]
bits = { full = 1 }
blocking = [ { bit = "full" } ]

[[answers]]
on = "announce"
send = "answer"
fields = [
  { name = "item", from = "item" },
  { name = "dest", decide = { rules = [ { field = "code", pattern = "(empty)", value = 0 } ], spread = [ { value = 1, weight = 2 }, { value = 2, weight = 1 }, { value = 3, weight = 1 } ], reject = 9, sticky = "code" } },
  { name = "flag", value = 1 },
]
```

`crates/veredas-maquinar/tests/controller_rules.rs` loads these three blocks and checks each of
these statements, and every key statement below, against the reader; it holds a copy of this
text, so an edit here is an edit there, beside the behaviour the sentence is held against.
`crates/veredas-maquinar/tests/controller.rs` runs the two files as the engine, tick by tick,
and then against each other over a real socket.

## Keys

Each key states in parentheses that it is required, or the value it takes when left out. A
name (of a register, a bit, a field, a fault, an outcome) is exact, with no surrounding
whitespace; a message name is one of the catalog's. A value compared with a field's (a
disposition's or a setting's `equals`, a rule's `pattern`, a reply's key) is compared as text,
the field's value trimmed. A field a disposition, a rule, a command or a `sticky` key names is
looked up when the message comes: a name the message does not carry simply never matches; only
an empty name is refused when the file loads.

**The top level**.
- `format` (required): always `"veredas-controller"`.
- `version` (required): this reader knows `1`. A file of another format or an unknown version is
  refused, never guessed.
- `seed` (none): the seed of the controller's draws, for a run that repeats; the clock when
  absent. A seed the bench gives the controller wins over the file's.
- `registers` (none): a table of named integers, the controller's memory: read by a field's
  `register` and by the items' `gate`, set by a command and by the bench; every register a
  file names must be in this table.
- `destinations` (none): the `[destinations]` table.
- `items` (none): the `[items]` table.
- `answers` (none): `[[answers]]` tables.
- `commands` (none): `[[commands]]` tables.
- `cyclic` (none): `[[cyclic]]` tables.

**The destinations** (`[destinations]`): what the controller serves, each with a state word.
- `list` (required): the destination numbers, at least one, none twice.
- `bits` (none): the named bits of the state word, each one bit (a power of two), no bit named
  twice.
- `resting` (none): the bits a destination has at the start when `initial` does not name it.
- `initial` (none): bits set at the start, by destination number written as text, exactly as
  the number prints (`"2"`, never `" 2"`, `"02"` or `"+2"`); a number not in the list is refused.
- `blocking` (none): the bits that refuse an item, as inline tables, in order of precedence:
  the first entry whose bit is set in a destination's word decides; a bit blocks once.

**A blocking bit** (an entry of `blocking`).
- `bit` (required): a named bit.
- `set` (none): the outcome fields it forces, by name (one the outcome does not render, or any
  when there is no outcome, is refused), overriding what the fields rendered.
- `requeue` (`false`): the item comes round again after a lap instead of ending.

**The items** (`[items]`): what the controller announces and awaits an answer for.
- `message` (required): the announcement, a message of the catalog.
- `interval_ms` (required): the announcement interval; `0` is refused. An item is announced
  only while a peer is connected (announced into nothing it would only time out) and the gate
  register, if any, is not 0, a requeued item coming round again included; each is due an
  interval after the last was due, and a pass of the engine announces every item due since the
  last pass, at most twenty (a longer stall skips the rest, keeping the phase); when announcing
  resumes, the first item comes at once.
- `key` (required): the announced field that identifies the item; one of the `fields`.
- `gate` (none): a register that must be non-zero for the controller to announce.
- `lap_ms` (`30000`): how long a requeued item takes to come round again; `0` is refused.
- `fields` (required): the announced fields, one for every name the message writes.
- `faults` (none): named faults, as inline tables.
- `reply` (none): the `[items.reply]` table.
- `outcome` (none): the `[items.outcome]` table; it needs a `reply` to follow.

**A fault** (an entry of `faults`): this share of items has these fields set instead.
- `name` (required): the fault's name, unique; the bench's knob and the status use it.
- `percent` (`0`): the share of announced items, 0 to 100, drawn item by item, fault by fault.
- `set` (required): announced fields by name and the value each takes.

**The reply** (`[items.reply]`): the answer awaited for an item.
- `message` (required): a message of the catalog. An answer for an item not awaited is
  counted and dropped.
- `key` (the items' `key`): the reply's field holding the item's key: a name the reply reads,
  as the catalog says (a binary field, a payload group, a repeat's numbered name).
- `destination` (none): the reply's field naming the destination, a name the reply reads;
  required when there is an outcome and there are destinations. A destination not in the list is counted and the item
  still travels.
- `when` (none): dispositions by a field's value, in order, as inline tables; the first match
  decides, else the item is delivered.

**A disposition** (an entry of `when`).
- `field` (required): the reply's field.
- `equals` (required): its value, compared as text, trimmed.
- `action` (required): `deliver` (the item travels and ends in an outcome, or ends delivered
  without one), `requeue` (round again after a lap) or `park` (asked about no more).

**The outcome** (`[items.outcome]`): what the controller reports when an item's travel ends.
- `message` (required): a message of the catalog.
- `after_ms` (`0`): the travel time from the answer to the report.
- `fields` (required): the outcome's fields, one for every name the message writes; they read
  the item's announced fields and its reply's, the reply's trimmed.
- `mix` (none): the weighted outcomes, as inline tables, drawn when the destination's state
  forces none; with none, or none with a weight above 0, the item ends `delivered`.

**A mixed outcome** (an entry of `mix`).
- `name` (required): the outcome's name, unique; the status counts items by it.
- `weight` (required): its share, a finite number of 0 or more.
- `set` (none): the fields it forces, by name (one the outcome does not render is refused),
  overriding what the fields rendered.
- `requeue` (`false`): the item comes round again after a lap instead of ending.

**An answer** (`[[answers]]`): a message sent in answer to another, to the peer it came from.
- `on` (required): the message answered, of the catalog.
- `send` (required): the message sent, of the catalog.
- `after_ms` (`0`): the delay before it goes.
- `fields` (required): the fields sent, one for every name the message writes; they read the
  incoming message's fields. An answer whose field cannot render (a decision not made, a name
  not there) is not sent, and counted.

**A command** (`[[commands]]`): an incoming message whose field's value sets registers.
- `on` (required): the message, of the catalog.
- `field` (required): the field read.
- `when` (required): settings by value, at least one, as inline tables; a value none matches
  sets nothing.

**A setting** (an entry of a command's `when`).
- `equals` (required): the field's value, compared as text, trimmed.
- `set` (required): registers by name and the value each takes.

**A cyclic message** (`[[cyclic]]`): a message sent on an interval, to every peer.
- `message` (required): a message of the catalog.
- `every_ms` (required): the interval; `0` is refused. Each is due an interval after the last
  was due, and sent at most once a pass of the engine: a state message is not worth a burst.
- `per_destination` (`false`): one message per destination each time, its fields reading
  `destination` (the number) and `state` (the word); it needs `[destinations]`.
- `fields` (required): the fields sent, one for every name the message writes; they read the
  registers.

**A field** (an entry of any `fields`): a named value and where it comes from, exactly one of
`value`, `range`, `counter`, `cycle`, `template`, `from`, `register`, `now` or `decide`. Fields
render in order, each later one seeing the earlier ones; every value is text on the wire.
- `name` (required): the field's name, unique within its list.
- `value` (one source): a constant, a number, a flag or text.
- `range` (one source): `[low, high]`, finite, `low` not above `high`: a draw within it, whole
  when `decimals` is 0 (from the ceiling of `low` to the floor of `high`), else to `decimals`
  places.
- `decimals` (`0`): the places of a `range` draw, at most 15.
- `counter` (one source): a number counting up from `first`, back to `first` after `last`.
- `cycle` (one source): the next of these values, round and round; at least one.
- `template` (one source): text with `{name}` placeholders of the earlier fields and of the
  context; a name not there is a value missing, and the message is not sent.
- `from` (one source): an earlier field of the list, else a field of the context: the incoming
  message's for an answer, the item's and its reply's for an outcome, `destination` or `state`
  for a cyclic message per destination; a plain channel's own message has no context. A name
  not there is a value missing, and the message is not sent.
- `register` (one source): a register's value; a plain channel's own message has none.
- `now` (one source): the wall clock in this `chrono` format.
- `decide` (one source): a destination decided by rules, in an answer only.

**A counter** (the value of `counter`).
- `first` (`1`): where it starts.
- `last` (none): where it wraps back to `first`, not below `first`; without it, at the top of
  a 64-bit number.

**A decision** (the value of `decide`): first the remembered decision for the key, then the
first matching rule, then a draw over the spread, each skipped when its value names a
destination blocked right now, then the reject value, never skipped; with nothing left, no
decision. At least one of `rules`, `spread` and `reject` is given.
- `rules` (none): first-match rules, as inline tables.
- `spread` (none): weighted values, as inline tables; a value with a weight of 0 is never
  drawn.
- `reject` (none): the value when no rule matches and nothing can be drawn.
- `sticky` (none): the context field whose value keys a remembered decision: the same key
  decides the same again, while that value is not blocked; a reject is never remembered; the
  newest 10000 keys are kept.

**A rule** (an entry of `rules`).
- `field` (required): the context field it looks at.
- `pattern` (required): a regex over the field's value, trimmed; `(empty)` matches a field that
  is absent or blank.
- `value` (required): the decision.

**A share** (an entry of `spread`).
- `value` (required): the decision.
- `weight` (required): its share, a finite number of 0 or more.

A field with no source or more than one, a range that is empty, not finite or, at 0 decimals,
holding no whole number or reaching outside the 64-bit integers, a counter ending before it
starts, an empty cycle, a bad template, a source its place does not take, a register
that does not exist, a decision with a bad pattern or nothing to decide with, a list that leaves
one of its message's names unwritten, a message the catalog does not hold, an interval or lap of
0, a key no field announces, a fault's share over 100 or a fault setting a field that is not
announced, a reply key or destination the reply does not read, an outcome without a
reply, a forced field no outcome field renders or forced with no outcome at all, a weight that
is not finite or below 0, an empty
field name in a disposition, a rule, a command or a `sticky` key, a command with nothing to
set or setting a register that does not exist, a destination list that is empty or repeats a number, a bit that
is not one bit or named twice, a resting or initial or blocking bit that is not named, an
initial destination not in the list, a bit blocking twice, and a misspelled key in any table are
refused when the file loads, with the place named.
