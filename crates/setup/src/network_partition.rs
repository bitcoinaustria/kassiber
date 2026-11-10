//! Non-destructive partition closure and encrypted, no-overwrite publication.
use super::*;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};
#[path = "network_scopes.rs"]
mod scopes;
type Tables = Vec<(&'static str, Vec<Value>)>;
const SHARED: &[&str] = &["profiles", "workspaces", "accounts", "tags"];
fn string_set(v: &Value, message: &str) -> Result<BTreeSet<String>> {
    if !truthy(v) {
        return Ok(BTreeSet::new());
    }
    let values = match v {
        Value::List(v) => v.clone(),
        Value::Dict(v) => v
            .iter()
            .map(|(k, _)| match k {
                kassiber_pyjson::Key::Str(k) => Value::Str(k.clone()),
                _ => Value::Null,
            })
            .collect(),
        Value::Str(v) => v
            .as_str()
            .ok_or_else(|| error("validation", message))?
            .chars()
            .map(s)
            .collect(),
        _ => {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "'{}' object is not iterable",
                    kassiber_envelope::type_name(v)
                ),
            ))
        }
    };
    if let Some(v) = values
        .iter()
        .find(|v| matches!(v, Value::List(_) | Value::Dict(_)))
    {
        return Err(AppError::new(
            "internal_error",
            format!("unhashable type: '{}'", kassiber_envelope::type_name(v)),
        ));
    }
    if values.iter().any(|v| !matches!(v, Value::Str(_))) {
        return Err(error("validation", message));
    }
    Ok(values.iter().map(py_str).collect())
}
fn root(parents: &[usize], mut index: usize) -> usize {
    while parents[index] != index {
        index = parents[index];
    }
    index
}
fn references(v: &Value, ids: &BTreeMap<String, usize>, found: &mut BTreeSet<usize>) {
    match v {
        Value::Str(text) => {
            if let Some(index) = ids.get(&py_str(v)) {
                found.insert(*index);
            } else if let Some(text) = text.as_str().filter(|s| s.starts_with(['{', '['])) {
                if let Ok(v) = kassiber_pytext::loads(text) {
                    references(&v, ids, found);
                }
            }
        }
        Value::Dict(v) => {
            for (_, v) in v {
                references(v, ids, found);
            }
        }
        Value::List(v) => {
            for v in v {
                references(v, ids, found);
            }
        }
        _ => (),
    }
}
fn partition(conn: &Connection, profile: &str, args: &Value) -> Result<(Value, Tables)> {
    read_snapshot(conn, || partition_snapshot(conn, profile, args))
}
fn partition_snapshot(conn: &Connection, profile: &str, args: &Value) -> Result<(Value, Tables)> {
    let environment = text(args, "environment");
    if networks(&environment).is_none() {
        return Err(error("validation", "Choose a supported target environment"));
    }
    let mut instance = value(args, "chain_instance_id").clone();
    if environment == "regtest" {
        let text = py_str(&instance).replace("urn:", "").replace("uuid:", "");
        instance = s(
            uuid::Uuid::parse_str(&text.trim_matches(['{', '}']).replace('-', ""))
                .map_err(|_| {
                    error(
                        "validation",
                        "Regtest partitions need a local instance UUID",
                    )
                })?
                .to_string(),
        );
    } else if truthy(&instance) {
        return Err(error(
            "validation",
            "Standard networks cannot use a local instance UUID",
        ));
    }
    let inventory = inventory(conn, profile)?;
    let wallets = list(value(&inventory, "wallets"));
    let selected = string_set(
        value(args, "wallet_ids"),
        "Select connections from this book for the partition",
    )?;
    let ids: BTreeSet<String> = wallets.iter().map(|w| text(w, "wallet_id")).collect();
    if selected.is_empty() || !selected.is_subset(&ids) {
        return Err(error(
            "validation",
            "Select connections from this book for the partition",
        ));
    }
    let tables = scopes::SCOPES
        .iter()
        .map(|&(table, sql)| rows(conn, sql, [profile]).map(|rows| (table, rows)))
        .collect::<Result<Tables>>()?;
    let entries: Vec<_> = tables
        .iter()
        .flat_map(|(table, rows)| rows.iter().map(move |r| (*table, r)))
        .collect();
    let mut parents: Vec<_> = (0..entries.len()).collect();
    let identities: BTreeMap<_, _> = entries
        .iter()
        .enumerate()
        .filter(|(_, (table, row))| !SHARED.contains(table) && truthy(value(row, "id")))
        .map(|(i, (_, r))| (text(r, "id"), i))
        .collect();
    for (index, (table, row)) in entries.iter().enumerate() {
        if SHARED.contains(table) {
            continue;
        }
        if let Value::Dict(fields) = row {
            for (field, v) in fields {
                if ["raw_json", "profile_id", "workspace_id", "account_id"]
                    .iter()
                    .any(|k| field == &(*k).into())
                {
                    continue;
                }
                let mut found = BTreeSet::new();
                references(v, &identities, &mut found);
                for other in found {
                    let from = root(&parents, index);
                    parents[from] = root(&parents, other);
                }
            }
        }
    }
    // Preserve Python's insertion order of wallet-bearing components.
    let mut component_wallets: Vec<(usize, BTreeSet<String>)> = Vec::new();
    for (index, (table, row)) in entries.iter().enumerate() {
        if *table == "wallets" {
            let root = root(&parents, index);
            let position = component_wallets
                .iter()
                .position(|(i, _)| *i == root)
                .unwrap_or_else(|| {
                    component_wallets.push((root, BTreeSet::new()));
                    component_wallets.len() - 1
                });
            component_wallets[position].1.insert(text(row, "id"));
        }
    }
    let mut legacy_rows = false;
    for table in rows(conn,"SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'gl\\_%' ESCAPE '\\' ORDER BY name",[])? {
        let table = text(&table,"name").replace('"',"\"\"");
        let scoped = rows(conn,&format!("PRAGMA table_info(\"{table}\")"),[])?.iter().any(|r| value(r,"name")==&s("profile_id"));
        let sql = format!("SELECT 1 FROM \"{table}\" {} LIMIT 1",if scoped { "WHERE profile_id=?" } else { "" });
        legacy_rows |= if scoped { !rows(conn,&sql,[profile])?.is_empty() } else { !rows(conn,&sql,[])?.is_empty() };
    }
    let mut blockers = Vec::new();
    if legacy_rows {
        blockers.push(dict([("code", s("legacy_ledger_present"))]));
    }
    for (_, wallets) in &component_wallets {
        if !wallets.is_disjoint(&selected) && !wallets.is_subset(&selected) {
            blockers.push(dict([
                ("code", s("relation_crosses_partition")),
                ("wallet_ids", strings(wallets)),
            ]));
        }
    }
    let declared = string_set(
        value(args, "declared_wallet_ids"),
        "Declare only selected partition connections",
    )?;
    if !declared.is_subset(&selected) {
        return Err(error(
            "validation",
            "Declare only selected partition connections",
        ));
    }
    for wallet in wallets {
        if !selected.contains(&text(wallet, "wallet_id")) {
            continue;
        }
        if truthy(value(wallet, "conflict_count"))
            || list(value(wallet, "environments"))
                .iter()
                .any(|v| v != &s(environment.as_str()))
            || list(value(wallet, "chain_instances"))
                .iter()
                .any(|v| v != &instance)
        {
            blockers.push(dict([
                ("code", s("incompatible_wallet_history")),
                ("wallet_id", value(wallet, "wallet_id").clone()),
            ]));
        }
        if flag(wallet, "requires_declaration") && !declared.contains(&text(wallet, "wallet_id")) {
            blockers.push(dict([
                ("code", s("scope_declaration_required")),
                ("wallet_id", value(wallet, "wallet_id").clone()),
            ]));
        }
    }
    let mut kept: Tables = tables.iter().map(|(t, _)| (*t, vec![])).collect();
    for (index, (table, row)) in entries.iter().enumerate() {
        if SHARED.contains(table)
            || component_wallets
                .iter()
                .any(|(i, w)| *i == root(&parents, index) && !w.is_disjoint(&selected))
        {
            kept.iter_mut()
                .find(|(t, _)| t == table)
                .expect("known table")
                .1
                .push((*row).clone());
        }
    }
    let kept_ids: BTreeMap<_, BTreeSet<_>> = kept
        .iter()
        .map(|(t, rs)| {
            (
                *t,
                rs.iter()
                    .filter(|r| truthy(value(r, "id")))
                    .map(|r| text(r, "id"))
                    .collect(),
            )
        })
        .collect();
    for (table, rs) in &kept {
        let fks = rows(conn, &format!("PRAGMA foreign_key_list(\"{table}\")"), [])?;
        for row in rs {
            for fk in &fks {
                let field = text(fk, "from");
                if value(fk, "to") == &s("id")
                    && truthy(value(row, &field))
                    && !kept_ids
                        .get(text(fk, "table").as_str())
                        .is_some_and(|ids| ids.contains(&text(row, &field)))
                {
                    blockers.push(dict([
                        ("code", s("unresolved_relation")),
                        ("table", s(*table)),
                        ("field", s(field)),
                    ]));
                }
            }
        }
    }
    let mut recipe = dict([
        ("profile_id", s(profile)),
        ("environment", s(environment)),
        ("chain_instance_id", instance),
        ("wallet_ids", strings(&selected)),
        ("declared_wallet_ids", strings(&declared)),
        (
            "inventory_digest",
            value(&inventory, "inventory_digest").clone(),
        ),
        (
            "authored_digest",
            s(digest(&dict(
                tables.iter().map(|(t, rs)| (*t, Value::List(rs.clone()))),
            ))?),
        ),
        ("legacy_ledger_rows", Value::Bool(legacy_rows)),
    ]);
    let id = digest(&recipe)?;
    replace(&mut recipe, "plan_id", s(id));
    replace(&mut recipe, "can_apply", Value::Bool(blockers.is_empty()));
    replace(&mut recipe, "blockers", Value::List(blockers));
    replace(
        &mut recipe,
        "counts",
        dict(
            kept.iter()
                .filter(|(_, rs)| !rs.is_empty())
                .map(|(t, rs)| (*t, Value::Int(rs.len().into()))),
        ),
    );
    replace(
        &mut recipe,
        "historical_reports_remain_in_original",
        Value::Bool(true),
    );
    replace(
        &mut recipe,
        "requires_source_reconnection",
        Value::Bool(true),
    );
    Ok((recipe, kept))
}
pub fn plan(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    partition(conn, profile, args).map(|(plan, _)| plan)
}

