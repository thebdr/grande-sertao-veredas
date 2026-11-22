//! A package with no `xl/styles.xml` (OOXML allows it; every cell is then
//! format 0) opens and patches: value, formula and structure edits write no
//! styles part, and a fill or border synthesizes a minimal one — with its
//! relationship and content-type override — only then.
//!
//! The fixture `tests/fixtures/no-styles.xlsx` is upstream IronCalc's
//! Excel-saved `forks/ironcalc/xlsx/tests/calc_tests/arithmetic.xlsx` (MIT OR
//! Apache-2.0) with its styles part, that part's relationship and override,
//! and every cell's `s="N"` stripped, as Veredas 1 made it (Excel opened it
//! there with no repair; carried as it was).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use veredas_abaco::patch::cached::read_elements;
use veredas_abaco::patch::styles::MINIMAL;
use veredas_abaco::patch::{patch, Edit, Fate, LineStyle, PatchOptions, PatchReport, Status};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/no-styles.xlsx");

fn parts(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        out.insert(f.name().unwrap().to_string(), b);
    }
    out
}

fn text(path: &Path, name: &str) -> String {
    String::from_utf8(parts(path).remove(name).unwrap_or_else(|| panic!("no {name}"))).unwrap()
}

fn run(edits: &[Edit]) -> (tempfile::TempDir, PathBuf, PathBuf, PatchReport) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("no-styles.xlsx");
    std::fs::copy(FIXTURE, &src).unwrap();
    let dst = dir.path().join("patched.xlsx");
    let report = patch(
        &src,
        &dst,
        edits,
        &PatchOptions {
            verify: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(report.outcomes.iter().all(|o| o.status == Status::Applied), "{report:?}");
    // ABACO_PATCH_KEEP=<folder>: keep every output for the real-Excel check
    if let Ok(keep) = std::env::var("ABACO_PATCH_KEEP") {
        let name = std::thread::current()
            .name()
            .unwrap_or("patch")
            .replace("::", "_");
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::copy(&dst, Path::new(&keep).join(format!("{name}.xlsx"))).unwrap();
    }
    (dir, src, dst, report)
}

fn fate(report: &PatchReport, f: Fate) -> Vec<String> {
    report
        .parts
        .iter()
        .filter(|p| p.fate == f)
        .map(|p| p.name.clone())
        .collect()
}

/// Every part the report calls identical is the source's bytes.
fn identical_really(src: &Path, dst: &Path, report: &PatchReport) {
    let (a, b) = (parts(src), parts(dst));
    for n in fate(report, Fate::Identical) {
        assert_eq!(a.get(&n), b.get(&n), "{n} must be byte-identical");
    }
}

/// Sheet1's `<c>` elements that differ, as addresses.
fn differing(src: &Path, dst: &Path, sheet: &str) -> BTreeSet<String> {
    let find = |p: &Path| {
        read_elements(&std::fs::read(p).unwrap())
            .unwrap()
            .into_iter()
            .find(|(n, _, _)| n == sheet)
            .unwrap()
            .2
    };
    let (a, b) = (find(src), find(dst));
    a.keys()
        .chain(b.keys())
        .filter(|k| a.get(k) != b.get(k))
        .map(|k| format!("{}{}", veredas_abaco::a1::column_name(k.1), k.0))
        .collect()
}

fn no_styles_anywhere(dst: &Path) {
    let p = parts(dst);
    assert!(!p.keys().any(|n| n.contains("styles")), "{:?}", p.keys());
    assert!(!text(dst, "[Content_Types].xml").contains("styles"));
    assert!(!text(dst, "xl/_rels/workbook.xml.rels").contains("/styles\""));
}

#[test]
fn the_fixture_has_no_styles_part() {
    let p = parts(Path::new(FIXTURE));
    assert!(!p.contains_key("xl/styles.xml"));
    no_styles_anywhere(Path::new(FIXTURE));
    for n in ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
        assert!(!text(Path::new(FIXTURE), n).contains(" s=\""), "{n}");
    }
}

