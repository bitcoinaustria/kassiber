use super::*;

const METHODS: &[&str] = &[
    "FIFO",
    "LIFO",
    "HIFO",
    "LOFO",
    "MOVING_AVERAGE",
    "MOVING_AVERAGE_AT",
];
const FIELDS: &[&str] = &[
    "label",
    "fiat_currency",
    "tax_country",
    "tax_long_term_days",
    "gains_algorithm",
    "require_coarse_review",
    "bitcoin_rail_carrying_value",
    "cost_basis_pool_scope",
];
fn negative(value: &Value) -> bool {
    matches!(value, Value::Int(n) if n < &0.into())
}
fn number(value: &Value) -> Result<i64> {
    match value {
        Value::Int(n) => i64::try_from(n.clone()).map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        }),
        _ => Err(AppError::new(
            "validation",
            "Tax long-term days must be an integer",
        )),
    }
}
fn policy(values: &Value, check_algorithm: bool, create: bool) -> Result<Value> {
    let raw_country = optional(values, "tax_country")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| "generic".into());
    let country = kassiber_pytext::strip(&raw_country).to_lowercase();
    let country = if country.is_empty() {
        "generic"
    } else {
        &country
    };
    if !["generic", "at"].contains(&country) {
        // The reference raises ValueError before its AppError wrapper here.
        return Err(AppError::new(
            "internal_error",
            format!("Unsupported tax country '{raw_country}'"),
        ));
    }
    let days = get(values, "tax_long_term_days")
        .cloned()
        .unwrap_or(Value::Int(365.into()));
    if country != "at" && negative(&days) {
        let e = AppError::new(
            if create { "app_error" } else { "validation" },
            "Tax long-term days cannot be negative",
        );
        return Err(if create {
            e
        } else {
            e.with_hint("Use a non-negative integer; pass 0 to treat every disposal as short-term.")
        });
    }
    let raw_algo = optional(values, "gains_algorithm").map(py_str);
    if check_algorithm {
        if let Some(a) = &raw_algo {
            if !METHODS.contains(&a.to_uppercase().as_str()) {
                return Err(AppError::new(
                    "validation",
                    format!("Unsupported gains algorithm '{a}'"),
                )
                .with_hint(format!("Choose one of: {}", METHODS.join(", "))));
            }
        }
    }
    let algo = if check_algorithm {
        kassiber_pytext::strip(raw_algo.as_deref().filter(|v| !v.is_empty()).unwrap_or(
            if country == "at" {
                "MOVING_AVERAGE_AT"
            } else {
                "FIFO"
            },
        ))
        .to_uppercase()
    } else {
        raw_algo.unwrap_or_default()
    };
    if check_algorithm && country == "generic" && algo == "MOVING_AVERAGE_AT" {
        return Err(AppError::new(
            "validation",
            format!(
                "Unsupported gains algorithm '{}' for tax_country='generic'",
                text(values, "gains_algorithm")
            ),
        )
        .with_hint("Choose one of: FIFO, HIFO, LIFO, LOFO, MOVING_AVERAGE"));
    }
    let raw_scope = optional(values, "cost_basis_pool_scope")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| "global".into());
    let scope = kassiber_pytext::strip(&raw_scope).to_lowercase();
    if scope != "global" {
        return Err(AppError::new(
            "validation",
            format!(
                "Unsupported cost-basis pool scope '{}'",
                text(values, "cost_basis_pool_scope")
            ),
        )
        .with_hint("Choose one of: global"));
    }
    let mut result = values.clone();
    replace(&mut result, "tax_country", s(country));
    replace(
        &mut result,
        "fiat_currency",
        s(if country == "at" {
            "EUR".into()
        } else {
            kassiber_pytext::strip(&text(values, "fiat_currency")).to_uppercase()
        }),
    );
    replace(
        &mut result,
        "tax_long_term_days",
        if country == "at" {
            Value::Int(i64::MAX.into())
        } else if !truthy(&days) {
            Value::Int(365.into())
        } else {
            days
        },
    );
    replace(&mut result, "gains_algorithm", s(algo));
    replace(&mut result, "cost_basis_pool_scope", s(scope));
    Ok(result)
}
pub(crate) fn create(conn: &Connection, args: &Value, sources: &dyn Sources) -> Result<Value> {
    let w = references::workspace(conn, args)?;
    let wid = text(&w, "id");
    // Negative holding days are refused before country/method validation.
    if get(args, "tax_long_term_days").is_some_and(negative) {
        return Err(AppError::new(
            "app_error",
            "Tax long-term days cannot be negative",
        ));
    }
    let normalized = policy(args, true, true)?;
    let id = sources.uuid(conn)?;
    conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,tax_long_term_days,gains_algorithm,bitcoin_rail_carrying_value,cost_basis_pool_scope,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)",rusqlite::params![id,wid,text(args,"label"),text(&normalized,"fiat_currency"),text(&normalized,"tax_country"),number(get(&normalized,"tax_long_term_days").unwrap())?,text(&normalized,"gains_algorithm"),optional(args,"bitcoin_rail_carrying_value").is_none_or(truthy),text(&normalized,"cost_basis_pool_scope"),sources.now(conn)?]).map_err(|e| {
        if integrity(&e) { AppError::new("conflict",format!("Profile '{}' already exists in workspace '{}'",text(args,"label"),text(&w,"label"))).with_hint("Choose a different profile label or update the existing profile.") }
        else { sql_error(e) }
    })?;
    conn.execute("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) VALUES(?,?,?,'treasury','Treasury','asset','BTC',?)",rusqlite::params![sources.uuid(conn)?,wid,id,sources.now(conn)?]).map_err(sql_error)?;
    set_setting(conn, "context_workspace", &wid)?;
    set_setting(conn, "context_profile", &id)?;
    one(conn, "SELECT * FROM profiles WHERE id=?", [id])
}
fn stored_pool_scope(p: &Value) -> Result<String> {
    // Detail reads validate the persisted policy too, as profile_cost_basis_pool_scope
    // and build_tax_policy do in Python. Listing raw rows has no such validation.
    let raw_country = optional(p, "tax_country")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| "generic".into());
    let country = kassiber_pytext::strip(&raw_country).to_lowercase();
    if !["", "generic", "at"].contains(&country.as_str()) {
        return Err(AppError::new(
            "internal_error",
            format!("Unsupported tax country '{raw_country}'"),
        ));
    }
    if country != "at" && get(p, "tax_long_term_days").is_some_and(negative) {
        return Err(AppError::new(
            "internal_error",
            "tax_long_term_days cannot be negative",
        ));
    }
    let raw_scope = optional(p, "cost_basis_pool_scope")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| "global".into());
    let scope = kassiber_pytext::strip(&raw_scope).to_lowercase();
    if scope != "global" {
        return Err(AppError::new(
            "validation",
            format!(
                "Unsupported cost-basis pool scope '{}'",
                text(p, "cost_basis_pool_scope")
            ),
        )
        .with_hint("Choose one of: global"));
    }
    Ok(scope)
}
fn details(conn: &Connection, w: &Value, p: &Value) -> Result<Value> {
    let scope = stored_pool_scope(p)?;
    let current = setting(conn, "context_profile")?.is_some_and(|id| id == text(p, "id"))
        && setting(conn, "context_workspace")?.is_some_and(|id| id == text(w, "id"));
    let mut fields = Vec::new();
    for k in [
        "id",
        "workspace_id",
        "workspace_label",
        "label",
        "fiat_currency",
        "tax_country",
        "tax_long_term_days",
        "gains_algorithm",
        "require_coarse_review",
        "bitcoin_rail_carrying_value",
        "cost_basis_pool_scope",
        "allowed_cost_basis_pool_scopes",
        "last_processed_at",
        "last_processed_tx_count",
        "created_at",
        "is_current",
    ] {
        let v = match k {
            "workspace_label" => get(w, "label").unwrap().clone(),
            "require_coarse_review" | "bitcoin_rail_carrying_value" => Value::Bool(flag(p, k)),
            "cost_basis_pool_scope" => s(scope.as_str()),
            "allowed_cost_basis_pool_scopes" => Value::List(vec![s("global")]),
            "is_current" => Value::Bool(current),
            _ => get(p, k).unwrap_or(&Value::Null).clone(),
        };
        fields.push((k, v));
    }
    Ok(dict(fields))
}
fn update(conn: &Connection, args: &Value) -> Result<Value> {
    let (w, mut p) = references::scope(conn, args)?;
    let current_pool = stored_pool_scope(&p)?;
    replace(&mut p, "cost_basis_pool_scope", s(current_pool));
    let mut merged = p.clone();
    for key in FIELDS {
        if let Some(v) = optional(args, key) {
            replace(&mut merged, key, v.clone());
        }
    }
    if let Some(v) = optional(args, "tax_long_term_days") {
        if negative(v) {
            return Err(
                AppError::new("validation", "Tax long-term days cannot be negative").with_hint(
                    "Use a non-negative integer; pass 0 to treat every disposal as short-term.",
                ),
            );
        }
    }
    if let Some(a) = optional(args, "gains_algorithm").map(py_str) {
        if !METHODS.contains(&a.to_uppercase().as_str()) {
            return Err(
                AppError::new("validation", format!("Unsupported gains algorithm '{a}'"))
                    .with_hint(format!("Choose one of: {}", METHODS.join(", "))),
            );
        }
    }
    let normalized = policy(
        &merged,
        optional(args, "gains_algorithm").is_some() || optional(args, "tax_country").is_some(),
        false,
    )?;
    let changed = FIELDS.iter().filter(|k| **k != "label").any(|k| {
        if ["require_coarse_review", "bitcoin_rail_carrying_value"].contains(k) {
            flag(&p, k) != flag(&normalized, k)
        } else {
            get(&p, k) != get(&normalized, k)
        }
    });
    conn.execute("UPDATE profiles SET label=?,fiat_currency=?,tax_country=?,tax_long_term_days=?,gains_algorithm=?,require_coarse_review=?,bitcoin_rail_carrying_value=?,cost_basis_pool_scope=? WHERE id=?",rusqlite::params![text(&normalized,"label"),text(&normalized,"fiat_currency"),text(&normalized,"tax_country"),number(get(&normalized,"tax_long_term_days").unwrap())?,text(&normalized,"gains_algorithm"),flag(&normalized,"require_coarse_review"),flag(&normalized,"bitcoin_rail_carrying_value"),text(&normalized,"cost_basis_pool_scope"),text(&p,"id")]).map_err(sql_error)?;
    if changed {
        conn.execute("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE id=?",[text(&p,"id")]).map_err(sql_error)?;
    }
    details(
        conn,
        &w,
        &one(conn, "SELECT * FROM profiles WHERE id=?", [text(&p, "id")])?,
    )
}
pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    if command == "ui.profiles.snapshot" {
        return super::profile_snapshot::snapshot(conn, sources);
    }
    match command {
        "profiles.create" => create(conn, args, sources),
        "profiles.list" => {
            let w = references::workspace(conn, args)?;
            let current = setting(conn, "context_profile")?.unwrap_or_default();
            let result=rows(conn,"SELECT id,label,fiat_currency,tax_country,tax_long_term_days,gains_algorithm,bitcoin_rail_carrying_value,cost_basis_pool_scope,created_at FROM profiles WHERE workspace_id=? ORDER BY created_at ASC",[text(&w,"id")])?;
            Ok(Value::List(
                result
                    .into_iter()
                    .map(|mut r| {
                        let rail = flag(&r, "bitcoin_rail_carrying_value");
                        replace(&mut r, "bitcoin_rail_carrying_value", Value::Bool(rail));
                        let created = get(&r, "created_at").unwrap().clone();
                        if let Value::Dict(fields) = &mut r {
                            fields.retain(|(k, _)| *k != "created_at".into());
                        }
                        let selected = text(&r, "id") == current;
                        replace(&mut r, "current", s(if selected { "yes" } else { "" }));
                        replace(&mut r, "created_at", created);
                        r
                    })
                    .collect(),
            ))
        }
        "profiles.get" => {
            let (w, p) = references::scope(conn, args)?;
            details(conn, &w, &p)
        }
        "profiles.set" => {
            if FIELDS.iter().all(|k| optional(args, k).is_none()) {
                return Err(AppError::new("validation","profiles set requires at least one field to update").with_hint("Pass one or more of --label, --fiat-currency, --tax-country, --tax-long-term-days, --gains-algorithm, --require-coarse-review, --bitcoin-rail-carrying-value, --cost-basis-pool-scope"));
            }
            update(conn, args)
        }
        _ => super::profile_desktop::execute(conn, command, args, sources),
    }
}
