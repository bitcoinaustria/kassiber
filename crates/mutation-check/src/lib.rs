//! Mutation catalogs: each entry breaks one rule in one file, and names the
//! tests that must fail once it is broken.
//!
//! A catalog is a JSON list of entries with exactly the fields `id`, `file`,
//! `find`, `replace`, `expect_failing` and `rule`. `find` must occur exactly
//! once in `file`; `expect_failing` names one or more cargo test filters.
//! `catalogs.json` beside this crate says how each catalog's tests run (the
//! packages, features and targets passed to `cargo test`, and the
//! environment).
//!
//! `run` copies the working tree into a scratch tree under `target/`, runs
//! every named test unmutated (each must pass and select at least one test),
//! then applies one entry at a time: the mutated tree must build, and every
//! named test must fail. A survivor, a test that selects nothing, or a tree
//! that does not build fails the run. The worktree is never written.
//!
//! Entries whose killers are pytest node ids, or that mutate Python files,
//! are legacy: `check` validates them and counts them, `check --strict` and
//! `run` refuse them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

pub mod runner;

/// The fields every catalog entry has, and no others.
pub const FIELDS: [&str; 6] = ["id", "file", "find", "replace", "expect_failing", "rule"];

/// One catalog entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mutation {
    pub id: String,
    pub file: String,
    pub find: String,
    pub replace: String,
    pub expect_failing: Vec<String>,
    pub rule: String,
    /// The catalog the entry came from (not a JSON field).
    pub catalog: String,
}

impl Mutation {
    /// Whether this entry still needs the Python stack: a pytest killer or
    /// a mutated Python file.
    pub fn is_legacy(&self) -> bool {
        self.file.ends_with(".py") || self.expect_failing.iter().any(|t| is_pytest_id(t))
    }
}

/// A pytest node id (`tests/x.py::Class::test`), as opposed to a cargo filter.
pub fn is_pytest_id(test: &str) -> bool {
    test.contains(".py::") || (test.starts_with("tests/") && test.ends_with(".py"))
}

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn err<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error(message.into()))
}

/// The entries of one catalog, with their shape checked.
pub fn load_catalog(path: &Path) -> Result<Vec<Mutation>, Error> {
    let text = fs::read_to_string(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let mut mutations =
        parse_catalog(&text).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    for mutation in &mut mutations {
        mutation.catalog = path.to_string_lossy().into_owned();
    }
    Ok(mutations)
}

pub fn parse_catalog(text: &str) -> Result<Vec<Mutation>, Error> {
    let data: serde_json::Value =
        serde_json::from_str(text).map_err(|e| Error(format!("not JSON: {e}")))?;
    let Some(items) = data.as_array() else {
        return err("a catalog must hold a list of mutations");
    };
    let mut mutations = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(object) = item.as_object() else {
            return err(format!("entry {index} is not an object"));
        };
        let keys: BTreeSet<&str> = object.keys().map(String::as_str).collect();
        if keys != FIELDS.iter().copied().collect() {
            return err(format!(
                "entry {index} must have exactly the fields {}",
                FIELDS.join(", ")
            ));
        }
        let text = |field: &str, allow_empty: bool| -> Result<String, Error> {
            match object[field].as_str() {
                Some(value) if allow_empty || !value.is_empty() => Ok(value.to_owned()),
                _ => err(format!("entry {index}: {field} must be a non-empty string")),
            }
        };
        let expect_failing = match &object["expect_failing"] {
            serde_json::Value::String(test) => vec![test.clone()],
            serde_json::Value::Array(tests) => tests
                .iter()
                .map(|t| t.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .unwrap_or_default(),
            _ => vec![],
        };
        if expect_failing.is_empty() || expect_failing.iter().any(String::is_empty) {
            return err(format!(
                "entry {index}: expect_failing must name at least one test"
            ));
        }
        mutations.push(Mutation {
            id: text("id", false)?,
            file: text("file", false)?,
            find: text("find", false)?,
            replace: text("replace", true)?,
            expect_failing,
            rule: text("rule", false)?,
            catalog: String::new(),
        });
    }
    Ok(mutations)
}

/// Whether `name` could reach outside `root`: absolute, `..`, a symlink on
/// the way, or a path that resolves elsewhere.
pub fn escapes(root: &Path, name: &str) -> bool {
    let relative = Path::new(name);
    if relative.is_absolute()
        || relative.components().next().is_none()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return true;
    }
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component);
        if path
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return true;
        }
    }
    match (path.canonicalize(), root.canonicalize()) {
        (Ok(target), Ok(root)) => !target.starts_with(root),
        _ => false,
    }
}

