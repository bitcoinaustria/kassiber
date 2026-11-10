//! The test-data lock is well formed, ignored, and fetched wherever tests run.
//!
//! Generated fixtures live on the data branch `tests/test-data.lock` pins
//! (docs/reference/testing.md#generated-test-data); scripts/fetch-test-data.sh
//! puts them in place. These checks need no fetched data.

use std::fs;
use std::process::Command;

use kassiber_repo_checks::root;
use serde_yaml::Value;

const BEGIN: &str =
    "# >>> generated test data: tests/test-data.lock (scripts/fetch-test-data.sh --ignore)";
const END: &str = "# <<< generated test data";

/// Header lines and `(blob, path)` entries.
type Lock = (Vec<(String, String)>, Vec<(String, String)>);

fn lock() -> Lock {
    let text = fs::read_to_string(root().join("tests/test-data.lock")).unwrap();
    let (mut header, mut entries) = (Vec::new(), Vec::new());
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        let blob = key.len() == 40
            && key
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
        if blob && !value.is_empty() && !value.contains(char::is_whitespace) {
            entries.push((key.to_owned(), value.to_owned()));
        } else {
            header.push((key.to_owned(), value.to_owned()));
        }
    }
    (header, entries)
}

fn header(key: &str) -> String {
    lock()
        .0
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .unwrap_or_default()
}

#[test]
fn lock_names_a_branch_a_commit_and_sorted_unique_files() {
    assert_eq!(header("branch"), "t3code/test-data");
    let commit = header("commit");
    assert!(
        commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "{commit}"
    );
    let paths: Vec<String> = lock().1.into_iter().map(|(_, p)| p).collect();
    assert!(!paths.is_empty(), "the lock pins no files");
    let mut sorted = paths.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(paths, sorted);
    assert!(paths
        .iter()
        .all(|p| !p.starts_with('/') && !p.split('/').any(|part| part == "..")));
}

#[test]
fn locked_files_are_ignored_and_not_tracked() {
    let paths: Vec<String> = lock().1.into_iter().map(|(_, p)| p).collect();
    let text = fs::read_to_string(root().join(".gitignore")).unwrap();
    let block = text
        .split_once(BEGIN)
        .expect("begin marker")
        .1
        .split_once(END)
        .expect("end marker")
        .0;
    let listed: Vec<&str> = block.split_whitespace().collect();
    let mut expected = vec!["/.test-data/".to_owned()];
    expected.extend(paths.iter().map(|p| format!("/{p}")));
    assert_eq!(listed, expected);
    let tracked = Command::new("git")
        .current_dir(root())
        .args(["ls-files", "--"])
        .args(&paths)
        .output()
        .unwrap();
    assert!(tracked.status.success());
    assert!(
        tracked.stdout.is_empty(),
        "generated test data belongs on the data branch, not in this tree"
    );
}

#[test]
fn fetch_script_is_executable_and_uses_git_only() {
    let script = root().join("scripts/fetch-test-data.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert!(fs::metadata(&script).unwrap().permissions().mode() & 0o111 != 0);
    }
    let text = fs::read_to_string(&script).unwrap();
    assert!(text.contains("tests/test-data.lock"));
    for tool in ["curl", "wget", "pip ", "uv ", "npm "] {
        assert!(!text.contains(tool), "{tool}");
    }
}

#[test]
fn ci_fetches_the_data_before_tests_and_rust_builds() {
    let text = fs::read_to_string(root().join(".github/workflows/ci.yml")).unwrap();
    let workflow: Value = serde_yaml::from_str(&text).unwrap();
    let needs = |run: &str| {
        run.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .any(|word| matches!(word, "cargo" | "pytest" | "python_test_shards"))
            || run.contains("uv sync")
    };
    let mut checked = 0;
    for (name, job) in workflow["jobs"].as_mapping().unwrap() {
        let steps = job
            .get("steps")
            .and_then(Value::as_sequence)
            .cloned()
            .unwrap_or_default();
        let run = |step: &Value| {
            step.get("run")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let Some(first) = steps.iter().position(|s| needs(&run(s))) else {
            continue;
        };
        let fetch = steps
            .iter()
            .position(|s| run(s).contains("scripts/fetch-test-data.sh"));
        assert!(fetch.is_some_and(|f| f < first), "{name:?}");
        checked += 1;
    }
    assert!(checked > 0);
}
