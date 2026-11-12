//! Let external agents read the encrypted book the desktop has unlocked
//! (`kassiber/daemon_agent_session.py` and the `ui.agent_access.*` kinds of
//! `kassiber/daemon.py`).
//!
//! The desktop already holds the passphrase of the book it opened. On the
//! user's click it hands that passphrase to the operator broker as an agent
//! session: a read-only lease that admits only `kassiber mcp` tool calls,
//! only from MCP server processes the user allowed here, and that ends
//! after an idle timeout. The daemon keeps a random control secret that only
//! it can use to allow an agent, so a program that talks to the broker
//! directly cannot allow itself. The renderer never sees the passphrase or
//! the control secret. The broker is contacted only for the user's clicks,
//! while the desktop's own session exists (to show waiting agents and
//! activity, and every 30 s from the request loop), and when it ends.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use kassiber_envelope::{dict, get, s, AppError};
use kassiber_operator::client::{BrokerClient, Host};
use kassiber_operator::values::py_eq;
use kassiber_pyjson::{Key, PyStr, Value};
use zeroize::Zeroizing;

/// Every agent-access kind, in `SUPPORTED_KINDS` order.
pub const KINDS: &[&str] = &[
    "ui.agent_access.status",
    "ui.agent_access.configure",
    "ui.agent_access.unlock",
    "ui.agent_access.lock",
    "ui.agent_access.pairing",
];

/// The lease ends with the desktop session; this bounds it if the desktop
/// crashes or is killed before it can lock.
pub const DESKTOP_AGENT_LEASE_SECONDS: i64 = 8 * 60 * 60;
/// Without a tool call (or an approval) for this long, the broker ends it.
pub const AGENT_IDLE_TIMEOUT_SECONDS: i64 = 15 * 60;
/// While the session exists, how often the request loop asks the broker
/// whether it ended on its own (idle, broker gone), to restore the mode
/// promptly.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(30);
/// How often the request loop retries a lock the broker did not confirm.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(5);
/// After this long past its expiry, the broker has certainly dropped the
/// lease.
pub const GIVE_UP_AFTER_EXPIRY: Duration = Duration::from_secs(60);
const CONTROL_BYTES: usize = 32;

/// The broker lease this desktop granted agents for its open book.
pub(crate) struct DesktopAgentLease {
    data_root: String,
    /// The broker's public id for this lease: locking names it, so a lease
    /// someone else opened since is never ended by the desktop.
    lease_id: Value,
    expires_at: Value,
    /// Measured from before the grant request, so it never ends later here
    /// than in the broker.
    expires: Instant,
    /// Measured from after the grant, so cleanup never gives up while the
    /// broker may still hold the lease.
    give_up: Instant,
    /// The broker binds a book to `brokered` mode when it grants a lease; a
    /// book that was `manual` goes back to it when the desktop's lease ends.
    restore_manual: bool,
    /// Proves to the broker that this desktop granted the session. Never
    /// leaves the daemon except to the broker.
    control: Zeroizing<Vec<u8>>,
    /// Set when ending it failed: kept (and shown) until the broker
    /// confirms, and retried from the request loop.
    ending: bool,
    retry_at: Option<Instant>,
    next_check: Instant,
}

impl std::fmt::Debug for DesktopAgentLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopAgentLease")
            .field("lease_id", &self.lease_id)
            .field("ending", &self.ending)
            .finish_non_exhaustive()
    }
}

/// What the agent session asks the operator broker; [`BrokerClient`] in the
/// product, a fake in tests.
pub(crate) trait AgentBroker {
    /// `unlock(data_root, passphrase, duration 8 h, capability, password,
    /// only_if_locked, agent_control, idle 15 min)`: the capability is the
    /// user's External agents grant, never admin.
    fn unlock(
        &self,
        data_root: &str,
        passphrase: &[u8],
        control: &[u8],
        capability: &str,
    ) -> kassiber_operator::Result<Value>;
    fn status(&self, data_root: &str) -> kassiber_operator::Result<Value>;
    fn decide(
        &self,
        data_root: &str,
        session_id: &str,
        allow: bool,
        control: &[u8],
    ) -> kassiber_operator::Result<Value>;
    fn lock(
        &self,
        data_root: &str,
        expected_lease_id: &str,
        restore_manual: bool,
    ) -> kassiber_operator::Result<Value>;
    fn ping(&self) -> kassiber_operator::Result<Value>;
}

