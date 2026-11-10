//! Connections services on the open book: saved BTCPay configuration is read
//! here and only a kind/URL view reaches the shared services.
use super::*;
use kassiber_envelope::get;
use kassiber_wallets::btcpay::{self, SavedBackend};

pub(super) const COMMANDS: &[&str] = &[
    "btcpay.key-url",
    "ui.connections.btcpay.key_guide",
    "ui.connections.btcpay.test",
    "ui.btcpay.provenance.sync",
    "btcpay.provenance.sync",
    "ui.connections.btcpay.discover",
    "btcpay.inspect",
    "ui.connections.btcpay.create",
    "btcpay.setup",
    "wallets.attach-btcpay",
    "ui.connections.bullbitcoin_wallet.create",
    "wallets.attach-bullbitcoin-wallet",
    "wallets.sync-btcpay",
];

fn saved_view(name: &str, backend: &Value) -> SavedBackend {
    let field = |key: &str| {
        get(backend, key)
            .map(kassiber_envelope::py_str)
            .map(|v| kassiber_pytext::strip(&v).to_owned())
            .filter(|v| !v.is_empty())
    };
    SavedBackend {
        name: field("name").unwrap_or_else(|| name.to_owned()),
        kind: kassiber_pytext::lower(&field("kind").unwrap_or_default()),
        url: field("url"),
    }
}

impl RealBook {
    /// The open book with mutable backend configuration for actions that
    /// may save an inline BTCPay instance.
    fn with_btcpay_book<T>(
        &mut self,
        f: impl FnOnce(&mut btcpay::create::Book<'_>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let conn = self
            .book
            .as_ref()
            .map(StoredBook::connection)
            .ok_or_else(|| AppError::new("internal_error", "book is closed"))?;
        let book_id = setting(conn, "database_instance_id")?
            .filter(|id| !id.is_empty())
            .ok_or_else(|| AppError::new("validation", "Book identity is unavailable"))?;
        let mut book = btcpay::create::Book {
            conn,
            backends: &mut self.backends,
            sources: &SetupSources,
            gate: self.lightning_gate.clone(),
            version: env!("CARGO_PKG_VERSION"),
            book_id,
        };
        f(&mut book)
    }

    fn btcpay_env(&self) -> Result<btcpay::connect::Env<'_>, AppError> {
        let conn = self.conn()?;
        let book_id = setting(conn, "database_instance_id")?
            .filter(|id| !id.is_empty())
            .ok_or_else(|| AppError::new("validation", "Book identity is unavailable"))?;
        Ok(btcpay::connect::Env {
            conn,
            backends: &self.backends,
            gate: self.lightning_gate.clone(),
            version: env!("CARGO_PKG_VERSION"),
            book_id,
            progress: None,
        })
    }

