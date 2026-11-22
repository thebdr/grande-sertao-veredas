# The test file: `veredas-test`, version 1, and the function file: `veredas-functions`, version 1

A test is a check a person runs against a plant through portal (`docs/veredas-plan.md`,
"veredas-comissionador: the testing platform"): it drives the machine through portal's gated
writes, waits for what the plant does, measures it, branches on what it measured, and ends in an
acceptance report. One plain-text file holds one test, diff-able in git; Oficina's designer shows
it as text and as a flow chart, two views of the same file (`docs/design/comissionador/`), and the
flow view's positions live in the file's last block. Functions are written once, in function
files, and placed in a test as blocks, the way a function block is placed in ladder: their inputs
are the block's left ports, their outputs its right ports. `docs/comissionador-plan.md` says how
the engine reads, checks and runs these files; this file says what they hold.

The syntax is the assistant's, not the architect's: it refines the design's draft, and which
syntax the suite keeps is open (`docs/questions.md` Q310), as are the script box's language
(Q311) and how a test names plant data (Q317). Nothing in this file names a device: the worked
example runs against the bench's controller reference (`docs/controller-schema.md`), and a
plant's tests are files like these, kept outside the suite's code.

Every plain-text file of Veredas 2 states its schema (`docs/questions.md` Q10). A test file and a
function file are not TOML, because a test is a program, but each opens with the suite's stamp
in TOML: `format` and `version` on lines of their own, read by the one header reader every file
shares (`veredas_core::schema`), so a file of another format, an unknown version or a missing
stamp is refused by the same texts as a bench or a project file, before anything else is read.

## The worked example: a destination full, items come round

The machine of the controller reference announces an item every 100 ms and reports each item's
outcome 500 ms after it was answered; its destination 4 starts full, so an item sent there comes
back `FULL` and goes round again. A portal session reads the machine on a channel named `sorter`
and answers every announcement with destination 4 by a reply rule. The test starts the machine
by a command, a gated write, and waits until the machine's status says it runs; it then follows
20 outcomes for destination 4 through a function, computes the share that came round in a script
box, and branches on it: 0.95 or more passes, 0.85 to 0.95 runs the routing once more, anything
else releases the machine, notes the count and fails. Either way the machine is stopped by a
second command. It sits beside a folder `functions` holding the function file.

```test
format = "veredas-test"
version = 1

# CHK-012 · Destination 4 full, items come round
test "Destination 4 full, items come round" {
  id      CHK-012
  area    outfeed
  portal  the press hall
  modes   live, replay
  timeout 2min
  use     functions/routing.fn
  let     items = 20

  step start      send sorter.command(code: 1)
  step running    wait status.running == 1 timeout 12s
                  on timeout -> fail_log
  step route      call ItemRoute(dest: 4, items: items)
                  -> delivered, rerouted
  step ratio      script {
                    ratio  = rerouted / items
                    margin = ratio - 0.95
                  }
  step verdict    branch ratio {
                    >= 0.95       -> release
                    0.85 .. 0.95  -> retry route max 1
                    else          -> fail_log
                  }
  step release    send sorter.command(code: 0)
  step summary    report ratio, margin, rerouted
                  stop pass
  step fail_log   send sorter.command(code: 0)
                  report "rerouted {rerouted}/{items}"
                  stop fail
}

# Flow-chart positions, written by the flow view.
layout { start 0,1  running 0,2  route 0,3  ratio 0,4
         verdict 0,5  fail_log 1,5  release 0,6  summary 0,7 }
```

The function file, `functions/routing.fn`:

```function
format = "veredas-functions"
version = 1

# Follows the outcomes reported for one destination and counts where
# the items ended up. Shows as a block: inputs left, outputs right.
function ItemRoute(dest: int, items: int) -> (delivered: int, rerouted: int) {
  reads   outcome
  timeout 60s

  delivered = 0
  rerouted = 0
  for n in 1 .. items {
    o = await outcome where o.dest == dest
    if o.result == "SORTED" { delivered += 1 }
    else if o.result == "FULL" { rerouted += 1 }
  }
}
```

Read, the test has eight steps, one branch, one function and three gated writes, its three
`send`s. The engine is held to this: run live against the machine, every outcome for
destination 4 is `FULL`, so `delivered` is 0, `rerouted` 20 and `ratio` 1, and the run passes at
`summary`; its window replayed, the three sends are not sent, the function meets the same 20
outcomes in the store, and the replay passes with the same values. The engine's grammar test,
`crates/veredas-comissionador/tests/grammar.rs`, loads these two blocks and holds each statement
of this file about reading and checking against the reader; it keeps a copy of this text, so an
edit here is an edit there.

