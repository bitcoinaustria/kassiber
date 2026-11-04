//! The sealed Typst world: deterministic, with only embedded sources, fonts
//! and in-memory files.
use kassiber_pdf::engine::{covered, render, Document, Stamp, REPORT};
use kassiber_pyjson::Value;

fn s(text: &str) -> Value {
    Value::Str(text.into())
}
fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Dict(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn stamp() -> Stamp {
    Stamp::parse("2026-01-02T03:04:05Z").unwrap()
}

/// A minimal document for the shared design: one section of facts.
fn data(value: &str) -> Value {
    obj(vec![
        (
            "meta",
            obj(vec![
                ("title", s("Summary report")),
                ("kicker", s("Kassiber")),
                ("running_title", s("Kassiber · Summary report")),
                ("book", s("Main / Book")),
                ("period", s("2024-01-01 to 2025-12-31")),
                (
                    "footer",
                    s("Generated 2026-01-02 03:04 UTC  ·  journal input v7"),
                ),
                ("page_format", s("Page {page} of {total}")),
                ("lang", s("en")),
                ("keywords", Value::List(vec![s("kassiber")])),
                (
                    "date",
                    Value::List(
                        [2026, 1, 2, 3, 4, 5]
                            .map(|v: i64| Value::Int(v.into()))
                            .to_vec(),
                    ),
                ),
            ]),
        ),
        (
            "sections",
            Value::List(vec![obj(vec![
                ("id", s("scope")),
                ("title", s("Scope")),
                ("lead", s("")),
                ("page_break", Value::Bool(false)),
                (
                    "blocks",
                    Value::List(vec![obj(vec![
                        ("t", s("facts")),
                        (
                            "rows",
                            Value::List(vec![obj(vec![
                                ("label", s("Value")),
                                ("value", s(value)),
                                ("tone", s("ink")),
                            ])]),
                        ),
                    ])]),
                ),
            ])]),
        ),
    ])
}

#[test]
fn reports_render_deterministically_with_bundled_fonts() {
    // ₿ comes from the fallback font; a missing glyph would be a warning,
    // which the engine refuses.
    let first = render(Document {
        template: REPORT,
        data: data("₿ 1.00000000 → ä €"),
        files: vec![],
        created: stamp(),
    })
    .unwrap();
    let second = render(Document {
        template: REPORT,
        data: data("₿ 1.00000000 → ä €"),
        files: vec![],
        created: stamp(),
    })
    .unwrap();
    assert_eq!(first, second);
    assert!(first.bytes.starts_with(b"%PDF-"));
    assert_eq!(first.pages.len(), 1);
    let text = String::from_utf8_lossy(&first.bytes);
    assert!(
        text.contains("DMSans") && !text.contains("JetBrainsMono"),
        "the facts block is set in DM Sans alone"
    );
    assert!(
        text.contains("2026-01-02"),
        "the report date is the metadata date"
    );
}

#[test]
fn labels_in_other_scripts_never_render_as_missing_glyphs() {
    // Greek and Cyrillic come from Liberation Sans; a script no bundled
    // font draws (CJK, hieroglyphs) becomes U+FFFD before layout.
    for c in ['Κ', 'ώ', 'Ж', 'ё', '₿', '\u{fffd}'] {
        assert!(covered(c), "{c}");
    }
    for c in ['冷', '\u{13000}'] {
        assert!(!covered(c), "{c}");
    }
    let rendered = render(Document {
        template: REPORT,
        data: data("Κρύο πορτοφόλι · Холодный кошелёк · 冷钱包 \u{13000}"),
        files: vec![],
        created: stamp(),
    });
    assert!(rendered.is_ok(), "{rendered:?}");
}

#[test]
fn the_world_refuses_files_packages_and_unknown_fonts() {
    for template in [
        "#read(\"/etc/hostname\")",
        "#read(\"report.typ\", encoding: none).len()\n#image(\"/etc/hostname\")",
        "#import \"@preview/cetz:0.3.0\": *",
        "#image(\"missing.svg\")",
        "#text(font: \"DejaVu Sans\")[x]",
    ] {
        let error = render(Document {
            template,
            data: Value::Dict(vec![]),
            files: vec![],
            created: stamp(),
        })
        .unwrap_err();
        assert!(!error.is_empty(), "{template}");
    }
}

#[test]
fn in_memory_files_are_the_only_files() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#;
    let rendered = render(Document {
        template: "#image(\"chart.svg\")",
        data: Value::Dict(vec![]),
        files: vec![("chart.svg".into(), svg.to_vec())],
        created: stamp(),
    });
    assert!(rendered.is_ok(), "{rendered:?}");
}
