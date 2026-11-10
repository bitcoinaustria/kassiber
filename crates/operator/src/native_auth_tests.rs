//! `tests/test_operator_native_auth.py` over a scripted [`Platform`]: the
//! environment, `codesign` answers by target, and helper launches that act
//! on their inherited descriptors through `/dev/fd`.

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

const TEAM: &str = "6Q4R2C3GJK";
const CDHASH: &str = "8b23350488a1ecd22e6cbd549dbe926c7214f397";

fn describe(identifier: &str, team: &str, cdhash: &str) -> Vec<u8> {
    format!(
        "Executable=/x\nIdentifier={identifier}\nFormat=app bundle with Mach-O thin (arm64)\nCDHash={cdhash}\nTeamIdentifier={team}\n"
    )
    .into_bytes()
}

fn signed() -> Vec<u8> {
    describe(NATIVE_AUTH_IDENTIFIER, TEAM, CDHASH)
}

/// What a scripted helper does when launched.
#[derive(Clone, Default)]
struct Behaviour {
    /// Write `R` to the ready descriptor.
    ready: bool,
    /// Bytes to write to `--output-fd`.
    output: Vec<u8>,
    exit: i32,
    stderr: Vec<u8>,
}

#[derive(Default)]
struct Fake {
    macos: bool,
    helper: Option<OsString>,
    parent: u32,
    /// `codesign -dv` stderr by target; a missing target fails.
    described: RefCell<HashMap<String, VecDeque<Vec<u8>>>>,
    /// `codesign --verify` result by target (default: verified).
    verified: HashMap<String, bool>,
    behaviour: Behaviour,
    log: RefCell<Vec<String>>,
    launched: RefCell<Vec<Vec<String>>>,
    /// A duplicate of the `--secret-fd` read end, kept open.
    secret_reader: RefCell<Option<std::fs::File>>,
    /// The `--go-fd` read ends, held as a waiting helper holds them.
    go_readers: RefCell<Vec<std::fs::File>>,
}

use std::collections::VecDeque;

