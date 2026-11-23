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
use veredas_abaco::patch::{Edit, Expect, PatchError, PatchOptions, Status, patch};
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

/// Sheets Data (1, 2, 3 in A1:A3), Chart1 (a chart sheet: a chart of Data!$A$1:$A$3 and a shape
/// linked to Data!$A$3) and Second (empty), the defined names `names`.
fn chart_sheet_book(src: &Path, names: &str) {
    let data: String = (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let chartsheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><chartsheet xmlns="{MAIN}" xmlns:r="{R}"><sheetViews><sheetView workbookViewId="0"/></sheetViews><drawing r:id="rId1"/></chartsheet>"#
    );
    let drawing = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:absoluteAnchor><xdr:pos x="0" y="0"/><xdr:ext cx="100" cy="100"/><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="{R}" r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:absoluteAnchor><xdr:absoluteAnchor><xdr:pos x="0" y="0"/><xdr:ext cx="10" cy="10"/><xdr:sp macro="" textlink="Data!$A$3"><xdr:nvSpPr><xdr:cNvPr id="3" name="Note"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr/></xdr:sp><xdr:clientData/></xdr:absoluteAnchor></xdr:wsDr>"#
    );
    let chart = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:idx val="0"/><c:val><c:numRef><c:f>Data!$A$1:$A$3</c:f></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
    let wb_rels = rels(&[
        rel("rId1", "worksheet", "worksheets/sheet1.xml"),
        rel("rId2", "chartsheet", "chartsheets/sheet1.xml"),
        rel("rId3", "worksheet", "worksheets/sheet2.xml"),
    ]);
    write_package(
        src,
        &[
            ("xl/workbook.xml", workbook(&["Data", "Chart1", "Second"], names)),
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
}

#[test]
fn a_chart_on_a_chart_sheet_follows_the_rows_it_reads_and_the_chart_sheet_counts_for_local_names() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("chart.xlsx");
    chart_sheet_book(&src, "");
    let dst = dir.path().join("out.xlsx");
    let local = Edit::SetName { name: "Local".into(), formula: "=Second!$A$1".into(), scope: Some("Second".into()) };
    patch(&src, &dst, &[rows("Data", 1, 2), local], &verified()).unwrap();
    assert!(part(&dst, "xl/charts/chart1.xml").contains("<c:f>Data!$A$3:$A$5</c:f>"));
    let (before, after) = (parts(&src), parts(&dst));
    for p in ["xl/chartsheets/sheet1.xml", "xl/chartsheets/_rels/sheet1.xml.rels"] {
        assert_eq!(before[p], after[p], "{p}");
    }
    // the chart sheet's drawing: only its shape's text link moved
    assert!(part(&dst, "xl/drawings/drawing1.xml").contains(r#"textlink="Data!$A$5""#));
    // the third `<sheet>` of the file, the chart sheet counted
    assert!(part(&dst, "xl/workbook.xml").contains(r#"<definedName name="Local" localSheetId="2">Second!$A$1</definedName>"#));
}

#[test]
fn a_name_local_to_a_chart_sheet_keeps_its_scope_and_moves_with_its_cells() {
    // the engine holds no chart sheet, so it cannot scope `OnChart`: the name is the file's alone
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("chart.xlsx");
    let on_chart = r#"<definedName name="OnChart" localSheetId="1">Data!$A$2</definedName>"#;
    chart_sheet_book(&src, &format!(r#"{on_chart}<definedName name="Rate">Data!$A$1</definedName>"#));
    let dst = dir.path().join("out.xlsx");
    let book = |p: &Path| part(p, "xl/workbook.xml");
    // another name set, then deleted: OnChart byte for byte, where it was
    let set = Edit::SetName { name: "Other".into(), formula: "=Second!$A$1".into(), scope: None };
    patch(&src, &dst, &[set], &verified()).unwrap();
    assert!(
        book(&dst).contains(&format!(r#"<definedNames>{on_chart}<definedName name="Rate">Data!$A$1</definedName><definedName name="Other">Second!$A$1</definedName></definedNames>"#)),
        "{}",
        book(&dst)
    );
    let _ = std::fs::remove_file(&dst);
    patch(&src, &dst, &[Edit::DeleteName { name: "Rate".into(), scope: None }], &verified()).unwrap();
    assert!(book(&dst).contains(&format!("<definedNames>{on_chart}</definedNames>")), "{}", book(&dst));
    // a row inserted above its cell: applied, the name moved, its scope kept
    let _ = std::fs::remove_file(&dst);
    patch(&src, &dst, &[rows("Data", 1, 1)], &verified()).unwrap();
    assert!(
        book(&dst).contains(r#"<definedNames><definedName name="OnChart" localSheetId="1">Data!$A$3</definedName><definedName name="Rate">Data!$A$2</definedName></definedNames>"#),
        "{}",
        book(&dst)
    );
    // a name of that scope is one an edit cannot reach
    let _ = std::fs::remove_file(&dst);
    for edit in [
        Edit::SetName { name: "OnChart".into(), formula: "=Data!$A$1".into(), scope: Some("Chart1".into()) },
        Edit::DeleteName { name: "OnChart".into(), scope: Some("Chart1".into()) },
    ] {
        assert!(patch(&src, &dst, &[edit.clone()], &verified()).is_err(), "{edit:?}");
        assert!(!dst.exists());
    }
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
    let drawing = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:twoCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>4</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>5</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:sp macro="" textlink="$A$5"><xdr:nvSpPr><xdr:cNvPr id="2" name="Box"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr/></xdr:sp><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#;
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
    // the shape's text read from a cell follows it
    assert!(d.contains(r#"textlink="$A$7""#), "{d}");
    assert!(part(&dst, "xl/threadedComments/threadedComment1.xml").contains(r#"<threadedComment ref="A7""#));
}

// ---- refute round 2 ---------------------------------------------------------------------------

fn value(sheet: &str, cell: &str, v: serde_json::Value) -> Edit {
    Edit::Value { sheet: sheet.into(), cell: cell.into(), value: v, expect: None }
}

/// The `<c>` element of a cell of a written sheet part, as it stands.
fn element<'a>(sheet: &'a str, cell: &str) -> Option<&'a str> {
    let at = sheet.find(&format!(r#"<c r="{cell}""#))?;
    let end = sheet[at..].find("</c>").map(|e| at + e + 4);
    let short = sheet[at..].find("/>").map(|e| at + e + 2);
    let close = sheet[at..].find('>').map(|e| at + e + 1)?;
    // a self-closing `<c …/>` ends at its first `>`
    if short == Some(close) { Some(&sheet[at..close]) } else { end.map(|e| &sheet[at..e]) }
}

#[test]
fn zero_and_empty_are_different_values() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("zero.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>0</v></c><c r="B1"><f>COUNT(A1)</f><v>1</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    // a 0 into an empty cell is written
    let report = patch(&src, &dst, &[value("S", "A2", serde_json::json!(0))], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
    assert_eq!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "A2"), Some(r#"<c r="A2"><v>0</v></c>"#));
    // a 0 cleared is gone, and what counted it follows
    std::fs::remove_file(&dst).unwrap();
    let report = patch(&src, &dst, &[value("S", "A1", serde_json::Value::Null)], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "A1"), None, "{sheet}");
    assert_eq!(element(&sheet, "B1"), Some(r#"<c r="B1"><f>COUNT(A1)</f><v>0</v></c>"#));
    // expecting an empty cell where a 0 is: a conflict
    std::fs::remove_file(&dst).unwrap();
    let expecting = Edit::Value {
        sheet: "S".into(),
        cell: "A1".into(),
        value: serde_json::json!(5),
        expect: Some(veredas_abaco::patch::Expect::Value { value: serde_json::Value::Null }),
    };
    let report = patch(&src, &dst, &[expecting], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Conflict);
    assert_eq!(report.outcomes[0].note, "S!A1 now holds 0");
}

#[test]
fn a_volatile_name_of_one_sheet_does_not_freeze_a_formula_of_another() {
    // the workbook's R reads Data!A1; Other's own R is RAND(); Data!B1 reads the workbook's R
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("scoped.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>R*2</f><v>2</v></c></row>"#;
    let other = r#"<row r="1"><c r="A1"><f>R</f><v>0.5</v></c></row>"#;
    let names = r#"<definedName name="R">Data!$A$1</definedName><definedName name="R" localSheetId="1">RAND()</definedName>"#;
    plain_book(&src, &[("Data", worksheet(data, "")), ("Other", worksheet(other, ""))], names, &[]);
    let dst = dir.path().join("out.xlsx");
    let report = patch(&src, &dst, &[value("Data", "A1", serde_json::json!(5))], &verified()).unwrap();
    assert_eq!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "B1"), Some(r#"<c r="B1"><f>R*2</f><v>10</v></c>"#));
    assert_eq!(report.refreshed, 1);
    // Other's own R stays as the file has it
    assert_eq!(parts(&src)["xl/worksheets/sheet2.xml"], parts(&dst)["xl/worksheets/sheet2.xml"]);
}

/// Sheet `S`: B1 a count, A1 a dynamic array of that many numbers counting down to 0.
fn sequence_book(path: &Path) {
    let data = r#"<row r="1"><c r="A1" cm="1"><f t="array" ref="A1:A3">_xlfn.SEQUENCE(B1,1,B1-1,-1)</f><v>2</v></c><c r="B1"><v>3</v></c></row><row r="2"><c r="A2"><v>1</v></c></row><row r="3"><c r="A3"><v>0</v></c></row>"#;
    plain_book(path, &[("S", worksheet(data, ""))], "", &[]);
}

#[test]
fn an_array_that_shrinks_leaves_no_constant_behind_and_its_ref_follows() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("seq.xlsx");
    sequence_book(&src);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[value("S", "B1", serde_json::json!(2))], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "A1"), Some(r#"<c r="A1" cm="1"><f t="array" ref="A1:A2">_xlfn.SEQUENCE(B1,1,B1-1,-1)</f><v>1</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "A2"), Some(r#"<c r="A2"><v>0</v></c>"#));
    assert_eq!(element(&sheet, "A3"), None, "{sheet}");
    assert_eq!(read_back(&dst, "S", "A3"), Value::Empty);
}

#[test]
fn an_array_that_grows_is_written_where_it_now_spills() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("seq.xlsx");
    sequence_book(&src);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[value("S", "B1", serde_json::json!(5))], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "A1"), Some(r#"<c r="A1" cm="1"><f t="array" ref="A1:A5">_xlfn.SEQUENCE(B1,1,B1-1,-1)</f><v>4</v></c>"#), "{sheet}");
    for (cell, v) in [("A2", "3"), ("A3", "2"), ("A4", "1"), ("A5", "0")] {
        assert_eq!(element(&sheet, cell), Some(format!(r#"<c r="{cell}"><v>{v}</v></c>"#).as_str()), "{sheet}");
    }
}

#[test]
fn an_array_whose_anchor_row_is_deleted_leaves_no_constant_behind() {
    // SEQUENCE(3,1,1,-1) spills 1, 0, -1 down A1:A3; row 1 deleted takes the anchor
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("anchor.xlsx");
    let data = r#"<row r="1"><c r="A1" cm="1"><f t="array" ref="A1:A3">_xlfn.SEQUENCE(3,1,1,-1)</f><v>1</v></c></row><row r="2"><c r="A2"><v>0</v></c></row><row r="3"><c r="A3"><v>-1</v></c></row><row r="4"><c r="A4"><v>7</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[Edit::DeleteRows { sheet: "S".into(), at: 1, count: 1 }], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "A1"), None, "{sheet}");
    assert_eq!(element(&sheet, "A2"), None, "{sheet}");
    assert_eq!(element(&sheet, "A3"), Some(r#"<c r="A3"><v>7</v></c>"#), "{sheet}");
    assert_eq!(read_back(&dst, "S", "A1"), Value::Empty);
}

#[test]
fn a_filter_that_grows_gets_its_new_cell_and_a_value_that_is_no_error_loses_its_error_mark() {
    // FILTER_ERRORS.xlsx: C1 filters A1:A4 above 3 (4 alone), E1 above 10 (none: #CALC!, marked
    // by `vm`); A1 set to 20 makes C1 spill 20 and 4, E1 return 20
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("out.xlsx");
    let src = fork_file("calc_tests/LOOKUP_AND_REFERENCE/FILTER_ERRORS.xlsx");
    patch(&src, &dst, &[value("Sheet1", "A1", serde_json::json!(20))], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    let c1 = element(&sheet, "C1").unwrap();
    assert!(c1.contains(r#"ref="C1:C2""#) && c1.ends_with("<v>20</v></c>"), "{c1}");
    assert_eq!(element(&sheet, "C2"), Some(r#"<c r="C2"><v>4</v></c>"#), "{sheet}");
    let e1 = element(&sheet, "E1").unwrap();
    assert!(!e1.contains("vm=") && !e1.contains(r#"t="e""#) && e1.ends_with("<v>20</v></c>"), "{e1}");
}

#[test]
fn a_format_or_a_validation_that_loses_its_first_row_keeps_reading_its_own_cells() {
    // A2:A10: a format `A2>5` and a validation `COUNTIF($A$2:$A$10,A2)=1`, both relative to A2
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("formats.xlsx");
    let data: String = (1..=10).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let after = r#"<conditionalFormatting sqref="A2:A10"><cfRule type="expression" dxfId="0" priority="1"><formula>A2&gt;5</formula></cfRule></conditionalFormatting><dataValidations count="1"><dataValidation type="custom" allowBlank="1" showErrorMessage="1" sqref="A2:A10"><formula1>COUNTIF($A$2:$A$10,A2)=1</formula1></dataValidation></dataValidations>"#;
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="{MAIN}"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles><dxfs count="1"><dxf><font><b/></font></dxf></dxfs></styleSheet>"#
    );
    write_package(
        &src,
        &[
            ("xl/workbook.xml", workbook(&["S"], "")),
            ("xl/_rels/workbook.xml.rels", rels(&[rel("rId1", "worksheet", "worksheets/sheet1.xml"), rel("rId2", "styles", "styles.xml")])),
            ("xl/worksheets/sheet1.xml", worksheet(&data, after)),
            ("xl/styles.xml", styles),
        ],
    );
    // the first row deleted: re-anchored on the old A3, now A2
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[Edit::DeleteRows { sheet: "S".into(), at: 2, count: 1 }], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<conditionalFormatting sqref="A2:A9"><cfRule type="expression" dxfId="0" priority="1"><formula>A2&gt;5</formula>"#), "{sheet}");
    assert!(sheet.contains(r#"sqref="A2:A9"><formula1>COUNTIF($A$2:$A$9,A2)=1</formula1>"#), "{sheet}");
    // rows inserted above: the range and its formulas move together
    std::fs::remove_file(&dst).unwrap();
    patch(&src, &dst, &[rows("S", 1, 2)], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<conditionalFormatting sqref="A4:A12"><cfRule type="expression" dxfId="0" priority="1"><formula>A4&gt;5</formula>"#), "{sheet}");
    assert!(sheet.contains(r#"sqref="A4:A12"><formula1>COUNTIF($A$4:$A$12,A4)=1</formula1>"#), "{sheet}");
}

#[test]
fn random_values_and_what_reads_them_keep_their_caches_on_every_run() {
    // Coin is a LAMBDA drawing 1 or 2, called by name; C1 draws itself; D1 reads C1. An edit
    // elsewhere leaves all three as the file has them, every time, and is never refused
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("coin.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>Coin()</f><v>1</v></c><c r="C1"><f>_xlfn.RANDBETWEEN(1,2)</f><v>2</v></c><c r="D1"><f>C1*10</f><v>20</v></c><c r="E1"><f>A1*2</f><v>2</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], r#"<definedName name="Coin">_xlfn.LAMBDA(_xlfn.RANDBETWEEN(1,2))</definedName>"#, &[]);
    let before = read_elements(&std::fs::read(&src).unwrap()).unwrap();
    for run in 0..20 {
        let dst = dir.path().join(format!("out{run}.xlsx"));
        let report = patch(&src, &dst, &[value("S", "A1", serde_json::json!(5))], &verified()).unwrap_or_else(|e| panic!("run {run}: {e}"));
        let after = read_elements(&std::fs::read(&dst).unwrap()).unwrap();
        for col in [2, 3, 4] {
            assert_eq!(before[0].2[&(1, col)], after[0].2[&(1, col)], "run {run}, column {col}");
        }
        assert_eq!(after[0].2[&(1, 5)], r#"<c r="E1"><f>A1*2</f><v>10</v></c>"#);
        assert_eq!(report.refreshed, 1, "run {run}");
    }
    // and the random functions are back to chance once the patch is done
    assert!(!ironcalc_base::random_is_seeded());
    assert_eq!(ironcalc_base::pinned_clock(), None);
}

#[test]
fn verification_is_on_by_default() {
    assert!(PatchOptions::default().verify);
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("zero.xlsx");
    plain_book(&src, &[("S", worksheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let report = patch(&src, &dst, &[value("S", "A1", serde_json::json!(2))], &PatchOptions::default()).unwrap();
    assert!(report.verified_cells > 0);
}

#[test]
fn a_text_reads_back_as_it_was_written_carriage_returns_and_control_characters_included() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("text.xlsx");
    plain_book(&src, &[("S", worksheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let texts = ["line1\r\nline2\rend", "a\u{1}b", "_x0041_ stays", "tab\there"];
    let edits: Vec<Edit> =
        texts.iter().enumerate().map(|(i, t)| value("S", &format!("B{}", i + 1), serde_json::json!(t))).collect();
    patch(&src, &dst, &edits, &verified()).unwrap();
    for (i, t) in texts.iter().enumerate() {
        assert_eq!(read_back(&dst, "S", &format!("B{}", i + 1)), Value::Text(t.to_string()), "{t:?}");
    }
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<t xml:space=\"preserve\">line1_x000D_\nline2_x000D_end</t>"), "{sheet}");
    assert!(sheet.contains("<t>_x005F_x0041_ stays</t>") && sheet.contains("<t>a_x0001_b</t>"), "{sheet}");
}

#[test]
fn a_row_deleted_inside_an_array_leaves_the_array_whole() {
    // SEQUENCE(5) down A1:A5; row 3 deleted: Excel spills it again over A1:A5
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("seq5.xlsx");
    let mut data = String::from(r#"<row r="1"><c r="A1" cm="1"><f t="array" ref="A1:A5">_xlfn.SEQUENCE(5)</f><v>1</v></c></row>"#);
    for r in 2..=5 {
        data.push_str(&format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#));
    }
    plain_book(&src, &[("S", worksheet(&data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[Edit::DeleteRows { sheet: "S".into(), at: 3, count: 1 }], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "A1"), Some(r#"<c r="A1" cm="1"><f t="array" ref="A1:A5">_xlfn.SEQUENCE(5)</f><v>1</v></c>"#), "{sheet}");
    for r in 2..=5 {
        assert_eq!(element(&sheet, &format!("A{r}")), Some(format!(r#"<c r="A{r}"><v>{r}</v></c>"#).as_str()), "{sheet}");
        assert_eq!(read_back(&dst, "S", &format!("A{r}")), Value::Number(r as f64));
    }
}

#[test]
fn a_cell_a_random_array_no_longer_reaches_goes() {
    // RANDARRAY(1,3) across A1:C1; a column inserted at B: the array spills over A1:C1 again, the
    // old C1, now at D1, is no part of it and must not stay behind as a constant
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("rand.xlsx");
    let data = r#"<row r="1"><c r="A1" cm="1"><f t="array" ref="A1:C1">_xlfn.RANDARRAY(1,3)</f><v>0.1</v></c><c r="B1"><v>0.2</v></c><c r="C1"><v>0.3</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[Edit::InsertColumns { sheet: "S".into(), at: "B".into(), count: 1 }], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(element(&sheet, "A1").unwrap().contains(r#"ref="A1:C1""#), "{sheet}");
    assert!(element(&sheet, "B1").is_some_and(|c| c.contains("<v>")), "{sheet}");
    assert_eq!(element(&sheet, "C1"), Some(r#"<c r="C1"><v>0.2</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "D1"), None, "{sheet}");
    // as Excel writes a volatile array: its spilled cells hold an empty `<f ca="1"/>`, a
    // placeholder and no formula of their own
    let data = r#"<row r="1"><c r="A1" cm="1"><f t="array" aca="1" ref="A1:C1" ca="1">_xlfn.RANDARRAY(1,3)</f><v>0.1</v></c><c r="B1"><f ca="1"/><v>0.2</v></c><c r="C1"><f ca="1"/><v>0.3</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    std::fs::remove_file(&dst).unwrap();
    patch(&src, &dst, &[Edit::InsertColumns { sheet: "S".into(), at: "B".into(), count: 1 }], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "C1"), Some(r#"<c r="C1"><f ca="1"/><v>0.2</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "D1"), None, "{sheet}");
}

// ---- refute round 3 ---------------------------------------------------------------------------

#[test]
fn a_formula_that_no_longer_reads_a_random_cell_gets_its_value() {
    let dir = tempfile::tempdir().unwrap();
    // D2 reads the random C1; deleting row 1 leaves it `#REF!*10`, deterministic
    let src = dir.path().join("reader.xlsx");
    let data = r#"<row r="1"><c r="C1"><f>RAND()</f><v>0.5</v></c></row><row r="2"><c r="D2"><f>C1*10</f><v>5</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[Edit::DeleteRows { sheet: "S".into(), at: 1, count: 1 }], &verified()).unwrap();
    assert_eq!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "D1"), Some(r#"<c r="D1" t="e"><f>#REF!*10</f><v>#REF!</v></c>"#));
    // C1 set to 3: D1 reads a value now
    let data = r#"<row r="1"><c r="C1"><f>RAND()</f><v>0.5</v></c><c r="D1"><f>C1*10</f><v>5</v></c><c r="E1"><f>A1+RAND()</f><v>1.5</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    std::fs::remove_file(&dst).unwrap();
    patch(&src, &dst, &[value("S", "C1", serde_json::json!(3))], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "D1"), Some(r#"<c r="D1"><f>C1*10</f><v>30</v></c>"#), "{sheet}");
    // a formula that calls a random function itself is volatile still: its cache stays
    assert_eq!(element(&sheet, "E1"), Some(r#"<c r="E1"><f>A1+RAND()</f><v>1.5</v></c>"#), "{sheet}");
}

#[test]
fn a_tables_totals_row_formula_follows_what_it_reads() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("totals.xlsx");
    let text = |c: &str, t: &str| format!(r#"<c r="{c}" t="inlineStr"><is><t>{t}</t></is></c>"#);
    let data = format!(
        r#"<row r="2">{}{}</row><row r="3">{}<c r="B3"><v>1</v></c></row><row r="4">{}<c r="B4"><v>2</v></c></row><row r="5">{}<c r="B5"><f>SUM(Other!A1:A3)</f><v>6</v></c></row>"#,
        text("A2", "Name"), text("B2", "Qty"), text("A3", "a"), text("A4", "b"), text("A5", "Total")
    );
    let other: String = (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let table = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><table xmlns="{MAIN}" id="1" name="Table1" displayName="Table1" ref="A2:B5" totalsRowCount="1"><autoFilter ref="A2:B4"/><tableColumns count="2"><tableColumn id="1" name="Name" totalsRowLabel="Total"/><tableColumn id="2" name="Qty" totalsRowFunction="custom"><totalsRowFormula>SUM(Other!A1:A3)</totalsRowFormula></tableColumn></tableColumns><tableStyleInfo name="TableStyleMedium2" showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/></table>"#
    );
    plain_book(
        &src,
        &[("T", worksheet(&data, r#"<tableParts count="1"><tablePart r:id="rId1"/></tableParts>"#)), ("Other", worksheet(&other, ""))],
        "",
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", rels(&[rel("rId1", "table", "../tables/table1.xml")])),
            ("xl/tables/table1.xml", table),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[rows("Other", 1, 1)], &verified()).unwrap();
    assert!(part(&dst, "xl/tables/table1.xml").contains("<totalsRowFormula>SUM(Other!A2:A4)</totalsRowFormula>"));
    assert_eq!(stored(&dst, "T", "B5").0.as_deref(), Some("SUM(Other!A2:A4)"));
}

#[test]
fn a_chartex_series_follows_the_rows_it_reads() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("chartex.xlsx");
    let data: String = (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let drawing = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"/>"#;
    let chartex = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cx:chartSpace xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex"><cx:chartData><cx:data id="0"><cx:numDim type="val"><cx:f dir="row">Data!$A$1:$A$3</cx:f></cx:numDim></cx:data></cx:chartData></cx:chartSpace>"#;
    let chartex_rel = rels(&[r#"<Relationship Id="rId1" Type="http://schemas.microsoft.com/office/2014/relationships/chartEx" Target="../charts/chartEx1.xml"/>"#.to_string()]);
    plain_book(
        &src,
        &[("Data", worksheet(&data, r#"<drawing r:id="rId1"/>"#))],
        "",
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", rels(&[rel("rId1", "drawing", "../drawings/drawing1.xml")])),
            ("xl/drawings/drawing1.xml", drawing.to_string()),
            ("xl/drawings/_rels/drawing1.xml.rels", chartex_rel),
            ("xl/charts/chartEx1.xml", chartex.to_string()),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[rows("Data", 1, 1)], &verified()).unwrap();
    assert!(part(&dst, "xl/charts/chartEx1.xml").contains(r#"<cx:f dir="row">Data!$A$2:$A$4</cx:f>"#));
}

/// Sheet `S`: A1:C5 under an autoFilter filtering column C, sorted by C.
fn filtered_book(path: &Path) {
    let data: String = (1..=5).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{r}</v></c><c r="C{r}"><v>{r}</v></c></row>"#)).collect();
    let after = r#"<autoFilter ref="A1:C5"><filterColumn colId="2"><filters><filter val="3"/></filters></filterColumn></autoFilter><sortState ref="A2:C5"><sortCondition ref="C2:C5"/></sortState>"#;
    plain_book(path, &[("S", worksheet(&data, after))], "", &[]);
}

#[test]
fn an_autofilter_keeps_filtering_and_sorting_its_own_column() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("filter.xlsx");
    filtered_book(&src);
    let dst = dir.path().join("out.xlsx");
    let cols = |at: &str, insert: bool| {
        if insert {
            Edit::InsertColumns { sheet: "S".into(), at: at.into(), count: 1 }
        } else {
            Edit::DeleteColumns { sheet: "S".into(), at: at.into(), count: 1 }
        }
    };
    for (edit, filter, sort) in [
        (cols("B", true), r#"<autoFilter ref="A1:D5"><filterColumn colId="3">"#, r#"<sortState ref="A2:D5"><sortCondition ref="D2:D5"/></sortState>"#),
        (cols("B", false), r#"<autoFilter ref="A1:B5"><filterColumn colId="1">"#, r#"<sortState ref="A2:B5"><sortCondition ref="B2:B5"/></sortState>"#),
        // the filtered column deleted: its filter and its sort go with it
        (cols("C", false), r#"<autoFilter ref="A1:B5"></autoFilter>"#, r#"<sortState ref="A2:B5"></sortState>"#),
    ] {
        let _ = std::fs::remove_file(&dst);
        patch(&src, &dst, &[edit.clone()], &verified()).unwrap();
        let sheet = part(&dst, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains(filter) && sheet.contains(sort), "{edit:?}: {sheet}");
    }
}

#[test]
fn a_clear_leaves_the_cells_an_array_spills_into_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("seq3.xlsx");
    let data = r#"<row r="1"><c r="A1" cm="1"><f t="array" ref="A1:A3">_xlfn.SEQUENCE(3)</f><v>1</v></c></row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3"><v>3</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let clear = |area: &str| Edit::Clear { sheet: "S".into(), area: area.into() };
    let report = patch(&src, &dst, &[clear("A2:A3")], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
    assert_eq!(report.outcomes[0].note, "S!A2:A3: 2 cell(s) an array spills into left to it");
    assert_eq!(parts(&src)["xl/worksheets/sheet1.xml"], parts(&dst)["xl/worksheets/sheet1.xml"]);
    // its anchor inside the area: the whole array goes
    std::fs::remove_file(&dst).unwrap();
    patch(&src, &dst, &[clear("A1:A3")], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    for cell in ["A1", "A2", "A3"] {
        assert_eq!(element(&sheet, cell), None, "{sheet}");
    }
}

#[test]
fn spill_and_calc_errors_are_cached_as_excel_spells_them() {
    // FILTER_ERRORS.xlsx: C1 filters A1:A4 above 3; A4 set to 0 leaves nothing: #CALC!
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("out.xlsx");
    let src = fork_file("calc_tests/LOOKUP_AND_REFERENCE/FILTER_ERRORS.xlsx");
    patch(&src, &dst, &[value("Sheet1", "A4", serde_json::json!(0))], &verified()).unwrap();
    let c1 = element(&part(&dst, "xl/worksheets/sheet1.xml"), "C1").unwrap().to_string();
    assert!(c1.contains(r#"t="e""#) && c1.ends_with("<v>#VALUE!</v></c>") && !c1.contains("vm="), "{c1}");
    assert_eq!(read_back(&dst, "Sheet1", "C1"), Value::Error("#CALC!".into()));
    // an array blocked by a value in its way: #SPILL!
    let src = dir.path().join("blocked.xlsx");
    let data = r#"<row r="1"><c r="A1" cm="1"><f t="array" ref="A1:A3">_xlfn.SEQUENCE(B1)</f><v>1</v></c><c r="B1"><v>3</v></c></row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3"><v>3</v></c></row><row r="5"><c r="A5"><v>7</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    std::fs::remove_file(&dst).unwrap();
    patch(&src, &dst, &[value("S", "B1", serde_json::json!(5))], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    let a1 = element(&sheet, "A1").unwrap();
    assert!(a1.contains(r#"t="e""#) && a1.ends_with("<v>#VALUE!</v></c>"), "{sheet}");
    assert_eq!(element(&sheet, "A2"), None, "{sheet}");
    assert_eq!(read_back(&dst, "S", "A1"), Value::Error("#SPILL!".into()));
}

#[test]
fn a_dropped_format_takes_its_anchor_with_it() {
    // a format on B2:B3, row 2 deleted twice: the format goes; the sparkline after it reads S!B5:D5,
    // now S!B3:D3, at E3
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("sparks.xlsx");
    let data: String = (1..=5).map(|r| format!(r#"<row r="{r}"><c r="B{r}"><v>{r}</v></c></row>"#)).collect();
    let after = r#"<conditionalFormatting sqref="B2:B3"><cfRule type="expression" dxfId="0" priority="1"><formula>B2&gt;1</formula></cfRule></conditionalFormatting><extLst><ext uri="{05C60535-1F16-4fd2-B633-F4F36F0B64E0}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:sparklineGroups xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:sparklineGroup><x14:sparklines><x14:sparkline><xm:f>S!B5:D5</xm:f><xm:sqref>E5</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext></extLst>"#;
    plain_book(&src, &[("S", worksheet(&data, after))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let delete = || Edit::DeleteRows { sheet: "S".into(), at: 2, count: 1 };
    patch(&src, &dst, &[delete(), delete()], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("<conditionalFormatting"), "{sheet}");
    assert!(sheet.contains("<xm:f>S!B3:D3</xm:f><xm:sqref>E3</xm:sqref>"), "{sheet}");
}

#[test]
fn an_x14_validation_dropped_by_a_move_is_no_longer_counted() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("x14.xlsx");
    let data: String = (1..=5).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let validation = |at: &str| format!(r#"<x14:dataValidation type="list" allowBlank="1"><x14:formula1><xm:f>Other!$A$1:$A$3</xm:f></x14:formula1><xm:sqref>{at}</xm:sqref></x14:dataValidation>"#);
    let after = format!(
        r#"<extLst><ext uri="{{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:dataValidations count="2" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main">{}{}</x14:dataValidations></ext></extLst>"#,
        validation("B2"),
        validation("C5")
    );
    let other: String = (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    plain_book(&src, &[("S", worksheet(&data, &after)), ("Other", worksheet(&other, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[Edit::DeleteRows { sheet: "S".into(), at: 2, count: 1 }], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<x14:dataValidations count="1""#) && sheet.contains("<xm:sqref>C4</xm:sqref>"), "{sheet}");
}

#[test]
fn a_workbook_is_computed_as_the_destination_it_is_written_to() {
    // CELL("filename") names the workbook: patched from in.xlsx to out.xlsx, the model is out
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("in.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1" t="str"><f>CELL("filename",A1)</f><v>[in.xlsx]S</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let report = patch(&src, &dst, &[value("S", "A1", serde_json::json!(2))], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
}

// ---- refute round 4 ---------------------------------------------------------------------------

#[test]
fn an_array_that_grows_fills_the_empty_cells_the_file_holds() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("seq.xlsx");
    let data = r#"<row r="1"><c r="A1" cm="1"><f t="array" ref="A1:A3">_xlfn.SEQUENCE(B1,1,B1-1,-1)</f><v>2</v></c><c r="B1"><v>3</v></c></row><row r="2"><c r="A2"><v>1</v></c></row><row r="3"><c r="A3"><v>0</v></c></row><row r="4"><c r="A4"/></row><row r="5"><c r="A5"/></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[value("S", "B1", serde_json::json!(6))], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(element(&sheet, "A1").unwrap().contains(r#"ref="A1:A6""#), "{sheet}");
    for (cell, v) in [("A4", "2"), ("A5", "1"), ("A6", "0")] {
        assert_eq!(element(&sheet, cell), Some(format!(r#"<c r="{cell}"><v>{v}</v></c>"#).as_str()), "{sheet}");
    }
}

#[test]
fn a_formula_the_engine_evaluates_as_an_array_is_written_as_one() {
    // SUM(A1:A3*B1:B3) is 140 as an array, 10 read with Excel's implicit intersection
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("arrays.xlsx");
    let data: String =
        (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{}</v></c></row>"#, r * 10)).collect();
    plain_book(&src, &[("S", worksheet(&data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let formula = |cell: &str, f: &str| Edit::Formula { sheet: "S".into(), cell: cell.into(), formula: f.into(), expect: None };
    patch(&src, &dst, &[formula("D1", "=SUM(A1:A3*B1:B3)"), formula("D2", "=A1+B1")], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "D1"), Some(r#"<c r="D1" cm="1"><f t="array" ref="D1">SUM(A1:A3*B1:B3)</f><v>140</v></c>"#), "{sheet}");
    // a plain one stays plain
    assert_eq!(element(&sheet, "D2"), Some(r#"<c r="D2"><f>A1+B1</f><v>11</v></c>"#), "{sheet}");
    assert!(part(&dst, "xl/metadata.xml").contains("fDynamic=\"1\""));
    // a workbook with its own dynamic-array metadata: its entry used, its part as it was
    let src = fork_file("calc_tests/LOOKUP_AND_REFERENCE/FILTER_ERRORS.xlsx");
    std::fs::remove_file(&dst).unwrap();
    let z = Edit::Formula { sheet: "Sheet1".into(), cell: "Z1".into(), formula: "=SUM(A1:A4*2)".into(), expect: None };
    patch(&src, &dst, &[z], &verified()).unwrap();
    let z1 = element(&part(&dst, "xl/worksheets/sheet1.xml"), "Z1").unwrap().to_string();
    assert!(z1.contains(r#"cm="1""#) && z1.contains(r#"<f t="array" ref="Z1">"#), "{z1}");
    assert_eq!(parts(&src)["xl/metadata.xml"], parts(&dst)["xl/metadata.xml"]);
}

#[test]
fn a_width_or_an_outline_keeps_what_survives_of_its_columns() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("widths.xlsx");
    plain_book(&src, &[("S", worksheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let width = Edit::ColumnWidth { sheet: "S".into(), columns: "B:D".into(), width: 30.0 };
    let outline = Edit::OutlineColumns { sheet: "S".into(), columns: "B:D".into(), level: 1, hidden: false };
    for gone in ["B", "D"] {
        for edit in [width.clone(), outline.clone()] {
            let _ = std::fs::remove_file(&dst);
            let delete = Edit::DeleteColumns { sheet: "S".into(), at: gone.into(), count: 1 };
            patch(&src, &dst, &[edit.clone(), delete], &verified()).unwrap();
            let sheet = part(&dst, "xl/worksheets/sheet1.xml");
            assert!(sheet.contains(r#"<col min="2" max="3""#), "{edit:?}, {gone} deleted: {sheet}");
        }
    }
}

#[test]
fn an_edit_of_a_table_header_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("table.xlsx");
    table_book(&src);
    for edit in [value("T", "B2", serde_json::json!("Amount")), Edit::Clear { sheet: "T".into(), area: "A2:B2".into() }] {
        let dst = dir.path().join("out.xlsx");
        let e = patch(&src, &dst, &[edit.clone()], &verified()).unwrap_err();
        assert!(e.to_string().contains("is a header of table Table1"), "{edit:?}: {e}");
        assert!(!dst.exists());
    }
    // a cell of its data: written
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[value("T", "B3", serde_json::json!(5))], &verified()).unwrap();
}

#[test]
fn a_new_sheet_takes_no_name_a_sheet_has_nor_one_excel_reserves() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["Chart1", "chart1", "Second", "History"] {
        let dst = dir.path().join("out.xlsx");
        let e = patch(&fork_file("example.xlsx"), &dst, &[Edit::AddSheet { name: name.into() }], &verified()).unwrap_err();
        assert!(e.to_string().contains("a sheet named") || e.to_string().contains("reserves"), "{name}: {e}");
    }
}

#[test]
fn an_error_marked_by_value_metadata_loses_the_mark_when_it_moves() {
    // A1's array blocked by A3 (#SPILL!, cached #VALUE! with its mark); B1 set to text: a real #VALUE!
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("marked.xlsx");
    let data = r#"<row r="1"><c r="A1" t="e" cm="1" vm="1"><f t="array" ref="A1">_xlfn.SEQUENCE(B1)</f><v>#VALUE!</v></c><c r="B1"><v>3</v></c></row><row r="3"><c r="A3"><v>7</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    assert_eq!(read_back(&src, "S", "A1"), Value::Error("#SPILL!".into()));
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[value("S", "B1", serde_json::json!("x"))], &verified()).unwrap();
    let a1 = element(&part(&dst, "xl/worksheets/sheet1.xml"), "A1").unwrap().to_string();
    assert!(!a1.contains("vm=") && a1.ends_with("<v>#VALUE!</v></c>"), "{a1}");
}

#[test]
fn clearing_what_blocks_an_array_lets_it_spill() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("blocked.xlsx");
    let data = r#"<row r="1"><c r="A1" t="e" cm="1" vm="1"><f t="array" ref="A1">_xlfn.SEQUENCE(3)</f><v>#VALUE!</v></c></row><row r="3"><c r="A3"><v>7</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let report = patch(&src, &dst, &[Edit::Clear { sheet: "S".into(), area: "A3".into() }], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "A2"), Some(r#"<c r="A2"><v>2</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "A3"), Some(r#"<c r="A3"><v>3</v></c>"#), "{sheet}");
    let a1 = element(&sheet, "A1").unwrap();
    assert!(a1.contains(r#"ref="A1:A3""#) && a1.ends_with("<v>1</v></c>") && !a1.contains("vm=") && !a1.contains(r#"t="e""#), "{a1}");
}

#[test]
fn a_name_that_reads_as_a_cell_is_refused_in_any_case() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("names.xlsx");
    plain_book(&src, &[("S", worksheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let set = |name: &str| Edit::SetName { name: name.into(), formula: "=S!$A$1".into(), scope: None };
    for name in ["q1", "Tax2023", "fy2024", "r1c1", "RC", "c3", "R"] {
        let e = patch(&src, &dst, &[set(name)], &verified()).unwrap_err();
        assert!(e.to_string().contains("reads as a cell"), "{name}: {e}");
        assert!(!dst.exists());
    }
    for name in ["Rate", "Coin", "Total2"] {
        let _ = std::fs::remove_file(&dst);
        patch(&src, &dst, &[set(name)], &verified()).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn a_value_edit_takes_only_the_errors_a_cell_holds() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("errors.xlsx");
    plain_book(&src, &[("S", worksheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    for e in ["#SPILL!", "#CALC!", "#WHAT?"] {
        let err = patch(&src, &dst, &[value("S", "B1", serde_json::json!({"error": e}))], &verified()).unwrap_err();
        assert!(err.to_string().contains("not an error a cell holds"), "{e}: {err}");
    }
    patch(&src, &dst, &[value("S", "B1", serde_json::json!({"error": "#N/A"}))], &verified()).unwrap();
    assert_eq!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "B1"), Some(r#"<c r="B1" t="e"><v>#N/A</v></c>"#));
}

#[test]
fn charts_data_label_ranges_user_shapes_and_form_controls_follow_their_cells() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("links.xlsx");
    let data: String = (1..=4).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{r}</v></c></row>"#)).collect();
    let drawing = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"/>"#;
    let chart = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:c15="http://schemas.microsoft.com/office/drawing/2012/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numRef><c:f>Data!$A$1:$A$3</c:f></c:numRef></c:val><c:extLst><c:ext uri="{02D57815-91ED-43cb-92C2-25804820EDAC}"><c15:datalabelsRange><c15:f>Data!$B$1:$B$3</c15:f></c15:datalabelsRange><c15:formulaRef><c15:sqref>Data!$A$1:$A$3</c15:sqref></c15:formulaRef></c:ext></c:extLst></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
    let shapes = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:userShapes xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:cdr="http://schemas.openxmlformats.org/drawingml/2006/chartDrawing"><cdr:relSizeAnchor><cdr:sp macro="" textlink="Data!$A$3"><cdr:nvSpPr><cdr:cNvPr id="2" name="Note"/><cdr:cNvSpPr/></cdr:nvSpPr><cdr:spPr/></cdr:sp></cdr:relSizeAnchor></c:userShapes>"#;
    let control = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><formControlPr xmlns="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" objectType="Drop" fmlaLink="$A$3" fmlaRange="$B$1:$B$3"/>"#;
    plain_book(
        &src,
        &[("Data", worksheet(&data, r#"<drawing r:id="rId1"/>"#))],
        "",
        &[
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                rels(&[rel("rId1", "drawing", "../drawings/drawing1.xml"), rel("rId2", "ctrlProp", "../ctrlProps/ctrlProp1.xml")]),
            ),
            ("xl/drawings/drawing1.xml", drawing.to_string()),
            ("xl/drawings/_rels/drawing1.xml.rels", rels(&[rel("rId1", "chart", "../charts/chart1.xml")])),
            ("xl/charts/chart1.xml", chart.to_string()),
            ("xl/charts/_rels/chart1.xml.rels", rels(&[rel("rId1", "chartUserShapes", "../drawings/drawing2.xml")])),
            ("xl/drawings/drawing2.xml", shapes.to_string()),
            ("xl/ctrlProps/ctrlProp1.xml", control.to_string()),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[rows("Data", 1, 1)], &verified()).unwrap();
    let chart = part(&dst, "xl/charts/chart1.xml");
    assert!(chart.contains("<c:f>Data!$A$2:$A$4</c:f>") && chart.contains("<c15:f>Data!$B$2:$B$4</c15:f>"), "{chart}");
    assert!(chart.contains("<c15:sqref>Data!$A$2:$A$4</c15:sqref>"), "{chart}");
    assert!(part(&dst, "xl/drawings/drawing2.xml").contains(r#"textlink="Data!$A$4""#));
    let control = part(&dst, "xl/ctrlProps/ctrlProp1.xml");
    assert!(control.contains(r#"fmlaLink="$A$4""#) && control.contains(r#"fmlaRange="$B$2:$B$4""#), "{control}");
}

// ---- refute round 5 ---------------------------------------------------------------------------

#[test]
fn a_conditional_formats_thresholds_follow_their_cells() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("scale.xlsx");
    let data: String = (1..=6).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="D{r}"><v>{r}</v></c></row>"#)).collect();
    let after = r#"<conditionalFormatting sqref="A3:A6"><cfRule type="colorScale" priority="1"><colorScale><cfvo type="num" val="$D$1"/><cfvo type="formula" val="$D$2*2"/><cfvo type="max"/><color rgb="FFF8696B"/><color rgb="FFFFEB84"/><color rgb="FF63BE7B"/></colorScale></cfRule></conditionalFormatting><conditionalFormatting sqref="B3:B6"><cfRule type="dataBar" priority="2"><dataBar><cfvo type="num" val="$D$4"/><cfvo type="formula" val="MAX($D$1:$D$5)"/><color rgb="FF638EC6"/></dataBar></cfRule><cfRule type="iconSet" priority="3"><iconSet><cfvo type="percent" val="0"/><cfvo type="num" val="$D$5"/><cfvo type="num" val="5"/></iconSet></cfRule></conditionalFormatting>"#;
    plain_book(&src, &[("S", worksheet(&data, after))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[rows("S", 1, 1)], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<conditionalFormatting sqref="A4:A7">"#), "{sheet}");
    assert!(sheet.contains(r#"<cfvo type="num" val="$D$2"/><cfvo type="formula" val="$D$3*2"/><cfvo type="max"/>"#), "{sheet}");
    assert!(sheet.contains(r#"<dataBar><cfvo type="num" val="$D$5"/><cfvo type="formula" val="MAX($D$2:$D$6)"/>"#), "{sheet}");
    assert!(sheet.contains(r#"<iconSet><cfvo type="percent" val="0"/><cfvo type="num" val="$D$6"/><cfvo type="num" val="5"/></iconSet>"#), "{sheet}");
}

#[test]
fn a_raw_tab_or_line_feed_in_a_moved_attribute_is_the_space_xml_reads_it_as() {
    // every XML reader takes a raw tab, line feed or carriage return in an attribute as a space;
    // `&#9;` is a tab
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("attr.xlsx");
    let data: String = (1..=5).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let after = "<conditionalFormatting sqref=\"A1:A5\"><cfRule type=\"colorScale\" priority=\"1\"><colorScale><cfvo type=\"formula\" val=\"LEN(&quot;a\tb&quot;)+$A$5\"/><cfvo type=\"formula\" val=\"LEN(&quot;a&#9;b\r\nc&quot;)+$A$4\"/><color rgb=\"FFF8696B\"/><color rgb=\"FF63BE7B\"/></colorScale></cfRule></conditionalFormatting>";
    plain_book(&src, &[("S", worksheet(&data, after))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[rows("S", 1, 1)], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<cfvo type="formula" val="LEN(&quot;a b&quot;)+$A$6"/>"#), "{sheet}");
    assert!(sheet.contains(r#"<cfvo type="formula" val="LEN(&quot;a&#9;b c&quot;)+$A$5"/>"#), "{sheet}");
}

#[test]
fn spreadsheetml_under_a_namespace_prefix_is_refused_with_its_reason() {
    // the .NET Open XML SDK's default spelling, which the engine reads: the patcher reads tags by
    // their unprefixed names and refuses it, with nothing to do or something
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("prefixed.xlsx");
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><x:worksheet xmlns:x="{MAIN}"><x:sheetData><x:row r="1"><x:c r="A1"><x:v>1</x:v></x:c></x:row></x:sheetData></x:worksheet>"#
    );
    let book = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><x:workbook xmlns:x="{MAIN}" xmlns:r="{R}"><x:sheets><x:sheet name="S" sheetId="1" r:id="rId1"/></x:sheets></x:workbook>"#
    );
    write_package(
        &src,
        &[
            ("xl/workbook.xml", book),
            ("xl/_rels/workbook.xml.rels", rels(&[rel("rId1", "worksheet", "worksheets/sheet1.xml")])),
            ("xl/worksheets/sheet1.xml", sheet),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    for edits in [vec![], vec![value("S", "A1", serde_json::json!(2))]] {
        match patch(&src, &dst, &edits, &verified()) {
            Err(PatchError::Unsupported(m)) => {
                assert!(m.contains("xl/workbook.xml") && m.contains("<x:workbook>") && m.contains("namespace prefix"), "{m}")
            }
            other => panic!("{edits:?}: {other:?}"),
        }
        assert!(!dst.exists());
    }
}

#[test]
fn a_plain_formula_is_not_already_the_array_an_edit_writes() {
    // D1 holds SUM(A1:A3*B1:B3) plain, as openpyxl or an older Excel write it: Excel reads 10
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("plain.xlsx");
    let mut data: String =
        (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{}</v></c></row>"#, r * 10)).collect();
    data = data.replacen("</row>", r#"<c r="D1"><f>SUM(A1:A3*B1:B3)</f><v>10</v></c></row>"#, 1);
    plain_book(&src, &[("S", worksheet(&data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let edit = Edit::Formula { sheet: "S".into(), cell: "D1".into(), formula: "=SUM(A1:A3*B1:B3)".into(), expect: None };
    let report = patch(&src, &dst, &[edit], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
    assert_eq!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "D1"), Some(r#"<c r="D1" cm="1"><f t="array" ref="D1">SUM(A1:A3*B1:B3)</f><v>140</v></c>"#));
}

#[test]
fn a_form_controls_vml_copy_follows_its_cells() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("control.xlsx");
    let data: String = (1..=4).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{r}</v></c></row>"#)).collect();
    let vml = r##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="_x0000_s1025" type="#_x0000_t201" style="position:absolute"><x:ClientData ObjectType="Drop"><x:Anchor>3, 0, 2, 0, 5, 0, 3, 0</x:Anchor><x:FmlaLink>$A$3</x:FmlaLink><x:FmlaRange>$B$1:$B$3</x:FmlaRange></x:ClientData></v:shape></xml>"##;
    let control = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><formControlPr xmlns="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" objectType="Drop" fmlaLink="$A$3" fmlaRange="$B$1:$B$3"/>"#;
    plain_book(
        &src,
        &[("S", worksheet(&data, r#"<legacyDrawing r:id="rId1"/>"#))],
        "",
        &[
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                rels(&[rel("rId1", "vmlDrawing", "../drawings/vmlDrawing1.vml"), rel("rId2", "ctrlProp", "../ctrlProps/ctrlProp1.xml")]),
            ),
            ("xl/drawings/vmlDrawing1.vml", vml.to_string()),
            ("xl/ctrlProps/ctrlProp1.xml", control.to_string()),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[rows("S", 1, 1)], &verified()).unwrap();
    let vml = part(&dst, "xl/drawings/vmlDrawing1.vml");
    assert!(vml.contains("<x:FmlaLink>$A$4</x:FmlaLink><x:FmlaRange>$B$2:$B$4</x:FmlaRange>"), "{vml}");
    assert!(vml.contains("<x:Anchor>3, 0, 3, 0, 5, 0, 4, 0</x:Anchor>"), "{vml}");
    assert!(part(&dst, "xl/ctrlProps/ctrlProp1.xml").contains(r#"fmlaLink="$A$4""#));
}

#[test]
fn a_one_cell_legacy_array_is_overwritten_and_a_member_of_a_larger_one_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("legacy.xlsx");
    let mut data: String =
        (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{}</v></c></row>"#, r * 10)).collect();
    data = data.replacen("</row>", r#"<c r="D1"><f t="array" ref="D1">SUM(A1:A3*B1:B3)</f><v>140</v></c><c r="E1"><f t="array" ref="E1:E2">A1:A2*2</f><v>2</v></c></row>"#, 1);
    data = data.replacen(r#"<c r="B2"><v>20</v></c></row>"#, r#"<c r="B2"><v>20</v></c><c r="E2"><v>4</v></c></row>"#, 1);
    plain_book(&src, &[("S", worksheet(&data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let report = patch(&src, &dst, &[value("S", "D1", serde_json::json!(7)), value("S", "E2", serde_json::json!(9))], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
    assert_eq!(report.outcomes[1].status, Status::Conflict);
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "D1"), Some(r#"<c r="D1"><v>7</v></c>"#), "{sheet}");
}

#[test]
fn several_edits_of_one_cell_leave_the_last() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("twice.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>2</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let formula = |cell: &str, f: &str| Edit::Formula { sheet: "S".into(), cell: cell.into(), formula: f.into(), expect: None };
    for (edits, cell, want) in [
        (vec![formula("D1", "=A1+B1"), value("S", "D1", serde_json::json!(4))], "D1", Some(r#"<c r="D1"><v>4</v></c>"#)),
        (vec![formula("D1", "=A1+B1"), Edit::Clear { sheet: "S".into(), area: "D1".into() }], "D1", None),
        (vec![formula("D1", "=SUM(A1:B1*2)"), value("S", "D1", serde_json::json!(4))], "D1", Some(r#"<c r="D1"><v>4</v></c>"#)),
        (vec![formula("D1", "=A1+B1"), rows("S", 1, 1), value("S", "D2", serde_json::json!(4))], "D2", Some(r#"<c r="D2"><v>4</v></c>"#)),
    ] {
        let dst = dir.path().join("out.xlsx");
        let _ = std::fs::remove_file(&dst);
        patch(&src, &dst, &edits, &verified()).unwrap_or_else(|e| panic!("{edits:?}: {e}"));
        assert_eq!(element(&part(&dst, "xl/worksheets/sheet1.xml"), cell), want, "{edits:?}");
    }
}

#[test]
fn a_one_value_function_given_a_range_is_written_as_an_array() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("lifted.xlsx");
    let data: String = (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>-{r}</v></c><c r="B{r}" t="inlineStr"><is><t>ab</t></is></c></row>"#)).collect();
    plain_book(&src, &[("S", worksheet(&data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    for (f, v) in [
        ("=SUM(ABS(A1:A3))", "6"),
        ("=SUM(LEN(B1:B3))", "6"),
        ("=SUMPRODUCT(ABS(A1:A3))", "6"),
        ("=MAX(INT(A1:A3))", "-1"),
        ("=MIN(ABS(A1:A3))", "1"),
    ] {
        let _ = std::fs::remove_file(&dst);
        let edit = Edit::Formula { sheet: "S".into(), cell: "D5".into(), formula: f.into(), expect: None };
        patch(&src, &dst, &[edit], &verified()).unwrap_or_else(|e| panic!("{f}: {e}"));
        let d5 = element(&part(&dst, "xl/worksheets/sheet1.xml"), "D5").unwrap().to_string();
        assert!(d5.contains(r#"cm="1""#) && d5.contains(r#"<f t="array" ref="D5">"#) && d5.ends_with(&format!("<v>{v}</v></c>")), "{f}: {d5}");
    }
}

#[test]
fn a_text_with_a_character_xml_cannot_carry_is_escaped() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("text.xlsx");
    plain_book(&src, &[("S", worksheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let edits = vec![value("S", "B1", serde_json::json!("a\u{FFFF}b")), value("S", "B2", serde_json::json!("c\u{FFFE}d"))];
    patch(&src, &dst, &edits, &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<t>a_xFFFF_b</t>") && sheet.contains("<t>c_xFFFE_d</t>"), "{sheet}");
    assert!(!sheet.contains('\u{FFFF}') && !sheet.contains('\u{FFFE}'));
    assert_eq!(read_back(&dst, "S", "B1"), Value::Text("a\u{FFFF}b".into()));
}

#[test]
fn rows_past_the_last_are_refused_and_a_new_part_takes_a_name_free_in_any_case() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("edges.xlsx");
    // its one worksheet's part is `Sheet1.xml`, capitalised
    write_package(
        &src,
        &[
            ("xl/workbook.xml", workbook(&["S"], "")),
            ("xl/_rels/workbook.xml.rels", rels(&[rel("rId1", "worksheet", "worksheets/Sheet1.xml")])),
            ("xl/worksheets/Sheet1.xml", worksheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, "")),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    let outline = Edit::OutlineRows { sheet: "S".into(), first: 1_048_575, last: 1_048_578, level: 1, hidden: false };
    assert!(patch(&src, &dst, &[outline], &verified()).is_err());
    assert!(!dst.exists());
    patch(&src, &dst, &[Edit::AddSheet { name: "New".into() }], &verified()).unwrap();
    let names: Vec<String> = parts(&dst).into_keys().filter(|p| p.to_lowercase().starts_with("xl/worksheets/")).collect();
    assert_eq!(names, ["xl/worksheets/Sheet1.xml", "xl/worksheets/sheet2.xml"]);
}

#[test]
fn a_metadata_part_without_a_dynamic_array_entry_gets_one() {
    // the workbook's metadata part holds rich values only (pictures in cells)
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("rich.xlsx");
    let data: String =
        (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{}</v></c></row>"#, r * 10)).collect();
    let metadata = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xlrd="http://schemas.microsoft.com/office/spreadsheetml/2017/richdata"><metadataTypes count="1"><metadataType name="XLRICHVALUE" minSupportedVersion="120000" copy="1" pasteAll="1" pasteValues="1" merge="1" splitFirst="1" rowColShift="1" clearFormats="1" clearComments="1" assign="1" coerce="1"/></metadataTypes><futureMetadata name="XLRICHVALUE" count="1"><bk><extLst><ext uri="{3e2802c4-a4d2-4d8b-9148-e3be6c30e623}"><xlrd:rvb i="0"/></ext></extLst></bk></futureMetadata><valueMetadata count="1"><bk><rc t="1" v="0"/></bk></valueMetadata></metadata>"#;
    write_package(
        &src,
        &[
            ("xl/workbook.xml", workbook(&["S"], "")),
            ("xl/_rels/workbook.xml.rels", rels(&[rel("rId1", "worksheet", "worksheets/sheet1.xml"), rel("rId2", "sheetMetadata", "metadata.xml")])),
            ("xl/worksheets/sheet1.xml", worksheet(&data, "")),
            ("xl/metadata.xml", metadata.to_string()),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    let edit = Edit::Formula { sheet: "S".into(), cell: "D1".into(), formula: "=SUM(A1:A3*B1:B3)".into(), expect: None };
    patch(&src, &dst, &[edit], &verified()).unwrap();
    let m = part(&dst, "xl/metadata.xml");
    assert!(m.contains(r#"<metadataTypes count="2">"#) && m.contains(r#"<metadataType name="XLDAPR""#), "{m}");
    assert!(m.contains(r#"<futureMetadata name="XLDAPR" count="1">"#) && m.contains(r#"fDynamic="1""#), "{m}");
    assert!(m.contains(r#"<cellMetadata count="1"><bk><rc t="2" v="0"/></bk></cellMetadata><valueMetadata"#), "{m}");
    // the rich values kept
    assert!(m.contains(r#"<valueMetadata count="1"><bk><rc t="1" v="0"/></bk></valueMetadata>"#), "{m}");
    assert!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "D1").unwrap().contains(r#"cm="1""#));
}

// ---- refute round 6 ---------------------------------------------------------------------------

#[test]
fn a_value_over_a_dynamic_arrays_anchor_takes_its_spill_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("spill.xlsx");
    let data = r#"<row r="1"><c r="H1" cm="1"><f t="array" ref="H1:H3">_xlfn.SEQUENCE(3)</f><v>1</v></c></row><row r="2"><c r="H2"><v>2</v></c></row><row r="3"><c r="C3" cm="1"><f t="array" ref="C3:C5">_xlfn.RANDARRAY(3)</f><v>0.1</v></c><c r="H3"><v>3</v></c></row><row r="4"><c r="C4"><v>0.2</v></c></row><row r="5"><c r="C5"><v>0.3</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    for v in [serde_json::json!(7), serde_json::json!("x"), serde_json::json!(true)] {
        let _ = std::fs::remove_file(&dst);
        let report = patch(&src, &dst, &[value("S", "H1", v.clone()), value("S", "C3", v.clone())], &verified()).unwrap();
        assert!(report.outcomes.iter().all(|o| o.status == Status::Applied), "{v}: {:?}", report.outcomes);
        let sheet = part(&dst, "xl/worksheets/sheet1.xml");
        for gone in ["H2", "H3", "C4", "C5"] {
            assert_eq!(element(&sheet, gone), None, "{v}: {gone} left behind in {sheet}");
        }
        assert!(!element(&sheet, "H1").unwrap().contains("<f"), "{sheet}");
    }
    // Excel's own file: SEQUENCE's anchor overwritten
    let src = fork_file("calc_tests/MATH_AND_TRIGONOMETRY/SEQUENCE.xlsx");
    let before = cached::read(&src).unwrap();
    let spilled: Vec<(i32, i32)> = before
        .iter()
        .filter(|((s, _, c), cell)| s == "Sheet1" && *c == 1 && cell.formula.is_none() && !cell.shared)
        .map(|((_, r, c), _)| (*r, *c))
        .filter(|(r, _)| *r > 1)
        .collect();
    let _ = std::fs::remove_file(&dst);
    patch(&src, &dst, &[value("Sheet1", "A1", serde_json::json!(7))], &verified()).unwrap();
    let after = cached::read(&dst).unwrap();
    assert_eq!(after.get(&("Sheet1".to_string(), 1, 1)).map(|c| c.value.clone()), Some(Value::Number(7.0)));
    let mut left = Vec::new();
    for (r, c) in spilled.iter().filter(|(r, _)| *r <= 4) {
        if let Some(cell) = after.get(&("Sheet1".to_string(), *r, *c)) {
            left.push((*r, cell.value.clone()));
        }
    }
    assert!(left.iter().all(|(_, v)| *v == Value::Empty), "A2:A4 left as constants: {left:?}");
}

#[test]
fn a_plain_formula_is_the_array_an_edit_writes_where_excel_365_shows_it_without_an_intersection() {
    // `Rate` one cell, `Flag` one cell: Excel 365 shows `Rate*2` and `IF(Flag,1,0)` as written;
    // `INDEX(A1:A3,2)`, a function that can return a range, it shows `=@INDEX(A1:A3,2)`, and
    // `IF(Span,1,0)`, `Span` three cells, `=IF(@Span,1,0)`
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("names.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>5</v></c><c r="D1"><f>Rate*2</f><v>10</v></c><c r="E1"><f>INDEX(A1:A3,2)</f><v>6</v></c><c r="F1"><f>IF(Flag,1,0)</f><v>1</v></c><c r="G1"><f>SUM(A1:A3*B1:B3)</f><v>50</v></c><c r="H1"><f>IF(Span,1,0)</f><v>1</v></c></row><row r="2"><c r="A2"><v>6</v></c><c r="B2"><v>1</v></c></row><row r="3"><c r="A3"><v>7</v></c><c r="B3"><v>1</v></c></row><row r="4"><c r="A4" t="b"><v>1</v></c></row>"#;
    let names = r#"<definedName name="Flag">S!$A$4</definedName><definedName name="Rate">S!$A$1</definedName><definedName name="Span">S!$A$1:$A$3</definedName>"#;
    let data = data.replace(r#"<c r="A1"><v>5</v></c>"#, r#"<c r="A1"><v>5</v></c><c r="B1"><v>10</v></c>"#);
    plain_book(&src, &[("S", worksheet(&data, ""))], names, &[]);
    let dst = dir.path().join("out.xlsx");
    // an expectation each formula meets: the edits applied
    let expecting = |cell: &str, f: &str| Edit::Value {
        sheet: "S".into(),
        cell: cell.into(),
        value: serde_json::json!(3),
        expect: Some(Expect::Formula { formula: f.into() }),
    };
    let statuses = |report: veredas_abaco::patch::PatchReport| -> Vec<Status> { report.outcomes.into_iter().map(|o| o.status).collect() };
    let report = patch(
        &src,
        &dst,
        &[expecting("D1", "=Rate*2"), expecting("E1", "=INDEX(A1:A3,2)"), expecting("F1", "=IF(Flag,1,0)"), expecting("H1", "=IF(Span,1,0)")],
        &verified(),
    )
    .unwrap();
    assert_eq!(statuses(report), [Status::Applied, Status::Conflict, Status::Applied, Status::Conflict]);
    // the same formulas written: already there, the file as it was, but INDEX's, written as the
    // array it is
    let _ = std::fs::remove_file(&dst);
    let formula = |cell: &str, f: &str| Edit::Formula { sheet: "S".into(), cell: cell.into(), formula: f.into(), expect: None };
    let report = patch(&src, &dst, &[formula("D1", "=Rate*2"), formula("F1", "=IF(Flag,1,0)")], &verified()).unwrap();
    assert_eq!(statuses(report), [Status::AlreadyThere, Status::AlreadyThere]);
    assert_eq!(parts(&dst), parts(&src));
    let _ = std::fs::remove_file(&dst);
    let report = patch(&src, &dst, &[formula("E1", "=INDEX(A1:A3,2)")], &verified()).unwrap();
    assert_eq!(statuses(report), [Status::Applied]);
    assert!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "E1").unwrap().contains(r#"<f t="array" ref="E1">"#));
    // a range under an operator, which Excel 365 shows intersected (`SUM(@A1:A3*@B1:B3)`, 50, A1*B1
    // alone): the edit writes the array (63, A1*B1+A2*B2+A3*B3) and an expectation of the formula
    // does not hold
    let _ = std::fs::remove_file(&dst);
    let report = patch(&src, &dst, &[formula("G1", "=SUM(A1:A3*B1:B3)"), expecting("G1", "=SUM(A1:A3*B1:B3)")], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
    // the second edit's expectation holds against what the first wrote, and its value lands
    assert_eq!(report.outcomes[1].status, Status::Applied, "{:?}", report.outcomes);
    assert_eq!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "G1"), Some(r#"<c r="G1"><v>3</v></c>"#));
    let _ = std::fs::remove_file(&dst);
    patch(&src, &dst, &[formula("G1", "=SUM(A1:A3*B1:B3)")], &verified()).unwrap();
    assert!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "G1").unwrap().contains(r#"<f t="array" ref="G1">"#));
    let _ = std::fs::remove_file(&dst);
    let report = patch(&src, &dst, &[expecting("G1", "=SUM(A1:A3*B1:B3)")], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Conflict, "{:?}", report.outcomes);
}

#[test]
fn an_error_only_the_engine_has_is_never_written_as_a_cache() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("errors.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>IF(A1=1,0,B1+1)</f><v>0</v></c></row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3"><v>3</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let formula = |cell: &str, f: &str| Edit::Formula { sheet: "S".into(), cell: cell.into(), formula: f.into(), expect: None };
    for (edits, error) in [
        (vec![formula("E5", "=E5+1")], "#CIRC!"),
        (vec![value("S", "A1", serde_json::json!(2))], "#CIRC!"),
        (vec![formula("D1", r#"=SUM(VALUE(A1:A3&""))"#)], "#N/IMPL"),
    ] {
        match patch(&src, &dst, &edits, &verified()) {
            Err(PatchError::Unsupported(m)) => {
                assert!(m.contains(error) && m.contains("an error no workbook file holds"), "{edits:?}: {m}")
            }
            other => panic!("{edits:?}: {other:?}"),
        }
        assert!(!dst.exists(), "{edits:?}");
    }
}

/// A metadata part holding an OLAP formula's cell metadata before any dynamic array's.
fn olap_metadata(dynamic: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><metadata xmlns="{MAIN}" xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray"><metadataTypes count="1"><metadataType name="XLMDX" minSupportedVersion="120000" copy="1" pasteAll="1" pasteValues="1" merge="1" splitFirst="1" rowColShift="1" clearFormats="1" clearComments="1" assign="1" coerce="1" cellMeta="1"/>{dynamic}</metadataTypes><metadataStrings count="1"><s v="[Measures].[x]"/></metadataStrings><mdxMetadata count="1"><mdx n="0" f="m"><t c="1"><n x="0" s="1"/></t></mdx></mdxMetadata><cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata><extLst><ext uri="{{x}}"/></extLst></metadata>"#
    )
}

#[test]
fn a_dynamic_formula_takes_the_entry_the_metadata_part_gives_it() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("olap.xlsx");
    let data: String =
        (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{}</v></c></row>"#, r * 10)).collect();
    let book = |path: &Path, metadata: String| {
        write_package(
            path,
            &[
                ("xl/workbook.xml", workbook(&["S"], "")),
                ("xl/_rels/workbook.xml.rels", rels(&[rel("rId1", "worksheet", "worksheets/sheet1.xml"), rel("rId2", "sheetMetadata", "metadata.xml")])),
                ("xl/worksheets/sheet1.xml", worksheet(&data, "")),
                ("xl/metadata.xml", metadata),
            ],
        )
    };
    book(&src, olap_metadata(""));
    let dst = dir.path().join("out.xlsx");
    let edit = Edit::Formula { sheet: "S".into(), cell: "D1".into(), formula: "=SUM(A1:A3*B1:B3)".into(), expect: None };
    patch(&src, &dst, std::slice::from_ref(&edit), &verified()).unwrap();
    let d1 = element(&part(&dst, "xl/worksheets/sheet1.xml"), "D1").unwrap().to_string();
    assert!(d1.contains(r#"cm="2""#) && d1.contains(r#"<f t="array" ref="D1">"#) && d1.ends_with("<v>140</v></c>"), "{d1}");
    assert_eq!(read_back(&dst, "S", "D1"), Value::Number(140.0));
    // a dynamic-array entry already there, its flag spelled `true`: taken, none added
    let entry = r#"<metadataType name="XLDAPR" minSupportedVersion="120000" copy="1" pasteAll="1" pasteValues="1" merge="1" splitFirst="1" rowColShift="1" clearFormats="1" clearComments="1" assign="1" coerce="1" cellMeta="1"/>"#;
    let with = olap_metadata(entry)
        .replace(r#"<metadataTypes count="1">"#, r#"<metadataTypes count="2">"#)
        .replace(
            "<cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>",
            r#"<futureMetadata name="XLDAPR" count="1"><bk><extLst><ext uri="{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}"><xda:dynamicArrayProperties fDynamic="true" fCollapsed="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="2"><bk><rc t="1" v="0"/></bk><bk><rc t="2" v="0"/></bk></cellMetadata>"#,
        );
    book(&src, with.clone());
    let _ = std::fs::remove_file(&dst);
    patch(&src, &dst, &[edit], &verified()).unwrap();
    assert!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "D1").unwrap().contains(r#"cm="2""#));
    assert_eq!(part(&dst, "xl/metadata.xml"), with);
}

#[test]
fn a_sparkline_goes_with_its_cell_and_a_group_with_its_last_sparkline() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("sparklines.xlsx");
    let data: String = (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{r}</v></c></row>"#)).collect();
    let group = |lines: &str| {
        format!(
            r#"<x14:sparklineGroup displayEmptyCellsAs="gap"><x14:colorSeries rgb="FF376092"/><x14:sparklines>{lines}</x14:sparklines></x14:sparklineGroup>"#
        )
    };
    let line = |r: i32| format!("<x14:sparkline><xm:f>S!A{r}:B{r}</xm:f><xm:sqref>C{r}</xm:sqref></x14:sparkline>");
    let ext = |groups: &str| {
        format!(
            r#"<extLst><ext uri="{{05C60535-1F16-4fd2-B633-F4F36F0B64E0}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:sparklineGroups xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main">{groups}</x14:sparklineGroups></ext></extLst>"#
        )
    };
    plain_book(&src, &[("S", worksheet(&data, &ext(&group(&(line(2) + &line(3))))))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let delete = |at: i32| Edit::DeleteRows { sheet: "S".into(), at, count: 1 };
    patch(&src, &dst, &[delete(2)], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.ends_with(&format!("{}</worksheet>", ext(&group("<x14:sparkline><xm:f>S!A2:B2</xm:f><xm:sqref>C2</xm:sqref></x14:sparkline>")))), "{sheet}");
    // its one sparkline gone, the group, the groups, the extension and its list go too
    plain_book(&src, &[("S", worksheet(&data, &ext(&group(&line(2)))))], "", &[]);
    let _ = std::fs::remove_file(&dst);
    patch(&src, &dst, &[delete(2)], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.ends_with("</sheetData></worksheet>"), "{sheet}");
}

#[test]
fn a_formula_or_a_name_holding_a_literal_escape_reads_back_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("literal.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1" t="str"><f>"_x005F_x0041_"&amp;A1</f><v>_x005F_x0041_1</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let edits = vec![
        Edit::Formula { sheet: "S".into(), cell: "C1".into(), formula: r#"="_x0041_"&"x""#.into(), expect: None },
        Edit::SetName { name: "Lit".into(), formula: r#""_x0041_""#.into(), scope: None },
        rows("S", 1, 1),
    ];
    patch(&src, &dst, &edits, &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<f>"_x005F_x0041_"&amp;"x"</f>"#), "{sheet}");
    assert!(sheet.contains(r#"<f>"_x005F_x0041_"&amp;A2</f>"#), "{sheet}");
    assert!(part(&dst, "xl/workbook.xml").contains(r#"<definedName name="Lit">"_x005F_x0041_"</definedName>"#));
    assert_eq!(read_back(&dst, "S", "C2"), Value::Text("_x0041_x".into()));
    assert_eq!(read_back(&dst, "S", "B2"), Value::Text("_x0041_1".into()));
}

#[test]
fn a_function_is_written_as_excel_spells_it_in_a_formula_and_a_name() {
    // DAYS, GAUSS, IMSECH and FORECAST.LINEAR are Excel 2013's and 2016's, prefixed in a file;
    // CEILING, GAMMALN and ISO.CEILING Excel writes plain (fork change 25)
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("spelling.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="A2"><v>3</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let formula = |cell: &str, f: &str| Edit::Formula { sheet: "S".into(), cell: cell.into(), formula: f.into(), expect: None };
    let edits = vec![
        formula("B1", "=DAYS(DATE(2025,1,10),DATE(2025,1,1))"),
        formula("C1", "=CEILING(2.5,1)+ISO.CEILING(2.5)+GAMMALN(1)"),
        formula("D1", "=GAUSS(0)+IMREAL(IMSECH(0))"),
        formula("E1", "=FORECAST.LINEAR(2,A1:A2,A1:A2)"),
        Edit::SetName { name: "Span".into(), formula: "=DAYS(DATE(2025,1,10),DATE(2025,1,1))".into(), scope: None },
    ];
    patch(&src, &dst, &edits, &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "B1"), Some(r#"<c r="B1"><f>_xlfn.DAYS(DATE(2025,1,10),DATE(2025,1,1))</f><v>9</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "C1"), Some(r#"<c r="C1"><f>CEILING(2.5,1)+ISO.CEILING(2.5)+GAMMALN(1)</f><v>6</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "D1"), Some(r#"<c r="D1"><f>_xlfn.GAUSS(0)+IMREAL(_xlfn.IMSECH(0))</f><v>1</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "E1"), Some(r#"<c r="E1"><f>_xlfn.FORECAST.LINEAR(2,A1:A2,A1:A2)</f><v>2</v></c>"#), "{sheet}");
    let book = part(&dst, "xl/workbook.xml");
    assert!(book.contains(r#"<definedName name="Span">_xlfn.DAYS(DATE(2025,1,10),DATE(2025,1,1))</definedName>"#), "{book}");
    assert_eq!(read_back(&dst, "S", "B1"), Value::Number(9.0));
}

#[test]
fn a_shared_group_moved_without_changing_what_its_members_read_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("shared.xlsx");
    let data = r#"<row r="1"><c r="B1"><f t="shared" ref="B1:B3" si="0">$Z$1</f><v>9</v></c></row><row r="2"><c r="B2"><f t="shared" si="0"/><v>9</v></c></row><row r="3"><c r="B3"><f t="shared" si="0"/><v>9</v></c></row><row r="4"><c r="Z1"><v>0</v></c></row>"#
        .replace(r#"<row r="4"><c r="Z1"><v>0</v></c></row>"#, "")
        .replace(r#"<row r="1"><c r="B1">"#, r#"<row r="1"><c r="B1">"#);
    let data = data.replace(r#"<v>9</v></c></row><row r="2">"#, r#"<v>9</v></c><c r="Z1"><v>9</v></c></row><row r="2">"#);
    plain_book(&src, &[("S", worksheet(&data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    // a row inserted inside the group: B3 moves to B4, the derived `$Z$1` is the same
    let report = patch(&src, &dst, &[rows("S", 3, 1)], &verified()).unwrap();
    assert_eq!(report.expanded, 0);
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "B1"), Some(r#"<c r="B1"><f t="shared" ref="B1:B4" si="0">$Z$1</f><v>9</v></c>"#));
    assert_eq!(element(&sheet, "B2"), Some(r#"<c r="B2"><f t="shared" si="0"/><v>9</v></c>"#));
    assert_eq!(element(&sheet, "B4"), Some(r#"<c r="B4"><f t="shared" si="0"/><v>9</v></c>"#));
    // a row inserted above it: the group moved whole
    let _ = std::fs::remove_file(&dst);
    let report = patch(&src, &dst, &[Edit::InsertRows { sheet: "S".into(), at: 1, count: 1 }], &verified()).unwrap();
    assert_eq!(report.expanded, 1, "Z1 moved too: the members' formula reads $Z$2 now");
}

// ---- refute round 7 ---------------------------------------------------------------------------

#[test]
fn a_plain_formula_is_judged_as_it_stands_after_the_edits() {
    // `Rate` one cell: `Rate*2` reads as written; `INDEX(A1:A3,2)` is shown `=@INDEX(…)`; F1 holds a
    // formula with no cache, as other writers leave one; G1 a range under an operator whose
    // intersected value happens to equal the array's (B2 and B3 zero), and stops to when B2 moves
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("inputs.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>5</v></c><c r="D1"><f>Rate*2</f><v>10</v></c><c r="E1"><f>INDEX(A1:A3,2)</f><v>6</v></c><c r="F1"><f>Rate*3</f></c></row><row r="2"><c r="A2"><v>6</v></c></row><row r="3"><c r="A3"><v>7</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], r#"<definedName name="Rate">S!$A$1</definedName>"#, &[]);
    let dst = dir.path().join("out.xlsx");
    let expecting = |cell: &str, f: &str| Edit::Value {
        sheet: "S".into(),
        cell: cell.into(),
        value: serde_json::json!(3),
        expect: Some(Expect::Formula { formula: f.into() }),
    };
    let formula = |cell: &str, f: &str| Edit::Formula { sheet: "S".into(), cell: cell.into(), formula: f.into(), expect: None };
    let statuses = |edits: &[Edit]| -> Vec<Status> {
        let _ = std::fs::remove_file(&dst);
        patch(&src, &dst, edits, &verified()).unwrap().outcomes.into_iter().map(|o| o.status).collect()
    };
    // the inputs changed first: `Rate*2` still holds and is still there; INDEX's expectation does
    // not hold, and its formula written is the array it is
    let inputs = [value("S", "A1", serde_json::json!(50)), value("S", "A2", serde_json::json!(60))];
    let mut edits = inputs.to_vec();
    edits.extend([expecting("D1", "=Rate*2"), expecting("E1", "=INDEX(A1:A3,2)")]);
    assert_eq!(statuses(&edits), [Status::Applied, Status::Applied, Status::Applied, Status::Conflict]);
    let mut edits = inputs.to_vec();
    edits.extend([formula("D1", "=Rate*2"), formula("E1", "=INDEX(A1:A3,2)")]);
    assert_eq!(statuses(&edits), [Status::Applied, Status::Applied, Status::AlreadyThere, Status::Applied]);
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "D1"), Some(r#"<c r="D1"><f>Rate*2</f><v>100</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "E1"), Some(r#"<c r="E1" cm="1"><f t="array" ref="E1">INDEX(A1:A3,2)</f><v>60</v></c>"#), "{sheet}");
    // no cache to judge by changes nothing: `Rate*3` reads as written
    assert_eq!(statuses(&[expecting("F1", "=Rate*3")]), [Status::Applied]);
    assert_eq!(statuses(&[formula("F1", "=Rate*3")]), [Status::AlreadyThere]);
    // G1, a range under an operator: its intersected value 50 and its array's 50 alike only while B2
    // and B3 are zero; B2 moved, the formula written is the array (56), not already there
    let src2 = dir.path().join("coincidence.xlsx");
    let g = |b2: i32, b3: i32, cache: i32| {
        format!(r#"<row r="1"><c r="A1"><v>5</v></c><c r="B1"><v>10</v></c><c r="G1"><f>SUM(A1:A3*B1:B3)</f><v>{cache}</v></c></row><row r="2"><c r="A2"><v>6</v></c><c r="B2"><v>{b2}</v></c></row><row r="3"><c r="A3"><v>7</v></c><c r="B3"><v>{b3}</v></c></row>"#)
    };
    plain_book(&src2, &[("S", worksheet(&g(0, 0, 50), ""))], "", &[]);
    let _ = std::fs::remove_file(&dst);
    let report = patch(&src2, &dst, &[value("S", "B2", serde_json::json!(1)), formula("G1", "=SUM(A1:A3*B1:B3)")], &verified()).unwrap();
    assert_eq!(report.outcomes[1].status, Status::Applied, "{:?}", report.outcomes);
    assert_eq!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "G1"), Some(r#"<c r="G1" cm="1"><f t="array" ref="G1">SUM(A1:A3*B1:B3)</f><v>56</v></c>"#));
    // rows 2:3 deleted, G1 left `SUM(A1:A1*B1:B1)`, one cell under the operator: its expectation
    // holds, its formula is already there
    plain_book(&src2, &[("S", worksheet(&g(1, 1, 50), ""))], "", &[]);
    let delete = Edit::DeleteRows { sheet: "S".into(), at: 2, count: 2 };
    let _ = std::fs::remove_file(&dst);
    let report = patch(&src2, &dst, &[delete.clone(), expecting("G1", "=SUM(A1:A1*B1:B1)")], &verified()).unwrap();
    assert_eq!(report.outcomes[1].status, Status::Applied, "{:?}", report.outcomes);
    let _ = std::fs::remove_file(&dst);
    let report = patch(&src2, &dst, &[delete, formula("G1", "=SUM(A1:A1*B1:B1)")], &verified()).unwrap();
    assert_eq!(report.outcomes[1].status, Status::AlreadyThere, "{:?}", report.outcomes);
    assert!(!parts(&dst).contains_key("xl/metadata.xml"));
}

#[test]
fn a_built_in_name_deleted_goes_and_one_not_deleted_stays_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("print.xlsx");
    let data: String = (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{r}</v></c></row>"#)).collect();
    let filter = r#"<definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">S!$A$1:$B$3</definedName>"#;
    let names = format!(r#"{filter}<definedName name="_xlnm.Print_Area" localSheetId="0">S!$A$1:$B$2</definedName>"#);
    plain_book(&src, &[("S", worksheet(&data, r#"<autoFilter ref="A1:B3"/>"#))], &names, &[]);
    let dst = dir.path().join("out.xlsx");
    let report = patch(&src, &dst, &[Edit::DeleteName { name: "_xlnm.Print_Area".into(), scope: Some("S".into()) }], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
    let wb = part(&dst, "xl/workbook.xml");
    assert!(!wb.contains("Print_Area"), "{wb}");
    assert!(wb.contains(filter), "{wb}");
}

#[test]
fn a_merge_cut_to_one_cell_is_unmerged_and_an_emptied_list_goes() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("merges.xlsx");
    let data: String = (1..=3)
        .map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><v>{r}</v></c><c r="E{r}"><v>{r}</v></c></row>"#))
        .collect();
    let after = r#"<mergeCells count="2"><mergeCell ref="B2:C2"/><mergeCell ref="E2:E3"/></mergeCells><hyperlinks><hyperlink ref="A3" location="S!A1" display="top"/></hyperlinks>"#;
    plain_book(&src, &[("S", worksheet(&data, after))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let edits = [Edit::DeleteColumns { sheet: "S".into(), at: "C".into(), count: 1 }, Edit::DeleteRows { sheet: "S".into(), at: 3, count: 1 }];
    patch(&src, &dst, &edits, &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("mergeCell") && !sheet.contains("hyperlink"), "{sheet}");
    assert!(sheet.ends_with("</sheetData></worksheet>"), "{sheet}");
    // a merge that keeps two cells stays, its list counted
    let _ = std::fs::remove_file(&dst);
    patch(&src, &dst, &[Edit::DeleteRows { sheet: "S".into(), at: 1, count: 1 }], &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<mergeCells count="2"><mergeCell ref="B1:C1"/><mergeCell ref="E1:E2"/></mergeCells>"#), "{sheet}");
}

#[test]
fn a_carriage_return_in_a_formula_or_a_name_reads_back_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("cr.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1" t="str"><f>"a_x000D_b"&amp;A1</f><v>a_x000D_b1</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let edits = vec![
        rows("S", 1, 1),
        Edit::Formula { sheet: "S".into(), cell: "C2".into(), formula: "=\"x\ry\"&A2".into(), expect: None },
        Edit::SetName { name: "Cr".into(), formula: "\"p\rq\"".into(), scope: None },
    ];
    for options in [verified(), PatchOptions { verify: false, ..Default::default() }] {
        let _ = std::fs::remove_file(&dst);
        patch(&src, &dst, &edits, &options).unwrap();
        let sheet = part(&dst, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains(r#"<f>"a_x000D_b"&amp;A2</f>"#) && sheet.contains(r#"<f>"x_x000D_y"&amp;A2</f>"#), "{sheet}");
        assert!(!sheet.contains('\r'));
        assert!(part(&dst, "xl/workbook.xml").contains(r#"<definedName name="Cr">"p_x000D_q"</definedName>"#));
        assert_eq!(read_back(&dst, "S", "B2"), Value::Text("a\rb1".into()));
        assert_eq!(read_back(&dst, "S", "C2"), Value::Text("x\ry1".into()));
    }
}

// ---- refute round 9 ---------------------------------------------------------------------------

#[test]
fn a_formula_or_a_name_spelled_otherwise_than_excel_spells_it_is_not_already_there() {
    // `IFNA`, `DAYS`, `CONCAT` unprefixed, as some writers leave them: the engine reads them as the
    // functions, Excel as unknown names (`#NAME?`)
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("unprefixed.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="C1"><f>IFNA(A1,0)</f><v>1</v></c><c r="D1"><f>DAYS(A2,A1)</f><v>1</v></c><c r="E1" t="str"><f>CONCAT(A1,A2)</f><v>12</v></c><c r="F1"><f>_xlfn.IFNA(A1,0)</f><v>1</v></c></row><row r="2"><c r="A2"><v>2</v></c></row>"#;
    let names = r#"<definedName name="D">DAYS(S!$A$2,S!$A$1)</definedName><definedName name="Ok">_xlfn.DAYS(S!$A$2,S!$A$1)</definedName>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], names, &[]);
    let dst = dir.path().join("out.xlsx");
    let formula = |cell: &str, f: &str| Edit::Formula { sheet: "S".into(), cell: cell.into(), formula: f.into(), expect: None };
    let set = |name: &str, f: &str| Edit::SetName { name: name.into(), formula: f.into(), scope: None };
    let report = patch(
        &src,
        &dst,
        &[
            formula("C1", "=IFNA(A1,0)"),
            formula("D1", "=_xlfn.DAYS(A2,A1)"),
            formula("E1", "=CONCAT(A1,A2)"),
            formula("F1", "=IFNA(A1,0)"),
            set("D", "=_xlfn.DAYS(S!$A$2,S!$A$1)"),
            set("Ok", "=DAYS(S!$A$2,S!$A$1)"),
        ],
        &verified(),
    )
    .unwrap();
    let statuses: Vec<Status> = report.outcomes.iter().map(|o| o.status).collect();
    assert_eq!(&statuses[..4], [Status::Applied, Status::Applied, Status::Applied, Status::AlreadyThere], "{:?}", report.outcomes);
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "C1"), Some(r#"<c r="C1"><f>_xlfn.IFNA(A1,0)</f><v>1</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "D1"), Some(r#"<c r="D1"><f>_xlfn.DAYS(A2,A1)</f><v>1</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "E1"), Some(r#"<c r="E1" t="str"><f>_xlfn.CONCAT(A1,A2)</f><v>12</v></c>"#), "{sheet}");
    assert_eq!(element(&sheet, "F1"), Some(r#"<c r="F1"><f>_xlfn.IFNA(A1,0)</f><v>1</v></c>"#), "{sheet}");
    let book = part(&dst, "xl/workbook.xml");
    assert!(
        book.contains(r#"<definedName name="D">_xlfn.DAYS(S!$A$2,S!$A$1)</definedName><definedName name="Ok">_xlfn.DAYS(S!$A$2,S!$A$1)</definedName>"#),
        "{book}"
    );
    // after a move, the cell is found where it came from
    let _ = std::fs::remove_file(&dst);
    let report = patch(&src, &dst, &[rows("S", 1, 2), formula("C3", "=IFNA(A3,0)"), formula("F3", "=IFNA(A3,0)")], &verified()).unwrap();
    assert_eq!(report.outcomes[1].status, Status::Applied, "{:?}", report.outcomes);
    assert_eq!(report.outcomes[2].status, Status::AlreadyThere, "{:?}", report.outcomes);
    assert!(element(&part(&dst, "xl/worksheets/sheet1.xml"), "C3").unwrap().contains("<f>_xlfn.IFNA(A3,0)</f>"));
}

#[test]
fn a_function_the_engine_does_not_compute_is_refused_in_a_formula_and_a_name() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("unknown.xlsx");
    let data: String = (1..=3).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    plain_book(&src, &[("S", worksheet(&data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let formula = |f: &str| Edit::Formula { sheet: "S".into(), cell: "D1".into(), formula: f.into(), expect: None };
    let set = |f: &str| Edit::SetName { name: "Tot".into(), formula: f.into(), scope: None };
    for (edit, called) in [
        (formula("=AGGREGATE(9,6,A1:A3)"), "AGGREGATE"),
        (formula("=_xlfn.AGGREGATE(9,6,A1:A3)"), "AGGREGATE"),
        (formula("=Image(A1)+1"), "IMAGE"),
        (formula("=LET(x,1,WEBSERVICE(x))"), "WEBSERVICE"),
        (set("=_xlfn.AGGREGATE(9,6,S!$A$1:$A$3)"), "AGGREGATE"),
        (set("=_xlfn.WEBSERVICE(\"http://x\")"), "WEBSERVICE"),
    ] {
        match patch(&src, &dst, std::slice::from_ref(&edit), &verified()) {
            Err(PatchError::Unsupported(m)) => assert!(m.contains(&format!("{called} is a function the engine does not compute")), "{m}"),
            other => panic!("{edit:?}: {other:?}"),
        }
        assert!(!dst.exists());
    }
    // a LAMBDA name and a function a LET binds are the workbook's own
    let report = patch(
        &src,
        &dst,
        &[
            set("=LAMBDA(x,x*x)"),
            Edit::Formula { sheet: "S".into(), cell: "D1".into(), formula: "=Tot(3)".into(), expect: None },
            Edit::Formula { sheet: "S".into(), cell: "D2".into(), formula: "=LET(f,LAMBDA(a,a*a),f(2))".into(), expect: None },
        ],
        &verified(),
    )
    .unwrap();
    assert!(report.outcomes.iter().all(|o| o.status == Status::Applied), "{:?}", report.outcomes);
    assert_eq!(read_back(&dst, "S", "D1"), Value::Number(9.0));
    assert_eq!(read_back(&dst, "S", "D2"), Value::Number(4.0));
}

#[test]
fn spreadsheetml_prefixed_on_an_inner_element_is_refused_too() {
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("out.xlsx");
    let data: String = (1..=5).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    // a name list under a prefix inside an unprefixed workbook
    let src = dir.path().join("names.xlsx");
    let book = workbook(&["S"], "").replace(&format!(r#"xmlns="{MAIN}""#), &format!(r#"xmlns="{MAIN}" xmlns:x="{MAIN}""#)).replace(
        "<calcPr",
        r#"<x:definedNames><x:definedName name="Rate">S!$A$3</x:definedName></x:definedNames><calcPr"#,
    );
    write_package(
        &src,
        &[("xl/workbook.xml", book), ("xl/_rels/workbook.xml.rels", rels(&[rel("rId1", "worksheet", "worksheets/sheet1.xml")])), ("xl/worksheets/sheet1.xml", worksheet(&data, ""))],
    );
    // a sheet whose `<sheetData>` is plain and its merges prefixed
    let src2 = dir.path().join("merges.xlsx");
    let sheet = worksheet(&data, r#"<x:mergeCells count="1"><x:mergeCell ref="A3:B3"/></x:mergeCells>"#)
        .replace(&format!(r#"xmlns="{MAIN}""#), &format!(r#"xmlns="{MAIN}" xmlns:x="{MAIN}""#));
    plain_book(&src2, &[("S", sheet)], "", &[]);
    for (src, part_name, element) in [(&src, "xl/workbook.xml", "x:definedNames"), (&src2, "xl/worksheets/sheet1.xml", "x:mergeCells")] {
        for edits in [vec![], vec![rows("S", 1, 2)]] {
            match patch(src, &dst, &edits, &verified()) {
                Err(PatchError::Unsupported(m)) => assert!(m.contains(part_name) && m.contains(&format!("<{element}>")), "{m}"),
                other => panic!("{part_name}: {other:?}"),
            }
            assert!(!dst.exists());
        }
    }
}

#[test]
fn an_attribute_in_single_quotes_is_rewritten_in_double_quotes() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("quoted.xlsx");
    let data: String = (1..=6).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let drawing = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:absoluteAnchor><xdr:pos x="0" y="0"/><xdr:ext cx="10" cy="10"/><xdr:sp macro="" textlink='&apos;My Data&apos;!$A$5'><xdr:nvSpPr><xdr:cNvPr id="2" name="Box"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr/></xdr:sp><xdr:clientData/></xdr:absoluteAnchor></xdr:wsDr>"#;
    plain_book(
        &src,
        &[("My Data", worksheet(&data, r#"<drawing r:id="rId1"/>"#))],
        "",
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", rels(&[rel("rId1", "drawing", "../drawings/drawing1.xml")])),
            ("xl/drawings/drawing1.xml", drawing.to_string()),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    patch(&src, &dst, &[rows("My Data", 1, 1)], &verified()).unwrap();
    let d = part(&dst, "xl/drawings/drawing1.xml");
    assert!(d.contains(r#"<xdr:sp macro="" textlink="'My Data'!$A$6">"#), "{d}");
}

#[test]
fn a_one_cell_legacy_array_is_not_already_the_array_an_edit_spills() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("legacy.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="E1"><f t="array" ref="E1">A1:A3*2</f><v>2</v></c><c r="F1"><f t="array" ref="F1">SUM(A1:A3*2)</f><v>12</v></c></row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3"><v>3</v></c></row>"#;
    plain_book(&src, &[("S", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let expecting = |cell: &str, f: &str| Edit::Value { sheet: "S".into(), cell: cell.into(), value: serde_json::json!(7), expect: Some(Expect::Formula { formula: f.into() }) };
    let report = patch(&src, &dst, &[expecting("E1", "=A1:A3*2"), expecting("F1", "=SUM(A1:A3*2)")], &verified()).unwrap();
    let statuses: Vec<Status> = report.outcomes.iter().map(|o| o.status).collect();
    // `{=A1:A3*2}` over one cell shows one value, the edit's formula three; `{=SUM(…)}` one
    assert_eq!(statuses, [Status::Conflict, Status::Applied], "{:?}", report.outcomes);
    let _ = std::fs::remove_file(&dst);
    let formula = |cell: &str, f: &str| Edit::Formula { sheet: "S".into(), cell: cell.into(), formula: f.into(), expect: None };
    let report = patch(&src, &dst, &[formula("F1", "=SUM(A1:A3*2)")], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::AlreadyThere);
    match patch(&src, &dst, &[formula("E1", "=A1:A3*2")], &verified()) {
        Err(PatchError::Unsupported(m)) => assert!(m.contains("spills over several cells"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_sheet_s_name_is_read_as_xml_reads_its_attribute() {
    // a raw tab in `<sheet name>` is a space to every reader, the engine's too
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("tab.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c></row>"#;
    plain_book(&src, &[("Data\tSheet", worksheet(data, ""))], "", &[]);
    let dst = dir.path().join("out.xlsx");
    let report = patch(&src, &dst, &[value("Data Sheet", "A1", serde_json::json!(2))], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::Applied);
    assert_eq!(read_back(&dst, "Data Sheet", "A1"), Value::Number(2.0));
}

// ---- refute round 10 --------------------------------------------------------------------------

/// Excel 365's metadata part marking `cm="1"` a dynamic array.
fn dynamic_metadata() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><metadata xmlns="{MAIN}" xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray"><metadataTypes count="1"><metadataType name="XLDAPR" minSupportedVersion="120000" copy="1" pasteAll="1" pasteValues="1" merge="1" splitFirst="1" rowColShift="1" clearFormats="1" clearComments="1" assign="1" coerce="1" cellMeta="1"/></metadataTypes><futureMetadata name="XLDAPR" count="1"><bk><extLst><ext uri="{{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}}"><xda:dynamicArrayProperties fDynamic="1" fCollapsed="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata></metadata>"#
    )
}

#[test]
fn spreadsheetml_under_a_prefix_of_the_strict_namespace_is_refused_too() {
    // the engine reads the strict namespace under any prefix (fork change 26): so must the refusal
    let strict = "http://purl.oclc.org/ooxml/spreadsheetml/main";
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("out.xlsx");
    let data: String = (1..=5).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let src = dir.path().join("names.xlsx");
    let book = workbook(&["S"], "").replace(&format!(r#"xmlns="{MAIN}""#), &format!(r#"xmlns="{MAIN}" xmlns:x="{strict}""#)).replace(
        "<calcPr",
        r#"<x:definedNames><x:definedName name="Rate">S!$A$3</x:definedName></x:definedNames><calcPr"#,
    );
    write_package(
        &src,
        &[("xl/workbook.xml", book), ("xl/_rels/workbook.xml.rels", rels(&[rel("rId1", "worksheet", "worksheets/sheet1.xml")])), ("xl/worksheets/sheet1.xml", worksheet(&data, ""))],
    );
    let src2 = dir.path().join("merges.xlsx");
    let sheet = worksheet(&data, r#"<x:mergeCells count="1"><x:mergeCell ref="A3:B3"/></x:mergeCells>"#)
        .replace(&format!(r#"xmlns="{MAIN}""#), &format!(r#"xmlns="{MAIN}" xmlns:x="{strict}""#));
    plain_book(&src2, &[("S", sheet)], "", &[]);
    for (src, part_name, element) in [(&src, "xl/workbook.xml", "x:definedNames"), (&src2, "xl/worksheets/sheet1.xml", "x:mergeCells")] {
        for edits in [vec![], vec![rows("S", 1, 2), Edit::Formula { sheet: "S".into(), cell: "C1".into(), formula: "=A1*2".into(), expect: None }]] {
            match patch(src, &dst, &edits, &verified()) {
                Err(PatchError::Unsupported(m)) => assert!(m.contains(part_name) && m.contains(&format!("<{element}>")), "{m}"),
                other => panic!("{part_name}: {other:?}"),
            }
            assert!(!dst.exists());
        }
    }
}

#[test]
fn a_let_or_lambda_parameter_spelled_without_its_prefix_is_not_already_there() {
    // Excel writes a parameter `_xlpm.x`; a file spelling it `x` is read alike by the engine
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("params.xlsx");
    let data = r#"<row r="1"><c r="A1"><v>1</v></c><c r="G1" cm="1"><f t="array" ref="G1">_xlfn.LET(x,1,x+A1)</f><v>2</v></c><c r="H1" cm="1"><f t="array" ref="H1">_xlfn.LET(_xlpm.x,1,_xlpm.x+A1)</f><v>2</v></c></row>"#;
    let names = r#"<definedName name="Dbl">_xlfn.LAMBDA(x,x*2)</definedName><definedName name="Ok">_xlfn.LAMBDA(_xlpm.x,_xlpm.x*2)</definedName>"#;
    write_package(
        &src,
        &[
            ("xl/workbook.xml", workbook(&["S"], names)),
            ("xl/_rels/workbook.xml.rels", rels(&[rel("rId1", "worksheet", "worksheets/sheet1.xml"), rel("rId2", "sheetMetadata", "metadata.xml")])),
            ("xl/worksheets/sheet1.xml", worksheet(data, "")),
            ("xl/metadata.xml", dynamic_metadata()),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    let formula = |cell: &str| Edit::Formula { sheet: "S".into(), cell: cell.into(), formula: "=LET(x,1,x+A1)".into(), expect: None };
    let set = |name: &str| Edit::SetName { name: name.into(), formula: "=LAMBDA(x,x*2)".into(), scope: None };
    let report = patch(&src, &dst, &[formula("G1"), formula("H1"), set("Dbl"), set("Ok")], &verified()).unwrap();
    let statuses: Vec<Status> = report.outcomes.iter().map(|o| o.status).collect();
    assert_eq!(&statuses[..2], [Status::Applied, Status::AlreadyThere], "{:?}", report.outcomes);
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(element(&sheet, "G1").unwrap().contains("_xlfn.LET(_xlpm.x,1,_xlpm.x+A1)"), "{sheet}");
    let book = part(&dst, "xl/workbook.xml");
    assert!(
        book.contains(r#"<definedName name="Dbl">_xlfn.LAMBDA(_xlpm.x,_xlpm.x*2)</definedName><definedName name="Ok">_xlfn.LAMBDA(_xlpm.x,_xlpm.x*2)</definedName>"#),
        "{book}"
    );
}

#[test]
fn a_lambda_name_local_to_another_sheet_is_no_function_a_cell_can_call() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("local.xlsx");
    let names = r#"<definedName name="Sq" localSheetId="1">_xlfn.LAMBDA(_xlpm.x,_xlpm.x*_xlpm.x)</definedName>"#;
    plain_book(&src, &[("S", worksheet("", "")), ("T", worksheet("", ""))], names, &[]);
    let dst = dir.path().join("out.xlsx");
    let formula = |sheet: &str| Edit::Formula { sheet: sheet.into(), cell: "D1".into(), formula: "=Sq(3)".into(), expect: None };
    for edit in [formula("S"), Edit::SetName { name: "G".into(), formula: "=Sq(4)".into(), scope: None }] {
        match patch(&src, &dst, std::slice::from_ref(&edit), &verified()) {
            Err(PatchError::Unsupported(m)) => assert!(m.contains("SQ is a function the engine does not compute"), "{m}"),
            other => panic!("{edit:?}: {other:?}"),
        }
        assert!(!dst.exists());
    }
    // on its own sheet, and in a name of that scope, it is the sheet's own
    let report = patch(&src, &dst, &[formula("T"), Edit::SetName { name: "L".into(), formula: "=Sq(4)".into(), scope: Some("T".into()) }], &verified()).unwrap();
    assert!(report.outcomes.iter().all(|o| o.status == Status::Applied), "{:?}", report.outcomes);
    assert_eq!(read_back(&dst, "T", "D1"), Value::Number(9.0));
}

#[test]
fn a_new_cell_in_a_formatted_row_takes_the_row_s_format() {
    // row 2 formatted whole (`s="1"`, `customFormat="1"`): xf 1 a percentage in bold
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("row.xlsx");
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="{MAIN}"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="10" fontId="1" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" applyFont="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
    );
    let data = r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" s="1" customFormat="1"><c r="A2" s="1"><v>0.5</v></c></row>"#;
    write_package(
        &src,
        &[
            ("xl/workbook.xml", workbook(&["S"], "")),
            ("xl/_rels/workbook.xml.rels", rels(&[rel("rId1", "worksheet", "worksheets/sheet1.xml"), rel("rId2", "styles", "styles.xml")])),
            ("xl/worksheets/sheet1.xml", worksheet(data, "")),
            ("xl/styles.xml", styles),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    let edits = [
        value("S", "B2", serde_json::json!(0.25)),
        Edit::Formula { sheet: "S".into(), cell: "C2".into(), formula: "=A2/2".into(), expect: None },
        Edit::Fill { sheet: "S".into(), area: "D2".into(), rgb: Some("FFFF00".into()) },
    ];
    patch(&src, &dst, &edits, &verified()).unwrap();
    let sheet = part(&dst, "xl/worksheets/sheet1.xml");
    assert_eq!(element(&sheet, "B2"), Some(r#"<c r="B2" s="1"><v>0.25</v></c>"#), "{sheet}");
    assert!(element(&sheet, "C2").unwrap().starts_with(r#"<c r="C2" s="1">"#), "{sheet}");
    // the fill on a clone of the row's format: its number format and font kept
    let d2 = element(&sheet, "D2").unwrap();
    let xf: usize = d2.split(r#"s=""#).nth(1).unwrap().split('"').next().unwrap().parse().unwrap();
    let styles = part(&dst, "xl/styles.xml");
    let xfs = styles.split("<cellXfs").nth(1).unwrap();
    let clone = xfs.split("<xf ").nth(xf + 1).unwrap();
    assert!(clone.contains(r#"numFmtId="10""#) && clone.contains(r#"fontId="1""#) && !clone.contains(r#"fillId="0""#), "{clone}");
}

#[test]
fn a_structured_reference_whose_column_holds_a_bracket_is_already_there() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("rate.xlsx");
    let text = |c: &str, t: &str| format!(r#"<c r="{c}" t="inlineStr"><is><t>{t}</t></is></c>"#);
    let data = format!(
        r#"<row r="1"><c r="D1"><f>SUM(Table1[Rate(%)])</f><v>7</v></c></row><row r="2">{}{}</row><row r="3">{}<c r="B3"><v>3</v></c></row><row r="4">{}<c r="B4"><v>4</v></c></row>"#,
        text("A2", "Name"),
        text("B2", "Rate(%)"),
        text("A3", "a"),
        text("A4", "b")
    );
    let table = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><table xmlns="{MAIN}" id="1" name="Table1" displayName="Table1" ref="A2:B4" totalsRowShown="0"><autoFilter ref="A2:B4"/><tableColumns count="2"><tableColumn id="1" name="Name"/><tableColumn id="2" name="Rate(%)"/></tableColumns><tableStyleInfo name="TableStyleMedium2" showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/></table>"#
    );
    write_package(
        &src,
        &[
            ("xl/workbook.xml", workbook(&["T"], "")),
            ("xl/_rels/workbook.xml.rels", rels(&[rel("rId1", "worksheet", "worksheets/sheet1.xml")])),
            ("xl/worksheets/sheet1.xml", worksheet(&data, r#"<tableParts count="1"><tablePart r:id="rId1"/></tableParts>"#)),
            ("xl/worksheets/_rels/sheet1.xml.rels", rels(&[rel("rId1", "table", "../tables/table1.xml")])),
            ("xl/tables/table1.xml", table),
        ],
    );
    let dst = dir.path().join("out.xlsx");
    let report =
        patch(&src, &dst, &[Edit::Formula { sheet: "T".into(), cell: "D1".into(), formula: "=SUM(Table1[Rate(%)])".into(), expect: None }], &verified()).unwrap();
    assert_eq!(report.outcomes[0].status, Status::AlreadyThere, "{:?}", report.outcomes);
}

#[test]
#[cfg(unix)]
fn the_patch_keeps_a_destination_s_name_as_the_system_gives_it() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.xlsx");
    plain_book(&src, &[("S", worksheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, ""))], "", &[]);
    let dst = dir.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9.xlsx"));
    let bak = dir.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9.xlsx.bak"));
    std::fs::write(&dst, b"previous").unwrap();
    std::fs::write(&bak, b"older").unwrap();
    patch(&src, &dst, &[value("S", "A1", serde_json::json!(2))], &verified()).unwrap();
    assert_eq!(std::fs::read(&bak).unwrap(), b"previous");
    let names: Vec<Vec<u8>> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().as_bytes().to_vec()).collect();
    assert!(!names.iter().any(|n| String::from_utf8_lossy(n).contains('\u{FFFD}') && std::str::from_utf8(n).is_ok()), "{names:?}");
    assert_eq!(names.len(), 3, "{names:?}");
}
