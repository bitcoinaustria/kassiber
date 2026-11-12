use super::*;

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
        Value::Number(n) if n.to_string().contains(['.', 'e', 'E']) => "float",
        Value::Number(_) => "int",
    }
}

fn repr(value: &Value) -> String {
    match value {
        Value::String(s) => kassiber_pytext::repr_str(s),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(repr).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(items) => format!(
            "{{{}}}",
            items
                .iter()
                .map(|(k, v)| format!("{}: {}", kassiber_pytext::repr_str(k), repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => string(value),
    }
}

pub(super) fn get<'a>(value: &'a Value, key: &str) -> Result<&'a Value, Refusal> {
    value
        .as_object()
        .map(|m| m.get(key).unwrap_or(&Value::Null))
        .ok_or_else(|| {
            Refusal::new(
                "AttributeError",
                format!("'{}' object has no attribute 'get'", kind(value)),
            )
        })
}

fn iterable(value: &Value) -> Result<Vec<Value>, Refusal> {
    match value {
        Value::Array(items) => Ok(items.clone()),
        Value::Object(items) => Ok(items.keys().cloned().map(Value::String).collect()),
        Value::String(text) => Ok(text.chars().map(|c| json!(c.to_string())).collect()),
        _ => Err(Refusal::new(
            "TypeError",
            format!("'{}' object is not iterable", kind(value)),
        )),
    }
}

fn decimal(value: &Value) -> Result<Decimal, Refusal> {
    value
        .as_str()
        .and_then(|s| Decimal::from_canonical(s).ok())
        .ok_or_else(|| {
            Refusal::new(
                "ValueError",
                format!("not a canonical engine decimal: {}", repr(value)),
            )
        })
}

fn dec_field(value: &Value, key: &str) -> Result<Decimal, Refusal> {
    decimal(required(value, key)?)
}

fn at<'a>(values: &'a [Value], index: &Value) -> Result<&'a Value, Refusal> {
    let index = crate::integer::integer(index)?;
    let text = index.to_string();
    let n = text
        .parse::<i128>()
        .map_err(|_| Refusal::new("IndexError", "cannot fit 'int' into an index-sized integer"))?;
    if n < isize::MIN as i128 || n > isize::MAX as i128 {
        return Err(Refusal::new(
            "IndexError",
            "cannot fit 'int' into an index-sized integer",
        ));
    }
    let n = if n < 0 { n + values.len() as i128 } else { n };
    usize::try_from(n)
        .ok()
        .and_then(|n| values.get(n))
        .ok_or_else(|| Refusal::new("IndexError", "list index out of range"))
}

fn row<'a>(rows: &'a HashMap<String, Value>, id: &Value) -> Result<&'a Value, Refusal> {
    ensure_hashable(id)?;
    Ok(id
        .as_str()
        .and_then(|id| rows.get(id))
        .unwrap_or(&Value::Null))
}

fn known_wallet<'a>(
    labels: &HashMap<String, &'a Value>,
    label: &Value,
    entry: bool,
) -> Result<Option<&'a Value>, Refusal> {
    ensure_hashable(label)?;
    let wallet = label.as_str().and_then(|s| labels.get(s)).copied();
    if entry && wallet.is_none() {
        let mut error = Refusal::new(
            "AppError",
            format!(
                "RP2 emitted transaction for unknown wallet '{}'",
                string(label)
            ),
        );
        error.code = Some("internal".into());
        return Err(error);
    }
    Ok(wallet)
}

fn head(
    inputs: &LedgerInputs<'_>,
    wallet: &Value,
    entry_type: &Value,
    asset: &Value,
    ids: &mut impl FnMut() -> String,
) -> Result<(), Refusal> {
    ensure_hashable(required(wallet, "id")?)?;
    ensure_hashable(entry_type)?;
    ensure_hashable(asset)?;
    required(inputs.profile, "id")?;
    ids();
    required(inputs.profile, "workspace_id")?;
    required(inputs.profile, "id")?;
    required(wallet, "id")?;
    required(wallet, "wallet_account_id")?;
    Ok(())
}

fn iso(value: &Value) -> Result<(), Refusal> {
    ensure_hashable(value)?;
    let timestamp = kassiber_custody::ledger_request::timestamp_fields(value);
    required(&timestamp, "iso")?;
    Ok(())
}

fn source(rows: &HashMap<String, Value>, facts: &Value) -> Result<(), Refusal> {
    if facts.is_null() {
        return Ok(());
    }
    let id = required(facts, "row")?;
    let source = row(rows, id)?;
    let crypto = dec_field(facts, "crypto_fee")?;
    iso(&source["occurred_at"])?;
    fixed(&dec_field(facts, "spot_price")?)?;
    fixed(&dec_field(facts, "fiat_fee")?)?;
    msat(&crypto)?;
    msat(&crypto)?;
    Ok(())
}