What is built, 2026-11-22: the reading and the check (`docs/comissionador-plan.md`, phase 1).
What a run does with each part, the outcomes of the actions, the routes a run takes, the values
and their comparisons, the clock, and the binding to a portal session, is the engine's, phases 2
and 3, and nothing of it runs yet; this file states it as the engine will be held to it.

## The file, line by line

**Text.** UTF-8, lines ending in LF or CRLF. `#` begins a comment that runs to the end of its
line, outside a string. Blank lines and comments go anywhere.

**The header.** Every line before the first line that opens with `test` (a test file) or
`function` (a function file) is the header: TOML holding `format` and `version` and nothing else
(comments and blank lines aside). A test file's `format` is `"veredas-test"`, a function file's
`"veredas-functions"`; this reader knows `version` 1 of each. Another key in the header is
refused by its name.

**Names.** A name is ASCII letters, digits and `_`, not starting with a digit. These words are
the language's own and name nothing else: `and`, `as`, `await`, `branch`, `call`, `check`,
`else`, `error`, `fail`, `false`, `for`, `function`, `if`, `in`, `layout`, `not`, `on`,
`or`, `pass`, `read`, `reads`, `report`, `retry`, `script`, `send`, `step`, `stop`, `test`,
`timeout`, `true`, `wait`, `where`, `write`. After a `.` any name is a column's or a message's,
the language's own words included (`a.read`); a table whose name is one of them is written in
backquotes (`` `check`.value ``).

**Lines.** A line ends what it holds, except inside parentheses, where lines may break freely,
and a line whose first symbol is `->` continues the line before it.

**Literals.** A whole number (`20`, `9_000`), a number with a fraction or an exponent (`0.95`,
`1e3`), `true` and `false`, a duration (a number and its unit with no space between: `250ms`,
`5s`, `2min`, `1h`) and a string in double quotes. In a string, `\"`, `\\`, `\n`, `\t`, `\{` and
`\}` are the character, and `{` an expression `}` is the expression's value as text
(`"rerouted {rerouted}/{items}"`).

## The test file

```
test "<title>" {
  <settings>
  <steps>
}
layout { <positions> }
```

The title is the test's name as the library and the report show it. The settings come first,
each on its own line, then the steps; the `layout` block, when there is one, follows the test.

### Settings

Each states in parentheses that it is required, or the value it takes when left out. `id`,
`area`, `portal` and `use` take the rest of their line, trimmed, a comment and a surrounding
pair of quotes removed.

- `id` (required): the test's identifier, letters, digits, `-` and `_`. Its runs are kept under
  it (`runs/<id, lowercased>/`).
- `area` (none): the group the library shows the test in.
- `portal` (none): the name of the portal session the test is written for; a run against a
  session of another name is refused before anything is written.
- `modes` (`live, replay`): the modes the test runs in, `live`, `replay` or both; a run in
  another mode is refused.
- `timeout` (`10min`): the whole run's bound. A wait with no timeout of its own is bounded by
  what is left of it.
- `use` (none; once per file): a function file, its path relative to the test file's folder.
- `let` (none; once per name): `let <name> = <expression>`, a constant, the expression made of
  literals and the constants above it.

### Steps

```
step <name> <action>
            <action>
            on <outcome>, <outcome> -> <route>
```

A step is a name, unique in the test, and one or more actions, each on its own line, run in
order; `branch` and `stop` are only ever a step's last action. `on` lines follow the actions. A
step's lines run from its `step` line to the next `step` line or the test's closing brace.

Every action ends in one **outcome**: `pass`, `fail`, `timeout` or `error`. The first action
that does not pass ends the step with its outcome; a step whose actions all pass, passes.

**The actions.** Each names the block it is drawn as in the flow view.

- `write <table>.<column> := <expression>`, a **write**, gated: the column must be an OPC UA tag
  the session's project marks writable, and portal's write switch armed. The value is written
  as the tag's type takes it. Passes when the plant confirms it; ends in `error` with portal's
  words when the switch is off, the tag is not writable, the server is away, the write may or
  may not have reached the plant, or the plant answers a bad status.
