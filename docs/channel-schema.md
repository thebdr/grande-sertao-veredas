# The channel file: `veredas-channel`, version 1

A telegram channel is configured, not programmed (`docs/questions.md` Q8, Q9). Its framing and
its link rules live in one plain-text TOML file, versioned like every plain-text file of the
suite (the plan, "Versioned files"). `veredas_core::telegram::schema::load` reads it into a
`ChannelSpec`; the framing engine (`telegram::framing`) and the channel (`telegram::channel`)
run by it. Core knows no device: a device's layout is a file like the one below, kept with that
device's other rules, outside the suite's code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed.

## The worked example: a segmented envelope

A start byte; an 11-byte envelope after it (a class byte, source and destination nodes, a
sequence number, three spare bytes, the data area's length, the segment count); segments of a
2-byte id, a 2-byte length and the body; a 3-byte tail (two spare bytes, a check byte) before
the end byte. A keep-alive is therefore 16 bytes, and one segment `AB` with id `0x0102` and
sequence 7 is `FD 00 01 02 07 00 00 00 06 00 01 00 02 01 02 00 41 42 00 00 00 FE`. Sequence
numbers 1 to 255 after a 0 on connect; a keep-alive probe after 10 s of silence.
`crates/veredas-core/tests/telegram_schema.rs` loads this very block and checks each of these
numbers.

```toml
format = "veredas-channel"
version = 1

[framing]
byte_order = "little"
start = "FD"
end = "FE"
max_frame = 65536
envelope = [
  { name = "class", size = 1 },
  { name = "source", size = 1, value = 1 },
  { name = "destination", size = 1, value = 2 },
  { name = "sequence", size = 1, role = "sequence" },
  { name = "spare", size = 3 },
  { name = "length", size = 2, role = "length" },
  { name = "count", size = 2, role = "count" },
]
segment = [
  { name = "id", size = 2 },
  { name = "length", size = 2, role = "length" },
]
tail = [
  { name = "tail_spare", size = 2 },
  { name = "check", size = 1 },
]

[sequence]
connect = 0
first = 1
last = 255
skip_duplicates = true
keepalive_counts = true

[keepalive]
on_connect = true
idle_ms = 10000

[link]
write_timeout_ms = 2000
outbox_limit_bytes = 16777216
max_peers = 8
```

## Keys

Each key states in parentheses that it is required, or the value it takes when left out.

**The header.** `format` (required: always `"veredas-channel"`) and `version` (required: this
reader knows `1`). A file of another format or an unknown version is refused, never guessed;
migrating an older version is portal's (P-010).

**`[framing]`** (required)
- `byte_order` (`"little"`): `"little"` or `"big"`, for every field that does not say otherwise.
- `start`, `end` (none): delimiter bytes, in hex (`"FD"`, `"02 03"`). Without a start delimiter
  a bad frame is skipped one byte at a time.
- `max_frame` (65536, at most 16777216: a peer may declare a frame this large and its bytes are
  held while they arrive): the largest frame in bytes, delimiters included. Larger frames are
  refused, never truncated.
- `envelope` (required): the fields after the start delimiter, in order.
- `segment` (none): the fields of each segment's header, in order; each header is followed by
  its body. Leave it out for a layout whose data area is one body.
- `tail` (none): the fields after the data area, before the end delimiter.

**A field.** `name` (required; unique across envelope and tail; unique within the segment
header), `size` (required; 1 to 8 bytes), `role` (`"opaque"`), `byte_order` (the framing's,
which it overrides), `value` (0; an opaque field's default when a frame does not name it; a
computed field states none, not even 0).

**A keep-alive** is a frame with no segments; in a layout without `segment`, a frame with an
empty body. The channel delivers it as a keep-alive event that carries the frame, so its fields
are never lost.

**Roles.**
- `"opaque"` (default): carried by name, read into the frame and written from it.
- `"sequence"` (envelope): stamped by the channel, by the `[sequence]` rules.
- `"length"`: in the envelope, the data area's size (every segment, headers included); in a
  segment header, the body's size. Computed on encode.
- `"count"` (envelope): the number of segments. Computed on encode.

A layout must be able to end: without `segment`, the envelope needs a `length`; with
`segment`, its header needs a `length`, and the envelope a `length`, a `count`, or both (both
are checked against each other).

**`[sequence]`** (required with an envelope field of role `"sequence"`, refused without one; an
empty `[sequence]` takes every default)
- `connect` (0): stamped on the connect keep-alive; outside `first..=last`, because the receiver
  tells the connect keep-alive by its number (Q11).
- `first` (1), `last` (255): the cycle frames are stamped with, at send time; a cycle of one
  number cannot skip duplicates and is refused with `skip_duplicates`.
- `skip_duplicates` (true): the receiver skips a frame repeating the last number it saw.
- `keepalive_counts` (true): keep-alives take numbers in the cycle like any frame, and the
  receiver counts a keep-alive's number (other than `connect`) as the last seen. With `false`
  a keep-alive (a probe, or one the application sends, such as an empty acknowledgement in a
  layout without segments) carries the last number stamped without taking one, and the
  receiver ignores it.
  Whether a device's keep-alives consume numbers is that device's (Q9).

**`[keepalive]`** (optional) `on_connect` (true): send a keep-alive (a frame with no segments)
on connect. `idle_ms` (10000): after this long with nothing read and nothing written, send a
probe; 0 never probes.

**`[link]`** (optional) `write_timeout_ms` (2000, at least 1): a peer whose socket takes nothing
for this long is dropped as not reading. `outbox_limit_bytes` (16 MiB, at least one `max_frame`):
bytes queued for one peer beyond what its socket took, a memory bound. `max_peers` (8, at least
1): simultaneous connections when listening.

Every key is checked: a misspelled key, an unknown role, a size out of range, a role in the
wrong part, two fields sharing a role, a computed field given a `value`, a layout that cannot
end, a sequence field without rules or rules without a field, `connect` inside the cycle, a frame limit over the ceiling or
under the fixed bytes, or a zero link setting is refused with its place named.
