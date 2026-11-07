//! The signed macOS Touch ID helper bridge (`kassiber/operator/native_auth.py`).
//!
//! Neither the CLI nor the broker calls LocalAuthentication. They start the
//! signed desktop executable `kassiber-ui --operator-native-auth <action>`;
//! the system prompt appears when that helper reads its biometric keychain
//! item. Every launch is held behind a pipe gate: the helper writes `R`, the
//! launcher checks the live process's code signature, and only then writes
//! `G`. Secrets cross only through inherited pipes, at most 16 KiB.
//!
//! Code identity is read by running `/usr/bin/codesign` on a path or a pid,
//! as Python does. Off macOS, or without `KASSIBER_NATIVE_AUTH_HELPER`,
//! every action answers `native_auth_unavailable` without starting anything.
//!
//! The platform (environment, parent pid, `codesign`, the helper launch) is
//! one seam, [`Platform`], as the Python tests patch `sys.platform`,
//! `os.environ`, `_inspect_code` and `subprocess.Popen`.

use crate::error::{app, Error, Result};
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

/// The managed setting whose rotation retires every enrolled credential.
pub const GENERATION_SETTING: &str = "operator_native_auth_generation";
/// The desktop app's signing identifier.
pub const NATIVE_AUTH_IDENTIFIER: &str = "at.bitcoinaustria.kassiber";
/// The desktop executable inside `<bundle>.app/Contents/MacOS`.
pub const MACOS_APP_EXECUTABLE_NAME: &str = "kassiber-ui";
/// The environment variable the signed CLI launchers set.
pub const HELPER_ENV: &str = "KASSIBER_NATIVE_AUTH_HELPER";
const CODESIGN: &str = "/usr/bin/codesign";
const SECRET_LIMIT: usize = 16 * 1024;
/// How long a helper has to reach its launch gate.
pub const GATE_TIMEOUT: Duration = Duration::from_secs(5);

/// A signing identity as `codesign -dv` reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeIdentity {
    pub identifier: String,
    pub team: String,
    pub cdhash: String,
}

impl CodeIdentity {
    /// The public-safe identity brokers and clients compare.
    pub fn opaque(&self) -> String {
        hex(&Sha256::digest(
            format!(
                "kassiber-native-auth-signature-v1:{}:{}:{}",
                self.identifier, self.team, self.cdhash
            )
            .as_bytes(),
        ))
    }

    /// The designated requirement a live process must satisfy: this
    /// identifier, this team, a Developer ID Application leaf.
    pub fn requirement(&self) -> String {
        format!(
            "anchor apple generic and identifier \"{}\" and certificate leaf[subject.OU] = \"{}\" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists",
            self.identifier, self.team
        )
    }
}

/// A launched helper.
pub trait HelperChild {
    fn id(&self) -> u32;
    /// `process.poll()`: the exit code once it exited.
    fn poll(&mut self) -> io::Result<Option<i32>>;
    /// `process.stderr.read()`.
    fn read_stderr(&mut self) -> io::Result<Vec<u8>>;
    /// `process.wait()`; a signal is its negative number.
    fn wait(&mut self) -> io::Result<i32>;
    /// `process.kill()`.
    fn kill(&mut self) -> io::Result<()>;
}

/// What this module asks of the operating system.
pub trait Platform {
    /// `sys.platform == "darwin"`.
    fn macos(&self) -> bool;
    /// `os.environ.get("KASSIBER_NATIVE_AUTH_HELPER")`.
    fn configured_helper(&self) -> Option<OsString>;
    /// `os.getppid()`.
    fn parent_pid(&self) -> u32;
    /// `subprocess.run(["/usr/bin/codesign", *args], stdin=DEVNULL,
    /// stdout=DEVNULL, stderr=PIPE)`: whether it succeeded, and its stderr.
    fn codesign(&self, args: &[&OsStr]) -> io::Result<(bool, Vec<u8>)>;
    /// `subprocess.Popen(argv, stdin=DEVNULL, stdout=DEVNULL, stderr=PIPE,
    /// pass_fds=inherit)`.
    #[cfg(unix)]
    fn spawn(
        &self,
        argv: &[OsString],
        inherit: &[std::os::fd::RawFd],
    ) -> io::Result<Box<dyn HelperChild>>;
}

/// The real process: its environment, `/usr/bin/codesign` and
/// `kassiber-ipc` launches.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemPlatform;

#[cfg(unix)]
struct SystemChild(kassiber_ipc::process::Child);

