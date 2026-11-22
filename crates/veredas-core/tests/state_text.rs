//! The controller's state text: one writer, one reader, the table of reason numbers and words
//! (contract: the state-text clause; `docs/state-text.md`; `docs/controlador-plan.md`, "The
//! state").
//!
//! The reality here is the plan's own text (its table and its seven examples, copied below and
//! read byte for byte) and TOML's own parser (the `toml` crate), which must read every value the
//! writer writes as the same value. The reader is held to be the writer's inverse over a seeded
//! corpus of states and over every small change to their texts.

use veredas_core::rng::Pcg32;
use veredas_core::state::*;

// ---------------------------------------------------------------------------------------------
// The plan's own words, copied: the table and the examples (docs/controlador-plan.md).
// ---------------------------------------------------------------------------------------------

/// The plan's table of reasons: number, word, the arguments after `id`.
const PLAN_REASONS: [(u16, &str, &[&str]); 18] = [
    (101, "command", &["source"]),
    (102, "power_on", &["was"]),
    (103, "acknowledged", &[]),
    (104, "no_program", &[]),
    (105, "realtime_refused", &[]),
    (201, "warm", &[]),
    (202, "cold", &[]),
    (401, "breakpoint", &["at"]),
    (402, "command", &["source"]),
    (403, "step", &[]),
    (404, "cycle", &[]),
    (501, "watchdog", &["task"]),
    (502, "program", &["at"]),
    (503, "user", &["code"]),
    (504, "handler", &["event"]),
    (505, "resource", &["kind"]),
    (506, "load", &["download"]),
    (507, "internal", &[]),
];

/// The plan's start-up phases: "1 load, 2 restore, 3 handler, 4 `devices(up, of)`".
const PLAN_PHASES: [(u8, &str, &[&str]); 4] = [
    (1, "load", &[]),
    (2, "restore", &[]),
    (3, "handler", &[]),
    (4, "devices", &["up", "of"]),
];

/// The plan's maintenance reasons: "1 `fault_on_update` (urgent), 2 `device` (as the device
/// reports), 3 `device_missing` (urgent), 4 `overruns` (not urgent), 5 `retain_reset` (not
/// urgent), 6 `user` (as the program states)".
const PLAN_MAINTENANCE: [(u8, &str, Option<bool>); 6] = [
    (1, "fault_on_update", Some(true)),
    (2, "device", None),
    (3, "device_missing", Some(true)),
    (4, "overruns", Some(false)),
    (5, "retain_reset", Some(false)),
    (6, "user", None),
];

fn cs(state: State) -> ControllerState {
    ControllerState::new(state)
}

/// The plan's seven examples, each with the state it means.
fn plan_examples() -> Vec<(&'static str, ControllerState)> {
    let mut second = cs(State::Run);
    second.conditions.breakpoints_set = 2;
    second.conditions.force = Force {
        inputs: 1,
        outputs: 2,
        variables: 3,
    };
    second.conditions.shadow = Some(Shadow {
        drives: 13,
        watches: 12,
    });
    second.conditions.sim = true;
    let mut third = cs(State::Hold(Some(HoldReason::Breakpoint {
        at: "conveyor.rs:42".into(),
    })));
    third.conditions.force = Force {
        inputs: 1,
        outputs: 1,
        variables: 3,
    };
    let mut seventh = cs(State::Run);
    seventh.maintenance.push(MaintenanceRequest::DeviceMissing);
    vec![
        ("RUN", cs(State::Run)),
        (
            "RUN breakpoint(set=2) force(in=1, out=2, var=3) shadow(drives=13, watches=12) sim()",
            second,
        ),
        (
            r#"HOLD breakpoint(id=401, at="conveyor.rs:42") force(in=1, out=1, var=3)"#,
            third,
        ),
        (
            r#"STOP command(id=101, source="oficina")"#,
            cs(State::Stop(Some(StopReason::Command {
                source: CommandSource::Oficina,
            }))),
        ),
        (
            r#"FAULT watchdog(id=501, task="main")"#,
            cs(State::Fault(Some(FaultReason::Watchdog {
                task: "main".into(),
            }))),
        ),
        (
            "STARTUP warm(id=201) devices(up=3, of=5)",
            cs(State::Startup {
                reason: Some(StartupReason::Warm),
                phase: Some(StartupPhase::Devices { up: 3, of: 5 }),
            }),
        ),
        (
            r#"RUN maintenance_request(id=3, reason="device_missing", urgent=true)"#,
            seventh,
        ),
    ]
}

#[test]
fn the_plans_seven_examples_read_and_write_back_byte_for_byte() {
    for (text, state) in plan_examples() {
        let read: ControllerState = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(read, state, "{text}");
        assert_eq!(state.to_string(), text);
    }
}

#[test]
fn the_table_is_the_plans_and_every_hundreds_digit_is_its_state() {
    let table: Vec<(u16, &str, &[&str])> =
        REASONS.iter().map(|r| (r.number, r.word, r.args)).collect();
    assert_eq!(table, PLAN_REASONS.to_vec());
    let mut numbers: Vec<u16> = REASONS.iter().map(|r| r.number).collect();
    numbers.dedup();
    assert_eq!(
        numbers.len(),
        REASONS.len(),
        "a number is stable and unique"
    );
    for r in REASONS {
        let state = r
            .state()
            .unwrap_or_else(|| panic!("{} has no state", r.number));
        assert_eq!(u16::from(state.number()), r.number / 100, "{}", r.word);
        assert_ne!(state, StateWord::Run, "RUN has no reason");
        assert_ne!(state, StateWord::Unknown);
        assert_eq!(reason_entry(r.number), Some(&r));
    }
    assert_eq!(reason_entry(301), None);
    let phases: Vec<(u8, &str, &[&str])> =
        PHASES.iter().map(|p| (p.number, p.word, p.args)).collect();
    assert_eq!(phases, PLAN_PHASES.to_vec());
    let maintenance: Vec<(u8, &str, Option<bool>)> = MAINTENANCE_REASONS
        .iter()
        .map(|m| (m.number, m.word, m.urgent))
        .collect();
    assert_eq!(maintenance, PLAN_MAINTENANCE.to_vec());
}

