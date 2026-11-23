//! Desktop exchange connections (Rust only): the explicit key check, the
//! provider wallet a new connection gets, and the wallets a sync may target.
//!
//! The check reads with the same signed requests, transport and credential
//! withholding as `wallets sync-<kind>`, under a consent the caller mints for
//! exactly this route. The wallet is the one the CLI sync creates on its first
//! run, so a desktop connection stores the CLI's bytes.
use super::*;

/// One check has two minutes; it reads a page or two at most.
const PROBE_BUDGET: Duration = Duration::from_secs(120);

/// `ui.connections.exchange.test`: the fewest signed reads that prove the
/// key reads what the sync needs, and, where the exchange tells, that it
/// cannot trade or withdraw. Nothing is written.
pub fn probe(
    conn: &Connection,
    backend: &Json,
    gate: &Gate,
    version: &str,
    consent: &mut dyn FnMut(&Route) -> Option<Consent>,
) -> Result<Py, AppError> {
    let backend = to_py(backend)?;
    let (name, kind) = backend_kind(&backend, "")?;
    let exchange = exchange(&kind)?;
    let prepared = prepare(&backend, exchange, &name)?;
    let binding: [u8; 32] = Sha256::digest(
        serde_json::to_vec(&json!([
            "ui.connections.exchange.test",
            book_id(conn)?,
            kind,
            prepared.transport
        ]))
        .expect("JSON binding"),
    )
    .into();
    let transport = Backend::new(&prepared.transport, false)?.with_action_binding(binding);
    let route = Route {
        request: transport.request().clone(),
    };
    let consent = consent(&route);
    let (key, secret, base) = (&prepared.key, &prepared.secret, &prepared.base);
    let client = Client {
        http: HttpClient::new(transport, gate.clone(), version).withholding(&[key, secret]),
        gate,
        request: &route.request,
        consent: consent.as_ref(),
        deadline: Instant::now() + PROBE_BUDGET,
        base,
        key,
        secret,
        source: exchange.provider,
    };
    // An echoed request credential (a signature, say) leaves every error.
    let read_only = (exchange.probe)(&client).map_err(|error| leaving(error, &client))?;
    Ok(dict([
        ("exchange", s(kind.as_str())),
        ("ok", Py::Bool(true)),
        // Kraken, Coinbase and Bitvavo do not say what else a key may do.
        ("read_only", read_only.map_or(Py::Null, Py::Bool)),
    ]))
}

/// The provider wallet for a new connection: `label`, `kind` and
/// `{"backend": name}`, exactly as `wallets sync-<kind>` creates it.
pub fn create_wallet(
    conn: &Connection,
    profile: &str,
    label: &str,
    kind: &str,
    backend: &str,
    identity: Option<&Identity>,
) -> Result<crate::Wallet, AppError> {
    if !KINDS.contains(&kind) {
        return Err(AppError::new(
            "validation",
            format!("Unsupported exchange '{kind}'"),
        ));
    }
    let mut ids = Ids { identity, at: 0 };
    super::create_wallet(conn, profile, label, kind, backend, &mut ids)
}

/// An exchange wallet in a profile and the backend its sync reads.
pub struct Target {
    pub id: String,
    pub label: String,
    pub kind: String,
    /// `config_json.backend`, when the wallet names one.
    pub backend: Option<String>,
}

fn target(id: String, label: String, kind: String, config: Option<String>) -> Target {
    let backend = config
        .and_then(|config| serde_json::from_str::<Json>(&config).ok())
        .and_then(|config| {
            config
                .get("backend")
                .and_then(Json::as_str)
                .map(str::to_owned)
        })
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    Target {
        id,
        label,
        kind,
        backend,
    }
}