/// What keeps `mutations` from applying to the tree at `root`.
pub fn problems(mutations: &[Mutation], root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    for mutation in mutations {
        // Ids are unique within a catalog; suites may join catalogs.
        if !seen.insert((mutation.catalog.as_str(), mutation.id.as_str())) {
            found.push(format!("{}: duplicate id", mutation.id));
        }
        if mutation.find == mutation.replace {
            found.push(format!("{}: replace equals find", mutation.id));
        }
        if escapes(root, &mutation.file) {
            found.push(format!(
                "{}: file {} is not a file inside the tree",
                mutation.id, mutation.file
            ));
            continue;
        }
        let path = root.join(&mutation.file);
        let Ok(text) = fs::read_to_string(&path) else {
            found.push(format!("{}: no file {}", mutation.id, mutation.file));
            continue;
        };
        let count = text.matches(mutation.find.as_str()).count();
        if count != 1 {
            found.push(format!(
                "{}: find occurs {count} times in {}, not once",
                mutation.id, mutation.file
            ));
        }
        for test in &mutation.expect_failing {
            if is_pytest_id(test) {
                if let Some(missing) = missing_pytest(root, test) {
                    found.push(format!("{}: {missing}", mutation.id));
                }
            }
        }
    }
    found
}

/// A pytest node id that names no class or function in its module. A text
/// scan, not an import: `class Name` / `def name` at the right nesting.
fn missing_pytest(root: &Path, node_id: &str) -> Option<String> {
    let (path, rest) = node_id.split_once("::")?;
    let Ok(source) = fs::read_to_string(root.join(path)) else {
        return Some(format!("{node_id:?}: no file {path}"));
    };
    let mut indent = 0usize;
    let mut lines: Vec<&str> = source.lines().collect();
    for name in rest.split("::") {
        let name = name.split('[').next().unwrap_or(name);
        let position = lines.iter().position(|line| {
            let stripped = line.trim_start();
            let at = line.len() - stripped.len();
            at == indent
                && [
                    format!("class {name}"),
                    format!("def {name}"),
                    format!("async def {name}"),
                ]
                .iter()
                .any(|head| {
                    stripped
                        .strip_prefix(head.as_str())
                        .is_some_and(|tail| tail.starts_with(['(', ':']))
                })
        });
        let Some(position) = position else {
            return Some(format!("{node_id:?} names no test in {path}"));
        };
        lines = lines.split_off(position + 1);
        indent = lines
            .iter()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.len() - l.trim_start().len())
            .unwrap_or(0);
    }
    None
}

