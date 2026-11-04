use super::{setting, Connection};
use kassiber_chain::lightning::{ClnReportClient, LndReportClient, ReportAction};
use kassiber_envelope::{dict, get, py_str, set, AppError};
use kassiber_net::{Consent, Gate, Request};
use kassiber_pyjson::Value;
use kassiber_reports::lightning::{self, ConnectionRef};
use kassiber_setup::backends::Config;
use std::path::Path;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, AppError>;

pub(super) fn network_gate() -> Gate {
    use kassiber_net::{Ledger, Offline};
    let gate = Gate::new(
        || match kassiber_paths::Context::from_env()
            .ok()
            .and_then(|ctx| kassiber_setup::offline::preference_path(&ctx).ok())
        {
            Some(path) if kassiber_setup::offline::enabled(&path) => Offline::Offline,
            Some(_) => Offline::Online,
            None => Offline::Unavailable,
        },
        Ledger::default(),
    );
    #[cfg(feature = "test-harness")]
    let gate = match std::env::var_os("KASSIBER_RUST_TEST_CLN_FIXTURE") {
        Some(program) => gate.with_cln_fixture(program.into()),
        None => gate,
    };
    gate
}

pub(super) const COMMANDS: &[&str] = &[
    "reports.lightning-profitability",
    "reports.export-lightning-profitability-csv",
    "ui.reports.lightning_profitability",
];

enum Client {
    Cln(ClnReportClient),
    Lnd(LndReportClient),
}

impl Client {
    fn request(&self) -> &Request {
        match self {
            Self::Cln(c) => c.request(),
            Self::Lnd(c) => c.request(),
        }
    }
}

struct Grant(Consent);

impl Drop for Grant {
    fn drop(&mut self) {
        self.0.revoke();
    }
}

fn capabilities(kind: &str, desktop: bool) -> Result<Value> {
    if !["coreln", "lnd"].contains(&kind) {
        let suffix = if desktop {
            " and configure that connection"
        } else {
            ""
        };
        return Err(AppError::new(
            "lightning_adapter_unavailable",
            format!("No Lightning sync adapter is registered for kind '{kind}'."),
        ).with_hint(format!("Registered Lightning kinds: coreln, lnd. Install the matching Lightning sync (LND or Core Lightning){suffix}.")));
    }
    Ok(dict(
        [
            "nodeSnapshot",
            "routingProfitability",
            "channelBalances",
            "channelLifecycle",
            "forwardEvents",
            "invoiceActivity",
            "paymentActivity",
            "onchainBalance",
        ]
        .map(|k| (k, Value::Bool(true))),
    ))
}

fn backend(conn: &Connection, config: &Config, connection: &ConnectionRef) -> Result<Value> {
    connection
        .backend_name
        .as_ref()
        .map(|name| {
            let name = name
                .as_str()
                .ok_or_else(|| AppError::new("internal_error", "Invalid backend name encoding"))?;
            config.resolve_lightning(conn, name)
        })
        .transpose()
        .map(|v| v.unwrap_or(Value::Null))
}

fn state(conn: &Connection, connection: &ConnectionRef) -> Result<Vec<Option<String>>> {
    let id = py_str(&connection.id);
    let wallet: Option<String> = conn
        .query_row("SELECT config_json FROM wallets WHERE id=?", [id], |r| {
            r.get(0)
        })
        .map_err(kassiber_store::map_sqlite_error)
        .map_err(super::error)?;
    Ok(vec![
        setting(conn, "database_instance_id")?,
        setting(conn, "context_workspace")?,
        setting(conn, "context_profile")?,
        wallet,
    ])
}

struct Prepared<'a> {
    conn: &'a Connection,
    config: &'a Config,
    args: Value,
    connection: ConnectionRef,
    backend: Value,
    state: Vec<Option<String>>,
    capabilities: Value,
    client: Client,
    gate: Gate,
    file: Option<String>,
}

