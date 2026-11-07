use super::*;
use artifact_csv::list;
use kassiber_pyjson::BigInt;
use std::collections::{BTreeMap, BTreeSet};

type Key = (String, String);
type Holdings = BTreeMap<Key, Holding>;
struct Holding {
    stored: BigInt,
    basis: Decimal,
    ordinal: usize,
}
#[derive(Default)]
pub(super) struct Facts {
    pub(super) types: BTreeSet<String>,
    pub(super) categories: BTreeSet<String>,
    amounts: BTreeMap<String, Decimal>,
}
#[derive(Default)]
struct Movement {
    units: BTreeMap<String, BigInt>,
    money: BTreeMap<String, Decimal>,
}
#[derive(Default)]
struct Activity {
    count: usize,
    first: String,
    last: String,
    assets: BTreeSet<String>,
}
#[derive(Clone)]
pub(super) struct Asset {
    pub(super) id: String,
    pub(super) code: String,
    pub(super) chain: &'static str,
    bitcoin: bool,
}
impl Default for Holding {
    fn default() -> Self {
        Self {
            stored: 0.into(),
            basis: Decimal::zero(),
            ordinal: 0,
        }
    }
}
impl Asset {
    pub(super) fn new(raw: &str) -> Self {
        let code = kassiber_pytext::strip(raw);
        let upper = kassiber_pytext::upper(code);
        let (id, code, chain, bitcoin) = if upper == "BTC" {
            ("bitcoin:btc".into(), "BTC".into(), "bitcoin", true)
        } else if ["LBTC", "L-BTC"].contains(&upper.as_str()) {
            ("liquid:lbtc".into(), "L-BTC".into(), "liquid", true)
        } else if code.len() == 64 && code.bytes().all(|c| c.is_ascii_hexdigit()) {
            let code = code.to_lowercase();
            (
                format!("liquid:{code}"),
                format!("{}…", &code[..8]),
                "liquid",
                false,
            )
        } else {
            let id = if code.contains(':') {
                code.into()
            } else {
                upper
            };
            let code = kassiber_pytext::upper(id.rsplit(':').next().unwrap());
            (id, code, "", false)
        };
        Self {
            id,
            code,
            chain,
            bitcoin,
        }
    }
    fn fields(&self) -> Value {
        dict([
            ("asset", s(self.code.as_str())),
            ("asset_id", s(self.id.as_str())),
            ("_decimals", Value::Int(11.into())),
        ])
    }
    fn quantity(&self, units: &BigInt) -> Result<Decimal> {
        Decimal::from_parts(units < &BigInt::from(0), units.magnitude().clone(), -11)
            .map_err(arithmetic)
    }
    pub(super) fn exact(&self, units: &BigInt) -> Result<String> {
        let raw = self
            .quantity(units)?
            .to_fixed_places(11, Rounding::HalfEven)
            .map_err(arithmetic)?;
        let (whole, fraction) = raw.split_once('.').unwrap();
        Ok(format!(
            "{whole}.{fraction:0<8}",
            fraction = fraction.trim_end_matches('0')
        ))
    }
    fn add_quantity(&self, row: &mut Value, key: &str, units: &BigInt) -> Result<()> {
        replace(row, key, Value::Decimal(self.quantity(units)?));
        replace(row, &format!("{key}_exact"), s(self.exact(units)?));
        Ok(())
    }
}
fn number(row: &Value, key: &str) -> BigInt {
    match value(row, key) {
        Value::Int(n) => n.clone(),
        _ => 0.into(),
    }
}
fn optional(row: &Value, key: &str) -> Result<Option<Decimal>> {
    let exact = value(row, &format!("{key}_exact"));
    let raw = if matches!(exact, Value::Null) || exact == &s("") {
        value(row, key)
    } else {
        exact
    };
    if matches!(raw, Value::Null) || raw == &s("") {
        Ok(None)
    } else {
        decimal(raw).map(Some)
    }
}
fn money(value: Option<&Decimal>) -> Result<Value> {
    value
        .map(float)
        .transpose()
        .map(|v| v.unwrap_or(Value::Null))
}
fn add(map: &mut BTreeMap<String, Decimal>, key: &str, amount: Option<&Decimal>) -> Result<()> {
    if let Some(amount) = amount {
        let current = map.entry(key.into()).or_insert_with(Decimal::zero);
        *current = current.add(amount, &DECIMAL).map_err(arithmetic)?;
    }
    Ok(())
}
fn joined(values: &BTreeSet<String>) -> String {
    values.iter().cloned().collect::<Vec<_>>().join(", ")
}
fn merge(row: &mut Value, fields: Value) {
    if let Value::Dict(fields) = fields {
        for (key, v) in fields {
            let kassiber_pyjson::Key::Str(key) = key else {
                unreachable!()
            };
            replace(row, key.as_str().unwrap(), v);
        }
    }
}
struct Period {
    year: i64,
    at: bool,
    start: String,
    end: String,
}
impl Period {
    fn new(profile: &Value, year: i64) -> Self {
        let at = text(profile, "tax_country").to_lowercase() == "at";
        let boundary = |year| {
            if at {
                format!("{}-12-31T23:00:00Z", year - 1)
            } else {
                format!("{year}-01-01T00:00:00Z")
            }
        };
        Self {
            year,
            at,
            start: boundary(year),
            end: boundary(year + 1),
        }
    }
    fn calendar(&self) -> &'static str {
        if self.at {
            "Europe/Vienna"
        } else {
            "UTC"
        }
    }
    fn date(&self, raw: &str) -> Result<(i64, String)> {
        if self.at {
            return calendar::vienna(raw);
        }
        let at = kassiber_rates::time::parse(raw)
            .filter(|_| raw == kassiber_pytext::strip(raw))
            .ok_or_else(|| {
                AppError::new(
                    "internal_error",
                    format!(
                        "Invalid isoformat string: {}",
                        kassiber_pytext::repr_str(raw)
                    ),
                )
            })?;
        let date = kassiber_rates::time::format(at)
            .ok_or_else(|| AppError::new("internal_error", "date value out of range"))?;
        Ok((date[..4].parse().unwrap(), date[..10].into()))
    }
    fn position(&self, raw: &str) -> Result<i64> {
        Ok((self.date(raw)?.0 - self.year).signum())
    }
}
fn apply(
    holding: &mut Holding,
    row: &Value,
    amounts: &BTreeMap<String, Option<Decimal>>,
) -> Result<()> {
    let quantity = number(row, "quantity");
    let kind = text(row, "entry_type");
    if !["income", "transfer_fee"].contains(&kind.as_str()) {
        holding.stored += &quantity;
    }
    if kind != "income" {
        if quantity >= 0.into() {
            holding.basis = holding
                .basis
                .add(amounts["fiat_value"].as_ref().unwrap(), &DECIMAL)
                .map_err(arithmetic)?;
        } else {
            holding.basis = holding
                .basis
                .sub(
                    amounts["cost_basis"].as_ref().unwrap_or(&Decimal::zero()),
                    &DECIMAL,
                )
                .map_err(arithmetic)?;
        }
    }
    Ok(())
}
fn apply_to(
    holdings: &mut Holdings,
    key: Key,
    row: &Value,
    amounts: &BTreeMap<String, Option<Decimal>>,
) -> Result<()> {
    let ordinal = holdings.len();
    let holding = holdings.entry(key).or_insert_with(|| Holding {
        ordinal,
        ..Default::default()
    });
    apply(holding, row, amounts)
}
fn allocate(holdings: &Holdings) -> Result<(BTreeMap<Key, Decimal>, BTreeMap<String, Decimal>)> {
    let mut quantities: BTreeMap<String, BigInt> = BTreeMap::new();
    let mut bases: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut ordered: Vec<_> = holdings.iter().collect();
    ordered.sort_by_key(|(_, holding)| holding.ordinal);
    for ((_, asset), holding) in ordered {
        *quantities.entry(asset.clone()).or_default() += &holding.stored;
        add(&mut bases, asset, Some(&holding.basis))?;
    }
    let mut allocated = BTreeMap::new();
    for (key, holding) in holdings {
        let asset = &key.1;
        let basis = if holding.stored > 0.into() && quantities[asset] > 0.into() {
            bases[asset]
                .mul(&decimal(&s(holding.stored.to_string()))?, &DECIMAL)
                .map_err(arithmetic)?
                .div(&decimal(&s(quantities[asset].to_string()))?, &DECIMAL)
                .map_err(arithmetic)?
        } else {
            Decimal::zero()
        };
        allocated.insert(key.clone(), basis);
    }
    Ok((allocated, bases))
}
fn quarantine(conn: &Connection, profile: &Value, period: &Period) -> Result<()> {
    let candidates = rows(conn, "SELECT jq.transaction_id,jq.reason,t.occurred_at,COALESCE(t.external_id,'') AS external_id,w.label AS wallet FROM journal_quarantines jq JOIN transactions t ON t.id=jq.transaction_id JOIN wallets w ON w.id=t.wallet_id WHERE jq.profile_id=? ORDER BY t.occurred_at,jq.transaction_id", [text(profile, "id")])?;
    let mut blocking = Vec::new();
    for row in candidates {
        if period.position(&text(&row, "occurred_at"))? <= 0 {
            blocking.push(row);
        }
    }
    if blocking.is_empty() {
        return Ok(());
    }
    Err(AppError::new("year_handoff_quarantined", format!("The {} handoff is blocked by quarantined transactions.", period.year))
        .with_hint(format!("Resolve or exclude every quarantined transaction dated in or before {}, then process journals again. A quarantined row is missing from the journal, so balances would not add up.", period.year))
        .with_details(dict([("year", Value::Int(period.year.into())), ("count", Value::Int(blocking.len().into())), ("transactions", Value::List(blocking.iter().take(20).map(|r| dict([
            ("transaction_ref", value(r,"transaction_id").clone()), ("transaction_id", value(r,"external_id").clone()), ("wallet", value(r,"wallet").clone()), ("occurred_at",value(r,"occurred_at").clone()), ("reason",value(r,"reason").clone())
        ])).collect()))])))
}
fn rates(
    conn: &Connection,
    profile: &Value,
    period: &Period,
    assets: &BTreeSet<String>,
    spellings: &BTreeMap<String, BTreeSet<String>>,
) -> Result<BTreeMap<String, Value>> {
    let mut result = BTreeMap::new();
    let cutoff = kassiber_rates::time::parse(&period.end).unwrap() - 1_000_000;
    for asset in assets {
        let spellings = &spellings[asset];
        let pair = spellings.iter().find_map(|s| {
            kassiber_rates::transaction_rate_pair(s, &text(profile, "fiat_currency"))
        });
        if let Some(pair) = pair {
            let cached = sql::optional_rows(conn, "SELECT timestamp,rate,rate_exact,source FROM rates_cache WHERE pair=? ORDER BY timestamp DESC,CASE WHEN source='manual' THEN 0 ELSE 1 END,fetched_at DESC", [&pair])?
                .into_iter().find(|r| kassiber_rates::time::parse(&text(r,"timestamp")).is_some_and(|ts| ts <= cutoff));
            if let Some(row) = cached {
                if let Some(rate) = optional(&row, "rate")? {
                    result.insert(
                        asset.clone(),
                        dict([
                            ("rate", s(rate.to_string())),
                            (
                                "source",
                                if truthy(value(&row, "source")) {
                                    value(&row, "source").clone()
                                } else {
                                    s("rates cache")
                                },
                            ),
                            ("timestamp", value(&row, "timestamp").clone()),
                        ]),
                    );
                    continue;
                }
            }
        }
        let params = std::iter::once(text(profile, "id"))
            .chain(spellings.iter().cloned())
            .chain(std::iter::once(period.end.clone()));
        let sql = format!("SELECT occurred_at,amount,fiat_rate,fiat_rate_exact,fiat_value,fiat_value_exact FROM transactions WHERE profile_id=? AND excluded=0 AND asset IN ({}) AND occurred_at<? AND (fiat_rate IS NOT NULL OR fiat_value IS NOT NULL) ORDER BY occurred_at DESC,created_at DESC,id DESC LIMIT 1", vec!["?";spellings.len()].join(","));
        if let Some(row) = rows(
            conn,
            &sql,
            kassiber_store::rusqlite::params_from_iter(params),
        )?
        .first()
        {
            let rate = optional(row, "fiat_rate")?;
            let cash = optional(row, "fiat_value")?;
            let rate = match (rate, cash) {
                (None, Some(cash)) if truthy(value(row, "amount")) => Some(
                    cash.div(
                        &Asset::new(spellings.first().unwrap()).quantity(&number(row, "amount"))?,
                        &DECIMAL,
                    )
                    .map_err(arithmetic)?,
                ),
                (rate, _) => rate,
            };
            if let Some(rate) = rate {
                result.insert(
                    asset.clone(),
                    dict([
                        ("rate", s(rate.to_string())),
                        ("source", s("transaction price")),
                        ("timestamp", value(row, "occurred_at").clone()),
                    ]),
                );
            }
        }
    }
    Ok(result)
}

