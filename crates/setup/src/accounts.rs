use super::*;

pub fn normalize_asset(raw: &str) -> String {
    let t = kassiber_pytext::strip(raw);
    if t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        t.to_ascii_lowercase()
    } else if ["l-btc", "lbtc"].contains(&t.to_lowercase().as_str()) {
        "LBTC".into()
    } else {
        t.to_uppercase()
    }
}
pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    if !["accounts.list", "accounts.create"].contains(&command) {
        return Err(AppError::new(
            "not_implemented",
            "account operation is not implemented",
        ));
    }
    let (w, p) = references::scope(conn, args)?;
    let wid = text(&w, "id");
    let pid = text(&p, "id");
    if command == "accounts.list" {
        return Ok(Value::List(rows(conn,"SELECT id,code,label,account_type,COALESCE(asset,'') AS asset,created_at FROM accounts WHERE profile_id=? ORDER BY code ASC",[pid])?));
    }
    let code = kassiber_pytext::strip(&text(args, "code"))
        .to_lowercase()
        .replace(' ', "-");
    if code.is_empty() {
        return Err(AppError::new("app_error", "Code cannot be empty"));
    }
    let kind = text(args, "type").to_lowercase();
    if !["asset", "liability", "equity", "income", "expense"].contains(&kind.as_str()) {
        return Err(AppError::new("app_error",format!("Unsupported account type '{kind}'. Supported: asset, equity, expense, income, liability")));
    }
    let id = sources.uuid(conn)?;
    let asset = optional(args, "asset")
        .filter(|v| truthy(v))
        .map(|v| normalize_asset(&py_str(v)));
    conn.execute("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) VALUES(?,?,?,?,?,?,?,?)",rusqlite::params![id,wid,pid,code,text(args,"label"),kind,asset,sources.now(conn)?]).map_err(|e| {
        if integrity(&e) { AppError::new("conflict",format!("Account '{code}' already exists in profile '{}'",text(&p,"label"))).with_hint("Choose a different account code or update the existing account.") }
        else { sql_error(e) }
    })?;
    one(conn, "SELECT * FROM accounts WHERE id=?", [id])
}
