//! Local wallet address derivation; never observes the chain or changes a book.
use crate::{database_error, descriptors, resolve};
use kassiber_envelope::{dict, py_str, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;

pub struct Derive {
    pub profile_id: String,
    pub wallet: String,
    pub branch: Option<String>,
    pub start: Value,
    pub count: Option<Value>,
}

fn integer(value: &Value) -> Result<kassiber_pyjson::BigInt, AppError> {
    py_str(value)
        .parse()
        .map_err(|_| AppError::new("app_error", "Index should be less then 2^31"))
}
pub fn derive_wallet(conn: &Connection, request: Derive) -> Result<Value, AppError> {
    let wallet = resolve(conn, &request.profile_id, &request.wallet)?;
    let raw: String = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id=?",
            [&wallet.id],
            |r| r.get::<_, Option<String>>(0),
        )
        .map_err(database_error)?
        .unwrap_or_else(|| "{}".into());
    let config: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|_| AppError::new("internal_error", "Invalid stored wallet configuration"))?;
    let plan = descriptors::load(&config)
        .map_err(descriptors::Error::into_app)?
        .ok_or_else(|| {
            AppError::new(
                "app_error",
                format!(
                    "Wallet '{}' does not have a descriptor configured",
                    wallet.label
                ),
            )
        })?;
    if py_str(&request.start).starts_with('-') {
        return Err(AppError::new(
            "app_error",
            "Descriptor derivation start must be non-negative",
        ));
    }
    let mut start = integer(&request.start)?;
    let count = request
        .count
        .as_ref()
        .filter(|v| **v != Value::Null)
        .map(integer)
        .transpose()?
        .unwrap_or(plan.gap_limit.into());
    if count <= 0.into() {
        return Err(AppError::new(
            "app_error",
            "Descriptor derivation count must be positive",
        ));
    }
    let branch = plan.branch_index(request.branch.as_deref())?;
    let mut end = &start + count;
    // Fixed descriptors emit at most index zero, even for a huge requested
    // range. Keep that clipping ahead of the BIP32 index bounds.
    if plan
        .branches
        .iter()
        .filter(|b| branch.is_none_or(|n| n == b.index))
        .all(|b| !b.ranged())
    {
        start = start.min(1.into());
        end = end.min(1.into());
    }
    let start = u32::try_from(start).map_err(|_| {
        AppError::new(
            "internal_error",
            "Hardened indexes are not allowed in wildcard",
        )
    })?;
    let end = u32::try_from(end).map_err(|_| {
        AppError::new(
            "internal_error",
            "Hardened indexes are not allowed in wildcard",
        )
    })?;
    if end > (1 << 31)
        && plan
            .branches
            .iter()
            .filter(|b| branch.is_none_or(|n| n == b.index))
            .any(|b| b.ranged())
    {
        return Err(AppError::new(
            "internal_error",
            "Hardened indexes are not allowed in wildcard",
        ));
    }
    let targets = plan
        .derive(branch, start, end)
        .map_err(descriptors::Error::into_app)?;
    Ok(Value::List(targets.iter().map(target_projection).collect()))
}

/// Python's sync_target_from_derived insertion order is part of the wire API.
fn target_projection(target: &serde_json::Value) -> Value {
    dict(
        [
            "chain",
            "network",
            "branch_index",
            "branch_label",
            "address_index",
            "address",
            "unconfidential_address",
            "script_pubkey",
            "derivation_path",
            "derivation_paths",
            "key_origins",
        ]
        .map(|key| {
            (
                key,
                kassiber_pytext::loads(&target[key].to_string()).expect("derived public JSON"),
            )
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derivation_is_read_only_scoped_and_preserves_range_refusals() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE accounts(id TEXT,code TEXT); CREATE TABLE wallets(id TEXT,profile_id TEXT,account_id TEXT,label TEXT,config_json TEXT);").unwrap();
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/vectors/python_descriptors.json")).unwrap();
        let config = &vectors["plans"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "wpkh-fixed")
            .unwrap()["config"];
        conn.execute(
            "INSERT INTO wallets VALUES('w','p',NULL,'Fixed',?)",
            [config.to_string()],
        )
        .unwrap();
        let changes = conn.total_changes();
        let request = |profile: &str, start: i64, count: i64| Derive {
            profile_id: profile.into(),
            wallet: "w".into(),
            branch: None,
            start: Value::from(start),
            count: Some(Value::from(count)),
        };
        assert_eq!(
            derive_wallet(&conn, request("q", 0, 1)).unwrap_err().code,
            "not_found"
        );
        assert_eq!(
            derive_wallet(&conn, request("p", -1, 1))
                .unwrap_err()
                .message,
            "Descriptor derivation start must be non-negative"
        );
        for count in [0, -1] {
            assert_eq!(
                derive_wallet(&conn, request("p", 0, count))
                    .unwrap_err()
                    .message,
                "Descriptor derivation count must be positive"
            );
        }
        let Value::List(targets) = derive_wallet(&conn, request("p", 0, i64::MAX)).unwrap() else {
            panic!("target list");
        };
        assert_eq!(targets.len(), 1);
        assert_eq!(
            derive_wallet(&conn, request("p", i64::MAX, 1)).unwrap(),
            Value::List(vec![])
        );
        assert_eq!(conn.total_changes(), changes);
    }
}
