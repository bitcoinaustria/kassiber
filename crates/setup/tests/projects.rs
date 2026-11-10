use kassiber_envelope::{dict, get, s};
use kassiber_paths::{Context, Platform};
use kassiber_pyjson::Value;
use kassiber_setup::projects;
use std::collections::BTreeMap;

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
    fn run(&self, command: &str, args: Value) -> Value {
        projects::execute(&self.context, command, &args).unwrap()
    }
    fn create(&self, name: &str, no_select: bool) -> Value {
        self.run(
            "projects.create",
            dict([("name", s(name)), ("no_select", Value::Bool(no_select))]),
        )
    }
    fn catalog(&self) -> std::path::PathBuf {
        self.context
            .default_state_root()
            .unwrap()
            .join("config/projects.json")
    }
}
fn field(value: &Value, key: &str) -> String {
    kassiber_envelope::py_str(get(value, key).unwrap())
}

#[test]
fn project_list_on_new_install_does_not_create_a_default_project() {
    let f = Fixture::new();
    let result = f.run("projects.list", dict::<&str>([]));
    assert_eq!(get(&result, "selected_project_id"), Some(&Value::Null));
    assert_eq!(get(&result, "projects"), Some(&Value::List(vec![])));
    assert!(!f.catalog().exists());
}
#[test]
fn create_preserves_catalog_order_and_no_select_keeps_selection() {
    let f = Fixture::new();
    f.create("Alpha", false);
    f.create("Beta", true);
    let result = f.run("projects.list", dict::<&str>([]));
    assert_eq!(get(&result, "selected_project_id"), Some(&s("alpha")));
    let Some(Value::List(entries)) = get(&result, "projects") else {
        panic!()
    };
    assert_eq!(field(&entries[0], "id"), "alpha");
    assert_eq!(field(&entries[1], "id"), "beta");
    assert_eq!(get(&entries[0], "selected"), Some(&Value::Bool(true)));
    assert_eq!(get(&entries[1], "selected"), Some(&Value::Bool(false)));
    assert!(!std::path::Path::new(&field(&entries[0], "database")).exists());
}
#[test]
fn first_no_select_project_still_establishes_a_selection() {
    let f = Fixture::new();
    f.create("Alpha", true);
    let persisted = kassiber_pytext::loads(&std::fs::read_to_string(f.catalog()).unwrap()).unwrap();
    assert_eq!(get(&persisted, "selected_project_id"), Some(&s("alpha")));
    assert_eq!(
        get(
            &f.run("projects.list", dict::<&str>([])),
            "selected_project_id"
        ),
        Some(&s("alpha"))
    );
}
#[test]
fn explicit_list_selection_does_not_change_saved_selection() {
    let f = Fixture::new();
    f.create("Alpha", false);
    f.create("Beta", true);
    let before = std::fs::read(f.catalog()).unwrap();
    let list = f.run("projects.list", dict([("project", s("beta"))]));
    assert_eq!(get(&list, "selected_project_id"), Some(&s("beta")));
    assert_eq!(std::fs::read(f.catalog()).unwrap(), before);
    f.run("projects.show", dict([("project_ref", s("BETA"))]));
    assert_eq!(std::fs::read(f.catalog()).unwrap(), before);
    f.run("projects.select", dict([("project_ref", s(" BETA "))]));
    assert_eq!(
        get(
            &f.run("projects.list", dict::<&str>([])),
            "selected_project_id"
        ),
        Some(&s("beta"))
    );
}
#[test]
fn duplicate_project_cannot_replace_metadata_or_touch_requested_path() {
    let f = Fixture::new();
    f.create("Alpha", false);
    let before = std::fs::read(f.catalog()).unwrap();
    let requested = f.root.path().join("must-not-create");
    let error = projects::execute(
        &f.context,
        "projects.create",
        &dict([
            ("name", s("Replacement")),
            ("project_id", s("ALPHA")),
            ("path", s(requested.to_str().unwrap())),
        ]),
    )
    .unwrap_err();
    assert_eq!(error.code, "project_exists");
    assert!(!requested.exists());
    assert_eq!(std::fs::read(f.catalog()).unwrap(), before);
}
#[test]
fn existing_database_is_never_overwritten_or_registered() {
    let f = Fixture::new();
    let root = f.root.path().join("external");
    std::fs::create_dir_all(root.join("data")).unwrap();
    let database = root.join("data/kassiber.sqlite3");
    std::fs::write(&database, b"existing book").unwrap();
    let error = projects::execute(
        &f.context,
        "projects.create",
        &dict([("name", s("External")), ("path", s(root.to_str().unwrap()))]),
    )
    .unwrap_err();
    assert_eq!(error.code, "database_exists");
    assert_eq!(std::fs::read(database).unwrap(), b"existing book");
    assert!(!f.catalog().exists());
}
#[test]
fn inspection_refreshes_encryption_without_exposing_secrets() {
    let f = Fixture::new();
    let created = f.create("Family Treasury", false);
    std::fs::write(field(&created, "database"), b"encrypted header").unwrap();
    let shown = f.run(
        "projects.show",
        dict([("project_ref", s("family-treasury"))]),
    );
    assert_eq!(get(&shown, "encrypted"), Some(&Value::Bool(true)));
    let saved = std::fs::read_to_string(f.catalog()).unwrap();
    assert!(saved.contains("\"encrypted\": true"));
    for forbidden in ["passphrase", "verifier", "xpub", "token"] {
        assert!(!saved.contains(forbidden));
    }
}

