//! The project file (contract: the project clause): read, versioned, validated, and documented by
//! a worked example this test loads from `docs/project-schema.md` itself, every key statement of
//! its reference held against the reader; a Veredas 1 project folder read into it; the
//! executable's check and migration run on it. The test holds a copy of the reference, so an edit
//! there is an edit here.

mod common;

use std::path::Path;

use chrono::NaiveDate;
use common::plant::{catalog_text, channel_text};
use veredas_core::telegram::Role;
use veredas_portal::project::{self, ActiveConfig, LogKind, Start, Store};

/// The reference, as text.
fn doc() -> String {
    let doc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/project-schema.md");
    std::fs::read_to_string(doc).unwrap().replace("\r\n", "\n")
}

/// The first ```toml block of the reference: the worked example.
fn documented_example() -> String {
    let text = doc();
    let start = text.find("```toml\n").unwrap();
    let body = &text[start + "```toml\n".len()..];
    body[..body.find("```").unwrap()].to_string()
}

/// The prose, its lines joined, so a phrase wrapped across lines still reads.
fn doc_prose() -> String {
    doc().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The count that follows `lead` in the prose, as digits or as a word (`one` to `ten`).
fn count_after(prose: &str, lead: &str) -> usize {
    let at = prose.find(lead).unwrap_or_else(|| panic!("the prose no longer says `{lead}`"));
    let word = prose[at + lead.len()..].split_whitespace().next().unwrap_or("").trim_matches(|c: char| !c.is_alphanumeric());
    let words = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
    words.iter().position(|w| *w == word).or_else(|| word.parse().ok()).unwrap_or_else(|| panic!("no count after `{lead}`: `{word}`"))
}

/// The backticked text that follows `lead` in the prose.
fn quoted_after(prose: &str, lead: &str) -> String {
    let at = prose.find(lead).unwrap_or_else(|| panic!("the prose no longer says `{lead}`"));
    let rest = &prose[at + lead.len()..];
    let rest = &rest[rest.find('`').unwrap() + 1..];
    rest[..rest.find('`').unwrap()].to_string()
}

/// What the reference says of `key` under the `### <section>` heading: the text in the
/// parentheses after the key, `required` or `default …`.
fn statement(section: &str, key: &str) -> String {
    let text = doc();
    let start = text.find(&format!("### {section}")).unwrap_or_else(|| panic!("no section {section}"));
    let body = &text[start..];
    let end = body[4..].find("\n### ").map(|e| e + 4).unwrap_or(body.len());
    let body = &body[..end];
    let lead = format!("`{key}` (");
    let at = body.find(&lead).unwrap_or_else(|| panic!("{section} says nothing of `{key}`"));
    let rest = &body[at + lead.len()..];
    rest[..rest.find(')').unwrap()].to_string()
}

/// The example beside the channel file and the catalog of the references, in a folder of its own.
fn example() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("channel.toml"), channel_text()).unwrap();
    std::fs::write(dir.path().join("catalog.toml"), catalog_text()).unwrap();
    (dir, documented_example())
}

