use crate::{
    remaining, Endpoint, Error, Gate, Observer, Operation, Request, Route, Subsystem,
    TestEgressBlocked, TlsTrust,
};
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, PartialEq, Eq)]
pub struct ClnConfig {
    pub program: PathBuf,
    pub lightning_dir: Option<String>,
    pub rpc_file: Option<String>,
    pub network: Option<String>,
    pub commando: Option<(String, String)>,
}

impl fmt::Debug for ClnConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClnConfig(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClnMethod {
    GetInfo,
    ListFunds,
    ListPeerChannels,
    ListForwards,
    ListPays,
    ListInvoices,
    ListIncome,
    ListBalances,
    ListAccountEvents,
}

impl ClnMethod {
    pub fn name(self) -> &'static str {
        match self {
            Self::GetInfo => "getinfo",
            Self::ListFunds => "listfunds",
            Self::ListPeerChannels => "listpeerchannels",
            Self::ListForwards => "listforwards",
            Self::ListPays => "listpays",
            Self::ListInvoices => "listinvoices",
            Self::ListIncome => "bkpr-listincome",
            Self::ListBalances => "bkpr-listbalances",
            Self::ListAccountEvents => "bkpr-listaccountevents",
        }
    }
}

impl Request {
    pub fn cln(config: ClnConfig, binding: [u8; 32]) -> Self {
        let mut request = Self::new(
            Subsystem::Sync,
            Endpoint::new("lightning-cli", 1).expect("fixed process label"),
            TlsTrust::Plaintext,
        )
        .with_action_binding(binding);
        request.cln = Some(config);
        request
    }
}

impl Gate {
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_cln_fixture(mut self, program: PathBuf) -> Self {
        self.cln_fixture = Some(program);
        self
    }
}

#[derive(Debug)]
pub enum ClnProcessError {
    Gate(Error),
    Missing,
    Failed,
    Timeout,
    OutputLimit,
    Io,
}