impl Fake {
    fn macos_with_helper(helper: &Path) -> Self {
        Fake {
            macos: true,
            helper: Some(helper.as_os_str().to_owned()),
            parent: 4242,
            behaviour: Behaviour {
                ready: true,
                ..Behaviour::default()
            },
            ..Fake::default()
        }
    }
    fn describes(&self, target: &str, stderr: Vec<u8>) {
        self.described
            .borrow_mut()
            .entry(target.to_owned())
            .or_default()
            .push_back(stderr);
    }
    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

struct FakeChild {
    exit: i32,
    stderr: Vec<u8>,
    waited: bool,
}

impl HelperChild for FakeChild {
    fn id(&self) -> u32 {
        777
    }
    fn poll(&mut self) -> io::Result<Option<i32>> {
        Ok(self.waited.then_some(self.exit))
    }
    fn read_stderr(&mut self) -> io::Result<Vec<u8>> {
        Ok(std::mem::take(&mut self.stderr))
    }
    fn wait(&mut self) -> io::Result<i32> {
        self.waited = true;
        Ok(self.exit)
    }
    fn kill(&mut self) -> io::Result<()> {
        self.waited = true;
        Ok(())
    }
}

fn fd_after(argv: &[String], flag: &str) -> Option<String> {
    argv.iter()
        .position(|value| value == flag)
        .map(|index| argv[index + 1].clone())
}

fn open_fd(fd: &str, write: bool) -> std::fs::File {
    std::fs::OpenOptions::new()
        .read(!write)
        .write(write)
        .open(format!("/dev/fd/{fd}"))
        .unwrap()
}

impl Platform for Fake {
    fn macos(&self) -> bool {
        self.macos
    }
    fn configured_helper(&self) -> Option<OsString> {
        self.helper.clone()
    }
    fn parent_pid(&self) -> u32 {
        self.parent
    }
    fn codesign(&self, args: &[&OsStr]) -> io::Result<(bool, Vec<u8>)> {
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        self.log
            .borrow_mut()
            .push(format!("codesign {}", args.join(" ")));
        let target = args.last().unwrap().clone();
        if args[0] == "-dv" {
            let mut described = self.described.borrow_mut();
            let queue = described.get_mut(&target);
            return Ok(match queue {
                // The last answer repeats.
                Some(queue) if queue.len() > 1 => (true, queue.pop_front().unwrap()),
                Some(queue) => (true, queue.front().cloned().unwrap_or_default()),
                None => (false, b"code object is not signed at all".to_vec()),
            });
        }
        Ok((*self.verified.get(&target).unwrap_or(&true), Vec::new()))
    }
    fn spawn(
        &self,
        argv: &[OsString],
        inherit: &[std::os::fd::RawFd],
    ) -> io::Result<Box<dyn HelperChild>> {
        let argv: Vec<String> = argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        // Every descriptor the helper is told about is inherited.
        for flag in ["--output-fd", "--secret-fd", "--ready-fd", "--go-fd"] {
            if let Some(fd) = fd_after(&argv, flag) {
                assert!(
                    inherit.contains(&fd.parse().unwrap()),
                    "{flag} not inherited"
                );
            }
        }
        self.log.borrow_mut().push(format!("spawn {}", argv[2]));
        self.launched.borrow_mut().push(argv.clone());
        if let Some(fd) = fd_after(&argv, "--output-fd") {
            open_fd(&fd, true)
                .write_all(&self.behaviour.output)
                .unwrap();
        }
        if let Some(fd) = fd_after(&argv, "--secret-fd") {
            *self.secret_reader.borrow_mut() = Some(open_fd(&fd, false));
        }
        self.go_readers
            .borrow_mut()
            .push(open_fd(&fd_after(&argv, "--go-fd").unwrap(), false));
        if self.behaviour.ready {
            open_fd(&fd_after(&argv, "--ready-fd").unwrap(), true)
                .write_all(b"R")
                .unwrap();
        }
        Ok(Box::new(FakeChild {
            exit: self.behaviour.exit,
            stderr: self.behaviour.stderr.clone(),
            waited: false,
        }))
    }
}

use std::io::Write;

/// A packaged helper path: `<tmp>/Kassiber.app/Contents/MacOS/kassiber-ui`,
/// an executable file.
fn packaged_helper(tmp: &tempfile::TempDir) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let helper = tmp
        .path()
        .join("Kassiber.app/Contents/MacOS")
        .join(MACOS_APP_EXECUTABLE_NAME);
    std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
    std::fs::write(&helper, b"signed-helper").unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::canonicalize(helper).unwrap()
}

fn signed_platform(tmp: &tempfile::TempDir) -> (Fake, String) {
    let helper = packaged_helper(tmp);
    let fake = Fake::macos_with_helper(&helper);
    let path = helper.to_string_lossy().into_owned();
    fake.describes(&path, signed());
    fake.describes("777", signed());
    fake.describes("4242", signed());
    (fake, path)
}

fn identity() -> CodeIdentity {
    CodeIdentity {
        identifier: NATIVE_AUTH_IDENTIFIER.into(),
        team: TEAM.into(),
        cdhash: CDHASH.into(),
    }
}

fn code<T: std::fmt::Debug>(result: Result<T>) -> String {
    match result.unwrap_err() {
        Error::App(error) => error.code,
        other => panic!("not an AppError: {other:?}"),
    }
}

fn message<T: std::fmt::Debug>(result: Result<T>) -> String {
    match result.unwrap_err() {
        Error::App(error) => error.message,
        other => panic!("not an AppError: {other:?}"),
    }
}

const ACCOUNT: &str = "abababababababababababababababababababababababababababababababab";

fn account() -> Result<String> {
    Ok(ACCOUNT.to_owned())
}

