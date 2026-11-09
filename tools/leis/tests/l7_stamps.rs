//! L7 (C-046): every TOML file of the repository that is not a cargo manifest, and every TOML block
//! of a reference in `docs/`, states a `veredas-*` `format` and a `version` from 1.

mod common;

use common::*;

fn stamped(entries: &[(&str, &str)]) -> leis::Suite {
    let mut s = suite(&ws(&["tools/leis"]), &[("tools/leis", LEIS)], &[]);
    s.stamped = entries.iter().map(|(place, text)| (place.to_string(), text.to_string())).collect();
    s
}

#[test]
fn l7_a_stamped_file_passes() {
    let s = stamped(&[
        ("docs/thing-schema.md block 1", "format = \"veredas-thing\"\nversion = 1\n[part]\nx = 1\n"),
        ("crates/veredas-x/tests/fixtures/a.toml", "version = 3\nformat = \"veredas-other\"\n"),
    ]);
    assert_eq!(strings(leis::stamps(&s)), NONE);
}

#[test]
fn l7_every_way_to_miss_the_stamp_is_named() {
    let s = stamped(&[
        ("docs/a.md block 1", "not = [toml"),
        ("docs/a.md block 2", "version = 1\n"),
        ("docs/a.md block 3", "format = \"bench\"\nversion = 1\n"),
        ("docs/a.md block 4", "format = 7\nversion = 1\n"),
        ("fixtures/b.toml", "format = \"veredas-b\"\n"),
        ("fixtures/c.toml", "format = \"veredas-c\"\nversion = 0\n"),
        ("fixtures/d.toml", "format = \"veredas-d\"\nversion = \"1\"\n"),
    ]);
    let got = strings(leis::stamps(&s));
    assert_eq!(got.len(), 7, "{got:?}");
    assert!(got[0].starts_with("L7 docs/a.md block 1: not TOML ("), "{}", got[0]);
    assert_eq!(got[1], "L7 docs/a.md block 2: no `format` (a plain-text file of the suite states `format` and `version`)");
    assert_eq!(got[2], "L7 docs/a.md block 3: `format` is `bench`, not a `veredas-*` format");
    assert_eq!(got[3], "L7 docs/a.md block 4: `format` is not a string");
    assert_eq!(got[4], "L7 fixtures/b.toml: no `version` (a plain-text file of the suite states `format` and `version`)");
    assert_eq!(got[5], "L7 fixtures/c.toml: `version` is 0, not a whole number from 1");
    assert_eq!(got[6], "L7 fixtures/d.toml: `version` is not a whole number");
}

