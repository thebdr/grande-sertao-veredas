//! Byte-faithful edits inside a touched part (contract: Ábaco's patcher clause): a `<c>` element
//! changes only when an edit wrote it, a move rewrote its formula or position, or the edits moved
//! its value; every other cell comes out byte for byte as it went in, also where the engine would
//! compute something else than the file caches.
//!
//! The fixture has the shape a real plant workbook a test suite once carried had: formulas that
//! point at an external link with no `externalLink` part, at a sheet that does not exist, at a
//! function the engine lacks, a shared child whose master is missing, plus a volatile `RAND()`. The engine evaluates each of those to something other than the
//! cache (`#REF!`, `#ERROR!`, `#NAME?`, a new random number); before the fix
//! the patcher wrote those values into every such cell of a touched sheet.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use veredas_abaco::patch::cached::{read_elements, SheetElements};
use veredas_abaco::patch::{patch, Edit, PatchOptions, PatchReport, Status};

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn sheet_xml(rows: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="{NS}" xmlns:r="{NS_R}"><dimension ref="A1:J9"/><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetFormatPr defaultRowHeight="15"/><sheetData>{rows}</sheetData><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>"#
    )
}

/// CABINET: A1 is what the edits change; B1 and A8 depend on cells the edits
/// change; everything else the engine cannot reproduce from the file.
fn cabinet() -> String {
    sheet_xml(concat!(
        r#"<row r="1" spans="1:10">"#,
        r#"<c r="A1"><v>10</v></c>"#,
        r#"<c r="B1"><f>A1*2</f><v>20</v></c>"#,
        // a link with no externalLink part: the engine says #REF!
        r#"<c r="C1" s="1"><f>[1]DB_TEST!$C$23</f><v>61000</v></c>"#,
        // a sheet that does not exist
        r#"<c r="D1" t="str"><f>X!A1</f><v>=S1</v></c>"#,
        // a function the engine lacks, over the edited cell
        r#"<c r="E1"><f>MAPX(A1)</f><v>5</v></c>"#,
        // volatile
        r#"<c r="F1"><f>RAND()</f><v>0.123456789</v></c>"#,
        // a shared child whose master is not in the file
        r#"<c r="G1" t="str"><f t="shared" si="7"/><v>shared cached</v></c>"#,
        r#"<c r="H1" t="inlineStr"><is><t>untouched</t></is></c>"#,
        r#"</row>"#,
        r#"<row r="2"><c r="A2" t="str"><f>SUBSTITUTE([1]DB_TEST!$K$57&amp;T2,"%1",B1)</f><v>CABINET 20 - AL00</v></c><c r="T2" t="inlineStr"><is><t>AL00</t></is></c></row>"#,
        r#"<row r="5"><c r="A5"><v>1</v></c><c r="B5"><f>[1]DB_TEST!$C$24+A5</f><v>101</v></c></row>"#,
        r#"<row r="6"><c r="A6"><v>2</v></c></row>"#,
        r#"<row r="7"><c r="A7"><v>3</v></c></row>"#,
        r#"<row r="8"><c r="A8"><f>SUM(A5:A7)</f><v>6</v></c><c r="B8"><f>A8+[1]DB_TEST!$C$25</f><v>106</v></c></row>"#,
    ))
}

/// OTHER: one formula reads CABINET!A1 (it becomes a touched part through
/// it), the rest cannot resolve.
fn other() -> String {
    sheet_xml(concat!(
        r#"<row r="1"><c r="A1"><f>CABINET!A1+1</f><v>11</v></c><c r="B1" t="str"><f>[1]DB_TEST!$K$57</f><v>CABINET %1 - </v></c></row>"#,
        r#"<row r="2"><c r="A2"><f>NOSUCH(1)</f><v>42</v></c></row>"#,
    ))
}

fn package(dir: &Path) -> PathBuf {
    let ct = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#;
    let root = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="{NS}" xmlns:r="{NS_R}"><sheets><sheet name="CABINET" sheetId="1" r:id="rId1"/><sheet name="OTHER" sheetId="2" r:id="rId2"/></sheets><calcPr calcId="191029"/></workbook>"#
    );
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="{NS}"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="3" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
    );
    let path = dir.join("incoherent.xlsx");
    let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in [
        ("[Content_Types].xml", ct.to_string()),
        ("_rels/.rels", root.to_string()),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", wb_rels.to_string()),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", cabinet()),
        ("xl/worksheets/sheet2.xml", other()),
    ] {
        z.start_file(name, opts).unwrap();
        z.write_all(text.as_bytes()).unwrap();
    }
    z.finish().unwrap();
    path
}

fn run(edits: &[Edit]) -> (tempfile::TempDir, PathBuf, PathBuf, PatchReport) {
    let dir = tempfile::tempdir().unwrap();
    let src = package(dir.path());
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
    assert!(report.outcomes.iter().all(|o| o.status == Status::Applied));
    (dir, src, dst, report)
}

