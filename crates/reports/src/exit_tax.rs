use super::*;
use std::collections::BTreeMap;

fn metadata() -> &'static Value {
    static META: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    META.get_or_init(|| kassiber_pytext::loads(include_str!("exit_tax_metadata.json")).unwrap())
}
fn note(key: &str) -> Value {
    value(metadata(), key).clone()
}
fn exact(row: &Value, key: &str) -> Result<Option<Decimal>> {
    for v in [value(row, &format!("{key}_exact")), value(row, key)] {
        if !matches!(v, Value::Null) && v != &s("") {
            return decimal(v).map(Some);
        }
    }
    Ok(None)
}
fn eur(d: Option<&Decimal>) -> Result<Value> {
    d.map(|d| {
        float(
            &d.quantize(-2, &Context::new(32, Rounding::HalfUp))
                .map_err(arithmetic)?,
        )
    })
    .transpose()
    .map(|v| v.unwrap_or(Value::Null))
}
fn sats(d: &Decimal) -> Result<Value> {
    austrian::integer(
        &d.mul(&Decimal::from(100_000_000), &DECIMAL)
            .map_err(arithmetic)?
            .quantize(0, &Context::new(32, Rounding::HalfUp))
            .map_err(arithmetic)?,
    )
}
fn marker(row: &Value) -> Option<String> {
    text(row, "description")
        .split(|c: char| kassiber_pytext::is_space(c as u32))
        .find_map(|t| t.strip_prefix("at_regime="))
        .filter(|v| ["alt", "neu"].contains(v))
        .map(String::from)
}
fn alt(row: &Value) -> Result<bool> {
    if let Some(m) = marker(row) {
        return Ok(m == "alt");
    }
    if int(row, "quantity") < 0 && truthy(value(row, "at_category")) {
        return Ok(text(row, "at_category").starts_with("alt"));
    }
    let at = text(row, "occurred_at");
    if !truthy(value(row, "occurred_at")) {
        return Ok(false);
    }
    let at = kassiber_rates::time::parse(&at).ok_or_else(|| {
        AppError::new(
            "internal_error",
            format!(
                "Invalid isoformat string: {}",
                kassiber_pytext::repr_str(&at)
            ),
        )
    })?;
    Ok(at < kassiber_rates::time::parse("2021-02-28T23:00:00Z").unwrap())
}
fn destination(raw: &Value) -> Result<String> {
    let v = if truthy(raw) {
        py_str(raw)
    } else {
        "eu_eea".into()
    };
    match kassiber_pytext::strip(&v)
        .to_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "eu" | "eea" | "eu_eea" | "euea" => Ok("eu_eea".into()),
        "third_country" | "third" | "non_eu" | "noneu" => Ok("third_country".into()),
        _ => Err(AppError::new(
            "validation",
            format!("Unsupported exit-tax destination '{}'", py_str(raw)),
        )
        .with_hint("Choose one of: eu_eea, third_country")),
    }
}
fn departure(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
    zone: Option<&str>,
) -> Result<(String, i64)> {
    let raw = value(args, "departure_date");
    let raw = if truthy(raw) {
        py_str(raw)
    } else {
        sources.now(conn)?
    };
    let stripped = kassiber_pytext::strip(&raw);
    let hint =
        "Use a date (YYYY-MM-DD) or a date-time with an offset, e.g. 2025-01-01T12:00:00+01:00.";
    if stripped.chars().count() != 10
        && !stripped.to_uppercase().contains('T')
        && !stripped.contains(' ')
    {
        return Err(AppError::new("validation", "Invalid exit-tax departure date").with_hint(hint));
    }
    let input = stripped.replace('Z', "+00:00");
    let (instant, has_offset) =
        kassiber_rates::time::parse_with_offset(&input).ok_or_else(|| {
            AppError::new("validation", "Invalid exit-tax departure date").with_hint(hint)
        })?;
    let civil_date = stripped.chars().count() == 10;
    if !civil_date && !has_offset {
        return Err(
            AppError::new("validation", "Exit-tax departure time requires an offset")
                .with_hint(hint),
        );
    }
    let zone = export_range::book_zone(zone, &[])?;
    let epoch = kassiber_rates::time::UNIX;
    let offset = zone
        .utc_offset((instant - epoch).div_euclid(1_000_000))
        .unwrap();
    let overflow = || AppError::new("internal_error", "date value out of range");
    let local = kassiber_rates::time::format(if civil_date {
        instant
    } else {
        instant + offset * 1_000_000
    })
    .ok_or_else(overflow)?;
    let day = local[..10].to_owned();
    let next = kassiber_rates::time::parse(&day).unwrap() + 86_400_000_000;
    kassiber_rates::time::format(next).ok_or_else(overflow)?;
    let offset = zone
        .local_offset((next - epoch).div_euclid(1_000_000))
        .unwrap();
    Ok((day, next - offset * 1_000_000))
}
fn transaction_value(row: &Value, key: &str) -> Result<Option<Decimal>> {
    let precise = value(row, &format!("{key}_exact"));
    let stored = if matches!(precise, Value::Null) {
        value(row, key)
    } else {
        precise
    };
    if matches!(stored, Value::Null) {
        Ok(None)
    } else {
        decimal(stored).map(Some)
    }
}
fn fallback_rates(
    conn: &Connection,
    profile: &Value,
    cutoff: i64,
) -> Result<BTreeMap<String, (Decimal, i64)>> {
    // Same-time rows keep their reverse source order through the stable
    // sort by instant, as `transaction_rate_quotes_for_profile` reads them.
    let mut candidates = rows(conn,&format!("SELECT asset,occurred_at,fiat_rate,fiat_value,fiat_rate_exact,fiat_value_exact,amount,created_at,id FROM transactions WHERE profile_id=? AND excluded=0 AND {} ORDER BY {}", kassiber_store::price_currency::price_in_book_currency_sql("transactions"), kassiber_store::transaction_order::same_time_order_sql("transactions", true)),[text(profile,"id")])?;
    candidates.retain(|row| {
        kassiber_rates::time::parse(&text(row, "occurred_at")).is_some_and(|at| at < cutoff)
    });
    candidates.sort_by_key(|row| {
        std::cmp::Reverse(kassiber_rates::time::parse(&text(row, "occurred_at")).unwrap())
    });
    let mut result = BTreeMap::new();
    for row in candidates {
        let asset = text(&row, "asset");
        if result.contains_key(&asset) {
            continue;
        }
        let rate = transaction_value(&row, "fiat_rate")?;
        let total = transaction_value(&row, "fiat_value")?;
        let rate = match rate {
            Some(rate) => Some(rate),
            None => total
                .filter(|_| int(&row, "amount") != 0)
                .map(|total| {
                    total
                        .div(&btc(int(&row, "amount"))?, &DECIMAL)
                        .map_err(arithmetic)
                })
                .transpose()?,
        };
        if let Some(rate) = rate {
            result.insert(
                asset,
                (
                    rate,
                    kassiber_rates::time::parse(&text(&row, "occurred_at")).unwrap(),
                ),
            );
        }
    }
    Ok(result)
}
fn fmv(
    conn: &Connection,
    asset: &str,
    fiat: &str,
    cutoff: i64,
    fallback: &BTreeMap<String, (Decimal, i64)>,
) -> Result<(Option<Decimal>, Value)> {
    let pair = kassiber_rates::transaction_rate_pair(asset, fiat);
    let mut quotes = Vec::new();
    if let Some(pair) = &pair {
        let mut cached = sql::optional_rows(conn,"SELECT timestamp,rate,rate_exact FROM rates_cache WHERE pair=? ORDER BY CASE WHEN source='manual' THEN 0 ELSE 1 END ASC,fetched_at DESC",[pair])?;
        cached.retain(|row| {
            kassiber_rates::time::parse(&text(row, "timestamp")).is_some_and(|at| at < cutoff)
        });
        cached.sort_by_key(|row| {
            std::cmp::Reverse(kassiber_rates::time::parse(&text(row, "timestamp")).unwrap())
        });
        if let Some(row) = cached.first() {
            if let Some(rate) = exact(row, "rate")? {
                quotes.push((
                    rate,
                    kassiber_rates::time::parse(&text(row, "timestamp")).unwrap(),
                    "cache",
                ));
            }
        }
    }
    if let Some((rate, at)) = fallback.get(asset) {
        quotes.push((rate.clone(), *at, "transaction"));
    }
    quotes.retain(|(rate, at, _)| {
        *rate > Decimal::zero() && 0 < cutoff - at && cutoff - at <= 86_400_000_000
    });
    // Stable sorting leaves cache first at equal instants.
    quotes.sort_by_key(|(_, at, _)| std::cmp::Reverse(*at));
    let (found, as_of, source) = match quotes.into_iter().next() {
        Some((rate, at, source)) => (
            Some(rate),
            s(kassiber_rates::time::format(at).unwrap()),
            source,
        ),
        None => (None, Value::Null, "missing"),
    };
    let row = dict([
        ("asset", s(asset)),
        (
            "rate",
            found
                .as_ref()
                .map(float)
                .transpose()?
                .unwrap_or(Value::Null),
        ),
        (
            "pair",
            s(pair.unwrap_or_else(|| format!("{asset}-{}", fiat.to_uppercase()))),
        ),
        ("asOf", as_of),
        ("source", s(source)),
    ]);
    Ok((found, row))
}

