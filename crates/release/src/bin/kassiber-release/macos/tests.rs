//! Ports of tests/test_macos_release.py: the release guards, without Apple
//! credentials or a signed artifact. Platform tools are faked per thread.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::OsString;

use super::*;

type Fake = Box<dyn FnMut(&[String]) -> Result<String>>;

thread_local! {
    static FAKE: RefCell<Option<Fake>> = RefCell::new(None);
    static CALLS: RefCell<Vec<Vec<String>>> = const { RefCell::new(Vec::new()) };
}

pub fn fake_run(args: &[OsString]) -> Option<Result<(String, String)>> {
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    FAKE.with(|fake| {
        let mut fake = fake.borrow_mut();
        let fake = fake.as_mut()?;
        CALLS.with(|calls| calls.borrow_mut().push(args.clone()));
        Some(fake(&args).map(|stdout| (stdout, String::new())))
    })
}

fn install(fake: impl FnMut(&[String]) -> Result<String> + 'static) {
    FAKE.with(|slot| *slot.borrow_mut() = Some(Box::new(fake)));
    CALLS.with(|calls| calls.borrow_mut().clear());
}

fn calls() -> Vec<Vec<String>> {
    CALLS.with(|calls| calls.borrow().clone())
}

fn scratch() -> tempdir::TempDir {
    tempdir::TempDir::new("kassiber-release-test-").unwrap()
}

fn json(text: &str) -> Json {
    kassiber_pytext::loads(text).unwrap()
}