#[cfg(unix)]
impl HelperChild for SystemChild {
    fn id(&self) -> u32 {
        self.0.id()
    }
    fn poll(&mut self) -> io::Result<Option<i32>> {
        Ok(self.0.try_wait()?.map(exit_code))
    }
    fn read_stderr(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        if let Some(stderr) = self.0.stderr.as_mut() {
            stderr.read_to_end(&mut bytes)?;
        }
        Ok(bytes)
    }
    fn wait(&mut self) -> io::Result<i32> {
        self.0.wait().map(exit_code)
    }
    fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }
}

/// `Popen.returncode`: the exit code, or the negative signal number.
#[cfg(unix)]
fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .or_else(|| status.signal().map(|signal| -signal))
        .unwrap_or(-1)
}

impl Platform for SystemPlatform {
    fn macos(&self) -> bool {
        cfg!(target_os = "macos")
    }
    fn configured_helper(&self) -> Option<OsString> {
        std::env::var_os(HELPER_ENV)
    }
    fn parent_pid(&self) -> u32 {
        #[cfg(unix)]
        {
            std::os::unix::process::parent_id()
        }
        #[cfg(not(unix))]
        {
            0
        }
    }
    fn codesign(&self, args: &[&OsStr]) -> io::Result<(bool, Vec<u8>)> {
        use kassiber_ipc::process::{ChildCommand, Stdio};
        let mut command = ChildCommand::new(CODESIGN);
        command
            .args(args)
            .stdin(Stdio::Null)
            .stdout(Stdio::Null)
            .stderr(Stdio::Pipe);
        let mut child = command.spawn()?;
        let mut stderr = Vec::new();
        if let Some(stream) = child.stderr.as_mut() {
            stream.read_to_end(&mut stderr)?;
        }
        Ok((child.wait()?.success(), stderr))
    }
    #[cfg(unix)]
    fn spawn(
        &self,
        argv: &[OsString],
        inherit: &[std::os::fd::RawFd],
    ) -> io::Result<Box<dyn HelperChild>> {
        use kassiber_ipc::process::{ChildCommand, Stdio};
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
        let mut command = ChildCommand::new(program);
        command
            .args(args)
            .stdin(Stdio::Null)
            .stdout(Stdio::Null)
            .stderr(Stdio::Pipe);
        for fd in inherit {
            command.inherit(*fd);
        }
        Ok(Box::new(SystemChild(command.spawn()?)))
    }
}

/// The bridge over a [`Platform`]. Every action that names a book takes its
/// account as a closure, so the account is derived exactly where Python
/// derives it relative to the helper checks.
pub struct NativeAuthHelper<P: Platform = SystemPlatform> {
    platform: P,
}

impl Default for NativeAuthHelper<SystemPlatform> {
    fn default() -> Self {
        NativeAuthHelper {
            platform: SystemPlatform,
        }
    }
}

type Account<'a> = &'a dyn Fn() -> Result<String>;

/// A spawned helper that passed its gate, killed if dropped while running.
#[cfg(unix)]
struct Launched(Option<Box<dyn HelperChild>>);

#[cfg(unix)]
impl Launched {
    fn child(&mut self) -> &mut dyn HelperChild {
        self.0.as_deref_mut().expect("launched helper")
    }
}