pub(super) fn build(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    require_context(conn, &profile, false)?;
    export_request::freshness(conn, &profile)?;
    if matches!(value(args, "year"), Value::Null) {
        return Err(AppError::new(
            "validation",
            "--year is required for Austrian E 1kv export",
        ));
    }
    let period = Period::new(&profile, austrian::normalize_tax_year(value(args, "year"))?);
    quarantine(conn, &profile, &period)?;
    let id = text(&profile, "id");
    let observed = rows(conn,"SELECT t.*,w.label AS wallet,COALESCE(t.external_id,'') AS external_id FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at,t.created_at,t.id",[&id])?;
    let observed_map: BTreeMap<_, _> = observed.iter().map(|r| (text(r, "id"), r)).collect();
    let mut assets = BTreeMap::new();
    let mut spellings: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut asset = |raw: &str| {
        let asset = Asset::new(raw);
        spellings
            .entry(asset.id.clone())
            .or_default()
            .insert(raw.into());
        assets
            .entry(asset.id.clone())
            .or_insert_with(|| asset.clone())
            .clone()
    };
    let wallet_list = kassiber_wallets::execute(
        conn,
        kassiber_wallets::Operation::List {
            profile_id: id.clone(),
        },
    )?;
    let configs = rows(
        conn,
        "SELECT id,config_json FROM wallets WHERE profile_id=?",
        [&id],
    )?;
    let configs: BTreeMap<_, _> = configs
        .iter()
        .map(|r| {
            (
                text(r, "id"),
                kassiber_pytext::loads(&text(r, "config_json")).unwrap_or(Value::Null),
            )
        })
        .collect();
    let mut wallets = BTreeMap::new();
    for wallet in list(&wallet_list) {
        let sync = value(
            configs.get(&text(wallet, "id")).unwrap_or(&Value::Null),
            "last_synced_at",
        );
        wallets.insert(
            text(wallet, "label"),
            dict([
                ("wallet_kind", value(wallet, "kind").clone()),
                ("chain", value(wallet, "chain").clone()),
                ("network", value(wallet, "network").clone()),
                (
                    "status",
                    s(if truthy(value(wallet, "deprecated")) {
                        "deprecated"
                    } else {
                        "active"
                    }),
                ),
                (
                    "last_synced_at",
                    if matches!(sync, Value::Str(_)) {
                        sync.clone()
                    } else {
                        s("")
                    },
                ),
            ]),
        );
    }
    let journal = rows(conn,"SELECT je.*,COALESCE(t.external_id,'') AS external_id,w.label AS wallet,CASE WHEN je.entry_type IN ('disposal','income') AND COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap' THEN 1 ELSE 0 END AS in_capital_gains_report FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? ORDER BY je.occurred_at,je.created_at,je.id",[&id])?;
    let mut opening = Holdings::new();
    let mut closing = Holdings::new();
    let mut roll: BTreeMap<String, Movement> = BTreeMap::new();
    let mut facts: BTreeMap<String, Facts> = BTreeMap::new();
    let mut timing: BTreeMap<Key, Vec<String>> = BTreeMap::new();
    let mut journal_rows = Vec::new();
    for row in &journal {
        let occurred = text(row, "occurred_at");
        let position = period.position(&occurred)?;
        let kind = text(row, "entry_type");
        let tx = text(row, "transaction_id");
        if let Some(observed) = observed_map.get(&tx) {
            if !["income", "transfer_fee"].contains(&kind.as_str()) {
                let other = period.position(&text(observed, "occurred_at"))?;
                if (position < 0) != (other < 0) || (position > 0) != (other > 0) {
                    let a = asset(&text(row, "asset"));
                    timing
                        .entry((text(row, "wallet"), a.id))
                        .or_default()
                        .push(format!(
                            "{}: booked {}, observed {}",
                            if truthy(value(observed, "external_id")) {
                                text(observed, "external_id")
                            } else {
                                tx.clone()
                            },
                            period.date(&occurred)?.1,
                            period.date(&text(observed, "occurred_at"))?.1
                        ));
                }
            }
        }
        let fact = facts.entry(tx).or_default();
        fact.types.insert(
            if kind == "disposal" && text(row, "at_category") == "neu_swap" {
                "neutral_disposal".into()
            } else {
                kind.clone()
            },
        );
        if truthy(value(row, "at_category")) {
            fact.categories.insert(text(row, "at_category"));
        }
        if position > 0 {
            continue;
        }
        let mut amounts = ["fiat_value", "cost_basis", "proceeds", "gain_loss"]
            .into_iter()
            .map(|key| Ok((key.to_string(), optional(row, key)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        if amounts["fiat_value"].is_none() {
            amounts.insert("fiat_value".into(), Some(Decimal::zero()));
        }
        let a = asset(&text(row, "asset"));
        let key = (text(row, "wallet"), a.id.clone());
        apply_to(&mut closing, key.clone(), row, &amounts)?;
        if position < 0 {
            apply_to(&mut opening, key, row, &amounts)?;
            continue;
        }
        let movement = roll.entry(a.id.clone()).or_default();
        let stored = number(row, "quantity");
        let magnitude = if stored < 0.into() {
            -stored.clone()
        } else {
            stored.clone()
        };
        let quantity_key = match kind.as_str() {
            "acquisition" => Some("acquired"),
            "disposal" => Some("disposed"),
            "fee" | "transfer_fee" => Some("fee"),
            "transfer_in" | "transfer_out" => Some("transfer_net"),
            "income" => None,
            _ => {
                return Err(AppError::new(
                    "year_handoff_unsupported_entry",
                    format!("Unknown journal entry type '{kind}' in the year handoff."),
                )
                .with_details(dict([
                    ("journal_entry_id", value(row, "id").clone()),
                    ("entry_type", s(kind)),
                ])))
            }
        };
        if let Some(key) = quantity_key {
            *movement.units.entry(key.into()).or_default() += if kind == "transfer_out" {
                -magnitude.clone()
            } else {
                magnitude.clone()
            };
        }
        let money_keys: &[(&str, &str)] = match kind.as_str() {
            "acquisition" => &[("acquired_value", "fiat_value")],
            "income" => &[("income_value", "fiat_value")],
            "disposal" => &[
                ("disposed_cost_basis", "cost_basis"),
                ("disposal_proceeds", "proceeds"),
                ("realized_gain_loss", "gain_loss"),
            ],
            "fee" | "transfer_fee" => &[("fee_cost_basis", "cost_basis")],
            "transfer_in" => &[("transfer_basis_net", "fiat_value")],
            "transfer_out" => &[("transfer_basis_net", "cost_basis")],
            _ => unreachable!(),
        };
        for (target, source) in money_keys {
            let amount = amounts[*source].clone().unwrap_or_else(Decimal::zero);
            let amount = if kind == "transfer_out" {
                amount.neg(&DECIMAL).map_err(arithmetic)?
            } else {
                amount
            };
            add(&mut movement.money, target, Some(&amount))?;
        }
        if kind == "transfer_fee" {
            *movement.units.entry("transfer_net".into()).or_default() += &magnitude;
        }
        let fact_keys: &[(&str, &str)] = match kind.as_str() {
            "income" => &[("income_value", "fiat_value")],
            "disposal" => &[
                ("disposal_proceeds", "proceeds"),
                ("disposal_cost_basis", "cost_basis"),
                ("disposal_gain_loss", "gain_loss"),
            ],
            "fee" | "transfer_fee" => &[("fee_cost_basis", "cost_basis")],
            _ => &[],
        };
        for (target, source) in fact_keys {
            add(&mut fact.amounts, target, amounts[*source].as_ref())?;
        }
        let mut output = a.fields();
        for key in [
            "occurred_at",
            "wallet",
            "entry_type",
            "capital_gains_type",
            "pricing_source_kind",
            "pricing_quality",
        ] {
            replace(
                &mut output,
                key,
                if truthy(value(row, key)) {
                    value(row, key).clone()
                } else {
                    s("")
                },
            );
        }
        replace(&mut output, "local_date", s(period.date(&occurred)?.1));
        replace(
            &mut output,
            "transaction_ref",
            value(row, "transaction_id").clone(),
        );
        replace(
            &mut output,
            "transaction_id",
            value(row, "external_id").clone(),
        );
        a.add_quantity(&mut output, "quantity", &stored)?;
        for (key, amount) in &amounts {
            replace(&mut output, key, money(amount.as_ref())?);
        }
        replace(
            &mut output,
            "in_capital_gains_report",
            value(row, "in_capital_gains_report").clone(),
        );
        replace(
            &mut output,
            "tax_category",
            if truthy(value(row, "at_category")) {
                value(row, "at_category").clone()
            } else {
                s("")
            },
        );
        // No crypto field before 1 March 2022, as Reports states it.
        let form_field =
            austrian::stated_kennzahl(value(row, "at_kennzahl"), &text(row, "occurred_at"))?;
        replace(
            &mut output,
            "tax_form_field",
            if matches!(form_field, Value::Null) {
                s("")
            } else {
                form_field
            },
        );
        let pricing_source = value(&output, "pricing_source_kind").clone();
        replace(&mut output, "pricing_source", pricing_source);
        replace(
            &mut output,
            "description",
            s((if truthy(value(row, "description")) {
                text(row, "description")
            } else {
                String::new()
            })
            .split(|c: char| kassiber_pytext::is_space(c as u32))
            .filter(|t| !t.is_empty() && !(t.starts_with("at_") && t.contains('=')))
            .collect::<Vec<_>>()
            .join(" ")),
        );
        replace(&mut output, "journal_entry_id", value(row, "id").clone());
        journal_rows.push(output);
    }
    let mut links: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    let mut swaps = BTreeSet::new();
    for row in rows(conn,"SELECT * FROM journal_custody_projection_relations WHERE profile_id=? AND relation_kind IN ('move','conversion','direct_payout')",[&id])? {
        let out = text(&row,"out_transaction_id"); let incoming = text(&row,"in_transaction_id");
        if text(&row,"relation_kind") != "move" && [&out,&incoming].iter().any(|id| facts.get(*id).is_some_and(|f| f.categories.contains("neu_swap"))) {
            for key in ["out_transaction_id","in_transaction_id"] {if truthy(value(&row,key)) {swaps.insert(text(&row,key));}}
        }
        let reference: String = text(&row,"id").chars().take(12).collect();
        for (key,other) in [("out_transaction_id","in_transaction_id"),("in_transaction_id","out_transaction_id")] {
            if !truthy(value(&row,key)) {continue;}
            let link = links.entry(text(&row,key)).or_default(); link.0.insert(reference.clone());
            let other_id = text(&row,other);
            let label = if truthy(value(&row,other)) {observed_map.get(&other_id).map(|r| text(r,"external_id")).filter(|s| !s.is_empty()).unwrap_or(other_id)} else if key == "out_transaction_id" && truthy(value(&row,"target_external_id")) {text(&row,"target_external_id")} else {String::new()};
            if !label.is_empty() {link.1.insert(label);}
        }
    }
    let exports = transaction_csv::report_rows(conn, &profile, None)?;
    let mut observed_open: BTreeMap<Key, BigInt> = BTreeMap::new();
    let mut flows: BTreeMap<Key, BTreeMap<String, BigInt>> = BTreeMap::new();
    let mut activity: BTreeMap<String, Activity> = BTreeMap::new();
    let mut transactions = Vec::new();
    for (source, row) in observed.iter().zip(exports) {
        let act = activity.entry(text(source, "wallet")).or_default();
        if act.first.is_empty() {
            act.first = text(source, "occurred_at");
        }
        act.last = text(source, "occurred_at");
        act.assets.insert(asset(&text(source, "asset")).code);
        let position = period.position(&text(source, "occurred_at"))?;
        if position > 0 {
            continue;
        }
        let a = asset(&text(source, "asset"));
        let key = (text(source, "wallet"), a.id.clone());
        let input = serde_json::json!({"direction":text(source,"direction"),"amount":int(source,"amount"),"fee":int(source,"fee"),"amount_includes_fee":truthy(value(source,"amount_includes_fee"))});
        let amounts = kassiber_custody::components::review::boundary_amounts(&input)
            .map_err(|e| AppError::new(e.code, e.message))?;
        let amounts = kassiber_pytext::loads(&amounts.to_string()).unwrap();
        let outbound =
            kassiber_pytext::strip(&text(source, "direction")).to_lowercase() == "outbound";
        let movement = number(&amounts, "wallet_movement_msat");
        let delta = if outbound {
            -movement.clone()
        } else {
            movement.clone()
        };
        if position < 0 {
            *observed_open.entry(key).or_default() += delta;
            continue;
        }
        act.count += 1;
        let flow = flows.entry(key).or_default();
        if outbound {
            *flow.entry("outflow".into()).or_default() += number(&amounts, "principal_msat");
            *flow.entry("fee".into()).or_default() += number(&amounts, "fee_msat");
        } else {
            *flow.entry("inflow".into()).or_default() += movement;
        }
        let tx = text(source, "id");
        let empty = Facts::default();
        let fact = facts.get(&tx).unwrap_or(&empty);
        let meta = wallets.get(&text(source, "wallet")).unwrap_or(&Value::Null);
        let mut output = row;
        merge(&mut output, a.fields());
        merge(&mut output, meta.clone());
        if !truthy(value(&output, "chain")) {
            replace(&mut output, "chain", s(a.chain));
        }
        replace(
            &mut output,
            "local_date",
            s(period.date(&text(source, "occurred_at"))?.1),
        );
        replace(&mut output, "transaction_ref", s(tx.as_str()));
        replace(
            &mut output,
            "source_kind",
            if truthy(value(source, "kind_override")) {
                value(source, "kind_override").clone()
            } else if truthy(value(source, "kind")) {
                value(source, "kind").clone()
            } else {
                s("")
            },
        );
        let link = links.get(&tx);
        replace(
            &mut output,
            "pair_ref",
            s(link.map(|l| joined(&l.0)).unwrap_or_default()),
        );
        replace(
            &mut output,
            "other_leg_transaction_id",
            s(link.map(|l| joined(&l.1)).unwrap_or_default()),
        );
        let other_wallet = value(&output, "transfer").clone();
        replace(&mut output, "other_wallet", other_wallet);
        replace(
            &mut output,
            "classification",
            s(classify(fact, swaps.contains(tx.as_str()))),
        );
        a.add_quantity(&mut output, "amount", &number(source, "amount"))?;
        a.add_quantity(&mut output, "fee", &number(source, "fee"))?;
        a.add_quantity(&mut output, "net_change", &delta)?;
        for (name, units) in [
            ("amount_sats", number(source, "amount")),
            ("fee_sats", number(source, "fee")),
            ("net_change_sats", delta),
        ] {
            replace(
                &mut output,
                name,
                if a.bitcoin {
                    Value::Decimal(
                        decimal(&s(units.to_string()))?
                            .div(&Decimal::from(1000), &DECIMAL)
                            .map_err(arithmetic)?,
                    )
                } else {
                    Value::Null
                },
            );
        }
        for key in [
            "income_value",
            "disposal_proceeds",
            "disposal_cost_basis",
            "disposal_gain_loss",
            "fee_cost_basis",
        ] {
            replace(&mut output, key, money(fact.amounts.get(key))?);
        }
        let types: BTreeSet<_> = fact
            .types
            .iter()
            .map(|t| {
                if t == "neutral_disposal" {
                    "disposal".into()
                } else {
                    t.clone()
                }
            })
            .collect();
        replace(&mut output, "journal_entry_types", s(joined(&types)));
        replace(&mut output, "tax_category", s(joined(&fact.categories)));
        transactions.push(output);
    }
    let excluded = excluded(conn, &profile, &period)?;
    let (opening_basis, opening_pool) = allocate(&opening)?;
    let (closing_basis, closing_pool) = allocate(&closing)?;
    let asset_ids: BTreeSet<_> = opening
        .keys()
        .chain(closing.keys())
        .map(|k| k.1.clone())
        .collect();
    let quotes = rates(conn, &profile, &period, &asset_ids, &spellings)?;
    let balance_keys: BTreeSet<_> = opening.keys().chain(closing.keys()).cloned().collect();
    let mut balances = Vec::new();
    for key in &balance_keys {
        let open = opening
            .get(key)
            .map(|h| h.stored.clone())
            .unwrap_or_default();
        let close = closing
            .get(key)
            .map(|h| h.stored.clone())
            .unwrap_or_default();
        if open == 0.into() && close == 0.into() {
            continue;
        }
        let a = &assets[&key.1];
        let mut row = a.fields();
        merge(
            &mut row,
            wallets.get(&key.0).cloned().unwrap_or(Value::Null),
        );
        replace(&mut row, "wallet", s(key.0.as_str()));
        if !truthy(value(&row, "chain")) {
            replace(&mut row, "chain", s(a.chain));
        }
        a.add_quantity(&mut row, "opening_quantity", &open)?;
        a.add_quantity(&mut row, "closing_quantity", &close)?;
        replace(
            &mut row,
            "opening_cost_basis",
            float(opening_basis.get(key).unwrap_or(&Decimal::zero()))?,
        );
        replace(
            &mut row,
            "closing_cost_basis",
            float(closing_basis.get(key).unwrap_or(&Decimal::zero()))?,
        );
        quote_fields(&mut row, a, &close, quotes.get(&key.1))?;
        replace(&mut row, "_closing_stored", Value::Int(close));
        balances.push(row);
    }
    let mut rolls = Vec::new();
    let roll_assets: BTreeSet<_> = asset_ids.iter().chain(roll.keys()).cloned().collect();
    for asset in &roll_assets {
        let a = &assets[asset];
        let mut row = a.fields();
        let movement = roll.entry(asset.clone()).or_default();
        let open: BigInt = opening
            .iter()
            .filter(|(k, _)| &k.1 == asset)
            .map(|(_, h)| h.stored.clone())
            .sum();
        let close: BigInt = closing
            .iter()
            .filter(|(k, _)| &k.1 == asset)
            .map(|(_, h)| h.stored.clone())
            .sum();
        let unit = |key| movement.units.get(key).cloned().unwrap_or_default();
        let check = &open + unit("acquired") - unit("disposed") - unit("fee")
            + unit("transfer_net")
            - &close;
        a.add_quantity(&mut row, "opening_quantity", &open)?;
        a.add_quantity(&mut row, "closing_quantity", &close)?;
        for (target, source) in [
            ("acquired_quantity", "acquired"),
            ("disposed_quantity", "disposed"),
            ("fee_quantity", "fee"),
            ("transfer_net", "transfer_net"),
        ] {
            replace(&mut row, target, Value::Decimal(a.quantity(&unit(source))?));
        }
        replace(
            &mut row,
            "quantity_check",
            Value::Decimal(a.quantity(&check)?),
        );
        replace(&mut row, "_quantity_check_stored", Value::Int(check));
        let open_basis = opening_pool
            .get(asset)
            .cloned()
            .unwrap_or_else(Decimal::zero);
        let close_basis = closing_pool
            .get(asset)
            .cloned()
            .unwrap_or_else(Decimal::zero);
        replace(&mut row, "opening_cost_basis", float(&open_basis)?);
        replace(&mut row, "closing_cost_basis", float(&close_basis)?);
        for key in [
            "acquired_value",
            "income_value",
            "disposed_cost_basis",
            "disposal_proceeds",
            "realized_gain_loss",
            "fee_cost_basis",
            "transfer_basis_net",
        ] {
            replace(
                &mut row,
                key,
                float(movement.money.get(key).unwrap_or(&Decimal::zero()))?,
            );
        }
        let amount = |key| {
            movement
                .money
                .get(key)
                .cloned()
                .unwrap_or_else(Decimal::zero)
        };
        let check = open_basis
            .add(&amount("acquired_value"), &DECIMAL)
            .and_then(|v| v.sub(&amount("disposed_cost_basis"), &DECIMAL))
            .and_then(|v| v.sub(&amount("fee_cost_basis"), &DECIMAL))
            .and_then(|v| v.add(&amount("transfer_basis_net"), &DECIMAL))
            .and_then(|v| v.sub(&close_basis, &DECIMAL))
            .map_err(arithmetic)?;
        replace(&mut row, "basis_check", float(&check)?);
        let market = quote_fields(&mut row, a, &close, quotes.get(asset))?;
        replace(
            &mut row,
            "unrealized_gain_loss",
            money(
                market
                    .map(|v| v.sub(&close_basis, &DECIMAL).map_err(arithmetic))
                    .transpose()?
                    .as_ref(),
            )?,
        );
        rolls.push(row);
    }
    let mut reconciliation = Vec::new();
    for key in balance_keys
        .iter()
        .chain(observed_open.keys())
        .chain(flows.keys())
        .collect::<BTreeSet<_>>()
    {
        let a = &assets[&key.1];
        let mut row = a.fields();
        replace(&mut row, "wallet", s(key.0.as_str()));
        let open = opening
            .get(key)
            .map(|h| h.stored.clone())
            .unwrap_or_default();
        let close = closing
            .get(key)
            .map(|h| h.stored.clone())
            .unwrap_or_default();
        let observed = observed_open.get(key).cloned().unwrap_or_default();
        let flow = |key2| {
            flows
                .get(key)
                .and_then(|f| f.get(key2))
                .cloned()
                .unwrap_or_default()
        };
        let observed_close = &observed + flow("inflow") - flow("outflow") - flow("fee");
        if [
            &open,
            &close,
            &observed,
            &observed_close,
            &flow("inflow"),
            &flow("outflow"),
            &flow("fee"),
        ]
        .iter()
        .all(|n| **n == 0.into())
        {
            continue;
        }
        let open_diff = &open - &observed;
        let close_diff = &close - &observed_close;
        for (name, n) in [
            ("opening_journal", open),
            ("closing_journal", close),
            ("opening_observed", observed),
            ("closing_observed", observed_close),
            ("inflow", flow("inflow")),
            ("outflow", flow("outflow")),
            ("fee", flow("fee")),
            ("opening_difference", open_diff.clone()),
            ("closing_difference", close_diff.clone()),
        ] {
            replace(&mut row, name, Value::Decimal(a.quantity(&n)?));
        }
        replace(
            &mut row,
            "opening_difference_exact",
            s(a.exact(&open_diff)?),
        );
        replace(
            &mut row,
            "closing_difference_exact",
            s(a.exact(&close_diff)?),
        );
        replace(
            &mut row,
            "status",
            s(if open_diff == 0.into() && close_diff == 0.into() {
                "OK"
            } else {
                "DIFF"
            }),
        );
        replace(
            &mut row,
            "timing_notes",
            s(timing.get(key).map(|v| v.join("\n")).unwrap_or_default()),
        );
        reconciliation.push(row);
    }
    let wallet_rows = wallets
        .keys()
        .chain(activity.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|label| {
            let mut row = wallets
                .get(label)
                .cloned()
                .unwrap_or_else(|| dict::<&str>([]));
            let empty = Activity::default();
            let act = activity.get(label).unwrap_or(&empty);
            replace(&mut row, "wallet", s(label.as_str()));
            replace(&mut row, "assets", s(joined(&act.assets)));
            replace(
                &mut row,
                "transactions_in_year",
                Value::Int(act.count.into()),
            );
            replace(
                &mut row,
                "excluded_in_year",
                Value::Int(
                    excluded
                        .iter()
                        .filter(|r| text(r, "wallet") == *label)
                        .count()
                        .into(),
                ),
            );
            replace(&mut row, "first_transaction_at", s(act.first.as_str()));
            replace(&mut row, "last_transaction_at", s(act.last.as_str()));
            row
        })
        .collect();
    let differences = reconciliation
        .iter()
        .filter(|r| text(r, "status") != "OK")
        .count();
    let roll_differences: Vec<_> = rolls
        .iter()
        .filter(|r| {
            number(r, "_quantity_check_stored") != 0.into()
                || matches!(value(r,"basis_check"),Value::Float(v) if v.abs()>=0.005)
        })
        .map(|r| text(r, "asset"))
        .collect();
    let missing: BTreeSet<_> = balances
        .iter()
        .filter(|r| truthy(value(r, "_closing_stored")) && text(r, "rate_source") == "missing")
        .map(|r| text(r, "asset"))
        .collect();
    let fallback: BTreeSet<_> = balances
        .iter()
        .filter(|r| {
            truthy(value(r, "_closing_stored")) && text(r, "rate_source") == "transaction price"
        })
        .map(|r| format!("{} ({})", text(r, "asset"), text(r, "rate_timestamp")))
        .collect();
    let mut warnings = Vec::new();
    if differences > 0 {
        warnings.push(s(format!("{differences} wallet/asset rows do not reconcile with observed wallet movements; see the Reconciliation sheet.")));
    }
    if !roll_differences.is_empty() {
        warnings.push(s(format!(
            "The roll-forward does not close for {}.",
            roll_differences.join(", ")
        )));
    }
    if !missing.is_empty() {
        warnings.push(s(format!(
            "No year-end rate for {}; market value is left blank.",
            joined(&missing)
        )));
    }
    if !fallback.is_empty() {
        warnings.push(s(format!("Year-end market value uses the last transaction price for {}, not a market rate at year end. Sync or set a rate before valuing holdings.",joined(&fallback))));
    }
    Ok(dict([
        ("workspace", workspace),
        ("profile", profile),
        ("year", Value::Int(period.year.into())),
        ("calendar", s(period.calendar())),
        ("period_start", s(period.start)),
        ("period_end", s(period.end)),
        ("generated_at", s(sources.now(conn)?)),
        (
            "kassiber_version",
            s(include_str!("../../kassiber/Cargo.toml")
                .lines()
                .find_map(|line| {
                    line.strip_prefix("version = \"")
                        .and_then(|v| v.strip_suffix('"'))
                })
                .unwrap()),
        ),
        ("wallets", Value::List(wallet_rows)),
        ("transactions", Value::List(transactions)),
        ("journal", Value::List(journal_rows)),
        ("balances", Value::List(balances)),
        ("roll_forward", Value::List(rolls)),
        ("reconciliation", Value::List(reconciliation)),
        ("excluded", Value::List(excluded)),
        (
            "reconciled",
            Value::Bool(differences == 0 && roll_differences.is_empty()),
        ),
        ("warnings", Value::List(warnings)),
    ]))
}
pub(super) fn classify(fact: &Facts, swap: bool) -> String {
    let mut labels = Vec::new();
    if ["transfer_in", "transfer_out", "transfer_fee"]
        .iter()
        .any(|t| fact.types.contains(*t))
    {
        labels.push("internal_transfer");
    }
    if swap || fact.categories.contains("neu_swap") {
        labels.push("swap");
    }
    if fact.types.contains("income") {
        labels.push("income");
    } else if fact.types.contains("acquisition") && !swap {
        labels.push("acquisition");
    }
    if fact.types.contains("disposal") {
        labels.push("disposal");
    }
    if labels.is_empty() && fact.types.contains("fee") {
        labels.push("fee");
    }
    if labels.is_empty() {
        "no_journal_entry".into()
    } else {
        labels.join("+")
    }
}
fn quote_fields(
    row: &mut Value,
    asset: &Asset,
    quantity: &BigInt,
    quote: Option<&Value>,
) -> Result<Option<Decimal>> {
    let rate = quote.map(|q| decimal(value(q, "rate"))).transpose()?;
    let market = rate
        .as_ref()
        .map(|r| {
            asset
                .quantity(quantity)?
                .mul(r, &DECIMAL)
                .map_err(arithmetic)
        })
        .transpose()?;
    replace(row, "year_end_rate", money(rate.as_ref())?);
    replace(row, "year_end_market_value", money(market.as_ref())?);
    replace(
        row,
        "rate_source",
        quote
            .map(|q| value(q, "source").clone())
            .unwrap_or_else(|| s("missing")),
    );
    replace(
        row,
        "rate_timestamp",
        quote
            .map(|q| value(q, "timestamp").clone())
            .unwrap_or_else(|| s("")),
    );
    Ok(market)
}
fn excluded(conn: &Connection, profile: &Value, period: &Period) -> Result<Vec<Value>> {
    let source = rows(conn,"SELECT t.*,w.label AS wallet,COALESCE(t.external_id,'') AS external_id,COALESCE(NULLIF(t.kind_override,''),t.kind,'') AS source_kind,(SELECT GROUP_CONCAT(code,', ') FROM (SELECT tg.code AS code FROM transaction_tags tt JOIN tags tg ON tg.id=tt.tag_id WHERE tt.transaction_id=t.id ORDER BY tg.code)) AS tags,exclusion.reason AS exclusion_reason,exclusion.changed_at AS excluded_at FROM transactions t LEFT JOIN transaction_edit_events exclusion ON exclusion.id=(SELECT event.id FROM transaction_edit_events event JOIN transaction_edit_fields field ON field.event_id=event.id WHERE event.transaction_id=t.id AND field.field='excluded' ORDER BY event.changed_at DESC,event.id DESC LIMIT 1) JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=1 ORDER BY t.occurred_at,t.created_at,t.id",[text(profile,"id")])?;
    let mut result = Vec::new();
    for row in source {
        if period.position(&text(&row, "occurred_at"))? != 0 {
            continue;
        }
        let a = Asset::new(&text(&row, "asset"));
        let mut output = a.fields();
        for key in [
            "occurred_at",
            "wallet",
            "direction",
            "source_kind",
            "excluded_at",
            "exclusion_reason",
            "description",
            "note",
            "tags",
        ] {
            replace(
                &mut output,
                key,
                if truthy(value(&row, key)) {
                    value(&row, key).clone()
                } else {
                    s("")
                },
            );
        }
        replace(&mut output, "transaction_ref", value(&row, "id").clone());
        replace(
            &mut output,
            "transaction_id",
            value(&row, "external_id").clone(),
        );
        replace(
            &mut output,
            "local_date",
            s(period.date(&text(&row, "occurred_at"))?.1),
        );
        a.add_quantity(&mut output, "amount", &number(&row, "amount"))?;
        a.add_quantity(&mut output, "fee", &number(&row, "fee"))?;
        result.push(output);
    }
    Ok(result)
}
