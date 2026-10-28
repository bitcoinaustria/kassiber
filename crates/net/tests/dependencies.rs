//! Pin the reviewed ASCII-only URL dependency surface, not just URL behavior.
#[test]
fn ascii_url_dependencies_do_not_restore_icu_or_unicode_mapping() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "--package",
            "kassiber-net",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--locked",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8(output.stdout).unwrap();
    assert!(tree
        .lines()
        .any(|line| line.starts_with("idna_adapter v1.0.")));
    for line in tree.lines() {
        let name = line.split_whitespace().next().unwrap_or_default();
        assert!(
            !name.starts_with("icu_")
                && ![
                    "idna_mapping",
                    "unicode-normalization",
                    "unicode-bidi",
                    "yoke",
                    "zerovec",
                    "zerofrom",
                    "zerotrie"
                ]
                .contains(&name),
            "unreviewed Unicode/ICU URL dependency: {name}"
        );
    }
}