#[test]
fn l7_the_reader_collects_toml_files_but_manifests_and_the_blocks_of_a_reference() {
    let tree = Tree::new(
        "l7",
        &[
            ("Cargo.toml", &ws(&["tools/leis"])),
            ("tools/leis/Cargo.toml", LEIS),
            ("docs/thing-schema.md", "# A thing\n\ntext\n\n```toml\nformat = \"veredas-thing\"\nversion = 1\n```\n\nmore\n\n```toml\nformat = \"veredas-thing\"\nversion = 1\nx = 2\n```\n"),
            // Every fence CommonMark allows opens a TOML block: a trailing space, a title after
            // the word, tildes, the word in capitals, a longer fence closed by one as long; a
            // block of another language, and a fence whose word only starts with `toml`, do not.
            (
                "docs/fences-schema.md",
                "# Fences\n\n```toml \nformat = \"veredas-fence\"\nversion = 1\n```\n\n```toml title=\"a\"\nformat = \"veredas-fence\"\nversion = 2\n```\n\n~~~toml\nformat = \"veredas-fence\"\nversion = 3\n~~~\n\n```TOML\nformat = \"veredas-fence\"\nversion = 4\n```\n\n````toml\nformat = \"veredas-fence\"\nversion = 5\ntext = \"\"\"\n```\nstill inside\n\"\"\"\n````\n\n```tomlx\nnot = \"toml\"\n```\n\n```json\n{}\n```\n",
            ),
            // A fence inside a blockquote (its marks taken off the body too), one opening a list
            // item, one two quotes deep, one holding a deeper fence line inside a string (not
            // its closer), one ended by a line without its quote (the blockquote ends there, as
            // CommonMark has it), a quote inside a list item, a list item inside a list item, a
            // list mark followed by a tab, and a block the text ends before closing.
            (
                "docs/held-schema.md",
                "# Held\n\n> ```toml\n> format = \"veredas-held\"\n> version = 1\n> ```\n\n- ```toml\n  format = \"veredas-held\"\n  version = 2\n  ```\n\n> > ```toml\n> > format = \"veredas-held\"\n> > version = 3\n> > ```\n\n> ```toml\n> format = \"veredas-held\"\n> version = 4\n> text = \"\"\"\n> > ```\n> \"\"\"\n> ```\n\n> ```toml\n> format = \"veredas-held\"\n> version = 5\nplain\n> ```\n\n- > ```toml\n  > format = \"veredas-held\"\n  > version = 6\n  > ```\n\n- - ```toml\n    format = \"veredas-held\"\n    version = 7\n    ```\n\n-\t```toml\n\tformat = \"veredas-held\"\n\tversion = 8\n\t```\n\n```toml\nformat = \"veredas-held\"\nversion = 9\n",
            ),
            ("docs/other-plan.md", "# plan\n\n```rust\nfn x() {}\n```\n"),
            ("docs/upper-schema.MD", "# Upper\n\n```toml\nformat = \"veredas-upper\"\nversion = 1\n```\n"),
            ("fixtures/a.toml", "format = \"veredas-a\"\nversion = 1\n"),
            ("fixtures/B.TOML", "format = \"veredas-b\"\nversion = 1\n"),
            ("tools/leis/tests/data/Cargo.toml", "[package]\nname = \"x\"\n"),
        ],
    );
    let s = leis::Suite::read(tree.path()).expect("the tree reads");
    let places: Vec<&str> = s.stamped.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        places,
        [
            "docs/fences-schema.md block 1",
            "docs/fences-schema.md block 2",
            "docs/fences-schema.md block 3",
            "docs/fences-schema.md block 4",
            "docs/fences-schema.md block 5",
            "docs/held-schema.md block 1",
            "docs/held-schema.md block 2",
            "docs/held-schema.md block 3",
            "docs/held-schema.md block 4",
            "docs/held-schema.md block 5",
            "docs/held-schema.md block 6",
            "docs/held-schema.md block 7",
            "docs/held-schema.md block 8",
            "docs/held-schema.md block 9",
            "docs/thing-schema.md block 1",
            "docs/thing-schema.md block 2",
            "docs/upper-schema.MD block 1",
            "fixtures/B.TOML",
            "fixtures/a.toml",
        ]
    );
    let block = |place: &str| s.stamped.iter().find(|(p, _)| p == place).unwrap().1.as_str();
    assert!(block("docs/fences-schema.md block 5").contains("```\nstill inside"), "a shorter fence inside a longer one is text");
    assert_eq!(block("docs/held-schema.md block 1"), "format = \"veredas-held\"\nversion = 1\n", "the quote marks come off the body");
    assert_eq!(block("docs/held-schema.md block 3"), "format = \"veredas-held\"\nversion = 3\n");
    assert!(block("docs/held-schema.md block 4").contains("> ```\n\"\"\"\n"), "a deeper fence line is body, not the closer: {:?}", block("docs/held-schema.md block 4"));
    assert_eq!(block("docs/held-schema.md block 5"), "format = \"veredas-held\"\nversion = 5\n", "a line without the quote ends the blockquote and the block");
    assert_eq!(block("docs/held-schema.md block 6"), "format = \"veredas-held\"\nversion = 6\n", "a quote inside a list item");
    assert_eq!(block("docs/held-schema.md block 9"), "format = \"veredas-held\"\nversion = 9\n", "a block the text ends before closing");
    assert_eq!(strings(leis::stamps(&s)), NONE);
}