#[test]
fn launch_gate_keeps_helper_alive_through_pid_validation() {
    // A real child: it signals readiness, then exits 0 only after `G`.
    let script =
        "eval \"printf R >&$2\"; s=$(eval \"dd bs=1 count=1 <&$4\" 2>/dev/null); [ \"$s\" = G ]";
    let helper = NativeAuthHelper::new(SystemPlatform);
    let validated = std::cell::Cell::new(false);
    let mut launched = helper
        .spawn_gated(
            vec!["/bin/sh".into(), "-c".into(), script.into(), "sh".into()],
            &[],
            |child| {
                // Still waiting at the gate while its pid is checked.
                assert_eq!(child.poll().unwrap(), None);
                validated.set(true);
                Ok(())
            },
        )
        .unwrap();
    assert!(validated.get());
    assert_eq!(launched.child().wait().unwrap(), 0);
}

#[test]
fn launch_gate_refuses_a_helper_that_never_signals() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut fake, _) = signed_platform(&tmp);
    fake.behaviour.ready = false;
    let helper = NativeAuthHelper::new(fake);
    let result = helper.run_no_secret("status", &account, None);
    assert_eq!(
        message(result),
        "the native-auth helper did not reach its signed launch gate"
    );
}

#[test]
fn helper_identity_detects_replacement_before_secret_handoff() {
    let tmp = tempfile::tempdir().unwrap();
    let helper_path = packaged_helper(&tmp);
    let fake = Fake::macos_with_helper(&helper_path);
    let path = helper_path.to_string_lossy().into_owned();
    fake.describes(&path, signed());
    fake.describes(
        &path,
        describe(NATIVE_AUTH_IDENTIFIER, TEAM, &"c".repeat(40)),
    );
    let helper = NativeAuthHelper::new(fake);
    let expected = helper.helper_identity().unwrap();
    assert_eq!(expected, identity().opaque());
    let error = helper.helper_path(Some(&expected)).unwrap_err();
    let Error::App(error) = error else { panic!() };
    assert_eq!(error.code, "native_auth_helper_mismatch");
    assert!(error.retryable);
    assert_eq!(
        error.hint.as_deref(),
        Some("Retry from the signed macOS app CLI after stopping idle broker work.")
    );
}

#[test]
fn signed_helper_path_matches_packaged_macos_executable() {
    let fake = Fake::macos_with_helper(Path::new("/unused"));
    let packaged = "/Applications/Kassiber.app/Contents/MacOS/kassiber-ui";
    fake.describes(packaged, signed());
    let helper = NativeAuthHelper::new(fake);
    assert_eq!(
        helper.signed_helper_code(Path::new(packaged)).unwrap(),
        identity()
    );
    for outside in [
        "/Applications/Kassiber.app/Contents/MacOS/Kassiber",
        "/Applications/Kassiber.app/Contents/Resources/kassiber-ui",
        "/usr/local/bin/kassiber-ui",
        "/Applications/Kassiber.app/Contents/MacOS/sub/kassiber-ui",
    ] {
        assert_eq!(
            message(helper.signed_helper_code(Path::new(outside))),
            "the native authentication helper is outside the Kassiber app bundle",
            "{outside}"
        );
    }
    // Only the packaged path was inspected, once, and verified against its
    // own requirement.
    let requirement = identity().requirement();
    assert_eq!(
        helper.platform().log(),
        [
            format!("codesign -dv --verbose=4 {packaged}"),
            format!("codesign --verify --strict --verbose=2 -R={requirement} {packaged}"),
        ]
    );
}