pub(super) fn report(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    inventory(conn, workspace, profile, args, sources, None)
}

/// `compute_deemed_disposal` over a caller's BTC/L-BTC journal entries with
/// no holdings or latest-rate fallbacks, as the overview's tax-free balance
/// passes it. Only the inventory walk and cached quotes apply.
pub fn overview_inventory(
    conn: &Connection,
    profile: &Value,
    entries: Vec<Value>,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let empty = dict::<&str>([]);
    inventory(conn, &empty, profile, &empty, sources, Some(entries))
}

fn inventory(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
    supplied: Option<Vec<Value>>,
) -> Result<Value> {
    let is_at = crate::country::austrian(
        &kassiber_pytext::strip(&text(profile, "tax_country")).to_lowercase(),
    );
    let fiat = if truthy(value(profile, "fiat_currency")) {
        text(profile, "fiat_currency").to_uppercase()
    } else {
        "EUR".into()
    };
    let zone =
        crate::country::zone(&kassiber_pytext::strip(&text(profile, "tax_country")).to_lowercase());
    let (day, cutoff) = departure(conn, args, sources, zone)?;
    // The 27.5 % rate and KZ 174/176 cover realisations from 1 March 2022, by
    // the departure day as the owner names it (not its UTC end, which is
    // already 1 March in Vienna for 28 February).
    let old_rules = is_at && day.as_str() < "2022-03-01";
    let overview = supplied.is_some();
    let entries = match supplied {
        Some(entries) => entries,
        None => rows(conn,"SELECT e.*,w.label AS wallet_label FROM journal_entries e LEFT JOIN wallets w ON w.id=e.wallet_id WHERE e.profile_id=? ORDER BY e.occurred_at ASC,e.created_at ASC,e.id ASC",[text(profile,"id")])?,
    };
    for row in entries.iter().filter(|_| !overview) {
        for field in [
            "fiat_value",
            "unit_cost",
            "cost_basis",
            "proceeds",
            "gain_loss",
        ] {
            exact(row, field)?;
        }
    }
    let mut holdings: Vec<Value> = Vec::new();
    let stored = if overview {
        Vec::new()
    } else {
        rows(conn,"SELECT wallet_id,wallet_label,account_code,asset,quantity,cost_basis FROM journal_wallet_holdings WHERE profile_id=?",[text(profile,"id")])?
    };
    for row in stored {
        exact(&row, "cost_basis")?;
        if let Some(previous) = holdings.iter_mut().find(|r| {
            ["wallet_id", "wallet_label", "account_code", "asset"]
                .iter()
                .all(|k| value(r, k) == value(&row, k))
        }) {
            *previous = row;
        } else {
            holdings.push(row);
        }
    }
    let fallback = if overview {
        BTreeMap::new()
    } else {
        fallback_rates(conn, profile, cutoff)?
    };
    let dest = destination(value(args, "destination"))?;
    if !is_at && truthy(value(profile, "tax_long_term_days")) {
        artifact_csv::history_limit(value(profile, "tax_long_term_days")).map_err(|e| {
            if e.code == "validation" {
                AppError::new("internal_error", "tax_long_term_days cannot be negative")
            } else {
                e
            }
        })?;
    }

    let labels: BTreeMap<_, _> = holdings
        .iter()
        .map(|r| (text(r, "wallet_id"), text(r, "wallet_label")))
        .collect();
    let mut wallet_quantities: BTreeMap<(String, String, String), Decimal> = BTreeMap::new();
    let mut assets: BTreeMap<String, [(Decimal, Decimal); 2]> = BTreeMap::new();
    let (mut income, mut untagged) = (false, false);
    for row in &entries {
        let instant = kassiber_rates::time::parse(&text(row, "occurred_at")).ok_or_else(|| {
            AppError::new(
                "validation",
                "Exit-tax inventory requires a valid journal timestamp.",
            )
        })?;
        if instant >= cutoff {
            continue;
        }
        let kind = text(row, "entry_type");
        let asset = text(row, "asset");
        let quantity = btc(int(row, "quantity"))?;
        if truthy(value(row, "asset"))
            && truthy(value(row, "wallet_id"))
            && !["income", "transfer_fee"].contains(&kind.as_str())
        {
            let wallet = text(row, "wallet_id");
            let label = if truthy(value(row, "wallet_label")) {
                text(row, "wallet_label")
            } else {
                labels
                    .get(&wallet)
                    .cloned()
                    .unwrap_or_else(|| wallet.clone())
            };
            let previous = wallet_quantities
                .entry((wallet, label, asset))
                .or_insert_with(Decimal::zero);
            *previous = previous.add(&quantity, &DECIMAL).map_err(arithmetic)?;
        }
        if ["transfer_in", "transfer_out"].contains(&kind.as_str()) {
            continue;
        }
        if kind == "income" {
            income = true;
            continue;
        }
        let asset = text(row, "asset");
        if !truthy(value(row, "asset")) {
            continue;
        }
        let quantity = btc(int(row, "quantity"))?;
        let regime = usize::from(is_at && alt(row)?);
        let bucket = &mut assets
            .entry(asset)
            .or_insert_with(|| std::array::from_fn(|_| (Decimal::zero(), Decimal::zero())))[regime];
        let basis = exact(row, "cost_basis")?;
        let basis = if quantity >= Decimal::zero() {
            match basis {
                Some(basis) => basis,
                None => exact(row, "fiat_value")?.unwrap_or_else(Decimal::zero),
            }
        } else {
            if is_at && !truthy(value(row, "at_category")) && marker(row).is_none() {
                untagged = true;
            }
            basis
                .unwrap_or_else(Decimal::zero)
                .neg(&DECIMAL)
                .map_err(arithmetic)?
        };
        bucket.0 = bucket.0.add(&quantity, &DECIMAL).map_err(arithmetic)?;
        bucket.1 = bucket.1.add(&basis, &DECIMAL).map_err(arithmetic)?;
    }
    let mut sources_out = Vec::new();
    let mut rates = BTreeMap::new();
    let held_assets: std::collections::BTreeSet<String> = assets
        .iter()
        .filter(|(_, buckets)| buckets.iter().any(|(q, _)| *q > Decimal::zero()))
        .map(|(a, _)| a.clone())
        .chain(
            wallet_quantities
                .iter()
                .filter(|(_, q)| **q > Decimal::zero())
                .map(|((_, _, a), _)| a.clone()),
        )
        .collect();
    for asset in &held_assets {
        let (rate, source) = fmv(conn, asset, &fiat, cutoff, &fallback)?;
        rates.insert(asset.clone(), rate);
        sources_out.push(source);
    }
    let mut lots = Vec::new();
    let (mut neu_qty, mut neu_basis, mut alt_qty) =
        (Decimal::zero(), Decimal::zero(), Decimal::zero());
    let (mut neu_market, mut neu_gain, mut alt_market) = (
        Some(Decimal::zero()),
        Some(Decimal::zero()),
        Some(Decimal::zero()),
    );
    for (asset, buckets) in &assets {
        for (regime, (quantity, basis)) in buckets.iter().enumerate() {
            let qty = quantity.clone().max(Decimal::zero());
            let basis = basis.clone().max(Decimal::zero());
            if qty <= Decimal::zero() {
                continue;
            }
            let market = rates[asset]
                .as_ref()
                .map(|r| qty.mul(r, &DECIMAL).map_err(arithmetic))
                .transpose()?;
            let gain = if regime == 0 {
                market
                    .as_ref()
                    .map(|m| m.sub(&basis, &DECIMAL).map_err(arithmetic))
                    .transpose()?
            } else {
                None
            };
            let category = if regime == 1 {
                "alt_taxfree"
            } else if gain.as_ref().is_none_or(|g| *g >= Decimal::zero()) {
                "neu_gain"
            } else {
                "neu_loss"
            };
            if regime == 0 {
                neu_qty = neu_qty.add(&qty, &DECIMAL).map_err(arithmetic)?;
                neu_basis = neu_basis.add(&basis, &DECIMAL).map_err(arithmetic)?;
                neu_market = add_optional(neu_market, market.as_ref())?;
                neu_gain = add_optional(neu_gain, gain.as_ref())?;
            } else {
                alt_qty = alt_qty.add(&qty, &DECIMAL).map_err(arithmetic)?;
                alt_market = add_optional(alt_market, market.as_ref())?;
            }
            lots.push(dict([
                ("asset", s(asset.as_str())),
                ("regime", s(if regime == 0 { "neu" } else { "alt" })),
                ("quantitySats", sats(&qty)?),
                ("marketValue", eur(market.as_ref())?),
                (
                    "costBasis",
                    if regime == 0 {
                        eur(Some(&basis))?
                    } else {
                        Value::Null
                    },
                ),
                ("gain", eur(gain.as_ref())?),
                ("taxable", Value::Bool(regime == 0)),
                ("category", s(if is_at { category } else { "unrealized" })),
                (
                    "kennzahl",
                    if is_at && regime == 0 && !old_rules {
                        Value::Int((if category == "neu_gain" { 174 } else { 176 }).into())
                    } else {
                        Value::Null
                    },
                ),
            ]));
        }
    }
    let missing: Vec<Value> = sources_out
        .iter()
        .filter(|r| text(r, "source") == "missing")
        .map(|r| value(r, "asset").clone())
        .collect();
    if !missing.is_empty() {
        neu_market = None;
        neu_gain = None;
        alt_market = None;
    }
    let taxable = neu_gain.as_ref().map(|g| g.clone().max(Decimal::zero()));
    let rate: Decimal = "0.275".parse().unwrap();
    let estimated = if is_at && !old_rules {
        taxable
            .as_ref()
            .map(|g| g.mul(&rate, &DECIMAL).map_err(arithmetic))
            .transpose()?
    } else {
        None
    };
    let mut wallet_holdings = Vec::new();
    for ((_, wallet, asset), qty) in wallet_quantities {
        if qty <= Decimal::zero() {
            continue;
        }
        let market = rates
            .get(&asset)
            .and_then(Option::as_ref)
            .map(|r| qty.mul(r, &DECIMAL).map_err(arithmetic))
            .transpose()?;
        wallet_holdings.push(dict([
            ("wallet", s(wallet)),
            ("asset", s(asset)),
            ("quantitySats", sats(&qty)?),
            ("marketValue", eur(market.as_ref())?),
        ]));
    }
    wallet_holdings.sort_by_key(|r| (text(r, "asset"), text(r, "wallet")));
    let mut assumptions = if is_at {
        vec![
            note("_ASSUMPTION_ALT_EXCLUDED"),
            note("_ASSUMPTION_RATE"),
            note(if dest == "eu_eea" {
                "_ASSUMPTION_DEFERRAL_EU"
            } else {
                "_ASSUMPTION_DEFERRAL_THIRD"
            }),
            note("_ASSUMPTION_FMV"),
        ]
    } else {
        vec![note("_ASSUMPTION_GENERIC"), note("_ASSUMPTION_FMV")]
    };
    if is_at {
        if dest == "eu_eea" {
            assumptions.push(note("_ASSUMPTION_FUTURE_TIGHTENING"));
        }
        if income {
            assumptions.push(note("_ASSUMPTION_DERIVED_TOKENS"));
        }
        if untagged {
            assumptions.push(note("_ASSUMPTION_UNTAGGED_DISPOSAL"));
        }
        if old_rules {
            assumptions.push(note("_ASSUMPTION_OLD_RULES"));
        }
    }
    if sources_out
        .iter()
        .any(|r| value(r, "source") == &s("missing"))
    {
        assumptions.push(note("missing"));
    }
    let quarantines: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_quarantines WHERE profile_id=?",
            [text(profile, "id")],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    Ok(dict([
        (
            "workspace",
            if truthy(value(workspace, "label")) {
                value(workspace, "label").clone()
            } else {
                value(profile, "workspace_id").clone()
            },
        ),
        (
            "profile",
            if truthy(value(profile, "label")) {
                value(profile, "label").clone()
            } else {
                value(profile, "id").clone()
            },
        ),
        ("jurisdictionCode", s(if is_at { "AT" } else { "generic" })),
        ("fiatCurrency", s(fiat)),
        ("departureDate", s(day)),
        (
            "departureTimezone",
            s(if is_at { "Europe/Vienna" } else { "UTC" }),
        ),
        ("destination", s(dest.as_str())),
        (
            "method",
            s(
                kassiber_pytext::strip(&if truthy(value(profile, "gains_algorithm")) {
                    text(profile, "gains_algorithm")
                } else if is_at {
                    "moving_average_at".into()
                } else {
                    "fifo".into()
                })
                .to_lowercase(),
            ),
        ),
        (
            "journalInputVersion",
            value(profile, "journal_input_version").clone(),
        ),
        ("fmvSource", Value::List(sources_out)),
        (
            "totals",
            dict([
                ("neuQuantitySats", sats(&neu_qty)?),
                ("neuMarketValue", eur(neu_market.as_ref())?),
                ("neuCostBasis", eur(Some(&neu_basis))?),
                ("neuGain", eur(neu_gain.as_ref())?),
                ("altQuantitySats", sats(&alt_qty)?),
                ("altMarketValue", eur(alt_market.as_ref())?),
                ("taxableGain", eur(taxable.as_ref())?),
                (
                    "estimatedTaxRate",
                    if is_at && !old_rules {
                        float(&rate)?
                    } else {
                        Value::Null
                    },
                ),
                ("estimatedTax", eur(estimated.as_ref())?),
                (
                    "collectionTiming",
                    s(if dest == "eu_eea" {
                        "deferred"
                    } else {
                        "immediate"
                    }),
                ),
            ]),
        ),
        ("lots", Value::List(lots)),
        ("walletHoldings", Value::List(wallet_holdings)),
        ("assumptions", Value::List(assumptions)),
        ("reviewGate", note("EXIT_TAX_REVIEW_GATE")),
        (
            "status",
            dict([
                ("needsJournals", Value::Bool(entries.is_empty())),
                ("quarantines", Value::Int(quarantines.into())),
                ("missingFmvAssets", Value::List(missing)),
            ]),
        ),
    ]))
}
fn add_optional(a: Option<Decimal>, b: Option<&Decimal>) -> Result<Option<Decimal>> {
    a.zip(b)
        .map(|(a, b)| a.add(b, &DECIMAL).map_err(arithmetic))
        .transpose()
}