impl From<Error> for ClnProcessError {
    fn from(value: Error) -> Self {
        Self::Gate(value)
    }
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Route {
    pub fn cln(
        self,
        method: ClnMethod,
        deadline: Instant,
        limit: usize,
    ) -> Result<Vec<u8>, ClnProcessError> {
        self.check()?;
        remaining(deadline).map_err(|_| ClnProcessError::Timeout)?;
        let config = self.request.cln.as_ref().ok_or(Error::InvalidEndpoint)?;
        if self.gate.observer != Observer::Disabled {
            self.gate
                .attempts
                .record(&self.request, &self.request.endpoint, Operation::Process, 0);
            if self.gate.observer == Observer::Strict
                || self.gate.cln_fixture.as_ref() != Some(&config.program)
            {
                std::panic::panic_any(TestEgressBlocked);
            }
        }
        let mut command = Command::new(&config.program);
        command.args(["--json", "--raw"]);
        if let Some((peer, rune)) = &config.commando {
            command.arg(format!("--commando-peer={peer}"));
            command.arg(format!("--commando-rune={rune}"));
        }
        for (flag, value) in [
            ("lightning-dir", &config.lightning_dir),
            ("rpc-file", &config.rpc_file),
            ("network", &config.network),
        ] {
            if let Some(value) = value {
                command.arg(format!("--{flag}={value}"));
            }
        }
        command
            .arg(method.name())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        self.check()?;
        self.gate
            .ledger
            .record(&self.request, &self.request.endpoint, Operation::Process, 0);
        let mut child = Running(command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ClnProcessError::Missing
            } else {
                ClnProcessError::Io
            }
        })?);
        let stdout = child.0.stdout.take().ok_or(ClnProcessError::Io)?;
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let mut bytes = Vec::new();
                let result = stdout
                    .take(limit.saturating_add(1) as u64)
                    .read_to_end(&mut bytes)
                    .map(|_| bytes)
                    .map_err(|_| ClnProcessError::Io);
                let _ = tx.send(result);
            });
            let result = (|| {
                let mut output = None;
                loop {
                    self.check()?;
                    let wait = remaining(deadline)
                        .map_err(|_| ClnProcessError::Timeout)?
                        .min(Duration::from_millis(5));
                    if output.is_none() {
                        match rx.recv_timeout(wait) {
                            Ok(result) => {
                                let bytes = result?;
                                if bytes.len() > limit {
                                    return Err(ClnProcessError::OutputLimit);
                                }
                                output = Some(bytes);
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                return Err(ClnProcessError::Io)
                            }
                        }
                    } else {
                        std::thread::sleep(wait);
                    }
                    if let Some(status) = child.0.try_wait().map_err(|_| ClnProcessError::Io)? {
                        if !status.success() {
                            return Err(ClnProcessError::Failed);
                        }
                        if let Some(bytes) = output.take() {
                            self.check()?;
                            return Ok(bytes);
                        }
                    }
                }
            })();
            let _ = child.0.kill();
            let _ = child.0.wait();
            result
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{Consent, Ledger, Offline};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    };

    fn fake(body: &str) -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fake-lightning-cli");
        std::fs::write(&path, format!("#!/usr/bin/python3\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (root, path)
    }
    fn config(program: PathBuf) -> ClnConfig {
        ClnConfig {
            program,
            lightning_dir: Some("private-dir".into()),
            rpc_file: Some("private-rpc".into()),
            network: Some("regtest".into()),
            commando: Some(("private-peer".into(), "private-rune".into())),
        }
    }
    fn gate(program: PathBuf) -> Gate {
        let mut gate = Gate::new(|| Offline::Online, Ledger::default()).with_cln_fixture(program);
        gate.observer = Observer::LoopbackAllowed;
        gate
    }
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }
    fn run(
        gate: &Gate,
        request: Request,
        deadline: Instant,
        limit: usize,
    ) -> Result<Vec<u8>, ClnProcessError> {
        let consent = Consent::explicit_trigger(&request);
        gate.authorize(request, Some(consent))
            .unwrap()
            .cln(ClnMethod::GetInfo, deadline, limit)
    }

    #[test]
    fn cln_process_pins_read_only_argv_and_redacted_ram_metadata() {
        let (_root, program) = fake("import json,sys; print(json.dumps(sys.argv[1:]))");
        let gate = gate(program.clone());
        let request = Request::cln(config(program), [7; 32]);
        let bytes = run(&gate, request.clone(), deadline(), 4096).unwrap();
        let args: Vec<String> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            args,
            [
                "--json",
                "--raw",
                "--commando-peer=private-peer",
                "--commando-rune=private-rune",
                "--lightning-dir=private-dir",
                "--rpc-file=private-rpc",
                "--network=regtest",
                "getinfo"
            ]
        );
        let records = gate.ledger().snapshot(0, 2000).records;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].operation, Operation::Process);
        assert_eq!(records[0].scheme, "cln");
        assert_eq!(records[0].host, "lightning-cli");
        let rendered = format!("{records:?} {request:?}");
        for secret in ["private-dir", "private-rpc", "private-peer", "private-rune"] {
            assert!(!rendered.contains(secret));
        }
        assert!(matches!(
            gate.authorize(request.clone(), Some(Consent::explicit_trigger(&request)))
                .unwrap()
                .connect(Duration::from_secs(1)),
            Err(Error::InvalidEndpoint)
        ));
        assert_eq!(
            [
                ClnMethod::GetInfo,
                ClnMethod::ListFunds,
                ClnMethod::ListPeerChannels,
                ClnMethod::ListForwards,
                ClnMethod::ListPays,
                ClnMethod::ListInvoices,
                ClnMethod::ListIncome,
                ClnMethod::ListBalances,
                ClnMethod::ListAccountEvents
            ]
            .map(ClnMethod::name),
            [
                "getinfo",
                "listfunds",
                "listpeerchannels",
                "listforwards",
                "listpays",
                "listinvoices",
                "bkpr-listincome",
                "bkpr-listbalances",
                "bkpr-listaccountevents"
            ]
        );
    }

    #[test]
    fn cln_process_timeout_revocation_offline_exit_and_output_limit_reap_child() {
        for (body, expected) in [
            ("import time; time.sleep(10)", "timeout"),
            ("import sys; sys.exit(9)", "failed"),
            ("import sys; sys.stdout.write('x'*100000)", "limit"),
        ] {
            let (_root, program) = fake(body);
            let gate = gate(program.clone());
            let request = Request::cln(config(program), [7; 32]);
            // A freshly written script's first exec can take ~100 ms on macOS;
            // only the timeout case may be that short.
            let budget = if expected == "timeout" {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(2)
            };
            let start = Instant::now();
            let result = run(&gate, request, start + budget, 32);
            assert!(
                match expected {
                    "timeout" => matches!(result, Err(ClnProcessError::Timeout)),
                    "failed" => matches!(result, Err(ClnProcessError::Failed)),
                    _ => matches!(result, Err(ClnProcessError::OutputLimit)),
                },
                "{result:?}"
            );
            assert!(start.elapsed() < budget + Duration::from_secs(2));
        }
        for revoke in [true, false] {
            let (_root, program) = fake("import time; time.sleep(10)");
            let state = Arc::new(AtomicU8::new(0));
            let observed = state.clone();
            let mut gate = Gate::new(
                move || {
                    if observed.load(Ordering::SeqCst) == 0 {
                        Offline::Online
                    } else {
                        Offline::Offline
                    }
                },
                Ledger::default(),
            )
            .with_cln_fixture(program.clone());
            gate.observer = Observer::LoopbackAllowed;
            let request = Request::cln(config(program), [7; 32]);
            let proof = Consent::explicit_trigger(&request);
            let route = gate.authorize(request, Some(proof.clone())).unwrap();
            std::thread::scope(|scope| {
                scope.spawn(move || {
                    std::thread::sleep(Duration::from_millis(30));
                    if revoke {
                        proof.revoke();
                    } else {
                        state.store(1, Ordering::SeqCst);
                    }
                });
                let start = Instant::now();
                let result = route.cln(ClnMethod::GetInfo, deadline(), 32);
                assert!(if revoke {
                    matches!(result, Err(ClnProcessError::Gate(Error::ConsentRevoked)))
                } else {
                    matches!(result, Err(ClnProcessError::Gate(Error::Offline)))
                });
                assert!(start.elapsed() < Duration::from_secs(1));
            });
        }
    }

    #[test]
    fn cln_process_test_observer_blocks_unapproved_children_before_spawn() {
        let (_root, program) = fake("raise SystemExit('guard did not stop child')");
        for strict in [false, true] {
            let mut gate = gate(program.clone());
            if strict {
                gate.observer = Observer::Strict;
            } else {
                gate.cln_fixture = None;
            }
            let request = Request::cln(config(program.clone()), [7; 32]);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(&gate, request, deadline(), 32)
            }));
            assert!(result.unwrap_err().is::<TestEgressBlocked>());
            assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
            assert_eq!(gate.attempts().records.len(), 1);
        }
    }
}