impl<H: Host> AgentBroker for BrokerClient<H> {
    fn unlock(
        &self,
        data_root: &str,
        passphrase: &[u8],
        control: &[u8],
        capability: &str,
    ) -> kassiber_operator::Result<Value> {
        BrokerClient::unlock(
            self,
            data_root,
            passphrase,
            Some(DESKTOP_AGENT_LEASE_SECONDS.into()),
            capability,
            "password",
            // A lease that already exists (say, from a terminal) is someone
            // else's session: never replace it, which could narrow it.
            true,
            // Agent-only: `mcp call` from processes allowed with `control`.
            Some(control),
            Some(AGENT_IDLE_TIMEOUT_SECONDS),
        )
    }
    fn status(&self, data_root: &str) -> kassiber_operator::Result<Value> {
        BrokerClient::status(self, Some(data_root))
    }
    fn decide(
        &self,
        data_root: &str,
        session_id: &str,
        allow: bool,
        control: &[u8],
    ) -> kassiber_operator::Result<Value> {
        self.decide_agent_session(data_root, session_id, allow, control)
    }
    fn lock(
        &self,
        data_root: &str,
        expected_lease_id: &str,
        restore_manual: bool,
    ) -> kassiber_operator::Result<Value> {
        BrokerClient::lock(self, data_root, Some(expected_lease_id), restore_manual)
    }
    fn ping(&self) -> kassiber_operator::Result<Value> {
        BrokerClient::ping(self)
    }
}

/// What the agent session reads of the daemon: `ctx.data_root`, whether a
/// book is open (`ctx.conn`), whether it was unlocked with a passphrase,
/// and the book's unlock mode.
pub(crate) struct Desktop<'a> {
    pub data_root: &'a str,
    pub book_open: bool,
    pub passphrase: Option<&'a str>,
    /// `_unlock_mode(data_root)`: any failure reads as `manual`.
    pub unlock_mode: &'a dyn Fn() -> String,
}

/// The daemon's agent-access state and its seams.
pub(crate) struct AgentAccess {
    pub(crate) lease: Option<DesktopAgentLease>,
    broker: Box<dyn Fn() -> Box<dyn AgentBroker> + Send>,
    clock: Box<dyn Fn() -> Instant + Send>,
    preference: Box<dyn Fn() -> Result<PathBuf, AppError> + Send>,
    entropy: Box<dyn Fn() -> Result<Zeroizing<Vec<u8>>, AppError> + Send>,
}

/// The `(OSError, EOFError, AppError)` Python catches around broker views;
/// any other failure propagates.
fn caught(error: &kassiber_operator::Error) -> bool {
    !matches!(error, kassiber_operator::Error::Value(_))
}

fn int(value: i64) -> Value {
    Value::from(value)
}

impl AgentAccess {
    /// The product: this build's broker client, the monotonic clock, the
    /// shared preference file and OS entropy.
    pub(crate) fn system(build: kassiber_runtime::BuildInfo) -> Self {
        AgentAccess::with(
            Box::new(move || {
                Box::new(BrokerClient::new(
                    kassiber_operator::client::SystemHost::new(
                        kassiber_operator::build::current_build_identity(
                            kassiber_operator::build::BuildInputs {
                                version: build.version,
                                channel: build.channel,
                                commit: build.commit,
                            },
                        ),
                    ),
                )) as Box<dyn AgentBroker>
            }),
            Box::new(Instant::now),
            Box::new(|| {
                let context = kassiber_paths::Context::from_env()
                    .map_err(|error| AppError::new(error.code, error.message.to_string()))?;
                kassiber_setup::agent_access::preference_path(&context)
            }),
            Box::new(|| {
                kassiber_operator::protocol::token_bytes(CONTROL_BYTES)
                    .map_err(kassiber_operator::error::into_app_error)
            }),
        )
    }

    pub(crate) fn with(
        broker: Box<dyn Fn() -> Box<dyn AgentBroker> + Send>,
        clock: Box<dyn Fn() -> Instant + Send>,
        preference: Box<dyn Fn() -> Result<PathBuf, AppError> + Send>,
        entropy: Box<dyn Fn() -> Result<Zeroizing<Vec<u8>>, AppError> + Send>,
    ) -> Self {
        AgentAccess {
            lease: None,
            broker,
            clock,
            preference,
            entropy,
        }
    }

