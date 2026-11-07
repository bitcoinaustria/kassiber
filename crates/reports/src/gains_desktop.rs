use super::*;
use kassiber_store::rusqlite::params;
use std::collections::BTreeSet;

pub(super) fn active_profile(conn: &Connection) -> Result<Option<Value>> {
    let settings = rows(
        conn,
        "SELECT key,value FROM settings WHERE key IN ('context_workspace','context_profile')",
        [],
    )?;
    let selected = |key| {
        settings
            .iter()
            .find(|r| value(r, "key") == &s(key))
            .map(|r| text(r, "value"))
            .unwrap_or_default()
    };
    let workspace = selected("context_workspace");
    let profile = selected("context_profile");
    if workspace.is_empty() || profile.is_empty() {
        return Ok(None);
    }
    rows(conn, "SELECT p.* FROM profiles p JOIN workspaces w ON w.id=p.workspace_id WHERE p.id=? AND w.id=?", [profile, workspace]).map(|mut r| r.pop())
}
fn number(v: &Value, key: &str) -> f64 {
    match value(v, key) {
        Value::Float(n) => *n,
        Value::Int(n) => n.to_string().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}
fn current_year(conn: &Connection, sources: &dyn kassiber_setup::Sources) -> Result<i64> {
    Ok(sources.now(conn)?[..4].parse().unwrap())
}
fn empty(year: i64) -> Value {
    dict([
        // No book: no country, so no Austrian rules to assume.
        ("jurisdictionCode", s("GENERIC")),
        ("year", Value::Int(year.into())),
        ("availableYears", Value::List(vec![Value::Int(year.into())])),
        ("method", s("fifo")),
        ("lots", Value::List(vec![])),
        ("kennzahlRows", Value::List(vec![])),
        (
            "status",
            dict([
                ("needsJournals", Value::Bool(false)),
                ("quarantines", Value::Int(0.into())),
            ]),
        ),
    ])
}
fn years(
    conn: &Connection,
    profile: &str,
    vienna: bool,
    primary: Option<bool>,
) -> Result<Vec<i64>> {
    let query = if let Some(primary) = primary {
        let filter = if primary {
            "((je.entry_type='disposal' AND COALESCE(je.at_category,'')!='neu_swap') OR (je.entry_type NOT IN ('fee','transfer_fee') AND je.at_kennzahl IS NOT NULL))"
        } else {
            "((je.entry_type IN ('disposal','income') AND COALESCE(je.at_category,'')!='neu_swap') OR (je.entry_type NOT IN ('fee','transfer_fee','disposal') AND je.at_kennzahl IS NOT NULL))"
        };
        format!("SELECT je.occurred_at FROM journal_entries je LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND COALESCE(t.taxability_override,1)!=0 AND {filter} AND je.occurred_at IS NOT NULL AND length(je.occurred_at)>=4")
    } else {
        "SELECT occurred_at FROM transactions WHERE profile_id=? AND excluded=0 AND occurred_at IS NOT NULL AND length(occurred_at)>=4".into()
    };
    let mut result = BTreeSet::new();
    for row in rows(conn, &query, [profile])? {
        let raw = text(&row, "occurred_at");
        if vienna {
            result.insert(calendar::vienna(&raw)?.0);
        } else {
            let prefix: String = raw.chars().take(4).collect();
            if prefix.chars().all(|c| c.is_numeric()) {
                let parsed = kassiber_pytext::parse_int(&prefix).map_err(|_| {
                    AppError::new(
                        "internal_error",
                        format!(
                            "invalid literal for int() with base 10: {}",
                            kassiber_pytext::repr_str(&prefix)
                        ),
                    )
                })?;
                if let Ok(n) = i64::try_from(parsed) {
                    result.insert(n);
                }
            }
        }
    }
    Ok(result.into_iter().rev().collect())
}
fn normalize_year(raw: &Value) -> Result<i64> {
    austrian::normalize_tax_year(raw).map_err(|mut e| {
        if e.code == "validation" {
            e.message = if e.message.contains("plausible") {
                "tax year must be a plausible four-digit year"
            } else {
                "tax year must be a four-digit year"
            }
            .into();
        }
        e
    })
}
fn date(raw: &str, vienna: bool) -> Result<String> {
    if vienna {
        calendar::vienna(raw).map(|(_, d)| d)
    } else {
        Ok(raw.chars().take(10).collect())
    }
}
fn sat(n: i64) -> Value {
    if n % 1000 == 0 {
        Value::Int((n / 1000).into())
    } else {
        Value::Float(n as f64 / 1000.0)
    }
}
fn in_year(row: &Value, year: i64, vienna: bool) -> Result<bool> {
    if vienna {
        Ok(calendar::vienna(&text(row, "occurred_at"))?.0 == year)
    } else {
        Ok(true)
    }
}
fn bounds(year: i64, vienna: bool) -> (String, String, String) {
    if vienna {
        let (start, end) = calendar::window(year);
        ("je.occurred_at>=? AND je.occurred_at<?".into(), start, end)
    } else {
        (
            "substr(je.occurred_at,1,4)=? AND ?=''".into(),
            year.to_string(),
            String::new(),
        )
    }
}

pub(super) fn report(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let Some(profile) = active_profile(conn)? else {
        return Ok(empty(current_year(conn, sources)?));
    };
    let id = text(&profile, "id");
    let vienna = text(&profile, "tax_country").to_lowercase() == "at";
    let primary = years(conn, &id, vienna, Some(true))?;
    let mut available: BTreeSet<_> = years(conn, &id, vienna, Some(false))?
        .into_iter()
        .chain(years(conn, &id, vienna, None)?)
        .filter(|n| (2009..=2100).contains(n))
        .collect();
    let year = if !matches!(value(args, "year"), Value::Null) {
        normalize_year(value(args, "year"))?
    } else if let Some(n) = primary.first() {
        *n
    } else if let Some(n) = available.last() {
        *n
    } else {
        current_year(conn, sources)?
    };
    let contained = available.contains(&year);
    available.insert(year);
    let mut available: Vec<_> = available
        .into_iter()
        .rev()
        .map(|n| Value::Int(n.into()))
        .collect();
    if !contained {
        available.retain(|n| n != &Value::Int(year.into()));
        available.insert(0, Value::Int(year.into()));
    }
    let (predicate, start, end) = bounds(year, vienna);
    let selected=rows(conn,&format!("SELECT je.id,je.occurred_at,je.entry_type,je.quantity,je.cost_basis,je.proceeds,je.gain_loss,je.capital_gains_type,je.transaction_id,je.asset,je.at_category FROM journal_entries je LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND je.entry_type IN ('disposal','income') AND COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap' AND {predicate} ORDER BY je.occurred_at DESC,je.created_at DESC,je.id DESC"),[&id,&start,&end])?;
    let mut selected = selected
        .into_iter()
        .filter_map(|r| match in_year(&r, year, vienna) {
            Ok(true) => Some(Ok(r)),
            Ok(false) => None,
            Err(e) => Some(Err(e)),
        })
        .collect::<Result<Vec<_>>>()?;
    // Totals cover the whole year, so a reader of the newest 200 rows does not
    // mistake their sum for the year's result.
    let totals = dict([
        ("rowCount", Value::Int(selected.len().into())),
        (
            "costEur",
            sum_eur(selected.iter().map(|r| value(r, "cost_basis")))?,
        ),
        ("proceedsEur", sum_eur(proceeds(&selected))?),
        ("incomeEur", sum_eur(income(&selected))?),
        (
            "gainLossEur",
            sum_eur(selected.iter().map(|r| value(r, "gain_loss")))?,
        ),
    ]);
    let neutral = neutral(conn, &id, year, vienna)?;
    let breakdown = breakdown(&selected, &neutral, vienna)?;
    selected.truncate(200);
    let mut lots = vec![];
    for row in selected.into_iter().rev() {
        let sats = (number(
            &dict([("quantity", float(&btc(int(&row, "quantity"))?)?)]),
            "quantity",
        )
        .abs()
            * 100_000_000.0)
            .round_ties_even();
        let income = if truthy(value(&row, "gain_loss")) {
            value(&row, "gain_loss")
        } else {
            value(&row, "proceeds")
        };
        let proceeds = if text(&row, "entry_type") == "income" {
            if truthy(income) {
                income.clone()
            } else {
                Value::Int(0.into())
            }
        } else {
            value(&row, "proceeds").clone()
        };
        if matches!(proceeds, Value::Null) {
            return Err(AppError::new(
                "internal_error",
                "float() argument must be a string or a real number, not 'NoneType'",
            ));
        }
        let category = value(&row, "at_category");
        let kennzahl = if vienna && truthy(category) {
            austrian::kennzahl_for_event(&py_str(category), &text(&row, "occurred_at"))?
        } else {
            None
        };
        lots.push(dict([
            (
                "explanationReference",
                gain_explanation::reference(conn, &profile, value(&row, "id"))?,
            ),
            ("acquired", s("")),
            ("disposed", s(date(&text(&row, "occurred_at"), vienna)?)),
            ("sats", austrian::integer(&decimal(&Value::Float(sats))?)?),
            ("costEur", Value::Float(number(&row, "cost_basis"))),
            (
                "proceedsEur",
                Value::Float(number(&dict([("p", proceeds)]), "p")),
            ),
            (
                "type",
                s(
                    if text(&row, "capital_gains_type").to_lowercase() == "long" {
                        "LT"
                    } else {
                        "ST"
                    },
                ),
            ),
            ("transactionId", value(&row, "transaction_id").clone()),
            ("asset", value(&row, "asset").clone()),
            ("entryType", value(&row, "entry_type").clone()),
            (
                "quantityMsat",
                Value::Int(int(&row, "quantity").unsigned_abs().into()),
            ),
            ("gainLossEur", Value::Float(number(&row, "gain_loss"))),
            (
                "atCategory",
                if truthy(category) {
                    category.clone()
                } else {
                    Value::Null
                },
            ),
            (
                "kennzahl",
                kennzahl.map_or(Value::Null, |code| Value::Int(code.into())),
            ),
        ]));
    }
    // `_austrian_kennzahl_snapshot_rows`, over the summary that states a year
    // before 2022 in KZ 801 only.
    let mut kennzahl = vec![];
    if vienna {
        for row in austrian::kennzahl_summary(conn, &profile, year)? {
            kennzahl.push(dict([
                ("code", s(text(&row, "kennzahl"))),
                ("label", value(&row, "label").clone()),
                ("form", value(&row, "form").clone()),
                ("formSection", value(&row, "form_section").clone()),
                (
                    "amount",
                    Value::Float(number(&row, "amount_eur_cents") / 100.0),
                ),
                ("amountEurCents", value(&row, "amount_eur_cents").clone()),
                ("rowCount", value(&row, "row_count").clone()),
                ("source", s("daemon")),
            ]));
        }
    }
    let quarantines: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_quarantines WHERE profile_id=?",
            [&id],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let country = if truthy(value(&profile, "tax_country")) {
        text(&profile, "tax_country").to_uppercase()
    } else {
        "GENERIC".into()
    };
    let fiat = if truthy(value(&profile, "fiat_currency")) {
        text(&profile, "fiat_currency").to_uppercase()
    } else {
        "EUR".into()
    };
    let method = if truthy(value(&profile, "gains_algorithm")) {
        text(&profile, "gains_algorithm").to_lowercase()
    } else {
        "fifo".into()
    };
    // Austria has no single holding period: only Altbestand held over a year
    // is tax-free, which its rules copy says instead.
    let long_term_days = if vienna {
        Value::Null
    } else {
        long_term_days(&profile)?
    };
    let eur_book = text(&profile, "fiat_currency").to_uppercase() == "EUR"
        && truthy(value(&profile, "fiat_currency"));
    let applies = austrian::e1kv_applies(year);
    let mut old_rule_rows = None;
    let mut old_rules = || -> Result<i64> {
        if old_rule_rows.is_none() {
            old_rule_rows = Some(austrian::rows_before_crypto_kennzahlen(
                conn, &profile, year,
            )?);
        }
        Ok(old_rule_rows.unwrap())
    };
    // The core's indicative tax for an Austrian EUR book, computed without the
    // report blockers: Reports marks it provisional for the causes it checks.
    let indicative = if vienna && eur_book {
        let pairs = kennzahl
            .iter()
            .map(|row| {
                Ok((
                    i64::try_from(austrian::py_int(value(row, "code"))?)
                        .map_err(|_| AppError::new("internal_error", "invalid Kennzahl"))?,
                    int(row, "amountEurCents"),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let estimate = tax_estimate::austrian_indicative_tax(year, &pairs, &[], old_rules()?);
        if text(&estimate, "status") == "not_applicable" {
            // Before the E 1kv there is no 27.5 % figure to show.
            Value::Null
        } else {
            dict([
                (
                    "estimatedTaxEurCents",
                    value(&estimate, "estimated_tax_eur_cents").clone(),
                ),
                ("rate", value(&estimate, "rate").clone()),
                ("basis", value(&estimate, "basis").clone()),
                (
                    "progressiveRateAmountsEurCents",
                    value(&estimate, "progressive_rate_amounts_eur_cents").clone(),
                ),
                ("notes", value(&estimate, "notes").clone()),
            ])
        }
    } else {
        Value::Null
    };
    // Rows the E 1kv export refuses: classified for a crypto Kennzahl but
    // dated before 1 March 2022.
    let before = if vienna && eur_book && applies {
        old_rules()?
    } else {
        0
    };
    Ok(dict([
        ("jurisdictionCode", s(country)),
        ("fiatCurrency", s(fiat)),
        ("longTermDays", long_term_days),
        ("year", Value::Int(year.into())),
        ("availableYears", Value::List(available)),
        ("method", s(method)),
        ("asset", Value::Null),
        ("lots", Value::List(lots)),
        ("totals", totals),
        ("breakdown", Value::List(breakdown)),
        ("neutralSwapLots", Value::List(neutral)),
        ("kennzahlRows", Value::List(kennzahl)),
        // An Austrian year before the crypto Kennzahlen: none, no estimate.
        ("e1kvApplies", Value::Bool(vienna && applies)),
        ("indicativeTax", indicative),
        ("rowsBeforeCryptoKennzahlen", Value::Int(before.into())),
        (
            "status",
            dict([
                (
                    "needsJournals",
                    Value::Bool(gain_explanation::fresh(conn, &profile)?.1),
                ),
                ("quarantines", Value::Int(quarantines.into())),
            ]),
        ),
    ]))
}

/// `build_tax_policy(profile).long_term_days` for a book that is not Austrian
/// by its stored country.
fn long_term_days(profile: &Value) -> Result<Value> {
    let raw = value(profile, "tax_country");
    let country = if truthy(raw) {
        kassiber_pytext::strip(&py_str(raw)).to_lowercase()
    } else {
        "generic".into()
    };
    match country.as_str() {
        "at" => Ok(Value::Int(i64::MAX.into())),
        "" | "generic" => {
            let days = value(profile, "tax_long_term_days");
            let days = if truthy(days) {
                austrian::py_int(days)?
            } else {
                365.into()
            };
            if days < 0.into() {
                return Err(AppError::new(
                    "internal_error",
                    "tax_long_term_days cannot be negative",
                ));
            }
            Ok(Value::Int(days))
        }
        _ => Err(AppError::new(
            "internal_error",
            format!("Unsupported tax country '{}'", py_str(raw)),
        )),
    }
}

fn proceeds(rows: &[Value]) -> impl Iterator<Item = &Value> {
    rows.iter()
        .filter(|r| text(r, "entry_type") != "income")
        .map(|r| value(r, "proceeds"))
}

fn income(rows: &[Value]) -> impl Iterator<Item = &Value> {
    rows.iter()
        .filter(|r| text(r, "entry_type") == "income")
        .map(|r| {
            if truthy(value(r, "gain_loss")) {
                value(r, "gain_loss")
            } else {
                value(r, "proceeds")
            }
        })
}

/// `_sum_eur`: an exact decimal sum quantized to the cent, as a float.
fn sum_eur<'a>(values: impl IntoIterator<Item = &'a Value>) -> Result<Value> {
    let mut total = Decimal::zero();
    for v in values {
        if truthy(v) {
            total = total.add(&decimal(v)?, &DECIMAL).map_err(arithmetic)?;
        }
    }
    float(&total.quantize(-2, &DECIMAL).map_err(arithmetic)?)
}

// Exact transaction links stop at the Transactions read's `txids` cap
// (`MAX_UI_TRANSACTION_IDS`); a larger group opens the whole year instead.
const LINK_LIMIT: usize = 128;
const ORDER: [&str; 10] = [
    "172",
    "174",
    "176",
    "801",
    "tax_free",
    "unassigned",
    "swaps",
    "short_term",
    "long_term",
    "income",
];

fn link_ids<'a>(ids: impl IntoIterator<Item = &'a Value>) -> (usize, Value) {
    let unique: BTreeSet<String> = ids.into_iter().filter(|v| truthy(v)).map(py_str).collect();
    let count = unique.len();
    let ids = if count <= LINK_LIMIT {
        Value::List(unique.into_iter().map(s).collect())
    } else {
        Value::Null
    };
    (count, ids)
}

/// `_capital_gains_breakdown`: the year's rows grouped the way the report
/// states them. Austrian rows group by E 1 / E 1kv field, with tax-free
/// Altbestand and tax-neutral swaps beside them; generic rows by holding
/// period and income.
fn breakdown(rows: &[Value], swaps: &[Value], vienna: bool) -> Result<Vec<Value>> {
    let mut grouped: Vec<(String, Vec<&Value>)> = vec![];
    for row in rows {
        let key = if vienna {
            let category = value(row, "at_category");
            let category = if truthy(category) {
                py_str(category)
            } else {
                String::new()
            };
            if category == "alt_taxfree" {
                "tax_free".to_owned()
            } else {
                // A crypto Kennzahl before 1 March 2022 is no field: unassigned.
                let kennzahl = if category.is_empty() {
                    None
                } else {
                    austrian::kennzahl_for_event(&category, &text(row, "occurred_at"))?
                };
                kennzahl.map_or_else(|| "unassigned".to_owned(), |code| code.to_string())
            }
        } else if text(row, "entry_type") == "income" {
            "income".to_owned()
        } else if text(row, "capital_gains_type").to_lowercase() == "long" {
            "long_term".to_owned()
        } else {
            "short_term".to_owned()
        };
        match grouped.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(row),
            None => grouped.push((key, vec![row])),
        }
    }
    let mut breakdown = vec![];
    for (key, members) in grouped {
        let (count, ids) = link_ids(members.iter().map(|r| value(r, "transaction_id")));
        let members: Vec<Value> = members.into_iter().cloned().collect();
        breakdown.push(dict([
            ("key", s(key)),
            ("rowCount", Value::Int(members.len().into())),
            ("transactionCount", Value::Int(count.into())),
            ("transactionIds", ids),
            (
                "costEur",
                sum_eur(members.iter().map(|r| value(r, "cost_basis")))?,
            ),
            ("proceedsEur", sum_eur(proceeds(&members))?),
            ("incomeEur", sum_eur(income(&members))?),
            (
                "gainLossEur",
                sum_eur(members.iter().map(|r| value(r, "gain_loss")))?,
            ),
        ]));
    }
    if !swaps.is_empty() {
        let (count, ids) = link_ids(swaps.iter().map(|swap| value(swap, "transactionId")));
        // A swap split across several conversions lists its journal entry once
        // per conversion; its amounts count once.
        let mut entries: Vec<(String, &Value)> = vec![];
        for swap in swaps {
            let entry = py_str(value(swap, "journalEntryId"));
            match entries.iter_mut().find(|(k, _)| *k == entry) {
                Some((_, last)) => *last = swap,
                None => entries.push((entry, swap)),
            }
        }
        breakdown.push(dict([
            ("key", s("swaps")),
            ("rowCount", Value::Int(entries.len().into())),
            ("transactionCount", Value::Int(count.into())),
            ("transactionIds", ids),
            (
                "costEur",
                sum_eur(entries.iter().map(|(_, swap)| value(swap, "costEur")))?,
            ),
            (
                "proceedsEur",
                sum_eur(entries.iter().map(|(_, swap)| value(swap, "proceedsEur")))?,
            ),
            ("incomeEur", Value::Float(0.0)),
            ("gainLossEur", Value::Float(0.0)),
        ]));
    }
    // A fixed order, so every book states its groups the same way.
    breakdown.sort_by_key(|bucket| {
        let key = text(bucket, "key");
        ORDER.iter().position(|k| *k == key).unwrap_or(ORDER.len())
    });
    Ok(breakdown)
}

fn neutral(conn: &Connection, id: &str, year: i64, vienna: bool) -> Result<Vec<Value>> {
    let (predicate, start, end) = bounds(year, vienna);
    let query=format!("SELECT je.id AS entry_id,je.occurred_at,je.transaction_id,je.asset AS entry_asset,je.quantity,je.cost_basis,je.proceeds,je.gain_loss,COALESCE(relation.id,'') AS pair_id,COALESCE(relation.kind,'') AS kind,COALESCE(relation.policy,'') AS policy,COALESCE(relation.swap_fee_msat,0) AS swap_fee_msat,COALESCE(relation.swap_fee_kind,'') AS swap_fee_kind,wout.label AS out_wallet,relation.out_asset,COALESCE(relation.out_amount,tout.amount) AS out_amount,win.label AS in_wallet,relation.in_asset,relation.in_amount FROM journal_entries je LEFT JOIN journal_custody_projection_relations relation ON relation.out_transaction_id=je.transaction_id AND relation.profile_id=je.profile_id AND relation.relation_kind='conversion' LEFT JOIN transactions tout ON tout.id=relation.out_transaction_id LEFT JOIN transactions tin ON tin.id=relation.in_transaction_id LEFT JOIN wallets wout ON wout.id=tout.wallet_id LEFT JOIN wallets win ON win.id=tin.wallet_id WHERE je.profile_id=? AND je.entry_type='disposal' AND je.at_category='neu_swap' AND COALESCE(tout.taxability_override,1)!=0 AND {predicate} ORDER BY je.occurred_at ASC,je.created_at ASC,je.id ASC");
    let mut result = vec![];
    for row in rows(conn, &query, params![id, start, end])? {
        if !in_year(&row, year, vienna)? {
            continue;
        }
        let fallback = |key| {
            if truthy(value(&row, key)) {
                value(&row, key).clone()
            } else {
                s("")
            }
        };
        let out = int(&row, "out_amount").unsigned_abs();
        let out = if out == 0 {
            int(&row, "quantity").unsigned_abs()
        } else {
            out
        };
        let amount = |n: u64| {
            if n.is_multiple_of(1000) {
                Value::Int((n / 1000).into())
            } else {
                Value::Float(n as f64 / 1000.0)
            }
        };
        result.push(dict([
            ("date", s(date(&text(&row, "occurred_at"), vienna)?)),
            ("pairId", value(&row, "pair_id").clone()),
            ("journalEntryId", value(&row, "entry_id").clone()),
            ("transactionId", value(&row, "transaction_id").clone()),
            ("asset", fallback("entry_asset")),
            ("kind", value(&row, "kind").clone()),
            ("policy", value(&row, "policy").clone()),
            ("outWallet", fallback("out_wallet")),
            ("outAsset", fallback("out_asset")),
            ("outSats", amount(out)),
            ("inWallet", fallback("in_wallet")),
            ("inAsset", fallback("in_asset")),
            ("inSats", amount(int(&row, "in_amount").unsigned_abs())),
            ("feeSats", sat(int(&row, "swap_fee_msat"))),
            ("feeKind", value(&row, "swap_fee_kind").clone()),
            ("costEur", Value::Float(number(&row, "cost_basis"))),
            ("proceedsEur", Value::Float(number(&row, "cost_basis"))),
            ("gainEur", Value::Float(0.0)),
            ("marketValueEur", Value::Float(number(&row, "proceeds"))),
            ("marketDeltaEur", Value::Float(number(&row, "gain_loss"))),
        ]));
    }
    Ok(result)
}