/// The wallet `reference` names (id first, then label) when it is an
/// exchange API wallet.
pub fn sync_target(conn: &Connection, profile: &str, reference: &str) -> Result<Target, AppError> {
    let wallet = resolve(conn, profile, reference)?;
    let (kind, config): (String, Option<String>) = conn
        .query_row(
            "SELECT lower(trim(kind)),config_json FROM wallets WHERE id=?",
            [&wallet.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(database_error)?;
    if !KINDS.contains(&kind.as_str()) {
        return Err(AppError::new(
            "validation",
            format!(
                "Wallet '{}' is not an exchange API connection",
                wallet.label
            ),
        )
        .with_hint("Use Refresh for wallets that sync from a chain backend or a file."));
    }
    let target = target(wallet.id, wallet.label, kind, config);
    // A wallet of an exchange kind that imports a file (a CSV export) is an
    // API connection only once a backend is attached to it.
    if target.backend.is_none() {
        return Err(AppError::new(
            "validation",
            format!("Wallet '{}' has no exchange API key attached", target.label),
        )
        .with_hint(
            "Refresh it from its file, or attach a read-only API key to it in Add connection.",
        ));
    }
    Ok(target)
}

/// Every exchange API wallet in the profile (an exchange kind with a backend
/// attached), by label. A file-only wallet of that kind is not one.
pub fn targets(conn: &Connection, profile: &str) -> Result<Vec<Target>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT id,label,lower(trim(kind)),config_json FROM wallets \
             WHERE profile_id=? ORDER BY label ASC,id ASC",
        )
        .map_err(database_error)?;
    let rows = stmt
        .query_map([profile], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?, r.get(3)?))
        })
        .map_err(database_error)?
        .collect::<kassiber_store::rusqlite::Result<Vec<_>>>()
        .map_err(database_error)?;
    Ok(rows
        .into_iter()
        .filter(|(_, _, kind, _)| KINDS.contains(&kind.as_str()))
        .map(|(id, label, kind, config)| target(id, label, kind, config))
        .filter(|target| target.backend.is_some())
        .collect())
}

/// The provider's name, as the sync labels its wallet; empty for a kind
/// that is no exchange.
pub fn provider_name(kind: &str) -> &'static str {
    contract::registry::by_kind(kind).map_or("", |exchange| exchange.provider)
}

/// The provider's public API, as `wallets sync-<kind>` defaults to it;
/// empty for a kind that is no exchange.
pub fn default_api_url(kind: &str) -> &'static str {
    contract::registry::by_kind(kind).map_or("", |exchange| exchange.default_url)
}

/// Attaches backend `backend` to an existing wallet of exchange `kind` that
/// has none yet, such as a CSV-imported Bitvavo wallet, keeping its file
/// source. The sync then imports into it and deduplicates against the file.
pub fn attach_backend(
    conn: &Connection,
    profile: &str,
    reference: &str,
    kind: &str,
    backend: &str,
) -> Result<crate::Wallet, AppError> {
    let wallet = resolve(conn, profile, reference)?;
    let (stored_kind, config): (String, Option<String>) = conn
        .query_row(
            "SELECT lower(trim(kind)),config_json FROM wallets WHERE id=?",
            [&wallet.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(database_error)?;
    if !KINDS.contains(&kind) || stored_kind != kind {
        return Err(AppError::new(
            "validation",
            format!(
                "Wallet '{}' is not a {} wallet",
                wallet.label,
                provider_name(kind)
            ),
        )
        .with_hint("Attach an exchange key only to a wallet of the same exchange."));
    }
    let existing = target(wallet.id.clone(), wallet.label.clone(), stored_kind, config);
    if let Some(attached) = existing.backend {
        return Err(AppError::new(
            "conflict",
            format!(
                "Wallet '{}' already syncs through backend '{attached}'",
                wallet.label
            ),
        ));
    }
    lifecycle::execute(
        conn,
        lifecycle::Operation::Update {
            profile_id: profile.into(),
            wallet: wallet.id.clone(),
            label: None,
            account: None,
            config: json!({"backend": backend}),
            clear: vec![],
        },
    )?;
    resolve(conn, profile, &wallet.id)
}
