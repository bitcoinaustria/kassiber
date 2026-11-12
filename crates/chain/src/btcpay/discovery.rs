//! `kassiber/btcpay/discovery.py`: the explicit setup inspection of one
//! BTCPay connection. Stores are read in order (Python reads them on a
//! thread pool and keeps the same result order). The receive addresses of
//! the wallet preview stay in `Inspection::previews` for local matching and
//! never enter the public payload.
use super::client::{scrub_api_key, HttpFailure};
use super::fetch::Reads;
use super::normalize::{self, field, text};
use super::payment_methods as methods;
use super::permissions::{
    capability_permissions, permission_label, server_version, KeyGrant, CAPABILITY_PERMISSIONS,
};
use super::url::{from_py, is_loopback_url, is_onion_url};
use kassiber_envelope::{dict, get, s, set, truthy, AppError};
use kassiber_pyjson::{PyStr, Value};

type Result<T> = std::result::Result<T, AppError>;

const PREVIEW_ADDRESS_COUNT: i64 = 5;
const DEFAULT_TOLERATE: [u16; 4] = [401, 403, 404, 405];

/// `store_id → payment_method_id → addresses`, in discovery order.
pub type Previews = Vec<(String, Vec<(String, Vec<String>)>)>;

pub struct Inspection {
    pub public: Value,
    pub previews: Previews,
    pub grant: KeyGrant,
}

fn warning(code: &str, message: String, extra: Vec<(&str, Value)>) -> Value {
    let mut fields = vec![("code", s(code))];
    fields.extend(extra);
    fields.push(("message", s(message)));
    dict(fields)
}

fn major_version(version: &Option<String>) -> Option<i64> {
    let text = version.as_deref().unwrap_or("");
    let head = text.split('.').next().unwrap_or("");
    kassiber_pytext::parse_int(head)
        .ok()
        .and_then(|n| i64::try_from(n).ok())
}

fn store_entry(raw: &Value) -> Result<Value> {
    let id = normalize::either(&[field(raw, "id"), field(raw, "storeId")]);
    if !truthy(id) {
        return Err(AppError::new(
            "protocol_error",
            "BTCPay store record is missing 'id'",
        ));
    }
    let id = text(id);
    let name = normalize::either(&[field(raw, "name"), field(raw, "label")]);
    let name = if truthy(name) { text(name) } else { id.clone() };
    let archived = match get(raw, "archived") {
        Some(v) => truthy(v),
        None => false,
    };
    Ok(dict([
        ("id", Value::Str(id)),
        ("name", Value::Str(name)),
        ("default_currency", field(raw, "defaultCurrency").clone()),
        ("archived", Value::Bool(archived)),
    ]))
}

fn fetch_preview(
    reads: Reads<'_>,
    store_id: &str,
    method: &Value,
    major: Option<i64>,
) -> Result<(Vec<String>, Option<HttpFailure>)> {
    let store = super::client::quote(store_id);
    let id = from_py(&text(field(method, "payment_method_id")));
    let mut paths = vec![format!(
        "/api/v1/stores/{store}/payment-methods/{}/wallet/preview",
        super::client::quote(&id)
    )];
    if major.is_some_and(|m| m < 2) {
        let currency = from_py(&text(field(method, "currency")));
        paths.push(format!(
            "/api/v1/stores/{store}/payment-methods/onchain/{}/preview",
            super::client::quote(&currency)
        ));
    }
    let mut last = None;
    for path in paths {
        let (payload, error) = reads.optional(
            &path,
            &[
                ("offset", "0".into()),
                ("count", PREVIEW_ADDRESS_COUNT.to_string()),
            ],
            &DEFAULT_TOLERATE,
        )?;
        match error {
            None => {
                let mut addresses = Vec::new();
                if let Value::List(rows) = field(&payload, "addresses") {
                    for row in rows {
                        let address = field(row, "address");
                        if matches!(row, Value::Dict(_)) && truthy(address) {
                            addresses.push(from_py(&text(address)));
                        }
                    }
                }
                return Ok((addresses, None));
            }
            Some(error) => {
                let status = error.status;
                last = Some(error);
                if status != 404 {
                    break;
                }
            }
        }
    }
    Ok((Vec::new(), last))
}

fn error_label(error: &HttpFailure) -> Value {
    if error.status == 403 {
        s("missing_permission")
    } else {
        s(format!("http_{}", error.status))
    }
}