/// A one-member stored ZIP with exactly this name and `external_attr`
/// (`mode << 16`), as the Python test built with `zipfile.ZipInfo`.
fn zip_with(path: &Path, name: &str, mode: u32) {
    let data = b"bad";
    let crc = crc32(data);
    let mut out = Vec::new();
    let local = |out: &mut Vec<u8>| {
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend(20u16.to_le_bytes()); // version needed
        out.extend(0u16.to_le_bytes()); // flags
        out.extend(0u16.to_le_bytes()); // stored
        out.extend([0u8; 4]); // time, date
        out.extend(crc.to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(name.as_bytes());
        out.extend(data);
    };
    local(&mut out);
    let central = out.len() as u32;
    out.extend(0x0201_4b50u32.to_le_bytes());
    out.extend((3u16 << 8 | 20).to_le_bytes()); // made by Unix
    out.extend(20u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(crc.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend((name.len() as u16).to_le_bytes());
    out.extend([0u8; 8]); // extra, comment, disk, internal attributes
    out.extend((mode << 16).to_le_bytes());
    out.extend(0u32.to_le_bytes()); // local header offset
    out.extend(name.as_bytes());
    let size = out.len() as u32 - central;
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(central.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    std::fs::write(path, out).unwrap();
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn reject_archive_before_platform_extraction() {
    let dir = scratch();
    for (name, mode) in [
        ("../outside", 0o100644),
        ("/outside", 0o100644),
        ("Kassiber.app/../../outside", 0o100644),
        ("Kassiber.app/link", 0o120777),
        ("Kassiber.app/socket", 0o140755),
        ("Other.app/file", 0o100644),
        ("Kassiber.app\\evil", 0o100644),
    ] {
        let archive = dir.path().join("input.zip");
        zip_with(&archive, name, mode);
        install(|_| panic!("no platform tool before the member checks"));
        assert!(
            matches!(
                extract_app(&archive, &dir.path().join("out")),
                Err(Error::Value(_))
            ),
            "{name}"
        );
        assert!(calls().is_empty(), "{name}");
    }
    let archive = dir.path().join("input.zip");
    zip_with(&archive, "Kassiber.app/Contents/Info.plist", 0o100644);
    check_app_archive(&archive).unwrap();
}

#[test]
fn digest_fails_closed() {
    let dir = scratch();
    let artifact = dir.path().join("file");
    std::fs::write(&artifact, b"release").unwrap();
    check_digest(&artifact, &sha256(&artifact).unwrap()).unwrap();
    assert!(check_digest(&artifact, &"0".repeat(64)).is_err());
}

fn protected(key: &str) -> Option<String> {
    Some(
        match key {
            "CI" => "true",
            "GITHUB_ACTIONS" => "true",
            "GITHUB_WORKFLOW_REF" => SIGNING_WORKFLOW_REF,
            "GITHUB_EVENT_NAME" => "workflow_dispatch",
            "GITHUB_REF" => "refs/heads/main",
            _ => return None,
        }
        .into(),
    )
}

#[test]
fn signing_refuses_untrusted_ci_and_allows_local_and_protected() {
    for (key, value) in [
        (
            "GITHUB_WORKFLOW_REF",
            "bitcoinaustria/kassiber/.github/workflows/ci.yml@refs/heads/main",
        ),
        (
            "GITHUB_WORKFLOW_REF",
            "someone/fork/.github/workflows/notarize-macos.yml@refs/heads/main",
        ),
        ("GITHUB_EVENT_NAME", "pull_request"),
        ("GITHUB_REF", "refs/heads/feature"),
        ("GITHUB_ACTIONS", ""),
    ] {
        let env = |name: &str| {
            if name == key {
                Some(value.to_owned())
            } else {
                protected(name)
            }
        };
        match require_trusted_signing_context(&env) {
            Err(Error::Value(message)) => assert!(message.contains("protected notarize-macos")),
            other => panic!("{key}={value}: {other:?}"),
        }
    }
    require_trusted_signing_context(&|_| None).unwrap();
    require_trusted_signing_context(&protected).unwrap();
}

fn write_app(dir: &Path, version: &str) -> PathBuf {
    let app = dir.join("Kassiber.app");
    std::fs::create_dir_all(app.join("Contents")).unwrap();
    std::fs::write(
        app.join("Contents/Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\t<key>CFBundleIdentifier</key>\n\t<string>{APP_ID}</string>\n\t<key>CFBundleShortVersionString</key>\n\t<string>{version}</string>\n</dict>\n</plist>\n"
        ),
    )
    .unwrap();
    app
}

#[test]
fn embedded_build_identity() {
    let dir = scratch();
    let app = write_app(dir.path(), "1.2.3");
    let metadata = app.join("Contents/BUILD_INFO.json");
    let commit = "a".repeat(40);
    std::fs::write(
        &metadata,
        format!(r#"{{"commit": "{commit}", "version": "1.2.3", "channel": "release"}}"#),
    )
    .unwrap();
    validate_app(&app, &commit, "1.2.3").unwrap();
    assert!(validate_app(&app, &"b".repeat(40), "1.2.3").is_err());
    assert!(validate_app(&app, &"a".repeat(12), "1.2.3").is_err());
    assert!(validate_app(&app, &commit, "1.2.4").is_err());
    std::fs::remove_file(&metadata).unwrap();
    match validate_app(&app, &commit, "1.2.3") {
        Err(Error::Value(message)) => assert!(message.contains("provenance")),
        other => panic!("{other:?}"),
    }
}

fn candidate_source() -> Json {
    json(&format!(
        r#"{{"kind": "candidate", "candidate_id": "macos-candidate-{a}-123", "commit": "{a}", "version": "1.2.3", "build_run": "123", "build_attempt": 1, "unsigned_app_sha256": "{d}"}}"#,
        a = "a".repeat(40),
        d = "d".repeat(64)
    ))
}

fn with_field(source: &Json, field: &str, value: Json) -> Json {
    let Json::Dict(pairs) = source else { panic!() };
    let mut pairs = pairs.clone();
    match pairs
        .iter_mut()
        .find(|(k, _)| matches!(k, kassiber_pyjson::Key::Str(t) if t.as_str() == Some(field)))
    {
        Some(pair) => pair.1 = value,
        None => pairs.push((field.into(), value)),
    }
    Json::Dict(pairs)
}

#[test]
fn candidate_requires_explicit_mode_and_never_satisfies_release_provenance() {
    let commit = "a".repeat(40);
    let digest = "d".repeat(64);
    let source = candidate_source();
    let candidate = format!("macos-candidate-{commit}-123");
    validate_source(&source, &commit, "1.2.3", Some(&candidate), Some(&digest)).unwrap();
    assert!(validate_source(&source, &commit, "1.2.3", None, None).is_err());
    let tagged = json(&format!(
        r#"{{"tag": "v1.2.3", "commit": "{commit}", "build_run": "123", "build_attempt": 1, "unsigned_app_sha256": "{digest}"}}"#
    ));
    validate_source(&tagged, &commit, "1.2.3", None, Some(&digest)).unwrap();
    assert!(validate_source(&tagged, &commit, "1.2.3", Some(&candidate), None).is_err());
    // An integer build run is `str()`-ed as the script did.
    let numeric = with_field(&tagged, "build_run", json("123"));
    validate_source(&numeric, &commit, "1.2.3", None, None).unwrap();
}

#[test]
fn candidate_provenance_mismatch_fails_closed() {
    let commit = "a".repeat(40);
    let expected = format!("macos-candidate-{commit}-123");
    for (field, value) in [
        ("commit", s("b".repeat(40))),
        ("version", s("1.2.4")),
        ("kind", s("release")),
        ("candidate_id", s(format!("macos-candidate-{commit}-124"))),
        ("build_run", s("124")),
        ("build_attempt", Json::Int(0.into())),
        ("build_attempt", Json::Bool(true)),
        ("unsigned_app_sha256", s("bad")),
        ("tag", s("v1.2.3")),
    ] {
        let source = with_field(&candidate_source(), field, value);
        assert!(
            validate_source(
                &source,
                &commit,
                "1.2.3",
                Some(&expected),
                Some(&"d".repeat(64))
            )
            .is_err(),
            "{field}"
        );
    }
    for candidate in [
        "v1.2.3".to_owned(),
        "macos-candidate-short-123".to_owned(),
        format!("macos-candidate-{}-123", "b".repeat(40)),
    ] {
        assert!(validate_source(
            &candidate_source(),
            &commit,
            "1.2.3",
            Some(&candidate),
            None
        )
        .is_err());
    }
}

#[test]
fn macho_inventory_rejects_symlinks() {
    let dir = scratch();
    std::fs::write(dir.path().join("lib"), [0xcf, 0xfa, 0xed, 0xfe, b'c']).unwrap();
    std::fs::write(dir.path().join("data"), b"data").unwrap();
    assert_eq!(
        code_files(dir.path()).unwrap(),
        vec![dir.path().join("lib")]
    );
    std::os::unix::fs::symlink("lib", dir.path().join("link")).unwrap();
    assert!(code_files(dir.path()).is_err());
}

type Slice = (Option<String>, Vec<String>, Vec<String>);

/// The relevant real `otool -arch all -l` shape, fat-binary headers included.
fn linkage_output(path: &str, slices: &[Slice]) -> String {
    let mut lines = Vec::new();
    for (index, (install_id, dependencies, rpaths)) in slices.iter().enumerate() {
        let arch = if index == 0 { "arm64" } else { "x86_64" };
        lines.push(format!("{path} (architecture {arch}):"));
        let mut commands = vec![("LC_SEGMENT_64", None)];
        if let Some(id) = install_id {
            commands.push(("LC_ID_DYLIB", Some(id.clone())));
        }
        commands.extend(
            dependencies
                .iter()
                .map(|d| ("LC_LOAD_DYLIB", Some(d.clone()))),
        );
        commands.extend(rpaths.iter().map(|r| ("LC_RPATH", Some(r.clone()))));
        for (number, (command, name)) in commands.into_iter().enumerate() {
            lines.push(format!("Load command {number}"));
            lines.push(format!("          cmd {command}"));
            lines.push("      cmdsize 64".into());
            if let Some(name) = name {
                let field = if command == "LC_RPATH" {
                    "path"
                } else {
                    "name"
                };
                lines.push(format!("         {field} {name} (offset 24)"));
            }
        }
    }
    lines.join("\n") + "\n"
}

struct MachO {
    _dir: tempdir::TempDir,
    app: PathBuf,
    library: PathBuf,
    slices: std::rc::Rc<RefCell<Vec<Slice>>>,
}

fn macho() -> MachO {
    let dir = scratch();
    let app = dir.path().join("Kassiber.app");
    let library = app.join("Contents/Resources/sidecar/_internal/libsecp256k1_darwin_x86_64.dylib");
    std::fs::create_dir_all(library.parent().unwrap()).unwrap();
    std::fs::write(&library, [0xcf, 0xfa, 0xed, 0xfe]).unwrap();
    let name = library.file_name().unwrap().to_string_lossy().into_owned();
    let slices = std::rc::Rc::new(RefCell::new(vec![(
        Some(format!("build/{name}")),
        vec!["/usr/lib/libSystem.B.dylib".to_owned()],
        vec!["@loader_path".to_owned()],
    )]));
    let state = slices.clone();
    let expected = library.to_string_lossy().into_owned();
    install(move |command| {
        if command[..4] == ["/usr/bin/otool", "-arch", "all", "-l"] {
            assert_eq!(command[4], expected);
            return Ok(linkage_output(&command[4], &state.borrow()));
        }
        assert_eq!(command[..2], ["/usr/bin/install_name_tool", "-id"]);
        assert_eq!(command[3], expected);
        let id = command[2].clone();
        for slice in state.borrow_mut().iter_mut() {
            slice.0 = Some(id.clone());
        }
        Ok(String::new())
    });
    MachO {
        _dir: dir,
        app,
        library,
        slices,
    }
}

fn renames() -> usize {
    calls()
        .iter()
        .filter(|c| c[0] == "/usr/bin/install_name_tool")
        .count()
}

#[test]
fn normalize_install_id_before_signing() {
    let fixture = macho();
    match verify_linkage(&fixture.app) {
        Err(Error::Value(message)) => assert!(message.contains("install ID")),
        other => panic!("{other:?}"),
    }
    prepare_linkage(&fixture.app).unwrap();
    verify_linkage(&fixture.app).unwrap();
    let name = fixture.library.file_name().unwrap().to_string_lossy();
    assert_eq!(fixture.slices.borrow()[0].0, Some(format!("@rpath/{name}")));
    assert_eq!(renames(), 1);
}

#[test]
fn unsafe_linkage_is_rejected_before_any_mutation() {
    for (kind, value) in [
        ("dependency", "/opt/homebrew/opt/openssl/lib/libssl.dylib"),
        ("dependency", "build/libsecp.dylib"),
        ("dependency", "@rpath/../libsecp.dylib"),
        ("dependency", "/usr/lib/../../tmp/libsecp.dylib"),
        ("dependency", "@loader_path/../../../../../../outside.dylib"),
        ("rpath", "/private/tmp/build/lib"),
        ("rpath", "lib"),
        ("rpath", "@rpath/lib"),
        ("rpath", "@loader_path/../../../../../../outside"),
        ("rpath", "@executable_path/../../../outside"),
        ("rpath", "@loader_path"), // Duplicate within one architecture.
    ] {
        let fixture = macho();
        {
            let mut slices = fixture.slices.borrow_mut();
            if kind == "dependency" {
                slices[0].1.push(value.into());
            } else {
                slices[0].2.push(value.into());
            }
        }
        assert!(prepare_linkage(&fixture.app).is_err(), "{kind} {value}");
        assert_eq!(renames(), 0, "{kind} {value}");
    }
}

#[test]
fn universal_linkage_is_validated_per_architecture() {
    let fixture = macho();
    let name = fixture
        .library
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    *fixture.slices.borrow_mut() = (0..2)
        .map(|_| {
            (
                Some(format!("@rpath/{name}")),
                vec!["@rpath/libother.dylib".to_owned()],
                vec!["@loader_path/..".to_owned()],
            )
        })
        .collect();
    // The same rpath in two slices is not a duplicate.
    verify_linkage(&fixture.app).unwrap();
    assert_eq!(renames(), 0);
    fixture.slices.borrow_mut()[1]
        .2
        .push("/usr/local/lib".into());
    assert!(verify_linkage(&fixture.app).is_err());
}

#[test]
fn ambiguous_universal_install_ids_fail_closed() {
    let fixture = macho();
    fixture
        .slices
        .borrow_mut()
        .push((Some("@rpath/different.dylib".into()), vec![], vec![]));
    assert!(prepare_linkage(&fixture.app).is_err());
    assert_eq!(renames(), 0);
}

#[test]
fn unknown_or_incomplete_linkage_output_fails_closed() {
    for output in [
        "",
        "not Mach-O\n",
        "file:\nLoad command 0\n cmd LC_RPATH\n cmdsize 32\n",
        "file:\nLoad command 0\n cmd LC_DYLD_ENVIRONMENT\n cmdsize 32\n",
    ] {
        assert!(parse_linkage(output).is_err(), "{output:?}");
    }
    for command in [
        "LC_LOAD_WEAK_DYLIB",
        "LC_REEXPORT_DYLIB",
        "LC_LOAD_UPWARD_DYLIB",
        "LC_LAZY_LOAD_DYLIB",
    ] {
        let output = linkage_output(
            "library",
            &[(None, vec!["/opt/local/lib/unsafe.dylib".into()], vec![])],
        )
        .replace("LC_LOAD_DYLIB", command);
        assert_eq!(
            parse_linkage(&output).unwrap()[0].dependencies,
            vec!["/opt/local/lib/unsafe.dylib".to_owned()]
        );
    }
}

fn plist_date(offset: i64) -> String {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    crate::notes::iso(now + offset, 0)
}

fn profile(expires: &str, groups: &str) -> plist::Value {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
         <key>TeamIdentifier</key><array><string>{TEAM}</string></array>\
         <key>ProvisionsAllDevices</key><true/>\
         <key>ExpirationDate</key><date>{expires}</date>\
         <key>Entitlements</key><dict>\
         <key>com.apple.application-identifier</key><string>{TEAM}.{APP_ID}</string>\
         <key>com.apple.developer.team-identifier</key><string>{TEAM}</string>\
         <key>keychain-access-groups</key><array>{groups}</array>\
         </dict></dict></plist>"
    );
    parse_plist(xml.as_bytes()).unwrap()
}

#[test]
fn profile_contract() {
    let now = SystemTime::now();
    let exact = format!("<string>{TEAM}.{APP_ID}</string>");
    validate_profile_data(&profile(&plist_date(86_400), &exact), now).unwrap();
    validate_profile_data(
        &profile(&plist_date(86_400), &format!("<string>{TEAM}.*</string>")),
        now,
    )
    .unwrap();
    assert!(validate_profile_data(&profile(&plist_date(-86_400), &exact), now).is_err());
    assert!(validate_profile_data(
        &profile(&plist_date(86_400), "<string>OTHER.*</string>"),
        now
    )
    .is_err());
    assert!(
        validate_profile_data(&plist::Value::Dictionary(plist::Dictionary::new()), now).is_err()
    );
}

#[test]
fn entitlements_plist_is_plistlib_bytes() {
    assert_eq!(
        parse_plist(ENTITLEMENTS_PLIST.as_bytes()).unwrap(),
        app_entitlements()
    );
}

#[test]
fn sha1_vectors() {
    assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(
        hex(&sha1(b"abc")),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert_eq!(
        hex(&sha1(&[b'a'; 1000])),
        "291e9a6c66994949b57ba5e650361e98fc36b1ba"
    );
}

#[test]
fn cli_archive_round_trip_and_tampering() {
    let dir = scratch();
    let app = dir.path().join("Kassiber.app");
    std::fs::create_dir_all(app.join("Contents/Resources/bin")).unwrap();
    std::fs::write(app.join("Contents/Resources/bin/kassiber"), b"launcher").unwrap();
    std::fs::write(app.join("Contents/Info.plist"), b"info").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        app.join("Contents/Resources/bin/kassiber"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let cli_root = dir.path().join("kassiber-cli-macos-arm64");
    std::fs::create_dir(&cli_root).unwrap();
    copytree(&app, &cli_root.join("Kassiber.app")).unwrap();
    std::os::unix::fs::symlink(
        "Kassiber.app/Contents/Resources/bin/kassiber",
        cli_root.join("kassiber"),
    )
    .unwrap();
    let archive = dir.path().join(CLI_TAR);
    write_cli_archive(&cli_root, &archive).unwrap();
    verify_cli_archive(&archive, &app).unwrap();
    std::fs::write(app.join("Contents/Info.plist"), b"other").unwrap();
    assert!(verify_cli_archive(&archive, &app).is_err());
    std::fs::write(app.join("Contents/Info.plist"), b"info").unwrap();
    std::fs::write(app.join("Contents/extra"), b"x").unwrap();
    assert!(verify_cli_archive(&archive, &app).is_err());
}

const SUBMISSION: &str = "b5e759d9-5951-453c-9462-e0275ff19a3d";

fn notary_args(dir: &Path, submission_id: &str) -> NotaryArgs {
    NotaryArgs {
        image: dir.join("input.dmg"),
        sha256: "a".repeat(64),
        commit: "b".repeat(40),
        version: "1.2.3".into(),
        candidate_id: None,
        output: dir.join("result"),
        profile: "profile".into(),
        keychain: dir.join("notary.keychain"),
        submission_id: submission_id.into(),
        wait_seconds: 0,
        pending_ok: true,
    }
}

#[test]
fn resume_queries_existing_submission_and_retains_binding() {
    let dir = scratch();
    install(|_| {
        Ok(format!(
            r#"{{"id": "{SUBMISSION}", "status": "In Progress"}}"#
        ))
    });
    let evidence = dir.path().join("notarization.json");
    let result = submit_and_wait(&notary_args(dir.path(), SUBMISSION), &evidence).unwrap();
    assert_eq!(status(&result), Some("In Progress"));
    let calls = calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][1..4], ["notarytool", "info", SUBMISSION]);
    assert_eq!(
        std::fs::read_to_string(&evidence).unwrap(),
        format!("{{\n  \"id\": \"{SUBMISSION}\",\n  \"status\": \"In Progress\"\n}}\n")
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("submission.json")).unwrap(),
        format!(
            "{{\n  \"submission_id\": \"{SUBMISSION}\",\n  \"sha256\": \"{}\",\n  \"commit\": \"{}\",\n  \"version\": \"1.2.3\",\n  \"candidate_id\": null\n}}\n",
            "a".repeat(64),
            "b".repeat(40)
        )
    );
}

#[test]
fn resume_retries_failed_read_without_submitting_and_submit_never_retries() {
    let dir = scratch();
    let mut replies: VecDeque<Result<String>> = VecDeque::from([
        Err(Error::Process),
        Ok(format!(r#"{{"id": "{SUBMISSION}", "status": "Accepted"}}"#)),
    ]);
    install(move |_| replies.pop_front().unwrap());
    let result = submit_and_wait(
        &notary_args(dir.path(), SUBMISSION),
        &dir.path().join("evidence.json"),
    )
    .unwrap();
    assert_eq!(status(&result), Some("Accepted"));
    assert!(calls().iter().all(|call| call[2] == "info"));
    install(|_| Err(Error::Process));
    assert!(matches!(
        submit_and_wait(
            &notary_args(dir.path(), ""),
            &dir.path().join("evidence.json")
        ),
        Err(Error::Process)
    ));
    assert_eq!(calls().len(), 1);
    assert!(matches!(
        submit_and_wait(
            &notary_args(dir.path(), "not-a-uuid"),
            &dir.path().join("e.json")
        ),
        Err(Error::Value(_))
    ));
}

fn prepare_args(dir: &Path, submit: bool) -> PrepareArgs {
    PrepareArgs {
        tag: "v1.2.3".into(),
        run_id: "1".into(),
        identity: "c".repeat(40),
        provisioning_profile: dir.join("profile"),
        work_dir: dir.join("work"),
        submit,
    }
}

#[test]
fn untrusted_build_never_reaches_signer() {
    for (field, value) in [
        ("conclusion", s("failure")),
        ("event", s("pull_request")),
        ("head_sha", s("b".repeat(40))),
        ("path", s("attacker.yml")),
        (
            "head_repository",
            json(r#"{"full_name": "attacker/kassiber"}"#),
        ),
    ] {
        let dir = scratch();
        let build = with_field(
            &json(&format!(
                r#"{{"conclusion": "success", "event": "push", "head_sha": "{}", "path": ".github/workflows/prerelease-binaries.yml", "head_repository": {{"full_name": "{REPO}"}}}}"#,
                "a".repeat(40)
            )),
            field,
            value,
        );
        let build = kassiber_pyjson::dumps(&build, &Options::new()).unwrap();
        let mut replies = VecDeque::from([
            r#"{"isDraft": true, "assets": []}"#.to_owned(),
            build,
            format!(r#"{{"sha": "{}"}}"#, "a".repeat(40)),
        ]);
        install(move |_| {
            Ok(replies
                .pop_front()
                .expect("no command after the build check"))
        });
        assert!(
            matches!(
                prepare(&prepare_args(dir.path(), true)),
                Err(Error::Value(_))
            ),
            "{field}"
        );
        assert!(!dir.path().join("work").exists());
        assert_eq!(calls().len(), 3);
    }
}

#[test]
fn initial_release_guard_prevents_signing() {
    for assets in [
        r#"{"isDraft": false, "assets": []}"#,
        r#"{"isDraft": true, "assets": [{"name": "manifest.txt.sig"}]}"#,
    ] {
        let dir = scratch();
        let reply = assets.to_owned();
        install(move |_| Ok(reply.clone()));
        match prepare(&prepare_args(dir.path(), true)) {
            Err(Error::Value(message)) => assert!(message.contains("unsigned draft")),
            other => panic!("{other:?}"),
        }
        assert_eq!(calls().len(), 1);
        assert!(!dir.path().join("work").exists());
    }
}

#[test]
fn bad_tags_and_run_ids_fail_before_any_command() {
    let dir = scratch();
    install(|_| panic!("no command"));
    for (tag, run) in [("1.2.3", "1"), ("v1.2", "1"), ("v1.2.3", "x")] {
        let mut args = prepare_args(dir.path(), false);
        args.tag = tag.into();
        args.run_id = run.into();
        assert!(matches!(prepare(&args), Err(Error::Value(_))));
    }
    assert!(tag_shape("v1.2.3-rc.1"));
}
