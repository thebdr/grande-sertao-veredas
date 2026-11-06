# The catalog file: `veredas-catalog`, version 1

A channel's messages are configured, not programmed (`docs/questions.md` Q8). The channel file
(`docs/channel-schema.md`) says how frames are framed and the link is held; the catalog file
says what the frames carry: each message by name, by the id its segment header (or envelope)
field carries, and by the rules its body is written with and read with.
`veredas_core::telegram::catalog::load` reads it into a `Catalog`; `telegram::compose` writes a
body from named fields and `telegram::capture` reads one back, from the same field list, so a
body a channel sends is a body the same catalog reads. Core knows no device: a device's catalog
is a file like the one below, kept with its channel file, outside the suite's code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed.

## The worked example: a conversation of three messages

A controller announces an item (a binary body of 32 bytes: the item's number, its lane, a read
flag as one text character, a code of up to 16 characters, a weight), the unit it talks to
answers with a destination (8 bytes: the item, the destination, a flag), and the controller
reports the outcome as a line of text. The ids are the segment ids of the channel file's worked
example, carried in its `id` field. Bytes are little-endian. The `answer` for item 7 to
destination 3 with flag 1 is `07 00 00 00 03 00 01 00`; the `outcome` for item 7 at destination
3 with result `SORTED` is the text `ITEM=7;DEST=3;RESULT=SORTED`, and reads back into those three
fields. `crates/veredas-core/tests/telegram_catalog.rs` loads this very block and checks each of
these statements.

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

Each key states in parentheses that it is required, or the value it takes when left out.

**The header.** `format` (required: always `"veredas-catalog"`) and `version` (required: this
reader knows `1`). A file of another format or an unknown version is refused, never guessed;
migrating an older version is portal's (P-010).

- `id_field` (required): the name of the frame field that carries a message's id: a segment
  header field in a layout with segments, an envelope field in one without.
- `byte_order` (`"little"`): `"little"` or `"big"`, for every binary field of every body.

**`[[message]]`** (none required; a catalog may be empty)
- `name` (required; unique): what the rules and the bench call the message.
- `id` (required; unique): the number the frame carries in `id_field`.
- `len` (none): a fixed body length. Written bodies are padded with `pad` up to it and refused
  beyond it; a received body of another length is not this message and reads as nothing; a
  field that ends past it is refused when the file loads. Without it a body is as long as what
  was written, and read whatever its length.
- `pad` (0): the byte that fills the gaps between fields and the padding to `len`.
- `fields` (none): binary fields at byte offsets, written and read alike. A field: `name`
  (required; unique within the message), `at` (required: the offset), `ty` (required: `U8`,
  `U16`, `U32`, `I16`, `I32`, `Ascii`, `Hex`), `len` (0; the byte count of an `Ascii` or `Hex`
  field: text is padded with NUL to it and refused when longer, hex must spell exactly that many
  bytes; 0 leaves either open-ended, written as given). An integer is written from decimal text
  and refused when it does not fit its type.
- `template` (none): a text body written first, `{name}` replaced by that value and `{{` and
  `}}` standing for literal braces; an unclosed brace, a stray `}` or an empty `{}` is refused
  when the file loads. A field written over the text overwrites its bytes.
- `payload` (none): a regex over the body as text, read back into its named groups.
- `repeat` (none): patterns captured repeatedly as `name1`, `name2`...: `name`, `pattern`,
  `max` (16).

Writing a body needs a value for every placeholder and every field, and refuses one that is
missing or does not fit; reading a body that does not satisfy the rules yields nothing, never a
partial map. A misspelled key in any table, an empty or repeated name, a repeated id, an empty
or repeated field name, a field ending past the body or past 16 MiB, a bad pattern or a bad
template is refused with its place named.