- `send <channel>.<message>(<field>: <expression>, ...)`, a **write**, gated: one message of the
  channel's catalog, composed from the fields and sent to the channel's peer. Passes once sent;
  `error` with portal's words as a write's.
- `wait <condition> [timeout <duration>]`, a **wait**: passes at the first moment the condition
  holds over the plant's values, at once when it holds as the step starts; `timeout` when the
  bound passes first. `wait <duration>` waits that long and passes.
- `await <table> [where <condition>] [timeout <duration>] -> <name>`, a **wait** for an event:
  the first row of the table stamped after the step starts for which the condition holds,
  bound to the name, which the condition uses to read the row (`await outcome where o.dest == 4
  -> o`). Passes on that row; `timeout` when the bound passes first.
- `read <reference>, ... -> <name>, ...`, a **read**: each reference's value as the step starts,
  into the names in order. `error` for a reference with no value yet.
- `check <condition> [as "<label>"]`, a **check**: a criterion of the acceptance report, under
  its label or its own text. Passes when the condition holds, `fail` when it does not, `error`
  when it cannot be judged.
- `call <Function>(<input>: <expression>, ...) [-> <name>, ...]`, a **function** block: the
  function of that name from a `use`d file, every input given once, its outputs into the names
  in order (fewer names keep the first outputs). The step ends in the function's outcome.
- `script { <statements> }`, a **script** box: a calculation over the test's variables. It reads
  no plant data and does not wait, so it computes the same in a live run and in a replay.
  Passes; `fail` by a `fail` statement; `error` on a fault (a division by zero, a name never
  assigned, two values that do not compare).
- `branch <expression> { <arms> }`, a **branch**: each arm on its own line, `<pattern>, ... ->
  <route>`, or `else -> <route>` last. The first arm with a pattern that matches routes the run.
  A pattern is a comparison and a value (`>= 0.95`, `< 3`, `== "FULL"`, `!= 0`), a range of two
  values, both included (`0.85 .. 0.95`), or a value, matched by equality. No arm matching and
  no `else`, the step ends in `fail`.
- `report <item>, ...`, a **report**: each item recorded in the report, a value under its text
  (`ratio` as `ratio = 0.9`), a string as a note. An item that cannot be evaluated (a name not
  assigned on the path taken) is recorded with the reason; a report always passes.
- `stop pass ["<reason>"]` and `stop fail ["<reason>"]`, the **end**: the run ends, passed or
  failed.

