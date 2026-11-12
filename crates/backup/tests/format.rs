use kassiber_backup::{validate, DATABASE};
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::{dumps, Options, Value};
use std::fs;
use std::path::Path;

fn manifest() -> Value {
    dict([
        ("schema_version", Value::from(1)),
        (
            "entries",
            dict([
                ("database", s(DATABASE)),
                ("attachments_files", Value::from(0)),
                ("backends_env", Value::Bool(false)),
                ("settings_json", Value::Bool(false)),
            ]),
        ),
    ])
}
fn write(root: &Path, manifest: &Value) {
    fs::write(
        root.join("manifest.json"),
        dumps(manifest, &Options::default()).unwrap(),
    )
    .unwrap();
}

#[test]
fn manifest_paths_schema_counts_and_declared_sidecars_match_python() {
    use kassiber_envelope::set;
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        validate(root.path()).unwrap_err().message,
        "backup is missing manifest.json"
    );
    let mut m = manifest();
    write(root.path(), &m);
    assert_eq!(
        validate(root.path()).unwrap_err().message,
        "decrypted backup is missing the SQLCipher database"
    );
    // Import validates archive structure, not the database codec or its bytes.
    fs::write(root.path().join(DATABASE), b"arbitrary database bytes").unwrap();
    validate(root.path()).unwrap();
    for schema in [Value::Null, Value::from(2), s("1")] {
        set(&mut m, "schema_version", schema);
        write(root.path(), &m);
        assert!(validate(root.path())
            .unwrap_err()
            .message
            .starts_with("unsupported backup manifest schema_version"));
    }
    for schema in [Value::Bool(true), Value::Float(1.0), Value::from(1)] {
        set(&mut m, "schema_version", schema);
        write(root.path(), &m);
        validate(root.path()).unwrap();
    }
    let mut entries = get(&m, "entries").unwrap().clone();
    set(&mut entries, "database", s("attachments/database"));
    set(&mut m, "entries", entries.clone());
    write(root.path(), &m);
    assert!(validate(root.path())
        .unwrap_err()
        .message
        .starts_with("manifest declares unexpected database path"));
    set(&mut entries, "database", s(DATABASE));
    for count in [
        Value::Bool(false),
        Value::Float(0.0),
        Value::from(-1),
        Value::Null,
        s("0"),
    ] {
        set(&mut entries, "attachments_files", count);
        set(&mut m, "entries", entries.clone());
        write(root.path(), &m);
        assert_eq!(
            validate(root.path()).unwrap_err().message,
            "manifest declares an invalid attachments_files count"
        );
    }
    set(&mut entries, "attachments_files", Value::from(1));
    set(&mut m, "entries", entries.clone());
    write(root.path(), &m);
    assert_eq!(
        validate(root.path()).unwrap_err().details,
        Some(dict([
            ("declared", Value::from(1)),
            ("actual", Value::from(0))
        ]))
    );
    fs::create_dir_all(root.path().join("attachments/a")).unwrap();
    fs::write(
        root.path().join("attachments/a/evidence"),
        b"counted without hashes",
    )
    .unwrap();
    validate(root.path()).unwrap();
    for (field, name) in [
        ("backends_env", "config/backends.env"),
        ("settings_json", "config/settings.json"),
    ] {
        set(&mut entries, field, Value::Bool(true));
        set(&mut m, "entries", entries.clone());
        write(root.path(), &m);
        assert_eq!(
            validate(root.path()).unwrap_err().message,
            format!("manifest declares {field} but the file is missing from the archive")
        );
        fs::create_dir_all(root.path().join("config")).unwrap();
        fs::write(root.path().join(name), b"preserved").unwrap();
        validate(root.path()).unwrap();
    }
    fs::write(
        root.path().join("attachments/a/evidence"),
        b"changed same count",
    )
    .unwrap();
    validate(root.path()).unwrap(); // This is the reference's count-only contract.
}

#[test]
fn export_plaintext_refusal_keeps_the_python_message_hint_and_destination() {
    let root = tempfile::tempdir().unwrap();
    let context = kassiber_paths::Context::from_env().unwrap();
    let paths = context
        .resolve_for_bootstrap(&kassiber_paths::ResolveOptions {
            data_root: Some(root.path().join("data")),
            ..Default::default()
        })
        .unwrap();
    fs::create_dir_all(&paths.data_root).unwrap();
    let conn = kassiber_store::rusqlite::Connection::open(&paths.database).unwrap();
    conn.execute("CREATE TABLE retained(value)", []).unwrap();
    drop(conn);
    let output = root.path().join("backup.kassiber");
    fs::write(&output, b"old ciphertext").unwrap();
    let error = match kassiber_backup::export(&paths, &output, None, Some("outer"), None, "test") {
        Err(e) => e,
        Ok(_) => panic!("plaintext export succeeded"),
    };
    assert_eq!(error.code, "plaintext_database");
    assert_eq!(
        error.message,
        format!(
            "refusing to back up plaintext database at {}",
            paths.database.display()
        )
    );
    assert_eq!(
        error.hint.as_deref(),
        Some("Run `kassiber secrets init` first.")
    );
    assert!(!error.retryable);
    assert_eq!(fs::read(output).unwrap(), b"old ciphertext");
}
