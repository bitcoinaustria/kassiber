use super::*;
use kassiber_pytext::{parse_int, strip};

pub fn handles(command: &str) -> bool {
    matches!(
        command,
        "source-funds.sources.list"
            | "source-funds.links.list"
            | "source-funds.cases.list"
            | "source-funds.recipients.list"
            | "ui.source_funds.sources.list"
            | "ui.source_funds.links.list"
            | "ui.source_funds.cases.list"
            | "ui.source_funds.recipients.list"
            | "ui.source_funds.evidence.list"
    )
}

pub fn execute(conn: &Connection, command: &str, args: &Value) -> Result<Value> {
    let desktop = command.starts_with("ui.");
    let empty = dict::<&str>([]);
    let args = if desktop {
        match args {
            Value::Null => &empty,
            Value::Dict(_) => args,
            _ => {
                return Err(AppError::new("validation", "daemon args must be an object")
                    .with_details(dict([("type", s(kassiber_envelope::type_name(args)))])))
            }
        }
    } else {
        args
    };
    let page = if command == "ui.source_funds.evidence.list" {
        Some(evidence_page(args)?)
    } else {
        None
    };
    let (_, profile) =
        kassiber_setup::references::scope(conn, if desktop { &empty } else { args })?;
    let profile_id = text(&profile, "id");
    let (key, result) = match command {
        "source-funds.sources.list" | "ui.source_funds.sources.list" => {
            ("sources", sources(conn, &profile_id)?)
        }
        "source-funds.links.list" | "ui.source_funds.links.list" => {
            ("links", links(conn, &profile_id, args, desktop)?)
        }
        "source-funds.cases.list" | "ui.source_funds.cases.list" => {
            ("cases", cases(conn, &profile_id)?)
        }
        "source-funds.recipients.list" | "ui.source_funds.recipients.list" => (
            "recipients",
            recipients(
                conn,
                &profile_id,
                desktop && truthy(value(args, "include_inactive")),
            )?,
        ),
        "ui.source_funds.evidence.list" => {
            let (offset, limit) = page.expect("evidence pagination validated");
            return evidence(conn, &profile_id, offset, limit);
        }
        _ => unreachable!(),
    };
    Ok(if desktop {
        dict([(key, result)])
    } else {
        result
    })
}

fn fallback(row: &Value, key: &str) -> Value {
    let v = value(row, key);
    if truthy(v) {
        v.clone()
    } else {
        s("")
    }
}

pub(super) fn amount(v: &Value) -> Result<Value> {
    if matches!(v, Value::Null) {
        Ok(Value::Null)
    } else {
        let n = integer(v).map_err(|message| AppError::new("internal_error", message))?;
        let d: Decimal = n
            .parse()
            .map_err(|_| AppError::new("internal_error", "invalid amount"))?;
        float(
            &d.div(&Decimal::from(100_000_000_000i64), &DECIMAL)
                .map_err(arithmetic)?,
        )
    }
}