#[test]
fn touch_id_status_is_truthful_without_signed_macos_helper() {
    let unavailable = dict([
        ("available", Value::Bool(false)),
        ("configured", Value::Bool(false)),
        ("reason", kassiber_envelope::s("native_auth_unavailable")),
    ]);
    for fake in [
        // No launcher set the helper.
        Fake {
            macos: true,
            ..Fake::default()
        },
        // An empty helper.
        Fake {
            macos: true,
            helper: Some(OsString::new()),
            ..Fake::default()
        },
        // Linux and Windows never use one.
        Fake {
            macos: false,
            helper: Some("/Applications/Kassiber.app/Contents/MacOS/kassiber-ui".into()),
            ..Fake::default()
        },
    ] {
        let helper = NativeAuthHelper::new(fake);
        assert_eq!(helper.status(&account).unwrap(), unavailable);
        assert!(helper.platform().log().is_empty());
        let error = helper.configured_helper_path().unwrap_err();
        let Error::App(error) = error else { panic!() };
        assert_eq!(
            error.message,
            "Touch ID operator authentication is unavailable in this build"
        );
        assert_eq!(
            error.hint.as_deref(),
            Some("Use password authentication or the signed macOS desktop CLI launcher.")
        );
    }
}

#[test]
fn touch_id_status_uses_the_live_validated_helper_gate() {
    for (exit, expected) in [
        (
            4,
            dict([
                ("available", Value::Bool(true)),
                ("configured", Value::Bool(false)),
            ]),
        ),
        (
            0,
            dict([
                ("available", Value::Bool(true)),
                ("configured", Value::Bool(true)),
            ]),
        ),
        (
            1,
            dict([
                ("available", Value::Bool(false)),
                ("configured", Value::Bool(false)),
                ("reason", kassiber_envelope::s("native_auth_failed")),
            ]),
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let (mut fake, path) = signed_platform(&tmp);
        fake.behaviour.exit = exit;
        let helper = NativeAuthHelper::new(fake);
        assert_eq!(helper.status(&account).unwrap(), expected);
        let launched = helper.platform().launched.borrow().clone();
        assert_eq!(launched.len(), 1);
        assert_eq!(
            launched[0][..5],
            [
                path.as_str(),
                "--operator-native-auth",
                "status",
                "--account",
                ACCOUNT
            ]
        );
        assert_eq!(launched[0][5], "--ready-fd");
        assert_eq!(launched[0][7], "--go-fd");
        assert_eq!(launched[0].len(), 9);
        // The live pid was checked against the helper's requirement.
        let requirement = identity().requirement();
        assert!(helper.platform().log().contains(&format!(
            "codesign --verify --strict --verbose=2 -R={requirement} 777"
        )));
    }
}

#[test]
fn touch_id_status_reports_a_refused_account_as_its_reason() {
    let tmp = tempfile::tempdir().unwrap();
    let (fake, _) = signed_platform(&tmp);
    let helper = NativeAuthHelper::new(fake);
    let unbound = || -> Result<String> {
        Err(Error::App(AppError::new(
            "operator_policy_binding_required",
            "the project unlock policy is not bound to an authenticated database",
        )))
    };
    assert_eq!(
        helper.status(&unbound).unwrap(),
        dict([
            ("available", Value::Bool(false)),
            ("configured", Value::Bool(false)),
            (
                "reason",
                kassiber_envelope::s("operator_policy_binding_required")
            ),
        ])
    );
    assert!(helper.platform().launched.borrow().is_empty());
}

#[test]
fn broker_touch_id_secret_uses_an_inherited_pipe() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut fake, _) = signed_platform(&tmp);
    fake.behaviour.output = b"touch-id-secret".to_vec();
    let helper = NativeAuthHelper::new(fake);
    let expected = identity().opaque();
    let secret = helper.broker_passphrase(&account, Some(&expected)).unwrap();
    assert_eq!(secret.as_slice(), b"touch-id-secret");
    let launched = helper.platform().launched.borrow().clone();
    let command = &launched[0];
    assert!(command.contains(&"broker-get".to_owned()));
    assert!(command.contains(&"--output-fd".to_owned()));
    // Only the opaque account names the book.
    assert!(!command
        .iter()
        .any(|value| value.contains(tmp.path().to_str().unwrap())
            && !value.ends_with(MACOS_APP_EXECUTABLE_NAME)));
}

