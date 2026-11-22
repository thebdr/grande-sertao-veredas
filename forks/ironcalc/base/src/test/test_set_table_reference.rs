#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;
use crate::types::{Table, TableColumn, TableStyleInfo};

/// A table `T` on Sheet1 at `reference`, a header row, the columns `Name` and `Qty`.
fn table(reference: &str) -> Table {
    Table {
        name: "T".to_string(),
        display_name: "T".to_string(),
        sheet_name: "Sheet1".to_string(),
        reference: reference.to_string(),
        totals_row_count: 0,
        header_row_count: 1,
        header_row_dxf_id: None,
        data_dxf_id: None,
        totals_row_dxf_id: None,
        columns: ["Name", "Qty"]
            .iter()
            .enumerate()
            .map(|(id, name)| TableColumn {
                id: id as u32,
                name: name.to_string(),
                ..Default::default()
            })
            .collect(),
        style_info: TableStyleInfo::default(),
        has_filters: false,
    }
}

#[test]
fn a_moved_table_is_read_where_it_is_by_formulas_parsed_after() {
    let mut model = new_empty_model();
    model._set("A1", "Name");
    model._set("B1", "Qty");
    model._set("B2", "1");
    model._set("B3", "2");
    model._set("B4", "4");
    model.workbook.tables.insert("T".to_string(), table("A1:B2"));
    // the parser learns of the table through the setter
    model.set_table_reference("T", "A1:B3").unwrap();
    model._set("D1", "=SUM(T[Qty])");
    model.evaluate();
    assert_eq!(model._get_text("D1"), *"3");
    // grown by a row: a formula parsed now reads three rows, one parsed before keeps its cells
    model.set_table_reference("T", "A1:B4").unwrap();
    model._set("D2", "=SUM(T[Qty])");
    model.evaluate();
    assert_eq!(model._get_text("D2"), *"7");
    assert_eq!(model._get_text("D1"), *"3");
    assert_eq!(model.workbook.tables["T"].reference, "A1:B4");
    // an unknown table, a reference that is no range
    assert!(model.set_table_reference("U", "A1:B4").is_err());
    assert!(model.set_table_reference("T", "not a range").is_err());
    assert_eq!(model.workbook.tables["T"].reference, "A1:B4");
}
