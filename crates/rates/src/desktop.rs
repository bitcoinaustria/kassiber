//! Desktop projections; AI projections remain in Python.
use super::*;

fn active_profile(conn: &Connection) -> Result<String> {
    let workspace = setting(conn, "context_workspace")?
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "app_error",
                "No workspace selected. Create one or run `kassiber context set --workspace ...`.",
            )
        })?;
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id=?)",
            [&workspace],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    if !exists {
        return Err(AppError::new(
            "not_found",
            format!("Workspace '{workspace}' not found"),
        ));
    }
    let profile = setting(conn, "context_profile")?
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "app_error",
                "No profile selected. Create one or run `kassiber context set --profile ...`.",
            )
        })?;
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM profiles WHERE id=? AND workspace_id=?)",
            [&profile, &workspace],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    if !exists {
        return Err(AppError::new(
            "not_found",
            format!("Profile '{profile}' not found in the selected workspace"),
        ));
    }
    Ok(profile)
}
fn active_pair(conn: &Connection, profile: &str) -> Result<String> {
    let fiat: String = conn
        .query_row(
            "SELECT fiat_currency FROM profiles WHERE id=?",
            [profile],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    transaction_rate_pair("BTC", &fiat)
        .ok_or_else(|| validation(format!("BTC market rates are not supported for {fiat}")))
}
fn string_arg<'a>(args: &'a Value, kind: &str, name: &str) -> Result<Option<&'a str>> {
    match get(args, name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(s)) => Ok(s.as_str()),
        _ => Err(validation(format!("{kind} {name} must be a string"))),
    }
}
fn summary(conn: &Connection) -> Result<Value> {
    let mut pairs=query(conn,"SELECT pair,COUNT(*) AS sample_count,MIN(timestamp) AS first_timestamp,MAX(timestamp) AS last_timestamp FROM rates_cache GROUP BY pair ORDER BY pair",[])?;
    for row in &mut pairs {
        let pair = text(row, "pair").unwrap_or("");
        let latest = latest(conn, pair)?;
        let latest = Value::Dict(match latest {
            Value::Dict(fields) => fields
                .into_iter()
                .filter(|(k, _)| *k != kassiber_pyjson::Key::from("pair"))
                .collect(),
            _ => unreachable!(),
        });
        if let Value::Dict(fields) = row {
            fields.push(("latest".into(), latest))
        }
    }
    let count = pairs.len();
    Ok(dict([
        ("pairs", Value::List(pairs)),
        ("summary", dict([("cached_pair_count", Value::from(count))])),
    ]))
}
pub fn coverage(conn: &Connection, args: &Value) -> Result<Value> {
    if let Value::Dict(fields) = args {
        let mut unknown: Vec<_> = fields
            .iter()
            .map(|(k, _)| match k {
                kassiber_pyjson::Key::Str(t) => t.as_str().unwrap_or(""),
                _ => "",
            })
            .filter(|k| *k != "limit")
            .collect();
        unknown.sort();
        if !unknown.is_empty() {
            return Err(
                validation("ui.rates.coverage received unsupported filters").with_details(dict([
                    ("unknown", Value::List(unknown.into_iter().map(s).collect())),
                ])),
            );
        }
    }
    let raw = get(args, "limit").cloned().unwrap_or(Value::from(25));
    let details = || dict([("limit", raw.clone())]);
    let limit = if raw == Value::Null {
        Err(validation("limit must be an integer"))
    } else {
        integer(args, "limit", 25)
    }
    .map_err(|e| e.with_details(details()))?;
    if limit <= 0 {
        return Err(validation("limit must be positive").with_details(details()));
    }
    let limit = limit.min(100) as usize;
    let profile = setting(conn, "context_profile")?.unwrap_or_default();
    let workspace = setting(conn, "context_workspace")?.unwrap_or_default();
    let profiles=query(conn,"SELECT p.id,p.label,p.fiat_currency,w.label AS workspace FROM profiles p JOIN workspaces w ON w.id=p.workspace_id WHERE p.id=? AND w.id=?",[&profile, &workspace])?;
    let empty = dict([
        ("active_transactions", Value::from(0)),
        ("priced_transactions", Value::from(0)),
        ("missing_price_transactions", Value::from(0)),
        ("cache_coverable_missing", Value::from(0)),
        ("cache_uncovered_missing", Value::from(0)),
    ]);
    let Some(p) = profiles.first() else {
        return Ok(dict([
            ("workspace", Value::Null),
            ("profile", Value::Null),
            ("summary", empty),
            ("items", Value::List(vec![])),
            ("filters", dict([("limit", Value::from(limit))])),
        ]));
    };
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
            [&profile],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let rows=query(conn,&format!("SELECT t.id,t.external_id,t.occurred_at,t.direction,t.asset,t.amount,t.fiat_currency,w.label AS wallet FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0 AND (t.amount>0 OR t.fee>0) AND {} ORDER BY t.occurred_at,t.created_at,t.id",providers::missing_price_sql("t.")),[profile])?;
    let mut items = Vec::new();
    let mut coverable = 0;
    for row in &rows {
        let asset = text(row, "asset").unwrap_or("").to_uppercase();
        let fiat = text(row, "fiat_currency")
            .filter(|v| !v.is_empty())
            .or_else(|| text(p, "fiat_currency"))
            .unwrap_or("EUR")
            .to_uppercase();
        let pair = format!("{}-{fiat}", if asset == "LBTC" { "BTC" } else { &asset });
        let at = text(row, "occurred_at").unwrap_or("");
        // Preserve this display projection's base ordering (it differs from
        // the pricing lookup's manual preference and uses occurred_at only).
        let cache=query(conn,"SELECT timestamp FROM rates_cache WHERE pair=? AND timestamp<=? ORDER BY timestamp DESC,fetched_at DESC LIMIT 1",[&pair,at])?.pop();
        if cache.is_some() {
            coverable += 1
        }
        if items.len() >= limit {
            continue;
        }
        let amount = integer(row, "amount", 0)?
            * if text(row, "direction") == Some("inbound") {
                1
            } else {
                -1
            };
        let sat = if amount % 1000 == 0 {
            Value::from(amount / 1000)
        } else {
            Value::Float(amount as f64 / 1000.0)
        };
        let field = |k| get(row, k).cloned().unwrap_or(Value::Null);
        items.push(dict([
            ("id", field("id")),
            ("externalId", field("external_id")),
            ("date", field("occurred_at")),
            ("wallet", field("wallet")),
            ("direction", field("direction")),
            ("asset", s(asset)),
            ("amountSat", sat),
            ("amountMsat", Value::from(amount)),
            ("fiatCurrency", s(fiat)),
            ("cachePair", s(pair)),
            ("cacheHasRate", Value::Bool(cache.is_some())),
            (
                "cacheRateAt",
                cache
                    .and_then(|r| get(&r, "timestamp").cloned())
                    .unwrap_or(Value::Null),
            ),
        ]));
    }
    Ok(dict([
        ("workspace", optional(text(p, "workspace"))),
        ("profile", optional(text(p, "label"))),
        (
            "summary",
            dict([
                ("active_transactions", Value::from(active)),
                (
                    "priced_transactions",
                    Value::from(active - rows.len() as i64),
                ),
                ("missing_price_transactions", Value::from(rows.len())),
                ("cache_coverable_missing", Value::from(coverable)),
                (
                    "cache_uncovered_missing",
                    Value::from(rows.len() as i64 - coverable),
                ),
            ]),
        ),
        ("items", Value::List(items)),
        ("filters", dict([("limit", Value::from(limit))])),
    ]))
}
pub fn run(conn: &Connection, kind: &str, args: &Value, providers: &Providers) -> Result<Value> {
    match kind {
        "ui.rates.summary" => summary(conn),
        "ui.rates.coverage" => coverage(conn, args),
        "ui.rates.latest" => {
            if let Value::Dict(fields) = args {
                let mut unknown: Vec<_> = fields
                    .iter()
                    .filter_map(|(k, _)| match k {
                        kassiber_pyjson::Key::Str(t) => t.as_str(),
                        _ => None,
                    })
                    .filter(|k| !["pair", "source"].contains(k))
                    .collect();
                unknown.sort();
                if !unknown.is_empty() {
                    return Err(validation("ui.rates.latest received unsupported arguments")
                        .with_details(dict([(
                            "unknown",
                            Value::List(unknown.into_iter().map(s).collect()),
                        )])));
                }
            }
            let source = string_arg(args, kind, "source")?
                .filter(|v| !v.trim().is_empty())
                .map(normalize_market_provider)
                .transpose()?
                .unwrap_or(market_provider(conn)?);
            let requested = string_arg(args, kind, "pair")?
                .filter(|v| !v.trim().is_empty())
                .map(require_supported_pair)
                .transpose()?;
            let profile = active_profile(conn)?;
            providers::require_live_enabled(conn, &profile)?;
            let pair = match requested {
                Some(pair) => pair,
                None => active_pair(conn, &profile)?,
            };
            let consent = FetchConsent::explicit(conn, FetchAction::Latest, &source, Some(&pair))?;
            let latest_quotes = providers.latest(
                conn,
                Some(&pair),
                Some(&source),
                Some(&consent),
                FetchAction::Latest,
            )?;
            let cached = match latest(conn, &pair) {
                Ok(v) => Some(v),
                Err(e) if e.code == "not_found" => None,
                Err(e) => return Err(e),
            };
            let market = cached
                .map(|r| {
                    let field = |k| get(&r, k).cloned().unwrap_or(Value::Null);
                    dict([
                        ("asset", s("BTC")),
                        ("fiatCurrency", s(pair.split_once('-').unwrap().1)),
                        ("pair", s(pair.as_str())),
                        ("rate", field("rate")),
                        ("timestamp", field("timestamp")),
                        ("source", field("source")),
                        ("fetchedAt", field("fetched_at")),
                        ("granularity", field("granularity")),
                        ("method", field("method")),
                    ])
                })
                .unwrap_or(Value::Null);
            Ok(dict([
                ("source", s(source)),
                ("pair", s(pair)),
                ("latest", latest_quotes),
                ("marketRate", market),
            ]))
        }
        "ui.rates.kraken_csv.import" => {
            let bundled = get(args, "use_bundled").is_some_and(truthy)
                || get(args, "bundled").is_some_and(truthy);
            let path = text(args, "path");
            if !bundled && path.is_none_or(|p| p.trim().is_empty()) {
                return Err(validation("ui.rates.kraken_csv.import requires args.path").with_hint("Choose a local Kraken OHLCVT .zip or .csv archive, or use the bundled BTC hourly seed."));
            }
            if bundled {
                string_arg(args, kind, "path")?;
            }
            let operation = get(args, "operation")
                .filter(|v| truthy(v))
                .or_else(|| get(args, "mode").filter(|v| truthy(v)))
                .map(py_str)
                .unwrap_or_else(|| "full".into())
                .trim()
                .to_lowercase();
            if !["full", "incremental"].contains(&operation.as_str()) {
                return Err(validation(
                    "ui.rates.kraken_csv.import operation must be full or incremental",
                ));
            }
            let pair = string_arg(args, kind, "pair")?
                .map(str::trim)
                .filter(|p| !p.is_empty());
            let path = if bundled {
                csv::bundled_path(true).to_string_lossy().into_owned()
            } else {
                path.unwrap().trim().into()
            };
            let args = dict([
                ("source", s("kraken-csv")),
                ("pair", optional(pair)),
                ("path", s(path.as_str())),
            ]);
            let summary = if bundled {
                let summary = csv::sync_bundled(conn, pair, true)?;
                commit(conn)?;
                summary
            } else {
                command(conn, "sync", &args, providers, None)?
            };
            let Value::List(rows) = &summary else {
                unreachable!()
            };
            let total = |key: &str| {
                rows.iter()
                    .map(|r| integer(r, key, 0).unwrap_or(0))
                    .sum::<i64>()
            };
            let skipped = rows
                .iter()
                .map(|r| integer(r, "skipped_files", 0).unwrap_or(0))
                .max()
                .unwrap_or(0);
            let totals = dict([
                ("pairs", Value::from(rows.len())),
                ("samples", Value::from(total("samples"))),
                ("rows", Value::from(total("rows"))),
                ("files", Value::from(total("files"))),
                ("skipped_rows", Value::from(total("skipped_rows"))),
                ("skipped_files", Value::from(skipped)),
            ]);
            Ok(dict([
                ("source", s("kraken-csv")),
                ("operation", s(operation)),
                ("path", s(path)),
                ("bundled", Value::Bool(bundled)),
                ("pair", optional(pair)),
                ("summary", summary),
                ("totals", totals),
            ]))
        }
        // Repricing includes journal composition, whose slice is unported.
        // Refuse before deleting cache/prices or contacting any provider.
        "ui.rates.rebuild" => Err(AppError::new(
            "not_implemented",
            "Desktop rate rebuild requires the unported pricing and journal services",
        )),
        _ => Err(AppError::new(
            "not_implemented",
            "Rates operation is not implemented",
        )),
    }
}