    /// `ui.btcpay.provenance.sync` and `btcpay provenance sync`.
    fn provenance_sync(&mut self, operation: &str, args: &Value) -> Result<Value, AppError> {
        let desktop = operation.starts_with("ui.");
        let text_arg = |key: &str| match get(args, key) {
            Some(Value::Str(t)) if !t.is("") => Some(t.clone()),
            _ => None,
        };
        let (backend, store, page_size, include_payouts) = if desktop {
            let backend = text_arg("backend").ok_or_else(|| {
                AppError::new(
                    "validation",
                    "ui.btcpay.provenance.sync requires args.backend",
                )
            })?;
            let store = text_arg("store_id").ok_or_else(|| {
                AppError::new(
                    "validation",
                    "ui.btcpay.provenance.sync requires args.store_id",
                )
            })?;
            let page_size = match get(args, "page_size").filter(|v| kassiber_envelope::truthy(v)) {
                None => 100,
                Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or(i64::MAX),
                Some(Value::Bool(b)) => i64::from(*b),
                Some(Value::Float(f)) if f.is_finite() => f.trunc() as i64,
                Some(Value::Str(t)) => t
                    .as_str()
                    .and_then(|t| kassiber_pytext::parse_int(t).ok())
                    .and_then(|n| i64::try_from(n).ok())
                    .ok_or_else(|| AppError::new("internal_error", "invalid literal for int()"))?,
                Some(_) => {
                    return Err(AppError::new(
                        "internal_error",
                        "page_size must be an integer",
                    ))
                }
            };
            (backend, store, page_size, true)
        } else {
            let all = get(args, "sync_all").is_some_and(kassiber_envelope::truthy);
            let backend = text_arg("backend");
            let store = text_arg("store_id");
            if all {
                if backend.is_some() || store.is_some() {
                    return Err(AppError::new(
                        "validation",
                        "Use either --all or --backend/--store-id",
                    ));
                }
                let page_size = match get(args, "page_size") {
                    Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or(i64::MAX),
                    _ => 100,
                };
                let payouts = get(args, "include_payouts").is_none_or(kassiber_envelope::truthy);
                let args = args.clone();
                return self.with_btcpay_book(|book| {
                    btcpay::create::sync_all(book, &args, page_size, payouts)
                });
            }
            let (Some(backend), Some(store)) = (backend, store) else {
                return Err(AppError::new(
                    "validation",
                    "btcpay provenance sync needs --backend and --store-id, or --all",
                )
                .with_hint("Run `kassiber btcpay inspect --backend NAME` to list the stores a key can read."));
            };
            let page_size = match get(args, "page_size") {
                Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or(i64::MAX),
                _ => 100,
            };
            let payouts = get(args, "include_payouts").is_none_or(kassiber_envelope::truthy);
            (backend, store, page_size, payouts)
        };
        let conn = self.conn()?;
        let scope_args = if desktop {
            kassiber_envelope::dict::<&str>([])
        } else {
            args.clone()
        };
        let (workspace, profile) = kassiber_setup::references::scope(conn, &scope_args)?;
        let workspace_id = kassiber_envelope::py_str(get(&workspace, "id").unwrap_or(&Value::Null));
        let profile_id = kassiber_envelope::py_str(get(&profile, "id").unwrap_or(&Value::Null));
        let env = self.btcpay_env()?;
        btcpay::provenance::sync(
            &env,
            btcpay::provenance::Sync {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
                backend: &kassiber_chain::btcpay::url::from_py(&backend),
                store_id: &store,
                page_size,
                include_payouts,
                suggest: true,
                kind: operation,
                args,
                checkpoint: &Value::Null,
            },
        )
    }

    pub(super) fn connection_service(
        &mut self,
        operation: &str,
        args: &Value,
    ) -> Result<Value, AppError> {
        match operation {
            "btcpay.key-url" => {
                let conn = self.conn()?;
                btcpay::key_url(args, |name| {
                    let backend = self.backends.resolve_configured(conn, Some(name))?;
                    let text = |key: &str| backend[key].as_str().map(str::to_owned);
                    Ok(SavedBackend {
                        name: text("name").unwrap_or_default(),
                        kind: kassiber_pytext::lower(kassiber_pytext::strip(
                            &text("kind").unwrap_or_default(),
                        )),
                        url: text("url"),
                    })
                })
            }
            "ui.connections.btcpay.key_guide" => btcpay::key_guide(args, |name| {
                self.backends
                    .configured(name)
                    .filter(|v| matches!(v, Value::Dict(_)))
                    .map(|v| saved_view(name, v))
            }),
            // One explicit desktop action; its grant ends with the request.
            "ui.connections.btcpay.test" => btcpay::connect::test(&self.btcpay_env()?, args),
            "ui.connections.btcpay.discover" => {
                btcpay::connect::discover(&self.btcpay_env()?, args)
            }
            "btcpay.inspect" => btcpay::connect::inspect(&self.btcpay_env()?, args),
            "ui.connections.btcpay.create" => {
                self.with_btcpay_book(|book| btcpay::create::create(book, args))
            }
            "btcpay.setup" => self.with_btcpay_book(|book| btcpay::create::setup_cli(book, args)),
            "ui.connections.bullbitcoin_wallet.create" => {
                kassiber_wallets::bull_connect::create(self.conn()?, args)
            }
            "wallets.attach-bullbitcoin-wallet" => {
                kassiber_wallets::bull_connect::attach(self.conn()?, args)
            }
            "wallets.sync-btcpay" => {
                let env = self.btcpay_env()?;
                kassiber_wallets::sync::btcpay::sync_into_wallet(
                    env.conn,
                    &kassiber_wallets::sync::btcpay::Context {
                        backends: env.backends,
                        gate: &env.gate,
                        version: env.version,
                        book_id: &env.book_id,
                        progress: None,
                    },
                    args,
                )
            }
            "wallets.attach-btcpay" => {
                self.with_btcpay_book(|book| btcpay::create::attach(book, args))
            }
            "ui.btcpay.provenance.sync" | "btcpay.provenance.sync" => {
                self.provenance_sync(operation, args)
            }
            _ => Err(AppError::new(
                "not_implemented",
                format!("Connections operation '{operation}' is not native"),
            )),
        }
    }
}
