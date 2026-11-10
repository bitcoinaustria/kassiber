//! Local desktop display projections. AI scoping stays in Python.
use super::*;
use kassiber_tax_countries::LongTerm;
use std::collections::BTreeSet;

pub fn human_tax_method(method: &str) -> String {
    let method = kassiber_pytext::strip(method).to_uppercase();
    match method.as_str() {
        "MOVING_AVERAGE_AT" => "ATM".into(),
        "MOVING_AVERAGE" => "moving average".into(),
        "" => "unknown".into(),
        _ => method,
    }
}
pub fn policy_method(profile: &Value) -> String {
    optional(profile, "gains_algorithm")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| "fifo".into())
        .to_lowercase()
}
pub fn tax_policy_label(profile: &Value) -> String {
    let country = optional(profile, "tax_country")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| kassiber_tax_countries::DEFAULT_CODE.into());
    let country = kassiber_pytext::strip(&country);
    let method = text(profile, "gains_algorithm");
    let fiat = text(profile, "fiat_currency");
    // The reference compares upper case.
    let upper = country.to_uppercase();
    let entry = kassiber_tax_countries::COUNTRIES
        .iter()
        .find(|entry| entry.code.to_uppercase() == upper);
    match entry {
        Some(entry) if entry.long_term == LongTerm::Never => {
            format!("{} - {} - {fiat}", entry.name_en, human_tax_method(&method))
        }
        Some(entry) if entry.long_term == LongTerm::MoreThanOneYear => format!(
            "{}{} - {method} - {fiat} - long-term after more than 1 year",
            entry.name_en,
            if entry.preview { " (preview)" } else { "" }
        ),
        // The generic label, and an unknown stored country's own code.
        _ => format!(
            "{} - {method} - {fiat} - {} day long-term",
            match entry {
                Some(entry) => entry.name_en.to_owned(),
                None => upper,
            },
            text(profile, "tax_long_term_days")
        ),
    }
}
fn relative_last(conn: &Connection, value: &str, now: &str) -> Result<String> {
    if value.is_empty() {
        return Ok("never".into());
    }
    let Some(dt) = kassiber_pytext::iso_datetime(&value.replace('Z', "+00:00")) else {
        return Ok(value.into());
    };
    let mut utc = dt.utc_us();
    if dt.offset_us.is_none() {
        // datetime.astimezone interprets a naive value in the machine timezone.
        // SQLite's utc modifier uses that same OS timezone without any I/O client.
        let seconds = dt.local_us.rem_euclid(86_400_000_000) / 1_000_000;
        let local = format!(
            "{}T{:02}:{:02}:{:02}",
            dt.date(),
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        );
        let epoch: Option<i64> = conn
            .query_row(
                "SELECT CAST(strftime('%s',?,'utc') AS INTEGER)",
                [local],
                |r| r.get(0),
            )
            .map_err(sql_error)?;
        utc = epoch.ok_or_else(|| AppError::new("internal_error", "date value out of range"))?
            * 1_000_000
            + 62_135_596_800_000_000
            + dt.local_us.rem_euclid(1_000_000);
    }
    if !(0..315_537_897_600_000_000).contains(&utc) {
        return Err(AppError::new("internal_error", "date value out of range"));
    }
    let now = kassiber_pytext::iso_datetime(now)
        .ok_or_else(|| AppError::new("internal_error", "invalid setup clock"))?
        .utc_us();
    let seconds = ((now - utc) / 1_000_000).max(0);
    Ok(if seconds < 60 {
        "just now".into()
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else if seconds < 30 * 86400 {
        format!("{}d ago", seconds / 86400)
    } else {
        dt.date()
    })
}
pub fn snapshot(conn: &Connection, sources: &dyn Sources) -> Result<Value> {
    let context = context::snapshot(conn)?;
    let profiles = rows(conn,"SELECT p.*, COUNT(DISTINCT a.id) AS account_count, COUNT(DISTINCT w.id) AS wallet_count FROM profiles p LEFT JOIN accounts a ON a.profile_id=p.id LEFT JOIN wallets w ON w.profile_id=p.id GROUP BY p.id ORDER BY p.created_at ASC,p.label ASC",[])?;
    let mut workspaces = vec![];
    for w in rows(
        conn,
        "SELECT id,label,created_at FROM workspaces ORDER BY created_at ASC,label ASC",
        [],
    )? {
        let mut currencies = BTreeSet::new();
        let mut countries = BTreeSet::new();
        let mut children = vec![];
        for p in profiles
            .iter()
            .filter(|p| get(p, "workspace_id") == get(&w, "id"))
        {
            let active = get(p, "id") == get(&context, "profile_id");
            let country = text(p, "tax_country");
            if !country.is_empty() {
                countries.insert(kassiber_pytext::strip(&country).to_lowercase());
            }
            let fiat = text(p, "fiat_currency");
            if !fiat.is_empty() {
                currencies.insert(kassiber_pytext::strip(&fiat).to_uppercase());
            }
            let last = optional(p, "last_processed_at")
                .filter(|v| truthy(v))
                .unwrap_or_else(|| get(p, "created_at").unwrap());
            let normalized = kassiber_pytext::strip(&country).to_lowercase();
            let entry = if normalized.is_empty() {
                kassiber_tax_countries::lookup(kassiber_tax_countries::DEFAULT_CODE)
            } else {
                kassiber_tax_countries::lookup(&normalized)
            };
            let Some(entry) = entry else {
                return Err(AppError::new(
                    "internal_error",
                    format!("Unsupported tax country '{country}'"),
                ));
            };
            if matches!(entry.long_term, LongTerm::ProfileDays { .. })
                && matches!(get(p,"tax_long_term_days"), Some(Value::Int(n)) if n < &0.into())
            {
                return Err(AppError::new(
                    "internal_error",
                    "tax_long_term_days cannot be negative",
                ));
            }
            let mut child = dict([
                ("id", get(p, "id").unwrap().clone()),
                ("name", get(p, "label").unwrap().clone()),
                ("taxPolicy", s(tax_policy_label(p))),
                ("fiatCurrency", get(p, "fiat_currency").unwrap().clone()),
                ("taxCountry", get(p, "tax_country").unwrap().clone()),
                (
                    "taxLongTermDays",
                    get(p, "tax_long_term_days").unwrap().clone(),
                ),
                ("gainsAlgorithm", get(p, "gains_algorithm").unwrap().clone()),
                (
                    "costBasisPoolScope",
                    get(p, "cost_basis_pool_scope").unwrap().clone(),
                ),
                (
                    "allowedCostBasisPoolScopes",
                    Value::List(entry.pool_scopes.iter().map(|scope| s(*scope)).collect()),
                ),
                ("accounts", get(p, "account_count").unwrap().clone()),
                ("wallets", get(p, "wallet_count").unwrap().clone()),
                (
                    "lastOpened",
                    s(if active {
                        "Just now".into()
                    } else {
                        relative_last(conn, &py_str(last), &sources.now(conn)?)?
                    }),
                ),
                ("active", Value::Bool(active)),
            ]);
            // Only preview countries carry the flag, so reviewed books keep
            // the reference's projection.
            if entry.preview {
                kassiber_envelope::set(&mut child, "taxCountryPreview", Value::Bool(true));
            }
            children.push(child);
        }
        let jurisdiction = if countries.is_empty() {
            "Generic".into()
        } else if let Some(entry) = (countries.len() == 1)
            .then(|| kassiber_tax_countries::lookup(countries.first().unwrap()))
            .flatten()
            .filter(|entry| entry.iso.is_some())
        {
            entry.name_en.into()
        } else if countries.len() == 1 {
            countries.first().unwrap().to_uppercase()
        } else {
            "Mixed".into()
        };
        workspaces.push(dict([
            ("id", get(&w, "id").unwrap().clone()),
            ("name", get(&w, "label").unwrap().clone()),
            (
                "currency",
                s(if currencies.len() == 1 {
                    currencies.first().unwrap().as_str()
                } else {
                    "Mixed"
                }),
            ),
            ("jurisdiction", s(jurisdiction)),
            (
                "created",
                s(text(&w, "created_at").chars().take(10).collect::<String>()),
            ),
            ("profiles", Value::List(children)),
        ]));
    }
    Ok(dict([
        ("workspaces", Value::List(workspaces)),
        (
            "activeWorkspaceId",
            get(&context, "workspace_id").unwrap().clone(),
        ),
        (
            "activeProfileId",
            get(&context, "profile_id").unwrap().clone(),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_profile_times_preserve_python_calendar_and_precision() {
        let conn = Connection::open_in_memory().unwrap();
        for (value, expected) in [
            ("", "never"),
            ("bad-date", "bad-date"),
            ("2025-12-31T23:59:00.000001Z", "just now"),
            ("2025-12-31T23:59:00Z", "1m ago"),
            ("2025-12-31T22:00:00+01:00", "3h ago"),
            ("2025-12-25T00:00:00Z", "7d ago"),
            ("20250101T120000Z", "2025-01-01"),
            ("2025-W01-1T12:00:00Z", "2024-12-30"),
            ("2027-01-01T00:00:00Z", "just now"),
        ] {
            assert_eq!(
                relative_last(&conn, value, "2026-01-01T00:00:00Z").unwrap(),
                expected,
                "{value}"
            );
        }
    }
}