#[test]
fn broker_touch_id_maps_the_helper_exit_codes() {
    let tmp = tempfile::tempdir().unwrap();
    /// Exit code, output, stderr, then the expected code and message.
    type Case = (
        i32,
        &'static [u8],
        &'static [u8],
        &'static str,
        &'static str,
    );
    let cases: [Case; 4] = [
        (
            4,
            b"",
            b"",
            "operator_native_auth_not_configured",
            "no Touch ID operator credential is enrolled for this project",
        ),
        (
            1,
            b"",
            b"  kassiber native auth: user canceled\n",
            "native_auth_failed",
            "kassiber native auth: user canceled",
        ),
        (
            1,
            b"",
            b"\n",
            "native_auth_failed",
            "Touch ID authentication failed",
        ),
        (
            0,
            b"",
            b"",
            "native_auth_failed",
            "the Touch ID helper returned an empty credential",
        ),
    ];
    for (exit, output, stderr, expected_code, expected_message) in cases {
        let (mut fake, _) = signed_platform(&tmp);
        fake.behaviour.exit = exit;
        fake.behaviour.output = output.to_vec();
        fake.behaviour.stderr = stderr.to_vec();
        let helper = NativeAuthHelper::new(fake);
        let error = helper.broker_passphrase(&account, None).unwrap_err();
        let Error::App(error) = error else { panic!() };
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            (expected_code, expected_message)
        );
        if exit == 4 {
            assert_eq!(
                error.hint.as_deref(),
                Some("Run `kassiber operator touch-id enroll` from the signed macOS app CLI.")
            );
        }
    }
}

#[test]
fn broker_touch_id_refuses_a_secret_over_16_kib() {
    let tmp = tempfile::tempdir().unwrap();
    for (size, accepted) in [(16 * 1024, true), (16 * 1024 + 1, false)] {
        let (mut fake, _) = signed_platform(&tmp);
        fake.behaviour.output = vec![b'x'; size];
        let helper = NativeAuthHelper::new(fake);
        let result = helper.broker_passphrase(&account, None);
        if accepted {
            assert_eq!(result.unwrap().len(), size);
        } else {
            assert_eq!(
                message(result),
                "native authentication secret exceeds 16 KiB"
            );
        }
    }
}

#[test]
fn store_validates_live_helper_before_writing_passphrase() {
    let tmp = tempfile::tempdir().unwrap();
    let (fake, path) = signed_platform(&tmp);
    // The running process is someone else's build.
    fake.described.borrow_mut().insert(
        "777".into(),
        VecDeque::from([describe(NATIVE_AUTH_IDENTIFIER, TEAM, &"e".repeat(40))]),
    );
    let helper = NativeAuthHelper::new(fake);
    let expected = identity().opaque();
    let result = helper.store(&account, b"fresh-passphrase", Some(&expected));
    assert_eq!(
        message(result),
        "the running native-auth helper does not match the signed CLI"
    );
    // Nothing reached the helper's secret pipe.
    let mut received = Vec::new();
    helper
        .platform()
        .secret_reader
        .borrow_mut()
        .take()
        .unwrap()
        .read_to_end(&mut received)
        .unwrap();
    assert!(received.is_empty());
    let launched = helper.platform().launched.borrow().clone();
    assert_eq!(
        launched[0][..5],
        [
            path.as_str(),
            "--operator-native-auth",
            "store",
            "--account",
            ACCOUNT
        ]
    );
    // Without an expected identity the live process must still be the
    // configured helper itself.
    let (fake, _) = signed_platform(&tmp);
    fake.described.borrow_mut().insert(
        "777".into(),
        VecDeque::from([describe(NATIVE_AUTH_IDENTIFIER, TEAM, &"e".repeat(40))]),
    );
    let helper = NativeAuthHelper::new(fake);
    assert_eq!(
        message(helper.store(&account, b"fresh-passphrase", None)),
        "the running native-auth helper does not match the signed CLI"
    );
}

