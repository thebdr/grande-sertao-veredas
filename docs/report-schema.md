# The acceptance report: `veredas-report`, version 1

Every run of a test (`docs/test-schema.md`) ends in an acceptance report, whatever its verdict
(`docs/comissionador-plan.md`, "Rules"). The engine keeps each run in a folder of its own,
`runs/<id, lowercased>/<nnnn>/`, numbered from `0001` in the order the runs were kept (`10000`
after `9999`): the exact text of the test that ran (`test.test`), every function file it used
(`functions/<n>.fn`), the rows of every table it read over its window (`evidence/<table>.csv`),
and `report.toml`, written last, every file on the disk before it, so a folder that holds a report
holds a whole run, a power cut included; once the
report has been changed, `report.lock` too, an empty file the engine locks while it reads, judges
and writes the report, so two changes never cross. `veredas_comissionador::record` keeps a run
(`keep`), reads a report back (`read`), lists a test's runs newest first, each with its report or
why it cannot be read (`list`), sets the tester's note (`note`) and signs a report (`sign`). The report is the record; a PDF or a page is a
drawing of it, Oficina's run-and-report screen among them (`docs/questions.md` Q312, taken
meanwhile: the record in TOML, anyone signing by a name typed).

Every plain-text file of Veredas 2 states its schema (`docs/questions.md` Q10): the report opens
with `format` and `version`, read by core's one header reader, and a key the format does not know
is refused rather than passed over.

## The worked example: the reference's test, passed live

The report the engine keeps for the worked example of `docs/test-schema.md`, run live against the
bench's machine through a portal session named `the press hall`: seven steps, one criterion, three
values reported, two sends, the rows of the two tables the test reads, a tester's note and one
signature. The times are the plant's clock on the store's axis, to the millisecond.
`crates/veredas-comissionador/tests/record.rs` loads this block, reads it as a report, holds every
statement below against what the engine writes for a real run of the same test, and keeps a copy
of this text, so an edit here is an edit there.

```toml
format = "veredas-report"
version = 1
note = "Destination 4 held full throughout; rerouting as specified."

[test]
id = "CHK-012"
title = "Destination 4 full, items come round"
file = "test.test"

[[test.functions]]
used_as = "functions/routing.fn"
file = "functions/1.fn"

[run]
number = 1
mode = "live"
session = "the press hall"
from = "2026-11-22 14:02:11.120"
to = "2026-11-22 14:02:18.311"
verdict = "passed"
decided_at = "summary"
reason = "`summary` passed the run"

[[step]]
step = "start"
attempt = 1
outcome = "pass"
detail = "sorter.command(code: 1): sent"
from = "2026-11-22 14:02:11.120"
to = "2026-11-22 14:02:11.128"

[[step]]
step = "running"
attempt = 1
outcome = "pass"
detail = "status.running == 1 held after 4605ms"
from = "2026-11-22 14:02:11.128"
to = "2026-11-22 14:02:15.733"

[[step]]
step = "route"
attempt = 1
outcome = "pass"
detail = "delivered 0 · rerouted 20"
from = "2026-11-22 14:02:15.733"
to = "2026-11-22 14:02:16.721"

[[step]]
step = "ratio"
attempt = 1
outcome = "pass"
detail = "margin = 0.050000000000000044, ratio = 1"
from = "2026-11-22 14:02:16.721"
to = "2026-11-22 14:02:16.721"

[[step]]
step = "verdict"
attempt = 1
outcome = "pass"
detail = "1 matched >= 0.95 -> release"
from = "2026-11-22 14:02:16.721"
to = "2026-11-22 14:02:16.721"

[[step]]
step = "release"
attempt = 1
outcome = "pass"
detail = "sorter.command(code: 0): sent"
from = "2026-11-22 14:02:16.721"
to = "2026-11-22 14:02:18.311"

[[step]]
step = "summary"
attempt = 1
outcome = "pass"
detail = "3 reported"
from = "2026-11-22 14:02:18.311"
to = "2026-11-22 14:02:18.311"

[[criterion]]
step = "verdict"
attempt = 1
text = "ratio: >= 0.95 -> release; 0.85 .. 0.95 -> retry route max 1; else -> fail_log"
measured = ["ratio = 1"]
result = ">= 0.95 -> release"

[[reported]]
step = "summary"
attempt = 1
text = "ratio = 1"
note = false

[[reported]]
step = "summary"
attempt = 1
text = "margin = 0.050000000000000044"
note = false

[[reported]]
step = "summary"
attempt = 1
text = "rerouted = 20"
note = false

[[write]]
step = "start"
attempt = 1
text = "sorter.command(code: 1)"
sent = true
answer = "sent"
ok = true

[[write]]
step = "release"
attempt = 1
text = "sorter.command(code: 0)"
sent = true
answer = "sent"
ok = true

[[evidence]]
table = "outcome"
file = "evidence/outcome.csv"
rows = 31

[[evidence]]
table = "status"
file = "evidence/status.csv"
rows = 1

[[signature]]
role = "tester"
name = "A. Tester"
at = "2026-11-22 14:10:42.000"
```