pub(super) fn integer(v: &Value) -> std::result::Result<String, String> {
    match v {
        Value::Int(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(i64::from(*b).to_string()),
        Value::Float(n) if n.is_finite() => Ok(if n.trunc() == 0.0 {
            "0".into()
        } else {
            format!("{:.0}", n.trunc())
        }),
        Value::Float(n) if n.is_infinite() => {
            Err("cannot convert float infinity to integer".into())
        }
        Value::Float(_) => Err("cannot convert float NaN to integer".into()),
        Value::Str(raw) => {
            let raw = raw.as_str().unwrap_or_default();
            parse_int(raw).map(|n| n.to_string()).map_err(|e| match e {
                kassiber_pytext::NumberError::Invalid => format!("invalid literal for int() with base 10: {}", kassiber_pytext::repr_str(raw)),
                kassiber_pytext::NumberError::TooManyDigits(n) => format!("Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"),
            })
        }
        _ => Err(format!(
            "int() argument must be a string, a bytes-like object or a real number, not '{}'",
            kassiber_envelope::type_name(v)
        )),
    }
}

fn attachment(row: &Value) -> Result<Value> {
    Ok(dict([
        ("id", value(row, "id").clone()),
        ("attachment_type", value(row, "attachment_type").clone()),
        ("label", s(attachment_label(row)?)),
        ("transaction_id", value(row, "transaction_id").clone()),
        ("media_type", fallback(row, "media_type")),
        ("sha256", fallback(row, "sha256")),
        ("source_url", fallback(row, "source_url")),
        ("stored_relpath", fallback(row, "stored_relpath")),
    ]))
}

fn attachments(conn: &Connection, table: &str, column: &str, id: &str) -> Result<Value> {
    Ok(Value::List(
        rows(conn, &format!("SELECT a.* FROM {table} j JOIN attachments a ON a.id=j.attachment_id WHERE j.{column}=? ORDER BY j.created_at ASC,a.id ASC"), [id])?
            .iter().map(attachment).collect::<Result<_>>()?,
    ))
}

fn sources(conn: &Connection, profile: &str) -> Result<Value> {
    rows(
        conn,
        "SELECT * FROM source_funds_sources WHERE profile_id=? ORDER BY created_at DESC,id DESC",
        [profile],
    )?
    .iter()
    .map(|row| source(conn, row))
    .collect::<Result<Vec<_>>>()
    .map(Value::List)
}

pub(crate) fn source(conn: &Connection, row: &Value) -> Result<Value> {
    Ok(dict([
        ("id", value(row, "id").clone()),
        ("source_type", value(row, "source_type").clone()),
        ("label", value(row, "label").clone()),
        ("asset", value(row, "asset").clone()),
        ("amount", amount(value(row, "amount"))?),
        ("amount_msat", value(row, "amount").clone()),
        ("fiat_currency", fallback(row, "fiat_currency")),
        ("fiat_value", value(row, "fiat_value").clone()),
        ("acquired_at", fallback(row, "acquired_at")),
        ("description", fallback(row, "description")),
        ("review_state", value(row, "review_state").clone()),
        (
            "attachments",
            attachments(
                conn,
                "source_funds_source_attachments",
                "source_id",
                &text(row, "id"),
            )?,
        ),
        ("created_at", value(row, "created_at").clone()),
        ("updated_at", value(row, "updated_at").clone()),
    ]))
}

pub(super) fn resolve_transaction(
    conn: &Connection,
    profile: &str,
    reference: &str,
) -> Result<String> {
    kassiber_transactions::lookup::resolve_transaction_str(conn, profile, reference, None)
        .map(|row| kassiber_transactions::lookup::row_id(&row))
}

fn filter_text(args: &Value, key: &str, desktop: bool) -> Option<String> {
    if desktop {
        match value(args, key) {
            Value::Str(v) => v
                .as_str()
                .map(strip)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
            _ => None,
        }
    } else {
        get(args, key).filter(|v| truthy(v)).map(py_str)
    }
}

fn links(conn: &Connection, profile: &str, args: &Value, desktop: bool) -> Result<Value> {
    let target = filter_text(args, "target_transaction", desktop)
        .map(|v| resolve_transaction(conn, profile, &v))
        .transpose()?;
    let state = filter_text(args, "state", desktop)
        .map(|v| {
            let normalized = kassiber_pytext::lower(strip(&v));
            let normalized = match normalized.as_str() {
                "accept" | "accepted" => "reviewed",
                "reject" | "rejected" => "rejected",
                _ => &normalized,
            };
            if !["suggested", "reviewed", "rejected"].contains(&normalized) {
                return Err(AppError::new(
                    "validation",
                    format!("Unsupported source-funds link state '{v}'"),
                )
                .with_hint("Choose one of: suggested, reviewed, rejected"));
            }
            Ok(normalized.to_owned())
        })
        .transpose()?;
    let mut query = "SELECT * FROM source_funds_links WHERE profile_id=?".to_owned();
    let mut params = vec![profile.to_owned()];
    if let Some(target) = target {
        query.push_str(" AND to_transaction_id=?");
        params.push(target);
    }
    if let Some(state) = state {
        query.push_str(" AND state=?");
        params.push(state);
    }
    query.push_str(" ORDER BY created_at DESC,id DESC");
    rows(
        conn,
        &query,
        kassiber_store::rusqlite::params_from_iter(params),
    )?
    .iter()
    .map(|row| link(conn, row))
    .collect::<Result<Vec<_>>>()
    .map(Value::List)
}

pub(super) fn link(conn: &Connection, row: &Value) -> Result<Value> {
    let mut fields: Vec<_> = [
        "id",
        "from_source_id",
        "from_transaction_id",
        "to_transaction_id",
        "link_type",
        "state",
        "confidence",
        "method",
        "asset",
    ]
    .map(|k| (k, value(row, k).clone()))
    .into();
    fields.extend([
        (
            "allocation_amount",
            amount(value(row, "allocation_amount"))?,
        ),
        (
            "allocation_amount_msat",
            value(row, "allocation_amount").clone(),
        ),
        (
            "from_asset",
            if truthy(value(row, "from_asset")) {
                value(row, "from_asset")
            } else {
                value(row, "asset")
            }
            .clone(),
        ),
        (
            "from_allocation_amount",
            amount(value(row, "from_allocation_amount"))?,
        ),
        (
            "from_allocation_amount_msat",
            value(row, "from_allocation_amount").clone(),
        ),
        ("allocation_policy", value(row, "allocation_policy").clone()),
        ("explanation", fallback(row, "explanation")),
        (
            "uses_chain_observation",
            Value::Bool(truthy(value(row, "uses_chain_observation"))),
        ),
        (
            "chain_data_confirmed",
            Value::Bool(truthy(value(row, "chain_data_confirmed"))),
        ),
        (
            "attachments",
            attachments(
                conn,
                "source_funds_link_attachments",
                "link_id",
                &text(row, "id"),
            )?,
        ),
        ("created_at", value(row, "created_at").clone()),
        ("updated_at", value(row, "updated_at").clone()),
    ]);
    Ok(dict(fields))
}

fn cases(conn: &Connection, profile: &str) -> Result<Value> {
    rows(
        conn,
        "SELECT * FROM source_funds_cases WHERE profile_id=? ORDER BY created_at DESC,id DESC",
        [profile],
    )?
    .iter()
    .map(|row| {
        Ok(dict([
            ("id", value(row, "id").clone()),
            ("label", fallback(row, "label")),
            (
                "target_transaction_id",
                value(row, "target_transaction_id").clone(),
            ),
            ("target_external_id", fallback(row, "target_external_id")),
            ("target_amount", amount(value(row, "target_amount"))?),
            ("target_amount_msat", value(row, "target_amount").clone()),
            ("asset", value(row, "asset").clone()),
            ("reveal_mode", value(row, "reveal_mode").clone()),
            ("status", value(row, "status").clone()),
            ("snapshot_hash", value(row, "snapshot_hash").clone()),
            ("recipient_id", value(row, "recipient_id").clone()),
            ("recipient_label", fallback(row, "recipient_label_snapshot")),
            ("recipient_kind", fallback(row, "recipient_kind_snapshot")),
            (
                "recipient_reveal_mode",
                fallback(row, "recipient_reveal_mode_snapshot"),
            ),
            ("created_at", value(row, "created_at").clone()),
            ("updated_at", value(row, "updated_at").clone()),
        ]))
    })
    .collect::<Result<Vec<_>>>()
    .map(Value::List)
}

fn recipients(conn: &Connection, profile: &str, include_inactive: bool) -> Result<Value> {
    let active = if include_inactive {
        ""
    } else {
        " AND active = 1"
    };
    let order = if include_inactive { "active DESC," } else { "" };
    Ok(Value::List(rows(conn, &format!("SELECT * FROM source_funds_recipients WHERE profile_id=?{active} ORDER BY {order}label COLLATE NOCASE ASC,id ASC"), [profile])?
        .iter().map(recipient).collect()))
}

pub(super) fn recipient(row: &Value) -> Value {
    dict([
        ("id", value(row, "id").clone()),
        ("label", value(row, "label").clone()),
        ("kind", value(row, "kind").clone()),
        (
            "default_reveal_mode",
            value(row, "default_reveal_mode").clone(),
        ),
        ("notes", fallback(row, "notes")),
        ("active", Value::Bool(truthy(value(row, "active")))),
        ("created_at", value(row, "created_at").clone()),
        ("updated_at", value(row, "updated_at").clone()),
    ])
}

fn evidence_page(args: &Value) -> Result<(i64, i64)> {
    let mut unknown: Vec<_> = if let Value::Dict(fields) = args {
        fields
            .iter()
            .filter_map(|(k, _)| {
                let kassiber_pyjson::Key::Str(k) = k else {
                    return None;
                };
                (!matches!(k.as_str(), Some("limit" | "cursor"))).then(|| k.clone())
            })
            .collect()
    } else {
        Vec::new()
    };
    unknown.sort();
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            "ui.source_funds.evidence.list received unsupported fields",
        )
        .with_details(dict([(
            "unknown",
            Value::List(unknown.into_iter().map(Value::Str).collect()),
        )])));
    }
    let offset = match value(args, "cursor") {
        Value::Null => 0,
        Value::Str(v)
            if v.as_str()
                .is_some_and(|v| !v.is_empty() && v.chars().all(is_digit)) =>
        {
            let raw = v.as_str().unwrap();
            let number = parse_int(raw).map_err(|e| match e {
                kassiber_pytext::NumberError::Invalid => AppError::new("internal_error", format!("invalid literal for int() with base 10: {}", kassiber_pytext::repr_str(raw))),
                kassiber_pytext::NumberError::TooManyDigits(n) => AppError::new("internal_error", format!("Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit")),
            })?;
            if number > 2_147_483_647i64.into() {
                return Err(AppError::new(
                    "validation",
                    "ui.source_funds.evidence.list cursor is out of range",
                ));
            }
            i64::try_from(number).expect("nonnegative bounded cursor")
        }
        _ => {
            return Err(AppError::new(
                "validation",
                "ui.source_funds.evidence.list cursor is invalid",
            ))
        }
    };
    let raw = get(args, "limit")
        .cloned()
        .unwrap_or(Value::Int(100.into()));
    let limit = match &raw {
        Value::Int(n) => Some(n.clone()),
        Value::Bool(b) => Some(i64::from(*b).into()),
        Value::Float(n) if n.is_finite() => parse_int(&format!("{:.0}", n.trunc())).ok(),
        Value::Float(n) if n.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Value::Str(v) => v.as_str().and_then(|s| parse_int(s).ok()),
        _ => None,
    }
    .ok_or_else(|| {
        AppError::new("validation", "limit must be an integer")
            .with_details(dict([("value", raw.clone())]))
    })?;
    if limit < 1.into() {
        return Err(AppError::new("validation", "limit must be positive")
            .with_details(dict([("value", raw)])));
    }
    Ok((
        offset,
        if limit > 200.into() {
            200
        } else {
            i64::try_from(limit).unwrap()
        },
    ))
}

