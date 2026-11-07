//! The controller file (contract: the controller rules clause): read against a catalog,
//! versioned, validated, and documented by a worked example this test loads from
//! `docs/controller-schema.md` itself, every number of its prose and every key statement of its
//! reference read from the text and held against the reader. What the loaded rules do, tick by
//! tick and over a socket, is `tests/controller.rs`.

use std::collections::BTreeMap;

use veredas_core::telegram::catalog::{self, Catalog};
use veredas_maquinar::controller::*;

/// The schema's documentation, as text.
fn doc() -> String {
    let doc = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/controller-schema.md");
    std::fs::read_to_string(doc).unwrap().replace("\r\n", "\n")
}

/// Every ```toml block of the documentation, in order: the catalog, the machine, the unit.
fn blocks() -> Vec<String> {
    let text = doc();
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("```toml\n") {
        let body = &rest[start + "```toml\n".len()..];
        let end = body.find("```").unwrap();
        out.push(body[..end].to_string());
        rest = &body[end + 3..];
    }
    out
}

fn catalog() -> Catalog {
    catalog::load(&blocks()[0]).unwrap()
}

fn machine_text() -> String {
    blocks()[1].clone()
}

fn unit_text() -> String {
    blocks()[2].clone()
}

/// The documentation's prose, its lines joined, so a phrase wrapped across lines still reads.
fn doc_prose() -> String {
    doc().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The backticked text that follows `lead` in the prose.
fn quoted_after(prose: &str, lead: &str) -> String {
    let at = prose.find(lead).unwrap_or_else(|| panic!("the prose no longer says `{lead}`"));
    let rest = &prose[at + lead.len()..];
    let rest = &rest[rest.find('`').unwrap() + 1..];
    rest[..rest.find('`').unwrap()].to_string()
}

/// The number that follows `lead` in the prose.
fn number_after(prose: &str, lead: &str) -> u64 {
    let at = prose.find(lead).unwrap_or_else(|| panic!("the prose no longer says `{lead}`"));
    let rest = prose[at + lead.len()..].trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or_else(|_| panic!("no number after `{lead}`: {rest:.20}"))
}

/// A text as paragraphs, each on one line with its whitespace collapsed, the TOML blocks left
/// out (they are loaded, not quoted).
fn paragraphs(text: &str) -> Vec<String> {
    let mut text = text.replace("\r\n", "\n");
    while let Some(start) = text.find("```toml\n") {
        let end = start + text[start + 7..].find("```").unwrap() + 7 + 3;
        text.replace_range(start..end, "");
    }
    text.split("\n\n").map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect()
}

/// The reference's tables: for each, the keys it lists with what it says of each (the text in
/// the parentheses right after the key).
fn documented_keys() -> Vec<(String, Vec<(String, String)>)> {
    let text = doc();
    let keys = &text[text.find("## Keys").unwrap()..];
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in keys.lines() {
        if let Some(rest) = line.strip_prefix("**") {
            out.push((rest[..rest.find("**").unwrap()].to_string(), Vec::new()));
            continue;
        }
        if let Some(rest) = line.strip_prefix("- `") {
            let key = rest[..rest.find('`').unwrap()].to_string();
            let after = &rest[rest.find('`').unwrap() + 1..];
            let open = after.find('(').unwrap();
            let statement = after[open + 1..after.find(')').unwrap()].to_string();
            out.last_mut().unwrap().1.push((key, statement));
        }
    }
    out
}

fn refused(text: &str) -> String {
    match load(text, &catalog()) {
        Ok(_) => panic!("accepted: {text}"),
        Err(e) => e.to_string(),
    }
}

/// The keys the reader lists in its refusal of a misspelled one (`expected one of `a`, `b`,
/// `c`` for three or more, `expected `a` or `b`` for two).
fn reader_keys(err: &str) -> Vec<String> {
    let at = err.find("expected ").unwrap_or_else(|| panic!("{err}"));
    let rest = err[at..].lines().next().unwrap();
    let mut keys: Vec<String> = rest.split('`').skip(1).step_by(2).map(String::from).collect();
    keys.sort();
    keys
}

fn header() -> &'static str {
    "format = \"veredas-controller\"\nversion = 1\n"
}

fn text_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// A file with every key of every table present, one key a line in each table, so that any
/// one can be left out.
fn full_file() -> String {
    format!(
        "{}seed = 1\nregisters = {{ r = 0 }}\n\n\
         [destinations]\nlist = [1]\nbits = {{ b = 1 }}\nresting = [\"b\"]\ninitial = {{ \"1\" = [\"b\"] }}\nblocking = [ {{ bit = \"b\", set = {{ result = \"X\" }}, requeue = false }} ]\n\n\
         [items]\nmessage = \"announce\"\ninterval_ms = 100\nkey = \"item\"\ngate = \"r\"\nlap_ms = 30000\n\
         fields = [ {{ name = \"item\", value = 1 }}, {{ name = \"lane\", value = 1 }}, {{ name = \"read\", value = \"Y\" }}, {{ name = \"code\", value = \"C\" }}, {{ name = \"weight\", value = 1 }} ]\n\
         faults = [ {{ name = \"f\", percent = 0, set = {{ read = \"N\" }} }} ]\n\n\
         [items.reply]\nmessage = \"answer\"\nkey = \"item\"\ndestination = \"dest\"\nwhen = [ {{ field = \"flag\", equals = 1, action = \"deliver\" }} ]\n\n\
         [items.outcome]\nmessage = \"outcome\"\nafter_ms = 0\n\
         fields = [ {{ name = \"item\", from = \"item\" }}, {{ name = \"dest\", from = \"dest\" }}, {{ name = \"result\", value = \"S\" }} ]\n\
         mix = [ {{ name = \"o\", weight = 1, set = {{ result = \"O\" }}, requeue = false }} ]\n\n\
         [[answers]]\non = \"announce\"\nsend = \"answer\"\nafter_ms = 0\n\
         fields = [ {{ name = \"item\", from = \"item\" }}, {{ name = \"dest\", decide = {{ rules = [ {{ field = \"code\", pattern = \"x\", value = 1 }} ], spread = [ {{ value = 1, weight = 1 }} ], reject = 9, sticky = \"code\" }} }}, {{ name = \"flag\", value = 1 }} ]\n\n\
         [[commands]]\non = \"command\"\nfield = \"code\"\nwhen = [ {{ equals = 1, set = {{ r = 1 }} }} ]\n\n\
         [[cyclic]]\nmessage = \"status\"\nevery_ms = 1000\nper_destination = false\n\
         fields = [ {{ name = \"running\", register = \"r\" }}, {{ name = \"speed\", register = \"r\" }} ]\n",
        header()
    )
}

/// The sections of a file: the lines before the first header under `""`, then each header with
/// its lines.
fn sections(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = vec![(String::new(), Vec::new())];
    for line in text.lines() {
        if line.starts_with('[') {
            out.push((line.to_string(), Vec::new()));
        } else if !line.trim().is_empty() {
            out.last_mut().unwrap().1.push(line.to_string());
        }
    }
    out
}

