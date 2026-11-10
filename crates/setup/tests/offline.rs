use kassiber_envelope::{dict, get, s};
use kassiber_paths::{Context, Platform};
use kassiber_pyjson::Value;
use kassiber_setup::offline;
use std::{collections::BTreeMap, fs};

struct Fixture {
    root: tempfile::TempDir,
    context: Context,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let context = Context {
            platform: Platform::Linux,
            home: root.path().into(),
            environ: BTreeMap::new(),
        };
        Self { root, context }
    }
    fn path(&self) -> std::path::PathBuf {
        offline::preference_path(&self.context).unwrap()
    }
    fn run(&self, command: &str, args: Value) -> Value {
        offline::execute(&self.context, command, &args).unwrap()
    }
    fn switch(&self, command: &str) -> Value {
        self.run(command, dict::<&str>([]))
    }
}

#[test]
fn offline_missing_read_is_online_and_creates_nothing() {
    let f = Fixture::new();
    assert_eq!(
        f.switch("offline.status"),
        dict([
            ("offline", Value::Bool(false)),
            ("environment_blocked", Value::Bool(false))
        ])
    );
    assert_eq!(fs::read_dir(f.root.path()).unwrap().count(), 0);
    let parent = f.root.path().join("file");
    fs::write(&parent, b"").unwrap();
    assert!(!offline::enabled(&parent.join("child")));
    assert!(!offline::enabled(&f.root.path().join("nul\0")));
}

#[test]
fn offline_preference_round_trips_private_bytes_and_is_shared_with_desktop() {
    let f = Fixture::new();
    assert_eq!(
        get(&f.switch("offline.on"), "offline"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        fs::read_to_string(f.path()).unwrap(),
        "{\"enabled\": true, \"schema_version\": 1}\n"
    );
    assert_eq!(
        get(&f.run("ui.network.offline", dict::<&str>([])), "offline"),
        Some(&Value::Bool(true))
    );
    f.run(
        "ui.network.offline.set",
        dict([("enabled", Value::Bool(false))]),
    );
    assert_eq!(
        get(&f.switch("offline.status"), "offline"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        fs::read_to_string(f.path()).unwrap(),
        "{\"enabled\": false, \"schema_version\": 1}\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(f.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(f.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[test]
fn offline_malformed_or_oversized_preferences_fail_closed() {
    let f = Fixture::new();
    fs::create_dir_all(f.path().parent().unwrap()).unwrap();
    for raw in [
        b"not json".as_slice(),
        b"[]",
        b"null",
        b"{}",
        b"\xff",
        b"\xef\xbb\xbf{}",
        b"{\"schema_version\": 2, \"enabled\": false}",
        b"{\"schema_version\": 1, \"enabled\": 0}",
        b"{\"schema_version\": 1, \"enabled\": \"false\"}",
        &vec![b' '; 4097],
    ] {
        fs::write(f.path(), raw).unwrap();
        assert!(offline::enabled(&f.path()), "{raw:?}");
    }
    for version in ["1", "1.0", "true"] {
        fs::write(
            f.path(),
            format!("{{\"schema_version\":{version},\"enabled\":false}}"),
        )
        .unwrap();
        assert!(!offline::enabled(&f.path()));
    }
    fs::remove_file(f.path()).unwrap();
    fs::create_dir(f.path()).unwrap();
    assert!(offline::enabled(&f.path()));
}

#[test]
fn offline_environment_override_does_not_replace_the_users_switch() {
    let mut f = Fixture::new();
    for (raw, blocked) in [
        ("1", true),
        ("\u{a0}TrUe\u{1c}", true),
        ("yes", true),
        ("on", true),
        ("0", false),
        ("false", false),
        ("enable", false),
    ] {
        f.context
            .environ
            .insert("KASSIBER_NO_EGRESS".into(), raw.into());
        let v = f.switch("offline.off");
        assert_eq!(get(&v, "environment_blocked"), Some(&Value::Bool(blocked)));
        assert_eq!(get(&v, "offline"), Some(&Value::Bool(false)));
        let v = f.switch("offline.on");
        assert_eq!(get(&v, "environment_blocked"), Some(&Value::Bool(blocked)));
        assert_eq!(get(&v, "offline"), Some(&Value::Bool(true)));
    }
}

#[test]
fn offline_desktop_validation_is_strict_and_never_writes_on_error() {
    let f = Fixture::new();
    for args in [
        dict::<&str>([]),
        dict([("enabled", s("true"))]),
        dict([("enabled", Value::Int(1.into()))]),
        dict([("enabled", Value::Null)]),
    ] {
        let error = offline::execute(&f.context, "ui.network.offline.set", &args).unwrap_err();
        assert_eq!(error.code, "validation");
        assert_eq!(
            error.message,
            "ui.network.offline.set requires a boolean enabled"
        );
        assert!(!f.path().exists());
    }
    for kind in ["ui.network.offline", "ui.network.offline.set"] {
        let error = offline::execute(
            &f.context,
            kind,
            &dict([("z", Value::Null), ("a", Value::Null)]),
        )
        .unwrap_err();
        assert_eq!(
            error.details,
            Some(dict([("unknown", Value::List(vec![s("a"), s("z")]))]))
        );
        assert!(!f.path().exists());
    }
}

#[test]
fn offline_path_override_uses_machine_home_and_preserves_metadata_permission_policy() {
    let mut f = Fixture::new();
    f.context.environ.insert(
        "KASSIBER_OFFLINE_PREFERENCE_FILE".into(),
        "~/custom/offline.json".into(),
    );
    assert_eq!(f.path(), f.root.path().join("custom/offline.json"));
    f.switch("offline.off");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(f.path(), fs::Permissions::from_mode(0o666)).unwrap();
        // Python's preference reader checks shape, not metadata ownership/mode.
        assert!(!offline::enabled(&f.path()));
        f.switch("offline.on");
        assert_eq!(
            fs::metadata(f.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn offline_symlinks_fail_closed_and_explicit_write_replaces_only_the_link() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let target = f.root.path().join("target");
    let bytes = b"{\"schema_version\": 1, \"enabled\": false}";
    fs::write(&target, bytes).unwrap();
    fs::create_dir_all(f.path().parent().unwrap()).unwrap();
    symlink(&target, f.path()).unwrap();
    assert!(offline::enabled(&f.path()));
    f.switch("offline.off");
    assert!(!fs::symlink_metadata(f.path())
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&target).unwrap(), bytes);
    fs::remove_file(f.path()).unwrap();
    symlink(f.root.path().join("missing"), f.path()).unwrap();
    assert!(offline::enabled(&f.path()));
    let looping = f.root.path().join("loop");
    symlink("loop", &looping).unwrap();
    assert!(offline::enabled(&looping));
    assert!(!offline::enabled(&looping.join("child")));
}