#[cfg(unix)]
impl Drop for Launched {
    /// `finally: if process.poll() is None: process.kill(); process.wait()`.
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            if matches!(child.poll(), Ok(None)) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl<P: Platform> NativeAuthHelper<P> {
    pub fn new(platform: P) -> Self {
        NativeAuthHelper { platform }
    }

    pub fn platform(&self) -> &P {
        &self.platform
    }

    /// `native_auth_helper_identity()`: the signed configured helper's
    /// public-safe identity.
    pub fn helper_identity(&self) -> Result<String> {
        Ok(self
            .signed_helper_code(&self.configured_helper_path()?)?
            .opaque())
    }

    /// `native_auth_caller_identity()`: bind a CLI invocation to its live
    /// signed macOS launcher, its parent process.
    pub fn caller_identity(&self) -> Result<String> {
        let helper = self.signed_helper_code(&self.configured_helper_path()?)?;
        let parent = self.inspect_code(
            &self.platform.parent_pid().to_string(),
            Some(&helper.requirement()),
        )?;
        if parent != helper {
            return Err(Error::App(app(
                "native_auth_helper_mismatch",
                "the native-auth helper does not match the live signed CLI launcher",
            )));
        }
        Ok(helper.opaque())
    }

    /// Whether this CLI process should hand the broker its identity:
    /// `ensure_native_auth_running` does only on macOS with a helper set.
    pub fn caller_gate(&self) -> bool {
        self.platform.macos()
            && self
                .platform
                .configured_helper()
                .is_some_and(|value| !value.is_empty())
    }

    /// `validate_native_auth_helper_identity(expected)`.
    pub fn validate(&self, expected: &str) -> Result<()> {
        self.helper_path(Some(expected)).map(|_| ())
    }

    /// `native_auth_runtime_available()`.
    pub fn runtime_available(&self) -> bool {
        self.helper_path(None).is_ok()
    }

    /// `broker_touch_id_passphrase(data_root, expected_helper_identity=...)`:
    /// the signed helper writes the passphrase to a broker-created pipe.
    #[cfg(unix)]
    pub fn broker_passphrase(
        &self,
        account: Account<'_>,
        expected: Option<&str>,
    ) -> Result<Zeroizing<Vec<u8>>> {
        use std::os::fd::AsRawFd;
        let account = account()?;
        let (mut reader, writer) = io::pipe().map_err(Error::Io)?;
        let mut value = Zeroizing::new(Vec::new());
        let helper = self.helper_path(expected)?;
        let mut launched = self.spawn_validated_helper(
            vec![
                helper.into_os_string(),
                "--operator-native-auth".into(),
                "broker-get".into(),
                "--account".into(),
                account.into(),
                "--output-fd".into(),
                writer.as_raw_fd().to_string().into(),
            ],
            &[writer.as_raw_fd()],
            expected,
        )?;
        drop(writer);
        read_limited(&mut reader, &mut value)?;
        let stderr = launched.child().read_stderr().map_err(Error::Io)?;
        let code = launched.child().wait().map_err(Error::Io)?;
        if code == 4 {
            return Err(Error::App(
                app(
                    "operator_native_auth_not_configured",
                    "no Touch ID operator credential is enrolled for this project",
                )
                .with_hint(
                    "Run `kassiber operator touch-id enroll` from the signed macOS app CLI.",
                ),
            ));
        }
        if code != 0 {
            return Err(native_error(&stderr));
        }
        if value.is_empty() {
            return Err(Error::App(app(
                "native_auth_failed",
                "the Touch ID helper returned an empty credential",
            )));
        }
        Ok(value)
    }

    /// `touch_id_store(data_root, passphrase, expected_helper_identity=...)`.
    #[cfg(unix)]
    pub fn store(&self, account: Account<'_>, secret: &[u8], expected: Option<&str>) -> Result<()> {
        use std::io::Write;
        use std::os::fd::AsRawFd;
        let helper = self.helper_path(expected)?;
        let (reader, mut writer) = io::pipe().map_err(Error::Io)?;
        let argv = vec![
            helper.into_os_string(),
            "--operator-native-auth".into(),
            "store".into(),
            "--account".into(),
            account()?.into(),
            "--secret-fd".into(),
            reader.as_raw_fd().to_string().into(),
        ];
        let mut launched = self.spawn_validated_helper(argv, &[reader.as_raw_fd()], expected)?;
        drop(reader);
        writer.write_all(secret).map_err(Error::Io)?;
        drop(writer);
        let stderr = launched.child().read_stderr().map_err(Error::Io)?;
        if launched.child().wait().map_err(Error::Io)? != 0 {
            return Err(native_error(&stderr));
        }
        Ok(())
    }

    /// `touch_id_delete(data_root, expected_helper_identity=...)`.
    #[cfg(unix)]
    pub fn delete(&self, account: Account<'_>, expected: Option<&str>) -> Result<()> {
        let (code, stderr) = self.run_no_secret("delete", account, expected)?;
        if code != 0 {
            return Err(native_error(&stderr));
        }
        Ok(())
    }

    /// `touch_id_status(data_root)`: `available`, `configured` and, when
    /// unavailable, the refusal's code as `reason`.
    #[cfg(unix)]
    pub fn status(&self, account: Account<'_>) -> Result<Value> {
        let (code, _) = match self.run_no_secret("status", account, None) {
            Ok(result) => result,
            Err(Error::App(error)) => {
                return Ok(dict([
                    ("available", Value::Bool(false)),
                    ("configured", Value::Bool(false)),
                    ("reason", kassiber_envelope::s(error.code.as_str())),
                ]))
            }
            Err(error) => return Err(error),
        };
        Ok(match code {
            0 => dict([
                ("available", Value::Bool(true)),
                ("configured", Value::Bool(true)),
            ]),
            4 => dict([
                ("available", Value::Bool(true)),
                ("configured", Value::Bool(false)),
            ]),
            _ => dict([
                ("available", Value::Bool(false)),
                ("configured", Value::Bool(false)),
                ("reason", kassiber_envelope::s("native_auth_failed")),
            ]),
        })
    }

    /// `_run_no_secret(action, data_root, expected_helper_identity=...)`:
    /// the exit code and stderr.
    #[cfg(unix)]
    fn run_no_secret(
        &self,
        action: &str,
        account: Account<'_>,
        expected: Option<&str>,
    ) -> Result<(i32, Vec<u8>)> {
        let helper = self.helper_path(expected)?;
        let argv = vec![
            helper.into_os_string(),
            "--operator-native-auth".into(),
            action.into(),
            "--account".into(),
            account()?.into(),
        ];
        let mut launched = self.spawn_validated_helper(argv, &[], expected)?;
        let stderr = launched.child().read_stderr().map_err(Error::Io)?;
        let code = launched.child().wait().map_err(Error::Io)?;
        Ok((code, stderr))
    }

    /// `_helper_path(expected_identity)`: the configured signed helper,
    /// which must still be the one the caller's identity names.
    pub fn helper_path(&self, expected: Option<&str>) -> Result<PathBuf> {
        let helper = self.configured_helper_path()?;
        let actual = self.signed_helper_code(&helper)?.opaque();
        if let Some(expected) = expected {
            if !constant_time_eq(actual.as_bytes(), expected.as_bytes()) {
                return Err(Error::App(
                    app(
                        "native_auth_helper_mismatch",
                        "the broker native-auth helper does not match this signed CLI",
                    )
                    .with_hint(
                        "Retry from the signed macOS app CLI after stopping idle broker work.",
                    )
                    .retryable(true),
                ));
            }
        }
        Ok(helper)
    }

    /// `_configured_helper_path()`: the executable the launcher named.
    pub fn configured_helper_path(&self) -> Result<PathBuf> {
        let configured = self
            .platform
            .configured_helper()
            .filter(|value| !value.is_empty());
        let Some(configured) = configured.filter(|_| self.platform.macos()) else {
            return Err(Error::App(
                app(
                    "native_auth_unavailable",
                    "Touch ID operator authentication is unavailable in this build",
                )
                .with_hint("Use password authentication or the signed macOS desktop CLI launcher."),
            ));
        };
        let expanded = kassiber_paths::Context::from_env()
            .and_then(|context| context.expand_user(Path::new(&configured)))
            .map_err(crate::error::from_paths)?;
        let helper = kassiber_paths::canonicalize_allow_missing(&expanded)
            .map_err(crate::error::from_paths)?;
        if !is_executable_file(&helper) {
            return Err(Error::App(app(
                "native_auth_unavailable",
                "the native authentication helper is unavailable",
            )));
        }
        Ok(helper)
    }

    /// `_signed_helper_code(helper)`: only `<bundle>.app/Contents/MacOS/
    /// kassiber-ui`, then its code identity.
    pub fn signed_helper_code(&self, helper: &Path) -> Result<CodeIdentity> {
        let bundle = helper.ancestors().skip(1).find(|ancestor| {
            ancestor
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        });
        let expected = bundle.map(|bundle| {
            bundle
                .join("Contents/MacOS")
                .join(MACOS_APP_EXECUTABLE_NAME)
        });
        if expected.as_deref() != Some(helper) {
            return Err(Error::App(app(
                "native_auth_unavailable",
                "the native authentication helper is outside the Kassiber app bundle",
            )));
        }
        self.inspect_code(&helper.to_string_lossy(), None)
    }

    /// `_inspect_code(target, requirement=...)`: the identity `codesign`
    /// reports for a path or a pid, then a strict verification against the
    /// requirement (its own when none is given).
    pub fn inspect_code(&self, target: &str, requirement: Option<&str>) -> Result<CodeIdentity> {
        let unavailable = |message: &str| Error::App(app("native_auth_unavailable", message));
        let (described, stderr) = self
            .platform
            .codesign(&[
                OsStr::new("-dv"),
                OsStr::new("--verbose=4"),
                OsStr::new(target),
            ])
            .map_err(Error::Io)?;
        if !described {
            return Err(unavailable(
                "the native authentication helper has no valid code signature",
            ));
        }
        let identity = codesign_identity(&stderr);
        if !valid_identity(&identity) {
            return Err(unavailable(
                "the native authentication helper has an unexpected signing identity",
            ));
        }
        let required = requirement.map_or_else(|| identity.requirement(), str::to_owned);
        let (verified, _) = self
            .platform
            .codesign(&[
                OsStr::new("--verify"),
                OsStr::new("--strict"),
                OsStr::new("--verbose=2"),
                OsStr::new(&format!("-R={required}")),
                OsStr::new(target),
            ])
            .map_err(Error::Io)?;
        if !verified {
            return Err(unavailable(
                "the native authentication helper failed dynamic signature validation",
            ));
        }
        Ok(identity)
    }

    /// `_validate_spawned_helper(process, expected_identity)`: the running
    /// helper is the signed configured one, by its live pid.
    #[cfg(unix)]
    fn validate_spawned_helper(&self, pid: u32, expected: Option<&str>) -> Result<()> {
        let helper = self.signed_helper_code(&self.helper_path(expected)?)?;
        let running = self.inspect_code(&pid.to_string(), Some(&helper.requirement()))?;
        if running != helper
            || expected.is_some_and(|expected| {
                !constant_time_eq(running.opaque().as_bytes(), expected.as_bytes())
            })
        {
            return Err(Error::App(app(
                "native_auth_helper_mismatch",
                "the running native-auth helper does not match the signed CLI",
            )));
        }
        Ok(())
    }

    /// `_spawn_validated_helper(command, inherited_fds=..., expected_identity=...)`.
    #[cfg(unix)]
    fn spawn_validated_helper(
        &self,
        argv: Vec<OsString>,
        inherit: &[std::os::fd::RawFd],
        expected: Option<&str>,
    ) -> Result<Launched> {
        self.spawn_gated(argv, inherit, |child| {
            self.validate_spawned_helper(child.id(), expected)
        })
    }

    /// The launch gate with its validation as a parameter
    /// (`test_launch_gate_keeps_helper_alive_through_pid_validation`).
    #[cfg(unix)]
    fn spawn_gated(
        &self,
        mut argv: Vec<OsString>,
        inherit: &[std::os::fd::RawFd],
        validate: impl FnOnce(&mut dyn HelperChild) -> Result<()>,
    ) -> Result<Launched> {
        use std::io::Write;
        use std::os::fd::AsRawFd;
        let (ready_read, ready_write) = io::pipe().map_err(Error::Io)?;
        let (go_read, mut go_write) = io::pipe().map_err(Error::Io)?;
        argv.extend([
            "--ready-fd".into(),
            ready_write.as_raw_fd().to_string().into(),
            "--go-fd".into(),
            go_read.as_raw_fd().to_string().into(),
        ]);
        let mut fds = inherit.to_vec();
        fds.extend([ready_write.as_raw_fd(), go_read.as_raw_fd()]);
        let mut launched = Launched(Some(self.platform.spawn(&argv, &fds).map_err(Error::Io)?));
        drop(ready_write);
        drop(go_read);
        if !ready_within(ready_read, GATE_TIMEOUT) {
            return Err(Error::App(app(
                "native_auth_failed",
                "the native-auth helper did not reach its signed launch gate",
            )));
        }
        validate(launched.child())?;
        go_write.write_all(b"G").map_err(Error::Io)?;
        drop(go_write);
        Ok(launched)
    }
}

/// Windows has no helper: every action refuses where Python's would, after
/// the same account derivation, without starting anything.
#[cfg(not(unix))]
impl<P: Platform> NativeAuthHelper<P> {
    pub fn broker_passphrase(
        &self,
        account: Account<'_>,
        expected: Option<&str>,
    ) -> Result<Zeroizing<Vec<u8>>> {
        account()?;
        self.helper_path(expected)?;
        Err(Error::App(unavailable_error()))
    }
    pub fn store(&self, _: Account<'_>, _: &[u8], expected: Option<&str>) -> Result<()> {
        self.helper_path(expected)?;
        Err(Error::App(unavailable_error()))
    }
    pub fn delete(&self, _: Account<'_>, expected: Option<&str>) -> Result<()> {
        self.helper_path(expected)?;
        Err(Error::App(unavailable_error()))
    }
    pub fn status(&self, _: Account<'_>) -> Result<Value> {
        let code = match self.helper_path(None) {
            Err(Error::App(error)) => error.code,
            Err(error) => return Err(error),
            Ok(_) => "native_auth_unavailable".to_owned(),
        };
        Ok(dict([
            ("available", Value::Bool(false)),
            ("configured", Value::Bool(false)),
            ("reason", kassiber_envelope::s(code.as_str())),
        ]))
    }
}

/// `select([ready_read], [], [], timeout)` and `os.read(ready_read, 1) ==
/// b"R"`. The reader thread ends when the helper writes or exits; a killed
/// helper closes its end.
#[cfg(unix)]
fn ready_within(mut ready: io::PipeReader, timeout: Duration) -> bool {
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("native-auth-gate".into())
        .spawn(move || {
            let mut byte = [0u8; 1];
            let ok = matches!(ready.read(&mut byte), Ok(1)) && byte == *b"R";
            let _ = sender.send(ok);
        });
    spawned.is_ok() && receiver.recv_timeout(timeout).unwrap_or(false)
}

/// `_read_fd_limited`: everything the helper wrote, at most 16 KiB.
#[cfg(unix)]
fn read_limited(reader: &mut impl Read, value: &mut Zeroizing<Vec<u8>>) -> Result<()> {
    let mut chunk = Zeroizing::new([0u8; 4096]);
    loop {
        let want = 4096.min(SECRET_LIMIT + 1 - value.len());
        let read = match reader.read(&mut chunk[..want]) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Error::Io(error)),
        };
        if read == 0 {
            return Ok(());
        }
        value.extend_from_slice(&chunk[..read]);
        if value.len() > SECRET_LIMIT {
            value.iter_mut().for_each(|byte| *byte = 0);
            value.clear();
            return Err(Error::App(app(
                "native_auth_failed",
                "native authentication secret exceeds 16 KiB",
            )));
        }
    }
}