#[test]
fn a_value_edit_patches_with_no_styles_part_written() {
    let (_d, src, dst, report) = run(&[Edit::Value {
        sheet: "Sheet1".into(),
        cell: "C2".into(),
        value: serde_json::json!(5),
        expect: None,
    }]);
    assert_eq!(fate(&report, Fate::Rewritten), vec!["xl/worksheets/sheet1.xml"]);
    assert!(fate(&report, Fate::Added).is_empty());
    assert!(fate(&report, Fate::Removed).is_empty());
    identical_really(&src, &dst, &report);
    no_styles_anywhere(&dst);
    let s = text(&dst, "xl/worksheets/sheet1.xml");
    assert!(s.contains(r#"<c r="C2"><v>5</v></c><c r="D2"><v>2</v></c><c r="E2"><f>C2+D2</f><v>7</v></c><c r="F2"><f>C2-D2</f><v>3</v></c><c r="G2"><f>C2*D2</f><v>10</v></c><c r="H2"><f>C2/D2</f><v>2.5</v></c>"#), "{s}");
    assert_eq!(
        differing(&src, &dst, "Sheet1"),
        ["C2", "E2", "F2", "G2", "H2"].iter().map(|s| s.to_string()).collect()
    );
}

#[test]
fn formulas_and_rows_patch_with_no_styles_part_written() {
    let (_d, src, dst, report) = run(&[
        Edit::Formula {
            sheet: "Sheet1".into(),
            cell: "J2".into(),
            formula: "=E2*10".into(),
            expect: None,
        },
        Edit::InsertRows {
            sheet: "Sheet1".into(),
            at: 2,
            count: 1,
        },
    ]);
    // the calc chain goes (formulas moved), so the content types and the
    // workbook rels change for it — and for nothing else
    assert_eq!(fate(&report, Fate::Removed), vec!["xl/calcChain.xml"]);
    assert!(fate(&report, Fate::Added).is_empty());
    identical_really(&src, &dst, &report);
    no_styles_anywhere(&dst);
    let s = text(&dst, "xl/worksheets/sheet1.xml");
    assert!(s.contains(r#"<c r="E3"><f>C3+D3</f><v>3</v></c>"#), "{s}");
    assert!(s.contains(r#"<c r="J3"><f>E3*10</f><v>30</v></c>"#), "{s}");
    assert!(!s.contains(" s=\""));
}

#[test]
fn a_fill_synthesizes_a_minimal_styles_part_with_its_rel_and_content_type() {
    let (_d, src, dst, report) = run(&[Edit::Fill {
        sheet: "Sheet1".into(),
        area: "C2:D2".into(),
        rgb: Some("B4D2F0".into()),
    }]);
    assert_eq!(fate(&report, Fate::Added), vec!["xl/styles.xml"]);
    assert_eq!(
        fate(&report, Fate::Rewritten),
        vec![
            "[Content_Types].xml",
            "xl/_rels/workbook.xml.rels",
            "xl/worksheets/sheet1.xml"
        ]
    );
    identical_really(&src, &dst, &report);
    let styles = text(&dst, "xl/styles.xml");
    let want = MINIMAL
        .replace(
            r#"<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>"#,
            r#"<fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFB4D2F0"/><bgColor indexed="64"/></patternFill></fill></fills>"#,
        )
        .replace(
            r#"<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>"#,
            r#"<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="0" fillId="2" borderId="0" xfId="0" applyFill="1"/></cellXfs>"#,
        );
    assert_eq!(styles, want);
    let rels = text(&dst, "xl/_rels/workbook.xml.rels");
    assert!(rels.ends_with(r#"<Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#), "{rels}");
    let before = text(&src, "xl/_rels/workbook.xml.rels");
    assert_eq!(
        rels.replace(r#"<Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#, ""),
        before
    );
    let ct = text(&dst, "[Content_Types].xml");
    assert_eq!(
        ct.replace(r#"<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#, ""),
        text(&src, "[Content_Types].xml")
    );
    assert!(ct.contains("/xl/styles.xml"));
    let s = text(&dst, "xl/worksheets/sheet1.xml");
    assert!(s.contains(r#"<c r="C2" s="1"><v>1</v></c><c r="D2" s="1"><v>2</v></c><c r="E2"><f>C2+D2</f><v>3</v></c>"#), "{s}");
    assert_eq!(
        differing(&src, &dst, "Sheet1"),
        ["C2", "D2"].iter().map(|s| s.to_string()).collect()
    );
}

#[test]
fn a_border_and_a_fill_share_one_synthesized_format() {
    let (_d, _src, dst, report) = run(&[
        Edit::Border {
            sheet: "Sheet1".into(),
            area: "J2".into(),
            rgb: Some("0070C0".into()),
            style: Some(LineStyle::Thin),
        },
        Edit::Fill {
            sheet: "Sheet1".into(),
            area: "J2".into(),
            rgb: Some("FAC88C".into()),
        },
        Edit::Value {
            sheet: "Sheet1".into(),
            cell: "J2".into(),
            value: serde_json::json!("ok"),
            expect: None,
        },
    ]);
    assert_eq!(fate(&report, Fate::Added), vec!["xl/styles.xml"]);
    let styles = text(&dst, "xl/styles.xml");
    assert!(styles.contains(r#"<borders count="2">"#), "{styles}");
    assert!(styles.contains(r#"<border><left style="thin"><color rgb="FF0070C0"/></left><right style="thin"><color rgb="FF0070C0"/></right><top style="thin"><color rgb="FF0070C0"/></top><bottom style="thin"><color rgb="FF0070C0"/></bottom><diagonal/></border></borders>"#), "{styles}");
    assert!(styles.contains(r#"<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="0" fillId="2" borderId="1" xfId="0" applyBorder="1" applyFill="1"/></cellXfs>"#), "{styles}");
    let s = text(&dst, "xl/worksheets/sheet1.xml");
    assert!(s.contains(r#"<c r="J2" s="1" t="inlineStr"><is><t>ok</t></is></c>"#), "{s}");
}