    fn now(&self) -> Instant {
        (self.clock)()
    }

    fn expired(&self, lease: &DesktopAgentLease) -> bool {
        self.now() >= lease.expires
    }

    /// When the request loop next has work: the next check of a live
    /// session, its expiry, or the next retry of a failed lock.
    pub(crate) fn due(&self) -> Option<Instant> {
        let lease = self.lease.as_ref()?;
        Some(if lease.ending || self.expired(lease) {
            lease.retry_at.unwrap_or_else(|| self.now())
        } else {
            lease.next_check.min(lease.expires)
        })
    }

    /// `session_state(ctx, refresh=...)`: the book's agent session as the
    /// desktop knows it. Without `refresh` this reads memory only. With it,
    /// and only while the desktop's own session exists, it asks the broker
    /// for waiting agents and activity, and notices a session that ended on
    /// its own.
    pub(crate) fn session_state(
        &mut self,
        desktop: &Desktop<'_>,
        refresh: bool,
    ) -> Result<Value, AppError> {
        if let Some(lease) = self
            .lease
            .as_ref()
            .filter(|lease| lease.data_root == desktop.data_root && !self.expired(lease))
        {
            let state = vec![
                ("needed", Value::Bool(true)),
                ("active", Value::Bool(true)),
                ("expires_at", lease.expires_at.clone()),
                ("idle_timeout_seconds", int(AGENT_IDLE_TIMEOUT_SECONDS)),
            ];
            if !refresh || lease.ending {
                return Ok(dict(state));
            }
            let agent = match self.broker_view() {
                Ok(agent) => agent,
                Err(error) if caught(&error) => return Ok(dict(state)),
                Err(error) => return Err(kassiber_operator::error::into_app_error(error)),
            };
            let Some(agent) = agent else {
                // Ended in the broker (idle timeout, broker gone): clean up.
                self.end_lease(false)?;
                return self.session_state(desktop, false);
            };
            let mut state = state;
            state.extend(agent);
            return Ok(dict(state));
        }
        // Plaintext books and books with a remembered (unattended) unlock
        // already open for agents without a lease.
        let needed = desktop.book_open
            && desktop.passphrase.is_some_and(|text| !text.is_empty())
            && (desktop.unlock_mode)() != "unattended";
        Ok(dict([
            ("needed", Value::Bool(needed)),
            ("active", Value::Bool(false)),
            ("expires_at", Value::Null),
        ]))
    }

    /// `_broker_view(lease)`: waiting agents and activity, or `None` when
    /// the lease no longer exists.
    fn broker_view(&self) -> kassiber_operator::Result<Option<Vec<(&'static str, Value)>>> {
        let lease = self.lease.as_ref().expect("a desktop agent lease");
        let status = (self.broker)().status(&lease.data_root)?;
        let field = |value: &Value, key: &str| get(value, key).cloned().unwrap_or(Value::Null);
        if field(&status, "lease") != s("unlocked")
            || !py_eq(&field(&status, "lease_id"), &lease.lease_id)
        {
            return Ok(None);
        }
        let agent = match get(&status, "agent") {
            Some(agent @ Value::Dict(_)) => agent.clone(),
            _ => dict::<&str>([]),
        };
        let sessions = match get(&agent, "sessions") {
            Some(Value::List(sessions)) => sessions.clone(),
            _ => Vec::new(),
        };
        let agents = sessions
            .iter()
            .filter(|session| {
                matches!(session, Value::Dict(_))
                    && matches!(get(session, "id"), Some(Value::Str(_)))
            })
            .map(|session| {
                dict([
                    ("id", field(session, "id")),
                    ("label", field(session, "label")),
                    ("pid", field(session, "pid")),
                    ("state", field(session, "state")),
                    (
                        "calls",
                        get(session, "calls").cloned().unwrap_or_else(|| int(0)),
                    ),
                    ("last_call_at", field(session, "last_call_at")),
                ])
            })
            .collect();
        Ok(Some(vec![
            (
                "calls",
                get(&agent, "calls").cloned().unwrap_or_else(|| int(0)),
            ),
            ("last_call_at", field(&agent, "last_call_at")),
            (
                "idle_remaining_seconds",
                field(&agent, "idle_remaining_seconds"),
            ),
            ("agents", Value::List(agents)),
        ]))
    }

