//! Every GitHub Actions `uses:` reference is pinned to a full commit SHA.
//!
//! The audit parses each workflow as YAML, so quoted, flow-mapping, explicit
//! (`? uses`) and escaped (`"uses"`) keys are all seen. Repository-local
//! actions (`./...`) are followed recursively; they may not leave the
//! repository, and a local Docker action must name a digest-pinned remote
//! image. `docker://` steps are not allowed at all.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml::Value;

fn is_full_sha(text: &str) -> bool {
    text.len() == 40
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_pinned_docker_image(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("docker://") else {
        return false;
    };
    let Some((image, digest)) = rest.split_once("@sha256:") else {
        return false;
    };
    !image.is_empty()
        && !image.chars().any(|c| c.is_whitespace() || c == '@')
        && digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn collect_uses(value: &Value, file: &Path, found: &mut Vec<String>) -> Result<(), String> {
    match value {
        Value::Sequence(items) => {
            for item in items {
                collect_uses(item, file, found)?;
            }
        }
        Value::Mapping(map) => {
            for (key, child) in map {
                let Value::String(key) = key else {
                    return Err(format!(
                        "{}: complex YAML mapping keys cannot be audited safely",
                        file.display()
                    ));
                };
                if key == "uses" {
                    let Value::String(reference) = child else {
                        return Err(format!("{}: uses value must be a string", file.display()));
                    };
                    let reference = reference.trim();
                    if reference.is_empty() || reference.chars().any(char::is_whitespace) {
                        return Err(format!(
                            "{}: uses value must be one non-empty scalar",
                            file.display()
                        ));
                    }
                    found.push(reference.to_owned());
                }
                collect_uses(child, file, found)?;
            }
        }
        Value::Tagged(tagged) => collect_uses(&tagged.value, file, found)?,
        _ => {}
    }
    Ok(())
}

fn yaml_uses(file: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut found = Vec::new();
    for document in serde_yaml::Deserializer::from_str(&text) {
        let value = <Value as serde::Deserialize>::deserialize(document)
            .map_err(|e| format!("{}: workflow YAML is invalid: {e}", file.display()))?;
        collect_uses(&value, file, &mut found)?;
    }
    Ok(found)
}

fn workflow_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "yml" | "yaml"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    if files.is_empty() {
        return Err("no GitHub Actions workflow files were found".into());
    }
    Ok(files)
}

fn resolve_local(reference: &str, file: &Path, repository: &Path) -> Result<PathBuf, String> {
    let joined = repository.join(&reference[2..]);
    let target = joined.canonicalize().unwrap_or(joined);
    if !target.starts_with(repository) || reference[2..].split('/').any(|part| part == "..") {
        return Err(format!(
            "{}: local action {reference} escapes the repository",
            file.display()
        ));
    }
    if target.is_dir() {
        let candidates: Vec<PathBuf> = ["action.yml", "action.yaml"]
            .iter()
            .map(|name| target.join(name))
            .filter(|path| path.is_file())
            .collect();
        let [action] = candidates.as_slice() else {
            return Err(format!(
                "{}: local action must contain exactly one action.yml or action.yaml",
                file.display()
            ));
        };
        let text = fs::read_to_string(action).map_err(|e| e.to_string())?;
        let metadata: Value = serde_yaml::from_str(&text)
            .map_err(|e| format!("{}: local action YAML is invalid: {e}", action.display()))?;
        let Value::Mapping(map) = &metadata else {
            return Err(format!(
                "{}: local action metadata must be a mapping",
                action.display()
            ));
        };
        if let Some(Value::Mapping(runs)) = map.get("runs") {
            let using = runs
                .get("using")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if using.eq_ignore_ascii_case("docker") {
                let image = runs
                    .get("image")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                if !is_pinned_docker_image(image) {
                    return Err(format!(
                        "{}: local Docker action image {image:?} is not a digest-pinned docker:// image",
                        file.display()
                    ));
                }
            }
        }
        return action.canonicalize().map_err(|e| e.to_string());
    }
    let workflow = target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "yml" | "yaml"));
    if target.is_file() && workflow {
        return Ok(target);
    }
    Err(format!(
        "{}: local action or workflow {reference} was not found",
        file.display()
    ))
}

/// Audit every workflow under `workflows`, following local actions inside
/// `repository`.
pub fn audit(workflows: &Path, repository: &Path) -> Result<usize, String> {
    let repository = repository.canonicalize().map_err(|e| e.to_string())?;
    let mut pending = workflow_files(workflows)?;
    let mut visited = BTreeSet::new();
    let mut references = 0;
    while let Some(file) = pending.pop() {
        let file = file.canonicalize().map_err(|e| e.to_string())?;
        if !file.starts_with(&repository) {
            return Err(format!(
                "{}: audited YAML escapes the repository",
                file.display()
            ));
        }
        if !visited.insert(file.clone()) {
            continue;
        }
        for reference in yaml_uses(&file)? {
            references += 1;
            if reference.starts_with("./") {
                pending.push(resolve_local(&reference, &file, &repository)?);
                continue;
            }
            if reference.starts_with("docker://") {
                return Err(format!(
                    "{}: {reference}: Docker actions are not allowed by the pin policy",
                    file.display()
                ));
            }
            match reference.rsplit_once('@') {
                Some((action, revision)) if !action.is_empty() && is_full_sha(revision) => {}
                _ => {
                    return Err(format!(
                        "{}: {reference:?} is not pinned to a full commit SHA",
                        file.display()
                    ))
                }
            }
        }
    }
    if references == 0 {
        return Err("no GitHub Actions uses references were found".into());
    }
    Ok(references)
}

fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

fn audit_repo(dir: &tempfile::TempDir) -> Result<usize, String> {
    audit(&dir.path().join(".github/workflows"), dir.path())
}

fn sha() -> String {
    "a".repeat(40)
}

#[test]
fn every_workflow_action_is_pinned_to_a_commit_sha() {
    let root = kassiber_repo_checks::root();
    let count = audit(&root.join(".github/workflows"), &root).unwrap();
    assert!(count > 10, "{count}");
}

#[test]
fn quoted_and_secondary_workflow_references_are_checked() {
    let pinned = sha();
    let first = format!("steps:\n  - uses: 'actions/checkout@{pinned}'\n");
    let dir = repo(&[
        (".github/workflows/first.yml", &first),
        (
            ".github/workflows/second.yaml",
            "steps:\n  - \"uses\": \"actions/setup-python@v5\"\n",
        ),
    ]);
    assert!(audit_repo(&dir).unwrap_err().contains("setup-python@v5"));
    let second = format!("steps:\n  - \"uses\": \"actions/setup-python@{pinned}\"\n");
    fs::write(dir.path().join(".github/workflows/second.yaml"), second).unwrap();
    assert_eq!(audit_repo(&dir), Ok(2));
}

#[test]
fn inline_explicit_and_escaped_uses_keys_are_audited() {
    let pinned = sha();
    let head = format!("steps:\n  - uses: actions/checkout@{pinned}\n");
    for (tail, ok) in [
        (
            "  - ? uses\n    : actions/setup-python@v5\n".to_owned(),
            false,
        ),
        (
            "  - \"us\\u0065s\": actions/setup-python@v5\n".to_owned(),
            false,
        ),
        (
            format!("  - {{ uses: actions/setup-python@{pinned} }}\n"),
            true,
        ),
    ] {
        let dir = repo(&[(
            ".github/workflows/adversarial.yml",
            &format!("{head}{tail}"),
        )]);
        match audit_repo(&dir) {
            Ok(_) => assert!(ok, "{tail}"),
            Err(error) => assert!(!ok && error.contains("setup-python@v5"), "{tail}: {error}"),
        }
    }
}

#[test]
fn repository_local_composite_actions_are_audited_recursively() {
    let dir = repo(&[
        (
            ".github/workflows/ci.yml",
            "steps:\n  - uses: ./.github/actions/outer\n",
        ),
        (
            ".github/actions/outer/action.yml",
            "runs:\n  using: composite\n  steps:\n    - uses: ./.github/actions/inner\n",
        ),
        (
            ".github/actions/inner/action.yaml",
            "runs:\n  using: composite\n  steps:\n    - uses: actions/setup-python@v5\n",
        ),
    ]);
    assert!(audit_repo(&dir).unwrap_err().contains("setup-python@v5"));
    let inner = format!(
        "runs:\n  using: composite\n  steps:\n    - uses: actions/setup-python@{}\n    - uses: ./.github/actions/outer\n",
        sha()
    );
    fs::write(dir.path().join(".github/actions/inner/action.yaml"), inner).unwrap();
    assert!(audit_repo(&dir).is_ok());
}

#[test]
fn repository_local_action_paths_cannot_escape() {
    let outer = tempfile::tempdir().unwrap();
    let repository = outer.path().join("repo");
    fs::create_dir_all(repository.join(".github/workflows")).unwrap();
    fs::create_dir_all(outer.path().join("outside-action")).unwrap();
    fs::write(
        repository.join(".github/workflows/ci.yml"),
        "steps:\n  - uses: ./../outside-action\n",
    )
    .unwrap();
    let error = audit(&repository.join(".github/workflows"), &repository).unwrap_err();
    assert!(error.contains("escapes the repository"), "{error}");
}

#[test]
fn repository_local_docker_actions_require_an_immutable_remote_image() {
    let dir = repo(&[
        (
            ".github/workflows/ci.yml",
            "steps:\n  - uses: ./.github/actions/docker\n",
        ),
        (".github/actions/docker/action.yml", ""),
    ]);
    let metadata = dir.path().join(".github/actions/docker/action.yml");
    for image in [
        "docker://alpine:latest",
        "DOCKER://alpine:latest",
        "Dockerfile",
    ] {
        fs::write(
            &metadata,
            format!("runs:\n  using: docker\n  image: {image}\n"),
        )
        .unwrap();
        let error = audit_repo(&dir).unwrap_err();
        assert!(
            error.contains("not a digest-pinned docker:// image"),
            "{image}: {error}"
        );
    }
    let pinned = format!(
        "runs:\n  using: docker\n  image: docker://alpine@sha256:{}\n",
        "a".repeat(64)
    );
    fs::write(&metadata, pinned).unwrap();
    assert!(audit_repo(&dir).is_ok());
}

#[test]
fn docker_steps_are_refused() {
    let dir = repo(&[(
        ".github/workflows/ci.yml",
        "steps:\n  - uses: docker://alpine:3\n",
    )]);
    assert!(audit_repo(&dir)
        .unwrap_err()
        .contains("Docker actions are not allowed"));
}