fn is_digit(c: char) -> bool {
    kassiber_pytext::decimal_value(c as u32).is_some()
        || matches!(c as u32, 0xb2..=0xb3 | 0xb9 | 0x1369..=0x1371 | 0x19da | 0x2070 | 0x2074..=0x2079 | 0x2080..=0x2089 | 0x2460..=0x2468 | 0x2474..=0x247c | 0x2488..=0x2490 | 0x24ea | 0x24f5..=0x24fd | 0x24ff | 0x2776..=0x277e | 0x2780..=0x2788 | 0x278a..=0x2792 | 0x10a40..=0x10a43 | 0x10e60..=0x10e68 | 0x11052..=0x1105a | 0x1f100..=0x1f10a)
}

fn evidence(conn: &Connection, profile: &str, offset: i64, limit: i64) -> Result<Value> {
    let mut records = rows(conn,
        "SELECT a.id,a.attachment_type,a.label,a.original_filename,a.source_url,a.media_type,a.size_bytes,a.sha256,a.copied_from_attachment_id,a.copied_from_transaction_id,a.created_at,t.id AS transaction_id,t.external_id,t.occurred_at,t.asset,w.label AS wallet FROM attachments a JOIN transactions t ON t.id=a.transaction_id JOIN wallets w ON w.id=t.wallet_id WHERE a.profile_id=? ORDER BY a.created_at DESC,a.id DESC LIMIT ? OFFSET ?",
        kassiber_store::rusqlite::params![profile, limit + 1, offset])?;
    let more = records.len() > limit as usize;
    records.truncate(limit as usize);
    for row in &mut records {
        for key in ["copied_from_attachment_id", "copied_from_transaction_id"] {
            replace(row, key, fallback(row, key));
        }
    }
    Ok(dict([
        ("attachments", Value::List(records)),
        (
            "next_cursor",
            if more {
                s((offset + limit).to_string())
            } else {
                Value::Null
            },
        ),
    ]))
}

