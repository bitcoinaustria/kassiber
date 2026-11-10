//! Desktop exchange API connections (Rust only; docs/reference/exchange-desktop.md).
//!
//! `status` and `create` are local. `test` and `sync` are the only kinds that
//! contact an exchange, each for one explicit user action under a consent
//! minted for exactly that route. `create` stores the backend and wallet rows
//! `backends create` and `wallets sync-<kind>` store, and `sync` runs that
//! very command, so the book holds the CLI's bytes. The last result of each
//! wallet's sync lives in this open book's memory only: the CLI stores none,
//! and its rows must stay Python's.
use super::*;
use kassiber_envelope::get;
use kassiber_wallets::file_import::exchange_api::{self, desktop};

pub(super) const COMMANDS: &[&str] = &[
    "ui.connections.exchange.status",
    "ui.connections.exchange.test",
    "ui.connections.exchange.create",
    "ui.connections.exchange.sync",
];

/// The fields each kind accepts; anything else is refused before any work.
fn allowed(operation: &str) -> &'static [&'static str] {
    match operation {
        "ui.connections.exchange.status" => &["wallet"],
        "ui.connections.exchange.test" => &["backend", "exchange", "api_key", "api_secret", "url"],
        "ui.connections.exchange.create" => &["exchange", "label", "api_key", "api_secret", "url"],
        _ => &["wallet"],
    }
}

fn check_fields(operation: &str, args: &Value) -> Result<(), AppError> {
    let Value::Dict(fields) = args else {
        return Ok(());
    };
    let mut unknown: Vec<String> = fields
        .iter()
        .map(|(key, _)| match key {
            kassiber_pyjson::Key::Str(key) => kassiber_envelope::py_str(&Value::Str(key.clone())),
            _ => String::from("<non-string key>"),
        })
        .filter(|key| !allowed(operation).contains(&key.as_str()))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort();
    Err(AppError::new(
        "validation",
        format!("{operation} received unsupported arguments"),
    )
    .with_details(dict([(
        "unsupported",
        Value::List(unknown.into_iter().map(s).collect()),
    )])))
}

/// A stripped, non-empty string argument.
fn text(args: &Value, key: &str) -> Result<Option<String>, AppError> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(value)) => Ok(value
            .as_str()
            .map(|v| kassiber_pytext::strip(v).to_owned())
            .filter(|v| !v.is_empty())),
        Some(_) => Err(AppError::new(
            "validation",
            format!("{key} must be a string"),
        )),
    }
}

fn exchange(args: &Value) -> Result<String, AppError> {
    let kind = text(args, "exchange")?
        .map(|kind| kassiber_pytext::lower(&kind))
        .ok_or_else(|| {
            AppError::new("validation", "Choose an exchange")
                .with_hint("One of kraken, coinbase, binance or bitpanda.")
        })?;
    if !exchange_api::KINDS.contains(&kind.as_str()) {
        return Err(
            AppError::new("validation", format!("Unsupported exchange '{kind}'"))
                .with_hint("One of kraken, coinbase, binance or bitpanda."),
        );
    }
    Ok(kind)
}

/// A pasted key (and secret, which Bitpanda does not use).
struct Credentials {
    key: String,
    secret: Option<String>,
}

fn credentials(kind: &str, args: &Value) -> Result<Credentials, AppError> {
    let key = text(args, "api_key")?.ok_or_else(|| {
        AppError::new("validation", "Paste the exchange API key")
            .with_hint("Create a read-only key in your exchange account first.")
    })?;
    let secret = text(args, "api_secret")?;
    if kind != "bitpanda" && secret.is_none() {
        return Err(AppError::new(
            "validation",
            "Paste the API secret that belongs to this key",
        )
        .with_hint("The exchange shows the secret once, when the key is created."));
    }
    // Bitpanda signs nothing; a stray secret is neither sent nor stored.
    let secret = secret.filter(|_| kind != "bitpanda");
    Ok(Credentials { key, secret })
}

fn url(kind: &str, args: &Value) -> Result<String, AppError> {
    Ok(text(args, "url")?.unwrap_or_else(|| desktop::default_api_url(kind).to_owned()))
}

impl RealBook {
    fn exchange_profile(&self) -> Result<String, AppError> {
        let (_, profile) = kassiber_setup::references::scope(self.conn()?, &dict::<&str>([]))?;
        Ok(kassiber_envelope::py_str(
            get(&profile, "id").unwrap_or(&Value::Null),
        ))
    }