fn join(sections: &[(String, Vec<String>)]) -> String {
    let mut out = String::new();
    for (header, lines) in sections {
        if !header.is_empty() {
            out.push_str(header);
            out.push('\n');
        }
        for l in lines {
            out.push_str(l);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// The file without the line `key = ...` of the section under `header` (`""` for the top).
fn without_line(text: &str, header: &str, key: &str) -> String {
    let mut s = sections(text);
    let section = s.iter_mut().find(|(h, _)| h == header).unwrap_or_else(|| panic!("no section {header}"));
    let before = section.1.len();
    section.1.retain(|l| !l.starts_with(&format!("{key} =")));
    assert_eq!(section.1.len() + 1, before, "no line `{key} =` in {header}");
    join(&s)
}

/// The file without whole sections.
fn without_sections(text: &str, headers: &[&str]) -> String {
    let s: Vec<(String, Vec<String>)> = sections(text).into_iter().filter(|(h, _)| !headers.contains(&h.as_str())).collect();
    join(&s)
}

#[test]
fn the_documented_example_loads_as_described() {
    let catalog = catalog();
    let prose = doc_prose();
    let names: Vec<(&str, u64)> = catalog.messages.iter().map(|m| (m.name.as_str(), m.id)).collect();
    assert_eq!(names, [("announce", 21), ("answer", 22), ("outcome", 23), ("command", 24), ("heartbeat", 25), ("status", 26)]);
    assert_eq!(
        (number_after(&prose, "the machine obeys (id"), number_after(&prose, "sends per destination (id"), number_after(&prose, "of its registers (id")),
        (24, 25, 26)
    );

    // The machine, as the prose describes it.
    let machine = load(&machine_text(), &catalog).unwrap();
    assert_eq!(machine.seed, Some(7));
    assert_eq!(machine.registers, BTreeMap::from([("running".to_string(), 1), ("speed".to_string(), 50)]));
    let items = machine.items.as_ref().unwrap();
    assert_eq!(items.message, "announce");
    assert_eq!(items.interval_ms, number_after(&prose, "announces an item every"));
    assert_eq!(items.interval_ms, 100);
    assert_eq!((items.key.as_str(), items.gate.as_deref()), ("item", Some("running")));
    assert_eq!(items.fields.names(), ["item", "lane", "read", "code", "weight"]);
    let one_in = number_after(&prose, "one item in");
    assert_eq!(items.faults, vec![("noread".to_string(), (100 / one_in) as u8, text_map(&[("read", "N"), ("code", "")]))]);
    assert_eq!(items.lap_ms, number_after(&prose, "round again after a lap of"));
    assert_eq!(items.lap_ms, 2000);
    let reply = items.reply.as_ref().unwrap();
    assert_eq!((reply.message.as_str(), reply.key.as_str(), reply.destination.as_deref()), ("answer", "item", Some("dest")));
    let requeue_flag = number_after(&prose, "destination: a `flag` of");
    let park_flag = number_after(&prose, "round again after a lap of 2000 ms, a `flag` of");
    assert_eq!(
        reply.when,
        vec![("flag".to_string(), requeue_flag.to_string(), Action::Requeue), ("flag".to_string(), park_flag.to_string(), Action::Park)]
    );
    let outcome = items.outcome.as_ref().unwrap();
    assert_eq!(outcome.message, "outcome");
    assert_eq!(outcome.after_ms, number_after(&prose, "anything else delivers it, and"));
    assert_eq!(outcome.after_ms, 500);
    assert_eq!(outcome.fields.names(), ["item", "dest", "result"]);
    let sorted = number_after(&prose, "drawn from a mix of");
    let lost = number_after(&prose, "`sorted` to");
    assert_eq!(
        outcome.mix,
        vec![("sorted".to_string(), sorted as f64, BTreeMap::new(), false), ("lost".to_string(), lost as f64, text_map(&[("result", "LOST")]), false)]
    );
    let destinations = machine.destinations.as_ref().unwrap();
    assert_eq!(destinations.list, [1, 2, 3, 4]);
    assert_eq!(destinations.bits, BTreeMap::from([("full".to_string(), 1), ("fault".to_string(), 2)]));
    assert_eq!(destinations.resting, 0, "rest with no bit set");
    assert_eq!(destinations.initial, BTreeMap::from([(number_after(&prose, "set; destination"), 1)]), "destination 4 starts full");
    assert_eq!(
        destinations.blocking,
        vec![
            ("full".to_string(), 1, text_map(&[("result", "FULL")]), true),
            ("fault".to_string(), 2, text_map(&[("result", "FAULT")]), false)
        ]
    );
    assert_eq!(destinations.blocking_of(3).map(|b| b.0.as_str()), Some("full"), "the first entry whose bit is set decides");
    assert_eq!(destinations.blocking_of(2).map(|b| b.0.as_str()), Some("fault"));
    assert_eq!(destinations.blocking_of(0), None);
    assert_eq!(destinations.names(3), ["fault", "full"]);
    assert_eq!(machine.answers.len(), 0);
    assert_eq!(machine.commands.len(), 1);
    let command = &machine.commands[0];
    assert_eq!((command.on.as_str(), command.field.as_str()), ("command", "code"));
    assert_eq!(
        command.when,
        vec![
            ("1".to_string(), BTreeMap::from([("running".to_string(), 1)])),
            ("0".to_string(), BTreeMap::from([("running".to_string(), 0)])),
            ("7".to_string(), BTreeMap::from([("speed".to_string(), 70)])),
        ]
    );
    assert_eq!(machine.cyclic.len(), 2);
    let (heartbeat, status) = (&machine.cyclic[0], &machine.cyclic[1]);
    assert_eq!((heartbeat.message.as_str(), heartbeat.every_ms, heartbeat.per_destination), ("heartbeat", number_after(&prose, "A `heartbeat` goes out every"), true));
    assert_eq!(heartbeat.fields.names(), ["dest", "state"]);
    assert_eq!((status.message.as_str(), status.every_ms, status.per_destination), ("status", number_after(&prose, "and a `status` line every"), false));
    assert_eq!(status.fields.names(), ["running", "speed"]);
    assert_eq!(machine.sends(), ["announce", "outcome", "heartbeat", "status"]);
    assert_eq!(machine.receives(), ["answer", "command"]);

    // The unit.
    let unit = load(&unit_text(), &catalog).unwrap();
    assert_eq!(unit.seed, Some(11));
    assert!(unit.registers.is_empty() && unit.items.is_none() && unit.commands.is_empty() && unit.cyclic.is_empty());
    let destinations = unit.destinations.as_ref().unwrap();
    assert_eq!((destinations.list.as_slice(), destinations.bits.get("full")), (&[1, 2, 3][..], Some(&1)));
    assert_eq!(unit.answers.len(), 1);
    let answer = &unit.answers[0];
    assert_eq!((answer.on.as_str(), answer.send.as_str(), answer.after_ms), ("announce", "answer", 0));
    assert_eq!(answer.fields.names(), ["item", "dest", "flag"]);
    assert_eq!(unit.sends(), ["answer"]);
    assert_eq!(unit.receives(), ["announce"]);
    assert_eq!(quoted_after(&prose, "an item whose"), "code");
    assert!(prose.contains("an item whose `code` is empty goes to destination 0, any other to destination 1, 2 or 3 in the shares 2, 1 and 1, the same destination again for the same `code`, and to 9, the reject, when every one of those is full."));
    assert_eq!(
        answer.fields.decide("dest"),
        Some(DecideView {
            rules: vec![("code".to_string(), "(empty)".to_string(), "0".to_string())],
            spread: vec![("1".to_string(), 2.0), ("2".to_string(), 1.0), ("3".to_string(), 1.0)],
            reject: Some("9".to_string()),
            sticky: Some("code".to_string()),
        })
    );
    assert_eq!(answer.fields.decide("item"), None, "a field with another source decides nothing");
    assert_eq!(answer.fields.decide("nope"), None);
    assert_eq!(destinations.blocking.iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), ["full"]);
    // The two files are the test's: the prose says which tests read them.
    assert!(prose.contains("`crates/veredas-maquinar/tests/controller_rules.rs` loads these three blocks"));
    assert!(prose.contains("`crates/veredas-maquinar/tests/controller.rs` runs the two files as the engine"));
}

#[test]
fn every_key_the_reference_lists_is_the_reader_s_and_states_what_the_reader_does() {
    let documented = documented_keys();
    let tables: Vec<&str> = documented.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        tables,
        [
            "The top level",
            "The destinations",
            "A blocking bit",
            "The items",
            "A fault",
            "The reply",
            "A disposition",
            "The outcome",
            "A mixed outcome",
            "An answer",
            "A command",
            "A setting",
            "A cyclic message",
            "A field",
            "A counter",
            "A decision",
            "A rule",
            "A share"
        ],
        "the reference's eighteen tables"
    );
    let full = full_file();
    load(&full, &catalog()).unwrap_or_else(|e| panic!("the full file loads: {e}"));
    let registers = BTreeMap::from([("r".to_string(), 0)]);

    // Where each table lives: a section of the file, or an inline table read as its own struct.
    let section_of = |table: &str| -> Option<&'static str> {
        Some(match table {
            "The top level" => "",
            "The destinations" => "[destinations]",
            "The items" => "[items]",
            "The reply" => "[items.reply]",
            "The outcome" => "[items.outcome]",
            "An answer" => "[[answers]]",
            "A command" => "[[commands]]",
            "A cyclic message" => "[[cyclic]]",
            _ => return None,
        })
    };
    let inline_full = |table: &str| -> &'static str {
        match table {
            "A blocking bit" => "bit = \"b\"\nset = { result = \"X\" }\nrequeue = false\n",
            "A fault" => "name = \"f\"\npercent = 0\nset = { read = \"N\" }\n",
            "A disposition" => "field = \"flag\"\nequals = 1\naction = \"deliver\"\n",
            "A mixed outcome" => "name = \"o\"\nweight = 1\nset = { result = \"O\" }\nrequeue = false\n",
            "A setting" => "equals = 1\nset = { r = 1 }\n",
            "A field" => "name = \"f\"\nvalue = 1\ndecimals = 0\n",
            "A counter" => "first = 1\nlast = 9\n",
            "A decision" => "rules = [ { field = \"code\", pattern = \"x\", value = 1 } ]\nspread = [ { value = 1, weight = 1 } ]\nreject = 0\nsticky = \"code\"\n",
            "A rule" => "field = \"code\"\npattern = \"x\"\nvalue = 1\n",
            "A share" => "value = 1\nweight = 1\n",
            other => panic!("{other}: a table this test does not know where to find"),
        }
    };
    let drop_line = |text: &str, key: &str| -> String {
        let out: Vec<&str> = text.lines().filter(|l| !l.starts_with(&format!("{key} ="))).collect();
        assert_eq!(out.len() + 1, text.lines().count(), "no line `{key} =` in {text}");
        out.join("\n") + "\n"
    };
    // The reader's own key list of each table, from its refusal of a misspelled key there, and
    // a misspelled key refused through `load` in every table.
    let misspelled = |table: &str| -> String {
        let zz = "zz = 1\n";
        match table {
            "The top level" => format!("{}{zz}", header()),
            "The destinations" => full.replace("[destinations]\n", &format!("[destinations]\n{zz}")),
            "The items" => full.replace("[items]\n", &format!("[items]\n{zz}")),
            "The reply" => full.replace("[items.reply]\n", &format!("[items.reply]\n{zz}")),
            "The outcome" => full.replace("[items.outcome]\n", &format!("[items.outcome]\n{zz}")),
            "An answer" => full.replace("[[answers]]\n", &format!("[[answers]]\n{zz}")),
            "A command" => full.replace("[[commands]]\n", &format!("[[commands]]\n{zz}")),
            "A cyclic message" => full.replace("[[cyclic]]\n", &format!("[[cyclic]]\n{zz}")),
            "A blocking bit" => full.replace("{ bit = \"b\",", "{ bit = \"b\", zz = 1,"),
            "A fault" => full.replace("{ name = \"f\",", "{ name = \"f\", zz = 1,"),
            "A disposition" => full.replace("{ field = \"flag\",", "{ field = \"flag\", zz = 1,"),
            "A mixed outcome" => full.replace("{ name = \"o\",", "{ name = \"o\", zz = 1,"),
            "A setting" => full.replace("{ equals = 1, set", "{ equals = 1, zz = 1, set"),
            "A field" => full.replace("{ name = \"lane\",", "{ name = \"lane\", zz = 1,"),
            "A counter" => full.replace("{ name = \"lane\", value = 1 }", "{ name = \"lane\", counter = { first = 1, zz = 1 } }"),
            "A decision" => full.replace("reject = 9, sticky", "reject = 0, zz = 1, sticky"),
            "A rule" => full.replace("{ field = \"code\", pattern", "{ field = \"code\", zz = 1, pattern"),
            "A share" => full.replace("{ value = 1, weight = 1 }", "{ value = 1, zz = 1, weight = 1 }"),
            other => panic!("{other}"),
        }
    };
    for (table, keys) in &documented {
        let text = misspelled(table);
        assert_ne!(text, full, "{table}: no misspelling placed");
        let err = refused(&text);
        assert!(err.contains("`zz`"), "{table}: {err}");
        let mut names: Vec<String> = keys.iter().map(|(k, _)| k.clone()).collect();
        names.sort();
        assert_eq!(names, reader_keys(&err), "{table}: the documented keys are the reader's");
    }

    // Each key's statement against the reader: required, one source, or loading with the
    // stated default when left out.
    for (table, keys) in &documented {
        for (key, statement) in keys {
            let required = statement.starts_with("required");
            let default = statement.split(';').next().unwrap().trim().trim_matches('`').to_string();
            if statement == "one source" {
                // One field with that source alone loads; with none, or with another too, refused.
                let value = match key.as_str() {
                    "value" => "value = 1",
                    "range" => "range = [1, 2]",
                    "counter" => "counter = { first = 1 }",
                    "cycle" => "cycle = [1]",
                    "template" => "template = \"x\"",
                    "from" => "from = \"item\"",
                    "register" => "register = \"r\"",
                    "now" => "now = \"%H\"",
                    "decide" => "decide = { reject = 0 }",
                    other => panic!("{other}: a source this test does not know"),
                };
                let spec = |body: &str| toml::from_str::<FieldSpec>(&format!("name = \"f\"\n{body}\n")).unwrap();
                Fields::compile("f", &[spec(value)], Place::Answer, &registers, None).unwrap_or_else(|e| panic!("{key} alone: {e}"));
                let none = Fields::compile("f", &[spec("")], Place::Answer, &registers, None).unwrap_err().to_string();
                assert_eq!(none, "f.fields[0] (`f`): exactly one of value, range, counter, cycle, template, from, register, now or decide (0 given)");
                let other = if key == "value" { "range = [1, 2]" } else { "value = 1" };
                let two = Fields::compile("f", &[spec(&format!("{value}\n{other}"))], Place::Answer, &registers, None).unwrap_err().to_string();
                assert!(two.ends_with("(2 given)"), "{key} with another source: {two}");
                continue;
            }
            if let Some(section) = section_of(table) {
                let text = match (table.as_str(), key.as_str()) {
                    // Nothing may read a register when there is no table of them.
                    ("The top level", "registers") => without_sections(&without_line(&without_line(&full, "", "registers"), "[items]", "gate"), &["[[commands]]", "[[cyclic]]"]),
                    // The top level's tables are sections of the file.
                    ("The top level", "destinations") => without_sections(&full, &["[destinations]"]),
                    ("The top level", "items") => without_sections(&full, &["[items]", "[items.reply]", "[items.outcome]"]),
                    ("The top level", "answers") => without_sections(&full, &["[[answers]]"]),
                    ("The top level", "commands") => without_sections(&full, &["[[commands]]"]),
                    ("The top level", "cyclic") => without_sections(&full, &["[[cyclic]]"]),
                    // The reply and the outcome are sections too; an outcome needs a reply.
                    ("The items", "reply") => without_sections(&full, &["[items.reply]", "[items.outcome]"]),
                    ("The items", "outcome") => without_sections(&full, &["[items.outcome]"]),
                    // Nothing may name a bit when there are none.
                    ("The destinations", "bits") => {
                        let mut text = full.clone();
                        for key in ["bits", "resting", "initial", "blocking"] {
                            text = without_line(&text, "[destinations]", key);
                        }
                        text
                    }
                    // The reply's destination is required while there is an outcome by destination.
                    ("The reply", "destination") => {
                        let only = without_line(&full, "[items.reply]", "destination");
                        assert_eq!(refused(&only), "items.reply.destination: an outcome by destination needs the reply's destination field");
                        without_sections(&only, &["[items.outcome]"])
                    }
                    _ => without_line(&full, section, key),
                };
                if required {
                    let err = refused(&text);
                    let expected = match key.as_str() {
                        "format" => "no `format`: a controller file states `format` and `version`".to_string(),
                        "version" => "no `version`: a controller file states `format` and `version`".to_string(),
                        _ => format!("missing field `{key}`"),
                    };
                    assert!(err.contains(&expected), "{table}.{key} is documented required ({statement}): {err}");
                    continue;
                }
                let rules = load(&text, &catalog()).unwrap_or_else(|e| panic!("{table}.{key} documented as `{statement}` but refused when left out: {e}"));
                let items = rules.items.as_ref();
                match (table.as_str(), key.as_str()) {
                    ("The top level", "seed") => assert!(default == "none" && rules.seed.is_none()),
                    ("The top level", "registers") => assert!(default == "none" && rules.registers.is_empty()),
                    ("The top level", "destinations") => assert!(default == "none" && rules.destinations.is_none()),
                    ("The top level", "items") => assert!(default == "none" && rules.items.is_none()),
                    ("The top level", "answers") => assert!(default == "none" && rules.answers.is_empty()),
                    ("The top level", "commands") => assert!(default == "none" && rules.commands.is_empty()),
                    ("The top level", "cyclic") => assert!(default == "none" && rules.cyclic.is_empty()),
                    ("The destinations", "bits") => assert!(default == "none" && rules.destinations.as_ref().unwrap().bits.is_empty()),
                    ("The destinations", "resting") => assert!(default == "none" && rules.destinations.as_ref().unwrap().resting == 0),
                    ("The destinations", "initial") => assert!(default == "none" && rules.destinations.as_ref().unwrap().initial.is_empty()),
                    ("The destinations", "blocking") => assert!(default == "none" && rules.destinations.as_ref().unwrap().blocking.is_empty()),
                    ("The items", "gate") => assert!(default == "none" && items.unwrap().gate.is_none()),
                    ("The items", "lap_ms") => assert_eq!((default.as_str(), items.unwrap().lap_ms), ("30000", 30000)),
                    ("The items", "faults") => assert!(default == "none" && items.unwrap().faults.is_empty()),
                    ("The items", "reply") => assert!(default == "none" && items.unwrap().reply.is_none()),
                    ("The items", "outcome") => assert!(default == "none" && items.unwrap().outcome.is_none()),
                    ("The reply", "key") => {
                        assert_eq!(statement, "the items' `key`");
                        assert_eq!(items.unwrap().reply.as_ref().unwrap().key, items.unwrap().key);
                    }
                    ("The reply", "destination") => assert!(default == "none" && items.unwrap().reply.as_ref().unwrap().destination.is_none()),
                    ("The reply", "when") => assert!(default == "none" && items.unwrap().reply.as_ref().unwrap().when.is_empty()),
                    ("The outcome", "after_ms") => assert_eq!((default.as_str(), items.unwrap().outcome.as_ref().unwrap().after_ms), ("0", 0)),
                    ("The outcome", "mix") => assert!(default == "none" && items.unwrap().outcome.as_ref().unwrap().mix.is_empty()),
                    ("An answer", "after_ms") => assert_eq!((default.as_str(), rules.answers[0].after_ms), ("0", 0)),
                    ("A cyclic message", "per_destination") => assert_eq!((default.as_str(), rules.cyclic[0].per_destination), ("false", false)),
                    other => panic!("{other:?}: a documented default this test does not know how to check"),
                }
                continue;
            }
            // An inline table, read as the struct the reader reads it with.
            let text = drop_line(inline_full(table), key);
            macro_rules! check {
                ($ty:ty, $ok:expr) => {{
                    let parsed = toml::from_str::<$ty>(&text);
                    if required {
                        let err = parsed.err().map(|e| e.to_string()).unwrap_or_else(|| panic!("{table}.{key} is documented required but loads without it"));
                        assert!(err.contains(&format!("missing field `{key}`")), "{table}.{key}: {err}");
                    } else {
                        let v = parsed.unwrap_or_else(|e| panic!("{table}.{key} documented as `{statement}` but refused when left out: {e}"));
                        let f: &dyn Fn(&$ty) -> bool = &$ok;
                        assert!(f(&v), "{table}.{key}: the default is not `{default}`: {v:?}");
                    }
                }};
            }
            match (table.as_str(), key.as_str()) {
                ("A blocking bit", "set") => check!(Blocking, |b| default == "none" && b.set.is_empty()),
                ("A blocking bit", "requeue") => check!(Blocking, |b| default == "false" && !b.requeue),
                ("A blocking bit", _) => check!(Blocking, |_| false),
                ("A fault", "percent") => check!(Fault, |f| default == "0" && f.percent == 0),
                ("A fault", _) => check!(Fault, |_| false),
                ("A disposition", _) => check!(Disposition, |_| false),
                ("A mixed outcome", "set") => check!(MixEntry, |m| default == "none" && m.set.is_empty()),
                ("A mixed outcome", "requeue") => check!(MixEntry, |m| default == "false" && !m.requeue),
                ("A mixed outcome", _) => check!(MixEntry, |_| false),
                ("A setting", _) => check!(Setting, |_| false),
                ("A field", "decimals") => check!(FieldSpec, |f| default == "0" && f.decimals == 0),
                ("A field", _) => check!(FieldSpec, |_| false),
                ("A counter", "first") => check!(Counter, |c| default == "1" && c.first == 1),
                ("A counter", "last") => check!(Counter, |c| default == "none" && c.last.is_none()),
                ("A decision", "rules") => check!(Decide, |d| default == "none" && d.rules.is_empty()),
                ("A decision", "spread") => check!(Decide, |d| default == "none" && d.spread.is_empty()),
                ("A decision", "reject") => check!(Decide, |d| default == "none" && d.reject.is_none()),
                ("A decision", "sticky") => check!(Decide, |d| default == "none" && d.sticky.is_none()),
                ("A rule", _) => check!(Rule, |_| false),
                ("A share", _) => check!(Share, |_| false),
                other => panic!("{other:?}: a key this test does not know how to check"),
            }
        }
    }
    // The reference's own rules about keys and names, and the constants it states.
    let prose = doc_prose();
    assert!(prose.contains("Each key states in parentheses that it is required, or the value it takes when left out."));
    assert!(prose.contains("`format` (required): always `\"veredas-controller\"`."));
    assert_eq!(FORMAT, "veredas-controller");
    assert!(prose.contains("`version` (required): this reader knows `1`."));
    assert_eq!(VERSION, 1);
    // The actions the reference names are the reader's.
    assert!(prose.contains("`action` (required): `deliver` (the item travels and ends in an outcome, or ends delivered without one), `requeue` (round again after a lap) or `park` (asked about no more)."));
    let err = toml::from_str::<Disposition>("field = \"f\"\nequals = 1\naction = \"x\"\n").unwrap_err().to_string();
    let mut actions: Vec<String> = err[err.find("expected").unwrap()..].split('`').skip(1).step_by(2).map(String::from).collect();
    actions.sort();
    assert_eq!(actions, ["deliver", "park", "requeue"]);
    assert!(prose.contains("`(empty)` matches a field that is absent or blank"));
    assert_eq!(EMPTY_PATTERN, "(empty)");
    assert!(prose.contains("the newest 10000 keys are kept"));
    assert_eq!(STICKY, 10000);
}

