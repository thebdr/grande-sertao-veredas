# The controller's state text

A controller's state is structured data; its text is one line with one writer and one reader,
both in core (`veredas_core::state`), so every screen, log and test shows the same characters
(`docs/controlador-plan.md`, "The state", where the model and its table were decided). This file
is the spelling. `ControllerState`'s `Display` is the writer; its `FromStr` is the reader, which
accepts exactly the texts the writer writes: a text and a state are one thing, and any other
spelling is refused by name with the byte where it went wrong.
`crates/veredas-core/tests/state_text.rs` holds each statement below against both.

The text stays English and is never translated or rewritten; a screen translates the details it
shows beside it, by the numbers.

## The two layers

```text
RUN
RUN breakpoint(set=2) force(in=1, out=2, var=3) shadow(drives=13, watches=12) sim()
HOLD breakpoint(id=401, at="conveyor.rs:42") force(in=1, out=1, var=3)
STOP command(id=101, source="oficina")
FAULT watchdog(id=501, task="main")
STARTUP warm(id=201) devices(up=3, of=5)
RUN maintenance_request(id=3, reason="device_missing", urgent=true)
```

Layer 1 is one uppercase word, its number the hundreds digit of its reasons: `UNKNOWN` 0, `STOP`
1, `STARTUP` 2, `RUN` 3, `HOLD` 4, `FAULT` 5. Layer 2 is terms after it, in this order:

1. at most one **reason**, a row of the table below, its number first as `id`;
2. in `STARTUP`, the **phase**: `load()`, `restore()`, `handler()`, `devices(up, of)` (numbered
   1 to 4, with no `id` in the text);
3. each **condition** that applies, in this fixed order: `breakpoint(set)`, `force(in, out,
   var)`, `shadow(drives, watches)`, `dry()`, `sim()`, `no_realtime()`, `devices(missing)`,
   `overruns(count)`, `retain(reset)`. A condition that is absent has no term: a count of zero
   is never written, and `force` is written, all three counts in that order, when any is set.
   `shadow` names download numbers;
4. each **maintenance request**, `maintenance_request(id, reason, urgent)`, by number, a request
   not urgent before an urgent one of the same number. Several may stand, the same one twice
   included.

| State | Reasons |
|---|---|
| STOP | 101 `command(source)`, 102 `power_on(was)`, 103 `acknowledged`, 104 `no_program`, 105 `realtime_refused` |
| STARTUP | 201 `warm`, 202 `cold` |
| RUN | none |
| HOLD | 401 `breakpoint(at)`, 402 `command(source)`, 403 `step`, 404 `cycle` |
| FAULT | 501 `watchdog(task)`, 502 `program(at)`, 503 `user(code)`, 504 `handler(event)`, 505 `resource(kind)`, 506 `load(download)`, 507 `internal` |

A reason of another state is refused, and so is an `id` that is not its row's number. `source`
is `"oficina"`, `"local"` or `"program"`; `was` is a layer-1 word; `at`, `task`, `event` and
`kind` are strings; `code` is any 64-bit integer; `download` and every count run from 0 to
4294967295.

Maintenance reasons: 1 `fault_on_update` (urgent), 2 `device` (as the device reports), 3
`device_missing` (urgent), 4 `overruns` (not urgent), 5 `retain_reset` (not urgent), 6 `user`
(as the program states). A fixed urgency written otherwise is refused. `urgent=false` is
PROFINET's maintenance required, `urgent=true` its maintenance demanded.

## Spelling

- One space between the word and each term; nothing before the word, nothing after the last
  term (no line ending: a caller strips its own).
- A term is its lowercase name and its arguments in parentheses, empty ones included: `sim()`.
- Arguments are `name=value` with no space around `=`, separated by a comma and one space, in
  the order the table gives.
- Values are TOML values, each written one way: an integer in decimal, `0` or an optional `-`
  and digits with no leading zero (no `+`, underscore, prefix, exponent or fraction); `true` or
  `false`; a string in double quotes.
- A string is written as its characters, except: `"` `\` backspace, tab, newline, form feed and
  carriage return as `\"` `\\` `\b` `\t` `\n` `\f` `\r`; every other control character, the line
  and paragraph separators (U+2028, U+2029) and Unicode's format characters (general category
  Cf: the invisible ones and the marks that reorder a line) as `\u` and four uppercase
  hexadecimal digits, or `\U` and eight past U+FFFF. So the text stays one line and shows the
  characters it holds. Any other escape, a lowercase digit, or one of those characters raw, is
  refused. TOML's own parser reads every value so written as the same value.

## Health