**Routes.** A route, after `->`, is where the run goes next: a step's name; `retry <step> max
<n>`, the same as naming the step at most `n` times in the run, after which the step that took
the route ends in `fail` with the reason; or `stop pass` or `stop fail`.

**Where the run goes.** A step that passes goes to the next step in the file; past the last
step the run passes. A step that ends in `fail` fails the run; one that ends in `timeout` or
`error` aborts it. An `on` line routes the outcomes it names: `on timeout -> fail_log` sends a
wait that timed out to `fail_log`, `on pass -> report` skips what lies between. An outcome is
routed once per step. A branch routes its `pass` by its arms, so a branch takes no `on pass`.

A run ends **passed**, **failed** or **aborted**: aborted when the plant did not let the test
judge it (a timeout or an error no route takes), when the tester stops the run, or, in a replay,
when the stored window ends before a step's outcome.

### The layout

```
layout { <step> <column>,<row> ... }
```

The flow view's positions, on its grid, whole numbers from 0, entries apart by spaces or line
ends. The view writes this block; a run ignores it. A position for a name that is no step is
refused; a step with no position is placed by the view.

## The function file

```
function <Name>(<input>: <type>, ...) -> (<output>: <type>, ...) {
  reads   <table>, ...
  timeout <duration>
  <statements>
}
```

A function file holds one or more functions. A function's name is unique across the files a
test uses, and is no built-in call's (`abs`, `min`, ...). Its inputs are its block's left ports and its outputs its right ports; the arrow
and the outputs are left out for a function that returns nothing. The comment lines right
above `function` are its description, shown on its block.

- **Types**: `int`, `float`, `bool`, `text`, `time` and `duration`. A call's value for an input
  is checked against the input's type as the call starts; an `int` is taken where a `float` is
  asked for.
- `reads` (none): the tables the function reads. A reference to another table is refused.
- `timeout` (none, the run's bound): the bound of one call.
- **The body**: statements over the inputs, which are read only, and names of its own; a name of
  the test is not one of them. Every output is assigned somewhere in the body, or the check
  refuses the function; one that the path a call takes leaves unassigned ends the call in
  `error`.
- **A function only reads.** No `write`, `send`, `report`, `check` or `stop` is a statement, so
  a block that is a function can never touch the plant. A function may call another of its own
  file, never itself, through others or directly.

## Statements

The body of a script box, of a function, and of an `if` or a `for`:

- `<name> = <expression>`, and `+=`, `-=`, `*=`, `/=`: assign a name.
- `if <condition> { ... } else if <condition> { ... } else { ... }`: the first branch whose
  condition holds.
- `for <name> in <expression> .. <expression> { ... }`: whole numbers, both ends included; a
  range whose end is below its start runs nothing.
- `fail ["<reason>"]`: ends the script or the call in `fail`.
- In a function only: `wait <condition> [timeout <duration>]` and `wait <duration>`;
  `<name> = await <table> [where <condition>] [timeout <duration>]`, the row the condition reads
  by that name (`o = await outcome where o.dest == dest`); and `<name>, ... = <Function>(<input>:
  <expression>, ...)`. A wait or an await that times out ends the call in `timeout`.

A block is `{`, statements one per line or apart by `;`, and `}`; it may sit on one line
(`{ delivered += 1 }`). In a script box a name assigned is a variable of the test, read by the
steps after it; a `let` constant is never assigned. In a function every name is the call's own.

## Expressions

**Values.** A whole number (64 bits), a number (64-bit floating point), a truth value, a text, a
moment (on the store's axis, below), a duration, and a row (an event an `await` took: its
columns by name, and `time`, `key`, `source`, `seq` and `arrival`, what portal stores with every
row).

**Names.** A name is a variable: a `let` constant, a name a script box, a `read`, an `await` or
a call assigned, an input, a loop's name. `<name>.<column>` reads a row's column when the name
is a variable; a name reads one column, so `a.b.c` is refused. **Any other `<table>.<column>`
is plant data**: the column's latest value in
portal's table of that name (a capture rule's, an OPC UA rule's, a message's, or a view with
portal's time column), stamped at or before the run's clock, a hole in a row read past.

**Operators**, loosest first: `or`; `and`; `not`; the comparisons `==`, `!=`, `<`, `<=`, `>`,
`>=`, one per comparison; `+`, `-`; `*`, `/`, `%`; a leading `-`; a call and `.`. Parentheses
group.

- Arithmetic: whole numbers give a whole number, except `/`, which gives a number; a number on
  either side gives a number; `%` takes whole numbers. A moment less a moment is a duration; a
  moment and a duration added or taken away give a moment; durations add, a duration times a
  number is a duration, and a duration divided by a duration is a number.
- Comparisons: numbers with numbers, whatever their kind; a truth value with a truth value, or
  with a whole number, `true` being 1 and `false` 0; a text with a text; a text with a number,
  the text trimmed and read as a number (a text that is no number is unequal to every number,
  and ordering it against one is an error); a moment with a moment, a duration with a duration.
  Anything else is an error.
- Calls: `abs(x)`, `min(a, b, ...)`, `max(a, b, ...)`, `round(x)` and `round(x, places)`,
  `seconds(d)` (a duration as a number of seconds), `now()` (the run's clock), and the
  conversions `int(x)`, `float(x)`, `text(x)`.

**No value yet.** Plant data with no row stamped at or before the clock, or a name not assigned
yet, has no value. In a wait's condition, a comparison with no value does not hold yet; anywhere
else it is the action's `error`, naming what had no value.

## Time

A run keeps a **clock**, a moment on the store's axis: portal's, the plant's local clock read as
UTC, the axis every row is indexed on. A live run starts it at the store's now; a replay at the
start of the stored window. A wait or an await that holds moves it to the stamp of the row it
held on; `wait <duration>` moves it by the duration; a timeout moves it to the bound. In a live
run a write or a send moves it to the moment the plant answered, so whatever the plant does
after a write is after the write; in a replay a write and a send are not sent and the clock does
not move. Every step reads the plant as of the clock. A live run reads a moment only once it is
250 ms past, so the rows stamped at or before it have reached the store; a replay of a live
run's own window therefore reads the rows the live run read.

## What a check refuses

Before a run, the engine reads the test and every function file it uses. Reading a file stops at
its first error of form, named with its file, line and column: a header that is not the stamp or
holds another key; something that cannot be read (a string, a backquote or a `{` not closed, an
unknown escape or character, a number run into letters, a duration that is not whole
milliseconds, a number past its range); a word where another is expected, a parenthesis or a
brace not closed; one of the language's words used as a name; a setting that is unknown, has no
value or comes after a step; a mode, an action, an outcome or a type that is none; an action after
a step's `branch` or `stop`, or after its `on` lines; a `read` whose values and names differ in
number; an `else` arm that is not the last, a branch with no arm; two comparisons in a row; a
name reading more than one column; a built-in call's name given to a function; `reads` or
`timeout` twice in a function; a write, a send or another step's action written as a statement;
an `await` not assigned.

The files read, it checks them whole and names every problem: `id` missing, or not letters,
digits, `-` and `_`; a setting given twice, a path `use`d twice, a constant defined twice; a `let`
made of anything but literals and the constants above it; a used file that cannot be read; two
functions of one name; a step's name given twice; a route to no step; `retry ... max 0`; an
outcome routed twice by one step; `on pass` on a branch; a `send`'s field given twice; a call to
no function, an input not given, given twice or not the function's, more names than outputs; a
built-in call given the wrong number of values, a call to no built-in, a function called inside
an expression; a function calling itself; a name read that nothing assigns; a `let` constant or a
function's input assigned; a port named twice; an output nothing assigns; plant data in a script
box, or a table a function does not declare in `reads`; a wait, an await or a call in a script
box; a step placed twice, or a position for no step, in the `layout`.

Bound to a portal session as the run starts, the run is refused when the session's name is not
the test's `portal`, the mode is not among the test's `modes`, a table or a column the test reads
is not in the session, a `write`'s column is not an OPC UA tag marked writable, a `send`'s channel
or message is not the session's, or, live, the test writes and portal's switch is off.

## Grammar

```
test-file     = header "test" string "{" { setting } { step } "}" [ layout ]
function-file = header function { function }
header        = TOML lines: format, version
setting       = ( "id" | "area" | "portal" | "use" ) rest-of-line
              | "modes" mode { "," mode } | "timeout" duration | "let" name "=" expr
step          = "step" name action { action } { on }
action        = "write" ref ":=" expr
              | "send" name "." name "(" [ args ] ")"
              | "wait" ( expr [ "timeout" duration ] | duration )
              | "await" table [ "where" expr ] [ "timeout" duration ] "->" name
              | "read" ref { "," ref } "->" name { "," name }
              | "check" expr [ "as" string ]
              | "call" name "(" [ args ] ")" [ "->" name { "," name } ]
              | "script" block
              | "branch" expr "{" { arm } [ "else" "->" route ] "}"
              | "report" ( expr | string ) { "," ( expr | string ) }
              | "stop" ( "pass" | "fail" ) [ string ]
on            = "on" outcome { "," outcome } "->" route
arm           = pattern { "," pattern } "->" route
pattern       = ( "==" | "!=" | "<" | "<=" | ">" | ">=" ) expr | expr ".." expr | expr
route         = name | "retry" name "max" integer | "stop" ( "pass" | "fail" )
outcome       = "pass" | "fail" | "timeout" | "error"
layout        = "layout" "{" { name integer "," integer } "}"
function      = "function" name "(" [ param { "," param } ] ")"
                [ "->" "(" param { "," param } ")" ] "{" [ "reads" table { "," table } ]
                [ "timeout" duration ] { statement } "}"
param         = name ":" type
type          = "int" | "float" | "bool" | "text" | "time" | "duration"
block         = "{" { statement } "}"
statement     = name ( "=" | "+=" | "-=" | "*=" | "/=" ) expr
              | "if" expr block { "else" "if" expr block } [ "else" block ]
              | "for" name "in" expr ".." expr block
              | "fail" [ string ]
              | "wait" ( expr [ "timeout" duration ] | duration )
              | name "=" "await" table [ "where" expr ] [ "timeout" duration ]
              | name { "," name } "=" name "(" [ args ] ")"
args          = name ":" expr { "," name ":" expr }
ref           = table "." name
table         = name | "`" any-name "`"
expr          = or-expr, by the operators above; a primary is a literal, a name, a name "." name,
                a call name "(" [ expr { "," expr } ] ")", or "(" expr ")"
```