pub fn public_wallet_config(config: &Value) -> Result<Value> {
    let config = object(config);
    let mut fields = Vec::new();
    for key in [
        "addresses",
        "altbestand",
        "chain",
        "chain_instance_id",
        "change_descriptor",
        "deprecated",
        "descriptor",
        "descriptor_source",
        "gap_limit",
        "network",
        "policy_asset",
        "script_types",
        "synthesize_change",
        "xpub",
    ] {
        if let Some(v) = get(&config, key) {
            if ["descriptor", "change_descriptor", "xpub"].contains(&key) {
                let Value::Str(v) = v else {
                    continue;
                };
                let Some(v) = v.as_str() else {
                    continue;
                };
                let text = kassiber_pytext::strip(v);
                if text.is_empty()
                    || text.to_lowercase().starts_with("ct(")
                    || text.to_lowercase().contains("slip77(")
                {
                    continue;
                }
                let mut private = false;
                for pattern in [
                    r"(?i)(?:^|[^a-z])(xprv|yprv|zprv|tprv|uprv|vprv)[a-z0-9]*",
                    r"(?<![A-Za-z0-9])[5KLc9][1-9A-HJ-NP-Za-km-z]{50,51}(?![A-Za-z0-9])",
                    r"(?<![0-9A-Fa-f])[0-9A-Fa-f]{64}(?![0-9A-Fa-f])",
                ] {
                    private |= fancy_regex::Regex::new(pattern)
                        .expect("constant regex")
                        .is_match(text)
                        .unwrap_or(true);
                }
                if private {
                    continue;
                }
            }
            fields.push((key, v.clone()));
        }
    }
    Ok(dict(fields))
}
pub struct Export<'a> {
    pub data_root: &'a Path,
    pub attachments_root: &'a Path,
    pub destination: &'a Path,
    pub recipient: Option<&'a str>,
    pub backup_passphrase: Option<&'a str>,
    pub db_passphrase: Option<&'a str>,
}
fn io_error(e: std::io::Error) -> AppError {
    AppError::new("internal_error", e.to_string())
}
fn age_binary() -> Result<std::path::PathBuf> {
    let paths = std::env::var_os("PATH").unwrap_or_else(|| "/bin:/usr/bin".into());
    for name in ["age", "rage"] {
        for directory in std::env::split_paths(&paths) {
            let path = directory.join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.into()
            });
            if !path.is_file() {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if !fs::metadata(&path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0) {
                    continue;
                }
            }
            return Ok(path);
        }
    }
    Err(AppError::new(
        "age_unavailable",
        "recipient-mode backups require an age or rage binary",
    )
    .with_hint(
        "Install `age` or `rage` on PATH, or use passphrase-mode backups with pyrage installed.",
    ))
}
pub fn export(
    conn: &Connection,
    profile: &str,
    args: &Value,
    options: Export<'_>,
    sources: &dyn Sources,
) -> Result<Value> {
    let (plan, kept) = partition(conn, profile, args)?;
    if value(&plan, "plan_id") != value(args, "plan_id") {
        return Err(error("stale_context", "Network partition preview is stale"));
    }
    if !flag(&plan, "can_apply") {
        return Err(error(
            "book_network_review_required",
            "Network partition has unresolved relationships",
        )
        .with_details(dict([("blockers", value(&plan, "blockers").clone())])));
    }
    let destination = options.destination;
    if destination.as_os_str().is_empty()
        || get(args, "file_path").is_some_and(|v| !matches!(v, Value::Str(_)))
    {
        return Err(error("validation", "Choose a backup destination"));
    }
    if destination.exists() {
        return Err(error("conflict", "Choose a new destination file"));
    }
    if options.recipient.is_some() == options.backup_passphrase.is_some() {
        return Err(error(
            "validation",
            "Choose one backup passphrase or age recipient",
        ));
    }
    if options.recipient.is_some_and(|v| !v.starts_with("age1")) {
        return Err(error("validation", "An age public recipient is required"));
    }
    if get(args, "backup_passphrase").is_some_and(|v| !matches!(v, Value::Null | Value::Str(_)))
        || options
            .backup_passphrase
            .is_some_and(|v| v.chars().count() < 12)
    {
        return Err(error(
            "validation",
            "Use a backup passphrase of at least 12 characters",
        ));
    }
    let database = kassiber_paths::resolve_database_path(options.data_root);
    if !kassiber_paths::looks_like_plaintext(&database).map_err(storage_error)?
        && options.db_passphrase.is_none()
    {
        return Err(error(
            "passphrase_required",
            "The unlocked project key is required for an encrypted partition",
        ));
    }
    let stage = tempfile::Builder::new()
        .prefix("kassiber-network-partition-")
        .tempdir()
        .map_err(io_error)?;
    let target =
        kassiber_store::Book::create(stage.path(), options.db_passphrase).map_err(storage_error)?;
    let target_conn = target.connection();
    target_conn
        .execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE")
        .map_err(sql_error)?;
    for (table, rs) in &kept {
        for row in rs {
            let mut row = row.clone();
            if *table == "transaction_edit_events" {
                if let Value::Dict(fields) = &mut row {
                    fields.retain(|(k, _)| {
                        ![
                            "sync_event_id",
                            "sync_replica_id",
                            "sync_replica_seq",
                            "sync_hlc",
                            "sync_author_member_id",
                            "sync_signature",
                            "sync_context_json",
                        ]
                        .iter()
                        .any(|v| k == &(*v).into())
                    });
                }
            }
            if *table == "profiles" {
                for (key, v) in [
                    ("last_processed_at", Value::Null),
                    ("last_processed_tx_count", Value::Int(0.into())),
                    ("last_processed_input_version", Value::Int(0.into())),
                ] {
                    replace(&mut row, key, v);
                }
            }
            if *table == "wallets" {
                let config = json(
                    &public_wallet_config(value(&row, "config_json"))?,
                    false,
                    true,
                )?;
                replace(&mut row, "config_json", s(config));
            }
            let Value::Dict(fields) = &row else {
                unreachable!()
            };
            let columns = fields
                .iter()
                .map(|(k, _)| {
                    format!(
                        "\"{}\"",
                        py_str(&Value::Str(match k {
                            kassiber_pyjson::Key::Str(v) => v.clone(),
                            _ => unreachable!(),
                        }))
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            let params = fields.iter().map(|(_, v)| match v {
                Value::Null => rusqlite::types::Value::Null,
                Value::Int(n) => rusqlite::types::Value::Integer(
                    i64::try_from(n.clone()).expect("SQLite integer"),
                ),
                Value::Float(n) => rusqlite::types::Value::Real(*n),
                _ => rusqlite::types::Value::Text(py_str(v)),
            });
            target_conn
                .execute(
                    &format!(
                        "INSERT INTO \"{table}\" ({columns}) VALUES({})",
                        vec!["?"; fields.len()].join(",")
                    ),
                    rusqlite::params_from_iter(params),
                )
                .map_err(sql_error)?;
        }
    }
    let workspace = text(
        &one(
            target_conn,
            "SELECT workspace_id FROM profiles WHERE id=?",
            [profile],
        )?,
        "workspace_id",
    );
    set_setting(target_conn, "context_profile", profile)?;
    set_setting(target_conn, "context_workspace", &workspace)?;
    let mut recipe = dict(
        ["environment", "chain_instance_id", "declared_wallet_ids"]
            .into_iter()
            .map(|k| (k, value(&plan, k).clone())),
    );
    let binding_plan = super::plan(target_conn, profile, &recipe)?;
    replace(
        &mut recipe,
        "plan_id",
        value(&binding_plan, "plan_id").clone(),
    );
    bind(target_conn, profile, &recipe, sources)?;
    if !rows(target_conn, "PRAGMA foreign_key_check", [])?.is_empty() {
        return Err(error(
            "book_network_review_required",
            "Partition does not preserve all required relationships",
        ));
    }
    target_conn.execute_batch("COMMIT").map_err(sql_error)?;
    target.close().map_err(|f| storage_error(f.error))?;
    let archive_path = stage.path().join("partition.tar");
    let mut archive = tar::Builder::new(File::create(&archive_path).map_err(io_error)?);
    archive
        .append_path_with_name(stage.path().join("kassiber.sqlite3"), "kassiber.sqlite3")
        .map_err(io_error)?;
    let attachments = kept
        .iter()
        .find(|(t, _)| *t == "attachments")
        .map(|(_, r)| r.as_slice())
        .unwrap_or(&[]);
    let manifest = dict([
        ("schema_version", Value::Int(1.into())),
        (
            "entries",
            dict([
                ("database", s("kassiber.sqlite3")),
                (
                    "attachments_files",
                    Value::Int(
                        attachments
                            .iter()
                            .filter(|r| truthy(value(r, "stored_relpath")))
                            .count()
                            .into(),
                    ),
                ),
                ("backends_env", Value::Bool(false)),
                ("settings_json", Value::Bool(false)),
            ]),
        ),
        (
            "notes",
            dict([
                ("scope", s("single_project_container")),
                (
                    "inner_db_encrypted",
                    Value::Bool(options.db_passphrase.is_some()),
                ),
                (
                    "inner_db_passphrase_required",
                    Value::Bool(options.db_passphrase.is_some()),
                ),
            ]),
        ),
        ("network_partition", {
            let mut p = dict([("version", Value::Int(1.into()))]);
            if let Value::Dict(fields) = &plan {
                for (k, v) in fields {
                    if let kassiber_pyjson::Key::Str(k) = k {
                        replace(&mut p, k.as_str().expect("key"), v.clone());
                    }
                }
            }
            p
        }),
    ]);
    let manifest = json(&manifest, false, true)?;
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    archive
        .append_data(&mut header, "manifest.json", manifest.as_bytes())
        .map_err(io_error)?;
    for attachment in attachments {
        if !truthy(value(attachment, "stored_relpath")) {
            continue;
        }
        let relative = text(attachment, "stored_relpath");
        let root = fs::canonicalize(options.attachments_root).map_err(|_| {
            error(
                "attachment_missing",
                "Partition attachment is missing or unsafe",
            )
        })?;
        let path =
            fs::canonicalize(root.join(kassiber_pytext::strip(&relative))).map_err(|_| {
                error(
                    "attachment_missing",
                    "Partition attachment is missing or unsafe",
                )
            })?;
        if Path::new(kassiber_pytext::strip(&relative)).is_absolute()
            || !path.starts_with(&root)
            || !path.is_file()
        {
            return Err(error(
                "attachment_missing",
                "Partition attachment is missing or unsafe",
            ));
        }
        let mut staged = tempfile::NamedTempFile::new_in(stage.path()).map_err(io_error)?;
        let mut source = File::open(path).map_err(io_error)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let size = source.read(&mut buffer).map_err(io_error)?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
            staged.write_all(&buffer[..size]).map_err(io_error)?;
        }
        let hash: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if value(attachment, "sha256") != &s(hash) {
            return Err(error(
                "attachment_changed",
                "Partition attachment no longer matches its stored hash",
            ));
        }
        archive
            .append_path_with_name(staged.path(), format!("attachments/{relative}"))
            .map_err(io_error)?;
    }
    archive.finish().map_err(io_error)?;
    drop(archive);
    let mut temporary =
        tempfile::NamedTempFile::new_in(destination.parent().unwrap_or(Path::new(".")))
            .map_err(io_error)?;
    if let Some(pass) = options.backup_passphrase {
        let encryption = age::Encryptor::with_user_passphrase(pass.to_owned().into());
        let mut writer = encryption
            .wrap_output(temporary.as_file_mut())
            .map_err(io_error)?;
        std::io::copy(
            &mut File::open(&archive_path).map_err(io_error)?,
            &mut writer,
        )
        .map_err(io_error)?;
        writer.finish().map_err(io_error)?;
    } else {
        // Retain Python's recipient-mode executable requirement and error behavior.
        let binary = age_binary()?;
        let recipient = options.recipient.expect("recipient");
        let status = std::process::Command::new(&binary)
            .args(["--recipient", recipient])
            .stdin(File::open(&archive_path).map_err(io_error)?)
            .stdout(temporary.reopen().map_err(io_error)?)
            .status()
            .map_err(io_error)?;
        if !status.success() {
            let command = py_str(&Value::List(vec![
                s(binary.to_string_lossy().into_owned()),
                s("--recipient"),
                s(recipient),
            ]));
            return Err(AppError::new(
                "internal_error",
                format!(
                    "Command '{command}' returned non-zero exit status {}.",
                    status.code().unwrap_or(-1)
                ),
            ));
        }
    }
    temporary.as_file_mut().sync_all().map_err(io_error)?;
    fs::hard_link(temporary.path(), destination).map_err(io_error)?;
    Ok(dict([
        ("file_path", s(destination.to_string_lossy().into_owned())),
        ("plan_id", value(&plan, "plan_id").clone()),
        ("requires_journal_rebuild", Value::Bool(true)),
        ("requires_source_reconnection", Value::Bool(true)),
    ]))
}
