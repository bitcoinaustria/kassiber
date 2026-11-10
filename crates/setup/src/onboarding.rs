//! `ui.onboarding.complete`: the first book set, book and backend policy in
//! one transaction. Choosing bundled presets configures them; it never grants
//! consent to contact them.
use super::*;

const BITCOIN_PRESETS: [&str; 2] = ["mempool", "fulcrum"];
const LIQUID_PRESETS: [&str; 2] = ["liquid", "liquid-blockstream"];

/// `_optional_string_arg(args, key)`.
fn optional_string(args: &Value, key: &str) -> Result<Option<String>> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(text)) => {
            let stripped = kassiber_pytext::strip_py(text);
            Ok((stripped.code_points().next().is_some()).then(|| py_str(&Value::Str(stripped))))
        }
        Some(_) => Err(AppError::new(
            "validation",
            format!("{key} must be a string"),
        )),
    }
}

fn save_tombstones(conn: &Connection, names: &BTreeSet<String>) -> Result<()> {
    let encoded = kassiber_pyjson::dumps(
        &Value::List(names.iter().map(s).collect()),
        &Default::default(),
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    set_setting(conn, "bootstrap_backend_tombstones", &encoded)
}

fn clear_defaults(conn: &Connection, removed: &[&str]) -> Result<()> {
    for key in ["default_backend", "bootstrap_default_backend"] {
        if setting(conn, key)?.is_some_and(|v| removed.contains(&v.as_str())) {
            conn.execute("DELETE FROM settings WHERE key = ?", [key])
                .map_err(sql_error)?;
        }
    }
    Ok(())
}

impl Config {
    /// `_onboarding_complete_payload(ctx, args)`. The runtime configuration
    /// changes only when the transaction commits.
    pub fn onboard(
        &mut self,
        conn: &Connection,
        args: &Value,
        sources: &dyn Sources,
    ) -> Result<Value> {
        if conn
            .query_row("SELECT 1 FROM workspaces LIMIT 1", [], |_| Ok(()))
            .optional()
            .map_err(sql_error)?
            .is_some()
        {
            return Err(
                AppError::new("conflict", "Onboarding has already been completed.").with_hint(
                    "Use Books and Backend settings to add or change an existing installation.",
                ),
            );
        }
        let Some(workspace_label) = optional_string(args, "workspace_label")? else {
            return Err(AppError::new("validation", "Books set name is required.")
                .with_hint("Enter a books set name."));
        };
        let Some(profile_label) = optional_string(args, "profile_label")? else {
            return Err(AppError::new("validation", "Book name is required.")
                .with_hint("Enter a book name."));
        };
        let tax_country = optional_string(args, "tax_country")?
            .unwrap_or_else(|| kassiber_tax_countries::DEFAULT_CODE.into());
        let fiat = optional_string(args, "fiat_currency")?.unwrap_or_else(|| "EUR".into());
        let gains = optional_string(args, "gains_algorithm")?;
        let days = match get(args, "tax_long_term_days") {
            None => kassiber_pyjson::BigInt::from(365),
            Some(raw) => {
                let invalid =
                    || AppError::new("validation", "tax_long_term_days must be an integer");
                match raw {
                    Value::Int(n) => n.clone(),
                    Value::Bool(b) => u8::from(*b).into(),
                    Value::Float(f) if f.is_nan() => return Err(invalid()),
                    // OverflowError escapes Python's (TypeError, ValueError).
                    Value::Float(f) if f.is_infinite() => {
                        return Err(AppError::new(
                            "internal_error",
                            "cannot convert float infinity to integer",
                        ))
                    }
                    Value::Float(f) => {
                        format!("{:.0}", f.trunc()).parse().map_err(|_| invalid())?
                    }
                    Value::Str(text) => {
                        kassiber_pytext::parse_int(text.as_str().unwrap_or_default())
                            .map_err(|_| invalid())?
                    }
                    _ => return Err(invalid()),
                }
            }
        };
        let backend_args = get(args, "backend").filter(|v| **v != Value::Null);
        let requested = optional_string(args, "backend_setup_mode")?;
        let mode = requested.clone().unwrap_or_else(|| {
            if backend_args.is_some() {
                "custom"
            } else {
                "default"
            }
            .into()
        });
        if !["default", "custom", "skip"].contains(&mode.as_str()) {
            return Err(AppError::new(
                "validation",
                "backend_setup_mode must be default, custom, or skip",
            ));
        }
        if requested.is_some() && ((mode == "custom") != backend_args.is_some()) {
            return Err(AppError::new(
                "validation",
                "Custom backend setup requires one backend; other modes must not include one",
            ));
        }
        let include_liquid = match get(args, "include_liquid_backends") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => {
                return Err(AppError::new(
                    "validation",
                    "include_liquid_backends must be a boolean",
                ))
            }
        };
        let (mut name, mut kind, mut url) = (None, None, None);
        if let Some(backend) = backend_args {
            if !matches!(backend, Value::Dict(_)) {
                return Err(AppError::new("validation", "backend must be an object"));
            }
            name = optional_string(backend, "name")?;
            kind = optional_string(backend, "kind")?;
            url = optional_string(backend, "url")?;
            if name.is_some() || kind.is_some() || url.is_some() {
                let (Some(n), Some(k), Some(_)) = (&name, &kind, &url) else {
                    return Err(AppError::new(
                        "validation",
                        "backend requires name, kind, and url",
                    ));
                };
                if !KINDS.contains(&kassiber_pytext::lower(k).as_str()) {
                    let mut kinds = KINDS.to_vec();
                    kinds.sort_unstable();
                    return Err(AppError::new(
                        "validation",
                        format!("Unsupported backend kind '{k}'"),
                    )
                    .with_hint(format!("Choose one of: {}", kinds.join(", "))));
                }
                let lowered = kassiber_pytext::lower(kassiber_pytext::strip(n));
                if conn
                    .query_row("SELECT 1 FROM backends WHERE name = ?", [&lowered], |_| {
                        Ok(())
                    })
                    .optional()
                    .map_err(sql_error)?
                    .is_some()
                {
                    return Err(
                        AppError::new("conflict", format!("Backend '{n}' already exists"))
                            .with_hint("Choose a different backend name."),
                    );
                }
            }
        }
        let complete = name.is_some() && kind.is_some() && url.is_some();
        if mode == "custom" && !complete {
            return Err(AppError::new(
                "validation",
                "Custom backend setup requires name, kind, and url",
            ));
        }
        let mut pending = self.clone();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sql_error)?;
        let staged = (|| -> Result<(Value, Value, Value, Value)> {
            let workspace = crate::workspaces::create(conn, &s(workspace_label.as_str()), sources)?;
            let mut profile_args = vec![
                ("workspace", get(&workspace, "id").cloned().unwrap()),
                ("label", s(profile_label.as_str())),
                ("fiat_currency", s(fiat.as_str())),
                ("tax_country", s(tax_country.as_str())),
                ("tax_long_term_days", Value::Int(days.clone())),
            ];
            profile_args.push(("gains_algorithm", gains.as_deref().map_or(Value::Null, s)));
            let profile = crate::profiles::create(conn, &dict(profile_args), sources)?;
            // `set_backend_bootstrap_mode(conn, mode)`.
            set_setting(
                conn,
                "backend_bootstrap_mode",
                if mode == "default" {
                    "public"
                } else {
                    "manual"
                },
            )?;
            if mode != "default" {
                let mut removed = tombstones(conn)?;
                removed.extend(BUILTINS.iter().map(|n| (*n).to_owned()));
                save_tombstones(conn, &removed)?;
                conn.execute(
                    "DELETE FROM settings WHERE key IN (?, ?)",
                    ["default_backend", "bootstrap_default_backend"],
                )
                .map_err(sql_error)?;
            } else {
                let restore = |conn: &Connection, names: &[&str]| -> Result<()> {
                    let removed = tombstones(conn)?;
                    if names.iter().any(|n| removed.contains(*n)) {
                        let kept = removed
                            .into_iter()
                            .filter(|n| !names.contains(&n.as_str()))
                            .collect();
                        save_tombstones(conn, &kept)?;
                    }
                    Ok(())
                };
                restore(conn, &BITCOIN_PRESETS)?;
                if include_liquid {
                    restore(conn, &LIQUID_PRESETS)?;
                } else {
                    // `omit_unused_bootstrap_presets`: keep referenced and
                    // environment-defined presets.
                    let mut omitted = Vec::new();
                    for preset in LIQUID_PRESETS {
                        let referenced = !matches!(wallet_refs(conn, preset)?, Value::List(ref v) if v.is_empty());
                        let environment = pending.dotenv.contains(preset)
                            || pending
                                .env_fields
                                .get(preset)
                                .is_some_and(|f| !f.is_empty());
                        if !referenced && !environment {
                            omitted.push(preset);
                        }
                    }
                    if !omitted.is_empty() {
                        let mut removed = tombstones(conn)?;
                        removed.extend(omitted.iter().map(|n| (*n).to_owned()));
                        save_tombstones(conn, &removed)?;
                        clear_defaults(conn, &omitted)?;
                    }
                    if omitted.contains(&pending.default_backend.as_str()) {
                        pending.default_backend = pending.bootstrap_default().to_owned();
                    }
                }
            }
            let mut backend = Value::Null;
            let default;
            if let (Some(backend_args), true) = (backend_args, complete) {
                let mut config = Vec::new();
                if let Some(certificate) = optional_string(backend_args, "certificate")? {
                    config.push(("certificate", s(certificate)));
                }
                if let Some(insecure) = get(backend_args, "insecure").filter(|v| **v != Value::Null)
                {
                    config.push(("insecure", Value::Bool(truthy(insecure))));
                }
                let optional = |key: &str| -> Result<Value> {
                    Ok(optional_string(backend_args, key)?.map_or(Value::Null, s))
                };
                let create_args = dict([
                    ("name", s(name.clone().unwrap())),
                    ("kind", s(kind.clone().unwrap())),
                    ("url", s(url.clone().unwrap())),
                    ("chain", optional("chain")?),
                    ("network", optional("network")?),
                    ("tor_proxy", optional("tor_proxy")?),
                    (
                        "config",
                        if config.is_empty() {
                            Value::Null
                        } else {
                            dict(config)
                        },
                    ),
                    ("notes", optional("notes")?),
                ]);
                backend = output::redact(&create(conn, &create_args, sources)?)?;
                pending.merge(conn)?;
                let chosen = set_default(
                    conn,
                    &dict([("name", s(name.clone().unwrap()))]),
                    &mut pending,
                )?;
                default = get(&chosen, "default_backend")
                    .cloned()
                    .unwrap_or(Value::Null);
            } else {
                pending.merge(conn)?;
                default = if pending.default_backend.is_empty() {
                    Value::Null
                } else {
                    s(pending.default_backend.as_str())
                };
            }
            Ok((workspace, profile, backend, default))
        })();
        let (workspace, profile, backend, default) = match staged {
            Ok(staged) => {
                conn.execute_batch("COMMIT").map_err(sql_error)?;
                staged
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(error);
            }
        };
        *self = pending;
        let profiles = crate::profile_snapshot::snapshot(conn, sources)?;
        Ok(dict([
            (
                "workspace",
                dict([
                    ("id", val(&workspace, "id")),
                    ("name", val(&workspace, "label")),
                ]),
            ),
            (
                "profile",
                dict([
                    ("id", val(&profile, "id")),
                    ("name", val(&profile, "label")),
                ]),
            ),
            (
                "defaults",
                dict([
                    ("fiat_currency", val(&profile, "fiat_currency")),
                    ("tax_country", val(&profile, "tax_country")),
                    ("tax_long_term_days", val(&profile, "tax_long_term_days")),
                    ("gains_algorithm", val(&profile, "gains_algorithm")),
                ]),
            ),
            ("backend", backend),
            ("default_backend", default),
            ("backend_setup_mode", s(mode.as_str())),
            (
                "include_liquid_backends",
                Value::Bool(mode == "default" && include_liquid),
            ),
            ("profiles", profiles),
        ]))
    }
}

fn val(row: &Value, key: &str) -> Value {
    get(row, key).cloned().unwrap_or(Value::Null)
}
