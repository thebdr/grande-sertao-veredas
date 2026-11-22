//! The patcher (contract: Ábaco's patcher clause) on the shapes a refute round found it got wrong,
//! each on a small hand-built package or a workbook among the fork's test files, each asserted on
//! the XML the patch writes: a shared group whose children read across a move, a chart on a chart
//! sheet, a table, a volatile function behind a defined name, the cells an array spilled into, an
//! external reference, a drawing and a threaded comment.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use veredas_abaco::links::{LinkChoice, LinkPolicy};
use veredas_abaco::patch::cached::{self, read_elements};
use veredas_abaco::patch::{Edit, PatchError, PatchOptions, Status, patch};
use veredas_abaco::{Addr, Value, Workbook};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn content_type(part: &str) -> Option<&'static str> {
    let types = [
        ("xl/workbook.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"),
        ("xl/worksheets/sheet", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"),
        ("xl/chartsheets/sheet", "application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml"),
        ("xl/drawings/drawing", "application/vnd.openxmlformats-officedocument.drawing+xml"),
        ("xl/charts/chart", "application/vnd.openxmlformats-officedocument.drawingml.chart+xml"),
        ("xl/tables/table", "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"),
        ("xl/threadedComments/", "application/vnd.ms-excel.threadedcomments+xml"),
        ("xl/persons/", "application/vnd.ms-excel.person+xml"),
    ];
    types.iter().find(|(p, _)| part.starts_with(p) && !part.contains("_rels")).map(|(_, t)| *t)
}

/// Writes a package of `parts` (the content types and the package relationship made here).
fn write_package(path: &Path, parts: &[(&str, String)]) {
    let mut ct = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#
    );
    for (name, _) in parts {
        if let Some(t) = content_type(name) {
            ct.push_str(&format!(r#"<Override PartName="/{name}" ContentType="{t}"/>"#));
        }
    }
    ct.push_str("</Types>");
    let root = rels(&[rel("rId1", "officeDocument", "xl/workbook.xml")]);
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in [("[Content_Types].xml", ct), ("_rels/.rels", root)].iter().map(|(n, t)| (*n, t.clone())).chain(parts.iter().map(|(n, t)| (*n, t.clone()))) {
        z.start_file(name, opts).unwrap();
        z.write_all(text.as_bytes()).unwrap();
    }
    z.finish().unwrap();
}

fn rel(id: &str, kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{id}" Type="{R}/{kind}" Target="{target}"/>"#)
}

fn rels(list: &[String]) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}</Relationships>"#,
        list.concat()
    )
}

/// `workbook.xml` with `sheets` (each `rId<n>` in order) and `names` inside `<definedNames>`.
fn workbook(sheets: &[&str], names: &str) -> String {
    let sheets: String =
        sheets.iter().enumerate().map(|(i, s)| format!(r#"<sheet name="{s}" sheetId="{}" r:id="rId{}"/>"#, i + 1, i + 1)).collect();
    let names = if names.is_empty() { String::new() } else { format!("<definedNames>{names}</definedNames>") };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="{MAIN}" xmlns:r="{R}"><sheets>{sheets}</sheets>{names}<calcPr calcId="191029"/></workbook>"#
    )
}

/// A worksheet holding `data` in `<sheetData>`, `after` behind it.
fn worksheet(data: &str, after: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="{MAIN}" xmlns:r="{R}"><sheetData>{data}</sheetData>{after}</worksheet>"#
    )
}

/// One worksheet per name, the workbook's relationships pointing at `xl/worksheets/sheet<n>.xml`.
fn plain_book(path: &Path, sheets: &[(&str, String)], names: &str, extra: &[(&str, String)]) {
    let names_list: Vec<&str> = sheets.iter().map(|(n, _)| *n).collect();
    let wb_rels: Vec<String> =
        (1..=sheets.len()).map(|i| rel(&format!("rId{i}"), "worksheet", &format!("worksheets/sheet{i}.xml"))).collect();
    let sheet_parts: Vec<(String, String)> =
        sheets.iter().enumerate().map(|(i, (_, x))| (format!("xl/worksheets/sheet{}.xml", i + 1), x.clone())).collect();
    let mut parts: Vec<(&str, String)> = vec![("xl/workbook.xml", workbook(&names_list, names)), ("xl/_rels/workbook.xml.rels", rels(&wb_rels))];
    parts.extend(sheet_parts.iter().map(|(n, x)| (n.as_str(), x.clone())));
    parts.extend(extra.iter().map(|(n, x)| (*n, x.clone())));
    write_package(path, &parts);
}

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