fn clean(raw: &str) -> String {
    raw.split(|c: char| kassiber_pytext::is_space(c as u32))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn attachment_label(row: &Value) -> Result<String> {
    let label = clean(&py_str(&fallback(row, "label")));
    if !label.is_empty() {
        return Ok(label);
    }
    if text(row, "attachment_type") != "url" {
        let filename = clean(&py_str(&fallback(row, "original_filename")));
        return Ok(if filename.is_empty() {
            "File attachment".into()
        } else {
            filename
        });
    }
    url_label(&py_str(&fallback(row, "source_url")))
}

fn url_label(raw: &str) -> Result<String> {
    let raw = clean(raw);
    let raw = raw.trim_start_matches(|c: char| c as u32 <= 32);
    let (scheme, rest) = raw
        .split_once(':')
        .filter(|(scheme, _)| {
            scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        })
        .map_or((String::new(), raw), |(scheme, rest)| {
            (scheme.to_ascii_lowercase(), rest)
        });
    let mut host = String::new();
    let mut path = rest;
    if let Some(authority) = rest.strip_prefix("//") {
        let end = authority.find(['/', '?', '#']).unwrap_or(authority.len());
        path = &authority[end..];
        let authority = &authority[..end];
        let hostname = authority.rsplit('@').next().unwrap_or_default();
        if authority.contains('[') != authority.contains(']') {
            return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
        }
        if authority.contains('[') && authority.contains(']') {
            let hostname = if let Some((before, bracketed)) = hostname.split_once('[') {
                if !before.is_empty() {
                    return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
                }
                let (hostname, suffix) = bracketed.split_once(']').unwrap_or((bracketed, ""));
                if !suffix.is_empty() && !suffix.starts_with(':') {
                    return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
                }
                hostname
            } else {
                hostname.split(':').next().unwrap_or_default()
            };
            let address = hostname.split('%').next().unwrap_or_default();
            if let Some(future) = address.strip_prefix('v') {
                if !future.split_once('.').is_some_and(|(v, a)| {
                    !v.is_empty() && v.chars().all(|c| c.is_ascii_hexdigit()) && !a.is_empty()
                }) {
                    return Err(AppError::new(
                        "internal_error",
                        "IPvFuture address is invalid",
                    ));
                }
            } else if address.parse::<std::net::Ipv6Addr>().is_err()
                || hostname
                    .split_once('%')
                    .is_some_and(|(_, zone)| zone.is_empty() || zone.contains('%'))
            {
                let message = if matches!(
                    address.parse::<std::net::IpAddr>(),
                    Ok(std::net::IpAddr::V4(_))
                ) {
                    "An IPv4 address cannot be in brackets".into()
                } else {
                    format!(
                        "{} does not appear to be an IPv4 or IPv6 address",
                        kassiber_pytext::repr_str(hostname)
                    )
                };
                return Err(AppError::new("internal_error", message));
            }
        }
        let hostname = if let Some((_, bracketed)) = hostname.split_once('[') {
            bracketed.split(']').next().unwrap_or_default()
        } else {
            hostname.split(':').next().unwrap_or_default()
        };
        if authority.contains(|c: char| "⁇⁈⁉℀℁℅℆⩴︓︖﹕﹖﹟﹫＃／：？＠".contains(c))
        {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "netloc '{}' contains invalid characters under NFKC normalization",
                    authority
                ),
            ));
        }
        host = if hostname.is_empty() {
            authority.into()
        } else {
            let (name, zone) = hostname.split_once('%').map_or((hostname, ""), |(h, z)| {
                (h, &hostname[h.len()..h.len() + z.len() + 1])
            });
            format!("{}{zone}", kassiber_pytext::lower(name))
        };
        if host
            .get(..4)
            .is_some_and(|v| v.eq_ignore_ascii_case("www."))
        {
            host.drain(..4);
        }
    }
    if !matches!(scheme.as_str(), "http" | "https") {
        return Ok("Link attachment".into());
    }
    path = path.split(['?', '#']).next().unwrap_or_default();
    if let Some(last) = path.rfind('/') {
        if let Some(params) = path[last..].find(';') {
            path = &path[..last + params];
        }
    } else {
        path = path.split(';').next().unwrap_or_default();
    }
    if matches!(host.as_str(), "docs.google.com" | "drive.google.com") {
        for (route, label) in [
            ("/document/d/", "Google Doc"),
            ("/spreadsheets/d/", "Google Sheet"),
            ("/presentation/d/", "Google Slides deck"),
            ("/forms/d/", "Google Form"),
            ("/drawings/d/", "Google Drawing"),
            ("/file/d/", "Google Drive file"),
            ("/drive/folders/", "Google Drive folder"),
        ] {
            if path.starts_with(route) {
                return Ok(label.into());
            }
        }
        return Ok(if host == "drive.google.com" {
            "Google Drive link"
        } else {
            "Google Workspace link"
        }
        .into());
    }
    let last = path
        .split('/')
        .map(strip)
        .rfind(|s| !s.is_empty())
        .unwrap_or_default();
    if !last.is_empty()
        && !(last.chars().count() >= 16 && last.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
    {
        let mut decoded = Vec::new();
        let mut chars = last.as_bytes().iter().copied().peekable();
        while let Some(c) = chars.next() {
            if c == b'%' {
                let pair: Vec<_> = chars.clone().take(2).collect();
                if pair.len() == 2 {
                    if let (Some(a), Some(b)) = (
                        (pair[0] as char).to_digit(16),
                        (pair[1] as char).to_digit(16),
                    ) {
                        decoded.push((a * 16 + b) as u8);
                        chars.next();
                        chars.next();
                        continue;
                    }
                }
            }
            decoded.push(c);
        }
        let decoded = String::from_utf8_lossy(&decoded);
        let title = decoded
            .rsplit_once('.')
            .filter(|(_, ext)| {
                (2..=5).contains(&ext.chars().count())
                    && ext
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, 'İ' | 'ı' | 'ſ' | 'K'))
            })
            .map_or(decoded.as_ref(), |(stem, _)| stem);
        let title = clean(&title.replace(['-', '_', '+'], " "));
        if !title.is_empty() && kassiber_pytext::lower(&title) != kassiber_pytext::lower(&host) {
            return Ok(format!("{host} - {title}"));
        }
    }
    Ok(if host.is_empty() {
        "Link attachment".into()
    } else {
        host
    })
}
