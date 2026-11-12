//! The notarization workflow's handoff steps, run for real against local
//! `git`, `gh` and release-tool mocks: a candidate binds its source commit and
//! version without a tag, every invalid input stops before the signing input
//! is downloaded, and promotion re-checks the draft and never finalizes.
//! Ported from `tests/test_macos_candidate_workflow.py` (plan 22).
#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_yaml::Value;

const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn candidate() -> String {
    format!("macos-candidate-{COMMIT}-123")
}

fn workflow() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/notarize-macos.yml");
    serde_yaml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn step(workflow: &Value, name: &str) -> Value {
    workflow["jobs"]["notarize"]["steps"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|step| step["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("no step {name}"))
        .clone()
}

/// A mock that logs its tool name and arguments, tab-separated, one call per
/// line, and answers the queries the steps make.
const MOCK: &str = r#"#!/bin/bash
tool="$(basename "$0")"
{ printf '%s' "$tool"; for arg in "$@"; do printf '\t%s' "$arg"; done; printf '\n'; } >> "$MOCK_LOG"
case "$tool" in
  git)
    case "$1" in
      ls-remote)
        if [ -n "${MOCK_TAG_FAILURE:-}" ]; then exit 1; fi
        if [ -n "${MOCK_TAG:-}" ]; then echo 'existing tag'; fi
        ;;
      show) echo '{"version": "1.2.3"}' ;;
      rev-parse) echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ;;
      merge-base) if [ -n "${MOCK_FOREIGN:-}" ]; then exit 1; fi ;;
      *) echo "unexpected git $*" >&2; exit 99 ;;
    esac
    ;;
  gh)
    if [ "$1 $2" = "release view" ]; then
      field=""
      while [ "$#" -gt 0 ]; do
        if [ "$1" = --json ]; then field="$2"; fi
        shift
      done
      case "$field" in
        isDraft) echo "${MOCK_DRAFT:-true}" ;;
        targetCommitish) echo "${MOCK_TARGET:-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}" ;;
        assets)
          if [ -n "${MOCK_ASSET_FAILURE:-}" ]; then exit 1; fi
          if [ -n "${MOCK_SIGNED:-}" ]; then echo manifest.sig
          elif [ -z "${MOCK_NO_INPUT:-}" ]; then echo kassiber-macos-signing-input.dmg
          fi
          ;;
        *) echo "unexpected gh field $field" >&2; exit 99 ;;
      esac
    elif [ "$1 $2" = "release download" ]; then
      printf 'signed input' > incoming/kassiber-macos-signing-input.dmg
    elif [ "$1 $2" != "release upload" ] && [ "$1 $2" != "release delete-asset" ]; then
      echo "unexpected gh $*" >&2; exit 99
    fi
    ;;
  kassiber-release) ;;
  *) echo "unexpected tool $tool" >&2; exit 99 ;;
esac
"#;

struct Runner {
    root: tempfile::TempDir,
    env: BTreeMap<String, String>,
}

impl Runner {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        for tool in ["git", "gh", "kassiber-release"] {
            let path = bin.join(tool);
            fs::write(&path, MOCK).unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = |name: &str| root.path().join(name).to_string_lossy().into_owned();
        let env = BTreeMap::from([
            (
                "PATH".into(),
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            ),
            ("TAG".into(), candidate()),
            ("CANDIDATE".into(), "true".into()),
            ("SOURCE_COMMIT".into(), COMMIT.into()),
            ("BUILD_RUN_ID".into(), String::new()),
            ("INPUT_SHA256".into(), "0".repeat(64)),
            ("SUBMISSION_ID".into(), String::new()),
            ("GITHUB_OUTPUT".into(), path("outputs")),
            ("GITHUB_STEP_SUMMARY".into(), path("summary")),
            ("MOCK_LOG".into(), path("commands")),
            (
                "KASSIBER_RELEASE_TOOL".into(),
                bin.join("kassiber-release").to_string_lossy().into_owned(),
            ),
        ]);
        Self { root, env }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn run(&self, name: &str, overrides: &[(&str, &str)]) -> Output {
        let script = step(&workflow(), name)["run"].as_str().unwrap().to_owned();
        let mut command = Command::new("/bin/bash");
        command
            .arg("-c")
            .arg(script)
            .current_dir(self.root.path())
            .env_clear()
            .envs(&self.env);
        for (key, value) in overrides {
            command.env(key, value);
        }
        command.output().unwrap()
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.path(name)).unwrap_or_default()
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.read("commands")
            .lines()
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
    }

    fn prepare_promotion(&self) {
        fs::create_dir(self.path("incoming")).unwrap();
        fs::write(
            self.path("incoming/kassiber-macos-signing-input.dmg"),
            "input",
        )
        .unwrap();
        fs::create_dir(self.path("notarized")).unwrap();
        for name in [
            "kassiber-macos-arm64.app.zip",
            "kassiber-macos-arm64.dmg",
            "kassiber-cli-macos-arm64.tar.gz",
        ] {
            fs::write(self.path("notarized").join(name), "sealed").unwrap();
        }
    }
}

const BIND: &str = "Download draft and bind source commit";
const PROMOTE: &str = "Update verified draft macOS artifacts";

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn has_call(calls: &[Vec<String>], prefix: &[&str]) -> bool {
    calls
        .iter()
        .any(|call| call.len() >= prefix.len() && call[..prefix.len()] == *prefix)
}