pub(super) fn render(report: &Value) -> Result<String> {
    Ok(lines(report)?.join("\n"))
}

/// `format_exit_tax_lines`: the report's text lines.
pub(super) fn lines(report: &Value) -> Result<Vec<String>> {
    let totals = value(report, "totals");
    let ccy = text(report, "fiatCurrency");
    let money = |key| {
        if matches!(value(totals, key), Value::Null) {
            "—".into()
        } else {
            format!("{} {ccy}", summary_text::number(value(totals, key), 2))
        }
    };
    let btc = |key| -> Result<String> {
        decimal(value(totals, key))?
            .div(&Decimal::from(100_000_000), &DECIMAL)
            .map_err(arithmetic)?
            .to_fixed_places(8, Rounding::HalfEven)
            .map_err(arithmetic)
    };
    let mut lines = vec![
        format!(
            "Exit-tax estimate — {} ({})",
            text(report, "profile"),
            text(report, "jurisdictionCode")
        ),
        format!(
            "Departure date: {}    Destination: {}",
            text(report, "departureDate"),
            text(report, "destination")
        ),
        format!("Departure timezone: {}", text(report, "departureTimezone")),
        format!("Accounting method: {}", text(report, "method")),
        format!(
            "Journal input version: {}",
            if matches!(value(report, "journalInputVersion"), Value::Null) {
                "unavailable".into()
            } else {
                py_str(value(report, "journalInputVersion"))
            }
        ),
        "FMV quotes:".into(),
    ];
    for quote in artifact_csv::list(value(report, "fmvSource")) {
        let rate = if matches!(value(quote, "rate"), Value::Null) {
            "—".into()
        } else {
            format!("{} {ccy}", summary_text::number(value(quote, "rate"), 2))
        };
        lines.push(format!(
            "  {} ({}): {rate}; source: {}; as of: {}",
            text(quote, "asset"),
            text(quote, "pair"),
            text(quote, "source"),
            if truthy(value(quote, "asOf")) {
                text(quote, "asOf")
            } else {
                "unavailable".into()
            }
        ));
    }
    let missing = artifact_csv::list(value(value(report, "status"), "missingFmvAssets"));
    if !missing.is_empty() {
        lines.push(format!(
            "Incomplete estimate: missing FMV for {}",
            missing.iter().map(py_str).collect::<Vec<_>>().join(", ")
        ));
    }
    lines.extend([
        String::new(),
        "Deemed disposal at fair market value:".into(),
        format!(
            "  Neubestand (taxable):  {} @ market {}, basis {}, gain {}",
            btc("neuQuantitySats")?,
            money("neuMarketValue"),
            money("neuCostBasis"),
            money("neuGain")
        ),
        format!(
            "  Altbestand (excluded): {} @ market {} — tax-free, not in the base",
            btc("altQuantitySats")?,
            money("altMarketValue")
        ),
        String::new(),
        format!("  Taxable gain:        {}", money("taxableGain")),
    ]);
    if !matches!(value(totals, "estimatedTaxRate"), Value::Null) {
        lines.push("  Rate:                27.5%".into());
    }
    lines.push(format!("  Estimated exit tax:  {}", money("estimatedTax")));
    lines.push(
        if text(totals, "collectionTiming") == "deferred" {
            "  Collection:          assessed but deferred until you sell (EU/EEA Nichtfestsetzung)"
        } else {
            "  Collection:          due immediately on departure (third country)"
        }
        .into(),
    );
    lines.push(String::new());
    let quarantines = int(value(report, "status"), "quarantines");
    if quarantines != 0 {
        lines.push(format!(
            "⚠ {quarantines} quarantined transaction(s) — estimate is incomplete until resolved."
        ));
        lines.push(String::new());
    }
    lines.push("Assumptions:".into());
    for note in artifact_csv::list(value(report, "assumptions")) {
        lines.push(format!("  - {}", py_str(note)));
    }
    lines.push(String::new());
    lines.push(text(report, "reviewGate"));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_rate_is_the_last_same_time_row_in_source_order() {
        let conn = crate::same_time_rate_book();
        let profile = dict([("id", Value::from("p"))]);
        let cutoff = kassiber_rates::time::parse("2025-01-01T00:00:00Z").unwrap();
        let rates = fallback_rates(&conn, &profile, cutoff).unwrap();
        assert_eq!(rates["BTC"].0.to_string(), "10000");
    }
    struct Fixed;
    impl kassiber_setup::Sources for Fixed {
        fn now(&self, _: &Connection) -> Result<String> {
            Ok("2026-01-02T03:04:05Z".into())
        }
        fn uuid(&self, _: &Connection) -> Result<String> {
            Ok("00000000-0000-4000-8000-000000000001".into())
        }
    }
    #[test]
    fn departure_defaults_to_the_jurisdiction_day_at_vienna_midnight() {
        struct Midnight;
        impl kassiber_setup::Sources for Midnight {
            fn now(&self, _: &Connection) -> Result<String> {
                Ok("2025-01-01T23:30:00Z".into())
            }
            fn uuid(&self, _: &Connection) -> Result<String> {
                unreachable!()
            }
        }
        let conn = Connection::open_in_memory().unwrap();
        for (is_at, day, cutoff) in [
            (true, "2025-01-02", "2025-01-02T23:00:00Z"),
            (false, "2025-01-01", "2025-01-02T00:00:00Z"),
        ] {
            assert_eq!(
                departure(
                    &conn,
                    &dict::<&str>([]),
                    &Midnight,
                    is_at.then_some("Europe/Vienna")
                )
                .unwrap(),
                (day.into(), kassiber_rates::time::parse(cutoff).unwrap())
            );
        }
    }
    fn normalize_numbers(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Number(number)
                if number.as_u64().is_none() && number.as_i64().is_none() =>
            {
                *number = serde_json::Number::from_f64(number.as_f64().unwrap()).unwrap();
            }
            serde_json::Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
            serde_json::Value::Object(fields) => fields.values_mut().for_each(normalize_numbers),
            _ => (),
        }
    }

    fn encoded(v: &Value) -> String {
        kassiber_pyjson::dumps(
            v,
            &kassiber_pyjson::Options {
                sort_keys: true,
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn error(e: AppError) -> Value {
        dict([
            ("code", s(e.code)),
            ("message", s(e.message)),
            ("hint", e.hint.map(s).unwrap_or(Value::Null)),
            ("details", e.details.unwrap_or(Value::Null)),
            ("retryable", Value::Bool(e.retryable)),
        ])
    }
    use crate::artifact_csv::list;
    /// The design (PDF and workbook) shows the report's own figures: each
    /// lot's quantity and gain, every quote, the departure time zone, the
    /// input stamp, the quarantine blocker and the review gate.
    fn design_shows_report(doc: &crate::report_design::Document, report: &Value, name: &str) {
        use crate::report_design::{Block, Cell};
        let number = |cell: &Cell| -> f64 {
            cell.csv()
                .parse()
                .unwrap_or_else(|_| panic!("{name}: {cell:?}"))
        };
        let float = |v: &Value| match v {
            Value::Float(f) => Some(*f),
            Value::Int(n) => Some(n.to_string().parse::<f64>().unwrap()),
            _ => None,
        };
        let section = |id: &str| doc.sections.iter().find(|s| s.id == id).unwrap();
        let table = |id: &str| {
            doc.sections
                .iter()
                .flat_map(|s| &s.blocks)
                .find_map(|b| match b {
                    Block::Table(t) if t.id == id => Some(t.clone()),
                    _ => None,
                })
        };
        let facts: Vec<_> = section("scope")
            .blocks
            .iter()
            .flat_map(|b| match b {
                Block::Facts(facts) => facts.clone(),
                _ => Vec::new(),
            })
            .collect();
        let fact = |key: &str| facts.iter().find(|f| f.key == key).unwrap().value.csv();
        assert_eq!(
            fact("departure_timezone"),
            text(report, "departureTimezone"),
            "{name}"
        );
        let stamp = match value(report, "journalInputVersion") {
            Value::Null => String::new(),
            v => py_str(v),
        };
        assert_eq!(fact("journal_input_version"), stamp, "{name}: input stamp");
        // A report without lots shows no lot table.
        let lots = table("deemed_disposal").map(|t| t.rows).unwrap_or_default();
        assert_eq!(lots.len(), list(value(report, "lots")).len(), "{name}");
        for (cells, lot) in lots.iter().zip(list(value(report, "lots"))) {
            let sats = int(lot, "quantitySats");
            assert_eq!(number(&cells[2]), sats as f64 / 1e8, "{name}: lot quantity");
            assert_eq!(
                cells[3].csv(),
                (sats * 1000).to_string(),
                "{name}: lot msat"
            );
            match float(value(lot, "gain")) {
                Some(gain) => assert_eq!(number(&cells[6]), gain, "{name}: lot gain"),
                None => assert!(matches!(cells[6], Cell::Empty), "{name}: lot gain"),
            }
        }
        let quotes = table("fmv_quotes").unwrap();
        assert_eq!(
            quotes.rows.len(),
            list(value(report, "fmvSource")).len(),
            "{name}: quotes"
        );
        for (cells, quote) in quotes.rows.iter().zip(list(value(report, "fmvSource"))) {
            match float(value(quote, "rate")) {
                Some(rate) => assert_eq!(number(&cells[2]), rate, "{name}: quote rate"),
                None => assert!(matches!(cells[2], Cell::Empty), "{name}: quote rate"),
            }
        }
        let status = value(report, "status");
        let blockers = section("summary")
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Note { accent: true, .. }))
            .count();
        let expected = usize::from(!list(value(status, "missingFmvAssets")).is_empty())
            + usize::from(int(status, "quarantines") > 0);
        assert_eq!(blockers, expected, "{name}: blockers");
        let gate = section("method")
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Note { text, accent: false } if !text.is_empty()));
        assert_eq!(
            gate,
            !text(report, "reviewGate").is_empty(),
            "{name}: review gate"
        );
    }
    #[test]
    fn exit_tax_vectors_pin_inventory_rates_disclosures_and_readiness() {
        let mut cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/exit_tax.json")).unwrap();
        normalize_numbers(&mut cases);
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        for case in cases.as_array().unwrap() {
            conn.execute_batch("SAVEPOINT exit_vector").unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let result = execute_with_sources(conn, "reports.exit-tax", &args, &Fixed);
            if case["expected"].get("error").is_some() {
                let expected =
                    kassiber_pytext::loads(&case["expected"]["error"].to_string()).unwrap();
                assert_eq!(
                    encoded(&error(result.unwrap_err())),
                    encoded(&expected),
                    "{}",
                    case["name"]
                );
                assert_eq!(
                    encoded(&error(exit_xlsx::project(conn, &args, &Fixed).unwrap_err())),
                    encoded(&expected),
                    "{} workbook",
                    case["name"]
                );
            } else {
                let report = result.unwrap();
                let expected =
                    kassiber_pytext::loads(&case["expected"]["report"].to_string()).unwrap();
                assert_eq!(encoded(&report), encoded(&expected), "{}", case["name"]);
                assert_eq!(
                    render(&report).unwrap(),
                    case["expected"]["text"].as_str().unwrap(),
                    "{} text",
                    case["name"]
                );
                // The workbook is the shared design over this same report.
                let (_, doc) = exit_xlsx::project(conn, &args, &Fixed).unwrap();
                let names: Vec<&str> = doc.sheets.iter().map(|s| s.name.as_str()).collect();
                assert_eq!(
                    names[..2],
                    ["summary", "deemed_disposal_totals"],
                    "{} workbook",
                    case["name"]
                );
                assert!(
                    kassiber_xlsx::render(&doc).is_ok(),
                    "{} workbook",
                    case["name"]
                );
                let model = exit_pdf::model(conn, &args, &Fixed).unwrap();
                design_shows_report(
                    &crate::report_design::exit_tax(&model),
                    &report,
                    case["name"].as_str().unwrap(),
                );
                // The PDF renders this same report in the shared design, the
                // same bytes every time.
                let pdf = exit_pdf::prepare(conn, &args, &Fixed).unwrap();
                let again = exit_pdf::prepare(conn, &args, &Fixed).unwrap();
                assert!(pdf.files[0].bytes.starts_with(b"%PDF-"), "{}", case["name"]);
                assert!(
                    pdf.files[0].bytes == again.files[0].bytes,
                    "{} PDF bytes repeat",
                    case["name"]
                );
            }
            conn.execute_batch("ROLLBACK TO exit_vector; RELEASE exit_vector")
                .unwrap();
        }
    }
    #[test]
    fn exit_departure_errors_distinguish_malformed_days_from_naive_times() {
        let conn = Connection::open_in_memory().unwrap();
        for is_at in [false, true] {
            for (input, message) in [
                ("20240331", "Invalid exit-tax departure date"),
                ("2024-03-31Z", "Invalid exit-tax departure date"),
                ("garbage", "Invalid exit-tax departure date"),
                ("2024-02-30", "Invalid exit-tax departure date"),
                ("1711843200", "Invalid exit-tax departure date"),
                ("T24:00", "Invalid exit-tax departure date"),
                (
                    "2024-03-31T12:00:00",
                    "Exit-tax departure time requires an offset",
                ),
            ] {
                let args = dict([("departure_date", s(input))]);
                let error =
                    departure(&conn, &args, &Fixed, is_at.then_some("Europe/Vienna")).unwrap_err();
                assert_eq!(error.code, "validation", "{input}");
                assert_eq!(error.message, message, "{input}");
                assert_eq!(error.hint.as_deref(), Some("Use a date (YYYY-MM-DD) or a date-time with an offset, e.g. 2025-01-01T12:00:00+01:00."));
            }
        }
    }
    #[test]
    fn exit_managed_filename_and_workbook_share_the_resolved_day_across_midnight() {
        struct Midnight(std::cell::Cell<usize>);
        impl kassiber_setup::Sources for Midnight {
            fn now(&self, _: &Connection) -> Result<String> {
                let call = self.0.get();
                self.0.set(call + 1);
                Ok(if call == 0 {
                    "2024-03-31T23:30:00Z"
                } else {
                    "2024-04-02T00:30:00Z"
                }
                .into())
            }
            fn uuid(&self, _: &Connection) -> Result<String> {
                unreachable!()
            }
        }
        for country in ["at", "generic"] {
            for departure in [Value::Null, s("2024-03-31"), s("2024-03-31T23:30:00Z")] {
                let root = tempfile::tempdir().unwrap();
                let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
                let conn = book.connection();
                conn.execute_batch(include_str!("../tests/data/book.sql"))
                    .unwrap();
                conn.execute("UPDATE profiles SET tax_country=?", [country])
                    .unwrap();
                let args = dict([("departure_date", departure.clone())]);
                let output = export_request::execute(
                    conn,
                    "ui.reports.export_exit_tax_xlsx",
                    &args,
                    &root.path().join("exports"),
                    &Midnight(std::cell::Cell::new(0)),
                )
                .unwrap();
                let day = if country == "at" && departure != s("2024-03-31") {
                    "2024-04-01"
                } else {
                    "2024-03-31"
                };
                assert_eq!(text(&output, "departure_date"), day);
                let filename = text(&output, "filename");
                assert!(
                    filename.starts_with(&format!("kassiber-exit-tax-{day}-")),
                    "{filename}"
                );
                assert!(!filename
                    .chars()
                    .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)));
                assert!(std::path::Path::new(&text(&output, "file")).is_file());
            }
        }
    }
    #[test]
    fn exit_xlsx_managed_publication_preserves_reference_without_snapshots() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        let args = dict([
            ("departure_date", s("2025-01-01")),
            ("destination", s("third_country")),
        ]);
        let output = export_request::execute(
            conn,
            "ui.reports.export_exit_tax_xlsx",
            &args,
            &root.path().join("exports"),
            &Fixed,
        )
        .unwrap();
        assert_eq!(value(&output, "format"), &s("xlsx"));
        assert_eq!(value(&output, "scope"), &s("exit_tax"));
        assert_eq!(value(&output, "departure_date"), &s("2025-01-01"));
        assert_eq!(value(&output, "destination"), &s("third_country"));
        assert_eq!(
            value(&output, "filename"),
            &s("kassiber-exit-tax-2025-01-01-20260102-030405.xlsx")
        );
        let bytes = std::fs::read(text(&output, "file")).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert_eq!(int(&output, "bytes") as usize, bytes.len());
        assert!(get(&output, "report_snapshot").is_none());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        conn.execute_batch("INSERT INTO freshness_source_states(source_key,profile_id,source_type,source_label,status,blocking_reports,updated_at) VALUES('blocked','p','wallet','Cold','failed',1,'fixed')").unwrap();
        assert_eq!(
            export_request::execute(
                conn,
                "ui.reports.export_exit_tax_xlsx",
                &args,
                &root.path().join("exports"),
                &Fixed
            )
            .unwrap_err()
            .code,
            "report_freshness_blocked"
        );
    }
}
