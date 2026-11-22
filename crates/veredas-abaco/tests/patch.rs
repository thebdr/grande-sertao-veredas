//! The byte-faithful patcher (contract: Ábaco's patcher clause) on a hand-built package that
//! carries what a plant's engineering workbooks carry and the edits must respect:
//! shared formulas, a merge, a conditional format, a data validation, a note
//! (comments + VML), defined names (one local), custom properties, printer
//! settings, a calculation chain. Every test patches it, checks the written
//! XML exactly where the edits land, checks every other part is
//! byte-identical, and lets the patcher's own verify re-read the file through
//! the engine.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use veredas_abaco::patch::cached;
use veredas_abaco::patch::{patch, Edit, Expect, Fate, PatchOptions, PatchReport, Status};
use veredas_abaco::Value;

const CT: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="bin" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.printerSettings"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/><Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/><Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/custom.xml" ContentType="application/vnd.openxmlformats-officedocument.custom-properties+xml"/></Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties" Target="docProps/custom.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;

const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>plant</dc:creator></cp:coreProperties>"#;

const CUSTOM: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Plant"><vt:lpwstr>PLANT-A</vt:lpwstr></property></Properties>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr defaultThemeVersion="124226"/><bookViews><workbookView xWindow="0" yWindow="0" windowWidth="20000" windowHeight="10000"/></bookViews><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Summary" sheetId="2" r:id="rId2"/></sheets><definedNames><definedName name="_xlnm.Print_Area" localSheetId="0">Data!$A$1:$G$6</definedName><definedName name="Prices">Data!$C$2:$C$6</definedName><definedName name="Total">Data!$E$2</definedName></definedNames><calcPr calcId="191029"/></workbook>"#;

const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/><family val="2"/></font><font><b/><sz val="11"/><name val="Calibri"/><family val="2"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles><dxfs count="1"><dxf><font><b/></font></dxf></dxfs></styleSheet>"#;

const SST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="8" uniqueCount="8"><si><t>Name</t></si><si><t>Qty</t></si><si><t>Price</t></si><si><t>Total</t></si><si><t>bolt</t></si><si><t>nut</t></si><si><t>washer</t></si><si><r><t>pin</t></r><r><rPr><b/></rPr><t>s</t></r></si></sst>"#;

/// `Data`: a price list. D2:D6 is one shared formula; E2 sums it.
fn data_sheet() -> String {
    let mut rows = String::from(
        r#"<row r="1" spans="1:7"><c r="A1" s="1" t="s"><v>0</v></c><c r="B1" s="1" t="s"><v>1</v></c><c r="C1" s="1" t="s"><v>2</v></c><c r="D1" s="1" t="s"><v>3</v></c><c r="F1" t="s"><v>3</v></c></row>"#,
    );
    let items = [
        (4, 10.0, 2.5),
        (5, 4.0, 30.0),
        (6, 100.0, 0.25),
        (7, 7.0, 3.0),
        (4, 1.0, 1.0),
    ];
    let mut total = 0.0;
    for (i, (name, qty, price)) in items.iter().enumerate() {
        let r = i + 2;
        let d = qty * price;
        total += d;
        let f = if r == 2 {
            r#"<f t="shared" ref="D2:D6" si="0">B2*C2</f>"#.to_string()
        } else {
            r#"<f t="shared" si="0"/>"#.to_string()
        };
        let e = if r == 2 {
            "<c r=\"E2\"><f>SUM(D2:D6)</f><v>TOTAL</v></c>".to_string()
        } else {
            String::new()
        };
        rows.push_str(&format!(
            r#"<row r="{r}" spans="1:7"><c r="A{r}" t="s"><v>{name}</v></c><c r="B{r}"><v>{qty}</v></c><c r="C{r}"><v>{price}</v></c><c r="D{r}">{f}<v>{d}</v></c>{e}</row>"#
        ));
    }
    let rows = rows.replace("TOTAL", &total.to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><dimension ref="A1:G6"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/><selection pane="bottomLeft" activeCell="B5" sqref="B5"/></sheetView></sheetViews><sheetFormatPr defaultRowHeight="15"/><cols><col min="1" max="1" width="20" customWidth="1"/><col min="2" max="3" width="9.140625" style="0"/></cols><sheetData>{rows}</sheetData><mergeCells count="1"><mergeCell ref="F1:G1"/></mergeCells><conditionalFormatting sqref="D2:D6"><cfRule type="expression" dxfId="0" priority="1"><formula>D2&gt;100</formula></cfRule></conditionalFormatting><dataValidations count="1"><dataValidation type="whole" allowBlank="1" showInputMessage="1" showErrorMessage="1" sqref="B2:B6"><formula1>0</formula1><formula2>1000</formula2></dataValidation></dataValidations><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><pageSetup paperSize="9" orientation="portrait"/><legacyDrawing r:id="rId2"/></worksheet>"#
    )
}

