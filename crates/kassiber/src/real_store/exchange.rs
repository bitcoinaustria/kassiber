//! Desktop exchange API connections (Rust only; docs/reference/exchange-desktop.md).
//!
//! `status` and `create` are local. `test` and `sync` are the only kinds that
//! contact an exchange, each for one explicit user action under a consent
//! minted for exactly that route. `create` stores the backend and wallet rows
//! `backends create` and `wallets sync-<kind>` store, and `sync` runs that
//! very command, so the book holds the CLI's bytes. Every sync, CLI or desktop,
//! leaves its outcome on the wallet (`config_json.exchange_sync`), which
//! `status` reads, so it survives a restart.
use super::*;
use kassiber_envelope::get;
use kassiber_wallets::file_import::assets::Scope;
use kassiber_wallets::file_import::exchange_api::{self, desktop};

pub(super) const COMMANDS: &[&str] = &[
    "ui.connections.exchange.status",
    "ui.connections.exchange.test",
    "ui.connections.exchange.preview",
    "ui.connections.exchange.create",
    "ui.connections.exchange.sync",
];

/// The fields each kind accepts; anything else is refused before any work.
fn allowed(operation: &str) -> &'static [&'static str] {
    match operation {
        "ui.connections.exchange.status" => &["wallet"],
        "ui.connections.exchange.test" => &["backend", "exchange", "api_key", "api_secret", "url"],
        "ui.connections.exchange.preview" => &["exchange", "api_key", "api_secret", "url"],
        "ui.connections.exchange.create" => &[
            "exchange",
            "label",
            "wallet",
            "api_key",
            "api_secret",
            "url",
            "asset_scope",
            "preview_token",
        ],
        _ => &["wallet", "changed_rows"],
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
                .with_hint("One of kraken, coinbase, binance, bitpanda, bitvavo, bitstamp, bitfinex or bitcoinde.")
        })?;
    if !exchange_api::KINDS.contains(&kind.as_str()) {
        return Err(
            AppError::new("validation", format!("Unsupported exchange '{kind}'"))
                .with_hint("One of kraken, coinbase, binance, bitpanda, bitvavo, bitstamp, bitfinex or bitcoinde."),
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

/// The `asset_scope` a wallet's config stores, if it stores one.
fn stored_scope(conn: &Connection, wallet_id: &str) -> Result<Option<Scope>, AppError> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id=?",
            [wallet_id],
            |r| r.get(0),
        )
        .map_err(|e| error(map_sqlite_error(e)))?;
    let config: serde_json::Value = stored
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    match config.get(Scope::KEY) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(_) => Scope::of_wallet(conn, wallet_id).map(Some),
    }
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
            "ui.connections.exchange.preview" => self.exchange_preview(args),
            "ui.connections.exchange.create" => self.exchange_create_after_preview(args),
            _ => self.exchange_sync(args),
        }
    }

    /// Every exchange wallet in the active profile, its backend's presence
    /// and its last stored sync. Local reads only.
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
            let scope = stored_scope(conn, &target.id)?;
            let mut row = vec![
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
                ("last_sync", self.last_sync(conn, &target.id)?),
            ];
            // Only a wallet that stores a scope names it (absent means
            // `bitcoin`), so books that never set one answer as before.
            if let Some(scope) = scope {
                row.push(("asset_scope", s(scope.as_str())));
            }
            rows.push(dict(row));
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
            crate::version::current(),
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

    /// `ui.connections.exchange.preview`: the history a pasted key reads,
    /// counted by asset before the connection exists (one explicit request
    /// of the Add connection dialog, under a consent for exactly this route).
    fn exchange_preview(&mut self, args: &Value) -> Result<Value, AppError> {
        let conn = self.conn()?;
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
        let mut minted = None;
        let result = desktop::preview(
            conn,
            &backend,
            &self.lightning_gate,
            crate::version::current(),
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
    /// With `wallet`, the backend is attached to that existing wallet of the
    /// same exchange instead (a CSV-imported one), which keeps its file source.
    fn exchange_create(&mut self, args: &Value) -> Result<Value, AppError> {
        let kind = exchange(args)?;
        let credentials = credentials(&kind, args)?;
        let url = url(&kind, args)?;
        let attach = text(args, "wallet")?;
        if attach.is_some() && text(args, "label")?.is_some() {
            return Err(AppError::new(
                "validation",
                "Name a new connection or attach to an existing wallet, not both",
            ));
        }
        // The scope chosen before the first sync; an existing wallet keeps
        // its own (`ui.wallets.update` changes it).
        let scope = text(args, "asset_scope")?
            .map(|value| Scope::parse(&value))
            .transpose()?;
        if attach.is_some() && scope.is_some() {
            return Err(AppError::new(
                "validation",
                "An attached wallet keeps its asset scope",
            ));
        }
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
        let taken: Option<(String, String, String, Option<String>)> = conn
            .query_row(
                "SELECT id,lower(trim(kind)),label,config_json FROM wallets WHERE profile_id=? AND lower(label)=lower(?) LIMIT 1",
                [profile.as_str(), label.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(|e| error(map_sqlite_error(e)))?;
        if let (None, Some((id, existing_kind, existing_label, config))) = (&attach, &taken) {
            let mut details = vec![("wallet", s(label.as_str()))];
            // The same exchange's CSV wallet can take the key instead.
            if *existing_kind == kind && backend_of(config.as_deref()).is_none() {
                details.push(("attachable_wallet_id", s(id.as_str())));
                details.push(("attachable_wallet", s(existing_label.as_str())));
            }
            return Err(AppError::new(
                "conflict",
                format!("A wallet named '{label}' already exists in this book"),
            )
            .with_hint(
                "Choose another name, or add the key to the existing wallet so it keeps its CSV history.",
            )
            .with_details(dict(details)));
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
        let created = match &attach {
            Some(reference) => desktop::attach_backend(conn, &profile, reference, &kind, &name),
            None => desktop::create_scoped_wallet(
                conn,
                &profile,
                &label,
                &kind,
                &name,
                scope,
                identity.as_ref(),
            ),
        };
        let wallet = match created {
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
            ("attached", Value::Bool(attach.is_some())),
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
    /// request. Its result, or its error, is stored as the wallet's last sync.
    fn exchange_sync(&mut self, args: &Value) -> Result<Value, AppError> {
        let reference = text(args, "wallet")?.ok_or_else(|| {
            AppError::new(
                "validation",
                "ui.connections.exchange.sync requires args.wallet",
            )
        })?;
        let changed_rows = text(args, "changed_rows")?
            .map(|value| kassiber_wallets::file_import::ChangedRows::parse(&value))
            .transpose()?;
        self.exchange_sync_wallet(&reference, changed_rows, None)
    }

    /// `ui.connections.exchange.create` with `preview_token`: the new
    /// connection and its first sync of exactly the previewed history. A
    /// history that changed since leaves no connection behind
    /// (`import_preview_stale`), so the dialog asks again; another sync
    /// failure keeps the connection, as a separate Sync now would.
    fn exchange_create_after_preview(&mut self, args: &Value) -> Result<Value, AppError> {
        let Some(token) = text(args, "preview_token")? else {
            return self.exchange_create(args);
        };
        if text(args, "wallet")?.is_some() {
            return Err(AppError::new(
                "validation",
                "preview_token is for a new connection, not an attached wallet",
            ));
        }
        let before = self.backends.clone();
        let created = self.exchange_create(args)?;
        let field = |path: &[&str]| {
            path.iter()
                .try_fold(&created, |v, k| get(v, k))
                .map(kassiber_envelope::py_str)
                .unwrap_or_default()
        };
        let (wallet, backend) = (field(&["wallet", "id"]), field(&["backend", "name"]));
        match self.exchange_sync_wallet(&wallet, None, Some(&token)) {
            Ok(sync) => {
                let mut created = created;
                kassiber_envelope::set(&mut created, "sync", sync);
                Ok(created)
            }
            Err(error) if error.code == kassiber_wallets::file_import::PREVIEW_STALE => {
                let conn = self.conn()?;
                conn.execute("DELETE FROM wallets WHERE id=?", [&wallet])
                    .map_err(|e| crate::real_store::error(map_sqlite_error(e)))?;
                conn.execute("DELETE FROM backends WHERE name=?", [&backend])
                    .map_err(|e| crate::real_store::error(map_sqlite_error(e)))?;
                self.backends = before;
                self.exchange_syncs.remove(&wallet);
                Err(error)
            }
            Err(error) => {
                let mut created = created;
                kassiber_envelope::set(
                    &mut created,
                    "sync_error",
                    get(&error.envelope(None), "error")
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                Ok(created)
            }
        }
    }

    /// One exchange wallet's sync on the user's request; its result, or its
    /// error, is this session's last sync.
    fn exchange_sync_wallet(
        &mut self,
        reference: &str,
        changed_rows: Option<kassiber_wallets::file_import::ChangedRows>,
        preview_token: Option<&str>,
    ) -> Result<Value, AppError> {
        let profile = self.exchange_profile()?;
        let target = desktop::sync_target(self.conn()?, &profile, reference)?;
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
        let result = self.exchange_import(
            &profile,
            &target.id,
            &backend,
            &target.kind,
            changed_rows,
            preview_token,
        );
        // `exchange_import` stores the outcome's counts on the wallet; what
        // the card shows besides them stays in this open book's memory, for
        // that stored sync only (`last_sync`).
        let at = stored_sync(self.conn()?, &target.id)?
            .and_then(|record| get(&record, "at").cloned())
            .unwrap_or(Value::Null);
        let mut extras = vec![("at", at)];
        match &result {
            Ok(payload) => {
                for key in CARD_EXTRAS {
                    if let Some(value) = get(payload, key) {
                        extras.push((key, value.clone()));
                    }
                }
            }
            Err(error) => extras.push(("error", kept_error(error))),
        }
        self.exchange_syncs.insert(target.id.clone(), dict(extras));
        result
    }
}

/// What the desktop sync card shows of a sync besides its stored counts.
const CARD_EXTRAS: [&str; 5] = [
    "provider_balance",
    "assets",
    "swap_pairs",
    "corrections_ignored",
    "changed_rows",
];

impl RealBook {
    /// The wallet's stored last sync, with the card's extras when this open
    /// book ran that very sync (the same `at`).
    fn last_sync(&self, conn: &Connection, wallet: &str) -> Result<Value, AppError> {
        let Some(mut record) = stored_sync(conn, wallet)? else {
            return Ok(Value::Null);
        };
        if let Some(extras) = self.exchange_syncs.get(wallet) {
            merge_extras(&mut record, extras);
        }
        Ok(record)
    }
}

/// What this open book keeps of a failed sync for its card. The stored
/// record keeps only the error's class; the card's answer to a changed-rows
/// refusal (Skip / Import as new) needs its code and counts. Provider text
/// is not kept.
fn kept_error(error: &AppError) -> Value {
    let mut kept = vec![("code", s(error.code.as_str()))];
    if let Some(details) = get(&error.envelope(None), "error")
        .and_then(|e| get(e, "details"))
        .filter(|_| error.code == "import_changed_rows")
    {
        kept.push(("details", details.clone()));
    }
    dict(kept)
}

/// The card's extras over the stored record, when they belong to that very
/// sync (the same `at`).
fn merge_extras(record: &mut Value, extras: &Value) {
    if get(extras, "at").is_none() || get(extras, "at") != get(record, "at") {
        return;
    }
    for key in CARD_EXTRAS {
        if let Some(value) = get(extras, key) {
            kassiber_envelope::set(record, key, value.clone());
        }
    }
    if let (Some(kept), Some(mut error)) = (get(extras, "error"), get(record, "error").cloned()) {
        for key in ["code", "details"] {
            if let Some(value) = get(kept, key) {
                kassiber_envelope::set(&mut error, key, value.clone());
            }
        }
        kassiber_envelope::set(record, "error", error);
    }
}

/// The persisted last sync of an exchange wallet (`None` before the first).
fn stored_sync(conn: &Connection, wallet: &str) -> Result<Option<Value>, AppError> {
    let config: Option<String> = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id=?",
            [wallet],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| error(map_sqlite_error(e)))?
        .flatten();
    Ok(kassiber_wallets::coverage::exchange_sync_record(
        config.as_deref(),
    ))
}

fn backend_of(config: Option<&str>) -> Option<String> {
    let config: serde_json::Value = serde_json::from_str(config?).ok()?;
    config
        .get("backend")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Codex 4683024207: a changed-rows refusal keeps its code and counts on
    /// the card for the sync that stored it, so Skip and Import as new are
    /// offered; the stored record's class and message stay.
    #[test]
    fn a_changed_rows_refusal_keeps_its_answerable_details_on_the_card() {
        let refusal =
            AppError::new("import_changed_rows", "Bitvavo changed 1 row").with_details(dict([
                ("reidentified", Value::Int(2.into())),
                ("corrected", Value::Int(1.into())),
            ]));
        let stored = || {
            dict([
                ("status", s("error")),
                ("at", s("2026-11-24T10:00:00Z")),
                (
                    "error",
                    dict([
                        ("code", s("exchange_error")),
                        ("class", s("other")),
                        ("message", s("The sync failed.")),
                    ]),
                ),
            ])
        };
        let extras = dict([
            ("at", s("2026-11-24T10:00:00Z")),
            ("error", kept_error(&refusal)),
        ]);
        let mut record = stored();
        merge_extras(&mut record, &extras);
        let error = get(&record, "error").unwrap();
        assert_eq!(get(error, "code"), Some(&s("import_changed_rows")));
        assert_eq!(get(error, "class"), Some(&s("other")));
        assert_eq!(get(error, "message"), Some(&s("The sync failed.")));
        let details = get(error, "details").unwrap();
        assert_eq!(get(details, "reidentified"), Some(&Value::Int(2.into())));
        assert_eq!(get(details, "corrected"), Some(&Value::Int(1.into())));
        // Another sync stored since: its record is shown as stored.
        let mut later = stored();
        kassiber_envelope::set(&mut later, "at", s("2026-11-24T11:00:00Z"));
        merge_extras(&mut later, &extras);
        assert_eq!(
            get(get(&later, "error").unwrap(), "code"),
            Some(&s("exchange_error"))
        );
        // Other failures keep only their code, never the provider's text.
        let other = kept_error(&AppError::new("auth_error", "provider said: bad key K"));
        assert!(get(&other, "details").is_none());
        assert!(get(&other, "message").is_none());
    }
}