#[test]
fn candidate_uses_source_version_without_a_git_tag() {
    let runner = Runner::new();
    let result = runner.run(BIND, &[]);
    assert!(result.status.success(), "{}", stderr(&result));
    let outputs = runner.read("outputs");
    assert!(outputs.contains(&format!("commit={COMMIT}")), "{outputs}");
    assert!(outputs.contains("version=1.2.3"), "{outputs}");
    assert!(
        outputs.contains(&format!("candidate_id={}", candidate())),
        "{outputs}"
    );
    let calls = runner.calls();
    assert!(!has_call(&calls, &["git", "rev-parse"]));
    assert!(has_call(&calls, &["git", "merge-base", "--is-ancestor"]));
}

#[test]
fn invalid_candidate_never_downloads_input() {
    let foreign = "b".repeat(40);
    let overrides: [(&str, &str); 8] = [
        ("SOURCE_COMMIT", "short"),
        ("TAG", "v1.2.3"),
        ("MOCK_TARGET", "main"),
        ("MOCK_TARGET", &foreign),
        ("MOCK_TAG", "1"),
        ("MOCK_FOREIGN", "1"),
        ("MOCK_DRAFT", "false"),
        ("MOCK_TAG_FAILURE", "1"),
    ];
    for (key, value) in overrides {
        let runner = Runner::new();
        assert!(
            !runner.run(BIND, &[(key, value)]).status.success(),
            "{key}={value}"
        );
        assert!(
            !runner
                .path("incoming/kassiber-macos-signing-input.dmg")
                .exists(),
            "{key}={value}"
        );
    }
}

#[test]
fn version_release_keeps_its_tag_path() {
    let runner = Runner::new();
    let result = runner.run(
        BIND,
        &[
            ("CANDIDATE", "false"),
            ("SOURCE_COMMIT", ""),
            ("TAG", "v1.2.3"),
        ],
    );
    assert!(result.status.success(), "{}", stderr(&result));
    assert!(runner
        .read("outputs")
        .contains(&format!("commit={COMMIT}\nversion=1.2.3")));
    assert!(has_call(
        &runner.calls(),
        &["git", "rev-parse", "refs/tags/v1.2.3^{commit}"]
    ));
}

#[test]
fn candidate_promotion_rechecks_draft_and_never_finalizes() {
    let foreign = "b".repeat(40);
    let cases: [&[(&str, &str)]; 7] = [
        &[],
        &[("MOCK_TARGET", &foreign)],
        &[("MOCK_TAG", "1")],
        &[("MOCK_DRAFT", "false")],
        &[("MOCK_SIGNED", "1")],
        &[("MOCK_TAG_FAILURE", "1")],
        &[("MOCK_ASSET_FAILURE", "1")],
    ];
    for case in cases {
        let runner = Runner::new();
        runner.prepare_promotion();
        let candidate = candidate();
        let mut overrides = vec![
            ("COMMIT", COMMIT),
            ("CANDIDATE_ID", candidate.as_str()),
            ("VERSION", "1.2.3"),
        ];
        overrides.extend_from_slice(case);
        let result = runner.run(PROMOTE, &overrides);
        let calls = runner.calls();
        let mutations = calls
            .iter()
            .filter(|call| {
                call.len() >= 3
                    && call[0] == "gh"
                    && call[1] == "release"
                    && (call[2] == "upload" || call[2] == "delete-asset")
            })
            .count();
        if case.is_empty() {
            assert!(result.status.success(), "{}", stderr(&result));
            assert_eq!(mutations, 2);
            assert!(runner.read("summary").contains("remains unpublished"));
            let manifests = fs::read_dir(runner.path("incoming"))
                .unwrap()
                .filter(|entry| {
                    entry
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .contains("manifest")
                })
                .count();
            assert_eq!(manifests, 0);
        } else {
            assert!(!result.status.success(), "{case:?}");
            assert_eq!(mutations, 0, "{case:?}");
        }
        assert!(!has_call(&calls, &["gh", "release", "edit"]), "{case:?}");
    }
}

#[test]
fn workflow_forwards_candidate_mode_to_notary_and_verifier() {
    let workflow = workflow();
    let body = step(&workflow, "Notarize and verify sealed distributions")["run"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(body.matches("--candidate-id \"$CANDIDATE_ID\"").count(), 2);
    assert!(body.contains("--commit \"$COMMIT\" --version \"$VERSION\""));
    let text = serde_yaml::to_string(&workflow).unwrap();
    for forbidden in [
        "finalize-signed-release",
        "gh release create",
        "gh release edit",
    ] {
        assert!(!text.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn promotion_succeeds_without_remote_handoff_asset() {
    let runner = Runner::new();
    runner.prepare_promotion();
    let candidate = candidate();
    let result = runner.run(
        PROMOTE,
        &[
            ("COMMIT", COMMIT),
            ("CANDIDATE_ID", &candidate),
            ("VERSION", "1.2.3"),
            ("MOCK_NO_INPUT", "1"),
        ],
    );
    assert!(result.status.success(), "{}", stderr(&result));
    assert!(!has_call(
        &runner.calls(),
        &["gh", "release", "delete-asset"]
    ));
}

#[test]
fn invalid_resume_fails_before_downloading() {
    let runner = Runner::new();
    assert!(!runner
        .run(BIND, &[("SUBMISSION_ID", "invalid")])
        .status
        .success());
    assert!(!runner.path("incoming").exists());
}

#[test]
fn pending_workflow_cannot_promote_and_retains_receipt() {
    let workflow = workflow();
    assert_eq!(
        step(&workflow, PROMOTE)["if"].as_str(),
        Some("steps.notary.outputs.status == 'Accepted'")
    );
    assert!(
        step(&workflow, "Preserve notarization evidence")["with"]["path"]
            .as_str()
            .unwrap()
            .contains("notarized/submission.json")
    );
    assert!(step(&workflow, "Sign exact release build in CI")["run"]
        .as_str()
        .unwrap()
        .contains("gh release upload"));
    assert_eq!(workflow["permissions"]["actions"].as_str(), Some("read"));
}
