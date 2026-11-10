//! `kassiber-release` against the frozen output of the Python release scripts
//! (scripts/export_tool_goldens.py): the same exit status, the same stdout
//! and stderr bytes, and the same output files with the same modes.
//! Uncaught Python tracebacks (bad renderer input) compare by exit status.
use std::path::{Path, PathBuf};
use std::process::Command;

use kassiber_envelope::{get, py_str};
use kassiber_pyjson::Value;
use sha2::{Digest, Sha256};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// A scratch directory under the system temporary directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "kassiber-release-tool-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(value: &Value) -> String {
    py_str(value)
}

fn pairs(value: &Value) -> Vec<(String, &Value)> {
    match value {
        Value::Dict(pairs) => pairs
            .iter()
            .map(|(key, value)| {
                let kassiber_pyjson::Key::Str(key) = key else {
                    panic!("string keys")
                };
                (key.as_str().unwrap().to_owned(), value)
            })
            .collect(),
        _ => panic!("an object"),
    }
}

fn list(value: &Value) -> Vec<String> {
    match value {
        Value::List(items) => items.iter().map(text).collect(),
        _ => panic!("a list"),
    }
}

fn write_files(root: &Path, files: &Value) {
    for (name, content) in pairs(files) {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text(content)).unwrap();
    }
}

fn tree(root: &Path, skip: &[String]) -> Vec<(String, String, String, Option<String>)> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if skip.contains(&name) {
                continue;
            }
            let raw = std::fs::read(&path).unwrap();
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                format!(
                    "0o{:o}",
                    path.metadata().unwrap().permissions().mode() & 0o777
                )
            };
            #[cfg(not(unix))]
            let mode = String::new();
            let digest: String = Sha256::digest(&raw)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let shown = String::from_utf8(raw).ok().filter(|t| t.len() <= 16384);
            out.push((name, digest, mode, shown));
        }
    }
    out.sort();
    out
}

#[test]
fn release_tool_matches_the_python_scripts() {
    let golden = kassiber_pytext::loads(include_str!("data/tool_goldens.json")).unwrap();
    let Some(Value::List(cases)) = get(&golden, "cases") else {
        panic!("cases")
    };
    let real_repo = repo();
    for case in cases {
        let field = |key: &str| get(case, key).unwrap_or_else(|| panic!("{key}"));
        let name = text(field("name"));
        let scratch = Scratch::new(&name);
        let work = scratch.0.join("work");
        std::fs::create_dir(&work).unwrap();
        write_files(&work, field("setup"));
        let repo_files = field("repo_files");
        let root = if pairs(repo_files).is_empty() {
            real_repo.clone()
        } else {
            let root = scratch.0.join("repo");
            std::fs::create_dir(&root).unwrap();
            write_files(&root, repo_files);
            root
        };
        let args: Vec<String> = list(field("args"))
            .into_iter()
            .map(|arg| arg.replace("{repo}", real_repo.to_str().unwrap()))
            .collect();
        let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber-release"));
        command
            .args(list(field("rust")))
            .args(&args)
            .current_dir(&work)
            .env("KASSIBER_REPO_ROOT", &root)
            .env_remove("SOURCE_DATE_EPOCH");
        for (key, value) in pairs(field("env")) {
            command.env(key, text(value));
        }
        let output = command.output().unwrap();
        let shown = |raw: &[u8]| {
            String::from_utf8(raw.to_vec())
                .unwrap()
                .replace(root.to_str().unwrap(), "{repo}")
                .replace(real_repo.to_str().unwrap(), "{repo}")
        };
        let stdout = shown(&output.stdout);
        let stderr = shown(&output.stderr);
        let expected_exit = match field("exit") {
            Value::Int(code) => i32::try_from(code.clone()).unwrap(),
            _ => panic!("exit"),
        };
        assert_eq!(
            output.status.code(),
            Some(expected_exit),
            "{name}: {stderr}"
        );
        assert_eq!(stdout, text(field("stdout")), "{name}");
        match field("stderr") {
            Value::Null => assert!(!stderr.is_empty(), "{name}: a refusal says why"),
            expected => assert_eq!(stderr, text(expected), "{name}"),
        }
        let skip: Vec<String> = pairs(field("setup")).into_iter().map(|(k, _)| k).collect();
        let actual = tree(&work, &skip);
        let mut expected: Vec<(String, String, String, Option<String>)> = pairs(field("files"))
            .into_iter()
            .map(|(file, record)| {
                (
                    file,
                    text(get(record, "sha256").unwrap()),
                    text(get(record, "mode").unwrap()),
                    get(record, "text").map(text),
                )
            })
            .collect();
        expected.sort();
        // Compare texts first for a readable diff, then everything.
        for (got, want) in actual.iter().zip(&expected) {
            assert_eq!((&got.0, &got.3), (&want.0, &want.3), "{name}");
        }
        assert_eq!(actual, expected, "{name}");
    }
    assert!(cases.len() >= 40);
}
