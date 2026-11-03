//! Desktop row presentation from stored projection relations and journal entries.
use super::*;

pub(super) fn pairs(
    conn: &Connection,
    raw: &[Value],
    current: bool,
) -> Result<BTreeMap<String, Value>> {
    let mut output = BTreeMap::new();
    if raw.is_empty() || !current {
        return Ok(output);
    }
    let terms = vec!["?"; raw.len()].join(",");
    let ids: Vec<_> = raw
        .iter()
        .map(|r| SqlValue::Text(text(r, "id").into()))
        .collect();
    let bindings: Vec<_> = ids.iter().chain(&ids).cloned().collect();
    let relations=rows(conn,&format!("SELECT relation.id AS pair_id,relation.kind,relation.policy,relation.swap_fee_msat AS fee_msat,relation.swap_fee_kind AS fee_kind,relation.out_transaction_id,relation.in_transaction_id,relation.out_asset,relation.out_amount AS out_amount_msat,relation.in_asset,relation.in_amount AS in_amount_msat,tout.fiat_rate AS out_fiat_rate,tin.fiat_rate AS in_fiat_rate,wout.label AS out_wallet,win.label AS in_wallet FROM journal_custody_projection_relations relation JOIN transactions tout ON tout.id=relation.out_transaction_id JOIN transactions tin ON tin.id=relation.in_transaction_id JOIN wallets wout ON wout.id=tout.wallet_id JOIN wallets win ON win.id=tin.wallet_id WHERE relation.relation_kind IN ('move','conversion') AND (relation.out_transaction_id IN ({terms}) OR relation.in_transaction_id IN ({terms})) ORDER BY relation.occurred_at,relation.id"),params_from_iter(bindings))?;
    for mut pair in relations {
        let swap = val(&pair, "out_asset") != val(&pair, "in_asset");
        set(
            &mut pair,
            "pair_type",
            s(if swap { "swap" } else { "transfer" }),
        );
        set(
            &mut pair,
            "label",
            s(if swap { "Swap" } else { "Transfer" }),
        );
        let rate = if val(&pair, "out_fiat_rate") != Value::Null {
            val(&pair, "out_fiat_rate")
        } else {
            val(&pair, "in_fiat_rate")
        };
        set(&mut pair, "display_rate", positive(&rate));
        for side in ["out", "in"] {
            let mut meta = pair.clone();
            set(&mut meta, "role", s(side));
            output
                .entry(text(&pair, &format!("{side}_transaction_id")).into())
                .or_insert(meta);
        }
    }
    let journal=rows(conn,&format!("SELECT jin.id AS in_entry_id,jout.id AS out_entry_id,jin.transaction_id AS in_transaction_id,jout.transaction_id AS out_transaction_id,jin.asset AS out_asset,jin.asset AS in_asset,ABS(jout.quantity) AS out_amount_msat,ABS(jin.quantity) AS in_amount_msat,tin.fiat_rate AS in_fiat_rate,tout.fiat_rate AS out_fiat_rate,win.label AS in_wallet,wout.label AS out_wallet FROM journal_entries jin JOIN journal_entries jout ON jout.profile_id=jin.profile_id AND jout.entry_type='transfer_out' AND jout.occurred_at=jin.occurred_at AND jout.asset=jin.asset AND jout.description=jin.description AND ABS(jout.quantity)=ABS(jin.quantity) JOIN wallets win ON win.id=jin.wallet_id JOIN wallets wout ON wout.id=jout.wallet_id LEFT JOIN transactions tin ON tin.id=jin.transaction_id LEFT JOIN transactions tout ON tout.id=jout.transaction_id WHERE jin.entry_type='transfer_in' AND jin.transaction_id IN ({terms}) ORDER BY jin.occurred_at ASC,jin.id ASC"),params_from_iter(ids))?;
    for mut pair in journal {
        let key = text(&pair, "in_transaction_id").to_owned();
        let pair_id = format!(
            "journal-transfer:{}:{}",
            text(&pair, "out_entry_id"),
            text(&pair, "in_entry_id")
        );
        set(&mut pair, "pair_id", s(pair_id));
        for (k, v) in [
            ("kind", "journal-derived"),
            ("policy", "carrying-value"),
            ("pair_type", "transfer"),
            ("label", "Transfer"),
            ("role", "in"),
        ] {
            set(&mut pair, k, s(v));
        }
        set(&mut pair, "fee_msat", Value::Int(0.into()));
        set(&mut pair, "fee_kind", Value::Null);
        let rate = if val(&pair, "out_fiat_rate") != Value::Null {
            val(&pair, "out_fiat_rate")
        } else {
            val(&pair, "in_fiat_rate")
        };
        set(&mut pair, "display_rate", positive(&rate));
        output.entry(key).or_insert(pair);
    }
    Ok(output)
}
pub(super) fn positive(raw: &Value) -> Value {
    if *raw == Value::Null {
        return Value::Null;
    }
    match kassiber_pytext::parse_float(&py_str(raw)) {
        Ok(n) if n > 0.0 => Value::Float(n),
        _ => Value::Null,
    }
}
pub(super) fn sat(msat: i64) -> Value {
    if msat % 1000 == 0 {
        Value::Int((msat / 1000).into())
    } else {
        // Python's int true division rounds the rational once, before JSON encoding.
        Value::Float(kassiber_pytext::parse_float(&format!("{msat}e-3")).unwrap())
    }
}
fn btc(msat: i64) -> f64 {
    kassiber_pytext::parse_float(&format!("{msat}e-11")).unwrap()
}
fn fiat(msat: i64, rate: &Value) -> Value {
    match rate {
        Value::Float(n) => Value::Float(-btc(msat) * n),
        _ => Value::Null,
    }
}
fn type_label(row: &Value) -> &'static str {
    let kind = if truthy(&val(row, "kind_override")) {
        text(row, "kind_override")
    } else {
        text(row, "kind")
    };
    let kind = lower(kind);
    let reason = lower(text(row, "quarantine_reason"));
    let inbound = text(row, "direction") == "inbound";
    if kind.contains("transfer") && !inbound {
        return "Transfer";
    }
    if kind.contains("swap") {
        return "Swap";
    }
    if kind.contains("fee") || reason == "missing_fee_price" {
        return "Fee";
    }
    if ["transfer", "pair", "swap"]
        .iter()
        .any(|t| reason.contains(t))
    {
        return "Transfer";
    }
    if inbound {
        match kind.as_str() {
            "buy" => "Buy",
            "deposit" => "Deposit",
            "income" | "routing_income" => "Income",
            "wages" => "Wages",
            "mining" | "mining_reward" => "Mining",
            "staking" => "Staking",
            "interest" | "lending_interest" => "Interest",
            "airdrop" => "Airdrop",
            "hardfork" | "hard_fork" => "Hard fork",
            "lnd_invoice" | "cln_invoice" => "LN invoice",
            "channel_close" => "Channel close",
            _ => "Acquired",
        }
    } else {
        match kind.as_str() {
            "sell" => "Sell",
            "withdrawal" => "Withdrawal",
            "spend" => "Spend",
            "gift" => "Gift",
            "donation" => "Donation",
            "lost" => "Lost",
            "stolen" => "Stolen",
            "lnd_pay" | "cln_pay" => "LN payment",
            "channel_open" => "Channel open",
            _ => "Expense",
        }
    }
}
fn status(row: &Value) -> &'static str {
    match lower(strip(text(row, "review_status")))
        .replace('-', "_")
        .as_str()
    {
        _ if truthy(&val(row, "quarantine_reason")) => "review",
        "review" | "needs_review" | "blocked" | "quarantined" => "review",
        "failed" | "error" => "failed",
        "completed" | "complete" => "completed",
        "pending" => "pending",
        _ if truthy(&val(row, "confirmed_at")) => "completed",
        _ => "pending",
    }
}
fn chain_network(row: &Value) -> (String, String) {
    let config = kassiber_pytext::loads(text(row, "wallet_config_json")).unwrap_or(Value::Null);
    let fallback = if ["LBTC", "L-BTC"].contains(&upper(text(row, "asset")).as_str())
        || lower(text(row, "wallet_kind")).contains("liquid")
    {
        "liquid"
    } else {
        "bitcoin"
    };
    let configured = val(&config, "chain");
    let chain = if !truthy(&configured) {
        fallback
    } else {
        match lower(strip(&py_str(&configured))).as_str() {
            "bitcoin" | "btc" | "onchain" | "" => "bitcoin",
            "liquid" | "lbtc" | "elements" => "liquid",
            _ => fallback,
        }
    };
    let configured = val(&config, "network");
    let raw = if truthy(&configured) {
        lower(strip(&py_str(&configured)))
    } else {
        String::new()
    };
    let network = if chain == "bitcoin" {
        match raw.as_str() {
            "test" | "testnet" => "test",
            "regtest" => "regtest",
            "signet" => "signet",
            _ => "main",
        }
    } else {
        match raw.as_str() {
            "liquidtestnet" | "test" | "testnet" => "liquidtestnet",
            "elements" | "elementsregtest" | "regtest" => "elementsregtest",
            _ => "liquidv1",
        }
    };
    (chain.into(), network.into())
}
pub(super) fn row(conn: &Connection, row: &Value, pair: Option<&Value>) -> Result<Value> {
    let tags=rows(conn,"SELECT tags.label FROM transaction_tags tt JOIN tags ON tags.id=tt.tag_id WHERE tt.transaction_id=? ORDER BY tags.code ASC",[text(row,"id")])?;
    let tags: Vec<_> = tags
        .iter()
        .map(|t| val(t, "label"))
        .filter(truthy)
        .collect();
    Ok(present(row, pair, tags))
}
/// Activity chart rows skip metadata tags, as Python's uncapped overview does.
pub(super) fn untagged(row: &Value, pair: Option<&Value>) -> Value {
    present(row, pair, Vec::new())
}
fn present(row: &Value, pair: Option<&Value>, tags: Vec<Value>) -> Value {
    let mut display: Vec<String> = tags.iter().map(py_str).collect();
    let mut rate = positive(&val(row, "fiat_rate"));
    let fee = num(row, "fee");
    let sign = if text(row, "direction") == "inbound" {
        1
    } else {
        -1
    };
    let mut amount = sign * num(row, "amount");
    let mut eur = match positive(&val(row, "fiat_value")) {
        Value::Float(n) => Value::Float(sign as f64 * n.abs()),
        _ => Value::Null,
    };
    let mut kind = type_label(row);
    let mut account = text(row, "wallet").to_owned();
    let description = text(row, "description");
    let description = if description.starts_with("Synced from ")
        || description.starts_with("Silent Payment sync from ")
    {
        ""
    } else {
        description
    };
    let mut counter = [text(row, "counterparty"), description, text(row, "note")]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_owned();
    let mut internal = lower(text(row, "kind")) == "transfer";
    let mut fee_sat = sat(fee);
    if let Some(pair) = pair {
        let label = text(pair, "label");
        let pair_fee = num(pair, "fee_msat").abs();
        let swap = text(pair, "pair_type") == "swap";
        rate = positive(&val(pair, "display_rate"));
        fee_sat = sat(pair_fee);
        amount = if swap {
            -num(pair, "out_amount_msat")
                .abs()
                .max(num(pair, "in_amount_msat").abs())
        } else {
            -pair_fee
        };
        eur = fiat(amount.abs(), &rate);
        kind = if swap { "Swap" } else { "Transfer" };
        internal = !swap;
        account = format!(
            "{} -> {}",
            text(pair, "out_wallet"),
            text(pair, "in_wallet")
        );
        counter = if swap {
            format!(
                "Swap {} -> {}",
                text(pair, "out_asset"),
                text(pair, "in_asset")
            )
        } else if text(pair, "kind") == "journal-derived" {
            format!(
                "Transfer {} -> {}",
                text(pair, "out_wallet"),
                text(pair, "in_wallet")
            )
        } else {
            format!(
                "Transfer fee - {} -> {}",
                text(pair, "out_asset"),
                text(pair, "in_asset")
            )
        };
        display.retain(|t| t != label);
        display.insert(0, label.into());
    } else {
        if text(row, "direction") == "outbound" && num(row, "amount") == 0 && fee > 0 {
            amount = -fee;
            let calculated = fiat(fee, &rate);
            if calculated != Value::Null {
                eur = calculated;
            }
            kind = "Fee";
        }
        if display.is_empty() {
            display.push(if truthy(&val(row, "quarantine_reason")) {
                "Review".into()
            } else if kind != "Expense" {
                kind.into()
            } else if !text(row, "kind").is_empty() {
                text(row, "kind").into()
            } else {
                text(row, "direction").into()
            });
        }
    }
    let external = text(row, "external_id");
    let trimmed = strip(external);
    let explorer = if trimmed.len() == 64 && trimmed.bytes().all(|c| c.is_ascii_hexdigit()) {
        s(trimmed)
    } else {
        Value::Null
    };
    let (chain, network) = chain_network(row);
    let date: String = text(row, "occurred_at")
        .chars()
        .take(16)
        .collect::<String>()
        .replace('T', " ");
    let mut output = dict([
        ("id", val(row, "id")),
        ("externalId", val(row, "external_id")),
        ("explorerId", explorer),
        ("date", s(date)),
        ("type", s(kind)),
        ("kind", val(row, "kind")),
        ("kindOverride", val(row, "kind_override")),
        ("asset", val(row, "asset")),
        ("chain", s(chain)),
        ("network", s(network)),
        ("paymentMethod", val(row, "payment_method")),
        ("account", s(account)),
        ("counter", s(counter)),
        ("amountSat", sat(amount)),
        ("feeSat", fee_sat.clone()),
        ("eur", eur),
        ("rate", rate),
        ("fiatCurrency", nullable(text(row, "fiat_currency"))),
        (
            "pricingSourceKind",
            nullable(text(row, "pricing_source_kind")),
        ),
        ("pricingQuality", nullable(text(row, "pricing_quality"))),
        (
            "pricingExternalRef",
            nullable(text(row, "pricing_external_ref")),
        ),
        ("pricingProvider", val(row, "pricing_provider")),
        ("pricingPair", val(row, "pricing_pair")),
        ("pricingTimestamp", val(row, "pricing_timestamp")),
        ("pricingFetchedAt", val(row, "pricing_fetched_at")),
        ("pricingGranularity", val(row, "pricing_granularity")),
        ("pricingMethod", val(row, "pricing_method")),
        ("reviewStatus", val(row, "review_status")),
        ("queueStatus", s(status(row))),
        (
            "taxable",
            if val(row, "taxability_override") == Value::Null {
                Value::Null
            } else {
                Value::Bool(truthy(&val(row, "taxability_override")))
            },
        ),
        ("atRegime", val(row, "at_regime_override")),
        ("atCategory", val(row, "at_category_override")),
        (
            "tag",
            s(if display.is_empty() {
                "Unlabeled".into()
            } else {
                display.join(", ")
            }),
        ),
        ("note", s(text(row, "note"))),
        ("excluded", Value::Bool(truthy(&val(row, "excluded")))),
        (
            "conf",
            Value::Int(i64::from(truthy(&val(row, "confirmed_at"))).into()),
        ),
        ("internal", Value::Bool(internal)),
    ]);
    if !tags.is_empty() || pair.is_none() {
        set(&mut output, "tags", Value::List(tags));
    }
    if let Some(pair) = pair {
        set(
            &mut output,
            "pair",
            dict([
                ("id", val(pair, "pair_id")),
                ("type", val(pair, "pair_type")),
                ("kind", val(pair, "kind")),
                ("policy", val(pair, "policy")),
                ("outWallet", val(pair, "out_wallet")),
                ("outAsset", val(pair, "out_asset")),
                ("outAmountSat", sat(num(pair, "out_amount_msat"))),
                ("inWallet", val(pair, "in_wallet")),
                ("inAsset", val(pair, "in_asset")),
                ("inAmountSat", sat(num(pair, "in_amount_msat"))),
                ("feeSat", fee_sat),
                ("feeKind", val(pair, "fee_kind")),
            ]),
        );
    }
    if truthy(&val(row, "quarantine_reason")) {
        set(
            &mut output,
            "quarantineReason",
            val(row, "quarantine_reason"),
        );
    }
    if truthy(&val(row, "occurred_at")) {
        set(&mut output, "occurredAt", val(row, "occurred_at"));
    }
    output
}