#[test]
fn the_header_is_refused_four_ways() {
    assert!(doc_prose().contains("A file of another format or an unknown version is refused, never guessed."));
    assert_eq!(refused("seed = 1\n"), "no `format`: a controller file states `format` and `version`");
    assert_eq!(refused("format = \"veredas-controller\"\n"), "no `version`: a controller file states `format` and `version`");
    assert_eq!(refused("format = \"veredas-catalog\"\nversion = 1\n"), "`format` is `veredas-catalog`, not `veredas-controller`");
    assert_eq!(refused("format = \"veredas-controller\"\nversion = 2\n"), "version 2 of `veredas-controller` is not known here (this reader knows 1)");
    assert!(refused("format = \"veredas-controller\"\nversion = 2\nnew_key = 1\n").starts_with("version 2"), "the header is judged before the body");
    assert!(refused("not toml [").starts_with("not a controller file: "));
    assert!(matches!(load("format = \"x\"\nversion = 1\n", &catalog()), Err(RulesError::WrongFormat(f)) if f == "x"));
    assert!(matches!(load("format = \"veredas-controller\"\nversion = 0\n", &catalog()), Err(RulesError::UnknownVersion(0))));
    assert!(refused("format = \"veredas-controller \"\nversion = 1\n").starts_with("`format` is `veredas-controller `"));
    assert!(refused("format = \"veredas-controller\"\nversion = 1.0\n").starts_with("not a controller file"));
    // The emptiest file: a controller that does nothing.
    let nothing = load(header(), &catalog()).unwrap();
    assert!(nothing.sends().is_empty() && nothing.receives().is_empty());
}

