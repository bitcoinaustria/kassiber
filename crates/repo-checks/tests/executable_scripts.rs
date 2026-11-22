use std::path::{Path, PathBuf};

const FIX: &str = "write test scripts with kassiber_test_support::write_executable \
     (a short-lived child writes the file, so no fork of this process can hold it \
     open for writing and exec cannot fail with ETXTBSY)";

const ETXTBSY_ALLOWED: &[(&str, &str)] = &[
    (
        "crates/provider-broker/src/executables.rs",
        "a provider CLI may be mid-update by its own installer",
    ),
    (
        "crates/kassiber-desktop/src/daemon.rs",
        "the sidecar may be mid-install by the updater",
    ),
    (
        "ui-tauri/src-tauri/src/supervisor.rs",
        "the sidecar may be mid-install by the updater",
    ),
    ("crates/test-support/src/lib.rs", "the helper itself"),
    (
        "crates/test-support/tests/write_executable.rs",
        "the helper's test",
    ),
    (
        "crates/repo-checks/tests/executable_scripts.rs",
        "this check",
    ),
];

#[test]
fn tests_write_their_scripts_through_the_shared_helper() {
    let root = kassiber_repo_checks::root();
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(files.len() > 1000, "found only {} Rust files", files.len());
    let mut findings = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(&path).unwrap();
        findings.extend(check(&relative, &source));
    }
    assert!(findings.is_empty(), "\n{}\n", findings.join("\n"));
}

#[test]
fn the_check_refuses_the_test_that_flaked_on_ci() {
    let source = r##"
#[cfg(test)]
mod tests {
    fn runs_the_cli_and_caps_its_output() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("kassiber-rune-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("kassiber");
        std::fs::write(&fake, "#!/bin/sh\nexit 9\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let r = run_cli_check(&fake, &dir, None).unwrap();
    }
}
"##;
    let findings = check("crates/kassiber-desktop/src/native/update.rs", source);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].contains(":10:") && findings[0].contains("write_executable"));
}

#[test]
fn the_check_refuses_a_local_etxtbsy_retry() {
    let source = r#"
fn install_script(path: &Path) {
    loop {
        match run_provider(ProviderId::Copilot, path, &["--version"], RunOptions::default()) {
            Err(failure) if failure.message().ends_with("ETXTBSY") => {}
            ran => return,
        }
    }
}
"#;
    let findings = check("crates/provider-broker/src/acp/tests.rs", source);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].contains(":5:"));
}

#[test]
fn the_check_allows_private_directories_beside_a_script() {
    let source = r##"
#[cfg(test)]
mod tests {
    fn fake() {
        let script = "#!/bin/sh\n";
        std::fs::create_dir_all(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
    }
}
"##;
    assert_eq!(
        check("crates/net/src/broker.rs", source),
        Vec::<String>::new()
    );
}

fn check(relative: &str, source: &str) -> Vec<String> {
    let mut findings = Vec::new();
    let mentions_etxtbsy = |line: &str| {
        let lower = line.to_ascii_lowercase();
        lower.contains("etxtbsy") || lower.contains("text file busy")
    };
    if !ETXTBSY_ALLOWED.iter().any(|(file, _)| *file == relative) {
        for (index, line) in source.lines().enumerate() {
            if mentions_etxtbsy(line) {
                findings.push(format!(
                    "{relative}:{}: handles ETXTBSY locally; {FIX}",
                    index + 1
                ));
            }
        }
    }
    let Some(test_from) = test_context_start(relative, source) else {
        return findings;
    };
    let lines: Vec<&str> = source.lines().collect();
    let test_lines = &lines[test_from..];
    let writes_scripts = test_lines.iter().any(|line| {
        line.contains("#!/") || (line.contains("include_str!(") && line.contains(".sh\")"))
    });
    if !writes_scripts
        || relative.starts_with("crates/test-support/")
        || relative == "crates/repo-checks/tests/executable_scripts.rs"
    {
        return findings;
    }
    let compact: String = test_lines
        .concat()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    for (offset, line) in test_lines.iter().enumerate() {
        let number = test_from + offset + 1;
        if line.contains(".mode(0o")
            && octal_grants_owner_execute(line.split_once(".mode(").unwrap().1)
        {
            findings.push(format!(
                "{relative}:{number}: opens a script executable; {FIX}"
            ));
        }
        if !line.contains("set_permissions(") {
            continue;
        }
        let statement = lines[test_from + offset..]
            .iter()
            .take(6)
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let (_, call) = statement.split_once("set_permissions(").unwrap();
        let Some((_, mode)) = call.split_once("from_mode(") else {
            continue;
        };
        let target = first_argument(call);
        let wrote = ["write(", "create("].iter().any(|call| {
            compact.contains(&format!("{call}{target}"))
                || compact.contains(&format!("{call}&{target}"))
        });
        if octal_grants_owner_execute(mode) && wrote {
            findings.push(format!(
                "{relative}:{number}: marks a script it wrote executable; {FIX}"
            ));
        }
    }
    findings
}

fn test_context_start(relative: &str, source: &str) -> Option<usize> {
    let name = relative.rsplit('/').next().unwrap();
    if relative.contains("/tests/") || name == "tests.rs" || name.ends_with("_tests.rs") {
        return Some(0);
    }
    source.lines().position(|line| {
        let line = line.trim();
        line.starts_with("#[cfg(test)]") || line.starts_with("#[cfg(all(test")
    })
}

fn first_argument(call: &str) -> String {
    let mut depth = 0;
    let end = call
        .char_indices()
        .find(|&(_, c)| {
            match c {
                '(' => depth += 1,
                ')' if depth == 0 => return true,
                ')' => depth -= 1,
                ',' if depth == 0 => return true,
                _ => {}
            }
            false
        })
        .map_or(call.len(), |(index, _)| index);
    call[..end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .trim_start_matches('&')
        .to_owned()
}

fn octal_grants_owner_execute(mode: &str) -> bool {
    let digits: String = mode
        .trim_start()
        .trim_start_matches("0o")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let owner = digits
        .len()
        .checked_sub(3)
        .and_then(|i| digits[i..].chars().next());
    owner
        .and_then(|c| c.to_digit(8))
        .is_some_and(|d| d & 1 == 1)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type().unwrap().is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}