fn summary_sheet() -> String {
    // values Excel would cache: total 25+120+25+21+1 = 192, qty 122, C4 = 0.25
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><dimension ref="A1:A4"/><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetFormatPr defaultRowHeight="15"/><sheetData><row r="1"><c r="A1"><f>Data!E2</f><v>192</v></c></row><row r="2"><c r="A2"><f>SUM(Data!B2:B6)</f><v>122</v></c></row><row r="3"><c r="A3"><f>Total*2</f><v>384</v></c></row><row r="4"><c r="A4"><f>'Data'!C4</f><v>0.25</v></c></row></sheetData><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>"#
        .to_string()
}

const SHEET1_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing" Target="../drawings/vmlDrawing1.vml"/><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/printerSettings" Target="../printerSettings/printerSettings1.bin"/></Relationships>"#;

const COMMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>op</author></authors><commentList><comment ref="A3" authorId="0"><text><r><t>check the nut size</t></r></text></comment></commentList></comments>"#;

const VML: &str = r##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/><v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype><v:shape id="_x0000_s1025" type="#_x0000_t202" style="position:absolute;margin-left:107pt;margin-top:1.5pt;width:108pt;height:59.25pt;z-index:1;visibility:hidden" fillcolor="#ffffe1" o:insetmode="auto"><v:fill color2="#ffffe1"/><v:shadow on="t" color="black" obscured="t"/><v:path o:connecttype="none"/><v:textbox style="mso-direction-alt:auto"><div style="text-align:left"></div></v:textbox><x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>1, 15, 1, 2, 3, 15, 5, 3</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>2</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>"##;

const CALC_CHAIN: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><c r="D2" i="1"/><c r="D3"/><c r="D4"/><c r="D5"/><c r="D6"/><c r="E2"/><c r="A1" i="2"/><c r="A2"/><c r="A3"/><c r="A4"/></calcChain>"#;

/// Writes the fixture package into `dir`; its path.
fn fixture(dir: &Path) -> PathBuf {
    let path = dir.join("plant.xlsx");
    let file = std::fs::File::create(&path).unwrap();
    let mut z = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let parts: Vec<(&str, Vec<u8>)> = vec![
        ("[Content_Types].xml", CT.into()),
        ("_rels/.rels", ROOT_RELS.into()),
        ("docProps/core.xml", CORE.into()),
        ("docProps/custom.xml", CUSTOM.into()),
        ("xl/workbook.xml", WORKBOOK.into()),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.into()),
        ("xl/styles.xml", STYLES.into()),
        ("xl/sharedStrings.xml", SST.into()),
        ("xl/worksheets/sheet1.xml", data_sheet().into_bytes()),
        ("xl/worksheets/sheet2.xml", summary_sheet().into_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", SHEET1_RELS.into()),
        ("xl/comments1.xml", COMMENTS.into()),
        ("xl/drawings/vmlDrawing1.vml", VML.into()),
        (
            "xl/printerSettings/printerSettings1.bin",
            vec![0u8, 1, 2, 3, 250, 251, 252, 253],
        ),
        ("xl/calcChain.xml", CALC_CHAIN.into()),
    ];
    for (name, bytes) in parts {
        z.start_file(name, opts).unwrap();
        z.write_all(&bytes).unwrap();
    }
    z.finish().unwrap();
    path
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
    String::from_utf8(
        parts(path)
            .remove(name)
            .unwrap_or_else(|| panic!("no part {name}")),
    )
    .unwrap()
}