#[test]
fn every_invalid_file_is_refused_with_its_place_named() {
    let h = header();
    let items = |body: &str| format!("{h}[items]\nmessage = \"announce\"\ninterval_ms = 100\nkey = \"item\"\n{body}");
    let five = "fields = [ { name = \"item\", value = 1 }, { name = \"lane\", value = 1 }, { name = \"read\", value = \"Y\" }, { name = \"code\", value = \"C\" }, { name = \"weight\", value = 1 } ]\n";
    let with = |line: &str| items(&format!("{five}{line}"));
    // Fields: sources, names, ranges, counters, cycles, templates, places, registers, decisions.
    let field = |spec: &str| items(&format!("fields = [ {{ name = \"item\", {spec} }}, {{ name = \"lane\", value = 1 }}, {{ name = \"read\", value = \"Y\" }}, {{ name = \"code\", value = \"C\" }}, {{ name = \"weight\", value = 1 }} ]\n"));
    assert_eq!(refused(&items("fields = [ { name = \"item\" } ]\n")), "items.fields[0] (`item`): exactly one of value, range, counter, cycle, template, from, register, now or decide (0 given)");
    assert_eq!(refused(&field("value = 1, range = [1, 2]")), "items.fields[0] (`item`): exactly one of value, range, counter, cycle, template, from, register, now or decide (2 given)");
    assert_eq!(refused(&items("fields = [ { name = \" \", value = 1 } ]\n")), "items.fields[0]: the field has no name");
    assert_eq!(refused(&items("fields = [ { name = \"item\", value = 1 }, { name = \"item\", value = 2 } ]\n")), "items.fields[1]: two fields are named `item`");
    assert_eq!(refused(&field("range = [5, 1]")), "items.fields[0] (`item`): the range is empty (5 > 1)");
    assert_eq!(refused(&field("range = [0, inf]")), "items.fields[0] (`item`): the range is not finite");
    assert_eq!(refused(&field("range = [0, 1], decimals = 16")), "items.fields[0] (`item`): decimals is 16, over 15");
    assert_eq!(refused(&field("counter = { first = 5, last = 1 }")), "items.fields[0] (`item`): the counter ends at 1, before its first 5");
    assert_eq!(refused(&field("cycle = []")), "items.fields[0] (`item`): the cycle is empty");
    assert_eq!(refused(&field("template = \"{\"")), "items.fields[0] (`item`): the template has an unclosed `{` at character 0");
    assert_eq!(refused(&field("register = \"x\"")), "items.fields[0] (`item`): no register `x`");
    assert_eq!(refused(&field("now = \"%\"")), "items.fields[0] (`item`): `%` is not a datetime format");
    assert_eq!(refused(&field("decide = { reject = 0 }")), "items.fields[0] (`item`): `decide` answers a message");
    // Every place, every source it does not take; and the bounds at their included ends.
    let outcome_with = |spec: &str| with(&format!("[items.reply]\nmessage = \"answer\"\n[items.outcome]\nmessage = \"outcome\"\nfields = [ {{ name = \"item\", {spec} }}, {{ name = \"dest\", value = 1 }}, {{ name = \"result\", value = \"S\" }} ]\n"));
    assert_eq!(refused(&outcome_with("decide = { reject = 0 }")), "items.outcome.fields[0] (`item`): `decide` answers a message");
    let cyclic_with = |spec: &str, per: bool| format!("{h}[destinations]\nlist = [1]\n\n[[cyclic]]\nmessage = \"heartbeat\"\nevery_ms = 1000\nper_destination = {per}\nfields = [ {{ name = \"dest\", {spec} }}, {{ name = \"state\", value = 0 }} ]\n");
    assert_eq!(refused(&cyclic_with("decide = { reject = 0 }", true)), "cyclic[0].fields[0] (`dest`): `decide` answers a message");
    assert_eq!(refused(&cyclic_with("from = \"destination\"", false)), "cyclic[0].fields[0] (`dest`): a cyclic message reads `destination` or `state`, per destination");
    assert_eq!(refused(&cyclic_with("from = \"state\"", false)), "cyclic[0].fields[0] (`dest`): a cyclic message reads `destination` or `state`, per destination");
    assert!(load(&cyclic_with("from = \"state\"", true), &catalog()).is_ok());
    assert!(load(&field("range = [1, 1]"), &catalog()).is_ok(), "a range of one value");
    assert!(load(&field("range = [1, 2], decimals = 15"), &catalog()).is_ok(), "fifteen decimals");
    assert!(load(&field("counter = { first = 3, last = 3 }"), &catalog()).is_ok(), "a counter of one value");
    assert!(load(&with("faults = [ { name = \"f\", percent = 100, set = { read = \"N\" } } ]\n"), &catalog()).is_ok(), "every item faulted");
    let weightless = format!("{h}[[answers]]\non = \"announce\"\nsend = \"answer\"\nfields = [ {{ name = \"item\", decide = {{ spread = [ {{ value = 1, weight = 0 }} ] }} }}, {{ name = \"dest\", value = 1 }}, {{ name = \"flag\", value = 1 }} ]\n");
    assert!(load(&weightless, &catalog()).is_ok(), "a weight of 0");
    let mix0 = with("[items.reply]\nmessage = \"answer\"\n[items.outcome]\nmessage = \"outcome\"\nfields = [ { name = \"item\", from = \"item\" }, { name = \"dest\", from = \"dest\" }, { name = \"result\", value = \"S\" } ]\nmix = [ { name = \"o\", weight = 0 } ]\n");
    assert!(load(&mix0, &catalog()).is_ok(), "a mix weight of 0");
    // A field's decimals reach its draw.
    let regs: BTreeMap<String, i64> = BTreeMap::new();
    let mut two = Fields::compile("f", &[toml::from_str::<FieldSpec>("name = \"f\"\nrange = [1, 2]\ndecimals = 2\n").unwrap()], Place::Answer, &regs, None).unwrap();
    let mut rng = veredas_core::rng::Pcg32::new(1, 1);
    let empty = std::collections::HashMap::new();
    let mut ctx = Context { fields: &empty, registers: &regs, blocked: &[], rng: &mut rng };
    let drawn = two.render(&mut ctx).unwrap()["f"].clone();
    assert!(drawn.len() == 4 && drawn.as_bytes()[1] == b'.', "two decimals: {drawn}");
    // Names are exact, in every place a file names one.
    assert_eq!(refused(&format!("{h}registers = {{ \" r \" = 1 }}\n")), "registers: the name ` r ` has surrounding whitespace");
    assert_eq!(refused(&format!("{h}[destinations]\nlist = [1]\nbits = {{ \" b \" = 1 }}\n")), "destinations.bits: the name ` b ` has surrounding whitespace");
    assert_eq!(refused(&format!("{h}[destinations]\nlist = [1]\ninitial = {{ \" 1 \" = [] }}\n")), "destinations.initial: ` 1 ` is not a destination number");
    assert_eq!(refused(&items("fields = [ { name = \" x \", value = 1 } ]\n")), "items.fields[0]: the name ` x ` has surrounding whitespace");
    assert_eq!(refused(&with("faults = [ { name = \" f \", set = {} } ]\n")), "items.faults[0]: the name ` f ` has surrounding whitespace");
    assert_eq!(refused(&mix0.replace("name = \"o\"", "name = \" o \"")), "items.outcome.mix[0]: the name ` o ` has surrounding whitespace");
    // A binary reply's key may be a repeat's numbered name, never the repeat's own name or one
    // past its max.
    let with_repeat = format!("{}\n[[message]]\nname = \"answer2\"\nid = 27\nfields = [ {{ name = \"item\", at = 0, ty = \"U32\" }} ]\nrepeat = [ {{ name = \"bc\", pattern = \"BC=(\\\\w+)\", max = 2 }} ]\n", blocks()[0]);
    let catalog2 = catalog::load(&with_repeat).unwrap();
    let keyed = |key: &str| with(&format!("[items.reply]\nmessage = \"answer2\"\nkey = \"{key}\"\n"));
    assert!(load(&keyed("bc1"), &catalog2).is_ok());
    assert!(load(&keyed("bc2"), &catalog2).is_ok());
    assert_eq!(load(&keyed("bc3"), &catalog2).unwrap_err().to_string(), "items.reply.key: `answer2` reads nothing named `bc3`");
    assert_eq!(load(&keyed("bc"), &catalog2).unwrap_err().to_string(), "items.reply.key: `answer2` reads nothing named `bc`");
    let answer = |fields: &str| format!("{h}[[answers]]\non = \"announce\"\nsend = \"answer\"\nfields = [ {fields}, {{ name = \"dest\", value = 1 }}, {{ name = \"flag\", value = 1 }} ]\n");
    assert!(refused(&answer("{ name = \"item\", decide = { rules = [ { field = \"code\", pattern = \"(\", value = 1 } ] } }")).starts_with("answers[0].fields[0] (`item`).decide.rules[0]: regex parse error"));
    assert_eq!(refused(&answer("{ name = \"item\", decide = { spread = [ { value = 1, weight = -1 } ] } }")), "answers[0].fields[0] (`item`).decide.spread[0]: a weight is not a finite, non-negative number");
    assert_eq!(refused(&answer("{ name = \"item\", decide = { sticky = \"code\" } }")), "answers[0].fields[0] (`item`).decide: nothing to decide with");
    assert_eq!(refused(&answer("{ name = \"item\", value = 1 }, { name = \"extra\", value = 1 }").replace(", { name = \"flag\", value = 1 }", "")), "answers[0]: the message needs `flag`, which no field writes");
    let registers = BTreeMap::new();
    let plain = |spec: &str| {
        let specs = vec![toml::from_str::<FieldSpec>(&format!("name = \"f\"\n{spec}\n")).unwrap()];
        Fields::compile("messages[0]", &specs, Place::Plain, &registers, None).unwrap_err().to_string()
    };
    assert_eq!(plain("from = \"x\""), "messages[0].fields[0] (`f`): `from` needs a message or an item to read from");
    assert_eq!(plain("register = \"x\""), "messages[0].fields[0] (`f`): a plain channel has no registers");
    let cyclic = |fields: &str| format!("{h}[destinations]\nlist = [1]\n\n[[cyclic]]\nmessage = \"heartbeat\"\nevery_ms = 1000\nper_destination = true\nfields = [ {fields} ]\n");
    assert_eq!(refused(&cyclic("{ name = \"dest\", from = \"item\" }, { name = \"state\", from = \"state\" }")), "cyclic[0].fields[0] (`dest`): a cyclic message reads `destination` or `state`, per destination");
    assert_eq!(refused(&cyclic("{ name = \"dest\", from = \"destination\" }").to_string()), "cyclic[0]: the message needs `state`, which no field writes");
    // Items: the message, the interval, the lap, the key, the gate, the faults.
    assert_eq!(refused(&items("fields = []\n").replace("announce", "nope")), "items.message: the catalog has no message `nope`");
    assert_eq!(refused(&with("").replace("interval_ms = 100", "interval_ms = 0")), "items.interval_ms is 0");
    assert_eq!(refused(&with("lap_ms = 0\n")), "items.lap_ms is 0");
    assert_eq!(refused(&with("").replace("key = \"item\"", "key = \"x\"")), "items.key: no field named `x` is announced");
    assert_eq!(refused(&with("gate = \"x\"\n")), "items.gate: no register `x`");
    assert_eq!(refused(&with("faults = [ { name = \" \", set = {} } ]\n")), "items.faults[0]: the fault has no name");
    assert_eq!(refused(&with("faults = [ { name = \"f\", set = {} }, { name = \"f\", set = {} } ]\n")), "items.faults[1]: two faults are named `f`");
    assert_eq!(refused(&with("faults = [ { name = \"f\", percent = 101, set = {} } ]\n")), "items.faults[0].percent is 101, over 100");
    assert_eq!(refused(&with("faults = [ { name = \"f\", set = { x = 1 } } ]\n")), "items.faults[0].set: no announced field named `x`");
    assert_eq!(refused(&format!("{h}registers = {{ \" \" = 1 }}\n")), "registers: the register has no name");
    // The reply and the outcome.
    assert_eq!(refused(&with("[items.reply]\nmessage = \"nope\"\n")), "items.reply.message: the catalog has no message `nope`");
    assert_eq!(refused(&with("[items.reply]\nmessage = \"answer\"\nkey = \"x\"\n")), "items.reply.key: `answer` reads nothing named `x`");
    assert_eq!(refused(&with("[items.reply]\nmessage = \"answer\"\ndestination = \"x\"\n")), "items.reply.destination: `answer` reads nothing named `x`");
    assert!(load(&with("[items.reply]\nmessage = \"outcome\"\nkey = \"item\"\ndestination = \"dest\"\n"), &catalog()).is_ok(), "a text reply reads its pattern's groups");
    assert_eq!(refused(&with("[items.reply]\nmessage = \"outcome\"\nkey = \"x\"\n")), "items.reply.key: `outcome` reads nothing named `x`");
    let outcome = "[items.outcome]\nmessage = \"outcome\"\nfields = [ { name = \"item\", from = \"item\" }, { name = \"dest\", from = \"dest\" }, { name = \"result\", value = \"S\" } ]\n";
    assert_eq!(refused(&with(outcome)), "items.outcome: an outcome needs a reply to follow");
    assert_eq!(
        refused(&format!("{}[destinations]\nlist = [1]\n", with(&format!("[items.reply]\nmessage = \"answer\"\n{outcome}")))),
        "items.reply.destination: an outcome by destination needs the reply's destination field"
    );
    assert_eq!(refused(&with("[items.reply]\nmessage = \"answer\"\n[items.outcome]\nmessage = \"nope\"\nfields = []\n")), "items.outcome.message: the catalog has no message `nope`");
    let mix = |entries: &str| with(&format!("[items.reply]\nmessage = \"answer\"\n{outcome}mix = [ {entries} ]\n"));
    assert_eq!(refused(&mix("{ name = \" \", weight = 1 }")), "items.outcome.mix[0]: the outcome has no name");
    assert_eq!(refused(&mix("{ name = \"o\", weight = 1 }, { name = \"o\", weight = 1 }")), "items.outcome.mix[1]: two outcomes are named `o`");
    assert_eq!(refused(&mix("{ name = \"o\", weight = nan }")), "items.outcome.mix[0].weight is not a finite, non-negative number");
    // Answers, commands, cyclic messages.
    assert_eq!(refused(&format!("{h}[[answers]]\non = \"nope\"\nsend = \"answer\"\nfields = []\n")), "answers[0].on: the catalog has no message `nope`");
    assert_eq!(refused(&format!("{h}[[answers]]\non = \"announce\"\nsend = \"nope\"\nfields = []\n")), "answers[0].send: the catalog has no message `nope`");
    let command = |body: &str| format!("{h}registers = {{ r = 0 }}\n[[commands]]\non = \"command\"\n{body}");
    assert_eq!(refused(&command("field = \"code\"\nwhen = []\n").replace("\"command\"", "\"nope\"")), "commands[0].on: the catalog has no message `nope`");
    assert_eq!(refused(&command("field = \" \"\nwhen = [ { equals = 1, set = { r = 1 } } ]\n")), "commands[0].field is empty");
    assert_eq!(refused(&command("field = \"code\"\nwhen = []\n")), "commands[0].when is empty: a command sets something");
    assert_eq!(refused(&command("field = \"code\"\nwhen = [ { equals = 1, set = { x = 1 } } ]\n")), "commands[0].when[0].set: no register `x`");
    let cyc = |body: &str| format!("{h}[[cyclic]]\nmessage = \"status\"\n{body}");
    assert_eq!(refused(&cyc("every_ms = 1000\nfields = []\n").replace("\"status\"", "\"nope\"")), "cyclic[0].message: the catalog has no message `nope`");
    assert_eq!(refused(&cyc("every_ms = 0\nfields = []\n")), "cyclic[0].every_ms is 0");
    assert_eq!(refused(&cyc("every_ms = 1000\nper_destination = true\nfields = []\n")), "cyclic[0]: per_destination needs [destinations]");
    assert_eq!(refused(&cyc("every_ms = 1000\nfields = []\n")), "cyclic[0]: the message needs `running`, which no field writes");
    // Destinations.
    let dest = |body: &str| format!("{h}[destinations]\n{body}");
    assert_eq!(refused(&dest("list = []\n")), "destinations.list is empty");
    assert_eq!(refused(&dest("list = [1, 1]\n")), "destinations.list: 1 is listed twice");
    assert_eq!(refused(&dest("list = [1]\nbits = { \" \" = 1 }\n")), "destinations.bits: the bit has no name");
    assert_eq!(refused(&dest("list = [1]\nbits = { b = 3 }\n")), "destinations.bits.b: 3 is not one bit");
    assert_eq!(refused(&dest("list = [1]\nbits = { b = 0 }\n")), "destinations.bits.b: 0 is not one bit");
    assert_eq!(refused(&dest("list = [1]\nbits = { b = 1, c = 1 }\n")), "destinations.bits.c: bit 1 is named twice");
    assert_eq!(refused(&dest("list = [1]\nresting = [\"x\"]\n")), "destinations.resting: no bit named `x`");
    assert_eq!(refused(&dest("list = [1]\ninitial = { \"1\" = [\"x\"] }\n")), "destinations.initial.1: no bit named `x`");
    assert_eq!(refused(&dest("list = [1]\ninitial = { \"9\" = [] }\n")), "destinations.initial: 9 is not in the list");
    assert_eq!(refused(&dest("list = [1]\ninitial = { \"x\" = [] }\n")), "destinations.initial: `x` is not a destination number");
    assert_eq!(refused(&dest("list = [1]\nblocking = [ { bit = \"x\" } ]\n")), "destinations.blocking[0]: no bit named `x`");
    assert_eq!(refused(&dest("list = [1]\nbits = { b = 1 }\nblocking = [ { bit = \"b\" }, { bit = \"b\" } ]\n")), "destinations.blocking[1]: `b` blocks twice");
    // The closing paragraph says all of it.
    assert!(doc_prose().contains("A field with no source or more than one, a range that is empty or not finite, a counter ending before it starts, an empty cycle, a bad template, a source its place does not take, a register that does not exist, a decision with a bad pattern or nothing to decide with, a list that leaves one of its message's names unwritten, a message the catalog does not hold, an interval or lap of 0, a key no field announces, a fault's share over 100 or a fault setting a field that is not announced, a reply key or destination the reply does not read, an outcome without a reply, a weight that is not finite or below 0, a command with nothing to set or setting a register that does not exist, a destination list that is empty or repeats a number, a bit that is not one bit or named twice, a resting or initial or blocking bit that is not named, an initial destination not in the list, a bit blocking twice, and a misspelled key in any table are refused when the file loads, with the place named."));
}