struct StoreResult {
    store: Value,
    methods: Vec<Value>,
    previews: Vec<(String, Vec<String>)>,
    warnings: Vec<Value>,
}

fn inspect_store(
    reads: Reads<'_>,
    entry: Value,
    grant: &KeyGrant,
    major: Option<i64>,
    version: Option<(i64, i64)>,
) -> Result<StoreResult> {
    let store_id = from_py(&text(field(&entry, "id")));
    let mut capabilities = if grant.known {
        grant.capabilities(Some(&store_id), version)
    } else {
        dict(
            CAPABILITY_PERMISSIONS
                .iter()
                .map(|(c, _)| (*c, Value::Null)),
        )
    };
    let mut missing: Vec<&str> = capability_permissions(version)[0]
        .iter()
        .filter(|(c, _)| *c != "wallet_history" && field(&capabilities, c) == &Value::Bool(false))
        .map(|(_, p)| *p)
        .collect();
    missing.sort();
    missing.dedup();
    let mut store = entry.clone();
    set(&mut store, "capabilities", capabilities.clone());
    set(
        &mut store,
        "missing_permissions",
        Value::List(missing.into_iter().map(s).collect()),
    );
    set(&mut store, "payment_methods_error", Value::Null);
    let mut result = StoreResult {
        store,
        methods: Vec::new(),
        previews: Vec::new(),
        warnings: Vec::new(),
    };
    if field(&capabilities, "store_catalog") == &Value::Bool(false) {
        set(
            &mut result.store,
            "payment_methods_error",
            s("missing_permission"),
        );
        result.warnings.push(warning(
            "store_catalog_unavailable",
            format!(
                "The key cannot list payment methods for {}. Add '{}' so Kassiber can tell which wallets this store uses.",
                from_py(&text(field(&entry, "name"))),
                permission_label(super::permissions::VIEW_STORE_SETTINGS)
            ),
            vec![("store_id", Value::Str(text(field(&entry, "id"))))],
        ));
        return Ok(result);
    }
    let (raw_methods, error) = reads.optional(
        &format!(
            "/api/v1/stores/{}/payment-methods",
            super::client::quote(&store_id)
        ),
        &[("onlyEnabled", "true".into())],
        &DEFAULT_TOLERATE,
    )?;
    if let Some(error) = error {
        set(
            &mut result.store,
            "payment_methods_error",
            error_label(&error),
        );
        if error.status == 403 && !grant.known {
            set(&mut capabilities, "store_catalog", Value::Bool(false));
            set(&mut capabilities, "wallet_preview", Value::Bool(false));
            set(&mut result.store, "capabilities", capabilities);
        }
        return Ok(result);
    }
    let store_py = PyStr::from(store_id.as_str());
    for raw in normalize::payment_method_rows(&raw_methods) {
        let Some(mut method) = normalize::normalize_payment_method(&store_py, &raw) else {
            continue;
        };
        let history = if truthy(field(&method, "sync_supported")) {
            field(&capabilities, "wallet_history").clone()
        } else {
            Value::Bool(false)
        };
        set(&mut method, "wallet_history_available", history);
        set(&mut method, "wallet_fingerprint", Value::Null);
        set(&mut method, "network", Value::Null);
        set(&mut method, "preview_error", Value::Null);
        if field(&method, "payment_method_id") == &s(methods::BARK_PAYMENT_METHOD_ID) {
            set(&mut method, "wallet_history_error", Value::Null);
            if field(&method, "wallet_history_available") != &Value::Bool(false)
                && truthy(field(&method, "enabled"))
            {
                // `_bark_wallet_served`: plugin 0.10 and later serve wallet history.
                let (available, error) =
                    match super::fetch::bark_wallet_history_status(reads, &store_id)? {
                        None => (Value::Bool(true), Value::Null),
                        Some("unavailable") => (Value::Null, s("unavailable")),
                        Some(reason) => (Value::Bool(false), s(reason)),
                    };
                set(&mut method, "wallet_history_available", available);
                set(&mut method, "wallet_history_error", error);
            }
        }
        let previewable = field(&method, "rail") == &s(methods::RAIL_ONCHAIN)
            && truthy(field(&method, "bitcoin_asset"))
            && truthy(field(&method, "enabled"));
        if previewable && field(&capabilities, "wallet_preview") == &Value::Bool(false) {
            // The key cannot read the preview on this server (BTCPay 2.4 needs
            // `canviewwallet`); say so instead of asking.
            set(&mut method, "preview_error", s("missing_permission"));
        } else if previewable {
            let (addresses, preview_error) = fetch_preview(reads, &store_id, &method, major)?;
            if !addresses.is_empty() {
                let id = from_py(&text(field(&method, "payment_method_id")));
                set(
                    &mut method,
                    "wallet_fingerprint",
                    normalize::wallet_fingerprint(&addresses).map_or(Value::Null, s),
                );
                let (chain, network) = normalize::network_from_address(&addresses[0]);
                if chain.map(s).unwrap_or(Value::Null) == *field(&method, "chain") {
                    set(&mut method, "network", network.map_or(Value::Null, s));
                }
                if let Some(slot) = result.previews.iter_mut().find(|(k, _)| *k == id) {
                    slot.1 = addresses;
                } else {
                    result.previews.push((id, addresses));
                }
            } else if let Some(error) = preview_error {
                set(&mut method, "preview_error", error_label(&error));
            }
        }
        result.methods.push(method);
    }
    Ok(result)
}