impl<'a> Prepared<'a> {
    fn new(
        conn: &'a Connection,
        config: &'a Config,
        command: &str,
        args: &Value,
        gate: Gate,
        version: &str,
    ) -> Result<Self> {
        if !COMMANDS.contains(&command) {
            return Err(AppError::new(
                "not_implemented",
                "Unsupported Lightning report action",
            ));
        }
        let desktop = command.starts_with("ui.");
        let mut args = args.clone();
        if desktop {
            for key in ["workspace", "profile"] {
                if !matches!(get(&args, key), Some(Value::Str(_))) {
                    set(&mut args, key, Value::Null);
                }
            }
        }
        let connection = lightning::resolve_connection(conn, &args)?;
        let capabilities = capabilities(&connection.kind, desktop)?;
        let window =
            lightning::daemon_window_days(get(&args, "window_days").unwrap_or(&Value::Null))?;
        let backend = backend(conn, config, &connection)?;
        let file = (command == "reports.export-lightning-profitability-csv")
            .then(|| get(&args, "file").map(py_str).unwrap_or_default());
        let state = state(conn, &connection)?;
        let book_id = state[0]
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AppError::new("validation", "Book identity is unavailable"))?;
        let action = ReportAction {
            operation: command.into(),
            book_id,
            profile_id: py_str(&connection.profile_id),
            connection_id: py_str(&connection.id),
            window_days: window,
            export_file: file.clone(),
        };
        let raw = kassiber_pyjson::dumps(&backend, &Default::default())
            .map_err(|_| AppError::new("config_error", "Invalid backend configuration"))?;
        let raw = serde_json::from_str(&raw)
            .map_err(|_| AppError::new("config_error", "Invalid backend configuration"))?;
        let client = match connection.kind.as_str() {
            "coreln" => Client::Cln(ClnReportClient::new(&raw, gate.clone(), action)?),
            _ => Client::Lnd(LndReportClient::new(&raw, gate.clone(), version, action)?),
        };
        Ok(Self {
            conn,
            config,
            args,
            connection,
            backend,
            state,
            capabilities,
            client,
            gate,
            file,
        })
    }

    fn check(&self) -> Result<()> {
        let current = (|| {
            let connection = lightning::resolve_connection(self.conn, &self.args)?;
            Ok(connection == self.connection
                && state(self.conn, &connection)? == self.state
                && backend(self.conn, self.config, &connection)? == self.backend)
        })();
        if !matches!(current, Ok::<_, AppError>(true)) {
            return Err(AppError::new(
                "interaction_required",
                "Lightning report scope or configuration changed; trigger the report again.",
            ));
        }
        Ok(())
    }

    fn run(&self, consent: Option<&Consent>) -> Result<Value> {
        self.check()?;
        let deadline = Instant::now() + Duration::from_secs(300);
        let snapshot = match &self.client {
            Client::Cln(c) => c.fetch_checked(consent, deadline, || self.check())?,
            Client::Lnd(c) => c.fetch_checked(consent, deadline, || self.check())?,
        };
        self.check()?;
        self.gate
            .authorize(self.client.request().clone(), consent.cloned())
            .map_err(|e| e.app_error())?;
        let snapshot = lightning::Snapshot {
            channels: snapshot
                .channels
                .into_iter()
                .map(|c| lightning::Channel {
                    id: c.id,
                    peer_alias: c.peer_alias,
                    capacity_sat: c.capacity_sat,
                    earned_routing_sat: c.earned_routing_sat,
                })
                .collect(),
            routing: snapshot.routing.map(|r| lightning::Routing {
                window_label: r.window_label.into(),
                routing_revenue_sat: r.routing_revenue_sat,
                payment_cost_sat: r.payment_cost_sat,
                rebalance_cost_sat: r.rebalance_cost_sat,
                onchain_cost_sat: r.onchain_cost_sat,
                net_profit_sat: r.net_profit_sat,
                forward_count: r.forward_count.into(),
                payment_count: r.payment_count.into(),
                rebalance_count: r.rebalance_count.into(),
            }),
        };
        let report = lightning::build_report(
            self.connection.clone(),
            snapshot,
            lightning::DEFAULT_OPEN_COST_SAT.into(),
        );
        if let Some(file) = &self.file {
            return report.export_csv(Path::new(file));
        }
        let mut payload = report.payload();
        let mut connection = get(&payload, "connection").unwrap().clone();
        set(
            &mut connection,
            "lightningCapabilities",
            self.capabilities.clone(),
        );
        set(&mut payload, "connection", connection);
        Ok(payload)
    }
}