#[test]
fn every_rule_the_reference_states_about_loading_is_the_reader_s() {
    // Each statement of the reference about what loads and what it loads as, quoted beside the
    // behaviour that pins it; what the loaded rules do is quoted in tests/controller.rs.
    let prose = doc_prose();
    let catalog = catalog();
    let h = header();
    assert!(prose.contains("A name (of a register, a bit, a field, a fault, an outcome) is exact, with no surrounding whitespace; a message name is one of the catalog's."));
    assert!(refused(&format!("{h}[[cyclic]]\nmessage = \" status\"\nevery_ms = 1\nfields = []\n")).contains("no message ` status`"));
    assert!(prose.contains("`seed` (none): the seed of the controller's draws, for a run that repeats; the clock when absent. A seed the bench gives the controller wins over the file's."));
    assert!(prose.contains("`registers` (none): a table of named integers, the controller's memory: read by a field's `register` and by the items' `gate`, set by a command and by the bench; every register a file names must be in this table."));
    let rules = load(&format!("{h}registers = {{ a = -5, b = 7 }}\n"), &catalog).unwrap();
    assert_eq!(rules.registers, BTreeMap::from([("a".to_string(), -5), ("b".to_string(), 7)]));
    assert!(prose.contains("`list` (required): the destination numbers, at least one, none twice."));
    assert!(prose.contains("`bits` (none): the named bits of the state word, each one bit (a power of two), no bit named twice."));
    assert!(prose.contains("`resting` (none): the bits a destination has at the start when `initial` does not name it."));
    assert!(prose.contains("`initial` (none): bits set at the start, by destination number written as text, exact, with no surrounding whitespace; a number not in the list is refused."));
    let d = load(&format!("{h}[destinations]\nlist = [1, 2]\nbits = {{ a = 1, b = 4 }}\nresting = [\"a\"]\ninitial = {{ \"2\" = [\"a\", \"b\"] }}\n"), &catalog).unwrap();
    let d = d.destinations.unwrap();
    assert_eq!((d.resting, d.initial), (1, BTreeMap::from([(2, 5)])));
    let c = Controller::new(load(&format!("{h}[destinations]\nlist = [1, 2]\nbits = {{ a = 1, b = 4 }}\nresting = [\"a\"]\ninitial = {{ \"2\" = [\"b\"] }}\n"), &catalog).unwrap(), None);
    assert_eq!(c.status().destinations, BTreeMap::from([(1, 1), (2, 4)]), "resting where initial does not say");
    assert!(prose.contains("`blocking` (none): the bits that refuse an item, as inline tables, in order of precedence: the first entry whose bit is set in a destination's word decides; a bit blocks once."));
    assert!(prose.contains("`bit` (required): a named bit."));
    assert!(prose.contains("`set` (none): the outcome fields it forces, by name, overriding what the fields rendered."));
    assert!(prose.contains("`requeue` (`false`): the item comes round again after a lap instead of ending."));
    assert!(prose.contains("`message` (required): the announcement, a message of the catalog."));
    assert!(prose.contains("`interval_ms` (required): the announcement interval; `0` is refused. An item is announced only while a peer is connected"));
    assert!(prose.contains("`key` (required): the announced field that identifies the item; one of the `fields`."));
    assert!(prose.contains("`gate` (none): a register that must be non-zero for the controller to announce."));
    assert!(prose.contains("`lap_ms` (`30000`): how long a requeued item takes to come round again; `0` is refused."));
    assert!(prose.contains("`fields` (required): the announced fields, one for every name the message writes."));
    assert!(prose.contains("`name` (required): the fault's name, unique; the bench's knob and the status use it."));
    assert!(prose.contains("`percent` (`0`): the share of announced items, 0 to 100, drawn item by item, fault by fault."));
    assert!(prose.contains("`set` (required): announced fields by name and the value each takes."));
    assert!(prose.contains("`key` (the items' `key`): the reply's field holding the item's key: a name the reply reads, as the catalog says (a binary field, a payload group, a repeat's numbered name)."));
    assert!(prose.contains("`destination` (none): the reply's field naming the destination, a name the reply reads; required when there is an outcome and there are destinations."));
    assert!(prose.contains("A field a disposition, a rule, a command or a `sticky` key names is looked up when the message comes: a name the message does not carry simply never matches."));
    assert!(load(&format!("{h}[[commands]]\non = \"command\"\nfield = \"nope\"\nwhen = [ {{ equals = 1, set = {{}} }} ]\n"), &catalog).is_ok(), "a command's field is looked up when the message comes");
    assert!(prose.contains("`when` (none): dispositions by a field's value, in order, as inline tables; the first match decides, else the item is delivered."));
    assert!(prose.contains("`equals` (required): its value, compared as text, trimmed."));
    assert!(prose.contains("`after_ms` (`0`): the travel time from the answer to the report."));
    assert!(prose.contains("`fields` (required): the outcome's fields, one for every name the message writes; they read the item's announced fields and its reply's, the reply's trimmed."));
    assert!(prose.contains("`mix` (none): the weighted outcomes, as inline tables, drawn when the destination's state forces none; with none, or none with a weight above 0, the item ends `delivered`."));
    assert!(prose.contains("`weight` (required): its share, a finite number of 0 or more."));
    assert!(prose.contains("`on` (required): the message answered, of the catalog."));
    assert!(prose.contains("`send` (required): the message sent, of the catalog."));
    assert!(prose.contains("`after_ms` (`0`): the delay before it goes."));
    assert!(prose.contains("`fields` (required): the fields sent, one for every name the message writes; they read the incoming message's fields."));
    assert!(prose.contains("`field` (required): the field read."));
    assert!(prose.contains("`when` (required): settings by value, at least one, as inline tables; a value none matches sets nothing."));
    assert!(prose.contains("`set` (required): registers by name and the value each takes."));
    assert!(prose.contains("`every_ms` (required): the interval; `0` is refused. Each is due an interval after the last was due, and sent at most once a pass of the engine: a state message is not worth a burst."));
    assert!(prose.contains("`per_destination` (`false`): one message per destination each time, its fields reading `destination` (the number) and `state` (the word); it needs `[destinations]`."));
    assert!(prose.contains("`fields` (required): the fields sent, one for every name the message writes; they read the registers."));
    // A field and its sources.
    assert!(prose.contains("exactly one of `value`, `range`, `counter`, `cycle`, `template`, `from`, `register`, `now` or `decide`. Fields render in order, each later one seeing the earlier ones; every value is text on the wire."));
    assert!(prose.contains("`name` (required): the field's name, unique within its list."));
    assert!(prose.contains("`value` (one source): a constant, a number, a flag or text."));
    assert!(prose.contains("`range` (one source): `[low, high]`, finite, `low` not above `high`: a draw within it, whole when `decimals` is 0 (from the ceiling of `low` to the floor of `high`), else to `decimals` places."));
    assert!(prose.contains("`decimals` (`0`): the places of a `range` draw, at most 15."));
    assert!(prose.contains("`counter` (one source): a number counting up from `first`, back to `first` after `last`."));
    assert!(prose.contains("`cycle` (one source): the next of these values, round and round; at least one."));
    assert!(prose.contains("`template` (one source): text with `{name}` placeholders of the earlier fields and of the context; a name not there is a value missing, and the message is not sent."));
    assert!(prose.contains("`from` (one source): an earlier field of the list, else a field of the context: the incoming message's for an answer, the item's and its reply's for an outcome, `destination` or `state` for a cyclic message per destination; a plain channel's own message has no context. A name not there is a value missing, and the message is not sent."));
    assert!(prose.contains("`register` (one source): a register's value; a plain channel's own message has none."));
    assert!(prose.contains("`now` (one source): the wall clock in this `chrono` format."));
    assert!(prose.contains("`decide` (one source): a destination decided by rules, in an answer only."));
    assert!(prose.contains("`first` (`1`): where it starts."));
    assert!(prose.contains("`last` (none): where it wraps back to `first`, not below `first`; without it, at the top of a 64-bit number."));
    assert!(prose.contains("first the remembered decision for the key, then the first matching rule, then a draw over the spread, each skipped when its value names a destination blocked right now, then the reject value, never skipped; with nothing left, no decision. At least one of `rules`, `spread` and `reject` is given."));
    assert!(prose.contains("`rules` (none): first-match rules, as inline tables."));
    assert!(prose.contains("`spread` (none): weighted values, as inline tables; a value with a weight of 0 is never drawn."));
    assert!(prose.contains("`reject` (none): the value when no rule matches and nothing can be drawn."));
    assert!(prose.contains("`sticky` (none): the context field whose value keys a remembered decision: the same key decides the same again, while that value is not blocked; the newest 10000 keys are kept."));
    assert!(prose.contains("`field` (required): the context field it looks at."));
    assert!(prose.contains("`pattern` (required): a regex over the field's value, trimmed; `(empty)` matches a field that is absent or blank."));
    assert!(prose.contains("`value` (required): the decision."));
    // Every field a message writes must be written, by name; a text message's names are its
    // placeholders.
    let text = format!("{h}[[cyclic]]\nmessage = \"status\"\nevery_ms = 1\nfields = [ {{ name = \"running\", value = 1 }} ]\n");
    assert_eq!(refused(&text), "cyclic[0]: the message needs `speed`, which no field writes");
    let text = format!("{h}[[cyclic]]\nmessage = \"status\"\nevery_ms = 1\nfields = [ {{ name = \"speed\", value = 1 }}, {{ name = \"running\", value = 1 }}, {{ name = \"more\", value = 1 }} ]\n");
    assert!(load(&text, &catalog).is_ok(), "a field the message does not write is allowed: it feeds the later ones");
}