The standard health value (NE 107; OPC UA's DeviceHealth) is derived: `FAULT` gives failure (1);
`STOP`, `STARTUP`, `HOLD` and any debug condition give function check (2); a maintenance request
gives maintenance required (4); a clean `RUN` gives normal (0). The numbers are OPC UA's
`DeviceHealthEnumeration`.

## Readings the plan left open, ruled

The plan left these open. Each was taken as written here, and the architect accepted all eight
as taken on 2026-11-25 (`docs/questions.md` Q100), ruling the review's entries 17 to 21 on this
text with them.

- The unknown state's word is `UNKNOWN`, and it has no health.
- The debug conditions are `breakpoint`, `force`, `shadow`, `dry`, `sim` and `no_realtime`; what
  the controller suffers (`devices`, `overruns`, `retain`) shows through its maintenance requests.
- `retain(reset)` counts the retained variables reset.
- The names stay as the plan has them: two terms may share a name (`breakpoint` the reason and
  the condition, `devices` the phase and the condition), told apart by their arguments. A
  `HOLD` at a breakpoint writes `breakpoint(set)` too while breakpoints are set.
- The text carries no version, and a term the reader does not know is refused.

## Ruled to come, spelling proposed (not built)

The architect ruled these additions on 2026-11-25 and 26 (`docs/questions.md` Q39, Q40, Q32, Q43,
Q234), each term's spelling left to core. The spellings below are core's proposals. None is built: until a
term is built with its first writer (controlador's), the reader refuses it as it refuses any term
it does not know, and the table above stays as it is.

Conditions, in the fixed order: `fallback` right after `shadow`, with the debug conditions;
`last_save` and then `date_unconfirmed` after `retain(reset)`, with what the controller suffers.
- `fallback(left)`: after an edit taken over live, the old version watching and able to take
  over again, `left` its time left in whole seconds, rounded up, so an armed fallback never
  reads 0 (Q32: "Yes, a term with its time left"). Proposed as a debug condition, giving
  function check while it stands, as `shadow` does. The term goes when the right to take over
  ends, by the time limit, by a check that ends it, or by the takeover itself ("drops the armed
  term", `docs/controlador/live-takeover.md`, ruled 2026-11-26); the limit is the engineer's, ten
  minutes by default, so `left` starts at 600 unless set otherwise. A takeover leaves the
  controller in RUN with the request `fault_on_update` (reason 1 below, T3 there).
- `last_save()`: on a target that cannot save at power loss, a start from the last save, raised
  only when cycles ran after it; no maintenance request goes with it (Q40: "No request, a
  condition"). Not a debug condition.
- `date_unconfirmed()`: while the controller's date is unconfirmed (Q39: "Yes, a condition
  term"). Not a debug condition; the maintenance requests below carry the health.

Maintenance reasons, the next numbers (Q39: "Two reasons" for the clock, and a reason of its own
for the warning cycle time beside `overruns`):
- 7 `clock_lost`: the clock chip lost its time (a board with no clock chip counts as one), its
  urgency as the situation stands;
- 8 `ntp_silent`: NTP silent after its waiting time, or not set, its urgency as the situation
  stands (urgency follows, the request stays until cleared, Q39);
- 9 `cycle_time` (not urgent): a task's cycle went over its warning cycle time.

The certificates (ruled 2026-11-16, a request a year before any certificate expires; Q39 on the
25th: "Its own and its peers'"; Q43 on the 26th: "Goes on, urgent request"):
- 10 `certificate_expiring` (not urgent): a certificate in the controller's store, its own or a
  peer's, ends within a year;
- 11 `certificate_expired` (urgent): a known peer's certificate is past its end date, the secured
  link going on, until the certificate is renewed.

Running unattended (Q234: "Count and say so (Recommended)"):
- 12 `memory_errors` (not urgent): corrected memory errors past the rate's threshold. An
  uncorrected one in the controller's memory is the FAULT 507 `internal` the table has;
- 13 `resource_trend` (not urgent): the controller's resident memory, open files or threads rising
  over days against the limits set at its start. A limit reached is the FAULT 505 `resource`.

The urgencies of 10, 12 and 13 are core's proposals with their names; 11's is ruled.

Ruled to have no term: a controller without a credential raises nothing of its own (Q43:
"Oficina's warning only", against "a condition in the state text"); a secured link accepted while
the date is unconfirmed is marked on the link, the controller's own `date_unconfirmed()` aside
(Q43: "Accept known peers, dates unjudged"). And FAULT 506 `load(download)` always names a
download: a program handed in by its host that cannot be made comes up in STOP 104 `no_program`
until downloads exist (Q233: "STOP no_program instead"), so `download=0` is no reading of "no
download". The reader still takes every number the table gives.

Confirmed the same day (Q45): the arguments inside `force` are always in the order in, out, var
(K1: "Yes: in, out, var"), and a start-up phase shows by its word alone, without an `id` (K5:
"Word alone, no id"), both as this file already states them.
