//! Pure reshaping of Greenfield payloads: invoice origins (`origins.py`),
//! payouts (`payouts.py`), invoice payments and the page fingerprints of the
//! incremental checkpoint (`sync_btcpay.py`), and inspection helpers
//! (`discovery.py`). Inputs and outputs keep Python's value model and order.
use super::payment_methods as methods;
use super::url::{from_py, urlsplit};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::{Key, PyStr, Value};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, AppError>;

const NULL: Value = Value::Null;

pub(crate) fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&NULL)
}

/// `a.get(k1) or b.get(k2) or ...`: the first truthy operand, else the last.
pub(crate) fn either<'a>(operands: &[&'a Value]) -> &'a Value {
    operands
        .iter()
        .find(|v| truthy(v))
        .copied()
        .unwrap_or_else(|| operands[operands.len() - 1])
}

/// `str(value)`.
pub fn text(value: &Value) -> PyStr {
    match value {
        Value::Str(text) => text.clone(),
        other => py_str(other).into(),
    }
}

fn is_none_or_empty(value: &Value) -> bool {
    matches!(value, Value::Null) || matches!(value, Value::Str(t) if t.is(""))
}

/// `origins._text` / `payouts._text`: `str(value).strip()` or `None`.
pub fn stripped(value: &Value) -> Option<PyStr> {
    if is_none_or_empty(value) {
        return None;
    }
    let text = kassiber_pytext::strip_py(&text(value));
    (!text.is("")).then_some(text)
}

/// `sync_btcpay._str_or_none`: `str(value)` unless `None` or `""`.
pub fn str_or_none(value: &Value) -> Option<PyStr> {
    (!is_none_or_empty(value)).then(|| text(value))
}

fn opt(value: Option<PyStr>) -> Value {
    value.map_or(Value::Null, Value::Str)
}

fn lower(text: &PyStr) -> String {
    kassiber_pytext::lower(&from_py(text))
}

fn is_dict(value: &Value) -> bool {
    matches!(value, Value::Dict(_))
}

fn concat(parts: &[&PyStr]) -> PyStr {
    PyStr::from_code_points(parts.iter().flat_map(|p| p.code_points())).expect("code points")
}

// --- Timestamps -------------------------------------------------------------------------

fn protocol(message: String) -> AppError {
    AppError::new("protocol_error", message)
}

/// `datetime.fromtimestamp(seconds, tz=utc).isoformat()` with `Z`; CPython's
/// out-of-range errors surface as `internal_error`.
fn iso(seconds: i128) -> Result<String> {
    let range = || AppError::new("internal_error", "timestamp out of range");
    let seconds = i64::try_from(seconds).map_err(|_| range())?;
    crate::lightning::utc_iso(seconds)?.ok_or_else(range)
}

/// `int(value)`, `None` where CPython raises `TypeError`/`ValueError`.
fn int(value: &Value) -> Result<Option<i128>> {
    let overflow = || AppError::new("internal_error", "cannot convert float infinity to integer");
    Ok(match value {
        Value::Bool(b) => Some(i128::from(*b)),
        Value::Int(n) => Some(i128::try_from(n.clone()).map_err(|_| overflow())?),
        Value::Float(n) if n.is_infinite() => return Err(overflow()),
        Value::Float(n) if n.is_nan() => None,
        Value::Float(n) => Some(n.trunc() as i128),
        Value::Str(t) => t
            .as_str()
            .and_then(|t| kassiber_pytext::parse_int(t).ok())
            .map(i128::try_from)
            .transpose()
            .map_err(|_| overflow())?,
        _ => None,
    })
}

/// `float(value)` for the second attempt of `_unix_to_iso`.
fn float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Int(n) => n.to_string().parse().ok(),
        Value::Float(n) => Some(*n),
        Value::Str(t) => t
            .as_str()
            .and_then(|t| kassiber_pytext::parse_float(t).ok()),
        _ => None,
    }
}

/// `_unix_to_iso`.
pub fn unix_to_iso(value: &Value) -> Result<String> {
    if let Some(seconds) = int(value)? {
        return iso(seconds);
    }
    match float(value) {
        Some(n) if n.is_infinite() => Err(AppError::new(
            "internal_error",
            "cannot convert float infinity to integer",
        )),
        Some(n) if !n.is_nan() => iso(n.trunc() as i128),
        _ => Err(protocol(format!(
            "Invalid BTCPay timestamp '{}'",
            py_str(value)
        ))),
    }
}

/// `_btcpay_time`: ISO-looking strings pass through; numbers are Unix seconds.
pub fn btcpay_time(value: &Value) -> Result<Value> {
    if is_none_or_empty(value) {
        return Ok(Value::Null);
    }
    if let Value::Str(t) = value {
        if t.code_points()
            .any(|p| [u32::from('T'), u32::from('Z'), u32::from('+')].contains(&p))
        {
            return Ok(value.clone());
        }
    }
    Ok(s(unix_to_iso(value)?))
}

/// `payouts._timestamp`.
fn payout_time(value: &Value) -> Result<Value> {
    if is_none_or_empty(value) {
        return Ok(Value::Null);
    }
    if let Value::Str(t) = value {
        let text = from_py(&kassiber_pytext::strip_py(t));
        let digits = text.trim_start_matches('-');
        // `str.isdigit()` accepts every Unicode digit; `int()` then decides.
        let numeric = !digits.is_empty()
            && digits
                .chars()
                .all(|c| kassiber_pytext::decimal_value(c.into()).is_some() || c.is_numeric());
        if !numeric {
            return Ok(value.clone());
        }
    }
    let seconds = match float(value) {
        Some(n) if n.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Some(n) if !n.is_nan() => n.trunc() as i128,
        _ => return Ok(Value::Null),
    };
    Ok(s(iso(seconds)?))
}

