use kassiber_diagnostics::report;
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
#[test]
fn every_argument_class_matches_python() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!("arguments.json")).unwrap();
    for fixture in fixtures.as_array().unwrap() {
        let args = kassiber_pytext::loads(&fixture["input"].to_string()).unwrap();
        let report = report::collect(
            &args,
            "test",
            "2026-10-29T12:34:56Z",
            None,
            dict([("database_integrity", s("unavailable"))]),
            None,
        );
        let actual = dumps(get(&report, "invocation").unwrap(), &Options::default()).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
            fixture["expected"]
        );
    }
}
#[test]
fn native_error_and_panic_contexts_never_export_home_usernames_or_frames() {
    for context in [
        "error at /home/planted-user/private-build/src/main.rs:123",
        "thread panicked at /Users/planted-user/private-build/src/main.rs:456",
        r"panic at C:\Users\planted-user\private-build\main.rs:789",
    ] {
        let error = AppError::new("validation", context)
            .with_details(dict([("path", s(context)), ("safe", s(context))]));
        let report = report::collect(
            &dict([("command", s("diagnostics"))]),
            "test",
            "2026-10-29T12:34:56Z",
            None,
            dict([("database_integrity", s("unavailable"))]),
            Some(&error),
        );
        let bytes = dumps(&report, &Options::default()).unwrap();
        assert_eq!(get(&report, "stack"), Some(&Value::List(vec![])));
        for private in [
            "planted-user",
            "private-build",
            "/home/",
            "/Users/",
            r"C:\Users",
        ] {
            assert!(!bytes.contains(private), "{bytes}");
        }
    }
}
#[test]
fn explicit_save_preserves_python_layout_umask_and_existing_inode() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nested/report.json");
    let args = dict::<&str>([]);
    let report = dict([
        ("generated_at", s("2026-10-29T12:34:56Z")),
        ("unicode", s("é")),
    ]);
    let saved = report::save(&report, path.to_str().unwrap(), &args)
        .unwrap()
        .unwrap();
    assert_eq!(saved.target, "custom");
    assert_eq!(saved.relative_path, None);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\n  \"generated_at\": \"2026-10-29T12:34:56Z\",\n  \"unicode\": \"\\u00e9\"\n}\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let inode = std::fs::metadata(&path).unwrap().ino();
        report::save(&report, path.to_str().unwrap(), &args).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(metadata.ino(), inode);
        assert_eq!(metadata.mode() & 0o777, 0o600);
    }
    assert!(report::save(&report, "", &args).unwrap().is_none());
}
#[test]
fn native_debug_preserves_presence_type_and_position_without_exporting_frames() {
    assert_eq!(report::native_debug(false, false), None);
    assert_eq!(report::native_debug(true, false), None);
    for (unhandled, requested) in [(true, true), (false, true)] {
        assert_eq!(report::native_debug(unhandled, requested), Some(""));
        let envelope = AppError::new("internal_error", "failure")
            .envelope(report::native_debug(unhandled, requested));
        let bytes = dumps(&envelope, &Options::default()).unwrap();
        assert!(bytes.ends_with("\"retryable\": false, \"debug\": \"\"}}"));
        for planted in [
            "/home/planted-user",
            "/Users/planted-user",
            "private-build",
            r"C:\Users\planted-user",
        ] {
            assert!(!bytes.contains(planted));
        }
    }
}