/// The example with one `key = …` line (and the lines a bracket it opens spans) removed from a
/// table: the top level when `table` is `None`, else the first table of that header.
fn without(text: &str, table: Option<&str>, key: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let (from, to) = match table {
        None => (0, lines.iter().position(|l| l.starts_with('[')).unwrap_or(lines.len())),
        Some(h) => {
            let from = lines.iter().position(|l| l.trim() == h).unwrap_or_else(|| panic!("no table {h}")) + 1;
            let to = from + lines[from..].iter().position(|l| l.starts_with('[')).unwrap_or(lines.len() - from);
            (from, to)
        }
    };
    let at = (from..to).find(|&i| lines[i].starts_with(&format!("{key} = "))).unwrap_or_else(|| panic!("no `{key}` in {table:?}"));
    let mut depth: i64 = 0;
    let mut end = at;
    loop {
        for c in lines[end].chars() {
            match c {
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            break;
        }
        end += 1;
    }
    let mut out: Vec<&str> = lines[..at].to_vec();
    out.extend_from_slice(&lines[end + 1..]);
    out.join("\n") + "\n"
}

/// The example with a table's `key = …` line replaced.
fn with(text: &str, table: &str, key: &str, line: &str) -> String {
    let removed = without(text, Some(table), key);
    removed.replacen(&format!("{table}\n"), &format!("{table}\n{line}\n"), 1)
}

fn refused(text: &str, dir: &Path) -> String {
    project::load(text, dir).err().expect("refused").to_string()
}

#[test]
fn the_worked_example_loads_as_its_prose_says() {
    let (dir, text) = example();
    let prose = doc_prose();
    let p = project::load(&text, dir.path()).unwrap();
    assert_eq!(p.name, "the press hall");
    assert_eq!(p.logs.len(), count_after(&prose, "this project has"));
    assert_eq!((p.opc.len(), p.opc[0].rules.len(), p.opc[0].rules[0].tags.iter().map(|t| 1 + t.bits.len()).sum::<usize>()), (1, 1, count_after(&prose, "one rule of")));
    assert_eq!((p.channels.len(), p.channels[0].catalog.messages.len()), (1, count_after(&prose, "whose catalog has")));
    assert_eq!((p.rules.len(), p.tables.len()), (1, 1));
    assert_eq!((&p.start, &p.store, p.writes_armed), (&Start::Midnight, &Store::Ram, false));
    assert_eq!(p.idle_after_ms, count_after(&prose, "idle after") as i64);
    assert_eq!(p.server.map(|a| a.to_string()), Some(quoted_after(&prose, "its server binds")));
    // The logs: a dated family, newest active, with its stamp; a file with two formats.
    let scanner = &p.logs[0];
    assert_eq!((scanner.id.as_str(), scanner.display_name.as_str()), ("scanner", "the scanner"));
    match &scanner.kind {
        LogKind::Family { dir: d, pattern, date_format, active, recursive } => {
            assert!(Path::new(d).ends_with("logs") && Path::new(d).is_absolute(), "{d}");
            assert_eq!((pattern.as_str(), date_format.as_deref(), active, *recursive), (r"(?P<date>\d{8})_scanner(?P<idx>\d+)\.log", Some("%Y%m%d"), &ActiveConfig::Newest, false));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(scanner.timestamp.as_ref().unwrap().formats, ["%Y-%m-%d %H:%M:%S%.3f"]);
    let press = &p.logs[1];
    assert_eq!(press.display_name, "press", "the id names it");
    assert!(matches!(&press.kind, LogKind::File { path } if Path::new(path).ends_with("press.log") && Path::new(path).is_absolute()));
    assert_eq!(press.timestamp.as_ref().unwrap().formats.len(), 2);
    // The rule, the source, the channel, the view.
    let rule = &p.rules[0];
    assert_eq!((rule.name.as_str(), rule.scope.as_deref(), rule.key.as_deref()), ("scans", Some("scanner"), Some("code")));
    assert_eq!(rule.types.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>(), [("code", "int"), ("weight", "float")]);
    let plc = &p.opc[0];
    assert_eq!((plc.source_id.as_str(), plc.display_name.as_str(), plc.user.as_deref(), plc.password_env.as_deref(), plc.poll_ms), ("plc", "the controller", Some("operator"), Some("PLC_PASSWORD"), 500));
    assert_eq!(plc.rules[0].tags[1].bits[0].series, "running");
    let sorter = &p.channels[0];
    assert_eq!((sorter.source_id.as_str(), sorter.display_name.as_str()), ("sorter", "the sorter"));
    assert_eq!(sorter.role, Role::Connect { host: "127.0.0.1".into(), port: 5020 });
    assert_eq!((sorter.replies.len(), sorter.replies[0].on.as_str(), sorter.replies[0].send.as_str(), sorter.replies[0].fields.len()), (1, "announce", "answer", 3));
    assert_eq!(p.tables, [("heavy".to_string(), "SELECT * FROM scans WHERE weight > 20.0".to_string())]);
    // The session it yields: the start resolved on the plant's clock, the view a table.
    let now = NaiveDate::from_ymd_opt(2026, 11, 8).unwrap().and_hms_opt(14, 30, 0).unwrap();
    let cfg = p.session_config(now);
    assert_eq!((cfg.name.as_str(), cfg.start, cfg.writes_armed, cfg.idle_after_ms), ("the press hall", NaiveDate::from_ymd_opt(2026, 11, 8).unwrap().and_hms_opt(0, 0, 0), false, 20_000));
    assert_eq!((cfg.logs.len(), cfg.rules.len(), cfg.opc.len(), cfg.channels.len(), cfg.tables.len()), (2, 1, 1, 1, 1));
    assert_eq!(cfg.tables[0].name, "heavy");
    // The other starts.
    assert_eq!(Start::Live.resolve(now), None);
    assert_eq!(Start::LastHours(4).resolve(now), Some(now - chrono::Duration::hours(4)));
    assert_eq!(Start::At(now).resolve(now), Some(now));
    // The start as the file spells it, read through the file: a date-time with or without its
    // seconds, a date at its midnight, the hours, and a word that is none of them.
    let day = NaiveDate::from_ymd_opt(2026, 11, 8).unwrap();
    for (spelled, at) in [
        ("\"2026-11-08 06:00\"", day.and_hms_opt(6, 0, 0).unwrap()),
        ("\"2026-11-08 06:00:30\"", day.and_hms_opt(6, 0, 30).unwrap()),
        ("\"2026-11-08\"", day.and_hms_opt(0, 0, 0).unwrap()),
    ] {
        let p = project::load(&with(&text, "[session]", "start", &format!("start = {spelled}")), dir.path()).unwrap();
        assert_eq!(p.start, Start::At(at), "{spelled}");
    }
    let p = project::load(&with(&text, "[session]", "start", "start = { last_hours = 6 }"), dir.path()).unwrap();
    assert_eq!(p.start, Start::LastHours(6));
    assert_eq!(
        refused(&with(&text, "[session]", "start", "start = \"yesterdayish\""), dir.path()),
        "session.start: `yesterdayish` is neither `midnight`, a date-time `YYYY-MM-DD HH:MM[:SS]`, a date `YYYY-MM-DD`, nor { last_hours = N }"
    );
}

#[test]
fn the_header_is_refused_before_the_body_by_the_texts_every_file_shares() {
    let (dir, text) = example();
    assert!(refused("not = [toml", dir.path()).starts_with("not a project file: "));
    assert_eq!(refused(&without(&text, None, "format"), dir.path()), "no `format`: a project file states `format` and `version`");
    assert_eq!(refused(&text.replacen("veredas-project", "veredas-bench", 1), dir.path()), "`format` is `veredas-bench`, not `veredas-project`");
    assert_eq!(refused(&without(&text, None, "version"), dir.path()), "no `version`: a project file states `format` and `version`");
    assert_eq!(refused(&text.replacen("version = 1", "version = 2", 1), dir.path()), "version 2 of `veredas-project` is not known here (this reader knows 1)");
    // A misspelled key, at the top and inside a table, is refused with its name.
    let top = refused(&text.replacen("name = ", "nmae = ", 1), dir.path());
    assert!(top.starts_with("not a project file: ") && top.contains("nmae"), "{top}");
    let inside = refused(&text.replacen("pattern = ", "patern = ", 1), dir.path());
    assert!(inside.contains("patern"), "{inside}");
    // Inside an inline table too: the keys of `start`, `store` and `active` are read by name,
    // a typo refused with it (an untagged reader would say only that nothing matched).
    for (table, key, line, typo) in [
        ("[session]", "start", "start = { last_hourz = 4 }", "last_hourz"),
        ("[session]", "start", "start = { last_hours = 4, typo = 1 }", "typo"),
        ("[session]", "store", "store = { disk = \"s.duckdb\", memory_limt_mb = 512 }", "memory_limt_mb"),
        ("[[log]]", "active", "active = { fixed = \"x1.log\", fixd = \"y\" }", "fixd"),
    ] {
        let e = refused(&with(&text, table, key, line), dir.path());
        assert!(e.starts_with("not a project file: ") && e.contains(&format!("`{typo}`")), "{line}: {e}");
    }
    // A key left out of the table, or a shape that is neither word nor table, named too.
    let e = refused(&with(&text, "[session]", "store", "store = { memory_limit_mb = 1 }"), dir.path());
    assert!(e.contains("missing field `disk`"), "{e}");
    let e = refused(&with(&text, "[[log]]", "active", "active = {}"), dir.path());
    assert!(e.contains("missing field `fixed`"), "{e}");
    let e = refused(&with(&text, "[session]", "start", "start = 5"), dir.path());
    assert!(e.contains("invalid type: integer `5`") && e.contains("last_hours"), "{e}");
    let e = refused(&with(&text, "[session]", "store", "store = { disk = \"s.duckdb\", memory_limit_mb = -1 }"), dir.path());
    assert!(e.contains("-1") && e.contains("a whole number"), "{e}");
}

#[test]
fn every_key_statement_is_the_readers() {
    let (dir, text) = example();
    // Required keys: the reference says so, and the reader refuses their absence with the name.
    for (section, table, key) in [
        ("The header and the name", None, "name"),
        ("`[server]`", Some("[server]"), "bind"),
        ("`[[log]]`", Some("[[log]]"), "id"),
        ("`[[rule]]`", Some("[[rule]]"), "name"),
        ("`[[rule]]`", Some("[[rule]]"), "pattern"),
        ("`[[opc]]`", Some("[[opc]]"), "source_id"),
        ("`[[opc]]`", Some("[[opc]]"), "endpoint_url"),
        ("`[[channel]]`", Some("[[channel]]"), "source_id"),
        ("`[[channel]]`", Some("[[channel]]"), "role"),
        ("`[[channel]]`", Some("[[channel]]"), "port"),
        ("`[[channel]]`", Some("[[channel]]"), "channel"),
        ("`[[channel]]`", Some("[[channel]]"), "catalog"),
        ("`[[table]]`", Some("[[table]]"), "name"),
        ("`[[table]]`", Some("[[table]]"), "sql"),
    ] {
        assert!(statement(section, key).starts_with("required"), "{section} {key}: {}", statement(section, key));
        let err = refused(&without(&text, table, key), dir.path());
        assert!(err.contains(&format!("`{key}`")), "{section} {key} left out: {err}");
    }
    assert!(statement("`[[opc]]`", "rules").starts_with("required"));
    assert!(refused(&without(&text, Some("[[opc]]"), "rules"), dir.path()).contains("rule"));
    // Defaults: the reference states them, and the reader reads them when the key is left out.
    assert!(statement("`[session]` (optional; its defaults when absent", "start").starts_with("default live"));
    assert!(statement("`[session]` (optional; its defaults when absent", "store").starts_with("default `\"ram\"`"));
    assert!(statement("`[session]` (optional; its defaults when absent", "writes_armed").starts_with("default `false`"));
    assert!(statement("`[session]` (optional; its defaults when absent", "idle_after_ms").starts_with("default `20000`"));
    let no_session = text.replacen("[session]\nstart = \"midnight\"\nstore = \"ram\"\nwrites_armed = false\nidle_after_ms = 20000\n", "", 1);
    assert!(no_session.len() < text.len());
    let p = project::load(&no_session, dir.path()).unwrap();
    assert_eq!((&p.start, &p.store, p.writes_armed, p.idle_after_ms), (&Start::Live, &Store::Ram, false, 20_000));
    let no_server = text.replacen("[server]\nbind = \"127.0.0.1:8901\"\n", "", 1);
    assert_eq!(project::load(&no_server, dir.path()).unwrap().server, None);
    assert!(statement("`[[log]]`", "display_name").starts_with("default the id"));
    assert!(statement("`[[log]]`", "active").starts_with("default `\"newest\"`"));
    assert!(statement("`[[log]]`", "recursive").starts_with("default `false`"));
    let p = project::load(&without(&without(&text, Some("[[log]]"), "display_name"), Some("[[log]]"), "active"), dir.path()).unwrap();
    assert_eq!(p.logs[0].display_name, "scanner");
    assert!(matches!(&p.logs[0].kind, LogKind::Family { active: ActiveConfig::Newest, recursive: false, .. }));
    assert!(statement("`[[rule]]`", "scope").starts_with("default every log"));
    assert!(statement("`[[rule]]`", "types").starts_with("default text"));
    let p = project::load(&without(&without(&text, Some("[[rule]]"), "scope"), Some("[[rule]]"), "types"), dir.path()).unwrap();
    assert_eq!((p.rules[0].scope.as_deref(), p.rules[0].types.len()), (None, 0));
    assert!(statement("`[[opc]]`", "poll_ms").starts_with("default `1000`"));
    assert!(statement("`[[opc]]`", "display_name").starts_with("default the id"));
    let p = project::load(&without(&without(&text, Some("[[opc]]"), "poll_ms"), Some("[[opc]]"), "display_name"), dir.path()).unwrap();
    assert_eq!((p.opc[0].poll_ms, p.opc[0].display_name.as_str()), (1000, "plc"));
    assert!(statement("`[[channel]]`", "replies").starts_with("default none"));
    let p = project::load(&without(&text, Some("[[channel]]"), "replies"), dir.path()).unwrap();
    assert!(p.channels[0].replies.is_empty());
    // A listening channel binds everywhere unless told where.
    let listening = with(&without(&text, Some("[[channel]]"), "host"), "[[channel]]", "role", "role = \"listen\"");
    let p = project::load(&listening, dir.path()).unwrap();
    assert_eq!(p.channels[0].role, Role::Listen { bind: "0.0.0.0".into(), port: 5020 });
}

#[test]
fn what_cannot_be_a_project_is_refused_by_name() {
    let (dir, text) = example();
    let r = |t: &str| refused(t, dir.path());
    assert_eq!(r(&with(&text, "[[log]]", "dir", "dir = \"logs\"\nfile = \"x.log\"")), "log `scanner`: a log is a `file` or a family (`dir` and `pattern`), not both");
    assert_eq!(r(&without(&text, Some("[[log]]"), "dir")), "log `scanner`: a family needs its `dir` beside `pattern`");
    assert_eq!(r(&without(&without(&without(&without(&text, Some("[[log]]"), "dir"), Some("[[log]]"), "pattern"), Some("[[log]]"), "date_format"), Some("[[log]]"), "active")), "log `scanner`: a log is a `file`, or a family (`dir` and `pattern`); this one is neither");
    assert_eq!(r(&with(&text, "[[log]]", "active", "active = \"latest\"")), "log `scanner`: `active` is `latest`, neither `newest` nor { fixed = \"<name>\" }");
    assert_eq!(r(&without(&text, Some("[[channel]]"), "host")), "channel `sorter`: a connecting channel names its `host`");
    assert_eq!(r(&with(&text, "[[channel]]", "role", "role = \"listen\"")), "channel `sorter`: `host` belongs to a connecting channel; a listening one has `bind`");
    assert_eq!(r(&text.replacen("send = \"answer\"", "send = \"nope\"", 1)), "channel `sorter`: a reply rule's `send` names no message `nope` of its catalog");
    assert_eq!(r(&with(&text, "[[table]]", "name", "name = \"scans\"")), "two tables would be named `scans`: capture rule `scans` and table `scans`");
    // Every kind of owner is named in a clash. (The claims compare names without regard to case,
    // as the store does, but a table name is lowercase by its shape, so no file can show it.)
    assert_eq!(r(&with(&text, "[[table]]", "name", "name = \"plc_main\"")), "two tables would be named `plc_main`: OPC rule `plc_main` of source `plc` and table `plc_main`");
    assert_eq!(r(&with(&text, "[[table]]", "name", "name = \"answer\"")), "two tables would be named `answer`: message `answer` of channel `sorter` and table `answer`");
    assert_eq!(r(&text.replacen("name = \"plc_main\"", "name = \"announce\"", 1)), "two tables would be named `announce`: OPC rule `announce` of source `plc` and message `announce` of channel `sorter`");
    assert_eq!(r(&with(&text, "[[opc]]", "source_id", "source_id = \"scanner\"")), "two sources share the id `scanner`");
    assert_eq!(r(&text.replacen("scope = \"scanner\"", "scope = \"nope\"", 1)), "rule `scans`: the scope `nope` is no log of this project");
    assert_eq!(r("format = \"veredas-project\"\nversion = 1\nname = \"bare\"\n"), "the project has no source: a `[[log]]`, an `[[opc]]` or a `[[channel]]`");
    assert_eq!(r(&without(&text, Some("[[log]]"), "timestamp")), "log `scanner`: a start point in the past needs a timestamp rule, to find the line to start at");
    assert_eq!(r(&text.replacen("weight=(?P<weight>[0-9.]+)", "weight=(?P<weight>[0-9.]+)(?P<Extra>.*)", 1)), "rule `scans`: group `Extra`: the name must start with a lowercase letter or `_`");
    let unread = r(&text.replacen("from = \"item\"", "from = \"nope\"", 1));
    assert!(unread.starts_with("channel `sorter`: ") && unread.contains("nope"), "{unread}");
    let missing = r(&text.replacen("channel = \"channel.toml\"", "channel = \"nope.toml\"", 1));
    assert!(missing.starts_with("channel `sorter`: its channel file: cannot read ") && missing.contains("nope.toml"), "{missing}");
    std::fs::write(dir.path().join("catalog.toml"), "format = \"veredas-catalog\"\nversion = 1\n").unwrap();
    let bad = r(&text);
    assert!(bad.starts_with("channel `sorter`: its catalog (") && bad.contains("catalog.toml"), "{bad}");
}

#[cfg(feature = "store")]
#[test]
fn a_veredas_1_project_folder_is_read_into_the_file_and_what_is_not_carried_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let v1 = dir.path().join("MyPlant");
    std::fs::create_dir_all(&v1).unwrap();
    let w = |name: &str, text: &str| std::fs::write(v1.join(name), text).unwrap();
    w("veredas.json", r#"{ "format": 1, "name": "Press Hall", "notes": "commissioning" }"#);
    w(
        "sources.json",
        r#"[
          { "id": "press-a", "display_name": "Press A", "enabled": true,
            "kind": { "Family": { "dir": "C:\\logs\\pressA", "pattern": "press_(?P<date>\\d{8})_(?P<idx>\\d+)\\.log", "date_format": "%Y%m%d", "active": "NewestByOrder", "recursive": false } },
            "ts": { "pattern": "^(?P<ts>\\S+ \\S+)", "formats": "%Y-%m-%d %H:%M:%S%.3f\n%Y-%m-%d %H:%M:%S" } },
          { "id": "events", "display_name": "", "kind": { "SingleFile": { "path": "C:\\logs\\events.log" } }, "ts": { "pattern": "", "formats": "" } },
          { "id": "plc-1", "display_name": "Line 1 PLC", "enabled": true,
            "kind": { "OpcUa": { "endpoint_url": "opc.tcp://192.168.0.10:4840/", "security": "None", "auth": { "UserPass": { "username": "operator" } }, "poll_ms": 1000 } } },
          { "id": "plc-2", "display_name": "Line 2 PLC",
            "kind": { "OpcUa": { "endpoint_url": "opc.tcp://192.168.0.11:4840/", "security": { "SignAndEncrypt": "Basic256Sha256" }, "auth": "Certificate", "poll_ms": 250 } } },
          { "id": "plc-3", "display_name": "spare", "kind": { "OpcUa": { "endpoint_url": "opc.tcp://192.168.0.12:4840/", "security": "Sign" } } },
          { "id": "msc", "display_name": "missions", "kind": { "Telegram": { "role": "Listen", "address": "0.0.0.0:2000", "endian": "Little", "channel": "CHM" } } },
          { "id": "parked", "display_name": "parked", "enabled": false, "kind": { "SingleFile": { "path": "C:\\logs\\old.log" } } }
        ]"#,
    );
    w(
        "rules.json",
        r#"{ "extracts": [
            { "name": "readings", "pattern": "(?P<sensor>\\w+)=(?P<v>-?\\d+\\.\\d+)", "scope": "press-a", "types": { "v": "float" }, "key_group": "sensor", "enabled": true, "page": "" },
            { "name": "unused", "pattern": "x", "enabled": false },
            { "name": "oldrule", "pattern": "y=(?P<y>\\d+)", "scope": "parked", "enabled": true } ],
          "opc": [ { "name": "line1", "source_id": "plc-1", "enabled": true, "page": "", "store_on_change": true, "tags": [
            { "node_id": "ns=2;s=line1.temperature", "series": "temperature", "ty": "float", "writable": false, "bits": [] },
            { "node_id": "ns=2;s=line1.status", "series": "status_word", "ty": "int", "writable": false, "bits": [ { "bit": 0, "width": 0, "series": "running" }, { "bit": 4, "width": 3, "series": "mode" } ] },
            { "node_id": "ns=2;s=line1.cmd.start", "series": "cmd_start", "writable": true },
            { "node_id": "ns=2;s=line1.flags", "series": "flags", "bits": [ { "bit": 1, "width": 1, "series": "alarm" } ] } ] },
            { "name": "line2", "source_id": "plc-2", "enabled": true, "page": "", "store_on_change": false, "tags": [
            { "node_id": "ns=2;s=line2.speed", "series": "speed", "ty": "float", "writable": false, "bits": [] } ] } ],
          "skew_threshold_ms": 2000 }"#,
    );
    w(
        "tables.json",
        r#"[ { "name": "hot", "sql": "SELECT * FROM readings WHERE v > 20.0", "enabled": true },
            { "name": "limits", "csv_path": "C:\\ref\\limits.csv", "csv_delim": ";", "csv_decimal_comma": true, "enabled": true },
            { "name": "gone", "sql": "SELECT 1", "enabled": false } ]"#,
    );
    // The absolute start as Veredas 1's screens wrote it, with a `T`: carried in this file's spelling.
    w("session.json", r#"{ "start": { "kind": "Absolute", "last_hours": 4, "absolute": "2026-11-08T06:00:00" }, "store": { "mode": "Disk", "disk_dir": "", "memory_limit_mb": 2048, "auto_export_mins": 0, "export_dir": "" } }"#);
    w("commands.json", r#"{ "allow_writes": true, "commands": [] }"#);
    w("layout.json", r#"{ "panels": [] }"#);

    let migrated = project::from_v1(&v1).unwrap();
    let p = project::load(&migrated.toml, &v1).unwrap_or_else(|e| panic!("{e}\n{}", migrated.toml));
    assert_eq!((p.name.as_str(), &p.start, p.writes_armed), ("Press Hall", &Start::At(NaiveDate::from_ymd_opt(2026, 11, 8).unwrap().and_hms_opt(6, 0, 0).unwrap()), true));
    assert_eq!(p.store, Store::Disk { path: v1.join("cache/session.duckdb"), memory_limit_mb: 2048 });
    assert_eq!(p.logs.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["press-a", "events"]);
    assert!(matches!(&p.logs[0].kind, LogKind::Family { date_format: Some(f), active: ActiveConfig::Newest, .. } if f == "%Y%m%d"));
    assert_eq!(p.logs[0].timestamp.as_ref().unwrap().formats.len(), 2, "one format per line");
    assert_eq!((p.logs[1].display_name.as_str(), p.logs[1].timestamp.is_none()), ("events", true));
    assert_eq!((p.rules.len(), p.rules[0].name.as_str(), p.rules[0].key.as_deref()), (1, "readings", Some("sensor")));
    let plc = &p.opc[0];
    assert_eq!((plc.source_id.as_str(), plc.user.as_deref(), plc.password_env.as_deref(), plc.poll_ms), ("plc-1", Some("operator"), Some("PLC_1_PASSWORD"), 1000));
    assert_eq!((plc.rules.len(), plc.rules[0].store_on_change, plc.rules[0].tags.len()), (1, true, 4));
    assert_eq!((plc.rules[0].tags[3].ty.as_str(), plc.rules[0].tags[3].bits.len()), ("int", 1), "bits on a float carried as an int word");
    assert_eq!((plc.rules[0].tags[1].bits[0].width, plc.rules[0].tags[2].ty.as_str(), plc.rules[0].tags[2].writable), (1, "float", true));
    // The second server, carried anonymous and unsecured with its one rule; the third, with no
    // rule to map its tags, is not carried (a source needs its rules).
    assert_eq!(p.opc.len(), 2, "{}", migrated.toml);
    let plc2 = &p.opc[1];
    assert_eq!((plc2.source_id.as_str(), plc2.user.as_deref(), plc2.password_env.as_deref(), plc2.poll_ms, plc2.rules.len()), ("plc-2", None, None, 250, 1));
    assert_eq!(p.tables.len(), 2);
    assert_eq!(p.tables[0], ("hot".to_string(), "SELECT * FROM readings WHERE v > 20.0".to_string()));
    assert!(p.tables[1].1.starts_with("SELECT * FROM read_csv(") && p.tables[1].1.contains("decimal_separator"), "{}", p.tables[1].1);
    assert!(p.channels.is_empty());
    let notes = migrated.notes.join("\n");
    for expected in [
        "source `parked` was disabled",
        "opc rule `line1`, tag `ns=2;s=line1.flags`: bits are carved out of an int word; its type `float` became `int`",
        "source `msc` (a telegram channel `CHM` at `0.0.0.0:2000`) is not carried",
        "opc `plc-1`: the password lived in the machine",
        "`PLC_1_PASSWORD`",
        "rule `unused` was disabled",
        "table `gone` was disabled",
        "not carried, by design: the dashboard layout",
        "opc `plc-2`: security `{\"SignAndEncrypt\":\"Basic256Sha256\"}` was never implemented; the source is carried without it",
        "opc `plc-2`: certificate authentication was never implemented; the source is carried anonymous",
        "opc `plc-3`: security `Sign` was never implemented",
        "opc `plc-3`: no rule mapped its tags: not carried",
        "rule `oldrule`: its scope `parked` is a log not carried: not carried (write it back with a scope the project has)",
        "the rules' skew-monitor threshold",
    ] {
        assert!(notes.contains(expected), "no note `{expected}` in:\n{notes}");
    }
    assert_eq!(migrated.notes.len(), 12, "{notes}");
    assert!(migrated.toml.starts_with("# Read into Veredas 2 from a Veredas 1 project folder"));
    // An absolute start in the other spelling is carried too; one that is no date-time is named
    // and the session starts live, never a guess.
    w("session.json", r#"{ "start": { "kind": "Absolute", "absolute": "2026-07-17 09:15" } }"#);
    let again = project::from_v1(&v1).unwrap();
    assert_eq!(project::load(&again.toml, &v1).unwrap().start, Start::At(NaiveDate::from_ymd_opt(2026, 7, 17).unwrap().and_hms_opt(9, 15, 0).unwrap()));
    w("session.json", r#"{ "start": { "kind": "Absolute", "absolute": "yesterdayish" } }"#);
    let again = project::from_v1(&v1).unwrap();
    assert_eq!(project::load(&again.toml, &v1).unwrap().start, Start::Live);
    assert!(again.notes.iter().any(|n| n == "session.start: the absolute start `yesterdayish` is not a date-time (`YYYY-MM-DD HH:MM[:SS]`, a `T` or a space between) nor a date; the session starts live"), "{}", again.notes.join("\n"));
    // Every session setting Veredas 1 had and this file has not is named: a start with an offset
    // (a date-time, but not on the plant's clock), a store mode it never had, a memory limit on an
    // in-memory store, the auto-export, and zero hours carried as one.
    let import = |session: &str| {
        w("session.json", session);
        let m = project::from_v1(&v1).unwrap();
        (project::load(&m.toml, &v1).unwrap(), m.notes.join("\n"))
    };
    let (p, notes) = import(r#"{ "start": { "kind": "Absolute", "absolute": "2026-11-08T06:00:00+02:00" } }"#);
    assert_eq!(p.start, Start::Live);
    assert!(notes.contains("session.start: the absolute start `2026-11-08T06:00:00+02:00` carries an offset, and this file's times are the plant's clock; set `start` by hand; the session starts live"), "{notes}");
    let (p, notes) = import(r#"{ "start": { "kind": "LastHours", "last_hours": 0 }, "store": { "mode": "Hybrid", "disk_dir": "d", "memory_limit_mb": 512, "auto_export_mins": 15, "export_dir": "exp" } }"#);
    assert_eq!((&p.start, &p.store), (&Start::LastHours(1), &Store::Ram));
    for expected in [
        "session.start: `last_hours` was 0 and is carried as 1 (a start names at least one hour)",
        "session.store: mode `Hybrid` is not known (Veredas 1 had `Ram` and `Disk`); the store is in memory",
        "session.store: the memory limit of 512 MB is not carried: an in-memory store has none here (a limit belongs to a disk store)",
        "session.store: the auto-export (every 15 minutes to `exp`) is not carried: a snapshot is taken on request (C-041), not on a timer",
    ] {
        assert!(notes.contains(expected), "no note `{expected}` in:\n{notes}");
    }
    let (p, notes) = import(r#"{ "store": { "mode": "Ram", "memory_limit_mb": 0 } }"#);
    assert_eq!((&p.start, &p.store), (&Start::Live, &Store::Ram));
    assert!(!notes.contains("session.store"), "an in-memory store with no limit needs no note:\n{notes}");
    // A folder that leaves nothing to carry yields no file: the import is refused with the
    // reason the file would have been, and its notes.
    let bare = dir.path().join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    std::fs::write(bare.join("veredas.json"), r#"{ "format": 1, "name": "bare" }"#).unwrap();
    std::fs::write(bare.join("sources.json"), r#"[ { "id": "off", "display_name": "", "enabled": false, "kind": { "SingleFile": { "path": "a.log" } } } ]"#).unwrap();
    let e = project::from_v1(&bare).unwrap_err().to_string();
    assert!(e.starts_with("the file read from the folder does not load: the project has no source") && e.contains("note: source `off` was disabled"), "{e}");
    // A folder that is no Veredas 1 project, and a manifest of another format.
    let not = project::from_v1(dir.path()).unwrap_err().to_string();
    assert!(not.starts_with("the Veredas 1 project's veredas.json: cannot read "), "{not}");
    std::fs::write(v1.join("veredas.json"), r#"{ "format": 2 }"#).unwrap();
    assert_eq!(project::from_v1(&v1).unwrap_err().to_string(), "veredas.json: format 2 is not the Veredas 1 project format (1)");
}

#[test]
fn the_executable_checks_a_project_and_reads_a_veredas_1_folder() {
    let Some(exe) = option_env!("CARGO_BIN_EXE_veredas-portal") else {
        return; // the light build has no executable
    };
    let (dir, text) = example();
    let file = dir.path().join("project.toml");
    std::fs::write(&file, &text).unwrap();
    let out = std::process::Command::new(exe).arg(&file).arg("--check").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    for line in ["project `the press hall`", "session: from midnight; store in memory; writes off; idle after 20000 ms", "server: 127.0.0.1:8901", "log `scanner`: family", "rule `scans` on `scanner`", "opc `plc`: opc.tcp://127.0.0.1:4840/ every 500 ms, 1 rule(s)", "channel `sorter`: 6 message(s) in its catalog, 1 reply rule(s)", "table `heavy`"] {
        assert!(stdout.contains(line), "no `{line}` in:\n{stdout}");
    }
    // A bad file: the reason on stderr, exit 2. No file: usage, exit 1.
    std::fs::write(&file, text.replacen("version = 1", "version = 2", 1)).unwrap();
    let out = std::process::Command::new(exe).arg(&file).arg("--check").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("version 2 of `veredas-project` is not known here"));
    let out = std::process::Command::new(exe).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage:"));
    // A Veredas 1 folder read into a file that loads.
    let v1 = dir.path().join("v1");
    std::fs::create_dir_all(&v1).unwrap();
    std::fs::write(v1.join("veredas.json"), r#"{ "format": 1, "name": "old" }"#).unwrap();
    std::fs::write(v1.join("sources.json"), r#"[ { "id": "a", "display_name": "", "kind": { "SingleFile": { "path": "a.log" } } } ]"#).unwrap();
    let out = std::process::Command::new(exe).arg("--from-v1").arg(&v1).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let toml = String::from_utf8_lossy(&out.stdout);
    let p = project::load(&toml, &v1).unwrap_or_else(|e| panic!("{e}\n{toml}"));
    assert_eq!((p.name.as_str(), p.logs.len()), ("old", 1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("note: not carried, by design"));
}

/// The run mode: the executable serves where the project says (the address it bound, the port
/// the system's when asked for 0), prints every source's health once a second, and ends well
/// when stdin closes.
#[test]
fn the_executable_runs_a_project_serving_and_reporting_until_stdin_closes() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let Some(exe) = option_env!("CARGO_BIN_EXE_veredas-portal") else {
        return; // the light build has no executable
    };
    let (dir, text) = example();
    std::fs::create_dir_all(dir.path().join("logs")).unwrap();
    std::fs::write(dir.path().join("press.log"), "").unwrap();
    // The plant and the channel's peer are away, so those two sources retry; the logs wait.
    let text = with(&text, "[server]", "bind", "bind = \"127.0.0.1:0\"");
    let file = dir.path().join("project.toml");
    std::fs::write(&file, &text).unwrap();
    // The OPC source's password is the variable the project names, read when the session starts.
    let mut child = Command::new(exe).arg(&file).env("PLC_PASSWORD", "secret").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut errs = child.stderr.take().unwrap();
    let stderr = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = errs.read_to_string(&mut s);
        s
    });
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    let mut before = String::new();
    let serving = loop {
        line.clear();
        if out.read_line(&mut line).unwrap() == 0 {
            let status = child.wait().unwrap();
            panic!("the executable ended ({status:?}) before it served:\n{before}\nstderr:\n{}", stderr.join().unwrap());
        }
        if let Some(rest) = line.trim().strip_prefix("serving on http://") {
            break rest.trim_end_matches('/').to_string();
        }
        before.push_str(&line);
    };
    assert!(serving.starts_with("127.0.0.1:") && serving != "127.0.0.1:0", "the address bound, not the one asked: {serving}");
    let mut s = TcpStream::connect(&serving).unwrap();
    s.write_all(format!("GET /api/session HTTP/1.1\r\nHost: {serving}\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut answer = String::new();
    s.read_to_string(&mut answer).unwrap();
    assert!(answer.starts_with("HTTP/1.1 200") && answer.contains("the press hall"), "{answer}");
    // A status line a second, naming every source: two of them, about a second apart.
    let mut seen = Vec::new();
    while seen.len() < 2 {
        line.clear();
        assert!(out.read_line(&mut line).unwrap() > 0, "the executable ended before two status lines");
        if line.contains("| ") && line.contains(" values") {
            for id in ["scanner:", "press:", "plc:", "sorter:"] {
                assert!(line.contains(id), "no `{id}` in the status line: {line}");
            }
            seen.push(Instant::now());
        }
    }
    let gap = seen[1] - seen[0];
    assert!(gap >= Duration::from_millis(500) && gap <= Duration::from_secs(3), "a status line a second: {gap:?} between two");
    // Stdin closed: the session stops and the process ends well, nothing on stderr.
    drop(child.stdin.take());
    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    let status = child.wait().unwrap();
    let errs = stderr.join().unwrap();
    assert!(status.success() && errs.trim().is_empty(), "exit {status:?}; stderr:\n{errs}");
}

/// The reference is the text this test holds: an edit there is an edit here, beside its pins.
#[test]
fn the_reference_is_the_text_this_test_holds() {
    let normalize = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(normalize(&doc()), normalize(REFERENCE));
}

const REFERENCE: &str = r##"# The project file: `veredas-project`, version 1

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
did not carry). No device is named anywhere: a plant's project is a folder of files like the ones
below, kept outside the suite's code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a misspelled
key refused, an unknown format or version refused rather than guessed. From this file on, the
header is read by one reader, `veredas_core::schema`, that every file of the suite shares.

## The worked example: a scanner's log family, a controller, and a sorter answered by rule

A scanner writes daily files that rotate on a line count, stamped to the millisecond; a controller
serves a speed and a run flag over OPC UA under a user whose password the operator sets in the
environment, never in the file; the machine of the controller reference listens on port 5020 and
the project dials it, answering every announcement by the reply rule of the channel clause; a
view keeps the heavy scans. The session starts at the day's midnight, keeps its store in memory,
and leaves its writes off; the server listens on the loopback. `crates/veredas-portal/tests/project.rs`
writes the channel file and the catalog beside this block, loads it, checks every statement
below against what it read, and runs the executable's check on it; it holds a copy of this text,
so an edit here is an edit there.

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
password_env = "PLC_PASSWORD"
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
in memory, writes off, idle after 20000 ms; its server binds `127.0.0.1:8901`. Every table name is
taken once across the rules, the OPC rules, the catalog's messages and the views: a second
`scans` anywhere is refused, naming both owners.

## Keys

Every key is spelled as written here; a key the reader does not know is refused with its name.
A relative path (`file`, `dir`, `disk`, `channel`, `catalog`) is resolved against the project
file's folder.

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
- `store` (default `"ram"`): `"ram"`, gone when the session ends, or
  `{ disk = "<file>", memory_limit_mb = 0 }`, a database file the engine pages out of core,
  `memory_limit_mb` its ceiling (0 for the engine's default).
- `writes_armed` (default `false`): the master switch. Off, every write path (an OPC write, a
  channel send, a reply rule) is refused before anything reaches the plant; the switch is set in
  process, never over the wire.
- `idle_after_ms` (default `20000`): a live source silent this long reads as idle; at least 1.

### `[server]` (optional: absent, nothing is served)
- `bind` (required): the address `host:port` the server listens on.

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
- `user` (optional) and `password_env` (required with `user`): the name of an environment
  variable holding the password, never the password (`docs/questions.md` Q15). The session
  reads it when it starts; the check does not need it set.
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
- `sql` (required): its SELECT over the base tables and the other views.
"##;
