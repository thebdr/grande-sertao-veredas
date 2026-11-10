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
use veredas_portal::secrets::{Key256, SecretsFile, SecretsSource};

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
    assert_eq!((plc.source_id.as_str(), plc.display_name.as_str(), plc.user.as_deref(), plc.secret.as_deref(), plc.poll_ms), ("plc", "the controller", Some("operator"), Some("plc"), 500));
    // The secrets (Q15): the file the prose names beside the project, the variable it names.
    assert_eq!(p.secrets, Some(SecretsSource { file: dir.path().join(quoted_after(&prose, "its secrets sit in")), key_env: quoted_after(&prose, "opened with the key the variable") }));
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
    assert_eq!(cfg.secrets, p.secrets, "the session opens the same secrets");
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
    // The store's two disk forms: a file pinned by name, and a folder holding a file of each
    // run's own, named from the project and the start (docs/questions.md Q16).
    let p = project::load(&with(&text, "[session]", "store", "store = { disk = \"s.duckdb\", memory_limit_mb = 64 }"), dir.path()).unwrap();
    assert_eq!(p.store, Store::Disk { path: dir.path().join("s.duckdb"), memory_limit_mb: 64 });
    let p = project::load(&with(&text, "[session]", "store", "store = { dir = \"runs\", memory_limit_mb = 256 }"), dir.path()).unwrap();
    assert_eq!(p.store, Store::PerSession { dir: dir.path().join("runs"), memory_limit_mb: 256 });
    let started = NaiveDate::from_ymd_opt(2026, 11, 8).unwrap().and_hms_opt(6, 0, 30).unwrap();
    assert_eq!(project::session_store_file("the press hall", started), "the_press_hall-20261108-060030.duckdb");
    assert_eq!(project::session_store_file("  ", started), "session-20261108-060030.duckdb");
    assert_eq!(
        p.session_config(started).residency,
        veredas_portal::store::Residency::Disk { path: dir.path().join("runs").join("the_press_hall-20261108-060030.duckdb"), memory_limit_mb: 256 }
    );
    let e = refused(&with(&text, "[session]", "store", "store = { disk = \"s.duckdb\", dir = \"runs\" }"), dir.path());
    assert!(e.contains("`disk` (a file pinned by name) and `dir` (a file per session) exclude each other: name one"), "{e}");
    assert_eq!(refused(&with(&text, "[session]", "store", "store = { dir = \" \" }"), dir.path()), "session.store: `dir` is empty: name the folder the session files go in");
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
        ("`[secrets]`", Some("[secrets]"), "key_env"),
        ("`[[opc]]`", Some("[[opc]]"), "secret"),
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
    assert!(statement("`[secrets]`", "file").starts_with("default `\"secrets.toml\"`"));
    let p = project::load(&text.replacen("[secrets]\n", "[secrets]\nfile = \"keys/plant.toml\"\n", 1), dir.path()).unwrap();
    assert_eq!(p.secrets.unwrap().file, dir.path().join("keys/plant.toml"), "the secrets file resolved beside the project");
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
    // The secrets (Q15): a source naming an entry needs the table saying where the file and the
    // key are; the table's own keys are checked; a secret and a user go together.
    let no_table = text.replacen("[secrets]\nkey_env = \"VEREDAS_KEY\"\n", "", 1);
    assert!(no_table.len() < text.len());
    assert_eq!(r(&no_table), "opc `plc`: it names a secret, but the project has no `[secrets]` table (its `file` and `key_env`)");
    let anonymous = without(&without(&no_table, Some("[[opc]]"), "secret"), Some("[[opc]]"), "user");
    assert!(project::load(&anonymous, dir.path()).unwrap().secrets.is_none(), "no secret named, no table needed");
    assert_eq!(r(&with(&text, "[secrets]", "key_env", "key_env = \"  \"")), "secrets: `key_env` is empty: name the environment variable holding the key");
    assert_eq!(r(&text.replacen("[secrets]\n", "[secrets]\nfile = \" \"\n", 1)), "secrets: `file` is empty: name the secrets file, or leave the key out for `secrets.toml`");
    let misspelled = r(&text.replacen("[secrets]\n", "[secrets]\nkey_evn = \"X\"\n", 1));
    assert!(misspelled.contains("`key_evn`"), "{misspelled}");
    assert_eq!(r(&without(&text, Some("[[opc]]"), "user")), "opc `plc`: source `plc`: `secret` is given but no `user` is");
    assert_eq!(r(&with(&text, "[[opc]]", "secret", "secret = \" \"")), "opc `plc`: source `plc`: the user `operator` names no `secret`");
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
            { "name": "gone", "sql": "SELECT 1", "enabled": false },
            { "name": "oldview", "sql": "SELECT y FROM oldrule", "enabled": true },
            { "name": "older", "sql": "SELECT * FROM OldView", "enabled": true },
            { "name": "v_csv", "csv_path": "C:\\ref\\oldrule.csv", "enabled": true },
            { "name": "v_str", "sql": "SELECT * FROM readings WHERE key <> 'oldrule' AND v > (SELECT avg(v) FROM readings)", "enabled": true },
            { "name": "v_msg", "sql": "SELECT * FROM chm_status", "enabled": true },
            { "name": "n_quoted", "sql": "SELECT * FROM \"oldrule\"", "enabled": true },
            { "name": "n_comma", "sql": "SELECT * FROM readings, oldrule", "enabled": true },
            { "name": "n_col", "sql": "SELECT nope FROM readings", "enabled": true },
            { "name": "v_extract", "sql": "SELECT EXTRACT(year FROM ts) AS y FROM readings", "enabled": true },
            { "name": "v_distinct", "sql": "SELECT * FROM readings WHERE v IS DISTINCT FROM 0", "enabled": true },
            { "name": "v_cte", "sql": "WITH x(a) AS (SELECT 1) SELECT * FROM x", "enabled": true },
            { "name": "reader_after", "sql": "SELECT * FROM v_csv", "enabled": true },
            { "name": "q3", "sql": "SELECT * FROM q2", "enabled": true },
            { "name": "q2", "sql": "SELECT * FROM q1", "enabled": true },
            { "name": "q1", "sql": "SELECT * FROM csv2", "enabled": true },
            { "name": "reader_before", "sql": "SELECT * FROM (SELECT * FROM csv2) t", "enabled": true },
            { "name": "csv2", "csv_path": "C:\\ref\\second.csv", "enabled": true },
            { "name": "both1", "sql": "SELECT * FROM v_csv, oldrule", "enabled": true },
            { "name": "both2", "sql": "SELECT * FROM oldrule, v_csv", "enabled": true },
            { "name": "col_stub", "sql": "SELECT missing_col FROM v_csv", "enabled": true },
            { "name": "rd_temp", "sql": "SELECT * FROM temp.v_csv", "enabled": true },
            { "name": "rr2", "sql": "SELECT * FROM reader_after", "enabled": true },
            { "name": "rr3", "sql": "SELECT * FROM rr2", "enabled": true },
            { "name": "bad_mid", "sql": "SELECT reader_after.*, readings.nope FROM reader_after JOIN readings ON true", "enabled": true },
            { "name": "bad_top", "sql": "SELECT * FROM bad_mid", "enabled": true },
            { "name": "col_mid", "sql": "SELECT reader_after.a FROM reader_after", "enabled": true },
            { "name": "col_top", "sql": "SELECT * FROM col_mid", "enabled": true } ]"#,
    );
    // The absolute start as Veredas 1's screens wrote it, with a `T`: carried in this file's spelling.
    w("session.json", r#"{ "start": { "kind": "Absolute", "last_hours": 4, "absolute": "2026-11-08T06:00:00" }, "store": { "mode": "Disk", "disk_dir": "", "memory_limit_mb": 2048, "auto_export_mins": 0, "export_dir": "" } }"#);
    w("commands.json", r#"{ "allow_writes": true, "commands": [] }"#);
    w("layout.json", r#"{ "panels": [] }"#);

    let migrated = project::from_v1(&v1).unwrap();
    let p = project::load(&migrated.toml, &v1).unwrap_or_else(|e| panic!("{e}\n{}", migrated.toml));
    assert_eq!((p.name.as_str(), &p.start, p.writes_armed), ("Press Hall", &Start::At(NaiveDate::from_ymd_opt(2026, 11, 8).unwrap().and_hms_opt(6, 0, 0).unwrap()), true));
    assert_eq!(p.store, Store::PerSession { dir: v1.join("cache"), memory_limit_mb: 2048 }, "a folder, a file per run, as Veredas 1 had it");
    assert_eq!(p.logs.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["press-a", "events"]);
    assert!(matches!(&p.logs[0].kind, LogKind::Family { date_format: Some(f), active: ActiveConfig::Newest, .. } if f == "%Y%m%d"));
    assert_eq!(p.logs[0].timestamp.as_ref().unwrap().formats.len(), 2, "one format per line");
    assert_eq!((p.logs[1].display_name.as_str(), p.logs[1].timestamp.is_none()), ("events", true));
    assert_eq!((p.rules.len(), p.rules[0].name.as_str(), p.rules[0].key.as_deref()), (1, "readings", Some("sensor")));
    let plc = &p.opc[0];
    assert_eq!((plc.source_id.as_str(), plc.user.as_deref(), plc.secret.as_deref(), plc.poll_ms), ("plc-1", Some("operator"), Some("plc-1"), 1000));
    assert_eq!(p.secrets, Some(SecretsSource { file: v1.join("secrets.toml"), key_env: "VEREDAS_KEY".into() }), "a password to seal: the secrets file beside the project, the suite's variable");
    assert!(migrated.toml.contains("[secrets]") && migrated.toml.contains("key_env = \"VEREDAS_KEY\"") && migrated.toml.contains("secret = \"plc-1\"") && !migrated.toml.contains("password"), "{}", migrated.toml);
    assert_eq!((plc.rules.len(), plc.rules[0].store_on_change, plc.rules[0].tags.len()), (1, true, 4));
    assert_eq!((plc.rules[0].tags[3].ty.as_str(), plc.rules[0].tags[3].bits.len()), ("int", 1), "bits on a float carried as an int word");
    assert_eq!((plc.rules[0].tags[1].bits[0].width, plc.rules[0].tags[2].ty.as_str(), plc.rules[0].tags[2].writable), (1, "float", true));
    // The second server, carried anonymous and unsecured with its one rule; the third, with no
    // rule to map its tags, is not carried (a source needs its rules).
    assert_eq!(p.opc.len(), 2, "{}", migrated.toml);
    let plc2 = &p.opc[1];
    assert_eq!((plc2.source_id.as_str(), plc2.user.as_deref(), plc2.secret.as_deref(), plc2.poll_ms, plc2.rules.len()), ("plc-2", None, None, 250, 1));
    assert_eq!(p.tables.len(), 18, "{}", migrated.notes.join("\n"));
    assert_eq!(p.tables[0], ("hot".to_string(), "SELECT * FROM readings WHERE v > 20.0".to_string()));
    assert!(p.tables[1].1.starts_with("SELECT * FROM read_csv(") && p.tables[1].1.contains("decimal_separator"), "{}", p.tables[1].1);
    assert!(p.channels.is_empty());
    let notes = migrated.notes.join("\n");
    for expected in [
        "source `parked` was disabled",
        "opc rule `line1`, tag `ns=2;s=line1.flags`: bits are carved out of an int word; its type `float` became `int`",
        "source `msc` (a telegram channel `CHM` at `0.0.0.0:2000`) is not carried",
        "opc `plc-1`: the password lived in the machine",
        "seal it as the entry `plc-1` with `veredas-portal --secret <project.toml> plc-1`, the key in the variable `VEREDAS_KEY`",
        "rule `unused` was disabled",
        "table `gone` was disabled",
        "not carried, by design: the dashboard layout",
        "opc `plc-2`: security `{\"SignAndEncrypt\":\"Basic256Sha256\"}` was never implemented; the source is carried without it",
        "opc `plc-2`: certificate authentication was never implemented; the source is carried anonymous",
        "opc `plc-3`: security `Sign` was never implemented",
        "opc `plc-3`: no rule mapped its tags: not carried",
        "rule `oldrule`: its scope `parked` is a log not carried: not carried (write it back with a scope the project has)",
        "table `oldview`: the engine refuses it over the tables this file creates (Catalog Error: Table with name oldrule does not exist!): not carried (write it back once what it reads is)",
        "table `older`: the engine refuses it over the tables this file creates (Catalog Error: Table with name OldView does not exist!): not carried",
        "table `v_msg`: the engine refuses it over the tables this file creates (Catalog Error: Table with name chm_status does not exist!): not carried",
        "table `n_quoted`: the engine refuses it over the tables this file creates (Catalog Error: Table with name oldrule does not exist!): not carried",
        "table `n_comma`: the engine refuses it over the tables this file creates (Catalog Error: Table with name oldrule does not exist!): not carried",
        "table `n_col`: the engine refuses it over the tables this file creates (Binder Error: Referenced column \"nope\" not found",
        "table `limits`: the engine cannot read its file here (IO Error: No files found that match the pattern \"C:\\ref\\limits.csv\"): carried as written for the plant's machine; check the path",
        "table `v_csv`: the engine cannot read its file here (IO Error:",
        "table `csv2`: the engine cannot read its file here (IO Error:",
        "table `reader_after`: it reads `v_csv`, a view over a file not here: carried as written; check it with the file",
        "table `reader_before`: it reads `csv2`, a view over a file not here: carried as written; check it with the file",
        "table `q1`: it reads `csv2`, a view over a file not here: carried as written; check it with the file",
        "table `q2`: it reads `q1`, a view over a file not here: carried as written; check it with the file",
        "table `q3`: it reads `q2`, a view over a file not here: carried as written; check it with the file",
        "table `rr2`: it reads `reader_after`, a view over a file not here: carried as written; check it with the file",
        "table `rr3`: it reads `rr2`, a view over a file not here: carried as written; check it with the file",
        "table `both1`: the engine refuses it over the tables this file creates (Catalog Error: Table with name oldrule does not exist!): not carried",
        "table `both2`: the engine refuses it over the tables this file creates (Catalog Error: Table with name oldrule does not exist!): not carried",
        "table `col_stub`: it reads `v_csv`, a view over a file not here: carried as written; check it with the file",
        "table `rd_temp`: the engine refuses it over the tables this file creates (Catalog Error: Table with name v_csv does not exist!): not carried",
        "table `bad_mid`: the engine refuses it over the tables this file creates (Binder Error: Table \"readings\" does not have a column named \"nope\"",
        "table `bad_top`: the engine refuses it over the tables this file creates (Catalog Error: Table with name bad_mid does not exist!): not carried",
        "table `col_mid`: it reads `reader_after`, a view over a file not here: carried as written; check it with the file",
        "table `col_top`: it reads `col_mid`, a view over a file not here: carried as written; check it with the file",
        "the rules' skew-monitor threshold",
    ] {
        assert!(notes.contains(expected), "no note `{expected}` in:\n{notes}");
    }
    assert_eq!(migrated.notes.len(), 36, "{notes}");
    // The engine, not a reading of the SQL, decides: the three CSV-linked views (their files on
    // the plant's machine, not here) are carried as written and named, and so are the views that
    // read one, listed after it or before it, or reading a column only the file can show, or
    // reading it through other views, three deep in either order; a view that reads a set-aside
    // view beside a table nothing creates falls for that table whichever it names first; a view
    // whose literal and subquery spell a dropped name, EXTRACT's FROM, IS DISTINCT FROM and a CTE
    // with a column list are carried, all in order.
    assert_eq!(
        p.tables.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ["hot", "limits", "v_csv", "v_str", "v_extract", "v_distinct", "v_cte", "reader_after", "q3", "q2", "q1", "reader_before", "csv2", "col_stub", "rr2", "rr3", "col_mid", "col_top"]
    );
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
    // To the minute and with a `Z` too: an offset, not "no date-time".
    let (p, notes) = import(r#"{ "start": { "kind": "Absolute", "absolute": "2026-11-08T06:00Z" } }"#);
    assert_eq!(p.start, Start::Live);
    assert!(notes.contains("the absolute start `2026-11-08T06:00Z` carries an offset"), "{notes}");
    let (p, notes) = import(r#"{ "start": { "kind": "Absolute", "absolute": "2026-11-08T06:00+02" } }"#);
    assert_eq!(p.start, Start::Live);
    assert!(notes.contains("the absolute start `2026-11-08T06:00+02` carries an offset"), "an offset to the hour: {notes}");
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
    let out = std::process::Command::new(exe).arg(&file).arg("--check").env_remove("VEREDAS_KEY").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    for line in ["project `the press hall`", "session: from midnight; store in memory; writes off; idle after 20000 ms", "server: 127.0.0.1:8901", "secrets: ", "opened with the key `VEREDAS_KEY` holds", "log `scanner`: family", "rule `scans` on `scanner`", "opc `plc`: opc.tcp://127.0.0.1:4840/ every 500 ms, 1 rule(s)", "channel `sorter`: 6 message(s) in its catalog, 1 reply rule(s)", "table `heavy`"] {
        assert!(stdout.contains(line), "no `{line}` in:\n{stdout}");
    }
    // The check needs neither the key (the variable removed above) nor the secrets file: one
    // beside the project that is no secrets file leaves the check untouched.
    std::fs::write(dir.path().join("secrets.toml"), "format = \"veredas-project\"\nversion = 1\n").unwrap();
    let out = std::process::Command::new(exe).arg(&file).arg("--check").env_remove("VEREDAS_KEY").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    std::fs::remove_file(dir.path().join("secrets.toml")).unwrap();
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
    // A log, and four servers with passwords: one with no rule (not carried, so it names no
    // entry and takes no word, though its id is the word a later one needs), one whose id is not
    // a plain word, one whose id is the plain word the previous one's entry takes, and one whose
    // id starts with `--`.
    std::fs::write(v1.join("sources.json"), r#"[ { "id": "a", "display_name": "", "kind": { "SingleFile": { "path": "a.log" } } }, { "id": "line_2_plc_2", "display_name": "", "kind": { "OpcUa": { "endpoint_url": "opc.tcp://192.168.0.11:4840/", "security": "None", "auth": { "UserPass": { "username": "operator" } }, "poll_ms": 1000 } } }, { "id": "line 2 plc", "display_name": "", "kind": { "OpcUa": { "endpoint_url": "opc.tcp://192.168.0.12:4840/", "security": "None", "auth": { "UserPass": { "username": "operator" } }, "poll_ms": 1000 } } }, { "id": "line_2_plc", "display_name": "", "kind": { "OpcUa": { "endpoint_url": "opc.tcp://192.168.0.13:4840/", "security": "None", "auth": { "UserPass": { "username": "operator" } }, "poll_ms": 1000 } } }, { "id": "--plc", "display_name": "", "kind": { "OpcUa": { "endpoint_url": "opc.tcp://192.168.0.14:4840/", "security": "None", "auth": { "UserPass": { "username": "operator" } }, "poll_ms": 1000 } } } ]"#).unwrap();
    std::fs::write(v1.join("rules.json"), r#"{ "extracts": [], "opc": [ { "name": "line2", "source_id": "line 2 plc", "enabled": true, "page": "", "store_on_change": false, "tags": [ { "node_id": "ns=2;s=x", "series": "x", "ty": "float", "writable": false, "bits": [] } ] }, { "name": "line2b", "source_id": "line_2_plc", "enabled": true, "page": "", "store_on_change": false, "tags": [ { "node_id": "ns=2;s=y", "series": "y", "ty": "float", "writable": false, "bits": [] } ] }, { "name": "dashes", "source_id": "--plc", "enabled": true, "page": "", "store_on_change": false, "tags": [ { "node_id": "ns=2;s=z", "series": "z", "ty": "float", "writable": false, "bits": [] } ] } ] }"#).unwrap();
    let out = std::process::Command::new(exe).arg("--from-v1").arg(&v1).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let toml = String::from_utf8_lossy(&out.stdout);
    let p = project::load(&toml, &v1).unwrap_or_else(|e| panic!("{e}\n{toml}"));
    // The entries are plain words made from the ids of the sources carried, kept distinct, a
    // leading `-` an `_`, so the notes' calls need no shell's quoting and the tool never reads
    // one as an option; the calls as given seal their entries.
    assert_eq!((p.name.as_str(), p.logs.len(), p.opc.iter().map(|o| o.secret.as_deref()).collect::<Vec<_>>()), ("old", 1, vec![Some("line_2_plc"), Some("line_2_plc_2"), Some("__plc")]));
    let notes = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(notes.contains("note: not carried, by design"));
    assert!(notes.contains("opc `line_2_plc_2`: no rule mapped its tags: not carried") && !notes.contains("opc `line_2_plc_2`: the password lived"), "{notes}");
    assert!(notes.contains("opc `line 2 plc`: the password lived in the machine's credential store; seal it as the entry `line_2_plc` with `veredas-portal --secret <project.toml> line_2_plc`, the key in the variable `VEREDAS_KEY`"), "{notes}");
    assert!(notes.contains("opc `line_2_plc`: the password lived in the machine's credential store; seal it as the entry `line_2_plc_2` with `veredas-portal --secret <project.toml> line_2_plc_2`, the key in the variable `VEREDAS_KEY`"), "{notes}");
    assert!(notes.contains("opc `--plc`: the password lived in the machine's credential store; seal it as the entry `__plc` with `veredas-portal --secret <project.toml> __plc`, the key in the variable `VEREDAS_KEY`"), "{notes}");
    std::fs::write(v1.join("project.toml"), toml.as_bytes()).unwrap();
    let key = String::from_utf8(std::process::Command::new(exe).arg("--new-key").output().unwrap().stdout).unwrap();
    let mut child = std::process::Command::new(exe).arg("--secret").arg(v1.join("project.toml")).arg("line_2_plc").env("VEREDAS_KEY", key.trim()).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), b"pw\n").unwrap();
    let sealed = child.wait_with_output().unwrap();
    assert!(sealed.status.success() && String::from_utf8_lossy(&sealed.stdout).starts_with("sealed `line_2_plc` in "), "{}", String::from_utf8_lossy(&sealed.stderr));
    let mut child = std::process::Command::new(exe).arg("--secret").arg(v1.join("project.toml")).arg("__plc").env("VEREDAS_KEY", key.trim()).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), b"pw\n").unwrap();
    let sealed = child.wait_with_output().unwrap();
    assert!(sealed.status.success() && String::from_utf8_lossy(&sealed.stdout).starts_with("sealed `__plc` in "), "{}", String::from_utf8_lossy(&sealed.stderr));
    assert_eq!(SecretsFile::load(&v1.join("secrets.toml")).unwrap().names().collect::<Vec<_>>(), ["__plc", "line_2_plc"]);
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
    // The secrets (Q15): a fresh key from the executable, the password sealed from stdin into the
    // file beside the project under it, the key in the variable the project names when it runs.
    let new_key = || {
        let out = Command::new(exe).arg("--new-key").output().unwrap();
        assert!(out.status.success() && out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
        let key = String::from_utf8(out.stdout).unwrap().trim().to_string();
        assert!(key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()), "a key, 64 hexadecimal characters: {key}");
        key
    };
    let key = new_key();
    assert_ne!(key, new_key(), "fresh every time");
    // The run refused by name, exit 2, the value nowhere.
    let refused_run = |project: &Path, env: Option<&str>| {
        let mut cmd = Command::new(exe);
        cmd.arg(project).env_remove("VEREDAS_KEY").stdin(Stdio::null());
        if let Some(k) = env {
            cmd.env("VEREDAS_KEY", k);
        }
        let out = cmd.output().unwrap();
        let (said, errs) = (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string());
        assert_eq!(out.status.code(), Some(2), "{said}{errs}");
        assert!(!errs.contains("s3cret") && !said.contains("s3cret"), "{said}{errs}");
        errs
    };
    // Before anything is sealed there is no secrets file (an imported project's first run): it
    // holds no entry, so the source is refused by the entry it names, as with any entry not there.
    assert!(!dir.path().join("secrets.toml").exists());
    let errs = refused_run(&file, Some(key.as_str()));
    assert!(errs.contains("the session did not start: source `plc`: the secret `plc` for the password of `operator` is not in the secrets file"), "{errs}");
    let seal = |project: &Path, entry: &str, env: Option<&str>, stdin: &[u8]| {
        let mut cmd = Command::new(exe);
        cmd.arg("--secret").arg(project).arg(entry).env_remove("VEREDAS_KEY").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(k) = env {
            cmd.env("VEREDAS_KEY", k);
        }
        let mut child = cmd.spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        let out = child.wait_with_output().unwrap();
        (out.status.code(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
    };
    let (code, said, errs) = seal(&file, "plc", Some(key.as_str()), b"s3cret\n");
    assert_eq!((code, errs.as_str()), (Some(0), ""), "{said}{errs}");
    assert!(said.starts_with("sealed `plc` in ") && said.contains("secrets.toml") && !said.contains("s3cret"), "{said}");
    let on_disk = std::fs::read_to_string(dir.path().join("secrets.toml")).unwrap();
    assert!(on_disk.contains("format = \"veredas-secrets\"") && on_disk.contains("version = 1") && on_disk.contains("\nplc = \"") && !on_disk.contains("s3cret"), "{on_disk}");
    // What was sealed opens under the key to exactly what stdin carried: the final line break
    // dropped, LF or CRLF, a bare carriage return and a line break inside kept; an entry's name
    // trimmed in the file and the echo.
    let (code, _, _) = seal(&file, "crlf", Some(key.as_str()), b"s3cret\r\n");
    assert_eq!(code, Some(0));
    let (code, _, _) = seal(&file, "cr", Some(key.as_str()), b"abc\r");
    assert_eq!(code, Some(0));
    let (code, _, _) = seal(&file, "multi", Some(key.as_str()), b"a\nb\n");
    assert_eq!(code, Some(0));
    let (code, said, _) = seal(&file, " spaced ", Some(key.as_str()), b"v\n");
    assert!(code == Some(0) && said.starts_with("sealed `spaced` in "), "{said}");
    let opened = SecretsFile::load(&dir.path().join("secrets.toml")).unwrap().open(&Key256::from_env("K", &|_| Some(key.clone())).unwrap(), "K").unwrap();
    assert_eq!((opened.get("plc"), opened.get("crlf"), opened.get("cr"), opened.get("multi"), opened.get("spaced")), (Some("s3cret"), Some("s3cret"), Some("abc\r"), Some("a\nb"), Some("v")));
    assert!(!dir.path().join("secrets.toml.new").exists(), "the file is moved into place, nothing left beside it");
    // The tool's refusals, by name: no key, no `[secrets]` table, a file that is not a secrets
    // file (its path and the header text every file of the suite shares), no entry, a stray argument.
    let (code, _, errs) = seal(&file, "plc", None, b"s3cret\n");
    assert_eq!(code, Some(2));
    assert!(errs.contains("the variable `VEREDAS_KEY` is not set: it holds the project's key"), "{errs}");
    let bare = dir.path().join("bare.toml");
    std::fs::write(&bare, text.replacen("[secrets]\nkey_env = \"VEREDAS_KEY\"\n", "", 1).replacen("secret = \"plc\"\n", "", 1).replacen("user = \"operator\"\n", "", 1)).unwrap();
    let (code, _, errs) = seal(&bare, "plc", Some(key.as_str()), b"s3cret\n");
    assert_eq!(code, Some(2));
    assert!(errs.contains("the project has no `[secrets]` table"), "{errs}");
    let badfile = dir.path().join("badfile.toml");
    std::fs::write(&badfile, text.replacen("[secrets]\n", "[secrets]\nfile = \"bad.toml\"\n", 1)).unwrap();
    std::fs::write(dir.path().join("bad.toml"), "format = \"veredas-project\"\nversion = 1\n").unwrap();
    let (code, _, errs) = seal(&badfile, "plc", Some(key.as_str()), b"x\n");
    assert_eq!(code, Some(2));
    assert!(errs.contains("the secrets file ") && errs.contains("bad.toml: `format` is `veredas-project`, not `veredas-secrets`"), "{errs}");
    // The run refuses the same file by its path and the header text.
    let errs = refused_run(&badfile, Some(key.as_str()));
    assert!(errs.contains("the session did not start: the secrets file ") && errs.contains("bad.toml: `format` is `veredas-project`, not `veredas-secrets`"), "{errs}");
    let out = Command::new(exe).arg("--secret").arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let out = Command::new(exe).arg("--secret").arg(&file).arg("plc").arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "a stray file after `--secret` is a usage error");
    // Arguments a shell may hand it wrong: a project file before `--secret`, `--secret` twice, an
    // option where the entry or the project file goes, `--new-key` twice or with anything after
    // it: a usage error each, nothing sealed, nothing on stdout.
    let file_s = file.to_str().unwrap();
    for args in [
        vec![file_s, "--secret", file_s, "e1"],
        vec!["--secret", file_s, "e2", "--secret", file_s, "e3"],
        vec!["--secret", file_s, "--check"],
        vec!["--secret", "--check", "e4"],
        vec!["--new-key", "--new-key"],
        vec!["--new-key", file_s],
        vec!["--new-key", "--check"],
    ] {
        let out = Command::new(exe).args(&args).env("VEREDAS_KEY", &key).stdin(Stdio::null()).output().unwrap();
        assert_eq!((out.status.code(), out.stdout.is_empty()), (Some(1), true), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    let names: Vec<String> = SecretsFile::load(&dir.path().join("secrets.toml")).unwrap().names().map(str::to_string).collect();
    assert_eq!(names, ["cr", "crlf", "multi", "plc", "spaced"], "nothing sealed by a refused call");
    // The run refused by name: the variable unset, one holding no key, a wrong key, and a project
    // naming an entry the file does not hold.
    assert!(refused_run(&file, None).contains("the session did not start: the variable `VEREDAS_KEY` is not set: it holds the project's key"));
    assert!(refused_run(&file, Some("not-a-key")).contains("the session did not start: the variable `VEREDAS_KEY` does not hold a key (64 hexadecimal characters)"));
    let other = new_key();
    let errs = refused_run(&file, Some(other.as_str()));
    // The entry named is the first in the file's order that the key does not open (`cr` sorts first).
    assert!(errs.contains("the session did not start: the secrets file ") && errs.contains(" does not open with the key `VEREDAS_KEY` holds (entry `cr`)") && !errs.contains(&other) && !errs.contains(&key), "{errs}");
    let renamed = dir.path().join("renamed.toml");
    std::fs::write(&renamed, with(&text, "[[opc]]", "secret", "secret = \"plc_b\"")).unwrap();
    assert!(refused_run(&renamed, Some(key.as_str())).contains("the session did not start: source `plc`: the secret `plc_b` for the password of `operator` is not in the secrets file"));
    // `--secret` under a key that does not open what is already there: refused by the file's
    // path, the variable and the first entry, and the file is left as it was.
    let before = std::fs::read(dir.path().join("secrets.toml")).unwrap();
    let (code, _, errs) = seal(&file, "zz", Some(other.as_str()), b"x\n");
    assert_eq!(code, Some(2), "{errs}");
    assert!(errs.contains("the secrets file ") && errs.contains(" does not open with the key `VEREDAS_KEY` holds (entry `cr`)") && !errs.contains(&other), "{errs}");
    assert_eq!(std::fs::read(dir.path().join("secrets.toml")).unwrap(), before, "nothing sealed under a key that does not open the file");
    // The run itself, the key in the variable.
    let mut child = Command::new(exe).arg(&file).env("VEREDAS_KEY", &key).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
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

/// The sealed password reaches the source through the session (Q15): the example's controller is
/// the bench's server with a user on it, the password sealed as the entry by the tool, and the
/// executable's run has the source live and polling; sealed wrong, the source retries and never
/// goes live; neither value shows in any line the run prints.
#[test]
fn the_sealed_password_reaches_the_source_through_the_session() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    use veredas_maquinar::bench::{self, Bench};
    let Some(exe) = option_env!("CARGO_BIN_EXE_veredas-portal") else {
        return; // the light build has no executable
    };
    let (dir, text) = example();
    std::fs::create_dir_all(dir.path().join("logs")).unwrap();
    std::fs::write(dir.path().join("press.log"), "").unwrap();
    // The bench's server `plc` on a free port, the user `operator` on it, two tags.
    let bench_text = "format = \"veredas-bench\"\nversion = 1\nseed = 3\n\n[[opc]]\nname = \"plc\"\nport = 0\nuser = { name = \"operator\", password = \"s3cret\" }\ntags = [\n  { id = \"speed\", type = \"float\", value = \"1.5\", randomize = { low = 1.0, high = 2.0, decimals = 2, interval_ms = 100 } },\n  { id = \"run\", type = \"int\", value = \"0\" },\n]\n";
    let bench = Bench::start(bench::load(bench_text, dir.path()).unwrap()).unwrap();
    let plc = bench.server("plc").unwrap();
    // The example's controller is that server; its rule reads the two tags; nothing is served.
    let text = with(&text, "[[opc]]", "endpoint_url", &format!("endpoint_url = {:?}", plc.endpoint_url));
    let text = with(
        &text,
        "[[opc]]",
        "rules",
        &format!("rules = [ {{ name = \"plc_main\", tags = [ {{ node_id = {:?}, series = \"speed\", type = \"float\" }}, {{ node_id = {:?}, series = \"run\", type = \"int\" }} ] }} ]", plc.node_id_str("speed"), plc.node_id_str("run")),
    );
    let text = text.replacen("[server]\nbind = \"127.0.0.1:8901\"\n", "", 1);
    let file = dir.path().join("project.toml");
    std::fs::write(&file, &text).unwrap();
    let out = Command::new(exe).arg("--new-key").output().unwrap();
    let key = String::from_utf8(out.stdout).unwrap().trim().to_string();
    let seal = |value: &[u8]| {
        let mut child = Command::new(exe).arg("--secret").arg(&file).arg("plc").env("VEREDAS_KEY", &key).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(value).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    // The run, its lines read until `plc` is live or four status lines are in, then stdin closed.
    let run = |until_live: bool| -> Vec<String> {
        let mut child = Command::new(exe).arg(&file).env("VEREDAS_KEY", &key).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut lines: Vec<String> = Vec::new();
        let started = Instant::now();
        let mut line = String::new();
        loop {
            line.clear();
            if out.read_line(&mut line).unwrap() == 0 {
                break;
            }
            lines.push(line.clone());
            let status = line.contains("| ") && line.contains(" values");
            // Sealed wrong, the run is read over four status lines at least and on until the
            // source reads retrying: the refusal comes after a connection, which a loaded machine
            // can take seconds to make.
            if (status && until_live && line.contains("plc: live (")) || (status && !until_live && line.contains("plc: retrying") && lines.iter().filter(|l| l.contains(" values")).count() >= 4) || started.elapsed() > Duration::from_secs(30) {
                break;
            }
        }
        drop(child.stdin.take());
        let mut rest = String::new();
        out.read_to_string(&mut rest).unwrap();
        let status = child.wait().unwrap();
        let mut errs = String::new();
        child.stderr.take().unwrap().read_to_string(&mut errs).unwrap();
        assert!(status.success(), "the run ended {status:?}:\n{errs}");
        lines.push(rest);
        lines.push(errs);
        lines
    };
    seal(b"s3cret\n");
    let lines = run(true);
    assert!(lines.iter().any(|l| l.contains("plc: live (")), "the source never went live with the sealed password:\n{}", lines.concat());
    seal(b"nope\n");
    let lines = run(false);
    assert!(lines.iter().all(|l| !l.contains("plc: live")), "a wrong password went live:\n{}", lines.concat());
    assert!(lines.iter().any(|l| l.contains("plc: retrying")), "a wrong password retries:\n{}", lines.concat());
    assert!(lines.iter().all(|l| !l.contains("s3cret") && !l.contains("nope")), "a value in the run's output:\n{}", lines.concat());
    bench.stop();
}

/// The secrets file through its own API, the one the executable writes with (Q15): a value sealed
/// twice gives two blobs with two nonces; the key that sealed opens it; an entry copied under
/// another name does not open, the name being bound in; another key is refused by the first
/// entry's name, the value nowhere.
#[test]
fn the_secrets_file_seals_each_value_afresh_and_binds_the_entry_s_name() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.toml");
    let key = Key256::fresh().unwrap();
    let blob = |name: &str| -> String {
        let v: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["entries"][name].as_str().unwrap().to_string()
    };
    let seal = |name: &str, value: &str| {
        let mut f = SecretsFile::load_or_empty(&path).unwrap();
        f.set(&key, name, value).unwrap();
        f.save().unwrap();
    };
    seal("a", "same");
    let one = blob("a");
    seal("a", "same");
    let two = blob("a");
    assert_ne!(one, two, "a fresh nonce each time");
    let nonce = |b: &str| base64::engine::general_purpose::STANDARD.decode(b).unwrap()[..12].to_vec();
    assert_ne!(nonce(&one), nonce(&two), "the first twelve bytes are the nonce");
    assert_eq!(SecretsFile::load(&path).unwrap().open(&key, "K").unwrap().get("a"), Some("same"));
    std::fs::write(&path, format!("{}b = {:?}\n", std::fs::read_to_string(&path).unwrap(), two)).unwrap();
    let e = SecretsFile::load(&path).unwrap().open(&key, "K").unwrap_err().to_string();
    assert!(e.ends_with("does not open with the key `K` holds (entry `b`)"), "{e}");
    let e = SecretsFile::load(&path).unwrap().open(&Key256::fresh().unwrap(), "K").unwrap_err().to_string();
    assert!(e.ends_with("(entry `a`)") && !e.contains("same"), "{e}");
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
- `sql` (required): its SELECT over the base tables and the other views.

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
"##;