#[test]
fn store_hands_the_passphrase_over_its_pipe() {
    let tmp = tempfile::tempdir().unwrap();
    let (fake, _) = signed_platform(&tmp);
    let helper = NativeAuthHelper::new(fake);
    helper
        .store(&account, b"fresh-passphrase", Some(&identity().opaque()))
        .unwrap();
    let mut received = Vec::new();
    helper
        .platform()
        .secret_reader
        .borrow_mut()
        .take()
        .unwrap()
        .read_to_end(&mut received)
        .unwrap();
    assert_eq!(received, b"fresh-passphrase");
}

#[test]
fn broker_touch_id_rejects_unbound_project_before_helper_lookup() {
    let tmp = tempfile::tempdir().unwrap();
    let (fake, _) = signed_platform(&tmp);
    let helper = NativeAuthHelper::new(fake);
    let unbound = || -> Result<String> {
        Err(Error::App(AppError::new(
            "operator_policy_binding_required",
            "the project unlock policy is not bound to an authenticated database",
        )))
    };
    assert_eq!(
        code(helper.broker_passphrase(&unbound, None)),
        "operator_policy_binding_required"
    );
    assert!(helper.platform().log().is_empty());
}

#[test]
fn caller_identity_binds_the_cli_to_its_live_signed_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let (fake, path) = signed_platform(&tmp);
    let helper = NativeAuthHelper::new(fake);
    assert_eq!(helper.caller_identity().unwrap(), identity().opaque());
    let requirement = identity().requirement();
    assert_eq!(
        helper.platform().log(),
        [
            format!("codesign -dv --verbose=4 {path}"),
            format!("codesign --verify --strict --verbose=2 -R={requirement} {path}"),
            "codesign -dv --verbose=4 4242".to_owned(),
            format!("codesign --verify --strict --verbose=2 -R={requirement} 4242"),
        ]
    );
    // A shell (or another build) as the parent is refused.
    let (fake, _) = signed_platform(&tmp);
    fake.described.borrow_mut().insert(
        "4242".into(),
        VecDeque::from([describe(NATIVE_AUTH_IDENTIFIER, TEAM, &"f".repeat(64))]),
    );
    let helper = NativeAuthHelper::new(fake);
    let error = helper.caller_identity().unwrap_err();
    let Error::App(error) = error else { panic!() };
    assert_eq!(error.code, "native_auth_helper_mismatch");
    assert_eq!(
        error.message,
        "the native-auth helper does not match the live signed CLI launcher"
    );
    assert!(!error.retryable);
    // An unsigned parent fails the requirement.
    let (mut fake, _) = signed_platform(&tmp);
    fake.verified.insert("4242".into(), false);
    let helper = NativeAuthHelper::new(fake);
    assert_eq!(
        message(helper.caller_identity()),
        "the native authentication helper failed dynamic signature validation"
    );
    assert!(helper.caller_gate());
}