    pub(super) fn exchange_service(
        &mut self,
        operation: &str,
        args: &Value,
    ) -> Result<Value, AppError> {
        check_fields(operation, args)?;
        match operation {
            "ui.connections.exchange.status" => self.exchange_status(args),
            "ui.connections.exchange.test" => self.exchange_test(args),
            "ui.connections.exchange.create" => self.exchange_create(args),
            _ => self.exchange_sync(args),
        }
    }

    /// Every exchange wallet in the active profile, its backend's presence
    /// and this session's last sync. Local reads only.
    fn exchange_status(&mut self, args: &Value) -> Result<Value, AppError> {
        let profile = self.exchange_profile()?;
        let conn = self.conn()?;
        let mut config = self.backends.clone();
        config.merge(conn)?;
        let only = text(args, "wallet")?
            .map(|reference| desktop::sync_target(conn, &profile, &reference))
            .transpose()?;
        let targets = match only {
            Some(target) => vec![target],
            None => desktop::targets(conn, &profile)?,
        };
        let mut rows = vec![];
        for target in targets {
            let resolved = target
                .backend
                .as_deref()
                .filter(|name| config.configured(&name.to_lowercase()).is_some())
                .map(|name| config.resolve(conn, name))
                .transpose()?;
            let present = |keys: &[&str]| {
                resolved.as_ref().is_some_and(|backend| {
                    keys.iter().any(|key| {
                        backend
                            .get(*key)
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|v| !v.trim().is_empty())
                    })
                })
            };
            let has_key = present(&["token", "api_key", "username"]);
            let has_secret = present(&["auth_header", "api_secret", "password"]);
            rows.push(dict([
                ("wallet_id", s(target.id.as_str())),
                ("wallet", s(target.label.as_str())),
                ("exchange", s(target.kind.as_str())),
                ("provider", s(desktop::provider_name(&target.kind))),
                ("backend", target.backend.clone().map_or(Value::Null, s)),
                ("backend_configured", Value::Bool(resolved.is_some())),
                ("has_api_key", Value::Bool(has_key)),
                (
                    "has_api_secret",
                    if target.kind == "bitpanda" {
                        Value::Null
                    } else {
                        Value::Bool(has_secret)
                    },
                ),
                (
                    "last_sync",
                    self.exchange_syncs
                        .get(&target.id)
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
            ]));
        }
        Ok(dict([("connections", Value::List(rows))]))
    }

    /// `ui.connections.exchange.test`: one explicit check of a saved backend
    /// or of a pasted key that is not stored.
    fn exchange_test(&mut self, args: &Value) -> Result<Value, AppError> {
        let conn = self.conn()?;
        let backend = match text(args, "backend")? {
            Some(name) => {
                if ["exchange", "api_key", "api_secret", "url"]
                    .iter()
                    .any(|key| !matches!(get(args, key), None | Some(Value::Null)))
                {
                    return Err(AppError::new(
                        "validation",
                        "Test either a saved exchange backend or a pasted key, not both",
                    ));
                }
                let mut config = self.backends.clone();
                config.merge(conn)?;
                config.resolve_configured(conn, Some(&name))?
            }
            None => {
                let kind = exchange(args)?;
                let credentials = credentials(&kind, args)?;
                let mut backend = serde_json::json!({
                    "name": kind,
                    "kind": kind,
                    "url": url(&kind, args)?,
                    "token": credentials.key,
                });
                if let Some(secret) = credentials.secret {
                    backend["auth_header"] = serde_json::json!(secret);
                }
                backend
            }
        };
        // This branch runs only for the user's explicit Test connection
        // request, after scope and unlock checks; the grant ends with it.
        let mut minted = None;
        let result = desktop::probe(
            conn,
            &backend,
            &self.lightning_gate,
            env!("CARGO_PKG_VERSION"),
            &mut |route| {
                let consent = kassiber_net::Consent::explicit_trigger(route.request());
                minted = Some(consent.clone());
                Some(consent)
            },
        );
        if let Some(consent) = minted {
            consent.revoke();
        }
        result
    }

