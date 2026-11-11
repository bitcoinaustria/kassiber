//! `update` and `verify-download`: no case may contact GitHub (consent,
//! offline mode or the disabling variable stop every update check first,
//! under the strict guard), and release signatures use throwaway keys whose
//! public halves and signatures are frozen under `tests/black_box/release/`.
//! Replaces the kill-holding tests of `tests/test_rust_release_cli.py`.

use std::path::Path;

use crate::shell::shell;
use crate::support::{Sandbox, Transcript};

const MANIFEST: &str = "kassiber-9.8.7-rc.1-manifest.txt";

fn sandbox() -> Sandbox {
    let mut sandbox = Sandbox::empty();
    shell(&mut sandbox);
    sandbox
        .unset("SSH_AUTH_SOCK")
        .unset("KASSIBER_DISABLE_UPDATE_CHECK");
    sandbox
}

type Setup = fn(&Path);
/// argv, extra environment, preference-file setup.
type Case<'a> = (Vec<String>, Vec<(&'a str, &'a str)>, Setup);

fn nothing(_: &Path) {}

fn put(prefs: &Path, name: &str, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = prefs.join(name);
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn enabled(prefs: &Path) {
    put(
        prefs,
        "update-checks.json",
        "{\"enabled\": true, \"schema_version\": 1}\n",
    );
}

fn offline(prefs: &Path) {
    put(
        prefs,
        "offline.json",
        "{\"enabled\": true, \"schema_version\": 1}\n",
    );
}

/// Each case from fresh preference files; the outcome and the files after.
fn run_cases(
    t: &mut Transcript,
    sandbox: &mut Sandbox,
    cases: &[Case<'_>],
) -> Vec<(crate::support::Output, String)> {
    let prefs = sandbox.path("prefs");
    let mut results = Vec::new();
    for (argv, extra, setup) in cases {
        if prefs.exists() {
            std::fs::remove_dir_all(&prefs).unwrap();
        }
        std::fs::create_dir_all(&prefs).unwrap();
        setup(&prefs);
        let p = |name: &str| prefs.join(name).to_string_lossy().into_owned();
        sandbox
            .set("KASSIBER_UPDATE_PREFERENCE_FILE", p("update-checks.json"))
            .set("KASSIBER_UPDATE_CACHE_FILE", p("update-check.json"))
            .set("KASSIBER_OFFLINE_PREFERENCE_FILE", p("offline.json"))
            .unset("KASSIBER_DISABLE_UPDATE_CHECK");
        for (key, value) in extra {
            sandbox.set(key, *value);
        }
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let out = sandbox.raw(argv, None);
        t.output(sandbox, &args, &out);
        let mut names: Vec<_> = std::fs::read_dir(&prefs)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        names.sort();
        let mut files = String::new();
        for path in names {
            files.push_str(&format!(
                "{}: {:?}\n",
                path.file_name().unwrap().to_string_lossy(),
                String::from_utf8_lossy(&std::fs::read(&path).unwrap())
            ));
        }
        t.section("preference files", &files);
        results.push((out, files));
    }
    results
}

fn strings(argv: &[&str]) -> Vec<String> {
    argv.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn test_update_preferences_and_refusals_match_python_without_contacting_github() {
    let mut sandbox = sandbox();
    let mut t = Transcript::new("release_cli/update_preferences_and_refusals");
    for fmt in ["json", "table", "plain"] {
        t.note(&format!("format {fmt}"));
        let base = |extra: &[&str]| {
            let mut argv = vec!["--format", fmt, "update"];
            argv.extend_from_slice(extra);
            strings(&argv)
        };
        let cases: Vec<Case<'_>> = vec![
            (base(&["--status"]), vec![], nothing),
            (base(&["--disable-checks"]), vec![], enabled),
            (base(&["--status"]), vec![], enabled),
            (
                base(&["--status"]),
                vec![("KASSIBER_DISABLE_UPDATE_CHECK", " Yes ")],
                enabled,
            ),
            (base(&["--status"]), vec![], |p| {
                put(
                    p,
                    "update-checks.json",
                    "{\"enabled\": true, \"schema_version\": true}",
                )
            }),
            (base(&["--status"]), vec![], |p| {
                put(
                    p,
                    "update-checks.json",
                    "{\"enabled\": \"yes\", \"schema_version\": 1}",
                )
            }),
            (base(&["--status"]), vec![], |p| {
                put(p, "update-checks.json", "not json")
            }),
            (base(&["--status"]), vec![], |p| {
                put(p, "update-checks.json", &"x".repeat(2000))
            }),
            (base(&[]), vec![], nothing),
            (base(&[]), vec![], |p| {
                enabled(p);
                offline(p);
            }),
            (base(&["--enable-checks"]), vec![], offline),
            (base(&[]), vec![], |p| put(p, "offline.json", "broken")),
            (base(&["--status", "--disable-checks"]), vec![], nothing),
            (
                strings(&["--machine", "update", "--status"]),
                vec![],
                nothing,
            ),
        ];
        let results = run_cases(&mut t, &mut sandbox, &cases);
        let (out, _) = &results[8];
        let both = format!("{}{}", out.text(), String::from_utf8_lossy(&out.stderr));
        assert!(both.contains("update checks are disabled"), "{both}");
        assert!(results[10].1.contains(
            "update-checks.json: \"{\\\"enabled\\\": true, \\\"schema_version\\\": 1}\\n\""
        ));
        assert!(!results[10].1.contains("update-check.json"));
    }
    t.finish();
}

#[test]
fn test_verify_download_matches_python_for_success_and_every_refusal() {
    let mut sandbox = sandbox();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/black_box/release");
    let d = sandbox.path("release");
    std::fs::create_dir_all(d.join("renamed")).unwrap();
    std::fs::create_dir_all(d.join("tampered")).unwrap();
    for name in [
        "kassiber-cli-linux-x64.tar.gz",
        "kassiber-macos-arm64.dmg",
        MANIFEST,
        "kassiber-9.8.7-rc.1-manifest.txt.sig",
        "other.sig",
        "namespace.sig",
        "release.pub",
    ] {
        std::fs::copy(fixtures.join(name), d.join(name)).unwrap();
    }
    std::fs::write(d.join("garbage.sig"), b"not a signature\n").unwrap();
    std::fs::copy(
        fixtures.join(MANIFEST),
        d.join("renamed/kassiber-9.9.9-manifest.txt"),
    )
    .unwrap();
    std::fs::copy(
        fixtures.join(MANIFEST),
        d.join("renamed/release-manifest.txt"),
    )
    .unwrap();
    std::fs::write(d.join("tampered.tar.gz"), b"tampered").unwrap();
    std::fs::write(
        d.join("tampered/kassiber-cli-linux-x64.tar.gz"),
        b"tampered",
    )
    .unwrap();
    let public = std::fs::read_to_string(fixtures.join("release.pub")).unwrap();
    std::fs::write(d.join("two.pub"), public.repeat(2)).unwrap();
    std::fs::write(
        d.join("rsa.pub"),
        "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ== x\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(d.join(MANIFEST), d.join("manifest-link.txt")).unwrap();
    let fingerprint = std::fs::read_to_string(fixtures.join("release.fingerprint")).unwrap();
    let fp = fingerprint.trim();
    let other_fp = std::fs::read_to_string(fixtures.join("other.fingerprint")).unwrap();
    let path = |name: &str| d.join(name).to_string_lossy().into_owned();
    let verify = |artifact: String,
                  manifest: String,
                  sig: String,
                  key: String,
                  fingerprint: &str,
                  fmt: &str| {
        strings(&[
            "--format",
            fmt,
            "verify-download",
            &artifact,
            "--manifest",
            &manifest,
            "--signature",
            &sig,
            "--public-key",
            &key,
            "--fingerprint",
            fingerprint,
        ])
    };
    let artifact = path("kassiber-cli-linux-x64.tar.gz");
    let manifest = path(MANIFEST);
    let sig = path("kassiber-9.8.7-rc.1-manifest.txt.sig");
    let key = path("release.pub");
    let ok = |fmt: &str| {
        verify(
            artifact.clone(),
            manifest.clone(),
            sig.clone(),
            key.clone(),
            fp,
            fmt,
        )
    };
    let padded = format!("  {fp}\n");
    let mut machine = vec!["--machine".to_owned()];
    machine.extend(ok("json")[2..].iter().cloned());
    let with_manifest =
        |m: String| verify(artifact.clone(), m, sig.clone(), key.clone(), fp, "json");
    let with_sig = |s: String| {
        verify(
            artifact.clone(),
            manifest.clone(),
            s,
            key.clone(),
            fp,
            "json",
        )
    };
    let with_key = |k: String| {
        verify(
            artifact.clone(),
            manifest.clone(),
            sig.clone(),
            k,
            fp,
            "json",
        )
    };
    let with_artifact =
        |a: String| verify(a, manifest.clone(), sig.clone(), key.clone(), fp, "json");
    let with_fp = |f: &str| {
        verify(
            artifact.clone(),
            manifest.clone(),
            sig.clone(),
            key.clone(),
            f,
            "json",
        )
    };
    let cases: Vec<Case<'_>> = vec![
        (ok("json"), vec![], nothing),
        (ok("table"), vec![], nothing),
        (ok("plain"), vec![], nothing),
        (machine, vec![], nothing),
        (with_fp(&padded), vec![], nothing),
        (with_fp("SHA256:short"), vec![], nothing),
        (with_fp(other_fp.trim()), vec![], nothing),
        (with_sig(path("other.sig")), vec![], nothing),
        (with_sig(path("namespace.sig")), vec![], nothing),
        (with_sig(path("garbage.sig")), vec![], nothing),
        (with_key(path("two.pub")), vec![], nothing),
        (with_key(path("rsa.pub")), vec![], nothing),
        (with_key(path("missing.pub")), vec![], nothing),
        (with_manifest(path("manifest-link.txt")), vec![], nothing),
        (
            with_manifest(d.to_string_lossy().into_owned()),
            vec![],
            nothing,
        ),
        (
            with_manifest(path("renamed/kassiber-9.9.9-manifest.txt")),
            vec![],
            nothing,
        ),
        (
            with_manifest(path("renamed/release-manifest.txt")),
            vec![],
            nothing,
        ),
        (with_artifact(path("tampered.tar.gz")), vec![], nothing),
        (
            with_artifact(path("tampered/kassiber-cli-linux-x64.tar.gz")),
            vec![],
            nothing,
        ),
        (
            with_artifact(path("kassiber-macos-arm64.dmg")),
            vec![],
            nothing,
        ),
    ];
    let mut t = Transcript::new("release_cli/verify_download");
    let results = run_cases(&mut t, &mut sandbox, &cases);
    assert_eq!(results[0].0.json()["data"]["verified"], true);
    // The SSHSIG is checked in process: no OpenSSH on PATH is needed.
    let empty = sandbox.path("empty-bin");
    std::fs::create_dir_all(&empty).unwrap();
    sandbox.set("PATH", empty.to_string_lossy().into_owned());
    let out = sandbox.raw(&ok("json"), None);
    assert_eq!(out.code, results[0].0.code);
    assert_eq!(out.stdout, results[0].0.stdout);
    t.finish();
}