    /// `decide(ctx, session_id, allow)`: allow or deny one waiting agent
    /// process for the desktop's session.
    pub(crate) fn decide(
        &mut self,
        desktop: &Desktop<'_>,
        session_id: Option<&Value>,
        allow: Option<&Value>,
    ) -> Result<Value, AppError> {
        let (Some(Value::Str(session_id)), Some(Value::Bool(allow))) = (session_id, allow) else {
            return Err(Self::pairing_arguments());
        };
        if session_id.code_points().next().is_none() {
            return Err(Self::pairing_arguments());
        }
        let Some(lease) = self.lease.as_ref().filter(|lease| {
            lease.data_root == desktop.data_root && !self.expired(lease) && !lease.ending
        }) else {
            return Err(AppError::new(
                "agent_unlock_unavailable",
                "this book is not unlocked for agents",
            )
            .with_hint("Unlock the book for agents first."));
        };
        (self.broker)()
            .decide(
                &lease.data_root,
                &kassiber_envelope::py_str(&Value::Str(session_id.clone())),
                *allow,
                &lease.control,
            )
            .map_err(kassiber_operator::error::into_app_error)?;
        self.session_state(desktop, true)
    }

    fn pairing_arguments() -> AppError {
        AppError::new(
            "validation",
            "ui.agent_access.pairing takes a session_id and a boolean allow",
        )
    }

    /// `unlock(ctx)`: grant agents a lease from this book's passphrase,
    /// only if no lease exists. The lease carries the user's External agents
    /// grant (never admin), so brokered agents can do exactly what the user
    /// chose for them.
    pub(crate) fn unlock(&mut self, desktop: &Desktop<'_>) -> Result<Value, AppError> {
        let grant = kassiber_setup::agent_access::require_mcp_access(&(self.preference)()?)?.grant;
        let passphrase = match desktop.passphrase.filter(|text| !text.is_empty()) {
            Some(passphrase) if desktop.book_open => passphrase,
            _ => {
                return Err(AppError::new(
                    "agent_unlock_unavailable",
                    "no unlocked encrypted book is open in Kassiber",
                )
                .with_hint(
                    "Open and unlock the encrypted book first; plaintext books need no unlock.",
                ))
            }
        };
        if (desktop.unlock_mode)() == "unattended" {
            return Err(AppError::new(
                "agent_unlock_not_needed",
                "this book already opens for agents with its remembered unlock",
            ));
        }
        if let Some(current) = &self.lease {
            if current.data_root == desktop.data_root && !self.expired(current) && !current.ending {
                return self.session_state(desktop, false);
            }
            self.end_lease(true)?;
        }
        let passphrase = Zeroizing::new(passphrase.as_bytes().to_vec());
        let control = (self.entropy)()?;
        let started = self.now();
        let granted = match (self.broker)().unlock(desktop.data_root, &passphrase, &control, grant)
        {
            Ok(granted) => granted,
            Err(kassiber_operator::Error::App(error)) if error.code == "operator_lease_exists" => {
                let mut state = self.session_state(desktop, false)?;
                kassiber_envelope::set(&mut state, "existing_lease", Value::Bool(true));
                return Ok(state);
            }
            Err(error) => return Err(kassiber_operator::error::into_app_error(error)),
        };
        drop(passphrase);
        let granted_at = self.now();
        let lease_seconds = Duration::from_secs(DESKTOP_AGENT_LEASE_SECONDS as u64);
        self.lease = Some(DesktopAgentLease {
            data_root: desktop.data_root.to_owned(),
            lease_id: get(&granted, "lease_id").cloned().unwrap_or(Value::Null),
            expires_at: get(&granted, "expires_at").cloned().unwrap_or(Value::Null),
            expires: started + lease_seconds,
            give_up: granted_at + lease_seconds + GIVE_UP_AFTER_EXPIRY,
            // The broker reports the mode its grant replaced, read in the
            // same transition; restore only a book that really was manual.
            restore_manual: get(&granted, "previous_mode") == Some(&s("manual")),
            control,
            ending: false,
            retry_at: None,
            next_check: granted_at + CHECK_INTERVAL,
        });
        self.session_state(desktop, false)
    }

