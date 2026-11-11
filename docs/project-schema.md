# The project file: `veredas-project`, version 1

A project is what the gateway runs (`docs/veredas-plan.md`, "veredas-portal: the gateway"):
the plant's logs, OPC UA servers and telegram channels it reads, the rules that turn their lines,
polls and messages into tables, the views over those tables, where the session starts and keeps
its store, whether its writes are armed, and where it serves. One TOML file says it all, and the
files a channel needs (core's channel file, `docs/channel-schema.md`, and its catalog,
`docs/catalog-schema.md`) sit beside it. `veredas_portal::project::load` reads the file and every
file it points at, resolves every path against the file's folder, and compiles every rule once,
so a project that loads is a session that can start; the executable `veredas-portal <project.toml>`
starts it (`--check` reads, checks and prints the plan without starting anything;
`--from-v1 <folder>` reads a Veredas 1 project folder into this file and prints it, naming what it
did not carry; `--new-key` and `--secret` keep the secrets file, below). No device is named
anywhere: a plant's project is a folder of files like the ones below, kept outside the suite's
code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a misspelled
key refused, an unknown format or version refused rather than guessed. From this file on, the
header is read by one reader, `veredas_core::schema`, that every file of the suite shares.

## The worked example: a scanner's log family, a controller, and a sorter answered by rule

A scanner writes daily files that rotate on a line count, stamped to the millisecond; a controller
serves a speed and a run flag over OPC UA under a user whose password sits sealed in the secrets
file beside the project, never in this one; the machine of the controller reference listens on
port 5020 and the project dials it, answering every announcement by the reply rule of the channel
clause; a view keeps the heavy scans. The session starts at the day's midnight, keeps its store in
memory, and leaves its writes off; the server listens on the loopback.
`crates/veredas-portal/tests/project.rs` writes the channel file and the catalog beside this
block, loads it, checks every statement below against what it read, and runs the executable's
check on it; it holds a copy of this text, so an edit here is an edit there.

```toml
format = "veredas-project"
version = 1
name = "the press hall"

[session]
start = "midnight"
store = "ram"
writes_armed = false
idle_after_ms = 20000

[server]
bind = "127.0.0.1:8901"

[secrets]
key_env = "VEREDAS_KEY"

[[log]]
id = "scanner"
display_name = "the scanner"
dir = "logs"
pattern = "(?P<date>\\d{8})_scanner(?P<idx>\\d+)\\.log"
date_format = "%Y%m%d"
active = "newest"
timestamp = { pattern = "^(?P<ts>\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\.\\d{3})", formats = ["%Y-%m-%d %H:%M:%S%.3f"] }

[[log]]
id = "press"
file = "press.log"
timestamp = { pattern = "^(?P<ts>\\S+ \\S+)", formats = ["%Y-%m-%d %H:%M:%S", "%d/%m/%Y %H:%M:%S"] }

[[rule]]
name = "scans"
pattern = "SCAN code=(?P<code>\\d+) weight=(?P<weight>[0-9.]+)"
scope = "scanner"
types = { code = "int", weight = "float" }
key = "code"

[[opc]]
source_id = "plc"
display_name = "the controller"
endpoint_url = "opc.tcp://127.0.0.1:4840/"
user = "operator"
secret = "plc"
poll_ms = 500
rules = [
  { name = "plc_main", tags = [
    { node_id = "ns=2;s=speed", series = "speed", type = "float" },
    { node_id = "ns=2;s=run", series = "run", type = "int", writable = true, bits = [ { bit = 0, series = "running" } ] },
  ] },
]

[[channel]]
source_id = "sorter"
display_name = "the sorter"
role = "connect"
host = "127.0.0.1"
port = 5020
channel = "channel.toml"
catalog = "catalog.toml"
replies = [ { on = "announce", send = "answer", fields = [ { name = "item", from = "item" }, { name = "dest", value = 1 }, { name = "flag", value = 1 } ] } ]

[[table]]
name = "heavy"
sql = "SELECT * FROM scans WHERE weight > 20.0"
```