    /// `ui.connections.exchange.create`: the backend `backends create` stores
    /// and the provider wallet `wallets sync-<kind>` creates. No network.
    fn exchange_create(&mut self, args: &Value) -> Result<Value, AppError> {
        let kind = exchange(args)?;
        let credentials = credentials(&kind, args)?;
        let url = url(&kind, args)?;
        let label =
            text(args, "label")?.unwrap_or_else(|| desktop::provider_name(&kind).to_owned());
        let profile = self.exchange_profile()?;
        let conn = self
            .book
            .as_ref()
            .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
            .connection();
        // The sync imports into any wallet already carrying the label, so a
        // new connection must not share one.
        let taken: bool = conn
            .query_row(
                "SELECT 1 FROM wallets WHERE profile_id=? AND lower(label)=lower(?) LIMIT 1",
                [profile.as_str(), label.as_str()],
                |_| Ok(true),
            )
            .optional()
            .map_err(|e| error(map_sqlite_error(e)))?
            .unwrap_or(false);
        if taken {
            return Err(AppError::new(
                "conflict",
                format!("A wallet named '{label}' already exists in this book"),
            )
            .with_hint("Choose another name for this exchange connection.")
            .with_details(dict([("wallet", s(label.as_str()))])));
        }
        let before = self.backends.clone();
        let mut merged = self.backends.clone();
        merged.merge(conn)?;
        let names = merged.names();
        let name = std::iter::once(kind.clone())
            .chain((2..).map(|n| format!("{kind}-{n}")))
            .find(|name| !names.contains(name))
            .expect("a free backend name");
        let mut fields = vec![
            ("name", s(name.as_str())),
            ("kind", s(kind.as_str())),
            ("url", s(url.as_str())),
            ("token", s(credentials.key.as_str())),
        ];
        if let Some(secret) = &credentials.secret {
            fields.push(("auth_header", s(secret.as_str())));
        }
        self.backends
            .execute(conn, "backends.create", &dict(fields), &SetupSources)?;
        self.backends.merge(conn)?;
        #[cfg(feature = "test-harness")]
        let identity = std::env::var("KASSIBER_RUST_TEST_WALLET_ID")
            .ok()
            .and_then(|id| {
                Some(kassiber_wallets::file_import::Identity {
                    ids: vec![id],
                    timestamp: std::env::var("KASSIBER_RUST_TEST_WALLET_TIME").ok()?,
                })
            });
        #[cfg(not(feature = "test-harness"))]
        let identity = None;
        let wallet =
            match desktop::create_wallet(conn, &profile, &label, &kind, &name, identity.as_ref()) {
                Ok(wallet) => wallet,
                Err(error) => {
                    // Leave no key behind for a connection that does not exist.
                    let _ = conn.execute("DELETE FROM backends WHERE name=?", [&name]);
                    self.backends = before;
                    return Err(error);
                }
            };
        Ok(dict([
            ("exchange", s(kind.as_str())),
            (
                "wallet",
                dict([
                    ("id", s(wallet.id.as_str())),
                    ("label", s(wallet.label.as_str())),
                    ("kind", s(kind.as_str())),
                ]),
            ),
            ("backend", self.backends.details(conn, &name)?),
        ]))
    }

    /// `ui.connections.exchange.sync`: `wallets sync-<kind> --backend B
    /// --wallet W` for the wallet's own backend, on the user's explicit
    /// request. Its result, or its error, is this session's last sync.
    fn exchange_sync(&mut self, args: &Value) -> Result<Value, AppError> {
        let reference = text(args, "wallet")?.ok_or_else(|| {
            AppError::new(
                "validation",
                "ui.connections.exchange.sync requires args.wallet",
            )
        })?;
        let profile = self.exchange_profile()?;
        let target = desktop::sync_target(self.conn()?, &profile, &reference)?;
        let Some(backend) = target.backend.clone() else {
            return Err(AppError::new(
                "config_error",
                format!(
                    "Exchange wallet '{}' names no API key backend",
                    target.label
                ),
            )
            .with_hint("Add the exchange again with a read-only API key."));
        };
        let result = self.import_exchange_api(&profile, &target.id, &backend, &target.kind);
        let at = kassiber_setup::Sources::now(&SetupSources, self.conn()?)?;
        let record = match &result {
            Ok(payload) => {
                let mut record = vec![("status", s("ok")), ("at", s(at))];
                for key in ["imported", "updated", "fetched", "skipped"] {
                    if let Some(value) = get(payload, key) {
                        record.push((key, value.clone()));
                    }
                }
                dict(record)
            }
            // The error exactly as the envelope reports it, already withheld.
            Err(error) => dict([
                ("status", s("error")),
                ("at", s(at)),
                (
                    "error",
                    get(&error.envelope(None), "error")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
            ]),
        };
        self.exchange_syncs.insert(target.id.clone(), record);
        result
    }
}