fn elements(path: &Path) -> BTreeMap<String, SheetElements> {
    read_elements(&std::fs::read(path).unwrap())
        .unwrap()
        .into_iter()
        .map(|(name, _, cells)| (name, cells))
        .collect()
}

fn at(cell: &str) -> (i32, i32) {
    let a = veredas_abaco::Addr::parse(cell).unwrap();
    (a.row, a.column)
}

/// Every `<c>` differing between source and patched, as `SHEET!A1`.
fn differing(src: &Path, dst: &Path) -> BTreeSet<String> {
    let (a, b) = (elements(src), elements(dst));
    let mut out = BTreeSet::new();
    for (name, ca) in &a {
        let cb = &b[name];
        let keys: BTreeSet<_> = ca.keys().chain(cb.keys()).collect();
        for k in keys {
            if ca.get(k) != cb.get(k) {
                out.insert(format!(
                    "{name}!{}{}",
                    veredas_abaco::a1::column_name(k.1),
                    k.0
                ));
            }
        }
    }
    out
}

fn set(cells: &[&str]) -> BTreeSet<String> {
    cells.iter().map(|s| s.to_string()).collect()
}

fn value(sheet: &str, cell: &str, v: serde_json::Value) -> Edit {
    Edit::Value {
        sheet: sheet.into(),
        cell: cell.into(),
        value: v,
        expect: None,
    }
}

#[test]
fn the_engine_reads_every_trap_differently_from_the_cache() {
    // the premise, measured: without the fix each of these was re-cached
    let dir = tempfile::tempdir().unwrap();
    let wb = veredas_abaco::Workbook::open(
        &package(dir.path()),
        veredas_abaco::links::LinkPolicy::AlwaysUseCached,
        |_| veredas_abaco::links::LinkChoice::UseCached,
    )
    .unwrap();
    let v = |s: u32, c: &str| {
        let a = veredas_abaco::Addr::parse(c).unwrap();
        wb.value_now(s, a).unwrap()
    };
    use veredas_abaco::Value;
    assert_eq!(v(0, "C1"), Value::Error("#REF!".into()));
    assert!(matches!(v(0, "D1"), Value::Error(_)), "{:?}", v(0, "D1"));
    assert_eq!(v(0, "E1"), Value::Error("#NAME?".into()));
    assert!(matches!(v(0, "F1"), Value::Number(n) if n != 0.123456789));
    assert_eq!(v(0, "A2"), Value::Error("#REF!".into()));
    assert_eq!(v(1, "B1"), Value::Error("#REF!".into()));
    assert_eq!(v(1, "A2"), Value::Error("#NAME?".into()));
}