#[test]
fn layer_one_is_one_uppercase_word_numbered_zero_to_five() {
    let words: Vec<(u8, &str)> = StateWord::ALL
        .iter()
        .map(|s| (s.number(), s.word()))
        .collect();
    assert_eq!(
        words,
        vec![
            (0, "UNKNOWN"),
            (1, "STOP"),
            (2, "STARTUP"),
            (3, "RUN"),
            (4, "HOLD"),
            (5, "FAULT")
        ]
    );
    for s in StateWord::ALL {
        assert_eq!(StateWord::from_number(s.number()), Some(s));
        assert_eq!(StateWord::from_word(s.word()), Some(s));
        assert_eq!(s.to_string(), s.word());
    }
    assert_eq!(StateWord::from_number(6), None);
    assert_eq!(StateWord::from_word("Run"), None);
    let sources: Vec<&str> = CommandSource::ALL.iter().map(|s| s.word()).collect();
    assert_eq!(
        sources,
        vec!["oficina", "local", "program"],
        "the source `peer` was withdrawn"
    );
    assert_eq!(CommandSource::from_word("peer"), None);
}

/// Every typed reason, phase and request names its row of the table.
#[test]
fn every_typed_reason_is_its_row_of_the_table() {
    let mut seen = Vec::new();
    let stops = [
        StopReason::Command {
            source: CommandSource::Local,
        },
        StopReason::PowerOn {
            was: StateWord::Hold,
        },
        StopReason::Acknowledged,
        StopReason::NoProgram,
        StopReason::RealtimeRefused,
    ];
    for r in &stops {
        seen.push((r.number(), r.word(), StateWord::Stop));
    }
    for r in [StartupReason::Warm, StartupReason::Cold] {
        seen.push((r.number(), r.word(), StateWord::Startup));
    }
    let holds = [
        HoldReason::Breakpoint { at: "a".into() },
        HoldReason::Command {
            source: CommandSource::Program,
        },
        HoldReason::Step,
        HoldReason::Cycle,
    ];
    for r in &holds {
        seen.push((r.number(), r.word(), StateWord::Hold));
    }
    let faults = [
        FaultReason::Watchdog { task: "t".into() },
        FaultReason::Program { at: "a".into() },
        FaultReason::User { code: -1 },
        FaultReason::Handler { event: "e".into() },
        FaultReason::Resource {
            kind: "memory".into(),
        },
        FaultReason::Load { download: 3 },
        FaultReason::Internal,
    ];
    for r in &faults {
        seen.push((r.number(), r.word(), StateWord::Fault));
    }
    let table: Vec<(u16, &str, StateWord)> = REASONS
        .iter()
        .map(|r| (r.number, r.word, r.state().unwrap()))
        .collect();
    assert_eq!(seen, table);
    let phases = [
        StartupPhase::Load,
        StartupPhase::Restore,
        StartupPhase::Handler,
        StartupPhase::Devices { up: 0, of: 0 },
    ];
    let typed: Vec<(u8, &str)> = phases.iter().map(|p| (p.number(), p.word())).collect();
    assert_eq!(
        typed,
        vec![(1, "load"), (2, "restore"), (3, "handler"), (4, "devices")]
    );
    let requests = [
        MaintenanceRequest::FaultOnUpdate,
        MaintenanceRequest::Device { urgent: false },
        MaintenanceRequest::DeviceMissing,
        MaintenanceRequest::Overruns,
        MaintenanceRequest::RetainReset,
        MaintenanceRequest::User { urgent: true },
    ];
    let typed: Vec<(u8, &str, bool)> = requests
        .iter()
        .map(|m| (m.number(), m.word(), m.urgent()))
        .collect();
    assert_eq!(
        typed,
        vec![
            (1, "fault_on_update", true),
            (2, "device", false),
            (3, "device_missing", true),
            (4, "overruns", false),
            (5, "retain_reset", false),
            (6, "user", true),
        ]
    );
    assert!(MaintenanceRequest::Device { urgent: true }.urgent());
    assert!(!MaintenanceRequest::User { urgent: false }.urgent());
}

#[test]
fn each_reason_writes_its_id_first_then_its_arguments_in_the_tables_order() {
    let cases: Vec<(State, &str)> = vec![
        (State::Stop(None), "STOP"),
        (
            State::Stop(Some(StopReason::Command {
                source: CommandSource::Local,
            })),
            r#"STOP command(id=101, source="local")"#,
        ),
        (
            State::Stop(Some(StopReason::PowerOn {
                was: StateWord::Fault,
            })),
            r#"STOP power_on(id=102, was="FAULT")"#,
        ),
        (
            State::Stop(Some(StopReason::Acknowledged)),
            "STOP acknowledged(id=103)",
        ),
        (
            State::Stop(Some(StopReason::NoProgram)),
            "STOP no_program(id=104)",
        ),
        (
            State::Stop(Some(StopReason::RealtimeRefused)),
            "STOP realtime_refused(id=105)",
        ),
        (
            State::Startup {
                reason: None,
                phase: None,
            },
            "STARTUP",
        ),
        (
            State::Startup {
                reason: Some(StartupReason::Cold),
                phase: None,
            },
            "STARTUP cold(id=202)",
        ),
        (
            State::Startup {
                reason: None,
                phase: Some(StartupPhase::Load),
            },
            "STARTUP load()",
        ),
        (
            State::Startup {
                reason: Some(StartupReason::Warm),
                phase: Some(StartupPhase::Restore),
            },
            "STARTUP warm(id=201) restore()",
        ),
        (
            State::Startup {
                reason: Some(StartupReason::Warm),
                phase: Some(StartupPhase::Handler),
            },
            "STARTUP warm(id=201) handler()",
        ),
        (State::Hold(None), "HOLD"),
        (
            State::Hold(Some(HoldReason::Command {
                source: CommandSource::Program,
            })),
            r#"HOLD command(id=402, source="program")"#,
        ),
        (State::Hold(Some(HoldReason::Step)), "HOLD step(id=403)"),
        (State::Hold(Some(HoldReason::Cycle)), "HOLD cycle(id=404)"),
        (State::Fault(None), "FAULT"),
        (
            State::Fault(Some(FaultReason::Program {
                at: "main.rs:7".into(),
            })),
            r#"FAULT program(id=502, at="main.rs:7")"#,
        ),
        (
            State::Fault(Some(FaultReason::User {
                code: -9223372036854775808,
            })),
            "FAULT user(id=503, code=-9223372036854775808)",
        ),
        (
            State::Fault(Some(FaultReason::User { code: 0 })),
            "FAULT user(id=503, code=0)",
        ),
        (
            State::Fault(Some(FaultReason::Handler {
                event: "device_lost".into(),
            })),
            r#"FAULT handler(id=504, event="device_lost")"#,
        ),
        (
            State::Fault(Some(FaultReason::Resource {
                kind: "call_depth".into(),
            })),
            r#"FAULT resource(id=505, kind="call_depth")"#,
        ),
        (
            State::Fault(Some(FaultReason::Load {
                download: 4294967295,
            })),
            "FAULT load(id=506, download=4294967295)",
        ),
        (
            State::Fault(Some(FaultReason::Internal)),
            "FAULT internal(id=507)",
        ),
        (State::Unknown, "UNKNOWN"),
    ];
    for (state, text) in cases {
        let s = cs(state);
        assert_eq!(s.to_string(), text);
        assert_eq!(text.parse::<ControllerState>().as_ref(), Ok(&s), "{text}");
    }
}

