# The catalog file: `veredas-catalog`, version 1

A channel's messages are configured, not programmed (`docs/questions.md` Q8). The channel file
(`docs/channel-schema.md`) says how frames are framed and the link is held; the catalog file
says what the frames carry: each message by name, by the id its segment header (or envelope)
field carries, and by the rules its body is written with and read with.
`veredas_core::telegram::catalog::load` reads it into a `Catalog`; `telegram::compose` writes a
body from named fields and `telegram::capture` reads one back, from the same field list, so a
body a channel sends is a body the same catalog reads. A loaded catalog also answers whether a
message reads a name (a binary field, a payload group, or a repeat's numbered name) and where
its ids ride in a channel file's framing, so a file read against it (the controller file,
`docs/controller-schema.md`) and a channel that speaks it never re-derive its rules. Core knows
no device: a device's catalog is a file like the one below, kept with its channel file, outside
the suite's code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed.

## The worked example: a conversation of three messages

A controller announces an item (a binary body of 32 bytes: the item's number, its lane, a read
flag as one text character, a code of up to 16 characters, a weight), the unit it talks to
answers with a destination (8 bytes: the item, the destination, a flag), and the controller
reports the outcome as a line of text. The ids are carried in the `id` field of the channel
file's worked example's segment header. Bytes are `little-endian`. The `answer` for item 7 to
destination 3 with flag 1 is `07 00 00 00 03 00 01 00`; the `outcome` for item 7 at destination
3 with result `SORTED` is the text `ITEM=7;DEST=3;RESULT=SORTED`, and reads back into those three
fields. `crates/veredas-core/tests/telegram_catalog.rs` loads this very block and checks each of
these statements, and every key statement below, against the reader; it holds a copy of this
text, so an edit here is an edit there, beside the behaviour the sentence is held against.

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
```

## Keys

Each key states in parentheses that it is required, or the value it takes when left out. A
name (of a message, a field, a repeat) is exact: compared as written, with no surrounding
whitespace.

**The top level**.
- `format` (required): always `"veredas-catalog"`.
- `version` (required): this reader knows `1`. A file of another format or an unknown version is
  refused, never guessed; migrating an older version is portal's (P-010).
- `id_field` (required): the name of the frame field that carries a message's id: a segment
  header field in a layout with segments, an envelope field in one without. It must be a field
  the framing leaves to the sender, an opaque one (`docs/channel-schema.md`, "Roles"), never the
  sequence, a length or a count, which a channel fills in itself, and wide enough for every id.
  The catalog knows no framing, so a channel that speaks it asks `Catalog::check_framing`
  before it starts; the same check refuses, in a layout without segments, a message that writes
  an empty body, which the wire carries as a keep-alive, not a message.
- `byte_order` (`"little"`): `"little"` or `"big"`, for every binary field of every body.
- `message` (none): the messages, as `[[message]]` tables; a catalog may be empty.

**A message** (`[[message]]`).
- `name` (required): what the rules and the bench call the message; unique.
- `id` (required): the number the frame carries in `id_field`; unique.
- `len` (none): a fixed body length. Written bodies are padded with `pad` up to it and refused
  beyond it; a received body of another length is not this message and reads as nothing; a
  field that ends past it, or a template whose literal text is longer, is refused when the file
  loads. Without it a body is as long as what was written, and read whatever its length.
- `pad` (`0`): the byte that fills the gaps between fields and the padding to `len`.
- `fields` (none): binary fields at byte offsets, written and read alike.
- `template` (none): a text body written first, `{name}` replaced by that value (the name read
  trimmed) and `{{` and `}}` standing for literal braces; an unclosed brace, a stray `}`, an empty
  `{}` or a `{` inside a placeholder is refused when the file loads. A field written over the
  text overwrites its bytes.
- `payload` (none): a regex over the body as text, read back into its named groups. A message
  with a payload or a repeat and neither a template nor a field can be read but never written.
  A text body padded to `len` is read with its padding, so a pattern anchored at the end does
  not match it.
- `repeat` (none): patterns captured repeatedly as `name1`, `name2`..., counted from 1; no
  numbered name may be a payload group's or another repeat's.

**A field** (an entry of `fields`).
- `name` (required): unique within the message, and never a name the payload's groups or a
  repeat's numbered names would read.
- `at` (required): the byte offset.
- `ty` (required): `U8`, `U16`, `U32`, `I16`, `I32`, `Ascii` or `Hex`. An integer is written from
  decimal text (a sign only where the type takes one) and refused when it does not fit; a
  text is read back without its trailing whitespace.
- `len` (`0`; required for an `Ascii` or `Hex` field): their byte count. Text is padded with NUL
  to it (a NUL inside the text refused) and refused when longer; hex must spell exactly that
  many bytes. No two fields may share a byte.

**A repeat** (an entry of `repeat`).
- `name` (required): the prefix of the numbered names.
- `pattern` (required): the regex captured again and again (group 1, else the whole match).
- `max` (`16`): the most occurrences read; `0` is refused.

Writing a body needs a value for every placeholder and every field, and refuses one that is
missing or does not fit; reading a body that does not satisfy the rules yields nothing, never a
partial map. A misspelled key in any table, an empty or repeated name, a repeated id, a field
ending past the body or past 16 MiB, a bad pattern or a bad template is refused with its place
named.