/// The parts the report calls rewritten / removed / added, and an assertion
/// that every other part is byte-identical to the source.
fn fates(src: &Path, dst: &Path, report: &PatchReport) -> (Vec<String>, Vec<String>, Vec<String>) {
    let a = parts(src);
    let b = parts(dst);
    let pick = |f: Fate| -> Vec<String> {
        report
            .parts
            .iter()
            .filter(|p| p.fate == f)
            .map(|p| p.name.clone())
            .collect()
    };
    for p in report.parts.iter().filter(|p| p.fate == Fate::Identical) {
        assert_eq!(
            a.get(&p.name),
            b.get(&p.name),
            "{} must be byte-identical",
            p.name
        );
    }
    for p in report.parts.iter().filter(|p| p.fate == Fate::Rewritten) {
        assert_ne!(
            a.get(&p.name),
            b.get(&p.name),
            "{} is reported rewritten but did not change",
            p.name
        );
    }
    // and the zip entries themselves: raw-copied, so same CRC and size
    let mut za = zip::ZipArchive::new(std::fs::File::open(src).unwrap()).unwrap();
    let mut zb = zip::ZipArchive::new(std::fs::File::open(dst).unwrap()).unwrap();
    for p in report.parts.iter().filter(|p| p.fate == Fate::Identical) {
        let (ea, eb) = (
            za.by_name(&p.name).unwrap().crc32(),
            zb.by_name(&p.name).unwrap().crc32(),
        );
        assert_eq!(ea, eb);
    }
    (
        pick(Fate::Rewritten),
        pick(Fate::Removed),
        pick(Fate::Added),
    )
}

fn run(edits: &[Edit]) -> (tempfile::TempDir, PathBuf, PathBuf, PatchReport) {
    let dir = tempfile::tempdir().unwrap();
    let src = fixture(dir.path());
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
    // ABACO_PATCH_KEEP=<folder>: keep every output (named after its test) for
    // the real-Excel check a new shape needs (the law: a formula shape is not
    // done until Excel opened the file)
    if let Ok(keep) = std::env::var("ABACO_PATCH_KEEP") {
        let name = std::thread::current()
            .name()
            .unwrap_or("patch")
            .replace("::", "_");
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::copy(&dst, Path::new(&keep).join(format!("{name}.xlsx"))).unwrap();
        std::fs::copy(&src, Path::new(&keep).join("fixture.xlsx")).unwrap();
    }
    (dir, src, dst, report)
}

fn value(sheet: &str, cell: &str, v: serde_json::Value) -> Edit {
    Edit::Value {
        sheet: sheet.into(),
        cell: cell.into(),
        value: v,
        expect: None,
    }
}

fn cell(path: &Path, sheet: &str, row: i32, col: i32) -> cached::CachedCell {
    cached::read(path)
        .unwrap()
        .remove(&(sheet.to_string(), row, col))
        .unwrap_or_else(|| panic!("no cell {sheet} R{row}C{col}"))
}

#[test]
fn untouched_fixture_is_consistent_with_the_engine() {
    // patching with no edit rewrites nothing: the fixture's caches are what
    // the engine computes, so not one value needs refreshing
    let (_d, src, dst, report) = run(&[]);
    let (rewritten, removed, added) = fates(&src, &dst, &report);
    assert_eq!(
        (rewritten.len(), removed.len(), added.len()),
        (0, 0, 0),
        "{report:?}"
    );
    assert_eq!(report.refreshed, 0);
    assert_eq!(report.verified_cells, 30); // 26 cells on Data, 4 on Summary
}