#[test]
fn the_reference_is_the_text_this_test_holds() {
    let actual = paragraphs(&doc());
    let expected = paragraphs(REFERENCE);
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(a, e, "paragraph {i} of docs/controller-schema.md differs from the copy this test holds");
    }
    assert_eq!(actual.len(), expected.len(), "docs/controller-schema.md has {} paragraphs, the copy {}", actual.len(), expected.len());
}

/// The reference, `docs/controller-schema.md`, as this test holds it: its paragraphs, the three
/// TOML blocks left out. Compared paragraph by paragraph, whitespace collapsed, by
/// `the_reference_is_the_text_this_test_holds`; every statement in it is held against the reader
/// here or against the engine in `tests/controller.rs`. An edit to the reference is an edit here.
const REFERENCE: &str = r#"
# The controller file: `veredas-controller`, version 1

What a controller does on its wire is configured, not programmed (`docs/questions.md` Q8). The
channel file (`docs/channel-schema.md`) frames the link and the catalog file
(`docs/catalog-schema.md`) names the messages; the controller file says what the bench's
simulated controller does with them: the items it announces at a rate and awaits an answer
for, the outcome it reports when an item's travel ends, the answers it gives to messages, the
commands it obeys, the messages it sends on an interval, and the destinations it serves, each
with a state word of named bits that may refuse an item. `veredas_maquinar::controller::load`
reads it against the catalog its messages come from, into `Rules`; a `Controller` runs them,
pure and tick-driven, and a bench channel drives it from the wire. The same file format plays
either side of a conversation: the machine that announces, or the stand-in for the unit that
answers it. No device is named anywhere: a device's rules are a file like the ones below, kept
with its channel and catalog files, outside the suite's code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed.