#[test]
fn every_condition_in_its_fixed_order_and_an_absent_one_unwritten() {
    let mut s = cs(State::Startup {
        reason: Some(StartupReason::Warm),
        phase: Some(StartupPhase::Devices { up: 3, of: 5 }),
    });
    s.conditions = Conditions {
        breakpoints_set: 1,
        force: Force {
            inputs: 0,
            outputs: 2,
            variables: 0,
        },
        shadow: Some(Shadow {
            drives: 0,
            watches: 0,
        }),
        dry: true,
        sim: true,
        no_realtime: true,
        devices_missing: 2,
        overruns: 7,
        retain_reset: 12,
    };
    s.maintenance = vec![
        MaintenanceRequest::User { urgent: true },
        MaintenanceRequest::DeviceMissing,
        MaintenanceRequest::User { urgent: false },
        MaintenanceRequest::DeviceMissing,
        MaintenanceRequest::FaultOnUpdate,
    ];
    let text = concat!(
        "STARTUP warm(id=201) devices(up=3, of=5) breakpoint(set=1) force(in=0, out=2, var=0) ",
        "shadow(drives=0, watches=0) dry() sim() no_realtime() devices(missing=2) overruns(count=7) ",
        "retain(reset=12) ",
        r#"maintenance_request(id=1, reason="fault_on_update", urgent=true) "#,
        r#"maintenance_request(id=3, reason="device_missing", urgent=true) "#,
        r#"maintenance_request(id=3, reason="device_missing", urgent=true) "#,
        r#"maintenance_request(id=6, reason="user", urgent=false) "#,
        r#"maintenance_request(id=6, reason="user", urgent=true)"#,
    );
    assert_eq!(
        s.to_string(),
        text,
        "requests in the table's order, whatever the list's"
    );
    let read: ControllerState = text.parse().unwrap();
    assert_eq!(read, s.clone().normalized());
    // Two `devices` terms in one start-up: the phase by its arguments (up, of), the condition by
    // its own (missing). Q100 and the review's entry 17.
    let mut only = cs(State::Startup {
        reason: None,
        phase: Some(StartupPhase::Devices { up: 1, of: 1 }),
    });
    only.conditions.devices_missing = 1;
    assert_eq!(
        only.to_string(),
        "STARTUP devices(up=1, of=1) devices(missing=1)"
    );
    // A HOLD at a breakpoint with breakpoints still set writes both terms.
    let mut hold = cs(State::Hold(Some(HoldReason::Breakpoint {
        at: "x.rs:1".into(),
    })));
    hold.conditions.breakpoints_set = 2;
    assert_eq!(
        hold.to_string(),
        r#"HOLD breakpoint(id=401, at="x.rs:1") breakpoint(set=2)"#
    );
    assert_eq!(hold.to_string().parse::<ControllerState>(), Ok(hold));
    // Nothing set: no term at all.
    assert_eq!(cs(State::Run).to_string(), "RUN");
    let zero = ControllerState {
        conditions: Conditions::default(),
        ..cs(State::Hold(None))
    };
    assert_eq!(zero.to_string(), "HOLD");
}