Loaded, this project has two logs, one OPC source with one rule of three columns, one channel
whose catalog has six messages, one capture rule and one view; its session starts at midnight,
in memory, writes off, idle after 20000 ms; its server binds `127.0.0.1:8901`; its secrets sit in
`secrets.toml` beside it, opened with the key the variable `VEREDAS_KEY` holds. Every table name is
taken once across the rules, the OPC rules, the catalog's messages and the views: a second
`scans` anywhere is refused, naming both owners.

## Keys

Every key is spelled as written here; a key the reader does not know is refused with its name.
A relative path (`file`, `dir`, `disk`, `channel`, `catalog`, the secrets' `file`) is resolved
against the project file's folder.

### The header and the name
- `format` (required): always `"veredas-project"`.
- `version` (required): this reader knows `1`. A file of another format or an unknown version is
  refused before the body is read, by the text every file of the suite shares.
- `name` (required): the session's name, as the server and the apps show it.

### `[session]` (optional; its defaults when absent)
- `start` (default live, every log followed from its end): `"midnight"` (the day's midnight on
  the plant's clock), a date-time `"YYYY-MM-DD HH:MM[:SS]"`, a date `"YYYY-MM-DD"`, or
  `{ last_hours = N }` with N at least 1. A start in the past reads a file whole and backfills a
  family from the located line (a family then needs its `timestamp`).
- `store` (default `"ram"`): `"ram"`, gone when the session ends; `{ disk = "<file>",
  memory_limit_mb = 0 }`, a database file pinned by name that the engine pages out of core,
  refused when one already stands at the path; or `{ dir = "<folder>", memory_limit_mb = 0 }`,
  a database file of each run's own in that folder, named from the project's name and the
  session's start (`<name>-<yyyymmdd>-<hhmmss>.duckdb`, the name's other characters `_`), so
  every run leaves its own. `memory_limit_mb` is the engine's ceiling (0 for its default).
- `writes_armed` (default `false`): the master switch. Off, every write path (an OPC write, a
  channel send, a reply rule) is refused before anything reaches the plant; the switch is set in
  process, never over the wire.
- `idle_after_ms` (default `20000`): a live source silent this long reads as idle; at least 1.

### `[server]` (optional: absent, nothing is served)
- `bind` (required): the address `host:port` the server listens on.

### `[secrets]` (optional; required once a source names a `secret`)
- `file` (default `"secrets.toml"`): the sealed secrets file beside the project, in the
  `veredas-secrets` format below, committed with the project. The session reads it when it
  starts and opens it in memory; nothing of it is written back, and no value of it reaches a log,
  a plan, an error or a debug form. A file not there yet (an imported project before its first
  `--secret`) holds no entry: a source naming one is refused by that entry's name.
- `key_env` (required): the name of an environment variable holding the key, 64 hexadecimal
  characters (256 bits), never the key. The variable unset, or not holding a key, refuses the
  session by the variable's name; a key that does not open the file refuses it by the file's path
  and the variable's name. The check needs neither the variable nor the file.

### `[[log]]`: a log source
- `id` (required): the source's id, unique across logs, OPC sources and channels.
- `display_name` (default the id).
- `file`: one growing file. A log is a `file`, or a family (`dir` and `pattern`), never both.
- `dir`, `pattern`: a rotating family in a folder, its members named by the pattern (a regex over
  the file name, matched whole; `(?P<date>…)` and `(?P<idx>…)` order it).
- `date_format` (optional): the chrono format of the pattern's `date` group; a group without
  its format, or a format without the group, is refused.
- `active` (default `"newest"`): which member is being written: `"newest"` by order, or
  `{ fixed = "<name>" }` for a constant name.
- `recursive` (default `false`): scan subfolders too; the pattern then matches the `/`-separated
  path under `dir`.
- `timestamp` (optional): `{ pattern = "…", formats = ["…"] }`, the stamp a line carries: a regex
  whose `ts` group locates it, and the chrono formats to try in order. Without it, lines are
  indexed on their arrival, and a start in the past with a family is refused: the stamps locate it.

### `[[rule]]`: a capture rule, one table named for it
- `name` (required): the table's name, lowercase `[a-z0-9_]`, taken once across every table.
- `pattern` (required): a regex whose named groups are the columns.
- `scope` (default every log): the one log the rule reads.
- `types` (default text for every group): a group's type, `int`, `float`, `text` or `time`.
- `key` (optional): the group that is the identity key (an item, a sensor); it is the `key`
  column, not a value column.

### `[[opc]]`: an OPC UA source, its rules each a table
- `source_id` (required), `display_name` (default the id), `endpoint_url` (required).
- `user` (optional) and `secret` (required with `user`): the name of an entry of the secrets file
  holding the user's password, never the password (`docs/questions.md` Q15); a `secret` needs the
  `[secrets]` table. The session reads the entry when it starts and refuses the source by the
  entry's name when the file has no such entry; the check does not need it.
- `poll_ms` (default `1000`): how often every tag is read, at least 50.
- `rules` (required, at least one): each `{ name, store_on_change = false, tags = [ … ] }`, a tag
  `{ node_id, series, type = "float", writable = false, bits = [ { bit, width = 1, series } ] }` (bits need `type = "int"`),
  as the OPC clause reads them (C-040).

### `[[channel]]`: a telegram channel, each catalog message a table named for the message
- `source_id` (required), `display_name` (default the id).
- `role` (required): `"listen"` (the peer dials in; `bind` default `"0.0.0.0"`) or `"connect"`
  (the project dials; `host` required). `port` (required) is the port either way.
- `channel` (required): the channel file beside the project, `docs/channel-schema.md`.
- `catalog` (required): the catalog beside the project, `docs/catalog-schema.md`.
- `replies` (default none): the reply rules, each `{ on, send, fields = [ … ] }` naming the
  catalog's messages (an unknown one is refused), a field `{ name, from }` copying the trigger's
  field or `{ name, value }` a literal, as the channel clause reads them (C-039).

### `[[table]]`: a view of the project
- `name` (required): the view's name, taken once across every table.
- `sql` (required): its one SELECT over the base tables and the other views (a second
  statement in it is refused, and nothing in it runs).

## The secrets file: `veredas-secrets`, version 1

The values a project must not show (an OPC user's password) sit sealed in this file, beside the
project and committed with it; the key that opens them is held in the environment variable the
project's `[secrets] key_env` names, set where the session runs and nowhere in the repository.

```toml
format = "veredas-secrets"
version = 1

[entries]
plc = "Kx3uDe2bI4YmXjzQ...AQ=="
```

- `format` (required): always `"veredas-secrets"`; `version` (required): this reader knows `1`.
  Another format or version is refused before the body is read, as every file of the suite is.
- `[entries]` (default none): one value per key, the key the entry's name a source's `secret`
  gives, the value the nonce and the ciphertext of ChaCha20-Poly1305 in base64, sealed under the
  project's key with a fresh 96-bit nonce at every writing and the entry's name bound in as the
  associated data, so an entry copied under another name does not open.

The executable writes it, never an editor: `veredas-portal --new-key` prints a fresh key, 64
hexadecimal characters, to stdout and nothing else; `veredas-portal --secret <project.toml> <entry>`
reads the value from stdin to its end (the final line break dropped, LF or CRLF; a bare carriage
return and a line break inside kept),
seals it under the key the project's `key_env` variable holds, writes or replaces the entry in the
project's secrets file (created, stamped, when there is none), prints `sealed` with the entry's
name and the file's path, and shows the value nowhere. A project that has no `[secrets]` table, a
variable unset or not a key, a key that does not open the entries already in the file (nothing is
written then), and a file that is not a secrets file (named by its path, with the header text
every file of the suite shares) are refused by name with exit code 2; a stray argument, or an
option given twice or where a name should be, is a usage error, exit code 1.