fn fragment(rows: &HashMap<String, Value>, value: &Value) -> Result<(), Refusal> {
    let amount = dec_field(value, "crypto_amount")?;
    let override_value = required(value, "unit_basis_override")?;
    let override_value = (!override_value.is_null())
        .then(|| decimal(override_value))
        .transpose()?;
    let lot = required(value, "lot")?;
    let crypto_in = truth(lot)
        .then(|| dec_field(lot, "crypto_in"))
        .transpose()?;
    msat(&amount)?;
    msat(&amount)?;
    for key in ["cost_basis", "proceeds", "gain_loss"] {
        fixed(&dec_field(value, key)?)?;
    }
    if let Some(value) = &override_value {
        fixed(value)?;
    }
    if truth(lot) {
        fixed(&dec_field(lot, "basis")?)?;
    }
    if let Some(value) = &crypto_in {
        msat(value)?;
        msat(value)?;
    }
    source(rows, required(value, "event")?)?;
    source(rows, lot)?;
    Ok(())
}

fn detail(value: &Value) -> Result<(), Refusal> {
    match value {
        Value::Object(items) if items.len() == 1 && items.contains_key("float") => {
            decimal(&items["float"])?;
        }
        Value::Object(items) => {
            for value in items.values() {
                detail(value)?;
            }
        }
        Value::Array(items) => {
            for value in items {
                detail(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn audit(inputs: &LedgerInputs<'_>, value: &Value) -> Result<Value, Refusal> {
    let pairs = iterable(required(inputs.projection, "intra_pairs")?)?;
    let pair = at(&pairs, required(value, "pair")?)?;
    let out = required(pair, "out")?;
    let inbound = required(pair, "in")?;
    let wallet = |leg: &Value| -> Result<&Value, Refusal> {
        let id = required(leg, "wallet_id")?;
        ensure_hashable(id)?;
        inputs
            .wallets
            .iter()
            .find(|wallet| field(wallet, "id") == id)
            .ok_or_else(|| Refusal::new("KeyError", repr(id)))
    };
    let from = wallet(out)?;
    let to = wallet(inbound)?;
    let spot = required(value, "spot_price")?;
    let mut rendered = json!({});
    for (key, leg, field) in [
        ("out_id", out, "id"),
        ("in_id", inbound, "id"),
        ("from_wallet_id", from, "id"),
        ("from_wallet_label", from, "label"),
        ("to_wallet_id", to, "id"),
        ("to_wallet_label", to, "label"),
    ] {
        rendered[key] = required(leg, field)?.clone();
    }
    get(out, "asset")?;
    rendered["asset"] = json!(string(get(out, "asset")?));
    rendered["occurred_at"] = required(out, "occurred_at")?.clone();
    required(out, "external_id")?;
    for (key, field) in [
        ("crypto_sent", "sent"),
        ("crypto_received", "received"),
        ("crypto_fee", "fee"),
    ] {
        rendered[key] = json!(float(&dec_field(value, field)?));
    }
    for field in ["sent_msat", "received_msat", "fee_msat"] {
        crate::integer::integer(required(value, field)?)?;
    }
    if !spot.is_null() {
        decimal(spot)?;
    }
    get(pair, "source")?;
    required(value, "at_regime")?;
    let flows = required(value, "regime_flows")?;
    if truth(flows) {
        for key in ["out", "in"] {
            let values = required(flows, key)?;
            if !values.is_object() {
                iterable(values)?;
            }
        }
    }
    required(value, "at_pool")?;
    Ok(rendered)
}

pub(crate) fn response_refusal(
    inputs: &LedgerInputs<'_>,
    response: &Value,
    mut ids: impl FnMut() -> String,
) -> Result<(), Refusal> {
    if get(response, "ok")? != &Value::Bool(true) {
        return Err(crate::boundary::ledger_error(fallback(
            get(response, "error")?,
            &json!({}),
        )));
    }
    let mut rows = HashMap::new();
    for value in iterable(required(inputs.projection, "rows")?)? {
        rows.insert(string(required(&value, "id")?), value);
    }
    for value in iterable(required(response, "carried_rows")?)? {
        let id = string(required(&value, "row")?);
        let original = rows
            .get_mut(&id)
            .ok_or_else(|| Refusal::new("KeyError", kassiber_pytext::repr_str(&id)))?;
        required(&value, "value")?;
        let rate = get(&value, "rate")?;
        if !rate.is_null() {
            match rate {
                Value::String(s) => {
                    kassiber_pytext::parse_float(s).map_err(|_| {
                        Refusal::new(
                            "ValueError",
                            format!("could not convert string to float: {}", repr(rate)),
                        )
                    })?;
                }
                Value::Number(_) | Value::Bool(_) => {}
                _ => {
                    return Err(Refusal::new(
                        "TypeError",
                        format!(
                            "float() argument must be a string or a real number, not '{}'",
                            kind(rate)
                        ),
                    ))
                }
            }
        }
        original["fiat_rate_exact"] = rate.clone();
        original["fiat_value_exact"] = required(&value, "value")?.clone();
    }
    let mut labels = HashMap::new();
    for wallet in inputs.wallets {
        let label = get(wallet, "label")?;
        if !label.is_null() && label != "" {
            labels.insert(string(required(wallet, "label")?), wallet);
        }
    }
    let raw_audits = iterable(required(response, "intra_audit")?)?;
    let audits = raw_audits
        .iter()
        .map(|value| audit(inputs, value))
        .collect::<Result<Vec<_>, _>>()?;
    for value in iterable(required(response, "entries")?)? {
        let entry_type = required(&value, "entry_type")?;
        if entry_type == "transfer_out" || entry_type == "transfer_in" {
            let index = required(&value, "audit")?;
            let rendered = at(&audits, index)?;
            let label = required(
                rendered,
                if entry_type == "transfer_out" {
                    "from_wallet_label"
                } else {
                    "to_wallet_label"
                },
            )?;
            let wallet = known_wallet(&labels, label, false)?
                .ok_or_else(|| Refusal::new("KeyError", repr(label)))?;
            head(
                inputs,
                wallet,
                entry_type,
                required(rendered, "asset")?,
                &mut ids,
            )?;
            required(at(&raw_audits, index)?, "description")?;
            continue;
        }
        let id = required(&value, "row")?;
        let row = row(&rows, id)?;
        let wallet = known_wallet(&labels, required(&value, "wallet")?, true)?
            .expect("entry wallet checked");
        dec_field(
            &value,
            if entry_type == "acquisition" {
                "fiat_value"
            } else {
                "proceeds"
            },
        )?;
        required(row, "occurred_at")?;
        let asset = required(&value, "asset")?;
        head(inputs, wallet, entry_type, asset, &mut ids)?;
        dec_field(&value, "quantity")?;
        if entry_type == "acquisition" {
            dec_field(&value, "unit_cost")?;
            required(&value, "description")?;
        } else {
            dec_field(&value, "cost_basis")?;
            dec_field(&value, "gain_loss")?;
            required(&value, "description")?;
            required(&value, "at_category")?;
            required(&value, "capital_gains_type")?;
            for value in iterable(required(&value, "fragments")?)? {
                fragment(&rows, &value)?;
            }
        }
    }
    let mut assets = Vec::<(HoldingKey, Vec<Value>)>::new();
    for value in iterable(required(response, "holdings")?)? {
        let asset = holding_key(required(&value, "asset")?)?;
        if let Some((_, items)) = assets.iter_mut().find(|(key, _)| *key == asset) {
            items.push(value);
        } else {
            assets.push((asset, vec![value]));
        }
    }
    for (_, values) in assets {
        for value in values {
            let Some(wallet) = known_wallet(&labels, required(&value, "wallet")?, false)? else {
                continue;
            };
            let quantity = dec_field(&value, "quantity")?;
            let basis = dec_field(&value, "cost_basis")?;
            for key in [
                "wallet_account_id",
                "account_code",
                "account_label",
                "id",
                "label",
            ] {
                required(wallet, key)?;
            }
            for key in ["wallet_account_id", "account_code", "account_label"] {
                ensure_hashable(required(wallet, key)?)?;
            }
            Decimal::zero().add(&quantity, &CTX)?;
            Decimal::zero().add(&basis, &CTX)?;
            ensure_hashable(required(wallet, "id")?)?;
            ensure_hashable(required(wallet, "label")?)?;
        }
    }
    iterable(required(inputs.projection, "quarantines")?)?;
    for value in iterable(required(response, "quarantines")?)? {
        let id = string(required(&value, "row")?);
        let row = rows
            .get(&id)
            .ok_or_else(|| Refusal::new("KeyError", kassiber_pytext::repr_str(&id)))?;
        let detail_kind = required(&value, "detail_kind")?;
        if !matches!(
            string(detail_kind).as_str(),
            "plain" | "partner_leg" | "pricing_review" | "privacy_hop" | "hold" | "price_currency"
        ) {
            let mut error = Refusal::new(
                "EngineContractError",
                format!(
                    "RequestError: unknown quarantine detail kind {}",
                    repr(detail_kind)
                ),
            );
            error.error_class = Some("RequestError".into());
            return Err(error);
        }
        let values = required(&value, "detail")?;
        let values = values.as_object().ok_or_else(|| {
            Refusal::new(
                "AttributeError",
                format!("'{}' object has no attribute 'items'", kind(values)),
            )
        })?;
        for value in values.values() {
            detail(value)?;
        }
        required(&value, "reason")?;
        required(inputs.profile, "workspace_id")?;
        required(inputs.profile, "id")?;
        required(row, "id")?;
    }
    direct_payouts(inputs)?;
    for value in iterable(required(response, "tax_summary")?)? {
        let quantity = dec_field(&value, "quantity")?;
        crate::integer::integer(required(&value, "year")?)?;
        for key in ["asset", "transaction_type", "long_term"] {
            required(&value, key)?;
        }
        msat(&quantity)?;
        for key in ["proceeds", "cost_basis", "gain_loss"] {
            dec_field(&value, key)?;
        }
    }
    Ok(())
}