    /// `lock(ctx)`.
    pub(crate) fn lock(&mut self, desktop: &Desktop<'_>) -> Result<Value, AppError> {
        self.end_lease(true)?;
        self.session_state(desktop, false)
    }

    /// `expire(ctx)`, from the request loop: end an expired lease, retry a
    /// failed lock, and every 30 s ask the broker whether it ended a live
    /// session on its own (idle timeout), so a manual book is restored
    /// promptly.
    pub(crate) fn expire(&mut self) -> Result<(), AppError> {
        let Some(lease) = &self.lease else {
            return Ok(());
        };
        let now = self.now();
        if !(lease.ending || self.expired(lease)) {
            if now < lease.next_check {
                return Ok(());
            }
            if let Some(lease) = self.lease.as_mut() {
                lease.next_check = now + CHECK_INTERVAL;
            }
            let ended = match self.broker_view() {
                Ok(view) => view.is_none(),
                Err(error) if caught(&error) => return Ok(()),
                Err(error) => return Err(kassiber_operator::error::into_app_error(error)),
            };
            if !ended {
                return Ok(());
            }
        } else if lease.retry_at.is_some_and(|retry_at| now < retry_at) {
            return Ok(());
        }
        self.end_lease(false)?;
        if self
            .lease
            .as_ref()
            .is_some_and(|lease| self.now() >= lease.give_up)
        {
            // Whatever holds the endpoint now, this lease no longer exists
            // in a broker; stop retrying, and leave the mode to whoever set
            // it since.
            self.lease = None;
        }
        Ok(())
    }

    /// `end_lease(ctx, raise_errors=...)`: lock the lease this desktop
    /// created, and only that one. Called whenever the desktop session for
    /// the book ends (lock, quit, passphrase rotation, restore), when agents
    /// are turned off, and after the lease expired. The lease is forgotten
    /// only once the broker confirms; otherwise it stays visible and the
    /// request loop retries.
    pub(crate) fn end_lease(&mut self, raise_errors: bool) -> Result<(), AppError> {
        let Some(lease) = &self.lease else {
            return Ok(());
        };
        if self.lock_in_broker(lease).is_err() {
            let retry_at = self.now() + RETRY_INTERVAL;
            if let Some(lease) = self.lease.as_mut() {
                lease.ending = true;
                lease.retry_at = Some(retry_at);
            }
            if raise_errors {
                return Err(AppError::new(
                    "agent_lease_lock_failed",
                    "the operator broker did not confirm locking the agent lease",
                )
                .with_hint(
                    "Kassiber keeps retrying while it runs, and the lease ends on its own within 8 hours.",
                )
                .retryable(true));
            }
            return Ok(());
        }
        self.lease = None;
        Ok(())
    }

    /// `_lock_in_broker(lease)`: one broker transition locks only this lease
    /// and restores the mode, so a terminal unlock can neither be ended nor
    /// lose its mode.
    fn lock_in_broker(&self, lease: &DesktopAgentLease) -> kassiber_operator::Result<()> {
        let broker = (self.broker)();
        // Always conditional: a missing id matches no live lease.
        let result = broker.lock(
            &lease.data_root,
            &kassiber_envelope::py_str(&lease.lease_id),
            lease.restore_manual,
        )?;
        if get(&result, "broker") != Some(&s("stopped")) {
            return Ok(());
        }
        // A lost connection is not proof the broker is gone; only a failed
        // ping is, and a broker's leases end with it. Its mode stays as it
        // is: writing it here could race a broker starting up and a new
        // terminal lease.
        match broker.ping() {
            Err(error) if error.is_connection() => Ok(()),
            Err(error) => Err(error),
            Ok(_) => Err(kassiber_operator::Error::App(AppError::new(
                "operator_protocol_error",
                "the operator broker did not confirm the lock",
            ))),
        }
    }