fn part(path: &Path, name: &str) -> String {
    String::from_utf8(parts(path).remove(name).unwrap_or_else(|| panic!("no part {name}"))).unwrap()
}

fn verified() -> PatchOptions {
    PatchOptions { verify: true, ..Default::default() }
}

fn rows(sheet: &str, at: i32, count: i32) -> Edit {
    Edit::InsertRows { sheet: sheet.into(), at, count }
}

/// The formula and the value of a cell of the written file, as it stores them.
fn stored(path: &Path, sheet: &str, cell: &str) -> (Option<String>, bool, Value) {
    let a = Addr::parse(cell).unwrap();
    let c = cached::read(path).unwrap().remove(&(sheet.to_string(), a.row, a.column)).unwrap_or_else(|| panic!("no {sheet}!{cell}"));
    (c.formula, c.shared, c.value)
}

/// The engine's value of a cell of a file, read back.
fn read_back(path: &Path, sheet: &str, cell: &str) -> Value {
    let wb = Workbook::open(path, LinkPolicy::AlwaysUseCached, |_| LinkChoice::UseCached).unwrap();
    wb.value_now(wb.sheet_index(sheet).unwrap(), Addr::parse(cell).unwrap()).unwrap()
}

fn fork_file(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../forks/ironcalc/xlsx/tests").join(rel)
}