// --- Origins ----------------------------------------------------------------------------

const PAYMENT_REQUEST_PREFIX: &str = "PAY_REQUEST_";

fn url_path(url: Option<&PyStr>) -> String {
    url.and_then(|u| urlsplit(&from_py(u)).ok())
        .map(|parts| parts.path)
        .unwrap_or_default()
}

fn delimiter(c: char) -> bool {
    matches!(c, '/' | '?' | '#')
}

/// `urllib.parse.unquote` (UTF-8, invalid sequences replaced).
fn unquote(raw: &str) -> String {
    if !raw.contains('%') {
        return raw.to_owned();
    }
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit));
        if let (b'%', Some(hex)) = (bytes[i], hex) {
            let hex = std::str::from_utf8(hex).expect("ASCII hex");
            out.push(u8::from_str_radix(hex, 16).expect("hex"));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `_app_route(order_url)`: `(app_id, app_type)`.
fn app_route(order_url: Option<&PyStr>) -> (Option<String>, Option<String>) {
    let path = url_path(order_url);
    let lowered = path.to_ascii_lowercase();
    // `_LEGACY_APP_ROUTE` (case-insensitive) is searched first.
    let mut start = 0;
    while let Some(found) = lowered[start..].find("/apps/") {
        let at = start + found + "/apps/".len();
        for kind in ["pos", "crowdfund"] {
            if !lowered[at..].starts_with(kind) {
                continue;
            }
            let after = at + kind.len();
            let rest = &path[after..];
            if let Some(tail) = rest.strip_prefix('/') {
                let id_len = tail.find(delimiter).unwrap_or(tail.len());
                if id_len > 0 {
                    return (Some(tail[..id_len].to_owned()), Some(kind.into()));
                }
            }
            if rest.is_empty() || rest.starts_with(delimiter) {
                return (None, Some(kind.into()));
            }
        }
        start = start + found + 1;
    }
    let mut start = 0;
    while let Some(found) = path[start..].find("/apps/") {
        let at = start + found + "/apps/".len();
        let tail = &path[at..];
        let id_len = tail.find(delimiter).unwrap_or(tail.len());
        if id_len > 0 && tail[id_len..].starts_with('/') {
            let rest = &tail[id_len + 1..];
            let type_len = rest.find(delimiter).unwrap_or(rest.len());
            if type_len > 0 {
                return (
                    Some(unquote(&tail[..id_len])),
                    Some(kassiber_pytext::lower(&rest[..type_len])),
                );
            }
        }
        start = start + found + 1;
    }
    (None, None)
}

fn pos_label(pos_data: &Value) -> Option<PyStr> {
    if !is_dict(pos_data) {
        return None;
    }
    for key in [
        "title",
        "name",
        "itemDesc",
        "itemDescription",
        "description",
    ] {
        if let Some(value) = stripped(field(pos_data, key)) {
            return Some(value);
        }
    }
    let Value::List(cart) = field(pos_data, "cart") else {
        return None;
    };
    let titles: Vec<PyStr> = cart
        .iter()
        .filter(|item| is_dict(item))
        .filter_map(|item| stripped(field(item, "title")).or_else(|| stripped(field(item, "id"))))
        .collect();
    if titles.is_empty() {
        return None;
    }
    let mut joined = Vec::new();
    for (index, title) in titles.iter().take(3).enumerate() {
        if index > 0 {
            joined.extend(", ".chars().map(u32::from));
        }
        joined.extend(title.code_points());
    }
    if titles.len() > 3 {
        joined.extend(" …".chars().map(u32::from));
    }
    Some(PyStr::from_code_points(joined).expect("code points"))
}

fn dict_keys(value: &Value) -> Vec<String> {
    match value {
        Value::Dict(pairs) => pairs
            .iter()
            .map(|(k, _)| match k {
                Key::Str(k) => lower(k),
                other => kassiber_pytext::lower(&format!("{other:?}")),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn ecommerce_source(metadata: &Value, order_url: Option<&PyStr>) -> Option<String> {
    let url = order_url.map(lower).unwrap_or_default();
    let pos_keys = dict_keys(field(metadata, "posData"));
    let has_pos = |k: &str| pos_keys.iter().any(|p| p == k);
    if has_pos("woocommerce")
        || url.contains("wp-admin")
        || url.contains("wc-api")
        || url.contains("woocommerce")
    {
        return Some("woocommerce".into());
    }
    let keys = dict_keys(metadata);
    let starts = |prefix: &str| keys.iter().any(|k| k.starts_with(prefix));
    if starts("shopify") || url.contains("myshopify.com") || has_pos("shopify") {
        return Some("shopify".into());
    }
    if starts("magento") || has_pos("magento") {
        return Some("magento".into());
    }
    if starts("prestashop") || has_pos("prestashop") {
        return Some("prestashop".into());
    }
    let source = stripped(either(&[
        field(metadata, "source"),
        field(metadata, "plugin"),
    ]))?;
    let source = lower(&source);
    [
        "woocommerce",
        "shopify",
        "magento",
        "prestashop",
        "odoo",
        "drupal",
        "whmcs",
        "wix",
        "ecwid",
    ]
    .contains(&source.as_str())
    .then_some(source)
}

/// `payment_request_id_for`.
pub fn payment_request_id(
    invoice: &Value,
    metadata: &Value,
    order_id: Option<&PyStr>,
) -> Option<PyStr> {
    if let Some(explicit) = stripped(either(&[
        field(metadata, "paymentRequestId"),
        field(metadata, "payment_request_id"),
        field(invoice, "paymentRequestId"),
    ])) {
        return Some(explicit);
    }
    let order_id = order_id?;
    if !kassiber_pytext::upper(&from_py(order_id)).starts_with(PAYMENT_REQUEST_PREFIX) {
        return None;
    }
    let tail: Vec<u32> = order_id
        .code_points()
        .skip(PAYMENT_REQUEST_PREFIX.len())
        .collect();
    stripped(&Value::Str(
        PyStr::from_code_points(tail).expect("code points"),
    ))
}

pub struct Origin {
    pub kind: &'static str,
    pub app_id: Value,
    pub app_type: Value,
    pub label: Value,
    pub url: Value,
    pub source: Value,
}

/// `classify_invoice_origin`.
pub fn classify_origin(
    invoice: &Value,
    metadata: &Value,
    order_id: Option<&PyStr>,
    titles: &dyn Fn(&PyStr) -> Option<PyStr>,
) -> Origin {
    let order_url = stripped(either(&[
        field(metadata, "orderUrl"),
        field(invoice, "orderUrl"),
    ]));
    let app_id = stripped(either(&[
        field(metadata, "appId"),
        field(metadata, "app_id"),
        field(metadata, "applicationId"),
    ]));
    let app_name = stripped(either(&[
        field(metadata, "appName"),
        field(metadata, "app_name"),
        field(metadata, "applicationName"),
    ]));
    let item_desc = stripped(either(&[
        field(metadata, "itemDesc"),
        field(metadata, "itemDescription"),
    ]));
    let pos_data = field(metadata, "posData");
    let pos_label = pos_label(pos_data);
    let (route_id, route_type) = app_route(order_url.as_ref());
    let request = payment_request_id(invoice, metadata, order_id);
    let lowered_url = order_url.as_ref().map(lower).unwrap_or_default();
    let lowered_order = order_id.map(lower).unwrap_or_default();
    let lowered_app = kassiber_pytext::lower(&format!(
        "{} {}",
        app_name.as_ref().map(from_py).unwrap_or_default(),
        app_id.as_ref().map(from_py).unwrap_or_default()
    ));
    let url = opt(order_url.clone());
    let first = |options: &[Option<PyStr>]| opt(options.iter().flatten().next().cloned());
    let route_id = route_id.map(|v| PyStr::from(v.as_str()));
    if let Some(source) = ecommerce_source(metadata, order_url.as_ref()) {
        let number = stripped(either(&[
            field(metadata, "orderNumber"),
            &order_id.cloned().map_or(Value::Null, Value::Str),
        ]));
        let label = item_desc
            .clone()
            .or_else(|| number.map(|n| concat(&[&PyStr::from("Order "), &n])));
        return Origin {
            kind: "ecommerce",
            app_id: Value::Null,
            app_type: Value::Null,
            label: opt(label),
            url,
            source: s(source),
        };
    }
    let order = order_id.cloned();
    if route_type.as_deref() == Some("pos")
        || !matches!(pos_data, Value::Null)
        || lowered_url.contains("/pos")
        || lowered_order.starts_with("pos")
    {
        return Origin {
            kind: "pos",
            app_id: first(&[app_id, route_id]),
            app_type: s("pos"),
            label: first(&[app_name, item_desc, pos_label, order]),
            url,
            source: s("btcpay_pos"),
        };
    }
    if route_type.as_deref() == Some("crowdfund")
        || lowered_app.contains("crowdfund")
        || lowered_url.contains("/crowdfund")
        || lowered_order.starts_with("crowdfund")
    {
        return Origin {
            kind: "crowdfund",
            app_id: first(&[app_id, route_id]),
            app_type: s("crowdfund"),
            label: first(&[app_name, item_desc, order]),
            url,
            source: s("btcpay_crowdfund"),
        };
    }
    if let Some(request) = request {
        let title = titles(&request);
        return Origin {
            kind: "payment_request",
            app_id: Value::Null,
            app_type: Value::Null,
            label: first(&[item_desc, title, Some(request)]),
            url,
            source: s("btcpay_payment_request"),
        };
    }
    if app_id.is_some() || app_name.is_some() || route_id.is_some() {
        return Origin {
            kind: "app",
            app_id: first(&[app_id, route_id]),
            app_type: route_type.map_or(Value::Null, s),
            label: first(&[app_name, item_desc, order]),
            url,
            source: s("btcpay_app"),
        };
    }
    if order_url.is_some() || order.is_some() {
        return Origin {
            kind: "external_order",
            app_id: Value::Null,
            app_type: Value::Null,
            label: first(&[item_desc, order]),
            url,
            source: Value::Null,
        };
    }
    Origin {
        kind: "unknown",
        app_id: Value::Null,
        app_type: Value::Null,
        label: opt(item_desc),
        url,
        source: Value::Null,
    }
}

// --- Invoices and payments --------------------------------------------------------------

fn looks_like_txid(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `str(value or "")` as Rust text.
fn or_empty(value: &Value) -> String {
    if truthy(value) {
        from_py(&text(value))
    } else {
        String::new()
    }
}

fn txid_from_payment_id(value: &Value, method: Option<&str>) -> Option<String> {
    let raw = or_empty(value);
    let raw = kassiber_pytext::strip(&raw);
    if looks_like_txid(raw) && methods::canonical(method.unwrap_or("")).ends_with("-CHAIN") {
        return Some(raw.to_owned());
    }
    match raw.split_once('-') {
        Some((first, _)) if looks_like_txid(first) => Some(first.to_owned()),
        _ => None,
    }
}

fn payment_hash_from_payment_id(value: &Value, method: Option<&str>) -> Option<String> {
    let raw = kassiber_pytext::lower(kassiber_pytext::strip(&or_empty(value)));
    let rail = methods::classify(method.unwrap_or("")).rail;
    (looks_like_txid(&raw) && [methods::RAIL_LIGHTNING, methods::RAIL_LNURL].contains(&rail))
        .then_some(raw)
}

fn canonical_value(value: &Value) -> Option<String> {
    Some(methods::canonical(&or_empty(value))).filter(|m| !m.is_empty())
}

/// `_normalize_invoice_payment`.
pub fn normalize_payment(invoice: &Value, payment: &Value) -> Result<Value> {
    let empty = dict::<&str>([]);
    let details = match field(payment, "details") {
        d @ Value::Dict(_) => d,
        _ => &empty,
    };
    let method = canonical_value(either(&[
        field(payment, "paymentMethod"),
        field(payment, "paymentMethodId"),
        field(payment, "paymentMethodData"),
        field(details, "paymentMethod"),
    ]));
    let payment_id = either(&[
        field(payment, "id"),
        field(payment, "paymentId"),
        field(payment, "accountedPaymentId"),
        field(payment, "transactionId"),
        field(details, "transactionId"),
    ]);
    let fallback = |derived: Option<String>| derived.map_or(Value::Null, s);
    let txid_derived = fallback(txid_from_payment_id(payment_id, method.as_deref()));
    let hash_derived = fallback(payment_hash_from_payment_id(payment_id, method.as_deref()));
    Ok(dict([
        ("payment", payment.clone()),
        ("payment_id", opt(str_or_none(payment_id))),
        ("payment_method_id", opt(method.as_deref().map(PyStr::from))),
        (
            "status",
            opt(str_or_none(either(&[
                field(payment, "status"),
                field(details, "status"),
            ]))),
        ),
        (
            "received_at",
            btcpay_time(either(&[
                field(payment, "receivedDate"),
                field(payment, "receivedTime"),
                field(payment, "createdTime"),
                field(details, "receivedDate"),
            ]))?,
        ),
        (
            "amount",
            opt(str_or_none(either(&[
                field(payment, "value"),
                field(payment, "cryptoAmount"),
                field(payment, "amount"),
                field(details, "value"),
            ]))),
        ),
        (
            "fee",
            opt(str_or_none(either(&[
                field(payment, "fee"),
                field(details, "fee"),
            ]))),
        ),
        (
            "rate",
            opt(str_or_none(either(&[
                field(payment, "rate"),
                field(details, "rate"),
            ]))),
        ),
        (
            "txid",
            opt(str_or_none(either(&[
                field(payment, "transactionId"),
                field(payment, "transactionHash"),
                field(details, "transactionId"),
                field(details, "transactionHash"),
                &txid_derived,
            ]))),
        ),
        (
            "payment_hash",
            opt(str_or_none(either(&[
                field(payment, "paymentHash"),
                field(payment, "preimageHash"),
                field(details, "paymentHash"),
                field(details, "preimageHash"),
                &hash_derived,
            ]))),
        ),
        (
            "destination",
            opt(str_or_none(either(&[
                field(payment, "destination"),
                field(payment, "address"),
                field(details, "destination"),
                field(details, "address"),
            ]))),
        ),
        (
            "invoice_currency",
            opt(str_or_none(field(invoice, "currency"))),
        ),
        ("invoice_amount", opt(str_or_none(field(invoice, "amount")))),
    ]))
}

fn set_default(pairs: &mut Vec<(Key, Value)>, key: &str, value: Value) {
    if !pairs.iter().any(|(k, _)| *k == Key::from(key)) {
        pairs.push((Key::from(key), value));
    }
}

/// `_payments_from_invoice_payment_methods`.
pub fn payments_from_methods(methods_list: &[Value]) -> Value {
    let mut payments = Vec::new();
    for method in methods_list.iter().filter(|m| is_dict(m)) {
        let id = canonical_value(either(&[
            field(method, "paymentMethodId"),
            field(method, "paymentMethod"),
        ]));
        let empty = Value::List(Vec::new());
        let list = either(&[field(method, "payments"), &empty]);
        let Value::List(list) = list else {
            continue;
        };
        for payment in list {
            let Value::Dict(pairs) = payment else {
                continue;
            };
            let mut enriched = pairs.clone();
            set_default(
                &mut enriched,
                "paymentMethodId",
                id.as_deref().map_or(Value::Null, s),
            );
            for key in ["rate", "destination"] {
                if !matches!(field(method, key), Value::Null) {
                    set_default(&mut enriched, key, field(method, key).clone());
                }
            }
            let mut value = Value::Dict(enriched);
            let txid = txid_from_payment_id(
                &Value::Str(text(either(&[field(&value, "id"), &s("")]))),
                id.as_deref(),
            );
            if !truthy(field(&value, "transactionId")) {
                if let Some(txid) = txid {
                    kassiber_envelope::set(&mut value, "transactionId", s(txid));
                }
            }
            payments.push(value);
        }
    }
    Value::List(payments)
}

/// `_invoice_metadata`.
pub fn invoice_metadata(invoice: &Value) -> Value {
    match field(invoice, "metadata") {
        m @ Value::Dict(_) => m.clone(),
        _ => dict::<&str>([]),
    }
}

/// `_normalize_invoice_provenance`.
pub fn normalize_invoice(
    store_id: &PyStr,
    invoice: &Value,
    titles: &dyn Fn(&PyStr) -> Option<PyStr>,
) -> Result<Value> {
    if !is_dict(invoice) {
        return Err(protocol(
            "BTCPay invoice record was not a JSON object".into(),
        ));
    }
    let invoice_id = either(&[field(invoice, "id"), field(invoice, "invoiceId")]);
    if !truthy(invoice_id) {
        return Err(protocol("BTCPay invoice record is missing 'id'".into()));
    }
    let payments = match either(&[field(invoice, "payments"), &Value::List(Vec::new())]) {
        Value::List(items) => items.clone(),
        _ => Vec::new(),
    };
    let metadata = invoice_metadata(invoice);
    let order_id = str_or_none(either(&[
        field(invoice, "orderId"),
        field(&metadata, "orderId"),
    ]));
    let origin = classify_origin(invoice, &metadata, order_id.as_ref(), titles);
    let mut normalized = Vec::new();
    for payment in payments.iter().filter(|p| is_dict(p)) {
        normalized.push(normalize_payment(invoice, payment)?);
    }
    Ok(dict([
        ("store_id", Value::Str(store_id.clone())),
        ("invoice", invoice.clone()),
        ("invoice_id", Value::Str(text(invoice_id))),
        ("order_id", opt(order_id.clone())),
        (
            "order_url",
            opt(str_or_none(either(&[
                field(&metadata, "orderUrl"),
                field(invoice, "orderUrl"),
            ]))),
        ),
        (
            "payment_request_id",
            opt(payment_request_id(invoice, &metadata, order_id.as_ref())),
        ),
        ("origin_kind", s(origin.kind)),
        ("origin_app_id", origin.app_id),
        ("origin_app_type", origin.app_type),
        ("origin_source", origin.source),
        ("origin_label", origin.label),
        ("origin_url", origin.url),
        ("status", opt(str_or_none(field(invoice, "status")))),
        (
            "additional_status",
            opt(str_or_none(field(invoice, "additionalStatus"))),
        ),
        ("invoice_type", opt(str_or_none(field(invoice, "type")))),
        (
            "created_at",
            btcpay_time(either(&[
                field(invoice, "createdTime"),
                field(invoice, "created"),
            ]))?,
        ),
        ("currency", opt(str_or_none(field(invoice, "currency")))),
        ("amount", opt(str_or_none(field(invoice, "amount")))),
        ("payments", Value::List(normalized)),
    ]))
}

// --- Page checkpoints -------------------------------------------------------------------

pub(crate) fn sorted_dumps(value: &Value) -> Result<String> {
    let options = kassiber_pyjson::Options {
        sort_keys: true,
        ..Default::default()
    };
    kassiber_pyjson::dumps(value, &options)
        .map_err(|e| AppError::new("internal_error", e.to_string()))
}

pub(crate) fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn stable_id(item: &Value, keys: &[&str]) -> Result<PyStr> {
    let mut operands: Vec<&Value> = keys.iter().map(|k| field(item, k)).collect();
    let dumped;
    if !operands.iter().any(|v| truthy(v)) {
        dumped = s(sorted_dumps(item)?);
        operands.push(&dumped);
    }
    Ok(text(either(&operands)))
}

fn sorted_ids(page: &[Value], keys: &[&str]) -> Result<Value> {
    let mut ids = page
        .iter()
        .filter(|v| is_dict(v))
        .map(|v| stable_id(v, keys))
        .collect::<Result<Vec<_>>>()?;
    ids.sort();
    Ok(Value::List(ids.into_iter().map(Value::Str).collect()))
}

const INVOICE_ID_KEYS: [&str; 2] = ["id", "invoiceId"];
const WALLET_ID_KEYS: [&str; 3] = ["transactionHash", "transactionId", "id"];

/// `_invoice_page_stable_ids`.
pub fn invoice_page_ids(page: &[Value]) -> Result<Value> {
    sorted_ids(page, &INVOICE_ID_KEYS)
}

/// `_page_stable_ids` (wallet history).
pub fn wallet_page_ids(page: &[Value]) -> Result<Value> {
    sorted_ids(page, &WALLET_ID_KEYS)
}

fn fingerprint(mut rows: Vec<(PyStr, Value)>) -> Result<String> {
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let rows = Value::List(rows.into_iter().map(|(_, row)| row).collect());
    Ok(sha256_hex(&sorted_dumps(&rows)?))
}

/// `_invoice_page_fingerprint`.
pub fn invoice_page_fingerprint(page: &[Value]) -> Result<String> {
    let mut rows = Vec::new();
    for invoice in page.iter().filter(|v| is_dict(v)) {
        let metadata = invoice_metadata(invoice);
        let id = stable_id(invoice, &INVOICE_ID_KEYS)?;
        rows.push((
            id.clone(),
            dict([
                ("id", Value::Str(id)),
                ("status", field(invoice, "status").clone()),
                (
                    "additionalStatus",
                    field(invoice, "additionalStatus").clone(),
                ),
                ("paidAmount", field(invoice, "paidAmount").clone()),
                (
                    "orderId",
                    either(&[field(invoice, "orderId"), field(&metadata, "orderId")]).clone(),
                ),
                (
                    "orderUrl",
                    either(&[field(invoice, "orderUrl"), field(&metadata, "orderUrl")]).clone(),
                ),
                (
                    "paymentRequestId",
                    either(&[
                        field(invoice, "paymentRequestId"),
                        field(&metadata, "paymentRequestId"),
                        field(&metadata, "payment_request_id"),
                    ])
                    .clone(),
                ),
                ("metadata", metadata.clone()),
                ("paymentMethods", field(invoice, "paymentMethods").clone()),
                ("payments", field(invoice, "payments").clone()),
            ]),
        ));
    }
    fingerprint(rows)
}

/// `parse_btcpay_labels`.
pub fn parse_labels(value: &Value) -> Value {
    let label = |item: &Value| -> Option<PyStr> {
        let text = kassiber_pytext::strip_py(&text(item));
        (!text.is("")).then_some(text)
    };
    let labels: Vec<PyStr> = match value {
        Value::Null => Vec::new(),
        Value::Str(t) if t.is("") => Vec::new(),
        Value::Dict(pairs) => pairs
            .iter()
            .filter_map(|(key, item)| {
                let mut found = if is_dict(item) {
                    field(item, "text").clone()
                } else {
                    item.clone()
                };
                if matches!(found, Value::Null) {
                    if let Key::Str(key) = key {
                        if !key.is("") {
                            found = Value::Str(key.clone());
                        }
                    }
                }
                if matches!(found, Value::Null) {
                    None
                } else {
                    label(&found)
                }
            })
            .collect(),
        Value::List(items) => items
            .iter()
            .filter_map(|item| {
                let found = if is_dict(item) {
                    field(item, "text")
                } else {
                    item
                };
                if matches!(found, Value::Null) {
                    None
                } else {
                    label(found)
                }
            })
            .collect(),
        other => {
            let whole = text(other);
            let mut parts = Vec::new();
            let mut current = Vec::new();
            for point in whole.code_points().chain(std::iter::once(u32::from(','))) {
                if point == u32::from(',') {
                    let part = kassiber_pytext::strip_py(
                        &PyStr::from_code_points(std::mem::take(&mut current)).expect("points"),
                    );
                    if !part.is("") {
                        parts.push(part);
                    }
                } else {
                    current.push(point);
                }
            }
            parts
        }
    };
    Value::List(labels.into_iter().map(Value::Str).collect())
}

/// `_page_fingerprint` (wallet history).
pub fn wallet_page_fingerprint(page: &[Value]) -> Result<String> {
    let mut rows = Vec::new();
    for tx in page.iter().filter(|v| is_dict(v)) {
        let id = stable_id(tx, &WALLET_ID_KEYS)?;
        rows.push((
            id.clone(),
            dict([
                ("id", Value::Str(id)),
                ("timestamp", field(tx, "timestamp").clone()),
                ("amount", field(tx, "amount").clone()),
                ("confirmations", field(tx, "confirmations").clone()),
                ("status", field(tx, "status").clone()),
                ("comment", field(tx, "comment").clone()),
                ("labels", parse_labels(field(tx, "labels"))),
            ]),
        ));
    }
    fingerprint(rows)
}

fn invoice_status(invoice: &Value, key: &str) -> String {
    kassiber_pytext::lower(kassiber_pytext::strip(&or_empty(field(invoice, key))))
}

/// `_invoice_may_have_payments`.
pub fn invoice_may_have_payments(invoice: &Value) -> bool {
    [
        "settled",
        "processing",
        "invalid",
        "paid",
        "confirmed",
        "complete",
    ]
    .contains(&invoice_status(invoice, "status").as_str())
        || !["", "none"].contains(&invoice_status(invoice, "additionalStatus").as_str())
}

/// `_invoice_page_has_open_invoice`.
pub fn invoice_page_has_open(page: &[Value]) -> bool {
    page.iter().filter(|v| is_dict(v)).any(|invoice| {
        let status = invoice_status(invoice, "status");
        status == "processing"
            || (["new", "processing", "paid"].contains(&status.as_str())
                && !["", "none"].contains(&invoice_status(invoice, "additionalStatus").as_str()))
    })
}

// --- Payouts ----------------------------------------------------------------------------

fn refund_invoice_id(pull: &Value) -> Option<PyStr> {
    if !is_dict(pull) {
        return None;
    }
    for key in ["name", "description"] {
        // `^\s*refund\s+(?P<invoice_id>[1-9A-HJ-NP-Za-km-z]{16,})\s*$` under
        // IGNORECASE: case folding readmits I, O and l, so only `0` is out.
        let raw = or_empty(field(pull, key));
        let trimmed = raw.trim_start_matches(|c: char| kassiber_pytext::is_space(c.into()));
        let Some(rest) = trimmed
            .get(..6)
            .filter(|p| p.eq_ignore_ascii_case("refund"))
            .map(|_| &trimmed[6..])
        else {
            continue;
        };
        let after = rest.trim_start_matches(|c: char| kassiber_pytext::is_space(c.into()));
        if after.len() == rest.len() {
            continue;
        }
        let id_len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() && c != '0'))
            .unwrap_or(after.len());
        let tail = &after[id_len..];
        // `$` also matches before one trailing newline.
        let tail = tail.strip_suffix('\n').unwrap_or(tail);
        if id_len >= 16 && tail.chars().all(|c| kassiber_pytext::is_space(c.into())) {
            return Some(PyStr::from(&after[..id_len]));
        }
    }
    None
}

fn safe_destination(value: &Value) -> Option<PyStr> {
    let text = stripped(value)?;
    let lowered = kassiber_pytext::lower(&from_py(&text));
    let body = lowered.strip_prefix("lightning:").unwrap_or(&lowered);
    let alnum = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
    };
    let bolt11 = ["lnbcrt", "lnbc", "lntbs", "lntb", "lnsb"]
        .iter()
        .any(|prefix| body.strip_prefix(prefix).is_some_and(alnum));
    let lnurl = body.strip_prefix("lnurl").is_some_and(alnum);
    (!(bolt11 || lnurl)).then_some(text)
}

fn sanitize_proof(proof: &Value) -> Value {
    match proof {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .filter(|(key, _)| {
                    let name = match key {
                        Key::Str(k) => lower(k),
                        _ => String::new(),
                    };
                    !["preimage", "paymentpreimage", "secret"].contains(&name.as_str())
                })
                .cloned()
                .collect(),
        ),
        _ => dict::<&str>([]),
    }
}

fn proof_references(proof: &Value, method: Option<&str>) -> (Option<String>, Option<String>) {
    let Value::Dict(pairs) = proof else {
        return (None, None);
    };
    let lookup = |name: &str| {
        pairs
            .iter()
            .rev()
            .find(|(k, _)| matches!(k, Key::Str(k) if lower(k) == name))
            .map(|(_, v)| v)
            .unwrap_or(&NULL)
    };
    let proof_type = kassiber_pytext::lower(&or_empty(lookup("prooftype")));
    let upper_method = kassiber_pytext::upper(method.unwrap_or(""));
    let lightning = proof_type.contains("lightning")
        || upper_method.ends_with("-LN")
        || upper_method.ends_with("-LNURL");
    let (mut txid, mut hash) = (None, None);
    for key in ["transactionid", "txid", "paymenthash", "id"] {
        let Some(text) = stripped(lookup(key)).map(|t| from_py(&t)) else {
            continue;
        };
        if !looks_like_txid(&text) {
            continue;
        }
        let lowered = text.to_ascii_lowercase();
        if lightning {
            hash = hash.or(Some(lowered));
        } else {
            txid = txid.or(Some(lowered));
        }
    }
    (txid, hash)
}

/// `normalize_payout`; `None` for a payout without an id.
pub fn normalize_payout(
    store_id: &PyStr,
    payout: &Value,
    pull_payments: &dyn Fn(&PyStr) -> Option<Value>,
) -> Result<Option<Value>> {
    let Some(payout_id) = stripped(field(payout, "id")) else {
        return Ok(None);
    };
    let method = canonical_value(either(&[
        field(payout, "payoutMethodId"),
        field(payout, "paymentMethod"),
        field(payout, "paymentMethodId"),
    ]));
    let pull_id = stripped(field(payout, "pullPaymentId"));
    let pull = pull_id.as_ref().and_then(pull_payments);
    let pull_value = pull.clone().unwrap_or(Value::Null);
    let invoice_id = refund_invoice_id(&pull_value);
    let proof = sanitize_proof(field(payout, "paymentProof"));
    let (txid, hash) = proof_references(&proof, method.as_deref());
    let asset = method.as_deref().and_then(methods::asset);
    let currency = stripped(either(&[
        field(payout, "payoutCurrency"),
        field(payout, "cryptoCode"),
    ]));
    let mut amount = stripped(either(&[
        field(payout, "payoutAmount"),
        field(payout, "paymentMethodAmount"),
    ]));
    if let (Some(currency), Some(asset)) = (&currency, &asset) {
        if kassiber_pytext::upper(&from_py(currency)) != *asset {
            amount = None;
        }
    }
    let original_currency = stripped(either(&[
        field(payout, "originalCurrency"),
        field(payout, "currency"),
    ]));
    let original_amount = stripped(either(&[
        field(payout, "originalAmount"),
        field(payout, "amount"),
    ]));
    let empty = dict::<&str>([]);
    let pull_dict = if pull.is_some() { &pull_value } else { &empty };
    let (kind, label) = if let Some(invoice) = &invoice_id {
        (
            "refund",
            stripped(field(pull_dict, "name"))
                .or_else(|| Some(concat(&[&PyStr::from("Refund "), invoice]))),
        )
    } else if let Some(pull_id) = &pull_id {
        (
            "pull_payment",
            stripped(field(pull_dict, "name")).or_else(|| Some(pull_id.clone())),
        )
    } else {
        let metadata = match field(payout, "metadata") {
            m @ Value::Dict(_) => m,
            _ => &empty,
        };
        (
            "store_payout",
            stripped(either(&[
                field(metadata, "source"),
                field(metadata, "label"),
                field(metadata, "description"),
                field(metadata, "name"),
            ])),
        )
    };
    let destination = opt(safe_destination(field(payout, "destination")));
    let mut raw = payout.clone();
    kassiber_envelope::set(
        &mut raw,
        "paymentProof",
        if truthy(&proof) {
            proof.clone()
        } else {
            Value::Null
        },
    );
    kassiber_envelope::set(&mut raw, "destination", destination.clone());
    let fiat = original_currency.filter(|c| {
        !["BTC", "SATS", "LBTC"].contains(&kassiber_pytext::upper(&from_py(c)).as_str())
    });
    let fiat_value = if fiat.is_some() {
        opt(original_amount)
    } else {
        Value::Null
    };
    Ok(Some(dict([
        ("store_id", Value::Str(store_id.clone())),
        ("payout_id", Value::Str(payout_id)),
        ("pull_payment_id", opt(pull_id)),
        ("invoice_id", opt(invoice_id)),
        ("payout_method_id", method.map_or(Value::Null, s)),
        ("state", opt(stripped(field(payout, "state")))),
        ("occurred_at", payout_time(field(payout, "date"))?),
        ("asset", asset.map_or(Value::Null, s)),
        ("amount", opt(amount)),
        ("txid", txid.map_or(Value::Null, s)),
        ("payment_hash", hash.map_or(Value::Null, s)),
        ("destination", destination),
        ("origin_kind", s(kind)),
        ("origin_label", opt(label)),
        ("origin_url", opt(stripped(field(pull_dict, "viewLink")))),
        ("fiat_currency", opt(fiat)),
        ("fiat_value", fiat_value),
        ("payout", raw),
    ])))
}

// --- Inspection helpers -----------------------------------------------------------------

/// `network_from_address`.
pub fn network_from_address(address: &str) -> (Option<&'static str>, Option<&'static str>) {
    let value = kassiber_pytext::lower(kassiber_pytext::strip(address));
    for (prefix, chain, network) in [
        ("bcrt1", "bitcoin", "regtest"),
        ("bc1", "bitcoin", "main"),
        ("tb1", "bitcoin", "test"),
        ("tlq1", "liquid", "liquidtestnet"),
        ("tex1", "liquid", "liquidtestnet"),
        ("lq1", "liquid", "liquidv1"),
        ("ex1", "liquid", "liquidv1"),
        ("el1", "liquid", "elementsregtest"),
        ("ert1", "liquid", "elementsregtest"),
    ] {
        if value.starts_with(prefix) {
            return (Some(chain), Some(network));
        }
    }
    (None, None)
}

/// `wallet_fingerprint`.
pub fn wallet_fingerprint(addresses: &[String]) -> Option<String> {
    let cleaned: std::collections::BTreeSet<&str> = addresses
        .iter()
        .map(|a| kassiber_pytext::strip(a))
        .filter(|a| !a.is_empty())
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let joined = cleaned.into_iter().collect::<Vec<_>>().join("\n");
    Some(format!("btcpay-wallet:{}", &sha256_hex(&joined)[..24]))
}

/// `_payment_method_rows`: 2.x lists and 1.x dicts.
pub fn payment_method_rows(raw: &Value) -> Vec<Value> {
    match raw {
        Value::List(items) => items.iter().filter(|v| is_dict(v)).cloned().collect(),
        Value::Dict(pairs) => pairs
            .iter()
            .map(|(key, value)| {
                let id = match key {
                    Key::Str(k) => Value::Str(k.clone()),
                    other => s(format!("{other:?}")),
                };
                match value {
                    Value::Dict(fields) => {
                        let mut row = vec![(Key::from("paymentMethodId"), id)];
                        for (k, v) in fields {
                            if let Some(slot) = row.iter_mut().find(|(existing, _)| existing == k) {
                                slot.1 = v.clone();
                            } else {
                                row.push((k.clone(), v.clone()));
                            }
                        }
                        Value::Dict(row)
                    }
                    other => dict([
                        ("paymentMethodId", id),
                        ("enabled", Value::Bool(truthy(other))),
                    ]),
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `normalize_payment_method`.
pub fn normalize_payment_method(store_id: &PyStr, raw: &Value) -> Option<Value> {
    let mut id = either(&[
        field(raw, "paymentMethodId"),
        field(raw, "paymentMethod"),
        field(raw, "id"),
    ])
    .clone();
    if !truthy(&id) {
        let crypto = either(&[field(raw, "cryptoCode"), field(raw, "currency")]);
        let payment_type = kassiber_pytext::lower(&or_empty(field(raw, "paymentType")));
        if truthy(crypto) && (payment_type.contains("chain") || payment_type.contains("onchain")) {
            id = Value::Str(concat(&[&text(crypto), &PyStr::from("-CHAIN")]));
        } else if truthy(crypto) && payment_type.contains("lightning") {
            id = Value::Str(concat(&[&text(crypto), &PyStr::from("-LN")]));
        }
    }
    let id = if truthy(&id) {
        canonical_value(&id)
    } else {
        None
    }?;
    let info = methods::classify(&id);
    let label = either(&[field(raw, "name"), field(raw, "label")]);
    let label = if truthy(label) {
        Value::Str(text(label))
    } else {
        s(info.label.as_str())
    };
    let enabled = match get(raw, "enabled") {
        Some(v) => truthy(v),
        None => true,
    };
    Some(dict([
        ("store_id", Value::Str(store_id.clone())),
        ("payment_method_id", s(id.as_str())),
        ("label", label),
        ("enabled", Value::Bool(enabled)),
        ("sync_supported", Value::Bool(info.wallet_history_supported)),
        ("rail", s(info.rail)),
        ("currency", s(info.currency.as_str())),
        ("chain", info.chain.map_or(Value::Null, s)),
        ("bitcoin_asset", Value::Bool(info.bitcoin_asset)),
        ("settlement", s(info.settlement)),
        (
            "settlement_hint",
            s(methods::settlement_hint(info.settlement)),
        ),
        (
            "ledger_group",
            info.ledger_group.as_deref().map_or(Value::Null, s),
        ),
    ]))
}