#[test]
fn a_value_edit_rewrites_its_row_and_the_dependents_caches_only() {
    let (_d, src, dst, report) = run(&[value("Data", "B3", serde_json::json!(6))]);
    assert_eq!(report.outcomes[0].status, Status::Applied);
    let (rewritten, removed, _) = fates(&src, &dst, &report);
    assert_eq!(
        rewritten,
        vec!["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"]
    );
    assert!(removed.is_empty(), "no formula moved: the calc chain stays");
    let s = part(&dst, "xl/worksheets/sheet1.xml");
    // B3 written, D3 (a shared child) gets the new cache, E2 too; row 4 untouched
    assert!(s.contains(r#"<row r="3" spans="1:7"><c r="A3" t="s"><v>5</v></c><c r="B3"><v>6</v></c><c r="C3"><v>30</v></c><c r="D3"><f t="shared" si="0"/><v>180</v></c></row>"#), "{s}");
    assert!(s.contains(r#"<c r="E2"><f>SUM(D2:D6)</f><v>252</v></c>"#));
    assert!(s.contains(
        r#"<row r="4" spans="1:7"><c r="A4" t="s"><v>6</v></c><c r="B4"><v>100</v></c>"#
    ));
    let sum = part(&dst, "xl/worksheets/sheet2.xml");
    assert!(
        sum.contains(r#"<c r="A1"><f>Data!E2</f><v>252</v></c>"#),
        "{sum}"
    );
    assert!(sum.contains(r#"<c r="A2"><f>SUM(Data!B2:B6)</f><v>124</v></c>"#));
    assert!(sum.contains(r#"<c r="A3"><f>Total*2</f><v>504</v></c>"#));
    assert_eq!(report.refreshed, 5); // D3, E2, A1, A2, A3
}

#[test]
fn formulas_are_spelled_for_the_file_whatever_spelling_they_come_in() {
    let (_d, src, dst, report) = run(&[
        Edit::Formula {
            sheet: "Summary".into(),
            cell: "B1".into(),
            formula: "=LET(x,Data!E2,x*2)".into(),
            expect: None,
        },
        // `_xlfn.` without `_xlpm.`, the half-spelling a writer once shipped
        Edit::Formula {
            sheet: "Summary".into(),
            cell: "B2".into(),
            formula: "_xlfn.LET(t,A2,t+1)".into(),
            expect: None,
        },
        Edit::Formula {
            sheet: "Summary".into(),
            cell: "B3".into(),
            formula: "=CONCAT(\"a\",\"b\")&IFNA(1,2)".into(),
            expect: None,
        },
    ]);
    fates(&src, &dst, &report);
    let s = part(&dst, "xl/worksheets/sheet2.xml");
    assert!(
        s.contains(r#"<c r="B1"><f>_xlfn.LET(_xlpm.x,Data!E2,_xlpm.x*2)</f><v>384</v></c>"#),
        "{s}"
    );
    assert!(
        s.contains(r#"<c r="B2"><f>_xlfn.LET(_xlpm.t,A2,_xlpm.t+1)</f><v>123</v></c>"#),
        "{s}"
    );
    assert!(
        s.contains(
            r#"<c r="B3" t="str"><f>_xlfn.CONCAT("a","b")&amp;_xlfn.IFNA(1,2)</f><v>ab1</v></c>"#
        ),
        "{s}"
    );
    // a formula appeared: the calc chain no longer lists every formula cell
    assert!(report.calc_chain_dropped);
    assert!(!part(&dst, "[Content_Types].xml").contains("calcChain"));
    assert!(!part(&dst, "xl/_rels/workbook.xml.rels").contains("calcChain"));
}

#[test]
fn text_is_written_inline_and_never_parsed() {
    let (_d, src, dst, report) = run(&[
        value("Data", "A2", serde_json::json!("0025")),
        value("Data", "A4", serde_json::json!("=not a formula")),
        value("Data", "A5", serde_json::json!(" spaced & <odd> \u{1} ")),
    ]);
    let (rewritten, _, _) = fates(&src, &dst, &report);
    assert!(!rewritten.contains(&"xl/sharedStrings.xml".to_string()));
    let s = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(
        s.contains(r#"<c r="A2" t="inlineStr"><is><t>0025</t></is></c>"#),
        "{s}"
    );
    assert!(s.contains(r#"<c r="A4" t="inlineStr"><is><t>=not a formula</t></is></c>"#));
    assert!(s.contains(r#"<c r="A5" t="inlineStr"><is><t xml:space="preserve"> spaced &amp; &lt;odd&gt; _x0001_ </t></is></c>"#), "{s}");
    assert_eq!(
        cell(&dst, "Data", 5, 1).value,
        Value::Text(" spaced & <odd> \u{1} ".into())
    );
}

#[test]
fn the_three_way_check_skips_what_moved_and_reports_what_is_already_there() {
    let (_d, src, dst, report) = run(&[
        // expects 999 but the file holds 10: conflict, nothing written
        Edit::Value {
            sheet: "Data".into(),
            cell: "B2".into(),
            value: serde_json::json!(11),
            expect: Some(Expect::Value {
                value: serde_json::json!(999),
            }),
        },
        // already holds 4
        value("Data", "B3", serde_json::json!(4)),
        // the expected formula in another spelling still matches
        Edit::Formula {
            sheet: "Summary".into(),
            cell: "A2".into(),
            formula: "=SUM(Data!B2:B5)".into(),
            expect: Some(Expect::Formula {
                formula: "sum( Data!B2:B6 )".into(),
            }),
        },
        Edit::Formula {
            sheet: "Summary".into(),
            cell: "A4".into(),
            formula: "=1".into(),
            expect: Some(Expect::Formula {
                formula: "Data!C5".into(),
            }),
        },
    ]);
    let st: Vec<Status> = report.outcomes.iter().map(|o| o.status).collect();
    assert_eq!(
        st,
        vec![
            Status::Conflict,
            Status::AlreadyThere,
            Status::Applied,
            Status::Conflict
        ]
    );
    assert_eq!(report.outcomes[0].note, "Data!B2 now holds 10");
    assert_eq!(
        report.outcomes[3].note,
        "Summary!A4 now holds ='Data'!C4".replace("'Data'", "Data")
    );
    let (rewritten, _, _) = fates(&src, &dst, &report);
    assert_eq!(
        rewritten
            .iter()
            .filter(|p| p.contains("worksheets"))
            .count(),
        1,
        "{rewritten:?}"
    );
    assert!(part(&dst, "xl/worksheets/sheet2.xml")
        .contains(r#"<c r="A2"><f>SUM(Data!B2:B5)</f><v>121</v></c>"#));
}

#[test]
fn overwriting_a_shared_master_expands_its_group() {
    let (_d, src, dst, report) = run(&[value("Data", "D2", serde_json::json!(0))]);
    assert_eq!(report.expanded, 1);
    fates(&src, &dst, &report);
    let s = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(s.contains(r#"<c r="D2"><v>0</v></c>"#), "{s}");
    assert!(s.contains(r#"<c r="D3"><f>B3*C3</f><v>120</v></c>"#), "{s}");
    assert!(s.contains(r#"<c r="D6"><f>B6*C6</f><v>1</v></c>"#));
    assert!(!s.contains("t=\"shared\""));
}

#[test]
fn inserted_rows_ripple_everywhere() {
    let (_d, src, dst, report) = run(&[Edit::InsertRows {
        sheet: "Data".into(),
        at: 4,
        count: 2,
    }]);
    let (mut rewritten, removed, _) = fates(&src, &dst, &report);
    rewritten.sort();
    assert_eq!(
        rewritten,
        vec![
            "[Content_Types].xml",
            "xl/_rels/workbook.xml.rels",
            "xl/drawings/vmlDrawing1.vml",
            "xl/workbook.xml",
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml"
        ]
    );
    assert_eq!(removed, vec!["xl/calcChain.xml"]);
    let s = part(&dst, "xl/worksheets/sheet1.xml");
    // rows 2-3 stay, 4-6 move to 6-8, the shared group is expanded and moved
    assert!(s.contains(r#"<dimension ref="A1:G8"/>"#), "{s}");
    assert!(
        s.contains(r#"<c r="D3"><f>B3*C3</f><v>120</v></c></row><row r="6" spans="1:7">"#),
        "{s}"
    );
    assert!(s.contains(r#"<row r="6" spans="1:7"><c r="A6" t="s"><v>6</v></c><c r="B6"><v>100</v></c><c r="C6"><v>0.25</v></c><c r="D6"><f>B6*C6</f><v>25</v></c></row>"#), "{s}");
    assert!(s.contains(r#"<c r="E2"><f>SUM(D2:D8)</f><v>192</v></c>"#));
    assert!(s.contains(r#"<conditionalFormatting sqref="D2:D8">"#));
    assert!(s.contains(r#"sqref="B2:B8"><formula1>0</formula1>"#));
    assert!(s.contains(r#"<mergeCell ref="F1:G1"/>"#));
    let sum = part(&dst, "xl/worksheets/sheet2.xml");
    assert!(sum.contains(r#"<f>SUM(Data!B2:B8)</f>"#), "{sum}");
    assert!(sum.contains(r#"<f>'Data'!C6</f><v>0.25</v>"#), "{sum}");
    let wb = part(&dst, "xl/workbook.xml");
    assert!(wb.contains(r#"<definedName name="_xlnm.Print_Area" localSheetId="0">Data!$A$1:$G$8</definedName><definedName name="Prices">Data!$C$2:$C$8</definedName><definedName name="Total">Data!$E$2</definedName>"#), "{wb}");
    // the note sits on A3, above the insertion: its comment stays; its box
    // spans rows 2–6, so it grows with the rows (Excel's size-with-cells)
    assert!(!rewritten.iter().any(|p| p.contains("comments")));
    assert!(part(&dst, "xl/drawings/vmlDrawing1.vml").contains("<x:Anchor>1, 15, 1, 2, 3, 15, 7, 3</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>2</x:Row>"));
}

#[test]
fn deleted_rows_take_their_note_and_leave_ref_errors() {
    let (_d, src, dst, report) = run(&[Edit::DeleteRows {
        sheet: "Data".into(),
        at: 3,
        count: 2,
    }]);
    let (rewritten, _, _) = fates(&src, &dst, &report);
    assert!(
        rewritten.contains(&"xl/comments1.xml".to_string()),
        "{rewritten:?}"
    );
    assert!(rewritten.contains(&"xl/drawings/vmlDrawing1.vml".to_string()));
    assert!(!part(&dst, "xl/comments1.xml").contains("<comment "));
    assert!(!part(&dst, "xl/drawings/vmlDrawing1.vml").contains("<v:shape "));
    let s = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(
        s.contains(r#"<c r="E2"><f>SUM(D2:D4)</f><v>47</v></c>"#),
        "{s}"
    );
    assert!(
        s.contains(r#"<row r="3" spans="1:7"><c r="A3" t="s"><v>7</v></c>"#),
        "{s}"
    );
    // the selection pointed at B5 (deleted rows 3–4): it follows to B3
    assert!(s.contains(r#"activeCell="B3" sqref="B3""#), "{s}");
    let sum = part(&dst, "xl/worksheets/sheet2.xml");
    // Excel's own spelling of a deleted reference keeps the sheet
    assert!(
        sum.contains(r#"<c r="A4" t="e"><f>'Data'!#REF!</f><v>#REF!</v></c>"#),
        "{sum}"
    );
    assert!(
        sum.contains(r#"<c r="A2"><f>SUM(Data!B2:B4)</f><v>18</v></c>"#),
        "{sum}"
    );
}

#[test]
fn inserted_columns_move_cells_formulas_and_widths() {
    let (_d, src, dst, report) = run(&[Edit::InsertColumns {
        sheet: "Data".into(),
        at: "B".into(),
        count: 1,
    }]);
    fates(&src, &dst, &report);
    let s = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(s.contains(r#"<c r="E3"><f>C3*D3</f><v>120</v></c>"#), "{s}");
    assert!(s.contains(r#"<c r="F2"><f>SUM(E2:E6)</f>"#), "{s}");
    assert!(s.contains(r#"<cols><col min="1" max="1" width="20" customWidth="1"/><col min="3" max="4" width="9.140625" style="0"/></cols>"#), "{s}");
    assert!(s.contains(r#"<mergeCell ref="G1:H1"/>"#));
    assert!(part(&dst, "xl/workbook.xml").contains("Data!$D$2:$D$6"));
}

#[test]
fn a_new_sheet_names_widths_outlines_and_formats() {
    let (_d, src, dst, report) = run(&[
        Edit::AddSheet {
            name: "_Log".into(),
        },
        value("_Log", "A1", serde_json::json!("patched")),
        Edit::Formula {
            sheet: "_Log".into(),
            cell: "B2".into(),
            formula: "=Total+1".into(),
            expect: None,
        },
        Edit::SetName {
            name: "Grand".into(),
            formula: "=_Log!$B$2".into(),
            scope: None,
        },
        Edit::ColumnWidth {
            sheet: "Data".into(),
            columns: "E:F".into(),
            width: 14.5,
        },
        Edit::OutlineRows {
            sheet: "Data".into(),
            first: 3,
            last: 5,
            level: 1,
            hidden: false,
        },
        Edit::OutlineColumns {
            sheet: "Data".into(),
            columns: "C".into(),
            level: 1,
            hidden: false,
        },
        Edit::Fill {
            sheet: "Data".into(),
            area: "A1:B1".into(),
            rgb: Some("B4D2F0".into()),
        },
        Edit::Border {
            sheet: "Data".into(),
            area: "C2".into(),
            rgb: Some("0070C0".into()),
            style: Some(veredas_abaco::patch::LineStyle::Thin),
        },
    ]);
    let (rewritten, _, added) = fates(&src, &dst, &report);
    assert_eq!(added, vec!["xl/worksheets/sheet3.xml"]);
    assert!(rewritten.contains(&"xl/styles.xml".to_string()));
    let wb = part(&dst, "xl/workbook.xml");
    assert!(
        wb.contains(r#"<sheet name="_Log" sheetId="3" r:id="rId6"/></sheets>"#),
        "{wb}"
    );
    assert!(
        wb.contains(r#"<definedName name="Grand">_Log!$B$2</definedName>"#),
        "{wb}"
    );
    assert!(part(&dst, "[Content_Types].xml")
        .contains(r#"<Override PartName="/xl/worksheets/sheet3.xml""#));
    let log = part(&dst, "xl/worksheets/sheet3.xml");
    assert!(log.contains(r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>patched</t></is></c></row><row r="2"><c r="B2"><f>Total+1</f><v>193</v></c></row></sheetData>"#), "{log}");
    assert!(log.contains(r#"<dimension ref="A1:B2"/>"#));
    let s = part(&dst, "xl/worksheets/sheet1.xml");
    assert!(
        s.contains(r#"<col min="3" max="3" width="9.140625" style="0" outlineLevel="1"/>"#),
        "{s}"
    );
    assert!(
        s.contains(r#"<col min="5" max="6" width="14.5" customWidth="1"/>"#),
        "{s}"
    );
    assert!(
        s.contains(
            r#"<sheetFormatPr defaultRowHeight="15" outlineLevelRow="1" outlineLevelCol="1"/>"#
        ),
        "{s}"
    );
    assert!(
        s.contains(r#"<row r="3" spans="1:7" outlineLevel="1">"#),
        "{s}"
    );
    assert!(
        s.contains(r#"<c r="A1" s="2" t="s"><v>0</v></c><c r="B1" s="2" t="s">"#),
        "{s}"
    );
    assert!(s.contains(r#"<c r="C2" s="3"><v>2.5</v></c>"#), "{s}");
    let st = part(&dst, "xl/styles.xml");
    assert!(st.contains(r#"<cellXfs count="4">"#));
    assert!(st.contains(r#"<xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="0" applyFont="1" applyFill="1"/>"#), "{st}");
}

#[test]
fn the_destinations_previous_version_is_kept_and_the_source_never_written() {
    let dir = tempfile::tempdir().unwrap();
    let src = fixture(dir.path());
    let before = std::fs::read(&src).unwrap();
    let dst = dir.path().join("out.xlsx");
    std::fs::write(&dst, b"previous").unwrap();
    patch(
        &src,
        &dst,
        &[value("Data", "B2", serde_json::json!(1))],
        &PatchOptions::default(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("out.xlsx.bak")).unwrap(),
        b"previous"
    );
    assert_eq!(std::fs::read(&src).unwrap(), before);
    assert!(!dir.path().join(".out.xlsx.abaco-tmp").exists());
}

#[test]
fn a_formula_that_does_not_parse_fails_the_patch_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let src = fixture(dir.path());
    let dst = dir.path().join("out.xlsx");
    let err = patch(
        &src,
        &dst,
        &[Edit::Formula {
            sheet: "Data".into(),
            cell: "Z1".into(),
            formula: "=SUM(".into(),
            expect: None,
        }],
        &PatchOptions::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("does not parse"), "{err}");
    assert!(!dst.exists());
}

#[test]
fn deleted_columns_move_cells_formulas_widths_names_and_formats() {
    let (_d, src, dst, report) = run(&[Edit::DeleteColumns {
        sheet: "Data".into(),
        at: "B".into(),
        count: 1,
    }]);
    fates(&src, &dst, &report);
    let s = part(&dst, "xl/worksheets/sheet1.xml");
    // Price moved from C to B; the products read the deleted quantity
    assert!(s.contains(r#"<c r="B3"><v>30</v></c>"#), "{s}");
    assert!(s.contains(r#"<c r="D2" t="e"><f>SUM(C2:C6)</f><v>#REF!</v></c>"#), "{s}");
    assert!(s.contains(r#"<f>#REF!*B3</f>"#), "{s}");
    assert!(s.contains(r#"<cols><col min="1" max="1" width="20" customWidth="1"/><col min="2" max="2" width="9.140625" style="0"/></cols>"#), "{s}");
    assert!(s.contains(r#"<mergeCell ref="E1:F1"/>"#), "{s}");
    assert!(s.contains(r#"<conditionalFormatting sqref="C2:C6"><cfRule type="expression" dxfId="0" priority="1"><formula>C2&gt;100</formula>"#), "{s}");
    // the validation covered the deleted column only: it goes
    assert!(!s.contains("<dataValidation "), "{s}");
    let wb = part(&dst, "xl/workbook.xml");
    assert!(wb.contains(r#"<definedName name="Prices">Data!$B$2:$B$6</definedName>"#), "{wb}");
    assert!(wb.contains(r#"<definedName name="_xlnm.Print_Area" localSheetId="0">Data!$A$1:$F$6</definedName>"#), "{wb}");
    let sum = part(&dst, "xl/worksheets/sheet2.xml");
    // a range deleted whole is `#REF!`, its sheet kept, as Excel writes it
    assert!(sum.contains(r#"<f>SUM(Data!#REF!)</f><v>#REF!</v>"#), "{sum}");
    assert!(sum.contains(r#"<c r="A4"><f>'Data'!B4</f><v>0.25</v></c>"#), "{sum}");
}

#[test]
fn a_cleared_area_loses_its_contents_and_what_reads_it_follows() {
    let (_d, src, dst, report) = run(&[Edit::Clear {
        sheet: "Data".into(),
        area: "B2:C3".into(),
    }]);
    fates(&src, &dst, &report);
    let s = part(&dst, "xl/worksheets/sheet1.xml");
    for gone in ["B2", "C2", "B3", "C3"] {
        assert!(!s.contains(&format!(r#"<c r="{gone}""#)), "{gone} in {s}");
    }
    assert_eq!(cell(&dst, "Data", 2, 4).value, Value::Number(0.0));
    assert_eq!(cell(&dst, "Data", 2, 5).value, Value::Number(47.0));
    assert_eq!(cell(&dst, "Summary", 2, 1).value, Value::Number(108.0));
    // what the clear did not reach keeps its bytes
    assert!(s.contains(r#"<c r="B4"><v>100</v></c>"#), "{s}");
}

#[test]
fn a_name_deleted_and_a_local_name_set() {
    let (_d, src, dst, report) = run(&[
        Edit::DeleteName {
            name: "Total".into(),
            scope: None,
        },
        Edit::SetName {
            name: "Rate".into(),
            formula: "=Data!$C$2".into(),
            scope: Some("Summary".into()),
        },
    ]);
    let (rewritten, _, _) = fates(&src, &dst, &report);
    assert!(rewritten.contains(&"xl/workbook.xml".to_string()));
    let wb = part(&dst, "xl/workbook.xml");
    assert!(!wb.contains(r#"name="Total""#), "{wb}");
    assert!(wb.contains(r#"<definedName name="Prices">Data!$C$2:$C$6</definedName>"#), "{wb}");
    assert!(wb.contains(r#"<definedName name="Rate" localSheetId="1">Data!$C$2</definedName>"#), "{wb}");
    // the formula that read the deleted name: its value moved, to Excel's #NAME?
    let sum = part(&dst, "xl/worksheets/sheet2.xml");
    assert!(sum.contains(r#"<c r="A3" t="e"><f>Total*2</f><v>#NAME?</v></c>"#), "{sum}");
}

#[test]
fn full_calc_on_load_is_set_when_asked_and_only_then() {
    let dir = tempfile::tempdir().unwrap();
    let src = fixture(dir.path());
    for (full, calc) in [(true, r#"<calcPr calcId="191029" fullCalcOnLoad="1"/>"#), (false, r#"<calcPr calcId="191029"/>"#)] {
        let dst = dir.path().join(format!("out-{full}.xlsx"));
        let options = PatchOptions {
            verify: true,
            full_calc_on_load: full,
        };
        patch(&src, &dst, &[value("Data", "B3", serde_json::json!(6))], &options).unwrap();
        assert!(part(&dst, "xl/workbook.xml").contains(calc), "{full}");
    }
}