#[test]
fn a_shared_group_whose_children_read_below_an_insertion_is_expanded_on_its_own_sheet() {
    // B1:B3 is `=A10` shared down: B2 reads A11, B3 reads A12; an insertion at row 11 moves those
    // two references and not the master's
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("shared.xlsx");
    let mut data = String::new();
    for r in 1..=12 {
        let b = match r {
            1 => r#"<c r="B1"><f t="shared" ref="B1:B3" si="0">A10</f><v>10</v></c>"#.to_string(),
            2 | 3 => format!(r#"<c r="B{r}"><f t="shared" si="0"/><v>{}</v></c>"#, 9 + r),
            _ => String::new(),
        };
        data.push_str(&format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c>{b}</row>"#));
    }
    plain_book(&src, &[("Data", worksheet(&data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let report = patch(&src, &dst, &[rows("Data", 11, 1)], &verified()).unwrap();
    assert_eq!(report.expanded, 1);
    assert_eq!(stored(&dst, "Data", "B1"), (Some("A10".into()), false, Value::Number(10.0)));
    assert_eq!(stored(&dst, "Data", "B2"), (Some("A12".into()), false, Value::Number(11.0)));
    assert_eq!(stored(&dst, "Data", "B3"), (Some("A13".into()), false, Value::Number(12.0)));
    assert!(report.verified_cells > 0);
}

#[test]
fn a_shared_group_reading_another_sheet_follows_an_insertion_there() {
    // Sum!A1:A4 is `=Data!A1` shared down; rows inserted at Data's row 3 move what A3 and A4 read
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("shared.xlsx");
    let data: String = (1..=6).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let sum: String = (1..=4)
        .map(|r| {
            let f = if r == 1 { r#"<f t="shared" ref="A1:A4" si="0">Data!A1</f>"#.to_string() } else { r#"<f t="shared" si="0"/>"#.to_string() };
            format!(r#"<row r="{r}"><c r="A{r}">{f}<v>{r}</v></c></row>"#)
        })
        .collect();
    plain_book(&src, &[("Data", worksheet(&data, "")), ("Sum", worksheet(&sum, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[rows("Data", 3, 1)], &verified()).unwrap();
    for (cell, reads, v) in [("A1", "Data!A1", 1.0), ("A2", "Data!A2", 2.0), ("A3", "Data!A4", 3.0), ("A4", "Data!A5", 4.0)] {
        assert_eq!(stored(&dst, "Sum", cell), (Some(reads.into()), false, Value::Number(v)), "Sum!{cell}");
        assert_eq!(read_back(&dst, "Sum", cell), Value::Number(v), "Sum!{cell} read back");
    }
}

#[test]
fn a_chart_on_a_chart_sheet_follows_the_rows_it_reads_and_the_chart_sheet_counts_for_local_names() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("chart.xlsx");
    let data: String = (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let chartsheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><chartsheet xmlns="{MAIN}" xmlns:r="{R}"><sheetViews><sheetView workbookViewId="0"/></sheetViews><drawing r:id="rId1"/></chartsheet>"#
    );
    let drawing = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:absoluteAnchor><xdr:pos x="0" y="0"/><xdr:ext cx="100" cy="100"/><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="{R}" r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:absoluteAnchor></xdr:wsDr>"#
    );
    let chart = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:idx val="0"/><c:val><c:numRef><c:f>Data!$A$1:$A$3</c:f></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
    let wb_rels = rels(&[
        rel("rId1", "worksheet", "worksheets/sheet1.xml"),
        rel("rId2", "chartsheet", "chartsheets/sheet1.xml"),
        rel("rId3", "worksheet", "worksheets/sheet2.xml"),
    ]);
    write_package(
        &src,
        &[
            ("xl/workbook.xml", workbook(&["Data", "Chart1", "Second"], "")),
            ("xl/_rels/workbook.xml.rels", wb_rels),
            ("xl/worksheets/sheet1.xml", worksheet(&data, "")),
            ("xl/chartsheets/sheet1.xml", chartsheet),
            ("xl/chartsheets/_rels/sheet1.xml.rels", rels(&[rel("rId1", "drawing", "../drawings/drawing1.xml")])),
            ("xl/drawings/drawing1.xml", drawing),
            ("xl/drawings/_rels/drawing1.xml.rels", rels(&[rel("rId1", "chart", "../charts/chart1.xml")])),
            ("xl/charts/chart1.xml", chart.to_string()),
            ("xl/worksheets/sheet2.xml", worksheet("", "")),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    let local = Edit::SetName { name: "Local".into(), formula: "=Second!$A$1".into(), scope: Some("Second".into()) };
    patch(&src, &dst, &[rows("Data", 1, 2), local], &verified()).unwrap();
    assert!(part(&dst, "xl/charts/chart1.xml").contains("<c:f>Data!$A$3:$A$5</c:f>"));
    let (before, after) = (parts(&src), parts(&dst));
    for p in ["xl/chartsheets/sheet1.xml", "xl/drawings/drawing1.xml", "xl/chartsheets/_rels/sheet1.xml.rels"] {
        assert_eq!(before[p], after[p], "{p}");
    }
    // the third `<sheet>` of the file, the chart sheet counted
    assert!(part(&dst, "xl/workbook.xml").contains(r#"<definedName name="Local" localSheetId="2">Second!$A$1</definedName>"#));
}

#[test]
fn a_name_scoped_to_a_sheet_after_a_chart_sheet_counts_the_chart_sheet() {
    // example.xlsx: Sheet1, Chart1 (a chart sheet), Second
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("out.xlsx");
    let local = Edit::SetName { name: "Local".into(), formula: "=Second!$A$1".into(), scope: Some("Second".into()) };
    patch(&fork_file("example.xlsx"), &dst, &[local], &verified()).unwrap();
    let names = part(&dst, "xl/workbook.xml");
    let at = names.find(r#"<definedName name="Local""#).unwrap();
    assert!(names[at..].starts_with(r#"<definedName name="Local" localSheetId="2">"#), "{}", &names[at..at + 80]);
}

/// Sheet `T`: a table at A2:B4 (`Name`, `Qty`; 3 and 4), D1 summing its `Qty`.
fn table_book(path: &Path) {
    let text = |c: &str, t: &str| format!(r#"<c r="{c}" t="inlineStr"><is><t>{t}</t></is></c>"#);
    let data = format!(
        r#"<row r="1"><c r="D1"><f>SUM(Table1[Qty])</f><v>7</v></c></row><row r="2">{}{}</row><row r="3">{}<c r="B3"><v>3</v></c></row><row r="4">{}<c r="B4"><v>4</v></c></row>"#,
        text("A2", "Name"),
        text("B2", "Qty"),
        text("A3", "a"),
        text("A4", "b")
    );
    let table = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><table xmlns="{MAIN}" id="1" name="Table1" displayName="Table1" ref="A2:B4" totalsRowShown="0"><autoFilter ref="A2:B4"/><tableColumns count="2"><tableColumn id="1" name="Name"/><tableColumn id="2" name="Qty"/></tableColumns><tableStyleInfo name="TableStyleMedium2" showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/></table>"#
    );
    plain_book(
        path,
        &[("T", worksheet(&data, r#"<tableParts count="1"><tablePart r:id="rId1"/></tableParts>"#))],
        "",
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", rels(&[rel("rId1", "table", "../tables/table1.xml")])),
            ("xl/tables/table1.xml", table),
        ],
    );
}

#[test]
fn a_move_that_cuts_into_a_table_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("table.xlsx");
    table_book(&src);
    for edit in [
        rows("T", 3, 1),
        Edit::DeleteRows { sheet: "T".into(), at: 4, count: 1 },
        Edit::InsertColumns { sheet: "T".into(), at: "B".into(), count: 1 },
        Edit::DeleteColumns { sheet: "T".into(), at: "A".into(), count: 1 },
    ] {
        let dst = dir.path().join("out.xlsx");
        let e = patch(&src, &dst, &[edit.clone()], &verified()).unwrap_err();
        assert!(matches!(e, PatchError::Unsupported(_)), "{edit:?}: {e}");
        assert!(e.to_string().contains("cuts into table Table1 at T!A2:B4"), "{e}");
        assert!(!dst.exists());
    }
}

#[test]
fn a_table_moved_whole_is_read_where_it_is_by_a_formula_written_after_the_move() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("table.xlsx");
    table_book(&src);
    let dst = dir.path().join("out.xlsx");
    let sum = Edit::Formula { sheet: "T".into(), cell: "H1".into(), formula: "=SUM(Table1[Qty])".into(), expect: None };
    patch(&src, &dst, &[rows("T", 1, 1), sum], &verified()).unwrap();
    let table = part(&dst, "xl/tables/table1.xml");
    assert!(table.contains(r#"ref="A3:B5""#) && table.contains(r#"<autoFilter ref="A3:B5"/>"#), "{table}");
    // the moved formula keeps its structured reference; the written one reads the moved rows (the
    // engine parses a structured reference to the cells it names, and the patcher writes the
    // engine's parse)
    assert_eq!(stored(&dst, "T", "D2"), (Some("SUM(Table1[Qty])".into()), false, Value::Number(7.0)));
    assert_eq!(stored(&dst, "T", "H1"), (Some("SUM($B$4:$B$5)".into()), false, Value::Number(7.0)));
}

#[test]
fn a_volatile_function_behind_a_name_is_never_counted_as_moved() {
    // R is RAND(); B1 reads R; an edit on the same sheet must leave B1's cache as the file has it
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("volatile.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>R</f><v>0.5</v></c><c r="C1"><f>A1*2</f><v>2</v></c><c r="D1"><f>B1*2</f><v>1</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], r#"<definedName name="Twice">R*2</definedName><definedName name="R">RAND()</definedName>"#, &[]);
    let dst = dir.path().join("out.xlsx");
    let edit = Edit::Value { sheet: "S".into(), cell: "A1".into(), value: serde_json::json!(5), expect: None };
    let report = patch(&src, &dst, &[edit], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
    let (before, after) = (read_elements(&std::fs::read(&src).unwrap()).unwrap(), read_elements(&std::fs::read(&dst).unwrap()).unwrap());
    let (b, a) = (&before[0].2, &after[0].2);
    // B1 (the name) and D1 (what reads B1) as they were; C1 moved with A1
    assert_eq!(b[&(1, 2)], a[&(1, 2)]);
    assert_eq!(b[&(1, 4)], a[&(1, 4)]);
    assert_eq!(a[&(1, 3)], r#"<c r="C1"><f>A1*2</f><v>10</v></c>"#);
    assert_eq!(report.refreshed, 1);
}

#[test]
fn the_cells_an_array_spilled_into_go_with_its_anchor() {
    // MMULT.xlsx: E2 spills MMULT over E2:F3; deleting row 2 deletes the anchor, and the cells it
    // spilled into, now at row 2, must not stay behind as constants
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("out.xlsx");
    let src = fork_file("calc_tests/MATH_AND_TRIGONOMETRY/MMULT.xlsx");
    let report = patch(&src, &dst, &[Edit::DeleteRows { sheet: "Sheet1".into(), at: 2, count: 1 }], &verified()).unwrap();
    assert!(report.verified_cells > 0);
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<c r="E2" s="10"/>"#) && sheet.contains(r#"<c r="F2" s="9"/>"#), "{}", &sheet[sheet.find(r#"<row r="2""#).unwrap()..][..400]);
    assert_eq!(read_back(&dst, "Sheet1", "E2"), Value::Empty);
    // the next array, E4:F5, moved up whole with its anchor
    assert!(sheet.contains(r#"<f t="array" ref="E3:F4">MMULT(A3:B4,C3:D4)</f>"#));
}

#[test]
fn an_external_reference_keeps_its_address_when_its_cell_moves() {
    // link_test.xlsx: Sheet1!B21 reads [1]Sheet1!A1, a cell of another workbook nothing moved
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("out.xlsx");
    let src = fork_file("link_test.xlsx");
    assert_eq!(stored(&src, "Sheet1", "B21").0.as_deref(), Some("[1]Sheet1!A1"));
    for (edit, cell) in [(rows("Sheet1", 2, 1), "B22"), (Edit::DeleteRows { sheet: "Sheet1".into(), at: 2, count: 1 }, "B20")] {
        let _ = std::fs::remove_file(&dst);
        patch(&src, &dst, &[edit], &verified()).unwrap();
        assert_eq!(stored(&dst, "Sheet1", cell).0.as_deref(), Some("[1]Sheet1!A1"), "{cell}");
    }
}

#[test]
fn drawings_and_threaded_comments_follow_their_cells() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("drawn.xlsx");
    let data: String = (1..=6).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let drawing = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:twoCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>4</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>5</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:sp macro="" textlink=""><xdr:nvSpPr><xdr:cNvPr id="2" name="Box"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr/></xdr:sp><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#;
    let threaded = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><ThreadedComments xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments"><threadedComment ref="A5" dT="2026-11-22T10:00:00.00" personId="{00000000-0000-0000-0000-000000000001}" id="{00000000-0000-0000-0000-000000000002}"><text>check</text></threadedComment></ThreadedComments>"#;
    plain_book(
        &src,
        &[("S", worksheet(&data, r#"<drawing r:id="rId1"/>"#))],
        "",
        &[
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                rels(&[
                    rel("rId1", "drawing", "../drawings/drawing1.xml"),
                    format!(r#"<Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2017/10/relationships/threadedComment" Target="../threadedComments/threadedComment1.xml"/>"#),
                ]),
            ),
            ("xl/drawings/drawing1.xml", drawing.to_string()),
            ("xl/threadedComments/threadedComment1.xml", threaded.to_string()),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[rows("S", 2, 2)], &verified()).unwrap();
    let d = part(&dst, "xl/drawings/drawing1.xml");
    assert!(d.contains("<xdr:row>6</xdr:row>") && d.contains("<xdr:row>7</xdr:row>"), "{d}");
    assert!(part(&dst, "xl/threadedComments/threadedComment1.xml").contains(r#"<threadedComment ref="A7""#));
}