#[test]
fn a_value_edit_changes_its_cell_and_the_values_it_moves_nothing_else() {
    let (_d, src, dst, report) = run(&[value("CABINET", "A1", serde_json::json!(20))]);
    // A1 edited; B1 = A1*2 and OTHER!A1 = CABINET!A1+1 moved; A2 reads B1
    // but also the missing link: #REF! before and after, so its cache stays
    assert_eq!(
        differing(&src, &dst),
        set(&["CABINET!A1", "CABINET!B1", "OTHER!A1"])
    );
    let (a, b) = (elements(&src), elements(&dst));
    assert_eq!(b["CABINET"][&at("A1")], r#"<c r="A1"><v>20</v></c>"#);
    assert_eq!(b["CABINET"][&at("B1")], r#"<c r="B1"><f>A1*2</f><v>40</v></c>"#);
    assert_eq!(b["OTHER"][&at("A1")], r#"<c r="A1"><f>CABINET!A1+1</f><v>21</v></c>"#);
    // and the traps, byte for byte
    for c in ["C1", "D1", "E1", "F1", "G1", "H1", "A2", "T2", "B5", "A8", "B8"] {
        assert_eq!(b["CABINET"][&at(c)], a["CABINET"][&at(c)], "CABINET!{c}");
    }
    for c in ["B1", "A2"] {
        assert_eq!(b["OTHER"][&at(c)], a["OTHER"][&at(c)], "OTHER!{c}");
    }
    assert_eq!(report.refreshed, 2);
}

#[test]
fn a_formula_edit_caches_the_engines_value_for_it_and_its_dependents_only() {
    let (_d, src, dst, report) = run(&[Edit::Formula {
        sheet: "CABINET".into(),
        cell: "A6".into(),
        formula: "=A5*10".into(),
        expect: None,
    }]);
    // A6: 2 → =A5*10 = 10; A8 = SUM(A5:A7) 6 → 14; B8 = A8+[1]… is #REF!
    // either way (cache kept)
    assert_eq!(differing(&src, &dst), set(&["CABINET!A6", "CABINET!A8"]));
    let b = elements(&dst);
    assert_eq!(b["CABINET"][&at("A6")], r#"<c r="A6"><f>A5*10</f><v>10</v></c>"#);
    assert_eq!(b["CABINET"][&at("A8")], r#"<c r="A8"><f>SUM(A5:A7)</f><v>14</v></c>"#);
    assert_eq!(report.refreshed, 1);
}

#[test]
fn inserted_rows_move_cells_without_touching_what_they_cache() {
    let (_d, src, dst, report) = run(&[Edit::InsertRows {
        sheet: "CABINET".into(),
        at: 1,
        count: 1,
    }]);
    let (a, b) = (elements(&src), elements(&dst));
    // every CABINET cell moved down one row: its element is the source's
    // with the new address and, where a reference moved, the new formula
    // text — the cached value always the file's (no value moved)
    let moved = |s: &str, from: &str, to: &str| s.replacen(&format!("r=\"{from}\""), &format!("r=\"{to}\""), 1);
    for (k, el) in &a["CABINET"] {
        let from = format!("{}{}", veredas_abaco::a1::column_name(k.1), k.0);
        let to = format!("{}{}", veredas_abaco::a1::column_name(k.1), k.0 + 1);
        let want = match from.as_str() {
            "B1" => r#"<c r="B2"><f>A2*2</f><v>20</v></c>"#.to_string(),
            "E1" => r#"<c r="E2"><f>MAPX(A2)</f><v>5</v></c>"#.to_string(),
            "A2" => r#"<c r="A3" t="str"><f>SUBSTITUTE([1]DB_TEST!$K$57&amp;T3,"%1",B2)</f><v>CABINET 20 - AL00</v></c>"#.to_string(),
            "B5" => r#"<c r="B6"><f>[1]DB_TEST!$C$24+A6</f><v>101</v></c>"#.to_string(),
            "A8" => r#"<c r="A9"><f>SUM(A6:A8)</f><v>6</v></c>"#.to_string(),
            "B8" => r#"<c r="B9"><f>A9+[1]DB_TEST!$C$25</f><v>106</v></c>"#.to_string(),
            _ => moved(el, &from, &to),
        };
        assert_eq!(b["CABINET"][&(k.0 + 1, k.1)], want, "CABINET!{from}");
    }
    assert_eq!(b["CABINET"].len(), a["CABINET"].len());
    // OTHER: its reference to CABINET!A1 rippled, value unchanged
    assert_eq!(b["OTHER"][&at("A1")], r#"<c r="A1"><f>CABINET!A2+1</f><v>11</v></c>"#);
    assert_eq!(b["OTHER"][&at("B1")], a["OTHER"][&at("B1")]);
    assert_eq!(b["OTHER"][&at("A2")], a["OTHER"][&at("A2")]);
    assert_eq!(report.refreshed, 0);
}

#[test]
fn deleted_rows_re_cache_what_the_deletion_moved_and_nothing_else() {
    let (_d, src, dst, report) = run(&[Edit::DeleteRows {
        sheet: "CABINET".into(),
        at: 6,
        count: 1,
    }]);
    let b = elements(&dst);
    let a = elements(&src);
    // SUM(A5:A7) → SUM(A5:A6) over 1 and 3: the value moved, 6 → 4
    assert_eq!(b["CABINET"][&at("A7")], r#"<c r="A7"><f>SUM(A5:A6)</f><v>4</v></c>"#);
    // B8 → B7: its formula moved with A8, its value is #REF! before and after
    assert_eq!(b["CABINET"][&at("B7")], r#"<c r="B7"><f>A7+[1]DB_TEST!$C$25</f><v>106</v></c>"#);
    assert_eq!(b["CABINET"][&at("A6")], r#"<c r="A6"><v>3</v></c>"#);
    // rows 1, 2 and 5 are above the deletion: untouched, byte for byte
    for (k, el) in a["CABINET"].iter().filter(|(k, _)| k.0 <= 5) {
        assert_eq!(&b["CABINET"][k], el, "CABINET R{}C{}", k.0, k.1);
    }
    assert_eq!(differing(&src, &dst), set(&["CABINET!A6", "CABINET!A7", "CABINET!A8", "CABINET!B7", "CABINET!B8"]));
    assert_eq!(report.refreshed, 1);
}

#[test]
fn a_package_with_nothing_to_do_comes_out_byte_identical() {
    let (_d, src, dst, report) = run(&[]);
    assert!(differing(&src, &dst).is_empty());
    assert_eq!(report.refreshed, 0);
    assert_eq!(
        report
            .parts
            .iter()
            .filter(|p| p.fate != veredas_abaco::patch::Fate::Identical)
            .count(),
        0,
        "{report:?}"
    );
}