/// `inspect_btcpay_connection(client)`.
pub fn inspect(reads: Reads<'_>) -> Result<Inspection> {
    let mut warnings = Vec::new();
    let base = reads.client.base_url().to_owned();
    let (info, _) = reads.optional("/api/v1/server/info", &[], &[403, 404, 405])?;
    let tor = is_onion_url(&base);
    let loopback = is_loopback_url(&base);
    let https = base.starts_with("https://");
    let mut version = None;
    let mut fully_synced = Value::Null;
    let mut supported = Vec::new();
    if matches!(info, Value::Dict(_)) {
        let raw = field(&info, "version");
        version = (truthy(raw))
            .then(|| from_py(&text(raw)))
            .filter(|v| !v.is_empty());
        fully_synced = field(&info, "fullySynched").clone();
        if let Value::List(items) = normalize::either(&[
            field(&info, "supportedPaymentMethods"),
            &Value::List(Vec::new()),
        ]) {
            for item in items.iter().filter(|v| truthy(v)) {
                supported.push(s(methods::canonical(&from_py(&text(item)))));
            }
        }
        if fully_synced == Value::Bool(false) {
            warnings.push(warning(
                "server_not_synced",
                "BTCPay Server is still syncing its node; recent payments may be missing.".into(),
                vec![],
            ));
        }
    }
    let major = major_version(&version);
    let server = dict([
        ("version", version.clone().map_or(Value::Null, s)),
        (
            "major_version",
            major.map_or(Value::Null, |m| Value::Int(m.into())),
        ),
        ("fully_synced", fully_synced),
        ("supported_payment_methods", Value::List(supported)),
        ("tor", Value::Bool(tor)),
        ("loopback", Value::Bool(loopback)),
        ("transport", s(if https { "https" } else { "http" })),
    ]);
    if !https && !tor && !loopback {
        warnings.push(warning(
            "unencrypted_transport",
            "This connection uses http://. The API key and store data travel unencrypted; use https:// or an onion address.".into(),
            vec![],
        ));
    }
    let (key_payload, key_error) =
        reads.optional("/api/v1/api-keys/current", &[], &[403, 404, 405])?;
    let scrubbed = if key_error.is_none() {
        scrub_api_key(&key_payload)
    } else {
        None
    };
    let grant = KeyGrant::parse(scrubbed.as_ref().map(|(_, p)| p.as_slice()));
    let mut api_key = vec![(
        "label".to_owned(),
        scrubbed
            .as_ref()
            .map_or(Value::Null, |(label, _)| label.clone()),
    )];
    let wallet_version = server_version(field(&server, "version"));
    if let Value::Dict(pairs) = grant.summary(wallet_version) {
        for (key, value) in pairs {
            if let kassiber_pyjson::Key::Str(key) = key {
                api_key.push((from_py(&key), value));
            }
        }
    }
    let api_key = dict(api_key);
    match field(&api_key, "risk").clone() {
        r if r == s("server_admin") => warnings.push(warning("key_server_admin", "This key has server-wide admin rights. Kassiber only reads store data; create a narrower key and revoke this one.".into(), vec![])),
        r if r == s("can_modify_store") => warnings.push(warning("key_can_modify_store", "This key can modify the store, including the wallet customers pay into. Kassiber only needs that for BTCPay's own wallet history.".into(), vec![])),
        r if r == s("can_write") => warnings.push(warning("key_can_write", "This key can create or change data (for example invoices or payouts). Kassiber only reads; a read-only key is safer.".into(), vec![])),
        _ => {}
    }
    let (raw_stores, stores_error) = reads.optional("/api/v1/stores", &[], &[403, 404])?;
    let mut stores = Vec::new();
    match (&stores_error, &raw_stores) {
        (None, Value::List(items)) => {
            for raw in items.iter().filter(|v| matches!(v, Value::Dict(_))) {
                let entry = store_entry(raw)?;
                if truthy(field(&entry, "archived")) {
                    continue;
                }
                stores.push(entry);
            }
        }
        (Some(error), _) if error.status == 404 => {
            return Err(AppError::new(
                "not_found",
                "This URL does not look like a BTCPay Server (the store list returned HTTP 404)",
            )
            .with_hint(
                "Check the server URL; enter the address you open BTCPay with, without /api/v1.",
            ));
        }
        _ => {}
    }
    if stores.is_empty() && grant.known && !grant.stores.is_empty() {
        for id in grant.stores.keys() {
            stores.push(dict([
                ("id", s(id.as_str())),
                ("name", s(id.as_str())),
                ("default_currency", Value::Null),
                ("archived", Value::Bool(false)),
            ]));
        }
    }
    if stores.is_empty() {
        warnings.push(warning(
            "no_store_access",
            "The key cannot see any store. Grant it 'View your stores' for the stores Kassiber should read.".into(),
            vec![],
        ));
    }
    let mut public_stores = Vec::new();
    let mut payment_methods = Vec::new();
    let mut previews = Vec::new();
    for entry in stores {
        let id = from_py(&text(field(&entry, "id")));
        let result = inspect_store(reads, entry, &grant, major, wallet_version)?;
        public_stores.push(result.store);
        payment_methods.extend(result.methods);
        if !result.previews.is_empty() {
            if let Some(slot) = previews
                .iter_mut()
                .find(|(k, _): &&mut (String, Vec<(String, Vec<String>)>)| *k == id)
            {
                slot.1 = result.previews;
            } else {
                previews.push((id, result.previews));
            }
        }
        warnings.extend(result.warnings);
    }
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    for method in &payment_methods {
        let fingerprint = field(method, "wallet_fingerprint");
        if !truthy(fingerprint) {
            continue;
        }
        let key = from_py(&text(fingerprint));
        let member = dict([
            ("store_id", field(method, "store_id").clone()),
            (
                "payment_method_id",
                field(method, "payment_method_id").clone(),
            ),
        ]);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1.push(member),
            None => groups.push((key, vec![member])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    let shared: Vec<(String, Vec<Value>)> =
        groups.into_iter().filter(|(_, m)| m.len() > 1).collect();
    for (fingerprint, members) in &shared {
        let mut names: Vec<String> = members
            .iter()
            .map(|member| {
                let id = field(member, "store_id");
                public_stores
                    .iter()
                    .rev()
                    .find(|store| field(store, "id") == id)
                    .map(|store| from_py(&text(field(store, "name"))))
                    .unwrap_or_else(|| from_py(&text(id)))
            })
            .collect();
        names.sort();
        names.dedup();
        warnings.push(warning(
            "shared_store_wallet",
            format!(
                "{} pay into the same wallet. Kassiber imports that wallet once and keeps each store's invoices as provenance.",
                names.join(", ")
            ),
            vec![("wallet_fingerprint", s(fingerprint.as_str()))],
        ));
    }
    let shared_wallets = Value::List(
        shared
            .into_iter()
            .map(|(fingerprint, members)| {
                dict([
                    ("wallet_fingerprint", s(fingerprint)),
                    ("members", Value::List(members)),
                ])
            })
            .collect(),
    );
    Ok(Inspection {
        public: dict([
            ("server", server),
            ("api_key", api_key),
            ("stores", Value::List(public_stores)),
            ("payment_methods", Value::List(payment_methods)),
            ("shared_wallets", shared_wallets),
            ("warnings", Value::List(warnings)),
        ]),
        previews,
        grant,
    })
}