/// `_native_error(stderr)`: the helper's own message, or a generic one.
#[cfg(unix)]
fn native_error(stderr: &[u8]) -> Error {
    let text = String::from_utf8_lossy(stderr);
    let message = kassiber_pytext::strip(&text);
    Error::App(app(
        "native_auth_failed",
        if message.is_empty() {
            "Touch ID authentication failed"
        } else {
            message
        },
    ))
}

/// `_codesign_fields`: `Identifier=`, `TeamIdentifier=` and `CDHash=`, the
/// last occurrence of each, values stripped; the cdhash lowercased.
fn codesign_identity(stderr: &[u8]) -> CodeIdentity {
    let text = String::from_utf8_lossy(stderr);
    let mut identity = CodeIdentity {
        identifier: String::new(),
        team: String::new(),
        cdhash: String::new(),
    };
    for line in splitlines(&text) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = kassiber_pytext::strip(value).to_owned();
        match key {
            "Identifier" => identity.identifier = value,
            "TeamIdentifier" => identity.team = value,
            "CDHash" => identity.cdhash = kassiber_pytext::lower(&value),
            _ => {}
        }
    }
    identity
}

/// The identity checks of `_inspect_code`.
fn valid_identity(identity: &CodeIdentity) -> bool {
    identity.identifier == NATIVE_AUTH_IDENTIFIER
        && identity.team.chars().count() == 10
        && identity
            .team
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && matches!(identity.cdhash.chars().count(), 40 | 64)
        && identity
            .cdhash
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// `helper.is_file() and os.access(helper, os.X_OK)`.
fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        nix::unistd::access(path, nix::unistd::AccessFlags::X_OK).is_ok()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// `hmac.compare_digest` for two strings.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

/// `str.splitlines()`.
fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let breaks = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !breaks {
            continue;
        }
        lines.push(&text[start..index]);
        let mut end = index + c.len_utf8();
        if c == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            chars.next();
            end += 1;
        }
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `operator_touch_id_account(...)`'s hash over the project identity and
/// the native-auth generation (`"initial"` when unset, `"invalid"` when not
/// a string); the storage crate reads both.
pub fn touch_id_account(project_identity: &str, generation: &str) -> String {
    hex(&Sha256::digest(
        format!("kassiber-operator-touch-id-v1:{project_identity}:{generation}").as_bytes(),
    ))
}

/// The refusal [`crate::server::NativeAuthUnavailable`] answers with.
pub fn unavailable_error() -> AppError {
    app(
        "native_auth_unavailable",
        "Touch ID operator authentication is unavailable in this build",
    )
    .with_hint("Use password authentication or the signed macOS desktop CLI launcher.")
}

#[cfg(test)]
#[path = "native_auth_tests.rs"]
mod tests;