## Keys

Each key states in parentheses that it is required, or the value it takes when left out. A moment
is the plant's clock on the store's axis, `YYYY-MM-DD HH:MM:SS.mmm`.

**The top level.**
- `format` (required): always `"veredas-report"`.
- `version` (required): this reader knows `1`. A report of another format or an unknown version is
  refused, never guessed.
- `note` (`""`): the tester's note.
- `test` (required), `run` (required): tables.
- `step`, `criterion`, `reported`, `write`, `evidence`, `signature` (none): arrays of tables, in
  the order of the run.

**The test** (`[test]`).
- `id` (required), `title` (required): the test's, as its file states them.
- `file` (required): the copy of the test that ran, in the run's folder: `test.test`.
- `functions` (none): one table per function file the test used: `used_as`, the path its `use`
  wrote, and `file`, its copy in the run's folder (`functions/<n>.fn`, in the order of the uses).
- `sign` (none): the roles the test names to sign the report (its `sign` setting), each once.
  Each of them signs once, and the report stands `awaiting <roles>` until all have, then
  `signed`; with none named, the first signature makes it `signed` (Q312 (e)).

**The run** (`[run]`).
- `number` (required): the run's place among the test's runs, its folder's name without zeros.
- `mode` (required): `live` or `replay`.
- `session` (required): the portal session's name.
- `from` (required), `to` (required): the window the run covered: its start, and its clock where it
  ended.
- `verdict` (required): `passed`, `failed` or `aborted`.
- `decided_at` (none): the step that decided it.
- `reason` (required): why, in words.

**A step** (`[[step]]`): every step the run took, in order, a step taken again once per attempt.
- `step` (required), `attempt` (required, from 1).
- `outcome` (none): `pass`, `fail`, `timeout` or `error`; absent for a step the run stopped inside.
- `detail` (required): what its actions did, in words.
- `from` (required), `to` (required): the clock when it started and when it ended.

**A criterion** (`[[criterion]]`): every `check` and every `branch` as judged.
- `step` (required), `attempt` (required).
- `text` (required): the check's label or its own text; a branch's value and its arms.
- `measured` (none): the values it read, `name = value`.
- `result` (required): `pass`, `fail` or `error` for a check; the arm a branch took, or `no arm`.

**A value reported** (`[[reported]]`).
- `step` (required), `attempt` (required).
- `text` (required): `name = value`, a note's text, or what could not be evaluated and why.
- `note` (required): whether it is a note.

**A write** (`[[write]]`): every gated write and send, as made.
- `step` (required), `attempt` (required).
- `text` (required): `table.column := value` or `channel.message(field: value, ...)`.
- `sent` (required): whether it left portal for the plant: the plant's answer, its own refusal
  and a write whose answer never came were sent, a refusal portal made first (the switch, a tag
  not writable, a value it cannot coerce, the server not reached) was not; a replay sends nothing.
- `answer` (required): the plant's answer or portal's refusal, in their words; `not sent: replay`.
- `ok` (required): whether it was taken.

**The evidence** (`[[evidence]]`): one table per table the test reads.
- `table` (required): the portal table.
- `file` (required): its CSV in the run's folder, `evidence/<table>.csv` for a plain name (ASCII
  letters, digits, `_` and `-`, 64 at most, no name Windows keeps for a device such as `NUL` or
  `COM1`), else `evidence/table.<n>.csv`, `n` its place among the evidence from 1,
  which no plain name can take: a header of `time`, `arrival`, `source`, `key`, `seq` and the
  table's columns, then every row stamped in the run's window, oldest first, each cell as the
  suite shows its value and a hole empty. `time` is the row's stamp and `arrival` when it reached
  portal, both moments on the store's axis.
- `rows` (required): how many.

**A signature** (`[[signature]]`).
- `role` (required): what the signer signs as, `tester` or `witness`.
- `name` (required): the signer's name, as typed (no accounts, Q20), holding a letter or a digit.
- `at` (required): when.

## What the engine refuses

A report with another stamp, a key the format does not know, a key missing or of another type. A
value outside the ones its key names (a mode, a verdict, an outcome, a role), a moment not in its
form, an attempt or a run's number below 1, a file named otherwise than the run's folder keeps it
(`test.test`, `functions/<n>.fn` in the order of the uses, the evidence's names), a table's
evidence given twice, in any ASCII case (the store's names fold it). A run past the last number a run can take
(4294967295), or with a moment a report cannot hold (a year past 9999). A test's folder, or a
run's, that cannot be read, which `list` names rather than pass over. A note on a report
signed by anyone, and a signature once the report is `signed`: a signed report is frozen, a
correction being a new run (Q312). A signature as a role the test does not name, or as one that
has signed already, when it names roles; a second signature when it names none. This is the engine's refusal, nothing sealed in the file. A signature
whose role is neither `tester` nor `witness`, or whose name holds no letter or digit. A run of a
test with no `id`, which has no folder.