/// `ui.backends.lightning.test` (`_test_lightning_backend_payload`): the
/// user's explicit test of one configured Lightning backend. Its only egress
/// is that node's snapshot through the gate, granted for this request alone;
/// node responses are reduced to the alias, network, height and counts.
pub(super) fn backend_test(
    conn: &Connection,
    config: &Config,
    args: &Value,
    gate: Gate,
    version: &str,
) -> Result<Value> {
    let reference = match get(args, "backend") {
        None | Some(Value::Null) => None,
        Some(Value::Str(text)) => {
            Some(kassiber_pytext::strip_py(text)).filter(|text| text.code_points().next().is_some())
        }
        Some(value) => {
            return Err(
                AppError::new("validation", "backend must be a string").with_details(dict([(
                    "type",
                    kassiber_envelope::s(kassiber_envelope::type_name(value)),
                )])),
            )
        }
    };
    let reference = reference.ok_or_else(|| {
        AppError::new("validation", "Lightning backend is required.")
            .with_hint("Enter lightning backend.")
    })?;
    let reference = reference
        .as_str()
        .ok_or_else(|| AppError::new("app_error", "Invalid backend name encoding"))?
        .to_owned();
    let backend = config.resolve_runtime(&reference)?;
    let name = match get(&backend, "name").filter(|v| kassiber_envelope::truthy(v)) {
        Some(value) => kassiber_pytext::strip(&py_str(value)).to_owned(),
        None => kassiber_pytext::strip(&reference).to_owned(),
    };
    let name = if name.is_empty() {
        reference.clone()
    } else {
        name
    };
    let kind = match get(&backend, "kind").filter(|v| kassiber_envelope::truthy(v)) {
        Some(value) => kassiber_pytext::lower(&py_str(value)),
        None => String::new(),
    };
    fn s(value: impl AsRef<str>) -> Value {
        kassiber_envelope::s(value.as_ref())
    }
    let mut logs = vec![s(format!(
        "Opening Lightning connection to backend '{name}'"
    ))];
    let refusal = |kind: Value, logs: Vec<Value>, error: Option<Value>| {
        let mut payload = dict([
            ("ok", Value::Bool(false)),
            ("backend", s(&name)),
            ("kind", kind),
            ("logs", Value::List(logs)),
        ]);
        if let Some(error) = error {
            set(&mut payload, "error", error);
        }
        payload
    };
    if !["coreln", "lnd", "nwc"].contains(&kind.as_str()) {
        logs.push(s(format!(
            "Backend kind '{}' is not a Lightning node backend.",
            if kind.is_empty() { "unknown" } else { &kind }
        )));
        let shown = if kind.is_empty() {
            Value::Null
        } else {
            s(&kind)
        };
        return Ok(refusal(shown, logs, None));
    }
    let capabilities = match capabilities(&kind, true) {
        Ok(capabilities) => capabilities,
        Err(error) => {
            logs.push(s(error.hint.as_deref().unwrap_or(&error.message)));
            let hint = error.hint.as_deref().map_or(Value::Null, s);
            let details = dict([
                ("code", s(&error.code)),
                ("message", s(&error.message)),
                ("hint", hint),
            ]);
            return Ok(refusal(s(&kind), logs, Some(details)));
        }
    };
    let fetched = (|| {
        let raw = config.resolve_lightning(conn, &reference)?;
        let raw = kassiber_pyjson::dumps(&raw, &Default::default())
            .map_err(|_| AppError::new("config_error", "Invalid backend configuration"))?;
        let raw = serde_json::from_str(&raw)
            .map_err(|_| AppError::new("config_error", "Invalid backend configuration"))?;
        let book_id = setting(conn, "database_instance_id")?
            .filter(|id| !id.is_empty())
            .ok_or_else(|| AppError::new("validation", "Book identity is unavailable"))?;
        let action = ReportAction {
            operation: "ui.backends.lightning.test".into(),
            book_id,
            profile_id: "backend-test".into(),
            connection_id: name.clone(),
            window_days: 1,
            export_file: None,
        };
        let client = match kind.as_str() {
            "coreln" => Client::Cln(ClnReportClient::new(&raw, gate.clone(), action)?),
            _ => Client::Lnd(LndReportClient::new(&raw, gate.clone(), version, action)?),
        };
        let grant = Grant(Consent::explicit_trigger(client.request()));
        let deadline = Instant::now() + Duration::from_secs(300);
        let snapshot = match &client {
            Client::Cln(c) => c.fetch(Some(&grant.0), deadline)?,
            Client::Lnd(c) => c.fetch(Some(&grant.0), deadline)?,
        };
        gate.authorize(client.request().clone(), Some(grant.0.clone()))
            .map_err(|e| e.app_error())?;
        Ok::<_, AppError>(snapshot)
    })();
    let snapshot = match fetched {
        Ok(snapshot) => snapshot,
        Err(error) => {
            logs.push(s(format!("Connection failed: {}", error.message)));
            return Ok(refusal(
                s(&kind),
                logs,
                Some(dict([("message", s(&error.message))])),
            ));
        }
    };
    let node = snapshot.node;
    let channels = snapshot.channels.len();
    let alias = if node.alias.code_points().next().is_some() {
        py_str(&Value::Str(node.alias.clone()))
    } else {
        name.clone()
    };
    logs.push(s(format!(
        "Connected to {alias}: {channels} channels, {} peers",
        node.peer_count
    )));
    if let Some(height) = &node.block_height {
        logs.push(s(format!("Block height: {height}")));
    }
    Ok(dict([
        ("ok", Value::Bool(true)),
        ("backend", s(&name)),
        ("kind", s(&kind)),
        ("lightningCapabilities", capabilities),
        ("alias", Value::Str(node.alias)),
        ("network", Value::Str(node.network)),
        (
            "block_height",
            node.block_height.map_or(Value::Null, Value::Int),
        ),
        ("peer_count", Value::Int(node.peer_count)),
        ("channel_count", Value::from(channels)),
        ("logs", Value::List(logs)),
    ]))
}

