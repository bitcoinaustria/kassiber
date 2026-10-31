use kassiber_xlsx::{Cell, Document, Operation, Sheet, Style};
use std::io::{Cursor, Read};

fn member(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    out
}

fn document() -> Document {
    Document {
        title: "Exact workbook".into(),
        author: "Kassiber".into(),
        created: "2026-01-02T03:04:05Z".into(),
        formats: vec![
            Style::default(),
            Style {
                bold: true,
                num_format: Some("#,##0.00".into()),
                ..Style::default()
            },
        ],
        names: vec![("Check".into(), "='Data €'!$C$3".into())],
        sheets: vec![Sheet {
            name: "Data €".into(),
            operations: vec![
                Operation::Row {
                    row: 0,
                    height: 28.0,
                },
                Operation::Column {
                    first: 0,
                    last: 2,
                    width: 22.0,
                },
                Operation::Merge {
                    first_row: 0,
                    first_col: 0,
                    last_row: 0,
                    last_col: 2,
                    text: "Header €".into(),
                    style: 1,
                },
                Operation::Freeze { row: 2, col: 0 },
                Operation::Cell {
                    row: 2,
                    col: 0,
                    value: Cell::Number(12346.802358123685),
                    style: 1,
                },
                Operation::Cell {
                    row: 2,
                    col: 1,
                    value: Cell::Boolean(true),
                    style: 0,
                },
                Operation::Cell {
                    row: 2,
                    col: 2,
                    value: Cell::Formula {
                        expression: "=A3/2".into(),
                        result: "6173.4011790618425".into(),
                    },
                    style: 1,
                },
                Operation::Cell {
                    row: 3,
                    col: 0,
                    value: Cell::String("=1+1".into()),
                    style: 0,
                },
                Operation::Cell {
                    row: 3,
                    col: 1,
                    value: Cell::Number(1e100),
                    style: 0,
                },
                Operation::Cell {
                    row: 3,
                    col: 2,
                    value: Cell::Blank,
                    style: 1,
                },
            ],
        }],
        ..Document::default()
    }
}

#[test]
fn typed_values_precision_and_formula_caches_are_preserved() {
    let bytes = kassiber_xlsx::render(&document()).unwrap();
    let sheet = member(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<v>12346.80235812368</v>"), "{sheet}");
    assert!(sheet.contains("t=\"b\"><v>1</v>"), "{sheet}");
    assert!(
        sheet.contains("<f>A3/2</f><v>6173.4011790618425</v>"),
        "{sheet}"
    );
    assert!(sheet.contains("<v>1E+100</v>"), "{sheet}");
    assert!(
        sheet.contains("r=\"C4\" s=\"1\"/>"),
        "styled blank lost: {sheet}"
    );
    assert!(member(&bytes, "xl/sharedStrings.xml").contains("<t>=1+1</t>"));
}

#[test]
fn meaningful_layout_and_fixed_properties_are_preserved() {
    let bytes = kassiber_xlsx::render(&document()).unwrap();
    let sheet = member(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("ht=\"28\""), "{sheet}");
    assert!(sheet.contains("width=\"22.7109375\""), "{sheet}");
    assert!(sheet.contains("mergeCell ref=\"A1:C1\""), "{sheet}");
    assert!(sheet.contains("topLeftCell=\"A3\""), "{sheet}");
    assert!(member(&bytes, "xl/styles.xml").contains("<b/>"));
    assert!(member(&bytes, "xl/styles.xml").contains("formatCode=\"#,##0.00\""));
    assert!(member(&bytes, "xl/workbook.xml").contains("name=\"Check\">'Data €'!$C$3"));
    let core = member(&bytes, "docProps/core.xml");
    assert_eq!(core.matches("2026-01-02T03:04:05Z").count(), 2, "{core}");
}

#[test]
fn overwritten_numbers_do_not_rewrite_later_cell_types() {
    let mut spec = document();
    spec.sheets[0].operations.push(Operation::Cell {
        row: 2,
        col: 0,
        value: Cell::String("Replacement".into()),
        style: 0,
    });
    let bytes = kassiber_xlsx::render(&spec).unwrap();
    let sheet = member(&bytes, "xl/worksheets/sheet1.xml");
    // A string refers to sharedStrings, never to the old numeric value.
    assert!(!sheet.contains("12346.80235812368"));
}

#[test]
fn empty_strings_keep_their_cell_type() {
    let mut spec = document();
    spec.sheets[0].operations.push(Operation::Cell {
        row: 4,
        col: 0,
        value: Cell::String(String::new()),
        style: 1,
    });
    let bytes = kassiber_xlsx::render(&spec).unwrap();
    let sheet = member(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet.contains("r=\"A5\" s=\"1\" t=\"inlineStr\"><is><t></t></is></c>"),
        "{sheet}"
    );
}