    /// `_agent_access_payload(ctx, kind, request)`.
    pub(crate) fn payload(
        &mut self,
        desktop: &Desktop<'_>,
        kind: &str,
        args: Option<&Value>,
    ) -> Result<Value, AppError> {
        let empty = dict::<&str>([]);
        let args = match args {
            None => &empty,
            Some(value) if !kassiber_envelope::truthy(value) => &empty,
            Some(value @ Value::Dict(_)) => value,
            Some(_) => {
                return Err(AppError::new(
                    "validation",
                    "agent access arguments must be an object",
                ))
            }
        };
        let keys: Vec<PyStr> = match args {
            Value::Dict(pairs) => pairs
                .iter()
                .filter_map(|(key, _)| match key {
                    Key::Str(key) => Some(key.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let unknown = |allowed: &[&str]| {
            let mut unknown: Vec<PyStr> = keys
                .iter()
                .filter(|key| !allowed.iter().any(|name| key.is(name)))
                .cloned()
                .collect();
            unknown.sort();
            unknown.dedup();
            unknown
        };
        let listed = |unknown: Vec<PyStr>| {
            dict([(
                "unknown",
                Value::List(unknown.into_iter().map(Value::Str).collect()),
            )])
        };
        let session = if kind == "ui.agent_access.configure" {
            let unknown = unknown(&[
                "mcp_enabled",
                "ai_features_enabled",
                "grant",
                "agent_access",
            ]);
            let value = |key: &str| get(args, key);
            let switches = [value("mcp_enabled"), value("ai_features_enabled")];
            let choices = [value("grant"), value("agent_access")];
            if !unknown.is_empty()
                || switches.iter().chain(choices.iter()).all(Option::is_none)
                || switches
                    .iter()
                    .flatten()
                    .any(|value| !matches!(value, Value::Bool(_)))
                || choices
                    .iter()
                    .flatten()
                    .any(|value| !matches!(value, Value::Str(_)))
            {
                return Err(AppError::new(
                    "validation",
                    "ui.agent_access.configure takes boolean mcp_enabled/ai_features_enabled and string grant/agent_access",
                )
                .with_details(listed(unknown)));
            }
            let flag = |value: Option<&Value>| match value {
                Some(Value::Bool(flag)) => Some(*flag),
                _ => None,
            };
            let path = (self.preference)()?;
            let previous_grant = kassiber_setup::agent_access::read(&path).grant;
            let result = kassiber_setup::agent_access::set(
                &path,
                kassiber_setup::agent_access::Changes {
                    mcp_enabled: flag(switches[0]),
                    ai_features_enabled: flag(switches[1]),
                    grant: choices[0].cloned(),
                    agent_access: choices[1].cloned(),
                },
            )?;
            if get(&result, "mcp_available") != Some(&Value::Bool(true))
                || get(&result, "grant") != Some(&s(previous_grant))
            {
                // Turning agents (or AI) off ends the lease the desktop
                // granted them, and so does a new grant: the next unlock
                // leases exactly that grant.
                self.end_lease(false)?;
            }
            let mut result = result;
            kassiber_envelope::set(&mut result, "session", self.session_state(desktop, false)?);
            return Ok(result);
        } else {
            let allowed: &[&str] = match kind {
                "ui.agent_access.status" => &["refresh"],
                "ui.agent_access.pairing" => &["session_id", "allow"],
                _ => &[],
            };
            let unknown = unknown(allowed);
            let refresh = get(args, "refresh");
            if !unknown.is_empty() || !matches!(refresh, None | Some(Value::Bool(_))) {
                return Err(AppError::new(
                    "validation",
                    format!("{kind} got unexpected arguments"),
                )
                .with_details(listed(unknown)));
            }
            match kind {
                "ui.agent_access.unlock" => self.unlock(desktop)?,
                "ui.agent_access.lock" => self.lock(desktop)?,
                "ui.agent_access.pairing" => {
                    self.decide(desktop, get(args, "session_id"), get(args, "allow"))?
                }
                // Refreshing contacts the broker only while the desktop's
                // own agent session exists; otherwise this reads memory.
                _ => self.session_state(desktop, refresh == Some(&Value::Bool(true)))?,
            }
        };
        let mut payload = kassiber_setup::agent_access::status(&(self.preference)()?);
        kassiber_envelope::set(&mut payload, "session", session);
        Ok(payload)
    }
}

#[cfg(test)]
#[path = "agent_access_tests.rs"]
mod tests;