pub(super) fn execute(
    conn: &Connection,
    config: &Config,
    command: &str,
    args: &Value,
    gate: Gate,
    version: &str,
) -> Result<Value> {
    let prepared = Prepared::new(conn, config, command, args, gate, version)?;
    let grant = Grant(Consent::explicit_trigger(prepared.client.request()));
    prepared.run(Some(&grant.0))
}

#[cfg(all(test, feature = "test-harness"))]
mod tests {
    use super::*;
    use kassiber_envelope::s;
    use kassiber_net::{test_support::FakeProvider, Ledger, Offline};
    use kassiber_store::Book;

    fn fixture(root: &Path, url: &str) -> (Book, Config, Value) {
        let book = Book::create(&root.join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute(
            "UPDATE wallets SET kind='lnd',config_json='{\"backend\":\"fake\"}' WHERE id='cold'",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO backends(name,kind,url,token,timeout,created_at,updated_at) VALUES('fake','lnd',?,'aabb',2,'fixed','fixed')", [url]).unwrap();
        let config = Config::load_with_env(&root.join("absent.env"), &Default::default()).unwrap();
        (
            book,
            config,
            dict([
                ("connection", s("cold")),
                ("workspace", s("ws")),
                ("profile", s("p")),
            ]),
        )
    }

    fn gate() -> Gate {
        Gate::new(|| Offline::Online, Ledger::default())
    }

    #[test]
    fn lightning_service_requires_exact_grant_and_current_book_config_without_traffic() {
        let node = FakeProvider::new("{}").unwrap();
        let root = tempfile::tempdir().unwrap();
        let (book, config, args) = fixture(root.path(), node.base());
        let conn = book.connection();
        let gate = gate();
        let prepared =
            Prepared::new(conn, &config, COMMANDS[0], &args, gate.clone(), "test").unwrap();
        assert_eq!(prepared.run(None).unwrap_err().code, "interaction_required");
        let consent = Consent::explicit_trigger(prepared.client.request());
        let desktop =
            Prepared::new(conn, &config, COMMANDS[2], &args, gate.clone(), "test").unwrap();
        assert_eq!(
            desktop.run(Some(&consent)).unwrap_err().code,
            "interaction_required"
        );
        let mut changed_args = args.clone();
        set(&mut changed_args, "window_days", Value::Int(7.into()));
        let changed = Prepared::new(
            conn,
            &config,
            COMMANDS[0],
            &changed_args,
            gate.clone(),
            "test",
        )
        .unwrap();
        assert_eq!(
            changed.run(Some(&consent)).unwrap_err().code,
            "interaction_required"
        );
        let mut export_args = args.clone();
        set(
            &mut export_args,
            "file",
            s(root
                .path()
                .join("report.csv")
                .to_string_lossy()
                .into_owned()),
        );
        let changed = Prepared::new(
            conn,
            &config,
            COMMANDS[1],
            &export_args,
            gate.clone(),
            "test",
        )
        .unwrap();
        assert_eq!(
            changed.run(Some(&consent)).unwrap_err().code,
            "interaction_required"
        );
        assert!(!root.path().join("report.csv").exists());
        consent.revoke();
        assert_eq!(
            prepared.run(Some(&consent)).unwrap_err().code,
            "interaction_required"
        );
        for (change, restore) in [
            (
                "UPDATE settings SET value='changed' WHERE key='database_instance_id'",
                "UPDATE settings SET value='0123456789abcdef0123456789abcdef' WHERE key='database_instance_id'",
            ),
            (
                "UPDATE settings SET value='other' WHERE key='context_profile'",
                "UPDATE settings SET value='p' WHERE key='context_profile'",
            ),
            (
                "UPDATE wallets SET config_json='{\"backend\":\"fake\",\"display\":\"changed\"}' WHERE id='cold'",
                "UPDATE wallets SET config_json='{\"backend\":\"fake\"}' WHERE id='cold'",
            ),
            (
                "UPDATE backends SET token='ccdd' WHERE name='fake'",
                "UPDATE backends SET token='aabb' WHERE name='fake'",
            ),
        ] {
            let prepared =
                Prepared::new(conn, &config, COMMANDS[0], &args, gate.clone(), "test").unwrap();
            let proof = Consent::explicit_trigger(prepared.client.request());
            conn.execute_batch(change).unwrap();
            assert_eq!(
                prepared.run(Some(&proof)).unwrap_err().code,
                "interaction_required",
                "{change}"
            );
            conn.execute_batch(restore).unwrap();
        }
        let offline = Gate::new(|| Offline::Offline, Ledger::default());
        assert_eq!(
            execute(conn, &config, COMMANDS[0], &args, offline, "test")
                .unwrap_err()
                .code,
            "network_egress_disabled"
        );
        assert_eq!(gate.attempts().records.len(), 0);
        assert_eq!(gate.ledger().snapshot(0, 2000).records.len(), 0);
        assert!(node.requests().is_empty());
    }

    #[test]
    fn lightning_service_capability_backend_and_grant_lifetime() {
        let node = FakeProvider::new("{}").unwrap();
        let root = tempfile::tempdir().unwrap();
        let (book, config, args) = fixture(root.path(), node.base());
        let conn = book.connection();
        let before: i64 = conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        let gate = gate();
        let prepared =
            Prepared::new(conn, &config, COMMANDS[0], &args, gate.clone(), "test").unwrap();
        let proof;
        {
            let grant = Grant(Consent::explicit_trigger(prepared.client.request()));
            proof = grant.0.clone();
            let payload = prepared.run(Some(&grant.0)).unwrap();
            assert_eq!(
                get(
                    get(&payload, "connection").unwrap(),
                    "lightningCapabilities"
                ),
                Some(&capabilities("lnd", false).unwrap())
            );
            let declared = get(
                get(&payload, "connection").unwrap(),
                "lightningCapabilities",
            )
            .unwrap();
            for key in [
                "nodeSnapshot",
                "routingProfitability",
                "channelBalances",
                "channelLifecycle",
                "forwardEvents",
                "invoiceActivity",
                "paymentActivity",
                "onchainBalance",
            ] {
                assert_eq!(get(declared, key), Some(&Value::Bool(true)));
            }
            assert_eq!(get(&payload, "windowLabel"), Some(&s("Last 30 days")));
        }
        assert!(gate
            .authorize(prepared.client.request().clone(), Some(proof))
            .is_err());
        assert_eq!(node.requests().len(), 9);
        assert_eq!(
            before,
            conn.query_row::<i64, _, _>("SELECT total_changes()", [], |r| r.get(0))
                .unwrap()
        );
        conn.execute("UPDATE wallets SET kind='nwc' WHERE id='cold'", [])
            .unwrap();
        let error = Prepared::new(conn, &config, COMMANDS[0], &args, gate.clone(), "test")
            .err()
            .unwrap();
        assert_eq!(error.code, "lightning_adapter_unavailable");
        conn.execute(
            "UPDATE wallets SET kind='lnd',config_json='{\"backend\":\"absent\"}' WHERE id='cold'",
            [],
        )
        .unwrap();
        let error = Prepared::new(conn, &config, COMMANDS[0], &args, gate.clone(), "test")
            .err()
            .unwrap();
        assert_eq!(error.code, "not_found");
        assert_eq!(node.requests().len(), 9);
    }

    #[test]
    fn lightning_backend_resolution_preserves_db_config_and_environment_precedence() {
        let root = tempfile::tempdir().unwrap();
        let (book, _, _) = fixture(root.path(), "http://must-not-resolve.invalid");
        let conn = book.connection();
        conn.execute("UPDATE backends SET config_json='{\"lightning_cli\":\"private-wrapper\",\"custom\":\"preserved\"}' WHERE name='fake'", []).unwrap();
        let config = Config::load_with_env(
            &root.path().join("absent.env"),
            &[
                ("KASSIBER_BACKEND_FAKE_KIND".into(), "lnd".into()),
                (
                    "KASSIBER_BACKEND_FAKE_URL".into(),
                    "http://must-not-resolve.invalid".into(),
                ),
                ("KASSIBER_BACKEND_FAKE_TOKEN".into(), "ccdd".into()),
            ]
            .into_iter()
            .collect(),
        )
        .unwrap();
        let raw = config.resolve_lightning(conn, " FAKE ").unwrap();
        assert_eq!(get(&raw, "token"), Some(&s("ccdd")));
        assert_eq!(get(&raw, "lightning_cli"), Some(&s("private-wrapper")));
        assert_eq!(get(&raw, "custom"), Some(&s("preserved")));
        conn.execute(
            "UPDATE backends SET config_json='malformed' WHERE name='fake'",
            [],
        )
        .unwrap();
        let raw = config.resolve_lightning(conn, "fake").unwrap();
        assert_eq!(get(&raw, "token"), Some(&s("ccdd")));
        assert!(get(&raw, "lightning_cli").is_none());
        assert_eq!(
            config.resolve_lightning(conn, "missing").unwrap_err().code,
            "not_found"
        );
    }
}
