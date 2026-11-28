//! Source coverage (Rust only; docs/reference/desktop.md#sources-live-sync-and-file-snapshots).
//!
//! `ui.sources.coverage` / `wallets coverage` read the book only (plus a local
//! `stat`, and a hash when one wallet is asked for, of the file a wallet
//! remembers importing). `ui.sources.attest_coverage` /
//! `wallets attest-coverage` store or clear the user's "no activity after"
//! statement, which clears coverage warnings and nothing else.
//! `ui.sources.groups` reads which wallets each BTCPay store pays into.
use super::*;
use kassiber_envelope::get;
use kassiber_wallets::coverage;

pub(super) const COMMANDS: &[&str] = &[
    "ui.sources.coverage",
    "ui.sources.attest_coverage",
    "ui.sources.groups",
    "wallets.coverage",
    "wallets.attest-coverage",
];

fn allowed(operation: &str) -> &'static [&'static str] {
    match operation {
        "ui.sources.coverage" => &["period_end", "wallet", "check_file"],
        "ui.sources.attest_coverage" => &["wallet", "kind", "date", "note", "clear"],
        "ui.sources.groups" => &[],
        "wallets.coverage" => &["workspace", "profile", "wallet", "period_end", "check_file"],
        _ => &[
            "workspace",
            "profile",
            "wallet",
            "kind",
            "date",
            "note",
            "clear",
        ],
    }
}

fn check_fields(operation: &str, args: &Value) -> Result<(), AppError> {
    let Value::Dict(fields) = args else {
        return Ok(());
    };
    let mut unknown: Vec<String> = fields
        .iter()
        .filter(|(_, value)| operation.starts_with("ui.") || *value != Value::Null)
        .map(|(key, _)| match key {
            kassiber_pyjson::Key::Str(key) => kassiber_envelope::py_str(&Value::Str(key.clone())),
            _ => String::from("<non-string key>"),
        })
        .filter(|key| !allowed(operation).contains(&key.as_str()))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort();
    Err(AppError::new(
        "validation",
        format!("{operation} received unsupported arguments"),
    )
    .with_details(dict([(
        "unsupported",
        Value::List(unknown.into_iter().map(s).collect()),
    )])))
}

fn text(args: &Value, key: &str) -> Result<Option<String>, AppError> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(value)) => Ok(value
            .as_str()
            .map(|v| kassiber_pytext::strip(v).to_owned())
            .filter(|v| !v.is_empty())),
        Some(_) => Err(AppError::new(
            "validation",
            format!("{key} must be a string"),
        )),
    }
}

fn flag(args: &Value, key: &str) -> Result<bool, AppError> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(AppError::new(
            "validation",
            format!("{key} must be true or false"),
        )),
    }
}

impl RealBook {
    fn sources_profile(&self, args: &Value, cli: bool) -> Result<String, AppError> {
        let scope = if cli {
            dict([
                (
                    "workspace",
                    get(args, "workspace").cloned().unwrap_or(Value::Null),
                ),
                (
                    "profile",
                    get(args, "profile").cloned().unwrap_or(Value::Null),
                ),
            ])
        } else {
            dict::<&str>([])
        };
        let (_, profile) = kassiber_setup::references::scope(self.conn()?, &scope)?;
        Ok(kassiber_envelope::py_str(
            get(&profile, "id").unwrap_or(&Value::Null),
        ))
    }

    pub(super) fn sources_service(
        &mut self,
        operation: &str,
        args: &Value,
    ) -> Result<Value, AppError> {
        check_fields(operation, args)?;
        let cli = operation.starts_with("wallets.");
        let profile = self.sources_profile(args, cli)?;
        let conn = self.conn()?;
        let now = kassiber_setup::Sources::now(&SetupSources, conn)?;
        if operation == "ui.sources.groups" {
            return kassiber_wallets::groups::report(conn, &profile);
        }
        if matches!(operation, "ui.sources.coverage" | "wallets.coverage") {
            let period_end = text(args, "period_end")?;
            let wallet = text(args, "wallet")?;
            let check_file = flag(args, "check_file")?;
            if check_file && wallet.is_none() {
                return Err(AppError::new(
                    "validation",
                    "check_file compares one source's file; name its wallet",
                ));
            }
            return coverage::report(
                conn,
                &profile,
                &coverage::Options {
                    period_end: period_end.as_deref(),
                    wallet: wallet.as_deref(),
                    skip_scan_bounds: false,
                    // Only on the user's explicit request, for one source.
                    check_file,
                },
                &now,
            );
        }
        let wallet = text(args, "wallet")?
            .ok_or_else(|| AppError::new("validation", format!("{operation} requires a wallet")))?;
        let clear = flag(args, "clear")?;
        let kind = text(args, "kind")?;
        let date = text(args, "date")?;
        let note = text(args, "note")?.unwrap_or_default();
        let attestation = match (clear, kind.as_deref(), date.as_deref()) {
            (true, None, None) => None,
            (true, ..) => {
                return Err(AppError::new(
                    "validation",
                    "Clear the attestation or set one, not both",
                ))
            }
            (false, Some(kind), Some(date)) => Some((kind, date, note.as_str())),
            (false, ..) => {
                return Err(
                    AppError::new("validation", "An attestation needs a kind and a date")
                        .with_hint("kind is no_activity_after or closed_on; date is YYYY-MM-DD."),
                )
            }
        };
        coverage::attest(
            conn,
            &profile,
            &coverage::AttestationRequest {
                wallet: &wallet,
                attestation,
            },
            &now,
        )
    }
}