## The worked example: a machine, the unit it talks to, and their catalog

The catalog below extends the catalog file's worked example with three messages: a `command`
the machine obeys (id 24), a `heartbeat` it sends per destination (id 25) and a `status` line
of its registers (id 26). The machine announces an item every 100 ms while a peer is connected
and its `running` register is not 0; one item in 10 carries the fault `noread` (its `read` flag
`N` and an empty `code`). It awaits an `answer` keyed by `item`, whose `dest` names the
destination: a `flag` of 2 sends the item round again after a lap of 2000 ms, a `flag` of 3
parks it, anything else delivers it, and 500 ms later the machine reports the `outcome`, drawn
from a mix of 9 `sorted` to 1 `lost`, unless the destination's state forces one: a destination
with its `full` bit set forces `FULL` and the item comes round again, one with `fault` set
forces `FAULT`. Destinations 1 to 4 rest with no bit set; destination 4 starts `full`. The
`command` message's `code` sets the registers: 1 starts the machine, 0 stops it, 7 sets `speed`
to 70. A `heartbeat` goes out every 1000 ms for each destination with its state word, and a
`status` line every 5000 ms with the registers.

The machine:

The unit answers every `announce` at once with an `answer` for the same `item`: an item whose
`code` is empty goes to destination 0, any other to destination 1, 2 or 3 in the shares 2, 1
and 1, the same destination again for the same `code`, and to 9, the reject, when every one of
those is full. The unit keeps its own `[destinations]`, so the bench may mark one `full` on it
too and the spread then avoids it. The `flag` is 1.

`crates/veredas-maquinar/tests/controller_rules.rs` loads these three blocks and checks each of
these statements, and every key statement below, against the reader; it holds a copy of this
text, so an edit here is an edit there, beside the behaviour the sentence is held against.
`crates/veredas-maquinar/tests/controller.rs` runs the two files as the engine, tick by tick,
and then against each other over a real socket.

## Keys