/// How one group of catalog entries runs.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    /// A unique name, for `run --suite`.
    pub name: String,
    /// The catalogs this suite runs, repository-relative.
    pub catalogs: Vec<String>,
    /// Only entries whose id contains this.
    #[serde(default)]
    pub id_contains: Option<String>,
    /// Arguments after `cargo test` (packages, features, targets). `--offline`
    /// and `--locked` are always added.
    #[serde(default)]
    pub cargo: Vec<String>,
    /// Per-group cargo arguments that replace `cargo`, keyed by the part of
    /// the id before its first `-` (`by: "id-prefix"`), by the crate
    /// directory of the mutated file (`by: "file-crate"`), or by the
    /// catalog's path (`by: "catalog"`).
    #[serde(default)]
    pub groups: Option<Groups>,
    /// Pick the integration target that defines a filter's test function.
    #[serde(default)]
    pub resolve_target: Option<ResolveTarget>,
    /// Extra environment for every cargo command.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Passed after `--` with the filter, e.g. `--test-threads=1`.
    #[serde(default)]
    pub test_args: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    1800
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Groups {
    pub by: GroupBy,
    pub args: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GroupBy {
    IdPrefix,
    FileCrate,
    Catalog,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveTarget {
    /// The directory of integration tests to search (`crates/x/tests`).
    pub tests_dir: String,
    /// Further targets: target name to the file that defines its tests.
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub suites: Vec<Suite>,
}

pub const CONFIG: &str = "crates/mutation-check/catalogs.json";

pub fn load_config(root: &Path) -> Result<Config, Error> {
    let path = root.join(CONFIG);
    let text = fs::read_to_string(&path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let config: Config =
        serde_json::from_str(&text).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let mut names = BTreeSet::new();
    for suite in &config.suites {
        if !names.insert(suite.name.as_str()) {
            return err(format!("{CONFIG}: duplicate suite {}", suite.name));
        }
    }
    Ok(config)
}

impl Suite {
    /// The entries this suite runs.
    pub fn mutations(&self, root: &Path) -> Result<Vec<Mutation>, Error> {
        let mut all = Vec::new();
        for catalog in &self.catalogs {
            let mut mutations = load_catalog(&root.join(catalog))?;
            for mutation in &mut mutations {
                mutation.catalog = catalog.clone();
            }
            all.extend(mutations);
        }
        if let Some(part) = &self.id_contains {
            all.retain(|m| m.id.contains(part.as_str()));
        }
        Ok(all)
    }

    /// The cargo arguments for one entry.
    pub fn cargo_args(&self, mutation: &Mutation) -> Result<Vec<String>, Error> {
        let Some(groups) = &self.groups else {
            return Ok(self.cargo.clone());
        };
        let key = match groups.by {
            GroupBy::IdPrefix => mutation.id.split('-').next().unwrap_or_default().to_owned(),
            GroupBy::FileCrate => {
                let mut parts = mutation.file.split('/');
                match (parts.next(), parts.next()) {
                    (Some("crates"), Some(name)) => name.to_owned(),
                    _ => String::new(),
                }
            }
            GroupBy::Catalog => mutation.catalog.clone(),
        };
        match groups.args.get(&key) {
            Some(args) => Ok(args.clone()),
            None => err(format!(
                "{}: no cargo group {key:?} in suite {}",
                mutation.id, self.name
            )),
        }
    }
}

/// Every `*mutations*.json` catalog in a `scripts/` directory under `crates/`
/// and `tax-engine/`.
pub fn catalogs_in_tree(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for top in ["crates", "tax-engine"] {
        walk(&root.join(top), &mut |path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let in_scripts = path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|d| d == "scripts");
            if in_scripts && name.ends_with(".json") && name.contains("mutations") {
                if let Ok(relative) = path.strip_prefix(root) {
                    found.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        });
    }
    found.sort();
    found
}

fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name == "target" || name.starts_with('.') || name == "node_modules" {
            continue;
        }
        match path.symlink_metadata() {
            Ok(meta) if meta.is_dir() => walk(&path, visit),
            Ok(meta) if meta.is_file() => visit(&path),
            _ => {}
        }
    }
}

/// The result of `check`.
#[derive(Debug, Default)]
pub struct Report {
    pub entries: usize,
    pub legacy: usize,
    pub problems: Vec<String>,
}

/// Validate every suite's entries against the tree, and that every catalog in
/// the tree belongs to a suite. `strict` also refuses legacy entries.
pub fn check(root: &Path, config: &Config, only: &[String], strict: bool) -> Report {
    let mut report = Report::default();
    let mut covered = BTreeSet::new();
    for suite in &config.suites {
        covered.extend(suite.catalogs.iter().cloned());
        if !only.is_empty() && !only.contains(&suite.name) {
            continue;
        }
        let mutations = match suite.mutations(root) {
            Ok(mutations) => mutations,
            Err(error) => {
                report.problems.push(error.0);
                continue;
            }
        };
        if mutations.is_empty() {
            report
                .problems
                .push(format!("suite {} selects no entry", suite.name));
        }
        for mutation in &mutations {
            if let Err(error) = suite.cargo_args(mutation) {
                report.problems.push(error.0);
            }
            if mutation.is_legacy() {
                report.legacy += 1;
                if strict {
                    report.problems.push(format!(
                        "{}: needs Python (a pytest killer or a Python file)",
                        mutation.id
                    ));
                }
            }
        }
        report.entries += mutations.len();
        report.problems.extend(
            problems(&mutations, root)
                .into_iter()
                .map(|p| format!("{}: {p}", suite.name)),
        );
    }
    if only.is_empty() {
        for catalog in catalogs_in_tree(root) {
            if !covered.contains(&catalog) {
                report
                    .problems
                    .push(format!("{catalog} belongs to no suite in {CONFIG}"));
            }
        }
        for catalog in &covered {
            if !root.join(catalog).is_file() {
                report
                    .problems
                    .push(format!("{CONFIG} names a missing catalog {catalog}"));
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, file: &str, find: &str, tests: &str) -> String {
        serde_json::json!({
            "id": id, "file": file, "find": find, "replace": "X",
            "expect_failing": [tests], "rule": "a rule",
        })
        .to_string()
    }

    #[test]
    fn entries_need_exactly_the_fields() {
        assert!(parse_catalog(r#"[{"id":"a"}]"#).is_err());
        assert!(parse_catalog("{}").is_err());
        let ok = format!("[{}]", entry("a", "f.rs", "x", "t"));
        assert_eq!(parse_catalog(&ok).unwrap()[0].expect_failing, vec!["t"]);
        let empty = ok.replace(r#"["t"]"#, "[]");
        assert!(parse_catalog(&empty).is_err());
    }

    #[test]
    fn problems_find_counts_duplicates_and_escapes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("f.rs"), "one two two").unwrap();
        let catalog = format!(
            "[{},{},{},{}]",
            entry("a", "f.rs", "one", "t"),
            entry("a", "f.rs", "two", "t"),
            entry("c", "../f.rs", "one", "t"),
            entry("d", "f.rs", "three", "t")
        );
        let found = problems(&parse_catalog(&catalog).unwrap(), dir.path());
        assert!(found.iter().any(|p| p == "a: duplicate id"), "{found:?}");
        assert!(
            found.iter().any(|p| p.contains("find occurs 2 times")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|p| p.starts_with("c: file ../f.rs is not")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|p| p.contains("find occurs 0 times")),
            "{found:?}"
        );
        // The first `a` finds "one" exactly once; only the second one is wrong.
        assert_eq!(found.len(), 4, "{found:?}");
    }

    #[test]
    fn pytest_ids_must_name_a_test() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("tests")).unwrap();
        fs::write(
            dir.path().join("tests/test_x.py"),
            "class T(unittest.TestCase):\n    def test_a(self):\n        pass\n\ndef test_b():\n    pass\n",
        )
        .unwrap();
        fs::write(dir.path().join("f.rs"), "x").unwrap();
        for (test, ok) in [
            ("tests/test_x.py::T::test_a", true),
            ("tests/test_x.py::test_b[p1]", true),
            ("tests/test_x.py::T::test_b", false),
            ("tests/test_x.py::test_a", false),
            ("tests/test_y.py::test_a", false),
        ] {
            let catalog = format!("[{}]", entry("a", "f.rs", "x", test));
            let found = problems(&parse_catalog(&catalog).unwrap(), dir.path());
            assert_eq!(found.is_empty(), ok, "{test}: {found:?}");
        }
    }

    #[test]
    fn legacy_entries_are_the_python_ones() {
        let mutation = |file: &str, test: &str| Mutation {
            id: "a".into(),
            file: file.into(),
            find: "x".into(),
            replace: "y".into(),
            expect_failing: vec![test.into()],
            rule: "r".into(),
            catalog: "c.json".into(),
        };
        assert!(mutation("a.py", "t").is_legacy());
        assert!(mutation("a.rs", "tests/test_a.py::T::t").is_legacy());
        assert!(!mutation("a.rs", "module::tests::name").is_legacy());
    }

    #[test]
    fn groups_pick_cargo_arguments() {
        let suite: Suite = serde_json::from_value(serde_json::json!({
            "name": "s", "catalogs": [],
            "groups": {"by": "id-prefix", "args": {"store": ["-p", "kassiber-store"]}},
        }))
        .unwrap();
        let mut mutation = parse_catalog(&format!("[{}]", entry("store-a", "f.rs", "x", "t")))
            .unwrap()
            .remove(0);
        assert_eq!(
            suite.cargo_args(&mutation).unwrap(),
            vec!["-p", "kassiber-store"]
        );
        mutation.id = "wallets-a".into();
        assert!(suite.cargo_args(&mutation).is_err());
    }
}