#[test]
fn inspect_code_pins_identifier_team_and_cdhash() {
    let accepted = [
        signed(),
        describe(NATIVE_AUTH_IDENTIFIER, "ABCDEFGHIJ", &"0123456789abcdef".repeat(4)),
        // `codesign` prints uppercase hashes on some systems.
        describe(NATIVE_AUTH_IDENTIFIER, TEAM, &CDHASH.to_uppercase()),
        // Values are stripped; the last occurrence wins.
        format!("Identifier=other\nIdentifier= {NATIVE_AUTH_IDENTIFIER} \nTeamIdentifier={TEAM}\r\nCDHash={CDHASH}\n").into_bytes(),
    ];
    for stderr in accepted {
        let fake = Fake::macos_with_helper(Path::new("/unused"));
        fake.describes("target", stderr.clone());
        let helper = NativeAuthHelper::new(fake);
        assert!(
            helper.inspect_code("target", None).is_ok(),
            "{}",
            String::from_utf8_lossy(&stderr)
        );
    }
    let refused = [
        describe("at.bitcoinaustria.kassiber.rust-dev", TEAM, CDHASH),
        describe("kassiber_ui-0992cffe1a53d924", "not set", CDHASH),
        describe(NATIVE_AUTH_IDENTIFIER, "not set", CDHASH),
        describe(NATIVE_AUTH_IDENTIFIER, "6q4r2c3gjk", CDHASH),
        describe(NATIVE_AUTH_IDENTIFIER, "6Q4R2C3GJ", CDHASH),
        describe(NATIVE_AUTH_IDENTIFIER, "6Q4R2C3GJKX", CDHASH),
        describe(NATIVE_AUTH_IDENTIFIER, "6Q4R2C3GJ-", CDHASH),
        describe(NATIVE_AUTH_IDENTIFIER, "6Q4R2C3GJÄ", CDHASH),
        describe(NATIVE_AUTH_IDENTIFIER, TEAM, &CDHASH[..39]),
        describe(NATIVE_AUTH_IDENTIFIER, TEAM, &format!("{CDHASH}0")),
        describe(NATIVE_AUTH_IDENTIFIER, TEAM, &format!("{}g", &CDHASH[..39])),
        describe(NATIVE_AUTH_IDENTIFIER, TEAM, ""),
    ];
    for stderr in refused {
        let fake = Fake::macos_with_helper(Path::new("/unused"));
        fake.describes("target", stderr.clone());
        let helper = NativeAuthHelper::new(fake);
        assert_eq!(
            message(helper.inspect_code("target", None)),
            "the native authentication helper has an unexpected signing identity",
            "{}",
            String::from_utf8_lossy(&stderr)
        );
        // No verification is attempted for a refused identity.
        assert_eq!(helper.platform().log().len(), 1);
    }
    // No signature at all.
    let helper = NativeAuthHelper::new(Fake::macos_with_helper(Path::new("/unused")));
    assert_eq!(
        message(helper.inspect_code("target", None)),
        "the native authentication helper has no valid code signature"
    );
}

/// The opaque identity and requirement, as Python's `_CodeIdentity`
/// computes them.
#[test]
fn identity_vectors_match_python() {
    let identity = identity();
    assert_eq!(
        identity.requirement(),
        "anchor apple generic and identifier \"at.bitcoinaustria.kassiber\" and certificate leaf[subject.OU] = \"6Q4R2C3GJK\" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
    );
    // hashlib.sha256(b"kassiber-native-auth-signature-v1:at.bitcoinaustria.kassiber:6Q4R2C3GJK:8b23350488a1ecd22e6cbd549dbe926c7214f397").hexdigest()
    assert_eq!(identity.opaque(), PYTHON_OPAQUE);
    // hashlib.sha256(b"kassiber-operator-touch-id-v1:" + b"a" * 64 + b":initial").hexdigest()
    assert_eq!(touch_id_account(&"a".repeat(64), "initial"), PYTHON_ACCOUNT);
}

#[test]
fn configured_helper_must_be_an_executable_file() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let helper = packaged_helper(&tmp);
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o600)).unwrap();
    for candidate in [
        helper.clone(),
        tmp.path().join("missing"),
        tmp.path().to_owned(),
    ] {
        let native = NativeAuthHelper::new(Fake::macos_with_helper(&candidate));
        assert_eq!(
            message(native.configured_helper_path()),
            "the native authentication helper is unavailable"
        );
    }
    // A link to the packaged helper resolves to it.
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&helper, &link).unwrap();
    let native = NativeAuthHelper::new(Fake::macos_with_helper(&link));
    assert_eq!(native.configured_helper_path().unwrap(), helper);
}

const PYTHON_OPAQUE: &str = "3e2672ed301089ac4cdfb32d3d2464b52f4cad3182a40f60b0a5132be4c582ef";
const PYTHON_ACCOUNT: &str = "6cd97dc16b6cc718deb713a1bfea994a060ee690577a24fad01d6b6f302acbb7";
