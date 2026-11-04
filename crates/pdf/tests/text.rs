//! The text writer against Python's `pdf_report` vectors.
use kassiber_pdf::text;
use serde_json::Value;

fn data() -> Value {
    serde_json::from_str(include_str!("data/text_vectors.json")).unwrap()
}
fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}
fn numbers(v: &Value) -> Vec<usize> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap() as usize)
        .collect()
}

#[test]
fn wrapping_matches_python_textwrap() {
    let data = data();
    for case in data["wrap"].as_array().unwrap() {
        let lines = strings(&case["lines"]);
        assert_eq!(
            text::wrap_report_lines(&lines),
            strings(&case["wrapped"]),
            "{lines:?}"
        );
    }
}

#[test]
fn tables_match_python_format_table() {
    let data = data();
    for case in data["tables"].as_array().unwrap() {
        let rows: Vec<Vec<String>> = case["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(strings)
            .collect();
        assert_eq!(
            text::format_table(
                &strings(&case["headers"]),
                &rows,
                &numbers(&case["widths"]),
                &numbers(&case["right"]),
            ),
            strings(&case["lines"]),
            "{case}"
        );
    }
}

#[test]
fn documents_match_python_bytes_and_pages() {
    let data = data();
    for case in data["documents"].as_array().unwrap() {
        let title = case["title"].as_str().unwrap();
        let rendered = text::render(title, &strings(&case["lines"]));
        let expected: Vec<u8> = case["pdf"]
            .as_str()
            .unwrap()
            .chars()
            .map(|c| c as u8)
            .collect();
        assert!(rendered.bytes == expected, "{title}");
        assert_eq!(
            rendered.pages as u64,
            case["pages"].as_u64().unwrap(),
            "{title}"
        );
    }
}