Each key states in parentheses that it is required, or the value it takes when left out. A
name (of a register, a bit, a field, a fault, an outcome) is exact, with no surrounding
whitespace; a message name is one of the catalog's. A value compared with a field's (a
disposition's or a setting's `equals`, a rule's `pattern`, a reply's key) is compared as text,
the field's value trimmed. A field a disposition, a rule, a command or a `sticky` key names is
looked up when the message comes: a name the message does not carry simply never matches.

**The top level**.
- `format` (required): always `"veredas-controller"`.
- `version` (required): this reader knows `1`. A file of another format or an unknown version is
  refused, never guessed.
- `seed` (none): the seed of the controller's draws, for a run that repeats; the clock when
  absent. A seed the bench gives the controller wins over the file's.
- `registers` (none): a table of named integers, the controller's memory: read by a field's
  `register` and by the items' `gate`, set by a command and by the bench; every register a
  file names must be in this table.
- `destinations` (none): the `[destinations]` table.
- `items` (none): the `[items]` table.
- `answers` (none): `[[answers]]` tables.
- `commands` (none): `[[commands]]` tables.
- `cyclic` (none): `[[cyclic]]` tables.

**The destinations** (`[destinations]`): what the controller serves, each with a state word.
- `list` (required): the destination numbers, at least one, none twice.
- `bits` (none): the named bits of the state word, each one bit (a power of two), no bit named
  twice.
- `resting` (none): the bits a destination has at the start when `initial` does not name it.
- `initial` (none): bits set at the start, by destination number written as text, exact, with
  no surrounding whitespace; a number not in the list is refused.
- `blocking` (none): the bits that refuse an item, as inline tables, in order of precedence:
  the first entry whose bit is set in a destination's word decides; a bit blocks once.

**A blocking bit** (an entry of `blocking`).
- `bit` (required): a named bit.
- `set` (none): the outcome fields it forces, by name, overriding what the fields rendered.
- `requeue` (`false`): the item comes round again after a lap instead of ending.

**The items** (`[items]`): what the controller announces and awaits an answer for.
- `message` (required): the announcement, a message of the catalog.
- `interval_ms` (required): the announcement interval; `0` is refused. An item is announced
  only while a peer is connected (announced into nothing it would only time out) and the gate
  register, if any, is not 0, a requeued item coming round again included; each is due an
  interval after the last was due, and a pass of the engine announces every item due since the
  last pass, at most twenty (a longer stall skips the rest, keeping the phase); when announcing
  resumes, the first item comes at once.
- `key` (required): the announced field that identifies the item; one of the `fields`.
- `gate` (none): a register that must be non-zero for the controller to announce.
- `lap_ms` (`30000`): how long a requeued item takes to come round again; `0` is refused.
- `fields` (required): the announced fields, one for every name the message writes.
- `faults` (none): named faults, as inline tables.
- `reply` (none): the `[items.reply]` table.
- `outcome` (none): the `[items.outcome]` table; it needs a `reply` to follow.

**A fault** (an entry of `faults`): this share of items has these fields set instead.
- `name` (required): the fault's name, unique; the bench's knob and the status use it.
- `percent` (`0`): the share of announced items, 0 to 100, drawn item by item, fault by fault.
- `set` (required): announced fields by name and the value each takes.

**The reply** (`[items.reply]`): the answer awaited for an item.
- `message` (required): a message of the catalog. An answer for an item not awaited is
  counted and dropped.
- `key` (the items' `key`): the reply's field holding the item's key: a name the reply reads,
  as the catalog says (a binary field, a payload group, a repeat's numbered name).
- `destination` (none): the reply's field naming the destination, a name the reply reads;
  required when there is an outcome and there are destinations. A destination not in the list is counted and the item
  still travels.
- `when` (none): dispositions by a field's value, in order, as inline tables; the first match
  decides, else the item is delivered.

**A disposition** (an entry of `when`).
- `field` (required): the reply's field.
- `equals` (required): its value, compared as text, trimmed.
- `action` (required): `deliver` (the item travels and ends in an outcome, or ends delivered
  without one), `requeue` (round again after a lap) or `park` (asked about no more).

**The outcome** (`[items.outcome]`): what the controller reports when an item's travel ends.
- `message` (required): a message of the catalog.
- `after_ms` (`0`): the travel time from the answer to the report.
- `fields` (required): the outcome's fields, one for every name the message writes; they read
  the item's announced fields and its reply's, the reply's trimmed.
- `mix` (none): the weighted outcomes, as inline tables, drawn when the destination's state
  forces none; with none, or none with a weight above 0, the item ends `delivered`.

**A mixed outcome** (an entry of `mix`).
- `name` (required): the outcome's name, unique; the status counts items by it.
- `weight` (required): its share, a finite number of 0 or more.
- `set` (none): the fields it forces, by name, overriding what the fields rendered.
- `requeue` (`false`): the item comes round again after a lap instead of ending.

**An answer** (`[[answers]]`): a message sent in answer to another, to the peer it came from.
- `on` (required): the message answered, of the catalog.
- `send` (required): the message sent, of the catalog.
- `after_ms` (`0`): the delay before it goes.
- `fields` (required): the fields sent, one for every name the message writes; they read the
  incoming message's fields. An answer whose field cannot render (a decision not made, a name
  not there) is not sent, and counted.

**A command** (`[[commands]]`): an incoming message whose field's value sets registers.
- `on` (required): the message, of the catalog.
- `field` (required): the field read.
- `when` (required): settings by value, at least one, as inline tables; a value none matches
  sets nothing.

**A setting** (an entry of a command's `when`).
- `equals` (required): the field's value, compared as text, trimmed.
- `set` (required): registers by name and the value each takes.

**A cyclic message** (`[[cyclic]]`): a message sent on an interval, to every peer.
- `message` (required): a message of the catalog.
- `every_ms` (required): the interval; `0` is refused. Each is due an interval after the last
  was due, and sent at most once a pass of the engine: a state message is not worth a burst.
- `per_destination` (`false`): one message per destination each time, its fields reading
  `destination` (the number) and `state` (the word); it needs `[destinations]`.
- `fields` (required): the fields sent, one for every name the message writes; they read the
  registers.

**A field** (an entry of any `fields`): a named value and where it comes from, exactly one of
`value`, `range`, `counter`, `cycle`, `template`, `from`, `register`, `now` or `decide`. Fields
render in order, each later one seeing the earlier ones; every value is text on the wire.
- `name` (required): the field's name, unique within its list.
- `value` (one source): a constant, a number, a flag or text.
- `range` (one source): `[low, high]`, finite, `low` not above `high`: a draw within it, whole
  when `decimals` is 0 (from the ceiling of `low` to the floor of `high`), else to `decimals`
  places.
- `decimals` (`0`): the places of a `range` draw, at most 15.
- `counter` (one source): a number counting up from `first`, back to `first` after `last`.
- `cycle` (one source): the next of these values, round and round; at least one.
- `template` (one source): text with `{name}` placeholders of the earlier fields and of the
  context; a name not there is a value missing, and the message is not sent.
- `from` (one source): an earlier field of the list, else a field of the context: the incoming
  message's for an answer, the item's and its reply's for an outcome, `destination` or `state`
  for a cyclic message per destination; a plain channel's own message has no context. A name
  not there is a value missing, and the message is not sent.
- `register` (one source): a register's value; a plain channel's own message has none.
- `now` (one source): the wall clock in this `chrono` format.
- `decide` (one source): a destination decided by rules, in an answer only.

**A counter** (the value of `counter`).
- `first` (`1`): where it starts.
- `last` (none): where it wraps back to `first`, not below `first`; without it, at the top of
  a 64-bit number.

**A decision** (the value of `decide`): first the remembered decision for the key, then the
first matching rule, then a draw over the spread, each skipped when its value names a
destination blocked right now, then the reject value, never skipped; with nothing left, no
decision. At least one of `rules`, `spread` and `reject` is given.
- `rules` (none): first-match rules, as inline tables.
- `spread` (none): weighted values, as inline tables; a value with a weight of 0 is never
  drawn.
- `reject` (none): the value when no rule matches and nothing can be drawn.
- `sticky` (none): the context field whose value keys a remembered decision: the same key
  decides the same again, while that value is not blocked; the newest 10000 keys are kept.

**A rule** (an entry of `rules`).
- `field` (required): the context field it looks at.
- `pattern` (required): a regex over the field's value, trimmed; `(empty)` matches a field that
  is absent or blank.
- `value` (required): the decision.

**A share** (an entry of `spread`).
- `value` (required): the decision.
- `weight` (required): its share, a finite number of 0 or more.

A field with no source or more than one, a range that is empty or not finite, a counter ending
before it starts, an empty cycle, a bad template, a source its place does not take, a register
that does not exist, a decision with a bad pattern or nothing to decide with, a list that leaves
one of its message's names unwritten, a message the catalog does not hold, an interval or lap of
0, a key no field announces, a fault's share over 100 or a fault setting a field that is not
announced, a reply key or destination the reply does not read, an outcome without a
reply, a weight that is not finite or below 0, a command with nothing to set or setting a
register that does not exist, a destination list that is empty or repeats a number, a bit that
is not one bit or named twice, a resting or initial or blocking bit that is not named, an
initial destination not in the list, a bit blocking twice, and a misspelled key in any table are
refused when the file loads, with the place named.
"#;