#[test]
fn inspection_uses_legacy_database_path_and_an_empty_file_is_plaintext() {
    let f = Fixture::new();
    let created = f.create("Legacy", false);
    let legacy = std::path::Path::new(&field(&created, "data_root")).join("satbooks.sqlite3");
    std::fs::write(&legacy, b"").unwrap();
    let shown = f.run("projects.show", dict([("project_ref", s("legacy"))]));
    assert_eq!(get(&shown, "encrypted"), Some(&Value::Bool(false)));
    assert_eq!(field(&shown, "database"), legacy.to_str().unwrap());
    std::fs::write(&legacy, b"encrypted header").unwrap();
    assert_eq!(
        get(
            &f.run("projects.show", dict([("project_ref", s("legacy"))])),
            "encrypted"
        ),
        Some(&Value::Bool(true))
    );
}
#[test]
fn encrypted_creation_preserves_no_select_and_only_selected_creation_marks_opened() {
    let f = Fixture::new();
    f.create("Alpha", false);
    let beta = f.create("Beta", true);
    std::fs::write(field(&beta, "database"), b"encrypted header").unwrap();
    let shown = projects::created_encrypted(&f.context, "beta", false, "fixed-clock").unwrap();
    assert_eq!(get(&shown, "last_opened_at"), Some(&Value::Null));
    assert_eq!(
        get(
            &f.run("projects.list", dict::<&str>([])),
            "selected_project_id"
        ),
        Some(&s("alpha"))
    );
    let shown = projects::created_encrypted(&f.context, "beta", true, "fixed-clock").unwrap();
    assert_eq!(get(&shown, "last_opened_at"), Some(&s("fixed-clock")));
    assert_eq!(
        get(
            &f.run("projects.list", dict::<&str>([])),
            "selected_project_id"
        ),
        Some(&s("beta"))
    );
}
#[test]
fn missing_and_invalid_project_references_leave_catalog_untouched() {
    let f = Fixture::new();
    f.create("Alpha", false);
    let before = std::fs::read(f.catalog()).unwrap();
    for (reference, code) in [("unknown", "project_not_found"), ("...", "validation")] {
        for command in ["projects.show", "projects.select"] {
            let e = projects::execute(&f.context, command, &dict([("project_ref", s(reference))]))
                .unwrap_err();
            assert_eq!(e.code, code);
        }
    }
    assert_eq!(std::fs::read(f.catalog()).unwrap(), before);
}
#[test]
fn two_encrypted_projects_keep_passphrases_and_initialization_independent() {
    let f = Fixture::new();
    let a = f.create("Alpha", false);
    let b = f.create("Beta", true);
    for (entry, passphrase) in [
        (&a, "alpha-passphrase-12345"),
        (&b, "beta-passphrase-12345"),
    ] {
        kassiber_store::encryption::create_empty_database(
            std::path::Path::new(&field(entry, "database")),
            passphrase,
        )
        .unwrap();
    }
    for (entry, passphrase) in [
        (&a, "alpha-passphrase-12345"),
        (&b, "beta-passphrase-12345"),
    ] {
        let book = kassiber_store::Book::open_migrating(
            std::path::Path::new(&field(entry, "data_root")),
            kassiber_store::OpenOptions {
                passphrase: Some(passphrase),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        assert!(book.encrypted());
        assert!(book.close().is_ok());
    }
    assert!(!kassiber_store::verify_passphrase(
        std::path::Path::new(&field(&b, "data_root")),
        "alpha-passphrase-12345"
    )
    .unwrap());
}