#[test]
fn health_is_derived_from_the_state() {
    let health = |s: &ControllerState| s.health();
    assert_eq!(health(&cs(State::Unknown)), None);
    assert_eq!(health(&cs(State::Run)), Some(Health::Normal));
    assert_eq!(health(&cs(State::Fault(None))), Some(Health::Failure));
    for s in [
        State::Stop(None),
        State::Startup {
            reason: None,
            phase: None,
        },
        State::Hold(None),
    ] {
        assert_eq!(health(&cs(s)), Some(Health::FunctionCheck));
    }
    let mut maint = cs(State::Run);
    maint.maintenance.push(MaintenanceRequest::Overruns);
    assert_eq!(health(&maint), Some(Health::MaintenanceRequired));
    // A fault outranks everything, a debug condition outranks a request.
    let mut fault = maint.clone();
    fault.state = State::Fault(Some(FaultReason::Internal));
    fault.conditions.force.variables = 1;
    assert_eq!(health(&fault), Some(Health::Failure));
    let debug: [fn(&mut Conditions); 6] = [
        |c| c.breakpoints_set = 1,
        |c| c.force.inputs = 1,
        |c| {
            c.shadow = Some(Shadow {
                drives: 2,
                watches: 1,
            })
        },
        |c| c.dry = true,
        |c| c.sim = true,
        |c| c.no_realtime = true,
    ];
    for set in debug {
        let mut s = maint.clone();
        set(&mut s.conditions);
        assert!(s.conditions.debug());
        assert_eq!(health(&s), Some(Health::FunctionCheck), "{s}");
    }
    // What the controller suffers is no debug condition: its maintenance request tells it.
    let trouble: [fn(&mut Conditions); 3] = [
        |c| c.devices_missing = 1,
        |c| c.overruns = 1,
        |c| c.retain_reset = 1,
    ];
    for set in trouble {
        let mut s = cs(State::Run);
        set(&mut s.conditions);
        assert!(!s.conditions.debug());
        assert_eq!(health(&s), Some(Health::Normal), "{s}");
    }
    let values: Vec<(String, u32)> = [
        Health::Normal,
        Health::Failure,
        Health::FunctionCheck,
        Health::MaintenanceRequired,
    ]
    .iter()
    .map(|h| (h.to_string(), h.device_health()))
    .collect();
    assert_eq!(
        values,
        vec![
            ("normal".to_string(), 0),
            ("failure".to_string(), 1),
            ("function check".to_string(), 2),
            ("maintenance required".to_string(), 4)
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// Strings: one spelling, one line, TOML's own escapes.
// ---------------------------------------------------------------------------------------------

/// The characters the writer spells by number: every control character but the five with
/// short escapes, the line and paragraph separators, and Unicode's format characters
/// (general category Cf), as this test's own copy of the list.
fn by_number(c: char) -> bool {
    let n = u32::from(c);
    let cf: &[(u32, u32)] = &[
        (0x00AD, 0x00AD),
        (0x0600, 0x0605),
        (0x061C, 0x061C),
        (0x06DD, 0x06DD),
        (0x070F, 0x070F),
        (0x0890, 0x0891),
        (0x08E2, 0x08E2),
        (0x180E, 0x180E),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x2064),
        (0x2066, 0x206F),
        (0xFEFF, 0xFEFF),
        (0xFFF9, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0001, 0xE0001),
        (0xE0020, 0xE007F),
    ];
    (n <= 0x1F && !matches!(n, 0x08 | 0x09 | 0x0A | 0x0C | 0x0D))
        || (0x7F..=0x9F).contains(&n)
        || n == 0x2028
        || n == 0x2029
        || cf.iter().any(|(a, b)| (*a..=*b).contains(&n))
}

fn fault_task(task: &str) -> ControllerState {
    cs(State::Fault(Some(FaultReason::Watchdog {
        task: task.to_string(),
    })))
}

#[test]
fn every_character_of_unicode_round_trips_and_toml_reads_the_same_string() {
    let all: Vec<char> = (0..=0x10FFFFu32).filter_map(char::from_u32).collect();
    let mut escaped_seen = 0;
    for chunk in all.chunks(4096) {
        let task: String = chunk.iter().collect();
        let s = fault_task(&task);
        let text = s.to_string();
        // One line, and nothing in it a screen hides or reorders.
        assert!(
            !text.chars().any(|c| c.is_control() || by_number(c)),
            "chunk from U+{:04X}",
            u32::from(chunk[0])
        );
        assert_eq!(
            text.parse::<ControllerState>().as_ref(),
            Ok(&s),
            "chunk from U+{:04X}",
            u32::from(chunk[0])
        );
        // TOML's own parser reads the value the writer wrote as the same string.
        let quoted = &text[text.find("task=").unwrap() + 5..text.len() - 1];
        let table: toml::Table = toml::from_str(&format!("v = {quoted}")).unwrap();
        assert_eq!(table["v"].as_str(), Some(task.as_str()));
        escaped_seen += chunk.iter().filter(|c| by_number(**c)).count();
    }
    assert!(escaped_seen > 200);
    // Each character by number, one at a time: the exact spelling.
    for c in all.iter().copied() {
        let written = fault_task(&c.to_string()).to_string();
        let value = &written[written.find("task=").unwrap() + 5..written.len() - 1];
        let n = u32::from(c);
        let want = match c {
            '"' => "\"\\\"\"".to_string(),
            '\\' => "\"\\\\\"".to_string(),
            '\u{8}' => "\"\\b\"".to_string(),
            '\t' => "\"\\t\"".to_string(),
            '\n' => "\"\\n\"".to_string(),
            '\u{c}' => "\"\\f\"".to_string(),
            '\r' => "\"\\r\"".to_string(),
            c if by_number(c) && n <= 0xFFFF => format!("\"\\u{n:04X}\""),
            c if by_number(c) => format!("\"\\U{n:08X}\""),
            c => format!("\"{c}\""),
        };
        assert_eq!(value, want, "U+{n:04X}");
    }
}

#[test]
fn the_reader_refuses_every_spelling_the_writer_never_writes_in_a_string() {
    let refused = [
        r#"FAULT watchdog(id=501, task="a\u0041")"#, // a printable character by number
        r#"FAULT watchdog(id=501, task="\u000A")"#,  // a newline by number, not `\n`
        r#"FAULT watchdog(id=501, task="\u0022")"#,  // a quote by number
        r#"FAULT watchdog(id=501, task="\u001b")"#,  // lowercase hexadecimal
        r#"FAULT watchdog(id=501, task="\U0000001B")"#, // the long form for a short one
        r#"FAULT watchdog(id=501, task="\u{1b}")"#,  // Rust's spelling
        r#"FAULT watchdog(id=501, task="\x1B")"#,    // TOML 1.1's \x
        r#"FAULT watchdog(id=501, task="\e")"#,      // TOML 1.1's \e
        r#"FAULT watchdog(id=501, task="\/")"#,      // JSON's \/
        r#"FAULT watchdog(id=501, task="\uD800")"#,  // a surrogate
        r#"FAULT watchdog(id=501, task="\U00110000")"#, // past Unicode
        r#"FAULT watchdog(id=501, task="\u12")"#,    // cut short
        r#"FAULT watchdog(id=501, task="\"#,         // an escape at the end
        r#"FAULT watchdog(id=501, task="main)"#,     // no closing quote
        r#"FAULT watchdog(id=501, task='main')"#,    // a literal string
        r#"FAULT watchdog(id=501, task="""main""")"#, // a multi-line string
        "FAULT watchdog(id=501, task=\"a\tb\")",     // a raw tab
        "FAULT watchdog(id=501, task=\"a\nb\")",     // a raw newline
        "FAULT watchdog(id=501, task=\"a\u{2028}b\")", // a raw line separator
        "FAULT watchdog(id=501, task=\"a\u{202E}b\")", // a raw right-to-left override
        "FAULT watchdog(id=501, task=\"a\u{7F}b\")", // a raw delete
        "FAULT watchdog(id=501, task=\"a\u{E0041}b\")", // a raw tag character
    ];
    for text in refused {
        let err = text.parse::<ControllerState>().unwrap_err();
        assert!(
            matches!(
                err,
                StateTextError::String { .. } | StateTextError::Expected { .. }
            ),
            "{text}: {err:?}"
        );
    }
    assert_eq!(
        r#"FAULT watchdog(id=501, task="a\u0041")"#.parse::<ControllerState>().unwrap_err().to_string(),
        "a string at 30 is not written as the writer writes it: `\\u0041` spells a character the writer writes as itself or by its short escape"
    );
    assert_eq!(
        r#"FAULT watchdog(id=501, task="main)"#.parse::<ControllerState>().unwrap_err().to_string(),
        "a string at 28 is not written as the writer writes it: no closing quote"
    );
    // The canonical spellings of the same characters read.
    for (text, task) in [
        (r#"FAULT watchdog(id=501, task="\u001B")"#, "\u{1b}"),
        (r#"FAULT watchdog(id=501, task="\n\t\"\\")"#, "\n\t\"\\"),
        (r#"FAULT watchdog(id=501, task="\U000E0041")"#, "\u{E0041}"),
        (r#"FAULT watchdog(id=501, task="")"#, ""),
        (r#"FAULT watchdog(id=501, task="ação 🚀 ≠")"#, "ação 🚀 ≠"),
    ] {
        assert_eq!(
            text.parse::<ControllerState>(),
            Ok(fault_task(task)),
            "{text}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The reader refuses by name.
// ---------------------------------------------------------------------------------------------

fn refusal(text: &str) -> StateTextError {
    match text.parse::<ControllerState>() {
        Ok(s) => panic!("{text:?} was read as {s:?}"),
        Err(e) => e,
    }
}

#[test]
fn the_reader_refuses_every_other_spelling_of_the_text() {
    use StateTextError as E;
    type Is = fn(&E) -> bool;
    let cases: Vec<(&str, Is)> = vec![
        ("", |e| matches!(e, E::Empty)),
        ("run", |e| matches!(e, E::Expected { at: 0, .. })),
        (" RUN", |e| matches!(e, E::Expected { at: 0, .. })),
        ("RUNNING", |e| matches!(e, E::UnknownWord { .. })),
        ("PAUSE", |e| matches!(e, E::UnknownWord { .. })),
        ("RUN ", |e| matches!(e, E::Expected { at: 4, .. })),
        ("RUN\n", |e| matches!(e, E::Expected { at: 3, .. })),
        ("RUN\r\n", |e| matches!(e, E::Expected { at: 3, .. })),
        ("\u{FEFF}RUN", |e| matches!(e, E::Expected { at: 0, .. })),
        ("RUN  sim()", |e| matches!(e, E::Expected { at: 4, .. })),
        ("RUN\tsim()", |e| matches!(e, E::Expected { at: 3, .. })),
        ("RUN sim", |e| matches!(e, E::Expected { at: 7, .. })),
        ("RUN sim ()", |e| matches!(e, E::Expected { at: 7, .. })),
        ("RUN sim( )", |e| matches!(e, E::Expected { at: 8, .. })),
        ("RUN Sim()", |e| matches!(e, E::Expected { at: 4, .. })),
        ("RUN _sim()", |e| matches!(e, E::Expected { .. })),
        ("RUN sim()sim()", |e| matches!(e, E::Expected { at: 9, .. })),
        ("RUN force(in = 1, out=0, var=0)", |e| {
            matches!(e, E::Expected { .. })
        }),
        ("RUN force(in=1,out=0, var=0)", |e| {
            matches!(e, E::Expected { .. })
        }),
        ("RUN force(in=1,  out=0, var=0)", |e| {
            matches!(e, E::Expected { .. })
        }),
        ("RUN force(in=1, out=0, var=0,)", |e| {
            matches!(e, E::Expected { .. })
        }),
        ("RUN force(in=1, out=0, var=0", |e| {
            matches!(e, E::Expected { .. })
        }),
        ("RUN force(in=+1, out=0, var=0)", |e| {
            matches!(e, E::Expected { .. })
        }),
        ("RUN force(in=01, out=0, var=0)", |e| {
            matches!(e, E::Integer { .. })
        }),
        ("RUN force(in=0x1, out=0, var=0)", |e| {
            matches!(e, E::Integer { .. })
        }),
        ("RUN force(in=1_0, out=0, var=0)", |e| {
            matches!(e, E::Integer { .. })
        }),
        ("RUN force(in=1.0, out=0, var=0)", |e| {
            matches!(e, E::Integer { .. })
        }),
        ("RUN force(in=1e1, out=0, var=0)", |e| {
            matches!(e, E::Integer { .. })
        }),
        ("RUN force(in=-0, out=1, var=0)", |e| {
            matches!(e, E::Integer { .. })
        }),
        ("RUN force(in=-, out=1, var=0)", |e| {
            matches!(e, E::Integer { .. })
        }),
        ("FAULT user(id=503, code=9223372036854775808)", |e| {
            matches!(e, E::Integer { .. })
        }),
        ("RUN force(in=True, out=1, var=0)", |e| {
            matches!(e, E::Expected { .. })
        }),
        ("RUN force(out=1, in=0, var=0)", |e| {
            matches!(e, E::Arguments { .. })
        }),
        ("RUN force(in=1, out=0)", |e| {
            matches!(e, E::Arguments { .. })
        }),
        ("RUN force(in=1, out=0, var=0, x=1)", |e| {
            matches!(e, E::Arguments { .. })
        }),
        ("RUN sim(on=true)", |e| matches!(e, E::Arguments { .. })),
        ("RUN breakpoint()", |e| matches!(e, E::Arguments { .. })),
        ("RUN fast()", |e| matches!(e, E::UnknownTerm { .. })),
        ("RUN date_unconfirmed()", |e| {
            matches!(e, E::UnknownTerm { .. })
        }),
        (r#"RUN command(id=101, source="oficina")"#, |e| {
            matches!(
                e,
                E::NotInState {
                    state: StateWord::Run,
                    ..
                }
            )
        }),
        (r#"RUN breakpoint(id=401, at="a")"#, |e| {
            matches!(e, E::NotInState { .. })
        }),
        ("HOLD warm(id=201)", |e| matches!(e, E::NotInState { .. })),
        ("FAULT load()", |e| matches!(e, E::Arguments { .. })),
        ("RUN load()", |e| matches!(e, E::NotInState { .. })),
        ("FAULT handler()", |e| matches!(e, E::Arguments { .. })),
        ("UNKNOWN internal(id=507)", |e| {
            matches!(e, E::NotInState { .. })
        }),
        (r#"STOP command(id=402, source="oficina")"#, |e| {
            matches!(
                e,
                E::Id {
                    expected: 101,
                    found: 402,
                    ..
                }
            )
        }),
        ("STOP acknowledged(id=0103)", |e| {
            matches!(e, E::Integer { .. })
        }),
        (r#"STOP acknowledged(id="103")"#, |e| {
            matches!(e, E::Type { .. })
        }),
        (r#"STOP command(id=101, source="peer")"#, |e| {
            matches!(e, E::Word { .. })
        }),
        (r#"STOP command(id=101, source="Oficina")"#, |e| {
            matches!(e, E::Word { .. })
        }),
        ("STOP command(id=101, source=oficina)", |e| {
            matches!(e, E::Expected { .. })
        }),
        (r#"STOP power_on(id=102, was="PAUSE")"#, |e| {
            matches!(e, E::Word { .. })
        }),
        ("STOP power_on(id=102, was=4)", |e| {
            matches!(e, E::Type { .. })
        }),
        (r#"FAULT user(id=503, code="5")"#, |e| {
            matches!(e, E::Type { .. })
        }),
        ("FAULT user(id=503, code=true)", |e| {
            matches!(e, E::Type { .. })
        }),
        ("FAULT load(id=506, download=-1)", |e| {
            matches!(e, E::Range { .. })
        }),
        ("FAULT load(id=506, download=4294967296)", |e| {
            matches!(e, E::Range { .. })
        }),
        ("RUN devices(missing=-1)", |e| matches!(e, E::Range { .. })),
        ("RUN breakpoint(set=0)", |e| matches!(e, E::Absent { .. })),
        ("RUN force(in=0, out=0, var=0)", |e| {
            matches!(e, E::Absent { .. })
        }),
        ("RUN devices(missing=0)", |e| matches!(e, E::Absent { .. })),
        ("RUN overruns(count=0)", |e| matches!(e, E::Absent { .. })),
        ("RUN retain(reset=0)", |e| matches!(e, E::Absent { .. })),
        ("RUN sim() dry()", |e| matches!(e, E::Order { .. })),
        ("RUN sim() sim()", |e| matches!(e, E::Order { .. })),
        (
            r#"HOLD breakpoint(set=1) breakpoint(id=401, at="a")"#,
            |e| matches!(e, E::Order { .. }),
        ),
        ("STARTUP devices(up=1, of=2) warm(id=201)", |e| {
            matches!(e, E::Order { .. })
        }),
        ("STARTUP devices(missing=1) devices(up=1, of=2)", |e| {
            matches!(e, E::Order { .. })
        }),
        ("STARTUP warm(id=201) cold(id=202)", |e| {
            matches!(e, E::Order { .. })
        }),
        ("STARTUP load() restore()", |e| matches!(e, E::Order { .. })),
        ("STARTUP devices(up=1)", |e| {
            matches!(e, E::Arguments { .. })
        }),
        (
            r#"RUN maintenance_request(id=3, reason="device_missing", urgent=false)"#,
            |e| matches!(e, E::Table { key: "urgent", .. }),
        ),
        (
            r#"RUN maintenance_request(id=4, reason="overruns", urgent=true)"#,
            |e| matches!(e, E::Table { key: "urgent", .. }),
        ),
        (
            r#"RUN maintenance_request(id=3, reason="device", urgent=true)"#,
            |e| matches!(e, E::Table { key: "reason", .. }),
        ),
        (
            r#"RUN maintenance_request(id=603, reason="device_missing", urgent=true)"#,
            |e| matches!(e, E::Table { key: "id", .. }),
        ),
        (
            r#"RUN maintenance_request(id=0, reason="device", urgent=true)"#,
            |e| matches!(e, E::Table { key: "id", .. }),
        ),
        (
            r#"RUN maintenance_request(id=2, reason="device", urgent=1)"#,
            |e| matches!(e, E::Type { .. }),
        ),
        (
            r#"RUN maintenance_request(reason="device", id=2, urgent=true)"#,
            |e| matches!(e, E::Arguments { .. }),
        ),
        (
            r#"RUN maintenance_request(id=4, reason="overruns", urgent=false) maintenance_request(id=3, reason="device_missing", urgent=true)"#,
            |e| matches!(e, E::MaintenanceOrder { .. }),
        ),
        (
            r#"RUN maintenance_request(id=2, reason="device", urgent=true) maintenance_request(id=2, reason="device", urgent=false)"#,
            |e| matches!(e, E::MaintenanceOrder { .. }),
        ),
        (
            r#"RUN maintenance_request(id=1, reason="fault_on_update", urgent=true) sim()"#,
            |e| matches!(e, E::Order { .. }),
        ),
    ];
    for (text, is) in cases {
        let e = refusal(text);
        assert!(is(&e), "{text:?} refused for another reason: {e:?} ({e})");
    }
}

#[test]
fn a_refusal_says_where_and_why() {
    let texts: Vec<String> = [
        "PAUSE",
        "RUN sim( )",
        r#"RUN command(id=101, source="oficina")"#,
        "RUN force(out=1, in=0, var=0)",
        r#"STOP command(id=402, source="oficina")"#,
        "RUN sim() dry()",
        "RUN breakpoint(set=0)",
        r#"RUN maintenance_request(id=3, reason="device_missing", urgent=false)"#,
        "FAULT load(id=506, download=-1)",
        "STARTUP devices(up=1)",
    ]
    .iter()
    .map(|t| refusal(t).to_string())
    .collect();
    assert_eq!(
        texts,
        vec![
            "`PAUSE` at 0 is no state word (UNKNOWN, STOP, STARTUP, RUN, HOLD, FAULT)",
            "expected an argument's name (lowercase letters) at 8, found ' '",
            "`command` at 4 is no term of RUN",
            "`force` at 4 takes (in, out, var), not (out, in, var)",
            "`command` at 5 has id=402, where its number is 101",
            "`dry` at 10 stands after `sim`: terms stand in a fixed order",
            "`breakpoint` at 4 counts nothing: the writer leaves out a condition that is absent",
            "`maintenance_request` at 4 has urgent=false, which is not its table's true for \"device_missing\"",
            "`load` at 6 has `download=-1`, out of its range 0 to 4294967295",
            "`devices` at 8 takes (up, of) or (missing), not (up)",
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// The reader is the writer's inverse, over a seeded corpus and every small change to it.
// ---------------------------------------------------------------------------------------------

/// Characters a string is drawn from: plain, the quote and backslash, every short escape,
/// characters spelled by number in both widths, and characters written as themselves that
/// look like syntax or sit far from ASCII.
const PALETTE: &[char] = &[
    'a',
    'Z',
    '0',
    ' ',
    ',',
    '=',
    '(',
    ')',
    ':',
    '.',
    '_',
    '-',
    '"',
    '\\',
    '\u{8}',
    '\t',
    '\n',
    '\u{c}',
    '\r',
    '\u{0}',
    '\u{1b}',
    '\u{7f}',
    '\u{85}',
    '\u{2028}',
    '\u{202e}',
    '\u{feff}',
    '\u{e0041}',
    'ç',
    'ã',
    '🚀',
    '\u{a0}',
    '\u{3000}',
    '\u{fffd}',
    '\'',
    'u',
    'U',
];

fn draw(rng: &mut Pcg32, n: u32) -> u32 {
    rng.next_u32() % n
}

fn text_of(rng: &mut Pcg32) -> String {
    let len = draw(rng, 6);
    (0..len)
        .map(|_| PALETTE[draw(rng, PALETTE.len() as u32) as usize])
        .collect()
}

fn count(rng: &mut Pcg32) -> u32 {
    match draw(rng, 5) {
        0 => 0,
        1 => 1,
        2 => u32::MAX,
        _ => draw(rng, 1000),
    }
}

fn source(rng: &mut Pcg32) -> CommandSource {
    CommandSource::ALL[draw(rng, 3) as usize]
}

fn random_state(rng: &mut Pcg32) -> ControllerState {
    let state = match draw(rng, 6) {
        0 => State::Unknown,
        1 => State::Stop(match draw(rng, 6) {
            0 => None,
            1 => Some(StopReason::Command {
                source: source(rng),
            }),
            2 => Some(StopReason::PowerOn {
                was: StateWord::ALL[draw(rng, 6) as usize],
            }),
            3 => Some(StopReason::Acknowledged),
            4 => Some(StopReason::NoProgram),
            _ => Some(StopReason::RealtimeRefused),
        }),
        2 => State::Startup {
            reason: [None, Some(StartupReason::Warm), Some(StartupReason::Cold)]
                [draw(rng, 3) as usize],
            phase: match draw(rng, 5) {
                0 => None,
                1 => Some(StartupPhase::Load),
                2 => Some(StartupPhase::Restore),
                3 => Some(StartupPhase::Handler),
                _ => Some(StartupPhase::Devices {
                    up: count(rng),
                    of: count(rng),
                }),
            },
        },
        3 => State::Run,
        4 => State::Hold(match draw(rng, 5) {
            0 => None,
            1 => Some(HoldReason::Breakpoint { at: text_of(rng) }),
            2 => Some(HoldReason::Command {
                source: source(rng),
            }),
            3 => Some(HoldReason::Step),
            _ => Some(HoldReason::Cycle),
        }),
        _ => State::Fault(match draw(rng, 8) {
            0 => None,
            1 => Some(FaultReason::Watchdog { task: text_of(rng) }),
            2 => Some(FaultReason::Program { at: text_of(rng) }),
            3 => Some(FaultReason::User {
                code: [0, -1, i64::MIN, i64::MAX, 42][draw(rng, 5) as usize],
            }),
            4 => Some(FaultReason::Handler {
                event: text_of(rng),
            }),
            5 => Some(FaultReason::Resource { kind: text_of(rng) }),
            6 => Some(FaultReason::Load {
                download: count(rng),
            }),
            _ => Some(FaultReason::Internal),
        }),
    };
    let flag = |rng: &mut Pcg32| draw(rng, 3) == 0;
    let conditions = Conditions {
        breakpoints_set: count(rng),
        force: if flag(rng) {
            Force {
                inputs: count(rng),
                outputs: count(rng),
                variables: count(rng),
            }
        } else {
            Force::default()
        },
        shadow: if flag(rng) {
            Some(Shadow {
                drives: count(rng),
                watches: count(rng),
            })
        } else {
            None
        },
        dry: flag(rng),
        sim: flag(rng),
        no_realtime: flag(rng),
        devices_missing: count(rng),
        overruns: count(rng),
        retain_reset: count(rng),
    };
    let maintenance = (0..draw(rng, 4))
        .map(|_| match draw(rng, 6) {
            0 => MaintenanceRequest::FaultOnUpdate,
            1 => MaintenanceRequest::Device { urgent: flag(rng) },
            2 => MaintenanceRequest::DeviceMissing,
            3 => MaintenanceRequest::Overruns,
            4 => MaintenanceRequest::RetainReset,
            _ => MaintenanceRequest::User { urgent: flag(rng) },
        })
        .collect();
    ControllerState {
        state,
        conditions,
        maintenance,
    }
}

fn corpus() -> Vec<ControllerState> {
    let mut rng = Pcg32::new(20_261_122, 70);
    (0..4000).map(|_| random_state(&mut rng)).collect()
}

#[test]
fn writing_then_reading_gives_the_state_back() {
    for s in corpus() {
        let text = s.to_string();
        assert_eq!(
            text.parse::<ControllerState>().as_ref(),
            Ok(&s.clone().normalized()),
            "{text}"
        );
        assert!(
            !text.contains('\n') && !text.contains('\r'),
            "one line: {text}"
        );
    }
}

/// The text cut into its terms by the grammar (a term ends at `)` outside a string), each
/// term's arguments read by TOML's own parser as an inline table.
fn toml_terms(text: &str) -> Vec<(String, toml::Table)> {
    let body = text.split_once(' ').map_or("", |(_, rest)| rest);
    let (mut in_string, mut escaped, mut start) = (false, false, 0);
    let mut out = Vec::new();
    for (i, c) in body.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            ')' if !in_string => {
                let piece = &body[start..=i];
                let open = piece.find('(').unwrap();
                let args = &piece[open + 1..piece.len() - 1];
                let parsed: toml::Table = toml::from_str(&format!("v = {{{args}}}"))
                    .unwrap_or_else(|e| panic!("TOML refused `{args}` of `{text}`: {e}"));
                let toml::Value::Table(table) = parsed["v"].clone() else {
                    panic!("{args}")
                };
                out.push((piece[..open].to_string(), table));
                start = i + 2;
            }
            _ => {}
        }
    }
    out
}

/// The terms a state's text must hold, built from the structured state by this test's own copy
/// of the plan's order, each with its arguments as TOML values.
fn expected_terms(s: &ControllerState) -> Vec<(String, toml::Table)> {
    use toml::Value::{Boolean as B, Integer as I, String as S};
    let mut out: Vec<(&str, Vec<(&str, toml::Value)>)> = Vec::new();
    let id = |n: u16| ("id", I(i64::from(n)));
    let n = |v: u32| I(i64::from(v));
    let src = |c: &CommandSource| ("source", S(c.word().into()));
    match &s.state {
        State::Unknown | State::Run => {}
        State::Stop(r) => match r {
            None => {}
            Some(StopReason::Command { source }) => {
                out.push(("command", vec![id(101), src(source)]))
            }
            Some(StopReason::PowerOn { was }) => {
                out.push(("power_on", vec![id(102), ("was", S(was.word().into()))]))
            }
            Some(StopReason::Acknowledged) => out.push(("acknowledged", vec![id(103)])),
            Some(StopReason::NoProgram) => out.push(("no_program", vec![id(104)])),
            Some(StopReason::RealtimeRefused) => out.push(("realtime_refused", vec![id(105)])),
        },
        State::Startup { reason, phase } => {
            match reason {
                Some(StartupReason::Warm) => out.push(("warm", vec![id(201)])),
                Some(StartupReason::Cold) => out.push(("cold", vec![id(202)])),
                None => {}
            }
            match phase {
                Some(StartupPhase::Load) => out.push(("load", vec![])),
                Some(StartupPhase::Restore) => out.push(("restore", vec![])),
                Some(StartupPhase::Handler) => out.push(("handler", vec![])),
                Some(StartupPhase::Devices { up, of }) => {
                    out.push(("devices", vec![("up", n(*up)), ("of", n(*of))]))
                }
                None => {}
            }
        }
        State::Hold(r) => match r {
            None => {}
            Some(HoldReason::Breakpoint { at }) => {
                out.push(("breakpoint", vec![id(401), ("at", S(at.clone()))]))
            }
            Some(HoldReason::Command { source }) => {
                out.push(("command", vec![id(402), src(source)]))
            }
            Some(HoldReason::Step) => out.push(("step", vec![id(403)])),
            Some(HoldReason::Cycle) => out.push(("cycle", vec![id(404)])),
        },
        State::Fault(r) => match r {
            None => {}
            Some(FaultReason::Watchdog { task }) => {
                out.push(("watchdog", vec![id(501), ("task", S(task.clone()))]))
            }
            Some(FaultReason::Program { at }) => {
                out.push(("program", vec![id(502), ("at", S(at.clone()))]))
            }
            Some(FaultReason::User { code }) => {
                out.push(("user", vec![id(503), ("code", I(*code))]))
            }
            Some(FaultReason::Handler { event }) => {
                out.push(("handler", vec![id(504), ("event", S(event.clone()))]))
            }
            Some(FaultReason::Resource { kind }) => {
                out.push(("resource", vec![id(505), ("kind", S(kind.clone()))]))
            }
            Some(FaultReason::Load { download }) => {
                out.push(("load", vec![id(506), ("download", n(*download))]))
            }
            Some(FaultReason::Internal) => out.push(("internal", vec![id(507)])),
        },
    }
    let c = &s.conditions;
    if c.breakpoints_set > 0 {
        out.push(("breakpoint", vec![("set", n(c.breakpoints_set))]));
    }
    if c.force.inputs > 0 || c.force.outputs > 0 || c.force.variables > 0 {
        out.push((
            "force",
            vec![
                ("in", n(c.force.inputs)),
                ("out", n(c.force.outputs)),
                ("var", n(c.force.variables)),
            ],
        ));
    }
    if let Some(sh) = c.shadow {
        out.push((
            "shadow",
            vec![("drives", n(sh.drives)), ("watches", n(sh.watches))],
        ));
    }
    for (on, name) in [
        (c.dry, "dry"),
        (c.sim, "sim"),
        (c.no_realtime, "no_realtime"),
    ] {
        if on {
            out.push((name, vec![]));
        }
    }
    for (v, name, key) in [
        (c.devices_missing, "devices", "missing"),
        (c.overruns, "overruns", "count"),
        (c.retain_reset, "retain", "reset"),
    ] {
        if v > 0 {
            out.push((name, vec![(key, n(v))]));
        }
    }
    let mut requests: Vec<(u8, bool, &str)> = s
        .maintenance
        .iter()
        .map(|m| (m.number(), m.urgent(), m.word()))
        .collect();
    requests.sort();
    for (number, urgent, word) in requests {
        out.push((
            "maintenance_request",
            vec![
                ("id", I(i64::from(number))),
                ("reason", S(word.into())),
                ("urgent", B(urgent)),
            ],
        ));
    }
    out.into_iter()
        .map(|(name, args)| {
            (
                name.to_string(),
                args.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            )
        })
        .collect()
}

/// Every term the writer writes is read by TOML's own parser, its arguments wrapped in braces
/// as an inline table, as the terms the state holds, in the plan's order.
#[test]
fn toml_reads_every_written_term_as_the_state_holds_it() {
    let mut terms = 0;
    for s in corpus()
        .into_iter()
        .chain(plan_examples().into_iter().map(|(_, s)| s))
    {
        let text = s.to_string();
        let read = toml_terms(&text);
        assert_eq!(read, expected_terms(&s), "{text}");
        terms += read.len();
    }
    assert!(terms > 10_000, "{terms} terms checked");
}

/// Whatever the reader accepts, the writer writes back as the same characters: over every
/// deletion, insertion and replacement of one character in the corpus's texts, a text either
/// is refused or reads to a state whose text it is.
#[test]
fn whatever_the_reader_accepts_the_writer_writes_back_unchanged() {
    let inserts: Vec<char> = " (),=\"\\_-019azAZRUNTSOPFLHDKtrufe\u{1b}\u{2028}é"
        .chars()
        .collect();
    let mut rng = Pcg32::new(70, 1);
    let mut accepted = 0;
    let mut refused = 0;
    let mut check = |t: &str| match t.parse::<ControllerState>() {
        Ok(s) => {
            assert_eq!(s.to_string(), t, "read {t:?} as {s:?}");
            accepted += 1;
        }
        Err(_) => refused += 1,
    };
    let texts: Vec<String> = corpus()
        .iter()
        .take(600)
        .map(ToString::to_string)
        .chain(plan_examples().iter().map(|(t, _)| t.to_string()))
        .collect();
    for text in &texts {
        let chars: Vec<char> = text.chars().collect();
        for i in 0..=chars.len() {
            if i < chars.len() {
                let mut del = chars.clone();
                del.remove(i);
                check(&del.iter().collect::<String>());
                let mut rep = chars.clone();
                rep[i] = inserts[draw(&mut rng, inserts.len() as u32) as usize];
                check(&rep.iter().collect::<String>());
            }
            let mut ins = chars.clone();
            ins.insert(i, inserts[draw(&mut rng, inserts.len() as u32) as usize]);
            check(&ins.iter().collect::<String>());
        }
    }
    assert!(
        accepted > 1000 && refused > 100_000,
        "accepted {accepted}, refused {refused}"
    );
}
